//! Cloud vaults on the user's own drive (Google Drive for now).
//!
//! - [`oauth`]: signing in with the user's own Google OAuth client (PKCE + loopback), and the
//!   Google Picker for choosing a folder someone shared.
//! - [`drive`]: the few Drive calls we need.
//! - [`sync`]: end-to-end encrypted sync of a vault through its drive folder.
//!
//! A cloud vault is `Astralis/<vault name>/` in the owner's Drive. The folder's private app
//! properties say which vault it is and which device owns it; invites add a one-time secret per
//! invited email. Someone invited picks the folder in the Google Picker, reads their secret, and
//! presents it to the owner's device over P2P to join (see `astralis-net`).

pub mod drive;
pub mod oauth;
pub mod sync;

/// The HTTP client used for Google (re-exported so the app uses the same version).
pub use reqwest;

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use astralis_core::cloud::{CloudVault, GOOGLE_DRIVE};
use astralis_core::vault::Vault;
use base64::Engine;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use drive::GoogleDrive;
pub use sync::{StoreHandle, SyncReport, sync_vault};

/// Folder properties (private to Astralis).
const PROP_KIND: &str = "astralis";
const PROP_VAULT: &str = "vault";
const PROP_OWNER: &str = "owner";
const PROP_OWNER_NAME: &str = "ownerName";

/// The property holding an invite for `email` (its secret is the value).
fn invite_prop(email: &str) -> String {
    let h = Sha256::digest(email.trim().to_ascii_lowercase().as_bytes());
    format!("i_{}", h.iter().take(8).map(|b| format!("{b:02x}")).collect::<String>())
}

