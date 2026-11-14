//! The Brain: a memory bank for AI coding agents (Claude Code, Codex, Cursor…).
//!
//! The goal is fewer tokens: an agent asks the Brain for exactly what it needs instead of
//! re-reading chats and files. So everything here is about returning the smallest useful text:
//! - a tiny **index** (one line per memory) to start a session with,
//! - **recall** by meaning that returns only the best few memories, trimmed to the matching lines,
//! - a **code map** (file → purpose) that says which files changed since they were described,
//! - **tidy** reports (duplicates, stale, maybe-conflicting) to keep the Brain small.
//!
//! Memories are ordinary notes in a "Brain" vault: one folder per project plus "Me" for things
//! that apply everywhere. A small front-matter header holds the memory's type, its one-line
//! hook, who wrote it and whether you've reviewed it — readable and editable like any note, and
//! mirrored to Markdown files (see [`Store::export_brain`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ai::embed::{LexicalEmbedder, dot};
use crate::ai::text::{content_words, sentences, stem};
use crate::error::{Error, Result};
use crate::notes::{ArchiveFilter, ListFilter, NewNote, Note, NotePatch};
use crate::store::Store;
use crate::util::now_ms;

pub const BRAIN_VAULT_NAME: &str = "Brain";
pub const ME_FOLDER: &str = "Me";
pub(crate) const VAULT_KEY: &str = "brain.vault";
const PROJECTS_KEY: &str = "brain.projects";
const STATS_KEY: &str = "brain.stats";
const MIRROR_KEY: &str = "brain.mirror";
const CODE_MAP_TITLE: &str = "Code map";
/// Index lines shown before "+N more".
const INDEX_LIMIT: usize = 40;
/// A memory nobody has recalled in this long, and that hasn't been touched, drops out of the index
/// (it's still found by searching).
const STALE_DAYS: i64 = 60;
/// The one memory per project that says where the last session stopped and what comes next.
pub const HANDOFF_HOOK: &str = "Where we left off";
const MAX_HANDOFF: usize = 1400;
/// Longest details for one memory: one idea per memory keeps the Brain small.
const MAX_DETAILS: usize = 1500;
/// A project with more memories than this gets its exact duplicates merged now and then.
const HOUSEKEEPING_OVER: usize = 25;

/// One standing instruction, in every index: the point of the Brain.
const RULE: &str = "Use the Brain, not old chats, transcripts or logs, for anything decided or learned before; if it isn't here, ask the user or read the code. Save durable findings with brain_remember as you go.";

pub(crate) const BRAIN_MIGRATION: &str = "
    CREATE TABLE brain_recalls (
        note_id BLOB PRIMARY KEY, last_at INTEGER NOT NULL, count INTEGER NOT NULL
    ) WITHOUT ROWID;
";

/// What a memory is about. Agents pick one; the index groups by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    /// How things are decided / constraints ("we use X because Y").
    Decision,
    /// How the user likes things done.
    Preference,
    /// Something surprising that wastes time if forgotten.
    Gotcha,
    /// How to do a recurring task (commands, steps).
    Howto,
    /// A fact about the project or its world.
    Fact,
    /// Work in progress / what's next.
    Context,
    /// A compressed summary of older memories.
    Summary,
    /// The project's file map (one per project).
    Codemap,
}

