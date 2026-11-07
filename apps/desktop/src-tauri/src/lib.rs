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
    pub store: Mutex<Store>,
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
            let data_dir = app.path().app_data_dir()?;
            let store = Store::open(data_dir.join("astralis.db"))?;
            let brain = Brain::new(find_model_dir(app.handle()));
            let state: SharedState = Arc::new(AppState {
                store: Mutex::new(store),
                brain: Mutex::new(brain),
                grammar: Mutex::new(None),
            });
            app.manage(state.clone());
            spawn_reminder_loop(app.handle().clone(), state.clone());
            spawn_brain_loop(app.handle().clone(), state);
            Ok(())
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("error while running Astralis");
}
