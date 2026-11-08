//! In-app updates: asks GitHub for the newest Astralis release and installs it in whatever
//! shape this copy was installed in. Ported from EloraNotes' updater.
//!
//! * **AppImage**: the image file is swapped for the new one, which is started again.
//! * **Flatpak**: the new `.flatpak` bundle is installed on the host with `flatpak install`
//!   (through `flatpak-spawn --host`), then the app is started again.
//! * **Windows setup** (the install wizard): the new setup runs in passive mode over the old
//!   install and starts the app again when it's done.
//! * **Windows portable**: the `.exe` is replaced in place (the running one is renamed aside).
//! * **macOS**: the downloaded `.app` replaces the old one, which is renamed aside.
//!
//! Every download is checked against the release's `SHA256SUMS` (or the SHA-256 GitHub lists
//! for the file) before anything is written into place, and a downloaded program is run once
//! with `--version` to make sure it starts on this system.
//!
//! Release files are named `Astralis_<version>_<platform><ending>`, e.g.
//! `Astralis_1.0.0_windows_x64_setup.exe`. (EloraNotes, which shared this repository, looks for
//! `-windows-x86_64-setup.exe` and the like, so it never mistakes an Astralis release for its own.)

use std::path::{Path, PathBuf};
use std::time::Duration;

use astralis_cloud::reqwest;
use astralis_core::{Error, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/noxilith/Astralis/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/noxilith/Astralis/releases/latest";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Published with every release: a SHA-256 for every other file, as `sha256sum` writes them.
const CHECKSUMS_ASSET: &str = "SHA256SUMS";
/// Refuse absurd downloads (an HTML error page, a swapped file).
const MIN_SIZE: u64 = 256 * 1024;
const MAX_SIZE: u64 = 1024 * 1024 * 1024;

fn fail(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

// ---- how this copy was installed ----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// Running from an AppImage; `$APPIMAGE` is the image file.
    AppImage(PathBuf),
    /// Inside the Flatpak sandbox. `user` is true for a per-user install.
    Flatpak { user: bool },
    /// Running from `Astralis.app`.
    MacBundle(PathBuf),
    /// Installed by the setup wizard (it leaves `uninstall.exe` next to the app).
    WindowsSetup,
    /// The portable `.exe`.
    WindowsPortable(PathBuf),
    /// Built from source or a dev build: updates come from wherever it was built.
    Unmanaged,
}

impl InstallKind {
    /// What the settings show ("AppImage", "Windows installer"…).
    pub fn label(&self) -> &'static str {
        match self {
            Self::AppImage(_) => "AppImage",
            Self::Flatpak { .. } => "Flatpak",
            Self::MacBundle(_) => "macOS app",
            Self::WindowsSetup => "Windows installer",
            Self::WindowsPortable(_) => "Windows portable",
            Self::Unmanaged => "Built from source",
        }
    }

    /// The ending of the release file that fits this installation.
    fn asset_ending(&self) -> Option<&'static str> {
        match self {
            Self::AppImage(_) => Some(".AppImage"),
            Self::Flatpak { .. } => Some(".flatpak"),
            Self::MacBundle(_) => Some(".app.zip"),
            Self::WindowsSetup => Some("_setup.exe"),
            Self::WindowsPortable(_) => Some("_portable.exe"),
            Self::Unmanaged => None,
        }
    }
}

pub fn install_kind() -> InstallKind {
    if cfg!(debug_assertions) {
        return InstallKind::Unmanaged;
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(image) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()).map(PathBuf::from) {
            return InstallKind::AppImage(image);
        }
        if let Ok(info) = std::fs::read_to_string("/.flatpak-info") {
            // `app-path=/home/me/.local/share/flatpak/app/…` for a per-user install.
            let user = info.lines().any(|l| l.starts_with("app-path=") && l.contains("/.local/share/flatpak/"));
            return InstallKind::Flatpak { user };
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(bundle) = std::env::current_exe().ok().and_then(|exe| exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)) {
            return InstallKind::MacBundle(bundle);
        }
    }
    #[cfg(windows)]
    {
        if let Ok(exe) = std::env::current_exe() {
            let installed = exe.parent().is_some_and(|dir| dir.join("uninstall.exe").is_file());
            return if installed { InstallKind::WindowsSetup } else { InstallKind::WindowsPortable(exe) };
        }
    }
    InstallKind::Unmanaged
}

/// The platform part of release file names for this machine (`None`: no builds for it).
pub fn platform_key() -> Option<&'static str> {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux_x64")
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows_x64")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("macos_arm64")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("macos_x64")
    } else {
        None
    }
}