impl MemoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Decision => "decision",
            MemoryKind::Preference => "preference",
            MemoryKind::Gotcha => "gotcha",
            MemoryKind::Howto => "howto",
            MemoryKind::Fact => "fact",
            MemoryKind::Context => "context",
            MemoryKind::Summary => "summary",
            MemoryKind::Codemap => "codemap",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "decision" | "decisions" => MemoryKind::Decision,
            "preference" | "preferences" | "pref" => MemoryKind::Preference,
            "gotcha" | "gotchas" | "pitfall" => MemoryKind::Gotcha,
            "howto" | "how-to" | "how to" | "procedure" => MemoryKind::Howto,
            "fact" | "facts" | "note" | "reference" => MemoryKind::Fact,
            "context" | "wip" | "status" | "todo" => MemoryKind::Context,
            "summary" => MemoryKind::Summary,
            "codemap" | "code map" | "map" => MemoryKind::Codemap,
            _ => return None,
        })
    }
    fn heading(self) -> &'static str {
        match self {
            MemoryKind::Decision => "Decisions",
            MemoryKind::Preference => "Preferences",
            MemoryKind::Gotcha => "Gotchas",
            MemoryKind::Howto => "How-tos",
            MemoryKind::Fact => "Facts",
            MemoryKind::Context => "Current work",
            MemoryKind::Summary => "Summaries",
            MemoryKind::Codemap => "Code map",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: Uuid,
    /// Short handle agents use ("a1b2c3").
    pub short: String,
    /// The project folder it lives in.
    pub folder: Uuid,
    pub project: String,
    pub kind: MemoryKind,
    /// One line: what this memory says (shown in the index).
    pub hook: String,
    /// The details (without the header).
    pub content: String,
    /// Who wrote it ("claude-code", "codex", "you"…).
    pub source: String,
    /// Not yet reviewed by you.
    pub new: bool,
    pub pinned: bool,
    pub updated_at: i64,
    pub recalled_at: Option<i64>,
    pub recall_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainProject {
    pub folder: Uuid,
    pub name: String,
    pub count: usize,
    pub new_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainStats {
    pub recalls: u64,
    /// Estimated tokens the Brain returned to agents.
    pub tokens_served: u64,
    /// Estimated tokens avoided vs. loading every memory each time. (Generous: nobody loads them
    /// all, so read it as an upper bound.)
    pub tokens_saved: u64,
    /// Searches that found something, and searches that found nothing.
    #[serde(default)]
    pub hits: u64,
    #[serde(default)]
    pub misses: u64,
    /// How many memories there are, and how many have been recalled at least once.
    #[serde(default)]
    pub memories: u64,
    #[serde(default)]
    pub used: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TidyPair {
    pub a: Uuid,
    pub b: Uuid,
    pub a_hook: String,
    pub b_hook: String,
    pub score: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TidyReport {
    /// Nearly the same: merge them.
    pub duplicates: Vec<TidyPair>,
    /// Same subject, different words: check they still agree.
    pub similar: Vec<TidyPair>,
    /// Not touched or recalled in a long time: archive or compress.
    pub stale: Vec<Uuid>,
}

/// The Brain as a network: memories, and links between related ones.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainGraph {
    pub nodes: Vec<Memory>,
    pub links: Vec<BrainLink>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainLink {
    pub a: Uuid,
    pub b: Uuid,
    /// How related (0–1).
    pub strength: f32,
}

/// One code map line: a file and what it's for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapEntry {
    pub path: String,
    pub purpose: String,
    /// The file's modification time (seconds) when it was described.
    pub mtime: Option<u64>,
}

// ---- header (front matter) -------------------------------------------------------------

struct Header {
    kind: Option<MemoryKind>,
    hook: Option<String>,
    source: Option<String>,
    status: Option<String>,
}

fn split_header(body: &str) -> (Header, &str) {
    let mut h = Header { kind: None, hook: None, source: None, status: None };
    let Some(rest) = body.strip_prefix("---\n") else { return (h, body) };
    let Some(end) = rest.find("\n---") else { return (h, body) };
    for line in rest[..end].lines() {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim().trim_matches('"').to_string();
            match k.trim() {
                "type" => h.kind = MemoryKind::parse(&v),
                "hook" => h.hook = Some(v),
                "source" => h.source = Some(v),
                "status" => h.status = Some(v),
                _ => {}
            }
        }
    }
    let after = &rest[end + 4..];
    (h, after.strip_prefix('\n').unwrap_or(after).trim_start_matches('\n'))
}

fn one_line(s: &str, max: usize) -> String {
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        return s;
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

fn render(kind: MemoryKind, hook: &str, source: &str, new: bool, content: &str) -> String {
    format!(
        "---\ntype: {}\nhook: {}\nsource: {}\nstatus: {}\n---\n{}",
        kind.as_str(),
        one_line(hook, 200),
        one_line(source, 40),
        if new { "new" } else { "reviewed" },
        content.trim()
    )
}

/// The search vectors of one memory, kept while its text stays the same.
pub(crate) struct MemVec {
    updated: i64,
    whole: Vec<(u32, f32)>,
    hook: Vec<(u32, f32)>,
    words: HashSet<String>,
}

/// Words that mean nearly the same thing when developers write them down, so "how do I build it"
/// finds a memory that says "compile". Each group is matched on word stems.
const SYNONYMS: &[&[&str]] = &[
    &["build", "compile", "bundle", "compilation"],
    &["run", "start", "launch", "execute"],
    &["delete", "remove", "erase", "drop"],
    &["error", "fail", "failure", "crash", "bug", "broken", "exception", "panic"],
    &["config", "configuration", "settings", "options", "setup"],
    &["test", "spec", "check"],
    &["install", "dependency", "dependencies", "package"],
    &["fix", "repair", "resolve", "workaround", "patch"],
    &["deploy", "release", "publish", "ship"],
    &["login", "auth", "authentication", "signin", "credentials"],
    &["slow", "performance", "lag", "latency", "speed"],
    &["database", "db", "sqlite", "sql", "storage"],
    &["command", "cli", "script", "terminal"],
    &["folder", "directory", "path"],
    &["decide", "decision", "choose", "chose", "because"],
    &["secret", "password", "token", "key"],
];

/// For each word of a query (as a stem): itself and the stems of words that mean the same.
fn query_groups(query: &str) -> Vec<Vec<String>> {
    static MAP: std::sync::OnceLock<HashMap<String, Vec<String>>> = std::sync::OnceLock::new();
    let map = MAP.get_or_init(|| {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for group in SYNONYMS {
            let stems: Vec<String> = group.iter().map(|w| stem(w)).collect();
            for s in &stems {
                map.entry(s.clone()).or_default().extend(stems.iter().filter(|o| *o != s).cloned());
            }
        }
        map
    });
    let mut seen = HashSet::new();
    let mut groups = Vec::new();
    for t in content_words(query).iter().map(|w| stem(w)) {
        if !seen.insert(t.clone()) {
            continue;
        }
        let mut g = vec![t.clone()];
        g.extend(map.get(&t).cloned().unwrap_or_default());
        groups.push(g);
    }
    groups
}

/// "2 hours ago", "3 days ago".
fn ago(ms: i64) -> String {
    let mins = (now_ms() - ms).max(0) / 60_000;
    match mins {
        0..=1 => "just now".into(),
        2..=59 => format!("{mins} min ago"),
        60..=1439 => format!("{} h ago", mins / 60),
        _ => format!("{} days ago", mins / 1440),
    }
}

/// Whether text looks like it holds a secret (an API key, token, private key or password). The Brain
/// keeps plain-text copies (mirrored to Markdown files, possibly synced), so such things are
/// refused: say where the secret lives instead. Returns what it looks like.
pub fn looks_like_secret(text: &str) -> Option<&'static str> {
    let is_token = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let run_after = |word: &str, min: usize| -> bool {
        let mut from = 0;
        while let Some(i) = text[from..].find(word) {
            let rest = &text[from + i + word.len()..];
            if rest.chars().take_while(|c| is_token(*c)).count() >= min {
                return true;
            }
            from += i + word.len();
        }
        false
    };
    if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY") {
        return Some("a private key");
    }
    if run_after("sk-", 20) || run_after("ghp_", 20) || run_after("gho_", 20) || run_after("ghs_", 20) || run_after("github_pat_", 20) || run_after("AIza", 30) || run_after("xoxb-", 10) || run_after("xoxp-", 10) {
        return Some("an API key or token");
    }
    // AWS access key ids: AKIA + 16 capitals/digits.
    let mut from = 0;
    while let Some(i) = text[from..].find("AKIA") {
        let rest = &text[from + i + 4..];
        if rest.chars().take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit()).count() >= 16 {
            return Some("an AWS access key");
        }
        from += i + 4;
    }
    // JSON web tokens: eyJ… . … . …
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')')) {
        let parts: Vec<&str> = word.split('.').collect();
        if parts.len() == 3 && parts[0].starts_with("eyJ") && parts.iter().all(|p| p.len() >= 10 && p.chars().all(is_token)) {
            return Some("a login token (JWT)");
        }
    }
    let lower = text.to_lowercase();
    for key in ["password", "passwd", "secret", "api_key", "api-key", "apikey", "access_token", "auth_token", "token", "bearer"] {
        let mut from = 0;
        while let Some(i) = lower[from..].find(key) {
            let after = lower[from + i + key.len()..].trim_start_matches(['"', '\'', ' ', '`']);
            let after = after.strip_prefix(['=', ':']).or_else(|| (key == "bearer").then_some(after));
            if let Some(v) = after {
                let v = v.trim_start_matches([' ', '"', '\'', '`']);
                let value: String = v.chars().take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | '`' | ',' | ';')).collect();
                let placeholder = value.starts_with(['<', '$', '{', '*', '[', '%', '(']) || value.contains("...") || value.contains("xxx") || value.contains("your") || value.contains("example") || value.contains("changeme") || value.contains("placeholder");
                if value.chars().count() >= 8 && !placeholder && value.chars().any(|c| c.is_ascii_digit()) {
                    return Some("a password or secret");
                }
            }
            from += i + key.len();
        }
    }
    None
}

fn refuse_secrets(parts: &[&str]) -> Result<()> {
    for p in parts {
        if let Some(what) = looks_like_secret(p) {
            return Err(Error::invalid(format!(
                "this looks like it contains {what}. The Brain keeps plain-text copies, so it won't save secrets: say where it lives instead (for example \"the key is in ~/.config/app/credentials\")"
            )));
        }
    }
    Ok(())
}

/// Paths that a memory mentions which no longer exist in the project: a path whose first folder is
/// in the project but which isn't there (so it was moved or deleted). Paths relative to some other
/// folder aren't flagged.
fn missing_paths(root: &Path, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | ',' | ';' | '[' | ']' | '<' | '>')) {
        let p = raw.trim_matches(|c: char| matches!(c, '.' | ':' | '*')).trim_start_matches("./");
        let Some((first, _)) = p.split_once('/') else { continue };
        let name = p.rsplit('/').next().unwrap_or("");
        let ext_ok = name.rsplit_once('.').is_some_and(|(stem, e)| !stem.is_empty() && (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()));
        if !ext_ok || p.contains("://") || p.contains('*') || p.contains('{') || p.starts_with(['~', '/', '$']) || first.is_empty() || first.contains('.') {
            continue;
        }
        if root.join(first).is_dir() && !root.join(p).exists() && !out.iter().any(|o| o == p) {
            out.push(p.to_string());
        }
    }
    out
}

fn short_of(id: Uuid) -> String {
    let s = id.simple().to_string();
    s[s.len() - 6..].to_string()
}

/// Rough token count (≈ 4 characters per token).
pub fn estimate_tokens(s: &str) -> u64 {
    (s.len() as u64).div_ceil(4)
}

// ---- ranking ----------------------------------------------------------------------------

