//! The Projects view as a small editor: list a project's files, read and save one, and run the
//! project's run command (written by the AI tool in `.astralis/run.json` or `.astralis/run.sh`)
//! with its output shown in the app.
//!
//! Safety: a run command can come from the cloud (it is part of the synced project), so it is only
//! run after the user has confirmed this exact command here: on first run, and again whenever the
//! command, or a script it names, changes.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};

use astralis_core::{Error, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::SharedState;

type S<'a> = State<'a, SharedState>;

const MAX_FILES: usize = 20_000;
const MAX_READ: u64 = 1_500_000;
const MAX_LINES: usize = 5_000;
const TRUST: &str = "projects.run.trust";

fn root_of(state: &SharedState, hash: &str) -> Result<PathBuf> {
    let entry = state.store.lock().unwrap().projects()?.remove(hash).ok_or_else(|| Error::Invalid("unknown project".into()))?;
    let root = PathBuf::from(&entry.root);
    if entry.root.is_empty() || !root.is_dir() {
        return Err(Error::Invalid("that project's folder isn't on this device".into()));
    }
    Ok(root)
}

/// A path inside the project (relative, no `..`, and not escaping through a symlink).
fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    let p = Path::new(rel);
    if rel.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(Error::Invalid("that path isn't inside the project".into()));
    }
    let full = root.join(p);
    let root_real = root.canonicalize()?;
    // The file may not exist yet (saving a new one): check its nearest existing parent.
    let mut probe = full.as_path();
    while !probe.exists() {
        probe = probe.parent().ok_or_else(|| Error::Invalid("that path isn't inside the project".into()))?;
    }
    if !probe.canonicalize()?.starts_with(&root_real) {
        return Err(Error::Invalid("that path isn't inside the project".into()));
    }
    Ok(full)
}

/// The project's files: tracked, plus new ones that aren't ignored (`.gitignore`).
fn list_files(root: &Path) -> Result<Vec<String>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| Error::Invalid(format!("can't run git: {e}")))?;
    if !out.status.success() {
        return Err(Error::Invalid("couldn't list the files (is this a git project?)".into()));
    }
    let mut files: Vec<String> = String::from_utf8_lossy(&out.stdout).split('\0').filter(|p| !p.is_empty() && root.join(p).is_file()).map(str::to_owned).collect();
    files.sort();
    files.dedup();
    files.truncate(MAX_FILES);
    Ok(files)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileData {
    text: Option<String>,
    binary: bool,
    too_big: bool,
    size: u64,
}

fn read_file(root: &Path, rel: &str) -> Result<FileData> {
    let path = resolve(root, rel)?;
    let size = std::fs::metadata(&path)?.len();
    if size > MAX_READ {
        return Ok(FileData { text: None, binary: false, too_big: true, size });
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&path)?.read_to_end(&mut bytes)?;
    if bytes.contains(&0) {
        return Ok(FileData { text: None, binary: true, too_big: false, size });
    }
    Ok(FileData { text: Some(String::from_utf8_lossy(&bytes).to_string()), binary: false, too_big: false, size })
}

// ---- the run command ----

#[derive(Clone, Debug, PartialEq)]
struct RunSpec {
    name: String,
    command: String,
    /// Where it came from (relative to the project).
    file: String,
    /// Fingerprint of the command and the scripts it names.
    fingerprint: String,
}

/// Files in the project that the command names (`./run.sh`, `sh scripts/dev.sh`), up to three.
fn named_scripts(root: &Path, command: &str) -> Vec<PathBuf> {
    command
        .split_whitespace()
        .map(|t| t.trim_matches(|c| c == '"' || c == '\''))
        .filter(|t| !t.is_empty() && !t.starts_with('-'))
        .filter_map(|t| resolve(root, t.trim_start_matches("./")).ok())
        .filter(|p| p.is_file())
        .take(3)
        .collect()
}

fn run_spec(root: &Path) -> Option<RunSpec> {
    let json = root.join(".astralis/run.json");
    let (name, command, file) = if let Ok(text) = std::fs::read_to_string(&json) {
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        let command = v["command"].as_str()?.trim().to_string();
        (v["name"].as_str().unwrap_or("Run").to_string(), command, ".astralis/run.json".to_string())
    } else if root.join(".astralis/run.sh").is_file() {
        let shell = if cfg!(windows) { "bash" } else { "sh" };
        ("Run".to_string(), format!("{shell} .astralis/run.sh"), ".astralis/run.sh".to_string())
    } else {
        return None;
    };
    if command.is_empty() {
        return None;
    }
    let mut h = Sha256::new();
    h.update(command.as_bytes());
    for script in named_scripts(root, &command).into_iter().chain(std::iter::once(root.join(&file))) {
        h.update([0]);
        h.update(std::fs::read(script).unwrap_or_default());
    }
    let fingerprint = h.finalize()[..12].iter().map(|b| format!("{b:02x}")).collect();
    Some(RunSpec { name, command, file, fingerprint })
}

