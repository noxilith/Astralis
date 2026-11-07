use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Row;
use serde::{Deserialize, Deserializer};
use uuid::Uuid;

/// Milliseconds since the Unix epoch (UTC).
pub type Timestamp = i64;

pub fn now_ms() -> Timestamp {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as Timestamp)
        .unwrap_or(0)
}

pub(crate) fn uuid_col(row: &Row<'_>, idx: usize) -> rusqlite::Result<Uuid> {
    let bytes: Vec<u8> = row.get(idx)?;
    Uuid::from_slice(&bytes).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(idx, rusqlite::types::Type::Blob, Box::new(e))
    })
}

pub(crate) fn opt_uuid_col(row: &Row<'_>, idx: usize) -> rusqlite::Result<Option<Uuid>> {
    let bytes: Option<Vec<u8>> = row.get(idx)?;
    bytes
        .map(|b| {
            Uuid::from_slice(&b).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    idx,
                    rusqlite::types::Type::Blob,
                    Box::new(e),
                )
            })
        })
        .transpose()
}

pub(crate) fn opt_bytes(id: Option<Uuid>) -> Option<[u8; 16]> {
    id.map(|u| *u.as_bytes())
}

/// Distinguishes "field absent" (`None`) from "field set to null" (`Some(None)`) in patches.
pub(crate) fn double_option<'de, T, D>(d: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// Stable 64-bit FNV-1a hash, used to detect content changes.
pub(crate) fn fnv1a(text: &str) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h as i64
}

/// Converts a char index into a UTF-16 code unit offset (what JavaScript strings use).
pub(crate) fn char_to_utf16_offsets(text: &str) -> Vec<usize> {
    let mut out = Vec::with_capacity(text.len() + 1);
    let mut acc = 0;
    for c in text.chars() {
        out.push(acc);
        acc += c.len_utf16();
    }
    out.push(acc);
    out
}
