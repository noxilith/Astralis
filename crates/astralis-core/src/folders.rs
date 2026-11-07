//! Folders: an unlimited-depth tree inside each vault for organizing notes and whiteboards.
//! In encrypted vaults folder names are encrypted too (a name like "Medical" leaks a lot).

use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::notes::Note;
use crate::store::Store;
use crate::util::{Timestamp, now_ms, opt_bytes, opt_uuid_col, uuid_col};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: Uuid,
    pub vault_id: Uuid,
    /// `None` = top level of the vault.
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub created_at: Timestamp,
}

const COLS: &str = "id, vault_id, parent_id, name, cipher, created_at";

struct RawFolder {
    folder: Folder,
    cipher: Option<Vec<u8>>,
}

fn raw_folder(r: &Row<'_>) -> rusqlite::Result<RawFolder> {
    Ok(RawFolder {
        folder: Folder {
            id: uuid_col(r, 0)?,
            vault_id: uuid_col(r, 1)?,
            parent_id: opt_uuid_col(r, 2)?,
            name: r.get(3)?,
            created_at: r.get(5)?,
        },
        cipher: r.get(4)?,
    })
}

fn clean_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::invalid("folder needs a name"));
    }
    if name.chars().count() > 200 {
        return Err(Error::invalid("folder name is too long"));
    }
    Ok(name)
}

impl Store {
    fn hydrate_folder(&self, raw: RawFolder) -> Result<Folder> {
        let RawFolder { mut folder, cipher } = raw;
        if let Some(c) = cipher {
            let plain = self.decrypt_for(folder.vault_id, folder.id, &c)?;
            folder.name = String::from_utf8(plain.to_vec())
                .map_err(|_| Error::Crypto("corrupt folder name".into()))?;
        }
        Ok(folder)
    }

    /// Returns (stored name, ciphertext) for a folder name in a vault.
    fn seal_name(
        &self,
        vault: Uuid,
        folder: Uuid,
        name: &str,
    ) -> Result<(String, Option<Vec<u8>>)> {
        if self.vault_is_encrypted(vault)? {
            Ok((
                String::new(),
                Some(self.encrypt_for(vault, folder, name.as_bytes())?),
            ))
        } else {
            Ok((name.to_owned(), None))
        }
    }

    pub(crate) fn check_folder_in_vault(&self, folder: Uuid, vault: Uuid) -> Result<()> {
        let v: Uuid = self
            .conn
            .query_row(
                "SELECT vault_id FROM folders WHERE id = ?1",
                [folder.as_bytes()],
                |r| uuid_col(r, 0),
            )
            .optional()?
            .ok_or(Error::NotFound("folder", folder))?;
        if v != vault {
            return Err(Error::invalid("folder belongs to another vault"));
        }
        Ok(())
    }

    pub fn create_folder(
        &self,
        vault_id: Option<Uuid>,
        parent_id: Option<Uuid>,
        name: &str,
    ) -> Result<Folder> {
        let vault = match vault_id {
            Some(v) => v,
            None => self.default_vault()?,
        };
        if let Some(p) = parent_id {
            self.check_folder_in_vault(p, vault)?;
        }
        let name = clean_name(name)?;
        let id = Uuid::now_v7();
        let (stored, cipher) = self.seal_name(vault, id, name)?;
        self.conn.execute(
            &format!("INSERT INTO folders ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6)"),
            params![
                id.as_bytes(),
                vault.as_bytes(),
                opt_bytes(parent_id),
                stored,
                cipher,
                now_ms()
            ],
        )?;
        self.get_folder(id)
    }

