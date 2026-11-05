//! capture tier 3: `footprints daemon`, an opt-in recorder for changes made by any process.
//!
//! the daemon watches a list of roots and records every file change under them as agent
//! `unknown`, since the operating system's change feed says what changed but not which agent
//! asked for it. The sources are:
//!
//! - macOS: FSEvents, through notify.
//! - Linux: inotify, through notify. With `--fanotify` and CAP_SYS_ADMIN, fanotify also names
//!   the process that closed each written file.
//! - Windows: ReadDirectoryChangesW, through notify. With `--usn` and administrator rights,
//!   the volume's USN change journal is read as well, which keeps every change even when the
//!   notification buffer overflows.
//!
//! a change is only reversible when its pre-image is known: the file was copied into the blob
//! store by `--snapshot` when the daemon started or the daemon saw an earlier version of it
//! while running. Anything else is recorded with its post-image only and marked not reversible.
//!
//! a change that a hook, the wrapper or an undo already recorded for the same path around the
//! same time is not recorded twice. It is counted as joined to that session. A change under a
//! directory that a running `footprints run` or shell hook is watching is left to that tier.

use crate::snapshot::{self, Before, Image, Meta};
use crate::store::{Action, Kind, Status, Store, now_ms};
use anyhow::{Context, Result, bail};
use notify::{RecursiveMode, Watcher};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const DAEMON_AGENT: &str = "unknown";
pub const DAEMON_TOOL: &str = "daemon";

/// a path is processed once it has been quiet this long, so a file written in several chunks
/// is recorded once with its final contents.
const SETTLE: Duration = Duration::from_millis(700);

/// how far either side of a change another tier's record of the same path still counts as the
/// same change. Hooks and undo record within milliseconds; the margin covers a slow disk.
const JOIN_WINDOW_MS: i64 = 10_000;

/// how long a pending hook row stands in for its post hook. A tool that waits on a person's
/// approval can take a while, but a denied call must not hide changes for long.
const PENDING_FILE_MS: i64 = 120_000;

/// a pending hook command older than this is assumed to have been denied or abandoned, which
/// matches the default 600 second command hook timeout.
const PENDING_LIFETIME_MS: i64 = 600_000;

const UNKNOWN_ORIGIN_NOTE: &str = "the daemon did not see this file arrive as a new file (it \
     was moved or renamed in, arrived with a directory, turned up while the snapshot was being \
     taken or was found by a rescan), so it may be the only copy of existing data. It is not \
     deleted on undo";

const MOVED_IN_NOTE: &str = "this file appeared inside a directory that arrived in one step (a \
     move or an unpacked archive), so it may be moved existing data. It is not deleted on undo";

#[derive(Debug, Clone, Default)]
pub struct DaemonOptions {
    pub roots: Vec<PathBuf>,
    /// copy every file under the roots into the blob store first, so the first change to each
    /// one is reversible
    pub snapshot: bool,
    /// stop after this long; for scripts and tests
    pub seconds: Option<u64>,
    /// Linux only: also read fanotify to name the writing process
    pub fanotify: bool,
    /// Windows only: also read the USN change journal of each root's volume
    pub usn: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DaemonReport {
    pub session: String,
    pub parent_id: i64,
    pub recorded: usize,
    pub joined: usize,
    pub events: usize,
}

/// what a change source says happened. Only `Create` can show that a path did not exist
/// before; a rename or move may bring in the only copy of existing data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Create,
    Rename,
    Modify,
    Remove,
    Other,
    /// the source lost events (an inotify queue overflow, a Windows buffer overflow or a
    /// watcher error), so every root must be walked again
    Rescan,
}

impl Change {
    /// None for an event that cannot mean a change, such as a file being opened or read. The
    /// daemon's own reads produce those, so following them would only make it read again.
    fn of(ev: &notify::Event) -> Option<Change> {
        use notify::EventKind;
        use notify::event::{AccessKind, AccessMode, ModifyKind};
        if ev.need_rescan() {
            return Some(Change::Rescan);
        }
        Some(match ev.kind {
            EventKind::Create(_) => Change::Create,
            EventKind::Modify(ModifyKind::Name(_)) => Change::Rename,
            EventKind::Modify(_) => Change::Modify,
            EventKind::Remove(_) => Change::Remove,
            EventKind::Access(AccessKind::Close(AccessMode::Write)) => Change::Modify,
            EventKind::Access(_) => return None,
            _ => Change::Other,
        })
    }
}

/// one observation from a change source.
#[derive(Debug, Clone)]
pub struct Event {
    pub path: PathBuf,
    pub kind: Change,
    pub pid: Option<u32>,
    pub process: Option<String>,
}

impl Event {
    pub fn rescan() -> Event {
        Event {
            path: PathBuf::new(),
            kind: Change::Rescan,
            pid: None,
            process: None,
        }
    }
}

#[derive(Debug, Clone)]
struct Seen {
    last: Instant,
    time_ms: i64,
    process: Option<String>,
    /// a create event arrived for this path after the daemon was ready
    created: bool,
    /// a rename or move touched this path or it was created before the daemon was ready
    untrusted: bool,
}

impl Seen {
    fn synthetic() -> Seen {
        Seen {
            last: Instant::now(),
            time_ms: now_ms(),
            process: None,
            created: false,
            untrusted: true,
        }
    }

    /// true when the path is known to have been absent just before this change
    fn new_file(&self) -> bool {
        self.created && !self.untrusted
    }
}

/// what the daemon last knew to be at a path.
#[derive(Debug, Clone)]
struct Known {
    img: Image,
}

impl Known {
    fn before(&self) -> Option<Before> {
        if !self.img.exists {
            return Some(Before::Absent);
        }
        if let Some(t) = &self.img.link {
            return Some(Before::Link(t.clone()));
        }
        if self.img.unstored.as_deref() == Some("it is a directory") {
            return None;
        }
        Some(Before::File {
            meta: Meta {
                size: self.img.size.unwrap_or(0) as u64,
                mtime_ns: 0,
                mode: self.img.mode,
                ino: 0,
                ctime_ns: 0,
            },
            stored: if self.img.unstored.is_none() {
                self.img.hash.clone()
            } else {
                None
            },
        })
    }
}

