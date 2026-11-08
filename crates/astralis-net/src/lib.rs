//! Astralis peer-to-peer: devices find each other on the local network (mDNS), pair with a
//! 6-digit code, then keep in sync over an encrypted QUIC connection (iroh) and stream live
//! collaboration (text, carets, whiteboard pointers) while connected.
//!
//! Wire format: length-prefixed JSON frames over one bidirectional stream per connection.

mod wire;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use astralis_core::Store;
use astralis_core::sync::{ApplyStats, VersionVector};
use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use iroh_mdns_address_lookup::{DiscoveryEvent, MdnsAddressLookup};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid_like::short_id;
pub use wire::Live;
use wire::{OfferRole, VaultOffer, Wire, read_frame, write_frame};
pub use wire::JoinRequest;
use astralis_core::sync::ShareRole;

pub const ALPN: &[u8] = b"astralis/sync/1";
const TRUSTED_KEY: &str = "p2p.trusted";
const NAME_KEY: &str = "p2p.name";
const COLOR_KEY: &str = "p2p.color";
/// Why a connection was refused when the devices aren't (or are no longer) paired.
const NOT_PAIRED: &str = "not paired";
const PAIRING_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_PAIR_ATTEMPTS: u32 = 5;
const SYNC_TICK: Duration = Duration::from_millis(700);

pub type StoreHandle = Arc<Mutex<Store>>;

mod uuid_like {
    /// Short readable form of an endpoint id for logs and UI.
    pub fn short_id(id: &iroh::EndpointId) -> String {
        id.to_string().chars().take(8).collect()
    }
}

/// A device this one has paired with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedPeer {
    pub id: String,
    pub name: String,
    pub paired_at: i64,
    /// The color they chose (last seen), shown while they're offline too.
    #[serde(default)]
    pub color: Option<String>,
}

/// A device as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerStatus {
    pub id: String,
    pub name: String,
    pub color: String,
    pub trusted: bool,
    /// Currently connected and syncing.
    pub connected: bool,
    /// Seen on the local network.
    pub nearby: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NetEvent {
    /// The device list changed.
    Peers { peers: Vec<PeerStatus> },
    /// Changes from a peer were applied.
    Synced { peer: String, stats: ApplyStats, notes: Vec<String> },
    /// A live collaboration message from a connected peer.
    Live { peer: String, name: String, color: String, live: Live },
    /// Pairing finished.
    Paired { peer: String, name: String },
    /// A shared vault appeared or was removed on this device.
    VaultsChanged,
}

pub type EventSink = Arc<dyn Fn(NetEvent) + Send + Sync>;

/// How to run the endpoint. Tests use loopback without relays or mDNS.
#[derive(Debug, Clone, Copy)]
pub struct NetOptions {
    /// Use n0's public relays (needed across the internet; direct LAN works without).
    pub relays: bool,
    /// Find devices on the local network.
    pub mdns: bool,
}

impl Default for NetOptions {
    fn default() -> Self {
        Self { relays: true, mdns: true }
    }
}

struct Session {
    name: String,
    color: String,
    conn: Connection,
    tx: mpsc::UnboundedSender<Wire>,
}

struct Pairing {
    code: String,
    started: Instant,
    attempts: u32,
}

struct Inner {
    endpoint: Endpoint,
    store: StoreHandle,
    name: RwLock<String>,
    color: RwLock<String>,
    trusted: Mutex<HashMap<EndpointId, TrustedPeer>>,
    pairing: Mutex<Option<Pairing>>,
    sessions: Mutex<HashMap<EndpointId, Session>>,
    nearby: Mutex<HashMap<EndpointId, String>>,
    events: EventSink,
}

/// Running P2P service. Dropping it shuts the endpoint down.
pub struct Net {
    inner: Arc<Inner>,
    router: Router,
}

/// The note a live message is about, if any.
fn live_note(live: &Live) -> Option<&str> {
    match live {
        Live::Presence { note_id, .. } => note_id.as_deref(),
        Live::Update { note_id, .. } | Live::Awareness { note_id, .. } | Live::Scene { note_id, .. } => Some(note_id),
        Live::Profile { .. } => None,
    }
}

#[derive(Default)]
struct Reconciled {
    /// Vaults to sync now, with what the peer has seen of each.
    sync: Vec<(uuid::Uuid, VersionVector)>,
    /// Our offer changed (we accepted something): send it again.
    resend: bool,
    vaults_changed: bool,
}

/// Colors offered in Settings (and the default pick per device).
pub const COLORS: [&str; 8] = ["#8b7bff", "#4fd1a5", "#ff9f5a", "#ff6b81", "#6bb4ff", "#ffd166", "#c792ea", "#5ee7df"];

/// A stable default color per device (for carets and cursors) until the user picks one.
pub fn color_for(id: &str) -> String {
    let h = id.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    COLORS[h as usize % COLORS.len()].to_owned()
}

