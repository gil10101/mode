//! Applying one hook frame inside the write transaction (05 §4.2 "Mode
//! stores"): the `sessions` upsert driven by [`transition`], the card copy,
//! the queue of cards behind the open one, `always` rules, file changes and
//! connector rows, and the `session_events` row. Helpers for rows live in
//! [`super::writes`].

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use mode_wire::{HookEvent, HookFrame, events};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::avatars;
use super::machine::{Change, EndChange, OpenCard, Transition, transition};
use super::naming::{self, RepoInfo, TerminalRef};
use super::summary::event_summary;
use super::writes::{
    EventIn, apply_subagent, close_children, event_exists, find_top_level, insert_event,
    mcp_tools_seen, snapshot_of, upsert_connector,
};
use super::{Pending, SessionState, WaitingKind};
use crate::db::Result;
use crate::files;
use crate::ipc::Server;
use crate::push;
use crate::redact;
use crate::repo::{self, Session};
use crate::settings;

/// Cards waiting behind the open one, per Mode session id.
pub type Queues = Arc<Mutex<HashMap<String, VecDeque<Pending>>>>;

/// `last_prompt_preview` cap (09 §5.2).
const PROMPT_PREVIEW_CHARS: usize = 500;
/// `last_assistant_preview` cap (09 §5.2).
const ASSISTANT_PREVIEW_CHARS: usize = 2000;

/// The card copy (`pending_json`) for an event that opens one.
pub fn build_pending(
    card: OpenCard,
    ev: &HookEvent,
    frame_id: &str,
    wants_decision: bool,
    base: Option<&str>,
    received_at: &str,
) -> Pending {
    let tool = ev.tool_name().map(str::to_string);
    let input = ev.get("tool_input");
    let message = ev.message.clone().filter(|m| !m.trim().is_empty());
    let (kind, summary, tool_input, questions, answerable, frame) = match card {
        OpenCard::Permission => (
            WaitingKind::Permission,
            naming::tool_summary(tool.as_deref().unwrap_or("tool"), input, base),
            input.map(redact::tool_field_for_storage),
            None,
            wants_decision,
            wants_decision.then(|| frame_id.to_string()),
        ),
        OpenCard::Question => (
            WaitingKind::Question,
            naming::tool_summary(events::ASK_USER_QUESTION_TOOL, input, base),
            // The answer echoes the original input back (`updatedInput`), so it is kept verbatim.
            input.cloned(),
            input.and_then(|i| i.get("questions")).cloned(),
            wants_decision,
            wants_decision.then(|| frame_id.to_string()),
        ),
        OpenCard::PermissionNotice => (
            WaitingKind::Permission,
            message
                .clone()
                .unwrap_or_else(|| "permission needed in the terminal".into()),
            None,
            None,
            false,
            None,
        ),
        OpenCard::Elicitation => (
            WaitingKind::Elicitation,
            message
                .clone()
                .unwrap_or_else(|| "needs input in the terminal".into()),
            None,
            None,
            false,
            None,
        ),
        OpenCard::AgentNeedsInput => (
            WaitingKind::AgentNeedsInput,
            message
                .clone()
                .unwrap_or_else(|| "a subagent needs input in the terminal".into()),
            None,
            None,
            false,
            None,
        ),
        OpenCard::IdlePrompt => (
            WaitingKind::IdlePrompt,
            message
                .clone()
                .unwrap_or_else(|| "turn ended, waiting for a prompt".into()),
            None,
            None,
            false,
            None,
        ),
    };
    Pending {
        kind,
        tool_name: if matches!(card, OpenCard::Permission | OpenCard::Question) {
            tool
        } else {
            None
        },
        tool_input,
        tool_use_id: ev.tool_use_id().map(str::to_string),
        summary: redact::scrub(&summary).into_owned(),
        questions,
        message: message.map(|m| redact::scrub(&naming::truncate_chars(&m, 2000)).into_owned()),
        received_at: received_at.to_string(),
        answerable,
        frame_id: frame,
        timed_out: false,
    }
}

/// Everything [`apply_in_tx`] needs that the engine resolved before the write.
#[derive(Debug, Clone)]
pub struct ApplyInput {
    pub frame: HookFrame,
    pub relay_stale: bool,
    /// Resolved for `SessionStart`, `CwdChanged` and first frames.
    pub repo: Option<RepoInfo>,
    pub terminal: Option<TerminalRef>,
    /// The `always` rule that answers this `PermissionRequest`.
    pub auto_rule: Option<String>,
    /// `launch_id` matched a pending launch (05 §8.1).
    pub launched: bool,
    /// The launch's mode (`embedded` | `headless`); `embedded` when unset.
    pub launch_mode: Option<String>,
    pub now: String,
}

