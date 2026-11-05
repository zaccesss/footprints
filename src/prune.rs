//! `footprints prune` and `footprints import`: removing old history and bringing it back.
//!
//! a prune changes nothing unless it is confirmed. By default the pruned rows and every blob
//! they refer to are first written as an archive folder laid out like a data directory. That
//! folder is moved to the system Trash. `footprints import` puts an archive back.
//! `FOOTPRINTS_HOME=<archive> footprints log` reads one in place.
//!
//! neither command is offered over MCP: an agent must never be able to erase its own trail.

use crate::output::format_time;
use crate::snapshot::Manifest;
use crate::store::{self, Action, Store, now_ms};
use crate::undo::Refused;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// when set, archives are moved into this folder instead of the system Trash. Tests use it so
/// the real Trash is never touched. It also gives a machine without a Trash a recoverable option.
pub const TRASH_DIR_VAR: &str = "FOOTPRINTS_TRASH_DIR";

/// a pending action younger than this may still get its post hook, so its session is treated
/// as still being recorded. An older one is a call that was denied or never reported back.
const LIVE_PENDING_MS: i64 = 24 * 60 * 60 * 1000;

/// where an archive is assembled before it is moved out. It sits inside the data directory so
/// the move to the Trash stays on one file system.
const BUILD_DIR: &str = "pruning";

/// the sessions shown in the plan before the rest are summarised in one line.
const MAX_SESSION_LINES: usize = 20;

/// which actions to prune. Every selector that is set must match (they combine with AND).
#[derive(Debug, Default, Clone)]
pub struct Selectors {
    pub older_than_ms: Option<i64>,
    pub before_ms: Option<i64>,
    pub session: Option<String>,
    pub agent: Option<String>,
    pub all: bool,
    /// prune the oldest matching sessions until the data directory fits in this many bytes
    pub max_size: Option<u64>,
}

impl Selectors {
    pub fn is_empty(&self) -> bool {
        self.older_than_ms.is_none()
            && self.before_ms.is_none()
            && self.session.is_none()
            && self.agent.is_none()
            && !self.all
            && self.max_size.is_none()
    }
}

/// what happens to the pruned data.
#[derive(Debug, Clone)]
pub enum Disposal {
    /// the archive goes to the system Trash or Recycle Bin
    Trash,
    /// the archive is moved into this folder
    Folder(PathBuf),
    /// no archive is written
    Permanent,
}

