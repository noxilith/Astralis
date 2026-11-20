//! Projects: a project's code (a git snapshot, with the uncommitted work) kept in the user's cloud
//! drive, so another computer can restore it or catch up. The AI tools' hand-off hook takes the
//! snapshots (see `astralis-mcp`); this uploads them and lists what's in the cloud. Nothing is
//! downloaded until the user asks: restore a project into a folder on a new computer, or update
//! one that another device has moved ahead.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use astralis_cloud::projects as cloud;
use astralis_core::projects::{self, Entry};
use astralis_core::{Error, Result};
use serde::Serialize;
use tauri::State;

use crate::SharedState;

type S<'a> = State<'a, SharedState>;

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// The app's data folder, where snapshots wait (`outbox`, `inbox`).
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

fn dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_default()
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("{e:#}"))
}

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

/// A path as typed: `~/x` means the home folder.
fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if path == "~" => home(),
        None => PathBuf::from(path),
    }
}

/// Uploads waiting snapshots, lists the cloud, and fetches newer copies. `force`: even when no
/// project is set to sync (to list what's in the cloud on a new computer). Returns whether it ran.
pub async fn sync(state: &SharedState, force: bool) -> Result<bool> {
    let wanted = force || {
        let s = state.store.lock().unwrap();
        s.projects_auto() || s.projects().map(|m| m.values().any(|e| e.sync)).unwrap_or(false)
    };
    if !wanted {
        return Ok(false);
    }
    let Some(d) = crate::cloud::projects_drive(state).await? else { return Ok(false) };
    let dir = dir();
    cloud::push(&d, &state.store, &dir).await.map_err(err)?;
    cloud::refresh(&d, &state.store).await.map_err(err)?;
    Ok(true)
}

