//! reversing recorded file actions.
//!
//! an undo only goes ahead when the file still matches the recorded post-image. Anything else
//! means someone changed the file since. Overwriting that silently would lose work.

use crate::snapshot::{self, Image};
use crate::store::{Action, Filter, Kind, Status, Store, now_ms};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub const UNDO_AGENT: &str = "footprints";

/// an undo that was declined on purpose. The CLI exits with status 3 for these so scripts can
/// tell a refusal from a crash.
#[derive(Debug)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// how one undo command behaves.
#[derive(Debug, Clone)]
pub struct UndoOpts {
    /// overwrite a conflicting change, only ever set from the command line
    pub force: bool,
    /// whether a conflict message may suggest `--force`; false for the MCP server, so an agent
    /// is never pointed at a way round the conflict check
    pub offer_force: bool,
    /// the session name the undo actions are recorded under
    pub batch: String,
}

impl UndoOpts {
    pub fn cli(force: bool) -> Self {
        UndoOpts {
            force,
            offer_force: true,
            batch: new_batch(),
        }
    }

    pub fn agent() -> Self {
        UndoOpts {
            force: false,
            offer_force: false,
            batch: new_batch(),
        }
    }
}

/// a new session name for one undo command, so the undo actions it records can be reversed
/// together without touching any other undo.
pub fn new_batch() -> String {
    let seed = format!("{}-{}", now_ms(), std::process::id());
    format!("undo-{}", &blake3::hash(seed.as_bytes()).to_hex()[..8])
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Undone {
    pub id: i64,
    pub path: String,
    /// restored, deleted or recreated
    pub effect: &'static str,
    /// the new ledger entry describing the undo, which can itself be undone
    pub undo_action: i64,
    pub session: String,
    pub forced: bool,
}

impl fmt::Display for Undone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "undid #{}: {} {}{} (recorded as #{} in session {})",
            self.id,
            self.effect,
            self.path,
            if self.forced { ", forced" } else { "" },
            self.undo_action,
            self.session
        )
    }
}

fn short(h: &Option<String>) -> String {
    match h {
        Some(h) => format!("blake3 {}", &h[..12.min(h.len())]),
        None => "not stored".into(),
    }
}

fn describe_expected(a: &Action) -> String {
    match a.post_size {
        Some(size) => format!("{} ({size} bytes)", short(&a.post_hash)),
        None => "file absent (the agent deleted it)".into(),
    }
}

fn describe_current(img: &Image) -> String {
    if !img.exists {
        return "file absent".into();
    }
    match &img.unstored {
        Some(why) => format!("present but not stored: {why}"),
        None => format!("{} ({} bytes)", short(&img.hash), img.size.unwrap_or(0)),
    }
}

fn matches_post(a: &Action, current: &Image) -> bool {
    match a.post_size {
        Some(_) => {
            current.exists
                && current.unstored.is_none()
                && current.hash.is_some()
                && current.hash == a.post_hash
        }
        None => !current.exists,
    }
}

/// why this action can never be undone. None means it can be.
fn not_undoable(a: &Action) -> Option<String> {
    if !a.kind.is_file() {
        return Some(match a.kind {
            Kind::Network => format!(
                "#{} is a network call. Network calls cannot be undone: the data has already left the machine",
                a.id
            ),
            _ => format!(
                "#{} is a shell command. A command cannot be undone as a whole; undo the file changes listed under it instead (footprints show {})",
                a.id, a.id
            ),
        });
    }
    match a.status {
        Status::Undone => {
            return Some(match a.undone_by {
                Some(by) => format!("#{} was already undone (see #{by})", a.id),
                None => format!("#{} was already undone", a.id),
            });
        }
        Status::Pending => {
            return Some(format!(
                "#{} has no post-image yet: the tool call may have been denied or may still be running",
                a.id
            ));
        }
        Status::Recorded => {}
    }
    if !a.reversible {
        return Some(format!(
            "#{} is marked not reversible: {}",
            a.id,
            a.note.as_deref().unwrap_or("its pre-image was not stored")
        ));
    }
    None
}

