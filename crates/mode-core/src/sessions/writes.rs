//! Row helpers for the session engine, all over a `&Connection` inside the
//! write transaction: lookups, `session_events` rows, connector rows, the
//! machine snapshot of a row, subagent child rows (05 §4.5), and the
//! `mode.*` rows a decision, a relay timeout or an inferred end leave behind.
//! Applying a hook frame itself is [`super::apply`].

use std::collections::HashMap;

use mode_wire::events;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::apply::{Applied, ApplyInput, max_ts, new_row};
use super::avatars;
use super::machine::{PendingRef, SessionSnapshot};
use super::naming;
use super::summary::event_summary;
use super::{Pending, SessionState};
use crate::db::Result;
use crate::push;
use crate::redact;
use crate::repo::{self, Session, SessionConnector, SessionEvent};
use crate::settings;

pub fn find_top_level(conn: &Connection, claude_session_id: &str) -> Result<Option<Session>> {
    let rows = repo::query::<Session>(
        conn,
        "claude_session_id = ? AND agent_id IS NULL",
        &[&claude_session_id],
    )?;
    Ok(rows.into_iter().next())
}

pub fn find_child(conn: &Connection, parent_id: &str, agent_id: &str) -> Result<Option<Session>> {
    let rows = repo::query::<Session>(
        conn,
        "parent_session_id = ? AND agent_id = ?",
        &[&parent_id, &agent_id],
    )?;
    Ok(rows.into_iter().next())
}

pub fn event_exists(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM session_events WHERE id = ?", [id], |r| {
            r.get::<_, i64>(0)
        })
        .optional()?
        .is_some())
}

pub fn next_seq(conn: &Connection, session_id: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM session_events WHERE session_id = ?",
        [session_id],
        |r| r.get(0),
    )?)
}

/// A `session_events` row to append.
pub struct EventIn<'a> {
    pub id: String,
    pub session_id: &'a str,
    pub source: &'a str,
    pub event: &'a str,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub agent_id: Option<String>,
    pub state_before: Option<SessionState>,
    pub state_after: Option<SessionState>,
    pub summary: String,
    pub payload: Option<Value>,
    pub payload_bytes: usize,
    pub occurred_at: &'a str,
    pub received_at: &'a str,
}

pub fn insert_event(conn: &Connection, e: EventIn<'_>) -> Result<i64> {
    let seq = next_seq(conn, e.session_id)?;
    let row = SessionEvent {
        id: e.id,
        session_id: e.session_id.to_string(),
        seq,
        source: e.source.to_string(),
        event: e.event.to_string(),
        tool_name: e.tool_name,
        tool_use_id: e.tool_use_id,
        agent_id: e.agent_id,
        state_before: e.state_before.map(|s| s.as_str().to_string()),
        state_after: e.state_after.map(|s| s.as_str().to_string()),
        summary: Some(e.summary),
        payload_json: e.payload.map(|p| p.to_string()),
        payload_bytes: e.payload_bytes as i64,
        compacted: false,
        occurred_at: e.occurred_at.to_string(),
        received_at: e.received_at.to_string(),
    };
    repo::insert(conn, &row)?;
    Ok(seq)
}

pub fn upsert_connector(
    conn: &Connection,
    session_id: &str,
    kind: &str,
    name: &str,
    status: &str,
    detail: Option<Value>,
    now: &str,
) -> Result<()> {
    let row = SessionConnector {
        session_id: session_id.to_string(),
        kind: kind.to_string(),
        name: name.to_string(),
        status: status.to_string(),
        detail_json: detail.map(|d| d.to_string()),
        seen_at: now.to_string(),
    };
    repo::upsert(conn, &row, &["session_id", "kind", "name"])
}

pub fn mcp_tools_seen(conn: &Connection, session_id: &str, server: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(DISTINCT tool_name) FROM session_events
          WHERE session_id = ? AND tool_name LIKE ?",
        params![session_id, format!("mcp__{server}__%")],
        |r| r.get(0),
    )?)
}

