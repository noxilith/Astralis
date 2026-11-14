//! The user's notes for AI agents (through the Brain's MCP server): find, read, write, edit and
//! archive notes in their vaults. Answers are short plain text, like the Brain's, to keep tokens
//! down: notes are referred to by a short `[handle]` (the end of the note's id).
//!
//! Agents only reach plain vaults: encrypted vaults can't be unlocked from outside the app, and
//! the Brain vault has its own tools. Every change goes through the same code the app uses, so
//! version history, links and search stay right, and nothing is ever deleted outright: "delete"
//! archives a note, which the user can restore.

use std::collections::HashMap;

use uuid::Uuid;

use crate::error::{Error, Result};
use crate::folders::Folder;
use crate::memory::VAULT_KEY;
use crate::notes::{ArchiveFilter, ListFilter, NewNote, Note, NoteKind, NotePatch};
use crate::store::Store;
use crate::util::Timestamp;
use crate::vault::Vault;

/// Settings key: what agents may do with notes ("edit", the default; "read"; or "off").
pub const ACCESS_KEY: &str = "agents.notes";
/// Longest note body handed to an agent in one read.
const MAX_READ: usize = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAccess {
    Off,
    Read,
    Edit,
}

impl AgentAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Read => "read",
            Self::Edit => "edit",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "read" => Some(Self::Read),
            "edit" => Some(Self::Edit),
            _ => None,
        }
    }
}

/// One change to a note. At most one of `body`, `append` and `replace` (old text, new text,
/// every occurrence) changes the text; `title` and `tags` can come along with any of them.
#[derive(Debug, Clone, Default)]
pub struct NoteEdit {
    pub title: Option<String>,
    pub body: Option<String>,
    pub append: Option<String>,
    pub replace: Option<(String, String, bool)>,
    pub tags: Option<Vec<String>>,
}

fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

/// The `[handle]` agents use for a note: the end of its id (the random part of a UUIDv7).
pub fn handle(id: Uuid) -> String {
    let s = id.simple().to_string();
    s[s.len() - 6..].to_string()
}