fn conflict_message(a: &Action, current: &Image, offer_force: bool) -> String {
    let advice = if !offer_force {
        "Forcing is only available from the command line".to_string()
    } else if current.recoverable() {
        format!(
            "Run `footprints undo {} --force` to overwrite it with the pre-image. The current \
             contents are stored first, so that undo can itself be undone",
            a.id
        )
    } else {
        format!(
            "--force is not available because the current contents cannot be stored first ({}). \
             Move the file aside by hand if the pre-image should win",
            current.unstored.as_deref().unwrap_or("unknown reason")
        )
    };
    format!(
        "conflict: {} changed after #{} was recorded\n  recorded post-image: {}\n  current file:        {}\n\
         Nothing was changed. Run `footprints diff {}` to see the recorded change. {advice}",
        a.target,
        a.id,
        describe_expected(a),
        describe_current(current),
        a.id,
    )
}

fn missing_blob(store: &Store, a: &Action) -> Option<String> {
    let h = a.pre_hash.as_ref()?;
    (!store.blobs.contains(h)).then(|| {
        format!(
            "#{}: the stored pre-image of {} (blake3 {}) is missing from the blob store",
            a.id,
            a.target,
            &h[..12.min(h.len())]
        )
    })
}

/// undoes one action. `force` overwrites a conflicting change, but only when the current
/// contents can be stored first, so a forced undo never destroys data outright.
pub fn undo_one(store: &Store, id: i64, o: &UndoOpts) -> Result<Undone> {
    let (force, batch) = (o.force, o.batch.as_str());
    let a = store.require(id)?;
    if let Some(why) = not_undoable(&a) {
        return Err(Refused(why).into());
    }
    if let Some(why) = missing_blob(store, &a) {
        return Err(Refused(why).into());
    }
    let current = snapshot::capture(store, Path::new(&a.target))?;
    let conflict = !matches_post(&a, &current);
    if conflict && (!force || !current.recoverable()) {
        return Err(Refused(conflict_message(&a, &current, o.offer_force)).into());
    }
    if let Some(blocker) = a
        .pre_size
        .and_then(|_| blocking_ancestor(Path::new(&a.target), &HashMap::new()))
    {
        return Err(Refused(format!(
            "#{}: cannot restore {} because {} is not a directory",
            a.id,
            a.target,
            blocker.display()
        ))
        .into());
    }
    apply(store, a, current, conflict, batch)
}

#[derive(Debug, Clone)]
enum Sim {
    File(Option<String>, Option<i64>),
    Absent,
    Dir,
}

fn sim_image(store: &Store, state: &HashMap<PathBuf, Sim>, path: &Path) -> Result<Image> {
    Ok(match state.get(path) {
        Some(Sim::File(hash, size)) => Image {
            exists: true,
            hash: hash.clone(),
            size: *size,
            ..Default::default()
        },
        Some(Sim::Absent) => Image::default(),
        Some(Sim::Dir) => Image {
            exists: true,
            size: Some(0),
            unstored: Some("it is a directory".into()),
            ..Default::default()
        },
        None => snapshot::capture(store, path)?,
    })
}

/// the nearest ancestor of `path` that is something other than a directory, which would stop
/// a restore from creating the file.
fn blocking_ancestor(path: &Path, state: &HashMap<PathBuf, Sim>) -> Option<PathBuf> {
    let mut cur = path.parent();
    while let Some(dir) = cur {
        if dir.parent().is_none() {
            break;
        }
        match state.get(dir) {
            Some(Sim::Dir) | Some(Sim::Absent) => {}
            Some(Sim::File(..)) => return Some(dir.to_path_buf()),
            None => {
                if let Ok(md) = fs::symlink_metadata(dir) {
                    // a symlinked parent is refused too, since following it could write
                    // outside the recorded path
                    if !md.is_dir() || md.file_type().is_symlink() {
                        return Some(dir.to_path_buf());
                    }
                }
            }
        }
        cur = dir.parent();
    }
    None
}

#[derive(Debug, serde::Serialize)]
pub struct SessionUndo {
    pub done: Vec<Undone>,
    pub skipped: Vec<(i64, String)>,
    pub session: String,
}

