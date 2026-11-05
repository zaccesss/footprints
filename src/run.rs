//! capture tier 2: `footprints run`, a wrapper that works for any agent.
//!
//! before the child starts, every file under the watched directory is cloned into a staging
//! area. On APFS a clone shares blocks with the original, so this costs almost nothing until a
//! file changes. While the child runs a notify watcher (FSEvents on macOS, inotify on Linux)
//! notes every touched path. Afterwards a rescan finds every created, changed and deleted file
//! and records it with its pre-image and post-image.
//!
//! the staging path and the list of files are saved on the pending command before the child
//! starts. If the wrapper is killed, `footprints recover` (also run at the start of every
//! `footprints run`) records the changes from that staging area later.

use crate::snapshot::{self, Before, Manifest};
use crate::store::{Action, Kind, Status, Store, now_ms};
use anyhow::{Context, Result, bail};
use notify::{RecursiveMode, Watcher};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

#[derive(Debug, Default, Clone)]
pub struct RunOptions {
    pub agent: Option<String>,
    pub session: Option<String>,
    pub dir: Option<PathBuf>,
    pub intent: Option<String>,
    pub command: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RunOutcome {
    pub exit_code: i32,
    pub session: String,
    pub command_id: i64,
    pub changes: Vec<i64>,
    pub events: usize,
}

/// FSEvents batches notifications with a short latency, so the watcher is given a moment to
/// deliver the last events after the child exits.
const SETTLE: Duration = Duration::from_millis(300);

/// a staging area that no pending command refers to is only removed once it is this old, so a
/// run that is still starting up is never disturbed.
const STALE_STAGING: Duration = Duration::from_secs(3600);

pub fn run(store: &Store, opts: &RunOptions) -> Result<RunOutcome> {
    if opts.command.is_empty() {
        bail!("no command given. Usage: footprints run [--agent NAME] -- <command...>");
    }
    for r in recover(store)? {
        eprintln!("footprints: {r}");
    }
    let root = match &opts.dir {
        Some(d) => d.clone(),
        None => std::env::current_dir()?,
    };
    let root = root
        .canonicalize()
        .with_context(|| format!("watched directory {} does not exist", root.display()))?;
    let excl = snapshot::exclusions(store);
    if excl.iter().any(|x| root.starts_with(x)) {
        bail!("the watched directory is inside the footprints data directory");
    }
    let _lock = RootLock::take(store, &root)?;
    let before = snapshot::walk(&root, &excl, snapshot::max_files())?;
    if before.truncated {
        bail!(
            "{} holds more than {} files, so a reliable pre-image cannot be taken. Pick a \
             narrower --dir or raise FOOTPRINTS_MAX_FILES",
            root.display(),
            snapshot::max_files()
        );
    }

    let agent = opts.agent.clone().unwrap_or_else(|| {
        Path::new(&opts.command[0])
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".into())
    });
    let session = opts.session.clone().unwrap_or_else(|| {
        let seed = format!("{}-{}", now_ms(), std::process::id());
        format!("run-{}", &blake3::hash(seed.as_bytes()).to_hex()[..8])
    });

    let staging_root = store.home().join("staging");
    crate::store::private_dir(&staging_root)?;
    let staging = staging_root.join(format!("{}-{}", std::process::id(), now_ms()));
    crate::store::private_dir(&staging)?;
    stage(&root, &staging, &before.files)?;

    let manifest = Manifest::from_walk(&root, &before);
    let mut cmd_action = Action::new(&agent, &session, Kind::Command, &shell_join(&opts.command));
    cmd_action.status = Status::Pending;
    cmd_action.tool = Some("run".into());
    cmd_action.cwd = Some(root.to_string_lossy().into_owned());
    cmd_action.intent = opts.intent.clone();
    cmd_action.manifest_hash = Some(store.blobs.put(&serde_json::to_vec(&manifest)?)?);
    cmd_action.staging = Some(staging.to_string_lossy().into_owned());
    cmd_action.id = store.insert(&cmd_action)?;

    let hints: Arc<Mutex<HashSet<PathBuf>>> = Arc::default();
    let events = Arc::new(Mutex::new(0usize));
    let watcher = start_watcher(&root, hints.clone(), events.clone());

    let status = {
        let _guard = SignalGuard::new();
        Command::new(&opts.command[0])
            .args(&opts.command[1..])
            .current_dir(&root)
            .env("FOOTPRINTS_SESSION", &session)
            .status()
            .with_context(|| format!("starting {}", opts.command[0]))
    };

    let status = match status {
        Ok(s) => s,
        Err(e) => {
            drop(watcher);
            let _ = fs::remove_dir_all(&staging);
            // a command that never started changed nothing, so its pending entry would only
            // mislead
            store.delete(cmd_action.id)?;
            return Err(e);
        }
    };
    std::thread::sleep(SETTLE);
    drop(watcher);
    let hinted = hints.lock().unwrap().clone();
    let n_events = *events.lock().unwrap();
    let code = exit_code(&status);
    let changes = finish(
        store,
        &mut cmd_action,
        &manifest,
        &staging,
        &hinted,
        &format!("exit status {code}; "),
        &format!(" from {n_events} filesystem event(s)"),
    )?;
    Ok(RunOutcome {
        exit_code: code,
        session,
        command_id: cmd_action.id,
        changes,
        events: n_events,
    })
}

/// compares the watched directory with its staged copy, records every change under the
/// command, marks the command recorded and removes the staging area.
fn finish(
    store: &Store,
    cmd: &mut Action,
    manifest: &Manifest,
    staging: &Path,
    hints: &HashSet<PathBuf>,
    note_prefix: &str,
    note_suffix: &str,
) -> Result<Vec<i64>> {
    let excl = snapshot::exclusions(store);
    let root = &manifest.root;
    let after = snapshot::walk(root, &excl, usize::MAX)?;
    let before_meta = manifest.metas();
    let mut paths: BTreeSet<PathBuf> = snapshot::candidates(&before_meta, &manifest.links, &after)
        .into_iter()
        .collect();
    // a file rewritten with the same size inside one mtime tick only shows up as an event
    for h in hints {
        if before_meta.contains_key(h) || after.files.contains_key(h) {
            paths.insert(h.clone());
        }
    }
    let opaque_moved = snapshot::opaque_changed(&manifest.opaque, &after.opaque);
    let mut changes = Vec::new();
    for path in paths {
        let before = if let Some(meta) = before_meta.get(&path) {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let copy = staging.join(rel);
            let stored = if meta.size <= snapshot::max_file_bytes() && copy.is_file() {
                Some(store.blobs.put_file(&copy)?)
            } else {
                None
            };
            Before::File {
                meta: *meta,
                stored,
            }
        } else if let Some(t) = manifest.links.get(&path) {
            Before::Link(t.clone())
        } else {
            Before::Absent
        };
        if let Some(mut a) = snapshot::change_action(store, cmd, &path, before, opaque_moved)? {
            a.tool = Some("run".into());
            changes.push(store.insert(&a)?);
        }
    }
    cmd.status = Status::Recorded;
    cmd.end_time_ms = Some(now_ms());
    cmd.staging = None;
    cmd.note = Some(format!(
        "{note_prefix}{} file change(s) recorded under {}{note_suffix}; changes outside it were \
         not captured",
        changes.len(),
        root.display()
    ));
    store.save(cmd)?;
    let _ = fs::remove_dir_all(staging);
    Ok(changes)
}

/// records unfinished runs whose wrapper is gone, then removes staging areas nothing refers
/// to. Returns a line for each run it recovered.
pub fn recover(store: &Store) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut referenced: HashSet<PathBuf> = HashSet::new();
    for mut cmd in store.unfinished_runs()? {
        let Some(staging) = cmd.staging.clone().map(PathBuf::from) else {
            continue;
        };
        if pid_of(&staging).is_some_and(process_alive) {
            referenced.insert(staging);
            continue;
        }
        let manifest: Option<Manifest> = cmd
            .manifest_hash
            .as_deref()
            .and_then(|h| store.blobs.get(h).ok())
            .and_then(|b| serde_json::from_slice(&b).ok());
        let Some(manifest) = manifest.filter(|_| staging.is_dir()) else {
            cmd.status = Status::Recorded;
            cmd.staging = None;
            cmd.note = Some(
                "the wrapper stopped before recording and its staging area is gone, so the file \
                 changes of this command are unknown"
                    .into(),
            );
            store.save(&cmd)?;
            out.push(format!(
                "#{} could not be recovered: staging area missing",
                cmd.id
            ));
            continue;
        };
        let changes = finish(
            store,
            &mut cmd,
            &manifest,
            &staging,
            &HashSet::new(),
            "recovered after the wrapper stopped early, exit status unknown; ",
            "",
        )?;
        out.push(format!(
            "recovered #{} from an interrupted run with {} file change(s)",
            cmd.id,
            changes.len()
        ));
    }
    clean_stale_staging(&store.home().join("staging"), &referenced);
    Ok(out)
}

