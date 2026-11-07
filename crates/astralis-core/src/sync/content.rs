//! Content that merges instead of "last writer wins": note text (a CRDT, sealed with the vault
//! key in encrypted vaults) and whiteboard scenes (element by element).

use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use super::crdt;
use crate::error::Result;
use crate::notes::SecretContent;
use crate::store::Store;
use crate::vault::{open, seal};

/// A note's content in plaintext (decrypted if needed).
#[derive(Debug, Clone)]
pub(crate) struct Content {
    pub kind: String,
    pub vault: Uuid,
    pub encrypted: bool,
    pub title: String,
    pub body: String,
    /// Plaintext CRDT state (text notes).
    pub state: Option<Vec<u8>>,
}

fn crdt_aad(id: Uuid) -> Vec<u8> {
    [id.as_bytes().as_slice(), b":crdt"].concat()
}

/// Merges two Excalidraw scenes element by element: the newer version of each element wins
/// (ties broken by the smaller nonce, like Excalidraw). Deleted elements are kept as
/// tombstones (`isDeleted`), so a delete on one device isn't undone by the other.
pub fn merge_scene(local: &str, remote: &str) -> String {
    use serde_json::{Map, Value};
    let (Ok(mut a), Ok(b)) = (serde_json::from_str::<Value>(local), serde_json::from_str::<Value>(remote)) else {
        return if serde_json::from_str::<Value>(remote).is_ok() { remote.to_owned() } else { local.to_owned() };
    };
    let num = |e: &Value, k: &str| e.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let mut elements: Vec<Value> = a.get("elements").and_then(Value::as_array).cloned().unwrap_or_default();
    for r in b.get("elements").and_then(Value::as_array).cloned().unwrap_or_default() {
        let id = r.get("id").and_then(Value::as_str).map(str::to_owned);
        match elements.iter_mut().find(|e| e.get("id").and_then(Value::as_str).map(str::to_owned) == id) {
            Some(l) => {
                let newer = num(&r, "version") > num(l, "version")
                    || (num(&r, "version") == num(l, "version") && num(&r, "versionNonce") < num(l, "versionNonce"));
                if newer {
                    *l = r;
                }
            }
            None => elements.push(r),
        }
    }
    let mut files: Map<String, Value> = a.get("files").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(bf) = b.get("files").and_then(Value::as_object) {
        for (k, v) in bf {
            files.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    if let Some(obj) = a.as_object_mut() {
        obj.insert("elements".into(), Value::Array(elements));
        obj.insert("files".into(), Value::Object(files));
    }
    a.to_string()
}

impl Store {
    /// The local note's content, or `None` if it doesn't exist or its vault is locked.
    pub(crate) fn local_content(&self, id: Uuid) -> Result<Option<Content>> {
        type Row = (String, Option<Uuid>, Option<Vec<u8>>, String, String, Option<Vec<u8>>);
        let row: Option<Row> = self
            .conn
            .query_row(
                "SELECT kind, vault_id, cipher, title, body, crdt FROM notes WHERE id = ?1",
                [id.as_bytes()],
                |r| Ok((r.get(0)?, crate::util::opt_uuid_col(r, 1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?;
        // No vault yet only happens mid-upgrade, before notes are assigned to one.
        let Some((kind, Some(vault), cipher, title, body, crdt)) = row else { return Ok(None) };
        self.decode_content(id, kind, vault, cipher, title, body, crdt)
    }

    /// Decrypts content if needed. `None` when the vault is locked.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decode_content(
        &self,
        id: Uuid,
        kind: String,
        vault: Uuid,
        cipher: Option<Vec<u8>>,
        title: String,
        body: String,
        crdt: Option<Vec<u8>>,
    ) -> Result<Option<Content>> {
        let Some(cipher) = cipher else {
            return Ok(Some(Content { kind, vault, encrypted: false, title, body, state: crdt }));
        };
        let Ok(key) = self.require_unlocked(vault) else { return Ok(None) };
        let plain = open(key, id.as_bytes(), &cipher)?;
        let c: SecretContent = serde_json::from_slice(&plain)?;
        let state = match crdt {
            Some(s) => Some(open(key, &crdt_aad(id), &s)?.to_vec()),
            None => None,
        };
        Ok(Some(Content { kind, vault, encrypted: true, title: c.title, body: c.body, state }))
    }

    /// Writes merged text/scene (and CRDT state) into a note, keeping its current title.
    /// Encrypted notes are re-sealed. Doesn't touch the change clock (callers decide).
    pub(crate) fn write_content_raw(&self, id: Uuid, body: &str, state: Option<&[u8]>) -> Result<()> {
        let Some(cur) = self.local_content(id)? else { return Ok(()) };
        if cur.encrypted {
            let key = self.require_unlocked(cur.vault)?;
            let plain = serde_json::to_vec(&SecretContent { title: cur.title, body: body.to_owned() })?;
            let cipher = seal(key, id.as_bytes(), &plain)?;
            let sealed_state = state.map(|s| seal(key, &crdt_aad(id), s)).transpose()?;
            self.conn.execute(
                "UPDATE notes SET cipher = ?2, crdt = ?3 WHERE id = ?1",
                params![id.as_bytes(), cipher, sealed_state],
            )?;
        } else {
            self.conn.execute(
                "UPDATE notes SET body = ?2, crdt = ?3 WHERE id = ?1",
                params![id.as_bytes(), body, state],
            )?;
        }
        Ok(())
    }

    /// Keeps a text note's CRDT state in step with its body (plain or encrypted).
    pub(crate) fn refresh_crdt(&self, id: Uuid) -> Result<()> {
        let Some(c) = self.local_content(id)? else { return Ok(()) };
        if c.kind != "note" {
            if c.state.is_some() {
                self.conn.execute("UPDATE notes SET crdt = NULL WHERE id = ?1", [id.as_bytes()])?;
            }
            return Ok(());
        }
        if let Some(s) = &c.state
            && crdt::read_text(s)? == c.body
        {
            return Ok(());
        }
        let next = crdt::set_text(c.state.as_deref(), &c.body, crdt::client_id(&self.device_id()))?;
        if c.encrypted {
            let key = self.require_unlocked(c.vault)?;
            let sealed = seal(key, &crdt_aad(id), &next)?;
            self.conn.execute("UPDATE notes SET crdt = ?2 WHERE id = ?1", params![id.as_bytes(), sealed])?;
        } else {
            self.conn.execute("UPDATE notes SET crdt = ?2 WHERE id = ?1", params![id.as_bytes(), next])?;
        }
        Ok(())
    }

    /// Merges two contents of the same note. Returns `(body, state)` or `None` if they can't
    /// merge (different kind or vault, or no CRDT on one side).
    pub(crate) fn merge_contents(local: &Content, remote: &Content) -> Result<Option<(String, Option<Vec<u8>>)>> {
        if local.kind != remote.kind || local.vault != remote.vault {
            return Ok(None);
        }
        match local.kind.as_str() {
            "note" => match (&local.state, &remote.state) {
                (Some(a), Some(b)) => {
                    let (state, text) = crdt::merge(a, b)?;
                    Ok(Some((text, Some(state))))
                }
                _ => Ok(None),
            },
            "whiteboard" => Ok(Some((merge_scene(&local.body, &remote.body), None))),
            _ => Ok(None),
        }
    }
}

impl Store {
    /// The incoming (remote) version of a note's content, decrypted if needed.
    pub(crate) fn remote_content(
        &self,
        id: &[u8],
        values: &std::collections::BTreeMap<String, super::SqlValue>,
    ) -> Result<Option<Content>> {
        use super::SqlValue;
        let text = |k: &str| match values.get(k) {
            Some(SqlValue::Text(t)) => t.clone(),
            _ => String::new(),
        };
        let blob = |k: &str| match values.get(k) {
            Some(SqlValue::Blob(b)) => Some(b.clone()),
            _ => None,
        };
        let (Ok(id), Some(vault)) = (Uuid::from_slice(id), blob("vault_id").and_then(|b| Uuid::from_slice(&b).ok())) else {
            return Ok(None);
        };
        self.decode_content(id, text("kind"), vault, blob("cipher"), text("title"), text("body"), blob("crdt"))
    }
}