/// Loads this device's secret key, creating it on first run.
pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    if let Ok(bytes) = std::fs::read(path) {
        let arr: [u8; 32] = bytes.as_slice().try_into().context("corrupt device key")?;
        return Ok(SecretKey::from_bytes(&arr));
    }
    let key = SecretKey::generate();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, key.to_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(key)
}

/// The person's username on this computer (what collaborators see until they change it).
fn default_name() -> String {
    ["USER", "USERNAME", "LOGNAME"]
        .iter()
        .find_map(|k| std::env::var(k).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "Me".into())
}

/// Accepts `#rgb` / `#rrggbb` colors only (they end up in CSS).
fn valid_color(c: &str) -> bool {
    let hex = c.strip_prefix('#').unwrap_or("");
    matches!(hex.len(), 3 | 6) && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

impl Net {
    pub async fn start(store: StoreHandle, key: SecretKey, options: NetOptions, events: EventSink) -> Result<Self> {
        let (name, color, trusted) = {
            let s = store.lock().unwrap();
            let name = s.get_setting(NAME_KEY)?.unwrap_or_else(default_name);
            let color = s.get_setting(COLOR_KEY)?.filter(|c| valid_color(c));
            let trusted: Vec<TrustedPeer> =
                s.get_setting(TRUSTED_KEY)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
            (name, color, trusted)
        };
        let user_data = iroh::endpoint_info::UserData::try_from(format!("astralis:{}", truncate(&name, 200)))
            .map_err(|e| anyhow!("{e}"))?;
        let builder = if options.relays { Endpoint::builder(presets::N0) } else { Endpoint::builder(presets::Minimal) };
        let endpoint = builder
            .secret_key(key)
            .alpns(vec![ALPN.to_vec()])
            .user_data_for_address_lookup(user_data)
            .bind()
            .await?;

        let trusted = trusted
            .into_iter()
            .filter_map(|p| p.id.parse::<EndpointId>().ok().map(|id| (id, p)))
            .collect();
        let inner = Arc::new(Inner {
            endpoint: endpoint.clone(),
            store,
            name: RwLock::new(name),
            color: RwLock::new(color.unwrap_or_else(|| color_for(&endpoint.id().to_string()))),
            trusted: Mutex::new(trusted),
            pairing: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            nearby: Mutex::new(HashMap::new()),
            events,
        });
        let router = Router::builder(endpoint.clone()).accept(ALPN, Handler(inner.clone())).spawn();

        if options.mdns {
            let mdns = MdnsAddressLookup::builder().build(endpoint.id())?;
            endpoint.address_lookup()?.add(mdns.clone());
            let inner2 = inner.clone();
            tokio::spawn(async move {
                use n0_future::StreamExt;
                let mut events = mdns.subscribe().await;
                while let Some(ev) = events.next().await {
                    match ev {
                        DiscoveryEvent::Discovered { endpoint_info, .. } => {
                            let Some(name) = endpoint_info
                                .user_data()
                                .and_then(|u| u.as_ref().strip_prefix("astralis:").map(str::to_owned))
                            else {
                                continue; // not an Astralis device
                            };
                            let id = endpoint_info.endpoint_id;
                            inner2.nearby.lock().unwrap().insert(id, name);
                            inner2.emit_peers();
                            inner2.clone().maybe_connect(id);
                        }
                        DiscoveryEvent::Expired { endpoint_id } => {
                            inner2.nearby.lock().unwrap().remove(&endpoint_id);
                            inner2.emit_peers();
                        }
                        _ => {}
                    }
                }
            });
        }

        // Keep trying to reach paired devices (they may come and go).
        let inner2 = inner.clone();
        tokio::spawn(async move {
            loop {
                let mut ids: Vec<EndpointId> = inner2.trusted.lock().unwrap().keys().copied().collect();
                ids.extend(inner2.vault_peers());
                for id in ids {
                    inner2.clone().maybe_connect(id);
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });

        Ok(Net { inner, router })
    }

    pub fn id(&self) -> String {
        self.inner.endpoint.id().to_string()
    }

    /// Full address (for tests, or manual connection across networks).
    pub fn addr(&self) -> EndpointAddr {
        self.inner.endpoint.addr()
    }

    pub fn name(&self) -> String {
        self.inner.name.read().unwrap().clone()
    }

    pub fn set_name(&self, name: &str) -> Result<()> {
        let name = truncate(name.trim(), 60);
        if name.is_empty() {
            bail!("device name can't be empty");
        }
        self.inner.store.lock().unwrap().set_setting(NAME_KEY, &name)?;
        *self.inner.name.write().unwrap() = name;
        self.inner.send_profile();
        Ok(())
    }

    /// This device's color (carets, cursors, avatars) as collaborators see it.
    pub fn color(&self) -> String {
        self.inner.color.read().unwrap().clone()
    }

    pub fn set_color(&self, color: &str) -> Result<()> {
        let color = color.trim().to_ascii_lowercase();
        if !valid_color(&color) {
            bail!("not a color: {color}");
        }
        self.inner.store.lock().unwrap().set_setting(COLOR_KEY, &color)?;
        *self.inner.color.write().unwrap() = color;
        self.inner.send_profile();
        Ok(())
    }

    pub fn peers(&self) -> Vec<PeerStatus> {
        self.inner.peers()
    }

    /// Starts pairing: returns a 6-digit code for the other device to enter (valid 5 minutes).
    pub fn start_pairing(&self) -> String {
        let n = u32::from_le_bytes(self.inner.endpoint.secret_key().to_bytes()[..4].try_into().unwrap())
            ^ (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos());
        let mut buf = [0u8; 4];
        let _ = getrandom_fill(&mut buf);
        let code = format!("{:06}", (u32::from_le_bytes(buf) ^ n) % 1_000_000);
        *self.inner.pairing.lock().unwrap() = Some(Pairing { code: code.clone(), started: Instant::now(), attempts: 0 });
        code
    }

    pub fn cancel_pairing(&self) {
        *self.inner.pairing.lock().unwrap() = None;
    }

    /// An invite for pairing from anywhere (not just the local network): this device's identity
    /// plus the pairing code. It contains no IP address — the other device finds this one by
    /// identity (published via DNS, reached directly or through an encrypted relay).
    pub fn invite(&self, code: &str) -> String {
        format!("astralis:{}:{}", self.inner.endpoint.id(), code)
    }

    /// Joins a device that is showing a pairing code. `input` is either the 6-digit code (finds
    /// the device on the local network) or an invite from [`Net::invite`] (works anywhere).
    pub async fn join(&self, input: &str, addr: Option<EndpointAddr>) -> Result<PeerStatus> {
        let (invited, code) = match parse_invite(input) {
            Some((id, code)) => (Some(EndpointAddr::from(id)), code),
            None => (None, input.chars().filter(|c| c.is_ascii_digit()).collect()),
        };
        if code.len() != 6 {
            bail!("enter the 6-digit code, or paste an invite");
        }
        let targets: Vec<EndpointAddr> = match addr.or(invited) {
            Some(a) => vec![a],
            None => {
                let trusted = self.inner.trusted.lock().unwrap();
                self.inner.nearby.lock().unwrap().keys().filter(|id| !trusted.contains_key(*id)).map(|id| (*id).into()).collect()
            }
        };
        if targets.is_empty() {
            bail!("no Astralis devices found nearby — make sure both are on the same network");
        }
        for target in targets {
            match self.inner.clone().dial(target, Some(code.clone()), None).await {
                Ok(peer) => return Ok(peer),
                Err(e) => tracing::debug!("pairing attempt failed: {e:#}"),
            }
        }
        bail!("no device accepted that code — check it and try again")
    }

    /// Forgets a paired device and disconnects it.
    pub fn unpair(&self, id: &str) -> Result<()> {
        let id: EndpointId = id.parse().map_err(|e| anyhow!("{e}"))?;
        self.inner.trusted.lock().unwrap().remove(&id);
        self.inner.save_trusted()?;
        self.inner.drop_shares(id);
        // Hang up: tell the other device (so it forgets us too), then close the connection.
        let session = self.inner.sessions.lock().unwrap().remove(&id);
        if let Some(s) = session {
            let _ = s.tx.send(Wire::Unpair);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                s.conn.close(1u32.into(), b"unpaired");
            });
        }
        self.inner.emit_peers();
        Ok(())
    }

    /// Sends a live message (text, caret, pointer) to every connected device.
    pub fn broadcast(&self, live: Live) {
        let peers: Vec<EndpointId> = self.inner.sessions.lock().unwrap().keys().copied().collect();
        // Live updates about a note only go to devices it's shared with.
        let allowed: Vec<EndpointId> = match live_note(&live) {
            Some(note) => peers.into_iter().filter(|p| self.inner.peer_may_see(p, note)).collect(),
            None => peers,
        };
        let sessions = self.inner.sessions.lock().unwrap();
        for p in allowed {
            if let Some(s) = sessions.get(&p) {
                let _ = s.tx.send(Wire::Live(live.clone()));
            }
        }
    }

    /// Joins a cloud vault someone invited us to: connects to the owner's device (by identity,
    /// anywhere) and presents the invite. The vault then syncs like any shared vault.
    pub async fn join_vault(&self, owner: &str, request: JoinRequest) -> Result<PeerStatus> {
        self.join_vault_at(owner, request, None).await
    }

    /// [`Net::join_vault`] with a known address (tests, where there's no address lookup).
    pub async fn join_vault_at(&self, owner: &str, request: JoinRequest, addr: Option<EndpointAddr>) -> Result<PeerStatus> {
        let id: EndpointId = owner.parse().map_err(|_| anyhow!("the invite has a bad owner id"))?;
        let vault = request.vault;
        if !self.inner.store.lock().unwrap().accept_shared_vault(vault, owner).map_err(|e| anyhow!("{e}"))? {
            bail!("you already have this vault");
        }
        let result = self.inner.clone().dial(addr.unwrap_or_else(|| id.into()), None, Some(request)).await;
        if let Err(e) = &result {
            // Undo the pending membership; nothing of theirs arrived.
            let _ = self.inner.store.lock().unwrap().remove_share(vault, owner);
            if e.to_string().contains("timed out") || e.to_string().contains("connect") {
                bail!("couldn't reach the owner's device — it needs to be online (Astralis open) to let you in the first time");
            }
            bail!("the owner's device didn't accept the invite ({e})");
        }
        result
    }

    /// Shares changed for a device (shared, unshared, left, deleted): tell it now if connected.
    pub fn shares_changed(&self, peer: &str) {
        let Ok(id) = peer.parse::<EndpointId>() else { return };
        let msg = self.inner.offers(&id);
        if let Some(s) = self.inner.sessions.lock().unwrap().get(&id) {
            let _ = s.tx.send(msg);
        }
    }

    pub fn connected_count(&self) -> usize {
        self.inner.sessions.lock().unwrap().len()
    }

    pub async fn shutdown(self) {
        let _ = self.router.shutdown().await;
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

fn getrandom_fill(buf: &mut [u8]) -> Result<()> {
    // Cheap entropy without another dependency: hash the time and a stack address.
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let x = t ^ ((buf.as_ptr() as usize as u128) << 17);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in x.to_le_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    for (i, b) in buf.iter_mut().enumerate() {
        *b = (h >> ((i % 8) * 8)) as u8;
    }
    Ok(())
}

#[derive(Clone)]
struct Handler(Arc<Inner>);

impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AstralisHandler")
    }
}

impl ProtocolHandler for Handler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let inner = self.0.clone();
        let peer = conn.remote_id();
        let (mut send, mut recv) = conn.accept_bi().await?;
        let hello = read_frame(&mut recv).await.map_err(|e| AcceptError::from_err(std::io::Error::other(e.to_string())))?;
        let Wire::Hello { name, pair_code, join } = hello else {
            return Ok(());
        };
        let trusted = inner.trusted.lock().unwrap().contains_key(&peer);
        let paired_now = !trusted && pair_code.as_deref().is_some_and(|c| inner.check_code(c));
        // Someone invited to a cloud vault by email: their invite secret lets them in (to that
        // vault only — they aren't paired).
        let joined = join.as_ref().is_some_and(|j| {
            inner.store.lock().unwrap().redeem_cloud_invite(j.vault, &j.email, &j.secret, &peer.to_string()).unwrap_or(false)
        });
        let accepted = trusted || paired_now || joined || inner.is_vault_peer(&peer);
        if !accepted {
            let _ = write_frame(&mut send, &Wire::Reject { reason: NOT_PAIRED.into() }).await;
            let _ = send.finish();
            conn.closed().await;
            return Ok(());
        }
        let my_name = inner.name.read().unwrap().clone();
        write_frame(&mut send, &Wire::Welcome { name: my_name })
            .await
            .map_err(|e| AcceptError::from_err(std::io::Error::other(e.to_string())))?;
        if paired_now {
            inner.trust(peer, &name);
        }
        if joined {
            inner.emit(NetEvent::VaultsChanged);
        }
        inner.run_session(conn, send, recv, peer, name).await;
        Ok(())
    }
}

