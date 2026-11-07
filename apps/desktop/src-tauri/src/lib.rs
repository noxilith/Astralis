mod commands;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use astralis_core::ai::Brain;
use astralis_core::grammar::Grammar;
use astralis_core::{Store, now_ms};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

const MODEL_NAME: &str = "all-MiniLM-L6-v2";

pub struct AppState {
    /// Shared with the P2P service, which reads and applies changes.
    pub store: Arc<Mutex<Store>>,
    /// Peer-to-peer sync (started in the background after launch).
    pub net: std::sync::OnceLock<Arc<astralis_net::Net>>,
    pub brain: Mutex<Brain>,
    pub grammar: Mutex<Option<Grammar>>,
}

pub type SharedState = Arc<AppState>;

/// The neural model can live in the user's data dir (downloaded) or be bundled as a resource.
fn find_model_dir(app: &AppHandle) -> Option<PathBuf> {
    let candidates = [
        app.path()
            .app_data_dir()
            .ok()
            .map(|d| d.join("models").join(MODEL_NAME)),
        app.path()
            .resource_dir()
            .ok()
            .map(|d| d.join("models").join(MODEL_NAME)),
        // Dev builds: the repo's resources folder.
        Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources/models")
                .join(MODEL_NAME),
        ),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|d| d.join("model.safetensors").is_file())
}

/// Fires task/event reminders as system notifications and tells the UI.
fn spawn_reminder_loop(app: AppHandle, state: SharedState) {
    std::thread::spawn(move || {
        loop {
            let due = {
                let store = state.store.lock().unwrap();
                let due = store.due_reminders(now_ms()).unwrap_or_default();
                for r in &due {
                    let _ = store.mark_reminded(r.kind, r.id);
                }
                due
            };
            for r in due {
                let _ = app
                    .notification()
                    .builder()
                    .title("Astralis reminder")
                    .body(&r.title)
                    .show();
                let _ = app.emit("reminder", &r);
            }
            std::thread::sleep(Duration::from_secs(15));
        }
    });
}

/// Keeps the Brain's index fresh in the background and unloads the model when idle.
fn spawn_brain_loop(app: AppHandle, state: SharedState) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(3));
            let store = state.store.lock().unwrap();
            let mut brain = state.brain.lock().unwrap();
            let pending = brain.status(&store).map(|s| s.pending).unwrap_or(0);
            if pending > 0 {
                match brain.index(&store, 16) {
                    Ok(n) if n > 0 => {
                        let _ = app.emit("brain-indexed", n);
                    }
                    Err(e) => eprintln!("astralis: indexing failed: {e}"),
                    _ => {}
                }
            } else {
                brain.unload_if_idle();
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // ASTRALIS_PROFILE=name runs a separate profile (own database and device identity),
            // e.g. to try sync with two windows on one computer.
            let mut data_dir = app.path().app_data_dir()?;
            if let Ok(profile) = std::env::var("ASTRALIS_PROFILE") {
                data_dir = data_dir.join(format!("profile-{profile}"));
            }
            let store = Store::open(data_dir.join("astralis.db"))?;
            let brain = Brain::new(find_model_dir(app.handle()));
            let state: SharedState = Arc::new(AppState {
                store: Arc::new(Mutex::new(store)),
                net: std::sync::OnceLock::new(),
                brain: Mutex::new(brain),
                grammar: Mutex::new(None),
            });
            app.manage(state.clone());
            spawn_reminder_loop(app.handle().clone(), state.clone());
            spawn_brain_loop(app.handle().clone(), state.clone());
            spawn_p2p(app.handle().clone(), state, data_dir.join("device.key"));
            Ok(())
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("error while running Astralis");
}

/// Starts peer-to-peer sync. Events go to the UI as `p2p`; synced data also refreshes views.
fn spawn_p2p(app: AppHandle, state: SharedState, key_path: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let key = match astralis_net::load_or_create_key(&key_path) {
            Ok(k) => k,
            Err(e) => return eprintln!("astralis: no device key, P2P disabled: {e:#}"),
        };
        let emitter = app.clone();
        let sink: astralis_net::EventSink = Arc::new(move |event| {
            let _ = emitter.emit("p2p", &event);
        });
        match astralis_net::Net::start(state.store.clone(), key, Default::default(), sink).await {
            Ok(net) => {
                let _ = state.net.set(Arc::new(net));
                let _ = app.emit("p2p-ready", ());
            }
            Err(e) => eprintln!("astralis: P2P failed to start: {e:#}"),
        }
    });
}
