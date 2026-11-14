//! The query cookbook of 09 §13 as typed functions: Q1–Q11 for the surfaces
//! and S1–S11 for the background jobs. Every function takes a `&Connection`
//! (a pooled reader, or the writer inside a job) and explicit parameters;
//! `:now` is an ISO-8601 UTC timestamp, `:today` / `:tomorrow` local dates,
//! computed by the app ([`Clock`]). Grouping views compile `filter_json`
//! through [`filter`]; search builds `MATCH` through [`fts`].

pub mod filter;
pub mod fts;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, named_params, params_from_iter};
use serde::{Deserialize, Serialize};

use crate::db::Result;
pub use filter::{Compiled, Filter, FilterContext, SortKey, compile, compile_sort};
pub use fts::{Fts, Query as FtsQuery, build as build_fts};

/// The three time parameters every surface query takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    /// ISO-8601 UTC, millisecond precision.
    pub now: String,
    /// Local `YYYY-MM-DD`.
    pub today: String,
    pub tomorrow: String,
}

impl Clock {
    /// Now, in the device's local zone for the dates (SQLite's `localtime`).
    pub fn current(conn: &Connection) -> Result<Clock> {
        Ok(conn.query_row(
            "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), date('now', 'localtime'), date('now', 'localtime', '+1 day')",
            [],
            |r| {
                Ok(Clock {
                    now: r.get(0)?,
                    today: r.get(1)?,
                    tomorrow: r.get(2)?,
                })
            },
        )?)
    }

    pub fn fixed(now: &str, today: &str, tomorrow: &str) -> Clock {
        Clock {
            now: now.into(),
            today: today.into(),
            tomorrow: tomorrow.into(),
        }
    }

    /// A [`FilterContext`] for this clock.
    pub fn filter_context(&self, closed_statuses: Vec<String>) -> FilterContext {
        FilterContext {
            today: self.today.clone(),
            tomorrow: self.tomorrow.clone(),
            closed_statuses,
        }
    }
}

