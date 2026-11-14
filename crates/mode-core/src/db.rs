//! SQLite open, pragmas, backup-before-migrate, embedded migrations, the
//! single-writer channel and the reader pool (09 §2, §12; 08 §2.3).
//!
//! `open()` creates the file (mode 0600 on unix), applies the pragmas of 09 §2,
//! verifies the migration ledger, backs the database up with `VACUUM INTO`
//! when migrations are pending, runs them transactionally (`PRAGMA
//! foreign_key_check` before commit), mints the device id once, re-applies the
//! premade seeds, and hands back a [`Db`]: one writer connection owned by a
//! dedicated thread fed through a tokio mpsc channel, plus a small pool of
//! read-only connections. Readers never wait on the writer (WAL).

pub mod seeds;

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

/// One embedded migration file from `migrations/`.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    /// `ddl` or `data` (09 §12).
    pub kind: &'static str,
    pub sql: &'static str,
}

impl Migration {
    /// sha256 of the migration text as embedded in the binary (before token substitution).
    pub fn checksum(&self) -> String {
        let digest = Sha256::digest(self.sql.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Every migration, in order. Adding a file to `migrations/` means adding a row here.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        kind: "ddl",
        sql: include_str!("../../../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "avatar_variant",
        kind: "ddl",
        sql: include_str!("../../../migrations/0002_avatar_variant.sql"),
    },
    Migration {
        version: 3,
        name: "stats",
        kind: "ddl",
        sql: include_str!("../../../migrations/0003_stats.sql"),
    },
    Migration {
        version: 4,
        name: "restart",
        kind: "ddl",
        sql: include_str!("../../../migrations/0004_restart.sql"),
    },
    Migration {
        version: 5,
        name: "agents",
        kind: "ddl",
        sql: include_str!("../../../migrations/0005_agents.sql"),
    },
    Migration {
        version: 6,
        name: "providers",
        kind: "ddl",
        sql: include_str!("../../../migrations/0006_providers.sql"),
    },
    Migration {
        version: 7,
        name: "agent_scope",
        kind: "ddl",
        sql: include_str!("../../../migrations/0007_agent_scope.sql"),
    },
    Migration {
        version: 8,
        name: "link_dismissals",
        kind: "ddl",
        sql: include_str!("../../../migrations/0008_link_dismissals.sql"),
    },
    Migration {
        version: 9,
        name: "media_plays",
        kind: "ddl",
        sql: include_str!("../../../migrations/0009_media_plays.sql"),
    },
];

/// Pre-migration copies kept in `<app_data_dir>/backups/` (09 §2, §10).
pub const BACKUPS_KEPT: usize = 5;

/// Read-only connections in the pool.
pub const DEFAULT_READERS: usize = 3;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "database schema version {found} is newer than this build supports ({supported}); update Mode"
    )]
    TooNew { found: u32, supported: u32 },
    #[error(
        "migration {version} was applied with checksum {applied} but this build embeds {embedded}"
    )]
    ChecksumMismatch {
        version: u32,
        applied: String,
        embedded: String,
    },
    #[error(
        "migration ledger is out of step: user_version={user_version} but migration {version} is not recorded"
    )]
    MissingLedgerRow { user_version: u32, version: u32 },
    #[error("database check failed: {0}")]
    Corrupt(String),
    #[error("foreign key check failed after migration {version}: {violations} violation(s)")]
    ForeignKeys { version: u32, violations: usize },
    #[error("the database is open read-only (pre-migration backup failed: {0})")]
    ReadOnly(String),
    #[error("the database writer has shut down")]
    Closed,
    #[error("a database job panicked")]
    Panicked,
    #[error("{table} row {key} not found")]
    NotFound { table: &'static str, key: String },
    #[error("invalid {what}: {detail}")]
    Invalid { what: &'static str, detail: String },
}

pub type Result<T> = std::result::Result<T, DbError>;

/// A write job for the writer thread.
type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// Options for [`open_with`].
#[derive(Debug, Clone)]
pub struct Options {
    /// Read-only connections in the pool (at least 1).
    pub readers: usize,
    /// Where pre-migration backups go; default `<db dir>/backups`.
    pub backups_dir: Option<PathBuf>,
    /// The migration list to apply; default [`MIGRATIONS`] (tests append fixtures).
    pub migrations: Option<&'static [Migration]>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            readers: DEFAULT_READERS,
            backups_dir: None,
            migrations: None,
        }
    }
}

/// An open, migrated database: the writer behind its channel and the reader pool.
/// `Send + Sync`; clone-free (hand out `&Db` or wrap in an `Arc`).
pub struct Db {
    writer: Option<mpsc::UnboundedSender<Job>>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
    readers: Arc<ReaderPool>,
    path: PathBuf,
    backups_dir: PathBuf,
    device_id: String,
    read_only: Option<String>,
    version: u32,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field("path", &self.path)
            .field("version", &self.version)
            .field("device_id", &self.device_id)
            .field("read_only", &self.read_only.is_some())
            .finish_non_exhaustive()
    }
}

/// Open (creating if needed) the database at `path` with default options.
pub fn open(path: &Path) -> Result<Db> {
    open_with(path, Options::default())
}