struct Ranker {
    e: LexicalEmbedder,
}

impl Ranker {
    fn new() -> Self {
        Self { e: LexicalEmbedder::default() }
    }
    fn vec(&self, s: &str) -> Vec<f32> {
        self.e.embed_one(s)
    }
    /// The same vector keeping only its non-zero entries (most are zero): comparing two of
    /// these is far cheaper, which matters when every memory is compared with many others.
    fn sparse(&self, s: &str) -> Vec<(u32, f32)> {
        self.e.embed_one(s).into_iter().enumerate().filter(|(_, v)| *v != 0.0).map(|(i, v)| (i as u32, v)).collect()
    }
}

/// Dot product of two sparse vectors (entries sorted by index).
fn sdot(a: &[(u32, f32)], b: &[(u32, f32)]) -> f32 {
    let (mut i, mut j, mut sum) = (0, 0, 0.0);
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                sum += a[i].1 * b[j].1;
                i += 1;
                j += 1;
            }
        }
    }
    sum
}

fn stems(s: &str) -> HashSet<String> {
    content_words(s).iter().map(|w| stem(w)).collect()
}

impl Store {
    // ---- the Brain vault and its projects ----

    /// The Brain vault (created plain on first use).
    pub fn brain_vault(&mut self) -> Result<Uuid> {
        if let Some(id) = self.get_setting(VAULT_KEY)?.and_then(|s| s.parse::<Uuid>().ok())
            && self.get_vault(id).is_ok()
        {
            return Ok(id);
        }
        let existing = self.list_vaults()?.into_iter().find(|v| v.name == BRAIN_VAULT_NAME && !v.encrypted && v.owner.is_none());
        let id = match existing {
            Some(v) => v.id,
            None => self.create_vault(BRAIN_VAULT_NAME, None)?.id,
        };
        self.set_setting(VAULT_KEY, &id.to_string())?;
        Ok(id)
    }

    /// Uses another vault as the Brain (e.g. one shared from your other device, so the memories
    /// carry over). Memories already in the old Brain vault stay there.
    pub fn set_brain_vault(&mut self, id: Uuid) -> Result<()> {
        let v = self.get_vault(id)?;
        if v.encrypted {
            return Err(Error::Invalid("the Brain can't live in an encrypted vault".into()));
        }
        self.set_setting(VAULT_KEY, &id.to_string())
    }

    fn folder_named(&self, vault: Uuid, name: &str) -> Result<Option<Uuid>> {
        Ok(self.list_folders(Some(vault))?.into_iter().find(|f| f.parent_id.is_none() && f.name == name).map(|f| f.id))
    }

    /// The "Me" folder: preferences that apply to every project.
    pub fn brain_me(&mut self) -> Result<Uuid> {
        let vault = self.brain_vault()?;
        match self.folder_named(vault, ME_FOLDER)? {
            Some(f) => Ok(f),
            None => Ok(self.create_folder(Some(vault), None, ME_FOLDER)?.id),
        }
    }

    /// The folder for a project, found by a stable key (a git remote, or the repo's path);
    /// created on first use with a readable name.
    pub fn brain_project(&mut self, key: &str, name: &str) -> Result<Uuid> {
        let vault = self.brain_vault()?;
        let mut map: BTreeMap<String, String> =
            self.get_setting(PROJECTS_KEY)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        if let Some(id) = map.get(key).and_then(|s| s.parse::<Uuid>().ok())
            && self.list_folders(Some(vault))?.iter().any(|f| f.id == id)
        {
            return Ok(id);
        }
        let name = one_line(name, 60);
        let name = if name.is_empty() || name == ME_FOLDER { "Project".to_string() } else { name };
        let folder = match self.folder_named(vault, &name)? {
            // A folder with that name but no recorded key (made by hand): adopt it.
            Some(f) if !map.values().any(|v| v == &f.to_string()) => f,
            Some(_) => self.create_folder(Some(vault), None, &format!("{name} ({})", &short_of(Uuid::now_v7())[..4]))?.id,
            None => self.create_folder(Some(vault), None, &name)?.id,
        };
        map.insert(key.to_string(), folder.to_string());
        self.set_setting(PROJECTS_KEY, &serde_json::to_string(&map)?)?;
        Ok(folder)
    }

    pub fn brain_projects(&mut self) -> Result<Vec<BrainProject>> {
        let vault = self.brain_vault()?;
        let all = self.brain_memories_in(vault, None)?;
        let mut out: Vec<BrainProject> = self
            .list_folders(Some(vault))?
            .into_iter()
            .filter(|f| f.parent_id.is_none())
            .map(|f| {
                let mine: Vec<&Memory> = all.iter().filter(|m| m.folder == f.id).collect();
                BrainProject { folder: f.id, name: f.name, count: mine.len(), new_count: mine.iter().filter(|m| m.new).count() }
            })
            .collect();
        out.sort_by(|a, b| (a.name != ME_FOLDER).cmp(&(b.name != ME_FOLDER)).then(a.name.cmp(&b.name)));
        Ok(out)
    }

    // ---- memories ----

    fn to_memory(&self, n: Note, folder_name: &str, recalls: &BTreeMap<Uuid, (i64, i64)>) -> Option<Memory> {
        let folder = n.folder_id?;
        let (h, content) = split_header(&n.body);
        let kind = h.kind.unwrap_or(if n.title == CODE_MAP_TITLE { MemoryKind::Codemap } else { MemoryKind::Fact });
        let hook = h.hook.filter(|s| !s.is_empty()).unwrap_or_else(|| one_line(&n.title, 200));
        let (recalled_at, recall_count) = recalls.get(&n.id).map(|(a, c)| (Some(*a), *c)).unwrap_or((None, 0));
        Some(Memory {
            id: n.id,
            short: short_of(n.id),
            folder,
            project: folder_name.to_string(),
            kind,
            hook,
            content: content.to_string(),
            source: h.source.unwrap_or_else(|| "you".into()),
            new: h.status.as_deref() == Some("new"),
            pinned: n.pinned,
            updated_at: n.updated_at,
            recalled_at,
            recall_count,
        })
    }

