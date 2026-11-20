//! Google Drive: sign-in, cloud vaults, email invites, and background sync through the drive.
//!
//! The OAuth client is the user's own (Settings → Connected drives). The refresh token lives in
//! this device's database; the access token only in memory.

use std::time::Duration;

use astralis_cloud::any::AnyDrive;
use astralis_cloud::drive::{CloudDrive, GoogleDrive};
use astralis_cloud::ms_oauth;
use astralis_cloud::oauth::{self, ClientConfig, Tokens};
use astralis_cloud::onedrive::OneDrive;
use astralis_cloud::reqwest;
use astralis_core::cloud::{CloudInvite, GOOGLE_DRIVE, ONEDRIVE};
use astralis_core::vault::Vault;
use astralis_core::{Error, Result};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::SharedState;

type S<'a> = State<'a, SharedState>;

const CLIENT_ID: &str = "google.client_id";
const CLIENT_SECRET: &str = "google.client_secret";
const REFRESH: &str = "google.refresh_token";
const EMAIL: &str = "google.email";
const MS_CLIENT_ID: &str = "onedrive.client_id";
const MS_REFRESH: &str = "onedrive.refresh_token";
const MS_EMAIL: &str = "onedrive.email";
/// How often cloud vaults sync through the drive (P2P handles live edits in between).
const SYNC_EVERY: Duration = Duration::from_secs(30);

/// Google sign-in state held in memory.
pub struct Cloud {
    http: reqwest::Client,
    tokens: tokio::sync::Mutex<Option<Tokens>>,
    ms_tokens: tokio::sync::Mutex<Option<Tokens>>,
}

impl Default for Cloud {
    fn default() -> Self {
        Self { http: astralis_cloud::http_client(), tokens: Default::default(), ms_tokens: Default::default() }
    }
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("{e:#}"))
}

fn setting(state: &SharedState, key: &str) -> Option<String> {
    state.store.lock().unwrap().get_setting(key).ok().flatten().filter(|s| !s.is_empty())
}

/// The Google client baked in at build time (see build.rs), if any.
fn builtin_client() -> Option<ClientConfig> {
    match (option_env!("ASTRALIS_BUILTIN_GOOGLE_CLIENT_ID"), option_env!("ASTRALIS_BUILTIN_GOOGLE_CLIENT_SECRET")) {
        (Some(id), Some(secret)) => Some(ClientConfig { client_id: id.into(), client_secret: secret.into() }),
        _ => None,
    }
}

/// A client entered in Settings wins; otherwise the built-in one.
fn client(state: &SharedState) -> Result<ClientConfig> {
    match (setting(state, CLIENT_ID), setting(state, CLIENT_SECRET)) {
        (Some(client_id), Some(client_secret)) => Ok(ClientConfig { client_id, client_secret }),
        _ => builtin_client().ok_or_else(|| Error::Invalid("set up your Google OAuth client in Settings → Connected drives first".into())),
    }
}

/// The app handle, kept for opening links on phones.
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

pub fn remember_app(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
}

/// Opens a URL in the user's browser.
fn open_browser(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let cmd = {
        use tauri_plugin_opener::OpenerExt;
        match APP.get() {
            Some(app) => app.opener().open_url(url, None::<&str>).map_err(std::io::Error::other),
            None => Err(std::io::Error::other("the app isn't ready")),
        }
    };
    cmd.map(|_| ()).map_err(|e| anyhow::anyhow!("couldn't open the browser: {e}"))
}

/// A Drive client with a fresh access token (refreshed as needed).
async fn drive(state: &SharedState) -> Result<GoogleDrive> {
    let mut tokens = state.cloud.tokens.lock().await;
    if let Some(t) = tokens.as_ref().filter(|t| t.fresh()) {
        return Ok(GoogleDrive::new(state.cloud.http.clone(), t.access_token.clone()));
    }
    let cfg = client(state)?;
    let refresh = setting(state, REFRESH).ok_or_else(|| Error::Invalid("connect Google Drive in Settings first".into()))?;
    let t = oauth::refresh(&state.cloud.http, &cfg, &refresh).await.map_err(err)?;
    let d = GoogleDrive::new(state.cloud.http.clone(), t.access_token.clone());
    *tokens = Some(t);
    Ok(d)
}

/// The Microsoft app's client ID: one entered in Settings wins, otherwise the built-in one.
fn ms_client_id(state: &SharedState) -> Result<String> {
    setting(state, MS_CLIENT_ID)
        .or_else(|| option_env!("ASTRALIS_BUILTIN_ONEDRIVE_CLIENT_ID").map(str::to_owned).filter(|s| !s.is_empty()))
        .ok_or_else(|| Error::Invalid("OneDrive isn't set up yet: add the Microsoft app's client ID in Settings → Connected drives first".into()))
}

