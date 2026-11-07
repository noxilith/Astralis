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
use wire::{Wire, read_frame, write_frame};

pub const ALPN: &[u8] = b"astralis/sync/1";
const TRUSTED_KEY: &str = "p2p.trusted";
const NAME_KEY: &str = "p2p.name";
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
    /// A live text update was merged into a note; `body` is the new text.
    RemoteText { peer: String, note_id: String, body: String },
    /// Pairing finished.
    Paired { peer: String, name: String },
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

/// A stable color per device (for carets and cursors).
pub fn color_for(id: &str) -> String {
    const COLORS: [&str; 8] = ["#8b7bff", "#4fd1a5", "#ff9f5a", "#ff6b81", "#6bb4ff", "#ffd166", "#c792ea", "#5ee7df"];
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

fn default_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "My device".into())
}

impl Net {
    pub async fn start(store: StoreHandle, key: SecretKey, options: NetOptions, events: EventSink) -> Result<Self> {
        let (name, trusted) = {
            let s = store.lock().unwrap();
            let name = s.get_setting(NAME_KEY)?.unwrap_or_else(default_name);
            let trusted: Vec<TrustedPeer> =
                s.get_setting(TRUSTED_KEY)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
            (name, trusted)
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
                let ids: Vec<EndpointId> = inner2.trusted.lock().unwrap().keys().copied().collect();
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
            match self.inner.clone().dial(target, Some(code.clone())).await {
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
        self.inner.sessions.lock().unwrap().remove(&id);
        self.inner.emit_peers();
        Ok(())
    }

    /// Sends a live message (text, caret, pointer) to every connected device.
    pub fn broadcast(&self, live: Live) {
        for s in self.inner.sessions.lock().unwrap().values() {
            let _ = s.tx.send(Wire::Live(live.clone()));
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
        let Wire::Hello { name, pair_code } = hello else {
            return Ok(());
        };
        let trusted = inner.trusted.lock().unwrap().contains_key(&peer);
        let accepted = trusted || pair_code.as_deref().is_some_and(|c| inner.check_code(c));
        if !accepted {
            let _ = write_frame(&mut send, &Wire::Reject { reason: "not paired".into() }).await;
            let _ = send.finish();
            conn.closed().await;
            return Ok(());
        }
        let my_name = inner.name.read().unwrap().clone();
        write_frame(&mut send, &Wire::Welcome { name: my_name })
            .await
            .map_err(|e| AcceptError::from_err(std::io::Error::other(e.to_string())))?;
        if !trusted {
            inner.trust(peer, &name);
        }
        inner.run_session(conn, send, recv, peer, name).await;
        Ok(())
    }
}

impl Inner {
    fn emit(&self, e: NetEvent) {
        (self.events)(e)
    }

    fn peers(&self) -> Vec<PeerStatus> {
        let trusted = self.trusted.lock().unwrap();
        let sessions = self.sessions.lock().unwrap();
        let nearby = self.nearby.lock().unwrap();
        let mut ids: Vec<EndpointId> = trusted.keys().chain(nearby.keys()).copied().collect();
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
                PeerStatus {
                    color: color_for(&s),
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
            TrustedPeer { id: id.to_string(), name: name.to_owned(), paired_at: astralis_core::now_ms() },
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
            || !self.trusted.lock().unwrap().contains_key(&id)
            || self.sessions.lock().unwrap().contains_key(&id)
            || self.endpoint.id() > id
        {
            return;
        }
        tokio::spawn(async move {
            if let Err(e) = self.dial(id.into(), None).await {
                tracing::debug!("couldn't reach {}: {e:#}", short_id(&id));
            }
        });
    }

    async fn dial(self: Arc<Self>, addr: EndpointAddr, pair_code: Option<String>) -> Result<PeerStatus> {
        let conn = tokio::time::timeout(Duration::from_secs(8), self.endpoint.connect(addr, ALPN))
            .await
            .context("timed out connecting")??;
        let peer = conn.remote_id();
        let (mut send, mut recv) = conn.open_bi().await?;
        let my_name = self.name.read().unwrap().clone();
        write_frame(&mut send, &Wire::Hello { name: my_name, pair_code: pair_code.clone() }).await?;
        match read_frame(&mut recv).await? {
            Wire::Welcome { name } => {
                if pair_code.is_some() || !self.trusted.lock().unwrap().contains_key(&peer) {
                    self.trust(peer, &name);
                }
                let status = PeerStatus {
                    id: peer.to_string(),
                    color: color_for(&peer.to_string()),
                    name: name.clone(),
                    trusted: true,
                    connected: true,
                    nearby: true,
                };
                let me = self.clone();
                tokio::spawn(async move { me.run_session(conn, send, recv, peer, name).await });
                Ok(status)
            }
            Wire::Reject { reason } => bail!("rejected: {reason}"),
            _ => bail!("unexpected reply"),
        }
    }

    /// Runs a connected, trusted session: sync until caught up, then stream changes and live
    /// messages both ways until the connection drops.
    async fn run_session(self: Arc<Self>, conn: Connection, mut send: SendStream, mut recv: RecvStream, peer: EndpointId, name: String) {
        let (tx, mut rx) = mpsc::unbounded_channel::<Wire>();
        let color = color_for(&peer.to_string());
        self.sessions.lock().unwrap().insert(peer, Session { name: name.clone(), tx: tx.clone() });
        self.emit_peers();

        // Writer: everything queued for this peer goes out in order.
        let writer = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if write_frame(&mut send, &msg).await.is_err() {
                    break;
                }
            }
        });

        // Tell the peer what we've seen; it answers with what we're missing.
        let my_vv = self.store.lock().unwrap().version_vector().unwrap_or_default();
        let _ = tx.send(Wire::Vv(my_vv));

        // What the peer is known to have (filled in once it sends its version vector).
        let peer_vv: Arc<Mutex<Option<VersionVector>>> = Arc::new(Mutex::new(None));

        // Pusher: send local changes as they happen.
        let pusher = {
            let me = self.clone();
            let tx = tx.clone();
            let peer_vv = peer_vv.clone();
            tokio::spawn(async move {
                let mut last: Option<VersionVector> = None;
                loop {
                    tokio::time::sleep(SYNC_TICK).await;
                    let Some(known) = peer_vv.lock().unwrap().clone() else { continue };
                    let (vv, cs) = {
                        let s = me.store.lock().unwrap();
                        let vv = s.version_vector().unwrap_or_default();
                        if last.as_ref() == Some(&vv) {
                            continue;
                        }
                        (vv.clone(), s.changes_since(&known))
                    };
                    last = Some(vv.clone());
                    if let Ok(cs) = cs {
                        if !cs.is_empty() && tx.send(Wire::Changes(cs)).is_err() {
                            break;
                        }
                        *peer_vv.lock().unwrap() = Some(merge_vv(&known, &vv));
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
            match msg {
                Wire::Vv(theirs) => {
                    let cs = self.store.lock().unwrap().changes_since(&theirs);
                    let mine = self.store.lock().unwrap().version_vector().unwrap_or_default();
                    if let Ok(cs) = cs {
                        if !cs.is_empty() {
                            let _ = tx.send(Wire::Changes(cs));
                        }
                    }
                    *peer_vv.lock().unwrap() = Some(merge_vv(&theirs, &mine));
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
                    let result = self.store.lock().unwrap().apply_changes(&cs);
                    match result {
                        Ok(stats) => self.emit(NetEvent::Synced { peer: peer.to_string(), stats, notes }),
                        Err(e) => tracing::warn!("applying changes from {} failed: {e:#}", short_id(&peer)),
                    }
                }
                Wire::Live(live) => {
                    if let Live::Text { note_id, state } = &live {
                        if let Ok(id) = note_id.parse::<uuid::Uuid>() {
                            let merged = self.store.lock().unwrap().merge_note_text(id, state);
                            if let Ok(Some(body)) = merged {
                                self.emit(NetEvent::RemoteText { peer: peer.to_string(), note_id: note_id.clone(), body });
                            }
                        }
                        continue;
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