impl Inner {
    fn emit(&self, e: NetEvent) {
        (self.events)(e)
    }

    /// Paired, or in a vault shared with us either way (e.g. joined a cloud vault by invite).
    fn allowed(&self, peer: &EndpointId) -> bool {
        self.trusted.lock().unwrap().contains_key(peer) || self.is_vault_peer(peer)
    }

    fn is_vault_peer(&self, peer: &EndpointId) -> bool {
        self.store
            .lock()
            .unwrap()
            .shares_with(&peer.to_string())
            .is_ok_and(|s| s.iter().any(|x| x.role != ShareRole::Left))
    }

    /// Devices we share vaults with (to keep connecting to them, paired or not).
    fn vault_peers(&self) -> Vec<EndpointId> {
        let s = self.store.lock().unwrap();
        let vaults = s.list_vaults().unwrap_or_default();
        let mut out: Vec<EndpointId> = Vec::new();
        for v in vaults {
            for sh in s.vault_shares(v.id).unwrap_or_default() {
                if sh.role != ShareRole::Left
                    && let Ok(id) = sh.peer.parse::<EndpointId>()
                    && !out.contains(&id)
                {
                    out.push(id);
                }
            }
        }
        out
    }

    /// Where one of our cloud vaults lives, for a device we share it with.
    fn cloud_info_for(&self, peer: &EndpointId, vault: uuid::Uuid) -> Option<Wire> {
        let s = self.store.lock().unwrap();
        let cv = s.cloud_vault(vault).ok().flatten().filter(|c| c.owned)?;
        let shared = s.vault_shares(vault).ok()?.iter().any(|x| x.peer == peer.to_string() && x.role == ShareRole::Member);
        shared.then(|| Wire::CloudInfo { vault, provider: cv.provider, folder: cv.folder_id, key: cv.sync_key })
    }

