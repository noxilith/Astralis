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
    /// An edit to a note's text: a Yjs update (v1) from the editor. Peers merge it into the
    /// note and into their open editor.
    #[serde(rename_all = "camelCase")]
    Update { note_id: String, update: Vec<u8> },
    /// Who is in a note and where their caret is: a Yjs awareness update (names, colors, and
    /// selections anchored to the text, so they move with edits).
    #[serde(rename_all = "camelCase")]
    Awareness { note_id: String, update: Vec<u8> },
    /// The sender's display name and color (sent on connect and whenever they change).
    #[serde(rename_all = "camelCase")]
    Profile { name: String, color: String },
    /// A whiteboard's changed elements (Excalidraw JSON), for live drawing.
    #[serde(rename_all = "camelCase")]
    Scene { note_id: String, elements: String },
}

/// An invite to a cloud vault, read from the drive folder it was shared in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequest {
    pub vault: uuid::Uuid,
    pub email: String,
    pub secret: String,
}

/// A shared vault, from the sender's point of view.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VaultOffer {
    pub vault: uuid::Uuid,
    pub role: OfferRole,
    pub seen: VersionVector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OfferRole {
    /// "I own this vault and share it with you."
    Mine,
    /// "You own this vault; I'm in it."
    Yours,
    /// "I left your vault."
    Left,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub(crate) enum Wire {
    /// First message from the dialing device.
    Hello {
        name: String,
        pair_code: Option<String>,
        /// Joining a cloud vault with an email invite (instead of pairing).
        #[serde(default)]
        join: Option<JoinRequest>,
    },
    Welcome { name: String },
    Reject { reason: String },
    /// The vaults shared between us and what I've seen of each; the peer replies with what's
    /// missing (and adds/drops shares to match).
    Vaults { offers: Vec<VaultOffer> },
    Changes(ChangeSet),
    Live(Live),
    /// "I unpaired you": forget me and hang up.
    Unpair,
    /// The owner of a shared cloud vault tells a member where it lives on the drive and the key
    /// its drive files are sealed with (only ever sent over this encrypted link).
    CloudInfo { vault: uuid::Uuid, provider: String, folder: String, key: Vec<u8> },
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
