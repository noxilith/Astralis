//! Ask Astralis on a device that can't run the local model (phones): the same commands, which say
//! so. The app hides the assistant there, so these are only a safety net.

use std::path::PathBuf;

use astralis_core::{Error, Result};
use tauri::State;

use crate::SharedState;

#[derive(Default)]
pub struct Assistant {
    /// Where the model would live (unused here).
    pub dir: std::sync::OnceLock<PathBuf>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantStatus {
    ready: bool,
    downloading: bool,
    loaded: bool,
    actions: Vec<String>,
}

fn unavailable() -> Error {
    Error::Invalid("Ask Astralis isn't available on this device".into())
}

pub fn spawn_idle_unload(_state: SharedState) {}

#[tauri::command]
pub fn assistant_status() -> Result<AssistantStatus> {
    Ok(AssistantStatus { ready: false, downloading: false, loaded: false, actions: vec![] })
}

#[tauri::command]
pub async fn assistant_download() -> Result<()> {
    Err(unavailable())
}

#[tauri::command]
pub async fn assistant_chat(_state: State<'_, SharedState>) -> Result<String> {
    Err(unavailable())
}

#[tauri::command]
pub fn assistant_cancel() {}

#[tauri::command]
pub async fn assistant_unload() -> Result<()> {
    Ok(())
}