/// Children still open when the parent ends are closed `inferred` (05 §4.5).
pub fn close_children(conn: &Connection, parent_id: &str, now: &str) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE sessions SET ended_at = ?1, end_reason = 'inferred', state = 'done', state_since = ?1,
                state_reason = 'SessionEnd:parent', acknowledged_at = COALESCE(acknowledged_at, ?1),
                current_tool = NULL, current_tool_use_id = NULL, current_mcp_server = NULL,
                waiting_kind = NULL, pending_json = NULL, updated_at = ?1
          WHERE parent_session_id = ?2 AND ended_at IS NULL",
        params![now, parent_id],
    )?)
}

/// The machine's view of a row.
pub fn snapshot_of(row: &Session, is_new: bool) -> SessionSnapshot {
    let pending = row.pending_json.as_deref().and_then(Pending::from_json);
    SessionSnapshot {
        state: row.state.parse().unwrap_or(SessionState::Idle),
        waiting_kind: row.waiting_kind.as_deref().and_then(|k| k.parse().ok()),
        pending: pending.as_ref().map(|p| PendingRef {
            kind: Some(p.kind),
            tool_use_id: p.tool_use_id.clone(),
            tool_name: p.tool_name.clone(),
            tool_input: p.tool_input.clone(),
            frame_open: p.answerable && !p.timed_out && p.frame_id.is_some(),
        }),
        current_tool_use_id: row.current_tool_use_id.clone(),
        ended: row.ended_at.is_some(),
        end_reason: row.end_reason.clone(),
        acknowledged: row.acknowledged_at.is_some(),
        last_event_at: (!is_new).then(|| row.last_event_at.clone()),
        is_subagent: row.parent_session_id.is_some(),
    }
}