/// A OneDrive client with a fresh access token (refreshed as needed; Microsoft hands out a new
/// refresh token each time, which is kept).
async fn onedrive(state: &SharedState) -> Result<OneDrive> {
    let mut tokens = state.cloud.ms_tokens.lock().await;
    if let Some(t) = tokens.as_ref().filter(|t| t.fresh()) {
        return OneDrive::connect(state.cloud.http.clone(), t.access_token.clone()).await.map_err(err);
    }
    let id = ms_client_id(state)?;
    let refresh = setting(state, MS_REFRESH).ok_or_else(|| Error::Invalid("connect OneDrive in Settings first".into()))?;
    let t = ms_oauth::refresh(&state.cloud.http, &id, &refresh).await.map_err(err)?;
    if let Some(r) = &t.refresh_token {
        state.store.lock().unwrap().set_setting(MS_REFRESH, r)?;
    }
    let d = OneDrive::connect(state.cloud.http.clone(), t.access_token.clone()).await.map_err(err)?;
    *tokens = Some(t);
    Ok(d)
}

/// The provider project snapshots go to: the one chosen (setting `projects.provider`), else
/// whichever of Google Drive and OneDrive is connected.
pub(crate) fn projects_provider(state: &SharedState) -> Option<&'static str> {
    let connected = |p: &str| setting(state, if p == ONEDRIVE { MS_REFRESH } else { REFRESH }).is_some();
    let chosen = setting(state, "projects.provider");
    [chosen.as_deref().unwrap_or(""), GOOGLE_DRIVE, ONEDRIVE].into_iter().find(|p| !p.is_empty() && connected(p)).map(|p| if p == ONEDRIVE { ONEDRIVE } else { GOOGLE_DRIVE })
}

/// The drive for project snapshots, if one is connected.
pub(crate) async fn projects_drive(state: &SharedState) -> Result<Option<AnyDrive>> {
    match projects_provider(state) {
        Some(p) => Ok(Some(any_drive(state, p).await?)),
        None => Ok(None),
    }
}

/// The drive of a provider ("onedrive", otherwise Google Drive).
async fn any_drive(state: &SharedState, provider: &str) -> Result<AnyDrive> {
    if provider == ONEDRIVE {
        Ok(AnyDrive::One(onedrive(state).await?))
    } else {
        Ok(AnyDrive::Google(drive(state).await?))
    }
}

fn provider_of(p: Option<String>) -> &'static str {
    if p.as_deref() == Some(ONEDRIVE) { ONEDRIVE } else { GOOGLE_DRIVE }
}

/// The drive a cloud vault lives on.
async fn vault_drive(state: &SharedState, vault: Uuid) -> Result<AnyDrive> {
    let cv = state.store.lock().unwrap().cloud_vault(vault)?.ok_or_else(|| Error::Invalid("this isn't a cloud vault".into()))?;
    any_drive(state, &cv.provider).await
}

