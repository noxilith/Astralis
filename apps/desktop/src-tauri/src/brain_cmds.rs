//! The Brain page: browse and curate what AI agents remember, tidy it, connect agents.

use std::time::Duration;

use astralis_core::memory::{BrainProject, BrainStats, Memory, TidyReport};
use astralis_core::{Error, Result};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::SharedState;

type S<'a> = State<'a, SharedState>;

/// Runs Brain work on a background thread: big Brains take a moment, and the window must keep
/// responding meanwhile (commands that wait on the store would otherwise freeze it).
async fn bg<T: Send + 'static>(state: &SharedState, f: impl FnOnce(&mut astralis_core::Store) -> Result<T> + Send + 'static) -> Result<T> {
    let st = state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut s = st.store.lock().unwrap();
        f(&mut s)
    })
    .await
    .map_err(|e| Error::Invalid(format!("background work failed: {e}")))?
}

#[tauri::command]
pub async fn brain_projects(state: S<'_>) -> Result<Vec<BrainProject>> {
    bg(&state, |s| s.brain_projects()).await
}

#[tauri::command]
pub async fn brain_memories(state: S<'_>, folder: Option<Uuid>) -> Result<Vec<Memory>> {
    bg(&state, move |s| s.brain_memories(folder)).await
}

#[tauri::command]
pub async fn brain_set_reviewed(state: S<'_>, id: Uuid, reviewed: bool) -> Result<Memory> {
    bg(&state, move |s| {
        let m = s.set_memory_reviewed(&id.to_string(), reviewed)?;
        let _ = s.sync_brain_mirror();
        Ok(m)
    })
    .await
}