    pub fn get_folder(&self, id: Uuid) -> Result<Folder> {
        let raw = self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM folders WHERE id = ?1"),
                [id.as_bytes()],
                raw_folder,
            )
            .optional()?
            .ok_or(Error::NotFound("folder", id))?;
        self.hydrate_folder(raw)
    }

    /// All folders in a vault (flat; build the tree from `parent_id`), sorted by name.
    pub fn list_folders(&self, vault_id: Option<Uuid>) -> Result<Vec<Folder>> {
        let vault = match vault_id {
            Some(v) => v,
            None => self.default_vault()?,
        };
        if self.vault_is_encrypted(vault)? {
            self.require_unlocked(vault)?;
        }
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLS} FROM folders WHERE vault_id = ?1"))?;
        let raws = stmt
            .query_map([vault.as_bytes()], raw_folder)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut folders: Vec<Folder> = raws
            .into_iter()
            .map(|r| self.hydrate_folder(r))
            .collect::<Result<_>>()?;
        folders.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        Ok(folders)
    }

    pub fn rename_folder(&self, id: Uuid, name: &str) -> Result<Folder> {
        let folder = self.get_folder(id)?;
        let (stored, cipher) = self.seal_name(folder.vault_id, id, clean_name(name)?)?;
        self.conn.execute(
            "UPDATE folders SET name = ?2, cipher = ?3 WHERE id = ?1",
            params![id.as_bytes(), stored, cipher],
        )?;
        self.get_folder(id)
    }

    /// Moves a folder under another folder (or to the top level). Refuses to create cycles.
    pub fn move_folder(&self, id: Uuid, new_parent: Option<Uuid>) -> Result<Folder> {
        let folder = self.get_folder(id)?;
        if let Some(p) = new_parent {
            self.check_folder_in_vault(p, folder.vault_id)?;
            let mut cursor = Some(p);
            while let Some(c) = cursor {
                if c == id {
                    return Err(Error::invalid("can't move a folder into itself"));
                }
                cursor = self.conn.query_row(
                    "SELECT parent_id FROM folders WHERE id = ?1",
                    [c.as_bytes()],
                    |r| opt_uuid_col(r, 0),
                )?;
            }
        }
        self.conn.execute(
            "UPDATE folders SET parent_id = ?2 WHERE id = ?1",
            params![id.as_bytes(), opt_bytes(new_parent)],
        )?;
        self.get_folder(id)
    }

    /// Deletes a folder. Nothing inside is lost: its notes and subfolders move up to its parent.
    pub fn delete_folder(&self, id: Uuid) -> Result<()> {
        let parent: Option<Uuid> = self
            .conn
            .query_row(
                "SELECT parent_id FROM folders WHERE id = ?1",
                [id.as_bytes()],
                |r| opt_uuid_col(r, 0),
            )
            .optional()?
            .ok_or(Error::NotFound("folder", id))?;
        self.conn.execute(
            "UPDATE notes SET folder_id = ?2 WHERE folder_id = ?1",
            params![id.as_bytes(), opt_bytes(parent)],
        )?;
        self.conn.execute(
            "UPDATE folders SET parent_id = ?2 WHERE parent_id = ?1",
            params![id.as_bytes(), opt_bytes(parent)],
        )?;
        self.conn
            .execute("DELETE FROM folders WHERE id = ?1", [id.as_bytes()])?;
        Ok(())
    }

    /// Files a note into a folder of its vault (or the vault root with `None`).
    pub fn move_note_to_folder(&self, note_id: Uuid, folder: Option<Uuid>) -> Result<Note> {
        let vault: Uuid = self
            .conn
            .query_row(
                "SELECT vault_id FROM notes WHERE id = ?1",
                [note_id.as_bytes()],
                |r| uuid_col(r, 0),
            )
            .optional()?
            .ok_or(Error::NotFound("note", note_id))?;
        if let Some(f) = folder {
            self.check_folder_in_vault(f, vault)?;
        }
        self.conn.execute(
            "UPDATE notes SET folder_id = ?2 WHERE id = ?1",
            params![note_id.as_bytes(), opt_bytes(folder)],
        )?;
        self.get_note(note_id)
    }
}
