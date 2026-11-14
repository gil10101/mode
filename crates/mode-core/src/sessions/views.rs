//! What the surfaces read (09 §13 Q1 / Q2 / Q7 / S10 plus the open card):
//! the roster rows with their `pending` copy (`sessions.changed` payload),
//! the pill candidate with its "+n" count (`pill.changed`) and the session
//! detail bundle. Field names are the wire names of `ui/src/state/contract.ts`.

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use super::Pending;
use super::thread::{self, ThreadItem};
use crate::db::Result;
use crate::queries::{self, Clock, ConnectorRow, PillCandidate, RosterRow, SessionFile};
use crate::repo::{self, Session};

/// One `sessions.changed` row: Q2 flattened plus the open card and the
/// three columns 02 §7.5 needs beyond Q2.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionItem {
    #[serde(flatten)]
    pub row: RosterRow,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
    pub end_reason: Option<String>,
    pub state_reason: Option<String>,
    pub current_mcp_server: Option<String>,
    // `AppRosterRow` additions (contract-sessions.ts): dates for the filter
    // and history, the last assistant line for the detail panel.
    pub claude_session_id: String,
    pub started_at: String,
    pub last_event_at: String,
    pub ended_at: Option<String>,
    pub acknowledged_at: Option<String>,
    pub last_assistant_preview: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Extra {
    pending_json: Option<String>,
    end_reason: Option<String>,
    state_reason: Option<String>,
    current_mcp_server: Option<String>,
    claude_session_id: String,
    started_at: String,
    last_event_at: String,
    ended_at: Option<String>,
    acknowledged_at: Option<String>,
    last_assistant_preview: Option<String>,
}

/// Q2 with the card copy (05 §11.4 order).
pub fn roster(conn: &Connection, clock: &Clock, fold_minutes: i64) -> Result<Vec<SessionItem>> {
    let rows = queries::roster(conn, clock, fold_minutes)?;
    let mut stmt = conn.prepare(
        "SELECT id, pending_json, end_reason, state_reason, current_mcp_server, claude_session_id,
                started_at, last_event_at, ended_at, acknowledged_at, last_assistant_preview
           FROM sessions WHERE parent_session_id IS NULL AND dismissed_at IS NULL",
    )?;
    let extras: std::collections::HashMap<String, Extra> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Extra {
                    pending_json: r.get(1)?,
                    end_reason: r.get(2)?,
                    state_reason: r.get(3)?,
                    current_mcp_server: r.get(4)?,
                    claude_session_id: r.get(5)?,
                    started_at: r.get(6)?,
                    last_event_at: r.get(7)?,
                    ended_at: r.get(8)?,
                    acknowledged_at: r.get(9)?,
                    last_assistant_preview: r.get(10)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let x = extras.get(&row.id).cloned().unwrap_or_default();
            SessionItem {
                pending: x.pending_json.as_deref().and_then(Pending::from_json),
                end_reason: x.end_reason,
                state_reason: x.state_reason,
                current_mcp_server: x.current_mcp_server,
                claude_session_id: x.claude_session_id,
                started_at: x.started_at,
                last_event_at: x.last_event_at,
                ended_at: x.ended_at,
                acknowledged_at: x.acknowledged_at,
                last_assistant_preview: x.last_assistant_preview,
                row,
            }
        })
        .collect())
}

/// `pill.changed`: the first candidate and how many others share its priority.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PillPayload {
    pub candidate: Option<PillCandidate>,
    pub count: i64,
}

pub fn pill(conn: &Connection, clock: &Clock, max_age_hours: i64) -> Result<PillPayload> {
    let candidate = queries::pill_candidate(conn, clock, max_age_hours)?;
    let count = match &candidate {
        Some(c) if c.priority != 4 => {
            (queries::pill_count(conn, clock, max_age_hours, c.priority)? - 1).max(0)
        }
        _ => 0,
    };
    Ok(PillPayload { candidate, count })
}

