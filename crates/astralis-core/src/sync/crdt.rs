//! Note text as a CRDT (yrs). Edits are recorded as minimal splices so that concurrent edits on
//! different devices merge character by character instead of one overwriting the other.

use yrs::updates::decoder::Decode;
use yrs::{
    Doc, GetString, OffsetKind, Options, ReadTxn, StateVector, Text, Transact, Update, WriteTxn,
};

use crate::error::{Error, Result};

const FIELD: &str = "body";

fn err(e: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("note text merge failed: {e}"))
}

/// A stable yrs client id for this device (53 bits, safe everywhere).
pub(crate) fn client_id(device: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in device.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    (h & 0x1F_FFFF_FFFF_FFFF).max(1)
}

fn doc(client: u64) -> Doc {
    Doc::with_options(Options {
        client_id: yrs::ClientID::new(client),
        offset_kind: OffsetKind::Utf16,
        ..Default::default()
    })
}

fn load(d: &Doc, state: &[u8]) -> Result<()> {
    let update = Update::decode_v1(state).map_err(err)?;
    d.transact_mut().apply_update(update).map_err(err)?;
    Ok(())
}

fn encode(d: &Doc) -> Vec<u8> {
    d.transact()
        .encode_state_as_update_v1(&StateVector::default())
}

fn text_of(d: &Doc) -> String {
    let mut txn = d.transact_mut();
    let t = txn.get_or_insert_text(FIELD);
    t.get_string(&txn)
}

pub(crate) fn read_text(state: &[u8]) -> Result<String> {
    let d = doc(1);
    load(&d, state)?;
    Ok(text_of(&d))
}

/// Records the change from the state's current text to `new_text` as a single splice.
pub(crate) fn set_text(state: Option<&[u8]>, new_text: &str, client: u64) -> Result<Vec<u8>> {
    let d = doc(client);
    if let Some(s) = state {
        load(&d, s)?;
    }
    {
        let mut txn = d.transact_mut();
        let t = txn.get_or_insert_text(FIELD);
        let old: Vec<u16> = t.get_string(&txn).encode_utf16().collect();
        let new: Vec<u16> = new_text.encode_utf16().collect();
        let mut prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        // Never split a surrogate pair.
        if prefix > 0 && (0xD800..0xDC00).contains(&old[prefix - 1]) {
            prefix -= 1;
        }
        let max_suffix = old.len().min(new.len()) - prefix;
        let mut suffix = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        if suffix > 0 && (0xDC00..0xE000).contains(&old[old.len() - suffix]) {
            suffix -= 1;
        }
        let removed = old.len() - prefix - suffix;
        if removed > 0 {
            t.remove_range(&mut txn, prefix as u32, removed as u32);
        }
        let inserted = &new[prefix..new.len() - suffix];
        if !inserted.is_empty() {
            t.insert(&mut txn, prefix as u32, &String::from_utf16_lossy(inserted));
        }
    }
    Ok(encode(&d))
}

/// Merges two states; returns the merged state and its text.
pub(crate) fn merge(a: &[u8], b: &[u8]) -> Result<(Vec<u8>, String)> {
    let d = doc(1);
    load(&d, a)?;
    load(&d, b)?;
    Ok((encode(&d), text_of(&d)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_edits_both_survive() {
        let base = set_text(None, "Hello world", 1).unwrap();
        let a = set_text(Some(&base), "Hello brave world", 2).unwrap();
        let b = set_text(Some(&base), "Hello world!", 3).unwrap();
        let (merged, text) = merge(&a, &b).unwrap();
        assert_eq!(text, "Hello brave world!");
        assert_eq!(merge(&b, &a).unwrap().1, text, "order doesn't matter");
        assert_eq!(merge(&merged, &a).unwrap().1, text, "idempotent");
    }

    #[test]
    fn handles_emoji() {
        let base = set_text(None, "a🚀b", 1).unwrap();
        let next = set_text(Some(&base), "a🚀🌍b", 2).unwrap();
        assert_eq!(read_text(&next).unwrap(), "a🚀🌍b");
    }
}