impl Disposal {
    /// the system Trash unless `FOOTPRINTS_TRASH_DIR` names a folder.
    pub fn from_env(permanent: bool) -> Disposal {
        if permanent {
            return Disposal::Permanent;
        }
        match std::env::var_os(TRASH_DIR_VAR).filter(|v| !v.is_empty()) {
            Some(d) => Disposal::Folder(PathBuf::from(d)),
            None => Disposal::Trash,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub session: String,
    pub agents: Vec<String>,
    pub actions: usize,
    pub first_ms: i64,
    pub last_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    /// ids of the actions to remove, oldest first
    pub actions: Vec<i64>,
    pub sessions: Vec<SessionSummary>,
    /// blobs that only the pruned actions use, deleted from the store
    pub freed_blobs: Vec<String>,
    /// every stored blob the pruned actions use, copied into the archive
    pub archived_blobs: Vec<String>,
    /// blobs the pruned actions use that stay because a remaining action uses them too
    pub shared_blobs: usize,
    pub blob_bytes: u64,
    /// an estimate: the ledger's share of the pruned rows, reclaimed by VACUUM
    pub ledger_bytes: u64,
    pub size_before: u64,
    pub budget: Option<u64>,
    /// sessions left alone because they are still being recorded
    pub live_sessions: Vec<String>,
    /// matching actions kept because the command they belong to (or a change under it) stays
    pub kept_with_command: usize,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub fn bytes_freed(&self) -> u64 {
        self.blob_bytes + self.ledger_bytes
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub plan: Plan,
    /// false for a dry run
    pub pruned: bool,
    /// `trash`, `folder` or `none`
    pub archive_kept_in: &'static str,
    pub archive_name: Option<String>,
    /// the archive's full path when it was moved into a folder
    pub archive_path: Option<PathBuf>,
    pub size_after: Option<u64>,
    /// set on a dry run when another process has the ledger open, since `--yes` would refuse
    pub busy: bool,
}

/// one row of the ledger as prune sees it, with every blob it refers to.
struct Row {
    id: i64,
    time_ms: i64,
    last_ms: i64,
    agent: String,
    session: String,
    parent_id: Option<i64>,
    live: bool,
    refs: Vec<String>,
    /// an estimate of the row's share of the ledger file
    bytes: u64,
}

/// the integers, status and record header a row stores beside its text, roughly.
const ROW_OVERHEAD: u64 = 100;

fn load_rows(store: &Store) -> Result<Vec<Row>> {
    let now = now_ms();
    let mut stmt = store.conn().prepare(
        "SELECT id, time_ms, COALESCE(end_time_ms, time_ms), agent, session, parent_id, status,
            staging, pre_hash, post_hash, manifest_hash,
            LENGTH(agent) + LENGTH(session) + LENGTH(target) + COALESCE(LENGTH(intent), 0)
            + COALESCE(LENGTH(note), 0) + COALESCE(LENGTH(cwd), 0) + COALESCE(LENGTH(tool), 0)
            + COALESCE(LENGTH(tool_use_id), 0) + COALESCE(LENGTH(pre_hash), 0)
            + COALESCE(LENGTH(post_hash), 0) + COALESCE(LENGTH(manifest_hash), 0)
            FROM actions ORDER BY id",
    )?;
    let mut rows = Vec::new();
    let mut q = stmt.query([])?;
    while let Some(r) = q.next()? {
        let time_ms: i64 = r.get(1)?;
        let status: String = r.get(6)?;
        let staging: Option<String> = r.get(7)?;
        let mut refs = Vec::new();
        for i in [8, 9] {
            if let Some(h) = r.get::<_, Option<String>>(i)? {
                refs.push(h);
            }
        }
        if let Some(m) = r.get::<_, Option<String>>(10)? {
            refs.extend(manifest_refs(store, &m));
            refs.push(m);
        }
        refs.sort();
        refs.dedup();
        rows.push(Row {
            id: r.get(0)?,
            time_ms,
            last_ms: r.get(2)?,
            agent: r.get(3)?,
            session: r.get(4)?,
            parent_id: r.get(5)?,
            // a killed run can still be recorded by `footprints recover`, however old it is
            live: status == "pending" && (staging.is_some() || time_ms >= now - LIVE_PENDING_MS),
            refs,
            bytes: r.get::<_, i64>(11)?.max(0) as u64 + ROW_OVERHEAD,
        });
    }
    Ok(rows)
}

/// the file images a shell command's manifest points at, so they count as in use.
fn manifest_refs(store: &Store, hash: &str) -> Vec<String> {
    let Ok(bytes) = store.blobs.get(hash) else {
        return Vec::new();
    };
    let Ok(m) = serde_json::from_slice::<Manifest>(&bytes) else {
        return Vec::new();
    };
    m.files.into_values().filter_map(|e| e.hash).collect()
}

fn matches(r: &Row, sel: &Selectors, now: i64) -> bool {
    sel.older_than_ms.is_none_or(|d| r.last_ms < now - d)
        && sel.before_ms.is_none_or(|t| r.last_ms < t)
        && sel.session.as_ref().is_none_or(|s| &r.session == s)
        && sel.agent.as_ref().is_none_or(|a| &r.agent == a)
}

/// works out what a prune would remove without changing anything.
pub fn plan(store: &Store, sel: &Selectors) -> Result<Plan> {
    if sel.is_empty() {
        return Err(Refused(
            "choose what to prune: --older-than, --before, --session, --agent, --max-size or --all"
                .into(),
        )
        .into());
    }
    let now = now_ms();
    let rows = load_rows(store)?;
    let live_sessions: BTreeSet<String> = rows
        .iter()
        .filter(|r| r.live)
        .map(|r| r.session.clone())
        .collect();
    let mut candidate: HashSet<i64> = rows
        .iter()
        .filter(|r| matches(r, sel, now) && !live_sessions.contains(&r.session))
        .map(|r| r.id)
        .collect();
    let matched_live = rows
        .iter()
        .filter(|r| matches(r, sel, now) && live_sessions.contains(&r.session))
        .map(|r| r.session.clone())
        .collect::<BTreeSet<_>>();
    // a command and the file changes recorded under it go together, so an undo or a diff never
    // meets half a command
    let family = |r: &Row| r.parent_id.unwrap_or(r.id);
    let mut whole: HashMap<i64, bool> = HashMap::new();
    for r in &rows {
        let e = whole.entry(family(r)).or_insert(true);
        *e &= candidate.contains(&r.id);
    }
    let family_of: HashMap<i64, i64> = rows.iter().map(|r| (r.id, family(r))).collect();
    let before = candidate.len();
    candidate.retain(|id| {
        family_of
            .get(id)
            .and_then(|f| whole.get(f))
            .copied()
            .unwrap_or(false)
    });
    let kept_with_command = before - candidate.len();

    let mut users: HashMap<&str, usize> = HashMap::new();
    for r in &rows {
        for h in &r.refs {
            *users.entry(h.as_str()).or_default() += 1;
        }
    }
    let size_before = data_size(store.home());

    // sessions oldest first, so a size budget gives up the oldest history first
    let mut by_session: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    for r in rows.iter().filter(|r| candidate.contains(&r.id)) {
        by_session.entry(r.session.clone()).or_default().push(r);
    }
    let mut order: Vec<(String, Vec<&Row>)> = by_session.into_iter().collect();
    order.sort_by_key(|(_, rs)| rs.iter().map(|r| r.time_ms).min().unwrap_or(0));

    let mut chosen: Vec<&Row> = Vec::new();
    let mut sessions = Vec::new();
    let mut freed: BTreeSet<String> = BTreeSet::new();
    let mut blob_bytes = 0u64;
    let mut ledger_bytes = 0u64;
    for (session, rs) in order {
        if let Some(budget) = sel.max_size {
            let projected = size_before.saturating_sub(blob_bytes + ledger_bytes);
            if projected <= budget {
                break;
            }
        }
        for r in &rs {
            for h in &r.refs {
                let n = users.get_mut(h.as_str()).expect("counted above");
                *n -= 1;
                if *n == 0
                    && let Some(len) = store.blobs.stored_size(h)
                {
                    freed.insert(h.clone());
                    blob_bytes += len;
                }
            }
        }
        let mut agents: Vec<String> = rs.iter().map(|r| r.agent.clone()).collect();
        agents.sort();
        agents.dedup();
        sessions.push(SessionSummary {
            session,
            agents,
            actions: rs.len(),
            first_ms: rs.iter().map(|r| r.time_ms).min().unwrap_or(0),
            last_ms: rs.iter().map(|r| r.last_ms).max().unwrap_or(0),
        });
        ledger_bytes += rs.iter().map(|r| r.bytes).sum::<u64>();
        chosen.extend(rs);
    }
    let archived: BTreeSet<String> = chosen
        .iter()
        .flat_map(|r| r.refs.iter())
        .filter(|h| store.blobs.contains(h))
        .cloned()
        .collect();
    let mut actions: Vec<i64> = chosen.iter().map(|r| r.id).collect();
    actions.sort_unstable();
    Ok(Plan {
        shared_blobs: archived.len() - freed.len(),
        actions,
        sessions,
        freed_blobs: freed.into_iter().collect(),
        archived_blobs: archived.into_iter().collect(),
        blob_bytes,
        ledger_bytes,
        size_before,
        budget: sel.max_size,
        live_sessions: matched_live.into_iter().collect(),
        kept_with_command,
    })
}

/// carries out a plan. The store must have been opened with [`Store::open_exclusive`], so no
/// other process can point a new action at a blob while it is being removed.
///
/// the order makes every failure safe: the archive is finished and moved out first, so a
/// failure before the rows are deleted leaves the ledger as it was. The blob files go only
/// after the rows that named them are gone.
pub fn apply(store: &Store, plan: &Plan, disposal: &Disposal) -> Result<Outcome> {
    let mut out = Outcome {
        plan: plan.clone(),
        pruned: false,
        archive_kept_in: "none",
        archive_name: None,
        archive_path: None,
        size_after: None,
        busy: false,
    };
    if plan.is_empty() {
        return Ok(out);
    }
    let build_root = store.home().join(BUILD_DIR);
    if build_root.exists() {
        // an earlier prune stopped before moving its archive out, so it also stopped before
        // deleting anything: this folder only holds copies of rows still in the ledger
        fs::remove_dir_all(&build_root)
            .with_context(|| format!("removing {}", build_root.display()))?;
    }
    stage_ids(store, plan)?;
    if !matches!(disposal, Disposal::Permanent) {
        store::private_dir(&build_root)?;
        let name = archive_name(disposal, &build_root);
        let dir = build_root.join(&name);
        let built = write_archive(store, plan, &dir);
        let moved = built.and_then(|()| dispose(&dir, disposal));
        let _ = fs::remove_dir_all(&build_root);
        let place = moved?;
        out.archive_kept_in = match disposal {
            Disposal::Folder(_) => "folder",
            _ => "trash",
        };
        out.archive_path = place;
        out.archive_name = Some(name);
    }
    store.batch(|s| {
        s.conn()
            .execute_batch("DELETE FROM actions WHERE id IN (SELECT id FROM temp.prune_ids)")?;
        let mut del = s.conn().prepare("DELETE FROM file_cache WHERE hash = ?1")?;
        for h in &plan.freed_blobs {
            del.execute(params![h])?;
        }
        Ok(())
    })?;
    for h in &plan.freed_blobs {
        let p = store.blobs.path_for(h)?;
        if let Err(e) = fs::remove_file(&p)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("footprints: warning: could not remove {}: {e}", p.display());
        }
    }
    store
        .conn()
        .execute_batch("DROP TABLE IF EXISTS temp.prune_ids; VACUUM;")?;
    // VACUUM in WAL mode writes through the log, so the log is folded back and truncated to
    // give the space back to the disk straight away
    store
        .conn()
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    out.pruned = true;
    out.size_after = Some(data_size(store.home()));
    Ok(out)
}

fn stage_ids(store: &Store, plan: &Plan) -> Result<()> {
    store.conn().execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS prune_ids (id INTEGER PRIMARY KEY);
         DELETE FROM temp.prune_ids;",
    )?;
    store.batch(|s| {
        let mut ins = s
            .conn()
            .prepare("INSERT INTO temp.prune_ids (id) VALUES (?1)")?;
        for id in &plan.actions {
            ins.execute(params![id])?;
        }
        Ok(())
    })
}

const ARCHIVE_README: &str = "This folder holds actions removed from a Footprints ledger by \
`footprints prune`, with every stored file copy they refer to.

It is laid out like a Footprints data directory:
  ledger.db  the pruned rows, with their original ids
  blobs/     the file copies, compressed with zstd and named by their BLAKE3 hash

To put the actions back into the ledger:
  footprints import <path to this folder>

To read it without restoring (macOS and Linux):
  FOOTPRINTS_HOME=<path to this folder> footprints log --all

The copies may include secrets from files an agent touched, so keep this folder private.
";

fn write_archive(store: &Store, plan: &Plan, dir: &Path) -> Result<()> {
    store::private_dir(dir)?;
    let db = dir.join("ledger.db");
    {
        let c = Connection::open(&db)?;
        c.execute_batch(store::SCHEMA)?;
        c.pragma_update(None, "user_version", store::SCHEMA_VERSION)?;
    }
    crate::snapshot::set_mode(&db, Some(0o600))?;
    let cols = store::COLUMNS;
    store.conn().execute(
        "ATTACH DATABASE ?1 AS archive",
        params![db.to_string_lossy()],
    )?;
    let copied = store.conn().execute(
        &format!(
            "INSERT INTO archive.actions ({cols}) SELECT {cols} FROM main.actions
             WHERE id IN (SELECT id FROM temp.prune_ids) ORDER BY id"
        ),
        [],
    );
    let detached = store.conn().execute_batch("DETACH DATABASE archive");
    let copied = copied?;
    detached?;
    if copied != plan.actions.len() {
        bail!(
            "the archive holds {copied} actions but {} were planned; nothing was pruned",
            plan.actions.len()
        );
    }
    let blobs = dir.join("blobs");
    store::private_dir(&blobs)?;
    for h in &plan.archived_blobs {
        let from = store.blobs.path_for(h)?;
        let shard = blobs.join(&h[..2]);
        if !shard.exists() {
            store::private_dir(&shard)?;
        }
        let to = shard.join(&h[2..]);
        fs::copy(&from, &to).with_context(|| format!("copying blob {h} into the archive"))?;
    }
    fs::write(dir.join("README.txt"), ARCHIVE_README)?;
    Ok(())
}

/// `footprints-pruned-<date>-<n>` with the lowest `n` not already used where the archive goes.
fn archive_name(disposal: &Disposal, build_root: &Path) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d");
    let places: Vec<PathBuf> = match disposal {
        Disposal::Folder(d) => vec![d.clone()],
        _ => system_trash_dirs(),
    };
    (1..)
        .map(|n| format!("footprints-pruned-{date}-{n}"))
        .find(|name| {
            !build_root.join(name).exists() && places.iter().all(|p| !p.join(name).exists())
        })
        .expect("an unused name exists")
}

