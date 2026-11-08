//! Shared vaults: sync is scoped to vaults, never the whole database.
//!
//! A vault is shared by its owner with paired devices. On their side it shows up as an extra
//! vault (same id, so edits flow both ways); nothing else of either device is touched. When
//! sharing ends — the owner unshares, the member leaves, or the devices unpair — the member's
//! copy is removed without leaving tombstones, so it can't delete anything anywhere.
//!
//! `vault_shares` lists, per vault, the devices it syncs with and their role:
//! - `owner`: that device owns the vault and shared it with us,
//! - `member`: we own it and shared it with that device,
//! - `left`: we left that device's vault and are waiting for it to learn (so it isn't re-added).

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::Ordering;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{ApplyingGuard, ChangeSet, VersionVector};
use crate::error::{Error, Result};
use crate::store::Store;
use crate::util::uuid_col;

/// Tables whose rows belong to a vault (the vault row itself, then its contents).
pub(crate) const VAULT_TABLES: &[&str] = &["vaults", "folders", "notes", "attachments"];

/// Migration: shares, per-vault tombstones and "seen" clocks.
pub(crate) const SHARE_MIGRATION: &str = "
    CREATE TABLE vault_shares (
        vault_id BLOB NOT NULL, peer TEXT NOT NULL, role TEXT NOT NULL,
        PRIMARY KEY (vault_id, peer)
    ) WITHOUT ROWID;
    CREATE INDEX vault_shares_peer_idx ON vault_shares(peer);
    ALTER TABLE tombstones ADD COLUMN vault_id BLOB;
    CREATE INDEX tombstones_vault_idx ON tombstones(vault_id);
    CREATE TABLE sync_seen_vault (
        vault_id BLOB NOT NULL, origin TEXT NOT NULL, hlc TEXT NOT NULL,
        PRIMARY KEY (vault_id, origin)
    ) WITHOUT ROWID;

    DROP TRIGGER vaults_sync_ad;
    CREATE TRIGGER vaults_sync_ad AFTER DELETE ON vaults WHEN astralis_applying() = 0 BEGIN
        INSERT OR REPLACE INTO tombstones(entity, id, hlc, origin, vault_id)
        VALUES ('vaults', old.id, astralis_hlc(), astralis_device(), old.id);
    END;
    DROP TRIGGER folders_sync_ad;
    CREATE TRIGGER folders_sync_ad AFTER DELETE ON folders WHEN astralis_applying() = 0 BEGIN
        INSERT OR REPLACE INTO tombstones(entity, id, hlc, origin, vault_id)
        VALUES ('folders', old.id, astralis_hlc(), astralis_device(), old.vault_id);
    END;
    DROP TRIGGER notes_sync_ad;
    CREATE TRIGGER notes_sync_ad AFTER DELETE ON notes WHEN astralis_applying() = 0 BEGIN
        INSERT OR REPLACE INTO tombstones(entity, id, hlc, origin, vault_id)
        VALUES ('notes', old.id, astralis_hlc(), astralis_device(), old.vault_id);
    END;
    DROP TRIGGER attachments_sync_ad;
    CREATE TRIGGER attachments_sync_ad AFTER DELETE ON attachments WHEN astralis_applying() = 0 BEGIN
        INSERT OR REPLACE INTO tombstones(entity, id, hlc, origin, vault_id)
        VALUES ('attachments', old.id, astralis_hlc(), astralis_device(), old.vault_id);
    END;

    -- Moving something out of a vault looks like a delete to that vault's other devices.
    CREATE TRIGGER notes_sync_move AFTER UPDATE OF vault_id ON notes
    WHEN astralis_applying() = 0 AND old.vault_id IS NOT new.vault_id AND old.vault_id IS NOT NULL BEGIN
        INSERT OR REPLACE INTO tombstones(entity, id, hlc, origin, vault_id)
        VALUES ('notes', old.id, astralis_hlc(), astralis_device(), old.vault_id);
    END;
    CREATE TRIGGER attachments_sync_move AFTER UPDATE OF vault_id ON attachments
    WHEN astralis_applying() = 0 AND old.vault_id IS NOT new.vault_id BEGIN
        INSERT OR REPLACE INTO tombstones(entity, id, hlc, origin, vault_id)
        VALUES ('attachments', old.id, astralis_hlc(), astralis_device(), old.vault_id);
    END;
";

/// Migration: vaults this device received from another one before sync was per-vault (back
/// then paired devices merged everything, so both ended up with the same vault id). When that
/// device now shares the vault, our copy becomes a separate vault of our own (new id, same
/// name and contents) and theirs arrives next to it — sharing never takes what you have.
pub(crate) const LEGACY_VAULTS_MIGRATION: &str = "
    CREATE TABLE legacy_vaults (id BLOB PRIMARY KEY) WITHOUT ROWID;
    INSERT INTO legacy_vaults
        SELECT id FROM vaults
        WHERE origin <> '' AND origin IS NOT (SELECT value FROM settings WHERE key = 'device_id');