    /// Drops a device from the paired list (it unpaired us).
    fn forget(&self, id: EndpointId) {
        if self.trusted.lock().unwrap().remove(&id).is_some() {
            if let Err(e) = self.save_trusted() {
                tracing::warn!("couldn't save paired devices: {e:#}");
            }
            tracing::info!("{} unpaired this device", short_id(&id));
        }
        self.drop_shares(id);
        self.emit_peers();
    }

    /// Unpaired: the vaults that device shared with us go away; ours stop syncing with it.
    fn drop_shares(&self, id: EndpointId) {
        match self.store.lock().unwrap().forget_peer(&id.to_string()) {
            Ok(true) => self.emit(NetEvent::VaultsChanged),
            Ok(false) => {}
            Err(e) => tracing::warn!("couldn't drop vaults shared with {}: {e:#}", short_id(&id)),
        }
    }

    /// The vaults we share with a device, and what we've seen of each.
    fn offers(&self, peer: &EndpointId) -> Wire {
        let s = self.store.lock().unwrap();
        let shares = s.shares_with(&peer.to_string()).unwrap_or_default();
        let offers = shares
            .into_iter()
            .map(|sh| VaultOffer {
                vault: sh.vault_id,
                role: match sh.role {
                    ShareRole::Member => OfferRole::Mine,
                    ShareRole::Owner => OfferRole::Yours,
                    ShareRole::Left => OfferRole::Left,
                },
                seen: if sh.role == ShareRole::Left { VersionVector::new() } else { s.vault_version_vector(sh.vault_id).unwrap_or_default() },
            })
            .collect();
        Wire::Vaults { offers }
    }