/// Open (creating if needed) the database at `path`, apply the pragmas of
/// 09 §2, back up and run pending migrations, start the writer and the readers.
pub fn open_with(path: &Path, options: Options) -> Result<Db> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let backups_dir = options.backups_dir.clone().unwrap_or_else(|| {
        path.parent()
            .map(|p| p.join("backups"))
            .unwrap_or_else(|| PathBuf::from("backups"))
    });

    let conn = Connection::open(path)?;
    restrict_permissions(path);
    if is_empty_schema(&conn)? {
        // Must precede the first table (09 §10: the file never grows after retention).
        conn.pragma_update(None, "auto_vacuum", 2)?;
    }
    apply_pragmas(&conn)?;

    let migrations = options.migrations.unwrap_or(MIGRATIONS);
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let supported = migrations.last().map(|m| m.version).unwrap_or(0);
    if current > supported {
        return Err(DbError::TooNew {
            found: current,
            supported,
        });
    }
    verify_ledger(&conn, migrations, current)?;

    let device_id = match stored_device_id(&conn)? {
        Some(id) => id,
        None => uuid::Uuid::now_v7().to_string(),
    };
    let ctx = seeds::SeedContext::detect(device_id.clone());

    let pending: Vec<&Migration> = migrations.iter().filter(|m| m.version > current).collect();
    let mut read_only = None;
    if let Some(first) = pending.first() {
        if current > 0 {
            match backup_before_migration(&conn, &backups_dir, first.version) {
                Ok(file) => tracing::info!(file = %file.display(), "pre-migration backup written"),
                Err(e) => {
                    tracing::warn!(error = %e, "pre-migration backup failed; opening read-only");
                    read_only = Some(e.to_string());
                }
            }
        }
        if read_only.is_none() {
            migrate(&conn, migrations, &ctx)?;
        }
    }

    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if read_only.is_none() {
        ensure_device_id(&conn, &device_id)?;
        seeds::refresh(&conn, &ctx)?;
    }
    if cfg!(debug_assertions) {
        let s = integrity_check(&conn)?;
        if s != "ok" {
            return Err(DbError::Corrupt(s));
        }
    }

    let readers = Arc::new(ReaderPool::open(path, options.readers.max(1))?);
    let (tx, rx) = mpsc::unbounded_channel::<Job>();
    let writer_thread = std::thread::Builder::new()
        .name("mode-db-writer".into())
        .spawn(move || writer_loop(conn, rx))?;

    tracing::info!(path = %path.display(), user_version = version, read_only = read_only.is_some(), "database open");
    Ok(Db {
        writer: Some(tx),
        writer_thread: Some(writer_thread),
        readers,
        path: path.to_path_buf(),
        backups_dir,
        device_id,
        read_only,
        version,
    })
}

fn writer_loop(mut conn: Connection, mut rx: mpsc::UnboundedReceiver<Job>) {
    while let Some(job) = rx.blocking_recv() {
        job(&mut conn);
    }
    let _ = conn.execute_batch("PRAGMA optimize;");
}

/// The highest migration version this build knows.
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map(|m| m.version).unwrap_or(0)
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

fn is_empty_schema(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r.get(0))?;
    Ok(n == 0)
}

/// The connection pragmas of 09 §2.
pub fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

fn stored_device_id(conn: &Connection) -> Result<Option<String>> {
    let has_settings: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'settings'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !has_settings {
        return Ok(None);
    }
    let raw: Option<String> = conn
        .query_row(
            "SELECT value_json FROM settings WHERE key = 'device.id'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_str().map(str::to_string)))
}

fn ensure_device_id(conn: &Connection, device_id: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value_json, scope, updated_at, rev, device_id)
         VALUES ('device.id', ?1, 'device', ?2, 1, ?3)",
        rusqlite::params![
            serde_json::Value::String(device_id.to_string()).to_string(),
            now_rfc3339(),
            device_id
        ],
    )?;
    Ok(())
}

/// Already-applied migrations must be ledgered with the embedded checksum.
fn verify_ledger(conn: &Connection, migrations: &[Migration], current: u32) -> Result<()> {
    let has_ledger: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    for m in migrations.iter().filter(|m| m.version <= current) {
        if !has_ledger {
            return Err(DbError::MissingLedgerRow {
                user_version: current,
                version: m.version,
            });
        }
        let applied: Option<String> = conn
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE version = ?1",
                [m.version],
                |r| r.get(0),
            )
            .optional()?;
        match applied {
            Some(applied) if applied == m.checksum() => {}
            Some(applied) => {
                return Err(DbError::ChecksumMismatch {
                    version: m.version,
                    applied,
                    embedded: m.checksum(),
                });
            }
            None => {
                return Err(DbError::MissingLedgerRow {
                    user_version: current,
                    version: m.version,
                });
            }
        }
    }
    Ok(())
}

