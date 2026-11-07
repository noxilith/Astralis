//! Sync foundation: everything P2P and cloud sync will move around.
//!
//! - Every change to a synced row is stamped by SQLite triggers with a **hybrid logical clock**
//!   (`hlc`, sortable as text) and the **origin device**. Deletes leave a **tombstone**.
//!   Ordinary app code doesn't know sync exists.
//! - Devices exchange [`ChangeSet`]s: "everything newer than what you've seen from each device"
//!   (a [`VersionVector`]). Applying a change set is idempotent and order-independent.
//! - Rows merge last-writer-wins by `hlc`, except note text, which merges as a **CRDT** (yrs), so
//!   concurrent edits to the same note on two devices both survive.
//! - Encrypted rows travel as ciphertext; vault keys never leave the device (another device
//!   unlocks with the same password, since the wrapped key syncs with the vault row).
//!
//! Derived, per-device data (search index, links, history, AI embeddings, settings) isn't synced;
//! it's rebuilt locally from the synced rows.

mod crdt;
pub mod hlc;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::functions::FunctionFlags;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notes::normalize_tag;
use crate::store::Store;
use hlc::Clock;

/// Synced tables, in the order rows are applied (parents before children).
pub const SYNCED_TABLES: &[&str] = &[
    "vaults",
    "folders",
    "planners",
    "templates",
    "notes",
    "tasks",
    "events",
    "attachments",
];

/// Highest clock value seen per origin device.
pub type VersionVector = BTreeMap<String, String>;

/// Shared between the [`Store`] and the SQL functions its triggers call.
#[derive(Default)]
pub struct SyncCtx {
    clock: Mutex<Clock>,
    device: Mutex<String>,
    /// While true, triggers leave rows alone (we're writing remote changes verbatim).
    applying: AtomicBool,
}

impl SyncCtx {
    fn now(&self) -> String {
        let device = self.device.lock().unwrap().clone();
        self.clock.lock().unwrap().now(&device)
    }
}

/// A SQLite value, serializable for transport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum SqlValue {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl From<ValueRef<'_>> for SqlValue {
    fn from(v: ValueRef<'_>) -> Self {
        match v {
            ValueRef::Null => SqlValue::Null,
            ValueRef::Integer(i) => SqlValue::Int(i),
            ValueRef::Real(f) => SqlValue::Real(f),
            ValueRef::Text(t) => SqlValue::Text(String::from_utf8_lossy(t).into_owned()),
            ValueRef::Blob(b) => SqlValue::Blob(b.to_vec()),
        }
    }
}

impl From<&SqlValue> for Value {
    fn from(v: &SqlValue) -> Self {
        match v {
            SqlValue::Null => Value::Null,
            SqlValue::Int(i) => Value::Integer(*i),
            SqlValue::Real(f) => Value::Real(*f),
            SqlValue::Text(t) => Value::Text(t.clone()),
            SqlValue::Blob(b) => Value::Blob(b.clone()),
        }
    }
}

impl SqlValue {
    fn as_text(&self) -> &str {
        match self {
            SqlValue::Text(t) => t,
            _ => "",
        }
    }
    fn as_blob(&self) -> Option<&[u8]> {
        match self {
            SqlValue::Blob(b) => Some(b),
            _ => None,
        }
    }
}

/// The latest state of one row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowChange {
    pub table: String,
    pub values: BTreeMap<String, SqlValue>,
    /// For notes: the note's tag names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

impl RowChange {
    fn hlc(&self) -> &str {
        self.values.get("hlc").map(SqlValue::as_text).unwrap_or("")
    }
    fn origin(&self) -> &str {
        self.values
            .get("origin")
            .map(SqlValue::as_text)
            .unwrap_or("")
    }
    fn id(&self) -> Option<&[u8]> {
        self.values.get("id").and_then(SqlValue::as_blob)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub entity: String,
    pub id: Vec<u8>,
    pub hlc: String,
    pub origin: String,
}

/// Everything one device has that another hasn't seen.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    /// The sending device.
    pub device: String,
    pub rows: Vec<RowChange>,
    pub tombstones: Vec<Tombstone>,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.tombstones.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyStats {
    pub inserted: usize,
    pub updated: usize,
    pub merged: usize,
    pub deleted: usize,
    pub skipped: usize,
}

/// Registers the SQL functions the change-tracking triggers call.
pub(crate) fn register_sql_functions(
    conn: &Connection,
    ctx: &Arc<SyncCtx>,
) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8;
    let c = ctx.clone();
    conn.create_scalar_function("astralis_hlc", 0, flags, move |_| Ok(c.now()))?;
    let c = ctx.clone();
    conn.create_scalar_function("astralis_device", 0, flags, move |_| {
        Ok(c.device.lock().unwrap().clone())
    })?;
    let c = ctx.clone();
    conn.create_scalar_function("astralis_applying", 0, flags, move |_| {
        Ok(c.applying.load(Ordering::SeqCst) as i64)
    })?;
    Ok(())
}

