//! Google Drive: sign-in, cloud vaults, email invites, and background sync through the drive.
//!
//! The OAuth client is the user's own (Settings → Connected drives). The refresh token lives in
//! this device's database; the access token only in memory.

use std::time::Duration;

use astralis_cloud::drive::GoogleDrive;
use astralis_cloud::oauth::{self, ClientConfig, Tokens};
use astralis_cloud::reqwest;
use astralis_core::cloud::{CloudInvite, GOOGLE_DRIVE};
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
/// How often cloud vaults sync through the drive (P2P handles live edits in between).
const SYNC_EVERY: Duration = Duration::from_secs(30);

/// Google sign-in state held in memory.
#[derive(Default)]
pub struct Cloud {
    http: reqwest::Client,
    tokens: tokio::sync::Mutex<Option<Tokens>>,
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

/// Opens a URL in the user's browser.
fn open_browser(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    let cmd = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
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

fn me(state: &SharedState) -> Result<(String, String)> {
    let net = state.net.get().ok_or_else(|| Error::Invalid("peer-to-peer is still starting; try again in a moment".into()))?;
    Ok((net.id(), net.name()))
}

/// A new vault in `Astralis/<name>/` on Google Drive.
#[tauri::command]
pub async fn cloud_create_vault(state: S<'_>, name: String) -> Result<Vault> {
    let (device, device_name) = me(&state)?;
    let d = drive(&state).await?;
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
    let d = drive(&state).await?;
    astralis_cloud::invite(&d, &state.store, vault_id, &email, &from).await.map_err(sharing_err)
}

/// Takes someone out of a cloud vault (Drive access and, if they joined, their copy).
#[tauri::command]
pub async fn cloud_uninvite(state: S<'_>, vault_id: Uuid, email: String) -> Result<()> {
    let d = drive(&state).await?;
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
    let vaults = state.store.lock().unwrap().cloud_vaults()?;
    let vaults: Vec<_> = vaults.into_iter().filter(|v| v.provider == GOOGLE_DRIVE).collect();
    if vaults.is_empty() || setting(state, REFRESH).is_none() {
        return Ok(());
    }
    let d = drive(state).await?;
    for v in vaults {
        match astralis_cloud::sync_vault(&d, &state.store, v.vault_id).await {
            Ok(r) if r.applied > 0 => {
                // Same event as P2P sync, so open views refresh the same way.
                let _ = app.emit("p2p", serde_json::json!({ "type": "synced", "peer": "drive", "notes": r.notes }));
            }
            Ok(_) => {}
            Err(e) => tracing_warn(&format!("drive sync for a vault failed: {e:#}")),
        }
    }
    Ok(())
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
