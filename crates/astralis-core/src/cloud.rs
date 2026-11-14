//! Cloud vaults: vaults backed by a folder on the user's own cloud drive.
//!
//! The drive holds only end-to-end encrypted sync data (`Astralis/<vault>/`): each device
//! uploads its changes as sealed files and applies the files it can read. The vault's sync key
//! never goes to the drive — it travels between members over the encrypted P2P link. Inviting
//! someone by email shares the drive folder and leaves a one-time invite secret, which their app
//! presents (P2P) to join; after that they're a member like any shared-vault member.
//!
//! This module only keeps the records; talking to drives lives in `astralis-cloud`.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::sync::VersionVector;
use crate::util::uuid_col;

pub(crate) const CLOUD_MIGRATION: &str = "
    CREATE TABLE cloud_vaults (
        vault_id    BLOB PRIMARY KEY,
        provider    TEXT NOT NULL,
        folder_id   TEXT NOT NULL,
        sync_key    BLOB NOT NULL,
        owned       INTEGER NOT NULL,
        uploaded_vv TEXT NOT NULL DEFAULT '{}'
    ) WITHOUT ROWID;
    CREATE TABLE cloud_seen (
        vault_id BLOB NOT NULL, file_id TEXT NOT NULL, PRIMARY KEY (vault_id, file_id)
    ) WITHOUT ROWID;
    CREATE TABLE cloud_invites (
        vault_id BLOB NOT NULL, email TEXT NOT NULL, secret TEXT NOT NULL,
        accepted_by TEXT,
        PRIMARY KEY (vault_id, email)
    ) WITHOUT ROWID;
";

