//! `astralis-mcp`: the Astralis Brain over the Model Context Protocol (stdio).
//!
//! AI agents (Claude Code, Codex, Cursor…) launch this and get a handful of tools that return
//! small, plain-text answers — the point is to spend fewer tokens than re-reading chats and
//! files. It opens the Astralis database directly (works whether or not the app is open) and
//! works out the current project from the git repository it's started in.
//!
//!   astralis-mcp [--db <path>] [--profile <name>]

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use astralis_core::Store;
use astralis_core::agent_notes::{AgentAccess, NoteEdit};
use astralis_core::memory::{MapEntry, MemoryKind, file_mtime};
use serde_json::{Value, json};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const INSTRUCTIONS: &str = "Astralis Brain is this project's long-term memory (shared across sessions and AI tools). Use it to save tokens:
1. At the start of a task, call brain_context once: a short index of what's already known (one line per memory).
2. Before re-reading files, grepping, or asking the user something you may have learned before, try brain_recall (by meaning) or brain_map (which file does what). Read a memory in full with brain_read only if the trimmed lines aren't enough.
3. When you learn something durable, save it with brain_remember: one idea per memory, a one-line hook that says the point, details only if needed. Good: decisions and why, gotchas, build/test commands, user preferences (scope \"me\" if they apply to every project), where things live. Don't save secrets, or anything obvious from the code or git history.
4. When you work out what files do, record them with brain_map_note (path + one-line purpose). brain_map flags files changed since — re-read only those.
5. If a memory turns out wrong or outdated, brain_update or brain_forget it.
The user's own notes (their notes app) are there too, only when they ask for something about them: notes_search or notes_list to find a note, notes_read to read it, then notes_edit (prefer a small old_text/new_text replacement over rewriting the whole body), notes_create, notes_move, notes_delete, and the notes_folder_* tools. Every edit keeps the note's history, and deleting archives a note (the user can restore it) unless you delete an already-archived note with permanent.";

fn data_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    let base = if cfg!(target_os = "macos") {
        home.join("Library/Application Support")
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or(home)
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"))
    };
    base.join("app.astralis.desktop")
}

/// The project the agent is working in: the enclosing git repository (or the folder).
struct Project {
    root: PathBuf,
    key: String,
    name: String,
}

fn detect_project(start: &Path) -> Project {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut dir = start.as_path();
    let root = loop {
        if dir.join(".git").exists() {
            break dir.to_path_buf();
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break start.clone(),
        }
    };
    // A remote URL survives clones and moves; otherwise the path.
    let remote = std::fs::read_to_string(root.join(".git/config")).ok().and_then(|cfg| {
        let mut in_origin = false;
        for line in cfg.lines() {
            let l = line.trim();
            if l.starts_with('[') {
                in_origin = l == "[remote \"origin\"]";
            } else if in_origin && let Some(url) = l.strip_prefix("url =") {
                return Some(url.trim().trim_end_matches(".git").to_string());
            }
        }
        None
    });
    let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Project".into());
    Project { key: remote.unwrap_or_else(|| root.to_string_lossy().to_string()), name, root }
}

struct Server {
    store: Store,
    default_root: PathBuf,
    source: String,
}

fn text_result(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }] })
}

fn error_result(text: impl Into<String>) -> Value {
    json!({ "content": [{ "type": "text", "text": text.into() }], "isError": true })
}

