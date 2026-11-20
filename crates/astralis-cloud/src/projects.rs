//! Project snapshots in the user's own drive: `Astralis/Projects/`.
//!
//! Each project is two sealed files, `<project id>.<time>.bundle.astrp` (the git bundle) and
//! `<project id>.<time>.meta.astrp` (what it is: name, branch, device…). The meta file is uploaded
//! last, so a snapshot only counts once it is whole. The project id is a hash of the project's
//! identity, so the file names don't reveal what the projects are. The files are sealed with a
//! key that lives on the drive's top-level Astralis folder (the same trust as a vault's key), so
//! a new device that connects the same drive can open them. Nothing is downloaded unless the user
//! asks (restore, or update a project that is behind).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, anyhow};
use astralis_core::projects::{Manifest, RemoteInfo, project_hash};
use astralis_core::vault;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::drive::CloudDrive;
use crate::sync::StoreHandle;

const FOLDER: &str = "Projects";
const KEY_PROP: &str = "projects_key";
const KEY_SETTING: &str = "projects.key";
const EXT: &str = ".astrp";
/// Snapshots kept per project (the newest, and the one before it in case the newest is bad).
const KEEP: usize = 2;

fn aad(hash: &str, what: &str) -> Vec<u8> {
    format!("astralis-project:{hash}:{what}").into_bytes()
}

fn decode(text: &str) -> Option<[u8; 32]> {
    URL_SAFE_NO_PAD.decode(text.trim()).ok()?.try_into().ok()
}

/// The key the snapshots are sealed with: the one on the drive if there is one (so every device
/// uses the same), otherwise this device's own, which is then put on the drive.
pub async fn ensure_key(drive: &impl CloudDrive, store: &StoreHandle) -> Result<[u8; 32]> {
    let root = drive.root_folder().await?;
    let on_drive = drive.folder(&root).await?.props.get(KEY_PROP).and_then(|v| decode(v));
    let local = store.lock().unwrap().get_setting(KEY_SETTING)?.and_then(|v| decode(&v));
    let key = match (on_drive, local) {
        (Some(d), _) => d,
        (None, Some(l)) => l,
        (None, None) => vault::random::<32>()?,
    };
    if on_drive != Some(key) {
        drive.set_props(&root, &BTreeMap::from([(KEY_PROP.to_string(), Some(URL_SAFE_NO_PAD.encode(key)))])).await?;
    }
    if local != Some(key) {
        store.lock().unwrap().set_setting(KEY_SETTING, &URL_SAFE_NO_PAD.encode(key))?;
    }
    Ok(key)
}

/// The `Projects` folder (made if it isn't there).
async fn folder(drive: &impl CloudDrive) -> Result<String> {
    let root = drive.root_folder().await?;
    if let Some(f) = drive.subfolders(&root).await?.into_iter().find(|f| f.name == FOLDER) {
        return Ok(f.id);
    }
    drive.create_folder(FOLDER, &root, &BTreeMap::new()).await
}

/// `<hash>.<time>.<meta|bundle>.astrp` taken apart.
fn parse_name(name: &str) -> Option<(&str, u64, &str)> {
    let rest = name.strip_suffix(EXT)?;
    let mut it = rest.splitn(3, '.');
    let (hash, ts, kind) = (it.next()?, it.next()?.parse().ok()?, it.next()?);
    matches!(kind, "meta" | "bundle").then_some((hash, ts, kind))
}

