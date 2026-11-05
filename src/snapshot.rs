//! walking a directory and capturing file images into the blob store.

use crate::store::{Action, CacheKey, Kind, Store, now_ms};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// directories that are either rebuilt from source or tracked by their own tool, so copying
/// them would cost a lot and protect nothing.
pub const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules"];

const DEFAULT_MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const DEFAULT_MAX_FILES: usize = 10_000;

/// files above this size are recorded but not stored, so one large log cannot fill the disk.
pub fn max_file_bytes() -> u64 {
    env_number("FOOTPRINTS_MAX_FILE_BYTES").unwrap_or(DEFAULT_MAX_FILE_BYTES)
}

/// a walk stops here, so a command run from the home directory does not copy the whole disk.
pub fn max_files() -> usize {
    env_number("FOOTPRINTS_MAX_FILES")
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_MAX_FILES)
}

fn env_number(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub size: u64,
    pub mtime_ns: i64,
    pub mode: Option<u32>,
    #[serde(default)]
    pub ino: u64,
    #[serde(default)]
    pub ctime_ns: i64,
}

impl Meta {
    pub fn of(path: &Path, md: &fs::Metadata) -> Meta {
        let mtime_ns = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let (ino, ctime_ns) = crate::platform::file_identity(path, md);
        Meta {
            size: md.len(),
            mtime_ns,
            mode: mode_of(md),
            ino,
            ctime_ns,
        }
    }

    fn cache_key<'a>(&self, path: &'a str) -> CacheKey<'a> {
        CacheKey {
            path,
            size: self.size as i64,
            mtime_ns: self.mtime_ns,
            ino: self.ino as i64,
            ctime_ns: self.ctime_ns,
        }
    }
}