fn trusted(state: &SharedState, hash: &str, spec: &RunSpec) -> bool {
    let all: HashMap<String, String> = state.store.lock().unwrap().get_setting(TRUST).ok().flatten().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    all.get(hash).is_some_and(|f| *f == spec.fingerprint)
}

fn trust(state: &SharedState, hash: &str, spec: &RunSpec) -> Result<()> {
    let s = state.store.lock().unwrap();
    let mut all: HashMap<String, String> = s.get_setting(TRUST)?.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    all.insert(hash.to_string(), spec.fingerprint.clone());
    s.set_setting(TRUST, &serde_json::to_string(&all)?)
}

/// One run of a project's command: its output so far, and how it ended.
#[derive(Default)]
struct Run {
    lines: Vec<String>,
    /// Lines dropped from the front (the buffer is capped), so positions stay absolute.
    dropped: usize,
    pid: u32,
    exit: Option<i32>,
}

static RUNS: LazyLock<Mutex<HashMap<String, Arc<Mutex<Run>>>>> = LazyLock::new(Default::default);

fn push_line(run: &Mutex<Run>, line: String) {
    let mut r = run.lock().unwrap();
    r.lines.push(line);
    if r.lines.len() > MAX_LINES {
        r.lines.remove(0);
        r.dropped += 1;
    }
}

/// Starts `command` in `root` and returns the run (its output fills in as the process writes).
fn start(root: &Path, command: &str) -> Result<Arc<Mutex<Run>>> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0); // its own group, so Stop can end everything it started
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| Error::Invalid(format!("couldn't start it: {e}")))?;
    let run = Arc::new(Mutex::new(Run { pid: child.id(), ..Default::default() }));
    let readers: Vec<_> = [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)]
        .into_iter()
        .flatten()
        .map(|mut stream| {
            let run = run.clone();
            std::thread::spawn(move || {
                let mut pending: Vec<u8> = Vec::new();
                let mut buf = [0u8; 4096];
                while let Ok(n) = stream.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    pending.extend_from_slice(&buf[..n]);
                    while let Some(i) = pending.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = pending.drain(..=i).collect();
                        push_line(&run, String::from_utf8_lossy(&line).trim_end_matches(['\n', '\r']).to_string());
                    }
                }
                if !pending.is_empty() {
                    push_line(&run, String::from_utf8_lossy(&pending).to_string());
                }
            })
        })
        .collect();
    let waiting = run.clone();
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        for r in readers {
            let _ = r.join();
        }
        waiting.lock().unwrap().exit = Some(code);
    });
    Ok(run)
}

fn stop_pid(pid: u32) {
    #[cfg(unix)]
    {
        // The run was started as its own process group, so a negative pid reaches everything in it.
        // SAFETY: plain signal call with ids we got from our own child.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000).status();
    }
}

// ---- commands ----

#[tauri::command]
pub fn project_files(state: S<'_>, hash: String) -> Result<Vec<String>> {
    list_files(&root_of(&state, &hash)?)
}

#[tauri::command]
pub fn project_read(state: S<'_>, hash: String, path: String) -> Result<FileData> {
    read_file(&root_of(&state, &hash)?, &path)
}

#[tauri::command]
pub fn project_write(state: S<'_>, hash: String, path: String, text: String) -> Result<()> {
    let full = resolve(&root_of(&state, &hash)?, &path)?;
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(full, text)?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    name: String,
    command: String,
    file: String,
    trusted: bool,
    running: bool,
}

/// The project's run command, if its AI tool has written one.
#[tauri::command]
pub fn project_run_info(state: S<'_>, hash: String) -> Result<Option<RunInfo>> {
    let root = root_of(&state, &hash)?;
    let running = RUNS.lock().unwrap().get(&hash).is_some_and(|r| r.lock().unwrap().exit.is_none());
    Ok(run_spec(&root).map(|s| RunInfo { trusted: trusted(&state, &hash, &s), name: s.name, command: s.command, file: s.file, running }))
}