// ---- versions and releases ------------------------------------------------------------------

fn version_tuple(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0), parts.next().flatten().unwrap_or(0)))
}

/// True when `candidate` is strictly newer than `current`. Unreadable versions are never newer.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((version_tuple(candidate), version_tuple(current)), (Some(a), Some(b)) if a > b)
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// `sha256:…` as GitHub lists it, when it does.
    pub digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub assets: Vec<Asset>,
}

fn parse_release(json: &str) -> Result<Release> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| fail(format!("GitHub sent something unexpected: {e}")))?;
    let tag = v["tag_name"].as_str().ok_or_else(|| fail("the latest release has no version"))?;
    let assets = v["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(Asset {
                        name: a["name"].as_str()?.to_string(),
                        url: a["browser_download_url"].as_str()?.to_string(),
                        size: a["size"].as_u64().unwrap_or(0),
                        digest: a["digest"].as_str().map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        notes: v["body"].as_str().unwrap_or("").to_string(),
        page: v["html_url"].as_str().unwrap_or(RELEASES_PAGE).to_string(),
        assets,
    })
}

/// The release file for this platform and kind of install.
fn select_asset<'a>(assets: &'a [Asset], platform: &str, kind: &InstallKind) -> Option<&'a Asset> {
    let ending = kind.asset_ending()?;
    assets.iter().find(|a| a.name.starts_with("Astralis_") && a.name.contains(&format!("_{platform}")) && a.name.ends_with(ending))
}

/// The hash `sha256sum` output gives for `name`, lowercased.
fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Astralis/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60 * 30))
        .connect_timeout(Duration::from_secs(15))
        .build()
        .expect("http client")
}

async fn latest_release() -> Result<Release> {
    let resp = http()
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| fail(format!("Couldn't reach GitHub: {e}")))?;
    if resp.status().as_u16() == 404 {
        return Err(fail("There are no releases yet."));
    }
    if !resp.status().is_success() {
        return Err(fail(format!("GitHub answered {}", resp.status())));
    }
    parse_release(&resp.text().await.map_err(|e| fail(format!("Couldn't read GitHub's answer: {e}")))?)
}

// ---- what the app sees --------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    /// The newest release, if it's newer than this copy.
    pub latest: Option<String>,
    /// It can be downloaded and installed from inside the app.
    pub can_install: bool,
    /// How this copy was installed ("AppImage", "Windows installer"…).
    pub install: String,
    /// What's new (the release notes, Markdown).
    pub notes: String,
    /// The release on GitHub.
    pub page: String,
    /// Download size in bytes.
    pub size: u64,
}

