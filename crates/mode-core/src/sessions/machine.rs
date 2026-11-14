//! The event → state table of 05 §4.1–§4.4 as one pure function:
//! [`transition`] takes what the row looks like now ([`SessionSnapshot`]),
//! the hook event and when it happened, and says what changes
//! ([`Transition`]). No I/O, no clock, no database: the engine applies the
//! result and `tests/state_machine.rs` is the table itself, one row per
//! event × state cell.
//!
//! Rules encoded here, in the order they are checked:
//!
//! 1. **Late frames** (§4.4): a row with `ended_at` set and
//!    `end_reason = 'inferred'` is re-opened by any frame; with any other
//!    `end_reason` only a `SessionStart` (not `compact`) re-opens it and
//!    every other frame is stored without changing state.
//! 2. **Sticky `failed`** (§4.1): nothing but `SessionStart(resume)` leaves
//!    `failed`. A decision event still opens its card (the user must be able
//!    to answer) and `SessionEnd` still closes the row, both without a state
//!    change.
//! 3. **Out-of-order frames** (§4.4): a frame older than the last one applied
//!    does not regress the state for the events whose meaning depends on
//!    order (`SessionStart`, `UserPromptSubmit`, `Stop`, `Notification`,
//!    `Elicitation`); the paired and terminal events always apply.
//! 4. The per-event cells of §4.2 / §4.3.
//! 5. **Promotion** (§4.1): an `untracked` row that received a hook event
//!    and would otherwise stay unchanged becomes `idle`.

use mode_wire::{HookEvent, events};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{SessionState, WaitingKind};
use crate::push;

/// What the engine knows about the row before applying an event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionSnapshot {
    pub state: SessionState,
    pub waiting_kind: Option<WaitingKind>,
    /// The open card, when any.
    pub pending: Option<PendingRef>,
    pub current_tool_use_id: Option<String>,
    /// `ended_at IS NOT NULL`.
    pub ended: bool,
    pub end_reason: Option<String>,
    /// `acknowledged_at IS NOT NULL`.
    pub acknowledged: bool,
    /// The newest `occurred_at` applied so far (for the out-of-order rule).
    pub last_event_at: Option<String>,
    /// The row is a subagent (never a pill candidate; same ladder).
    pub is_subagent: bool,
}

impl SessionSnapshot {
    pub fn new(state: SessionState) -> Self {
        Self {
            state,
            ..Default::default()
        }
    }

    pub fn working() -> Self {
        Self::new(SessionState::Working)
    }

    pub fn waiting(kind: WaitingKind, pending: Option<PendingRef>) -> Self {
        Self {
            state: SessionState::Waiting,
            waiting_kind: Some(kind),
            pending,
            ..Default::default()
        }
    }
}

/// The identity of the open card, enough for the pairing rules of §4.4.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PendingRef {
    pub kind: Option<WaitingKind>,
    pub tool_use_id: Option<String>,
    pub tool_name: Option<String>,
    /// The raw `tool_input` of the request (a 2.1.305 `PermissionRequest`
    /// carries no `tool_use_id`, so the pair is matched by tool and input).
    pub tool_input: Option<Value>,
    /// A relay frame is still open for it.
    pub frame_open: bool,
}

impl PendingRef {
    /// Does a `PostToolUse` / `PostToolUseFailure` event close this card?
    pub fn matches(&self, event: &HookEvent) -> bool {
        match (&self.tool_use_id, event.tool_use_id()) {
            (Some(a), Some(b)) => a == b,
            _ => {
                self.tool_name.is_some()
                    && self.tool_name.as_deref() == event.tool_name()
                    && self.tool_input.as_ref() == event.get("tool_input")
            }
        }
    }
}

/// A three-way change for an optional column.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Change<T> {
    #[default]
    Keep,
    Clear,
    Set(T),
}

impl<T> Change<T> {
    pub fn is_keep(&self) -> bool {
        matches!(self, Change::Keep)
    }
}

/// Which card the engine should open (it builds the [`super::Pending`] from the payload).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenCard {
    /// `PermissionRequest`: `always` rules are checked by the engine first.
    Permission,
    /// `PreToolUse` on `AskUserQuestion`.
    Question,
    /// `Notification` type `permission_prompt` with no frame open ("answer in the terminal").
    PermissionNotice,
    /// `Elicitation` hook or `elicitation_*` notification.
    Elicitation,
    /// `Notification` type `agent_needs_input`.
    AgentNeedsInput,
    /// `Notification` type `idle_prompt`.
    IdlePrompt,
}

