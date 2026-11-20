//! Moving a project between computers.
//!
//! A snapshot is a git bundle of the whole repository (every branch and tag, so it restores with no
//! network and no git credentials) plus one extra commit holding the uncommitted work, taken
//! without touching the user's index or files. The app seals it and keeps it in the user's cloud
//! drive (see `astralis-cloud`'s `projects`); another computer restores it into a folder, or
//! applies a newer one onto a project it already has. Files git ignores (`.env`, `node_modules`,
//! build output) are never included.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::store::Store;

const REGISTRY: &str = "projects.registry";
const REMOTE: &str = "projects.remote";
const AUTO: &str = "projects.auto";
/// Extra things a project can leave out of its snapshot: a `.astralisignore` file in the project
/// folder, written like a `.gitignore`. It only trims the uncommitted work: whatever is already
/// committed is part of the repository's history and always travels with it.
pub const IGNORE_FILE: &str = ".astralisignore";

/// Always left out of the uncommitted work (generated files nobody needs to carry over).
const DEFAULT_IGNORE: &str = "node_modules/
target/
dist/
build/
out/
.next/
.nuxt/
.svelte-kit/
.turbo/
.parcel-cache/
.gradle/
.idea/
__pycache__/
*.pyc
.venv/
venv/
.pytest_cache/
.mypy_cache/
.cache/
coverage/
*.log
*.tmp
.DS_Store
Thumbs.db";

/// Where the snapshot of the work in progress is kept in the repository (a private ref).
const WIP_REF: &str = "refs/astralis/wip";

/// What a snapshot says about itself (sealed in the cloud along with the bundle).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub key: String,
    pub name: String,
    /// The `origin` URL with any password or token removed.
    pub remote: Option<String>,
    /// The checked-out branch ("HEAD" when detached).
    pub branch: String,
    pub head: String,
    /// The commit holding the uncommitted work (the same as `head` when there was none).
    pub wip: String,
    /// When it was taken, in milliseconds since 1970 (also what tells which copy is newer).
    pub ts: u64,
    pub device: String,
    pub device_name: String,
    pub bytes: u64,
    /// A fingerprint of everything in the snapshot, to skip one that wouldn't change anything.
    #[serde(default)]
    pub state: String,
}

/// A project this device knows about.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    pub key: String,
    pub name: String,
    /// Where the project is on this device ("" when it only exists in the cloud).
    pub root: String,
    /// Keep its snapshot in the cloud.
    pub sync: bool,
    /// The snapshot (by time) this device last uploaded or applied.
    pub last_ts: u64,
    /// The fingerprint of the last snapshot taken here.
    pub state: String,
}

/// The newest snapshot of a project in the cloud, as last listed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteInfo {
    pub manifest: Manifest,
    pub meta_id: String,
    pub bundle_id: String,
}

/// A snapshot made on this device, waiting to be uploaded.
pub struct Snapshot {
    pub manifest: Manifest,
    pub bundle: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullOutcome {
    /// The project now matches the snapshot.
    Updated,
    /// It already did.
    AlreadyThere,
    /// Nothing was changed, and here is why.
    Blocked(String),
}

/// Which project a folder belongs to.
pub struct ProjectId {
    /// The enclosing git repository (or the folder itself).
    pub root: PathBuf,
    /// The same on every device: the `origin` URL (without `.git`), or the path when there is none.
    pub key: String,
    pub name: String,
}

/// Finds the project a folder is in.
pub fn identify(start: &Path) -> ProjectId {
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut dir = start.as_path();
    let root = loop {
        if dir.join(".git").exists() {
            break dir.to_path_buf();
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break start.clone(),
        }
    };
    // A remote URL survives clones and moves; otherwise the path.
    let remote = std::fs::read_to_string(root.join(".git/config")).ok().and_then(|cfg| {
        let mut in_origin = false;
        for line in cfg.lines() {
            let l = line.trim();
            if l.starts_with('[') {
                in_origin = l == "[remote \"origin\"]";
            } else if in_origin && let Some(url) = l.strip_prefix("url =") {
                return Some(url.trim().trim_end_matches(".git").to_string());
            }
        }
        None
    });
    let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Project".into());
    ProjectId { key: remote.unwrap_or_else(|| root.to_string_lossy().to_string()), name, root }
}

/// A project's id in the cloud: the same on every device, and it doesn't reveal the project's name.
pub fn project_hash(key: &str) -> String {
    Sha256::digest(key.as_bytes())[..8].iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// This computer's name, for "from <device>" messages.
pub fn device_label() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "another device".into())
}

