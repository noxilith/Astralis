use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::util::{Timestamp, now_ms, opt_bytes, opt_uuid_col, uuid_col};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteKind {
    /// A Markdown text note.
    #[default]
    Note,
    /// A visual canvas; `body` holds the scene JSON.
    Whiteboard,
}

impl NoteKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            NoteKind::Note => "note",
            NoteKind::Whiteboard => "whiteboard",
        }
    }
    pub(crate) fn parse(s: &str) -> Self {
        if s == "whiteboard" {
            NoteKind::Whiteboard
        } else {
            NoteKind::Note
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// UUIDv7: time-sortable and safe to generate offline on any device.
    pub id: Uuid,
    pub kind: NoteKind,
    pub title: String,
    pub body: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub pinned: bool,
    /// `Some` when archived; the value is when it was archived.
    pub archived_at: Option<Timestamp>,
    /// The vault (workspace) the note belongs to.
    pub vault_id: Uuid,
    /// Folder within the vault; `None` = the vault root.
    pub folder_id: Option<Uuid>,
    /// True when the note's vault is encrypted (content is stored as ciphertext).
    pub encrypted: bool,
    pub tags: Vec<String>,
}

impl Note {
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }
}

/// A lightweight pointer to a note, for lists of links, search hits, AI results, etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteRef {
    pub id: Uuid,
    pub title: String,
}

/// Input for creating a note. Everything is optional so a quick note is just `NewNote::default()`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NewNote {
    pub kind: NoteKind,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub pinned: bool,
    /// Defaults to the default vault.
    pub vault_id: Option<Uuid>,
    /// Folder to create the note in (must belong to the same vault).
    pub folder_id: Option<Uuid>,
}

