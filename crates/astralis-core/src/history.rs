//! Note history: automatic snapshots before edits, browsing and restoring versions.
//!
//! Only text notes in plain vaults keep history — plaintext snapshots would defeat
//! encryption, and whiteboard scenes are too large to snapshot on every save.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notes::{Note, NoteKind};
use crate::store::Store;
use crate::util::{Timestamp, now_ms, uuid_col};

/// Edits closer together than this are folded into one version (autosave while typing).
pub const SNAPSHOT_INTERVAL_MS: i64 = 5 * 60 * 1000;
/// Oldest versions beyond this count are pruned per note.
pub const MAX_VERSIONS_PER_NOTE: i64 = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteVersion {
    pub id: Uuid,
    pub note_id: Uuid,
    pub title: String,
    pub body: String,
    pub created_at: Timestamp,
}

fn keeps_history(note: &Note) -> bool {
    !note.encrypted && note.kind == NoteKind::Note
}

impl Store {
    /// Saves the note's current content as a version unless one was saved very recently.
    pub(crate) fn maybe_snapshot(&self, note: &Note, now: Timestamp) -> Result<()> {
        if !keeps_history(note) || (note.title.is_empty() && note.body.is_empty()) {
            return Ok(());
        }
        let last: Option<Timestamp> = self
            .conn
            .query_row(
                "SELECT MAX(created_at) FROM note_versions WHERE note_id = ?1",
                [note.id.as_bytes()],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if last.is_some_and(|t| now - t < SNAPSHOT_INTERVAL_MS) {
            return Ok(());
        }
        self.insert_version(note, now)
    }

    fn insert_version(&self, note: &Note, now: Timestamp) -> Result<()> {
        self.conn.execute(
            "INSERT INTO note_versions(id, note_id, title, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![Uuid::now_v7().as_bytes(), note.id.as_bytes(), note.title, note.body, now],
        )?;
        self.conn.execute(
            "DELETE FROM note_versions WHERE note_id = ?1 AND id NOT IN (
                SELECT id FROM note_versions WHERE note_id = ?1 ORDER BY created_at DESC LIMIT ?2)",
            params![note.id.as_bytes(), MAX_VERSIONS_PER_NOTE],
        )?;
        Ok(())
    }

    /// Explicitly saves the current content as a version (e.g. a "Save version" button).
    pub fn snapshot_note(&self, id: Uuid) -> Result<()> {
        let note = self.get_note(id)?;
        if !keeps_history(&note) {
            return Err(Error::invalid("this note doesn't keep history"));
        }
        self.insert_version(&note, now_ms())
    }

    /// Versions, newest first.
    pub fn note_history(&self, id: Uuid) -> Result<Vec<NoteVersion>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, note_id, title, body, created_at FROM note_versions
             WHERE note_id = ?1 ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([id.as_bytes()], |r| {
            Ok(NoteVersion {
                id: uuid_col(r, 0)?,
                note_id: uuid_col(r, 1)?,
                title: r.get(2)?,
                body: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Restores a version. The current content is saved as a version first, so restores are undoable.
    pub fn restore_version(&self, version_id: Uuid) -> Result<Note> {
        let (note_id, title, body): (Uuid, String, String) = self
            .conn
            .query_row(
                "SELECT note_id, title, body FROM note_versions WHERE id = ?1",
                [version_id.as_bytes()],
                |r| Ok((uuid_col(r, 0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or(Error::NotFound("version", version_id))?;
        let current = self.get_note(note_id)?;
        self.insert_version(&current, now_ms())?;
        self.write_content(&current, title, body, false)
    }
}