/// the user's own Trash folders where they can be seen, only to avoid reusing a name. The
/// Recycle Bin keeps its own names, so nothing is checked on Windows.
fn system_trash_dirs() -> Vec<PathBuf> {
    let Some(home) = crate::platform::home_dir() else {
        return Vec::new();
    };
    if cfg!(target_os = "macos") {
        vec![home.join(".Trash")]
    } else if cfg!(windows) {
        Vec::new()
    } else {
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        vec![data.join("Trash/files")]
    }
}

/// moves the finished archive out of the data directory. Returns its new path when it went
/// into a folder.
fn dispose(dir: &Path, disposal: &Disposal) -> Result<Option<PathBuf>> {
    match disposal {
        Disposal::Permanent => Ok(None),
        Disposal::Folder(target) => {
            fs::create_dir_all(target).with_context(|| format!("creating {}", target.display()))?;
            let dest = target.join(dir.file_name().expect("archive has a name"));
            if fs::rename(dir, &dest).is_err() {
                // another file system: copy, then the build folder is removed by the caller
                copy_dir(dir, &dest).with_context(|| {
                    format!(
                        "copying the archive to {}; nothing was pruned",
                        dest.display()
                    )
                })?;
            }
            Ok(Some(dest))
        }
        Disposal::Trash => {
            #[allow(unused_mut)]
            let mut ctx = trash::TrashContext::default();
            #[cfg(target_os = "macos")]
            {
                use trash::macos::{DeleteMethod, TrashContextExtMacos};
                // the Finder method asks for permission to control Finder and fails over SSH;
                // the file manager call needs neither
                ctx.set_delete_method(DeleteMethod::NsFileManager);
            }
            ctx.delete(dir).map_err(|e| {
                anyhow::Error::from(Refused(format!(
                    "the archive could not be moved to the {}: {e}. Nothing was pruned. Set \
                     {TRASH_DIR_VAR} to a folder to keep archives there instead. Run again \
                     with --permanent to delete without an archive",
                    trash_word()
                )))
            })?;
            Ok(None)
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    store::private_dir(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &dest)?;
        } else {
            fs::copy(e.path(), &dest)?;
        }
    }
    Ok(())
}

