//! Image attachments. Notes reference them as `![name|width](attachments/<id>.<ext>)`
//! (Obsidian-style width). Bytes live in the database; in encrypted vaults they're sealed
//! with the vault key and the original file name isn't kept.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::util::{Timestamp, now_ms, uuid_col};

/// Largest image accepted (bytes).
pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: Uuid,
    pub vault_id: Uuid,
    pub name: String,
    pub mime: String,
    pub size: i64,
    /// What goes in the note: `attachments/<id>.<ext>`.
    pub path: String,
    pub created_at: Timestamp,
}

fn extension(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/avif" => "avif",
        "image/bmp" => "bmp",
        _ => return None,
    })
}

pub fn attachment_path(id: Uuid, mime: &str) -> String {
    format!("attachments/{id}.{}", extension(mime).unwrap_or("bin"))
}

/// Attachment ids referenced by a note body (`attachments/<uuid>.<ext>`).
pub fn attachment_ids_in(body: &str) -> Vec<Uuid> {
    let mut out = Vec::new();
    for (i, _) in body.match_indices("attachments/") {
        let rest = &body[i + "attachments/".len()..];
        if let Some(id) = rest.get(..36).and_then(|s| Uuid::parse_str(s).ok()) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

impl Store {
    /// Stores an image in a vault and returns its reference.
    pub fn add_attachment(
        &self,
        vault_id: Option<Uuid>,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<Attachment> {
        let vault = match vault_id {
            Some(v) => v,
            None => self.default_vault()?,
        };
        if extension(mime).is_none() {
            return Err(Error::invalid(format!("unsupported image type: {mime}")));
        }
        if bytes.is_empty() {
            return Err(Error::invalid("the image is empty"));
        }
        if bytes.len() > MAX_ATTACHMENT_BYTES {
            return Err(Error::invalid("images can be at most 25 MB"));
        }
        let id = Uuid::now_v7();
        let encrypted = self.vault_is_encrypted(vault)?;
        let (stored_name, data) = if encrypted {
            (String::new(), self.encrypt_for(vault, id, bytes)?)
        } else {
            let name: String = name.trim().chars().take(200).collect();
            (name, bytes.to_vec())
        };
        self.conn.execute(
            "INSERT INTO attachments(id, vault_id, name, mime, size, data, encrypted, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id.as_bytes(),
                vault.as_bytes(),
                stored_name,
                mime,
                bytes.len() as i64,
                data,
                encrypted,
                now_ms()
            ],
        )?;
        Ok(Attachment {
            id,
            vault_id: vault,
            name: name.trim().to_owned(),
            mime: mime.to_owned(),
            size: bytes.len() as i64,
            path: attachment_path(id, mime),
            created_at: now_ms(),
        })
    }

    /// Returns `(mime, bytes)`, decrypting if needed (the vault must be unlocked).
    pub fn get_attachment(&self, id: Uuid) -> Result<(String, Vec<u8>)> {
        let (vault, mime, data, encrypted): (Uuid, String, Vec<u8>, bool) = self
            .conn
            .query_row(
                "SELECT vault_id, mime, data, encrypted FROM attachments WHERE id = ?1",
                [id.as_bytes()],
                |r| Ok((uuid_col(r, 0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .ok_or(Error::NotFound("attachment", id))?;
        if encrypted {
            Ok((mime, self.decrypt_for(vault, id, &data)?.to_vec()))
        } else {
            Ok((mime, data))
        }
    }

    /// Moves the images a note uses into `target`, re-sealing or unsealing them to match it.
    pub(crate) fn rehome_attachments(&self, body: &str, target: Uuid) -> Result<()> {
        let target_encrypted = self.vault_is_encrypted(target)?;
        for id in attachment_ids_in(body) {
            let Ok((mime, bytes)) = self.get_attachment(id) else {
                continue;
            };
            let data = if target_encrypted {
                self.encrypt_for(target, id, &bytes)?
            } else {
                bytes
            };
            self.conn.execute(
                "UPDATE attachments SET vault_id = ?2, data = ?3, encrypted = ?4,
                    name = CASE WHEN ?4 THEN '' ELSE name END, mime = ?5 WHERE id = ?1",
                params![
                    id.as_bytes(),
                    target.as_bytes(),
                    data,
                    target_encrypted,
                    mime
                ],
            )?;
        }
        Ok(())
    }
}