struct State<'a> {
    store: &'a Store,
    parent: Action,
    roots: Vec<PathBuf>,
    /// roots taken with --snapshot, under which a path the daemon does not know is known to
    /// have been absent
    snapped: Vec<PathBuf>,
    excl: Vec<PathBuf>,
    known: BTreeMap<PathBuf, Known>,
    recorded: usize,
    joined: usize,
    /// set once the snapshot is complete; create events before then prove nothing
    ready: bool,
    /// when the daemon became ready, in nanoseconds since the Unix epoch
    ready_ns: i64,
    needs_rescan: bool,
}

pub fn run(store: &Store, opts: &DaemonOptions) -> Result<DaemonReport> {
    if opts.roots.is_empty() {
        bail!("give at least one --root to watch");
    }
    if opts.fanotify && !cfg!(target_os = "linux") {
        bail!("--fanotify is only available on Linux");
    }
    if opts.usn && !cfg!(windows) {
        bail!("--usn is only available on Windows");
    }
    let excl = snapshot::exclusions(store);
    let mut roots = Vec::new();
    for r in &opts.roots {
        let c = crate::platform::canonical(r)
            .with_context(|| format!("watched root {} does not exist", r.display()))?;
        if !c.is_dir() {
            bail!("{} is not a directory", c.display());
        }
        if excl.iter().any(|x| c.starts_with(x)) {
            bail!("{} is inside the footprints data directory", c.display());
        }
        roots.push(c);
    }
    let locks = store.home().join("locks");
    crate::store::private_dir(&locks)?;
    let Some(_lock) = crate::run::DirLock::try_take(&locks.join("daemon.lock"))? else {
        bail!(
            "another footprints daemon is already recording into {}",
            store.home().display()
        );
    };
    close_stale(store)?;

    let seed = format!("{}-{}", now_ms(), std::process::id());
    let session = format!("daemon-{}", &blake3::hash(seed.as_bytes()).to_hex()[..8]);
    let mut parent = Action::new(
        DAEMON_AGENT,
        &session,
        Kind::Command,
        &format!(
            "footprints daemon {}",
            roots
                .iter()
                .map(|r| r.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );
    parent.status = Status::Pending;
    parent.tool = Some(DAEMON_TOOL.into());
    parent.cwd = Some(roots[0].to_string_lossy().into_owned());
    parent.note = Some("the daemon is running".into());
    parent.id = store.insert(&parent)?;

    let mut st = State::new(store, parent, roots.clone(), excl.clone());

    crate::signals::on_stop_request();
    // the watcher starts before the snapshot, so a file that changes while the snapshot runs
    // still has an event; its create event, if any, is not trusted
    let (tx, rx) = mpsc::channel::<Event>();
    let _watcher = start_notify(&roots, tx.clone())?;
    // only Linux and Windows add a second source
    #[allow(unused_mut)]
    let mut sources = String::from("notify");
    #[cfg(target_os = "linux")]
    if opts.fanotify {
        fanotify::start(&roots, tx.clone())?;
        sources.push_str(", fanotify");
    }
    #[cfg(windows)]
    if opts.usn {
        usn::start(&roots, tx.clone())?;
        sources.push_str(", usn");
    }
    drop(tx);
    let mut seen: BTreeMap<PathBuf, Seen> = BTreeMap::new();
    let mut events = 0usize;
    if opts.snapshot {
        for r in &roots {
            st.snapshot_root(r)?;
        }
    }
    while let Ok(ev) = rx.try_recv() {
        events += 1;
        st.note_event(&mut seen, ev);
    }
    st.mark_ready();
    eprintln!(
        "footprints: daemon recording changes under {} root(s) as session {session} ({}). \
         Press Ctrl+C to stop",
        roots.len(),
        sources
    );

    let deadline = opts
        .seconds
        .map(|s| Instant::now() + Duration::from_secs(s));
    let result = (|| -> Result<()> {
        loop {
            let stopping =
                crate::signals::stop_requested() || deadline.is_some_and(|d| Instant::now() >= d);
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ev) => {
                    events += 1;
                    st.note_event(&mut seen, ev);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    bail!("every change source stopped");
                }
            }
            while let Ok(ev) = rx.try_recv() {
                events += 1;
                st.note_event(&mut seen, ev);
            }
            if st.needs_rescan {
                let all = std::mem::take(&mut seen);
                st.flush(all, &BTreeSet::new())?;
                st.reconcile()?;
            }
            if stopping {
                // give the sources a moment to deliver the last events, then record all
                std::thread::sleep(SETTLE);
                while let Ok(ev) = rx.try_recv() {
                    events += 1;
                    st.note_event(&mut seen, ev);
                }
                let all = std::mem::take(&mut seen);
                st.flush(all, &BTreeSet::new())?;
                if st.needs_rescan {
                    st.reconcile()?;
                }
                return Ok(());
            }
            let due: Vec<PathBuf> = seen
                .iter()
                .filter(|(_, s)| s.last.elapsed() >= SETTLE)
                .map(|(p, _)| p.clone())
                .collect();
            if !due.is_empty() {
                let batch = due
                    .into_iter()
                    .filter_map(|p| seen.remove(&p).map(|s| (p, s)))
                    .collect();
                let waiting: BTreeSet<PathBuf> = seen.keys().cloned().collect();
                st.flush(batch, &waiting)?;
            }
        }
    })();
    st.parent.status = Status::Recorded;
    st.parent.end_time_ms = Some(now_ms());
    st.parent.note = Some(format!(
        "{} change(s) recorded and {} already recorded by a hook, the wrapper or an undo, from {} \
         event(s) under {} root(s){}",
        st.recorded,
        st.joined,
        events,
        st.roots.len(),
        match &result {
            Ok(()) => String::new(),
            Err(e) => format!("; stopped early: {e:#}"),
        }
    ));
    store.save(&st.parent)?;
    crate::signals::mark_finished();
    result?;
    Ok(DaemonReport {
        session,
        parent_id: st.parent.id,
        recorded: st.recorded,
        joined: st.joined,
        events,
    })
}