/// Apply every migration above the current `user_version`: each one, its ledger
/// row and the `user_version` bump in one transaction, with `foreign_keys` off
/// for the duration of DDL and `PRAGMA foreign_key_check` before commit (09 §12).
/// Returns the versions applied.
pub fn migrate(
    conn: &Connection,
    migrations: &[Migration],
    ctx: &seeds::SeedContext,
) -> Result<Vec<u32>> {
    let current: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let mut applied = Vec::new();
    for m in migrations.iter().filter(|m| m.version > current) {
        let sql = ctx.render(m.sql);
        if m.kind == "ddl" {
            conn.pragma_update(None, "foreign_keys", "OFF")?;
        }
        let result = (|| -> Result<()> {
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(&sql)?;
            tx.execute(
                "INSERT INTO schema_migrations (version, name, kind, checksum, app_version, applied_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    m.version,
                    m.name,
                    m.kind,
                    m.checksum(),
                    crate::VERSION,
                    now_rfc3339()
                ],
            )?;
            tx.pragma_update(None, "user_version", m.version)?;
            let violations = foreign_key_violations(&tx)?;
            if violations > 0 {
                return Err(DbError::ForeignKeys {
                    version: m.version,
                    violations,
                });
            }
            tx.commit()?;
            Ok(())
        })();
        if m.kind == "ddl" {
            conn.pragma_update(None, "foreign_keys", "ON")?;
        }
        result?;
        applied.push(m.version);
        tracing::info!(version = m.version, name = m.name, "migration applied");
    }
    Ok(applied)
}

/// `PRAGMA quick_check`, then `VACUUM INTO '<backups>/mode-pre-<NNNN>-<stamp>.db'`,
/// then prune to [`BACKUPS_KEPT`]. Returns the file written.
pub fn backup_before_migration(
    conn: &Connection,
    backups_dir: &Path,
    next_version: u32,
) -> Result<PathBuf> {
    let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(DbError::Corrupt(check));
    }
    std::fs::create_dir_all(backups_dir)?;
    let stamp: String = now_rfc3339()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let file = backups_dir.join(format!("mode-pre-{next_version:04}-{stamp}.db"));
    let target = file.to_str().ok_or_else(|| DbError::Invalid {
        what: "backup path",
        detail: "not valid UTF-8".into(),
    })?;
    conn.execute("VACUUM INTO ?1", [target])?;
    prune_backups(backups_dir, BACKUPS_KEPT)?;
    Ok(file)
}

/// Keep the newest `keep` `mode-pre-*.db` files (by name: version, then UTC stamp).
/// Returns the files removed.
pub fn prune_backups(backups_dir: &Path, keep: usize) -> Result<Vec<PathBuf>> {
    let mut names: Vec<PathBuf> = match std::fs::read_dir(backups_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("mode-pre-") && n.ends_with(".db"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    names.sort();
    let mut removed = Vec::new();
    while names.len() > keep {
        let oldest = names.remove(0);
        // A copy the user has open elsewhere (Windows refuses to delete an
        // open file) is skipped and tried again on the next prune.
        match std::fs::remove_file(&oldest) {
            Ok(()) => removed.push(oldest),
            Err(e) => tracing::warn!(path = %oldest.display(), error = %e, "backup not pruned"),
        }
    }
    Ok(removed)
}

/// Prefix of the "Backup now" copies (03 §8.16), next to the pre-migration ones.
pub const MANUAL_BACKUP_PREFIX: &str = "mode-backup-";

/// "Backup now" (03 §8.16): `PRAGMA quick_check`, then `VACUUM INTO
/// '<backups>/mode-backup-<stamp>.db'`, keeping the newest [`BACKUPS_KEPT`]
/// of these (the pre-migration copies are pruned on their own). Returns the
/// file written.
pub fn backup_now(conn: &Connection, backups_dir: &Path) -> Result<PathBuf> {
    let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(DbError::Corrupt(check));
    }
    std::fs::create_dir_all(backups_dir)?;
    let stamp: String = now_rfc3339()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let mut file = backups_dir.join(format!("{MANUAL_BACKUP_PREFIX}{stamp}.db"));
    let mut n = 1;
    while file.exists() {
        n += 1;
        file = backups_dir.join(format!("{MANUAL_BACKUP_PREFIX}{stamp}-{n}.db"));
    }
    let target = file.to_str().ok_or_else(|| DbError::Invalid {
        what: "backup path",
        detail: "not valid UTF-8".into(),
    })?;
    conn.execute("VACUUM INTO ?1", [target])?;
    restrict_permissions(&file);
    prune_prefixed(backups_dir, MANUAL_BACKUP_PREFIX, BACKUPS_KEPT)?;
    Ok(file)
}

fn prune_prefixed(dir: &Path, prefix: &str, keep: usize) -> Result<Vec<PathBuf>> {
    let mut names: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".db"))
            })
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    names.sort();
    let mut removed = Vec::new();
    while names.len() > keep {
        let oldest = names.remove(0);
        // A copy the user has open elsewhere (Windows refuses to delete an
        // open file) is skipped and tried again on the next prune.
        match std::fs::remove_file(&oldest) {
            Ok(()) => removed.push(oldest),
            Err(e) => tracing::warn!(path = %oldest.display(), error = %e, "backup not pruned"),
        }
    }
    Ok(removed)
}

