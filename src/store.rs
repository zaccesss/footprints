//! the ledger: one SQLite database of actions plus the blob store holding file images.

use crate::blob::BlobStore;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

const SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    FileWrite,
    FileDelete,
    FileCreate,
    Command,
    Network,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::FileWrite => "file_write",
            Kind::FileDelete => "file_delete",
            Kind::FileCreate => "file_create",
            Kind::Command => "command",
            Kind::Network => "network",
        }
    }

    pub fn is_file(self) -> bool {
        matches!(self, Kind::FileWrite | Kind::FileDelete | Kind::FileCreate)
    }

    /// picks the file kind from whether the file existed before and after.
    pub fn for_file(existed_before: bool, exists_after: bool) -> Kind {
        match (existed_before, exists_after) {
            (false, true) => Kind::FileCreate,
            (true, false) => Kind::FileDelete,
            _ => Kind::FileWrite,
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Kind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "file_write" => Kind::FileWrite,
            "file_delete" => Kind::FileDelete,
            "file_create" => Kind::FileCreate,
            "command" => Kind::Command,
            "network" => Kind::Network,
            other => bail!(
                "unknown kind {other:?}: expected file_write, file_delete, file_create, command or network"
            ),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// the pre-image is stored but the tool has not reported back yet
    Pending,
    Recorded,
    Undone,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Recorded => "recorded",
            Status::Undone => "undone",
        }
    }
}

impl FromStr for Status {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "pending" => Status::Pending,
            "recorded" => Status::Recorded,
            "undone" => Status::Undone,
            other => bail!("unknown status {other:?}"),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Action {
    pub id: i64,
    /// milliseconds since the Unix epoch
    pub time_ms: i64,
    pub end_time_ms: Option<i64>,
    pub agent: String,
    pub session: String,
    pub kind: Kind,
    /// a path for file actions, a command line for commands and a URL or query for network calls
    pub target: String,
    pub pre_hash: Option<String>,
    pub post_hash: Option<String>,
    /// set whenever the file existed before, even when its contents could not be stored
    pub pre_size: Option<i64>,
    /// set whenever the file existed after, even when its contents could not be stored
    pub post_size: Option<i64>,
    pub pre_mode: Option<u32>,
    pub intent: Option<String>,
    pub reversible: bool,
    pub status: Status,
    pub tool: Option<String>,
    pub tool_use_id: Option<String>,
    pub parent_id: Option<i64>,
    pub cwd: Option<String>,
    pub note: Option<String>,
    pub undone_by: Option<i64>,
    pub undo_of: Option<i64>,
    #[serde(skip)]
    pub manifest_hash: Option<String>,
    /// for a `footprints run` command that has not finished: where its pre-image copies live,
    /// so a run that was killed can still be recorded later
    #[serde(skip)]
    pub staging: Option<String>,
}

impl Action {
    /// a blank action with the given identity; callers fill in the rest before inserting.
    pub fn new(agent: &str, session: &str, kind: Kind, target: &str) -> Self {
        Action {
            id: 0,
            time_ms: now_ms(),
            end_time_ms: None,
            agent: agent.to_string(),
            session: session.to_string(),
            kind,
            target: target.to_string(),
            pre_hash: None,
            post_hash: None,
            pre_size: None,
            post_size: None,
            pre_mode: None,
            intent: None,
            reversible: false,
            status: Status::Recorded,
            tool: None,
            tool_use_id: None,
            parent_id: None,
            cwd: None,
            note: None,
            undone_by: None,
            undo_of: None,
            manifest_hash: None,
            staging: None,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub session: Option<String>,
    pub agent: Option<String>,
    pub kind: Option<Kind>,
    /// keep only the most recent N matching actions
    pub limit: Option<usize>,
}

/// identifies one version of a file on disk. The inode and ctime are included because a
/// rewrite can keep the size and put the old mtime back (`touch -r`, `cp -p`), but the kernel
/// always moves ctime forward and a user cannot set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheKey<'a> {
    pub path: &'a str,
    pub size: i64,
    pub mtime_ns: i64,
    pub ino: i64,
    pub ctime_ns: i64,
}

pub struct Store {
    conn: Connection,
    pub blobs: BlobStore,
    home: PathBuf,
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// the data directory: `FOOTPRINTS_HOME` or the platform default from [`crate::platform`].
pub fn default_home() -> Result<PathBuf> {
    crate::platform::default_home()
}

/// creates a directory that only the owner can read, since it holds copies of whatever files
/// an agent touched, secrets included.
pub fn private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("creating directory {}", path.display()))?;
    crate::snapshot::set_mode(path, Some(0o700))
}

const COLUMNS: &str = "id, time_ms, end_time_ms, agent, session, kind, target, pre_hash, post_hash, \
     pre_size, post_size, pre_mode, intent, reversible, status, tool, tool_use_id, parent_id, cwd, \
     note, undone_by, undo_of, manifest_hash, staging";

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS actions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        time_ms INTEGER NOT NULL,
        end_time_ms INTEGER,
        agent TEXT NOT NULL,
        session TEXT NOT NULL,
        kind TEXT NOT NULL,
        target TEXT NOT NULL,
        pre_hash TEXT,
        post_hash TEXT,
        pre_size INTEGER,
        post_size INTEGER,
        pre_mode INTEGER,
        intent TEXT,
        reversible INTEGER NOT NULL,
        status TEXT NOT NULL,
        tool TEXT,
        tool_use_id TEXT,
        parent_id INTEGER REFERENCES actions(id),
        cwd TEXT,
        note TEXT,
        undone_by INTEGER,
        undo_of INTEGER,
        manifest_hash TEXT,
        staging TEXT
    );
    CREATE INDEX IF NOT EXISTS actions_session ON actions(session);
    CREATE INDEX IF NOT EXISTS actions_agent ON actions(agent);
    CREATE INDEX IF NOT EXISTS actions_tool_use ON actions(tool_use_id);
    CREATE TABLE IF NOT EXISTS file_cache (
        path TEXT PRIMARY KEY,
        size INTEGER NOT NULL,
        mtime_ns INTEGER NOT NULL,
        ino INTEGER NOT NULL,
        ctime_ns INTEGER NOT NULL,
        hash TEXT NOT NULL
    );";