/// a daemon that was killed outright leaves its session pending; it is closed with a note so
/// the timeline does not show a daemon that is no longer running.
fn close_stale(store: &Store) -> Result<()> {
    for mut a in store.pending_daemons()? {
        a.status = Status::Recorded;
        a.end_time_ms = Some(now_ms());
        a.note = Some(
            "the daemon stopped without shutting down cleanly; changes it had not yet \
             recorded were lost"
                .into(),
        );
        store.save(&a)?;
    }
    Ok(())
}

fn start_notify(roots: &[PathBuf], tx: mpsc::Sender<Event>) -> Result<notify::RecommendedWatcher> {
    let mut w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
        Ok(ev) => {
            let Some(kind) = Change::of(&ev) else {
                return;
            };

            if kind == Change::Rescan {
                let _ = tx.send(Event::rescan());
                return;
            }
            for p in ev.paths {
                let _ = tx.send(Event {
                    path: p,
                    kind,
                    pid: None,
                    process: None,
                });
            }
        }
        Err(e) => {
            // an error can mean lost events, so everything is walked again
            eprintln!("footprints: daemon watcher error ({e}); walking every root again");
            let _ = tx.send(Event::rescan());
        }
    })
    .context("starting the file watcher")?;
    for r in roots {
        w.watch(r, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", r.display()))?;
    }
    Ok(w)
}

impl<'a> State<'a> {
    fn new(store: &'a Store, parent: Action, roots: Vec<PathBuf>, excl: Vec<PathBuf>) -> Self {
        State {
            store,
            parent,
            roots,
            snapped: Vec::new(),
            excl,
            known: BTreeMap::new(),
            recorded: 0,
            joined: 0,
            ready: false,
            ready_ns: 0,
            needs_rescan: false,
        }
    }

    fn mark_ready(&mut self) {
        self.ready = true;
        self.ready_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    }

