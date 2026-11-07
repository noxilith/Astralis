//! Combining several notes into one.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notes::{NewNote, Note, NoteKind};
use crate::store::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRequest {
    /// Notes to merge, in the order their content should appear.
    pub note_ids: Vec<Uuid>,
    /// Title for the merged note; defaults to the first note's title.
    #[serde(default)]
    pub title: Option<String>,
    /// Archive the originals (default) instead of leaving them untouched. Never deletes.
    #[serde(default = "yes")]
    pub archive_originals: bool,
}

fn yes() -> bool {
    true
}

impl Store {
    /// Creates a new note with every note's content under its own heading, the union of their
    /// tags, pinned if any were pinned. Links pointing at the originals are redirected to it.
    /// All notes must be text notes in the same plain vault.
    pub fn merge_notes(&self, req: MergeRequest) -> Result<Note> {
        if req.note_ids.len() < 2 {
            return Err(Error::invalid("pick at least two notes to merge"));
        }
        let notes: Vec<Note> = req
            .note_ids
            .iter()
            .map(|id| self.get_note(*id))
            .collect::<Result<_>>()?;
        if notes.iter().any(|n| n.encrypted) {
            return Err(Error::invalid("notes in encrypted vaults cannot be merged"));
        }
        if notes.iter().any(|n| n.kind != NoteKind::Note) {
            return Err(Error::invalid("only text notes can be merged"));
        }
        let vault = notes[0].vault_id;
        if notes.iter().any(|n| n.vault_id != vault) {
            return Err(Error::invalid("notes must be in the same vault"));
        }
        let title = req
            .title
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| notes[0].title.clone());

        let mut body = String::new();
        for n in &notes {
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            let heading = if n.title.trim().is_empty() {
                "Untitled"
            } else {
                n.title.trim()
            };
            body.push_str(&format!("## {heading}\n\n{}", n.body.trim_end()));
        }
        let mut tags: Vec<String> = notes.iter().flat_map(|n| n.tags.clone()).collect();
        tags.sort();
        tags.dedup();

        let merged = self.create_note(NewNote {
            kind: NoteKind::Note,
            title: title.clone(),
            body,
            tags,
            pinned: notes.iter().any(|n| n.pinned),
            vault_id: Some(vault),
            folder_id: notes[0].folder_id,
        })?;
        for n in &notes {
            if !n.title.trim().is_empty()
                && n.title.trim().to_lowercase() != title.trim().to_lowercase()
            {
                self.rewrite_links(vault, &n.title, &title, merged.id)?;
            }
            if req.archive_originals {
                self.archive_note(n.id)?;
            }
        }
        self.get_note(merged.id)
    }
}