/// what the platform calls its Trash, for messages.
pub fn trash_word() -> &'static str {
    if cfg!(windows) {
        "Recycle Bin"
    } else {
        "Trash"
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Imported {
    pub actions: usize,
    pub sessions: usize,
    pub blobs_added: usize,
    /// blobs a restored action names that neither the archive nor the ledger holds
    pub blobs_missing: usize,
}

/// restores a pruned archive into the ledger, keeping every action's original id. Refuses
/// when any of those ids is already in the ledger, which means it was restored before.
pub fn import(store: &Store, archive: &Path) -> Result<Imported> {
    let db = archive.join("ledger.db");
    if !db.is_file() {
        bail!(
            "{} is not a pruned archive: it has no ledger.db",
            archive.display()
        );
    }
    if let (Ok(a), Ok(h)) = (
        crate::platform::canonical(archive),
        crate::platform::canonical(store.home()),
    ) && a == h
    {
        bail!("that folder is the data directory itself");
    }
    let src = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {}", db.display()))?;
    let version: i64 = src.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version != store::SCHEMA_VERSION {
        bail!(
            "the archive uses ledger schema {version} and this footprints uses {}, so it cannot \
             be imported by this version",
            store::SCHEMA_VERSION
        );
    }
    let mut stmt = src.prepare(&format!(
        "SELECT {} FROM actions ORDER BY id",
        store::COLUMNS
    ))?;
    let actions: Vec<Action> = stmt
        .query_map([], store::row_to_action)?
        .collect::<rusqlite::Result<_>>()
        .context("reading the archive")?;
    for a in &actions {
        if store.get(a.id)?.is_some() {
            return Err(Refused(format!(
                "action #{} is already in the ledger, so this archive looks restored already. \
                 Nothing was changed",
                a.id
            ))
            .into());
        }
    }
    // the blobs are checked against their hashes before anything is written
    let mut blobs = Vec::new();
    let blob_root = archive.join("blobs");
    if blob_root.is_dir() {
        for shard in fs::read_dir(&blob_root)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for e in fs::read_dir(shard.path())? {
                let e = e?;
                let hash = format!(
                    "{}{}",
                    shard.file_name().to_string_lossy(),
                    e.file_name().to_string_lossy()
                );
                if store.blobs.contains(&hash) {
                    continue;
                }
                let bytes = zstd::decode_all(&fs::read(e.path())?[..])
                    .with_context(|| format!("blob {hash} in the archive is damaged"))?;
                if crate::blob::hash_bytes(&bytes) != hash {
                    bail!("blob {hash} in the archive is corrupt; nothing was imported");
                }
                blobs.push(bytes);
            }
        }
    }
    let blobs_added = blobs.len();
    for b in blobs {
        store.blobs.put(&b)?;
    }
    store.batch(|s| {
        for a in &actions {
            s.insert_with_id(a)?;
        }
        Ok(())
    })?;
    let blobs_missing = actions
        .iter()
        .flat_map(|a| [&a.pre_hash, &a.post_hash, &a.manifest_hash])
        .flatten()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|h| !store.blobs.contains(h))
        .count();
    Ok(Imported {
        sessions: actions
            .iter()
            .map(|a| &a.session)
            .collect::<BTreeSet<_>>()
            .len(),
        actions: actions.len(),
        blobs_added,
        blobs_missing,
    })
}