/// Partial update; `None` leaves the field unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NotePatch {
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveFilter {
    /// Only notes that are not archived.
    #[default]
    Active,
    Archived,
    All,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListFilter {
    pub archive: ArchiveFilter,
    pub pinned_only: bool,
    /// Which vault to list; defaults to the default vault. Encrypted vaults must be unlocked.
    pub vault_id: Option<Uuid>,
    pub tag: Option<String>,
    pub kind: Option<NoteKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCount {
    pub name: String,
    pub count: i64,
}

pub(crate) const NOTE_COLUMNS: &str =
    "id, title, body, created_at, updated_at, pinned, archived_at, vault_id, cipher,
     (SELECT group_concat(t.name, char(31)) FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
      WHERE nt.note_id = notes.id), kind, folder_id";

/// A note row as stored: encrypted notes still carry their ciphertext.
pub(crate) struct RawNote {
    note: Note,
    cipher: Option<Vec<u8>>,
}

pub(crate) fn raw_note(row: &Row<'_>) -> rusqlite::Result<RawNote> {
    let tags: Option<String> = row.get(9)?;
    let mut tags: Vec<String> = tags
        .map(|t| t.split('\u{1f}').map(str::to_owned).collect())
        .unwrap_or_default();
    tags.sort();
    let cipher: Option<Vec<u8>> = row.get(8)?;
    let kind: String = row.get(10)?;
    Ok(RawNote {
        note: Note {
            id: uuid_col(row, 0)?,
            kind: NoteKind::parse(&kind),
            title: row.get(1)?,
            body: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
            pinned: row.get(5)?,
            archived_at: row.get(6)?,
            vault_id: uuid_col(row, 7)?,
            folder_id: opt_uuid_col(row, 11)?,
            encrypted: cipher.is_some(),
            tags,
        },
        cipher,
    })
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SecretContent {
    pub title: String,
    pub body: String,
}

pub(crate) fn normalize_tag(tag: &str) -> Option<String> {
    let t = tag.trim().trim_start_matches('#').trim().to_lowercase();
    let t: String = t
        .chars()
        .map(|c| if c.is_whitespace() { '-' } else { c })
        .collect();
    (!t.is_empty() && t.len() <= 64).then_some(t)
}

/// Minimal facts about a stored note, without decrypting it.
pub(crate) struct NoteMeta {
    pub encrypted: bool,
}

impl Store {
    pub(crate) fn hydrate(&self, raw: RawNote) -> Result<Note> {
        let RawNote { mut note, cipher } = raw;
        if let Some(cipher) = cipher {
            let plain = self.decrypt_for(note.vault_id, note.id, &cipher)?;
            let content: SecretContent = serde_json::from_slice(&plain)?;
            note.title = content.title;
            note.body = content.body;
        }
        Ok(note)
    }

    pub(crate) fn note_meta(&self, id: Uuid) -> Result<NoteMeta> {
        self.conn
            .query_row(
                "SELECT cipher IS NOT NULL FROM notes WHERE id = ?1",
                [id.as_bytes()],
                |r| {
                    Ok(NoteMeta {
                        encrypted: r.get(0)?,
                    })
                },
            )
            .optional()?
            .ok_or(Error::NotFound("note", id))
    }

    pub fn create_note(&self, new: NewNote) -> Result<Note> {
        let vault_id = match new.vault_id {
            Some(v) => v,
            None => self.default_vault()?,
        };
        let encrypted = self.vault_is_encrypted(vault_id)?;
        if let Some(folder) = new.folder_id {
            self.check_folder_in_vault(folder, vault_id)?;
        }
        let now = now_ms();
        let id = Uuid::now_v7();
        let body = if new.kind == NoteKind::Whiteboard && new.body.trim().is_empty() {
            "{}".to_owned()
        } else {
            new.body
        };
        let (title, body, cipher) = if encrypted {
            if !new.tags.is_empty() {
                return Err(Error::invalid("notes in encrypted vaults cannot have tags"));
            }
            let plain = serde_json::to_vec(&SecretContent {
                title: new.title,
                body,
            })?;
            (
                String::new(),
                String::new(),
                Some(self.encrypt_for(vault_id, id, &plain)?),
            )
        } else {
            (new.title, body, None)
        };
        self.conn.execute(
            "INSERT INTO notes (id, title, body, created_at, updated_at, pinned, archived_at, vault_id, cipher, kind, folder_id)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5, NULL, ?6, ?7, ?8, ?9)",
            params![id.as_bytes(), title, body, now, new.pinned, vault_id.as_bytes(), cipher, new.kind.as_str(), opt_bytes(new.folder_id)],
        )?;
        if !encrypted {
            self.set_tags(id, &new.tags)?;
            if new.kind == NoteKind::Note {
                self.sync_links(id, &body)?;
            }
        }
        self.get_note(id)
    }

    /// Creates a note from free text: the first line becomes the title.
    pub fn quick_capture(&self, vault_id: Option<Uuid>, text: &str) -> Result<Note> {
        let text = text.trim();
        let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
        let title = first.trim().trim_start_matches('#').trim();
        let title: String = title.chars().take(120).collect();
        self.create_note(NewNote {
            title,
            body: rest.trim_start().to_owned(),
            vault_id,
            ..Default::default()
        })
    }

    pub fn get_note(&self, id: Uuid) -> Result<Note> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1"),
                [id.as_bytes()],
                raw_note,
            )
            .optional()?
            .ok_or(Error::NotFound("note", id))?;
        self.hydrate(raw)
    }

    pub fn update_note(&self, id: Uuid, patch: NotePatch) -> Result<Note> {
        let current = self.get_note(id)?;
        let title = patch.title.unwrap_or_else(|| current.title.clone());
        let body = patch.body.unwrap_or_else(|| current.body.clone());
        if title == current.title && body == current.body {
            return Ok(current);
        }
        self.write_content(&current, title, body, true)
    }

    /// Writes new content for a note, handling history, encryption, link sync and renames.
    pub(crate) fn write_content(
        &self,
        current: &Note,
        title: String,
        body: String,
        snapshot: bool,
    ) -> Result<Note> {
        let now = now_ms();
        if current.encrypted {
            let plain = serde_json::to_vec(&SecretContent { title, body })?;
            let cipher = self.encrypt_for(current.vault_id, current.id, &plain)?;
            self.conn.execute(
                "UPDATE notes SET cipher = ?2, updated_at = ?3 WHERE id = ?1",
                params![current.id.as_bytes(), cipher, now],
            )?;
        } else {
            if snapshot {
                self.maybe_snapshot(current, now)?;
            }
            if title != current.title && !current.title.trim().is_empty() {
                self.rewrite_links(current.vault_id, &current.title, &title, current.id)?;
            }
            self.conn.execute(
                "UPDATE notes SET title = ?2, body = ?3, updated_at = ?4 WHERE id = ?1",
                params![current.id.as_bytes(), title, body, now],
            )?;
            if current.kind == NoteKind::Note {
                self.sync_links(current.id, &body)?;
            }
        }
        self.get_note(current.id)
    }

    /// Pinned notes first, then most recently updated.
    pub fn list_notes(&self, filter: &ListFilter) -> Result<Vec<Note>> {
        let vault = match filter.vault_id {
            Some(v) => v,
            None => self.default_vault()?,
        };
        if self.vault_is_encrypted(vault)? {
            self.require_unlocked(vault)?;
        }
        let mut clauses: Vec<String> = vec!["vault_id = ?1".into()];
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(vault.as_bytes().to_vec())];
        match filter.archive {
            ArchiveFilter::Active => clauses.push("archived_at IS NULL".into()),
            ArchiveFilter::Archived => clauses.push("archived_at IS NOT NULL".into()),
            ArchiveFilter::All => {}
        }
        if filter.pinned_only {
            clauses.push("pinned = 1".into());
        }
        if let Some(kind) = filter.kind {
            args.push(Box::new(kind.as_str()));
            clauses.push(format!("kind = ?{}", args.len()));
        }
        if let Some(tag) = filter.tag.as_deref().and_then(normalize_tag) {
            args.push(Box::new(tag));
            clauses.push(format!(
                "id IN (SELECT nt.note_id FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE t.name = ?{})",
                args.len()
            ));
        }
        let sql = format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE {} ORDER BY pinned DESC, updated_at DESC, id DESC",
            clauses.join(" AND ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let raws = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), raw_note)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        raws.into_iter().map(|r| self.hydrate(r)).collect()
    }

    pub fn set_pinned(&self, id: Uuid, pinned: bool) -> Result<Note> {
        self.exec_note_flag(
            "UPDATE notes SET pinned = ?2 WHERE id = ?1",
            params![id.as_bytes(), pinned],
            id,
        )
    }

    pub fn archive_note(&self, id: Uuid) -> Result<Note> {
        self.exec_note_flag(
            "UPDATE notes SET archived_at = COALESCE(archived_at, ?2) WHERE id = ?1",
            params![id.as_bytes(), now_ms()],
            id,
        )
    }

    pub fn unarchive_note(&self, id: Uuid) -> Result<Note> {
        self.exec_note_flag(
            "UPDATE notes SET archived_at = NULL WHERE id = ?1",
            params![id.as_bytes()],
            id,
        )
    }

    /// Permanently removes a note and its history. Prefer `archive_note` for anything recoverable.
    pub fn delete_note(&self, id: Uuid) -> Result<()> {
        let n = self
            .conn
            .execute("DELETE FROM notes WHERE id = ?1", [id.as_bytes()])?;
        if n == 0 {
            Err(Error::NotFound("note", id))
        } else {
            Ok(())
        }
    }

    fn exec_note_flag(&self, sql: &str, params: impl rusqlite::Params, id: Uuid) -> Result<Note> {
        if self.conn.execute(sql, params)? == 0 {
            return Err(Error::NotFound("note", id));
        }
        self.get_note(id)
    }

    // ---- tags ----

    /// Replaces a note's tags. Tags are normalized to lowercase, `#` stripped, spaces → `-`.
    pub fn set_tags(&self, id: Uuid, tags: &[String]) -> Result<Note> {
        if self.note_meta(id)?.encrypted {
            if tags.is_empty() {
                return self.get_note(id);
            }
            return Err(Error::invalid("notes in encrypted vaults cannot have tags"));
        }
        let mut names: Vec<String> = tags.iter().filter_map(|t| normalize_tag(t)).collect();
        names.sort();
        names.dedup();
        self.conn
            .execute("DELETE FROM note_tags WHERE note_id = ?1", [id.as_bytes()])?;
        for name in &names {
            self.conn
                .execute("INSERT OR IGNORE INTO tags(name) VALUES (?1)", [name])?;
            self.conn.execute(
                "INSERT OR IGNORE INTO note_tags(note_id, tag_id) SELECT ?1, id FROM tags WHERE name = ?2",
                params![id.as_bytes(), name],
            )?;
        }
        self.prune_tags()?;
        self.get_note(id)
    }

    pub(crate) fn prune_tags(&self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM note_tags)",
            [],
        )?;
        Ok(())
    }

    pub fn add_tag(&self, id: Uuid, tag: &str) -> Result<Note> {
        let mut tags = self.get_note(id)?.tags;
        tags.push(tag.to_owned());
        self.set_tags(id, &tags)
    }

    pub fn remove_tag(&self, id: Uuid, tag: &str) -> Result<Note> {
        let target = normalize_tag(tag);
        let tags: Vec<String> = self
            .get_note(id)?
            .tags
            .into_iter()
            .filter(|t| Some(t) != target.as_ref())
            .collect();
        self.set_tags(id, &tags)
    }

    /// Tags in use on a vault's active notes, most used first.
    pub fn list_tags(&self, vault_id: Option<Uuid>) -> Result<Vec<TagCount>> {
        let vault = match vault_id {
            Some(v) => v,
            None => self.default_vault()?,
        };
        let mut stmt = self.conn.prepare(
            "SELECT t.name, COUNT(*) FROM tags t
             JOIN note_tags nt ON nt.tag_id = t.id
             JOIN notes n ON n.id = nt.note_id AND n.archived_at IS NULL AND n.vault_id = ?1
             GROUP BY t.id ORDER BY COUNT(*) DESC, t.name",
        )?;
        let rows = stmt.query_map([vault.as_bytes()], |r| {
            Ok(TagCount {
                name: r.get(0)?,
                count: r.get(1)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
