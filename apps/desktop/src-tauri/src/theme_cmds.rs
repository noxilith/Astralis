//! The Matugen theme: colors generated from the wallpaper by matugen, written to a file that
//! Astralis reads and follows.

use serde::Serialize;

#[derive(Serialize)]
pub struct MatugenFile {
    /// Where Astralis looks for matugen's output.
    path: String,
    /// The file's text (None if it isn't there yet).
    json: Option<String>,
}

fn config_dir() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from).unwrap_or_default();
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".config"))
    }
}

/// Astralis's own template output, where it tells people to point matugen.
fn colors_path() -> std::path::PathBuf {
    config_dir().join("astralis").join("matugen.json")
}

/// Files that hold matugen's palette. Besides our template's output, wallpaper tools that run
/// `matugen --json` themselves write the whole palette (`{"mode", "colors": {...}}`) somewhere
/// like `shelltint/matugen-colors.json`, and never trigger templates. The newest file wins.
fn candidates() -> Vec<std::path::PathBuf> {
    let c = config_dir();
    vec![colors_path(), c.join("shelltint").join("matugen-colors.json")]
}

fn modified(p: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).ok().filter(|m| m.is_file() && m.len() < 64 * 1024).and_then(|m| m.modified().ok())
}

/// The newest matugen palette on this computer.
#[tauri::command]
pub fn matugen_colors() -> MatugenFile {
    let best = candidates().into_iter().filter_map(|p| modified(&p).map(|t| (t, p))).max_by_key(|(t, _)| *t).map(|(_, p)| p);
    match best {
        Some(p) => MatugenFile { json: std::fs::read_to_string(&p).ok(), path: p.to_string_lossy().to_string() },
        None => MatugenFile { path: colors_path().to_string_lossy().to_string(), json: None },
    }
}

/// Tells the window the moment matugen rewrites its colors file (checked twice a second; a
/// stat is cheap), so the theme follows the wallpaper right away.
pub fn spawn_matugen_watch(app: tauri::AppHandle) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        let stamp = || candidates().iter().map(|p| std::fs::metadata(p).ok().map(|m| (m.modified().ok(), m.len()))).collect::<Vec<_>>();
        let mut last = stamp();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let now = stamp();
            if now != last {
                last = now;
                let _ = app.emit("matugen-changed", ());
            }
        }
    });
}