fn pid_of(staging: &Path) -> Option<i32> {
    staging
        .file_name()?
        .to_str()?
        .split('-')
        .next()?
        .parse()
        .ok()
}

/// removes old staging areas left by a run whose ledger entry no longer refers to them.
fn clean_stale_staging(dir: &Path, referenced: &HashSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if referenced.contains(&path) || pid_of(&path).is_some_and(process_alive) {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_STAGING);
        if old {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(unix)]
fn process_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists and sends nothing
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn process_alive(_pid: i32) -> bool {
    true
}

/// one lock file per running wrapper, holding the root it watches. Two runs over the same or
/// nested directories would each record the other's changes, so the second one refuses.
struct RootLock {
    path: PathBuf,
}

impl RootLock {
    fn take(store: &Store, root: &Path) -> Result<RootLock> {
        let dir = store.home().join("locks");
        crate::store::private_dir(&dir)?;
        let _guard = DirLock::take(&dir.join(".guard"))?;
        for e in fs::read_dir(&dir)?.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("lock") {
                continue;
            }
            let Some(pid) = p
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<i32>().ok())
            else {
                continue;
            };
            if !process_alive(pid) {
                let _ = fs::remove_file(&p);
                continue;
            }
            let other = PathBuf::from(fs::read_to_string(&p).unwrap_or_default().trim());
            if other.as_os_str().is_empty() {
                continue;
            }
            if other.starts_with(root) || root.starts_with(&other) {
                bail!(
                    "another footprints run (process {pid}) is already watching {}, which \
                     overlaps {}. Wait for it to finish or pick a separate --dir",
                    other.display(),
                    root.display()
                );
            }
        }
        let path = dir.join(format!("{}.lock", std::process::id()));
        fs::write(&path, root.to_string_lossy().as_bytes())?;
        Ok(RootLock { path })
    }
}