/// An `always` rule answered the frame (05 §7.2).
#[derive(Debug, Clone, PartialEq)]
pub struct AutoDecision {
    pub frame_id: String,
    pub rule: String,
    pub output: Value,
    /// The engine sent it to the relay.
    pub sent: bool,
}

/// What a frame did (the engine's answer to the pump).
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// `sessions.id`.
    pub session_id: String,
    pub claude_session_id: String,
    pub event: String,
    pub state_before: SessionState,
    pub state_after: SessionState,
    pub state_changed: bool,
    /// The card now open on the row.
    pub pending: Option<Pending>,
    pub auto_decision: Option<AutoDecision>,
    /// `repo_root` when a debounced git pass is due (05 §6.2).
    pub git_pass: Option<String>,
    pub pushes: Vec<push::Event>,
    pub note: &'static str,
}

pub fn new_row(
    ev: &HookEvent,
    input: &ApplyInput,
    custom: &HashMap<String, String>,
    occurred: &str,
) -> Session {
    let repo = input.repo.clone().unwrap_or_default();
    let title = ev
        .get("session_title")
        .and_then(Value::as_str)
        .map(str::to_string);
    let display_name = custom.get(&ev.session_id).cloned().unwrap_or_else(|| {
        naming::display_name(
            title.as_deref(),
            repo.repo_root.as_deref(),
            &ev.cwd,
            &ev.session_id,
        )
    });
    Session {
        id: repo::new_id(),
        claude_session_id: ev.session_id.clone(),
        tracking: if input.launched { "launched" } else { "hooked" }.into(),
        launch_mode: if input.launched {
            input.launch_mode.as_deref().unwrap_or("embedded")
        } else {
            "external"
        }
        .into(),
        state: SessionState::Idle.as_str().into(),
        state_since: occurred.to_string(),
        display_name,
        title,
        avatar_color: naming::avatar_color(repo.repo_root.as_deref(), &ev.cwd),
        cwd: ev.cwd.clone(),
        repo_root: repo.repo_root,
        git_branch: repo.git_branch,
        started_at: occurred.to_string(),
        last_event_at: occurred.to_string(),
        created_at: input.now.clone(),
        updated_at: input.now.clone(),
        ..Default::default()
    }
}

pub fn max_ts(a: &str, b: &str) -> String {
    if b > a { b.to_string() } else { a.to_string() }
}

fn preview(text: &str, max: usize) -> String {
    redact::scrub(&naming::truncate_chars(text.trim(), max)).into_owned()
}

/// Pop the next queued card whose relay frame is still open.
pub fn pop_open_queued(
    queued: &Queues,
    server: Option<&Server>,
    session_id: &str,
) -> Option<Pending> {
    let mut map = queued.lock().unwrap_or_else(|p| p.into_inner());
    let q = map.get_mut(session_id)?;
    while let Some(p) = q.pop_front() {
        let open = match (&p.frame_id, server) {
            (Some(id), Some(s)) => s.is_pending(id),
            (Some(_), None) => true,
            (None, _) => false,
        };
        if open {
            return Some(p);
        }
    }
    None
}

fn apply_columns(row: &mut Session, t: &Transition, occurred: &str, now: &str) {
    if let Some(s) = t.state {
        row.state = s.as_str().into();
        row.state_since = occurred.to_string();
        row.state_reason = t.reason.clone();
    }
    match &t.waiting {
        Change::Keep => {}
        Change::Clear => row.waiting_kind = None,
        Change::Set(k) => row.waiting_kind = Some(k.as_str().into()),
    }
    match &t.current_tool {
        Change::Keep => {}
        Change::Clear => {
            row.current_tool = None;
            row.current_tool_use_id = None;
            row.current_mcp_server = None;
        }
        Change::Set(tool) => {
            row.current_mcp_server = naming::mcp_server(&tool.name).map(str::to_string);
            row.current_tool = Some(tool.name.clone());
            row.current_tool_use_id = tool.tool_use_id.clone();
        }
    }
    match &t.ended {
        EndChange::Keep => {}
        EndChange::End(reason) => {
            row.ended_at = Some(occurred.to_string());
            row.end_reason = Some(reason.clone());
        }
        EndChange::Reopen => {
            row.ended_at = None;
            row.end_reason = None;
        }
    }
    if t.touches_last_event {
        row.last_event_at = max_ts(&row.last_event_at, occurred);
    }
    row.updated_at = now.to_string();
}