/// `SubagentStart` / `SubagentStop` (05 §4.5): a child row keyed by
/// `agent_id`; the parent stores the event and keeps its state.
pub fn apply_subagent(conn: &Connection, input: &ApplyInput) -> Result<Applied> {
    let frame = &input.frame;
    let ev = &frame.payload;
    let now = input.now.as_str();
    let occurred = if frame.sent_at.is_empty() {
        now
    } else {
        frame.sent_at.as_str()
    };
    let name = ev.hook_event_name.as_str();
    let agent_id = ev.agent_id.clone().unwrap_or_default();
    let custom: HashMap<String, String> =
        settings::get_as_in(conn, "sessions.custom_names").unwrap_or_default();
    let parent = match find_top_level(conn, &ev.session_id)? {
        Some(mut p) => {
            p.last_event_at = max_ts(&p.last_event_at, occurred);
            p.updated_at = now.to_string();
            repo::replace(conn, &p)?;
            p
        }
        None => {
            let mut p = new_row(ev, input, &custom, occurred);
            p.avatar_variant = avatars::next_variant(conn)?;
            p.state_reason = Some(format!("{name}:implicit"));
            repo::insert(conn, &p)?;
            p
        }
    };
    let state_before: SessionState = parent.state.parse().unwrap_or(SessionState::Idle);
    let existing = find_child(conn, &parent.id, &agent_id)?;
    let transcript = parent
        .transcript_path
        .as_deref()
        .and_then(|p| crate::transcript::subagents_dir_of(&crate::paths::to_os(p)))
        .and_then(|dir| crate::transcript::subagent_file(&dir, &agent_id))
        .map(|p| crate::paths::from_os(&p));
    let mut child = existing.unwrap_or_else(|| Session {
        id: repo::new_id(),
        claude_session_id: parent.claude_session_id.clone(),
        parent_session_id: Some(parent.id.clone()),
        agent_id: Some(agent_id.clone()),
        agent_type: ev.agent_type.clone(),
        tracking: parent.tracking.clone(),
        launch_mode: parent.launch_mode.clone(),
        state: SessionState::Working.as_str().into(),
        state_since: occurred.to_string(),
        display_name: ev
            .agent_type
            .clone()
            .unwrap_or_else(|| format!("agent {agent_id}")),
        avatar_color: parent.avatar_color.clone(),
        avatar_variant: parent.avatar_variant.clone(),
        cwd: parent.cwd.clone(),
        repo_root: parent.repo_root.clone(),
        git_branch: parent.git_branch.clone(),
        started_at: occurred.to_string(),
        last_event_at: occurred.to_string(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        ..Default::default()
    });
    let is_new_child =
        child.created_at == now && child.updated_at == now && child.state_since == occurred;
    if let Some(t) = &ev.agent_type {
        child.agent_type = Some(t.clone());
        if child.display_name.starts_with("agent ") {
            child.display_name = t.clone();
        }
    }
    if child.transcript_path.is_none() {
        child.transcript_path = transcript;
    }
    let mut summary = event_summary(ev, None, false, None);
    match name {
        events::SUBAGENT_START => {
            child.state = SessionState::Working.as_str().into();
            child.state_since = occurred.to_string();
            child.state_reason = Some(name.into());
            child.ended_at = None;
            child.end_reason = None;
        }
        _ => {
            child.state = SessionState::Done.as_str().into();
            child.state_since = occurred.to_string();
            child.state_reason = Some(name.into());
            child.ended_at = Some(occurred.to_string());
            child.end_reason = Some("subagent_stop".into());
            child.acknowledged_at = Some(now.to_string());
            child.current_tool = None;
            child.current_tool_use_id = None;
            child.current_mcp_server = None;
            if let (Some(a), Some(b)) = (
                crate::db::parse_timestamp(&child.started_at),
                crate::db::parse_timestamp(occurred),
            ) {
                summary = format!(
                    "{} · {}",
                    ev.agent_type
                        .as_deref()
                        .or(child.agent_type.as_deref())
                        .unwrap_or("agent"),
                    naming::elapsed_text((b - a).whole_seconds())
                );
            }
        }
    }
    child.last_event_at = max_ts(&child.last_event_at, occurred);
    child.updated_at = now.to_string();
    let exists = find_child(conn, &parent.id, &agent_id)?.is_some();
    if exists {
        repo::replace(conn, &child)?;
    } else {
        repo::insert(conn, &child)?;
    }
    let _ = is_new_child;
    let payload = redact::for_storage(ev, false);
    let payload_bytes = serde_json::to_vec(ev).map(|v| v.len()).unwrap_or(0);
    insert_event(
        conn,
        EventIn {
            id: frame.id.clone(),
            session_id: &parent.id,
            source: "hook",
            event: name,
            tool_name: None,
            tool_use_id: ev.tool_use_id().map(str::to_string),
            agent_id: Some(agent_id),
            state_before: Some(state_before),
            state_after: None,
            summary,
            payload: Some(payload),
            payload_bytes,
            occurred_at: occurred,
            received_at: now,
        },
    )?;
    Ok(Applied {
        session_id: parent.id,
        claude_session_id: parent.claude_session_id,
        event: name.to_string(),
        state_before,
        state_after: state_before,
        state_changed: false,
        pending: parent.pending_json.as_deref().and_then(Pending::from_json),
        auto_decision: None,
        git_pass: None,
        pushes: vec![push::Event::Sessions],
        note: "subagent row",
    })
}

/// What a decision leaves behind (05 §7.1 step 6, §7.3).
pub struct DecisionRecord {
    pub event: &'static str,
    pub summary: String,
    pub tool_rule: Option<(String, &'static str, &'static str)>,
    /// The next queued card, which keeps the row `waiting`.
    pub next: Option<Pending>,
}

pub fn record_decision(
    conn: &Connection,
    row: &Session,
    pending: &Pending,
    rec: DecisionRecord,
    now: &str,
) -> Result<SessionState> {
    let before: SessionState = row.state.parse().unwrap_or(SessionState::Working);
    let (state, kind, pending_json, reason) = match &rec.next {
        Some(next) => (
            SessionState::Waiting,
            Some(next.kind.as_str().to_string()),
            Some(next.to_json()),
            "queued card".to_string(),
        ),
        None => (SessionState::Working, None, None, rec.event.to_string()),
    };
    let keep_state = before == SessionState::Failed || row.ended_at.is_some();
    conn.execute(
        "UPDATE sessions SET state = ?1, state_since = ?2, state_reason = ?3, waiting_kind = ?4,
                pending_json = ?5, updated_at = ?2
          WHERE id = ?6",
        params![
            if keep_state {
                before.as_str()
            } else {
                state.as_str()
            },
            now,
            reason,
            if keep_state { None } else { kind },
            pending_json,
            row.id,
        ],
    )?;
    insert_event(
        conn,
        EventIn {
            id: repo::new_id(),
            session_id: &row.id,
            source: "mode",
            event: rec.event,
            tool_name: pending.tool_name.clone(),
            tool_use_id: pending.tool_use_id.clone(),
            agent_id: None,
            state_before: Some(before),
            state_after: (!keep_state && state != before).then_some(state),
            summary: rec.summary,
            payload: None,
            payload_bytes: 0,
            occurred_at: now,
            received_at: now,
        },
    )?;
    if let Some((name, status, origin)) = rec.tool_rule {
        upsert_connector(
            conn,
            &row.id,
            "tool_rule",
            &name,
            status,
            Some(serde_json::json!({"origin": origin})),
            now,
        )?;
    }
    Ok(if keep_state { before } else { state })
}

/// The relay gave up on the open card (05 §7.4): grayed, state kept.
pub fn record_timeout(
    conn: &Connection,
    row: &Session,
    pending: &Pending,
    now: &str,
) -> Result<()> {
    let mut p = pending.clone();
    p.answerable = false;
    p.timed_out = true;
    conn.execute(
        "UPDATE sessions SET pending_json = ?, updated_at = ? WHERE id = ?",
        params![p.to_json(), now, row.id],
    )?;
    let before: SessionState = row.state.parse().unwrap_or(SessionState::Waiting);
    insert_event(
        conn,
        EventIn {
            id: repo::new_id(),
            session_id: &row.id,
            source: "mode",
            event: "mode.timeout",
            tool_name: pending.tool_name.clone(),
            tool_use_id: pending.tool_use_id.clone(),
            agent_id: None,
            state_before: Some(before),
            state_after: None,
            summary: "timed out · answer in the terminal".into(),
            payload: None,
            payload_bytes: 0,
            occurred_at: now,
            received_at: now,
        },
    )?;
    Ok(())
}

/// Close a row whose process is gone (05 §4.2 "Liveness probe").
pub fn close_inferred(
    conn: &Connection,
    session_id: &str,
    now: &str,
) -> Result<Option<SessionState>> {
    close_inferred_as(conn, session_id, now, InferredEnd::ProcessGone)
}

/// Why an end was inferred rather than reported (05 §4.2): the probe found
/// the process gone, Mode restarted (its own children died with it, or an
/// external process is gone since), or the machine rebooted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferredEnd {
    ProcessGone,
    Restart,
    Reboot,
}

