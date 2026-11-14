//! Settings → Advanced and the Stats view (package 9;
//! `contract-settings.ts`): `diagnostics_get` (03 §8.16), `logs_tail`
//! (§8.17), `diagnostics_export`, `db_backup`, `retention_sweep`, and
//! `stats_days` / `stats_summary` (03 §7A; Q11 and Q10 in tokens, never
//! currency). App window only.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use mode_core::db::BackupFile;
use mode_core::diag::{self, Snapshot};
use mode_core::queries::{self, Clock};
use mode_core::retention;
use mode_core::{hooks_db, hooks_install, settings};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::State;

use crate::cmd_error::{CmdError, CmdResult, blocking};
use crate::commands_settings::WrittenFile;
use crate::state::{CoreState, RecentFrame};

type Core<'a> = State<'a, Arc<CoreState>>;

// ------------------------------------------------------------- diagnostics

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaudeInfo {
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelayInfo {
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookInstallRow {
    pub scope: String,
    pub settings_path: String,
    pub status: String,
    pub relay_version: Option<String>,
    pub installed_at: Option<String>,
    pub verified_at: Option<String>,
    pub backup_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IpcInfo {
    pub endpoint: String,
    pub listening: bool,
    pub last_event_at: Option<String>,
    /// The last 20 hook events, newest first: name, time, latency only.
    pub recent: Vec<RecentFrame>,
    /// The server's live counters (frames, acks, decisions, timeouts …).
    pub metrics: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexInfo {
    pub vault_path: String,
    pub files_indexed: i64,
    pub last_full_scan_at: Option<String>,
    pub pending_touchups: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableRows {
    pub name: String,
    pub rows: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatabaseInfo {
    pub path: String,
    pub size_bytes: u64,
    pub wal_bytes: u64,
    pub tables: Vec<TableRows>,
    pub last_sweep_at: Option<String>,
    pub backups: Vec<BackupFile>,
    pub read_only: bool,
}

/// `Diagnostics` (03 §8.16).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Diagnostics {
    pub core: Snapshot,
    pub claude: ClaudeInfo,
    pub relay: RelayInfo,
    pub hooks: Vec<HookInstallRow>,
    pub ipc: IpcInfo,
    pub index: IndexInfo,
    pub database: DatabaseInfo,
    /// Sessions with hook activity, not ended (the Claude Code card).
    pub sessions_tracked: i64,
    /// Top-level sessions started today.
    pub sessions_today: i64,
    /// Embedded sessions with a live process.
    pub sessions_embedded: usize,
    /// `os` (keyring) or `file` (development store).
    pub secrets_store: &'static str,
    /// `logs/mode.log`, when the app logs to a file.
    pub log_path: Option<String>,
}

/// Gather the Diagnostics page (blocking: database reads, `--version` of
/// the relay and `claude`).
pub fn gather(core: &CoreState) -> CmdResult<Diagnostics> {
    let probe = crate::claude::probe(core);
    let relay_path = CoreState::lock(&core.relay_path).clone();
    let relay_version = relay_path.as_deref().and_then(hooks_install::relay_version);
    let hooks = core
        .db
        .read(hooks_db::rows)?
        .into_iter()
        .map(|r| HookInstallRow {
            scope: r.scope,
            settings_path: r.settings_path,
            status: r.status,
            relay_version: Some(r.relay_version).filter(|v| !v.is_empty()),
            installed_at: Some(r.installed_at).filter(|v| !v.is_empty()),
            verified_at: r.verified_at,
            backup_path: r.backup_path,
        })
        .collect();
    let recent: Vec<RecentFrame> = CoreState::lock(&core.recent_frames)
        .iter()
        .cloned()
        .collect();
    let ipc = IpcInfo {
        endpoint: core.endpoint.describe(),
        listening: core.ipc_listening.load(Ordering::Relaxed),
        last_event_at: recent.first().map(|f| f.at.clone()),
        metrics: core
            .server()
            .and_then(|s| serde_json::to_value(s.metrics()).ok()),
        recent,
    };
    let vault_path = match core.notes.get() {
        Some(n) => n.root().to_string_lossy().into_owned(),
        None => settings::get_string(&core.db, "notes.vault_path")?.unwrap_or_default(),
    };
    let (files_indexed, pending_touchups, tracked, today, last_sweep_at, tables) =
        core.db.read(|c| {
            let one = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
            let clock = Clock::current(c)?;
            let today_start = queries::local_day_bounds(c, &clock.today, &clock.today)?.0;
            let today: i64 = c.query_row(
                "SELECT COUNT(*) FROM sessions WHERE parent_session_id IS NULL AND started_at >= ?1",
                [&today_start],
                |r| r.get(0),
            )?;
            Ok((
                one("SELECT COUNT(*) FROM notes WHERE deleted_at IS NULL")?,
                one("SELECT COUNT(*) FROM notes WHERE frontmatter_dirty = 1 AND deleted_at IS NULL")?,
                one("SELECT COUNT(*) FROM sessions WHERE tracking IN ('hooked','launched') AND ended_at IS NULL AND parent_session_id IS NULL")?,
                today,
                settings::get_string_in(c, settings::LAST_SWEEP_KEY)?,
                mode_core::db::table_counts(c)?,
            ))
        })?;
    let (size_bytes, wal_bytes) = core.db.file_sizes();
    Ok(Diagnostics {
        core: core.snapshot(),
        claude: ClaudeInfo {
            found: probe.path.is_some(),
            path: probe
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            version: probe.version.clone(),
        },
        relay: RelayInfo {
            path: relay_path.map(|p| p.to_string_lossy().into_owned()),
            version: relay_version,
        },
        hooks,
        ipc,
        index: IndexInfo {
            vault_path,
            files_indexed,
            last_full_scan_at: CoreState::lock(&core.last_full_scan_at).clone(),
            pending_touchups,
        },
        database: DatabaseInfo {
            path: core.db_path.to_string_lossy().into_owned(),
            size_bytes,
            wal_bytes,
            tables: tables
                .into_iter()
                .map(|(name, rows)| TableRows { name, rows })
                .collect(),
            last_sweep_at,
            backups: mode_core::db::list_backups(&core.backups_dir())?,
            read_only: core.db.is_read_only(),
        },
        sessions_tracked: tracked,
        sessions_today: today,
        sessions_embedded: crate::quit::live_sessions(core).len(),
        secrets_store: core.keyring().kind(),
        log_path: diag::current_log_file().map(|p| p.to_string_lossy().into_owned()),
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn diagnostics_get(state: Core<'_>) -> CmdResult<Diagnostics> {
    let core = Arc::clone(&state);
    blocking(move || gather(&core)).await
}

// -------------------------------------------------------------------- logs

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogsTail {
    pub path: String,
    pub lines: Vec<String>,
}

/// The log file this run writes, else where it would be.
pub fn log_file(core: &CoreState) -> std::path::PathBuf {
    diag::current_log_file().unwrap_or_else(|| diag::log_dir(&core.data_dir).join(diag::LOG_FILE))
}

/// The last `lines` lines of `logs/mode.log` (2 000 by default, at most
/// 10 000), oldest first.
#[tauri::command(rename_all = "snake_case")]
pub async fn logs_tail(state: Core<'_>, lines: Option<usize>) -> CmdResult<LogsTail> {
    let path = log_file(&state);
    blocking(move || {
        let lines = diag::tail(&path, lines.unwrap_or(2_000))?;
        Ok(LogsTail {
            path: path.to_string_lossy().into_owned(),
            lines,
        })
    })
    .await
}

/// "Export diagnostics" (03 §8.16, 08 §12): one JSON file in
/// `<data_dir>/diagnostics/` with the Diagnostics page, the settings (no
/// secrets: they are in the keyring), the `hook_installs` rows, the hook
/// status, the last 20 hook latencies and the last 2 000 log lines. No
/// bodies, prompts or tokens.
#[tauri::command(rename_all = "snake_case")]
pub async fn diagnostics_export(state: Core<'_>) -> CmdResult<WrittenFile> {
    let core = Arc::clone(&state);
    blocking(move || {
        let d = gather(&core)?;
        let settings = settings::all_effective(&core.db)?;
        let log = diag::tail(&log_file(&core), 2_000).unwrap_or_default();
        let hook_status = CoreState::lock(&core.hooks).clone();
        let doc = json!({
            "kind": "mode-diagnostics",
            "format": 1,
            "written_at": mode_core::db::now_rfc3339(),
            "diagnostics": d,
            "hook_status": hook_status,
            "settings": settings,
            "log_tail": log,
        });
        let dir = core.data_dir.join("diagnostics");
        std::fs::create_dir_all(&dir)?;
        let stamp: String = mode_core::db::now_rfc3339()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        let path = crate::dialogs::unique_in(&dir, &format!("mode-diagnostics-{stamp}.json"));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&doc).map_err(CmdError::internal)?,
        )?;
        tracing::info!(path = %path.display(), "diagnostics exported");
        Ok(WrittenFile {
            path: Some(path.to_string_lossy().into_owned()),
        })
    })
    .await
}

// ---------------------------------------------------------------- database

/// "Backup now" (`VACUUM INTO`, 03 §8.16): the copy written.
#[tauri::command(rename_all = "snake_case")]
pub async fn db_backup(state: Core<'_>) -> CmdResult<BackupFile> {
    let db = Arc::clone(&state.db);
    blocking(move || {
        let path = db.backup_now()?;
        let listed = mode_core::db::list_backups(db.backups_dir())?;
        let row = listed
            .into_iter()
            .find(|b| b.path == path)
            .ok_or_else(|| CmdError::internal("the backup is not in the backups folder"))?;
        tracing::info!(path = %row.path.display(), bytes = row.size_bytes, "database backed up");
        Ok(row)
    })
    .await
}

/// `retention_sweep`: the sweep report plus `deleted` (rows per table).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SweepResult {
    #[serde(flatten)]
    pub report: retention::Report,
    pub deleted: BTreeMap<String, usize>,
}

/// The rows a sweep removed or rewrote, by table (non-zero only).
pub fn deleted_by_table(r: &retention::Report) -> BTreeMap<String, usize> {
    let mut m: BTreeMap<String, usize> = [
        ("session_events", r.session_events_deleted),
        ("sessions", r.sessions_deleted),
        ("note_revisions", r.note_revisions_deleted),
        ("messages", r.messages_deleted),
        ("threads", r.threads_deleted),
        ("calendar_events", r.calendar_events_deleted),
        ("usage_samples", r.usage_samples_deleted),
        ("alert_log", r.alert_log_deleted),
        ("connector_writebacks", r.writebacks_deleted),
        ("chat_turns", r.chat_turns_deleted),
        ("automation_runs", r.automation_runs_deleted),
        ("activity_days", r.activity_days_deleted),
        ("change_log", r.change_log_pruned),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .map(|(k, n)| (k.to_string(), n))
    .collect();
    for (table, n) in &r.tombstones_purged {
        if *n > 0 {
            *m.entry(table.clone()).or_insert(0) += n;
        }
    }
    m
}

/// "Run sweep now" (03 §8.16; 09 §10): the nightly sweep, or with
/// `dry_run` what it would do.
#[tauri::command(rename_all = "snake_case")]
pub async fn retention_sweep(state: Core<'_>, dry_run: Option<bool>) -> CmdResult<SweepResult> {
    let core = Arc::clone(&state);
    let r = blocking(move || {
        let report = retention::run_with(
            &core.db,
            dry_run.unwrap_or(false),
            mode_core::stats::claude_cache::path().as_deref(),
        )?;
        Ok(SweepResult {
            deleted: deleted_by_table(&report),
            report,
        })
    })
    .await?;
    if !r.report.dry_run && !r.deleted.is_empty() {
        state.publish([
            mode_core::push::Event::Sessions,
            mode_core::push::Event::Pill,
        ]);
    }
    Ok(r)
}

// ------------------------------------------------------------------- stats

use mode_core::stats::summary::StatsError;
pub use mode_core::stats::{STATS_MAX_DAYS, StatsDay, StatsSummary, date_of, state_word, window};

fn stats_err(e: StatsError) -> CmdError {
    match e {
        StatsError::Window(w) => CmdError::invalid(w.0),
        StatsError::Db(d) => d.into(),
    }
}

/// Q11 for `from ..= to`, every day present (zeros where nothing happened).
pub fn stats_days_in(
    conn: &rusqlite::Connection,
    from: &str,
    to: &str,
) -> CmdResult<Vec<StatsDay>> {
    mode_core::stats::days(conn, from, to).map_err(stats_err)
}

/// The day tracker (03 §7A): Mode activity per local day.
#[tauri::command(rename_all = "snake_case")]
pub async fn stats_days(state: Core<'_>, from: String, to: String) -> CmdResult<Vec<StatsDay>> {
    let db = Arc::clone(&state.db);
    blocking(move || db.read(|c| Ok(stats_days_in(c, &from, &to)))?).await
}

/// `StatsSummary` (03 §7A): the spend tiles, the metrics and the breakdown of a window.
pub fn stats_summary_in(
    conn: &rusqlite::Connection,
    from: &str,
    to: &str,
) -> CmdResult<StatsSummary> {
    mode_core::stats::summary(conn, from, to).map_err(stats_err)
}

/// The Stats view's figures for a window (03 §7A).
#[tauri::command(rename_all = "snake_case")]
pub async fn stats_summary(state: Core<'_>, from: String, to: String) -> CmdResult<StatsSummary> {
    let db = Arc::clone(&state.db);
    blocking(move || db.read(|c| Ok(stats_summary_in(c, &from, &to)))?).await
}