/// Apply one top-level hook frame. `None` when the frame was already stored.
pub fn apply_in_tx(
    conn: &Connection,
    input: &ApplyInput,
    queued: &Queues,
    server: Option<&Server>,
) -> Result<Option<Applied>> {
    let frame = &input.frame;
    let ev = &frame.payload;
    let now = input.now.as_str();
    let occurred = if frame.sent_at.is_empty() {
        now
    } else {
        frame.sent_at.as_str()
    };
    if event_exists(conn, &frame.id)? {
        return Ok(None);
    }
    let name = ev.hook_event_name.as_str();
    if matches!(name, events::SUBAGENT_START | events::SUBAGENT_STOP) && ev.agent_id.is_some() {
        return apply_subagent(conn, input).map(Some);
    }
    let keep_prompts =
        settings::get_bool_in(conn, "sessions.store_prompt_previews")?.unwrap_or(false);
    let custom: HashMap<String, String> =
        settings::get_as_in(conn, "sessions.custom_names").unwrap_or_default();
    let existing = find_top_level(conn, &ev.session_id)?;
    let is_new = existing.is_none();
    let mut row = existing.unwrap_or_else(|| new_row(ev, input, &custom, occurred));
    if is_new {
        row.avatar_variant = avatars::next_variant(conn)?;
    }
    let snapshot = snapshot_of(&row, is_new);
    let state_before = snapshot.state;
    let base = row
        .repo_root
        .clone()
        .or_else(|| (!row.cwd.is_empty()).then(|| row.cwd.clone()));

    let mut t = transition(&snapshot, ev, occurred);
    let mut auto_decision = None;
    let mut note = t.note;
    let wants_card = matches!(
        t.card,
        Change::Set(OpenCard::Permission | OpenCard::Question)
    );

    if name == events::PERMISSION_REQUEST {
        if let Some(rule) = &input.auto_rule {
            // 05 §7.2: answered before any card is drawn; the state does not move.
            t = Transition {
                touches_last_event: true,
                pushes: vec![push::Event::Sessions],
                note: "always rule answered",
                ..Default::default()
            };
            note = t.note;
            auto_decision = Some(AutoDecision {
                frame_id: frame.id.clone(),
                rule: rule.clone(),
                output: mode_wire::PermissionDecision::allow().output(),
                sent: false,
            });
        }
    }
    let card_open = snapshot.state == SessionState::Waiting
        && snapshot.pending.as_ref().is_some_and(|p| p.frame_open);
    if wants_card && auto_decision.is_none() && card_open {
        // Several tool calls asked at once: the open card stays, this one waits.
        let p = build_pending(
            match t.card {
                Change::Set(c) => c,
                _ => OpenCard::Permission,
            },
            ev,
            &frame.id,
            frame.wants_decision,
            base.as_deref(),
            occurred,
        );
        queued
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(row.id.clone())
            .or_default()
            .push_back(p);
        t = Transition {
            touches_last_event: true,
            pushes: vec![push::Event::Sessions],
            note: "queued behind the open card",
            ..Default::default()
        };
        note = t.note;
    }
    if matches!(name, events::SESSION_END | events::SESSION_START) {
        queued
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&row.id);
    }

    apply_columns(&mut row, &t, occurred, now);
    let mut pending: Option<Pending> = match &t.card {
        Change::Keep => row.pending_json.as_deref().and_then(Pending::from_json),
        Change::Clear => None,
        Change::Set(card) => Some(build_pending(
            *card,
            ev,
            &frame.id,
            frame.wants_decision,
            base.as_deref(),
            occurred,
        )),
    };
    let mut state_after = t.state_after(state_before);
    if matches!(t.card, Change::Clear) && name != events::SESSION_END {
        if let Some(next) = pop_open_queued(queued, server, &row.id) {
            row.state = SessionState::Waiting.as_str().into();
            row.state_since = occurred.to_string();
            row.state_reason = Some("queued card".into());
            row.waiting_kind = Some(next.kind.as_str().into());
            state_after = SessionState::Waiting;
            pending = Some(next);
        }
    }
    row.pending_json = pending.as_ref().map(Pending::to_json);

    // Per-event columns (05 §4.2 "Mode stores").
    if !ev.transcript_path.is_empty() && row.transcript_path.is_none() {
        row.transcript_path = Some(ev.transcript_path.clone());
    }
    if let Some(mode) = &ev.permission_mode {
        row.permission_mode = Some(mode.clone());
    }
    if let Some(term) = &input.terminal {
        row.terminal_app = Some(term.app.clone());
        row.terminal_ref = Some(term.reference.clone());
    }
    if input.launched {
        row.tracking = "launched".into();
        row.launch_mode = input
            .launch_mode
            .clone()
            .unwrap_or_else(|| "embedded".into());
    } else if row.tracking == "discovered" {
        row.tracking = "hooked".into();
    }
    let custom_name = custom.get(&row.claude_session_id).cloned();
    let mut git_pass = None;
    match name {
        events::SESSION_START => {
            if !ev.cwd.is_empty() {
                row.cwd = ev.cwd.clone();
            }
            if !ev.transcript_path.is_empty() {
                row.transcript_path = Some(ev.transcript_path.clone());
            }
            if let Some(m) = ev.get("model").and_then(Value::as_str) {
                row.model = Some(m.to_string());
            }
            if let Some(title) = ev.get("session_title").and_then(Value::as_str) {
                if !title.trim().is_empty() {
                    row.title = Some(title.trim().to_string());
                }
            }
            if let Some(ctx) = ev.get("context_tokens").and_then(Value::as_i64) {
                row.context_tokens = Some(ctx);
            }
            if let Some(repo) = &input.repo {
                row.repo_root = repo.repo_root.clone();
                row.git_branch = repo.git_branch.clone().or(row.git_branch.take());
            }
            row.display_name = custom_name.clone().unwrap_or_else(|| {
                naming::display_name(
                    row.title.as_deref(),
                    row.repo_root.as_deref(),
                    &row.cwd,
                    &row.claude_session_id,
                )
            });
        }
        events::USER_PROMPT_SUBMIT => {
            row.num_turns += 1;
            if keep_prompts {
                if let Some(p) = ev.get("prompt").and_then(Value::as_str) {
                    row.last_prompt_preview = Some(preview(p, PROMPT_PREVIEW_CHARS));
                }
            }
        }
        events::STOP => {
            if let Some(m) = &ev.last_assistant_message {
                row.last_assistant_preview = Some(preview(m, ASSISTANT_PREVIEW_CHARS));
            }
            git_pass = row.repo_root.clone();
        }
        events::SESSION_END => {
            git_pass = row.repo_root.clone();
        }
        events::CWD_CHANGED => {
            let cwd = ev
                .get("new_cwd")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| ev.cwd.clone());
            if !cwd.is_empty() {
                row.cwd = cwd;
            }
            if let Some(repo) = &input.repo {
                row.repo_root = repo.repo_root.clone();
                row.git_branch = repo.git_branch.clone();
            }
            if row.title.is_none() && custom_name.is_none() {
                row.display_name = naming::display_name(
                    None,
                    row.repo_root.as_deref(),
                    &row.cwd,
                    &row.claude_session_id,
                );
            }
            git_pass = row.repo_root.clone();
        }
        events::POST_MODEL_SWITCH => {
            if let Some(m) = ev.get("to_model").and_then(Value::as_str) {
                row.model = Some(m.to_string());
            }
        }
        _ => {}
    }
    if is_new
        && input.repo.is_some()
        && name != events::SESSION_START
        && name != events::CWD_CHANGED
    {
        if let Some(repo) = &input.repo {
            row.repo_root = repo.repo_root.clone();
            row.git_branch = repo.git_branch.clone();
            row.avatar_color = naming::avatar_color(row.repo_root.as_deref(), &row.cwd);
            if custom_name.is_none() {
                row.display_name = naming::display_name(
                    row.title.as_deref(),
                    row.repo_root.as_deref(),
                    &row.cwd,
                    &row.claude_session_id,
                );
            }
        }
    }
    if is_new {
        repo::insert(conn, &row)?;
    } else {
        repo::replace(conn, &row)?;
    }

    // Files changed (05 §6.1) and the `(+N −M)` in the summary.
    let mut file_delta = None;
    if let Some(change) = files::from_post_tool_use(ev) {
        let paths = files::SessionPaths {
            id: row.id.clone(),
            cwd: row.cwd.clone(),
            repo_root: row.repo_root.clone(),
            started_at: row.started_at.clone(),
        };
        files::apply_hook_change(
            conn,
            &paths,
            &change,
            ev.tool_use_id(),
            ev.get("tool_response"),
            now,
        )?;
        files::recompute_totals(conn, &row.id)?;
        file_delta = Some((change.added, change.removed));
        git_pass = row.repo_root.clone();
    }

    // The hook event row.
    let mut payload = redact::for_storage(ev, keep_prompts);
    if frame.truncated {
        if let Value::Object(m) = &mut payload {
            m.insert("truncated".into(), Value::Bool(true));
        }
    }
    let payload_bytes = serde_json::to_vec(ev).map(|v| v.len()).unwrap_or(0);
    let state_changed = state_after != state_before;
    insert_event(
        conn,
        EventIn {
            id: frame.id.clone(),
            session_id: &row.id,
            source: "hook",
            event: name,
            tool_name: ev.tool_name().map(str::to_string),
            tool_use_id: ev.tool_use_id().map(str::to_string),
            agent_id: ev.agent_id.clone(),
            state_before: Some(state_before),
            state_after: state_changed.then_some(state_after),
            summary: event_summary(ev, base.as_deref(), keep_prompts, file_delta),
            payload: Some(payload),
            payload_bytes,
            occurred_at: occurred,
            received_at: now,
        },
    )?;
    if let Some(auto) = &auto_decision {
        insert_event(
            conn,
            EventIn {
                id: repo::new_id(),
                session_id: &row.id,
                source: "mode",
                event: "mode.auto_allow",
                tool_name: ev.tool_name().map(str::to_string),
                tool_use_id: ev.tool_use_id().map(str::to_string),
                agent_id: None,
                state_before: Some(state_before),
                state_after: None,
                summary: format!("allowed · rule: {}", auto.rule),
                payload: Some(serde_json::json!({"rule": auto.rule})),
                payload_bytes: 0,
                occurred_at: occurred,
                received_at: now,
            },
        )?;
        upsert_connector(
            conn,
            &row.id,
            "tool_rule",
            &auto.rule,
            "allowed",
            Some(serde_json::json!({"origin": "always rule"})),
            now,
        )?;
    }

    // Connectors (05 §6A.1).
    upsert_connector(
        conn,
        &row.id,
        "relay",
        "mode-hook",
        if input.relay_stale {
            "failed"
        } else {
            "connected"
        },
        Some(serde_json::json!({
            "last_event_at": occurred,
            "relay_version": frame.relay_version,
            "stale": input.relay_stale,
        })),
        now,
    )?;
    if let Some(server_name) = ev.tool_name().and_then(naming::mcp_server) {
        let status = match name {
            events::PRE_TOOL_USE => {
                let current: Option<String> = conn
                    .query_row(
                        "SELECT status FROM session_connectors WHERE session_id = ? AND kind = 'mcp_server' AND name = ?",
                        params![row.id, server_name],
                        |r| r.get(0),
                    )
                    .optional()?;
                match current.as_deref() {
                    Some("connected") => Some("connected"),
                    _ => Some("pending"),
                }
            }
            events::POST_TOOL_USE => Some("connected"),
            events::POST_TOOL_USE_FAILURE => Some("failed"),
            _ => None,
        };
        if let Some(status) = status {
            let tools = mcp_tools_seen(conn, &row.id, server_name)?;
            upsert_connector(
                conn,
                &row.id,
                "mcp_server",
                server_name,
                status,
                Some(serde_json::json!({"tools": tools})),
                now,
            )?;
        }
    }
    if name == events::PRE_TOOL_USE
        && matches!(
            row.permission_mode.as_deref(),
            Some("bypassPermissions") | Some("auto")
        )
    {
        if let Some(tool) = ev.tool_name() {
            upsert_connector(
                conn,
                &row.id,
                "tool_rule",
                tool,
                "allowed",
                Some(serde_json::json!({"origin": "permission mode"})),
                now,
            )?;
        }
    }
    if name == events::SESSION_END {
        close_children(conn, &row.id, now)?;
    }

    let mut pushes = t.pushes.clone();
    if !pushes.contains(&push::Event::Sessions) {
        pushes.push(push::Event::Sessions);
    }
    if is_new && !pushes.contains(&push::Event::Pill) {
        pushes.push(push::Event::Pill);
    }
    Ok(Some(Applied {
        session_id: row.id.clone(),
        claude_session_id: row.claude_session_id.clone(),
        event: name.to_string(),
        state_before,
        state_after,
        state_changed,
        pending,
        auto_decision,
        git_pass,
        pushes,
        note,
    }))
}