fn secret() -> Result<String> {
    let b = astralis_core::vault::random::<18>().map_err(|e| anyhow!("{e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b))
}

/// Creates a vault backed by a new `Astralis/<name>/` folder in the user's Drive.
pub async fn create_cloud_vault(
    drive: &GoogleDrive,
    store: &StoreHandle,
    name: &str,
    device: &str,
    device_name: &str,
) -> Result<Vault> {
    let name = name.trim();
    if name.is_empty() {
        bail!("give the vault a name");
    }
    let vault = store.lock().unwrap().create_vault(name, None).map_err(|e| anyhow!("{e}"))?;
    if let Err(e) = make_cloud(drive, store, vault.id, name, device, device_name).await {
        // Don't leave a half-made vault behind.
        let _ = store.lock().unwrap().delete_vault(vault.id, None);
        return Err(e);
    }
    store.lock().unwrap().get_vault(vault.id).map_err(|e| anyhow!("{e}"))
}

/// Puts an existing (plain) vault on the Drive: makes its `Astralis/<name>/` folder and a fresh
/// sync key. Used for new cloud vaults and for turning the Brain into a cloud vault.
pub async fn make_cloud(drive: &GoogleDrive, store: &StoreHandle, vault: Uuid, name: &str, device: &str, device_name: &str) -> Result<()> {
    {
        let s = store.lock().unwrap();
        if s.cloud_vault(vault).map_err(|e| anyhow!("{e}"))?.is_some() {
            bail!("this vault is already in the cloud");
        }
        let v = s.get_vault(vault).map_err(|e| anyhow!("{e}"))?;
        if v.encrypted || v.owner.is_some() {
            bail!("only your own, unencrypted vaults can go to the cloud");
        }
    }
    let root = drive.root_folder().await?;
    let props = BTreeMap::from([
        (PROP_KIND.to_string(), "vault".to_string()),
        (PROP_VAULT.to_string(), vault.to_string()),
        (PROP_OWNER.to_string(), device.to_string()),
        (PROP_OWNER_NAME.to_string(), device_name.chars().take(60).collect()),
    ]);
    let folder = drive.create_folder(name, &root, &props).await?;
    let key = astralis_core::vault::random::<32>().map_err(|e| anyhow!("{e}"))?;
    store
        .lock()
        .unwrap()
        .set_cloud_vault(&CloudVault {
            vault_id: vault,
            provider: GOOGLE_DRIVE.into(),
            folder_id: folder,
            sync_key: key.to_vec(),
            owned: true,
            uploaded_vv: Default::default(),
        })
        .map_err(|e| anyhow!("{e}"))
}

/// Invites someone by email: shares the folder with them (Google emails them) and leaves a
/// one-time secret their app uses to join.
pub async fn invite(drive: &GoogleDrive, store: &StoreHandle, vault: Uuid, email: &str, from: &str) -> Result<()> {
    let email = email.trim().to_ascii_lowercase();
    if !email.contains('@') || email.len() > 254 {
        bail!("that doesn't look like an email address");
    }
    let cv = owned_cloud_vault(store, vault)?;
    let secret = secret()?;
    drive
        .set_props(&cv.folder_id, &BTreeMap::from([(invite_prop(&email), Some(secret.clone()))]))
        .await?;
    let note = format!(
        "{from} shared an Astralis vault with you. Open Astralis, then in the vault menu choose \
         \u{201c}Join shared vault from Google Drive\u{2026}\u{201d} and pick this folder."
    );
    drive.share_with(&cv.folder_id, &email, &note).await?;
    store.lock().unwrap().add_cloud_invite(vault, &email, &secret).map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

/// Takes someone out: removes their Drive access and invite. Returns the device that had
/// joined with it (the caller stops sharing with that device so its copy is removed).
pub async fn uninvite(drive: &GoogleDrive, store: &StoreHandle, vault: Uuid, email: &str) -> Result<Option<String>> {
    let cv = owned_cloud_vault(store, vault)?;
    drive.unshare_with(&cv.folder_id, email).await?;
    drive.set_props(&cv.folder_id, &BTreeMap::from([(invite_prop(email), None)])).await?;
    store.lock().unwrap().remove_cloud_invite(vault, email).map_err(|e| anyhow!("{e}"))
}

/// Google refuses to change who a folder is shared with while it holds files this app can't touch.
/// With the drive.file permission, the app only reaches files the owner's own copy created, so
/// files a member uploaded (Astralis before 1.0.2 did) block inviting and removing people until
/// that member's updated Astralis removes them (see `sync::sync_vault`).
pub fn members_files_in_the_way(e: &anyhow::Error) -> bool {
    format!("{e:#}").contains("has not granted the app")
}

fn owned_cloud_vault(store: &StoreHandle, vault: Uuid) -> Result<CloudVault> {
    match store.lock().unwrap().cloud_vault(vault).map_err(|e| anyhow!("{e}"))? {
        Some(cv) if cv.owned => Ok(cv),
        Some(_) => bail!("only the vault's owner can invite people"),
        None => bail!("this isn't a cloud vault"),
    }
}

/// What someone invited needs to join: read from the folder they picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinTicket {
    pub vault: Uuid,
    pub folder_id: String,
    pub name: String,
    /// The owner's device (P2P identity) and name.
    pub owner: String,
    pub owner_name: String,
    pub email: String,
    pub secret: String,
}

/// Reads the invite for `email` from a picked folder.
pub async fn join_ticket(drive: &GoogleDrive, folder_id: &str, email: &str) -> Result<JoinTicket> {
    let f = drive.folder(folder_id).await?;
    if f.props.get(PROP_KIND).map(String::as_str) != Some("vault") {
        bail!("“{}” isn't an Astralis vault folder", f.name);
    }
    let vault: Uuid = f.props.get(PROP_VAULT).and_then(|v| v.parse().ok()).ok_or_else(|| anyhow!("vault folder is missing its id"))?;
    let owner = f.props.get(PROP_OWNER).cloned().ok_or_else(|| anyhow!("vault folder is missing its owner"))?;
    let secret = f
        .props
        .get(&invite_prop(email))
        .cloned()
        .ok_or_else(|| anyhow!("“{}” wasn't shared with {email} in Astralis (ask the owner to invite this address)", f.name))?;
    Ok(JoinTicket {
        vault,
        folder_id: f.id,
        name: f.name,
        owner,
        owner_name: f.props.get(PROP_OWNER_NAME).cloned().unwrap_or_default(),
        email: email.trim().to_ascii_lowercase(),
        secret,
    })
}

#[cfg(test)]
mod tests;