    /// Lines our shares up with a device's offer: accepts vaults it shares with us, drops ones
    /// it stopped sharing (or we left), and says which vaults to sync.
    fn reconcile(&self, peer: &EndpointId, theirs: &[VaultOffer]) -> Reconciled {
        let p = peer.to_string();
        let mut out = Reconciled::default();
        let mut s = self.store.lock().unwrap();
        let mine = s.shares_with(&p).unwrap_or_default();
        let offered: HashMap<uuid::Uuid, &VaultOffer> = theirs.iter().map(|o| (o.vault, o)).collect();
        let mut result = || -> astralis_core::Result<()> {
            for sh in &mine {
                let theirs = offered.get(&sh.vault_id);
                match (sh.role, theirs.map(|o| o.role)) {
                    (ShareRole::Member, Some(OfferRole::Yours)) | (ShareRole::Owner, Some(OfferRole::Mine)) => {
                        out.sync.push((sh.vault_id, theirs.unwrap().seen.clone()));
                    }
                    // They left our vault.
                    // (Our updated offer tells them we heard, so a later re-share isn't ignored.)
                    (ShareRole::Member, Some(OfferRole::Left)) => {
                        s.remove_share(sh.vault_id, &p)?;
                        out.resend = true;
                    }
                    // Both of us claim it: a leftover from the old merge-everything sync. The device
                    // that didn't create it joins the other's (it can't happen for newer vaults).
                    (ShareRole::Member, Some(OfferRole::Mine)) => {
                        if s.accept_shared_vault(sh.vault_id, &p)? {
                            out.sync.push((sh.vault_id, theirs.unwrap().seen.clone()));
                            out.resend = true;
                            out.vaults_changed = true;
                        }
                    }
                    // Shared while they were away: they'll accept when they see our offer.
                    (ShareRole::Member, _) => {}
                    // The owner no longer shares it with us (or deleted it): drop our copy.
                    (ShareRole::Owner, _) => {
                        s.drop_shared_vault(sh.vault_id)?;
                        out.vaults_changed = true;
                    }
                    // We left; once the owner stops offering it, forget we did.
                    (ShareRole::Left, Some(OfferRole::Mine)) => {}
                    (ShareRole::Left, _) => s.remove_share(sh.vault_id, &p)?,
                }
            }
            for o in theirs.iter().filter(|o| !mine.iter().any(|m| m.vault_id == o.vault)) {
                match o.role {
                    // A vault shared with us: accept it and ask for its contents.
                    OfferRole::Mine => {
                        if s.accept_shared_vault(o.vault, &p)? {
                            out.sync.push((o.vault, o.seen.clone()));
                            out.resend = true;
                            out.vaults_changed = true;
                        }
                    }
                    // They think they're in a vault of ours we stopped sharing: our offer says so.
                    OfferRole::Yours => out.resend = true,
                    OfferRole::Left => {}
                }
            }
            Ok(())
        };
        if let Err(e) = result() {
            tracing::warn!("couldn't line up shared vaults with {}: {e:#}", short_id(peer));
        }
        out
    }

