//! Bakes the Google OAuth client into the app, so nobody has to paste it in Settings.
//!
//! Looks for, in order:
//! - `ASTRALIS_GOOGLE_CLIENT_ID` / `ASTRALIS_GOOGLE_CLIENT_SECRET` environment variables,
//! - `google-oauth.json` next to this file, or any `client_secret_*.json` Google let you
//!   download (Desktop-app clients). These files are git-ignored: keep them out of the repo.
//!
//! Without one, the app still builds; Settings then asks for a client instead.

use std::path::{Path, PathBuf};

fn from_file(dir: &Path) -> Option<(String, String)> {
    let mut candidates: Vec<PathBuf> = vec![dir.join("google-oauth.json")];
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("client_secret_") && name.ends_with(".json") {
                candidates.push(e.path());
            }
        }
    }
    for path in candidates {
        println!("cargo:rerun-if-changed={}", path.display());
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            println!("cargo:warning={} isn't valid JSON; ignoring it", path.display());
            continue;
        };
        // Google's download wraps it in "installed"; also accept a flat object.
        let c = json.get("installed").unwrap_or(&json);
        if let (Some(id), Some(secret)) = (c["client_id"].as_str(), c["client_secret"].as_str()) {
            return Some((id.to_owned(), secret.to_owned()));
        }
        println!("cargo:warning={} has no client_id/client_secret; ignoring it", path.display());
    }
    None
}

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-changed={}", dir.display());
    println!("cargo:rerun-if-env-changed=ASTRALIS_GOOGLE_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=ASTRALIS_GOOGLE_CLIENT_SECRET");
    let client = match (std::env::var("ASTRALIS_GOOGLE_CLIENT_ID"), std::env::var("ASTRALIS_GOOGLE_CLIENT_SECRET")) {
        (Ok(id), Ok(secret)) if !id.is_empty() && !secret.is_empty() => Some((id, secret)),
        _ => from_file(&dir),
    };
    if let Some((id, secret)) = client {
        println!("cargo:rustc-env=ASTRALIS_BUILTIN_GOOGLE_CLIENT_ID={id}");
        println!("cargo:rustc-env=ASTRALIS_BUILTIN_GOOGLE_CLIENT_SECRET={secret}");
    }
    tauri_build::build()
}
