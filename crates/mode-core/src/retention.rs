//! The nightly sweep (09 §6.5, §10): roll completed days into `activity_days`
//! (S9), apply every retention window and cap, tombstone links and cancel
//! snoozes whose targets are gone, purge tombstones past their window, prune
//! `change_log`, prune pre-migration backups, then `incremental_vacuum` and
//! `optimize`. [`sweep`] runs on the writer inside one transaction; with
//! `dry_run` the transaction is rolled back and the report says what would
//! have happened.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, named_params};
use serde::{Deserialize, Serialize};

use crate::db::{self, Db, Result};
use crate::queries::Clock;
use crate::settings;
use crate::stats;

/// Every window and cap of 09 §10. The `retention.*` settings feed the first
/// five; the rest are the document's fixed defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub session_events_days: i64,
    pub sessions_days: i64,
    pub messages_days: i64,
    pub message_body_days: i64,
    pub usage_days: i64,
    /// `discovered` / `untracked` sessions, from `last_event_at`.
    pub discovered_days: i64,
    /// Hard cap on `session_events.payload_json` bytes; oldest ended sessions compacted first.
    pub session_events_cap_bytes: i64,
    pub note_revisions_keep: i64,
    pub note_revisions_days: i64,
    pub messages_per_account_cap: i64,
    pub calendar_past_days: i64,
    pub calendar_future_days: i64,
    pub alert_log_days: i64,
    pub writebacks_hours: i64,
    pub chat_turns_days: i64,
    pub automation_runs_days: i64,
    pub activity_days: i64,
    /// `media_plays` (the Music tab): nothing older than this…
    pub media_plays_days: i64,
    /// …and no more than this many plays.
    pub media_plays_max: i64,
    pub tombstone_days: i64,
    pub change_log_days: i64,
    pub change_log_max: i64,
    pub backups_kept: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self::from_retention(settings::Retention::default())
    }
}

impl Policy {
    pub fn from_retention(r: settings::Retention) -> Self {
        Self {
            session_events_days: r.session_events_days,
            sessions_days: r.sessions_days,
            messages_days: r.messages_days,
            message_body_days: r.message_body_days,
            usage_days: r.usage_days,
            discovered_days: 30,
            session_events_cap_bytes: 200 * 1024 * 1024,
            note_revisions_keep: 50,
            note_revisions_days: 180,
            messages_per_account_cap: 20_000,
            calendar_past_days: 30,
            calendar_future_days: 90,
            alert_log_days: 7,
            writebacks_hours: 24,
            chat_turns_days: r.chat_days,
            automation_runs_days: 90,
            activity_days: 400,
            media_plays_days: crate::media_history::KEEP_DAYS,
            media_plays_max: crate::media_history::KEEP_PLAYS,
            tombstone_days: 30,
            change_log_days: 30,
            change_log_max: 50_000,
            backups_kept: db::BACKUPS_KEPT,
        }
    }

    /// The policy with the stored `retention.*` settings.
    pub fn from_settings(conn: &Connection) -> Result<Self> {
        Ok(Self::from_retention(settings::retention_in(conn)?))
    }
}

/// What a sweep did (or, dry-run, would do).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub dry_run: bool,
    pub activity_days_rolled: usize,
    pub activity_models_rolled: usize,
    pub activity_hours_rolled: usize,
    /// Days of history taken from Claude Code's stats cache (ADR-0052).
    pub claude_stats_days: usize,
    pub session_events_deleted: usize,
    pub session_events_compacted: usize,
    pub sessions_deleted: usize,
    pub note_revisions_deleted: usize,
    pub messages_deleted: usize,
    pub message_bodies_cleared: usize,
    pub threads_deleted: usize,
    pub threads_recomputed: usize,
    pub calendar_events_deleted: usize,
    pub usage_samples_deleted: usize,
    pub alert_log_deleted: usize,
    pub writebacks_deleted: usize,
    pub chat_turns_deleted: usize,
    pub automation_runs_deleted: usize,
    pub activity_days_deleted: usize,
    pub media_plays_deleted: usize,
    pub links_tombstoned: usize,
    pub snoozes_cancelled: usize,
    pub snoozes_expired_deleted: usize,
    /// Tombstones purged, per table.
    pub tombstones_purged: BTreeMap<String, usize>,
    pub change_log_pruned: usize,
    pub backups_removed: Vec<PathBuf>,
}