    /// Whether a device may see (or send live updates for) a note: it's in a vault shared with it.
    fn peer_may_see(&self, peer: &EndpointId, note_id: &str) -> bool {
        let Ok(id) = note_id.parse::<uuid::Uuid>() else { return false };
        let s = self.store.lock().unwrap();
        let Ok(Some(vault)) = s.note_vault(id) else { return false };
        s.sync_scope(&peer.to_string()).is_ok_and(|scope| scope.vaults.contains(&vault))
    }

    fn profile(&self) -> Wire {
        Wire::Live(Live::Profile { name: self.name.read().unwrap().clone(), color: self.color.read().unwrap().clone() })
    }

    /// Tells connected devices our new name/color.
    fn send_profile(&self) {
        let msg = self.profile();
        for s in self.sessions.lock().unwrap().values() {
            let _ = s.tx.send(msg.clone());
        }
    }

    /// Keeps a paired device's latest name and color for when it's offline.
    fn remember_profile(&self, id: EndpointId, name: &str, color: &str) {
        let changed = match self.trusted.lock().unwrap().get_mut(&id) {
            Some(t) if t.name != name || t.color.as_deref() != Some(color) => {
                t.name = name.to_owned();
                t.color = Some(color.to_owned());
                true
            }
            _ => false,
        };
        if changed && let Err(e) = self.save_trusted() {
            tracing::warn!("couldn't save paired devices: {e:#}");
        }
    }

    fn peers(&self) -> Vec<PeerStatus> {
        let trusted = self.trusted.lock().unwrap();
        let sessions = self.sessions.lock().unwrap();
        let nearby = self.nearby.lock().unwrap();
        let mut ids: Vec<EndpointId> = trusted.keys().chain(nearby.keys()).chain(sessions.keys()).copied().collect();
        ids.sort();
        ids.dedup();
        ids.into_iter()
            .map(|id| {
                let s = id.to_string();
                let name = sessions
                    .get(&id)
                    .map(|x| x.name.clone())
                    .or_else(|| trusted.get(&id).map(|t| t.name.clone()))
                    .or_else(|| nearby.get(&id).cloned())
                    .unwrap_or_else(|| short_id(&id));
                let color = sessions
                    .get(&id)
                    .map(|x| x.color.clone())
                    .or_else(|| trusted.get(&id).and_then(|t| t.color.clone()))
                    .unwrap_or_else(|| color_for(&s));
                PeerStatus {
                    color,
                    id: s,
                    name,
                    trusted: trusted.contains_key(&id),
                    connected: sessions.contains_key(&id),
                    nearby: nearby.contains_key(&id),
                }
            })
            .collect()
    }

    fn emit_peers(&self) {
        let peers = self.peers();
        self.emit(NetEvent::Peers { peers });
    }

    fn check_code(&self, code: &str) -> bool {
        let mut guard = self.pairing.lock().unwrap();
        let Some(p) = guard.as_mut() else { return false };
        if p.started.elapsed() > PAIRING_TTL || p.attempts >= MAX_PAIR_ATTEMPTS {
            *guard = None;
            return false;
        }
        if p.code == code {
            *guard = None; // one device per code
            true
        } else {
            p.attempts += 1;
            false
        }
    }

    fn trust(&self, id: EndpointId, name: &str) {
        self.trusted.lock().unwrap().insert(
            id,
            TrustedPeer { id: id.to_string(), name: name.to_owned(), paired_at: astralis_core::now_ms(), color: None },
        );
        if let Err(e) = self.save_trusted() {
            tracing::warn!("couldn't save paired devices: {e:#}");
        }
        self.emit(NetEvent::Paired { peer: id.to_string(), name: name.to_owned() });
    }

    fn save_trusted(&self) -> Result<()> {
        let list: Vec<TrustedPeer> = self.trusted.lock().unwrap().values().cloned().collect();
        self.store.lock().unwrap().set_setting(TRUSTED_KEY, &serde_json::to_string(&list)?)?;
        Ok(())
    }

    /// Connects to a trusted peer if not already connected. The device with the smaller id
    /// dials, so two devices don't open duplicate connections to each other.
    fn maybe_connect(self: Arc<Self>, id: EndpointId) {
        if id == self.endpoint.id()
            || !self.allowed(&id)
            || self.sessions.lock().unwrap().contains_key(&id)
            || self.endpoint.id() > id
        {
            return;
        }
        tokio::spawn(async move {
            if let Err(e) = self.dial(id.into(), None, None).await {
                tracing::debug!("couldn't reach {}: {e:#}", short_id(&id));
            }
        });
    }

