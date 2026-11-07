//! Vaults: separate workspaces of notes. A vault is either plain or encrypted.
//!
//! Encrypted vaults:
//! - Have a random 256-bit data key (DEK). Note content is encrypted with XChaCha20-Poly1305
//!   under the DEK, with the note id as associated data (ciphertext can't be swapped between
//!   notes).
//! - Store the DEK wrapped by a key derived from the password with Argon2id. Changing the
//!   password only re-wraps the DEK; biometric unlock can later wrap the same DEK with an OS
//!   keychain key.
//! - Keep unlocked DEKs only in memory (zeroized on lock/drop).
//! - Have no tags, links, history, search index or AI embeddings — all would leak plaintext.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::notes::{Note, NoteKind, SecretContent};
use crate::store::Store;
use crate::util::{Timestamp, now_ms, uuid_col};

const NONCE_LEN: usize = 24;
// Cheap parameters in tests only; Argon2id at 64 MiB is deliberately slow.
const KDF_M_KIB: u32 = if cfg!(test) { 1024 } else { 64 * 1024 };
const KDF_T: u32 = if cfg!(test) { 1 } else { 3 };
const KDF_P: u32 = 1;
const MIN_PASSWORD_LEN: usize = 8;
pub const DEFAULT_VAULT_NAME: &str = "Personal";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vault {
    pub id: Uuid,
    pub name: String,
    pub created_at: Timestamp,
    pub encrypted: bool,
    /// Plain vaults are always unlocked.
    pub unlocked: bool,
    pub note_count: i64,
}

fn random<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).map_err(|e| Error::Crypto(e.to_string()))?;
    Ok(buf)
}

fn derive_kek(password: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(m, t, p, Some(32)).map_err(|e| Error::Crypto(e.to_string()))?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, out.as_mut())
        .map_err(|e| Error::Crypto(e.to_string()))?;
    Ok(out)
}

fn check_password(password: &str) -> Result<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(Error::invalid(format!(
            "password must be at least {MIN_PASSWORD_LEN} characters"
        )));
    }
    Ok(())
}

/// Encrypts `plain` → `nonce || ciphertext`.
pub(crate) fn seal(key: &[u8; 32], aad: &[u8], plain: &[u8]) -> Result<Vec<u8>> {
    let cipher =
        XChaCha20Poly1305::new_from_slice(key).map_err(|e| Error::Crypto(e.to_string()))?;
    let nonce_bytes = random::<NONCE_LEN>()?;
    let nonce =
        XNonce::try_from(&nonce_bytes[..]).map_err(|_| Error::Crypto("bad nonce".into()))?;
    let ct = cipher
        .encrypt(&nonce, Payload { msg: plain, aad })
        .map_err(|_| Error::Crypto("encryption failed".into()))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub(crate) fn open(key: &[u8; 32], aad: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if sealed.len() < NONCE_LEN {
        return Err(Error::Crypto("ciphertext too short".into()));
    }
    let cipher =
        XChaCha20Poly1305::new_from_slice(key).map_err(|e| Error::Crypto(e.to_string()))?;
    let nonce =
        XNonce::try_from(&sealed[..NONCE_LEN]).map_err(|_| Error::Crypto("bad nonce".into()))?;
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &sealed[NONCE_LEN..],
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| Error::Crypto("decryption failed".into()))
}