/// Runs it. `confirmed`: the user just agreed to this exact command (remembered until it changes).
#[tauri::command]
pub fn project_run(state: S<'_>, hash: String, confirmed: bool) -> Result<()> {
    let root = root_of(&state, &hash)?;
    let spec = run_spec(&root).ok_or_else(|| Error::Invalid("this project has no run command yet".into()))?;
    if confirmed {
        trust(&state, &hash, &spec)?;
    } else if !trusted(&state, &hash, &spec) {
        return Err(Error::Invalid("confirm the run command first".into()));
    }
    if RUNS.lock().unwrap().get(&hash).is_some_and(|r| r.lock().unwrap().exit.is_none()) {
        return Err(Error::Invalid("it's already running".into()));
    }
    let run = start(&root, &spec.command)?;
    RUNS.lock().unwrap().insert(hash, run);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOutput {
    /// Lines from position `from` on.
    lines: Vec<String>,
    /// The position to ask from next time.
    next: usize,
    running: bool,
    exit: Option<i32>,
}

/// Output since position `from` (0 for everything).
#[tauri::command]
pub fn project_run_output(hash: String, from: usize) -> Result<RunOutput> {
    let Some(run) = RUNS.lock().unwrap().get(&hash).cloned() else {
        return Ok(RunOutput { lines: vec![], next: 0, running: false, exit: None });
    };
    let r = run.lock().unwrap();
    let start = from.saturating_sub(r.dropped).min(r.lines.len());
    Ok(RunOutput { lines: r.lines[start..].to_vec(), next: r.dropped + r.lines.len(), running: r.exit.is_none(), exit: r.exit })
}

#[tauri::command]
pub fn project_stop(hash: String) -> Result<()> {
    if let Some(run) = RUNS.lock().unwrap().get(&hash) {
        let r = run.lock().unwrap();
        if r.exit.is_none() {
            stop_pid(r.pid);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("astralis-ide-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn paths_stay_inside_the_project() {
        let root = tmp("paths");
        std::fs::write(root.join("a.txt"), "x").unwrap();
        assert!(resolve(&root, "a.txt").is_ok());
        assert!(resolve(&root, "new/dir/file.rs").is_ok(), "a file that doesn't exist yet");
        for bad in ["", "../x", "a/../../x", "/etc/passwd"] {
            assert!(resolve(&root, bad).is_err(), "{bad}");
        }
        #[cfg(unix)]
        {
            let outside = tmp("outside");
            std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
            assert!(resolve(&root, "link/secret").is_err(), "a symlink out of the project");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn files_are_listed_without_ignored_ones_and_read_with_limits() {
        let root = tmp("files");
        let git = |args: &[&str]| assert!(Command::new("git").arg("-C").arg(&root).args(args).output().unwrap().status.success());
        git(&["init", "-q"]);
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("target/junk.o"), "x").unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("img.bin"), [0u8, 1, 2]).unwrap();
        std::fs::write(root.join("big.txt"), vec![b'a'; (MAX_READ + 1) as usize]).unwrap();
        let files = list_files(&root).unwrap();
        assert_eq!(files, vec![".gitignore", "big.txt", "img.bin", "src/main.rs"]);
        assert_eq!(read_file(&root, "src/main.rs").unwrap().text.as_deref(), Some("fn main() {}\n"));
        assert!(read_file(&root, "img.bin").unwrap().binary);
        assert!(read_file(&root, "big.txt").unwrap().too_big);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_run_command_comes_from_astralis_run_and_changes_when_its_script_does() {
        let root = tmp("spec");
        assert!(run_spec(&root).is_none());
        std::fs::create_dir_all(root.join(".astralis")).unwrap();
        std::fs::write(root.join(".astralis/run.sh"), "echo one\n").unwrap();
        let a = run_spec(&root).unwrap();
        assert!(a.command.ends_with(".astralis/run.sh") && a.file == ".astralis/run.sh");
        std::fs::write(root.join(".astralis/run.sh"), "echo two\n").unwrap();
        assert_ne!(run_spec(&root).unwrap().fingerprint, a.fingerprint, "an edited script has to be confirmed again");

        std::fs::write(root.join("dev.sh"), "echo a\n").unwrap();
        std::fs::write(root.join(".astralis/run.json"), r#"{"name":"Dev server","command":"sh ./dev.sh --port 3000"}"#).unwrap();
        let j = run_spec(&root).unwrap();
        assert_eq!((j.name.as_str(), j.file.as_str()), ("Dev server", ".astralis/run.json"), "run.json wins");
        std::fs::write(root.join("dev.sh"), "echo b\n").unwrap();
        assert_ne!(run_spec(&root).unwrap().fingerprint, j.fingerprint, "a script the command names is part of the fingerprint");
        std::fs::write(root.join(".astralis/run.json"), r#"{"command":"  "}"#).unwrap();
        assert!(run_spec(&root).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_run_collects_its_output_and_can_be_stopped() {
        let root = tmp("run");
        let run = start(&root, "echo hello; echo oops 1>&2; printf partial").unwrap();
        for _ in 0..100 {
            if run.lock().unwrap().exit.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let r = run.lock().unwrap();
        assert_eq!(r.exit, Some(0));
        let mut lines = r.lines.clone();
        lines.sort();
        assert_eq!(lines, vec!["hello", "oops", "partial"]);
        drop(r);

        let long = start(&root, "echo started; sleep 30").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let pid = long.lock().unwrap().pid;
        assert!(long.lock().unwrap().exit.is_none(), "still running");
        stop_pid(pid);
        for _ in 0..100 {
            if long.lock().unwrap().exit.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(long.lock().unwrap().exit.is_some(), "stopped");
        let _ = std::fs::remove_dir_all(&root);
    }
}