pub async fn check() -> Result<UpdateInfo> {
    let kind = install_kind();
    let release = latest_release().await?;
    let newer = is_newer(&release.version, CURRENT_VERSION);
    let asset = platform_key().and_then(|p| select_asset(&release.assets, p, &kind));
    Ok(UpdateInfo {
        current: CURRENT_VERSION.to_string(),
        latest: newer.then(|| release.version.clone()),
        can_install: newer && asset.is_some(),
        install: kind.label().to_string(),
        notes: release.notes.clone(),
        page: release.page.clone(),
        size: asset.map_or(0, |a| a.size),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    /// "download", "verify", "install".
    stage: &'static str,
    done: u64,
    total: u64,
}

/// Downloads the newest release, checks it, puts it in place and starts it. On success the app
/// has to quit right away (the caller does) so the new copy can take over.
pub async fn install(app: &AppHandle) -> Result<()> {
    let kind = install_kind();
    let platform = platform_key().ok_or_else(|| fail("There are no Astralis builds for this kind of computer yet."))?;
    let release = latest_release().await?;
    if !is_newer(&release.version, CURRENT_VERSION) {
        return Err(fail("You already have the newest version."));
    }
    let asset = select_asset(&release.assets, platform, &kind)
        .ok_or_else(|| fail("This release doesn't have a download for this kind of install; get it from the releases page."))?
        .clone();
    if asset.size != 0 && !(MIN_SIZE..=MAX_SIZE).contains(&asset.size) {
        return Err(fail("The release file has an unexpected size; get it from the releases page."));
    }

    // Download, with progress.
    let progress = |stage, done, total| {
        let _ = app.emit("update-progress", Progress { stage, done, total });
    };
    let mut resp = http().get(&asset.url).send().await.map_err(|e| fail(format!("The download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(fail(format!("The download failed: {}", resp.status())));
    }
    let total = resp.content_length().unwrap_or(asset.size);
    let mut data: Vec<u8> = Vec::with_capacity(total as usize);
    let mut last = std::time::Instant::now();
    while let Some(chunk) = resp.chunk().await.map_err(|e| fail(format!("The download failed: {e}")))? {
        data.extend_from_slice(&chunk);
        if data.len() as u64 > MAX_SIZE {
            return Err(fail("The download is far bigger than expected; stopped."));
        }
        if last.elapsed() > Duration::from_millis(200) {
            last = std::time::Instant::now();
            progress("download", data.len() as u64, total);
        }
    }
    progress("verify", data.len() as u64, total);

    // Check it's exactly the file the release lists.
    let expected = match release.assets.iter().find(|a| a.name == CHECKSUMS_ASSET) {
        Some(sums) => {
            let text = http().get(&sums.url).send().await.and_then(|r| r.error_for_status()).map_err(|e| fail(format!("Couldn't get the checksums: {e}")))?;
            let text = text.text().await.map_err(|e| fail(format!("Couldn't read the checksums: {e}")))?;
            checksum_for(&text, &asset.name).ok_or_else(|| fail(format!("The release's checksums don't list {}.", asset.name)))?
        }
        None => asset
            .digest
            .as_deref()
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(str::to_ascii_lowercase)
            .ok_or_else(|| fail("The release has no checksums, so the download can't be checked; get it from the releases page."))?,
    };
    let actual = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect::<String>();
    if actual != expected {
        return Err(fail("The download doesn't match the release's checksum, so it wasn't installed."));
    }

    progress("install", total, total);
    let data_owned = data;
    let kind_owned = kind.clone();
    tauri::async_runtime::spawn_blocking(move || put_in_place(&kind_owned, &data_owned, &asset.name))
        .await
        .map_err(|e| fail(format!("The update stopped unexpectedly: {e}")))?
}

// ---- putting it in place ------------------------------------------------------------------

/// Runs a downloaded program once (`--version` makes Astralis print its version and quit) so a
/// build that can't run here fails before it replaces anything.
fn can_run(program: &Path) -> Result<()> {
    let status = std::process::Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| fail(format!("The new version can't run on this system: {e}")))?;
    if status.success() { Ok(()) } else { Err(fail("The new version can't run on this system.")) }
}

fn start(program: &Path, args: &[&str]) -> Result<()> {
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| fail(format!("Couldn't start the new version: {e}")))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| fail(format!("Couldn't make the update executable: {e}")))
}

fn put_in_place(kind: &InstallKind, data: &[u8], name: &str) -> Result<()> {
    match kind {
        InstallKind::AppImage(image) => {
            #[cfg(unix)]
            {
                let staged = image.with_file_name(format!(".{}.update", image.file_name().and_then(|n| n.to_str()).unwrap_or("Astralis.AppImage")));
                std::fs::write(&staged, data).map_err(|e| fail(format!("Couldn't save the update next to the AppImage: {e}")))?;
                make_executable(&staged)?;
                if let Err(e) = can_run(&staged) {
                    let _ = std::fs::remove_file(&staged);
                    return Err(e);
                }
                std::fs::rename(&staged, image).map_err(|e| fail(format!("Couldn't replace the AppImage: {e}")))?;
                start(image, &[])
            }
            #[cfg(not(unix))]
            {
                let _ = (image, data);
                Err(fail("AppImages only run on Linux."))
            }
        }
        InstallKind::Flatpak { user } => {
            // The sandbox's cache folder is the same path on the host, so the host's flatpak
            // can read the bundle from there.
            let dir = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
            let bundle = dir.join(name);
            std::fs::write(&bundle, data).map_err(|e| fail(format!("Couldn't save the update: {e}")))?;
            let scope = if *user { "--user" } else { "--system" };
            let status = std::process::Command::new("flatpak-spawn")
                .args(["--host", "flatpak", "install", scope, "--noninteractive", "-y", "--bundle"])
                .arg(&bundle)
                .status()
                .map_err(|e| fail(format!("Couldn't run flatpak: {e}")))?;
            let _ = std::fs::remove_file(&bundle);
            if !status.success() {
                return Err(fail("flatpak couldn't install the update."));
            }
            let id = std::env::var("FLATPAK_ID").unwrap_or_else(|_| "app.astralis.desktop".into());
            start(Path::new("flatpak-spawn"), &["--host", "flatpak", "run", &id])
        }
        InstallKind::WindowsSetup => {
            if !data.starts_with(b"MZ") {
                return Err(fail("The download isn't a Windows installer."));
            }
            let setup = std::env::temp_dir().join(name);
            std::fs::write(&setup, data).map_err(|e| fail(format!("Couldn't save the update: {e}")))?;
            // Passive (a progress bar, no questions), over the old install, then start Astralis.
            start(&setup, &["/P", "/R", "/UPDATE"])
        }
        InstallKind::WindowsPortable(exe) => {
            if !data.starts_with(b"MZ") {
                return Err(fail("The download isn't a Windows program."));
            }
            let staged = exe.with_file_name("Astralis.update.exe");
            std::fs::write(&staged, data).map_err(|e| fail(format!("Couldn't save the update next to Astralis: {e}")))?;
            if let Err(e) = can_run(&staged) {
                let _ = std::fs::remove_file(&staged);
                return Err(e);
            }
            // Windows lets a running program be renamed, not overwritten.
            let old = exe.with_extension("exe.old");
            let _ = std::fs::remove_file(&old);
            std::fs::rename(exe, &old).map_err(|e| fail(format!("Couldn't move the old Astralis aside: {e}")))?;
            if let Err(e) = std::fs::rename(&staged, exe) {
                let _ = std::fs::rename(&old, exe);
                return Err(fail(format!("Couldn't put the update in place: {e}")));
            }
            start(exe, &[])
        }
        InstallKind::MacBundle(bundle) => install_mac(bundle, data),
        InstallKind::Unmanaged => Err(fail("This copy was built from source; update it by building the new version.")),
    }
}

fn install_mac(bundle: &Path, data: &[u8]) -> Result<()> {
    let parent = bundle.parent().ok_or_else(|| fail("Couldn't find the folder Astralis.app is in."))?;
    let staged = parent.join(".astralis-update");
    let _ = std::fs::remove_dir_all(&staged);
    if let Err(e) = unzip(data, &staged) {
        let _ = std::fs::remove_dir_all(&staged);
        return Err(e);
    }
    let new_app = std::fs::read_dir(&staged)
        .ok()
        .and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.extension().is_some_and(|e| e == "app")))
        .ok_or_else(|| fail("The download doesn't contain Astralis.app."))?;
    let program = std::fs::read_dir(new_app.join("Contents/MacOS"))
        .ok()
        .and_then(|d| d.flatten().map(|e| e.path()).find(|p| p.is_file()))
        .ok_or_else(|| fail("The downloaded app is missing its program."))?;
    #[cfg(unix)]
    make_executable(&program)?;
    can_run(&program)?;
    let backup = parent.join(".Astralis.app.old");
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::rename(bundle, &backup).map_err(|e| fail(format!("Couldn't move the old Astralis.app aside: {e}")))?;
    if let Err(e) = std::fs::rename(&new_app, bundle) {
        let _ = std::fs::rename(&backup, bundle);
        return Err(fail(format!("Couldn't install the update: {e}")));
    }
    let _ = std::fs::remove_dir_all(&staged);
    start(Path::new("open"), &["-n", &bundle.to_string_lossy()])
}