fn tools() -> Value {
    let project = json!({ "type": "string", "description": "Path of the project (defaults to the current repository)." });
    let vault = json!({ "type": "string", "description": "Vault name (default: the user's default vault). See notes_vaults." });
    let note = json!({ "type": "string", "description": "The note's [handle] (from notes_search/notes_list) or exact title." });
    json!([
        {
            "name": "brain_context",
            "description": "Short index of everything remembered about this project (and the user's general preferences): one line per memory with a [handle]. Call once at the start of a task.",
            "inputSchema": { "type": "object", "properties": { "project": project } }
        },
        {
            "name": "brain_recall",
            "description": "Search memories by meaning; returns the best few, each trimmed to its most relevant lines. Try this before re-reading files or old chats.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string", "description": "What you want to know, in plain words." },
                "limit": { "type": "integer", "description": "Max memories (default 4).", "minimum": 1, "maximum": 12 },
                "project": project
            }, "required": ["query"] }
        },
        {
            "name": "brain_read",
            "description": "A memory in full, by its [handle].",
            "inputSchema": { "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }
        },
        {
            "name": "brain_remember",
            "description": "Save something worth knowing next time (one idea per memory). If a memory already says nearly the same, it's updated instead of duplicated.",
            "inputSchema": { "type": "object", "properties": {
                "type": { "type": "string", "enum": ["decision", "preference", "gotcha", "howto", "fact", "context"], "description": "decision: what was decided and why · preference: how the user likes things · gotcha: a trap that wastes time · howto: commands/steps · fact: about the project · context: current work in progress" },
                "hook": { "type": "string", "description": "One line that states the point (shown in the index)." },
                "details": { "type": "string", "description": "Optional details: reasons, commands, file paths. Keep it short." },
                "scope": { "type": "string", "enum": ["project", "me"], "description": "\"me\" for preferences that apply to every project (default: project)." },
                "project": project
            }, "required": ["type", "hook"] }
        },
        {
            "name": "brain_update",
            "description": "Correct a memory (by [handle]): new hook, details or type.",
            "inputSchema": { "type": "object", "properties": {
                "id": { "type": "string" },
                "hook": { "type": "string" },
                "details": { "type": "string" },
                "type": { "type": "string", "enum": ["decision", "preference", "gotcha", "howto", "fact", "context"] }
            }, "required": ["id"] }
        },
        {
            "name": "brain_forget",
            "description": "Remove a memory that's wrong or no longer true (by [handle]; it can be restored from the app's archive).",
            "inputSchema": { "type": "object", "properties": { "id": { "type": "string" } }, "required": ["id"] }
        },
        {
            "name": "brain_map",
            "description": "The project's code map: which file does what. With a query, only the matching files. Files changed or deleted since they were described are flagged — re-read only those.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string", "description": "What you're looking for (e.g. \"where are sync conflicts resolved\"). Omit for the whole map." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 100 },
                "project": project
            } }
        },
        {
            "name": "brain_map_note",
            "description": "Record what files do (path relative to the project root + a one-line purpose). An empty purpose removes the file from the map.",
            "inputSchema": { "type": "object", "properties": {
                "files": { "type": "array", "items": { "type": "object", "properties": {
                    "path": { "type": "string" },
                    "purpose": { "type": "string" }
                }, "required": ["path", "purpose"] } },
                "project": project
            }, "required": ["files"] }
        },
        {
            "name": "notes_vaults",
            "description": "The user's note vaults (workspaces) you can use, with note counts. Encrypted vaults are never available.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "notes_search",
            "description": "Search the user's notes by words; returns [handle] Title — vault/folder: snippet. Searches every vault unless one is named.",
            "inputSchema": { "type": "object", "properties": {
                "query": { "type": "string" },
                "vault": vault,
                "limit": { "type": "integer", "minimum": 1, "maximum": 30, "description": "Max results (default 8)." }
            }, "required": ["query"] }
        },
        {
            "name": "notes_list",
            "description": "List the notes in a vault ([handle] Title — folder), optionally only one folder or tag.",
            "inputSchema": { "type": "object", "properties": {
                "vault": vault,
                "folder": { "type": "string", "description": "Folder path like \"Work/Projects\" or a folder name." },
                "tag": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 300, "description": "Max notes (default 60)." }
            } }
        },
        {
            "name": "notes_read",
            "description": "A note in full (Markdown), by [handle] or exact title.",
            "inputSchema": { "type": "object", "properties": { "note": note }, "required": ["note"] }
        },
        {
            "name": "notes_create",
            "description": "Create a note (Markdown). The folder is created if it doesn't exist.",
            "inputSchema": { "type": "object", "properties": {
                "title": { "type": "string" },
                "body": { "type": "string", "description": "The note's text, in Markdown." },
                "vault": vault,
                "folder": { "type": "string", "description": "Folder path like \"Work/Projects\" (optional)." },
                "tags": { "type": "array", "items": { "type": "string" } }
            }, "required": ["title", "body"] }
        },
        {
            "name": "notes_edit",
            "description": "Change a note. For the text, use ONE of: old_text + new_text (replace an exact passage — preferred, cheapest), append (add to the end), or body (rewrite the whole text). title and tags can be changed alongside. The previous version stays in the note's history.",
            "inputSchema": { "type": "object", "properties": {
                "note": note,
                "old_text": { "type": "string", "description": "Exact text to replace (as notes_read shows it). Must appear once, unless replace_all." },
                "new_text": { "type": "string" },
                "replace_all": { "type": "boolean" },
                "append": { "type": "string" },
                "body": { "type": "string" },
                "title": { "type": "string" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "The note's full set of tags." }
            }, "required": ["note"] }
        },
        {
            "name": "notes_delete",
            "description": "Delete a note. It goes to the archive, where the user can restore it. To remove it for good, delete it again with permanent: true (only works on archived notes; only when the user asked for that).",
            "inputSchema": { "type": "object", "properties": {
                "note": note,
                "permanent": { "type": "boolean", "description": "Remove an already-archived note for good. It can't be restored." }
            }, "required": ["note"] }
        },
        {
            "name": "notes_move",
            "description": "Move a note OR a folder (with everything in it): into a folder, out to the top level (to_folder \"/\"), and/or to another vault (to_vault). Missing folders are created. Images go along.",
            "inputSchema": { "type": "object", "properties": {
                "note": { "type": "string", "description": "The note to move ([handle] or title)." },
                "folder": { "type": "string", "description": "Or the folder to move (path or name), in 'vault'." },
                "vault": vault,
                "to_folder": { "type": "string", "description": "Destination folder path; \"/\" for the top level." },
                "to_vault": { "type": "string", "description": "Destination vault, if it's another one." }
            } }
        },
        {
            "name": "notes_folders",
            "description": "A vault's folders as a tree, with note counts.",
            "inputSchema": { "type": "object", "properties": { "vault": vault } }
        },
        {
            "name": "notes_folder_create",
            "description": "Create a folder; a path like \"Work/Projects\" makes the missing folders along the way.",
            "inputSchema": { "type": "object", "properties": { "path": { "type": "string" }, "vault": vault }, "required": ["path"] }
        },
        {
            "name": "notes_folder_rename",
            "description": "Rename a folder (path or name).",
            "inputSchema": { "type": "object", "properties": { "folder": { "type": "string" }, "name": { "type": "string" }, "vault": vault }, "required": ["folder", "name"] }
        },
        {
            "name": "notes_folder_delete",
            "description": "Delete a folder. Nothing in it is lost: its notes and subfolders move up a level.",
            "inputSchema": { "type": "object", "properties": { "folder": { "type": "string" }, "vault": vault }, "required": ["folder"] }
        },
        {
            "name": "notes_restore",
            "description": "Bring an archived note back.",
            "inputSchema": { "type": "object", "properties": { "note": note }, "required": ["note"] }
        }
    ])
}

