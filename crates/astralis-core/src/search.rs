//! Full-text search within a plain vault, using SQLite FTS5. Whiteboards match by title.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::Result;
use crate::notes::NoteKind;
use crate::store::Store;
use crate::util::{Timestamp, uuid_col};

/// Marks the start of a highlighted match inside `SearchHit::snippet`.
pub const HIGHLIGHT_START: char = '\u{2}';
/// Marks the end of a highlighted match inside `SearchHit::snippet`.
pub const HIGHLIGHT_END: char = '\u{3}';

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: Uuid,
    pub kind: NoteKind,
    pub title: String,
    /// Body excerpt; matches are wrapped in `\u{2}` … `\u{3}` (plain text, never HTML).
    pub snippet: String,
    pub archived: bool,
    pub updated_at: Timestamp,
    pub score: f64,
}

/// Turns free text into a safe FTS5 query: every word must match, the last as a prefix.
fn fts_query(input: &str) -> Option<String> {
    let words: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{w}\""))
        .collect();
    if words.is_empty() {
        return None;
    }
    let mut q = words.join(" ");
    q.push('*');
    Some(q)
}

impl Store {
    pub fn search(
        &self,
        vault: Option<Uuid>,
        query: &str,
        include_archived: bool,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let Some(q) = fts_query(query) else {
            return Ok(vec![]);
        };
        let vault = match vault {
            Some(v) => v,
            None => self.default_vault()?,
        };
        let mut stmt = self.conn.prepare_cached(
            "SELECT n.id, n.title,
                    snippet(notes_fts, 2, char(2), char(3), '…', 16),
                    n.archived_at IS NOT NULL, n.updated_at,
                    bm25(notes_fts, 0.0, 5.0, 1.0), n.kind
             FROM notes_fts JOIN notes n ON n.id = notes_fts.note_id
             WHERE notes_fts MATCH ?1 AND n.vault_id = ?4 AND n.cipher IS NULL
               AND (?2 OR n.archived_at IS NULL)
             ORDER BY bm25(notes_fts, 0.0, 5.0, 1.0) LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![q, include_archived, limit as i64, vault.as_bytes()],
            |r| {
                let kind: String = r.get(6)?;
                Ok(SearchHit {
                    id: uuid_col(r, 0)?,
                    kind: NoteKind::parse(&kind),
                    title: r.get(1)?,
                    snippet: r.get(2)?,
                    archived: r.get(3)?,
                    updated_at: r.get(4)?,
                    score: -r.get::<_, f64>(5)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