/// Unpacks a zip into `target`, refusing entries that would land outside it.
fn unzip(archive: &[u8], target: &Path) -> Result<()> {
    std::fs::create_dir_all(target).map_err(|e| fail(format!("Couldn't create the update folder: {e}")))?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| fail(format!("The download is unreadable: {e}")))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| fail(format!("The download is unreadable: {e}")))?;
        let relative = entry.enclosed_name().ok_or_else(|| fail("The download contains an unsafe path."))?;
        let out = target.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| fail(format!("Couldn't unpack the update: {e}")))?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| fail(format!("Couldn't unpack the update: {e}")))?;
        }
        // Symlinks (inside frameworks) are stored as small files holding the link target.
        #[cfg(unix)]
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            let mut link = String::new();
            std::io::Read::read_to_string(&mut entry, &mut link).map_err(|e| fail(format!("Couldn't unpack the update: {e}")))?;
            std::os::unix::fs::symlink(link, &out).map_err(|e| fail(format!("Couldn't unpack the update: {e}")))?;
            continue;
        }
        let mut file = std::fs::File::create(&out).map_err(|e| fail(format!("Couldn't unpack the update: {e}")))?;
        std::io::copy(&mut entry, &mut file).map_err(|e| fail(format!("Couldn't unpack the update: {e}")))?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o777));
        }
    }
    Ok(())
}

