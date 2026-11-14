//! "Ask Astralis": the local writing model, downloaded once and run on this device.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use astralis_cloud::reqwest;
use astralis_core::ai::assistant::{ACTIONS, ChatTurn, Output, Request, build_prompt, clean_reply};
use astralis_core::ai::llm::{self, Llm};
use astralis_core::{Error, Result};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::SharedState;

type S<'a> = State<'a, SharedState>;

/// Longest reply, in tokens (a page or so).
const MAX_REPLY: usize = 1100;
/// How much note + conversation the model sees (characters; ~3 per token leaves room to reply).
const PROMPT_BUDGET: usize = 8000;
/// Free the model's memory after this long unused.
const IDLE_UNLOAD: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct Assistant {
    /// Where the model lives (shared by every profile).
    pub dir: std::sync::OnceLock<PathBuf>,
    model: Mutex<Option<(Llm, Instant)>>,
    cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    downloading: AtomicBool,
}

fn dir(state: &SharedState) -> Result<PathBuf> {
    state.assistant.dir.get().cloned().ok_or_else(|| Error::Invalid("the assistant isn't ready yet".into()))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantStatus {
    /// The model is downloaded.
    ready: bool,
    downloading: bool,
    /// Loaded in memory (replies start right away).
    loaded: bool,
    actions: Vec<ActionInfo>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionInfo {
    id: &'static str,
    label: &'static str,
    hint: &'static str,
    group: &'static str,
    output: Output,
}

#[tauri::command]
pub fn assistant_status(state: S<'_>) -> Result<AssistantStatus> {
    let d = dir(&state)?;
    Ok(AssistantStatus {
        ready: llm::files_present(&d),
        downloading: state.assistant.downloading.load(Ordering::SeqCst),
        // Peek only: while a reply is being written the model is busy (and loaded).
        loaded: state.assistant.model.try_lock().map(|m| m.is_some()).unwrap_or(true),
        actions: ACTIONS.iter().map(|a| ActionInfo { id: a.id, label: a.label, hint: a.hint, group: a.group, output: a.output }).collect(),
    })
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    file: String,
    done: u64,
    total: Option<u64>,
}

/// Downloads the model (about 2.4 GB) with progress events (`assistant-download`).
#[tauri::command]
pub async fn assistant_download(app: AppHandle, state: S<'_>) -> Result<()> {
    if state.assistant.downloading.swap(true, Ordering::SeqCst) {
        return Err(Error::Invalid("already downloading".into()));
    }
    let result = download(&app, &dir(&state)?).await;
    state.assistant.downloading.store(false, Ordering::SeqCst);
    result
}

async fn download(app: &AppHandle, dir: &PathBuf) -> Result<()> {
    let fail = |e: &dyn std::fmt::Display| Error::Invalid(format!("download failed: {e}"));
    std::fs::create_dir_all(dir)?;
    let http = reqwest::Client::new();
    for (file, url) in llm::DOWNLOADS {
        let target = dir.join(file);
        if target.metadata().is_ok_and(|m| m.len() > 0) {
            continue;
        }
        let part = dir.join(format!("{file}.part"));
        let mut resp = http.get(*url).send().await.map_err(|e| fail(&e))?;
        if !resp.status().is_success() {
            return Err(fail(&format!("{} for {file}", resp.status())));
        }
        let total = resp.content_length();
        let mut out = std::fs::File::create(&part)?;
        let mut done: u64 = 0;
        let mut last_emit = Instant::now();
        while let Some(chunk) = resp.chunk().await.map_err(|e| fail(&e))? {
            out.write_all(&chunk)?;
            done += chunk.len() as u64;
            if last_emit.elapsed() > Duration::from_millis(250) {
                last_emit = Instant::now();
                let _ = app.emit("assistant-download", Progress { file: file.to_string(), done, total });
            }
        }
        out.flush()?;
        drop(out);
        if total.is_some_and(|t| t != done) {
            let _ = std::fs::remove_file(&part);
            return Err(fail(&format!("{file} was cut short; try again")));
        }
        std::fs::rename(&part, &target)?;
        let _ = app.emit("assistant-download", Progress { file: file.to_string(), done, total });
    }
    Ok(())
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Token {
    id: String,
    text: String,
}

/// Works on a note (or the selected part of it): an action, or a typed request. Streams the reply
/// as `assistant-token` events and returns it.
#[tauri::command]
pub async fn assistant_chat(
    app: AppHandle,
    state: S<'_>,
    request_id: String,
    note_id: Uuid,
    history: Vec<ChatTurn>,
    action: Option<String>,
    message: String,
    selection: Option<String>,
) -> Result<String> {
    let st = state.inner().clone();
    let cancel = Arc::new(AtomicBool::new(false));
    st.assistant.cancels.lock().unwrap().insert(request_id.clone(), cancel.clone());
    let id = request_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<String> {
        let note = st.store.lock().unwrap().get_note(note_id)?;
        let (prompt, _) = build_prompt(&note.title, &note.body, &history, &Request { action, message, selection }, PROMPT_BUDGET).map_err(Error::Invalid)?;
        let dir = dir(&st)?;
        let mut slot = st.assistant.model.lock().unwrap();
        if slot.is_none() {
            if !llm::files_present(&dir) {
                return Err(Error::Invalid("download the assistant's model first".into()));
            }
            let _ = app.emit("assistant-token", Token { id: id.clone(), text: String::new() });
            *slot = Some((Llm::load(&dir)?, Instant::now()));
        }
        let (model, used) = slot.as_mut().unwrap();
        *used = Instant::now();
        let reply = model.generate(&prompt, MAX_REPLY, |piece| {
            let _ = app.emit("assistant-token", Token { id: id.clone(), text: piece.to_string() });
            !cancel.load(Ordering::SeqCst)
        })?;
        *used = Instant::now();
        Ok(clean_reply(&reply))
    })
    .await
    .map_err(|e| Error::Invalid(format!("the assistant stopped unexpectedly: {e}")))?;
    state.assistant.cancels.lock().unwrap().remove(&request_id);
    result
}

#[tauri::command]
pub fn assistant_cancel(state: S<'_>, request_id: String) {
    if let Some(c) = state.assistant.cancels.lock().unwrap().get(&request_id) {
        c.store(true, Ordering::SeqCst);
    }
}

/// Frees the model's memory when it hasn't been used for a while.
pub fn spawn_idle_unload(state: SharedState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Ok(mut slot) = state.assistant.model.try_lock()
                && slot.as_ref().is_some_and(|(_, used)| used.elapsed() > IDLE_UNLOAD)
            {
                *slot = None;
            }
        }
    });
}

/// Frees the model right away (the panel was closed), stopping any reply being written.
#[tauri::command]
pub async fn assistant_unload(state: S<'_>) -> Result<()> {
    for c in state.assistant.cancels.lock().unwrap().values() {
        c.store(true, Ordering::SeqCst);
    }
    let st = state.inner().clone();
    // Waits for a stopped reply to let go of the model (it checks for a stop every word).
    tauri::async_runtime::spawn_blocking(move || *st.assistant.model.lock().unwrap() = None)
        .await
        .map_err(|e| Error::Invalid(format!("the assistant stopped unexpectedly: {e}")))
}