/// One row of the Agents tab (05 §4.5).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentRow {
    pub id: String,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub state: String,
    pub state_since: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub current_tool: Option<String>,
    /// `PreToolUse` events attributed to the agent.
    pub tools_used: i64,
    pub last_assistant_preview: Option<String>,
    pub transcript_path: Option<String>,
}

pub fn agents(conn: &Connection, parent_id: &str) -> Result<Vec<AgentRow>> {
    let children = repo::query::<Session>(
        conn,
        "parent_session_id = ? ORDER BY started_at",
        &[&parent_id],
    )?;
    let mut out = Vec::with_capacity(children.len());
    for c in children {
        let tools_used: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_events
              WHERE (session_id = ?1 OR (session_id = ?2 AND agent_id = ?3))
                AND event IN ('PreToolUse', 'transcript.tool_use')",
            rusqlite::params![c.id, parent_id, c.agent_id.clone().unwrap_or_default()],
            |r| r.get(0),
        )?;
        out.push(AgentRow {
            id: c.id,
            agent_id: c.agent_id,
            agent_type: c.agent_type,
            state: c.state,
            state_since: c.state_since,
            started_at: c.started_at,
            ended_at: c.ended_at,
            current_tool: c.current_tool,
            tools_used,
            last_assistant_preview: c.last_assistant_preview,
            transcript_path: c.transcript_path,
        });
    }
    Ok(out)
}

/// A link row (03 §3.11; S1), with the target's availability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkRow {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub link_type: String,
    pub available: bool,
}

fn links(conn: &Connection, session_id: &str) -> Result<Vec<LinkRow>> {
    let mut out = Vec::new();
    for l in queries::links_for(conn, "session", session_id)? {
        let table = match l.other_kind.as_str() {
            "note" => Some("notes"),
            "message" => Some("messages"),
            "thread" => Some("threads"),
            "event" => Some("calendar_events"),
            _ => None,
        };
        let available = match table {
            Some(t) => conn
                .query_row(
                    &format!("SELECT 1 FROM {t} WHERE id = ? AND deleted_at IS NULL"),
                    [&l.other_id],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?
                .is_some(),
            None => false,
        };
        out.push(LinkRow {
            id: l.id,
            title: l.label.unwrap_or_else(|| l.other_id.clone()),
            kind: l.other_kind,
            link_type: l.link_type,
            available,
        });
    }
    Ok(out)
}

/// Where the transcript is and whether it is there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptInfo {
    pub path: Option<String>,
    pub size_bytes: Option<u64>,
    pub exists: bool,
}

/// The header columns of 05 §11.5 plus what the tabs need: `repo::Session`
/// without the card copy and without `cost_usd` (never shown).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionHeader {
    pub id: String,
    pub claude_session_id: String,
    pub display_name: String,
    pub title: Option<String>,
    pub avatar_color: String,
    pub avatar_variant: String,
    pub state: String,
    pub state_since: String,
    pub state_reason: Option<String>,
    pub waiting_kind: Option<String>,
    pub cwd: String,
    pub repo_root: Option<String>,
    pub git_branch: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    /// The agent profile the session was launched as (03 §7E).
    pub profile_id: Option<String>,
    pub tracking: String,
    pub launch_mode: String,
    pub current_tool: Option<String>,
    pub current_mcp_server: Option<String>,
    pub transcript_path: Option<String>,
    pub terminal_app: Option<String>,
    pub last_assistant_preview: Option<String>,
    pub lines_added: i64,
    pub lines_removed: i64,
    pub files_changed: i64,
    pub num_turns: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub context_tokens: Option<i64>,
    pub started_at: String,
    pub last_event_at: String,
    pub ended_at: Option<String>,
    pub end_reason: Option<String>,
    pub acknowledged_at: Option<String>,
    pub dismissed_at: Option<String>,
}