/// Uploads the snapshots waiting in `<dir>/outbox` (made when an agent saved a hand-off). Returns how many.
pub async fn push(drive: &impl CloudDrive, store: &StoreHandle, dir: &Path) -> Result<usize> {
    let outbox = dir.join("outbox");
    let waiting: Vec<_> = match std::fs::read_dir(&outbox) {
        Ok(r) => r.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect(),
        Err(_) => return Ok(0),
    };
    if waiting.is_empty() {
        return Ok(0);
    }
    let key = ensure_key(drive, store).await?;
    let folder = folder(drive).await?;
    let mut done = 0;
    for meta_path in waiting {
        let manifest: Manifest = serde_json::from_slice(&std::fs::read(&meta_path)?)?;
        let hash = project_hash(&manifest.key);
        let bundle_path = outbox.join(format!("{hash}.bundle"));
        // Another device has newer work in the cloud that this one hasn't taken: uploading would
        // bury it. The user updates from the cloud first (Projects → Update), then it uploads again.
        if store.lock().unwrap().project_cloud_newer(&hash)?.is_some() {
            let _ = std::fs::remove_file(&meta_path);
            let _ = std::fs::remove_file(&bundle_path);
            continue;
        }
        let bundle = std::fs::read(&bundle_path).map_err(|e| anyhow!("snapshot of {} is missing its bundle: {e}", manifest.name))?;
        let ts = manifest.ts;
        drive.upload(&folder, &format!("{hash}.{ts}.bundle{EXT}"), vault::seal(&key, &aad(&hash, "bundle"), &bundle)?).await?;
        drive.upload(&folder, &format!("{hash}.{ts}.meta{EXT}"), vault::seal(&key, &aad(&hash, "meta"), &serde_json::to_vec(&manifest)?)?).await?;
        // Older snapshots of this project go (the newest KEEP stay).
        let mut mine: Vec<(u64, String)> = drive
            .list(&folder)
            .await?
            .into_iter()
            .filter_map(|f| parse_name(&f.name).filter(|(h, _, _)| *h == hash).map(|(_, t, _)| (t, f.id)))
            .collect();
        let mut stamps: Vec<u64> = mine.iter().map(|(t, _)| *t).collect();
        stamps.sort_unstable();
        stamps.dedup();
        if stamps.len() > KEEP {
            let cut = stamps[stamps.len() - KEEP];
            mine.retain(|(t, _)| *t < cut);
            for (_, id) in mine {
                let _ = drive.delete(&id).await;
            }
        }
        store.lock().unwrap().project_mark(&hash, Some(ts), Some(&manifest.state))?;
        let _ = std::fs::remove_file(&meta_path);
        let _ = std::fs::remove_file(&bundle_path);
        done += 1;
    }
    Ok(done)
}

/// Lists what's in the cloud and remembers the newest whole snapshot of each project.
pub async fn refresh(drive: &impl CloudDrive, store: &StoreHandle) -> Result<()> {
    let key = ensure_key(drive, store).await?;
    let folder = folder(drive).await?;
    // project -> time -> (meta id, bundle id)
    let mut found: BTreeMap<String, BTreeMap<u64, (Option<String>, Option<String>)>> = BTreeMap::new();
    for f in drive.list(&folder).await? {
        if let Some((hash, ts, kind)) = parse_name(&f.name) {
            let slot = found.entry(hash.to_string()).or_default().entry(ts).or_default();
            if kind == "meta" {
                slot.0 = Some(f.id);
            } else {
                slot.1 = Some(f.id);
            }
        }
    }
    let old = store.lock().unwrap().projects_remote()?;
    let mut now = BTreeMap::new();
    for (hash, versions) in found {
        let newest = versions.iter().rev().find_map(|(ts, (m, b))| Some((*ts, m.clone()?, b.clone()?)));
        let Some((ts, meta_id, bundle_id)) = newest else { continue };
        match old.get(&hash) {
            Some(r) if r.manifest.ts == ts && r.meta_id == meta_id && r.bundle_id == bundle_id => {
                now.insert(hash, r.clone());
            }
            _ => {
                let plain = vault::open(&key, &aad(&hash, "meta"), &drive.download(&meta_id).await?)?;
                let manifest: Manifest = serde_json::from_slice(&plain)?;
                now.insert(hash, RemoteInfo { manifest, meta_id, bundle_id });
            }
        }
    }
    store.lock().unwrap().set_projects_remote(&now)?;
    Ok(())
}

/// Downloads a project's newest snapshot to `dest` (unsealed) and returns what it is.
pub async fn download(drive: &impl CloudDrive, store: &StoreHandle, hash: &str, dest: &Path) -> Result<Manifest> {
    let info = store.lock().unwrap().projects_remote()?.remove(hash).ok_or_else(|| anyhow!("that project isn't in the cloud"))?;
    let key = ensure_key(drive, store).await?;
    let plain = vault::open(&key, &aad(hash, "bundle"), &drive.download(&info.bundle_id).await?)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, &*plain)?;
    Ok(info.manifest)
}

