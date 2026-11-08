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
use astralis_core::memory::{MapEntry, MemoryKind, file_mtime};
use serde_json::{Value, json};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const INSTRUCTIONS: &str = "Astralis Brain is this project's long-term memory (shared across sessions and AI tools). Use it to save tokens:
1. At the start of a task, call brain_context once: a short index of what's already known (one line per memory).
2. Before re-reading files, grepping, or asking the user something you may have learned before, try brain_recall (by meaning) or brain_map (which file does what). Read a memory in full with brain_read only if the trimmed lines aren't enough.
3. When you learn something durable, save it with brain_remember: one idea per memory, a one-line hook that says the point, details only if needed. Good: decisions and why, gotchas, build/test commands, user preferences (scope \"me\" if they apply to every project), where things live. Don't save secrets, or anything obvious from the code or git history.
4. When you work out what files do, record them with brain_map_note (path + one-line purpose). brain_map flags files changed since — re-read only those.
5. If a memory turns out wrong or outdated, brain_update or brain_forget it.";

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