#[cfg(unix)]
pub fn mode_of(md: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(md.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
pub fn mode_of(_md: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
pub fn set_mode(path: &Path, mode: Option<u32>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(m) = mode {
        fs::set_permissions(path, fs::Permissions::from_mode(m))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn set_mode(_path: &Path, _mode: Option<u32>) -> Result<()> {
    Ok(())
}

pub use crate::platform::normalise;

#[derive(Debug, Default)]
pub struct Walk {
    pub files: BTreeMap<PathBuf, Meta>,
    /// symlinks and where they point. They are recorded so a change involving one is shown,
    /// but they are never followed or restored.
    pub links: BTreeMap<PathBuf, PathBuf>,
    /// true when the walk hit the file limit, so changes beyond it were not seen
    pub truncated: bool,
    /// directories whose contents were not looked at (`.git`, `target`, `node_modules` and
    /// unreadable ones) plus entries whose names are not UTF-8. Files appearing from one of
    /// these look new but may be the only copy of existing data.
    pub opaque: BTreeSet<PathBuf>,
}

/// lists regular files and symlinks under `root`.
pub fn walk(root: &Path, exclude: &[PathBuf], limit: usize) -> Result<Walk> {
    let mut out = Walk::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => {
                out.opaque.insert(dir);
                continue;
            }
        };
        let mut sorted: Vec<_> = entries.filter_map(|e| e.ok()).collect();
        sorted.sort_by_key(|e| e.file_name());
        for entry in sorted {
            let path = entry.path();
            if exclude.iter().any(|x| path.starts_with(x)) {
                continue;
            }
            // the ledger stores paths as UTF-8 text, so other names cannot be restored exactly
            if entry.file_name().to_str().is_none() {
                out.opaque.insert(path);
                continue;
            }
            let Ok(md) = fs::symlink_metadata(&path) else {
                out.opaque.insert(path);
                continue;
            };
            if md.file_type().is_symlink() {
                let target = fs::read_link(&path).unwrap_or_default();
                out.links.insert(path, target);
            } else if md.is_dir() {
                let name = entry.file_name();
                if SKIP_DIRS.iter().any(|s| name == *s) {
                    out.opaque.insert(path);
                } else {
                    stack.push(path);
                }
            } else if md.is_file() {
                if out.files.len() >= limit {
                    out.truncated = true;
                    return Ok(out);
                }
                let meta = Meta::of(&path, &md);
                out.files.insert(path, meta);
            }
        }
    }
    Ok(out)
}

/// what a file looked like at one moment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Image {
    pub exists: bool,
    pub hash: Option<String>,
    pub size: Option<i64>,
    pub mode: Option<u32>,
    /// why the contents were not stored, when something exists at the path but cannot be
    /// restored: too large, a symlink or a directory
    pub unstored: Option<String>,
    /// where the path points when it is a symlink
    pub link: Option<PathBuf>,
}

impl Image {
    /// true when the path is absent or its exact bytes are in the blob store, so whatever is
    /// there now can be brought back after it is overwritten.
    pub fn recoverable(&self) -> bool {
        !self.exists || (self.unstored.is_none() && self.hash.is_some())
    }
}

/// captures what is at a path by reading it. Undo and conflict checks always use this, never
/// the cache.
pub fn capture(store: &Store, path: &Path) -> Result<Image> {
    capture_inner(store, path, None)
}

/// a key and hash waiting to be written to the file cache in one batch.
pub type CacheEntry = (String, Meta, String);

/// a recently modified file may still be changing within one timestamp tick (Linux mtimes can
/// be coarse), so the cache is only trusted for files untouched for this long.
const CACHE_MIN_AGE_NS: i64 = 2_000_000_000;

fn capture_inner(store: &Store, path: &Path, cache: Option<&mut Vec<CacheEntry>>) -> Result<Image> {
    let md = match fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(_) => return Ok(Image::default()),
    };
    if md.file_type().is_symlink() {
        return Ok(Image {
            exists: true,
            size: Some(0),
            unstored: Some("it is a symlink".into()),
            link: Some(fs::read_link(path).unwrap_or_default()),
            ..Default::default()
        });
    }
    if !md.is_file() {
        return Ok(Image {
            exists: true,
            size: Some(0),
            unstored: Some(if md.is_dir() {
                "it is a directory".into()
            } else {
                "it is not a regular file".into()
            }),
            ..Default::default()
        });
    }
    let meta = Meta::of(path, &md);
    let mut img = Image {
        exists: true,
        size: Some(meta.size as i64),
        mode: meta.mode,
        ..Default::default()
    };
    if meta.size > max_file_bytes() {
        img.unstored = Some(format!(
            "it is larger than {} bytes (FOOTPRINTS_MAX_FILE_BYTES)",
            max_file_bytes()
        ));
        return Ok(img);
    }
    let key = path.to_string_lossy().into_owned();
    let now_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(i64::MAX);
    // without a change time a same-size rewrite that keeps the mtime would look unchanged.
    // FAT and exFAT on Windows give a file index but no change time
    let settled = meta.ctime_ns != 0 && now_ns.saturating_sub(meta.mtime_ns) > CACHE_MIN_AGE_NS;
    if let Some(entries) = cache {
        if settled && let Some(h) = store.cache_get(meta.cache_key(&key))? {
            img.hash = Some(h);
            return Ok(img);
        }
        let h = store.blobs.put_file(path)?;
        if settled {
            entries.push((key, meta, h.clone()));
        }
        img.hash = Some(h);
        return Ok(img);
    }
    img.hash = Some(store.blobs.put_file(path)?);
    Ok(img)
}

/// a stored list of every file under a directory, used to find what a command changed. Hashes
/// are present for a Bash hook manifest and absent for a run, whose pre-images are staged
/// copies instead.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub root: PathBuf,
    pub truncated: bool,
    pub files: BTreeMap<PathBuf, ManifestEntry>,
    #[serde(default)]
    pub opaque: BTreeSet<PathBuf>,
    #[serde(default)]
    pub links: BTreeMap<PathBuf, PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub meta: Meta,
    /// none when the file was too large to store or was staged instead
    pub hash: Option<String>,
}

impl Manifest {
    pub fn from_walk(root: &Path, w: &Walk) -> Manifest {
        Manifest {
            root: root.to_path_buf(),
            truncated: w.truncated,
            files: w
                .files
                .iter()
                .map(|(p, m)| {
                    (
                        p.clone(),
                        ManifestEntry {
                            meta: *m,
                            hash: None,
                        },
                    )
                })
                .collect(),
            opaque: w.opaque.clone(),
            links: w.links.clone(),
        }
    }

    pub fn metas(&self) -> BTreeMap<PathBuf, Meta> {
        self.files
            .iter()
            .map(|(p, e)| (p.clone(), e.meta))
            .collect()
    }
}