/// One database copy under `backups/` (Diagnostics → Database).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BackupFile {
    pub path: PathBuf,
    pub size_bytes: u64,
    /// The file's modification time, RFC 3339 UTC.
    pub created_at: String,
}

/// The `mode-*.db` copies in `backups_dir` (pre-migration and "Backup
/// now"), newest first.
pub fn list_backups(backups_dir: &Path) -> Result<Vec<BackupFile>> {
    let rd = match std::fs::read_dir(backups_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out: Vec<BackupFile> = rd
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("mode-") && n.ends_with(".db"))
        })
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let created_at = meta
                .modified()
                .ok()
                .map(|t| format_timestamp(time::OffsetDateTime::from(t)))
                .unwrap_or_default();
            Some(BackupFile {
                path: e.path(),
                size_bytes: meta.len(),
                created_at,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.path.cmp(&a.path))
    });
    Ok(out)
}

/// Row counts of every ordinary table (the FTS shadow tables and SQLite's
/// own excluded), by name.
pub fn table_counts(conn: &Connection) -> Result<Vec<(String, i64)>> {
    let mut out = Vec::new();
    for name in user_tables(conn)? {
        let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM \"{name}\""), [], |r| {
            r.get(0)
        })?;
        out.push((name, n));
    }
    Ok(out)
}

/// The ordinary tables: not `sqlite_*`, not virtual (FTS5) and not an FTS
/// shadow table.
fn user_tables(conn: &Connection) -> Result<Vec<String>> {
    let virtuals = virtual_tables(conn)?;
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(names
        .into_iter()
        .filter(|n| {
            !virtuals
                .iter()
                .any(|v| n == v || n.starts_with(&format!("{v}_")))
        })
        .collect())
}

fn virtual_tables(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND sql LIKE 'CREATE VIRTUAL TABLE%'",
    )?;
    let names = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(names)
}

/// What [`Db::reset_contents`] removed.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ResetReport {
    /// Rows deleted per table (tables that were already empty are left out).
    pub deleted: std::collections::BTreeMap<String, i64>,
}

/// Empty every table but the migration ledger, keep the device id, and
/// re-insert the premade seeds: the database a fresh install would have,
/// in place (the writer and the readers stay open; "Delete all Mode data",
/// 03 §8.15). Runs on the writer connection, outside any transaction.
pub fn reset_contents_in(conn: &mut Connection, device_id: &str) -> Result<ResetReport> {
    let mut report = ResetReport::default();
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = (|| -> Result<()> {
        let tx = conn.transaction()?;
        let tables: Vec<String> = user_tables(&tx)?
            .into_iter()
            .filter(|t| t != "schema_migrations")
            .collect();
        // `change_log` last: the delete triggers of the sync tables log into it.
        let (mut first, last): (Vec<String>, Vec<String>) =
            tables.into_iter().partition(|t| t != "change_log");
        first.extend(last);
        for t in &first {
            let n = tx.execute(&format!("DELETE FROM \"{t}\""), [])? as i64;
            if n > 0 {
                report.deleted.insert(t.clone(), n);
            }
        }
        for v in virtual_tables(&tx)? {
            tx.execute(
                &format!("INSERT INTO \"{v}\"(\"{v}\") VALUES('delete-all')"),
                [],
            )?;
        }
        ensure_device_id(&tx, device_id)?;
        seeds::refresh(&tx, &seeds::SeedContext::detect(device_id.to_string()))?;
        tx.commit()?;
        Ok(())
    })();
    conn.pragma_update(None, "foreign_keys", true)?;
    result?;
    conn.execute_batch("PRAGMA incremental_vacuum; PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(report)
}

/// `PRAGMA integrity_check` as one string (`"ok"` when clean).
pub fn integrity_check(conn: &Connection) -> Result<String> {
    let mut stmt = conn.prepare("PRAGMA integrity_check")?;
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows.join("\n"))
}

/// Rows reported by `PRAGMA foreign_key_check`.
pub fn foreign_key_violations(conn: &Connection) -> Result<usize> {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let n = stmt.query_map([], |_| Ok(()))?.count();
    Ok(n)
}

/// Current UTC time as ISO-8601 with millisecond precision and `Z`, 24 chars
/// (the `TEXT` timestamp convention of 09 §4.1).
pub fn now_rfc3339() -> String {
    format_timestamp(time::OffsetDateTime::now_utc())
}

/// Format an instant the 09 §4.1 way: `2026-10-18T20:17:03.123Z`.
pub fn format_timestamp(t: time::OffsetDateTime) -> String {
    let t = t.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.millisecond()
    )
}