impl Report {
    pub fn total_rows(&self) -> usize {
        self.session_events_deleted
            + self.session_events_compacted
            + self.sessions_deleted
            + self.note_revisions_deleted
            + self.messages_deleted
            + self.message_bodies_cleared
            + self.threads_deleted
            + self.calendar_events_deleted
            + self.usage_samples_deleted
            + self.alert_log_deleted
            + self.writebacks_deleted
            + self.chat_turns_deleted
            + self.automation_runs_deleted
            + self.activity_days_deleted
            + self.media_plays_deleted
            + self.links_tombstoned
            + self.snoozes_cancelled
            + self.snoozes_expired_deleted
            + self.tombstones_purged.values().sum::<usize>()
            + self.change_log_pruned
    }
}

const TS: &str = "strftime('%Y-%m-%dT%H:%M:%fZ'";

fn shift(conn: &Connection, now: &str, modifier: &str) -> Result<String> {
    Ok(conn.query_row(
        &format!("SELECT {TS}, ?1, ?2)"),
        rusqlite::params![now, modifier],
        |r| r.get(0),
    )?)
}

/// S-class tables whose tombstones are purged after `tombstone_days`, with
/// the rows that are never purged (premade / builtin rows keep their
/// tombstone so a re-seed never resurrects them).
const TOMBSTONED: &[(&str, &str)] = &[
    ("folders", ""),
    ("notes", ""),
    ("tags", ""),
    ("groupings", " AND kind <> 'premade'"),
    ("contacts", ""),
    ("contact_identities", ""),
    ("links", ""),
    ("alert_rules", " AND kind <> 'premade'"),
    ("themes", " AND kind <> 'builtin'"),
    ("automations", ""),
];

/// The per-kind "does the target still exist" predicate, over columns `k` and `v`.
fn target_exists(k: &str, v: &str) -> String {
    format!(
        "CASE {k}
           WHEN 'note' THEN EXISTS (SELECT 1 FROM notes WHERE id = {v} AND deleted_at IS NULL)
           WHEN 'message' THEN EXISTS (SELECT 1 FROM messages WHERE id = {v})
           WHEN 'thread' THEN EXISTS (SELECT 1 FROM threads WHERE id = {v})
           WHEN 'session' THEN EXISTS (SELECT 1 FROM sessions WHERE id = {v})
           WHEN 'calendar_event' THEN EXISTS (SELECT 1 FROM calendar_events WHERE id = {v})
           WHEN 'contact' THEN EXISTS (SELECT 1 FROM contacts WHERE id = {v} AND deleted_at IS NULL)
           WHEN 'url' THEN 1
           ELSE 0 END"
    )
}

/// Run the sweep on the writer connection. One transaction; rolled back when
/// `dry_run`. Backups are pruned by [`run`], not here (no connection needed).
pub fn sweep(
    conn: &mut Connection,
    policy: &Policy,
    clock: &Clock,
    device_id: &str,
    dry_run: bool,
) -> Result<Report> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let report = sweep_in(&tx, policy, clock, device_id, dry_run)?;
    if dry_run {
        tx.rollback()?;
    } else {
        tx.commit()?;
        conn.execute_batch("PRAGMA incremental_vacuum; PRAGMA optimize;")?;
    }
    Ok(report)
}