    async fn dial(self: Arc<Self>, addr: EndpointAddr, pair_code: Option<String>, join: Option<JoinRequest>) -> Result<PeerStatus> {
        let conn = tokio::time::timeout(Duration::from_secs(8), self.endpoint.connect(addr, ALPN))
            .await
            .context("timed out connecting")??;
        let peer = conn.remote_id();
        let (mut send, mut recv) = conn.open_bi().await?;
        let my_name = self.name.read().unwrap().clone();
        let joining = join.is_some();
        write_frame(&mut send, &Wire::Hello { name: my_name, pair_code: pair_code.clone(), join }).await?;
        match read_frame(&mut recv).await? {
            Wire::Welcome { name } => {
                // Only pairing makes devices trusted; vault members stay scoped to their vaults.
                if pair_code.is_some() {
                    self.trust(peer, &name);
                }
                let status = PeerStatus {
                    id: peer.to_string(),
                    color: color_for(&peer.to_string()),
                    name: name.clone(),
                    trusted: pair_code.is_some() || self.trusted.lock().unwrap().contains_key(&peer),
                    connected: true,
                    nearby: true,
                };
                let me = self.clone();
                tokio::spawn(async move { me.run_session(conn, send, recv, peer, name).await });
                Ok(status)
            }
            Wire::Reject { reason } => {
                // A device we trusted no longer knows us: it unpaired while we were apart.
                // (Not when an invite was refused: that says nothing about pairing.)
                if pair_code.is_none() && !joining && reason == NOT_PAIRED {
                    self.forget(peer);
                }
                bail!("rejected: {reason}")
            }
            _ => bail!("unexpected reply"),
        }
    }