    /// a move keeps a file's modification time, while a file made now has a fresh one.
    /// Windows reports a file moved in from another folder as added and a move across file
    /// systems is a copy everywhere, so a create event alone cannot tell them apart.
    fn written_since_ready(&self, p: &Path) -> bool {
        // Linux stamps files from a coarse clock that can lag the real time by a few
        // milliseconds, so a small allowance keeps a file made just after start new. Coarser
        // file systems such as FAT can make a new file look older still, which only makes it
        // not reversible, never wrongly deletable
        const SLACK_NS: i64 = 50_000_000;
        std::fs::symlink_metadata(p)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|d| d.as_nanos() as i64 >= self.ready_ns - SLACK_NS)
    }

    fn snapshot_root(&mut self, root: &Path) -> Result<()> {
        let w = snapshot::walk(root, &self.excl, snapshot::max_files())?;
        if w.truncated {
            bail!(
                "{} holds more than {} files, so --snapshot cannot take a complete pre-image. \
                 Pick a narrower --root, raise FOOTPRINTS_MAX_FILES or run without --snapshot",
                root.display(),
                snapshot::max_files()
            );
        }
        for p in w.files.keys().chain(w.links.keys()) {
            let img = snapshot::capture(self.store, p)?;
            self.known.insert(p.clone(), Known { img });
        }
        self.snapped.push(root.to_path_buf());
        Ok(())
    }

    fn watched(&self, p: &Path) -> bool {
        self.roots
            .iter()
            .any(|r| crate::platform::key_starts_with(p, r))
            && !self
                .excl
                .iter()
                .any(|x| crate::platform::key_starts_with(p, x))
            && !p.components().any(|c| {
                snapshot::SKIP_DIRS
                    .iter()
                    .any(|s| c.as_os_str() == std::ffi::OsStr::new(s))
            })
    }

    fn note_event(&mut self, seen: &mut BTreeMap<PathBuf, Seen>, ev: Event) {
        if ev.kind == Change::Rescan {
            self.needs_rescan = true;
            return;
        }
        let path = crate::platform::normalise(&ev.path);
        if !self.watched(&path) {
            return;
        }
        let process = ev
            .process
            .or_else(|| ev.pid.map(|p| format!("process {p}")));
        let e = seen.entry(path).or_insert(Seen {
            last: Instant::now(),
            time_ms: now_ms(),
            process: None,
            created: false,
            untrusted: false,
        });
        e.last = Instant::now();
        match ev.kind {
            Change::Create if self.ready => e.created = true,
            Change::Create | Change::Rename => e.untrusted = true,
            _ => {}
        }
        if process.is_some() {
            e.process = process;
        }
    }

    /// walks every root again after the change feed lost events and records whatever differs
    /// from what the daemon knew, deletions included. Files it finds that it never knew have
    /// no trusted origin, so they are recorded as not reversible.
    fn reconcile(&mut self) -> Result<()> {
        self.needs_rescan = false;
        let mut work: BTreeMap<PathBuf, (Seen, bool)> = BTreeMap::new();
        for r in self.roots.clone() {
            let w = snapshot::walk(&r, &self.excl, snapshot::max_files())?;
            if w.truncated {
                eprintln!(
                    "footprints: {} holds more than {} files, so the rescan after lost events \
                     stopped there",
                    r.display(),
                    snapshot::max_files()
                );
            }
            for f in w.files.keys().chain(w.links.keys()) {
                work.insert(f.clone(), (Seen::synthetic(), false));
            }
            if !w.truncated {
                for k in self.known_under(&r) {
                    work.entry(k).or_insert_with(|| (Seen::synthetic(), false));
                }
            }
        }
        eprintln!("footprints: daemon walked every root again after lost events");
        self.process_all(work)
    }

    /// paths the daemon knows under `dir`, excluding `dir` itself.
    fn known_under(&self, dir: &Path) -> Vec<PathBuf> {
        self.known
            .range(dir.to_path_buf()..)
            .take_while(|(k, _)| k.starts_with(dir))
            .filter(|(k, _)| k.as_path() != dir)
            .map(|(k, _)| k.clone())
            .collect()
    }

    fn process_all(&mut self, work: BTreeMap<PathBuf, (Seen, bool)>) -> Result<()> {
        let context = self.store.recent(
            now_ms() - JOIN_WINDOW_MS * 2,
            now_ms() - PENDING_LIFETIME_MS,
        )?;
        for (p, (s, own_event)) in work {
            if self.watched(&p) {
                self.process(&p, &s, own_event, &context)?;
            }
        }
        Ok(())
    }

    /// records the paths in `batch`. Paths in `waiting` still have events settling and are
    /// recorded later with their own event, so a directory's walk leaves them alone.
    fn flush(&mut self, batch: BTreeMap<PathBuf, Seen>, waiting: &BTreeSet<PathBuf>) -> Result<()> {
        let mut work: BTreeMap<PathBuf, (Seen, bool)> = BTreeMap::new();
        for (p, s) in batch {
            let is_dir = std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir());
            // files the daemon knew under the path, which vanish when a directory is removed
            // or moved away and may also be gone from a directory that is still there
            for k in self.known_under(&p) {
                if !waiting.contains(&k) {
                    work.entry(k).or_insert_with(|| (s.clone(), false));
                }
            }
            // only a directory that appeared needs its contents walked; a directory whose
            // listing merely changed reports its new files with events of their own
            if is_dir && !(s.created || s.untrusted) {
                continue;
            }
            if is_dir {
                // a directory that appeared in one step (a move or an unpacked archive) may
                // hold files that never had an event of their own
                let w = snapshot::walk(&p, &self.excl, snapshot::max_files())?;
                for f in w.files.keys().chain(w.links.keys()) {
                    if !waiting.contains(f) {
                        work.entry(f.clone()).or_insert_with(|| (s.clone(), false));
                    }
                }
                continue;
            }
            work.insert(p, (s, true));
        }
        self.process_all(work)
    }

    fn process(&mut self, p: &Path, s: &Seen, own_event: bool, context: &[Action]) -> Result<()> {
        // a file that is locked, unreadable or gone mid-read is reported and skipped, so one
        // path never stops recording for every root
        let cur = match snapshot::capture(self.store, p) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("footprints: daemon skipped {}: {e:#}", p.display());
                return Ok(());
            }
        };
        if cur.unstored.as_deref() == Some("it is a directory") {
            return Ok(());
        }
        let prev = self.known.get(p).cloned();
        let under_snapshot = self.snapped.iter().any(|r| p.starts_with(r));
        let existed = prev.as_ref().is_some_and(|k| k.img.exists);
        // a path that was absent counts as new only when a create event proves it. A rename,
        // a move from outside the root or a file that turned up during the snapshot may be
        // the only copy of existing data, so it gets no "did not exist" pre-image
        // FSEvents keeps a path's create flag for a while, so a create event alone does not
        // prove absence either. It counts only where the daemon also knows the path was
        // empty: under a snapshot or after it saw the file go
        let known_empty = under_snapshot || prev.is_some();
        let before = match &prev {
            Some(k) if existed => k.before(),
            _ if own_event && s.new_file() && known_empty && self.written_since_ready(p) => {
                Some(Before::Absent)
            }
            _ => None,
        };
        let unchanged = match (&before, &prev) {
            (Some(Before::Absent), _) | (None, _) if !existed => !cur.exists,
            (_, Some(k)) => {
                k.img.exists == cur.exists
                    && k.img.hash == cur.hash
                    && k.img.link == cur.link
                    && k.img.unstored.is_none()
                    && cur.unstored.is_none()
            }
            _ => false,
        };
        self.known
            .insert(p.to_path_buf(), Known { img: cur.clone() });
        if unchanged {
            return Ok(());
        }
        if covered(p, &cur, s.time_ms, context, self.parent.id) {
            self.joined += 1;
            return Ok(());
        }
        let mut a = match before {
            Some(b) => {
                // moved-in files are treated like the wrapper's files from an opaque directory
                let Some(mut a) =
                    snapshot::change_action_from(&self.parent, p, b, !own_event, cur.clone())?
                else {
                    return Ok(());
                };
                if let Some(n) = &a.note
                    && n.contains(snapshot::OPAQUE_NOTE)
                {
                    a.note = Some(n.replace(snapshot::OPAQUE_NOTE, MOVED_IN_NOTE));
                }
                a
            }
            None => {
                let kind = match (cur.exists, under_snapshot || prev.is_some()) {
                    (true, true) => Kind::FileCreate,
                    (true, false) => Kind::FileWrite,
                    (false, _) => Kind::FileDelete,
                };
                let mut a = Action::new(
                    DAEMON_AGENT,
                    &self.parent.session,
                    kind,
                    &p.to_string_lossy(),
                );
                a.post_hash = cur.hash.clone();
                a.post_size = if cur.exists { cur.size } else { None };
                a.parent_id = Some(self.parent.id);
                a.tool = Some(DAEMON_TOOL.into());
                a.cwd = self.parent.cwd.clone();
                a.end_time_ms = Some(now_ms());
                a.note = Some(if under_snapshot || prev.is_some() {
                    UNKNOWN_ORIGIN_NOTE.to_string()
                } else {
                    "no pre-image: the daemon had not seen this file before it changed. Start \
                     the daemon with --snapshot to keep pre-images from the start"
                        .to_string()
                });
                a
            }
        };
        a.time_ms = s.time_ms;
        let mut extra = Vec::new();
        if let Some(proc_) = &s.process {
            extra.push(format!("written by {proc_}"));
        }
        extra.extend(running_commands(p, context, self.parent.id));
        let active = active_sessions(s.time_ms, context);
        if !active.is_empty() {
            extra.push(format!(
                "no hook or wrapper recorded this path; sessions active at the time: {}",
                active.join(", ")
            ));
        }
        if !extra.is_empty() {
            a.note = Some(match a.note.take() {
                Some(n) => format!("{n}; {}", extra.join("; ")),
                None => extra.join("; "),
            });
        }
        self.store.insert(&a)?;
        self.recorded += 1;
        Ok(())
    }
}

/// the path as another tier would have stored it may go through a symlink (macOS keeps temp
/// folders behind `/var` to `/private/var`), so the parent is resolved before comparing.
fn same_path(target: &str, p: &Path) -> bool {
    let t = Path::new(target);
    crate::platform::same_key(t, p)
        || crate::platform::same_key(&crate::platform::resolve_parent(t), p)
}

