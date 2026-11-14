//! What the app window's Sessions view reads beyond the roster (03 §3.3,
//! §3.4, §3.5; 05 §12): the history rows (dismissed sessions within
//! `retention.sessions_days`), undismiss, the launch dialog's recent
//! folders, and session snoozes (`snoozes.target_kind = 'session'`) with
//! the preset times of 03 §4.7 and the S2 time sweep.
//!
//! History holds the dismissed rows only: ended rows that were not
//! dismissed are already on the roster (Q2 has no `ended_at` filter), so the
//! toggle adds exactly the rows the roster hides.

use rusqlite::{Connection, OptionalExtension, named_params};
use serde::{Deserialize, Serialize};

use super::Pending;
use super::views::SessionItem;
use crate::db::{DbError, Result};
use crate::queries::{Clock, DueSnooze, RosterRow};
use crate::repo::{self, Snooze};

/// Default and ceiling of a history page.
pub const HISTORY_PAGE: i64 = 200;
pub const HISTORY_MAX: i64 = 1_000;
/// Recent folders offered by the launch dialog.
pub const RECENT_FOLDERS: i64 = 20;

/// A history row: the roster row plus when it was dismissed
/// (`AppRosterRow.dismissed_at` in `contract-sessions.ts`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryItem {
    #[serde(flatten)]
    pub item: SessionItem,
    pub dismissed_at: Option<String>,
}

/// `sessions_history` arguments; every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryQuery {
    /// Rows per page (default 200, at most 1 000).
    #[serde(default)]
    pub limit: Option<i64>,
    /// Paging cursor: rows dismissed before this timestamp (the last
    /// row's `dismissed_at`).
    #[serde(default)]
    pub before: Option<String>,
    /// Substring over `display_name`, `title`, `cwd`, `git_branch`
    /// (03 §3.3 search), case-insensitive.
    #[serde(default)]
    pub query: Option<String>,
}

const HISTORY_SQL: &str = "
SELECT s.id, s.display_name, s.avatar_color, s.state, s.waiting_kind, s.title,
       s.cwd, s.repo_root, s.git_branch, s.model, s.current_tool,
       s.lines_added, s.lines_removed, s.files_changed, s.tracking, s.launch_mode,
       CAST((julianday(:now) - julianday(s.state_since)) * 86400 AS INTEGER) AS state_seconds,
       CASE WHEN s.state = 'done' AND s.ended_at IS NOT NULL
             AND (julianday(:now) - julianday(s.ended_at)) * 1440 > :fold_minutes THEN 1 ELSE 0 END AS folded,
       (SELECT COUNT(*) FROM sessions c WHERE c.parent_session_id = s.id AND c.ended_at IS NULL) AS live_agents,
       s.pending_json, s.end_reason, s.state_reason, s.current_mcp_server, s.claude_session_id,
       s.started_at, s.last_event_at, s.ended_at, s.acknowledged_at, s.last_assistant_preview,
       s.dismissed_at, s.avatar_variant
  FROM sessions s
 WHERE s.parent_session_id IS NULL AND s.dismissed_at IS NOT NULL
   AND COALESCE(s.ended_at, s.dismissed_at) >= strftime('%Y-%m-%dT%H:%M:%fZ', :now, '-' || :days || ' days')
   AND (:before IS NULL OR s.dismissed_at < :before)
   AND (:q IS NULL OR s.display_name LIKE :q ESCAPE '\\' OR COALESCE(s.title, '') LIKE :q ESCAPE '\\'
        OR s.cwd LIKE :q ESCAPE '\\' OR COALESCE(s.git_branch, '') LIKE :q ESCAPE '\\')
 ORDER BY s.dismissed_at DESC, s.id DESC
 LIMIT :limit";