/// Parse a 09 §4.1 timestamp (`2026-10-18T20:17:03.123Z`; seconds and
/// milliseconds optional, `Z` or no offset). The `time` crate's parser is
/// not compiled in; this is the one format Mode writes and reads back.
pub fn parse_timestamp(s: &str) -> Option<time::OffsetDateTime> {
    let s = s.trim().trim_end_matches('Z');
    let (date, rest) = s.split_once('T')?;
    let mut d = date.split('-');
    let year: i32 = d.next()?.parse().ok()?;
    let month: u8 = d.next()?.parse().ok()?;
    let day: u8 = d.next()?.parse().ok()?;
    let mut t = rest.split(':');
    let hour: u8 = t.next()?.parse().ok()?;
    let minute: u8 = t.next()?.parse().ok()?;
    let (second, milli) = match t.next() {
        Some(sec) => {
            let (whole, frac) = sec.split_once('.').unwrap_or((sec, ""));
            let frac: String = frac.chars().take(3).collect();
            let milli: u16 = if frac.is_empty() {
                0
            } else {
                format!("{frac:0<3}").parse().ok()?
            };
            (whole.parse::<u8>().ok()?, milli)
        }
        None => (0, 0),
    };
    let date =
        time::Date::from_calendar_date(year, time::Month::try_from(month).ok()?, day).ok()?;
    let time = time::Time::from_hms_milli(hour, minute, second, milli).ok()?;
    Some(time::PrimitiveDateTime::new(date, time).assume_utc())
}

/// A small pool of read-only connections; `with` runs a closure on a free one.
pub struct ReaderPool {
    conns: Vec<Mutex<Connection>>,
    next: AtomicUsize,
}

impl ReaderPool {
    fn open(path: &Path, n: usize) -> Result<Self> {
        let mut conns = Vec::with_capacity(n);
        for _ in 0..n {
            let c = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX
                    | OpenFlags::SQLITE_OPEN_URI,
            )?;
            c.pragma_update(None, "busy_timeout", 5000)?;
            c.pragma_update(None, "query_only", 1)?;
            c.pragma_update(None, "temp_store", "MEMORY")?;
            conns.push(Mutex::new(c));
        }
        Ok(Self {
            conns,
            next: AtomicUsize::new(0),
        })
    }

    /// Run `f` on a free reader (round-robin, falling back to waiting on one).
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let n = self.conns.len();
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        for i in 0..n {
            if let Ok(guard) = self.conns[(start + i) % n].try_lock() {
                return f(&guard);
            }
        }
        let guard = self.conns[start % n]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&guard)
    }

    pub fn len(&self) -> usize {
        self.conns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.conns.is_empty()
    }
}