#[tauri::command]
pub async fn brain_forget(state: S<'_>, id: Uuid) -> Result<()> {
    bg(&state, move |s| {
        s.forget(&id.to_string())?;
        let _ = s.sync_brain_mirror();
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn brain_tidy(state: S<'_>, folder: Option<Uuid>) -> Result<TidyReport> {
    bg(&state, move |s| s.brain_tidy(folder)).await
}

#[tauri::command]
pub async fn brain_merge(state: S<'_>, keep: Uuid, other: Uuid) -> Result<Memory> {
    bg(&state, move |s| {
        let m = s.brain_merge(&keep.to_string(), &other.to_string())?;
        let _ = s.sync_brain_mirror();
        Ok(m)
    })
    .await
}

#[tauri::command]
pub async fn brain_compact(state: S<'_>, ids: Vec<Uuid>, hook: String) -> Result<Memory> {
    bg(&state, move |s| {
        let handles: Vec<String> = ids.iter().map(Uuid::to_string).collect();
        let m = s.brain_compact(&handles, &hook)?;
        let _ = s.sync_brain_mirror();
        Ok(m)
    })
    .await
}

#[tauri::command]
pub async fn brain_graph(state: S<'_>) -> Result<astralis_core::memory::BrainGraph> {
    bg(&state, |s| s.brain_graph()).await
}

#[tauri::command]
pub async fn brain_stats(state: S<'_>) -> Result<BrainStats> {
    bg(&state, |s| s.brain_stats()).await
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
pub async fn brain_setup(state: S<'_>) -> Result<BrainSetup> {
    let exe = std::env::current_exe().map_err(|e| Error::Invalid(e.to_string()))?;
    let name = if cfg!(windows) { "astralis-mcp.exe" } else { "astralis-mcp" };
    let server = exe.parent().map(|d| d.join(name)).unwrap_or_else(|| name.into());
    let args = match std::env::var("ASTRALIS_PROFILE") {
        Ok(p) if !p.is_empty() => vec!["--profile".to_string(), p],
        _ => vec![],
    };
    let (mirror, vault) = bg(&state, |s| Ok((s.brain_mirror_dir()?.map(|p| p.to_string_lossy().to_string()), s.brain_vault()?))).await?;
    Ok(BrainSetup { server_exists: server.exists(), server: server.to_string_lossy().to_string(), args, mirror, vault })
}

/// Turns the Markdown mirror on (at `dir`, or the default) or off.
#[tauri::command]
pub async fn brain_set_mirror(state: S<'_>, enabled: bool, dir: Option<String>) -> Result<Option<String>> {
    bg(&state, move |s| {
        if enabled {
            let dir = dir.filter(|d| !d.trim().is_empty());
            s.set_brain_mirror_dir(Some(dir.as_deref().unwrap_or("")))?;
            s.sync_brain_mirror()?;
        } else {
            s.set_brain_mirror_dir(None)?;
        }
        Ok(s.brain_mirror_dir()?.map(|p| p.to_string_lossy().to_string()))
    })
    .await
}

/// Background: agents write to the Brain while the app is open — keep the mirror current and
/// tell the UI when memories change.
pub fn spawn_brain_watch(app: AppHandle, state: SharedState) {
    tauri::async_runtime::spawn(async move {
        let mut last: Option<(i64, i64)> = None;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let stamp = bg(&state, |s| s.brain_change_stamp()).await.ok();
            if stamp.is_some() && stamp != last {
                let first = last.is_none();
                last = stamp;
                let _ = bg(&state, |s| s.sync_brain_mirror()).await;
                if !first {
                    let _ = app.emit("brain-changed", ());
                }
            }
        }
    });
}

/// Notices notes changed by another process (AI agents through the Brain's MCP server) and tells
/// the UI like a sync would, so the note list refreshes and an open note merges the change.
/// (Peers get the changes on their own: P2P sync reads the database on every round.)
pub fn spawn_outside_edits_watch(app: AppHandle, state: SharedState) {
    tauri::async_runtime::spawn(async move {
        let mut seen: Option<i64> = None;
        let mut since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let Ok(version) = bg(&state, |s| s.data_version()).await else { continue };
            if seen.is_some_and(|v| v != version) {
                if let Ok((ids, newest)) = bg(&state, move |s| s.notes_changed_since(since)).await {
                    since = newest;
                    if !ids.is_empty() {
                        let ids: Vec<String> = ids.into_iter().map(|i| i.to_string()).collect();
                        let _ = app.emit("outside-edits", ids);
                    }
                }
            }
            seen = Some(version);
        }
    });
}

/// What AI agents may do with the user's notes: "edit", "read" or "off".
#[tauri::command]
pub async fn agents_notes_access(state: S<'_>) -> Result<String> {
    bg(&state, |s| Ok(s.agent_access()?.as_str().to_string())).await
}

#[tauri::command]
pub async fn agents_set_notes_access(state: S<'_>, access: String) -> Result<()> {
    let a = astralis_core::agent_notes::AgentAccess::parse(&access).ok_or_else(|| Error::Invalid("access is edit, read or off".into()))?;
    bg(&state, move |s| s.set_agent_access(a)).await
}

// ---- the astralis-brain skill (teaches agents to use the Brain well) ----

const SKILL: &str = include_str!("../../../../skills/astralis-brain/SKILL.md");

fn home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from)
}

/// Where each tool looks for user-level skills.
fn skill_path(target: &str) -> Result<std::path::PathBuf> {
    let base = match target {
        "claude" => std::env::var_os("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from).or_else(|| home().map(|h| h.join(".claude"))),
        "codex" => std::env::var_os("CODEX_HOME").map(std::path::PathBuf::from).or_else(|| home().map(|h| h.join(".codex"))),
        _ => None,
    };
    base.map(|b| b.join("skills").join("astralis-brain").join("SKILL.md"))
        .ok_or_else(|| Error::Invalid(format!("don't know where {target} keeps skills")))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillStatus {
    target: String,
    path: String,
    /// "missing", "installed" or "outdated".
    state: String,
}

#[tauri::command]
pub fn brain_skill_status() -> Result<Vec<SkillStatus>> {
    ["claude", "codex"]
        .iter()
        .map(|t| {
            let path = skill_path(t)?;
            let state = match std::fs::read_to_string(&path) {
                Ok(s) if s == SKILL => "installed",
                Ok(_) => "outdated",
                Err(_) => "missing",
            };
            Ok(SkillStatus { target: t.to_string(), path: path.to_string_lossy().to_string(), state: state.into() })
        })
        .collect()
}

/// Installs (or updates) the skill for Claude Code or Codex.
#[tauri::command]
pub fn brain_install_skill(target: String) -> Result<String> {
    let path = skill_path(&target)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, SKILL)?;
    Ok(path.to_string_lossy().to_string())
}

/// The skill's text (to copy).
#[tauri::command]
pub fn brain_skill_text() -> String {
    SKILL.to_string()
}
