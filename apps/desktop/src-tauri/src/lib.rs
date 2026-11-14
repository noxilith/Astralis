// Ask Astralis needs the local model (the "ml" feature); phones build without it.
#[cfg(feature = "ml")]
mod assistant_cmds;
#[cfg(not(feature = "ml"))]
#[path = "assistant_stub.rs"]
mod assistant_cmds;
mod brain_cmds;
mod cloud;
mod commands;
mod theme_cmds;
pub mod update;

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
    /// Google Drive sign-in (cloud vaults).
    pub cloud: cloud::Cloud,
    /// "Ask Astralis": the local writing model.
    pub assistant: assistant_cmds::Assistant,
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
        let mut scheduled = mobile_reminders::Scheduled::default();
        loop {
            scheduled.sync(&app, &state);
            let due = {
                let store = state.store.lock().unwrap();
                let due = store.due_reminders(now_ms()).unwrap_or_default();
                for r in &due {
                    let _ = store.mark_reminded(r.kind, r.id);
                }
                due
            };
            for r in due {
                // On a phone the system already showed the ones it was handed.
                if !scheduled.handed_over(&r) {
                    let _ = app
                        .notification()
                        .builder()
                        .title("Astralis reminder")
                        .body(&r.title)
                        .show();
                }
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
        .plugin_opener()
        .setup(|app| {
            cloud::remember_app(app.handle());
            // Leftovers from an update that just finished (the old .exe or .app).
            update::clean_up_after_update();
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
                cloud: Default::default(),
                assistant: Default::default(),
            });
            // One copy of the model for every profile.
            let _ = state.assistant.dir.set(app.path().app_data_dir()?.join("models").join("qwen2.5-1.5b"));
            assistant_cmds::spawn_idle_unload(state.clone());
            app.manage(state.clone());
            spawn_reminder_loop(app.handle().clone(), state.clone());
            spawn_brain_loop(app.handle().clone(), state.clone());
            cloud::spawn_sync_loop(app.handle().clone(), state.clone());
            brain_cmds::spawn_brain_watch(app.handle().clone(), state.clone());
            brain_cmds::spawn_outside_edits_watch(app.handle().clone(), state.clone());
            theme_cmds::spawn_matugen_watch(app.handle().clone());
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

/// The opener plugin exists on phones only.
trait OpenerPlugin {
    fn plugin_opener(self) -> Self;
}
impl OpenerPlugin for tauri::Builder<tauri::Wry> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    fn plugin_opener(self) -> Self {
        self.plugin(tauri_plugin_opener::init())
    }
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn plugin_opener(self) -> Self {
        self
    }
}

/// Phones suspend the app in the background, so its reminder loop can't fire them. The next
/// reminders are handed to the system as scheduled notifications instead (and re-handed whenever
/// they change). On desktop none of this does anything.
mod mobile_reminders {
    use super::*;

    #[derive(Default)]
    pub struct Scheduled {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        sent: std::collections::HashMap<uuid::Uuid, i64>,
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    impl Scheduled {
        pub fn sync(&mut self, _: &AppHandle, _: &SharedState) {}
        pub fn handed_over(&self, _: &astralis_core::Reminder) -> bool {
            false
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    impl Scheduled {
        pub fn sync(&mut self, app: &AppHandle, state: &SharedState) {
            use tauri_plugin_notification::Schedule;
            const AHEAD: i64 = 14 * 24 * 3600 * 1000;
            let now = now_ms();
            let upcoming = state.store.lock().unwrap().upcoming_reminders(now, now + AHEAD).unwrap_or_default();
            let wanted: std::collections::HashMap<_, _> = upcoming.iter().take(60).map(|r| (r.id, r.remind_at)).collect();
            // Only touch the system when something was added, moved or removed.
            let stale = self.sent.iter().any(|(id, at)| *at > now && wanted.get(id) != Some(at));
            if !stale && wanted.iter().all(|(id, at)| self.sent.get(id) == Some(at)) {
                return;
            }
            let n = app.notification();
            let _ = n.cancel_all();
            self.sent.retain(|_, at| *at <= now); // the ones that already fired stay known
            for r in upcoming.iter().take(60) {
                let Ok(date) = time::OffsetDateTime::from_unix_timestamp_nanos(r.remind_at as i128 * 1_000_000) else { continue };
                let id = (r.id.as_u128() as u32 & 0x7fff_ffff) as i32;
                let shown = n
                    .builder()
                    .id(id)
                    .title("Astralis reminder")
                    .body(&r.title)
                    .schedule(Schedule::At { date, repeating: false, allow_while_idle: true })
                    .show();
                if shown.is_ok() {
                    self.sent.insert(r.id, r.remind_at);
                }
            }
        }

        pub fn handed_over(&self, r: &astralis_core::Reminder) -> bool {
            self.sent.get(&r.id) == Some(&r.remind_at)
        }
    }
}