impl Db {
    /// Run a write job on the writer connection and wait for its result.
    /// Never call this from inside another write job (the writer is one thread).
    pub fn write<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        if let Some(reason) = &self.read_only {
            return Err(DbError::ReadOnly(reason.clone()));
        }
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<T>>(1);
        self.submit(Box::new(move |conn| {
            let out = std::panic::catch_unwind(AssertUnwindSafe(|| f(conn)))
                .unwrap_or(Err(DbError::Panicked));
            let _ = tx.send(out);
        }))?;
        rx.recv().map_err(|_| DbError::Closed)?
    }

    /// The async twin of [`Db::write`].
    pub async fn write_async<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        if let Some(reason) = &self.read_only {
            return Err(DbError::ReadOnly(reason.clone()));
        }
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<T>>();
        self.submit(Box::new(move |conn| {
            let out = std::panic::catch_unwind(AssertUnwindSafe(|| f(conn)))
                .unwrap_or(Err(DbError::Panicked));
            let _ = tx.send(out);
        }))?;
        rx.await.map_err(|_| DbError::Closed)?
    }

    fn submit(&self, job: Job) -> Result<()> {
        self.writer
            .as_ref()
            .ok_or(DbError::Closed)?
            .send(job)
            .map_err(|_| DbError::Closed)
    }

    /// Run a read on a pooled read-only connection, on the calling thread.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        self.readers.with(f)
    }

    /// Run a read on the blocking pool (for async callers).
    pub async fn read_async<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let pool = Arc::clone(&self.readers);
        tokio::task::spawn_blocking(move || pool.with(f))
            .await
            .map_err(|_| DbError::Panicked)?
    }

    /// The reader pool itself (for long-lived subscribers).
    pub fn readers(&self) -> Arc<ReaderPool> {
        Arc::clone(&self.readers)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `<app_data_dir>/backups`.
    pub fn backups_dir(&self) -> &Path {
        &self.backups_dir
    }

    /// `settings['device.id']`, minted on first launch and never hardware-derived.
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Why the database is read-only (a failed pre-migration backup), if it is.
    pub fn read_only_reason(&self) -> Option<&str> {
        self.read_only.as_deref()
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only.is_some()
    }

    /// `PRAGMA user_version` as seen at open.
    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn user_version(&self) -> Result<u32> {
        self.read(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
    }

    /// `PRAGMA integrity_check` is "ok".
    pub fn integrity_ok(&self) -> Result<bool> {
        self.read(|c| Ok(integrity_check(c)? == "ok"))
    }

    /// `PRAGMA foreign_key_check` reports nothing.
    pub fn foreign_keys_ok(&self) -> Result<bool> {
        self.read(|c| Ok(foreign_key_violations(c)? == 0))
    }

    /// "Backup now" into [`Db::backups_dir`] ([`backup_now`]).
    pub fn backup_now(&self) -> Result<PathBuf> {
        let dir = self.backups_dir.clone();
        self.write(move |c| backup_now(c, &dir))
    }

    /// Empty the database in place ([`reset_contents_in`]).
    pub fn reset_contents(&self) -> Result<ResetReport> {
        let device = self.device_id.clone();
        self.write(move |c| reset_contents_in(c, &device))
    }

    /// Fold the WAL back into the main file (quit, 08 §2.6).
    pub fn checkpoint(&self) -> Result<()> {
        self.write(|c| {
            c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
            Ok(())
        })
    }

    /// Sizes of the database file and its WAL, in bytes.
    pub fn file_sizes(&self) -> (u64, u64) {
        let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        let mut wal = self.path.clone().into_os_string();
        wal.push("-wal");
        (size(&self.path), size(Path::new(&wal)))
    }

    /// Versions recorded in the ledger, ascending.
    pub fn applied_versions(&self) -> Result<Vec<u32>> {
        self.read(|c| {
            let mut stmt = c.prepare("SELECT version FROM schema_migrations ORDER BY version")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            Ok(rows.collect::<std::result::Result<Vec<u32>, _>>()?)
        })
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        // Close the channel, then let the writer finish its queue and close its connection.
        self.writer.take();
        if let Some(handle) = self.writer_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamps_round_trip_through_the_fixed_parser() {
        let t = super::parse_timestamp("2026-10-18T20:17:03.123Z").unwrap();
        assert_eq!(super::format_timestamp(t), "2026-10-18T20:17:03.123Z");
        assert_eq!(
            super::format_timestamp(super::parse_timestamp("2026-10-18T20:17:03Z").unwrap()),
            "2026-10-18T20:17:03.000Z"
        );
        assert_eq!(
            super::format_timestamp(super::parse_timestamp("2026-10-18T20:17:03.5Z").unwrap()),
            "2026-10-18T20:17:03.500Z"
        );
        assert!(super::parse_timestamp("nope").is_none());
        assert!(super::parse_timestamp("2026-13-01T00:00:00Z").is_none());
    }

    use super::*;

    fn pragma_str(conn: &Connection, name: &str) -> String {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| {
            r.get::<_, rusqlite::types::Value>(0)
        })
        .map(|v| match v {
            rusqlite::types::Value::Text(s) => s,
            rusqlite::types::Value::Integer(i) => i.to_string(),
            other => format!("{other:?}"),
        })
        .unwrap()
    }

    /// Every migration version, in order: what `applied_versions` reports at head.
    fn all_versions() -> Vec<u32> {
        MIGRATIONS.iter().map(|m| m.version).collect()
    }

    #[test]
    fn open_creates_file_with_pragmas_and_migrates_to_head() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("mode.db");
        let db = open(&path).unwrap();
        assert!(path.exists());
        assert_eq!(db.user_version().unwrap(), latest_version());
        assert_eq!(db.applied_versions().unwrap(), all_versions());
        assert!(db.integrity_ok().unwrap());
        assert!(db.foreign_keys_ok().unwrap());
        assert!(!db.is_read_only());
        assert_eq!(db.backups_dir(), path.parent().unwrap().join("backups"));

        db.write(|c| {
            assert_eq!(pragma_str(c, "journal_mode"), "wal");
            assert_eq!(pragma_str(c, "foreign_keys"), "1");
            assert_eq!(pragma_str(c, "synchronous"), "1");
            assert_eq!(pragma_str(c, "busy_timeout"), "5000");
            assert_eq!(pragma_str(c, "temp_store"), "2");
            assert_eq!(pragma_str(c, "auto_vacuum"), "2", "incremental auto_vacuum");
            Ok(())
        })
        .unwrap();

        let (name, kind, checksum, app_version): (String, String, String, String) = db
            .read(|c| {
                Ok(c.query_row(
                    "SELECT name, kind, checksum, app_version FROM schema_migrations WHERE version = 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?)
            })
            .unwrap();
        assert_eq!((name.as_str(), kind.as_str()), ("initial", "ddl"));
        assert_eq!(checksum, MIGRATIONS[0].checksum());
        assert_eq!(checksum.len(), 64);
        assert_eq!(app_version, crate::VERSION);
    }

    #[test]
    fn readers_are_read_only_and_see_committed_writes() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("mode.db")).unwrap();
        let denied = db.read(|c| {
            c.execute("INSERT INTO chat_turns (id, conversation_id, role, content, created_at) VALUES ('a','c','user','x','t')", [])?;
            Ok(())
        });
        assert!(denied.is_err(), "readers must not write");
        db.write(|c| {
            c.execute("INSERT INTO chat_turns (id, conversation_id, role, content, created_at) VALUES ('a','c','user','x','t')", [])?;
            Ok(())
        })
        .unwrap();
        let n: i64 = db
            .read(|c| Ok(c.query_row("SELECT COUNT(*) FROM chat_turns", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(db.readers().len(), DEFAULT_READERS);
    }

    #[tokio::test]
    async fn async_write_and_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("mode.db")).unwrap();
        db.write_async(|c| {
            c.execute("INSERT INTO chat_turns (id, conversation_id, role, content, created_at) VALUES ('b','c','user','y','t')", [])?;
            Ok(())
        })
        .await
        .unwrap();
        let content: String = db
            .read_async(|c| {
                Ok(
                    c.query_row("SELECT content FROM chat_turns WHERE id = 'b'", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(content, "y");
    }

    #[test]
    fn a_panicking_job_does_not_kill_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("mode.db")).unwrap();
        let r: Result<()> = db.write(|_| panic!("boom"));
        assert!(matches!(r, Err(DbError::Panicked)));
        assert!(db.write(|_| Ok(1)).is_ok(), "the writer survives");
    }

    #[test]
    fn reopening_is_idempotent_and_keeps_the_device_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mode.db");
        let first = open(&path).unwrap().device_id().to_string();
        let db = open(&path).unwrap();
        assert_eq!(db.applied_versions().unwrap(), all_versions());
        assert_eq!(db.user_version().unwrap(), latest_version());
        assert_eq!(db.device_id(), first);
        assert_eq!(
            uuid::Uuid::parse_str(db.device_id())
                .unwrap()
                .get_version_num(),
            7
        );
        assert!(
            !db.backups_dir().exists(),
            "no backup for a database with nothing pending"
        );
    }

    #[test]
    fn checksum_mismatch_refuses_to_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mode.db");
        {
            let db = open(&path).unwrap();
            db.write(|c| {
                c.execute(
                    "UPDATE schema_migrations SET checksum = 'deadbeef' WHERE version = 1",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        }
        match open(&path) {
            Err(DbError::ChecksumMismatch {
                version: 1,
                applied,
                ..
            }) => assert_eq!(applied, "deadbeef"),
            other => panic!("expected a checksum mismatch, got {other:?}"),
        }
    }

    #[test]
    fn newer_database_than_build_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mode.db");
        {
            let db = open(&path).unwrap();
            db.write(|c| Ok(c.pragma_update(None, "user_version", 999)?))
                .unwrap();
        }
        match open(&path) {
            Err(DbError::TooNew {
                found: 999,
                supported,
            }) => assert_eq!(supported, latest_version()),
            other => panic!("expected TooNew, got {other:?}"),
        }
    }

    #[test]
    fn strict_and_json_checks_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("mode.db")).unwrap();
        let bad = db.write(|c| {
            c.execute(
                "INSERT INTO settings (key, value_json, scope, updated_at, device_id) VALUES ('x', 'not json', 'user', 't', 'd')",
                [],
            )?;
            Ok(())
        });
        assert!(bad.is_err(), "json_valid CHECK must reject invalid JSON");
        let bad_scope = db.write(|c| {
            c.execute(
                "INSERT INTO settings (key, value_json, scope, updated_at, device_id) VALUES ('x', '1', 'global', 't', 'd')",
                [],
            )?;
            Ok(())
        });
        assert!(bad_scope.is_err(), "scope CHECK must reject unknown scopes");
    }

    #[test]
    fn timestamps_are_24_char_utc_milliseconds() {
        let s = now_rfc3339();
        assert_eq!(s.len(), 24, "{s}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[10..11], "T");
        let t = time::OffsetDateTime::from_unix_timestamp(1_760_818_623).unwrap()
            + time::Duration::milliseconds(123);
        assert_eq!(format_timestamp(t), "2025-10-18T20:17:03.123Z");
    }

    #[test]
    fn backup_is_written_before_a_pending_migration_and_pruned_to_five() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mode.db");
        let backups = dir.path().join("backups");
        {
            drop(open(&path).unwrap());
        }
        // Drive the backup the way `open` does for a database at head with the next version pending.
        let conn = Connection::open(&path).unwrap();
        apply_pragmas(&conn).unwrap();
        let next = latest_version() + 1;
        for _ in 0..7 {
            let file = backup_before_migration(&conn, &backups, next).unwrap();
            assert!(file.exists());
            let name = file.file_name().unwrap().to_str().unwrap().to_string();
            assert!(name.starts_with(&format!("mode-pre-{next:04}-")), "{name}");
            assert!(name.ends_with(".db"));
            // A backup is a consistent database with the ledger inside.
            let b = Connection::open(&file).unwrap();
            let v: i64 = b
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(v, i64::from(latest_version()));
        }
        let n = std::fs::read_dir(&backups).unwrap().count();
        assert_eq!(n, BACKUPS_KEPT);
    }

    #[test]
    fn migrate_runs_a_pending_migration_with_a_backup_and_fk_check() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mode.db");
        drop(open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        apply_pragmas(&conn).unwrap();
        let next = latest_version() + 1;
        let extra = Migration {
            version: next,
            name: "scratch",
            kind: "ddl",
            sql: "CREATE TABLE scratch (x INTEGER) STRICT; INSERT INTO scratch VALUES (1);",
        };
        let list: Vec<Migration> = MIGRATIONS.iter().copied().chain([extra]).collect();
        let ctx = seeds::SeedContext::detect("dev".into());
        let applied = migrate(&conn, &list, &ctx).unwrap();
        assert_eq!(applied, vec![next]);
        let v: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, next);
        assert_eq!(pragma_str(&conn, "foreign_keys"), "1", "restored after DDL");

        let broken = Migration {
            version: next + 1,
            name: "broken",
            kind: "ddl",
            sql: "CREATE TABLE child (p TEXT REFERENCES scratch_missing(x)) STRICT; INSERT INTO child VALUES ('nope');",
        };
        let list: Vec<Migration> = MIGRATIONS.iter().copied().chain([extra, broken]).collect();
        let err = migrate(&conn, &list, &ctx).unwrap_err();
        assert!(
            matches!(err, DbError::ForeignKeys { version, .. } if version == next + 1),
            "{err}"
        );
        let v: u32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, next, "the failed migration rolled back");
    }

    #[test]
    fn backup_now_writes_a_readable_copy_listed_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("mode.db")).unwrap();
        db.write(|c| {
            c.execute(
                "INSERT INTO settings (key, value_json, scope, updated_at, rev, device_id)
                 VALUES ('made.up', '1', 'user', 't', 1, 'd')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let first = db.backup_now().unwrap();
        let second = db.backup_now().unwrap();
        assert_ne!(first, second);
        let name = first.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(MANUAL_BACKUP_PREFIX) && name.ends_with(".db"));
        let copy = Connection::open(&second).unwrap();
        let v: String = copy
            .query_row(
                "SELECT value_json FROM settings WHERE key = 'made.up'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, "1");
        // Windows refuses to delete an open file: close the copy before pruning.
        drop(copy);
        let listed = list_backups(db.backups_dir()).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(
            listed
                .iter()
                .all(|b| b.size_bytes > 0 && b.created_at.len() == 24)
        );
        for _ in 0..BACKUPS_KEPT + 1 {
            db.backup_now().unwrap();
        }
        assert_eq!(list_backups(db.backups_dir()).unwrap().len(), BACKUPS_KEPT);
        assert!(
            list_backups(&dir.path().join("nowhere"))
                .unwrap()
                .is_empty()
        );
        let (size, _wal) = db.file_sizes();
        assert!(size > 0);
        db.checkpoint().unwrap();
    }

    #[test]
    fn reset_contents_empties_the_tables_and_keeps_device_and_seeds() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("mode.db")).unwrap();
        let device = db.device_id().to_string();
        let seeded = |db: &Db| {
            db.read(|c| {
                Ok(c.query_row("SELECT COUNT(*) FROM groupings", [], |r| r.get::<_, i64>(0))?)
            })
            .unwrap()
        };
        let premade = seeded(&db);
        assert!(premade > 0);
        db.write(|c| {
            c.execute(
                "INSERT INTO settings (key, value_json, scope, updated_at, rev, device_id)
                 VALUES ('sounds.volume', '80', 'user', 't', 1, 'd')
                 ON CONFLICT(key) DO UPDATE SET value_json = '80'",
                [],
            )?;
            c.execute(
                "INSERT INTO folders (id, path, name, created_at, updated_at, device_id)
                 VALUES ('f1', 'Inbox', 'Inbox', 't', 't', 'd')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let counts: std::collections::BTreeMap<String, i64> =
            db.read(table_counts).unwrap().into_iter().collect();
        assert_eq!(counts["folders"], 1);
        assert!(!counts.keys().any(|k| k.contains("_fts")), "{counts:?}");
        let report = db.reset_contents().unwrap();
        assert_eq!(report.deleted.get("folders"), Some(&1));
        assert!(report.deleted.contains_key("settings"));
        assert_eq!(db.device_id(), device);
        let after: std::collections::BTreeMap<String, i64> =
            db.read(table_counts).unwrap().into_iter().collect();
        assert_eq!(after["folders"], 0);
        assert_eq!(
            after["schema_migrations"],
            i64::from(latest_version()),
            "the ledger stays"
        );
        assert_eq!(seeded(&db), premade, "premade rows re-seeded");
        let stored: String = db
            .read(|c| {
                Ok(c.query_row(
                    "SELECT value_json FROM settings WHERE key = 'device.id'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert!(stored.contains(&device));
        assert!(db.foreign_keys_ok().unwrap() && db.integrity_ok().unwrap());
        // Still writable through the same handle; reopening keeps the device.
        db.write(|c| {
            c.execute(
                "INSERT INTO folders (id, path, name, created_at, updated_at, device_id)
                 VALUES ('f2', 'Later', 'Later', 't', 't', 'd')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        drop(db);
        let again = open(&dir.path().join("mode.db")).unwrap();
        assert_eq!(again.device_id(), device);
    }

    #[test]
    fn prune_keeps_the_newest_by_name() {
        let dir = tempfile::tempdir().unwrap();
        for n in [
            "0001-a", "0001-b", "0002-a", "0002-b", "0002-c", "0003-a", "0003-b",
        ] {
            std::fs::write(dir.path().join(format!("mode-pre-{n}.db")), b"x").unwrap();
        }
        std::fs::write(dir.path().join("unrelated.txt"), b"x").unwrap();
        let removed = prune_backups(dir.path(), 5).unwrap();
        assert_eq!(removed.len(), 2);
        assert!(!dir.path().join("mode-pre-0001-a.db").exists());
        assert!(!dir.path().join("mode-pre-0001-b.db").exists());
        assert!(dir.path().join("mode-pre-0003-b.db").exists());
        assert!(dir.path().join("unrelated.txt").exists());
        assert!(
            prune_backups(&dir.path().join("missing"), 5)
                .unwrap()
                .is_empty()
        );
    }
}