";

/// Another device's relation to one of our vaults (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShareRole {
    Owner,
    Member,
    Left,
}

impl ShareRole {
    fn as_str(self) -> &'static str {
        match self {
            ShareRole::Owner => "owner",
            ShareRole::Member => "member",
            ShareRole::Left => "left",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(ShareRole::Owner),
            "member" => Some(ShareRole::Member),
            "left" => Some(ShareRole::Left),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultShare {
    pub vault_id: Uuid,
    pub peer: String,
    pub role: ShareRole,
}

/// What a device may change on this device through sync: rows of these vaults only.
#[derive(Debug, Clone, Default)]
pub struct SyncScope {
    pub vaults: HashSet<Uuid>,
    /// Vaults the sending device owns (only the owner can delete a vault itself).
    pub owned_by_sender: HashSet<Uuid>,
}

impl Store {
    fn shares_where(&self, sql: &str, arg: &dyn rusqlite::ToSql) -> Result<Vec<VaultShare>> {
        let mut stmt = self.conn.prepare(&format!("SELECT vault_id, peer, role FROM vault_shares WHERE {sql}"))?;
        let rows = stmt.query_map([arg], |r| Ok((uuid_col(r, 0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (vault_id, peer, role) = row?;
            if let Some(role) = ShareRole::parse(&role) {
                out.push(VaultShare { vault_id, peer, role });
            }
        }
        Ok(out)
    }

    /// Every device a vault is shared with (or from).
    pub fn vault_shares(&self, vault: Uuid) -> Result<Vec<VaultShare>> {
        self.shares_where("vault_id = ?1", &vault.as_bytes().as_slice())
    }

    /// Every vault shared with (or from) a device.
    pub fn shares_with(&self, peer: &str) -> Result<Vec<VaultShare>> {
        self.shares_where("peer = ?1", &peer)
    }

    /// The device that shared this vault with us, if it isn't ours.
    pub fn vault_owner(&self, vault: Uuid) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT peer FROM vault_shares WHERE vault_id = ?1 AND role = 'owner'",
                [vault.as_bytes()],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn set_share(&self, vault: Uuid, peer: &str, role: ShareRole) -> Result<()> {
        self.conn.execute(
            "INSERT INTO vault_shares(vault_id, peer, role) VALUES (?1, ?2, ?3)
             ON CONFLICT(vault_id, peer) DO UPDATE SET role = excluded.role",
            params![vault.as_bytes(), peer, role.as_str()],
        )?;
        Ok(())
    }

    /// Stops syncing a vault with a device (e.g. it left our vault).
    pub fn remove_share(&self, vault: Uuid, peer: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM vault_shares WHERE vault_id = ?1 AND peer = ?2", params![vault.as_bytes(), peer])?;
        Ok(())
    }

    /// Shares one of our vaults with a paired device.
    pub fn share_vault(&self, vault: Uuid, peer: &str) -> Result<()> {
        self.get_vault(vault)?;
        if self.vault_owner(vault)?.is_some() {
            return Err(Error::invalid("only the vault's owner can share it"));
        }
        self.set_share(vault, peer, ShareRole::Member)
    }

    /// Stops sharing one of our vaults with a device (its copy is removed on its side).
    pub fn unshare_vault(&self, vault: Uuid, peer: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM vault_shares WHERE vault_id = ?1 AND peer = ?2 AND role = 'member'",
            params![vault.as_bytes(), peer],
        )?;
        Ok(())
    }

    /// A device we're paired with shares its vault with us: start syncing it (unless we just
    /// left it and the owner hasn't heard yet). Returns whether it was accepted.
    pub fn accept_shared_vault(&mut self, vault: Uuid, owner: &str) -> Result<bool> {
        let current: Option<String> = self
            .conn
            .query_row(
                "SELECT role FROM vault_shares WHERE vault_id = ?1 AND peer = ?2",
                params![vault.as_bytes(), owner],
                |r| r.get(0),
            )
            .optional()?;
        if current.as_deref() == Some("left") {
            return Ok(false);
        }
        let exists: bool = self
            .conn
            .query_row("SELECT 1 FROM vaults WHERE id = ?1", [vault.as_bytes()], |_| Ok(true))
            .optional()?
            .unwrap_or(false);
        if exists && self.vault_owner(vault)?.as_deref() != Some(owner) {
            // A vault of ours with the same id. Only a leftover copy from the old
            // merge-everything sync can collide like that; anything else is ours and stays ours.
            let legacy: bool = self
                .conn
                .query_row("SELECT 1 FROM legacy_vaults WHERE id = ?1", [vault.as_bytes()], |_| Ok(true))
                .optional()?
                .unwrap_or(false);
            if !legacy {
                return Ok(false);
            }
            // Keep our copy as our own vault (new id), then receive theirs beside it.
            if self.vault_is_encrypted(vault)? && !self.vault_keys.contains_key(&vault) {
                return Ok(false); // needs the password to copy; accepted once unlocked
            }
            self.fork_vault(vault)?;
            self.purge_vault(vault)?;
            self.set_share(vault, owner, ShareRole::Owner)?;
            return Ok(true);
        }
        self.set_share(vault, owner, ShareRole::Owner)?;
        Ok(true)
    }

    /// Leaves a vault someone shared with us: our copy is removed. Returns the owner, who
    /// should be told.
    pub fn leave_vault(&mut self, vault: Uuid) -> Result<String> {
        let owner = self.vault_owner(vault)?.ok_or_else(|| Error::invalid("this vault is yours; delete it instead"))?;
        self.purge_vault(vault)?;
        self.set_share(vault, &owner, ShareRole::Left)?;
        Ok(owner)
    }

    /// The shared vault is gone on the owner's side, or they stopped sharing: drop our copy.
    pub fn drop_shared_vault(&mut self, vault: Uuid) -> Result<()> {
        if self.vault_owner(vault)?.is_none() {
            return Ok(()); // ours: never removed by someone else
        }
        self.purge_vault(vault)
    }

    /// A device is no longer paired: drop the vaults it shared with us, forget what we shared
    /// with it. Returns whether any vault was removed.
    pub fn forget_peer(&mut self, peer: &str) -> Result<bool> {
        let mut removed = false;
        for s in self.shares_with(peer)? {
            if s.role == ShareRole::Owner {
                self.purge_vault(s.vault_id)?;
                removed = true;
            }
        }
        self.conn.execute("DELETE FROM vault_shares WHERE peer = ?1", [peer])?;
        Ok(removed)
    }

    /// Removes a vault and everything in it from this device only: no tombstones, so other
    /// devices aren't affected.
    pub(crate) fn purge_vault(&mut self, vault: Uuid) -> Result<()> {
        let id = vault.as_bytes();
        {
            self.sync.applying.store(true, Ordering::SeqCst);
            let _guard = ApplyingGuard(&self.sync.applying);
            self.conn.execute("DELETE FROM vaults WHERE id = ?1", [id])?;
            self.conn.execute("DELETE FROM tombstones WHERE vault_id = ?1", [id])?;
            self.conn.execute("DELETE FROM sync_seen_vault WHERE vault_id = ?1", [id])?;
            self.conn.execute("DELETE FROM vault_shares WHERE vault_id = ?1", [id])?;
        }
        self.forget_cloud_vault(vault)?;
        self.vault_keys.remove(&vault);
        self.prune_tags()?;
        self.ensure_own_vault()?;
        Ok(())
    }

    /// Copies a vault into a new one of our own (same name, password, folders, notes, tags,
    /// pins, archive state and images; tasks follow their notes). Returns the new vault's id.
    pub(crate) fn fork_vault(&mut self, old: Uuid) -> Result<Uuid> {
        use std::collections::HashMap;
        let new = Uuid::now_v7();
        // The vault row, with the same encryption parameters (so the same password opens it).
        let cols: Vec<String> = {
            let mut stmt = self.conn.prepare("PRAGMA table_info(vaults)")?;
            stmt.query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|c| !matches!(c.as_str(), "id" | "hlc" | "origin"))
                .collect()
        };
        self.conn.execute(
            &format!("INSERT INTO vaults (id, {0}) SELECT ?1, {0} FROM vaults WHERE id = ?2", cols.join(", ")),
            params![new.as_bytes(), old.as_bytes()],
        )?;
        if let Some(key) = self.vault_keys.get(&old).cloned() {
            self.vault_keys.insert(new, key);
        }

        // Folders, parents first.
        let mut folder_map: HashMap<Uuid, Uuid> = HashMap::new();
        let mut pending = self.list_folders(Some(old))?;
        while !pending.is_empty() {
            let before = pending.len();
            let mut rest = Vec::new();
            for f in pending {
                let parent = match f.parent_id {
                    None => Some(None),
                    Some(p) => folder_map.get(&p).map(|n| Some(*n)),
                };
                match parent {
                    Some(parent) => {
                        let created = self.create_folder(Some(new), parent, &f.name)?;
                        folder_map.insert(f.id, created.id);
                    }
                    None => rest.push(f),
                }
            }
            if rest.len() == before {
                // Orphans (parent missing): put them at the top.
                for f in rest {
                    let created = self.create_folder(Some(new), None, &f.name)?;
                    folder_map.insert(f.id, created.id);
                }
                break;
            }
            pending = rest;
        }

        // Notes (with their images), keeping pins, tags and archive state.
        let notes = self.list_notes(&crate::notes::ListFilter {
            vault_id: Some(old),
            archive: crate::notes::ArchiveFilter::All,
            ..Default::default()
        })?;
        for n in notes {
            let mut body = n.body.clone();
            for att in crate::attachments::attachment_ids_in(&n.body) {
                if let Ok((mime, bytes)) = self.get_attachment(att)
                    && let Ok(copy) = self.add_attachment(Some(new), "image", &mime, &bytes)
                {
                    body = body.replace(&format!("attachments/{att}"), &format!("attachments/{}", copy.id));
                }
            }
            let created = self.create_note(crate::notes::NewNote {
                kind: n.kind,
                title: n.title.clone(),
                body,
                tags: n.tags.clone(),
                pinned: n.pinned,
                vault_id: Some(new),
                folder_id: n.folder_id.and_then(|f| folder_map.get(&f).copied()),
            })?;
            if n.archived_at.is_some() {
                self.archive_note(created.id)?;
            }
            self.conn.execute(
                "UPDATE tasks SET note_id = ?2 WHERE note_id = ?1",
                params![n.id.as_bytes(), created.id.as_bytes()],
            )?;
        }
        if self.get_setting("auto_vault")?.as_deref() == Some(old.to_string().as_str()) {
            self.set_setting("auto_vault", &new.to_string())?;
        }
        Ok(new)
    }

    /// Makes sure this device has a vault of its own (not one shared by someone else).
    pub(crate) fn ensure_own_vault(&self) -> Result<()> {
        let own: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM vaults v
             WHERE NOT EXISTS (SELECT 1 FROM vault_shares s WHERE s.vault_id = v.id AND s.role = 'owner')",
            [],
            |r| r.get(0),
        )?;
        if own == 0 {
            self.create_default_vault()?;
        }
        Ok(())
    }

    /// The highest clock seen per origin, for one vault's rows.
    pub fn vault_version_vector(&self, vault: Uuid) -> Result<VersionVector> {
        let mut vv = VersionVector::new();
        let id = vault.as_bytes();
        let mut sources: Vec<String> = VAULT_TABLES
            .iter()
            .map(|t| {
                let col = if *t == "vaults" { "id" } else { "vault_id" };
                format!("SELECT origin, MAX(hlc) FROM {t} WHERE {col} = ?1 GROUP BY origin")
            })
            .collect();
        sources.push("SELECT origin, MAX(hlc) FROM tombstones WHERE vault_id = ?1 GROUP BY origin".into());
        sources.push("SELECT origin, MAX(hlc) FROM sync_seen_vault WHERE vault_id = ?1 GROUP BY origin".into());
        for sql in sources {
            let mut stmt = self.conn.prepare_cached(&sql)?;
            for row in stmt.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (o, h) = row?;
                if o.is_empty() {
                    continue;
                }
                let e = vv.entry(o).or_default();
                if h > *e {
                    *e = h;
                }
            }
        }
        Ok(vv)
    }

    /// One vault's changes newer than `seen`.
    pub fn vault_changes_since(&self, vault: Uuid, seen: &VersionVector) -> Result<ChangeSet> {
        self.collect_changes(seen, Some(vault))
    }

    /// Clocks per vault for a set of vaults (what we tell a device we've seen).
    pub fn vault_version_vectors(&self, vaults: &[Uuid]) -> Result<BTreeMap<Uuid, VersionVector>> {
        vaults.iter().map(|v| Ok((*v, self.vault_version_vector(*v)?))).collect()
    }

    /// What a device may sync with us: the vaults we share with it, either way.
    pub fn sync_scope(&self, peer: &str) -> Result<SyncScope> {
        let mut scope = SyncScope::default();
        for s in self.shares_with(peer)? {
            match s.role {
                ShareRole::Owner => {
                    scope.vaults.insert(s.vault_id);
                    scope.owned_by_sender.insert(s.vault_id);
                }
                ShareRole::Member => {
                    scope.vaults.insert(s.vault_id);
                }
                ShareRole::Left => {}
            }
        }
        Ok(scope)
    }

    /// The vault a note is in (for routing live edits).
    pub fn note_vault(&self, note: Uuid) -> Result<Option<Uuid>> {
        Ok(self
            .conn
            .query_row("SELECT vault_id FROM notes WHERE id = ?1", [note.as_bytes()], |r| crate::util::opt_uuid_col(r, 0))
            .optional()?
            .flatten())
    }
}
