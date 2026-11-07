//! Messages between devices, framed as `u32 length (big-endian) + JSON`.

use anyhow::{Result, bail};
use astralis_core::sync::{ChangeSet, VersionVector};
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};

/// Largest frame accepted (change sets with images can be big).
const MAX_FRAME: usize = 64 * 1024 * 1024;

/// Real-time collaboration while connected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Live {
    /// Where this device's user is: a note and caret/selection (UTF-16 offsets), or a
    /// whiteboard pointer (scene coordinates). All `None` = not looking at a note.
    #[serde(rename_all = "camelCase")]
    Presence {
        note_id: Option<String>,
        anchor: Option<u32>,
        head: Option<u32>,
        pointer: Option<[f64; 2]>,
    },
    /// A note's CRDT text state after a local edit.
    #[serde(rename_all = "camelCase")]
    Text { note_id: String, state: Vec<u8> },
    /// A whiteboard's changed elements (Excalidraw JSON), for live drawing.
    #[serde(rename_all = "camelCase")]
    Scene { note_id: String, elements: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub(crate) enum Wire {
    /// First message from the dialing device.
    Hello { name: String, pair_code: Option<String> },
    Welcome { name: String },
    Reject { reason: String },
    /// "This is what I've seen" — the peer replies with what's missing.
    Vv(VersionVector),
    Changes(ChangeSet),
    Live(Live),
}

pub(crate) async fn write_frame(send: &mut SendStream, msg: &Wire) -> Result<()> {
    let bytes = serde_json::to_vec(msg)?;
    if bytes.len() > MAX_FRAME {
        bail!("message too large");
    }
    send.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    send.write_all(&bytes).await?;
    Ok(())
}

pub(crate) async fn read_frame(recv: &mut RecvStream) -> Result<Wire> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        bail!("frame too large");
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}