/// the total size of every file under the data directory, without a half-built archive.
pub fn data_size(home: &Path) -> u64 {
    fn walk(dir: &Path, skip: &Path) -> u64 {
        let Ok(entries) = fs::read_dir(dir) else {
            return 0;
        };
        let mut n = 0;
        for e in entries.flatten() {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                if p != skip {
                    n += walk(&p, skip);
                }
            } else if t.is_file() {
                n += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        n
    }
    walk(home, &home.join(BUILD_DIR))
}

/// `30d`, `12h`, `2w`, `45m` or `90s`, as milliseconds.
pub fn parse_duration(s: &str) -> std::result::Result<i64, String> {
    let s = s.trim();
    let split = s
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| format!("{s:?} needs a unit: s, m, h, d or w (as in 30d)"))?;
    let (num, unit) = s.split_at(split);
    let n: i64 = num
        .parse()
        .map_err(|_| format!("{s:?} does not start with a whole number"))?;
    let unit_ms: i64 = match unit {
        "s" => 1000,
        "m" => 60 * 1000,
        "h" => 60 * 60 * 1000,
        "d" => 24 * 60 * 60 * 1000,
        "w" => 7 * 24 * 60 * 60 * 1000,
        _ => return Err(format!("unknown unit {unit:?}: use s, m, h, d or w")),
    };
    n.checked_mul(unit_ms)
        .filter(|ms| *ms > 0)
        .ok_or_else(|| format!("{s:?} is out of range"))
}