/// The current tool after a `PreToolUse`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRef {
    pub name: String,
    pub tool_use_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EndChange {
    #[default]
    Keep,
    /// `ended_at = occurred_at`, `end_reason = reason`.
    End(String),
    /// `ended_at = NULL`, `end_reason = NULL` (resume / late frame on an inferred end).
    Reopen,
}

/// What an event does to the row. Every field defaults to "no change".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Transition {
    /// The new state, or `None` when the state is unchanged.
    pub state: Option<SessionState>,
    /// `sessions.state_reason` when the state changes.
    pub reason: Option<String>,
    pub waiting: Change<WaitingKind>,
    pub card: Change<OpenCard>,
    pub current_tool: Change<ToolRef>,
    pub ended: EndChange,
    /// The event moves `last_event_at` (false for out-of-order frames).
    pub touches_last_event: bool,
    /// The event closed the open card from the terminal side ("answered in terminal").
    pub answered_in_terminal: bool,
    /// Human-readable note on what the table decided (summaries, logs, tests).
    pub note: &'static str,
    pub pushes: Vec<push::Event>,
}

impl Transition {
    fn unchanged(note: &'static str) -> Self {
        Self {
            note,
            touches_last_event: true,
            ..Default::default()
        }
    }

    fn to(state: SessionState, reason: impl Into<String>, note: &'static str) -> Self {
        Self {
            state: Some(state),
            reason: Some(reason.into()),
            note,
            touches_last_event: true,
            pushes: vec![push::Event::Sessions, push::Event::Pill],
            ..Default::default()
        }
    }

    /// The state after this transition from `before`.
    pub fn state_after(&self, before: SessionState) -> SessionState {
        self.state.unwrap_or(before)
    }

    /// The waiting kind after this transition from `before`.
    pub fn waiting_after(&self, before: Option<WaitingKind>) -> Option<WaitingKind> {
        match &self.waiting {
            Change::Keep => before,
            Change::Clear => None,
            Change::Set(k) => Some(*k),
        }
    }

    fn with_waiting(mut self, kind: WaitingKind, card: OpenCard) -> Self {
        self.waiting = Change::Set(kind);
        self.card = Change::Set(card);
        self
    }

    fn clear_waiting(mut self) -> Self {
        self.waiting = Change::Clear;
        self.card = Change::Clear;
        self
    }

    fn clear_tool(mut self) -> Self {
        self.current_tool = Change::Clear;
        self
    }

    fn push(mut self, ev: push::Event) -> Self {
        if !self.pushes.contains(&ev) {
            self.pushes.push(ev);
        }
        self
    }
}

/// The notification types that set a waiting sub-reason (05 §4.3).
pub const WAITING_NOTIFICATIONS: &[&str] = &[
    "permission_prompt",
    "idle_prompt",
    "elicitation_dialog",
    "elicitation_url_dialog",
    "agent_needs_input",
];

fn source_is(event: &HookEvent, source: &str) -> bool {
    event.source.as_deref() == Some(source)
}

