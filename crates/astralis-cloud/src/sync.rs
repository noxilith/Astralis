//! Syncing a cloud vault through its drive folder.
//!
//! Each device uploads its new changes as one sealed file (`<device>-<time>.astr`) and applies
//! the files it hasn't seen yet. Files are sealed with the vault's sync key (XChaCha20-Poly1305,
//! bound to the vault id), so the drive only ever holds ciphertext. Applying goes through the
//! same scope checks as P2P: a file can only change this one vault.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use astralis_core::Store;
use astralis_core::sync::{ChangeSet, SqlValue, SyncScope, VersionVector};
use uuid::Uuid;

use crate::drive::DriveFiles;

pub type StoreHandle = Arc<Mutex<Store>>;

/// Sync files on the drive end with this.
pub const EXT: &str = ".astr";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub uploaded: bool,
    /// Files from other devices applied this round.
    pub applied: usize,
    /// Notes that changed (for refreshing open views).
    pub notes: Vec<String>,
}

fn aad(vault: Uuid) -> Vec<u8> {
    [vault.as_bytes().as_slice(), b":cloud"].concat()
}

fn key_of(raw: &[u8]) -> Result<[u8; 32]> {
    raw.try_into().map_err(|_| anyhow!("bad sync key"))
}

/// Seals a change set for the drive.
pub fn seal_changes(key: &[u8], vault: Uuid, cs: &ChangeSet) -> Result<Vec<u8>> {
    let plain = astralis_core::sync::encode(cs).map_err(|e| anyhow!("{e}"))?;
    astralis_core::vault::seal(&key_of(key)?, &aad(vault), &plain).map_err(|e| anyhow!("{e}"))
}

pub fn open_changes(key: &[u8], vault: Uuid, sealed: &[u8]) -> Result<ChangeSet> {
    let plain = astralis_core::vault::open(&key_of(key)?, &aad(vault), sealed).map_err(|e| anyhow!("{e}"))?;
    astralis_core::sync::decode(&plain).map_err(|e| anyhow!("{e}"))
}

/// The highest clock per origin in a change set.
fn vv_of(cs: &ChangeSet) -> VersionVector {
    let mut vv = VersionVector::new();
    let mut bump = |o: &str, h: &str| {
        if o.is_empty() {
            return;
        }
        let e = vv.entry(o.to_owned()).or_default();
        if h > e.as_str() {
            *e = h.to_owned();
        }
    };
    let text = |v: Option<&SqlValue>| match v {
        Some(SqlValue::Text(t)) => t.clone(),
        _ => String::new(),
    };
    for r in &cs.rows {
        bump(&text(r.values.get("origin")), &text(r.values.get("hlc")));
    }
    for t in &cs.tombstones {
        bump(&t.origin, &t.hlc);
    }
    vv
}

fn merge(a: &mut VersionVector, b: &VersionVector) {
    for (k, v) in b {
        let e = a.entry(k.clone()).or_default();
        if v > e {
            *e = v.clone();
        }
    }
}

/// One round for one cloud vault: apply what's new on the drive, then upload what's new here.
pub async fn sync_vault<D: DriveFiles>(drive: &D, store: &StoreHandle, vault: Uuid) -> Result<SyncReport> {
    let Some(cv) = store.lock().unwrap().cloud_vault(vault).map_err(|e| anyhow!("{e}"))? else {
        return Ok(SyncReport::default());
    };
    let mut report = SyncReport::default();
    let mut uploaded = cv.uploaded_vv.clone();
    let scope = SyncScope { vaults: HashSet::from([vault]), owned_by_sender: HashSet::new() };

    // Pull.
    let files = drive.list(&cv.folder_id).await?;
    for f in files.iter().filter(|f| f.name.ends_with(EXT)) {
        if store.lock().unwrap().cloud_file_seen(vault, &f.id).map_err(|e| anyhow!("{e}"))? {
            continue;
        }
        let bytes = match drive.download(&f.id).await {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!("can't read {} yet: {e:#}", f.name);
                continue; // e.g. not readable by this account; try again later
            }
        };
        let s = store.lock().unwrap();
        match open_changes(&cv.sync_key, vault, &bytes) {
            Ok(cs) => {
                s.apply_vault_changes(&cs, &scope).map_err(|e| anyhow!("{e}"))?;
                merge(&mut uploaded, &vv_of(&cs)); // don't send it back up
                report.applied += 1;
                report.notes.extend(cs.rows.iter().filter(|r| r.table == "notes").filter_map(|r| match r.values.get("id") {
                    Some(SqlValue::Blob(b)) => Uuid::from_slice(b).ok().map(|u| u.to_string()),
                    _ => None,
                }));
            }
            Err(e) => tracing::warn!("skipping {} on the drive: {e:#}", f.name),
        }
        s.mark_cloud_file_seen(vault, &f.id).map_err(|e| anyhow!("{e}"))?;
    }

    // Push.
    let (current, cs, device) = {
        let s = store.lock().unwrap();
        let current = s.vault_version_vector(vault).map_err(|e| anyhow!("{e}"))?;
        let cs = s.vault_changes_since(vault, &uploaded).map_err(|e| anyhow!("{e}"))?;
        (current, cs, s.device_id())
    };
    if !cs.is_empty() {
        let sealed = seal_changes(&cv.sync_key, vault, &cs)?;
        let name = format!("{device}-{}{EXT}", Uuid::now_v7().simple());
        let id = drive.upload(&cv.folder_id, &name, sealed).await?;
        store.lock().unwrap().mark_cloud_file_seen(vault, &id).map_err(|e| anyhow!("{e}"))?;
        report.uploaded = true;
    }
    merge(&mut uploaded, &current);
    if uploaded != cv.uploaded_vv {
        store.lock().unwrap().set_cloud_uploaded(vault, &uploaded).map_err(|e| anyhow!("{e}"))?;
    }
    Ok(report)
}