fn like_pattern(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 2);
    out.push('%');
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// History rows (03 §3.3), most recently dismissed first. `retention_days`
/// is `retention.sessions_days` (90), `fold_minutes` the roster's fold.
pub fn history(
    conn: &Connection,
    clock: &Clock,
    fold_minutes: i64,
    retention_days: i64,
    q: &HistoryQuery,
) -> Result<Vec<HistoryItem>> {
    let limit = q.limit.unwrap_or(HISTORY_PAGE).clamp(1, HISTORY_MAX);
    let pattern = q
        .query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(like_pattern);
    let before = q.before.as_deref().filter(|b| !b.is_empty());
    let mut stmt = conn.prepare(HISTORY_SQL)?;
    let rows = stmt
        .query_map(
            named_params! {
                ":now": clock.now,
                ":fold_minutes": fold_minutes,
                ":days": retention_days.max(0),
                ":before": before,
                ":q": pattern,
                ":limit": limit,
            },
            |r| {
                let row = RosterRow {
                    id: r.get(0)?,
                    display_name: r.get(1)?,
                    avatar_color: r.get(2)?,
                    avatar_variant: r.get(30)?,
                    state: r.get(3)?,
                    waiting_kind: r.get(4)?,
                    title: r.get(5)?,
                    cwd: r.get(6)?,
                    repo_root: r.get(7)?,
                    git_branch: r.get(8)?,
                    model: r.get(9)?,
                    current_tool: r.get(10)?,
                    lines_added: r.get(11)?,
                    lines_removed: r.get(12)?,
                    files_changed: r.get(13)?,
                    tracking: r.get(14)?,
                    launch_mode: r.get(15)?,
                    state_seconds: r.get(16)?,
                    folded: r.get(17)?,
                    live_agents: r.get(18)?,
                };
                let pending_json: Option<String> = r.get(19)?;
                Ok(HistoryItem {
                    item: SessionItem {
                        row,
                        pending: pending_json.as_deref().and_then(Pending::from_json),
                        end_reason: r.get(20)?,
                        state_reason: r.get(21)?,
                        current_mcp_server: r.get(22)?,
                        claude_session_id: r.get(23)?,
                        started_at: r.get(24)?,
                        last_event_at: r.get(25)?,
                        ended_at: r.get(26)?,
                        acknowledged_at: r.get(27)?,
                        last_assistant_preview: r.get(28)?,
                    },
                    dismissed_at: r.get(29)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Bring a dismissed row back to the roster (the 8 s Undo of 05 §12, and
/// the history row's "restore"). `false` when it was not dismissed.
pub fn undismiss(conn: &Connection, session_id: &str, now: &str) -> Result<bool> {
    let exists: Option<Option<String>> = conn
        .query_row(
            "SELECT dismissed_at FROM sessions WHERE id = ?",
            [session_id],
            |r| r.get(0),
        )
        .optional()?;
    match exists {
        None => Err(DbError::NotFound {
            table: "sessions",
            key: session_id.to_string(),
        }),
        Some(None) => Ok(false),
        Some(Some(_)) => {
            conn.execute(
                "UPDATE sessions SET dismissed_at = NULL, updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, session_id],
            )?;
            Ok(true)
        }
    }
}

/// The launch dialog's recent folders (03 §3.5): distinct top-level
/// `sessions.cwd`, most recently started first.
pub fn recent_folders(conn: &Connection, limit: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT cwd FROM sessions
          WHERE parent_session_id IS NULL AND cwd <> ''
          GROUP BY cwd
          ORDER BY MAX(started_at) DESC
          LIMIT ?",
    )?;
    let rows = stmt
        .query_map([limit.max(1)], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// ------------------------------------------------------------------ snooze

/// The session snooze presets (03 §3.4 "presets as for messages", 03 §4.7;
/// `pick_date` / `until_reply` are message-only and come with an explicit
/// `until`).
pub const PRESETS: &[&str] = &["later_today", "tomorrow", "next_week"];

/// When a preset ends, in UTC, from `now` (UTC) and the device's local zone
/// (SQLite's `localtime`): later today = +3 h capped at 18:00 (after 15:00
/// it is 18:00, after 18:00 tomorrow 09:00); tomorrow 09:00; next week =
/// the next Monday 09:00. `None` for an unknown preset.
pub fn preset_until(conn: &Connection, now: &str, preset: &str) -> Result<Option<String>> {
    if !PRESETS.contains(&preset) {
        return Ok(None);
    }
    Ok(conn.query_row(
        "SELECT CASE :preset
           WHEN 'tomorrow' THEN strftime('%Y-%m-%dT%H:%M:%fZ', date(:now, 'localtime', '+1 day') || ' 09:00:00', 'utc')
           WHEN 'next_week' THEN strftime('%Y-%m-%dT%H:%M:%fZ', date(:now, 'localtime', '+1 day', 'weekday 1') || ' 09:00:00', 'utc')
           ELSE CASE
             WHEN time(:now, 'localtime') >= '18:00:00'
               THEN strftime('%Y-%m-%dT%H:%M:%fZ', date(:now, 'localtime', '+1 day') || ' 09:00:00', 'utc')
             WHEN time(:now, 'localtime') >= '15:00:00'
               THEN strftime('%Y-%m-%dT%H:%M:%fZ', date(:now, 'localtime') || ' 18:00:00', 'utc')
             ELSE strftime('%Y-%m-%dT%H:%M:%fZ', :now, '+3 hours')
           END
         END",
        named_params! { ":preset": preset, ":now": now },
        |r| r.get(0),
    )?)
}

/// Cancel the session's active snoozes (the Undo, or a new snooze
/// replacing the old one). Returns how many were canceled.
pub fn cancel_snooze(conn: &Connection, device_id: &str, session_id: &str) -> Result<usize> {
    let active = repo::query::<Snooze>(
        conn,
        "target_kind = 'session' AND target_id = ? AND fired_at IS NULL AND cancelled_at IS NULL AND deleted_at IS NULL",
        &[&session_id],
    )?;
    let now = crate::db::now_rfc3339();
    for s in &active {
        repo::patch::<Snooze>(conn, device_id, &s.id, &[("cancelled_at", &now)])?;
    }
    Ok(active.len())
}

/// Snooze a session until `until` (UTC): it leaves the pill and the alerts
/// until then (03 §3.4); an active snooze of the same session is replaced.
pub fn snooze(
    conn: &Connection,
    device_id: &str,
    session_id: &str,
    until: &str,
    preset: Option<&str>,
) -> Result<Snooze> {
    let known: Option<i64> = conn
        .query_row("SELECT 1 FROM sessions WHERE id = ?", [session_id], |r| {
            r.get(0)
        })
        .optional()?;
    if known.is_none() {
        return Err(DbError::NotFound {
            table: "sessions",
            key: session_id.to_string(),
        });
    }
    cancel_snooze(conn, device_id, session_id)?;
    let mut row = Snooze {
        id: repo::new_id(),
        target_kind: "session".into(),
        target_id: session_id.to_string(),
        until: until.to_string(),
        preset: preset.map(str::to_string),
        wake_on_reply: false,
        ..Default::default()
    };
    repo::create(conn, device_id, &mut row)?;
    Ok(row)
}

/// The active snooze of a session, if any.
pub fn active_snooze(conn: &Connection, clock: &Clock, session_id: &str) -> Result<Option<Snooze>> {
    Ok(repo::query::<Snooze>(
        conn,
        "target_kind = 'session' AND target_id = ? AND fired_at IS NULL AND cancelled_at IS NULL
           AND deleted_at IS NULL AND until > ? ORDER BY until DESC LIMIT 1",
        &[&session_id, &clock.now],
    )?
    .into_iter()
    .next())
}

/// The time half of the S2 sweep (07 §6): every snooze whose `until` has
/// passed gets `fired_at = now`, `fired_reason = 'time'`. Returns what fired
/// so the caller can push the affected areas.
pub fn fire_due(conn: &Connection, device_id: &str, clock: &Clock) -> Result<Vec<DueSnooze>> {
    let due = crate::queries::snoozes_due(conn, clock)?;
    for d in &due {
        repo::patch::<Snooze>(
            conn,
            device_id,
            &d.id,
            &[("fired_at", &clock.now), ("fired_reason", &"time")],
        )?;
    }
    Ok(due)
}