impl InferredEnd {
    /// The thread's end reason (`ItemBody::End`) and `sessions.state_reason`.
    pub fn reason(self) -> &'static str {
        match self {
            InferredEnd::ProcessGone => "inferred",
            InferredEnd::Restart => "restart",
            InferredEnd::Reboot => "reboot",
        }
    }

    fn state_reason(self) -> &'static str {
        match self {
            InferredEnd::ProcessGone => "liveness:inferred",
            InferredEnd::Restart => "restart:inferred",
            InferredEnd::Reboot => "reboot:inferred",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            InferredEnd::ProcessGone => "process gone · ended",
            InferredEnd::Restart => "Mode was not running · ended",
            InferredEnd::Reboot => "the machine restarted · ended",
        }
    }
}

/// Close a live row whose process cannot be reached any more, naming why.
pub fn close_inferred_as(
    conn: &Connection,
    session_id: &str,
    now: &str,
    why: InferredEnd,
) -> Result<Option<SessionState>> {
    let Some(row) = repo::get_by_id::<Session>(conn, session_id)? else {
        return Ok(None);
    };
    if row.ended_at.is_some() {
        return Ok(None);
    }
    let before: SessionState = row.state.parse().unwrap_or(SessionState::Idle);
    let after = match before {
        SessionState::Failed => SessionState::Failed,
        SessionState::Done => SessionState::Done,
        _ if row.acknowledged_at.is_some() => SessionState::Done,
        _ => SessionState::Unread,
    };
    conn.execute(
        "UPDATE sessions SET ended_at = ?1, end_reason = 'inferred', state = ?2, state_since = ?1,
                state_reason = ?4, waiting_kind = NULL, pending_json = NULL,
                current_tool = NULL, current_tool_use_id = NULL, current_mcp_server = NULL, updated_at = ?1
          WHERE id = ?3",
        params![now, after.as_str(), session_id, why.state_reason()],
    )?;
    close_children(conn, session_id, now)?;
    let payload =
        (why != InferredEnd::ProcessGone).then(|| serde_json::json!({ "reason": why.reason() }));
    let payload_bytes = payload.as_ref().map_or(0, |p| p.to_string().len());
    insert_event(
        conn,
        EventIn {
            id: repo::new_id(),
            session_id,
            source: "mode",
            event: "mode.inferred_end",
            tool_name: None,
            tool_use_id: None,
            agent_id: None,
            state_before: Some(before),
            state_after: (after != before).then_some(after),
            summary: why.summary().into(),
            payload,
            payload_bytes,
            occurred_at: now,
            received_at: now,
        },
    )?;
    Ok(Some(after))
}