/// Runs the browser sign-in (with the Picker if `picker`) and remembers the account.
async fn sign_in(state: &SharedState, picker: bool) -> Result<(Vec<String>, String)> {
    let cfg = client(state)?;
    let done = oauth::sign_in(&state.cloud.http, &cfg, picker, open_browser).await.map_err(err)?;
    let d = GoogleDrive::new(state.cloud.http.clone(), done.tokens.access_token.clone());
    let email = d.email().await.map_err(err)?;
    {
        let s = state.store.lock().unwrap();
        if let Some(r) = &done.tokens.refresh_token {
            s.set_setting(REFRESH, r)?;
        }
        s.set_setting(EMAIL, &email)?;
    }
    *state.cloud.tokens.lock().await = Some(done.tokens);
    Ok((done.picked, email))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleStatus {
    /// An OAuth client is available (built in or entered).
    configured: bool,
    /// The app comes with one (nothing to set up).
    builtin: bool,
    client_id: Option<String>,
    /// Signed in as.
    email: Option<String>,
}

#[tauri::command]
pub fn google_status(state: S<'_>) -> Result<GoogleStatus> {
    Ok(GoogleStatus {
        configured: client(&state).is_ok(),
        builtin: builtin_client().is_some(),
        client_id: setting(&state, CLIENT_ID),
        email: setting(&state, REFRESH).and(setting(&state, EMAIL)),
    })
}

#[tauri::command]
pub fn google_set_client(state: S<'_>, client_id: String, client_secret: String) -> Result<()> {
    let id = client_id.trim();
    if !id.ends_with(".apps.googleusercontent.com") {
        return Err(Error::Invalid("that doesn't look like a Google OAuth client ID (it ends in .apps.googleusercontent.com)".into()));
    }
    if client_secret.trim().is_empty() {
        return Err(Error::Invalid("paste the client secret too".into()));
    }
    let s = state.store.lock().unwrap();
    s.set_setting(CLIENT_ID, id)?;
    s.set_setting(CLIENT_SECRET, client_secret.trim())?;
    Ok(())
}

/// Opens Google sign-in in the browser; returns the connected account's email.
#[tauri::command]
pub async fn google_connect(state: S<'_>) -> Result<String> {
    Ok(sign_in(state.inner(), false).await?.1)
}

#[tauri::command]
pub async fn google_disconnect(state: S<'_>) -> Result<()> {
    if let Some(r) = setting(&state, REFRESH) {
        // Best effort: also revoke it at Google.
        let _ = state.cloud.http.post("https://oauth2.googleapis.com/revoke").form(&[("token", r)]).send().await;
    }
    *state.cloud.tokens.lock().await = None;
    let s = state.store.lock().unwrap();
    s.set_setting(REFRESH, "")?;
    s.set_setting(EMAIL, "")?;
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OneDriveStatus {
    /// A Microsoft app is available (built in or entered).
    configured: bool,
    builtin: bool,
    client_id: Option<String>,
    email: Option<String>,
}

#[tauri::command]
pub fn onedrive_status(state: S<'_>) -> Result<OneDriveStatus> {
    Ok(OneDriveStatus {
        configured: ms_client_id(&state).is_ok(),
        builtin: option_env!("ASTRALIS_BUILTIN_ONEDRIVE_CLIENT_ID").is_some_and(|s| !s.is_empty()),
        client_id: setting(&state, MS_CLIENT_ID),
        email: setting(&state, MS_REFRESH).and(setting(&state, MS_EMAIL)),
    })
}

#[tauri::command]
pub fn onedrive_set_client(state: S<'_>, client_id: String) -> Result<()> {
    let id = client_id.trim();
    let looks_right = id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') && id.matches('-').count() == 4;
    if !looks_right {
        return Err(Error::Invalid("that doesn't look like an Application (client) ID (it's 36 characters with dashes, like 11111111-2222-3333-4444-555555555555)".into()));
    }
    state.store.lock().unwrap().set_setting(MS_CLIENT_ID, id)?;
    Ok(())
}

/// Opens Microsoft sign-in in the browser; returns the connected account's email.
#[tauri::command]
pub async fn onedrive_connect(state: S<'_>) -> Result<String> {
    let id = ms_client_id(&state)?;
    let tokens = ms_oauth::sign_in(&state.cloud.http, &id, open_browser).await.map_err(err)?;
    let d = OneDrive::connect(state.cloud.http.clone(), tokens.access_token.clone()).await.map_err(err)?;
    let email = CloudDrive::email(&d).await.map_err(err)?;
    {
        let s = state.store.lock().unwrap();
        if let Some(r) = &tokens.refresh_token {
            s.set_setting(MS_REFRESH, r)?;
        }
        s.set_setting(MS_EMAIL, &email)?;
    }
    *state.cloud.ms_tokens.lock().await = Some(tokens);
    Ok(email)
}

#[tauri::command]
pub async fn onedrive_disconnect(state: S<'_>) -> Result<()> {
    *state.cloud.ms_tokens.lock().await = None;
    let s = state.store.lock().unwrap();
    s.set_setting(MS_REFRESH, "")?;
    s.set_setting(MS_EMAIL, "")?;
    Ok(())
}

fn me(state: &SharedState) -> Result<(String, String)> {
    let net = state.net.get().ok_or_else(|| Error::Invalid("peer-to-peer is still starting; try again in a moment".into()))?;
    Ok((net.id(), net.name()))
}

/// A new vault in `Astralis/<name>/` on Google Drive.
#[tauri::command]
pub async fn cloud_create_vault(state: S<'_>, name: String, provider: Option<String>) -> Result<Vault> {
    let (device, device_name) = me(&state)?;
    let d = any_drive(&state, provider_of(provider)).await?;
    astralis_cloud::create_cloud_vault(&d, &state.store, &name, &device, &device_name).await.map_err(err)
}

/// Puts one of the user's own plain vaults on the Drive (used for the Brain).
pub async fn cloud_enable(state: &SharedState, vault: Uuid, name: &str) -> Result<()> {
    let (device, device_name) = me(state)?;
    let d = drive(state).await?;
    astralis_cloud::make_cloud(&d, &state.store, vault, name, &device, &device_name).await.map_err(err)
}

#[tauri::command]
pub fn cloud_invites(state: S<'_>, vault_id: Uuid) -> Result<Vec<CloudInvite>> {
    state.store.lock().unwrap().cloud_invites(vault_id)
}

/// Starts the error the app recognizes: a member's older Astralis still has files in the folder
/// (see `astralis_cloud::members_files_in_the_way`).
const MEMBERS_FILES: &str = "members-files-in-folder";

fn sharing_err(e: anyhow::Error) -> Error {
    if astralis_cloud::members_files_in_the_way(&e) {
        Error::Invalid(format!(
            "{MEMBERS_FILES}: someone in this vault still has files in its Drive folder from an older Astralis, and Google won't change who the folder is shared with until they're gone. They disappear once everyone in the vault has updated Astralis (1.0.2 or newer) and opened it."
        ))
    } else {
        err(e)
    }
}

/// Invites someone by Google email (shares the drive folder; Google emails them).
#[tauri::command]
pub async fn cloud_invite(state: S<'_>, vault_id: Uuid, email: String) -> Result<()> {
    let (_, from) = me(&state)?;
    let d = vault_drive(&state, vault_id).await?;
    astralis_cloud::invite(&d, &state.store, vault_id, &email, &from).await.map_err(sharing_err)
}

/// Takes someone out of a cloud vault (Drive access and, if they joined, their copy).
#[tauri::command]
pub async fn cloud_uninvite(state: S<'_>, vault_id: Uuid, email: String) -> Result<()> {
    let d = vault_drive(&state, vault_id).await?;
    let device = astralis_cloud::uninvite(&d, &state.store, vault_id, &email).await.map_err(sharing_err)?;
    if let (Some(device), Some(net)) = (device, state.net.get()) {
        net.shares_changed(&device);
    }
    Ok(())
}

/// Joins a vault someone shared: pick its folder in the Google Picker, then connect to the
/// owner's device with the invite. Returns the vault's name.
#[tauri::command]
pub async fn cloud_join(state: S<'_>) -> Result<String> {
    let net = state.net.get().cloned().ok_or_else(|| Error::Invalid("peer-to-peer is still starting; try again in a moment".into()))?;
    let (picked, email) = sign_in(state.inner(), true).await?;
    let folder = picked.first().ok_or_else(|| Error::Invalid("no folder was picked".into()))?;
    let d = drive(&state).await?;
    let ticket = astralis_cloud::join_ticket(&d, folder, &email).await.map_err(err)?;
    let request = astralis_net::JoinRequest { vault: ticket.vault, email: ticket.email.clone(), secret: ticket.secret.clone() };
    net.join_vault(&ticket.owner, request).await.map_err(err)?;
    Ok(ticket.name)
}

/// A vault someone shared on OneDrive (OneDrive lists them; Google uses its folder picker).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedVault {
    pub folder_id: String,
    pub name: String,
    pub owner_name: String,
}

/// The Astralis vaults other people shared with this OneDrive account.
#[tauri::command]
pub async fn cloud_shared_vaults(state: S<'_>) -> Result<Vec<SharedVault>> {
    let d = onedrive(&state).await?;
    let list = CloudDrive::shared_folders(&d).await.map_err(err)?;
    Ok(list
        .into_iter()
        .map(|f| SharedVault { folder_id: f.id, name: f.name, owner_name: f.props.get("ownerName").cloned().unwrap_or_default() })
        .collect())
}

/// Joins a vault shared on OneDrive: connects to the owner's device with the invite. Returns the
/// vault's name.
#[tauri::command]
pub async fn cloud_join_onedrive(state: S<'_>, folder_id: String) -> Result<String> {
    let net = state.net.get().cloned().ok_or_else(|| Error::Invalid("peer-to-peer is still starting; try again in a moment".into()))?;
    let d = onedrive(&state).await?;
    let email = CloudDrive::email(&d).await.map_err(err)?;
    let ticket = astralis_cloud::join_ticket(&d, &folder_id, &email).await.map_err(err)?;
    let request = astralis_net::JoinRequest { vault: ticket.vault, email: ticket.email.clone(), secret: ticket.secret.clone() };
    net.join_vault(&ticket.owner, request).await.map_err(err)?;
    Ok(ticket.name)
}

/// A cloud vault in the user's Drive, for "Add existing cloud vault".
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyCloudVault {
    pub folder_id: String,
    pub vault_id: Uuid,
    pub name: String,
    pub owner_name: String,
    /// Already here (so there is nothing to add).
    pub on_this_device: bool,
    /// Its key is on the Drive (the device that made it has been opened since).
    pub ready: bool,
}

/// The cloud vaults in this Google account's Drive (made by Astralis on any of its devices).
#[tauri::command]
pub async fn cloud_my_vaults(state: S<'_>, provider: Option<String>) -> Result<Vec<MyCloudVault>> {
    let d = any_drive(&state, provider_of(provider)).await?;
    let list = astralis_cloud::my_cloud_vaults(&d).await.map_err(err)?;
    let s = state.store.lock().unwrap();
    Ok(list
        .into_iter()
        .map(|v| MyCloudVault {
            on_this_device: s.get_vault(v.vault).is_ok(),
            folder_id: v.folder_id,
            vault_id: v.vault,
            name: v.name,
            owner_name: v.owner_name,
            ready: v.ready,
        })
        .collect())
}

/// Adds one of this account's cloud vaults to this device, from the Drive alone (no other device
/// has to be open): the key is read from the Astralis folder, the notes come with a sync.
#[tauri::command]
pub async fn cloud_add_mine(app: AppHandle, state: S<'_>, folder_id: String, provider: Option<String>) -> Result<String> {
    let d = any_drive(&state, provider_of(provider)).await?;
    let (vault, name) = astralis_cloud::add_my_cloud_vault(&d, &state.store, &folder_id).await.map_err(err)?;
    match astralis_cloud::sync_vault(&d, &state.store, vault).await {
        Ok(r) => {
            let _ = app.emit("p2p", serde_json::json!({ "type": "synced", "peer": "drive", "notes": r.notes }));
        }
        // The vault is added; the notes arrive with the next sync.
        Err(e) => tracing_warn(&format!("first sync of an added cloud vault failed: {e:#}")),
    }
    Ok(name)
}

/// Opens a web page in the user's browser (setup links). Only https.
#[tauri::command]
pub fn open_url(url: String) -> Result<()> {
    if !url.starts_with("https://") {
        return Err(Error::Invalid("only https links can be opened".into()));
    }
    open_browser(&url).map_err(err)
}

/// Syncs every cloud vault through the drive now.
#[tauri::command]
pub async fn cloud_sync_now(app: AppHandle, state: S<'_>) -> Result<()> {
    sync_all(&app, state.inner()).await
}

async fn sync_all(app: &AppHandle, state: &SharedState) -> Result<()> {
    let all = state.store.lock().unwrap().cloud_vaults()?;
    for (provider, refresh_key) in [(GOOGLE_DRIVE, REFRESH), (ONEDRIVE, MS_REFRESH)] {
        let vaults: Vec<_> = all.iter().filter(|v| v.provider == provider).cloned().collect();
        if vaults.is_empty() || setting(state, refresh_key).is_none() {
            continue;
        }
        match any_drive(state, provider).await {
            Ok(d) => sync_provider(app, state, &d, vaults).await,
            Err(e) => tracing_warn(&format!("{provider} sync: {e}")),
        }
    }
    if let Err(e) = crate::projects_cmds::sync(state, false).await {
        tracing_warn(&format!("project sync: {e}"));
    }
    Ok(())
}

async fn sync_provider(app: &AppHandle, state: &SharedState, d: &AnyDrive, vaults: Vec<astralis_core::cloud::CloudVault>) {
    for v in vaults {
        // So this account's other devices can add the vault from the Drive (once per vault).
        if v.owned {
            let _ = astralis_cloud::ensure_key_on_drive(d, &state.store, v.vault_id).await;
        }
        match astralis_cloud::sync_vault(d, &state.store, v.vault_id).await {
            Ok(r) if r.applied > 0 => {
                // Same event as P2P sync, so open views refresh the same way.
                let _ = app.emit("p2p", serde_json::json!({ "type": "synced", "peer": "drive", "notes": r.notes }));
            }
            Ok(_) => {}
            Err(e) => tracing_warn(&format!("drive sync for a vault failed: {e:#}")),
        }
    }
}

fn tracing_warn(msg: &str) {
    eprintln!("astralis: {msg}");
}

/// Background: sync cloud vaults through the drive every so often.
pub fn spawn_sync_loop(app: AppHandle, state: SharedState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(SYNC_EVERY).await;
            if let Err(e) = sync_all(&app, &state).await {
                tracing_warn(&format!("drive sync: {e}"));
            }
        }
    });
}
