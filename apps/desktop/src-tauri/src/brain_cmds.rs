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
    bg(&state, |s| s.brain_stats_report()).await
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
                    // Even with no note changed, tasks or calendar events may have (the lists reload).
                    let ids: Vec<String> = ids.into_iter().map(|i| i.to_string()).collect();
                    let _ = app.emit("outside-edits", ids);
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

/// What AI agents may do with the user's tasks, planners and calendar: "edit", "read" or "off".
#[tauri::command]
pub async fn agents_plan_access(state: S<'_>) -> Result<String> {
    bg(&state, |s| Ok(s.agent_plan_access()?.as_str().to_string())).await
}

#[tauri::command]
pub async fn agents_set_plan_access(state: S<'_>, access: String) -> Result<()> {
    let a = astralis_core::agent_notes::AgentAccess::parse(&access).ok_or_else(|| Error::Invalid("access is edit, read or off".into()))?;
    bg(&state, move |s| s.set_agent_plan_access(a)).await
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

/// Puts the Brain on Google Drive: its memories sync through the Drive, and your other devices
/// can hook up to it (pair them, then share the Brain vault with them).
#[tauri::command]
pub async fn brain_cloud_on(state: S<'_>) -> Result<()> {
    let vault = bg(&state, |s| s.brain_vault()).await?;
    crate::cloud::cloud_enable(&state, vault, astralis_core::memory::BRAIN_VAULT_NAME).await
}

/// Stops syncing the Brain through the Drive on this device (the memories stay here).
#[tauri::command]
pub async fn brain_cloud_off(state: S<'_>) -> Result<()> {
    let vault = bg(&state, |s| s.brain_vault()).await?;
    state.store.lock().unwrap().leave_cloud(vault)
}

/// Uses a vault shared from your other device as this device's Brain.
#[tauri::command]
pub async fn brain_use_vault(state: S<'_>, vault_id: Uuid) -> Result<()> {
    bg(&state, move |s| s.set_brain_vault(vault_id)).await
}

// ---- Claude Code hooks (so the Brain is used without the AI having to remember to ask) ----

/// Claude Code's user settings file (it honours \`CLAUDE_CONFIG_DIR\`).
fn claude_settings_path() -> Result<std::path::PathBuf> {
    let base = std::env::var_os("CLAUDE_CONFIG_DIR").map(std::path::PathBuf::from).or_else(|| home().map(|h| h.join(".claude")));
    base.map(|b| b.join("settings.json")).ok_or_else(|| Error::Invalid("can't tell where Claude Code keeps its settings".into()))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksStatus {
    /// The session-start and per-message hooks are in Claude Code's settings.
    installed: bool,
    /// The optional save reminder is too.
    stop: bool,
    path: String,
}

/// A command line for the hook, quoted so a path with spaces works in a shell (and on Windows).
fn hook_command(server: &str, args: &[String], event: &str) -> String {
    let q = |s: &str| if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._:\\@+-".contains(c)) { s.to_string() } else { format!("\"{}\"", s.replace('"', "\\\"")) };
    let mut parts = vec![q(server)];
    parts.extend(args.iter().map(|a| q(a)));
    parts.push("hook".into());
    parts.push(event.into());
    parts.join(" ")
}

/// Whether a hook entry is one of ours (it runs \`astralis-mcp … hook <something>\`).
fn is_ours(entry: &serde_json::Value) -> bool {
    entry["hooks"].as_array().is_some_and(|hs| hs.iter().any(|h| h["command"].as_str().is_some_and(|c| c.contains("astralis-mcp") && c.contains(" hook "))))
}

const HOOK_EVENTS: [(&str, &str); 3] = [("SessionStart", "session-start"), ("UserPromptSubmit", "prompt"), ("Stop", "stop")];

/// Puts our hooks into a settings document, replacing earlier copies of ours and leaving
/// everything else as it was. \`stop\`: also the save reminder (otherwise it's removed).
fn merge_hooks(root: &mut serde_json::Value, server: &str, args: &[String], stop: bool) -> Result<()> {
    use serde_json::json;
    let obj = root.as_object_mut().ok_or_else(|| Error::Invalid("Claude Code's settings.json isn't a JSON object; fix it first".into()))?;
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().ok_or_else(|| Error::Invalid("the \"hooks\" part of Claude Code's settings.json isn't an object; fix it first".into()))?;
    for (event, arg) in HOOK_EVENTS {
        let list = hooks.entry(event).or_insert_with(|| json!([]));
        let list = list.as_array_mut().ok_or_else(|| Error::Invalid(format!("\"hooks.{event}\" in Claude Code's settings.json isn't a list; fix it first")))?;
        list.retain(|e| !is_ours(e));
        if event != "Stop" || stop {
            list.push(json!({ "hooks": [{ "type": "command", "command": hook_command(server, args, arg), "timeout": 10 }] }));
        }
        if list.is_empty() {
            hooks.remove(event);
        }
    }
    if hooks.is_empty() {
        obj.remove("hooks");
    }
    Ok(())
}

fn hooks_status_of(root: &serde_json::Value, path: &std::path::Path) -> HooksStatus {
    let has = |event: &str| root["hooks"][event].as_array().is_some_and(|l| l.iter().any(is_ours));
    HooksStatus { installed: has("SessionStart") && has("UserPromptSubmit"), stop: has("Stop"), path: path.to_string_lossy().to_string() }
}