impl Store {
    pub fn open_default() -> Result<Self> {
        Self::open(&default_home()?)
    }

    pub fn open(home: &Path) -> Result<Self> {
        let db = home.join("ledger.db");
        let first_use = !db.exists();
        std::fs::create_dir_all(home)
            .with_context(|| format!("creating data directory {}", home.display()))?;
        if first_use {
            // narrowed when the ledger is first made there; a directory already in use is left
            // as its owner set it
            crate::snapshot::set_mode(home, Some(0o700))?;
        }
        let conn = Connection::open(&db)?;
        crate::snapshot::set_mode(&db, Some(0o600))?;
        // hooks for parallel tool calls can write at the same moment, so wait rather than fail
        conn.busy_timeout(Duration::from_secs(10))?;
        // journal_mode is persistent, so it is only written once and later hooks open without
        // taking a write lock
        let mode: String = conn.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            conn.pragma_update(None, "journal_mode", "WAL")?;
        }
        // NORMAL is durable against application crashes in WAL mode and avoids an fsync per write
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            bail!(
                "the ledger at {} was written by a newer footprints (schema {version}); upgrade to read it",
                home.display()
            );
        }
        if version != SCHEMA_VERSION {
            if version == 1 {
                // the cache is only an optimisation, so it is rebuilt with the new key rather
                // than migrated
                conn.execute_batch(
                    "ALTER TABLE actions ADD COLUMN staging TEXT;
                     DROP TABLE IF EXISTS file_cache;",
                )?;
            }
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        let blobs = BlobStore::open(home.join("blobs"))?;
        Ok(Store {
            conn,
            blobs,
            home: home.to_path_buf(),
        })
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn insert(&self, a: &Action) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO actions (time_ms, end_time_ms, agent, session, kind, target, pre_hash,
                post_hash, pre_size, post_size, pre_mode, intent, reversible, status, tool,
                tool_use_id, parent_id, cwd, note, undone_by, undo_of, manifest_hash, staging)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19, ?20, ?21, ?22, ?23)",
            params![
                a.time_ms,
                a.end_time_ms,
                a.agent,
                a.session,
                a.kind.as_str(),
                a.target,
                a.pre_hash,
                a.post_hash,
                a.pre_size,
                a.post_size,
                a.pre_mode,
                a.intent,
                a.reversible,
                a.status.as_str(),
                a.tool,
                a.tool_use_id,
                a.parent_id,
                a.cwd,
                a.note,
                a.undone_by,
                a.undo_of,
                a.manifest_hash,
                a.staging,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// writes every mutable field of an existing action back to the ledger.
    pub fn save(&self, a: &Action) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE actions SET end_time_ms = ?2, kind = ?3, target = ?4, pre_hash = ?5,
                post_hash = ?6, pre_size = ?7, post_size = ?8, pre_mode = ?9, intent = ?10,
                reversible = ?11, status = ?12, note = ?13, undone_by = ?14, manifest_hash = ?15,
                staging = ?16
             WHERE id = ?1",
            params![
                a.id,
                a.end_time_ms,
                a.kind.as_str(),
                a.target,
                a.pre_hash,
                a.post_hash,
                a.pre_size,
                a.post_size,
                a.pre_mode,
                a.intent,
                a.reversible,
                a.status.as_str(),
                a.note,
                a.undone_by,
                a.manifest_hash,
                a.staging,
            ],
        )?;
        if n == 0 {
            bail!("no action with id {}", a.id);
        }
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM actions WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn get(&self, id: i64) -> Result<Option<Action>> {
        let sql = format!("SELECT {COLUMNS} FROM actions WHERE id = ?1");
        Ok(self
            .conn
            .query_row(&sql, params![id], row_to_action)
            .optional()?)
    }

    pub fn require(&self, id: i64) -> Result<Action> {
        self.get(id)?
            .with_context(|| format!("no action with id {id}; run `footprints log` to list them"))
    }

    /// matching actions in time order, oldest first. A row that cannot be read is skipped with
    /// a warning naming its id, so one damaged entry never hides the rest of the timeline.
    pub fn list(&self, f: &Filter) -> Result<Vec<Action>> {
        let mut sql = format!("SELECT {COLUMNS} FROM actions WHERE 1 = 1");
        let mut args: Vec<String> = Vec::new();
        if let Some(s) = &f.session {
            args.push(s.clone());
            sql.push_str(&format!(" AND session = ?{}", args.len()));
        }
        if let Some(a) = &f.agent {
            args.push(a.clone());
            sql.push_str(&format!(" AND agent = ?{}", args.len()));
        }
        if let Some(k) = f.kind {
            args.push(k.as_str().to_string());
            sql.push_str(&format!(" AND kind = ?{}", args.len()));
        }
        sql.push_str(" ORDER BY id DESC");
        if let Some(n) = f.limit {
            sql.push_str(&format!(" LIMIT {n}"));
        }
        let mut out = self.query_lenient(&sql, rusqlite::params_from_iter(args.iter()))?;
        out.reverse();
        Ok(out)
    }

    pub fn children(&self, parent: i64) -> Result<Vec<Action>> {
        let sql = format!("SELECT {COLUMNS} FROM actions WHERE parent_id = ?1 ORDER BY id");
        self.query_lenient(&sql, params![parent])
    }

    /// `footprints run` commands that never finished and still have a staging area.
    pub fn unfinished_runs(&self) -> Result<Vec<Action>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM actions WHERE status = 'pending' AND staging IS NOT NULL
             ORDER BY id"
        );
        self.query_lenient(&sql, [])
    }

    /// actions that finished at or after `since_ms`, plus pending ones that are recent or
    /// belong to a running wrapper. The daemon uses these to tell whether a change it saw was
    /// already recorded by a hook, the wrapper or an undo.
    pub fn recent(&self, since_ms: i64, pending_since_ms: i64) -> Result<Vec<Action>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM actions WHERE COALESCE(end_time_ms, time_ms) >= ?1
             OR (status = 'pending' AND (time_ms >= ?2 OR staging IS NOT NULL)) ORDER BY id"
        );
        self.query_lenient(&sql, params![since_ms, pending_since_ms])
    }

    /// daemon sessions left pending by a daemon that stopped without shutting down cleanly.
    pub fn pending_daemons(&self) -> Result<Vec<Action>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM actions WHERE status = 'pending' AND tool = 'daemon'
             AND parent_id IS NULL ORDER BY id"
        );
        self.query_lenient(&sql, [])
    }

    fn query_lenient(&self, sql: &str, args: impl rusqlite::Params) -> Result<Vec<Action>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(args, |r| {
            Ok(match row_to_action(r) {
                Ok(a) => Some(a),
                Err(e) => {
                    let id: i64 = r.get(0).unwrap_or(-1);
                    eprintln!("footprints: warning: skipping unreadable action #{id}: {e}");
                    None
                }
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            if let Some(a) = r? {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// finds the pending action a post hook belongs to. With a tool use id the tool and target
    /// must match too, so a reused or forged id cannot attach a post-image to another file. A
    /// row with a different tool use id belongs to another call, often a denied one. It is never
    /// adopted, since its old pre-image would let an undo erase later edits.
    pub fn find_pending(
        &self,
        session: Option<&str>,
        tool_use_id: Option<&str>,
        tool: &str,
        target: Option<&str>,
    ) -> Result<Option<Action>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM actions WHERE status = 'pending' AND parent_id IS NULL
             AND tool = ?1 AND (?2 IS NULL OR {t2}) AND (?3 IS NULL OR session = ?3)
             AND ((?4 IS NOT NULL AND tool_use_id = ?4) OR (?4 IS NULL AND tool_use_id IS NULL))
             ORDER BY id DESC LIMIT 1",
            t2 = target_eq(2)
        );
        let found = self
            .conn
            .query_row(
                &sql,
                params![tool, target, session, tool_use_id],
                row_to_action,
            )
            .optional()?;
        if found.is_some() || tool_use_id.is_none() {
            return Ok(found);
        }
        // a pre hook without an id paired with a post hook that has one
        let sql = format!(
            "SELECT {COLUMNS} FROM actions WHERE status = 'pending' AND parent_id IS NULL
             AND tool = ?1 AND (?2 IS NULL OR {t2}) AND (?3 IS NULL OR session = ?3)
             AND tool_use_id IS NULL ORDER BY id DESC LIMIT 1",
            t2 = target_eq(2)
        );
        Ok(self
            .conn
            .query_row(&sql, params![tool, target, session], row_to_action)
            .optional()?)
    }

    /// the newest action of any kind that names this target.
    pub fn latest_for_target(&self, target: &str) -> Result<Option<Action>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM actions WHERE {} ORDER BY id DESC LIMIT 1",
            target_eq(1)
        );
        Ok(self
            .conn
            .query_row(&sql, params![target], row_to_action)
            .optional()?)
    }

    /// true when any action newer than `id` names the same target.
    pub fn newer_for_target(&self, target: &str, id: i64) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM actions WHERE {} AND id > ?2",
                target_eq(1)
            ),
            params![target, id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn cache_get(&self, k: CacheKey<'_>) -> Result<Option<String>> {
        let hash: Option<String> = self
            .conn
            .query_row(
                "SELECT hash FROM file_cache WHERE path = ?1 AND size = ?2 AND mtime_ns = ?3
                 AND ino = ?4 AND ctime_ns = ?5",
                params![k.path, k.size, k.mtime_ns, k.ino, k.ctime_ns],
                |r| r.get(0),
            )
            .optional()?;
        // a cache entry is only useful while its blob is still on disk
        Ok(hash.filter(|h| self.blobs.contains(h)))
    }

    pub fn cache_put(&self, k: CacheKey<'_>, hash: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO file_cache (path, size, mtime_ns, ino, ctime_ns, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET size = ?2, mtime_ns = ?3, ino = ?4, ctime_ns = ?5,
                hash = ?6",
            params![k.path, k.size, k.mtime_ns, k.ino, k.ctime_ns, hash],
        )?;
        Ok(())
    }

    /// runs `f` inside one transaction, which turns thousands of cache writes into one commit.
    pub fn batch<T>(&self, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// for tests that need a damaged row.
    #[doc(hidden)]
    pub fn execute_raw(&self, sql: &str) -> Result<()> {
        self.conn.execute_batch(sql)?;
        Ok(())
    }
}