fn rows<T>(
    conn: &Connection,
    sql: &str,
    params: &[(&str, &dyn rusqlite::ToSql)],
    f: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>> {
    let mut stmt = conn.prepare(sql)?;
    let out = stmt
        .query_map(params, f)?
        .collect::<rusqlite::Result<Vec<T>>>()?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Q1 — overlay compact pill: "the first thing that applies"
// ---------------------------------------------------------------------------

/// A pill candidate; `priority` 1 session waiting, 2 urgent message, 3 session
/// finished (failed first), 4 active session count (`label` is the count),
/// 5 next event. `None` means the overlay stays hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PillCandidate {
    pub priority: i64,
    pub kind: String,
    pub r#ref: Option<String>,
    pub label: String,
    pub at: Option<String>,
}

/// The candidate CTE shared by Q1 and the "+n" count of 02 §5.2.
const Q1_CANDIDATES: &str = "
WITH snoozed(kind, target_id) AS (
  SELECT target_kind, target_id FROM snoozes
   WHERE fired_at IS NULL AND cancelled_at IS NULL AND deleted_at IS NULL AND until > :now
),
candidates(priority, kind, ref, label, at) AS (
  SELECT 1, 'session_waiting', s.id, s.display_name, s.state_since
    FROM sessions s
   WHERE s.state = 'waiting' AND s.dismissed_at IS NULL AND s.parent_session_id IS NULL
     AND NOT EXISTS (SELECT 1 FROM snoozed z WHERE z.kind = 'session' AND z.target_id = s.id)
  UNION ALL
  SELECT 2, 'urgent_message', m.id, m.sender_name || ': ' || COALESCE(m.subject, m.snippet), m.received_at
    FROM messages m
   WHERE m.urgency_tier IN ('vip','direct','ai-urgent') AND m.is_read = 0 AND m.is_archived = 0
     AND m.received_at > strftime('%Y-%m-%dT%H:%M:%fZ', :now, '-' || :max_age_hours || ' hours')
     AND NOT EXISTS (SELECT 1 FROM snoozed z WHERE z.kind = 'message' AND z.target_id = m.id)
     AND NOT EXISTS (SELECT 1 FROM snoozed z WHERE z.kind = 'thread' AND z.target_id = m.thread_id)
  UNION ALL
  SELECT 3, 'session_finished', s.id, s.display_name,
         CASE WHEN s.state = 'failed' THEN '0000' ELSE COALESCE(s.ended_at, s.state_since) END
    FROM sessions s
   WHERE s.state IN ('failed','unread') AND s.dismissed_at IS NULL AND s.parent_session_id IS NULL
     AND NOT EXISTS (SELECT 1 FROM snoozed z WHERE z.kind = 'session' AND z.target_id = s.id)
  UNION ALL
  SELECT 4, 'active_count', NULL, CAST(COUNT(*) AS TEXT), MAX(s.last_event_at)
    FROM sessions s
   WHERE s.state IN ('working','idle') AND s.ended_at IS NULL AND s.parent_session_id IS NULL
  HAVING COUNT(*) > 0
  UNION ALL
  SELECT 5, 'next_event', e.id, e.title, e.starts_at
    FROM calendar_events e
   WHERE e.status <> 'cancelled' AND e.my_response IS NOT 'declined'
     AND e.starts_at > :now AND e.starts_at <= strftime('%Y-%m-%dT%H:%M:%fZ', :now, '+2 hours')
)";

const Q1: &str =
    "SELECT priority, kind, ref, label, at FROM candidates ORDER BY priority, at LIMIT 1";

/// Q1. `max_age_hours` is `urgency.pill_message_max_age_hours` (24).
pub fn pill_candidate(
    conn: &Connection,
    clock: &Clock,
    max_age_hours: i64,
) -> Result<Option<PillCandidate>> {
    let sql = format!("{Q1_CANDIDATES}\n{Q1}");
    Ok(conn
        .query_row(
            &sql,
            named_params! { ":now": clock.now, ":max_age_hours": max_age_hours },
            |r| {
                Ok(PillCandidate {
                    priority: r.get(0)?,
                    kind: r.get(1)?,
                    r#ref: r.get(2)?,
                    label: r.get(3)?,
                    at: r.get(4)?,
                })
            },
        )
        .optional()?)
}

/// How many Q1 candidates share `priority` (the overlay's "+n" is this
/// minus one; 02 §5.2). `active_count` is one candidate by construction.
pub fn pill_count(
    conn: &Connection,
    clock: &Clock,
    max_age_hours: i64,
    priority: i64,
) -> Result<i64> {
    let sql =
        format!("{Q1_CANDIDATES}\nSELECT COUNT(*) FROM candidates WHERE priority = :priority");
    Ok(conn.query_row(
        &sql,
        named_params! { ":now": clock.now, ":max_age_hours": max_age_hours, ":priority": priority },
        |r| r.get(0),
    )?)
}

// ---------------------------------------------------------------------------
// Q2 — roster
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RosterRow {
    pub id: String,
    pub display_name: String,
    pub avatar_color: String,
    /// 05 §11.2: the accessory the bot mark wears for this session, `""` for the plain mark.
    pub avatar_variant: String,
    pub state: String,
    pub waiting_kind: Option<String>,
    pub title: Option<String>,
    pub cwd: String,
    pub repo_root: Option<String>,
    pub git_branch: Option<String>,
    pub model: Option<String>,
    pub current_tool: Option<String>,
    pub lines_added: i64,
    pub lines_removed: i64,
    pub files_changed: i64,
    pub tracking: String,
    pub launch_mode: String,
    pub state_seconds: i64,
    pub folded: bool,
    pub live_agents: i64,
}

const Q2: &str = "
SELECT s.id, s.display_name, s.avatar_color, s.state, s.waiting_kind, s.title,
       s.cwd, s.repo_root, s.git_branch, s.model, s.current_tool,
       s.lines_added, s.lines_removed, s.files_changed, s.tracking, s.launch_mode,
       CAST((julianday(:now) - julianday(s.state_since)) * 86400 AS INTEGER) AS state_seconds,
       CASE WHEN s.state = 'done' AND s.ended_at IS NOT NULL
             AND (julianday(:now) - julianday(s.ended_at)) * 1440 > :fold_minutes THEN 1 ELSE 0 END AS folded,
       (SELECT COUNT(*) FROM sessions c WHERE c.parent_session_id = s.id AND c.ended_at IS NULL) AS live_agents,
       s.avatar_variant
  FROM sessions s
 WHERE s.parent_session_id IS NULL AND s.dismissed_at IS NULL
 ORDER BY CASE s.state WHEN 'waiting' THEN 0 WHEN 'working' THEN 1 WHEN 'failed' THEN 2 WHEN 'unread' THEN 3
                       WHEN 'idle' THEN 4 WHEN 'done' THEN 5 WHEN 'untracked' THEN 6 ELSE 7 END,
          s.last_event_at DESC";

/// Q2. `fold_minutes` is `sessions.fold_after_minutes` (10).
pub fn roster(conn: &Connection, clock: &Clock, fold_minutes: i64) -> Result<Vec<RosterRow>> {
    rows(
        conn,
        Q2,
        named_params! { ":now": clock.now, ":fold_minutes": fold_minutes },
        |r| {
            Ok(RosterRow {
                id: r.get(0)?,
                display_name: r.get(1)?,
                avatar_color: r.get(2)?,
                avatar_variant: r.get(19)?,
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
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Q3 — urgent inbox
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UrgentMessage {
    pub id: String,
    pub provider: String,
    pub account: String,
    pub account_color: Option<String>,
    pub kind: String,
    pub channel_name: Option<String>,
    pub thread_title: Option<String>,
    pub sender_name: String,
    pub sender_avatar_url: Option<String>,
    pub vip: bool,
    pub subject: Option<String>,
    pub snippet: String,
    pub received_at: String,
    pub urgency_tier: String,
    pub urgency_why: Option<String>,
    pub source_url: String,
    pub dedup_key: String,
}

const Q3: &str = "
SELECT m.id, m.provider, a.display_name AS account, a.color AS account_color,
       m.kind, m.channel_name, t.title AS thread_title,
       m.sender_name, m.sender_avatar_url, COALESCE(c.vip, 0) AS vip,
       m.subject, m.snippet, m.received_at, m.urgency_tier, m.urgency_why, m.source_url, m.dedup_key
  FROM messages m
  JOIN accounts a ON a.id = m.account_id
  LEFT JOIN threads t ON t.id = m.thread_id
  LEFT JOIN contacts c ON c.id = m.sender_contact_id
 WHERE m.is_read = 0 AND m.is_archived = 0
   AND m.urgency_tier IN ('vip','direct','ai-urgent')
   AND NOT EXISTS (SELECT 1 FROM snoozes z
                    WHERE z.fired_at IS NULL AND z.cancelled_at IS NULL AND z.deleted_at IS NULL AND z.until > :now
                      AND ((z.target_kind = 'message' AND z.target_id = m.id)
                        OR (z.target_kind = 'thread'  AND z.target_id = m.thread_id)))
 ORDER BY CASE m.urgency_tier WHEN 'vip' THEN 0 WHEN 'direct' THEN 1 ELSE 2 END, m.received_at DESC
 LIMIT :limit";

/// Q3: unread, not archived, not snoozed, tiers vip/direct/ai-urgent, important people first.
/// Twins (same `dedup_key`) are returned as rows; the app groups them.
pub fn urgent_inbox(conn: &Connection, clock: &Clock, limit: i64) -> Result<Vec<UrgentMessage>> {
    rows(
        conn,
        Q3,
        named_params! { ":now": clock.now, ":limit": limit },
        |r| {
            Ok(UrgentMessage {
                id: r.get(0)?,
                provider: r.get(1)?,
                account: r.get(2)?,
                account_color: r.get(3)?,
                kind: r.get(4)?,
                channel_name: r.get(5)?,
                thread_title: r.get(6)?,
                sender_name: r.get(7)?,
                sender_avatar_url: r.get(8)?,
                vip: r.get(9)?,
                subject: r.get(10)?,
                snippet: r.get(11)?,
                received_at: r.get(12)?,
                urgency_tier: r.get(13)?,
                urgency_why: r.get(14)?,
                source_url: r.get(15)?,
                dedup_key: r.get(16)?,
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Q4 — grouping views
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteCard {
    pub id: String,
    pub title: String,
    pub status: String,
    pub urgent: bool,
    pub when_date: Option<String>,
    pub path: String,
    pub excerpt: String,
    pub task_total: i64,
    pub task_done: i64,
    pub pinned: bool,
    pub archived_at: Option<String>,
    pub folder_id: Option<String>,
    pub updated_at: String,
    /// `overdue` | `today` | `tomorrow` | `scheduled` | `none`.
    pub when_bucket: String,
    /// Tag display names, space-joined, in frontmatter order.
    pub tag_names: Option<String>,
}

/// Q4: the notes of a grouping (or of the query bar), compiled from its filter and sort.
pub fn grouping_notes(
    conn: &Connection,
    filter: &Filter,
    sort: &[SortKey],
    ctx: &FilterContext,
    limit: i64,
) -> Result<Vec<NoteCard>> {
    let mut params: Vec<SqlValue> = Vec::new();
    let bucket = filter::when_bucket_case(ctx, &mut params);
    let compiled = compile(filter, ctx)?;
    params.extend(compiled.params);
    params.push(SqlValue::Integer(limit));
    let sql = format!(
        "SELECT n.id, n.title, n.status, n.urgent, n.when_date, n.path, n.excerpt, n.task_total, n.task_done,
                n.pinned, n.archived_at, n.folder_id, n.updated_at,
                {bucket} AS when_bucket,
                (SELECT group_concat(t.name, ' ' ORDER BY nt.position) FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
                  WHERE nt.note_id = n.id) AS tag_names
           FROM notes n
          WHERE {}
          ORDER BY {}
          LIMIT ?",
        compiled.where_sql,
        compile_sort(sort)?
    );
    let mut stmt = conn.prepare(&sql)?;
    let out = stmt
        .query_map(params_from_iter(params), |r| {
            Ok(NoteCard {
                id: r.get(0)?,
                title: r.get(1)?,
                status: r.get(2)?,
                urgent: r.get(3)?,
                when_date: r.get(4)?,
                path: r.get(5)?,
                excerpt: r.get(6)?,
                task_total: r.get(7)?,
                task_done: r.get(8)?,
                pinned: r.get(9)?,
                archived_at: r.get(10)?,
                folder_id: r.get(11)?,
                updated_at: r.get(12)?,
                when_bucket: r.get(13)?,
                tag_names: r.get(14)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(out)
}

/// Q4: the count badge for a grouping's card header.
pub fn grouping_count(conn: &Connection, filter: &Filter, ctx: &FilterContext) -> Result<i64> {
    let compiled = compile(filter, ctx)?;
    let sql = format!("SELECT COUNT(*) FROM notes n WHERE {}", compiled.where_sql);
    Ok(conn.query_row(&sql, params_from_iter(compiled.params), |r| r.get(0))?)
}

/// Q4 for a stored grouping row: parse its `filter_json` / `sort_json` and run it.
pub fn grouping_view(
    conn: &Connection,
    grouping: &crate::repo::Grouping,
    ctx: &FilterContext,
    limit: i64,
) -> Result<Vec<NoteCard>> {
    let f = Filter::parse(&grouping.filter_json)?;
    let s = filter::parse_sort(&grouping.sort_json)?;
    grouping_notes(conn, &f, &s, ctx, limit)
}

// ---------------------------------------------------------------------------
// Q5 / Q6 — search
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteHit {
    pub id: String,
    pub title: String,
    pub path: String,
    pub status: String,
    pub urgent: bool,
    pub when_date: Option<String>,
    pub archived_at: Option<String>,
    pub snippet: String,
    pub rank: f64,
}

const Q5: &str = "
SELECT n.id, n.title, n.path, n.status, n.urgent, n.when_date, n.archived_at,
       snippet(notes_fts, 1, '[', ']', '…', 12) AS snippet,
       bm25(notes_fts, 4.0, 1.0, 2.0, 0.0) AS rank
  FROM notes_fts
  JOIN notes n ON n.seq = notes_fts.rowid
 WHERE notes_fts MATCH :query AND n.deleted_at IS NULL
   AND (:include_archived = 1 OR n.archived_at IS NULL)
   AND (:scope IS NULL OR n.path LIKE :scope || '%')
 ORDER BY rank
 LIMIT :limit";

/// Q5: notes search with ranking and a highlighted snippet. `text` is the raw
/// query bar text (built through [`build_fts`]); empty text returns nothing.
pub fn search_notes(
    conn: &Connection,
    text: &str,
    prefix_last: bool,
    include_archived: bool,
    limit: i64,
) -> Result<Vec<NoteHit>> {
    let Some(q) = build_fts(text, Fts::Notes, prefix_last) else {
        return Ok(Vec::new());
    };
    let Some(match_expr) = q.match_expr else {
        return Ok(Vec::new());
    };
    rows(
        conn,
        Q5,
        named_params! {
            ":query": match_expr,
            ":include_archived": include_archived,
            ":scope": q.in_scope,
            ":limit": limit,
        },
        |r| {
            Ok(NoteHit {
                id: r.get(0)?,
                title: r.get(1)?,
                path: r.get(2)?,
                status: r.get(3)?,
                urgent: r.get(4)?,
                when_date: r.get(5)?,
                archived_at: r.get(6)?,
                snippet: r.get(7)?,
                rank: r.get(8)?,
            })
        },
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageHit {
    pub id: String,
    pub sender_name: String,
    pub subject: Option<String>,
    pub snippet: String,
    pub received_at: String,
    pub urgency_tier: String,
    pub account: String,
    pub hit: String,
    pub rank: f64,
}

const Q6: &str = "
SELECT m.id, m.sender_name, m.subject, m.snippet, m.received_at, m.urgency_tier, a.display_name AS account,
       COALESCE(snippet(messages_fts, 2, '[', ']', '…', 12), snippet(messages_fts, -1, '[', ']', '…', 12)) AS hit,
       bm25(messages_fts, 3.0, 1.0, 1.0, 2.0) AS rank
  FROM messages_fts
  JOIN messages m ON m.seq = messages_fts.rowid
  JOIN accounts a ON a.id = m.account_id
 WHERE messages_fts MATCH :query
   AND (:account_id IS NULL OR m.account_id = :account_id)
   AND (:scope IS NULL OR m.channel_name = :scope)
 ORDER BY rank
 LIMIT :limit";

/// Q6: messages search, optionally scoped to one account.
pub fn search_messages(
    conn: &Connection,
    text: &str,
    prefix_last: bool,
    account_id: Option<&str>,
    limit: i64,
) -> Result<Vec<MessageHit>> {
    let Some(q) = build_fts(text, Fts::Messages, prefix_last) else {
        return Ok(Vec::new());
    };
    let Some(match_expr) = q.match_expr else {
        return Ok(Vec::new());
    };
    rows(
        conn,
        Q6,
        named_params! {
            ":query": match_expr,
            ":account_id": account_id,
            ":scope": q.in_scope,
            ":limit": limit,
        },
        |r| {
            Ok(MessageHit {
                id: r.get(0)?,
                sender_name: r.get(1)?,
                subject: r.get(2)?,
                snippet: r.get(3)?,
                received_at: r.get(4)?,
                urgency_tier: r.get(5)?,
                account: r.get(6)?,
                hit: r.get(7)?,
                rank: r.get(8)?,
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Q7 — session detail
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub seq: i64,
    pub occurred_at: String,
    pub source: String,
    pub event: String,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub agent_id: Option<String>,
    pub state_before: Option<String>,
    pub state_after: Option<String>,
    pub summary: Option<String>,
    pub payload_json: Option<String>,
}

const Q7_EVENTS: &str = "
SELECT e.seq, e.occurred_at, e.source, e.event, e.tool_name, e.tool_use_id, e.agent_id,
       e.state_before, e.state_after, e.summary, e.payload_json
  FROM session_events e
 WHERE e.session_id = :session_id AND e.seq < :before_seq
 ORDER BY e.seq DESC
 LIMIT :limit";

/// Q7: the Overview timeline, paged newest first (`before_seq = None` for the first page).
pub fn session_timeline(
    conn: &Connection,
    session_id: &str,
    before_seq: Option<i64>,
    limit: i64,
) -> Result<Vec<TimelineEvent>> {
    rows(
        conn,
        Q7_EVENTS,
        named_params! {
            ":session_id": session_id,
            ":before_seq": before_seq.unwrap_or(i64::MAX),
            ":limit": limit,
        },
        |r| {
            Ok(TimelineEvent {
                seq: r.get(0)?,
                occurred_at: r.get(1)?,
                source: r.get(2)?,
                event: r.get(3)?,
                tool_name: r.get(4)?,
                tool_use_id: r.get(5)?,
                agent_id: r.get(6)?,
                state_before: r.get(7)?,
                state_after: r.get(8)?,
                summary: r.get(9)?,
                payload_json: r.get(10)?,
            })
        },
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionFile {
    pub id: String,
    pub path: String,
    pub rel_path: Option<String>,
    pub change_kind: String,
    pub old_path: Option<String>,
    pub lines_added: i64,
    pub lines_removed: i64,
    pub edit_count: i64,
    pub source: String,
    pub last_seen_at: String,
}

const Q7_FILES: &str = "
SELECT f.id, f.path, f.rel_path, f.change_kind, f.old_path, f.lines_added, f.lines_removed,
       f.edit_count, f.source, f.last_seen_at
  FROM file_changes f
 WHERE f.session_id = :session_id
 ORDER BY f.last_seen_at DESC";

/// Q7: the Files tab.
pub fn session_files(conn: &Connection, session_id: &str) -> Result<Vec<SessionFile>> {
    rows(
        conn,
        Q7_FILES,
        named_params! { ":session_id": session_id },
        |r| {
            Ok(SessionFile {
                id: r.get(0)?,
                path: r.get(1)?,
                rel_path: r.get(2)?,
                change_kind: r.get(3)?,
                old_path: r.get(4)?,
                lines_added: r.get(5)?,
                lines_removed: r.get(6)?,
                edit_count: r.get(7)?,
                source: r.get(8)?,
                last_seen_at: r.get(9)?,
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Q8 — inbox list by thread and thread view
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxThread {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub channel_name: Option<String>,
    pub unread_count: i64,
    pub message_count: i64,
    pub last_message_at: Option<String>,
    pub last_snippet: Option<String>,
    pub urgency_tier: Option<String>,
    pub account: String,
    pub sender_name: Option<String>,
    pub sender_avatar_url: Option<String>,
}

const Q8_THREADS: &str = "
SELECT t.id, t.kind, t.title, t.channel_name, t.unread_count, t.message_count,
       t.last_message_at, t.last_snippet, t.urgency_tier, a.display_name AS account,
       m.sender_name, m.sender_avatar_url
  FROM threads t
  JOIN accounts a ON a.id = t.account_id
  LEFT JOIN messages m ON m.id = (SELECT id FROM messages WHERE thread_id = t.id ORDER BY received_at DESC LIMIT 1)
 WHERE t.is_archived = 0 AND (:account_id IS NULL OR t.account_id = :account_id)
 ORDER BY t.last_message_at DESC
 LIMIT :limit";

/// Q8: inbox rows are threads with their latest message.
pub fn inbox_threads(
    conn: &Connection,
    account_id: Option<&str>,
    limit: i64,
) -> Result<Vec<InboxThread>> {
    rows(
        conn,
        Q8_THREADS,
        named_params! { ":account_id": account_id, ":limit": limit },
        |r| {
            Ok(InboxThread {
                id: r.get(0)?,
                kind: r.get(1)?,
                title: r.get(2)?,
                channel_name: r.get(3)?,
                unread_count: r.get(4)?,
                message_count: r.get(5)?,
                last_message_at: r.get(6)?,
                last_snippet: r.get(7)?,
                urgency_tier: r.get(8)?,
                account: r.get(9)?,
                sender_name: r.get(10)?,
                sender_avatar_url: r.get(11)?,
            })
        },
    )
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub id: String,
    pub sender_name: String,
    pub sender_avatar_url: Option<String>,
    pub sender_address: String,
    pub snippet: String,
    pub body_text: Option<String>,
    pub body_truncated: bool,
    pub received_at: String,
    pub is_read: bool,
    pub urgency_tier: String,
    pub urgency_why: Option<String>,
    pub source_url: String,
}

const Q8_MESSAGES: &str = "
SELECT m.id, m.sender_name, m.sender_avatar_url, m.sender_address, m.snippet, m.body_text, m.body_truncated,
       m.received_at, m.is_read, m.urgency_tier, m.urgency_why, m.source_url
  FROM messages m
 WHERE m.thread_id = :thread_id
 ORDER BY m.received_at";

/// Q8: the messages of one thread, oldest first.
pub fn thread_messages(conn: &Connection, thread_id: &str) -> Result<Vec<ThreadMessage>> {
    rows(
        conn,
        Q8_MESSAGES,
        named_params! { ":thread_id": thread_id },
        |r| {
            Ok(ThreadMessage {
                id: r.get(0)?,
                sender_name: r.get(1)?,
                sender_avatar_url: r.get(2)?,
                sender_address: r.get(3)?,
                snippet: r.get(4)?,
                body_text: r.get(5)?,
                body_truncated: r.get(6)?,
                received_at: r.get(7)?,
                is_read: r.get(8)?,
                urgency_tier: r.get(9)?,
                urgency_why: r.get(10)?,
                source_url: r.get(11)?,
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Q9 — agenda / next event
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgendaEvent {
    pub id: String,
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    pub all_day: bool,
    pub location: Option<String>,
    pub join_url: Option<String>,
    pub conference: Option<String>,
    pub status: String,
    pub my_response: Option<String>,
    pub account: String,
    pub account_color: Option<String>,
    pub prepare_note_id: Option<String>,
}

const Q9: &str = "
SELECT e.id, e.title, e.starts_at, e.ends_at, e.all_day, e.location, e.join_url, e.conference,
       e.status, e.my_response, a.display_name AS account, a.color AS account_color,
       (SELECT l.to_id FROM links l
         WHERE l.from_kind = 'calendar_event' AND l.from_id = e.id
           AND l.to_kind = 'note' AND l.link_type = 'prepare_for' AND l.deleted_at IS NULL
         LIMIT 1) AS prepare_note_id
  FROM calendar_events e
  JOIN accounts a ON a.id = e.account_id
 WHERE e.status <> 'cancelled' AND e.my_response IS NOT 'declined'
   AND e.ends_at >= :day_start AND e.starts_at < :day_end
 ORDER BY e.all_day DESC, e.starts_at";

/// Q9: the events overlapping `[day_start, day_end)` with their prepare-note.
pub fn agenda(conn: &Connection, day_start: &str, day_end: &str) -> Result<Vec<AgendaEvent>> {
    rows(
        conn,
        Q9,
        named_params! { ":day_start": day_start, ":day_end": day_end },
        |r| {
            Ok(AgendaEvent {
                id: r.get(0)?,
                title: r.get(1)?,
                starts_at: r.get(2)?,
                ends_at: r.get(3)?,
                all_day: r.get(4)?,
                location: r.get(5)?,
                join_url: r.get(6)?,
                conference: r.get(7)?,
                status: r.get(8)?,
                my_response: r.get(9)?,
                account: r.get(10)?,
                account_color: r.get(11)?,
                prepare_note_id: r.get(12)?,
            })
        },
    )
}

/// Q9 variant: the next event starting after `now` (within `hours`).
pub fn next_event(conn: &Connection, clock: &Clock, hours: i64) -> Result<Option<AgendaEvent>> {
    let end: String = conn.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', ?1, '+' || ?2 || ' hours')",
        rusqlite::params![clock.now, hours],
        |r| r.get(0),
    )?;
    let mut list = agenda(conn, &clock.now, &end)?;
    list.retain(|e| e.starts_at > clock.now);
    list.sort_by(|a, b| a.starts_at.cmp(&b.starts_at));
    Ok(list.into_iter().next())
}

// ---------------------------------------------------------------------------
// Q10 — usage (tokens only; cost_usd is stored, never summed for display)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRow {
    pub source: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub turns: i64,
}

const Q10: &str = "
SELECT u.source, u.model,
       SUM(u.input_tokens) AS input_tokens, SUM(u.output_tokens) AS output_tokens,
       SUM(u.cache_read_tokens) AS cache_read_tokens, SUM(u.cache_write_tokens) AS cache_write_tokens,
       COUNT(*) AS turns
  FROM usage_samples u
 WHERE u.sampled_at >= :window_start AND u.sampled_at < :window_end
 GROUP BY u.source, u.model
 ORDER BY input_tokens + output_tokens DESC";

/// Q10: the gauge for `[window_start, window_end)` per source and model.
pub fn usage_window(
    conn: &Connection,
    window_start: &str,
    window_end: &str,
) -> Result<Vec<UsageRow>> {
    rows(
        conn,
        Q10,
        named_params! { ":window_start": window_start, ":window_end": window_end },
        |r| {
            Ok(UsageRow {
                source: r.get(0)?,
                model: r.get(1)?,
                input_tokens: r.get(2)?,
                output_tokens: r.get(3)?,
                cache_read_tokens: r.get(4)?,
                cache_write_tokens: r.get(5)?,
                turns: r.get(6)?,
            })
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageDay {
    pub day: String,
    pub source: String,
    pub tokens: i64,
    pub turns: i64,
}

const Q10_DAYS: &str = "
SELECT date(u.sampled_at, 'localtime') AS day, u.source,
       SUM(u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens) AS tokens, COUNT(*) AS turns
  FROM usage_samples u
 WHERE u.sampled_at >= :window_start AND u.sampled_at < :window_end
 GROUP BY day, u.source
 ORDER BY day";

/// Q10: the Stats view's tokens-per-day bars.
pub fn usage_by_day(
    conn: &Connection,
    window_start: &str,
    window_end: &str,
) -> Result<Vec<UsageDay>> {
    rows(
        conn,
        Q10_DAYS,
        named_params! { ":window_start": window_start, ":window_end": window_end },
        |r| {
            Ok(UsageDay {
                day: r.get(0)?,
                source: r.get(1)?,
                tokens: r.get(2)?,
                turns: r.get(3)?,
            })
        },
    )
}

/// One day's tokens for one source and model (the Stats bars' hover split).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSplit {
    pub day: String,
    pub source: String,
    pub model: String,
    pub tokens: i64,
}

const Q10_SPLIT: &str = "
SELECT date(u.sampled_at, 'localtime') AS day, u.source, u.model,
       SUM(u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens) AS tokens
  FROM usage_samples u
 WHERE u.sampled_at >= :window_start AND u.sampled_at < :window_end
 GROUP BY day, u.source, u.model
 ORDER BY day, tokens DESC";

/// Q10 grouped by local day, source and model (03 §7A.2 "Tokens per day").
/// Tokens only: `cost_usd` is never read.
pub fn usage_split_by_day(
    conn: &Connection,
    window_start: &str,
    window_end: &str,
) -> Result<Vec<UsageSplit>> {
    rows(
        conn,
        Q10_SPLIT,
        named_params! { ":window_start": window_start, ":window_end": window_end },
        |r| {
            Ok(UsageSplit {
                day: r.get(0)?,
                source: r.get(1)?,
                model: r.get(2)?,
                tokens: r.get(3)?,
            })
        },
    )
}

/// The UTC instants bounding the local days `from_day ..= to_day`
/// (`YYYY-MM-DD`): local midnight of `from_day` and of the day after `to_day`.
pub fn local_day_bounds(
    conn: &Connection,
    from_day: &str,
    to_day: &str,
) -> Result<(String, String)> {
    Ok(conn.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ', :from || ' 00:00:00', 'utc'),
                strftime('%Y-%m-%dT%H:%M:%fZ', date(:to, '+1 day') || ' 00:00:00', 'utc')",
        named_params! { ":from": from_day, ":to": to_day },
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// Top-level sessions with activity (`last_event_at`) in
/// `[window_start, window_end)`, counted by `state` (03 §7A.2 "Sessions by
/// state"; the app folds `unread` + `done` into "finished").
pub fn sessions_by_state(
    conn: &Connection,
    window_start: &str,
    window_end: &str,
) -> Result<Vec<(String, i64)>> {
    rows(
        conn,
        "SELECT s.state, COUNT(*) FROM sessions s
          WHERE s.parent_session_id IS NULL
            AND s.last_event_at >= :window_start AND s.last_event_at < :window_end
          GROUP BY s.state ORDER BY s.state",
        named_params! { ":window_start": window_start, ":window_end": window_end },
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
}

/// The start of the usage week containing `now` (03 §3.12, 05 §13): local
/// midnight of the latest `weekday` (0 = Sunday … 6 = Saturday, SQLite's
/// numbering) on or before today, as UTC.
pub fn usage_week_start(conn: &Connection, now: &str, weekday: u8) -> Result<String> {
    Ok(conn.query_row(
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ',
                         date(:now, 'localtime', '-6 days', 'weekday ' || :wd) || ' 00:00:00', 'utc')",
        named_params! { ":now": now, ":wd": i64::from(weekday.min(6)) },
        |r| r.get(0),
    )?)
}

/// Q10: Claude Code tokens (input + output + cache read + cache write) from
/// `since` on, the profile row's "1.2M tokens this week" (03 §2.3.1).
pub fn claude_code_tokens_since(conn: &Connection, since: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens), 0)
           FROM usage_samples
          WHERE source = 'claude_code' AND sampled_at >= :since",
        named_params! { ":since": since },
        |r| r.get(0),
    )?)
}

/// Q10: Mode-side AI tokens since `since` for one `purpose` (or all), against
/// `ai.budget_tokens_month` (and the daily scoring pace cap with `purpose = Some("scoring")`).
pub fn ai_tokens_used(conn: &Connection, since: &str, purpose: Option<&str>) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens), 0)
           FROM usage_samples
          WHERE source IN ('claude_api', 'openai_compat') AND sampled_at >= :since AND (:purpose IS NULL OR purpose = :purpose)",
        named_params! { ":since": since, ":purpose": purpose },
        |r| r.get(0),
    )?)
}

// ---------------------------------------------------------------------------
// Q11 — Stats day tracker
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityRow {
    pub day: String,
    pub sessions_started: i64,
    pub files_changed: i64,
    pub tokens: i64,
    /// `sessions_started + files_changed`; the app maps it to lvl-0 … lvl-4 by quartile.
    pub activity: i64,
}

const Q11: &str = "
WITH live(day, sessions_started, files_changed, tokens) AS (
  SELECT date(s.started_at, 'localtime'), 1, 0, 0
    FROM sessions s
   WHERE s.parent_session_id IS NULL AND date(s.started_at, 'localtime') >= :cutoff_day
  UNION ALL
  SELECT date(f.first_seen_at, 'localtime'), 0, 1, 0
    FROM file_changes f
   WHERE date(f.first_seen_at, 'localtime') >= :cutoff_day
  UNION ALL
  SELECT date(u.sampled_at, 'localtime'), 0, 0,
         u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens
    FROM usage_samples u
   WHERE u.source = 'claude_code' AND date(u.sampled_at, 'localtime') >= :cutoff_day
),
rolled(day, sessions_started, files_changed, tokens) AS (
  SELECT day, sessions_started, files_changed, tokens
    FROM activity_days
   WHERE day >= :from_day AND day < :cutoff_day
)
SELECT day,
       SUM(sessions_started)                       AS sessions_started,
       SUM(files_changed)                          AS files_changed,
       SUM(tokens)                                 AS tokens,
       SUM(sessions_started) + SUM(files_changed)  AS activity
  FROM (SELECT * FROM live UNION ALL SELECT * FROM rolled)
 WHERE day >= :from_day
 GROUP BY day
 ORDER BY day";

/// Q11: Mode activity per local day since `from_day` (today minus 53 weeks);
/// live tables from the local day `cutoff_day` (`retention.sessions_days`
/// ago) on, the `activity_days` roll-up before it, so no day counts twice.
pub fn activity_days(
    conn: &Connection,
    from_day: &str,
    cutoff_day: &str,
) -> Result<Vec<ActivityRow>> {
    rows(
        conn,
        Q11,
        named_params! { ":from_day": from_day, ":cutoff_day": cutoff_day },
        |r| {
            Ok(ActivityRow {
                day: r.get(0)?,
                sessions_started: r.get(1)?,
                files_changed: r.get(2)?,
                tokens: r.get(3)?,
                activity: r.get(4)?,
            })
        },
    )
}

/// The streak line of the tracker: (current streak, longest streak) over ordered rows.
pub fn streaks(rows: &[ActivityRow], today: &str) -> (i64, i64) {
    let mut longest = 0;
    let mut run = 0;
    let mut prev: Option<&str> = None;
    for r in rows.iter().filter(|r| r.activity > 0) {
        let consecutive = prev.map(|p| is_next_day(p, &r.day)).unwrap_or(false);
        run = if consecutive { run + 1 } else { 1 };
        longest = longest.max(run);
        prev = Some(&r.day);
    }
    let current = match prev {
        Some(last) if last == today || is_next_day(last, today) => run,
        _ => 0,
    };
    (current, longest)
}

fn is_next_day(a: &str, b: &str) -> bool {
    match (day_number(a), day_number(b)) {
        (Some(a), Some(b)) => b == a + 1,
        _ => false,
    }
}

/// Days since 1970-01-01 for a `YYYY-MM-DD` string (proleptic Gregorian).
pub fn day_number(date: &str) -> Option<i64> {
    let mut it = date.splitn(3, '-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

// ---------------------------------------------------------------------------
// S1–S11 — supporting queries for background jobs
// ---------------------------------------------------------------------------

/// S1: one end of a link seen from an item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedItem {
    pub id: String,
    pub link_type: String,
    pub label: Option<String>,
    pub created_by: String,
    pub other_kind: String,
    pub other_id: String,
}

/// S1: links for an item, both directions.
pub fn links_for(conn: &Connection, kind: &str, id: &str) -> Result<Vec<LinkedItem>> {
    rows(
        conn,
        "SELECT l.id, l.link_type, l.label, l.created_by,
                CASE WHEN l.from_kind = :kind AND l.from_id = :id THEN l.to_kind ELSE l.from_kind END AS other_kind,
                CASE WHEN l.from_kind = :kind AND l.from_id = :id THEN l.to_id   ELSE l.from_id   END AS other_id
           FROM links l
          WHERE l.deleted_at IS NULL
            AND ((l.from_kind = :kind AND l.from_id = :id) OR (l.to_kind = :kind AND l.to_id = :id))
          ORDER BY l.created_at",
        named_params! { ":kind": kind, ":id": id },
        |r| {
            Ok(LinkedItem {
                id: r.get(0)?,
                link_type: r.get(1)?,
                label: r.get(2)?,
                created_by: r.get(3)?,
                other_kind: r.get(4)?,
                other_id: r.get(5)?,
            })
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DueSnooze {
    pub id: String,
    pub target_kind: String,
    pub target_id: String,
}

/// S2: snoozes whose time has come (the every-minute sweep).
pub fn snoozes_due(conn: &Connection, clock: &Clock) -> Result<Vec<DueSnooze>> {
    rows(
        conn,
        "SELECT id, target_kind, target_id FROM snoozes
          WHERE fired_at IS NULL AND cancelled_at IS NULL AND deleted_at IS NULL AND until <= :now
          ORDER BY until",
        named_params! { ":now": clock.now },
        |r| {
            Ok(DueSnooze {
                id: r.get(0)?,
                target_kind: r.get(1)?,
                target_id: r.get(2)?,
            })
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScorerItem {
    pub id: String,
    pub subject: Option<String>,
    pub excerpt: String,
    pub sender_name: String,
    pub is_direct: bool,
}

/// S3: the scorer queue (unscored, oldest first); subject + the first
/// `scoring_chars` of the snippet is all the scorer gets.
pub fn scorer_queue(conn: &Connection, scoring_chars: i64, limit: i64) -> Result<Vec<ScorerItem>> {
    rows(
        conn,
        "SELECT id, subject, substr(snippet, 1, :scoring_chars) AS excerpt, sender_name, is_direct
           FROM messages WHERE urgency_tier = 'unscored' ORDER BY received_at LIMIT :limit",
        named_params! { ":scoring_chars": scoring_chars, ":limit": limit },
        |r| {
            Ok(ScorerItem {
                id: r.get(0)?,
                subject: r.get(1)?,
                excerpt: r.get(2)?,
                sender_name: r.get(3)?,
                is_direct: r.get(4)?,
            })
        },
    )
}

/// S4: tombstone the notes whose file disappeared in the last scan
/// (`scan_paths` = every `.md` path the scanner saw), clearing the caches so
/// the FTS trigger drops the text. Runs on the writer. Returns the count.
pub fn tombstone_missing_notes(
    conn: &Connection,
    device_id: &str,
    clock: &Clock,
    scan_paths: &[String],
) -> Result<usize> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS temp.scan_paths; CREATE TEMP TABLE scan_paths (path TEXT PRIMARY KEY) WITHOUT ROWID;",
    )?;
    {
        let mut ins = conn.prepare("INSERT OR IGNORE INTO temp.scan_paths (path) VALUES (?1)")?;
        for p in scan_paths {
            ins.execute([p])?;
        }
    }
    let n = conn.execute(
        "UPDATE notes SET deleted_at = :now, body = '', excerpt = '', tags_text = '',
                          rev = rev + 1, device_id = :device_id, updated_at = :now
          WHERE deleted_at IS NULL AND path NOT IN (SELECT path FROM temp.scan_paths)",
        named_params! { ":now": clock.now, ":device_id": device_id },
    )?;
    conn.execute_batch("DROP TABLE IF EXISTS temp.scan_paths;")?;
    Ok(n)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingChange {
    pub seq: i64,
    pub table_name: String,
    pub row_id: String,
    pub op: String,
    pub rev: i64,
    pub changed_at: String,
    pub row_json: Option<String>,
}

/// S5: the future sync push queue for this device.
pub fn pending_changes(
    conn: &Connection,
    device_id: &str,
    limit: i64,
) -> Result<Vec<PendingChange>> {
    rows(
        conn,
        "SELECT seq, table_name, row_id, op, rev, changed_at, row_json FROM change_log
          WHERE device_id = :device_id AND pushed_at IS NULL ORDER BY seq LIMIT :limit",
        named_params! { ":device_id": device_id, ":limit": limit },
        |r| {
            Ok(PendingChange {
                seq: r.get(0)?,
                table_name: r.get(1)?,
                row_id: r.get(2)?,
                op: r.get(3)?,
                rev: r.get(4)?,
                changed_at: r.get(5)?,
                row_json: r.get(6)?,
            })
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BadgeCounts {
    pub waiting: i64,
    pub urgent: i64,
    pub overdue: i64,
}

/// S6: the sidebar badge counts in one pass.
pub fn badge_counts(conn: &Connection, clock: &Clock) -> Result<BadgeCounts> {
    Ok(conn.query_row(
        "SELECT (SELECT COUNT(*) FROM sessions WHERE state = 'waiting' AND dismissed_at IS NULL AND parent_session_id IS NULL) AS waiting,
                (SELECT COUNT(*) FROM messages m WHERE m.is_read = 0 AND m.is_archived = 0 AND m.urgency_tier IN ('vip','direct','ai-urgent')
                   AND NOT EXISTS (SELECT 1 FROM snoozes z WHERE z.fired_at IS NULL AND z.cancelled_at IS NULL AND z.deleted_at IS NULL AND z.until > :now
                                     AND ((z.target_kind = 'message' AND z.target_id = m.id) OR (z.target_kind = 'thread' AND z.target_id = m.thread_id)))) AS urgent,
                (SELECT COUNT(*) FROM notes WHERE deleted_at IS NULL AND archived_at IS NULL AND status <> 'done' AND when_date < :today) AS overdue",
        named_params! { ":now": clock.now, ":today": clock.today },
        |r| {
            Ok(BadgeCounts {
                waiting: r.get(0)?,
                urgent: r.get(1)?,
                overdue: r.get(2)?,
            })
        },
    )?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionRow {
    pub rev: i64,
    pub origin: String,
    pub created_at: String,
    pub device_id: String,
    pub bytes: i64,
}

/// S7: the note history panel.
pub fn note_history(conn: &Connection, note_id: &str, limit: i64) -> Result<Vec<RevisionRow>> {
    rows(
        conn,
        "SELECT r.rev, r.origin, r.created_at, r.device_id, length(r.content) AS bytes
           FROM note_revisions r WHERE r.note_id = :id ORDER BY r.rev DESC LIMIT :limit",
        named_params! { ":id": note_id, ":limit": limit },
        |r| {
            Ok(RevisionRow {
                rev: r.get(0)?,
                origin: r.get(1)?,
                created_at: r.get(2)?,
                device_id: r.get(3)?,
                bytes: r.get(4)?,
            })
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DueAutomation {
    pub id: String,
    pub name: String,
    pub schedule_json: String,
    pub action_kind: String,
    pub action_json: String,
    pub next_run_at: Option<String>,
}

/// S8: automations due (the scheduler tick); one running run per automation.
pub fn automations_due(conn: &Connection, clock: &Clock) -> Result<Vec<DueAutomation>> {
    rows(
        conn,
        "SELECT a.id, a.name, a.schedule_json, a.action_kind, a.action_json, a.next_run_at
           FROM automations a
          WHERE a.enabled = 1 AND a.deleted_at IS NULL AND a.next_run_at <= :now
            AND NOT EXISTS (SELECT 1 FROM automation_runs r WHERE r.automation_id = a.id AND r.state = 'running')
          ORDER BY a.next_run_at, a.position",
        named_params! { ":now": clock.now },
        |r| {
            Ok(DueAutomation {
                id: r.get(0)?,
                name: r.get(1)?,
                schedule_json: r.get(2)?,
                action_kind: r.get(3)?,
                action_json: r.get(4)?,
                next_run_at: r.get(5)?,
            })
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorRow {
    pub kind: String,
    pub name: String,
    pub status: String,
    pub detail_json: Option<String>,
    pub seen_at: String,
}

/// S10: the Connectors tab rows for one session, in the tab's order.
pub fn session_connectors(conn: &Connection, session_id: &str) -> Result<Vec<ConnectorRow>> {
    rows(
        conn,
        "SELECT kind, name, status, detail_json, seen_at
           FROM session_connectors
          WHERE session_id = :session_id
          ORDER BY CASE kind WHEN 'mcp_server' THEN 0 WHEN 'tool_rule' THEN 1 WHEN 'plugin' THEN 2 ELSE 3 END, name",
        named_params! { ":session_id": session_id },
        |r| {
            Ok(ConnectorRow {
                kind: r.get(0)?,
                name: r.get(1)?,
                status: r.get(2)?,
                detail_json: r.get(3)?,
                seen_at: r.get(4)?,
            })
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRow {
    pub id: String,
    pub trigger_kind: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub state: String,
    pub result_kind: Option<String>,
    pub result_id: Option<String>,
    pub error: Option<String>,
}

/// S11: run history for one automation, newest first.
pub fn automation_runs(conn: &Connection, automation_id: &str, limit: i64) -> Result<Vec<RunRow>> {
    rows(
        conn,
        "SELECT r.id, r.trigger_kind, r.started_at, r.finished_at, r.state, r.result_kind, r.result_id, r.error
           FROM automation_runs r
          WHERE r.automation_id = :automation_id
          ORDER BY r.started_at DESC
          LIMIT :limit",
        named_params! { ":automation_id": automation_id, ":limit": limit },
        |r| {
            Ok(RunRow {
                id: r.get(0)?,
                trigger_kind: r.get(1)?,
                started_at: r.get(2)?,
                finished_at: r.get(3)?,
                state: r.get(4)?,
                result_kind: r.get(5)?,
                result_id: r.get(6)?,
                error: r.get(7)?,
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streaks_count_consecutive_active_days() {
        let row = |day: &str, activity: i64| ActivityRow {
            day: day.into(),
            sessions_started: activity,
            files_changed: 0,
            tokens: 0,
            activity,
        };
        let rows = vec![
            row("2026-10-10", 1),
            row("2026-10-11", 2),
            row("2026-10-12", 0),
            row("2026-10-19", 1),
            row("2026-10-20", 1),
            row("2026-10-21", 3),
        ];
        assert_eq!(streaks(&rows, "2026-10-21"), (3, 3));
        assert_eq!(streaks(&rows[..3], "2026-10-21"), (0, 2));
        assert_eq!(
            streaks(&rows[..5], "2026-10-21"),
            (2, 2),
            "yesterday keeps the streak alive"
        );
        assert_eq!(streaks(&[], "2026-10-21"), (0, 0));
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(
            day_number("2026-03-01"),
            day_number("2026-02-28").map(|d| d + 1)
        );
        assert!(is_next_day("2026-12-31", "2027-01-01"));
        assert_eq!(day_number("nope"), None);
    }

    #[test]
    fn usage_splits_day_bounds_and_sessions_by_state() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::open(&dir.path().join("mode.db")).unwrap();
        let (start, end) = db
            .read(|c| local_day_bounds(c, "2026-10-20", "2026-10-21"))
            .unwrap();
        assert_eq!(start.len(), 24);
        assert!(start < end);
        let span =
            crate::db::parse_timestamp(&end).unwrap() - crate::db::parse_timestamp(&start).unwrap();
        assert!((47..=49).contains(&span.whole_hours()), "two local days");
        let inside = db
            .read(|c| local_day_bounds(c, "2026-10-21", "2026-10-21"))
            .unwrap();
        let mid = crate::db::format_timestamp(
            crate::db::parse_timestamp(&inside.0).unwrap() + time::Duration::hours(12),
        );
        let early = "2020-01-01T00:00:00.000Z".to_string();
        let (m, e) = (mid.clone(), early.clone());
        db.write(move |c| {
            for (id, at, source, model, n) in [
                ("u1", m.as_str(), "claude_code", "m-a", 100),
                ("u2", m.as_str(), "claude_code", "m-a", 50),
                ("u3", m.as_str(), "claude_api", "m-b", 7),
                ("u4", e.as_str(), "claude_code", "m-a", 9_999),
            ] {
                c.execute(
                    "INSERT INTO usage_samples (id, sampled_at, source, purpose, model, input_tokens,
                                                output_tokens, cache_read_tokens, cache_write_tokens, cost_usd, created_at)
                     VALUES (?1, ?2, ?3, 'session', ?4, ?5, 1, 0, 0, 12.5, ?2)",
                    (id, at, source, model, n),
                )?;
            }
            for (id, state, parent, at) in [
                ("s1", "working", None, m.as_str()),
                ("s2", "unread", None, m.as_str()),
                ("s3", "done", None, m.as_str()),
                ("s4", "working", Some("s1"), m.as_str()),
                ("s5", "idle", None, e.as_str()),
            ] {
                c.execute(
                    "INSERT INTO sessions (id, claude_session_id, parent_session_id, cwd, display_name,
                                           avatar_color, tracking, state, state_since, started_at,
                                           last_event_at, created_at, updated_at)
                     VALUES (?1, ?1, ?2, '/x', 'x', 'accent', 'hooked', ?3, ?4, ?4, ?4, ?4, ?4)",
                    (id, parent, state, at),
                )?;
            }
            Ok(())
        })
        .unwrap();
        let split = db
            .read(|c| usage_split_by_day(c, &inside.0, &inside.1))
            .unwrap();
        assert_eq!(split.len(), 2);
        assert_eq!(
            (
                split[0].source.as_str(),
                split[0].model.as_str(),
                split[0].tokens
            ),
            ("claude_code", "m-a", 152)
        );
        assert_eq!(split[1].tokens, 8);
        let by_state = db
            .read(|c| sessions_by_state(c, &inside.0, &inside.1))
            .unwrap();
        assert_eq!(
            by_state,
            vec![
                ("done".to_string(), 1),
                ("unread".to_string(), 1),
                ("working".to_string(), 1)
            ],
            "subagents and sessions outside the window are not counted"
        );
    }
}