/// undoes every reversible file action of a session, newest first.
///
/// before any file is touched the whole session is simulated: each step must match what is on
/// disk at that point, its pre-image must be in the blob store and its parent directories must
/// be creatable. A file that replaced a directory is removed before anything is restored under
/// that directory's path. Only when every step passes does the real undo start. If the disk
/// still changes underneath it or a write fails, it stops at that step and reports what was
/// already undone.
pub fn undo_session(store: &Store, session: &str, o: &UndoOpts) -> Result<SessionUndo> {
    let (force, batch) = (o.force, o.batch.as_str());
    let mut actions = store.list(&Filter {
        session: Some(session.to_string()),
        ..Default::default()
    })?;
    if actions.is_empty() {
        return Err(Refused(format!("no actions recorded for session {session:?}")).into());
    }
    actions.reverse();
    let mut remaining = Vec::new();
    let mut skipped = Vec::new();
    for a in actions {
        match not_undoable(&a) {
            // already undone or a command is expected in a session, so it is noted, not fatal
            Some(why) => skipped.push((a.id, why)),
            None => remaining.push(a),
        }
    }
    let total = remaining.len();
    let mut state: HashMap<PathBuf, Sim> = HashMap::new();
    let mut order: Vec<Action> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    while !remaining.is_empty() {
        let mut picked = None;
        let mut seen: HashSet<&str> = HashSet::new();
        for (i, a) in remaining.iter().enumerate() {
            // a path's own actions are always undone newest first
            if !seen.insert(a.target.as_str()) {
                continue;
            }
            if a.pre_size.is_some() && blocking_ancestor(Path::new(&a.target), &state).is_some() {
                continue;
            }
            picked = Some(i);
            break;
        }
        let Some(i) = picked else {
            for a in &remaining {
                let blocker = blocking_ancestor(Path::new(&a.target), &state)
                    .map(|b| b.display().to_string())
                    .unwrap_or_else(|| "a parent path".into());
                failures.push(format!(
                    "#{}: cannot restore {} because {blocker} is not a directory",
                    a.id, a.target
                ));
            }
            break;
        };
        let a = remaining.remove(i);
        let path = PathBuf::from(&a.target);
        if let Some(why) = missing_blob(store, &a) {
            failures.push(why);
        }
        let current = sim_image(store, &state, &path)?;
        if !matches_post(&a, &current) && (!force || !current.recoverable()) {
            failures.push(conflict_message(&a, &current, o.offer_force));
        }
        if a.pre_size.is_some() {
            let mut anc = path.parent();
            while let Some(d) = anc {
                if d.parent().is_none() {
                    break;
                }
                state.insert(d.to_path_buf(), Sim::Dir);
                anc = d.parent();
            }
            state.insert(path, Sim::File(a.pre_hash.clone(), a.pre_size));
        } else {
            state.insert(path, Sim::Absent);
        }
        order.push(a);
    }
    if !failures.is_empty() {
        return Err(Refused(format!(
            "{} problem(s) found in session {session:?} with {total} reversible action(s), so nothing was undone:\n\n{}",
            failures.len(),
            failures.join("\n\n")
        ))
        .into());
    }
    let mut done: Vec<Undone> = Vec::new();
    let n = order.len();
    for (step, a) in order.into_iter().enumerate() {
        let (id, target) = (a.id, a.target.clone());
        let result = (|| {
            let current = snapshot::capture(store, Path::new(&a.target))?;
            let conflict = !matches_post(&a, &current);
            if conflict && (!force || !current.recoverable()) {
                return Err(Refused(conflict_message(&a, &current, o.offer_force)).into());
            }
            apply(store, a, current, conflict, batch)
        })();
        match result {
            Ok(u) => done.push(u),
            Err(e) => {
                let undone: Vec<String> = done.iter().map(|u| format!("#{}", u.id)).collect();
                return Err(Refused(format!(
                    "undo stopped at step {} of {n}, #{id} ({target}): {e:#}\nalready undone: {}\n\
                     Those can be reversed with `footprints undo --session {batch}`",
                    step + 1,
                    if undone.is_empty() {
                        "none".into()
                    } else {
                        undone.join(" ")
                    }
                ))
                .into());
            }
        }
    }
    Ok(SessionUndo {
        done,
        skipped,
        session: batch.to_string(),
    })
}

fn same(a: &Image, b: &Image) -> bool {
    a.exists == b.exists && a.hash == b.hash && a.link == b.link && a.unstored == b.unstored
}