fn day(ts: Timestamp) -> String {
    chrono::DateTime::from_timestamp_millis(ts).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

impl Store {
    pub fn agent_access(&self) -> Result<AgentAccess> {
        Ok(self.get_setting(ACCESS_KEY)?.and_then(|s| AgentAccess::parse(&s)).unwrap_or(AgentAccess::Edit))
    }

    pub fn set_agent_access(&self, access: AgentAccess) -> Result<()> {
        self.set_setting(ACCESS_KEY, access.as_str())
    }

    /// Vaults agents can use: plain ones (encrypted vaults stay locked to them), not the Brain.
    fn agent_vaults_list(&self) -> Result<Vec<Vault>> {
        let brain = self.get_setting(VAULT_KEY)?.and_then(|s| s.parse::<Uuid>().ok());
        Ok(self.list_vaults()?.into_iter().filter(|v| !v.encrypted && Some(v.id) != brain).collect())
    }

    /// A vault by name (any case) or id; `None` means the default vault.
    fn agent_vault(&self, name: Option<&str>) -> Result<Vault> {
        let vaults = self.agent_vaults_list()?;
        match name.map(str::trim).filter(|n| !n.is_empty()) {
            None => {
                let default = self.default_vault()?;
                vaults.into_iter().find(|v| v.id == default).ok_or_else(|| invalid("the default vault is encrypted; name a vault (see notes_vaults)"))
            }
            Some(n) => {
                let lower = n.to_lowercase();
                vaults
                    .iter()
                    .find(|v| v.name.to_lowercase() == lower || v.id.to_string() == n)
                    .cloned()
                    .ok_or_else(|| invalid(format!("no vault called \"{n}\" that agents can use; vaults: {}", vaults.iter().map(|v| v.name.as_str()).collect::<Vec<_>>().join(", "))))
            }
        }
    }

    /// Folder paths ("Work/Projects") by folder id, for one vault.
    fn folder_paths(&self, vault: Uuid) -> Result<HashMap<Uuid, String>> {
        let folders = self.list_folders(Some(vault))?;
        let by_id: HashMap<Uuid, &Folder> = folders.iter().map(|f| (f.id, f)).collect();
        let mut out = HashMap::new();
        for f in &folders {
            let mut parts = vec![f.name.clone()];
            let mut cur = f.parent_id;
            let mut guard = 0;
            while let (Some(p), true) = (cur, guard < 64) {
                let Some(parent) = by_id.get(&p) else { break };
                parts.push(parent.name.clone());
                cur = parent.parent_id;
                guard += 1;
            }
            parts.reverse();
            out.insert(f.id, parts.join("/"));
        }
        Ok(out)
    }

    /// A folder by path ("Work/Projects") or name; with `create`, makes the missing ones.
    fn agent_folder(&self, vault: Uuid, path: &str, create: bool) -> Result<Option<Uuid>> {
        let path = path.trim().trim_matches('/');
        if path.is_empty() {
            return Ok(None);
        }
        let paths = self.folder_paths(vault)?;
        let lower = path.to_lowercase();
        if let Some((id, _)) = paths.iter().find(|(_, p)| p.to_lowercase() == lower) {
            return Ok(Some(*id));
        }
        // A bare name that's unique in the vault.
        let named: Vec<_> = paths.iter().filter(|(_, p)| p.rsplit('/').next().is_some_and(|n| n.to_lowercase() == lower)).collect();
        if named.len() == 1 {
            return Ok(Some(*named[0].0));
        }
        if !create {
            return Err(invalid(format!("no folder \"{path}\" in this vault")));
        }
        let mut parent: Option<Uuid> = None;
        let mut so_far = String::new();
        for part in path.split('/').map(str::trim).filter(|p| !p.is_empty()) {
            so_far = if so_far.is_empty() { part.to_string() } else { format!("{so_far}/{part}") };
            let found = self.folder_paths(vault)?.into_iter().find(|(_, p)| p.to_lowercase() == so_far.to_lowercase()).map(|(id, _)| id);
            parent = Some(match found {
                Some(id) => id,
                None => self.create_folder(Some(vault), parent, part)?.id,
            });
        }
        Ok(parent)
    }

    /// A note by `[handle]` (or its full id) or exact title, in the vaults agents can use.
    fn agent_note(&self, which: &str) -> Result<Note> {
        let key = which.trim().trim_start_matches('[').trim_end_matches(']').trim();
        if key.is_empty() {
            return Err(invalid("say which note: its [handle] or title"));
        }
        if let Ok(id) = key.parse::<Uuid>() {
            return self.agent_note_by_id(id);
        }
        let mut by_handle = Vec::new();
        let mut by_title = Vec::new();
        for v in self.agent_vaults_list()? {
            for n in self.list_notes(&ListFilter { vault_id: Some(v.id), archive: ArchiveFilter::All, ..Default::default() })? {
                if handle(n.id) == key.to_lowercase() {
                    by_handle.push(n);
                } else if n.title.trim().eq_ignore_ascii_case(key) {
                    by_title.push(n);
                }
            }
        }
        let found = if by_handle.is_empty() { by_title } else { by_handle };
        match found.len() {
            1 => Ok(found.into_iter().next().unwrap()),
            0 => Err(invalid(format!("no note \"{key}\" (use a [handle] from notes_search or notes_list)"))),
            n => Err(invalid(format!("{n} notes are called \"{key}\"; use one of their [handles]: {}", found.iter().map(|n| format!("[{}]", handle(n.id))).collect::<Vec<_>>().join(" ")))),
        }
    }

    fn agent_note_by_id(&self, id: Uuid) -> Result<Note> {
        let note = self.get_note(id)?;
        if !self.agent_vaults_list()?.iter().any(|v| v.id == note.vault_id) {
            return Err(invalid("that note is in a vault agents can't use"));
        }
        Ok(note)
    }

    /// Where a note is, for showing it ("Personal/Work/Projects").
    fn note_place(&self, note: &Note, vault_names: &HashMap<Uuid, String>, folders: &HashMap<Uuid, String>) -> String {
        let vault = vault_names.get(&note.vault_id).cloned().unwrap_or_default();
        match note.folder_id.and_then(|f| folders.get(&f)) {
            Some(path) => format!("{vault}/{path}"),
            None => vault,
        }
    }

    // ---- what agents call ----

    /// The vaults agents can use, with note counts.
    pub fn agent_vaults(&self) -> Result<String> {
        let vaults = self.agent_vaults_list()?;
        let default = self.default_vault().ok();
        let mut out = String::new();
        for v in &vaults {
            let mut what = vec![];
            if Some(v.id) == default {
                what.push("default");
            }
            if v.cloud.is_some() {
                what.push("Google Drive");
            }
            if v.owner.is_some() {
                what.push("shared with you");
            } else if !v.shared_with.is_empty() {
                what.push("shared");
            }
            let extra = if what.is_empty() { String::new() } else { format!(" ({})", what.join(", ")) };
            out.push_str(&format!("{} — {} notes{extra}\n", v.name, v.note_count));
        }
        let locked = self.list_vaults()?.iter().filter(|v| v.encrypted).count();
        if locked > 0 {
            out.push_str(&format!("({locked} encrypted vault{} not available to agents)\n", if locked == 1 { " is" } else { "s are" }));
        }
        if vaults.is_empty() {
            out.push_str("No vaults agents can use.\n");
        }
        Ok(out)
    }

    /// Best matches for words, across every vault agents can use (or one).
    pub fn agent_search(&self, query: &str, vault: Option<&str>, limit: usize) -> Result<String> {
        let vaults = match vault {
            Some(_) => vec![self.agent_vault(vault)?],
            None => self.agent_vaults_list()?,
        };
        let names: HashMap<Uuid, String> = vaults.iter().map(|v| (v.id, v.name.clone())).collect();
        let mut hits = Vec::new();
        for v in &vaults {
            for h in self.search(Some(v.id), query, false, limit)? {
                hits.push((v.id, h));
            }
        }
        hits.sort_by(|a, b| a.1.score.partial_cmp(&b.1.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        if hits.is_empty() {
            return Ok(format!("No notes match \"{query}\"."));
        }
        let mut folders: HashMap<Uuid, HashMap<Uuid, String>> = HashMap::new();
        let mut out = String::new();
        for (vault, h) in hits {
            let paths = match folders.get(&vault) {
                Some(p) => p,
                None => folders.entry(vault).or_insert(self.folder_paths(vault)?),
            };
            let note = self.get_note(h.id)?;
            let place = self.note_place(&note, &names, paths);
            let snippet = h.snippet.replace('\u{2}', "«").replace('\u{3}', "»").replace('\n', " ");
            let kind = if h.kind == NoteKind::Whiteboard { " (whiteboard)" } else { "" };
            out.push_str(&format!("[{}] {}{kind} — {place}: {}\n", handle(h.id), h.title, snippet.trim()));
        }
        Ok(out)
    }

    /// The notes in a vault (optionally one folder, or with a tag).
    pub fn agent_list(&self, vault: Option<&str>, folder: Option<&str>, tag: Option<&str>, limit: usize) -> Result<String> {
        let v = self.agent_vault(vault)?;
        let paths = self.folder_paths(v.id)?;
        let only_folder = match folder {
            Some(f) if !f.trim().is_empty() => Some(self.agent_folder(v.id, f, false)?),
            _ => None,
        };
        let tag = tag.map(|t| t.trim().trim_start_matches('#').to_string()).filter(|t| !t.is_empty());
        let mut notes: Vec<Note> = self
            .list_notes(&ListFilter { vault_id: Some(v.id), tag, ..Default::default() })?
            .into_iter()
            .filter(|n| only_folder.is_none_or(|f| n.folder_id == f))
            .collect();
        let place = |n: &Note| n.folder_id.and_then(|f| paths.get(&f).cloned()).unwrap_or_default();
        notes.sort_by(|a, b| place(a).to_lowercase().cmp(&place(b).to_lowercase()).then(a.title.to_lowercase().cmp(&b.title.to_lowercase())));
        let total = notes.len();
        let mut out = format!("{}: {total} note{}\n", v.name, if total == 1 { "" } else { "s" });
        for n in notes.iter().take(limit) {
            let folder = place(n);
            let at = if folder.is_empty() { String::new() } else { format!(" — {folder}") };
            let kind = if n.kind == NoteKind::Whiteboard { " (whiteboard)" } else { "" };
            let pin = if n.pinned { " 📌" } else { "" };
            out.push_str(&format!("[{}] {}{kind}{pin}{at}\n", handle(n.id), if n.title.is_empty() { "Untitled" } else { &n.title }));
        }
        if total > limit {
            out.push_str(&format!("… and {} more (narrow it with folder or tag, or use notes_search)\n", total - limit));
        }
        Ok(out)
    }

    /// A note in full.
    pub fn agent_read(&self, which: &str) -> Result<String> {
        let n = self.agent_note(which)?;
        let names: HashMap<Uuid, String> = self.agent_vaults_list()?.into_iter().map(|v| (v.id, v.name)).collect();
        let place = self.note_place(&n, &names, &self.folder_paths(n.vault_id)?);
        let mut head = format!("[{}] {}\n{place}", handle(n.id), if n.title.is_empty() { "Untitled" } else { &n.title });
        if !n.tags.is_empty() {
            head.push_str(&format!(" · tags: {}", n.tags.join(", ")));
        }
        head.push_str(&format!(" · updated {}", day(n.updated_at)));
        if n.archived_at.is_some() {
            head.push_str(" · ARCHIVED");
        }
        if n.kind == NoteKind::Whiteboard {
            return Ok(format!("{head}\n(This is a whiteboard: a drawing, not text.)"));
        }
        let mut body = n.body.clone();
        if body.len() > MAX_READ {
            let mut cut = MAX_READ;
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
            body.push_str("\n… (cut off here; the note is longer)");
        }
        Ok(format!("{head}\n---\n{body}"))
    }

    /// A new note. The folder (a path like "Work/Projects") is made if it doesn't exist yet.
    pub fn agent_create(&self, title: &str, body: &str, vault: Option<&str>, folder: Option<&str>, tags: &[String]) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(invalid("a note needs a title"));
        }
        let v = self.agent_vault(vault)?;
        let folder_id = match folder {
            Some(f) => self.agent_folder(v.id, f, true)?,
            None => None,
        };
        let tags: Vec<String> = tags.iter().map(|t| t.trim().trim_start_matches('#').to_string()).filter(|t| !t.is_empty()).collect();
        let n = self.create_note(NewNote { title: title.into(), body: body.into(), tags, vault_id: Some(v.id), folder_id, ..Default::default() })?;
        let names = HashMap::from([(v.id, v.name.clone())]);
        Ok(format!("Created [{}] {} in {}.", handle(n.id), n.title, self.note_place(&n, &names, &self.folder_paths(v.id)?)))
    }

    /// Changes a note (see [`NoteEdit`]). The previous version stays in the note's history.
    pub fn agent_edit(&self, which: &str, edit: NoteEdit) -> Result<String> {
        let n = self.agent_note(which)?;
        if n.archived_at.is_some() {
            return Err(invalid("that note is archived; restore it first (notes_restore)"));
        }
        let text_ops = [edit.body.is_some(), edit.append.is_some(), edit.replace.is_some()].iter().filter(|b| **b).count();
        if text_ops > 1 {
            return Err(invalid("change the text one way at a time: body, append or old_text/new_text"));
        }
        if text_ops > 0 && n.kind == NoteKind::Whiteboard {
            return Err(invalid("that's a whiteboard (a drawing); only its title and tags can be changed"));
        }
        let body = if let Some(b) = edit.body {
            Some(b)
        } else if let Some(more) = edit.append {
            let sep = if n.body.is_empty() || n.body.ends_with("\n\n") {
                ""
            } else if n.body.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            Some(format!("{}{sep}{more}", n.body))
        } else if let Some((old, new, all)) = edit.replace {
            if old.is_empty() {
                return Err(invalid("old_text is empty"));
            }
            let count = n.body.matches(&old).count();
            match (count, all) {
                (0, _) => return Err(invalid("old_text isn't in the note (it must match exactly, spaces and line breaks included; notes_read shows the text)")),
                (1, _) | (_, true) => Some(n.body.replace(&old, &new)),
                (c, false) => return Err(invalid(format!("old_text is in the note {c} times; include more of the text around it, or set replace_all"))),
            }
        } else {
            None
        };
        let title = edit.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
        let mut changed = vec![];
        let mut note = n.clone();
        if title.is_some() || body.is_some() {
            if title.as_deref().is_some_and(|t| t != n.title) {
                changed.push("title");
            }
            if body.as_deref().is_some_and(|b| b != n.body) {
                changed.push("text");
            }
            note = self.update_note(n.id, NotePatch { title, body })?;
        }
        if let Some(tags) = edit.tags {
            let tags: Vec<String> = tags.iter().map(|t| t.trim().trim_start_matches('#').to_string()).filter(|t| !t.is_empty()).collect();
            note = self.set_tags(n.id, &tags)?;
            changed.push("tags");
        }
        if changed.is_empty() {
            return Ok(format!("[{}] {} is unchanged.", handle(n.id), n.title));
        }
        Ok(format!("Updated [{}] {} ({}). The previous version is in its history.", handle(note.id), note.title, changed.join(", ")))
    }

    /// "Deletes" a note: archives it (the user can restore it).
    pub fn agent_archive(&self, which: &str) -> Result<String> {
        let n = self.agent_note(which)?;
        if n.archived_at.is_some() {
            return Ok(format!("[{}] {} is already archived.", handle(n.id), n.title));
        }
        self.archive_note(n.id)?;
        Ok(format!("Archived [{}] {}. It can be restored from the archive.", handle(n.id), n.title))
    }

    pub fn agent_restore(&self, which: &str) -> Result<String> {
        let n = self.agent_note(which)?;
        if n.archived_at.is_none() {
            return Ok(format!("[{}] {} isn't archived.", handle(n.id), n.title));
        }
        self.unarchive_note(n.id)?;
        Ok(format!("Restored [{}] {}.", handle(n.id), n.title))
    }

    /// Deletes a note: by default it goes to the archive (restorable). `permanent` removes an
    /// already-archived note for good, so a permanent delete always takes two deliberate steps.
    pub fn agent_delete(&self, which: &str, permanent: bool) -> Result<String> {
        if !permanent {
            return self.agent_archive(which);
        }
        let n = self.agent_note(which)?;
        if n.archived_at.is_none() {
            return Err(invalid("only archived notes can be deleted for good: delete it without permanent first (it goes to the archive), then again with permanent"));
        }
        self.delete_note(n.id)?;
        Ok(format!("Deleted [{}] {} for good (it can't be restored).", handle(n.id), n.title))
    }

    // ---- folders ----

    /// A vault's folders as a tree, with how many notes each holds.
    pub fn agent_folders(&self, vault: Option<&str>) -> Result<String> {
        let v = self.agent_vault(vault)?;
        let folders = self.list_folders(Some(v.id))?;
        if folders.is_empty() {
            return Ok(format!("{} has no folders.", v.name));
        }
        let mut counts: HashMap<Option<Uuid>, usize> = HashMap::new();
        for n in self.list_notes(&ListFilter { vault_id: Some(v.id), ..Default::default() })? {
            *counts.entry(n.folder_id).or_default() += 1;
        }
        let mut children: HashMap<Option<Uuid>, Vec<&Folder>> = HashMap::new();
        for f in &folders {
            children.entry(f.parent_id).or_default().push(f);
        }
        for list in children.values_mut() {
            list.sort_by_key(|f| f.name.to_lowercase());
        }
        let top = counts.get(&None).copied().unwrap_or(0);
        let mut out = format!("{} folders (top level: {top} note{})\n", v.name, if top == 1 { "" } else { "s" });
        fn walk(out: &mut String, at: Option<Uuid>, depth: usize, children: &HashMap<Option<Uuid>, Vec<&Folder>>, counts: &HashMap<Option<Uuid>, usize>) {
            for f in children.get(&at).into_iter().flatten() {
                let n = counts.get(&Some(f.id)).copied().unwrap_or(0);
                out.push_str(&format!("{}{} ({n} note{})\n", "  ".repeat(depth), f.name, if n == 1 { "" } else { "s" }));
                if depth < 32 {
                    walk(out, Some(f.id), depth + 1, children, counts);
                }
            }
        }
        walk(&mut out, None, 0, &children, &counts);
        Ok(out)
    }

    /// Makes a folder (and any missing folders above it), e.g. "Work/Projects/Launch".
    pub fn agent_folder_create(&self, path: &str, vault: Option<&str>) -> Result<String> {
        let v = self.agent_vault(vault)?;
        if path.trim().trim_matches('/').is_empty() {
            return Err(invalid("give the folder a name or path"));
        }
        let existed = self.agent_folder(v.id, path, false).is_ok();
        self.agent_folder(v.id, path, true)?;
        Ok(if existed { format!("{}/{} already exists.", v.name, path.trim().trim_matches('/')) } else { format!("Created {}/{}.", v.name, path.trim().trim_matches('/')) })
    }

    pub fn agent_folder_rename(&self, folder: &str, name: &str, vault: Option<&str>) -> Result<String> {
        let v = self.agent_vault(vault)?;
        let id = self.agent_folder(v.id, folder, false)?.ok_or_else(|| invalid("that's the top level, not a folder"))?;
        let name = name.trim();
        if name.is_empty() || name.contains('/') {
            return Err(invalid("a folder name can't be empty or contain /"));
        }
        self.rename_folder(id, name)?;
        Ok(format!("Renamed {folder} to {name} in {}.", v.name))
    }

    /// Deletes a folder. Nothing in it is lost: its notes and subfolders move up a level.
    pub fn agent_folder_delete(&self, folder: &str, vault: Option<&str>) -> Result<String> {
        let v = self.agent_vault(vault)?;
        let id = self.agent_folder(v.id, folder, false)?.ok_or_else(|| invalid("that's the top level, not a folder"))?;
        self.delete_folder(id)?;
        Ok(format!("Deleted the folder {folder} in {}; what was in it moved up a level.", v.name))
    }

    // ---- moving ----

    /// Moves a note into a folder (a path; "" or "/" is the top level), and/or to another vault.
    pub fn agent_move_note(&self, which: &str, to_folder: Option<&str>, to_vault: Option<&str>) -> Result<String> {
        let n = self.agent_note(which)?;
        let target = match to_vault {
            Some(v) => self.agent_vault(Some(v))?,
            None => self.agent_vaults_list()?.into_iter().find(|v| v.id == n.vault_id).ok_or_else(|| invalid("that note is in a vault agents can't use"))?,
        };
        if to_folder.is_none() && target.id == n.vault_id {
            return Err(invalid("say where to: to_folder, to_vault, or both"));
        }
        let mut note = if target.id != n.vault_id { self.move_note_to_vault(n.id, target.id)? } else { n.clone() };
        // A move to another vault lands at its top level unless a folder is given.
        let folder = match to_folder {
            Some(f) => self.agent_folder(target.id, f, true)?,
            None if target.id != n.vault_id => None,
            None => note.folder_id,
        };
        if note.folder_id != folder {
            note = self.move_note_to_folder(n.id, folder)?;
        }
        let names = HashMap::from([(target.id, target.name.clone())]);
        Ok(format!("Moved [{}] {} to {}.", handle(n.id), note.title, self.note_place(&note, &names, &self.folder_paths(target.id)?)))
    }

    /// Moves a folder (with everything in it) into another folder, to the top level ("" or "/"),
    /// or to another vault.
    pub fn agent_move_folder(&self, folder: &str, vault: Option<&str>, to_folder: Option<&str>, to_vault: Option<&str>) -> Result<String> {
        let from = self.agent_vault(vault)?;
        let id = self.agent_folder(from.id, folder, false)?.ok_or_else(|| invalid("that's the top level, not a folder"))?;
        let target = match to_vault {
            Some(v) => self.agent_vault(Some(v))?,
            None => from.clone(),
        };
        let parent = match to_folder {
            Some(f) => self.agent_folder(target.id, f, true)?,
            None => None,
        };
        if target.id == from.id {
            let moved = self.move_folder(id, parent)?;
            let path = self.folder_paths(from.id)?.get(&moved.id).cloned().unwrap_or(moved.name);
            return Ok(format!("Moved the folder to {}/{path}.", from.name));
        }
        // To another vault: the same folders there (joining one of the same name), every note
        // moved across (with its images), then the emptied folders here removed.
        let moved = self.move_folder_tree(id, from.id, target.id, parent)?;
        let path = self.folder_paths(target.id)?.get(&moved).cloned().unwrap_or_default();
        Ok(format!("Moved the folder and everything in it to {}/{path}.", target.name))
    }

    pub fn move_folder_to_vault(&self, folder: Uuid, from: Uuid, to: Uuid) -> Result<Uuid> {
        if from == to {
            return Ok(folder);
        }
        self.move_folder_tree(folder, from, to, None)
    }

    fn move_folder_tree(&self, folder: Uuid, from: Uuid, to: Uuid, parent: Option<Uuid>) -> Result<Uuid> {
        let src = self.list_folders(Some(from))?.into_iter().find(|f| f.id == folder).ok_or(Error::NotFound("folder", folder))?;
        let existing = self.list_folders(Some(to))?.into_iter().find(|f| f.parent_id == parent && f.name.eq_ignore_ascii_case(&src.name));
        let dest = match existing {
            Some(f) => f.id,
            None => self.create_folder(Some(to), parent, &src.name)?.id,
        };
        let notes = self.list_notes(&ListFilter { vault_id: Some(from), archive: ArchiveFilter::All, ..Default::default() })?;
        for n in notes.iter().filter(|n| n.folder_id == Some(folder)) {
            self.move_note_to_vault(n.id, to)?;
            self.move_note_to_folder(n.id, Some(dest))?;
        }
        let subs: Vec<Uuid> = self.list_folders(Some(from))?.into_iter().filter(|f| f.parent_id == Some(folder)).map(|f| f.id).collect();
        for sub in subs {
            self.move_folder_tree(sub, from, to, Some(dest))?;
        }
        self.delete_folder(folder)?;
        Ok(dest)
    }

    // ---- for the app: noticing changes made by agents (another process) ----

    /// Goes up whenever another connection (e.g. the Brain's MCP server) commits a change.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA data_version", [], |r| r.get(0))?)
    }

    /// Notes changed after `since` (any vault, archived or not), and the newest change time.
    pub fn notes_changed_since(&self, since: Timestamp) -> Result<(Vec<Uuid>, Timestamp)> {
        let mut stmt = self.conn.prepare_cached("SELECT id, updated_at FROM notes WHERE updated_at > ?1")?;
        let rows = stmt.query_map([since], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, i64>(1)?)))?;
        let mut ids = Vec::new();
        let mut newest = since;
        for row in rows {
            let (id, at) = row?;
            if let Ok(id) = Uuid::from_slice(&id) {
                ids.push(id);
            }
            newest = newest.max(at);
        }
        Ok((ids, newest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        let personal = s.default_vault().unwrap();
        s.create_note(NewNote { title: "Game idea".into(), body: "A space farming game.\n\n## Features\n- crops\n- crops in space".into(), tags: vec!["game".into()], vault_id: Some(personal), ..Default::default() }).unwrap();
        s.create_note(NewNote { title: "Groceries".into(), body: "milk, eggs".into(), vault_id: Some(personal), ..Default::default() }).unwrap();
        let work = s.create_vault("Work", None).unwrap();
        s.create_note(NewNote { title: "Standup".into(), body: "Daily standup notes about the launch.".into(), vault_id: Some(work.id), ..Default::default() }).unwrap();
        // Off limits: an encrypted vault and the Brain.
        let secret = s.create_vault("Diary", Some("password1")).unwrap();
        s.create_note(NewNote { title: "Secret launch plans".into(), body: "launch".into(), vault_id: Some(secret.id), ..Default::default() }).unwrap();
        let brain = s.brain_vault().unwrap();
        s.create_note(NewNote { title: "Brain memory about launch".into(), body: "launch".into(), vault_id: Some(brain), ..Default::default() }).unwrap();
        s
    }

    fn handle_of(s: &Store, title: &str) -> String {
        let line = s.agent_search(title, None, 10).unwrap();
        line.lines().find(|l| l.contains(title)).and_then(|l| l.split(']').next()).unwrap().trim_start_matches('[').to_string()
    }

    #[test]
    fn agents_see_plain_vaults_only() {
        let s = store();
        let vaults = s.agent_vaults().unwrap();
        assert!(vaults.contains("Work") && vaults.contains("default"));
        assert!(!vaults.contains("Diary") && !vaults.contains("Brain"));
        assert!(vaults.contains("1 encrypted vault is not available"));
        let found = s.agent_search("launch", None, 10).unwrap();
        assert!(found.contains("Standup"), "{found}");
        assert!(!found.contains("Secret") && !found.contains("Brain memory"), "{found}");
        assert!(s.agent_read("Secret launch plans").is_err());
    }

    #[test]
    fn notes_are_found_by_handle_or_title_and_listed() {
        let s = store();
        let h = handle_of(&s, "Game idea");
        let by_handle = s.agent_read(&format!("[{h}]")).unwrap();
        let by_title = s.agent_read("game IDEA").unwrap();
        assert_eq!(by_handle, by_title);
        assert!(by_handle.contains("tags: game") && by_handle.contains("## Features"));
        let list = s.agent_list(None, None, None, 50).unwrap();
        assert!(list.contains("2 notes") && list.contains("Groceries"));
        assert!(s.agent_list(Some("work"), None, None, 50).unwrap().contains("Standup"));
        assert!(s.agent_list(Some("Nope"), None, None, 50).unwrap_err().to_string().contains("vaults: "));
    }

    #[test]
    fn edits_replace_exact_text_append_and_keep_history() {
        let s = store();
        // Ambiguous and missing text are refused, with a hint.
        let twice = s.agent_edit("Game idea", NoteEdit { replace: Some(("crops".into(), "plants".into(), false)), ..Default::default() });
        assert!(twice.unwrap_err().to_string().contains("2 times"));
        assert!(s.agent_edit("Game idea", NoteEdit { replace: Some(("nope".into(), "x".into(), false)), ..Default::default() }).is_err());
        // Unique text, then everything at once.
        s.agent_edit("Game idea", NoteEdit { replace: Some(("- crops in space".into(), "- crops in low gravity".into(), false)), ..Default::default() }).unwrap();
        s.agent_edit("Game idea", NoteEdit { replace: Some(("crops".into(), "plants".into(), true)), ..Default::default() }).unwrap();
        s.agent_edit("Game idea", NoteEdit { append: Some("## Plan\n- prototype".into()), title: Some("Space farm".into()), tags: Some(vec!["#game".into(), "ideas".into()]), ..Default::default() }).unwrap();
        let read = s.agent_read("Space farm").unwrap();
        assert!(read.contains("- plants in low gravity") && read.contains("## Plan") && read.contains("tags: game, ideas"), "{read}");
        // Two text changes at once are refused.
        assert!(s.agent_edit("Space farm", NoteEdit { body: Some("x".into()), append: Some("y".into()), ..Default::default() }).is_err());
        // The earlier text is in the note's history.
        let id = s.agent_note("Space farm").unwrap().id;
        assert!(s.note_history(id).unwrap().iter().any(|v| v.body.contains("crops in space")));
    }

    #[test]
    fn create_makes_folders_and_archive_can_be_undone() {
        let s = store();
        let made = s.agent_create("Roadmap", "- ship 1.0", Some("Work"), Some("Projects/Launch"), &["plan".into()]).unwrap();
        assert!(made.contains("Work/Projects/Launch"), "{made}");
        assert!(s.agent_list(Some("Work"), Some("Launch"), None, 50).unwrap().contains("Roadmap"));
        s.agent_archive("Roadmap").unwrap();
        assert!(!s.agent_list(Some("Work"), None, None, 50).unwrap().contains("Roadmap"));
        assert!(s.agent_edit("Roadmap", NoteEdit { append: Some("x".into()), ..Default::default() }).unwrap_err().to_string().contains("archived"));
        s.agent_restore("Roadmap").unwrap();
        assert!(s.agent_list(Some("Work"), None, None, 50).unwrap().contains("Roadmap"));
    }

    #[test]
    fn whiteboards_keep_their_drawing() {
        let s = store();
        s.create_note(NewNote { kind: NoteKind::Whiteboard, title: "Sketch".into(), body: "{}".into(), ..Default::default() }).unwrap();
        assert!(s.agent_read("Sketch").unwrap().contains("whiteboard"));
        assert!(s.agent_edit("Sketch", NoteEdit { body: Some("text".into()), ..Default::default() }).is_err());
        assert!(s.agent_edit("Sketch", NoteEdit { title: Some("Board".into()), ..Default::default() }).is_ok());
    }

    #[test]
    fn cloud_vault_notes_can_be_edited_and_go_out_with_the_next_sync() {
        use crate::cloud::{CloudVault, GOOGLE_DRIVE};
        let mut s = store();
        let cloud = s.create_vault("Game studio", None).unwrap();
        s.set_cloud_vault(&CloudVault { vault_id: cloud.id, provider: GOOGLE_DRIVE.into(), folder_id: "f".into(), sync_key: vec![7; 32], owned: true, uploaded_vv: Default::default() }).unwrap();
        s.create_note(NewNote { title: "Pitch".into(), body: "A cozy game.".into(), vault_id: Some(cloud.id), ..Default::default() }).unwrap();
        // Everything so far has been "uploaded".
        let uploaded = s.vault_version_vector(cloud.id).unwrap();

        assert!(s.agent_vaults().unwrap().contains("Game studio — 1 notes (Google Drive)"));
        s.agent_edit("Pitch", NoteEdit { append: Some("With trading.".into()), ..Default::default() }).unwrap();
        assert!(s.agent_create("Plan", "- prototype", Some("game studio"), None, &[]).unwrap().contains("Game studio"));

        // The edit and the new note are among the changes the Drive sync uploads next.
        let next = s.vault_changes_since(cloud.id, &uploaded).unwrap();
        let titles: Vec<String> = next
            .rows
            .iter()
            .filter(|r| r.table == "notes")
            .filter_map(|r| match r.values.get("title") {
                Some(crate::sync::SqlValue::Text(t)) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(titles.contains(&"Pitch".to_string()) && titles.contains(&"Plan".to_string()), "{titles:?}");
    }

    #[test]
    fn folders_and_notes_move_around_and_between_vaults() {
        let s = store();
        s.agent_folder_create("Games/Ideas", None).unwrap();
        s.agent_move_note("Game idea", Some("Games/Ideas"), None).unwrap();
        let tree = s.agent_folders(None).unwrap();
        assert!(tree.contains("Games (0 notes)") && tree.contains("  Ideas (1 note)"), "{tree}");
        // Out of a folder again.
        let out = s.agent_move_note("Game idea", Some("/"), None).unwrap();
        assert!(out.ends_with("to Personal."), "{out}");
        s.agent_move_note("Game idea", Some("Ideas"), None).unwrap();

        // A folder into another, then renamed.
        s.agent_folder_create("Archive", None).unwrap();
        s.agent_move_folder("Games", None, Some("Archive"), None).unwrap();
        s.agent_folder_rename("Archive/Games", "Old games", None).unwrap();
        assert!(s.agent_read("Game idea").unwrap().contains("Personal/Archive/Old games/Ideas"));

        // The whole folder to another vault: its folders come along, the notes move, the
        // emptied folders here go.
        let moved = s.agent_move_folder("Old games", None, Some("Projects"), Some("Work")).unwrap();
        assert!(moved.contains("Work/Projects/Old games"), "{moved}");
        assert!(s.agent_read("Game idea").unwrap().contains("Work/Projects/Old games/Ideas"));
        assert!(!s.agent_folders(None).unwrap().contains("Old games"));
        // A single note to another vault (its top level unless a folder is given).
        assert!(s.agent_move_note("Groceries", None, Some("Work")).unwrap().ends_with("to Work."));
        // A folder can't go into itself; deleting one keeps what's in it.
        assert!(s.agent_move_folder("Projects", Some("Work"), Some("Projects/Old games"), None).is_err());
        s.agent_folder_delete("Old games", Some("Work")).unwrap();
        assert!(s.agent_read("Game idea").unwrap().contains("Work/Projects/Ideas"));
    }

    #[test]
    fn deleting_for_good_takes_two_steps() {
        let s = store();
        assert!(s.agent_delete("Groceries", true).unwrap_err().to_string().contains("archived"));
        assert!(s.agent_delete("Groceries", false).unwrap().contains("Archived"));
        assert!(s.agent_delete("Groceries", true).unwrap().contains("for good"));
        assert!(s.agent_read("Groceries").is_err());
    }

    #[test]
    fn access_defaults_to_edit_and_can_be_limited() {
        let s = store();
        assert_eq!(s.agent_access().unwrap(), AgentAccess::Edit);
        s.set_agent_access(AgentAccess::Read).unwrap();
        assert_eq!(s.agent_access().unwrap(), AgentAccess::Read);
    }
}