impl Server {
    fn project(&mut self, args: &Value) -> Result<(uuid::Uuid, PathBuf), String> {
        let start = args["project"].as_str().map(PathBuf::from).unwrap_or_else(|| self.default_root.clone());
        let p = detect_project(&start);
        let folder = self.store.brain_project(&p.key, &p.name).map_err(|e| e.to_string())?;
        Ok((folder, p.root))
    }

    fn after_write(&mut self) {
        if let Err(e) = self.store.sync_brain_mirror() {
            eprintln!("astralis-mcp: couldn't update the Markdown mirror: {e}");
        }
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<String, String> {
        let s = |v: &Value| v.as_str().map(str::to_owned);
        let err = |e: astralis_core::Error| e.to_string();
        match name {
            "brain_context" => {
                let (folder, root) = self.project(args)?;
                self.store.brain_context(folder, Some(&root)).map_err(err)
            }
            "brain_recall" => {
                let (folder, _) = self.project(args)?;
                let query = s(&args["query"]).ok_or("query is required")?;
                let limit = args["limit"].as_u64().unwrap_or(4) as usize;
                self.store.brain_recall(folder, &query, limit).map(|(t, _)| t).map_err(err)
            }
            "brain_read" => self.store.brain_read(&s(&args["id"]).ok_or("id is required")?).map_err(err),
            "brain_remember" => {
                let kind = s(&args["type"]).and_then(|k| MemoryKind::parse(&k)).ok_or("type must be decision, preference, gotcha, howto, fact or context")?;
                let hook = s(&args["hook"]).ok_or("hook is required")?;
                let details = s(&args["details"]).unwrap_or_default();
                let folder = if s(&args["scope"]).as_deref() == Some("me") { self.store.brain_me().map_err(err)? } else { self.project(args)?.0 };
                let (m, merged) = self.store.remember(folder, kind, &hook, &details, &self.source.clone()).map_err(err)?;
                self.after_write();
                Ok(if merged { format!("Updated [{}] (it already covered this).", m.short) } else { format!("Saved [{}].", m.short) })
            }
            "brain_update" => {
                let id = s(&args["id"]).ok_or("id is required")?;
                let kind = s(&args["type"]).and_then(|k| MemoryKind::parse(&k));
                let m = self
                    .store
                    .update_memory(&id, s(&args["hook"]).as_deref(), s(&args["details"]).as_deref(), kind, &self.source.clone())
                    .map_err(err)?;
                self.after_write();
                Ok(format!("Updated [{}].", m.short))
            }
            "brain_forget" => {
                let m = self.store.forget(&s(&args["id"]).ok_or("id is required")?).map_err(err)?;
                self.after_write();
                Ok(format!("Forgot [{}] {}.", m.short, m.hook))
            }
            "brain_map" => {
                let (folder, root) = self.project(args)?;
                let limit = args["limit"].as_u64().unwrap_or(12) as usize;
                self.store.brain_map(folder, s(&args["query"]).as_deref(), Some(&root), limit).map_err(err)
            }
            "brain_map_note" => {
                let (folder, root) = self.project(args)?;
                let files = args["files"].as_array().ok_or("files must be a list of {path, purpose}")?;
                let entries: Vec<MapEntry> = files
                    .iter()
                    .filter_map(|f| {
                        let path = f["path"].as_str()?.trim().trim_start_matches("./").to_string();
                        let abs = if Path::new(&path).is_absolute() { PathBuf::from(&path) } else { root.join(&path) };
                        let rel = abs.strip_prefix(&root).map(|p| p.to_string_lossy().to_string()).unwrap_or(path);
                        Some(MapEntry { mtime: file_mtime(&abs), path: rel, purpose: f["purpose"].as_str().unwrap_or("").to_string() })
                    })
                    .collect();
                let n = self.store.brain_map_note(folder, &entries, &self.source.clone()).map_err(err)?;
                self.after_write();
                Ok(format!("Code map updated ({n} files)."))
            }
            n if n.starts_with("notes_") => self.notes(n, args),
            other => Err(format!("unknown tool {other}")),
        }
    }

    /// The user's notes. Brain → Connect in the app decides what agents may do with them.
    fn notes(&mut self, name: &str, args: &Value) -> Result<String, String> {
        let s = |v: &Value| v.as_str().map(str::to_owned);
        let err = |e: astralis_core::Error| e.to_string();
        let access = self.store.agent_access().map_err(err)?;
        let writes = !matches!(name, "notes_vaults" | "notes_search" | "notes_list" | "notes_read" | "notes_folders");
        match access {
            AgentAccess::Off => return Err("The user has turned off AI access to their notes (Astralis → Brain → Connect).".into()),
            AgentAccess::Read if writes => return Err("The user lets AI agents read their notes but not change them (Astralis → Brain → Connect). Suggest the change to the user instead.".into()),
            _ => {}
        }
        let note = || s(&args["note"]).ok_or("note is required (a [handle] or title)".to_string());
        let tags = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect::<Vec<_>>());
        match name {
            "notes_vaults" => self.store.agent_vaults().map_err(err),
            "notes_search" => {
                let query = s(&args["query"]).ok_or("query is required")?;
                let limit = args["limit"].as_u64().unwrap_or(8).clamp(1, 30) as usize;
                self.store.agent_search(&query, s(&args["vault"]).as_deref(), limit).map_err(err)
            }
            "notes_list" => {
                let limit = args["limit"].as_u64().unwrap_or(60).clamp(1, 300) as usize;
                self.store.agent_list(s(&args["vault"]).as_deref(), s(&args["folder"]).as_deref(), s(&args["tag"]).as_deref(), limit).map_err(err)
            }
            "notes_read" => self.store.agent_read(&note()?).map_err(err),
            "notes_create" => {
                let title = s(&args["title"]).ok_or("title is required")?;
                let body = s(&args["body"]).unwrap_or_default();
                self.store
                    .agent_create(&title, &body, s(&args["vault"]).as_deref(), s(&args["folder"]).as_deref(), &tags(&args["tags"]).unwrap_or_default())
                    .map_err(err)
            }
            "notes_edit" => {
                let replace = match (s(&args["old_text"]), s(&args["new_text"])) {
                    (Some(old), Some(new)) => Some((old, new, args["replace_all"].as_bool().unwrap_or(false))),
                    (Some(_), None) => return Err("old_text needs new_text (use \"\" to delete the passage)".into()),
                    (None, Some(_)) => return Err("new_text needs old_text (or use append or body)".into()),
                    (None, None) => None,
                };
                let edit = NoteEdit { title: s(&args["title"]), body: s(&args["body"]), append: s(&args["append"]), replace, tags: tags(&args["tags"]) };
                self.store.agent_edit(&note()?, edit).map_err(err)
            }
            "notes_delete" | "notes_archive" => self.store.agent_delete(&note()?, args["permanent"].as_bool().unwrap_or(false)).map_err(err),
            "notes_move" => {
                let (to_folder, to_vault) = (s(&args["to_folder"]), s(&args["to_vault"]));
                match (s(&args["note"]), s(&args["folder"])) {
                    (Some(n), None) => self.store.agent_move_note(&n, to_folder.as_deref(), to_vault.as_deref()).map_err(err),
                    (None, Some(f)) => self.store.agent_move_folder(&f, s(&args["vault"]).as_deref(), to_folder.as_deref(), to_vault.as_deref()).map_err(err),
                    _ => Err("give either note or folder".into()),
                }
            }
            "notes_folders" => self.store.agent_folders(s(&args["vault"]).as_deref()).map_err(err),
            "notes_folder_create" => self.store.agent_folder_create(&s(&args["path"]).ok_or("path is required")?, s(&args["vault"]).as_deref()).map_err(err),
            "notes_folder_rename" => self
                .store
                .agent_folder_rename(&s(&args["folder"]).ok_or("folder is required")?, &s(&args["name"]).ok_or("name is required")?, s(&args["vault"]).as_deref())
                .map_err(err),
            "notes_folder_delete" => self.store.agent_folder_delete(&s(&args["folder"]).ok_or("folder is required")?, s(&args["vault"]).as_deref()).map_err(err),
            "notes_restore" => self.store.agent_restore(&note()?).map_err(err),
            other => Err(format!("unknown tool {other}")),
        }
    }

    fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned()?; // notifications get no reply
        let method = msg["method"].as_str().unwrap_or("");
        let params = &msg["params"];
        let result = match method {
            "initialize" => {
                // Remember which agent this is, to credit its memories.
                if let Some(n) = params["clientInfo"]["name"].as_str() {
                    let n = n.to_ascii_lowercase();
                    self.source = if n.contains("claude") {
                        "claude-code".into()
                    } else if n.contains("codex") {
                        "codex".into()
                    } else if n.contains("cursor") {
                        "cursor".into()
                    } else {
                        n.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(30).collect()
                    };
                }
                let version = params["protocolVersion"].as_str().unwrap_or("2025-06-18");
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "astralis-brain", "title": "Astralis Brain", "version": VERSION },
                    "instructions": INSTRUCTIONS
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                Ok(match self.call(name, &args) {
                    Ok(text) => text_result(text),
                    Err(e) => error_result(e),
                })
            }
            "resources/list" => Ok(json!({ "resources": [] })),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            _ => Err(json!({ "code": -32601, "message": format!("method not found: {method}") })),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e }),
        })
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut db: Option<PathBuf> = None;
    let mut profile = std::env::var("ASTRALIS_PROFILE").ok();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--db" => db = args.next().map(PathBuf::from),
            "--profile" => profile = args.next(),
            "--help" | "-h" => {
                println!("astralis-mcp {VERSION} — the Astralis Brain as an MCP server (stdio)\n\n  --db <path>       Astralis database (default: the app's)\n  --profile <name>  use an Astralis profile (like ASTRALIS_PROFILE)");
                return;
            }
            "--version" => {
                println!("{VERSION}");
                return;
            }
            _ => {}
        }
    }
    if let Some(p) = profile.as_deref().filter(|p| !p.is_empty()) {
        // Same profile → same mirror folder as that app window (see Store::brain_mirror_dir).
        // Safe: nothing else is running yet.
        unsafe { std::env::set_var("ASTRALIS_PROFILE", p) };
    }
    let db = db.unwrap_or_else(|| {
        let dir = data_dir();
        match profile.filter(|p| !p.is_empty()) {
            Some(p) => dir.join(format!("profile-{p}")).join("astralis.db"),
            None => dir.join("astralis.db"),
        }
    });
    let store = match Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("astralis-mcp: can't open {}: {e}", db.display());
            std::process::exit(1);
        }
    };
    // Agents start servers in the project folder (Claude Code also passes CLAUDE_PROJECT_DIR).
    let default_root = std::env::var_os("CLAUDE_PROJECT_DIR").map(PathBuf::from).or_else(|| std::env::current_dir().ok()).unwrap_or_default();
    let mut server = Server { store, default_root, source: "agent".into() };

    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let replies: Vec<Value> = batch.iter().filter_map(|m| server.handle(m)).collect();
                (!replies.is_empty()).then(|| Value::Array(replies))
            }
            Ok(msg) => server.handle(&msg),
            Err(e) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } })),
        };
        if let Some(r) = reply {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}