/// a local date (`2026-10-01`, meaning its midnight) or an RFC 3339 time, as milliseconds.
pub fn parse_date(s: &str) -> std::result::Result<i64, String> {
    use chrono::{Local, NaiveDate, TimeZone};
    let s = s.trim();
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(t.timestamp_millis());
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| format!("{s:?} is not a date: use YYYY-MM-DD or an RFC 3339 time"))?;
    let midnight = d.and_hms_opt(0, 0, 0).expect("midnight exists");
    Local
        .from_local_datetime(&midnight)
        .earliest()
        .map(|t| t.timestamp_millis())
        .ok_or_else(|| format!("{s:?} has no local midnight"))
}

/// `500MB`, `2GiB`, `750k` or a plain number of bytes. KB, MB, GB and TB are powers of 1000;
/// KiB, MiB, GiB and TiB are powers of 1024.
pub fn parse_size(s: &str) -> std::result::Result<u64, String> {
    let t = s.trim();
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num
        .parse()
        .map_err(|_| format!("{s:?} does not start with a number"))?;
    let mult: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1.0,
        "k" | "kb" => 1e3,
        "m" | "mb" => 1e6,
        "g" | "gb" => 1e9,
        "t" | "tb" => 1e12,
        "kib" => 1024.0,
        "mib" => 1024.0 * 1024.0,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        other => {
            return Err(format!(
                "unknown size unit {other:?}: use KB, MB, GB or KiB, MiB, GiB"
            ));
        }
    };
    let bytes = n * mult;
    if !bytes.is_finite() || bytes < 0.0 || bytes > u64::MAX as f64 {
        return Err(format!("{s:?} is out of range"));
    }
    Ok(bytes as u64)
}