    fn recall_stats(&self) -> Result<BTreeMap<Uuid, (i64, i64)>> {
        let mut stmt = self.conn.prepare("SELECT note_id, last_at, count FROM brain_recalls")?;
        let rows = stmt.query_map([], |r| Ok((crate::util::uuid_col(r, 0)?, r.get(1)?, r.get(2)?)))?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (id, a, c) = row?;
            out.insert(id, (a, c));
        }
        Ok(out)
    }

    fn brain_memories_in(&self, vault: Uuid, folders: Option<&[Uuid]>) -> Result<Vec<Memory>> {
        let names: BTreeMap<Uuid, String> = self.list_folders(Some(vault))?.into_iter().map(|f| (f.id, f.name)).collect();
        let recalls = self.recall_stats()?;
        let notes = self.list_notes(&ListFilter { vault_id: Some(vault), archive: ArchiveFilter::Active, ..Default::default() })?;
        Ok(notes
            .into_iter()
            .filter(|n| n.folder_id.is_some_and(|f| folders.is_none_or(|fs| fs.contains(&f))))
            .filter_map(|n| {
                let name = names.get(&n.folder_id?)?.clone();
                self.to_memory(n, &name, &recalls)
            })
            .collect())
    }

    /// Memories in a project folder (or every project with `None`), newest first.
    pub fn brain_memories(&mut self, folder: Option<Uuid>) -> Result<Vec<Memory>> {
        let vault = self.brain_vault()?;
        let mut m = self.brain_memories_in(vault, folder.as_ref().map(std::slice::from_ref))?;
        m.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_at.cmp(&a.updated_at)));
        Ok(m)
    }

    /// Finds a memory by its short handle or full id.
    pub fn brain_memory(&mut self, handle: &str) -> Result<Memory> {
        let handle = handle.trim().trim_matches(['[', ']']).to_ascii_lowercase();
        let all = self.brain_memories(None)?;
        if let Ok(id) = handle.parse::<Uuid>() {
            return all.into_iter().find(|m| m.id == id).ok_or(Error::NotFound("memory", id));
        }
        let mut hits = all.into_iter().filter(|m| !handle.is_empty() && m.short.ends_with(&handle));
        match (hits.next(), hits.next()) {
            (Some(m), None) => Ok(m),
            (Some(_), Some(_)) => Err(Error::invalid(format!("“{handle}” matches several memories; use more characters"))),
            _ => Err(Error::invalid(format!("no memory “{handle}”"))),
        }
    }

    /// Saves a memory. If one in the same project already says (nearly) the same thing, that one
    /// is updated instead. Returns the memory and whether it was merged into an existing one.
    pub fn remember(&mut self, folder: Uuid, kind: MemoryKind, hook: &str, content: &str, source: &str) -> Result<(Memory, bool)> {
        let hook = one_line(hook, 200);
        if hook.is_empty() {
            return Err(Error::invalid("a memory needs a one-line hook"));
        }
        refuse_secrets(&[&hook, content])?;
        if content.chars().count() > MAX_DETAILS {
            return Err(Error::invalid(format!("the details are {} characters; keep one memory under {MAX_DETAILS} (one idea per memory): shorten it, or split it into several", content.chars().count())));
        }
        let vault = self.brain_vault()?;
        // A busy project tidies its exact duplicates now and then.
        let _ = self.brain_housekeeping(folder);
        let r = Ranker::new();
        let incoming = r.sparse(&format!("{hook}\n{content}"));
        let incoming_hook = r.sparse(&hook);
        let existing = self.brain_memories_in(vault, Some(&[folder]))?;
        // The same point (nearly identical hook), or the same thing said overall.
        let twin = existing
            .iter()
            .filter(|m| m.kind != MemoryKind::Codemap && kind != MemoryKind::Codemap)
            .map(|m| {
                let whole = sdot(&incoming, &r.sparse(&format!("{}\n{}", m.hook, m.content)));
                let hooks = sdot(&incoming_hook, &r.sparse(&m.hook));
                (m, if hooks >= 0.92 { hooks } else { whole })
            })
            .filter(|(_, s)| *s >= 0.86)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((m, _)) = twin {
            let merged = merge_text(&m.content, content);
            let id = m.id;
            self.update_note(id, NotePatch { title: Some(hook.clone()), body: Some(render(kind, &hook, source, true, &merged)) })?;
            return Ok((self.brain_memory(&id.to_string())?, true));
        }
        let note = self.create_note(NewNote {
            title: hook.clone(),
            body: render(kind, &hook, source, source != "you", content),
            vault_id: Some(vault),
            folder_id: Some(folder),
            ..Default::default()
        })?;
        Ok((self.brain_memory(&note.id.to_string())?, false))
    }

    /// Changes a memory's hook, content or type (keeps the rest).
    pub fn update_memory(&mut self, handle: &str, hook: Option<&str>, content: Option<&str>, kind: Option<MemoryKind>, source: &str) -> Result<Memory> {
        refuse_secrets(&[hook.unwrap_or(""), content.unwrap_or("")])?;
        let m = self.brain_memory(handle)?;
        let hook = hook.map(|h| one_line(h, 200)).filter(|h| !h.is_empty()).unwrap_or(m.hook.clone());
        let kind = kind.unwrap_or(m.kind);
        let content = content.unwrap_or(&m.content);
        let new = source != "you" || m.new;
        self.update_note(m.id, NotePatch { title: Some(hook.clone()), body: Some(render(kind, &hook, &m.source, new, content)) })?;
        self.brain_memory(&m.id.to_string())
    }

    /// Marks a memory as reviewed (or new again).
    pub fn set_memory_reviewed(&mut self, handle: &str, reviewed: bool) -> Result<Memory> {
        let m = self.brain_memory(handle)?;
        self.update_note(m.id, NotePatch { body: Some(render(m.kind, &m.hook, &m.source, !reviewed, &m.content)), ..Default::default() })?;
        self.brain_memory(&m.id.to_string())
    }

    /// Forgets a memory (archived, so it can be restored from the archive).
    pub fn forget(&mut self, handle: &str) -> Result<Memory> {
        let m = self.brain_memory(handle)?;
        self.archive_note(m.id)?;
        Ok(m)
    }

    fn note_recalls(&self, ids: &[Uuid]) -> Result<()> {
        let now = now_ms();
        for id in ids {
            self.conn.execute(
                "INSERT INTO brain_recalls(note_id, last_at, count) VALUES (?1, ?2, 1)
                 ON CONFLICT(note_id) DO UPDATE SET last_at = excluded.last_at, count = count + 1",
                params![id.as_bytes(), now],
            )?;
        }
        Ok(())
    }

    pub fn brain_stats(&self) -> Result<BrainStats> {
        Ok(self.get_setting(STATS_KEY)?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
    }

    /// `found`: for a search, whether it found anything (counted as a hit or a miss).
    fn count_served(&self, served: &str, whole: u64, found: Option<bool>) -> Result<()> {
        let mut s = self.brain_stats()?;
        let t = estimate_tokens(served);
        s.recalls += 1;
        match found {
            Some(true) => s.hits += 1,
            Some(false) => s.misses += 1,
            None => {}
        }
        s.tokens_served += t;
        s.tokens_saved += whole.saturating_sub(t);
        self.set_setting(STATS_KEY, &serde_json::to_string(&s)?)
    }

    // ---- what agents read ----

    /// The session-start index: one line per memory (project + Me), grouped by type, with
    /// handles for details. Small on purpose.
    pub fn brain_context(&mut self, folder: Uuid, root: Option<&Path>) -> Result<String> {
        self.brain_context_for(folder, root, None)
    }

    /// The same index, ordered for `task` (what's about to be worked on) when it's given: the
    /// memories most relevant to it come first, so the short list holds what's needed.
    pub fn brain_context_for(&mut self, folder: Uuid, root: Option<&Path>, task: Option<&str>) -> Result<String> {
        let me = self.brain_me()?;
        let vault = self.brain_vault()?;
        let project_name = self.list_folders(Some(vault))?.into_iter().find(|f| f.id == folder).map(|f| f.name).unwrap_or_default();
        let mut mine = self.brain_memories_in(vault, Some(&[folder]))?;
        let everywhere = if folder == me { vec![] } else { self.brain_memories_in(vault, Some(&[me]))? };
        let map = mine.iter().find(|m| m.kind == MemoryKind::Codemap).cloned();
        mine.retain(|m| m.kind != MemoryKind::Codemap);
        let handoff = mine.iter().find(|m| m.hook == HANDOFF_HOOK).cloned();
        mine.retain(|m| m.hook != HANDOFF_HOOK);
        let total = mine.len();
        // Old, never-used memories stay searchable but don't take room in the index.
        let now = now_ms();
        let stale = |m: &Memory| !m.pinned && m.recall_count == 0 && now - m.updated_at > STALE_DAYS * 86_400_000;
        mine.retain(|m| !stale(m));
        // Most useful first: pinned, then (given a task) the most relevant, then often recalled, recent.
        let task = task.map(str::trim).filter(|t| !stems(t).is_empty());
        let mut relevance: HashMap<Uuid, f32> = HashMap::new();
        if let Some(t) = task {
            let r = Ranker::new();
            self.warm_vecs(&r, &mine);
            let (qs, groups) = (r.sparse(t), query_groups(t));
            for m in &mine {
                relevance.insert(m.id, self.relevance(&qs, &groups, m));
            }
        }
        let rel = |m: &Memory| relevance.get(&m.id).copied().unwrap_or(0.0);
        mine.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(rel(b).total_cmp(&rel(a)))
                .then(b.recall_count.cmp(&a.recall_count))
                .then(b.updated_at.cmp(&a.updated_at))
        });
        let shown: Vec<&Memory> = mine.iter().take(INDEX_LIMIT).collect();
        let mut out = format!(
            "Brain · {project_name}: {total} memories. Handles in [brackets] — brain_read for details, brain_recall to search.\n{RULE}\n"
        );
        if let Some(h) = &handoff {
            let old = now_ms() - h.updated_at > 14 * 86_400_000;
            out.push_str(&format!(
                "Where we left off ({}{}):\n{}\n",
                ago(h.updated_at),
                if old { " — old, check it still holds" } else { "" },
                h.content.trim()
            ));
        } else {
            out.push_str("No hand-off note yet: before you stop, save where you are and what's next with brain_handoff.\n");
        }
        if total == 0 {
            out.push_str("Nothing remembered yet for this project. Save what you learn with brain_remember.\n");
        }
        let mut groups: BTreeMap<MemoryKind, Vec<&Memory>> = BTreeMap::new();
        for m in &shown {
            groups.entry(m.kind).or_default().push(m);
        }
        for (kind, list) in groups {
            out.push_str(&format!("{}:\n", kind.heading()));
            for m in list {
                let gone = root.map(|r| missing_paths(r, &format!("{} {}", m.hook, m.content))).unwrap_or_default();
                let flag = gone.first().map(|p| format!(" ⚠ {p} is gone")).unwrap_or_default();
                out.push_str(&format!("- [{}] {}{}{}\n", m.short, if m.pinned { "★ " } else { "" }, m.hook, flag));
            }
        }
        if total > shown.len() {
            out.push_str(&format!("(+{} more — brain_recall finds them)\n", total - shown.len()));
        }
        if !everywhere.is_empty() {
            out.push_str("Me (applies everywhere):\n");
            for m in everywhere.iter().take(25) {
                out.push_str(&format!("- [{}] {}\n", m.short, m.hook));
            }
        }
        if let Some(map) = map {
            let entries = parse_map(&map.content);
            let changed = root.map(|r| entries.iter().filter(|e| map_status(r, e).is_some()).count()).unwrap_or(0);
            out.push_str(&format!(
                "Code map: {} files described{} — brain_map to look up where things are.\n",
                entries.len(),
                if changed > 0 { format!(", {changed} changed or gone since") } else { String::new() }
            ));
        } else {
            out.push_str("No code map yet — as you learn what files do, save them with brain_map_note.\n");
        }
        let whole: u64 = mine.iter().chain(everywhere.iter()).map(|m| estimate_tokens(&m.content) + estimate_tokens(&m.hook)).sum();
        self.count_served(&out, whole, None)?;
        Ok(out)
    }

    // ---- hand-off: where the last session stopped ----

    /// The project's hand-off note, if any.
    pub fn brain_handoff(&mut self, folder: Uuid) -> Result<Option<Memory>> {
        let vault = self.brain_vault()?;
        Ok(self.brain_memories_in(vault, Some(&[folder]))?.into_iter().find(|m| m.hook == HANDOFF_HOOK))
    }

    /// Replaces the project's hand-off note (there's only ever one): what's in progress, what comes
    /// next. A new session reads it from the index instead of reading the last chat.
    pub fn brain_handoff_save(&mut self, folder: Uuid, text: &str, source: &str) -> Result<Memory> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::invalid("say where things stand and what comes next"));
        }
        refuse_secrets(&[text])?;
        if text.chars().count() > MAX_HANDOFF {
            return Err(Error::invalid(format!("the hand-off is {} characters; keep it under {MAX_HANDOFF}: what's in progress, the next step, anything half-done", text.chars().count())));
        }
        match self.brain_handoff(folder)? {
            Some(m) => {
                self.update_note(m.id, NotePatch { title: Some(HANDOFF_HOOK.into()), body: Some(render(MemoryKind::Context, HANDOFF_HOOK, source, m.new && source != "you", text)) })?;
                self.brain_memory(&m.id.to_string())
            }
            None => {
                let vault = self.brain_vault()?;
                let note = self.create_note(NewNote {
                    title: HANDOFF_HOOK.into(),
                    body: render(MemoryKind::Context, HANDOFF_HOOK, source, false, text),
                    vault_id: Some(vault),
                    folder_id: Some(folder),
                    ..Default::default()
                })?;
                self.brain_memory(&note.id.to_string())
            }
        }
    }

    // ---- keeping it small ----

    /// Memories that overlap a new one without being the same (so the agent can check whether it
    /// replaces or contradicts one): similar enough to be about the same thing, not so similar that
    /// saving merged them.
    pub fn brain_similar(&mut self, folder: Uuid, hook: &str, content: &str, skip: Option<Uuid>) -> Result<Vec<Memory>> {
        let me = self.brain_me()?;
        let vault = self.brain_vault()?;
        let pool: Vec<Memory> = self
            .brain_memories_in(vault, Some(&[folder, me]))?
            .into_iter()
            .filter(|m| m.kind != MemoryKind::Codemap && m.hook != HANDOFF_HOOK && Some(m.id) != skip)
            .collect();
        let r = Ranker::new();
        self.warm_vecs(&r, &pool);
        let incoming = r.sparse(&format!("{hook}\n{content}"));
        let mut scored: Vec<(f32, Memory)> = pool
            .into_iter()
            .filter_map(|m| {
                let s = self.brain_vecs.get(&m.id).map(|v| sdot(&incoming, &v.whole))?;
                (0.45..0.86).contains(&s).then_some((s, m))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        Ok(scored.into_iter().take(2).map(|(_, m)| m).collect())
    }

    /// Merges a project's near-identical memories (at most once a day, and only in projects with
    /// plenty of them), keeping the newer one. Returns how many were merged.
    pub fn brain_housekeeping(&mut self, folder: Uuid) -> Result<usize> {
        let vault = self.brain_vault()?;
        if self.brain_memories_in(vault, Some(&[folder]))?.len() <= HOUSEKEEPING_OVER {
            return Ok(0);
        }
        let key = format!("brain.housekeeping.{folder}");
        let now = now_ms();
        if self.get_setting(&key)?.and_then(|s| s.parse::<i64>().ok()).is_some_and(|t| now - t < 86_400_000) {
            return Ok(0);
        }
        self.set_setting(&key, &now.to_string())?;
        let report = self.brain_tidy(Some(folder))?;
        let mut merged = 0;
        for p in report.duplicates.iter().filter(|p| p.score >= 0.9) {
            let (Ok(a), Ok(b)) = (self.brain_memory(&p.a.to_string()), self.brain_memory(&p.b.to_string())) else { continue };
            if a.hook == HANDOFF_HOOK || b.hook == HANDOFF_HOOK {
                continue;
            }
            let (keep, other) = if a.updated_at >= b.updated_at { (a, b) } else { (b, a) };
            if self.brain_merge(&keep.id.to_string(), &other.id.to_string()).is_ok() {
                merged += 1;
            }
        }
        Ok(merged)
    }

    /// The stats shown in the app: what's been served, how searches went, and how many memories
    /// have ever been of use.
    pub fn brain_stats_report(&mut self) -> Result<BrainStats> {
        let mut s = self.brain_stats()?;
        let mems: Vec<Memory> = self.brain_memories(None)?.into_iter().filter(|m| m.kind != MemoryKind::Codemap && m.hook != HANDOFF_HOOK).collect();
        s.memories = mems.len() as u64;
        s.used = mems.iter().filter(|m| m.recall_count > 0).count() as u64;
        Ok(s)
    }

    /// Makes sure the search vectors of these memories are made (they're kept while a memory's
    /// text stays the same, so a recall doesn't re-read every memory each time).
    fn warm_vecs(&mut self, r: &Ranker, mems: &[Memory]) {
        for m in mems {
            if self.brain_vecs.get(&m.id).is_some_and(|v| v.updated == m.updated_at) {
                continue;
            }
            let v = MemVec {
                updated: m.updated_at,
                whole: r.sparse(&format!("{}\n{}", m.hook, m.content)),
                hook: r.sparse(&m.hook),
                words: stems(&format!("{} {}", m.hook, m.content)),
            };
            self.brain_vecs.insert(m.id, v);
        }
    }

    /// How well a memory matches a question (needs `warm_vecs` first).
    fn relevance(&self, qs: &[(u32, f32)], groups: &[Vec<String>], m: &Memory) -> f32 {
        let Some(v) = self.brain_vecs.get(&m.id) else { return 0.0 };
        let body = sdot(qs, &v.whole);
        let hook = sdot(qs, &v.hook);
        // How many of the query's words (or words meaning the same) the memory has.
        let overlap = if groups.is_empty() { 0.0 } else { groups.iter().filter(|g| g.iter().any(|t| v.words.contains(t))).count() as f32 / groups.len() as f32 };
        0.55 * body + 0.25 * hook + 0.2 * overlap + if m.pinned { 0.03 } else { 0.0 }
    }

    /// Memories matching a question, by meaning: the best few, each trimmed to its most relevant
    /// lines. Searches the project and Me.
    pub fn brain_recall(&mut self, folder: Uuid, query: &str, limit: usize) -> Result<(String, Vec<Memory>)> {
        self.recall(folder, query, limit, 0.1, 3, true, &HashSet::new())
    }

    /// For a hook that runs on every message the user sends: the few memories that clearly match
    /// it, or nothing at all (no tokens spent when there's nothing relevant). Stricter than a
    /// recall the agent asks for, since it runs unasked.
    pub fn brain_hook_prompt(&mut self, folder: Uuid, prompt: &str) -> Result<Option<String>> {
        Ok(self.brain_hook_prompt_new(folder, prompt, &HashSet::new())?.map(|(text, _)| text))
    }

    /// The same, leaving out memories whose handles are in `shown` (already given to this session,
    /// so they aren't paid for again), and also returning the handles of the ones it does give.
    pub fn brain_hook_prompt_new(&mut self, folder: Uuid, prompt: &str, shown: &HashSet<String>) -> Result<Option<(String, Vec<String>)>> {
        if stems(prompt).len() < 2 {
            return Ok(None);
        }
        let (text, found) = self.recall(folder, prompt, 3, 0.3, 2, false, shown)?;
        if found.is_empty() {
            return Ok(None);
        }
        let handles = found.iter().map(|m| m.short.clone()).collect();
        Ok(Some((format!("Brain — remembered from earlier, possibly relevant (brain_read [handle] for the whole memory):\n{text}"), handles)))
    }

    #[allow(clippy::too_many_arguments)]
    fn recall(&mut self, folder: Uuid, query: &str, limit: usize, min_score: f32, lines: usize, report: bool, skip: &HashSet<String>) -> Result<(String, Vec<Memory>)> {
        let me = self.brain_me()?;
        let vault = self.brain_vault()?;
        let pool: Vec<Memory> = self
            .brain_memories_in(vault, Some(&[folder, me]))?
            .into_iter()
            .filter(|m| m.kind != MemoryKind::Codemap && m.hook != HANDOFF_HOOK && !skip.contains(&m.short))
            .collect();
        let r = Ranker::new();
        self.warm_vecs(&r, &pool);
        let q = r.vec(query);
        let qs = r.sparse(query);
        let groups = query_groups(query);
        let qterms: HashSet<String> = groups.iter().flatten().cloned().collect();
        let mut scored: Vec<(f32, &Memory)> = pool.iter().map(|m| (self.relevance(&qs, &groups, m), m)).filter(|(s, _)| *s >= min_score).collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.truncate(limit.clamp(1, 12));
        let mut out = String::new();
        if scored.is_empty() && report {
            out.push_str(&format!("No memories about “{}”. (If you work it out, brain_remember it.)\n", one_line(query, 80)));
        }
        for (_, m) in &scored {
            out.push_str(&format!("[{}] {} · {}{}\n", m.short, m.kind.as_str(), m.hook, if m.folder == me { " (Me)" } else { "" }));
            for line in best_lines(&r, &q, &qterms, &m.content, lines) {
                out.push_str(&format!("  {line}\n"));
            }
        }
        let found: Vec<Memory> = scored.iter().map(|(_, m)| (*m).clone()).collect();
        if report || !found.is_empty() {
            self.note_recalls(&found.iter().map(|m| m.id).collect::<Vec<_>>())?;
            let whole: u64 = pool.iter().map(|m| estimate_tokens(&m.content) + estimate_tokens(&m.hook)).sum();
            self.count_served(&out, whole, report.then_some(!found.is_empty()))?;
        }
        Ok((out, found))
    }

    /// A memory in full.
    pub fn brain_read(&mut self, handle: &str) -> Result<String> {
        let m = self.brain_memory(handle)?;
        self.note_recalls(&[m.id])?;
        let out = format!("[{}] {} · {}\n{}\n", m.short, m.kind.as_str(), m.hook, m.content.trim());
        self.count_served(&out, estimate_tokens(&out), None)?;
        Ok(out)
    }

    // ---- code map ----

    fn code_map(&mut self, folder: Uuid) -> Result<Option<Memory>> {
        let vault = self.brain_vault()?;
        Ok(self.brain_memories_in(vault, Some(&[folder]))?.into_iter().find(|m| m.kind == MemoryKind::Codemap))
    }

    pub fn brain_map_entries(&mut self, folder: Uuid) -> Result<Vec<MapEntry>> {
        Ok(self.code_map(folder)?.map(|m| parse_map(&m.content)).unwrap_or_default())
    }

    /// Adds or updates code map lines (a blank purpose removes the file).
    pub fn brain_map_note(&mut self, folder: Uuid, entries: &[MapEntry], source: &str) -> Result<usize> {
        let mut map = self.brain_map_entries(folder)?;
        for e in entries {
            let path = e.path.trim().trim_start_matches("./").to_string();
            if path.is_empty() {
                continue;
            }
            map.retain(|x| x.path != path);
            if !e.purpose.trim().is_empty() {
                map.push(MapEntry { path, purpose: one_line(&e.purpose, 160), mtime: e.mtime });
            }
        }
        map.sort_by(|a, b| a.path.cmp(&b.path));
        let content = render_map(&map);
        let hook = format!("Code map: {} files", map.len());
        let body = render(MemoryKind::Codemap, &hook, source, false, &content);
        match self.code_map(folder)? {
            Some(m) => {
                self.update_note(m.id, NotePatch { title: Some(CODE_MAP_TITLE.into()), body: Some(body) })?;
            }
            None => {
                let vault = self.brain_vault()?;
                self.create_note(NewNote { title: CODE_MAP_TITLE.into(), body, vault_id: Some(vault), folder_id: Some(folder), ..Default::default() })?;
            }
        }
        Ok(map.len())
    }

    /// Code map lines relevant to `query` (or all of them, compactly), flagging files that changed
    /// or vanished since they were described.
    pub fn brain_map(&mut self, folder: Uuid, query: Option<&str>, root: Option<&Path>, limit: usize) -> Result<String> {
        let map = self.brain_map_entries(folder)?;
        if map.is_empty() {
            return Ok("No code map yet for this project. As you learn what files do, save them with brain_map_note.\n".into());
        }
        let mut entries: Vec<&MapEntry> = map.iter().collect();
        if let Some(q) = query.filter(|q| !q.trim().is_empty()) {
            let r = Ranker::new();
            let qv = r.vec(q);
            let qterms = stems(q);
            let mut scored: Vec<(f32, &MapEntry)> = entries
                .iter()
                .map(|e| {
                    let text = format!("{} {}", e.path.replace(['/', '_', '-', '.'], " "), e.purpose);
                    let words = stems(&text);
                    let overlap = if qterms.is_empty() { 0.0 } else { qterms.iter().filter(|t| words.contains(*t)).count() as f32 / qterms.len() as f32 };
                    (0.6 * dot(&qv, &r.vec(&text)) + 0.4 * overlap, *e)
                })
                .filter(|(s, _)| *s > 0.08)
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0));
            entries = scored.into_iter().take(limit.max(1)).map(|(_, e)| e).collect();
            if entries.is_empty() {
                return Ok(format!("Nothing in the code map matches “{}”.\n", one_line(q, 60)));
            }
        }
        let mut out = String::new();
        for e in &entries {
            let flag = root.and_then(|r| map_status(r, e)).map(|s| format!(" ({s})")).unwrap_or_default();
            out.push_str(&format!("{} — {}{}\n", e.path, e.purpose, flag));
        }
        let whole: u64 = map.iter().map(|e| estimate_tokens(&e.path) + estimate_tokens(&e.purpose) + 2).sum();
        self.count_served(&out, whole, None)?;
        Ok(out)
    }

    /// Every memory (except code maps) and, for each, its few most related memories.
    pub fn brain_graph(&mut self) -> Result<BrainGraph> {
        let nodes: Vec<Memory> = self.brain_memories(None)?.into_iter().filter(|m| m.kind != MemoryKind::Codemap).collect();
        let r = Ranker::new();
        let vecs: Vec<Vec<(u32, f32)>> = nodes.iter().map(|m| r.sparse(&format!("{}\n{}", m.hook, m.content))).collect();
        // Memories link within their project, and with Me (which links with everything).
        let me = self.brain_me()?;
        let mut by_folder: std::collections::HashMap<Uuid, Vec<usize>> = std::collections::HashMap::new();
        for (i, m) in nodes.iter().enumerate() {
            by_folder.entry(m.folder).or_default().push(i);
        }
        let everyone: Vec<usize> = (0..nodes.len()).collect();
        let none: Vec<usize> = Vec::new();
        let mut seen = HashSet::new();
        let mut links = Vec::new();
        for i in 0..nodes.len() {
            let folder = nodes[i].folder;
            let candidates = if folder == me {
                everyone.iter()
            } else {
                by_folder.get(&folder).unwrap_or(&none).iter()
            };
            let extra = if folder == me { none.iter() } else { by_folder.get(&me).unwrap_or(&none).iter() };
            let mut best: Vec<(usize, f32)> = candidates
                .chain(extra)
                .copied()
                .filter(|&j| j != i)
                .map(|j| (j, sdot(&vecs[i], &vecs[j])))
                .filter(|(_, s)| *s >= 0.22)
                .collect();
            best.sort_by(|a, b| b.1.total_cmp(&a.1));
            for (j, s) in best.into_iter().take(3) {
                let key = (i.min(j), i.max(j));
                if seen.insert(key) {
                    links.push(BrainLink { a: nodes[key.0].id, b: nodes[key.1].id, strength: s });
                }
            }
        }
        Ok(BrainGraph { nodes, links })
    }

    // ---- keeping it small ----

    /// Duplicates, maybe-conflicting pairs and stale memories in a project (or everywhere).
    pub fn brain_tidy(&mut self, folder: Option<Uuid>) -> Result<TidyReport> {
        let all: Vec<Memory> = self.brain_memories(folder)?.into_iter().filter(|m| m.kind != MemoryKind::Codemap).collect();
        let r = Ranker::new();
        let vecs: Vec<Vec<(u32, f32)>> = all.iter().map(|m| r.sparse(&format!("{}\n{}", m.hook, m.content))).collect();
        let mut report = TidyReport::default();
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                if all[i].folder != all[j].folder {
                    continue;
                }
                let s = sdot(&vecs[i], &vecs[j]);
                let pair = || TidyPair { a: all[i].id, b: all[j].id, a_hook: all[i].hook.clone(), b_hook: all[j].hook.clone(), score: s };
                if s >= 0.8 {
                    report.duplicates.push(pair());
                } else if s >= 0.55 && all[i].kind == all[j].kind && matches!(all[i].kind, MemoryKind::Decision | MemoryKind::Preference | MemoryKind::Fact) {
                    report.similar.push(pair());
                }
            }
        }
        let now = now_ms();
        let day = 86_400_000;
        report.stale = all
            .iter()
            .filter(|m| !m.pinned && m.kind != MemoryKind::Summary && now - m.updated_at > 30 * day && m.recalled_at.is_none_or(|t| now - t > 45 * day))
            .map(|m| m.id)
            .collect();
        report.duplicates.sort_by(|a, b| b.score.total_cmp(&a.score));
        report.similar.sort_by(|a, b| b.score.total_cmp(&a.score));
        Ok(report)
    }

    /// Folds `other` into `keep` (its new lines appended) and archives `other`.
    pub fn brain_merge(&mut self, keep: &str, other: &str) -> Result<Memory> {
        let a = self.brain_memory(keep)?;
        let b = self.brain_memory(other)?;
        if a.id == b.id {
            return Ok(a);
        }
        let merged = merge_text(&a.content, &format!("{}\n{}", b.hook, b.content));
        self.update_note(a.id, NotePatch { body: Some(render(a.kind, &a.hook, &a.source, a.new, &merged)), ..Default::default() })?;
        self.archive_note(b.id)?;
        self.brain_memory(&a.id.to_string())
    }

    /// Compresses several memories into one summary (each becomes a line) and archives them.
    pub fn brain_compact(&mut self, handles: &[String], hook: &str) -> Result<Memory> {
        let ms: Vec<Memory> = handles.iter().map(|h| self.brain_memory(h)).collect::<Result<_>>()?;
        let Some(first) = ms.first() else { return Err(Error::invalid("pick memories to compress")) };
        let folder = first.folder;
        let mut content = String::new();
        for m in &ms {
            let first_line = sentences(&m.content).into_iter().next().map(|s| format!(": {}", one_line(&s, 160))).unwrap_or_default();
            content.push_str(&format!("- {}{}\n", m.hook, first_line));
        }
        let hook = if hook.trim().is_empty() { format!("Summary of {} older memories", ms.len()) } else { hook.to_string() };
        let vault = self.brain_vault()?;
        let note = self.create_note(NewNote {
            title: one_line(&hook, 200),
            body: render(MemoryKind::Summary, &hook, "you", false, &content),
            vault_id: Some(vault),
            folder_id: Some(folder),
            ..Default::default()
        })?;
        for m in &ms {
            self.archive_note(m.id)?;
        }
        self.brain_memory(&note.id.to_string())
    }

    /// Changes whenever a Brain note does (count, latest update) — cheap, for watching.
    pub fn brain_change_stamp(&self) -> Result<(i64, i64)> {
        let Some(vault) = self.get_setting(VAULT_KEY)?.and_then(|s| s.parse::<Uuid>().ok()) else { return Ok((0, 0)) };
        Ok(self.conn.query_row(
            "SELECT COUNT(*), COALESCE(MAX(updated_at), 0) + COALESCE(SUM(archived_at IS NOT NULL), 0) FROM notes WHERE vault_id = ?1",
            [vault.as_bytes()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    // ---- Markdown mirror ----

    /// Where the Brain is mirrored as Markdown (default `~/Astralis Brain`), or `None` if off.
    pub fn brain_mirror_dir(&self) -> Result<Option<std::path::PathBuf>> {
        match self.get_setting(MIRROR_KEY)?.as_deref() {
            Some("off") => Ok(None),
            Some(p) if !p.trim().is_empty() => Ok(Some(std::path::PathBuf::from(p))),
            // Each profile mirrors to its own folder (so a test profile never mixes into yours).
            _ => {
                let name = match std::env::var("ASTRALIS_PROFILE") {
                    Ok(p) if !p.trim().is_empty() => format!("Astralis Brain ({})", p.trim()),
                    _ => "Astralis Brain".to_string(),
                };
                Ok(std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(|h| std::path::PathBuf::from(h).join(name)))
            }
        }
    }

    /// Sets the mirror folder (`None` turns the mirror off).
    pub fn set_brain_mirror_dir(&self, dir: Option<&str>) -> Result<()> {
        self.set_setting(MIRROR_KEY, dir.unwrap_or("off"))
    }

    /// Re-exports the mirror if it's on (after changes).
    pub fn sync_brain_mirror(&mut self) -> Result<()> {
        if let Some(d) = self.brain_mirror_dir()? {
            self.export_brain(&d)?;
        }
        Ok(())
    }


    /// Writes the Brain as Markdown files (one folder per project with an INDEX.md and one file
    /// per memory), removing files it wrote before that no longer exist. Obsidian-compatible.
    pub fn export_brain(&mut self, dir: &Path) -> Result<usize> {
        std::fs::create_dir_all(dir)?;
        let manifest_path = dir.join(".astralis-brain.json");
        let old: Vec<String> = std::fs::read_to_string(&manifest_path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut written: Vec<String> = Vec::new();
        let projects = self.brain_projects()?;
        for p in &projects {
            let folder_rel = sanitize(&p.name);
            let pdir = dir.join(&folder_rel);
            std::fs::create_dir_all(&pdir)?;
            let memories = self.brain_memories(Some(p.folder))?;
            let mut index = format!("# {}\n\n", p.name);
            for m in &memories {
                let file = format!("{}-{}.md", sanitize(&one_line(&m.hook, 48)), m.short);
                let note = self.get_note(m.id)?;
                write_if_changed(&pdir.join(&file), &note.body)?;
                index.push_str(&format!("- [[{}|{}]] · {}\n", file.trim_end_matches(".md"), m.hook, m.kind.as_str()));
                written.push(format!("{folder_rel}/{file}"));
            }
            write_if_changed(&pdir.join("INDEX.md"), &index)?;
            written.push(format!("{folder_rel}/INDEX.md"));
        }
        for f in old.iter().filter(|f| !written.contains(f)) {
            let _ = std::fs::remove_file(dir.join(f));
            // Remove the project folder too once it's empty (remove_dir fails if it isn't).
            if let Some(parent) = Path::new(f).parent().filter(|p| !p.as_os_str().is_empty()) {
                let _ = std::fs::remove_dir(dir.join(parent));
            }
        }
        write_if_changed(&manifest_path, &serde_json::to_string_pretty(&written)?)?;
        Ok(written.len())
    }
}

/// The lines of `content` most relevant to the query (in their original order).
fn best_lines(r: &Ranker, q: &[f32], qterms: &HashSet<String>, content: &str, n: usize) -> Vec<String> {
    let lines: Vec<String> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("```"))
        .flat_map(|l| if l.len() > 240 { sentences(l) } else { vec![l.to_string()] })
        .collect();
    if lines.len() <= n {
        return lines.into_iter().map(|l| one_line(&l, 240)).collect();
    }
    let mut scored: Vec<(usize, f32)> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let words = stems(l);
            let overlap = if qterms.is_empty() { 0.0 } else { qterms.iter().filter(|t| words.contains(*t)).count() as f32 / qterms.len() as f32 };
            (i, 0.6 * dot(q, &r.vec(l)) + 0.4 * overlap)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut keep: Vec<usize> = scored.into_iter().take(n).map(|(i, _)| i).collect();
    keep.sort_unstable();
    let mut out: Vec<String> = keep.into_iter().map(|i| one_line(&lines[i], 240)).collect();
    out.push(format!("… ({} more lines — brain_read for all)", lines.len() - n));
    out
}

/// Appends the lines of `extra` that `base` doesn't already have.
fn merge_text(base: &str, extra: &str) -> String {
    let have: HashSet<String> = base.lines().map(|l| l.trim().to_lowercase()).collect();
    let mut out = base.trim_end().to_string();
    for l in extra.lines() {
        if !l.trim().is_empty() && !have.contains(&l.trim().to_lowercase()) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(l.trim_end());
        }
    }
    out
}

fn parse_map(content: &str) -> Vec<MapEntry> {
    content
        .lines()
        .filter_map(|l| {
            let l = l.trim().strip_prefix("- ")?;
            let (path, rest) = if let Some(after) = l.strip_prefix('`') {
                let end = after.find('`')?;
                (after[..end].to_string(), after[end + 1..].to_string())
            } else {
                let (p, r) = l.split_once(" — ")?;
                (p.to_string(), format!(" — {r}"))
            };
            let rest = rest.trim().trim_start_matches('—').trim();
            let (purpose, mtime) = match rest.rsplit_once("<!--m:") {
                Some((p, m)) => (p.trim().to_string(), m.trim_end_matches("-->").trim().parse().ok()),
                None => (rest.to_string(), None),
            };
            Some(MapEntry { path, purpose, mtime })
        })
        .collect()
}

fn render_map(entries: &[MapEntry]) -> String {
    entries
        .iter()
        .map(|e| match e.mtime {
            Some(m) => format!("- `{}` — {} <!--m:{m}-->", e.path, e.purpose),
            None => format!("- `{}` — {}", e.path, e.purpose),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// "changed" / "gone" if the file isn't as it was when described.
fn map_status(root: &Path, e: &MapEntry) -> Option<&'static str> {
    let meta = std::fs::metadata(root.join(&e.path)).ok();
    match (meta, e.mtime) {
        (None, _) => Some("gone"),
        (Some(m), Some(then)) => {
            let now = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            (now > then + 1).then_some("changed since described")
        }
        _ => None,
    }
}

/// A file's modification time in seconds (for code map entries).
pub fn file_mtime(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// A file name from a title: punctuation dropped, spaces collapsed.
fn sanitize(s: &str) -> String {
    let out: String = s.chars().map(|c| if c.is_alphanumeric() || " -_()".contains(c) { c } else { ' ' }).collect();
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let out = out.trim_matches(['.', '-', ' ']).to_string();
    if out.is_empty() { "untitled".into() } else { out }
}

fn write_if_changed(path: &Path, text: &str) -> Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
        std::fs::write(path, text)?;
    }
    Ok(())
}
