use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::Result;
use crate::sync::SyncCtx;

/// Each entry is applied once, in order; `PRAGMA user_version` tracks progress.
/// Never edit an existing entry — append a new one.
pub(crate) const MIGRATIONS: &[&str] = &[
    // v1: notes
    r#"
    CREATE TABLE notes (
        id          BLOB PRIMARY KEY,
        title       TEXT NOT NULL,
        body        TEXT NOT NULL,
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL,
        pinned      INTEGER NOT NULL DEFAULT 0,
        archived_at INTEGER
    ) WITHOUT ROWID;
    CREATE INDEX notes_updated_idx ON notes(updated_at DESC);
    "#,
    // v2: everything else
    r#"
    CREATE TABLE vaults (
        id          BLOB PRIMARY KEY,
        name        TEXT NOT NULL,
        salt        BLOB NOT NULL,
        kdf_m       INTEGER NOT NULL,
        kdf_t       INTEGER NOT NULL,
        kdf_p       INTEGER NOT NULL,
        wrapped_key BLOB NOT NULL,
        created_at  INTEGER NOT NULL
    ) WITHOUT ROWID;

    ALTER TABLE notes ADD COLUMN vault_id BLOB REFERENCES vaults(id) ON DELETE CASCADE;
    ALTER TABLE notes ADD COLUMN cipher BLOB;
    CREATE INDEX notes_vault_idx ON notes(vault_id);
    CREATE INDEX notes_title_idx ON notes(lower(title));

    CREATE TABLE tags (
        id   INTEGER PRIMARY KEY,
        name TEXT NOT NULL UNIQUE
    );
    CREATE TABLE note_tags (
        note_id BLOB NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
        tag_id  INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
        PRIMARY KEY (note_id, tag_id)
    ) WITHOUT ROWID;
    CREATE INDEX note_tags_tag_idx ON note_tags(tag_id);

    CREATE TABLE links (
        source_id    BLOB NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
        target_title TEXT NOT NULL,
        PRIMARY KEY (source_id, target_title)
    ) WITHOUT ROWID;
    CREATE INDEX links_target_idx ON links(target_title);

    CREATE TABLE note_versions (
        id         BLOB PRIMARY KEY,
        note_id    BLOB NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
        title      TEXT NOT NULL,
        body       TEXT NOT NULL,
        created_at INTEGER NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX note_versions_note_idx ON note_versions(note_id, created_at DESC);

    CREATE TABLE templates (
        id         BLOB PRIMARY KEY,
        name       TEXT NOT NULL,
        title      TEXT NOT NULL,
        body       TEXT NOT NULL,
        builtin    INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) WITHOUT ROWID;

    CREATE TABLE planners (
        id          BLOB PRIMARY KEY,
        name        TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        color       TEXT NOT NULL DEFAULT '#7c6cff',
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL
    ) WITHOUT ROWID;

    CREATE TABLE tasks (
        id               BLOB PRIMARY KEY,
        title            TEXT NOT NULL,
        notes            TEXT NOT NULL DEFAULT '',
        status           TEXT NOT NULL DEFAULT 'todo',
        priority         INTEGER NOT NULL DEFAULT 0,
        due_at           INTEGER,
        all_day          INTEGER NOT NULL DEFAULT 0,
        remind_at        INTEGER,
        reminded         INTEGER NOT NULL DEFAULT 0,
        recurrence       TEXT,
        planner_id       BLOB REFERENCES planners(id) ON DELETE SET NULL,
        note_id          BLOB REFERENCES notes(id) ON DELETE SET NULL,
        completed_at     INTEGER,
        completion_count INTEGER NOT NULL DEFAULT 0,
        position         INTEGER NOT NULL DEFAULT 0,
        created_at       INTEGER NOT NULL,
        updated_at       INTEGER NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX tasks_due_idx ON tasks(due_at);
    CREATE INDEX tasks_remind_idx ON tasks(remind_at) WHERE reminded = 0;
    CREATE INDEX tasks_planner_idx ON tasks(planner_id);

    CREATE TABLE events (
        id          BLOB PRIMARY KEY,
        title       TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        location    TEXT NOT NULL DEFAULT '',
        start_at    INTEGER NOT NULL,
        end_at      INTEGER NOT NULL,
        all_day     INTEGER NOT NULL DEFAULT 0,
        remind_at   INTEGER,
        reminded    INTEGER NOT NULL DEFAULT 0,
        color       TEXT,
        created_at  INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX events_start_idx ON events(start_at);

    CREATE TABLE whiteboards (
        id         BLOB PRIMARY KEY,
        title      TEXT NOT NULL,
        scene      TEXT NOT NULL DEFAULT '{}',
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) WITHOUT ROWID;

    CREATE TABLE embeddings (
        note_id      BLOB PRIMARY KEY REFERENCES notes(id) ON DELETE CASCADE,
        model           TEXT NOT NULL,
        content_hash    INTEGER NOT NULL,
        note_updated_at INTEGER NOT NULL,
        vector          BLOB NOT NULL
    ) WITHOUT ROWID;

    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) WITHOUT ROWID;

    CREATE VIRTUAL TABLE notes_fts USING fts5(
        note_id UNINDEXED, title, body, tokenize = 'unicode61 remove_diacritics 2'
    );
    INSERT INTO notes_fts(note_id, title, body) SELECT id, title, body FROM notes;
    CREATE TRIGGER notes_fts_ai AFTER INSERT ON notes BEGIN
        INSERT INTO notes_fts(note_id, title, body) VALUES (new.id, new.title, new.body);
    END;
    CREATE TRIGGER notes_fts_ad AFTER DELETE ON notes BEGIN
        DELETE FROM notes_fts WHERE note_id = old.id;
    END;
    CREATE TRIGGER notes_fts_au AFTER UPDATE OF title, body ON notes BEGIN
        DELETE FROM notes_fts WHERE note_id = old.id;
        INSERT INTO notes_fts(note_id, title, body) VALUES (new.id, new.title, new.body);
    END;
    "#,
    // v3: vaults become workspaces (plain or encrypted); whiteboards become a kind of note.
    // Notes with a NULL vault are assigned to the default vault in `ensure_default_vault`.
    r#"
    ALTER TABLE vaults ADD COLUMN encrypted INTEGER NOT NULL DEFAULT 1;
    ALTER TABLE notes ADD COLUMN kind TEXT NOT NULL DEFAULT 'note';
    CREATE INDEX notes_vault_kind_idx ON notes(vault_id, kind);

    -- Only text notes have their body full-text indexed (whiteboard bodies are scene JSON).
    DROP TRIGGER notes_fts_ai;
    DROP TRIGGER notes_fts_au;
    CREATE TRIGGER notes_fts_ai AFTER INSERT ON notes BEGIN
        INSERT INTO notes_fts(note_id, title, body)
        VALUES (new.id, new.title, CASE WHEN new.kind = 'note' THEN new.body ELSE '' END);
    END;
    CREATE TRIGGER notes_fts_au AFTER UPDATE OF title, body, kind ON notes BEGIN
        DELETE FROM notes_fts WHERE note_id = old.id;
        INSERT INTO notes_fts(note_id, title, body)
        VALUES (new.id, new.title, CASE WHEN new.kind = 'note' THEN new.body ELSE '' END);
    END;

    INSERT INTO notes(id, title, body, created_at, updated_at, pinned, archived_at, vault_id, cipher, kind)
        SELECT id, title, scene, created_at, updated_at, 0, NULL, NULL, NULL, 'whiteboard' FROM whiteboards;
    DROP TABLE whiteboards;
    "#,
    // v4: nested folders inside vaults. Names in encrypted vaults are stored as ciphertext.
    r#"
    CREATE TABLE folders (
        id         BLOB PRIMARY KEY,
        vault_id   BLOB NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
        parent_id  BLOB REFERENCES folders(id) ON DELETE CASCADE,
        name       TEXT NOT NULL,
        cipher     BLOB,
        created_at INTEGER NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX folders_vault_idx ON folders(vault_id, parent_id);
    ALTER TABLE notes ADD COLUMN folder_id BLOB REFERENCES folders(id) ON DELETE SET NULL;
    CREATE INDEX notes_folder_idx ON notes(folder_id);
    "#,
    // v5: image attachments, stored in the database (sealed in encrypted vaults).
    r#"
    CREATE TABLE attachments (
        id         BLOB PRIMARY KEY,
        vault_id   BLOB NOT NULL REFERENCES vaults(id) ON DELETE CASCADE,
        name       TEXT NOT NULL,
        mime       TEXT NOT NULL,
        size       INTEGER NOT NULL,
        data       BLOB NOT NULL,
        encrypted  INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL
    ) WITHOUT ROWID;
    CREATE INDEX attachments_vault_idx ON attachments(vault_id);
    "#,
];

/// The Astralis local database plus in-memory state (unlocked vault keys).
pub struct Store {
    pub(crate) conn: Connection,
    pub(crate) vault_keys: HashMap<Uuid, Zeroizing<[u8; 32]>>,
    /// Sync state shared with the SQL change-tracking functions (clock, device id, apply flag).
    pub(crate) sync: Arc<SyncCtx>,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let sync = Arc::new(SyncCtx::default());
        crate::sync::register_sql_functions(&conn, &sync)?;
        let mut store = Self {
            conn,
            vault_keys: HashMap::new(),
            sync,
        };
        store.migrate()?;
        store.init_sync()?;
        store.ensure_default_vault()?;
        store.seed_builtin_templates()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let current: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        let all: Vec<String> = MIGRATIONS
            .iter()
            .map(|s| s.to_string())
            .chain(std::iter::once(crate::sync::sync_migration()))
            .collect();
        for (i, sql) in all.iter().enumerate().skip(current as usize) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }
}