/// a byte count in decimal units, matching how `--max-size` reads `MB` and `GB`.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// the plan and what was done, as plain sentences and one aligned table.
pub fn render(o: &Outcome) -> String {
    let p = &o.plan;
    let mut s = String::new();
    if p.is_empty() {
        match p.budget {
            Some(b) if p.size_before <= b => s.push_str(&format!(
                "Nothing to prune: the data folder is {}, within the {} budget.\n",
                human(p.size_before),
                human(b)
            )),
            _ => s.push_str("Nothing to prune: no action matches.\n"),
        }
        push_kept(&mut s, p);
        return s;
    }
    let verb = if o.pruned { "Pruned" } else { "Would prune" };
    s.push_str(&format!(
        "{verb} {} action(s) in {} session(s).\n\n",
        p.actions.len(),
        p.sessions.len()
    ));
    let headers = ["SESSION", "AGENT", "ACTIONS", "FIRST", "LAST"];
    let rows: Vec<[String; 5]> = p
        .sessions
        .iter()
        .take(MAX_SESSION_LINES)
        .map(|x| {
            [
                x.session.clone(),
                x.agents.join(", "),
                x.actions.to_string(),
                format_time(x.first_ms),
                format_time(x.last_ms),
            ]
        })
        .collect();
    let mut w = headers.map(str::len);
    for r in &rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(c.chars().count());
        }
    }
    let line = |cells: [&str; 5]| {
        let l = format!(
            "{:<a$}  {:<b$}  {:>c$}  {:<d$}  {}",
            cells[0],
            cells[1],
            cells[2],
            cells[3],
            cells[4],
            a = w[0],
            b = w[1],
            c = w[2],
            d = w[3]
        );
        format!("{}\n", l.trim_end())
    };
    s.push_str(&line(headers));
    for r in &rows {
        s.push_str(&line([&r[0], &r[1], &r[2], &r[3], &r[4]]));
    }
    if p.sessions.len() > MAX_SESSION_LINES {
        s.push_str(&format!(
            "and {} more session(s); --json lists them all\n",
            p.sessions.len() - MAX_SESSION_LINES
        ));
    }
    s.push('\n');
    s.push_str(&format!(
        "{} {} of stored file copies ({} blob(s)) and about {} of ledger.\n",
        if o.pruned { "Freed" } else { "Frees" },
        human(p.blob_bytes),
        p.freed_blobs.len(),
        human(p.ledger_bytes)
    ));
    if p.shared_blobs > 0 {
        s.push_str(&format!(
            "Kept {} blob(s) that remaining actions still use.\n",
            p.shared_blobs
        ));
    }
    match o.size_after {
        Some(after) => s.push_str(&format!(
            "Data folder: {} before, {} now.\n",
            human(p.size_before),
            human(after)
        )),
        None => s.push_str(&format!(
            "Data folder: {} now, about {} after.\n",
            human(p.size_before),
            human(p.size_before.saturating_sub(p.bytes_freed()))
        )),
    }
    push_kept(&mut s, p);
    if o.pruned {
        match (o.archive_kept_in, &o.archive_name, &o.archive_path) {
            ("folder", _, Some(path)) => s.push_str(&format!(
                "The pruned actions are archived in {}. Restore them with `footprints import {}`.\n",
                path.display(),
                path.display()
            )),
            ("trash", Some(name), _) => s.push_str(&format!(
                "The pruned actions are archived as {name} in the {}. To restore them, take the \
                 folder out of the {} and run `footprints import <folder>`.\n",
                trash_word(),
                trash_word()
            )),
            _ => s.push_str("No archive was kept (--permanent).\n"),
        }
    } else {
        if o.busy {
            s.push_str(
                "Another footprints process has the ledger open, so --yes would be refused until \
                 it stops.\n",
            );
        }
        s.push_str(&format!(
            "Nothing was changed. Run again with --yes to prune. The pruned actions then go to \
             the {} as one archive folder, unless --permanent is given.\n",
            trash_word()
        ));
    }
    s
}