/// builds the manifest a Bash hook compares against afterwards. Cache writes are collected and
/// committed in one short transaction at the end, so parallel hooks are not kept waiting on the
/// database lock while files are read.
pub fn build_manifest(store: &Store, root: &Path, exclude: &[PathBuf]) -> Result<Manifest> {
    let w = walk(root, exclude, max_files())?;
    let mut m = Manifest::from_walk(root, &w);
    if m.truncated {
        // a partial manifest would report files beyond the limit as deleted, so none is kept
        m.files.clear();
        return Ok(m);
    }
    let budget = snapshot_budget();
    let deadline = std::time::Instant::now() + snapshot_time_budget();
    let mut fresh: u64 = 0;
    let mut entries = Vec::new();
    for (path, meta) in &w.files {
        if std::time::Instant::now() > deadline {
            m.truncated = true;
            break;
        }
        let key = path.to_string_lossy();
        let cached = store.cache_get(meta.cache_key(&key))?;
        if cached.is_none() && meta.size <= max_file_bytes() {
            fresh += meta.size;
            if fresh > budget {
                m.truncated = true;
                break;
            }
        }
        let img = capture_inner(store, path, Some(&mut entries))?;
        if let Some(e) = m.files.get_mut(path) {
            e.hash = img.hash;
        }
    }
    if m.truncated {
        m.files.clear();
    }
    // the cache is kept even when the budget ran out, so a later call gets further
    store.batch(|store| {
        for (k, meta, h) in &entries {
            store.cache_put(meta.cache_key(k), h)?;
        }
        Ok(())
    })?;
    Ok(m)
}

const DEFAULT_SNAPSHOT_BUDGET: u64 = 256 * 1024 * 1024;
const DEFAULT_SNAPSHOT_SECONDS: u64 = 10;

/// cold reads can be far slower than the byte budget suggests, so a hook snapshot also stops
/// after this long. The command hook timeout is 600 seconds by default, but an agent waits on
/// every hook call, so the budget is kept far below it.
pub fn snapshot_time_budget() -> std::time::Duration {
    std::time::Duration::from_secs(
        env_number("FOOTPRINTS_SNAPSHOT_SECONDS").unwrap_or(DEFAULT_SNAPSHOT_SECONDS),
    )
}

/// the most new bytes one shell hook snapshot reads, so a first call in a large project cannot
/// stall the agent.
pub fn snapshot_budget() -> u64 {
    env_number("FOOTPRINTS_SNAPSHOT_BUDGET_BYTES").unwrap_or(DEFAULT_SNAPSHOT_BUDGET)
}

/// true when a directory that was not looked inside appeared, disappeared or became readable,
/// since files that seem new may then be moved existing data.
pub fn opaque_changed(before: &BTreeSet<PathBuf>, after: &BTreeSet<PathBuf>) -> bool {
    before != after
}

pub const OPAQUE_NOTE: &str = "a skipped or unreadable directory (.git, target, node_modules or \
     one without read access) changed during this command, so this new file may be moved \
     existing data. It is not deleted on undo";

/// paths that may differ between two walks: files added, removed or with different metadata
/// plus symlinks added, removed or repointed. Callers confirm file changes by hash, since a
/// touched file may be unchanged.
pub fn candidates(
    before: &BTreeMap<PathBuf, Meta>,
    before_links: &BTreeMap<PathBuf, PathBuf>,
    after: &Walk,
) -> Vec<PathBuf> {
    let mut out: BTreeSet<PathBuf> = BTreeSet::new();
    for (p, m) in before {
        match after.files.get(p) {
            Some(n) if n == m => {}
            _ => {
                out.insert(p.clone());
            }
        }
    }
    for p in after.files.keys() {
        if !before.contains_key(p) {
            out.insert(p.clone());
        }
    }
    for (p, t) in before_links {
        if after.links.get(p) != Some(t) {
            out.insert(p.clone());
        }
    }
    for (p, t) in &after.links {
        if before_links.get(p) != Some(t) {
            out.insert(p.clone());
        }
    }
    out.into_iter().collect()
}

/// what was at a path before a command ran.
pub enum Before {
    Absent,
    /// a regular file; `stored` is the blob hash of its contents when they could be kept
    File {
        meta: Meta,
        stored: Option<String>,
    },
    Link(PathBuf),
}

/// builds the ledger entry for one path a command may have changed. None means the path is in
/// fact unchanged. Anything footprints cannot put back exactly is marked not reversible
/// with the reason in its note.
pub fn change_action(
    store: &Store,
    parent: &Action,
    path: &Path,
    before: Before,
    opaque_moved: bool,
) -> Result<Option<Action>> {
    let img = capture(store, path)?;
    change_action_from(parent, path, before, opaque_moved, img)
}