/// Removes `user:password@` from a URL (a token in a clone URL must never leave the machine).
fn without_credentials(url: &str) -> String {
    if let Some((scheme, rest)) = url.split_once("://") {
        let host_end = rest.find('/').unwrap_or(rest.len());
        if let Some(at) = rest[..host_end].rfind('@') {
            return format!("{scheme}://{}", &rest[at + 1..]);
        }
    }
    url.to_string()
}

// ---- git ----

fn command(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(root).stdin(Stdio::null()).env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

fn run(cmd: &mut Command) -> Result<String> {
    let out = cmd.output().map_err(|e| Error::invalid(format!("can't run git (is it installed?): {e}")))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = said.lines().rev().filter(|l| !l.trim().is_empty()).take(2).collect();
        return Err(Error::invalid(format!("git: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" "))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    run(command(root).args(args))
}

fn git_ok(root: &Path, args: &[&str]) -> bool {
    command(root).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Puts the uncommitted work (new files included, ignored files excluded) into a commit on top of
/// HEAD, using a throwaway index so the user's own index and files stay exactly as they are.
/// Returns that commit (or HEAD when there is no uncommitted work) and the tree of the work.
fn wip_commit(root: &Path, head: &str) -> Result<(String, String)> {
    let git_dir = PathBuf::from(git(root, &["rev-parse", "--absolute-git-dir"])?);
    let index = git_dir.join(format!("astralis-index-{}", std::process::id()));
    let exclude = git_dir.join(format!("astralis-exclude-{}", std::process::id()));
    let result = (|| {
        let with_index = |args: &[&str]| run(command(root).env("GIT_INDEX_FILE", &index).args(args));
        with_index(&["read-tree", "HEAD"])?;
        with_index(&["add", "-A"])?;
        // Leave out what the project's .astralisignore (and the built-in list) names: back to what
        // HEAD has for tracked files, and out of the snapshot for new ones.
        let mut patterns = format!("{DEFAULT_IGNORE}\n");
        patterns.push_str(&std::fs::read_to_string(root.join(IGNORE_FILE)).unwrap_or_default());
        std::fs::write(&exclude, patterns)?;
        let exclude_arg = exclude.to_string_lossy().to_string();
        let listed = with_index(&["ls-files", "-z", "-i", "-c", "--exclude-from", &exclude_arg])?;
        let skipped: Vec<&str> = listed.split('\0').filter(|p| !p.is_empty()).collect();
        for chunk in skipped.chunks(100) {
            let mut reset = vec!["reset", "-q", "HEAD", "--"];
            reset.extend(chunk.iter().copied());
            let _ = with_index(&reset);
        }
        let tree = with_index(&["write-tree"])?;
        if tree == git(root, &["rev-parse", "HEAD^{tree}"])? {
            return Ok((head.to_string(), tree));
        }
        let wip = run(command(root)
            .env("GIT_AUTHOR_NAME", "Astralis")
            .env("GIT_AUTHOR_EMAIL", "astralis@localhost")
            .env("GIT_COMMITTER_NAME", "Astralis")
            .env("GIT_COMMITTER_EMAIL", "astralis@localhost")
            .args(["commit-tree", &tree, "-p", head, "-m", "Astralis snapshot of work in progress"]))?;
        Ok((wip, tree))
    })();
    let _ = std::fs::remove_file(&index);
    let _ = std::fs::remove_file(&exclude);
    result
}

/// Takes a snapshot of the repository at `root` into `out` (`<hash>.bundle`). `None` when nothing
/// changed since `last_state`.
pub fn snapshot(root: &Path, key: &str, name: &str, device: &str, device_name: &str, out: &Path, last_state: &str) -> Result<Option<Snapshot>> {
    let head = git(root, &["rev-parse", "--verify", "HEAD"]).map_err(|_| Error::invalid("this project has no commits yet, so there is nothing to snapshot"))?;
    let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let (wip, tree) = wip_commit(root, &head)?;
    git(root, &["update-ref", WIP_REF, &wip])?;
    let refs = git(root, &["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads", "refs/tags"])?;
    let state: String = Sha256::digest(format!("{head}\n{tree}\n{refs}").as_bytes())[..16].iter().map(|b| format!("{b:02x}")).collect();
    if state == last_state {
        return Ok(None);
    }
    std::fs::create_dir_all(out)?;
    let bundle = out.join(format!("{}.bundle", project_hash(key)));
    let _ = std::fs::remove_file(&bundle);
    let bundle_arg = bundle.to_string_lossy().to_string();
    git(root, &["bundle", "create", "--quiet", &bundle_arg, "--branches", "--tags", WIP_REF])?;
    let remote = git(root, &["remote", "get-url", "origin"]).ok().filter(|u| !u.is_empty()).map(|u| without_credentials(&u));
    let manifest = Manifest {
        key: key.to_string(),
        name: name.to_string(),
        remote,
        branch,
        head,
        wip,
        ts: now_ms(),
        device: device.to_string(),
        device_name: device_name.to_string(),
        bytes: std::fs::metadata(&bundle).map_or(0, |m| m.len()),
        state,
    };
    Ok(Some(Snapshot { manifest, bundle }))
}

/// Writes the uncommitted work of the snapshot onto a working tree that sits at `head`: changed
/// and new files are copied in (left unstaged, as they were), deleted files are removed.
fn apply_wip(root: &Path, head: &str, wip: &str) -> Result<()> {
    if head == wip {
        return Ok(());
    }
    let diff = git(root, &["diff", "--name-status", "--no-renames", "-z", head, wip])?;
    let mut parts = diff.split('\0').filter(|p| !p.is_empty());
    let mut changed: Vec<String> = Vec::new();
    while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
        if status.starts_with('D') {
            match std::fs::remove_file(root.join(path)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        } else {
            changed.push(path.to_string());
        }
    }
    for chunk in changed.chunks(100) {
        let mut checkout = vec!["checkout", wip, "--"];
        checkout.extend(chunk.iter().map(String::as_str));
        git(root, &checkout)?;
        let mut reset = vec!["reset", "-q", "--"];
        reset.extend(chunk.iter().map(String::as_str));
        let _ = git(root, &reset);
    }
    Ok(())
}

/// Sets up the project in `dest` (a missing or empty folder) from a snapshot.
pub fn restore(dest: &Path, bundle: &Path, m: &Manifest) -> Result<()> {
    if dest.exists() && dest.read_dir()?.next().is_some() {
        return Err(Error::invalid("that folder isn't empty; pick a new or empty one"));
    }
    std::fs::create_dir_all(dest)?;
    let detached = m.branch == "HEAD" || m.branch.is_empty();
    let branch = if detached { "main" } else { m.branch.as_str() };
    git(dest, &["init", "-q"])?;
    git(dest, &["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")])?;
    let bundle_arg = bundle.to_string_lossy().to_string();
    git(dest, &["fetch", "-q", "--update-head-ok", &bundle_arg, "+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*", &format!("+{WIP_REF}:{WIP_REF}")])?;
    if detached {
        git(dest, &["checkout", "-q", "-f", "--detach", &m.head])?;
    } else {
        git(dest, &["checkout", "-q", "-f", branch])?;
    }
    if let Some(url) = &m.remote {
        let _ = git(dest, &["remote", "add", "origin", url]);
    }
    apply_wip(dest, &m.head, &m.wip)?;
    let _ = git(dest, &["update-ref", "-d", WIP_REF]);
    Ok(())
}

/// Brings a project that already exists here up to a newer snapshot. Never overwrites: it stops
/// (changing nothing) if there is unsaved work here, if another branch is checked out, or if this
/// device has commits the snapshot lacks.
pub fn pull(root: &Path, bundle: &Path, m: &Manifest) -> Result<PullOutcome> {
    if !git(root, &["status", "--porcelain"])?.is_empty() {
        return Ok(PullOutcome::Blocked("This device has changes that aren't committed. Commit or stash them first, then pull again.".into()));
    }
    if m.branch == "HEAD" || m.branch.is_empty() {
        return Ok(PullOutcome::Blocked("The cloud copy was taken on a detached HEAD, so it can't be applied automatically.".into()));
    }
    let current = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if current != m.branch {
        return Ok(PullOutcome::Blocked(format!("The cloud copy is on branch {}, but this device is on {current}. Switch to {} first.", m.branch, m.branch)));
    }
    let bundle_arg = bundle.to_string_lossy().to_string();
    git(root, &["fetch", "-q", &bundle_arg, &format!("+refs/heads/*:refs/astralis/in/*"), &format!("+{WIP_REF}:refs/astralis/in-wip")])?;
    let local = git(root, &["rev-parse", "HEAD"])?;
    if local != m.head {
        if git_ok(root, &["merge-base", "--is-ancestor", &local, &m.head]) {
            git(root, &["merge", "--ff-only", "--quiet", &m.head])?;
        } else if git_ok(root, &["merge-base", "--is-ancestor", &m.head, &local]) {
            return Ok(PullOutcome::Blocked("This device already has newer commits than the cloud copy.".into()));
        } else {
            return Ok(PullOutcome::Blocked(format!("This device and the cloud copy both have new commits. Nothing was changed; the cloud copy is saved as refs/astralis/in/{} for you to merge.", m.branch)));
        }
    } else if m.head == m.wip {
        return Ok(PullOutcome::AlreadyThere);
    }
    apply_wip(root, &m.head, &m.wip)?;
    Ok(PullOutcome::Updated)
}

// ---- what this device knows (settings) ----

impl Store {
    fn read_map<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<BTreeMap<String, T>> {
        Ok(self.get_setting(key)?.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
    }

    /// The projects this device knows, by project id.
    pub fn projects(&self) -> Result<BTreeMap<String, Entry>> {
        self.read_map(REGISTRY)
    }

    fn save_projects(&self, all: &BTreeMap<String, Entry>) -> Result<()> {
        self.set_setting(REGISTRY, &serde_json::to_string(all)?)
    }

    /// Whether new projects are kept in the cloud from the start (on unless the user turned it off).
    pub fn projects_auto(&self) -> bool {
        self.get_setting(AUTO).ok().flatten().is_none_or(|v| v != "off")
    }

    pub fn set_projects_auto(&self, on: bool) -> Result<()> {
        self.set_setting(AUTO, if on { "on" } else { "off" })
    }

    /// Notes that a project exists at `root` (called when an agent works in it). A new one is
    /// added to the list and starts synced, unless the "all projects" switch was turned off.
    pub fn project_seen(&self, key: &str, name: &str, root: &Path) -> Result<String> {
        let hash = project_hash(key);
        let mut all = self.projects()?;
        let auto = self.projects_auto();
        let root = root.to_string_lossy().to_string();
        match all.get_mut(&hash) {
            Some(e) if e.root == root && e.name == name => return Ok(hash),
            Some(e) => {
                e.root = root;
                e.name = name.to_string();
            }
            None => {
                all.insert(hash.clone(), Entry { key: key.to_string(), name: name.to_string(), root, sync: auto, ..Default::default() });
            }
        }
        self.save_projects(&all)?;
        Ok(hash)
    }

    /// Adds a project that was restored or chosen by hand.
    pub fn project_put(&self, entry: Entry) -> Result<String> {
        let hash = project_hash(&entry.key);
        let mut all = self.projects()?;
        all.insert(hash.clone(), entry);
        self.save_projects(&all)?;
        Ok(hash)
    }

    pub fn project_set_sync(&self, hash: &str, on: bool) -> Result<()> {
        let mut all = self.projects()?;
        let e = all.get_mut(hash).ok_or_else(|| Error::invalid("unknown project"))?;
        e.sync = on;
        if on {
            e.state.clear(); // take a fresh snapshot next time
        }
        self.save_projects(&all)
    }

    pub fn project_forget(&self, hash: &str) -> Result<()> {
        let mut all = self.projects()?;
        all.remove(hash);
        self.save_projects(&all)
    }

    /// Records what this device last uploaded or applied.
    pub fn project_mark(&self, hash: &str, last_ts: Option<u64>, state: Option<&str>) -> Result<()> {
        let mut all = self.projects()?;
        if let Some(e) = all.get_mut(hash) {
            if let Some(t) = last_ts {
                e.last_ts = t;
            }
            if let Some(s) = state {
                e.state = s.to_string();
            }
            self.save_projects(&all)?;
        }
        Ok(())
    }

    /// The newest cloud snapshot of each project, as of the last refresh.
    pub fn projects_remote(&self) -> Result<BTreeMap<String, RemoteInfo>> {
        self.read_map(REMOTE)
    }

    pub fn set_projects_remote(&self, all: &BTreeMap<String, RemoteInfo>) -> Result<()> {
        self.set_setting(REMOTE, &serde_json::to_string(all)?)
    }

    /// The cloud has a newer snapshot of this project than this device has used, from another device.
    pub fn project_cloud_newer(&self, hash: &str) -> Result<Option<Manifest>> {
        let entry = self.projects()?.remove(hash);
        let remote = self.projects_remote()?.remove(hash);
        Ok(match (entry, remote) {
            (Some(e), Some(r)) if r.manifest.ts > e.last_ts && r.manifest.device != self.device_id() => Some(r.manifest),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("astralis-proj-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn g(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn write(dir: &Path, file: &str, text: &str) {
        let p = dir.join(file);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn read(dir: &Path, file: &str) -> Option<String> {
        std::fs::read_to_string(dir.join(file)).ok()
    }

    #[test]
    fn a_project_with_unsaved_work_moves_to_another_folder_and_stays_in_step() {
        let base = tmp("move");
        let a = base.join("a");
        let out = base.join("out");
        std::fs::create_dir_all(&a).unwrap();
        g(&a, &["init", "-q", "-b", "main"]);
        write(&a, ".gitignore", "secret.env\n");
        write(&a, "src/lib.rs", "one\n");
        write(&a, "gone.txt", "to delete\n");
        g(&a, &["add", "-A"]);
        g(&a, &["commit", "-q", "-m", "first"]);
        g(&a, &["remote", "add", "origin", "https://user:token@example.com/me/app.git"]);
        // Uncommitted: an edit, a new file, a deleted file, and an ignored secret.
        write(&a, "src/lib.rs", "one\ntwo\n");
        write(&a, "new/file.txt", "fresh\n");
        std::fs::remove_file(a.join("gone.txt")).unwrap();
        write(&a, "secret.env", "TOKEN=1\n");
        // Generated or unwanted files that git doesn't ignore: the built-in list and .astralisignore.
        write(&a, "node_modules/pkg/index.js", "x");
        write(&a, "renders/huge.bin", "big");
        write(&a, ".astralisignore", "renders/\n");
        let status_before = git(&a, &["status", "--porcelain"]).unwrap();

        let snap = snapshot(&a, "https://example.com/me/app", "app", "dev1", "Laptop", &out, "").unwrap().expect("a snapshot");
        assert_eq!(snap.manifest.remote.as_deref(), Some("https://example.com/me/app.git"), "no token leaves the machine");
        assert_eq!(snap.manifest.branch, "main");
        // The next snapshot reuses the file name, so keep this one.
        let first_bundle = base.join("first.bundle");
        std::fs::copy(&snap.bundle, &first_bundle).unwrap();
        assert_ne!(snap.manifest.head, snap.manifest.wip);
        assert_eq!(git(&a, &["status", "--porcelain"]).unwrap(), status_before, "the user's files and index are untouched");
        // The commit that holds the work is stamped with the time; the fingerprint must not depend on it.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(snapshot(&a, "https://example.com/me/app", "app", "dev1", "Laptop", &out, &snap.manifest.state).unwrap().is_none(), "nothing changed");

        // Restore on a "new computer".
        let b = base.join("b");
        restore(&b, &snap.bundle, &snap.manifest).unwrap();
        assert_eq!(read(&b, "src/lib.rs").as_deref(), Some("one\ntwo\n"));
        assert_eq!(read(&b, "new/file.txt").as_deref(), Some("fresh\n"));
        assert!(read(&b, "gone.txt").is_none());
        assert!(read(&b, "secret.env").is_none(), "ignored files are never copied");
        assert!(read(&b, "node_modules/pkg/index.js").is_none() && read(&b, "renders/huge.bin").is_none(), "the built-in list and .astralisignore are left out");
        assert!(read(&b, ".astralisignore").is_some(), "the ignore file itself travels");
        assert_eq!(git(&b, &["remote", "get-url", "origin"]).unwrap(), "https://example.com/me/app.git");
        assert_eq!(git(&b, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap(), "main");
        assert!(restore(&b, &snap.bundle, &snap.manifest).is_err(), "won't write into a folder that has files");

        // More work on A, committed this time; B (with its restored work saved) catches up.
        g(&b, &["add", "-A"]);
        g(&b, &["commit", "-q", "-m", "work from A"]);
        let on_b_head = git(&b, &["rev-parse", "HEAD"]).unwrap();
        g(&a, &["add", "-A"]);
        g(&a, &["commit", "-q", "-m", "work from A"]);
        write(&a, "src/lib.rs", "one\ntwo\nthree\n");
        let snap2 = snapshot(&a, "https://example.com/me/app", "app", "dev1", "Laptop", &out, &snap.manifest.state).unwrap().expect("changed");
        // B committed the same work under another commit id, so the histories diverged: nothing changes.
        match pull(&b, &snap2.bundle, &snap2.manifest).unwrap() {
            PullOutcome::Blocked(why) => assert!(why.contains("both have new commits"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(git(&b, &["rev-parse", "HEAD"]).unwrap(), on_b_head);

        // A clean copy that is simply behind is brought forward, with the unsaved edit applied.
        let c = base.join("c");
        restore(&c, &first_bundle, &snap.manifest).unwrap();
        g(&c, &["checkout", "-q", "--", "."]);
        g(&c, &["clean", "-fdq"]);
        assert_eq!(pull(&c, &snap2.bundle, &snap2.manifest).unwrap(), PullOutcome::Updated);
        assert_eq!(read(&c, "src/lib.rs").as_deref(), Some("one\ntwo\nthree\n"));
        write(&c, "dirty.txt", "x");
        assert!(matches!(pull(&c, &snap2.bundle, &snap2.manifest).unwrap(), PullOutcome::Blocked(_)), "unsaved work here blocks a pull");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn what_a_device_knows_about_projects_is_remembered() {
        let store = Store::open_in_memory().unwrap();
        let hash = project_hash("k");
        assert_eq!(hash, project_hash("k"));
        assert_eq!(hash.len(), 16);
        assert_eq!(store.project_seen("k", "app", Path::new("/p/app")).unwrap(), hash);
        assert!(store.projects().unwrap()[&hash].sync, "a project an agent works in is added and kept");
        store.set_projects_auto(false).unwrap();
        store.project_seen("k2", "other", Path::new("/p/other")).unwrap();
        assert!(!store.projects().unwrap()[&project_hash("k2")].sync, "unless the user turned that off");
        store.project_mark(&hash, Some(5), Some("s")).unwrap();
        let e = &store.projects().unwrap()[&hash];
        assert!(e.sync && e.last_ts == 5 && e.state == "s");
        let m = Manifest { key: "k".into(), ts: 9, device: "someone-else".into(), ..Default::default() };
        store.set_projects_remote(&BTreeMap::from([(hash.clone(), RemoteInfo { manifest: m, meta_id: "m".into(), bundle_id: "b".into() })])).unwrap();
        assert!(store.project_cloud_newer(&hash).unwrap().is_some());
        store.project_mark(&hash, Some(9), None).unwrap();
        assert!(store.project_cloud_newer(&hash).unwrap().is_none());
        store.project_forget(&hash).unwrap();
        assert!(!store.projects().unwrap().contains_key(&hash));
    }
}