impl Store {
    /// Makes sure at least one plain vault exists and every note belongs to a vault.
    pub(crate) fn ensure_default_vault(&self) -> Result<()> {
        let orphans: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM notes WHERE vault_id IS NULL",
            [],
            |r| r.get(0),
        )?;
        let vaults: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM vaults", [], |r| r.get(0))?;
        let plain: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT id FROM vaults WHERE encrypted = 0 ORDER BY created_at LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let plain = match plain {
            Some(id) => id,
            None if vaults == 0 || orphans > 0 => {
                let id = Uuid::now_v7();
                self.insert_plain_vault(id, DEFAULT_VAULT_NAME)?;
                id.as_bytes().to_vec()
            }
            None => return Ok(()),
        };
        if orphans > 0 {
            self.conn.execute(
                "UPDATE notes SET vault_id = ?1 WHERE vault_id IS NULL",
                [plain],
            )?;
        }
        Ok(())
    }

    fn insert_plain_vault(&self, id: Uuid, name: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO vaults(id, name, salt, kdf_m, kdf_t, kdf_p, wrapped_key, created_at, encrypted)
             VALUES (?1, ?2, x'', 0, 0, 0, x'', ?3, 0)",
            params![id.as_bytes(), name, now_ms()],
        )?;
        Ok(())
    }

    /// The vault used when none is specified: the oldest plain vault (or oldest vault).
    pub fn default_vault(&self) -> Result<Uuid> {
        self.conn
            .query_row(
                "SELECT id FROM vaults ORDER BY encrypted, created_at LIMIT 1",
                [],
                |r| uuid_col(r, 0),
            )
            .optional()?
            .ok_or_else(|| Error::invalid("there are no vaults"))
    }

    pub(crate) fn vault_is_encrypted(&self, id: Uuid) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT encrypted FROM vaults WHERE id = ?1",
                [id.as_bytes()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound("vault", id))
    }

    pub(crate) fn require_unlocked(&self, vault_id: Uuid) -> Result<&[u8; 32]> {
        self.vault_keys
            .get(&vault_id)
            .map(|k| &**k)
            .ok_or(Error::VaultLocked(vault_id))
    }

    pub(crate) fn encrypt_for(
        &self,
        vault_id: Uuid,
        note_id: Uuid,
        plain: &[u8],
    ) -> Result<Vec<u8>> {
        seal(self.require_unlocked(vault_id)?, note_id.as_bytes(), plain)
    }

    pub(crate) fn decrypt_for(
        &self,
        vault_id: Uuid,
        note_id: Uuid,
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>> {
        open(self.require_unlocked(vault_id)?, note_id.as_bytes(), sealed)
    }

    /// Creates a vault. With a password it is encrypted (and left unlocked).
    pub fn create_vault(&mut self, name: &str, password: Option<&str>) -> Result<Vault> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::invalid("vault needs a name"));
        }
        let id = Uuid::now_v7();
        match password {
            None => self.insert_plain_vault(id, name)?,
            Some(password) => {
                check_password(password)?;
                let salt = random::<16>()?;
                let dek = Zeroizing::new(random::<32>()?);
                let kek = derive_kek(password, &salt, KDF_M_KIB, KDF_T, KDF_P)?;
                let wrapped = seal(&kek, id.as_bytes(), dek.as_ref())?;
                self.conn.execute(
                    "INSERT INTO vaults(id, name, salt, kdf_m, kdf_t, kdf_p, wrapped_key, created_at, encrypted)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1)",
                    params![id.as_bytes(), name, &salt[..], KDF_M_KIB, KDF_T, KDF_P, wrapped, now_ms()],
                )?;
                self.vault_keys.insert(id, dek);
            }
        }
        self.get_vault(id)
    }

    pub fn list_vaults(&self) -> Result<Vec<Vault>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.id, v.name, v.created_at, v.encrypted,
                    (SELECT COUNT(*) FROM notes n WHERE n.vault_id = v.id AND n.archived_at IS NULL)
             FROM vaults v ORDER BY v.encrypted, v.created_at",
        )?;
        let rows = stmt.query_map([], |r| {
            let id = uuid_col(r, 0)?;
            let encrypted: bool = r.get(3)?;
            Ok(Vault {
                id,
                name: r.get(1)?,
                created_at: r.get(2)?,
                encrypted,
                unlocked: !encrypted || self.vault_keys.contains_key(&id),
                note_count: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_vault(&self, id: Uuid) -> Result<Vault> {
        self.list_vaults()?
            .into_iter()
            .find(|v| v.id == id)
            .ok_or(Error::NotFound("vault", id))
    }

    pub fn rename_vault(&self, id: Uuid, name: &str) -> Result<Vault> {
        if name.trim().is_empty() {
            return Err(Error::invalid("vault needs a name"));
        }
        if self.conn.execute(
            "UPDATE vaults SET name = ?2 WHERE id = ?1",
            params![id.as_bytes(), name.trim()],
        )? == 0
        {
            return Err(Error::NotFound("vault", id));
        }
        self.get_vault(id)
    }

    fn unwrap_dek(&self, id: Uuid, password: &str) -> Result<Zeroizing<[u8; 32]>> {
        if !self.vault_is_encrypted(id)? {
            return Err(Error::invalid("vault is not encrypted"));
        }
        let (salt, m, t, p, wrapped): (Vec<u8>, u32, u32, u32, Vec<u8>) = self.conn.query_row(
            "SELECT salt, kdf_m, kdf_t, kdf_p, wrapped_key FROM vaults WHERE id = ?1",
            [id.as_bytes()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        let kek = derive_kek(password, &salt, m, t, p)?;
        let dek = open(&kek, id.as_bytes(), &wrapped).map_err(|_| Error::WrongPassword)?;
        if dek.len() != 32 {
            return Err(Error::Crypto("corrupt vault key".into()));
        }
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&dek);
        Ok(key)
    }

    pub fn unlock_vault(&mut self, id: Uuid, password: &str) -> Result<Vault> {
        let key = self.unwrap_dek(id, password)?;
        self.vault_keys.insert(id, key);
        self.get_vault(id)
    }

    pub fn lock_vault(&mut self, id: Uuid) {
        self.vault_keys.remove(&id);
    }

    pub fn lock_all_vaults(&mut self) {
        self.vault_keys.clear();
    }

    pub fn change_vault_password(&mut self, id: Uuid, old: &str, new: &str) -> Result<()> {
        check_password(new)?;
        let dek = self.unwrap_dek(id, old)?;
        let salt = random::<16>()?;
        let kek = derive_kek(new, &salt, KDF_M_KIB, KDF_T, KDF_P)?;
        let wrapped = seal(&kek, id.as_bytes(), dek.as_ref())?;
        self.conn.execute(
            "UPDATE vaults SET salt = ?2, kdf_m = ?3, kdf_t = ?4, kdf_p = ?5, wrapped_key = ?6 WHERE id = ?1",
            params![id.as_bytes(), &salt[..], KDF_M_KIB, KDF_T, KDF_P, wrapped],
        )?;
        Ok(())
    }

    /// Permanently deletes a vault and every note in it. Encrypted vaults require the password.
    /// The last remaining vault can't be deleted.
    pub fn delete_vault(&mut self, id: Uuid, password: Option<&str>) -> Result<()> {
        if self.vault_is_encrypted(id)? {
            self.unwrap_dek(id, password.unwrap_or_default())?;
        }
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM vaults", [], |r| r.get(0))?;
        if count <= 1 {
            return Err(Error::invalid("can't delete the only vault"));
        }
        self.conn
            .execute("DELETE FROM vaults WHERE id = ?1", [id.as_bytes()])?;
        self.vault_keys.remove(&id);
        self.prune_tags()?;
        Ok(())
    }

    /// Moves a note to another vault, re-encrypting or decrypting as needed. Moving into an
    /// encrypted vault scrubs everything derived from the plaintext (history, tags, links,
    /// search index, embeddings).
    pub fn move_note_to_vault(&self, note_id: Uuid, target: Uuid) -> Result<Note> {
        let note = self.get_note(note_id)?;
        if note.vault_id == target {
            return Ok(note);
        }
        let id = note_id.as_bytes();
        if self.vault_is_encrypted(target)? {
            let plain = serde_json::to_vec(&SecretContent {
                title: note.title,
                body: note.body.clone(),
            })?;
            let cipher = self.encrypt_for(target, note_id, &plain)?;
            self.conn
                .execute("DELETE FROM note_versions WHERE note_id = ?1", [id])?;
            self.conn
                .execute("DELETE FROM note_tags WHERE note_id = ?1", [id])?;
            self.conn
                .execute("DELETE FROM links WHERE source_id = ?1", [id])?;
            self.conn
                .execute("DELETE FROM embeddings WHERE note_id = ?1", [id])?;
            self.conn.execute(
                "UPDATE notes SET title = '', body = '', cipher = ?2, vault_id = ?3, folder_id = NULL, updated_at = ?4 WHERE id = ?1",
                params![id, cipher, target.as_bytes(), now_ms()],
            )?;
            self.prune_tags()?;
        } else {
            self.conn.execute(
                "UPDATE notes SET title = ?2, body = ?3, cipher = NULL, vault_id = ?4, folder_id = NULL, updated_at = ?5 WHERE id = ?1",
                params![id, note.title, note.body, target.as_bytes(), now_ms()],
            )?;
            if note.kind == NoteKind::Note {
                self.sync_links(note_id, &note.body)?;
            }
        }
        // Images travel with the note (re-encrypted or decrypted as needed).
        self.rehome_attachments(&note.body, target)?;
        self.get_note(note_id)
    }
}