fn push_kept(s: &mut String, p: &Plan) {
    if !p.live_sessions.is_empty() {
        s.push_str(&format!(
            "Kept {} session(s) still being recorded: {}. A killed run is recorded by \
             `footprints recover`.\n",
            p.live_sessions.len(),
            p.live_sessions.join(", ")
        ));
    }
    if p.kept_with_command > 0 {
        s.push_str(&format!(
            "Kept {} matching action(s) whose command or file changes do not all match, since a \
             command and its changes are pruned together.\n",
            p.kept_with_command
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90s").unwrap(), 90_000);
        assert_eq!(parse_duration("12h").unwrap(), 12 * 3_600_000);
        assert_eq!(parse_duration("30d").unwrap(), 30 * 86_400_000);
        assert_eq!(parse_duration("2w").unwrap(), 14 * 86_400_000);
        assert!(parse_duration("30").is_err());
        assert!(parse_duration("0d").is_err());
        assert!(parse_duration("3y").is_err());
        assert!(parse_duration("d").is_err());
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("500MB").unwrap(), 500_000_000);
        assert_eq!(parse_size("1.5 GB").unwrap(), 1_500_000_000);
        assert_eq!(parse_size("2GiB").unwrap(), 2 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("750k").unwrap(), 750_000);
        assert_eq!(parse_size("4096").unwrap(), 4096);
        assert!(parse_size("10 parsecs").is_err());
        assert!(parse_size("MB").is_err());
    }

    #[test]
    fn dates() {
        let t = parse_date("2026-01-02T03:04:05Z").unwrap();
        assert_eq!(t, 1_767_323_045_000);
        assert!(parse_date("2026-10-01").is_ok());
        assert!(parse_date("yesterday").is_err());
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(999), "999 B");
        assert_eq!(human(1_500), "1.5 KB");
        assert_eq!(human(3_200_000), "3.2 MB");
    }
}