fn apply(
    store: &Store,
    mut a: Action,
    current: Image,
    forced: bool,
    batch: &str,
) -> Result<Undone> {
    if forced && !current.recoverable() {
        return Err(Refused(conflict_message(&a, &current, true)).into());
    }
    let path = PathBuf::from(&a.target);
    let effect = if a.pre_size.is_some() {
        let hash = a
            .pre_hash
            .as_deref()
            .with_context(|| format!("#{} has no stored pre-image", a.id))?;
        let bytes = store.blobs.get(hash)?;
        write_atomic(store, &path, &bytes, a.pre_mode, &current)?;
        if current.exists {
            "restored"
        } else {
            "recreated"
        }
    } else {
        if current.exists {
            let now = snapshot::capture(store, &path)?;
            if !same(&now, &current) {
                return Err(Refused(format!(
                    "{} changed while the undo was running, so it was left alone",
                    a.target
                ))
                .into());
            }
            fs::remove_file(&path).with_context(|| format!("deleting {}", path.display()))?;
        }
        "deleted"
    };
    let restored = snapshot::capture(store, &path)?;
    let mut u = Action::new(
        UNDO_AGENT,
        batch,
        Kind::for_file(current.exists, restored.exists),
        &a.target,
    );
    u.pre_hash = current.hash.clone();
    u.pre_size = if current.exists { current.size } else { None };
    u.pre_mode = current.mode;
    u.post_hash = restored.hash;
    u.post_size = if restored.exists { restored.size } else { None };
    u.reversible = current.recoverable();
    u.intent = Some(format!("undo of #{}", a.id));
    u.undo_of = Some(a.id);
    u.end_time_ms = Some(now_ms());
    if forced {
        u.note = Some(
            "forced over a conflicting change; the overwritten contents are this action's pre-image"
                .into(),
        );
    }
    let uid = store.insert(&u)?;
    a.status = Status::Undone;
    a.undone_by = Some(uid);
    store.save(&a)?;
    // undoing an undo puts the original change back, so the original becomes undoable again
    if let Some(orig) = a.undo_of
        && let Some(mut o) = store.get(orig)?
        && o.status == Status::Undone
        && o.undone_by == Some(a.id)
    {
        o.status = Status::Recorded;
        o.undone_by = None;
        store.save(&o)?;
    }
    Ok(Undone {
        id: a.id,
        path: a.target,
        effect,
        undo_action: uid,
        session: batch.to_string(),
        forced,
    })
}

/// writes a new temporary file beside the target, checks the target is still exactly what was
/// examined, then renames over it. The temporary name is random and opened with `create_new`,
/// so an existing file or symlink there is never followed or reused.
fn write_atomic(
    store: &Store,
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
    expected: &Image,
) -> Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let seed = format!("{}-{}-{}", now_ms(), std::process::id(), name);
    let rand = &blake3::hash(seed.as_bytes()).to_hex()[..12];
    let tmp = path.with_file_name(format!(".{name}.footprints-{rand}"));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let result = (|| -> Result<()> {
        let mut f = opts
            .open(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(bytes)?;
        // flushed before the rename so a crash cannot leave an empty file in place of the original
        f.sync_all()?;
        drop(f);
        snapshot::set_mode(&tmp, mode)?;
        let now = snapshot::capture(store, path)?;
        if !same(&now, expected) {
            return Err(Refused(format!(
                "{} changed while the undo was running, so it was left alone",
                path.display()
            ))
            .into());
        }
        fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let store = Store::open(&d.path().join("home")).unwrap();
        (d, store)
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".footprints-"))
            .collect()
    }

    #[test]
    fn write_atomic_refuses_when_the_target_changed_since_it_was_checked() {
        let (d, store) = setup();
        let f = d.path().join("f");
        fs::write(&f, "seen\n").unwrap();
        let seen = snapshot::capture(&store, &f).unwrap();
        // someone writes between the check and the rename
        fs::write(&f, "late edit\n").unwrap();
        let err = write_atomic(&store, &f, b"restored\n", None, &seen).unwrap_err();
        assert!(err.is::<Refused>());
        assert_eq!(fs::read_to_string(&f).unwrap(), "late edit\n");
        assert!(
            leftovers(d.path()).is_empty(),
            "the temporary file is removed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_replaces_a_symlink_without_following_it() {
        let (d, store) = setup();
        let outside = d.path().join("outside");
        fs::write(&outside, "must not change\n").unwrap();
        let f = d.path().join("f");
        std::os::unix::fs::symlink(&outside, &f).unwrap();
        let seen = snapshot::capture(&store, &f).unwrap();
        write_atomic(&store, &f, b"restored\n", Some(0o644), &seen).unwrap();
        assert_eq!(fs::read_to_string(&outside).unwrap(), "must not change\n");
        assert!(!fs::symlink_metadata(&f).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&f).unwrap(), "restored\n");
        assert!(leftovers(d.path()).is_empty());
    }
}
