use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use astralis_core::Store;
use astralis_core::cloud::{CloudVault, GOOGLE_DRIVE};
use astralis_core::notes::{ListFilter, NewNote, NotePatch};

use super::*;
use crate::drive::{DriveFiles, RemoteFile};
use crate::sync::{EXT, open_changes, seal_changes};

/// An in-memory drive. `hidden` files exist but this account can't read them (like files
/// another person's copy of the app made, under the drive.file scope).
#[derive(Default)]
struct MemDrive {
    files: Mutex<Vec<(String, String, String, Vec<u8>)>>, // id, folder, name, bytes
    hidden: Mutex<Vec<String>>,
}

impl DriveFiles for MemDrive {
    async fn list(&self, folder: &str) -> Result<Vec<RemoteFile>> {
        Ok(self
            .files
            .lock()
            .unwrap()
            .iter()
            .filter(|f| f.1 == folder)
            .map(|f| RemoteFile { id: f.0.clone(), name: f.2.clone() })
            .collect())
    }
    async fn download(&self, id: &str) -> Result<Vec<u8>> {
        if self.hidden.lock().unwrap().iter().any(|h| h == id) {
            return Err(anyhow!("not found"));
        }
        self.files.lock().unwrap().iter().find(|f| f.0 == id).map(|f| f.3.clone()).ok_or_else(|| anyhow!("not found"))
    }
    async fn upload(&self, folder: &str, name: &str, bytes: Vec<u8>) -> Result<String> {
        let id = format!("f{}", self.files.lock().unwrap().len());
        self.files.lock().unwrap().push((id.clone(), folder.into(), name.into(), bytes));
        Ok(id)
    }
}

fn handle() -> StoreHandle {
    Arc::new(Mutex::new(Store::open_in_memory().unwrap()))
}

/// Two devices of the same account with the vault and its key (as after joining).
fn two_devices() -> (StoreHandle, StoreHandle, uuid::Uuid) {
    let a = handle();
    let b = handle();
    let v = a.lock().unwrap().create_vault("Cloud", None).unwrap();
    let key = astralis_core::vault::random::<32>().unwrap().to_vec();
    for (s, owned) in [(&a, true), (&b, false)] {
        s.lock()
            .unwrap()
            .set_cloud_vault(&CloudVault {
                vault_id: v.id,
                provider: GOOGLE_DRIVE.into(),
                folder_id: "folder".into(),
                sync_key: key.clone(),
                owned,
                uploaded_vv: Default::default(),
            })
            .unwrap();
    }
    (a, b, v.id)
}

fn titles(s: &StoreHandle, vault: uuid::Uuid) -> Vec<String> {
    let mut t: Vec<String> = s
        .lock()
        .unwrap()
        .list_notes(&ListFilter { vault_id: Some(vault), ..Default::default() })
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    t.sort();
    t
}

#[tokio::test]
async fn devices_sync_a_vault_through_the_drive_without_being_online_together() {
    let (a, b, v) = two_devices();
    let drive = MemDrive::default();
    let n = a.lock().unwrap().create_note(NewNote { title: "Trip".into(), body: "pack".into(), vault_id: Some(v), ..Default::default() }).unwrap();
    // A uploads and goes away; B comes online later and catches up from the drive alone.
    assert!(sync_vault(&drive, &a, v).await.unwrap().uploaded);
    let got = sync_vault(&drive, &b, v).await.unwrap();
    assert_eq!(got.applied, 1);
    assert_eq!(titles(&b, v), vec!["Trip"]);
    assert!(got.notes.contains(&n.id.to_string()));

    // Edits both ways merge (note text is a CRDT); nothing is uploaded twice.
    b.lock().unwrap().update_note(n.id, NotePatch { body: Some("pack socks".into()), ..Default::default() }).unwrap();
    sync_vault(&drive, &b, v).await.unwrap();
    sync_vault(&drive, &a, v).await.unwrap();
    assert_eq!(a.lock().unwrap().get_note(n.id).unwrap().body, "pack socks");
    let before = drive.files.lock().unwrap().len();
    assert!(!sync_vault(&drive, &a, v).await.unwrap().uploaded, "what came from the drive isn't sent back");
    assert!(!sync_vault(&drive, &b, v).await.unwrap().uploaded);
    assert_eq!(drive.files.lock().unwrap().len(), before);
}

#[tokio::test]
async fn the_drive_only_ever_sees_ciphertext_and_cant_reach_other_vaults() {
    let (a, b, v) = two_devices();
    let drive = MemDrive::default();
    a.lock().unwrap().create_note(NewNote { title: "Secret plans".into(), body: "launch on friday".into(), vault_id: Some(v), ..Default::default() }).unwrap();
    sync_vault(&drive, &a, v).await.unwrap();
    for f in drive.files.lock().unwrap().iter() {
        assert!(f.2.ends_with(EXT));
        assert!(!f.3.windows(6).any(|w| w == b"friday"), "plaintext on the drive");
    }

    // Someone with drive access but not the key can't inject changes, and even a member's
    // file can't touch another vault.
    let b_private = b.lock().unwrap().default_vault().unwrap();
    drive.upload("folder", &format!("evil{EXT}"), b"not sealed".to_vec()).await.unwrap();
    let key = a.lock().unwrap().cloud_vault(v).unwrap().unwrap().sync_key;
    let mut forged = a.lock().unwrap().vault_changes_since(v, &Default::default()).unwrap();
    for r in forged.rows.iter_mut().filter(|r| r.table == "notes") {
        r.values.insert("vault_id".into(), astralis_core::sync::SqlValue::Blob(b_private.as_bytes().to_vec()));
    }
    drive.upload("folder", &format!("forged{EXT}"), seal_changes(&key, v, &forged).unwrap()).await.unwrap();
    sync_vault(&drive, &b, v).await.unwrap();
    assert_eq!(titles(&b, v), vec!["Secret plans"]);
    assert!(titles(&b, b_private).is_empty());

    // A file sealed for one vault can't be replayed into another.
    let sealed = drive.files.lock().unwrap()[0].3.clone();
    assert!(open_changes(&key, uuid::Uuid::now_v7(), &sealed).is_err());
}

#[tokio::test]
async fn files_this_account_cant_read_are_retried_later() {
    let (a, b, v) = two_devices();
    let drive = MemDrive::default();
    a.lock().unwrap().create_note(NewNote { title: "Later".into(), vault_id: Some(v), ..Default::default() }).unwrap();
    sync_vault(&drive, &a, v).await.unwrap();
    let id = drive.files.lock().unwrap()[0].0.clone();
    drive.hidden.lock().unwrap().push(id.clone());
    assert_eq!(sync_vault(&drive, &b, v).await.unwrap().applied, 0);
    drive.hidden.lock().unwrap().clear();
    assert_eq!(sync_vault(&drive, &b, v).await.unwrap().applied, 1);
    assert_eq!(titles(&b, v), vec!["Later"]);
}

#[test]
fn invite_properties_are_per_email_and_case_insensitive() {
    assert_eq!(invite_prop("Friend@Gmail.com "), invite_prop("friend@gmail.com"));
    assert_ne!(invite_prop("a@x.com"), invite_prop("b@x.com"));
    // Drive limits a property's key + value to 124 bytes.
    assert!(invite_prop("someone@example.com").len() + secret().unwrap().len() < 124);
}
