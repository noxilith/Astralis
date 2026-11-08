//! The Brain page: browse and curate what AI agents remember, tidy it, connect agents.

use std::time::Duration;

use astralis_core::memory::{BrainProject, BrainStats, Memory, TidyReport};
use astralis_core::{Error, Result};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::SharedState;

type S<'a> = State<'a, SharedState>;

#[tauri::command]
pub fn brain_projects(state: S<'_>) -> Result<Vec<BrainProject>> {
    state.store.lock().unwrap().brain_projects()
}

#[tauri::command]
pub fn brain_memories(state: S<'_>, folder: Option<Uuid>) -> Result<Vec<Memory>> {
    state.store.lock().unwrap().brain_memories(folder)
}

#[tauri::command]
pub fn brain_set_reviewed(state: S<'_>, id: Uuid, reviewed: bool) -> Result<Memory> {
    let mut s = state.store.lock().unwrap();
    let m = s.set_memory_reviewed(&id.to_string(), reviewed)?;
    let _ = s.sync_brain_mirror();
    Ok(m)
}

#[tauri::command]
pub fn brain_forget(state: S<'_>, id: Uuid) -> Result<()> {
    let mut s = state.store.lock().unwrap();
    s.forget(&id.to_string())?;
    let _ = s.sync_brain_mirror();
    Ok(())
}

#[tauri::command]
pub fn brain_tidy(state: S<'_>, folder: Option<Uuid>) -> Result<TidyReport> {
    state.store.lock().unwrap().brain_tidy(folder)
}

#[tauri::command]
pub fn brain_merge(state: S<'_>, keep: Uuid, other: Uuid) -> Result<Memory> {
    let mut s = state.store.lock().unwrap();
    let m = s.brain_merge(&keep.to_string(), &other.to_string())?;
    let _ = s.sync_brain_mirror();
    Ok(m)
}

#[tauri::command]
pub fn brain_compact(state: S<'_>, ids: Vec<Uuid>, hook: String) -> Result<Memory> {
    let mut s = state.store.lock().unwrap();
    let handles: Vec<String> = ids.iter().map(Uuid::to_string).collect();
    let m = s.brain_compact(&handles, &hook)?;
    let _ = s.sync_brain_mirror();
    Ok(m)
}

#[tauri::command]
pub fn brain_stats(state: S<'_>) -> Result<BrainStats> {
    state.store.lock().unwrap().brain_stats()
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainSetup {
    /// The MCP server program agents launch.
    server: String,
    server_exists: bool,
    /// Extra arguments (e.g. the profile this window uses).
    args: Vec<String>,
    /// Where the Brain is mirrored as Markdown (None = off).
    mirror: Option<String>,
    /// The Brain vault (to open it).
    vault: Uuid,
}

#[tauri::command]
pub fn brain_setup(state: S<'_>) -> Result<BrainSetup> {
    let exe = std::env::current_exe().map_err(|e| Error::Invalid(e.to_string()))?;
    let name = if cfg!(windows) { "astralis-mcp.exe" } else { "astralis-mcp" };
    let server = exe.parent().map(|d| d.join(name)).unwrap_or_else(|| name.into());
    let args = match std::env::var("ASTRALIS_PROFILE") {
        Ok(p) if !p.is_empty() => vec!["--profile".to_string(), p],
        _ => vec![],
    };
    let mut s = state.store.lock().unwrap();
    Ok(BrainSetup {
        server_exists: server.exists(),
        server: server.to_string_lossy().to_string(),
        args,
        mirror: s.brain_mirror_dir()?.map(|p| p.to_string_lossy().to_string()),
        vault: s.brain_vault()?,
    })
}

/// Turns the Markdown mirror on (at `dir`, or the default) or off.
#[tauri::command]
pub fn brain_set_mirror(state: S<'_>, enabled: bool, dir: Option<String>) -> Result<Option<String>> {
    let mut s = state.store.lock().unwrap();
    if enabled {
        let dir = dir.filter(|d| !d.trim().is_empty());
        s.set_brain_mirror_dir(Some(dir.as_deref().unwrap_or("")))?;
        s.sync_brain_mirror()?;
    } else {
        s.set_brain_mirror_dir(None)?;
    }
    Ok(s.brain_mirror_dir()?.map(|p| p.to_string_lossy().to_string()))
}

/// Background: agents write to the Brain while the app is open — keep the mirror current and
/// tell the UI when memories change.
pub fn spawn_brain_watch(app: AppHandle, state: SharedState) {
    tauri::async_runtime::spawn(async move {
        let mut last: Option<(i64, i64)> = None;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let stamp = {
                let s = state.store.lock().unwrap();
                s.brain_change_stamp().ok()
            };
            if stamp.is_some() && stamp != last {
                let first = last.is_none();
                last = stamp;
                let _ = state.store.lock().unwrap().sync_brain_mirror();
                if !first {
                    let _ = app.emit("brain-changed", ());
                }
            }
        }
    });
}