/// Removes what an earlier update left behind (the old portable `.exe`, the old `.app`).
pub fn clean_up_after_update() {
    match install_kind() {
        InstallKind::WindowsPortable(exe) => {
            let _ = std::fs::remove_file(exe.with_extension("exe.old"));
            let _ = std::fs::remove_file(exe.with_file_name("Astralis.update.exe"));
        }
        InstallKind::MacBundle(bundle) => {
            if let Some(parent) = bundle.parent() {
                let _ = std::fs::remove_dir_all(parent.join(".Astralis.app.old"));
                let _ = std::fs::remove_dir_all(parent.join(".astralis-update"));
            }
        }
        InstallKind::AppImage(image) => {
            let _ = std::fs::remove_file(image.with_file_name(format!(".{}.update", image.file_name().and_then(|n| n.to_str()).unwrap_or(""))));
        }
        _ => {}
    }
}

// ---- commands -------------------------------------------------------------------------------

#[tauri::command]
pub async fn update_check() -> Result<UpdateInfo> {
    check().await
}

/// Installs the newest release and quits so it can take over.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<()> {
    install(&app).await?;
    // Give the new copy (or the installer) a moment to start, then make way for it.
    tokio::time::sleep(Duration::from_millis(400)).await;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset { name: name.into(), url: format!("https://x/{name}"), size: 1, digest: None }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v1.0.0", "0.15.9"));
        assert!(is_newer("1.10.0", "1.9.3"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("nonsense", "1.0.0"));
        assert!(is_newer("1.0.1-beta", "1.0.0"));
    }

    #[test]
    fn each_install_finds_its_own_file() {
        let assets: Vec<Asset> = [
            "Astralis_1.2.0_linux_x64.AppImage",
            "Astralis_1.2.0_linux_x64.flatpak",
            "Astralis_1.2.0_windows_x64_setup.exe",
            "Astralis_1.2.0_windows_x64_portable.exe",
            "Astralis_1.2.0_macos_arm64.dmg",
            "Astralis_1.2.0_macos_arm64.app.zip",
            "SHA256SUMS",
        ]
        .iter()
        .map(|n| asset(n))
        .collect();
        let pick = |platform, kind: InstallKind| select_asset(&assets, platform, &kind).map(|a| a.name.clone());
        assert_eq!(pick("linux_x64", InstallKind::AppImage("/x".into())).unwrap(), "Astralis_1.2.0_linux_x64.AppImage");
        assert_eq!(pick("linux_x64", InstallKind::Flatpak { user: true }).unwrap(), "Astralis_1.2.0_linux_x64.flatpak");
        assert_eq!(pick("windows_x64", InstallKind::WindowsSetup).unwrap(), "Astralis_1.2.0_windows_x64_setup.exe");
        assert_eq!(pick("windows_x64", InstallKind::WindowsPortable("x.exe".into())).unwrap(), "Astralis_1.2.0_windows_x64_portable.exe");
        assert_eq!(pick("macos_arm64", InstallKind::MacBundle("/A.app".into())).unwrap(), "Astralis_1.2.0_macos_arm64.app.zip");
        assert_eq!(pick("macos_x64", InstallKind::MacBundle("/A.app".into())), None);
        assert_eq!(pick("linux_x64", InstallKind::Unmanaged), None);
    }

    #[test]
    fn eloranotes_files_are_never_picked() {
        let assets = vec![asset("EloraNotes-0.15.9-linux-x86_64.AppImage"), asset("EloraNotes-0.15.9-windows-x86_64-setup.exe")];
        assert_eq!(select_asset(&assets, "linux_x64", &InstallKind::AppImage("/x".into())).map(|a| &a.name), None);
        assert_eq!(select_asset(&assets, "windows_x64", &InstallKind::WindowsSetup).map(|a| &a.name), None);
    }

    #[test]
    fn checksums_are_read_like_sha256sum_writes_them() {
        let h = "a".repeat(64);
        let sums = format!("{h}  Astralis_1.0.0_linux_x64.AppImage\n{}  *other.exe\n", "B".repeat(64));
        assert_eq!(checksum_for(&sums, "Astralis_1.0.0_linux_x64.AppImage"), Some(h));
        assert_eq!(checksum_for(&sums, "other.exe"), Some("b".repeat(64)));
        assert_eq!(checksum_for(&sums, "missing"), None);
    }

    #[test]
    fn a_github_release_is_read() {
        let json = r#"{"tag_name":"v1.2.0","body":"New things","html_url":"https://github.com/noxilith/Astralis/releases/tag/v1.2.0",
            "assets":[{"name":"Astralis_1.2.0_linux_x64.AppImage","browser_download_url":"https://x/a","size":123456,"digest":"sha256:ab"}]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "1.2.0");
        assert_eq!(r.assets[0].size, 123456);
        assert_eq!(r.assets[0].digest.as_deref(), Some("sha256:ab"));
    }
}