/// true when another tier has recorded this change or is about to. A finished record
/// only counts when its post-image is what is on disk now, so an earlier undo or edit of the
/// same file is never mistaken for this one.
fn covered(p: &Path, cur: &Image, t_ms: i64, context: &[Action], own_parent: i64) -> bool {
    let now = now_ms();
    let own =
        |a: &Action| a.tool.as_deref() == Some(DAEMON_TOOL) || a.parent_id == Some(own_parent);
    // only the newest record of the path counts; an older one was overtaken by whatever came
    // after it, such as an undo that put the file back
    if let Some(a) = context
        .iter()
        .rev()
        .find(|a| a.kind.is_file() && same_path(&a.target, p))
        && !own(a)
    {
        let end = a.end_time_ms.unwrap_or(a.time_ms);
        let same_result = match a.post_size {
            Some(_) => cur.exists && a.post_hash.is_some() && a.post_hash == cur.hash,
            None => !cur.exists,
        };
        // a pending row only covers while its post hook could still be on the way, so a call
        // that was denied never blinds the daemon to that path for long
        let pending_live = a.status == Status::Pending && a.time_ms >= t_ms - PENDING_FILE_MS;
        if pending_live || (same_result && end >= t_ms - JOIN_WINDOW_MS && a.time_ms <= now) {
            return true;
        }
    }
    for a in context {
        if own(a) || a.kind != Kind::Command || a.status != Status::Pending {
            continue;
        }
        // a running wrapper records everything under its directory itself. A wrapper that was
        // killed outright is no longer going to record anything
        let live = a.staging.as_ref().is_some_and(|staging| {
            crate::run::staging_pid(Path::new(staging)).is_some_and(crate::platform::process_alive)
        });
        if live && let Some(cwd) = &a.cwd {
            let root =
                crate::platform::canonical(Path::new(cwd)).unwrap_or_else(|_| PathBuf::from(cwd));
            if crate::platform::key_starts_with(p, &root) {
                return true;
            }
        }
    }
    false
}

/// a shell hook command that is still pending may record this change itself, but it may also
/// have been denied and never record anything. The change is recorded either way, with a note
/// naming the command, rather than skipped.
fn running_commands(p: &Path, context: &[Action], own_parent: i64) -> Vec<String> {
    let now = now_ms();
    context
        .iter()
        .filter(|a| {
            a.kind == Kind::Command
                && a.status == Status::Pending
                && a.staging.is_none()
                && a.manifest_hash.is_some()
                && a.id != own_parent
                && a.time_ms >= now - PENDING_FILE_MS
        })
        .filter(|a| {
            a.cwd.as_deref().is_some_and(|cwd| {
                let root = crate::platform::canonical(Path::new(cwd))
                    .unwrap_or_else(|_| PathBuf::from(cwd));
                crate::platform::key_starts_with(p, &root)
            })
        })
        .map(|a| {
            format!(
                "shell command #{} from {} was running in this directory; if it finishes, its \
                 own record may list this change too",
                a.id, a.agent
            )
        })
        .collect()
}

/// sessions of other agents with an action close to this moment. This is only a time
/// correlation. The note says so.
fn active_sessions(t_ms: i64, context: &[Action]) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for a in context {
        if a.agent == DAEMON_AGENT || a.agent == crate::undo::UNDO_AGENT {
            continue;
        }
        let end = a.end_time_ms.unwrap_or(a.time_ms);
        if a.time_ms <= t_ms + 2_000 && end >= t_ms - 30_000 {
            out.insert(format!("{} ({})", a.session, a.agent));
        }
    }
    out.into_iter().collect()
}

/// fanotify, which can report the process behind each write on a whole mount. It needs
/// CAP_SYS_ADMIN. Only close-after-write events are read, since they come once per written
/// file rather than once per write call.
#[cfg(target_os = "linux")]
mod fanotify {
    use super::Event;
    use anyhow::{Result, bail};
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;
    use std::sync::mpsc;

    pub fn start(roots: &[PathBuf], tx: mpsc::Sender<Event>) -> Result<()> {
        // SAFETY: plain syscalls with constant arguments; the result is checked
        let fd = unsafe {
            libc::fanotify_init(
                libc::FAN_CLASS_NOTIF | libc::FAN_CLOEXEC,
                (libc::O_RDONLY | libc::O_LARGEFILE | libc::O_CLOEXEC) as u32,
            )
        };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            bail!(
                "fanotify is not available ({e}). It needs CAP_SYS_ADMIN, for example sudo; \
                 the daemon still works without --fanotify"
            );
        }
        for r in roots {
            let c = CString::new(r.as_os_str().as_bytes())?;
            // SAFETY: fd is a fanotify descriptor and c a valid path for the call
            let rc = unsafe {
                libc::fanotify_mark(
                    fd,
                    libc::FAN_MARK_ADD | libc::FAN_MARK_MOUNT,
                    libc::FAN_CLOSE_WRITE,
                    libc::AT_FDCWD,
                    c.as_ptr(),
                )
            };
            if rc < 0 {
                let e = std::io::Error::last_os_error();
                // SAFETY: fd was opened above and is not used again
                unsafe { libc::close(fd) };
                bail!("fanotify could not watch the mount of {}: {e}", r.display());
            }
        }
        let me = std::process::id();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                // SAFETY: buf is valid for its length
                let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 {
                    break;
                }
                for (fd_ev, pid) in parse(&buf[..n as usize]) {
                    let path = std::fs::read_link(format!("/proc/self/fd/{fd_ev}"));
                    // SAFETY: each event descriptor is owned by this reader and closed once
                    unsafe { libc::close(fd_ev) };
                    if pid == me {
                        continue;
                    }
                    if let Ok(path) = path {
                        let process = crate::platform::process_name(pid)
                            .map(|n| format!("process {pid} ({n})"))
                            .or_else(|| Some(format!("process {pid}")));
                        if tx
                            .send(Event {
                                path,
                                kind: super::Change::Modify,
                                pid: Some(pid),
                                process,
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
            }
        });
        Ok(())
    }