/// The event → state table (05 §4.2). `occurred_at` is the relay's `sent_at`.
pub fn transition(cur: &SessionSnapshot, event: &HookEvent, occurred_at: &str) -> Transition {
    let name = event.hook_event_name.as_str();
    let is_session_start = name == events::SESSION_START;
    let compact_start = is_session_start && source_is(event, "compact");
    let resume_start = is_session_start && source_is(event, "resume");

    // 1. Late frames (§4.4).
    let mut reopen = false;
    if cur.ended {
        if cur.end_reason.as_deref() == Some("inferred") || (is_session_start && !compact_start) {
            reopen = true;
        } else if name != events::SESSION_END {
            return Transition {
                touches_last_event: false,
                note: "late frame after SessionEnd: stored, no state change",
                ..Default::default()
            };
        }
    }

    // 2. Sticky `failed` (§4.1).
    if cur.state == SessionState::Failed && !resume_start {
        let mut t = match name {
            events::PERMISSION_REQUEST => Transition::unchanged("failed is sticky; card opened")
                .with_waiting(WaitingKind::Permission, OpenCard::Permission)
                .push(push::Event::Sessions),
            events::PRE_TOOL_USE if event.tool_name() == Some(events::ASK_USER_QUESTION_TOOL) => {
                Transition::unchanged("failed is sticky; card opened")
                    .with_waiting(WaitingKind::Question, OpenCard::Question)
                    .push(push::Event::Sessions)
            }
            events::SESSION_END => {
                let mut t = Transition::unchanged("failed stays failed; row closed")
                    .clear_waiting()
                    .clear_tool()
                    .push(push::Event::Sessions);
                t.ended = EndChange::End(event.reason.clone().unwrap_or_else(|| "other".into()));
                t
            }
            _ => Transition::unchanged("failed is sticky"),
        };
        // `waiting` is the only state that carries `waiting_kind`; a card on a
        // failed row keeps the state word and only the pending copy.
        if matches!(t.waiting, Change::Set(_)) {
            t.waiting = Change::Keep;
        }
        if reopen {
            t.ended = EndChange::Reopen;
        }
        return t;
    }

    // 3. Out-of-order frames (§4.4).
    let stale = cur
        .last_event_at
        .as_deref()
        .is_some_and(|last| occurred_at < last && !occurred_at.is_empty());
    if stale
        && matches!(
            name,
            events::SESSION_START
                | events::USER_PROMPT_SUBMIT
                | events::STOP
                | events::NOTIFICATION
                | events::ELICITATION
        )
    {
        return Transition {
            touches_last_event: false,
            ended: if reopen {
                EndChange::Reopen
            } else {
                EndChange::Keep
            },
            note: "out of order: older than the last event, state kept",
            ..Default::default()
        };
    }

    let state = cur.state;
    let kind = cur.waiting_kind;
    let mut t = match name {
        // ------------------------------------------------------------ start
        events::SESSION_START => {
            if compact_start && !cur.ended {
                Transition::unchanged("compact: same session continues")
            } else if state == SessionState::Idle && !cur.ended && kind.is_none() {
                Transition::unchanged("duplicate SessionStart on an idle row")
            } else {
                let source = event.source.clone().unwrap_or_else(|| "startup".into());
                Transition::to(
                    SessionState::Idle,
                    format!("SessionStart:{source}"),
                    "start",
                )
                .clear_waiting()
                .clear_tool()
            }
        }

        // ----------------------------------------------------------- prompt
        events::USER_PROMPT_SUBMIT => {
            if state == SessionState::Working && kind.is_none() {
                Transition::unchanged("prompt while working (queued or stop-hook turn)")
            } else {
                Transition::to(SessionState::Working, "UserPromptSubmit", "prompt").clear_waiting()
            }
        }

        // ------------------------------------------------------------ tools
        events::PRE_TOOL_USE => {
            let tool = ToolRef {
                name: event.tool_name().unwrap_or("tool").to_string(),
                tool_use_id: event.tool_use_id().map(str::to_string),
            };
            if event.tool_name() == Some(events::ASK_USER_QUESTION_TOOL) {
                let mut t = Transition::to(
                    SessionState::Waiting,
                    "PreToolUse:AskUserQuestion",
                    "question",
                )
                .with_waiting(WaitingKind::Question, OpenCard::Question);
                t.current_tool = Change::Set(tool);
                t
            } else {
                let mut t = match (state, kind) {
                    (SessionState::Waiting, Some(WaitingKind::Permission))
                    | (SessionState::Waiting, Some(WaitingKind::Question))
                    | (SessionState::Waiting, Some(WaitingKind::AgentNeedsInput)) => {
                        Transition::unchanged("tool call while a card is open")
                    }
                    (SessionState::Working, _) => Transition::unchanged("tool call"),
                    _ => Transition::to(SessionState::Working, "PreToolUse", "tool call")
                        .clear_waiting(),
                };
                t.current_tool = Change::Set(tool);
                t.push(push::Event::Sessions)
            }
        }

        events::PERMISSION_REQUEST => {
            Transition::to(SessionState::Waiting, "PermissionRequest", "permission")
                .with_waiting(WaitingKind::Permission, OpenCard::Permission)
        }

        events::POST_TOOL_USE | events::POST_TOOL_USE_FAILURE => {
            let closes_card = cur.pending.as_ref().is_some_and(|p| {
                p.kind.is_some_and(WaitingKind::holds_decision) && p.matches(event)
            });
            let mut t = if closes_card {
                let mut t = Transition::to(SessionState::Working, name, "answered in terminal")
                    .clear_waiting();
                t.answered_in_terminal = true;
                t
            } else {
                match (state, kind) {
                    (SessionState::Waiting, Some(WaitingKind::Permission))
                    | (SessionState::Waiting, Some(WaitingKind::Question))
                    | (SessionState::Waiting, Some(WaitingKind::AgentNeedsInput)) => {
                        Transition::unchanged("tool finished while a card is open")
                    }
                    (SessionState::Working, _) => Transition::unchanged("tool finished"),
                    _ => {
                        Transition::to(SessionState::Working, name, "tool finished").clear_waiting()
                    }
                }
            };
            let same_tool = match (&cur.current_tool_use_id, event.tool_use_id()) {
                (Some(a), Some(b)) => a == b,
                (None, _) => true,
                (Some(_), None) => true,
            };
            if same_tool {
                t.current_tool = Change::Clear;
            }
            t.push(push::Event::Sessions)
        }

        // ---------------------------------------------------- notifications
        events::NOTIFICATION => match event.notification_type.as_deref() {
            Some("permission_prompt") => {
                let frame_open = cur
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.frame_open && p.kind == Some(WaitingKind::Permission));
                if frame_open || kind == Some(WaitingKind::Permission) {
                    Transition::unchanged("permission_prompt duplicates the open PermissionRequest")
                } else if kind == Some(WaitingKind::Question) {
                    Transition::unchanged("question card stays over a permission notice")
                } else {
                    Transition::to(
                        SessionState::Waiting,
                        "Notification:permission_prompt",
                        "permission notice (answer in the terminal)",
                    )
                    .with_waiting(WaitingKind::Permission, OpenCard::PermissionNotice)
                }
            }
            Some("idle_prompt") => match (state, kind) {
                (SessionState::Waiting, Some(WaitingKind::IdlePrompt)) => {
                    Transition::unchanged("duplicate idle_prompt")
                }
                (SessionState::Waiting, Some(_)) => {
                    Transition::unchanged("idle_prompt does not replace an open card")
                }
                _ => Transition::to(
                    SessionState::Waiting,
                    "Notification:idle_prompt",
                    "idle prompt",
                )
                .with_waiting(WaitingKind::IdlePrompt, OpenCard::IdlePrompt),
            },
            Some("elicitation_dialog" | "elicitation_url_dialog") => match (state, kind) {
                (SessionState::Waiting, Some(WaitingKind::Elicitation)) => {
                    Transition::unchanged("duplicate elicitation notice")
                }
                (SessionState::Waiting, Some(WaitingKind::Permission))
                | (SessionState::Waiting, Some(WaitingKind::Question)) => {
                    Transition::unchanged("elicitation notice does not replace an open card")
                }
                _ => Transition::to(
                    SessionState::Waiting,
                    format!(
                        "Notification:{}",
                        event.notification_type.as_deref().unwrap_or("")
                    ),
                    "elicitation",
                )
                .with_waiting(WaitingKind::Elicitation, OpenCard::Elicitation),
            },
            Some("elicitation_complete" | "elicitation_response") => {
                if kind == Some(WaitingKind::Elicitation) {
                    Transition::to(
                        SessionState::Working,
                        format!(
                            "Notification:{}",
                            event.notification_type.as_deref().unwrap_or("")
                        ),
                        "elicitation answered",
                    )
                    .clear_waiting()
                } else {
                    Transition::unchanged("elicitation ended; nothing was waiting")
                }
            }
            Some("agent_needs_input") => match (state, kind) {
                (SessionState::Waiting, Some(WaitingKind::AgentNeedsInput)) => {
                    Transition::unchanged("duplicate agent_needs_input")
                }
                (SessionState::Waiting, Some(WaitingKind::Permission))
                | (SessionState::Waiting, Some(WaitingKind::Question)) => {
                    Transition::unchanged("agent notice does not replace an open card")
                }
                _ => Transition::to(
                    SessionState::Waiting,
                    "Notification:agent_needs_input",
                    "a subagent needs input",
                )
                .with_waiting(WaitingKind::AgentNeedsInput, OpenCard::AgentNeedsInput),
            },
            Some("agent_completed") => {
                if kind == Some(WaitingKind::AgentNeedsInput) {
                    Transition::to(
                        SessionState::Working,
                        "Notification:agent_completed",
                        "subagent done",
                    )
                    .clear_waiting()
                } else {
                    Transition::unchanged("agent completed: info line")
                }
            }
            Some("quota_auto_resume_stale" | "quota_auto_resume_disabled") => {
                if state == SessionState::Waiting {
                    Transition::unchanged("usage limit note; the open card stays")
                } else {
                    Transition::to(
                        SessionState::Idle,
                        format!(
                            "Notification:{}",
                            event.notification_type.as_deref().unwrap_or("")
                        ),
                        "paused at the usage limit",
                    )
                    .clear_tool()
                }
            }
            Some("quota_auto_resume_fired") => {
                Transition::unchanged("resumed after the usage limit reset: info line")
            }
            Some("auth_success") => Transition::unchanged("signed in: info line"),
            _ => Transition::unchanged("notification of an unknown type: info line"),
        },

        // ------------------------------------------------------------- stop
        events::STOP => match state {
            SessionState::Waiting => Transition::unchanged("Stop while a card is open"),
            SessionState::Unread => Transition::unchanged("duplicate Stop"),
            _ => Transition::to(SessionState::Unread, "Stop", "turn finished").clear_tool(),
        },

        events::STOP_FAILURE => Transition::to(SessionState::Failed, "StopFailure", "turn failed")
            .clear_waiting()
            .clear_tool(),

        // -------------------------------------------------------- subagents
        events::SUBAGENT_START => {
            Transition::unchanged("subagent started: child row").push(push::Event::Sessions)
        }
        events::SUBAGENT_STOP => {
            Transition::unchanged("subagent stopped: child row").push(push::Event::Sessions)
        }

        // ------------------------------------------------------ bookkeeping
        events::PRE_COMPACT => Transition::unchanged("compaction starts: event row only"),
        events::POST_COMPACT => Transition::unchanged("compaction done: event row only"),
        events::CWD_CHANGED => {
            Transition::unchanged("cwd changed: repo label").push(push::Event::Sessions)
        }
        events::PRE_MODEL_SWITCH => Transition::unchanged("model switch announced"),
        events::POST_MODEL_SWITCH => {
            Transition::unchanged("model switched").push(push::Event::Sessions)
        }

        // ------------------------------------------------------ elicitation
        events::ELICITATION => match (state, kind) {
            (SessionState::Waiting, Some(WaitingKind::Permission))
            | (SessionState::Waiting, Some(WaitingKind::Question)) => {
                Transition::unchanged("elicitation does not replace an open card")
            }
            (SessionState::Waiting, Some(WaitingKind::Elicitation)) => {
                Transition::unchanged("duplicate Elicitation")
            }
            _ => Transition::to(SessionState::Waiting, "Elicitation", "elicitation")
                .with_waiting(WaitingKind::Elicitation, OpenCard::Elicitation),
        },

        // -------------------------------------------------------------- end
        events::SESSION_END => {
            let reason = event.reason.clone().unwrap_or_else(|| "other".into());
            let mut t = match state {
                SessionState::Done => Transition::unchanged("done stays done"),
                SessionState::Unread if cur.ended => Transition::unchanged("already ended"),
                _ if cur.acknowledged && state != SessionState::Waiting => {
                    Transition::to(SessionState::Done, "SessionEnd", "ended, acknowledged")
                }
                _ => Transition::to(SessionState::Unread, "SessionEnd", "ended"),
            }
            .clear_waiting()
            .clear_tool()
            .push(push::Event::Sessions)
            .push(push::Event::Pill);
            t.ended = EndChange::End(reason);
            return t;
        }

        _ => Transition::unchanged("unknown hook event: stored"),
    };

    if reopen {
        t.ended = EndChange::Reopen;
        if t.state.is_none() {
            t.pushes = vec![push::Event::Sessions, push::Event::Pill];
        }
    }

    // 5. Promotion (§4.1): the first hook event makes an untracked row live.
    if state == SessionState::Untracked && t.state.is_none() {
        t.state = Some(SessionState::Idle);
        t.reason = Some(format!("{name}:promoted"));
        t.pushes = vec![push::Event::Sessions, push::Event::Pill];
    }

    t
}

/// Serializable view of a transition for diagnostics and tests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionSummary {
    pub state: Option<SessionState>,
    pub waiting: Option<WaitingKind>,
    pub note: String,
}

impl TransitionSummary {
    pub fn of(t: &Transition, before: &SessionSnapshot) -> Self {
        Self {
            state: t.state,
            waiting: t.waiting_after(before.waiting_kind),
            note: t.note.to_string(),
        }
    }
}
