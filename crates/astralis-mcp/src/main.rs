//! `astralis-mcp`: the Astralis Brain over the Model Context Protocol (stdio).
//!
//! AI agents (Claude Code, Codex, Cursor…) launch this and get a handful of tools that return
//! small, plain-text answers — the point is to spend fewer tokens than re-reading chats and
//! files. It opens the Astralis database directly (works whether or not the app is open) and
//! works out the current project from the git repository it's started in.
//!
//!   astralis-mcp [--db <path>] [--profile <name>]

#![recursion_limit = "512"]

mod code;

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use astralis_core::Store;
use astralis_core::agent_notes::{AgentAccess, NoteEdit};
use astralis_core::agent_plan::{AgentEvent, AgentEventEdit, AgentTask, AgentTaskEdit};
use astralis_core::memory::{MapEntry, MemoryKind, file_mtime};
use serde_json::{Value, json};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const INSTRUCTIONS: &str = "Astralis Brain is this project's long-term memory (shared across sessions and AI tools). Use it to save tokens:
0. The Brain replaces digging through old chats. Never search or read earlier chat logs, session transcripts or history files (for example ~/.claude/projects) to find what was decided, learned or done before: ask the Brain. If it isn't there, ask the user or read the code, then save the answer so nobody has to look again.
1. At the start of a task, call brain_context once, with the task in a sentence: a short index of what's already known (one line per memory).
2. Before re-reading files, grepping, or asking the user something you may have learned before, try brain_recall (by meaning) or brain_map (which file does what). Read a memory in full with brain_read only if the trimmed lines aren't enough.
3. When you learn something durable, save it with brain_remember: one idea per memory, a one-line hook that says the point, details only if needed. Good: decisions and why, gotchas, build/test commands, user preferences (scope \"me\" if they apply to every project), where things live. Don't save secrets, or anything obvious from the code or git history.
4. When you work out what files do, record them with brain_map_note (path + one-line purpose). brain_map flags files changed since — re-read only those.
5. If a memory turns out wrong or outdated, brain_update or brain_forget it.
6. Before you stop or at a natural pause, call brain_handoff: where things stand and what's next. It leads the next session's index, so nobody has to read this chat to continue.
7. To see what's in a file, call code_outline (definitions with line numbers) and read only the lines you need; to find where something is defined, call code_find before grepping.
The user's own notes (their notes app) are there too, only when they ask for something about them: notes_search or notes_list to find a note, notes_read to read it, then notes_edit (prefer a small old_text/new_text replacement over rewriting the whole body), notes_create, notes_move, notes_delete, and the notes_folder_* tools. Every edit keeps the note's history, and deleting archives a note (the user can restore it) unless you delete an already-archived note with permanent.
The user's tasks and calendar work the same way, when they ask: plan_overview first (it also gives today's date and time), then planner_create to plan a project with its tasks, tasks_add / tasks_edit / tasks_list for to-dos, and calendar_add / calendar_edit / calendar_list for appointments and work sessions. Times are the user's local time (YYYY-MM-DD or YYYY-MM-DD HH:MM). Mark tasks done rather than deleting them.";

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
            "description": "Short index of what's remembered about this project (and the user's general preferences): one line per memory with a [handle]. Call once at the start of a task, with the task in plain words so the relevant memories come first.",
            "inputSchema": { "type": "object", "properties": { "project": project, "task": { "type": "string", "description": "What you're about to work on, in a sentence." } } }
        },
        {
            "name": "brain_recall",
            "description": "Search memories by meaning; returns the best few, each trimmed to its most relevant lines. Use this INSTEAD of searching old chats, transcripts or logs for what was decided or learned before, and before re-reading files.",
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
            "name": "brain_handoff",
            "description": "Save the hand-off note for this project: where things stand and what comes next, in a few lines (what's done, what's half-done, the next step, what to watch out for). It replaces the previous note and is shown first in brain_context, so the next session continues without reading this chat. Call it before you stop or at a natural pause.",
            "inputSchema": { "type": "object", "properties": {
                "text": { "type": "string", "description": "A few lines, under about 1,400 characters." },
                "project": project
            }, "required": ["text"] }
        },
        {
            "name": "code_outline",
            "description": "The definitions in one file (functions, types, classes, headings…) with line numbers and signatures: a few hundred tokens instead of the whole file. Read the outline first, then read only the lines you need.",
            "inputSchema": { "type": "object", "properties": {
                "path": { "type": "string", "description": "File path, relative to the project root." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 400, "description": "Most definitions to list (default 120)." },
                "project": project
            }, "required": ["path"] }
        },
        {
            "name": "code_find",
            "description": "Where a function, type, class or constant is defined, anywhere in the project: path:line and its signature. Use it before grepping.",
            "inputSchema": { "type": "object", "properties": {
                "name": { "type": "string", "description": "The identifier, e.g. brain_recall or SyncScope." },
                "limit": { "type": "integer", "minimum": 1, "maximum": 60, "description": "Most results (default 20)." },
                "project": project
            }, "required": ["name"] }
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
        },
        {
            "name": "plan_overview",
            "description": "The user's planners (projects) with progress, and the current local date and time. Start here for anything about tasks, to-dos or the calendar.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "planner_create",
            "description": "Make a planner (e.g. for building an app) with its tasks in one call: break the work into concrete steps, with priorities and due dates when they know them.",
            "inputSchema": { "type": "object", "properties": {
                "name": { "type": "string" }, "description": { "type": "string" },
                "tasks": { "type": "array", "items": { "type": "object", "properties": { "title": { "type": "string" }, "notes": { "type": "string", "description": "Details." }, "status": { "type": "string", "enum": ["todo","doing","done"] }, "priority": { "type": "string", "enum": ["none","low","medium","high","urgent"] }, "due": { "type": "string", "description": "YYYY-MM-DD (all day) or YYYY-MM-DD HH:MM, the user's local time." }, "remind": { "type": "string", "description": "When to remind: YYYY-MM-DD HH:MM." }, "repeat": { "type": "string", "description": "daily, weekly, monthly, yearly, \"2 weeks\"… (needs a due date)." } }, "required": ["title"] } }
            }, "required": ["name"] }
        },
        {
            "name": "planner_update",
            "description": "Rename a planner or change its description.",
            "inputSchema": { "type": "object", "properties": { "planner": { "type": "string" }, "name": { "type": "string" }, "description": { "type": "string" } }, "required": ["planner"] }
        },
        {
            "name": "planner_delete",
            "description": "Delete a planner. Its tasks are kept (unassigned) unless with_tasks is true.",
            "inputSchema": { "type": "object", "properties": { "planner": { "type": "string" }, "with_tasks": { "type": "boolean" } }, "required": ["planner"] }
        },
        {
            "name": "tasks_list",
            "description": "The user's tasks as [handle] Title — details. Open tasks by default, most important first.",
            "inputSchema": { "type": "object", "properties": { "planner": { "type": "string" }, "include_done": { "type": "boolean" }, "due_before": { "type": "string", "description": "Only tasks due before this date (YYYY-MM-DD) or time." }, "limit": { "type": "integer", "minimum": 1, "maximum": 300 } } }
        },
        {
            "name": "tasks_add",
            "description": "Add one or more tasks / to-dos.",
            "inputSchema": { "type": "object", "properties": { "tasks": { "type": "array", "items": { "type": "object", "properties": { "title": { "type": "string" }, "notes": { "type": "string", "description": "Details." }, "status": { "type": "string", "enum": ["todo","doing","done"] }, "priority": { "type": "string", "enum": ["none","low","medium","high","urgent"] }, "due": { "type": "string", "description": "YYYY-MM-DD (all day) or YYYY-MM-DD HH:MM, the user's local time." }, "remind": { "type": "string", "description": "When to remind: YYYY-MM-DD HH:MM." }, "repeat": { "type": "string", "description": "daily, weekly, monthly, yearly, \"2 weeks\"… (needs a due date)." }, "planner": { "type": "string", "description": "Planner name or [handle]." } }, "required": ["title"] } } }, "required": ["tasks"] }
        },
        {
            "name": "tasks_edit",
            "description": "Change a task (by [handle] or title): rename, reschedule, set status (\"done\" ticks it off, and a repeating task moves to its next date), priority, planner. An empty due / remind / planner / repeat clears it.",
            "inputSchema": { "type": "object", "properties": { "task": { "type": "string" }, "title": { "type": "string" }, "notes": { "type": "string", "description": "Details." }, "status": { "type": "string", "enum": ["todo","doing","done"] }, "priority": { "type": "string", "enum": ["none","low","medium","high","urgent"] }, "due": { "type": "string", "description": "YYYY-MM-DD (all day) or YYYY-MM-DD HH:MM, the user's local time." }, "remind": { "type": "string", "description": "When to remind: YYYY-MM-DD HH:MM." }, "repeat": { "type": "string", "description": "daily, weekly, monthly, yearly, \"2 weeks\"… (needs a due date)." }, "planner": { "type": "string", "description": "Planner name or [handle]." } }, "required": ["task"] }
        },
        {
            "name": "tasks_delete",
            "description": "Delete a task for good (prefer marking it done unless the user asked to delete it).",
            "inputSchema": { "type": "object", "properties": { "task": { "type": "string" } }, "required": ["task"] }
        },
        {
            "name": "calendar_list",
            "description": "What's on the user's calendar between two dates: events and tasks due, including repeats.",
            "inputSchema": { "type": "object", "properties": { "from": { "type": "string", "description": "YYYY-MM-DD or YYYY-MM-DD HH:MM" }, "to": { "type": "string", "description": "YYYY-MM-DD (through the end of that day) or YYYY-MM-DD HH:MM" } }, "required": ["from", "to"] }
        },
        {
            "name": "calendar_add",
            "description": "Put an event on the calendar (appointment, work session, trip). A date without a time makes an all-day event; a start time alone makes it an hour long.",
            "inputSchema": { "type": "object", "properties": { "title": { "type": "string" }, "start": { "type": "string", "description": "YYYY-MM-DD or YYYY-MM-DD HH:MM, local time." }, "end": { "type": "string" }, "location": { "type": "string" }, "description": { "type": "string" }, "remind_minutes": { "type": "integer", "description": "Remind this many minutes before." } }, "required": ["title", "start"] }
        },
        {
            "name": "calendar_edit",
            "description": "Change an event (by [handle] or title). Moving only the start keeps its length.",
            "inputSchema": { "type": "object", "properties": { "event": { "type": "string" }, "title": { "type": "string" }, "start": { "type": "string" }, "end": { "type": "string" }, "location": { "type": "string" }, "description": { "type": "string" }, "remind_minutes": { "type": "integer", "description": "Negative removes the reminder." } }, "required": ["event"] }
        },
        {
            "name": "calendar_delete",
            "description": "Remove an event from the calendar.",
            "inputSchema": { "type": "object", "properties": { "event": { "type": "string" } }, "required": ["event"] }
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
                self.store.brain_context_for(folder, Some(&root), s(&args["task"]).as_deref()).map_err(err)
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
                let mut reply = if merged { format!("Updated [{}] (it already covered this).", m.short) } else { format!("Saved [{}].", m.short) };
                // Memories on the same thing that weren't merged: the agent should check for a conflict.
                if !merged && let Ok(similar) = self.store.brain_similar(folder, &m.hook, &m.content, Some(m.id)) && !similar.is_empty() {
                    let list: Vec<String> = similar.iter().map(|o| format!("[{}] {}", o.short, o.hook)).collect();
                    reply.push_str(&format!(" Similar memories already exist: {}. If this replaces or contradicts one, brain_update that one (or brain_forget the wrong one) instead of keeping both.", list.join("; ")));
                }
                Ok(reply)
            }
            "brain_handoff" => {
                let (folder, _) = self.project(args)?;
                let m = self.store.brain_handoff_save(folder, &s(&args["text"]).ok_or("text is required")?, &self.source.clone()).map_err(err)?;
                self.after_write();
                Ok(format!("Hand-off saved ([{}]). It's the first thing the next session sees.", m.short))
            }
            "code_outline" => {
                let (_, root) = self.project(args)?;
                let limit = args["limit"].as_u64().unwrap_or(120).clamp(1, 400) as usize;
                code::outline(&root, &s(&args["path"]).ok_or("path is required")?, limit)
            }
            "code_find" => {
                let (_, root) = self.project(args)?;
                let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 60) as usize;
                code::find(&root, &s(&args["name"]).ok_or("name is required")?, limit)
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
            n if n.starts_with("plan") || n.starts_with("tasks_") || n.starts_with("calendar_") => self.plan(n, args),
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

    /// Tasks, planners and the calendar. Brain → Connect in the app decides what agents may do.
    fn plan(&mut self, name: &str, args: &Value) -> Result<String, String> {
        let s = |v: &Value| v.as_str().map(str::to_owned);
        let err = |e: astralis_core::Error| e.to_string();
        let writes = !matches!(name, "plan_overview" | "tasks_list" | "calendar_list");
        match self.store.agent_plan_access().map_err(err)? {
            AgentAccess::Off => return Err("The user has turned off AI access to their tasks and calendar (Astralis → Brain → Connect).".into()),
            AgentAccess::Read if writes => return Err("The user lets AI agents read their tasks and calendar but not change them (Astralis → Brain → Connect). Suggest the change to the user instead.".into()),
            _ => {}
        }
        let task_of = |t: &Value| AgentTask {
            title: s(&t["title"]).unwrap_or_default(),
            notes: s(&t["notes"]),
            status: s(&t["status"]),
            priority: s(&t["priority"]),
            due: s(&t["due"]),
            remind: s(&t["remind"]),
            repeat: s(&t["repeat"]),
            planner: s(&t["planner"]),
        };
        let need = |k: &str| s(&args[k]).ok_or(format!("{k} is required"));
        match name {
            "plan_overview" => self.store.agent_planners().map_err(err),
            "planner_create" => {
                let tasks: Vec<AgentTask> = args["tasks"].as_array().map(|a| a.iter().map(task_of).collect()).unwrap_or_default();
                self.store.agent_planner_create(&need("name")?, &s(&args["description"]).unwrap_or_default(), &tasks).map_err(err)
            }
            "planner_update" => self.store.agent_planner_update(&need("planner")?, s(&args["name"]).as_deref(), s(&args["description"]).as_deref()).map_err(err),
            "planner_delete" => self.store.agent_planner_delete(&need("planner")?, args["with_tasks"].as_bool().unwrap_or(false)).map_err(err),
            "tasks_list" => {
                let limit = args["limit"].as_u64().unwrap_or(60).clamp(1, 300) as usize;
                self.store.agent_tasks(s(&args["planner"]).as_deref(), args["include_done"].as_bool().unwrap_or(false), s(&args["due_before"]).as_deref(), limit).map_err(err)
            }
            "tasks_add" => {
                let tasks: Vec<AgentTask> = args["tasks"].as_array().ok_or("tasks must be a list of {title, …}")?.iter().map(task_of).collect();
                self.store.agent_tasks_add(&tasks).map_err(err)
            }
            "tasks_edit" => {
                let e = AgentTaskEdit { title: s(&args["title"]), notes: s(&args["notes"]), status: s(&args["status"]), priority: s(&args["priority"]), due: s(&args["due"]), remind: s(&args["remind"]), repeat: s(&args["repeat"]), planner: s(&args["planner"]) };
                self.store.agent_task_edit(&need("task")?, &e).map_err(err)
            }
            "tasks_delete" => self.store.agent_task_delete(&need("task")?).map_err(err),
            "calendar_list" => self.store.agent_calendar(&need("from")?, &need("to")?).map_err(err),
            "calendar_add" => {
                let ev = AgentEvent { title: need("title")?, start: need("start")?, end: s(&args["end"]), location: s(&args["location"]), description: s(&args["description"]), remind_minutes: args["remind_minutes"].as_i64() };
                self.store.agent_event_add(&ev).map_err(err)
            }
            "calendar_edit" => {
                let ed = AgentEventEdit { title: s(&args["title"]), start: s(&args["start"]), end: s(&args["end"]), location: s(&args["location"]), description: s(&args["description"]), remind_minutes: args["remind_minutes"].as_i64() };
                self.store.agent_event_edit(&need("event")?, &ed).map_err(err)
            }
            "calendar_delete" => self.store.agent_event_delete(&need("event")?).map_err(err),
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

/// Commands for agent hooks (`astralis-mcp hook <event>`): they read the hook's JSON from stdin and
/// print what should be added to the agent's context, so the Brain is used without the agent having
/// to remember to ask. Always exit quietly: a hook must never get in the way of the session.
fn run_hook(event: &str, db: &Path) {
    use std::io::Read;
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let input: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    if let Some(text) = hook_output(event, &input, db) {
        println!("{text}");
    }
}

/// What a hook should add to the agent's context (nothing is `None`).
fn hook_output(event: &str, input: &Value, db: &Path) -> Option<String> {
    let cwd = input["cwd"].as_str().map(PathBuf::from).or_else(|| std::env::var_os("CLAUDE_PROJECT_DIR").map(PathBuf::from)).or_else(|| std::env::current_dir().ok()).unwrap_or_default();
    let mut store = Store::open(db).ok()?;
    let project = detect_project(&cwd);
    let folder = store.brain_project(&project.key, &project.name).ok()?;
    match event {
        // The index of what's remembered, at the start of a session (and after /clear or compaction).
        "session-start" => store.brain_context_for(folder, Some(&project.root), None).ok(),
        // Memories that clearly match the message just sent; nothing when none do, and nothing
        // that this session was already given (it would only cost tokens again).
        "prompt" => {
            let prompt = input["prompt"].as_str()?;
            let session = input["session_id"].as_str().unwrap_or("");
            let mut seen = hook_seen(db, session);
            let (text, handles) = store.brain_hook_prompt_new(folder, prompt, &seen).ok()??;
            seen.extend(handles);
            remember_hook_seen(db, session, &seen);
            Some(text)
        }
        // When the agent is about to finish: once, remind it to save what it learned.
        "stop" => {
            if input["stop_hook_active"].as_bool().unwrap_or(false) || !save_reminder_due(db, &project.key) {
                return None;
            }
            Some(json!({ "decision": "block", "reason": "Before you finish: if this session settled a decision, hit a gotcha, or worked out a command or where something lives, save it now with brain_remember (one idea per memory, a one-line hook that states the point), and update the hand-off note with brain_handoff (what's done, what's next), so no one has to dig through this chat later. If there is nothing worth saving, just finish." }).to_string())
        }
        _ => None,
    }
}

/// Handles of the memories already added to this session's messages (kept for the last 30 sessions).
fn hook_seen(db: &Path, session: &str) -> std::collections::HashSet<String> {
    if session.is_empty() {
        return Default::default();
    }
    let file = db.parent().map(|d| d.join("brain-hook-seen.json")).unwrap_or_default();
    let all: serde_json::Map<String, Value> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    all.get(session).and_then(|v| v["h"].as_array()).map(|a| a.iter().filter_map(|h| h.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

fn remember_hook_seen(db: &Path, session: &str, seen: &std::collections::HashSet<String>) {
    if session.is_empty() {
        return;
    }
    let file = db.parent().map(|d| d.join("brain-hook-seen.json")).unwrap_or_default();
    let mut all: serde_json::Map<String, Value> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut handles: Vec<&String> = seen.iter().collect();
    handles.sort();
    all.insert(session.to_string(), json!({ "t": now, "h": handles }));
    if all.len() > 30 {
        let mut by_age: Vec<(String, u64)> = all.iter().map(|(k, v)| (k.clone(), v["t"].as_u64().unwrap_or(0))).collect();
        by_age.sort_by_key(|(_, t)| *t);
        for (k, _) in by_age.into_iter().take(all.len() - 30) {
            all.remove(&k);
        }
    }
    let _ = std::fs::write(&file, Value::Object(all).to_string());
}

/// The save reminder runs at most once every 20 minutes per project (it costs the agent a turn).
fn save_reminder_due(db: &Path, key: &str) -> bool {
    let dir = db.parent().map(Path::to_path_buf).unwrap_or_default();
    let file = dir.join("brain-reminders.json");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let mut seen: serde_json::Map<String, Value> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if seen.get(key).and_then(Value::as_u64).is_some_and(|t| now.saturating_sub(t) < 20 * 60) {
        return false;
    }
    seen.insert(key.to_string(), json!(now));
    let _ = std::fs::write(&file, Value::Object(seen).to_string());
    true
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
    // `astralis-mcp hook <event>`: for agent hooks (see run_hook), not the MCP server.
    if let Some(pos) = std::env::args().position(|a| a == "hook") {
        if let Some(event) = std::env::args().nth(pos + 1) {
            run_hook(&event, &db);
        }
        return;
    }
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

#[cfg(test)]
mod hook_tests {
    use super::*;

    /// A project folder and a database, both in a fresh temp folder.
    fn setup(tag: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("astralis-hooks-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("proj/.git")).unwrap();
        (dir.join("proj"), dir.join("astralis.db"))
    }

    #[test]
    fn hooks_give_the_index_then_only_new_relevant_memories_and_one_reminder() {
        let (proj, db) = setup("flow");
        let cwd = proj.to_string_lossy().to_string();
        {
            let mut store = Store::open(&db).unwrap();
            let project = detect_project(&proj);
            let folder = store.brain_project(&project.key, &project.name).unwrap();
            store.remember(folder, MemoryKind::Gotcha, "Python is broken on this machine", "Use node for scripted edits instead of python.", "claude-code").unwrap();
        }
        let input = |extra: Value| {
            let mut v = json!({ "cwd": cwd, "session_id": "s1" });
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            v
        };
        // Session start: the index (with the rule), including what's remembered.
        let start = hook_output("session-start", &input(json!({})), &db).expect("an index");
        assert!(start.contains("Python is broken") && start.contains("not old chats"), "{start}");
        // A relevant message gets the memory; the same memory isn't added to the next message.
        let first = hook_output("prompt", &input(json!({ "prompt": "write a python script to edit these files" })), &db).expect("recalled");
        assert!(first.contains("Python is broken"), "{first}");
        assert!(hook_output("prompt", &input(json!({ "prompt": "another python script for editing more files" })), &db).is_none(), "already given this session");
        // Another session is given it again; unrelated or empty messages add nothing.
        let other = json!({ "cwd": cwd, "session_id": "s2", "prompt": "write a python script to edit these files" });
        assert!(hook_output("prompt", &other, &db).is_some());
        assert!(hook_output("prompt", &input(json!({ "prompt": "thanks, that works" })), &db).is_none());
        // The save reminder: once, never while already continuing from one.
        assert!(hook_output("stop", &input(json!({ "stop_hook_active": true })), &db).is_none());
        let stop = hook_output("stop", &input(json!({ "stop_hook_active": false })), &db).expect("reminder");
        assert!(stop.contains("\"decision\":\"block\"") && stop.contains("brain_handoff"));
        assert!(hook_output("stop", &input(json!({ "stop_hook_active": false })), &db).is_none(), "rate limited");
        assert!(hook_output("nonsense", &input(json!({})), &db).is_none());
        let _ = std::fs::remove_dir_all(db.parent().unwrap());
    }
}