    /// the descriptor and process id of each event in one read, skipping overflow markers.
    fn parse(buf: &[u8]) -> Vec<(i32, u32)> {
        let size = std::mem::size_of::<libc::fanotify_event_metadata>();
        let mut out = Vec::new();
        let mut off = 0;
        while off + size <= buf.len() {
            // SAFETY: the kernel writes whole metadata records; read_unaligned copes with any
            // alignment of the buffer
            let m: libc::fanotify_event_metadata =
                unsafe { std::ptr::read_unaligned(buf[off..].as_ptr().cast()) };
            if m.event_len as usize == 0 {
                break;
            }
            if m.fd >= 0 {
                if m.vers == libc::FANOTIFY_METADATA_VERSION {
                    out.push((m.fd, m.pid as u32));
                } else {
                    // an event in a layout this reader does not know still owns a descriptor
                    // SAFETY: the descriptor came from the kernel with this event and is closed once
                    unsafe { libc::close(m.fd) };
                }
            }
            off += m.event_len as usize;
        }
        out
    }
}

/// the NTFS USN change journal. Reading it needs a handle on the volume, which needs
/// administrator rights. Each record names a file and its parent directory by file id, so
/// the parent's path is looked up once and cached.
#[cfg(windows)]
mod usn {
    use super::Event;
    use anyhow::{Result, bail};
    use std::collections::HashMap;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::sync::mpsc;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_DESCRIPTOR, FILE_NAME_NORMALIZED,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdType,
        GetFinalPathNameByHandleW, OPEN_EXISTING, OpenFileById, VOLUME_NAME_DOS,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0,
        USN_JOURNAL_DATA_V0, USN_REASON_DATA_EXTEND, USN_REASON_DATA_OVERWRITE,
        USN_REASON_DATA_TRUNCATION, USN_REASON_FILE_CREATE, USN_REASON_FILE_DELETE,
        USN_REASON_RENAME_NEW_NAME, USN_REASON_RENAME_OLD_NAME,
    };

    const REASONS: u32 = USN_REASON_DATA_OVERWRITE
        | USN_REASON_DATA_EXTEND
        | USN_REASON_DATA_TRUNCATION
        | USN_REASON_FILE_CREATE
        | USN_REASON_FILE_DELETE
        | USN_REASON_RENAME_OLD_NAME
        | USN_REASON_RENAME_NEW_NAME;

    struct Volume(HANDLE);
    // SAFETY: a volume handle may be used from any thread
    unsafe impl Send for Volume {}

    impl Drop for Volume {
        fn drop(&mut self) {
            // SAFETY: the handle was opened by CreateFileW and is closed once
            unsafe { CloseHandle(self.0) };
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain([0]).collect()
    }

    pub fn start(roots: &[PathBuf], tx: mpsc::Sender<Event>) -> Result<()> {
        let mut drives: Vec<char> = Vec::new();
        for r in roots {
            let Some(Component::Prefix(p)) = r.components().next() else {
                bail!(
                    "{} has no drive letter, so its USN journal cannot be read",
                    r.display()
                );
            };
            let Prefix::Disk(d) = p.kind() else {
                bail!(
                    "{} is not on a lettered drive, so its USN journal cannot be read",
                    r.display()
                );
            };
            let d = (d as char).to_ascii_uppercase();
            if !drives.contains(&d) {
                drives.push(d);
            }
        }
        for d in drives {
            let name = wide(&format!("\\\\.\\{d}:"));
            // SAFETY: name is NUL-terminated and outlives the call; the result is checked
            let h = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if h == INVALID_HANDLE_VALUE {
                let e = std::io::Error::last_os_error();
                bail!(
                    "could not open volume {d}: for its USN journal ({e}). Reading the journal \
                     needs an administrator prompt; the daemon still works without --usn"
                );
            }
            let vol = Volume(h);
            let mut data: USN_JOURNAL_DATA_V0 = unsafe { std::mem::zeroed() };
            let mut ret = 0u32;
            // SAFETY: data is a plain struct of the size given
            let ok = unsafe {
                DeviceIoControl(
                    vol.0,
                    FSCTL_QUERY_USN_JOURNAL,
                    std::ptr::null(),
                    0,
                    (&raw mut data).cast(),
                    std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32,
                    &mut ret,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                let e = std::io::Error::last_os_error();
                bail!("volume {d}: has no readable USN journal ({e})");
            }
            let tx = tx.clone();
            std::thread::spawn(move || read_loop(vol, d, data, tx));
        }
        Ok(())
    }

    fn read_loop(vol: Volume, drive: char, data: USN_JOURNAL_DATA_V0, tx: mpsc::Sender<Event>) {
        let mut next = data.NextUsn;
        let mut parents: HashMap<u64, Option<PathBuf>> = HashMap::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut req = READ_USN_JOURNAL_DATA_V0 {
                StartUsn: next,
                ReasonMask: REASONS,
                ReturnOnlyOnClose: 0,
                Timeout: 0,
                BytesToWaitFor: 0,
                UsnJournalID: data.UsnJournalID,
            };
            let mut ret = 0u32;
            // SAFETY: req and buf are valid for the sizes given
            let ok = unsafe {
                DeviceIoControl(
                    vol.0,
                    FSCTL_READ_USN_JOURNAL,
                    (&raw mut req).cast(),
                    std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() as u32,
                    buf.as_mut_ptr().cast(),
                    buf.len() as u32,
                    &mut ret,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                eprintln!(
                    "footprints: stopped reading the USN journal of {drive}: ({})",
                    std::io::Error::last_os_error()
                );
                return;
            }
            let got = &buf[..ret as usize];
            if got.len() < 8 {
                std::thread::sleep(std::time::Duration::from_millis(250));
                continue;
            }
            next = i64::from_le_bytes(got[..8].try_into().unwrap());
            let records = super::parse_usn_records(&got[8..]);
            if records.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(250));
                continue;
            }
            for r in records {
                if r.directory
                    && r.reason & (USN_REASON_RENAME_NEW_NAME | USN_REASON_RENAME_OLD_NAME) != 0
                {
                    // a renamed directory makes every cached path under it stale
                    parents.clear();
                }
                let dir = parents
                    .entry(r.parent)
                    .or_insert_with(|| path_of(&vol, r.parent))
                    .clone();
                if let Some(dir) = dir
                    && tx
                        .send(Event {
                            path: dir.join(&r.name),
                            kind: super::usn_change(r.reason),
                            pid: None,
                            process: None,
                        })
                        .is_err()
                {
                    return;
                }
            }
        }
    }

    fn path_of(vol: &Volume, id: u64) -> Option<PathBuf> {
        let mut desc: FILE_ID_DESCRIPTOR = unsafe { std::mem::zeroed() };
        desc.dwSize = std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32;
        desc.Type = FileIdType;
        desc.Anonymous.FileId = id as i64;
        // SAFETY: desc is fully initialised; the handle is checked and closed
        unsafe {
            let h = OpenFileById(
                vol.0,
                &desc,
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                FILE_FLAG_BACKUP_SEMANTICS,
            );
            if h == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut buf = vec![0u16; 1024];
            let n = GetFinalPathNameByHandleW(
                h,
                buf.as_mut_ptr(),
                buf.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            );
            CloseHandle(h);
            if n == 0 || n as usize >= buf.len() {
                return None;
            }
            let p = PathBuf::from(std::ffi::OsString::from_wide(&buf[..n as usize]));
            Some(crate::platform::strip_verbatim(Path::new(&p)))
        }
    }
}