fn read_settings(path: &std::path::Path) -> Result<serde_json::Value> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(serde_json::json!({})),
        Ok(text) => serde_json::from_str(&text).map_err(|e| Error::Invalid(format!("Claude Code's settings.json isn't valid JSON ({e}); fix it first, then try again"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(serde_json::json!({})),
        Err(e) => Err(Error::Invalid(format!("can't read {}: {e}", path.display()))),
    }
}

/// Writes the settings back safely: the first time, the original is kept beside it as
/// \`settings.json.astralis-backup\`, and the new file replaces the old in one step.
fn write_settings(path: &std::path::Path, root: &serde_json::Value) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if path.exists() {
        let backup = path.with_extension("json.astralis-backup");
        if !backup.exists() {
            std::fs::copy(path, &backup)?;
        }
    }
    let tmp = path.with_extension("json.astralis-tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(root).map_err(|e| Error::Invalid(e.to_string()))? + "\n")?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn mcp_server() -> Result<(String, Vec<String>)> {
    let exe = std::env::current_exe().map_err(|e| Error::Invalid(e.to_string()))?;
    let name = if cfg!(windows) { "astralis-mcp.exe" } else { "astralis-mcp" };
    let server = exe.parent().map(|d| d.join(name)).unwrap_or_else(|| name.into());
    let args = match std::env::var("ASTRALIS_PROFILE") {
        Ok(p) if !p.is_empty() => vec!["--profile".to_string(), p],
        _ => vec![],
    };
    Ok((server.to_string_lossy().to_string(), args))
}

#[tauri::command]
pub fn brain_hooks_status() -> Result<HooksStatus> {
    let path = claude_settings_path()?;
    let root = read_settings(&path).unwrap_or_else(|_| serde_json::json!({}));
    Ok(hooks_status_of(&root, &path))
}

/// Adds the Brain's hooks to Claude Code's settings (on the user's click in Brain → Connect).
#[tauri::command]
pub fn brain_install_hooks(stop: bool) -> Result<HooksStatus> {
    let path = claude_settings_path()?;
    let mut root = read_settings(&path)?;
    let (server, args) = mcp_server()?;
    merge_hooks(&mut root, &server, &args, stop)?;
    write_settings(&path, &root)?;
    Ok(hooks_status_of(&root, &path))
}

/// Takes the Brain's hooks out again, leaving every other setting alone.
#[tauri::command]
pub fn brain_remove_hooks() -> Result<HooksStatus> {
    let path = claude_settings_path()?;
    let mut root = read_settings(&path)?;
    if root.get("hooks").is_some() {
        // Merging with "stop off" re-adds the other two, so remove ours directly instead.
        if let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) {
            for (event, _) in HOOK_EVENTS {
                if let Some(list) = hooks.get_mut(event).and_then(|l| l.as_array_mut()) {
                    list.retain(|e| !is_ours(e));
                }
                if hooks.get(event).and_then(|l| l.as_array()).is_some_and(|l| l.is_empty()) {
                    hooks.remove(event);
                }
            }
        }
        if root["hooks"].as_object().is_some_and(|h| h.is_empty()) {
            root.as_object_mut().map(|o| o.remove("hooks"));
        }
        write_settings(&path, &root)?;
    }
    Ok(hooks_status_of(&root, &path))
}

#[cfg(test)]
mod hook_install_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn our_hooks_merge_in_without_touching_anything_else_and_can_be_replaced() {
        let mut root = json!({
            "model": "opus",
            "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "echo hi" }] }],
                       "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "/usr/local/bin/other-tool" }] }] }
        });
        merge_hooks(&mut root, "/opt/Astralis App/astralis-mcp", &[], false).unwrap();
        assert_eq!(root["model"], "opus");
        assert_eq!(root["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
        let submit = root["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(submit.len(), 2, "someone else's hook stays: {root}");
        assert!(submit.iter().any(|e| e["hooks"][0]["command"] == "\"/opt/Astralis App/astralis-mcp\" hook prompt"));
        assert!(root["hooks"]["Stop"].is_null());
        assert!(hooks_status_of(&root, std::path::Path::new("x")).installed);
        // Again, now with the save reminder: ours are replaced, not duplicated.
        merge_hooks(&mut root, "/opt/Astralis App/astralis-mcp", &[], true).unwrap();
        assert_eq!(root["hooks"]["UserPromptSubmit"].as_array().unwrap().len(), 2);
        assert_eq!(root["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
        assert!(hooks_status_of(&root, std::path::Path::new("x")).stop);
        // Odd files are refused, not overwritten.
        assert!(merge_hooks(&mut json!([1]), "s", &[], false).is_err());
        assert!(merge_hooks(&mut json!({ "hooks": "nope" }), "s", &[], false).is_err());
        assert!(merge_hooks(&mut json!({ "hooks": { "Stop": 3 } }), "s", &[], false).is_err());
        // A profile is passed along.
        let cmd = hook_command("/x/astralis-mcp", &["--profile".into(), "work".into()], "prompt");
        assert_eq!(cmd, "/x/astralis-mcp --profile work hook prompt");
    }
}
