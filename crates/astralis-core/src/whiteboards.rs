//! Whiteboards are notes of kind `Whiteboard` whose body is the scene JSON (owned by the
//! frontend). They get pinning, archiving, tags, vaults and encryption from notes for free.

use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notes::{NewNote, Note, NoteKind};
use crate::store::Store;

/// Scenes larger than this are rejected (embedded images should stay reasonable).
pub const MAX_SCENE_BYTES: usize = 32 * 1024 * 1024;

impl Store {
    pub fn create_whiteboard(&self, vault_id: Option<Uuid>, title: &str) -> Result<Note> {
        let title = if title.trim().is_empty() {
            "Untitled whiteboard"
        } else {
            title.trim()
        };
        self.create_note(NewNote {
            kind: NoteKind::Whiteboard,
            title: title.to_owned(),
            body: "{}".into(),
            vault_id,
            ..Default::default()
        })
    }

    /// Saves a whiteboard's scene (no history snapshot; scenes are large and saved often).
    pub fn save_whiteboard_scene(&self, id: Uuid, scene: &str) -> Result<()> {
        if scene.len() > MAX_SCENE_BYTES {
            return Err(Error::invalid("whiteboard is too large"));
        }
        serde_json::from_str::<serde_json::Value>(scene)?;
        let current = self.get_note(id)?;
        if current.kind != NoteKind::Whiteboard {
            return Err(Error::invalid("not a whiteboard"));
        }
        if current.body != scene {
            self.write_content(&current, current.title.clone(), scene.to_owned(), false)?;
        }
        Ok(())
    }
}