/// Providers a cloud vault can live on.
pub const GOOGLE_DRIVE: &str = "google-drive";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudVault {
    pub vault_id: Uuid,
    pub provider: String,
    /// The vault's folder on the drive.
    pub folder_id: String,
    /// Key that seals everything uploaded for this vault. Never leaves the device except to
    /// other members over P2P.
    #[serde(skip)]
    pub sync_key: Vec<u8>,
    /// We created it (it lives in our drive).
    pub owned: bool,
    /// What's already been uploaded from this device.
    #[serde(skip)]
    pub uploaded_vv: VersionVector,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudInvite {
    pub email: String,
    /// The device that used the invite, once someone joined with it.
    pub accepted_by: Option<String>,
}

fn normalize_email(e: &str) -> String {
    e.trim().to_ascii_lowercase()
}

impl Store {
    pub fn set_cloud_vault(&self, cv: &CloudVault) -> Result<()> {
        if cv.sync_key.len() != 32 {
            return Err(Error::invalid("bad cloud sync key"));
        }
        self.conn.execute(
            "INSERT INTO cloud_vaults(vault_id, provider, folder_id, sync_key, owned, uploaded_vv)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(vault_id) DO UPDATE SET provider = excluded.provider, folder_id = excluded.folder_id,
                 sync_key = excluded.sync_key, owned = excluded.owned",
            params![
                cv.vault_id.as_bytes(),
                cv.provider,
                cv.folder_id,
                cv.sync_key,
                cv.owned,
                serde_json::to_string(&cv.uploaded_vv)?
            ],
        )?;
        Ok(())
    }

    pub fn cloud_vault(&self, vault: Uuid) -> Result<Option<CloudVault>> {
        Ok(self
            .cloud_vaults_where("WHERE vault_id = ?1", &vault.as_bytes().as_slice())?
            .into_iter()
            .next())
    }

    pub fn cloud_vaults(&self) -> Result<Vec<CloudVault>> {
        self.cloud_vaults_where("WHERE ?1 IS NOT NULL", &1)
    }

    fn cloud_vaults_where(&self, filter: &str, arg: &dyn rusqlite::ToSql) -> Result<Vec<CloudVault>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT vault_id, provider, folder_id, sync_key, owned, uploaded_vv FROM cloud_vaults {filter}"
        ))?;
        let rows = stmt.query_map([arg], |r| {
            Ok(CloudVault {
                vault_id: uuid_col(r, 0)?,
                provider: r.get(1)?,
                folder_id: r.get(2)?,
                sync_key: r.get(3)?,
                owned: r.get(4)?,
                uploaded_vv: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_cloud_uploaded(&self, vault: Uuid, vv: &VersionVector) -> Result<()> {
        self.conn.execute(
            "UPDATE cloud_vaults SET uploaded_vv = ?2 WHERE vault_id = ?1",
            params![vault.as_bytes(), serde_json::to_string(vv)?],
        )?;
        Ok(())
    }

    pub fn cloud_file_seen(&self, vault: Uuid, file_id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM cloud_seen WHERE vault_id = ?1 AND file_id = ?2",
                params![vault.as_bytes(), file_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn mark_cloud_file_seen(&self, vault: Uuid, file_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO cloud_seen(vault_id, file_id) VALUES (?1, ?2)",
            params![vault.as_bytes(), file_id],
        )?;
        Ok(())
    }

    /// Records an email invite (the secret also goes into the drive folder's private app data).
    pub fn add_cloud_invite(&self, vault: Uuid, email: &str, secret: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO cloud_invites(vault_id, email, secret) VALUES (?1, ?2, ?3)
             ON CONFLICT(vault_id, email) DO UPDATE SET secret = excluded.secret, accepted_by = NULL",
            params![vault.as_bytes(), normalize_email(email), secret],
        )?;
        Ok(())
    }

    pub fn cloud_invites(&self, vault: Uuid) -> Result<Vec<CloudInvite>> {
        let mut stmt =
            self.conn.prepare("SELECT email, accepted_by FROM cloud_invites WHERE vault_id = ?1 ORDER BY email")?;
        let rows = stmt.query_map([vault.as_bytes()], |r| Ok(CloudInvite { email: r.get(0)?, accepted_by: r.get(1)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A device presents an invite: valid once, for the vault and email it was made for.
    /// On success the vault is shared with that device.
    pub fn redeem_cloud_invite(&self, vault: Uuid, email: &str, secret: &str, device: &str) -> Result<bool> {
        let row: Option<(String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT secret, accepted_by FROM cloud_invites WHERE vault_id = ?1 AND email = ?2",
                params![vault.as_bytes(), normalize_email(email)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((expected, accepted_by)) = row else { return Ok(false) };
        let same = expected.len() == secret.len() && expected.bytes().zip(secret.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0;
        if !same || accepted_by.as_deref().is_some_and(|d| d != device) {
            return Ok(false);
        }
        self.conn.execute(
            "UPDATE cloud_invites SET accepted_by = ?3 WHERE vault_id = ?1 AND email = ?2",
            params![vault.as_bytes(), normalize_email(email), device],
        )?;
        self.share_vault(vault, device)?;
        Ok(true)
    }

    /// Removes an invite. Returns the device that had joined with it (to stop sharing with).
    pub fn remove_cloud_invite(&self, vault: Uuid, email: &str) -> Result<Option<String>> {
        let accepted: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT accepted_by FROM cloud_invites WHERE vault_id = ?1 AND email = ?2",
                params![vault.as_bytes(), normalize_email(email)],
                |r| r.get(0),
            )
            .optional()?;
        self.conn.execute(
            "DELETE FROM cloud_invites WHERE vault_id = ?1 AND email = ?2",
            params![vault.as_bytes(), normalize_email(email)],
        )?;
        let device = accepted.flatten();
        if let Some(d) = &device {
            self.unshare_vault(vault, d)?;
        }
        Ok(device)
    }

    /// Forgets the cloud side of a vault on this device (the local vault stays).
    /// Stops syncing a vault through the drive on this device (the vault and the Drive folder stay).
    pub fn leave_cloud(&self, vault: Uuid) -> Result<()> {
        self.forget_cloud_vault(vault)
    }

    pub(crate) fn forget_cloud_vault(&self, vault: Uuid) -> Result<()> {
        for t in ["cloud_vaults", "cloud_seen", "cloud_invites"] {
            self.conn.execute(&format!("DELETE FROM {t} WHERE vault_id = ?1"), [vault.as_bytes()])?;
        }
        Ok(())
    }
}