/// what a USN record's reason flags mean for the daemon. The flags of one file accumulate
/// until it is closed, so a rename anywhere in them wins over a create.
pub fn usn_change(reason: u32) -> Change {
    const FILE_CREATE: u32 = 0x100;
    const FILE_DELETE: u32 = 0x200;
    const RENAME_OLD_NAME: u32 = 0x1000;
    const RENAME_NEW_NAME: u32 = 0x2000;
    if reason & (RENAME_OLD_NAME | RENAME_NEW_NAME) != 0 {
        Change::Rename
    } else if reason & FILE_CREATE != 0 {
        Change::Create
    } else if reason & FILE_DELETE != 0 {
        Change::Remove
    } else {
        Change::Modify
    }
}

/// one USN_RECORD_V2, the format NTFS writes. ReFS writes version 3 records with 128-bit ids,
/// which are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsnRecord {
    pub parent: u64,
    pub reason: u32,
    pub directory: bool,
    pub name: String,
}

/// parses the records after the leading next-USN value of an FSCTL_READ_USN_JOURNAL reply.
/// Kept platform independent and byte based, so the layout is tested on every OS.
pub fn parse_usn_records(mut buf: &[u8]) -> Vec<UsnRecord> {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    let u16_at = |b: &[u8], o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
    let u32_at = |b: &[u8], o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u64_at = |b: &[u8], o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    let mut out = Vec::new();
    while buf.len() >= 60 {
        let len = u32_at(buf, 0) as usize;
        if len < 60 || len > buf.len() {
            break;
        }
        let rec = &buf[..len];
        if u16_at(rec, 4) == 2 {
            let name_len = u16_at(rec, 56) as usize;
            let name_off = u16_at(rec, 58) as usize;
            if name_off + name_len <= len && name_len.is_multiple_of(2) {
                let units: Vec<u16> = rec[name_off..name_off + name_len]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| u16::from_le_bytes(*c))
                    .collect();
                out.push(UsnRecord {
                    parent: u64_at(rec, 16),
                    reason: u32_at(rec, 40),
                    directory: u32_at(rec, 52) & FILE_ATTRIBUTE_DIRECTORY != 0,
                    name: String::from_utf16_lossy(&units),
                });
            }
        }
        // records are padded to 8 bytes and RecordLength includes the padding
        buf = &buf[len..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Filter;
    use std::fs;

    /// a daemon state over `root` in a fresh ledger, with the parent entry the real daemon
    /// makes, driven event by event instead of by a watcher.
    fn daemon(d: &tempfile::TempDir) -> (Store, PathBuf) {
        let root = crate::platform::canonical(d.path()).unwrap().join("root");
        fs::create_dir_all(&root).unwrap();
        let store = Store::open(&d.path().join("home")).unwrap();
        (store, root)
    }

    fn state<'a>(store: &'a Store, root: &Path, snapshot: bool) -> State<'a> {
        let mut parent = Action::new(DAEMON_AGENT, "daemon-test", Kind::Command, "daemon");
        parent.tool = Some(DAEMON_TOOL.into());
        parent.status = Status::Pending;
        parent.id = store.insert(&parent).unwrap();
        let mut st = State::new(
            store,
            parent,
            vec![root.to_path_buf()],
            snapshot::exclusions(store),
        );
        if snapshot {
            st.snapshot_root(root).unwrap();
        }
        st.mark_ready();
        st
    }

    fn ev(path: &Path, kind: Change) -> Event {
        Event {
            path: path.to_path_buf(),
            kind,
            pid: None,
            process: None,
        }
    }

    fn drive(st: &mut State<'_>, events: Vec<Event>) {
        let mut seen = BTreeMap::new();
        for e in events {
            st.note_event(&mut seen, e);
        }
        st.flush(seen, &BTreeSet::new()).unwrap();
        if st.needs_rescan {
            st.reconcile().unwrap();
        }
    }

    fn file_rows(store: &Store) -> Vec<Action> {
        store
            .list(&Filter::default())
            .unwrap()
            .into_iter()
            .filter(|a| a.kind.is_file())
            .collect()
    }

    #[test]
    fn a_file_moved_in_is_never_undone_as_a_delete() {
        let d = tempfile::tempdir().unwrap();
        let (store, root) = daemon(&d);
        let outside = d.path().join("outside.txt");
        fs::write(&outside, "only copy\n").unwrap();
        let mut st = state(&store, &root, true);
        let moved = root.join("moved.txt");
        fs::rename(&outside, &moved).unwrap();
        drive(&mut st, vec![ev(&moved, Change::Rename)]);
        let rows = file_rows(&store);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].reversible, "{:?}", rows[0].note);
        assert!(
            rows[0]
                .note
                .as_deref()
                .unwrap()
                .contains("not deleted on undo")
        );
    }

    #[test]
    fn a_create_seen_before_the_daemon_was_ready_proves_nothing() {
        let d = tempfile::tempdir().unwrap();
        let (store, root) = daemon(&d);
        let mut st = state(&store, &root, true);
        let f = root.join("early.txt");
        fs::write(&f, "x").unwrap();
        st.ready = false;
        let mut seen = BTreeMap::new();
        st.note_event(&mut seen, ev(&f, Change::Create));
        st.mark_ready();
        st.flush(seen, &BTreeSet::new()).unwrap();
        assert!(!file_rows(&store)[0].reversible);
        // a create after the daemon is ready is a real new file
        std::thread::sleep(Duration::from_millis(100));
        let g = root.join("late.txt");
        fs::write(&g, "y").unwrap();
        drive(&mut st, vec![ev(&g, Change::Create)]);
        let late = file_rows(&store)
            .into_iter()
            .find(|a| a.target.ends_with("late.txt"))
            .unwrap();
        assert!(late.reversible);
        assert_eq!(late.kind, Kind::FileCreate);
    }

    #[test]
    fn a_create_event_for_a_file_with_an_old_mtime_is_not_a_new_file() {
        let d = tempfile::tempdir().unwrap();
        let (store, root) = daemon(&d);
        let mut st = state(&store, &root, true);
        // what Windows reports when a file is moved in from another folder: an add event for
        // a file whose modification time is from before the daemon started
        let f = root.join("moved.txt");
        fs::write(&f, "only copy\n").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(old)
            .unwrap();
        drive(&mut st, vec![ev(&f, Change::Create)]);
        let rows = file_rows(&store);
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].reversible, "{:?}", rows[0].note);
    }

    #[test]
    fn a_rescan_after_lost_events_records_deletions_and_edits() {
        let d = tempfile::tempdir().unwrap();
        let (store, root) = daemon(&d);
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/gone.txt"), "bye\n").unwrap();
        fs::write(root.join("kept.txt"), "v1\n").unwrap();
        let mut st = state(&store, &root, true);
        fs::remove_file(root.join("sub/gone.txt")).unwrap();
        fs::write(root.join("kept.txt"), "v2\n").unwrap();
        fs::write(root.join("stray.txt"), "?\n").unwrap();
        // the queue overflowed, so none of those changes has an event of its own
        drive(&mut st, vec![Event::rescan()]);
        let rows = file_rows(&store);
        let find = |name: &str| rows.iter().find(|a| a.target.ends_with(name)).unwrap();
        assert_eq!(find("gone.txt").kind, Kind::FileDelete);
        assert!(find("gone.txt").reversible);
        assert_eq!(find("kept.txt").kind, Kind::FileWrite);
        assert!(find("kept.txt").reversible);
        assert!(
            !find("stray.txt").reversible,
            "a file found by a rescan has no known origin"
        );
    }

    #[test]
    fn an_event_on_a_directory_catches_files_that_vanished_under_it() {
        let d = tempfile::tempdir().unwrap();
        let (store, root) = daemon(&d);
        fs::create_dir(root.join("dir")).unwrap();
        fs::write(root.join("dir/a.txt"), "a\n").unwrap();
        fs::write(root.join("dir/b.txt"), "b\n").unwrap();
        let mut st = state(&store, &root, true);
        fs::remove_file(root.join("dir/a.txt")).unwrap();
        drive(&mut st, vec![ev(&root.join("dir"), Change::Modify)]);
        let rows = file_rows(&store);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, Kind::FileDelete);
        assert!(rows[0].target.ends_with("a.txt"));
    }

    #[test]
    fn a_pending_shell_hook_does_not_hide_its_directory() {
        let d = tempfile::tempdir().unwrap();
        let (store, root) = daemon(&d);
        fs::write(root.join("f.txt"), "v1\n").unwrap();
        // a Bash pre hook whose call was then denied, so no post hook will ever come
        let mut cmd = Action::new("claude-code", "s", Kind::Command, "rm -rf .");
        cmd.status = Status::Pending;
        cmd.tool = Some("Bash".into());
        cmd.cwd = Some(root.to_string_lossy().into_owned());
        cmd.manifest_hash = Some(store.blobs.put(b"{}").unwrap());
        store.insert(&cmd).unwrap();
        let mut st = state(&store, &root, true);
        fs::write(root.join("f.txt"), "v2\n").unwrap();
        drive(&mut st, vec![ev(&root.join("f.txt"), Change::Modify)]);
        let rows = file_rows(&store);
        assert_eq!(rows.len(), 1, "the change is recorded, not skipped");
        assert!(rows[0].reversible);
        assert!(rows[0].note.as_deref().unwrap().contains("shell command"));
    }

    #[test]
    fn usn_reasons_map_to_changes() {
        assert_eq!(usn_change(0x100), Change::Create);
        assert_eq!(usn_change(0x100 | 0x2000), Change::Rename);
        assert_eq!(usn_change(0x200), Change::Remove);
        assert_eq!(usn_change(0x2), Change::Modify);
    }

    fn record(parent: u64, reason: u32, attrs: u32, name: &str) -> Vec<u8> {
        let units: Vec<u8> = name.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let mut len = 60 + units.len();
        len = len.div_ceil(8) * 8;
        let mut r = vec![0u8; len];
        r[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        r[4..6].copy_from_slice(&2u16.to_le_bytes());
        r[8..16].copy_from_slice(&7u64.to_le_bytes());
        r[16..24].copy_from_slice(&parent.to_le_bytes());
        r[40..44].copy_from_slice(&reason.to_le_bytes());
        r[52..56].copy_from_slice(&attrs.to_le_bytes());
        r[56..58].copy_from_slice(&(units.len() as u16).to_le_bytes());
        r[58..60].copy_from_slice(&60u16.to_le_bytes());
        r[60..60 + units.len()].copy_from_slice(&units);
        r
    }

    #[test]
    fn usn_records_are_parsed_with_padding_and_unicode_names() {
        let mut buf = record(42, 0x2, 0x20, "notes.txt");
        buf.extend(record(43, 0x1000, 0x10, "dossier é"));
        // a version 3 record is skipped
        let mut v3 = record(44, 0x1, 0, "skip");
        v3[4..6].copy_from_slice(&3u16.to_le_bytes());
        buf.extend(v3);
        let got = parse_usn_records(&buf);
        assert_eq!(
            got,
            vec![
                UsnRecord {
                    parent: 42,
                    reason: 2,
                    directory: false,
                    name: "notes.txt".into()
                },
                UsnRecord {
                    parent: 43,
                    reason: 0x1000,
                    directory: true,
                    name: "dossier é".into()
                },
            ]
        );
    }

    #[test]
    fn a_truncated_record_stops_the_parse() {
        let mut buf = record(1, 1, 0, "a");
        buf.truncate(buf.len() - 9);
        assert!(parse_usn_records(&buf).is_empty());
    }
}