fn sweep_in(
    conn: &Connection,
    p: &Policy,
    clock: &Clock,
    device_id: &str,
    dry_run: bool,
) -> Result<Report> {
    let now = clock.now.as_str();
    let mut r = Report {
        dry_run,
        ..Default::default()
    };

    // S9 first: roll completed days up before anything is pruned.
    let rolled = stats::rollup::rollup(conn, clock, p.sessions_days)?;
    r.activity_days_rolled = rolled.days;
    r.activity_models_rolled = rolled.models;
    r.activity_hours_rolled = rolled.hours;

    // session_events: after `ended_at` + window, state-changing rows are kept compacted, the rest deleted.
    let ev_cutoff = shift(conn, now, &format!("-{} days", p.session_events_days))?;
    r.session_events_deleted += conn.execute(
        "DELETE FROM session_events
          WHERE state_after IS NULL AND compacted = 0
            AND session_id IN (SELECT id FROM sessions WHERE ended_at IS NOT NULL AND ended_at < :cutoff)",
        named_params! { ":cutoff": ev_cutoff },
    )?;
    r.session_events_compacted += conn.execute(
        "UPDATE session_events SET payload_json = NULL, compacted = 1
          WHERE state_after IS NOT NULL AND compacted = 0
            AND session_id IN (SELECT id FROM sessions WHERE ended_at IS NOT NULL AND ended_at < :cutoff)",
        named_params! { ":cutoff": ev_cutoff },
    )?;
    // Hard cap on payload bytes: compact the oldest ended sessions first.
    for _ in 0..10_000 {
        let bytes: i64 = conn.query_row(
            "SELECT COALESCE(SUM(length(payload_json)), 0) FROM session_events",
            [],
            |row| row.get(0),
        )?;
        if bytes <= p.session_events_cap_bytes {
            break;
        }
        let victim: Option<String> = conn
            .query_row(
                "SELECT s.id FROM sessions s
                  WHERE s.ended_at IS NOT NULL
                    AND EXISTS (SELECT 1 FROM session_events e WHERE e.session_id = s.id AND e.compacted = 0)
                  ORDER BY s.ended_at LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();
        let Some(victim) = victim else { break };
        r.session_events_deleted += conn.execute(
            "DELETE FROM session_events WHERE session_id = ?1 AND state_after IS NULL AND compacted = 0",
            [&victim],
        )?;
        r.session_events_compacted += conn.execute(
            "UPDATE session_events SET payload_json = NULL, compacted = 1 WHERE session_id = ?1 AND compacted = 0",
            [&victim],
        )?;
    }

    // sessions: `sessions_days` after `ended_at`; discovered rows `discovered_days` after their last event.
    let s_cutoff = shift(conn, now, &format!("-{} days", p.sessions_days))?;
    let d_cutoff = shift(conn, now, &format!("-{} days", p.discovered_days))?;
    r.sessions_deleted += conn.execute(
        "DELETE FROM sessions
          WHERE parent_session_id IS NULL
            AND ((ended_at IS NOT NULL AND ended_at < :s_cutoff)
              OR ((tracking = 'discovered' OR state = 'untracked') AND last_event_at < :d_cutoff))",
        named_params! { ":s_cutoff": s_cutoff, ":d_cutoff": d_cutoff },
    )?;

    // note_revisions: newest `keep` per note and nothing older than `days`.
    let nr_cutoff = shift(conn, now, &format!("-{} days", p.note_revisions_days))?;
    r.note_revisions_deleted += conn.execute(
        "DELETE FROM note_revisions WHERE created_at < :cutoff",
        named_params! { ":cutoff": nr_cutoff },
    )?;
    r.note_revisions_deleted += conn.execute(
        "DELETE FROM note_revisions WHERE id IN (
            SELECT id FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY note_id ORDER BY rev DESC) AS rn FROM note_revisions)
             WHERE rn > :keep)",
        named_params! { ":keep": p.note_revisions_keep },
    )?;

    // messages: metadata window from `received_at`, per-account cap, body window.
    let m_cutoff = shift(conn, now, &format!("-{} days", p.messages_days))?;
    let before: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
    r.messages_deleted += conn.execute(
        "DELETE FROM messages WHERE received_at < :cutoff",
        named_params! { ":cutoff": m_cutoff },
    )?;
    r.messages_deleted += conn.execute(
        "DELETE FROM messages WHERE id IN (
            SELECT id FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY account_id ORDER BY received_at DESC) AS rn FROM messages)
             WHERE rn > :cap)",
        named_params! { ":cap": p.messages_per_account_cap },
    )?;
    r.message_bodies_cleared += conn.execute(
        &format!(
            "UPDATE messages SET body_text = NULL, body_truncated = 0, body_retained_until = NULL
              WHERE body_text IS NOT NULL
                AND COALESCE(body_retained_until, {TS}, fetched_at, '+' || :days || ' days')) <= :now"
        ),
        named_params! { ":days": p.message_body_days, ":now": now },
    )?;
    let after: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))?;
    r.threads_deleted += conn.execute(
        "DELETE FROM threads WHERE NOT EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = threads.id)",
        [],
    )?;
    if before != after {
        r.threads_recomputed += conn.execute(
            "UPDATE threads SET
                message_count = (SELECT COUNT(*) FROM messages m WHERE m.thread_id = threads.id),
                unread_count  = (SELECT COUNT(*) FROM messages m WHERE m.thread_id = threads.id AND m.is_read = 0),
                last_message_at = (SELECT MAX(m.received_at) FROM messages m WHERE m.thread_id = threads.id),
                urgency_tier  = (SELECT m.urgency_tier FROM messages m WHERE m.thread_id = threads.id AND m.is_read = 0
                                  ORDER BY CASE m.urgency_tier WHEN 'vip' THEN 0 WHEN 'direct' THEN 1 WHEN 'ai-urgent' THEN 2
                                                               WHEN 'ai-normal' THEN 3 WHEN 'low' THEN 4 ELSE 5 END LIMIT 1),
                updated_at = :now",
            named_params! { ":now": now },
        )?;
    }

    // calendar_events: the −past / +future window around now.
    let c_past = shift(conn, now, &format!("-{} days", p.calendar_past_days))?;
    let c_future = shift(conn, now, &format!("+{} days", p.calendar_future_days))?;
    r.calendar_events_deleted += conn.execute(
        "DELETE FROM calendar_events WHERE ends_at < :past OR starts_at > :future",
        named_params! { ":past": c_past, ":future": c_future },
    )?;

    let u_cutoff = shift(conn, now, &format!("-{} days", p.usage_days))?;
    r.usage_samples_deleted += conn.execute(
        "DELETE FROM usage_samples WHERE sampled_at < :cutoff",
        named_params! { ":cutoff": u_cutoff },
    )?;

    let a_cutoff = shift(conn, now, &format!("-{} days", p.alert_log_days))?;
    r.alert_log_deleted += conn.execute(
        "DELETE FROM alert_log WHERE created_at < :cutoff",
        named_params! { ":cutoff": a_cutoff },
    )?;

    let w_cutoff = shift(conn, now, &format!("-{} hours", p.writebacks_hours))?;
    r.writebacks_deleted += conn.execute(
        "DELETE FROM connector_writebacks WHERE created_at < :cutoff",
        named_params! { ":cutoff": w_cutoff },
    )?;

    let ch_cutoff = shift(conn, now, &format!("-{} days", p.chat_turns_days))?;
    r.chat_turns_deleted += conn.execute(
        "DELETE FROM chat_turns WHERE created_at < :cutoff",
        named_params! { ":cutoff": ch_cutoff },
    )?;

    // automation_runs: newest `keep_runs` per automation and the day window; a running run is never pruned.
    let ar_cutoff = shift(conn, now, &format!("-{} days", p.automation_runs_days))?;
    r.automation_runs_deleted += conn.execute(
        "DELETE FROM automation_runs WHERE state <> 'running' AND started_at < :cutoff",
        named_params! { ":cutoff": ar_cutoff },
    )?;
    r.automation_runs_deleted += conn.execute(
        "DELETE FROM automation_runs WHERE state <> 'running' AND id IN (
            SELECT x.id FROM (SELECT id, automation_id,
                                     ROW_NUMBER() OVER (PARTITION BY automation_id ORDER BY started_at DESC) AS rn
                                FROM automation_runs) x
              JOIN automations a ON a.id = x.automation_id
             WHERE x.rn > a.keep_runs)",
        [],
    )?;

    r.activity_days_deleted += stats::rollup::prune(conn, clock, p.activity_days)?;

    // Dangling polymorphic references: tombstone links, cancel snoozes.
    r.links_tombstoned += conn.execute(
        &format!(
            "UPDATE links SET deleted_at = :now, updated_at = :now, rev = rev + 1, device_id = :device
              WHERE deleted_at IS NULL
                AND (NOT ({}) OR NOT ({}))",
            target_exists("from_kind", "from_id"),
            target_exists("to_kind", "to_id")
        ),
        named_params! { ":now": now, ":device": device_id },
    )?;
    r.snoozes_cancelled += conn.execute(
        &format!(
            "UPDATE snoozes SET cancelled_at = :now, updated_at = :now, rev = rev + 1, device_id = :device
              WHERE deleted_at IS NULL AND fired_at IS NULL AND cancelled_at IS NULL
                AND NOT ({})",
            target_exists("target_kind", "target_id")
        ),
        named_params! { ":now": now, ":device": device_id },
    )?;
    // media_plays (class L): the Music tab's history, 90 days and the newest 500 plays.
    r.media_plays_deleted +=
        crate::media_history::sweep_in(conn, now, p.media_plays_days, p.media_plays_max)?;
    // Removed suggestions whose thread, event or target is gone (class L, hard delete).
    conn.execute(
        &format!(
            "DELETE FROM link_dismissals WHERE NOT ({}) OR NOT ({})",
            target_exists("anchor_kind", "anchor_id"),
            target_exists("target_kind", "target_id")
        ),
        [],
    )?;

    // S-class tombstones past their window; fired / canceled snoozes likewise.
    let t_cutoff = shift(conn, now, &format!("-{} days", p.tombstone_days))?;
    for (table, extra) in TOMBSTONED {
        let n = conn.execute(
            &format!(
                "DELETE FROM {table} WHERE deleted_at IS NOT NULL AND deleted_at < :cutoff{extra}"
            ),
            named_params! { ":cutoff": t_cutoff },
        )?;
        if n > 0 {
            r.tombstones_purged.insert((*table).to_string(), n);
        }
    }
    let n = conn.execute(
        "DELETE FROM snoozes WHERE deleted_at IS NOT NULL AND deleted_at < :cutoff",
        named_params! { ":cutoff": t_cutoff },
    )?;
    if n > 0 {
        r.tombstones_purged.insert("snoozes".into(), n);
    }
    r.snoozes_expired_deleted += conn.execute(
        "DELETE FROM snoozes WHERE deleted_at IS NULL
            AND COALESCE(fired_at, cancelled_at) IS NOT NULL AND COALESCE(fired_at, cancelled_at) < :cutoff",
        named_params! { ":cutoff": t_cutoff },
    )?;

    // change_log: older than the window or beyond the newest N.
    let cl_cutoff = shift(conn, now, &format!("-{} days", p.change_log_days))?;
    r.change_log_pruned += conn.execute(
        "DELETE FROM change_log
          WHERE changed_at < :cutoff
             OR seq < (SELECT COALESCE(MIN(seq), 0) FROM (SELECT seq FROM change_log ORDER BY seq DESC LIMIT :max))",
        named_params! { ":cutoff": cl_cutoff, ":max": p.change_log_max },
    )?;

    Ok(r)
}