/// Migration adding clocks, tombstones and change-tracking triggers.
pub(crate) fn sync_migration() -> String {
    let mut sql = String::from(
        "CREATE TABLE tombstones (
            entity TEXT NOT NULL, id BLOB NOT NULL, hlc TEXT NOT NULL, origin TEXT NOT NULL,
            PRIMARY KEY (entity, id)
        ) WITHOUT ROWID;
        CREATE TABLE sync_seen (origin TEXT PRIMARY KEY, hlc TEXT NOT NULL) WITHOUT ROWID;
        ALTER TABLE notes ADD COLUMN crdt BLOB;\n",
    );
    for t in SYNCED_TABLES {
        sql.push_str(&format!(
            "ALTER TABLE {t} ADD COLUMN hlc TEXT NOT NULL DEFAULT '';
             ALTER TABLE {t} ADD COLUMN origin TEXT NOT NULL DEFAULT '';
             CREATE INDEX {t}_hlc_idx ON {t}(origin, hlc);
             CREATE TRIGGER {t}_sync_ai AFTER INSERT ON {t} WHEN astralis_applying() = 0 BEGIN
                 UPDATE {t} SET hlc = astralis_hlc(), origin = astralis_device() WHERE id = new.id;
             END;
             CREATE TRIGGER {t}_sync_au AFTER UPDATE ON {t} WHEN astralis_applying() = 0 AND new.hlc IS old.hlc BEGIN
                 UPDATE {t} SET hlc = astralis_hlc(), origin = astralis_device() WHERE id = new.id;
             END;
             CREATE TRIGGER {t}_sync_ad AFTER DELETE ON {t} WHEN astralis_applying() = 0 BEGIN
                 INSERT OR REPLACE INTO tombstones(entity, id, hlc, origin)
                 VALUES ('{t}', old.id, astralis_hlc(), astralis_device());
             END;\n"
        ));
    }
    // A note's tags are part of the note: changing them counts as changing the note.
    sql.push_str(
        "CREATE TRIGGER note_tags_sync_ai AFTER INSERT ON note_tags WHEN astralis_applying() = 0 BEGIN
             UPDATE notes SET hlc = astralis_hlc(), origin = astralis_device() WHERE id = new.note_id;
         END;
         CREATE TRIGGER note_tags_sync_ad AFTER DELETE ON note_tags WHEN astralis_applying() = 0 BEGIN
             UPDATE notes SET hlc = astralis_hlc(), origin = astralis_device() WHERE id = old.note_id;
         END;\n",
    );
    sql
}

/// Resets the "applying" flag even if applying fails part-way.
struct ApplyingGuard<'a>(&'a AtomicBool);
impl Drop for ApplyingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn column_names(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(names)
}