impl From<Session> for SessionHeader {
    fn from(s: Session) -> Self {
        SessionHeader {
            id: s.id,
            claude_session_id: s.claude_session_id,
            display_name: s.display_name,
            title: s.title,
            avatar_color: s.avatar_color,
            avatar_variant: s.avatar_variant,
            state: s.state,
            state_since: s.state_since,
            state_reason: s.state_reason,
            waiting_kind: s.waiting_kind,
            cwd: s.cwd,
            repo_root: s.repo_root,
            git_branch: s.git_branch,
            model: s.model,
            permission_mode: s.permission_mode,
            profile_id: s.profile_id,
            tracking: s.tracking,
            launch_mode: s.launch_mode,
            current_tool: s.current_tool,
            current_mcp_server: s.current_mcp_server,
            transcript_path: s.transcript_path,
            terminal_app: s.terminal_app,
            last_assistant_preview: s.last_assistant_preview,
            lines_added: s.lines_added,
            lines_removed: s.lines_removed,
            files_changed: s.files_changed,
            num_turns: s.num_turns,
            input_tokens: s.input_tokens,
            output_tokens: s.output_tokens,
            cache_read_tokens: s.cache_read_tokens,
            cache_write_tokens: s.cache_write_tokens,
            context_tokens: s.context_tokens,
            started_at: s.started_at,
            last_event_at: s.last_event_at,
            ended_at: s.ended_at,
            end_reason: s.end_reason,
            acknowledged_at: s.acknowledged_at,
            dismissed_at: s.dismissed_at,
        }
    }
}

/// The session detail bundle (03 §3.6–§3.12; `contract-sessions.ts`
/// `SessionDetail`): header, the open card, the Overview thread (oldest
/// first), Files, Agents, Connectors, links and where the transcript is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionDetail {
    pub session: SessionHeader,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
    /// `pty` / `headless` arrive with the PTY host; `none` until then (05 §14).
    pub driver: &'static str,
    /// `ended_at IS NULL` and the known process (if any) is alive.
    pub alive: bool,
    pub thread: Vec<ThreadItem>,
    pub thread_has_more: bool,
    pub files: Vec<SessionFile>,
    pub agents: Vec<AgentRow>,
    pub connectors: Vec<ConnectorRow>,
    /// `init` for headless turns (later); `hooks` for everything seen so far.
    pub connectors_source: &'static str,
    pub links: Vec<LinkRow>,
    pub transcript: TranscriptInfo,
}

/// Timeline page size of the first detail load.
pub const TIMELINE_PAGE: i64 = 200;

/// `alive` answers whether the session's process is still there (the engine
/// knows the pid; `true` when no pid was ever reported).
pub fn detail(
    conn: &Connection,
    id: &str,
    timeline_limit: i64,
    alive: impl Fn(&Session) -> bool,
) -> Result<Option<SessionDetail>> {
    let Some(session) = repo::get_by_id::<Session>(conn, id)? else {
        return Ok(None);
    };
    let pending = session.pending_json.as_deref().and_then(Pending::from_json);
    let events = queries::session_timeline(conn, id, None, timeline_limit)?;
    let agents = agents(conn, id)?;
    let thread = thread::build(&events, &agents);
    let transcript = {
        let path = session.transcript_path.clone();
        let meta = path
            .as_deref()
            .and_then(|p| std::fs::metadata(crate::paths::to_os(p)).ok());
        TranscriptInfo {
            exists: meta.is_some(),
            size_bytes: meta.map(|m| m.len()),
            path,
        }
    };
    let is_alive = session.ended_at.is_none() && alive(&session);
    Ok(Some(SessionDetail {
        pending,
        driver: "none",
        alive: is_alive,
        thread_has_more: events.len() as i64 >= timeline_limit,
        thread,
        files: queries::session_files(conn, id)?,
        agents,
        connectors: queries::session_connectors(conn, id)?,
        connectors_source: "hooks",
        links: links(conn, id)?,
        transcript,
        session: SessionHeader::from(session),
    }))
}