/// Prune pre-migration backups to `keep`; with `dry_run` only lists them.
pub fn prune_backups(dir: &Path, keep: usize, dry_run: bool) -> Result<Vec<PathBuf>> {
    if !dry_run {
        return db::prune_backups(dir, keep);
    }
    let mut names: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("mode-pre-") && n.ends_with(".db"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => return Ok(Vec::new()),
    };
    names.sort();
    let excess = names.len().saturating_sub(keep);
    Ok(names.into_iter().take(excess).collect())
}

/// Whether the sweep is due: never run, or more than 24 h since the last run.
pub fn is_due(conn: &Connection, clock: &Clock) -> Result<bool> {
    let last = settings::get_string_in(conn, settings::LAST_SWEEP_KEY)?;
    match last {
        None => Ok(true),
        Some(last) => {
            let cutoff = shift(conn, &clock.now, "-24 hours")?;
            Ok(last <= cutoff)
        }
    }
}

/// The whole nightly job on a [`Db`]: policy from settings, the sweep on the
/// writer, backups pruned, `maintenance.last_sweep_at` recorded (not in a dry run).
pub fn run(db: &Db, dry_run: bool) -> Result<Report> {
    run_with(db, dry_run, None)
}

/// [`run`], then import the history Claude Code's own stats cache holds for
/// the days Mode has not counted itself (ADR-0052) when `claude_stats` names
/// the file (the app passes [`stats::claude_cache::path`]; tests pass nothing).
pub fn run_with(db: &Db, dry_run: bool, claude_stats: Option<&Path>) -> Result<Report> {
    let (policy, clock) = db.read(|c| Ok((Policy::from_settings(c)?, Clock::current(c)?)))?;
    let device_id = db.device_id().to_string();
    let backups_dir = db.backups_dir().to_path_buf();
    let keep = policy.backups_kept;
    let keep_days = policy.activity_days;
    let clock_for_job = clock.clone();
    let mut report =
        db.write(move |conn| sweep(conn, &policy, &clock_for_job, &device_id, dry_run))?;
    report.backups_removed = prune_backups(&backups_dir, keep, dry_run)?;
    if let (false, Some(path)) = (dry_run, claude_stats) {
        let path = path.to_path_buf();
        let clock_for_import = clock.clone();
        let imported = db.write(move |conn| {
            stats::claude_cache::import_file(conn, &path, &clock_for_import, keep_days)
        })?;
        report.claude_stats_days = imported.days;
    }
    if !dry_run {
        settings::set(
            db,
            settings::LAST_SWEEP_KEY,
            &serde_json::Value::String(clock.now.clone()),
        )?;
    }
    tracing::info!(dry_run, rows = report.total_rows(), "retention sweep done");
    Ok(report)
}