impl Store {
    /// Loads (or creates) this device's id and stamps rows that predate change tracking.
    pub(crate) fn init_sync(&self) -> Result<()> {
        let device = match self.get_setting("device_id")? {
            Some(d) => d,
            None => {
                let d = Uuid::now_v7().simple().to_string();
                self.set_setting("device_id", &d)?;
                d
            }
        };
        *self.sync.device.lock().unwrap() = device;
        for t in SYNCED_TABLES {
            self.conn.execute(
                &format!(
                    "UPDATE {t} SET hlc = astralis_hlc(), origin = astralis_device() WHERE hlc = ''"
                ),
                [],
            )?;
        }
        let ids: Vec<Vec<u8>> = {
            let mut stmt = self.conn.prepare(
                "SELECT id FROM notes WHERE crdt IS NULL AND cipher IS NULL AND kind = 'note'",
            )?;
            stmt.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        for id in ids {
            if let Ok(id) = Uuid::from_slice(&id) {
                self.refresh_crdt(id)?;
            }
        }
        Ok(())
    }

    /// This device's sync id.
    pub fn device_id(&self) -> String {
        self.sync.device.lock().unwrap().clone()
    }

    /// Keeps a note's CRDT text state in step with its `body` (plain text notes only; encrypted
    /// notes and whiteboards merge last-writer-wins for now).
    pub(crate) fn refresh_crdt(&self, id: Uuid) -> Result<()> {
        let row: Option<(String, bool, String, Option<Vec<u8>>)> = self
            .conn
            .query_row(
                "SELECT kind, cipher IS NOT NULL, body, crdt FROM notes WHERE id = ?1",
                [id.as_bytes()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((kind, encrypted, body, state)) = row else {
            return Ok(());
        };
        if kind != "note" || encrypted {
            if state.is_some() {
                self.conn.execute(
                    "UPDATE notes SET crdt = NULL WHERE id = ?1",
                    [id.as_bytes()],
                )?;
            }
            return Ok(());
        }
        let client = crdt::client_id(&self.device_id());
        if let Some(s) = &state
            && crdt::read_text(s)? == body {
                return Ok(());
            }
        let next = crdt::set_text(state.as_deref(), &body, client)?;
        self.conn.execute(
            "UPDATE notes SET crdt = ?2 WHERE id = ?1",
            params![id.as_bytes(), next],
        )?;
        Ok(())
    }

    /// The highest clock this device has seen from every origin.
    pub fn version_vector(&self) -> Result<VersionVector> {
        let mut vv = VersionVector::new();
        let mut bump = |origin: String, hlc: String| {
            if origin.is_empty() {
                return;
            }
            let e = vv.entry(origin).or_default();
            if hlc > *e {
                *e = hlc;
            }
        };
        let mut sources: Vec<String> = SYNCED_TABLES.iter().map(|t| t.to_string()).collect();
        sources.push("tombstones".into());
        sources.push("sync_seen".into());
        for t in sources {
            let mut stmt = self
                .conn
                .prepare(&format!("SELECT origin, MAX(hlc) FROM {t} GROUP BY origin"))?;
            for row in
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            {
                let (o, h) = row?;
                bump(o, h);
            }
        }
        Ok(vv)
    }

    /// Everything newer than what `seen` covers. Built-in templates are per-device, not synced.
    pub fn changes_since(&self, seen: &VersionVector) -> Result<ChangeSet> {
        let newer = |origin: &str, hlc: &str| seen.get(origin).is_none_or(|s| hlc > s.as_str());
        let mut rows = Vec::new();
        for &table in SYNCED_TABLES {
            let mut stmt = self.conn.prepare(&format!("SELECT * FROM {table}"))?;
            let names: Vec<String> = stmt.column_names().map(str::to_owned).collect();
            let mut q = stmt.query([])?;
            while let Some(r) = q.next()? {
                let mut values = BTreeMap::new();
                for (i, n) in names.iter().enumerate() {
                    values.insert(n.clone(), SqlValue::from(r.get_ref(i)?));
                }
                let change = RowChange {
                    table: table.into(),
                    values,
                    tags: None,
                };
                if !newer(change.origin(), change.hlc()) {
                    continue;
                }
                if table == "templates"
                    && matches!(change.values.get("builtin"), Some(SqlValue::Int(1)))
                {
                    continue;
                }
                rows.push(change);
            }
        }
        for row in rows.iter_mut().filter(|r| r.table == "notes") {
            let id = row.id().unwrap_or_default().to_vec();
            let mut stmt = self.conn.prepare_cached(
                "SELECT t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id = ?1 ORDER BY t.name",
            )?;
            row.tags = Some(
                stmt.query_map([id], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?,
            );
        }
        let tombstones = {
            let mut stmt = self
                .conn
                .prepare("SELECT entity, id, hlc, origin FROM tombstones")?;
            let all: Vec<Tombstone> = stmt
                .query_map([], |r| {
                    Ok(Tombstone {
                        entity: r.get(0)?,
                        id: r.get(1)?,
                        hlc: r.get(2)?,
                        origin: r.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?;
            all.into_iter()
                .filter(|t| newer(&t.origin, &t.hlc))
                .collect()
        };
        Ok(ChangeSet {
            device: self.device_id(),
            rows,
            tombstones,
        })
    }

    /// Applies another device's changes. Safe to repeat and to apply in any order.
    pub fn apply_changes(&self, cs: &ChangeSet) -> Result<ApplyStats> {
        self.sync.applying.store(true, Ordering::SeqCst);
        let _guard = ApplyingGuard(&self.sync.applying);
        self.conn
            .execute_batch("BEGIN IMMEDIATE; PRAGMA defer_foreign_keys = ON;")?;
        let result = self.apply_inner(cs);
        match result {
            Ok(stats) => {
                self.conn.execute_batch("COMMIT")?;
                drop(_guard);
                self.after_apply(cs)?;
                Ok(stats)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    fn apply_inner(&self, cs: &ChangeSet) -> Result<ApplyStats> {
        let mut stats = ApplyStats::default();
        let mut columns: HashMap<&str, HashSet<String>> = HashMap::new();
        for t in SYNCED_TABLES {
            columns.insert(t, column_names(&self.conn, t)?);
        }
        let mut seen: HashMap<String, String> = HashMap::new();
        let mut note_seen = |o: &str, h: &str| {
            let e = seen.entry(o.to_owned()).or_default();
            if h > e.as_str() {
                *e = h.to_owned();
            }
        };

        let mut rows: Vec<&RowChange> = cs.rows.iter().collect();
        rows.sort_by_key(|r| {
            SYNCED_TABLES
                .iter()
                .position(|t| *t == r.table)
                .unwrap_or(usize::MAX)
        });
        for row in rows {
            let Some(&table) = SYNCED_TABLES.iter().find(|t| **t == row.table) else {
                stats.skipped += 1;
                continue;
            };
            let Some(id) = row.id() else {
                stats.skipped += 1;
                continue;
            };
            self.sync.clock.lock().unwrap().observe(row.hlc());
            note_seen(row.origin(), row.hlc());
            let cols = &columns[table];
            // Only columns this device knows about (never trust remote column names).
            let values: Vec<(&String, &SqlValue)> = row
                .values
                .iter()
                .filter(|(k, _)| cols.contains(*k))
                .collect();

            let tomb: Option<String> = self
                .conn
                .query_row(
                    "SELECT hlc FROM tombstones WHERE entity = ?1 AND id = ?2",
                    params![table, id],
                    |r| r.get(0),
                )
                .optional()?;
            if tomb.as_deref().is_some_and(|t| t >= row.hlc()) {
                stats.skipped += 1; // deleted after this change
                continue;
            }
            let local: Option<(String, Option<Vec<u8>>)> = self
                .conn
                .query_row(
                    &format!(
                        "SELECT hlc, {} FROM {table} WHERE id = ?1",
                        if table == "notes" { "crdt" } else { "NULL" }
                    ),
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;

            // Note text merges as a CRDT regardless of which side "wins" the other fields.
            let mut merged: Option<(Vec<u8>, String)> = None;
            if table == "notes"
                && let (Some((_, Some(mine))), Some(SqlValue::Blob(theirs))) =
                    (&local, row.values.get("crdt"))
                {
                    merged = Some(crdt::merge(mine, theirs)?);
                }

            match &local {
                None => {
                    let names: Vec<&str> = values.iter().map(|(k, _)| k.as_str()).collect();
                    let marks: Vec<String> = (1..=names.len()).map(|i| format!("?{i}")).collect();
                    let args: Vec<Value> = values.iter().map(|(_, v)| Value::from(*v)).collect();
                    self.conn.execute(
                        &format!(
                            "INSERT INTO {table} ({}) VALUES ({})",
                            names.join(", "),
                            marks.join(", ")
                        ),
                        params_from_iter(args),
                    )?;
                    if tomb.is_some() {
                        self.conn.execute(
                            "DELETE FROM tombstones WHERE entity = ?1 AND id = ?2",
                            params![table, id],
                        )?;
                    }
                    self.apply_tags(table, id, row)?;
                    stats.inserted += 1;
                }
                Some((local_hlc, _)) if row.hlc() > local_hlc.as_str() => {
                    let mut sets = Vec::new();
                    let mut args: Vec<Value> = Vec::new();
                    for (k, v) in &values {
                        if k.as_str() == "id" {
                            continue;
                        }
                        let v = match (k.as_str(), &merged) {
                            ("crdt", Some((state, _))) => Value::Blob(state.clone()),
                            ("body", Some((_, text))) => Value::Text(text.clone()),
                            _ => Value::from(*v),
                        };
                        args.push(v);
                        sets.push(format!("{k} = ?{}", args.len()));
                    }
                    args.push(Value::Blob(id.to_vec()));
                    self.conn.execute(
                        &format!(
                            "UPDATE {table} SET {} WHERE id = ?{}",
                            sets.join(", "),
                            args.len()
                        ),
                        params_from_iter(args),
                    )?;
                    self.apply_tags(table, id, row)?;
                    stats.updated += 1;
                }
                Some(_) => match merged {
                    // Local fields are newer, but the text still merges in the other side's edits.
                    Some((state, text)) => {
                        self.conn.execute(
                            "UPDATE notes SET crdt = ?2, body = ?3 WHERE id = ?1",
                            params![id, state, text],
                        )?;
                        stats.merged += 1;
                    }
                    None => stats.skipped += 1,
                },
            }
        }

        for t in &cs.tombstones {
            let Some(&table) = SYNCED_TABLES.iter().find(|x| **x == t.entity) else {
                continue;
            };
            self.sync.clock.lock().unwrap().observe(&t.hlc);
            note_seen(&t.origin, &t.hlc);
            let local: Option<String> = self
                .conn
                .query_row(
                    &format!("SELECT hlc FROM {table} WHERE id = ?1"),
                    [&t.id],
                    |r| r.get(0),
                )
                .optional()?;
            match local {
                Some(h) if h.as_str() > t.hlc.as_str() => {
                    stats.skipped += 1; // edited after the delete: keep it
                    continue;
                }
                Some(_) => {
                    self.conn
                        .execute(&format!("DELETE FROM {table} WHERE id = ?1"), [&t.id])?;
                    stats.deleted += 1;
                }
                None => {}
            }
            self.conn.execute(
                "INSERT INTO tombstones(entity, id, hlc, origin) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(entity, id) DO UPDATE SET hlc = excluded.hlc, origin = excluded.origin
                 WHERE excluded.hlc > tombstones.hlc",
                params![table, t.id, t.hlc, t.origin],
            )?;
        }

        for (origin, hlc) in seen {
            self.conn.execute(
                "INSERT INTO sync_seen(origin, hlc) VALUES (?1, ?2)
                 ON CONFLICT(origin) DO UPDATE SET hlc = excluded.hlc WHERE excluded.hlc > sync_seen.hlc",
                params![origin, hlc],
            )?;
        }
        Ok(stats)
    }

    fn apply_tags(&self, table: &str, id: &[u8], row: &RowChange) -> Result<()> {
        let (true, Some(tags)) = (table == "notes", &row.tags) else {
            return Ok(());
        };
        self.conn
            .execute("DELETE FROM note_tags WHERE note_id = ?1", [id])?;
        for name in tags.iter().filter_map(|t| normalize_tag(t)) {
            self.conn
                .execute("INSERT OR IGNORE INTO tags(name) VALUES (?1)", [&name])?;
            self.conn.execute(
                "INSERT OR IGNORE INTO note_tags(note_id, tag_id) SELECT ?1, id FROM tags WHERE name = ?2",
                params![id, name],
            )?;
        }
        Ok(())
    }

    /// Rebuilds per-device derived data for notes that changed.
    fn after_apply(&self, cs: &ChangeSet) -> Result<()> {
        for row in cs.rows.iter().filter(|r| r.table == "notes") {
            let Some(id) = row.id().and_then(|b| Uuid::from_slice(b).ok()) else {
                continue;
            };
            let note: Option<(String, String, bool)> = self
                .conn
                .query_row(
                    "SELECT body, kind, cipher IS NOT NULL FROM notes WHERE id = ?1",
                    [id.as_bytes()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            if let Some((body, kind, encrypted)) = note {
                self.conn
                    .execute("DELETE FROM embeddings WHERE note_id = ?1", [id.as_bytes()])?;
                if kind == "note" && !encrypted {
                    self.sync_links(id, &body)?;
                }
            }
        }
        self.prune_tags()?;
        Ok(())
    }
}

/// Encodes a change set for transport (compact JSON for now).
pub fn encode(cs: &ChangeSet) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(cs)?)
}

pub fn decode(bytes: &[u8]) -> Result<ChangeSet> {
    serde_json::from_slice(bytes).map_err(|e| Error::Invalid(format!("bad change set: {e}")))
}