/// Mode restarted while this external row was open and its process is alive
/// or unknown (05 §4.2 "Mode starts"): a pending decision died with the relay
/// connection (Claude Code prompted in the terminal), a tool in flight may
/// have finished unseen, so `waiting` and `working` go back to `idle` with a
/// state marker; the next frame restores the real state. Other states keep
/// theirs. `Ok(true)` when the row changed.
pub fn reset_after_restart(conn: &Connection, session_id: &str, now: &str) -> Result<bool> {
    let Some(row) = repo::get_by_id::<Session>(conn, session_id)? else {
        return Ok(false);
    };
    if row.ended_at.is_some() {
        return Ok(false);
    }
    let before: SessionState = row.state.parse().unwrap_or(SessionState::Idle);
    if !matches!(before, SessionState::Waiting | SessionState::Working) {
        return Ok(false);
    }
    conn.execute(
        "UPDATE sessions SET state = 'idle', state_since = ?1, state_reason = 'restart',
                waiting_kind = NULL, pending_json = NULL,
                current_tool = NULL, current_tool_use_id = NULL, current_mcp_server = NULL, updated_at = ?1
          WHERE id = ?2",
        params![now, session_id],
    )?;
    insert_event(
        conn,
        EventIn {
            id: repo::new_id(),
            session_id,
            source: "mode",
            event: "mode.restart",
            tool_name: None,
            tool_use_id: None,
            agent_id: None,
            state_before: Some(before),
            state_after: Some(SessionState::Idle),
            summary: "Mode was not running · waiting for the next event".into(),
            payload: None,
            payload_bytes: 0,
            occurred_at: now,
            received_at: now,
        },
    )?;
    Ok(true)
}