/// an SQL test that the `target` column names the same path as parameter `n`. Windows file
/// systems ignore case, so there the comparison folds ASCII case; the stored target keeps its
/// spelling.
fn target_eq(n: usize) -> String {
    if cfg!(windows) {
        format!("lower(target) = lower(?{n})")
    } else {
        format!("target = ?{n}")
    }
}

fn row_to_action(r: &Row<'_>) -> rusqlite::Result<Action> {
    let kind: String = r.get(5)?;
    let status: String = r.get(14)?;
    let conv = |e: anyhow::Error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    };
    Ok(Action {
        id: r.get(0)?,
        time_ms: r.get(1)?,
        end_time_ms: r.get(2)?,
        agent: r.get(3)?,
        session: r.get(4)?,
        kind: kind.parse().map_err(conv)?,
        target: r.get(6)?,
        pre_hash: r.get(7)?,
        post_hash: r.get(8)?,
        pre_size: r.get(9)?,
        post_size: r.get(10)?,
        pre_mode: r.get(11)?,
        intent: r.get(12)?,
        reversible: r.get(13)?,
        status: status.parse().map_err(conv)?,
        tool: r.get(15)?,
        tool_use_id: r.get(16)?,
        parent_id: r.get(17)?,
        cwd: r.get(18)?,
        note: r.get(19)?,
        undone_by: r.get(20)?,
        undo_of: r.get(21)?,
        manifest_hash: r.get(22)?,
        staging: r.get(23)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("home")).unwrap();
        (dir, store)
    }

    fn key(path: &str, mtime_ns: i64, ctime_ns: i64) -> CacheKey<'_> {
        CacheKey {
            path,
            size: 1,
            mtime_ns,
            ino: 7,
            ctime_ns,
        }
    }

    #[test]
    fn insert_get_and_save() {
        let (_d, store) = temp_store();
        let mut a = Action::new("tester", "s1", Kind::FileWrite, "/tmp/x");
        a.pre_hash = Some(store.blobs.put(b"before").unwrap());
        a.reversible = true;
        a.status = Status::Pending;
        let id = store.insert(&a).unwrap();
        let mut got = store.require(id).unwrap();
        assert_eq!(got.agent, "tester");
        assert_eq!(got.status, Status::Pending);
        got.post_hash = Some(store.blobs.put(b"after").unwrap());
        got.status = Status::Recorded;
        store.save(&got).unwrap();
        let again = store.require(id).unwrap();
        assert_eq!(again.status, Status::Recorded);
        assert_eq!(again.post_hash, got.post_hash);
    }

    #[test]
    fn list_filters_and_limits() {
        let (_d, store) = temp_store();
        for (agent, session) in [("a", "s1"), ("b", "s1"), ("a", "s2"), ("a", "s2")] {
            store
                .insert(&Action::new(agent, session, Kind::Command, "echo hi"))
                .unwrap();
        }
        let all = store.list(&Filter::default()).unwrap();
        assert_eq!(all.len(), 4);
        assert!(all[0].id < all[3].id, "oldest first");
        let s2 = store
            .list(&Filter {
                session: Some("s2".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(s2.len(), 2);
        let a = store
            .list(&Filter {
                agent: Some("a".into()),
                limit: Some(2),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[1].id, all[3].id, "limit keeps the newest");
    }

    #[test]
    fn a_damaged_row_is_skipped_not_fatal() {
        let (_d, store) = temp_store();
        store
            .insert(&Action::new("a", "s", Kind::Command, "one"))
            .unwrap();
        let bad = store
            .insert(&Action::new("a", "s", Kind::Command, "two"))
            .unwrap();
        store
            .insert(&Action::new("a", "s", Kind::Command, "three"))
            .unwrap();
        store
            .execute_raw(&format!(
                "UPDATE actions SET kind = 'garbage' WHERE id = {bad}"
            ))
            .unwrap();
        let all = store.list(&Filter::default()).unwrap();
        let targets: Vec<_> = all.iter().map(|a| a.target.as_str()).collect();
        assert_eq!(targets, ["one", "three"]);
    }

    #[test]
    fn same_content_in_many_actions_is_stored_once() {
        let (_d, store) = temp_store();
        for i in 0..5 {
            let mut a = Action::new("t", "s", Kind::FileWrite, &format!("/f{i}"));
            a.pre_hash = Some(store.blobs.put(b"shared content").unwrap());
            store.insert(&a).unwrap();
        }
        assert_eq!(store.blobs.count().unwrap(), 1);
    }

    #[test]
    fn find_pending_needs_id_tool_and_target_to_agree() {
        let (_d, store) = temp_store();
        let mut a = Action::new("t", "s", Kind::FileWrite, "/f");
        a.status = Status::Pending;
        a.tool = Some("Write".into());
        a.tool_use_id = Some("tu_1".into());
        let first = store.insert(&a).unwrap();
        a.tool_use_id = Some("tu_2".into());
        store.insert(&a).unwrap();
        let found = store
            .find_pending(Some("s"), Some("tu_1"), "Write", Some("/f"))
            .unwrap()
            .unwrap();
        assert_eq!(found.id, first);
        // the same id on another file is not the same call
        assert!(
            store
                .find_pending(Some("s"), Some("tu_1"), "Write", Some("/other"))
                .unwrap()
                .is_none()
        );
        // nor is a different session
        assert!(
            store
                .find_pending(Some("s9"), Some("tu_1"), "Write", Some("/f"))
                .unwrap()
                .is_none()
        );
        // and a row with another id is never adopted
        assert!(
            store
                .find_pending(Some("s"), Some("tu_3"), "Write", Some("/f"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn file_cache_needs_the_whole_key() {
        let (_d, store) = temp_store();
        let h = store.blobs.put(b"x").unwrap();
        store.cache_put(key("/p", 42, 100), &h).unwrap();
        assert_eq!(store.cache_get(key("/p", 42, 100)).unwrap(), Some(h));
        assert_eq!(store.cache_get(key("/p", 43, 100)).unwrap(), None);
        // same size and mtime but a newer ctime is a different file version
        assert_eq!(store.cache_get(key("/p", 42, 101)).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn data_directory_and_ledger_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let (d, store) = temp_store();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(store.home()), 0o700);
        assert_eq!(mode(&d.path().join("home/blobs")), 0o700);
        assert_eq!(mode(&d.path().join("home/ledger.db")), 0o600);
    }

    #[test]
    fn kind_parsing() {
        for k in [
            Kind::FileWrite,
            Kind::FileDelete,
            Kind::FileCreate,
            Kind::Command,
            Kind::Network,
        ] {
            assert_eq!(k.as_str().parse::<Kind>().unwrap(), k);
        }
        assert!("nope".parse::<Kind>().is_err());
    }
}