/// [`change_action`] with an image the caller already captured, so the post-image recorded is
/// exactly the one the caller saw.
pub fn change_action_from(
    parent: &Action,
    path: &Path,
    before: Before,
    opaque_moved: bool,
    img: Image,
) -> Result<Option<Action>> {
    let unchanged = match &before {
        Before::Absent => !img.exists,
        Before::File {
            stored: Some(h), ..
        } => img.exists && img.link.is_none() && img.hash.as_ref() == Some(h),
        Before::File { stored: None, .. } => false,
        Before::Link(t) => img.link.as_ref() == Some(t),
    };
    if unchanged {
        return Ok(None);
    }
    let existed = !matches!(before, Before::Absent);
    let mut a = Action::new(
        &parent.agent,
        &parent.session,
        Kind::for_file(existed, img.exists),
        &path.to_string_lossy(),
    );
    let mut reasons: Vec<String> = Vec::new();
    match &before {
        Before::Absent => {}
        Before::File { meta, stored } => {
            a.pre_hash = stored.clone();
            a.pre_size = Some(meta.size as i64);
            a.pre_mode = meta.mode;
            if stored.is_none() {
                reasons.push(format!(
                    "the old contents were not stored (larger than {} bytes or unreadable)",
                    max_file_bytes()
                ));
            }
        }
        Before::Link(t) => {
            a.pre_size = Some(0);
            reasons.push(format!(
                "it was a symlink to {} before; symlinks are recorded but not restored",
                t.display()
            ));
        }
    }
    a.post_hash = img.hash.clone();
    a.post_size = if img.exists { img.size } else { None };
    if let Some(t) = &img.link {
        reasons.push(format!(
            "it is now a symlink to {}; symlinks are recorded but not restored",
            t.display()
        ));
    } else if let Some(why) = &img.unstored {
        reasons.push(format!("the new contents were not stored because {why}"));
    }
    if !existed && opaque_moved {
        reasons.push(OPAQUE_NOTE.into());
    }
    a.reversible = reasons.is_empty();
    if !reasons.is_empty() {
        a.note = Some(reasons.join("; "));
    }
    a.tool = parent.tool.clone();
    a.tool_use_id = parent.tool_use_id.clone();
    a.parent_id = Some(parent.id);
    a.cwd = parent.cwd.clone();
    a.intent = parent.intent.clone();
    a.end_time_ms = Some(now_ms());
    Ok(Some(a))
}

/// the data directory must never be watched. Otherwise recording an action would itself look
/// like a change to record.
pub fn exclusions(store: &Store) -> Vec<PathBuf> {
    let home = store.home();
    let mut v = vec![home.to_path_buf()];
    if let Ok(c) = crate::platform::canonical(home) {
        v.push(c);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_skips_build_and_vcs_dirs() {
        let d = tempfile::tempdir().unwrap();
        for p in [
            "a.txt",
            "src/b.rs",
            ".git/HEAD",
            "target/x",
            "node_modules/y/z.js",
        ] {
            let full = d.path().join(p);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, p).unwrap();
        }
        let w = walk(d.path(), &[], 100).unwrap();
        let names: Vec<_> = w
            .files
            .keys()
            .map(|p| {
                p.strip_prefix(d.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(names, vec!["a.txt", "src/b.rs"]);
        assert!(!w.truncated);
        assert_eq!(w.opaque.len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn walk_records_symlinks_without_following_them() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("real"), "x").unwrap();
        std::os::unix::fs::symlink("real", d.path().join("link")).unwrap();
        let w = walk(d.path(), &[], 100).unwrap();
        assert_eq!(w.files.len(), 1);
        assert_eq!(
            w.links.get(&d.path().join("link")),
            Some(&PathBuf::from("real"))
        );
    }

    #[test]
    fn walk_reports_truncation() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..5 {
            fs::write(d.path().join(format!("f{i}")), "x").unwrap();
        }
        let w = walk(d.path(), &[], 3).unwrap();
        assert_eq!(w.files.len(), 3);
        assert!(w.truncated);
    }

    #[test]
    fn candidates_finds_adds_removes_edits_and_links() {
        let m = |size| Meta {
            size,
            mtime_ns: 1,
            mode: None,
            ino: 1,
            ctime_ns: 1,
        };
        let before: BTreeMap<_, _> = [("a".into(), m(1)), ("b".into(), m(1))].into();
        let links: BTreeMap<PathBuf, PathBuf> = [("l".into(), "x".into())].into();
        let after = Walk {
            files: [("b".into(), m(2)), ("c".into(), m(1))].into(),
            links: [("l".into(), "y".into())].into(),
            ..Default::default()
        };
        let c = candidates(&before, &links, &after);
        assert_eq!(
            c,
            vec![PathBuf::from("a"), "b".into(), "c".into(), "l".into()]
        );
    }
}