impl Drop for RootLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// an exclusive advisory lock held while the lock directory is read and written, so two runs
/// starting together cannot both see the other as absent.
struct DirLock {
    _file: fs::File,
}

impl DirLock {
    fn take(path: &Path) -> Result<DirLock> {
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // SAFETY: the descriptor is valid for the lifetime of `file`
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if rc != 0 {
                bail!("could not lock {}", path.display());
            }
        }
        Ok(DirLock { _file: file })
    }
}

/// copies every file into the staging area. Files that are too large or unreadable are left
/// out and later recorded as not reversible.
fn stage(root: &Path, staging: &Path, files: &BTreeMap<PathBuf, snapshot::Meta>) -> Result<()> {
    let max = snapshot::max_file_bytes();
    for (path, meta) in files {
        if meta.size > max {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(path);
        let dest = staging.join(rel);
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        let _ = clone_or_copy(path, &dest);
    }
    Ok(())
}

/// from `sys/clonefile.h`; the libc crate does not export it.
#[cfg(target_os = "macos")]
const CLONE_NOFOLLOW: u32 = 0x0001;

#[cfg(target_os = "macos")]
fn clone_or_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let s = CString::new(src.as_os_str().as_bytes())?;
    let d = CString::new(dst.as_os_str().as_bytes())?;
    // SAFETY: both pointers are valid NUL-terminated strings that outlive the call
    let rc = unsafe { libc::clonefile(s.as_ptr(), d.as_ptr(), CLONE_NOFOLLOW) };
    if rc == 0 {
        return Ok(());
    }
    // clonefile fails across volumes and on non-APFS disks, where a real copy still works
    fs::copy(src, dst).map(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn clone_or_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::copy(src, dst).map(|_| ())
}

fn start_watcher(
    root: &Path,
    hints: Arc<Mutex<HashSet<PathBuf>>>,
    events: Arc<Mutex<usize>>,
) -> Option<notify::RecommendedWatcher> {
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut w = match notify::recommended_watcher(tx) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("footprints: file watcher unavailable ({e}); relying on the rescan");
            return None;
        }
    };
    if let Err(e) = w.watch(root, RecursiveMode::Recursive) {
        eprintln!(
            "footprints: could not watch {} ({e}); relying on the rescan",
            root.display()
        );
        return None;
    }
    std::thread::spawn(move || {
        // the loop ends when the watcher is dropped and the channel closes
        for ev in rx.into_iter().flatten() {
            *events.lock().unwrap() += 1;
            let mut h = hints.lock().unwrap();
            for p in ev.paths {
                h.insert(p);
            }
        }
    });
    Some(w)
}

fn exit_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(c) = status.code() {
        return c;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return 128 + sig;
        }
    }
    1
}

/// quotes arguments only where a shell would split them, so the log shows a command that can
/// be pasted back into a terminal.
pub fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            let plain = !a.is_empty()
                && a.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c));
            if plain {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(unix)]
const GUARDED: [libc::c_int; 4] = [libc::SIGINT, libc::SIGQUIT, libc::SIGHUP, libc::SIGTERM];

/// while the child runs, Ctrl+C, a closed terminal or a plain `kill` would stop footprints
/// before it records what the child did. A no-op handler keeps it alive until the child exits.
/// A handler, unlike SIG_IGN, is reset on exec, so the child still gets the default behaviour.
/// SIGKILL cannot be caught; `footprints recover` covers that case.
struct SignalGuard {
    #[cfg(unix)]
    old: [libc::sighandler_t; 4],
}

#[cfg(unix)]
extern "C" fn ignore_signal(_: libc::c_int) {}

impl SignalGuard {
    fn new() -> Self {
        #[cfg(unix)]
        {
            let h = ignore_signal as *const () as libc::sighandler_t;
            // SAFETY: the handler is async-signal-safe because it does nothing
            let old = GUARDED.map(|sig| unsafe { libc::signal(sig, h) });
            SignalGuard { old }
        }
        #[cfg(not(unix))]
        SignalGuard {}
    }
}

impl Drop for SignalGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        for (sig, old) in GUARDED.iter().zip(self.old) {
            // SAFETY: restores the disposition saved in new()
            unsafe {
                libc::signal(*sig, old);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_join_quotes_only_when_needed() {
        let args: Vec<String> = ["sh", "-c", "echo 'hi' > a.txt"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(shell_join(&args), r#"sh -c 'echo '\''hi'\'' > a.txt'"#);
    }
}