/// Takes a snapshot of a project on this device now (the work in progress included).
fn snapshot_now(state: &SharedState, hash: &str) -> Result<()> {
    let (entry, device) = {
        let s = state.store.lock().unwrap();
        (s.projects()?.remove(hash), s.device_id())
    };
    let Some(entry) = entry.filter(|e| e.sync && !e.root.is_empty()) else { return Ok(()) };
    let id = projects::identify(Path::new(&entry.root));
    let outbox = dir().join("outbox");
    if let Some(snap) = projects::snapshot(&id.root, &entry.key, &entry.name, &device, &projects::device_label(), &outbox, &entry.state)? {
        std::fs::write(outbox.join(format!("{hash}.json")), serde_json::to_vec(&snap.manifest)?)?;
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCopy {
    ts: u64,
    device_name: String,
    branch: String,
    bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRow {
    hash: String,
    name: String,
    /// Where it is on this device ("" if nowhere).
    root: String,
    /// The folder is on this device.
    here: bool,
    sync: bool,
    /// synced · waiting · behind · off · restore · missing
    status: String,
    cloud: Option<CloudCopy>,
    /// A folder to offer for restoring it.
    suggest: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsInfo {
    /// The drive snapshots go to ("Google Drive" or "OneDrive"), if one is connected.
    drive: Option<String>,
    /// Keep every project the AI tools work in, from now on.
    auto: bool,
    rows: Vec<ProjectRow>,
}

#[tauri::command]
pub fn projects_list(state: S<'_>) -> Result<ProjectsInfo> {
    let (entries, mut remote, auto, device) = {
        let s = state.store.lock().unwrap();
        (s.projects()?, s.projects_remote()?, s.projects_auto(), s.device_id())
    };
    let pending = |hash: &str| dir().join("outbox").join(format!("{hash}.json")).exists();
    let suggest = |name: &str| {
        let clean: String = name.chars().map(|c| if c.is_alphanumeric() || "-_. ".contains(c) { c } else { '-' }).collect();
        home().join("Projects").join(clean).to_string_lossy().to_string()
    };
    let copy = |r: &astralis_core::projects::RemoteInfo| CloudCopy { ts: r.manifest.ts, device_name: r.manifest.device_name.clone(), branch: r.manifest.branch.clone(), bytes: r.manifest.bytes };
    let mut rows = Vec::new();
    for (hash, e) in entries {
        let r = remote.remove(&hash);
        let here = !e.root.is_empty() && Path::new(&e.root).join(".git").exists();
        let behind = r.as_ref().is_some_and(|r| r.manifest.ts > e.last_ts && r.manifest.device != device);
        let status = if !here {
            if r.is_some() { "restore" } else { "missing" }
        } else if !e.sync {
            "off"
        } else if behind {
            "behind"
        } else if pending(&hash) || r.is_none() {
            "waiting"
        } else {
            "synced"
        };
        rows.push(ProjectRow { suggest: suggest(&e.name), name: e.name, root: e.root, here, sync: e.sync, status: status.into(), cloud: r.as_ref().map(copy), hash });
    }
    // Projects that are only in the cloud (made on another computer).
    for (hash, r) in remote {
        rows.push(ProjectRow { suggest: suggest(&r.manifest.name), name: r.manifest.name.clone(), root: String::new(), here: false, sync: false, status: "restore".into(), cloud: Some(copy(&r)), hash });
    }
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(ProjectsInfo { drive: crate::cloud::projects_provider(&state).map(str::to_owned), auto, rows })
}

/// Takes snapshots of every synced project here, then uploads and looks in the cloud.
#[tauri::command]
pub async fn projects_sync_now(state: S<'_>) -> Result<()> {
    let state = state.inner().clone();
    let hashes: Vec<String> = state.store.lock().unwrap().projects()?.into_iter().filter(|(_, e)| e.sync).map(|(h, _)| h).collect();
    let s2 = state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        for h in hashes {
            // One project's trouble (a folder that's gone, no commits) doesn't stop the rest.
            let _ = snapshot_now(&s2, &h);
        }
    })
    .await
    .map_err(err)?;
    if !sync(&state, true).await? {
        return Err(Error::Invalid("connect Google Drive or OneDrive in Settings → Connected drives first".into()));
    }
    Ok(())
}

#[tauri::command]
pub fn projects_set_auto(state: S<'_>, on: bool) -> Result<()> {
    state.store.lock().unwrap().set_projects_auto(on)
}

#[tauri::command]
pub async fn projects_set_sync(state: S<'_>, hash: String, on: bool) -> Result<()> {
    state.store.lock().unwrap().project_set_sync(&hash, on)?;
    if on {
        let s = state.inner().clone();
        let _ = tauri::async_runtime::spawn_blocking(move || snapshot_now(&s, &hash)).await.map_err(err)?;
    }
    Ok(())
}

/// Adds a project folder by hand (it must be a git repository).
#[tauri::command]
pub async fn projects_add(state: S<'_>, path: String) -> Result<()> {
    let id = projects::identify(&expand(&path));
    if !id.root.join(".git").exists() {
        return Err(Error::Invalid("that folder isn't a git project (there's no .git in it or above it)".into()));
    }
    let hash = {
        let s = state.store.lock().unwrap();
        let hash = s.project_seen(&id.key, &id.name, &id.root)?;
        s.project_set_sync(&hash, true)?;
        hash
    };
    let s = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || snapshot_now(&s, &hash)).await.map_err(err)?
}

/// Stops keeping a project (here only; the cloud copy stays until it's replaced).
#[tauri::command]
pub fn projects_forget(state: S<'_>, hash: String) -> Result<()> {
    state.store.lock().unwrap().project_forget(&hash)
}

/// Brings a project that is behind up to the newest cloud copy. Changes nothing (and says why) if
/// there is unsaved work here, another branch is checked out, or this device has commits the cloud
/// copy lacks.
#[tauri::command]
pub async fn projects_update(state: S<'_>, hash: String) -> Result<()> {
    let state = state.inner().clone();
    let Some(d) = crate::cloud::projects_drive(&state).await? else {
        return Err(Error::Invalid("connect Google Drive or OneDrive in Settings → Connected drives first".into()));
    };
    let entry = state.store.lock().unwrap().projects()?.remove(&hash).filter(|e| !e.root.is_empty()).ok_or_else(|| Error::Invalid("that project isn't on this device".into()))?;
    let bundle = dir().join("restore").join(format!("{hash}.bundle"));
    let manifest = cloud::download(&d, &state.store, &hash, &bundle).await.map_err(err)?;
    let (b, m, root) = (bundle.clone(), manifest.clone(), PathBuf::from(&entry.root));
    let outcome = tauri::async_runtime::spawn_blocking(move || projects::pull(&root, &b, &m)).await.map_err(err)?;
    let _ = std::fs::remove_file(&bundle);
    match outcome? {
        projects::PullOutcome::Updated | projects::PullOutcome::AlreadyThere => {
            state.store.lock().unwrap().project_mark(&hash, Some(manifest.ts), None)?;
            Ok(())
        }
        projects::PullOutcome::Blocked(why) => Err(Error::Invalid(why)),
    }
}

/// Sets a project up in `dest` from its newest cloud snapshot (the folder must be new or empty).
#[tauri::command]
pub async fn projects_restore(state: S<'_>, hash: String, dest: String) -> Result<String> {
    let state = state.inner().clone();
    let Some(d) = crate::cloud::projects_drive(&state).await? else {
        return Err(Error::Invalid("connect Google Drive or OneDrive in Settings → Connected drives first".into()));
    };
    let dest_path = expand(&dest);
    if dest.trim().is_empty() {
        return Err(Error::Invalid("choose a folder for the project".into()));
    }
    let bundle = dir().join("restore").join(format!("{hash}.bundle"));
    let manifest = cloud::download(&d, &state.store, &hash, &bundle).await.map_err(err)?;
    let (b, m, p) = (bundle.clone(), manifest.clone(), dest_path.clone());
    let done = tauri::async_runtime::spawn_blocking(move || projects::restore(&p, &b, &m)).await.map_err(err)?;
    let _ = std::fs::remove_file(&bundle);
    done?;
    state.store.lock().unwrap().project_put(Entry { key: manifest.key, name: manifest.name, root: dest_path.to_string_lossy().to_string(), sync: true, last_ts: manifest.ts, state: manifest.state })?;
    Ok(dest_path.to_string_lossy().to_string())
}