    /// Runs a connected, trusted session: sync until caught up, then stream changes and live
    /// messages both ways until the connection drops.
    async fn run_session(self: Arc<Self>, conn: Connection, mut send: SendStream, mut recv: RecvStream, peer: EndpointId, name: String) {
        let (tx, mut rx) = mpsc::unbounded_channel::<Wire>();
        let mut name = name;
        let mut color =
            self.trusted.lock().unwrap().get(&peer).and_then(|t| t.color.clone()).unwrap_or_else(|| color_for(&peer.to_string()));
        self.sessions.lock().unwrap().insert(peer, Session { name: name.clone(), color: color.clone(), conn: conn.clone(), tx: tx.clone() });
        self.emit_peers();

        // Writer: everything queued for this peer goes out in order.
        let writer = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if write_frame(&mut send, &msg).await.is_err() {
                    break;
                }
            }
        });

        // Who we are (name and color), then the vaults we share and what we've seen of each;
        // the peer answers with what we're missing. Nothing outside shared vaults is ever sent.
        let _ = tx.send(self.profile());
        let _ = tx.send(self.offers(&peer));

        // Per shared vault: what the peer is known to have.
        let known: Arc<Mutex<HashMap<uuid::Uuid, VersionVector>>> = Arc::new(Mutex::new(HashMap::new()));

        // Pusher: send local changes to shared vaults as they happen.
        let pusher = {
            let me = self.clone();
            let tx = tx.clone();
            let known = known.clone();
            tokio::spawn(async move {
                let mut last: HashMap<uuid::Uuid, VersionVector> = HashMap::new();
                loop {
                    tokio::time::sleep(SYNC_TICK).await;
                    let vaults: Vec<(uuid::Uuid, VersionVector)> = known.lock().unwrap().iter().map(|(k, v)| (*k, v.clone())).collect();
                    for (vault, seen) in vaults {
                        let (vv, cs) = {
                            let s = me.store.lock().unwrap();
                            let Ok(vv) = s.vault_version_vector(vault) else { continue };
                            if last.get(&vault) == Some(&vv) {
                                continue;
                            }
                            (vv.clone(), s.vault_changes_since(vault, &seen))
                        };
                        last.insert(vault, vv.clone());
                        if let Ok(cs) = cs {
                            if !cs.is_empty() && tx.send(Wire::Changes(cs)).is_err() {
                                return;
                            }
                            if let Some(k) = known.lock().unwrap().get_mut(&vault) {
                                *k = merge_vv(&seen, &vv);
                            }
                        }
                    }
                }
            })
        };

        // Reader.
        loop {
            let msg = match read_frame(&mut recv).await {
                Ok(m) => m,
                Err(_) => break,
            };
            // Nothing more from a device once it's unpaired / out of every shared vault (even if
            // the connection lingers).
            if !self.allowed(&peer) {
                break;
            }
            match msg {
                Wire::Unpair => {
                    self.forget(peer);
                    break;
                }
                Wire::CloudInfo { vault, provider, folder, key } => {
                    // Only from the vault's owner, about a vault they shared with us.
                    let s = self.store.lock().unwrap();
                    if s.vault_owner(vault).ok().flatten().as_deref() == Some(peer.to_string().as_str()) && key.len() == 32 {
                        let uploaded_vv = s.cloud_vault(vault).ok().flatten().map(|c| c.uploaded_vv).unwrap_or_default();
                        let _ = s.set_cloud_vault(&astralis_core::cloud::CloudVault {
                            vault_id: vault,
                            provider,
                            folder_id: folder,
                            sync_key: key,
                            owned: false,
                            uploaded_vv,
                        });
                        drop(s);
                        self.emit(NetEvent::VaultsChanged);
                    }
                }
                Wire::Vaults { offers: theirs } => {
                    let outcome = self.reconcile(&peer, &theirs);
                    // Send each shared vault's missing changes, and track what the peer has.
                    let mut k = known.lock().unwrap();
                    k.retain(|v, _| outcome.sync.iter().any(|(s, _)| s == v));
                    for (vault, seen) in &outcome.sync {
                        let (cs, mine) = {
                            let s = self.store.lock().unwrap();
                            (s.vault_changes_since(*vault, seen), s.vault_version_vector(*vault).unwrap_or_default())
                        };
                        if let Ok(cs) = cs
                            && !cs.is_empty()
                        {
                            let _ = tx.send(Wire::Changes(cs));
                        }
                        k.insert(*vault, merge_vv(seen, &mine));
                    }
                    drop(k);
                    // Members of our cloud vaults learn where it lives on the drive (and its key).
                    for (vault, _) in &outcome.sync {
                        if let Some(info) = self.cloud_info_for(&peer, *vault) {
                            let _ = tx.send(info);
                        }
                    }
                    if outcome.resend {
                        let _ = tx.send(self.offers(&peer));
                    }
                    if outcome.vaults_changed {
                        self.emit(NetEvent::VaultsChanged);
                    }
                }
                Wire::Changes(cs) => {
                    let notes: Vec<String> = cs
                        .rows
                        .iter()
                        .filter(|r| r.table == "notes")
                        .filter_map(|r| match r.values.get("id") {
                            Some(astralis_core::sync::SqlValue::Blob(b)) => uuid::Uuid::from_slice(b).ok().map(|u| u.to_string()),
                            _ => None,
                        })
                        .collect();
                    let result = {
                        let s = self.store.lock().unwrap();
                        s.sync_scope(&peer.to_string()).and_then(|scope| s.apply_vault_changes(&cs, &scope))
                    };
                    match result {
                        Ok(stats) => self.emit(NetEvent::Synced { peer: peer.to_string(), stats, notes }),
                        Err(e) => tracing::warn!("applying changes from {} failed: {e:#}", short_id(&peer)),
                    }
                }
                Wire::Live(Live::Profile { name: n, color: c }) => {
                    name = truncate(n.trim(), 60);
                    if valid_color(&c) {
                        color = c;
                    }
                    if let Some(s) = self.sessions.lock().unwrap().get_mut(&peer) {
                        s.name = name.clone();
                        s.color = color.clone();
                    }
                    self.remember_profile(peer, &name, &color);
                    self.emit_peers();
                }
                Wire::Live(live) => {
                    // Only about notes in vaults we share with this device.
                    if let Some(note_id) = live_note(&live)
                        && !self.peer_may_see(&peer, note_id)
                    {
                        continue;
                    }
                    // Fold text edits into the stored note right away (the editor merges them too).
                    if let Live::Update { note_id, update } = &live
                        && let Ok(id) = note_id.parse::<uuid::Uuid>()
                        && let Err(e) = self.store.lock().unwrap().apply_note_update(id, update, false)
                    {
                        tracing::debug!("live edit for {note_id} not stored: {e:#}");
                    }
                    self.emit(NetEvent::Live { peer: peer.to_string(), name: name.clone(), color: color.clone(), live });
                }
                _ => {}
            }
        }

        pusher.abort();
        drop(tx);
        writer.abort();
        conn.close(0u32.into(), b"bye");
        let mut sessions = self.sessions.lock().unwrap();
        sessions.remove(&peer);
        drop(sessions);
        self.emit_peers();
        // Clear this peer's carets/cursors on the other side of the UI.
        self.emit(NetEvent::Live {
            peer: peer.to_string(),
            name,
            color,
            live: Live::Presence { note_id: None, anchor: None, head: None, pointer: None },
        });
    }
}

fn merge_vv(a: &VersionVector, b: &VersionVector) -> VersionVector {
    let mut out = a.clone();
    for (k, v) in b {
        let e = out.entry(k.clone()).or_default();
        if v > e {
            *e = v.clone();
        }
    }
    out
}

#[cfg(test)]
mod tests;

/// Parses `astralis:<device id>:<6-digit code>` (whitespace and line breaks tolerated).
pub fn parse_invite(s: &str) -> Option<(EndpointId, String)> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let rest = s.strip_prefix("astralis:")?;
    let (id, code) = rest.rsplit_once(':')?;
    let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    (code.len() == 6).then_some(())?;
    Some((id.parse().ok()?, code))
}

impl Net {
    /// Waits until this device is reachable from the internet (relay connected, identity
    /// published), or the timeout passes. Returns whether it got online.
    pub async fn wait_online(&self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, self.inner.endpoint.online()).await.is_ok()
    }
}
