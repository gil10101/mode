//! Sessions (05 §4, §7, §9–§12, §17): the state vocabulary, the pure
//! table-driven state machine ([`machine`]), the persisting engine
//! ([`engine`]) that turns relay frames into `sessions` / `session_events` /
//! `file_changes` / `session_connectors` rows, the `always` rules of §7.2
//! ([`always`]), roster naming ([`naming`]), untracked discovery
//! ([`untracked`]) and process liveness ([`liveness`]).
//!
//! The [`AgentAdapter`] trait is the §17 seam: Claude Code is the only
//! implementation in v1 (ADR-0004); the machine and the engine consume only
//! what the adapter exposes.

pub mod always;
pub mod apply;
pub mod avatars;
pub mod embedded;
pub mod engine;
pub mod history;
pub mod liveness;
pub mod machine;
pub mod naming;
pub mod summary;
pub mod thread;
pub mod untracked;
pub mod views;
pub mod writes;

use std::fmt;
use std::str::FromStr;

use mode_wire::HookEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use engine::{
    Answer, Applied, AutoDecision, Changed, DecideError, DecideOutcome, Decision, Engine, JumpInfo,
    Reconciled, SessionRef,
};
pub use machine::{SessionSnapshot, Transition, transition};
pub use thread::{CardStatus, ItemBody, ThreadItem};
pub use views::{
    AgentRow, LinkRow, PillPayload, SessionDetail, SessionHeader, SessionItem, TranscriptInfo,
};
pub use writes::InferredEnd;

/// Roster states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Working,
    /// Needs you: permission, question, idle prompt.
    Waiting,
    Failed,
    /// Finished, not yet looked at.
    Unread,
    /// Open, no activity (the state a new row starts in).
    #[default]
    Idle,
    /// Finished and acknowledged.
    Done,
    /// Seen on disk, no hooks.
    Untracked,
}

impl SessionState {
    pub const ALL: &[SessionState] = &[
        SessionState::Working,
        SessionState::Waiting,
        SessionState::Failed,
        SessionState::Unread,
        SessionState::Idle,
        SessionState::Done,
        SessionState::Untracked,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Working => "working",
            SessionState::Waiting => "waiting",
            SessionState::Failed => "failed",
            SessionState::Unread => "unread",
            SessionState::Idle => "idle",
            SessionState::Done => "done",
            SessionState::Untracked => "untracked",
        }
    }

    /// The states that count as "needs you" for the pill and the badge.
    pub fn needs_you(self) -> bool {
        matches!(self, SessionState::Waiting)
    }

    /// Order in the roster (05 §11.4): `waiting` → `working` → `failed` →
    /// `unread` → `idle` → `done` → `untracked`.
    pub fn roster_rank(self) -> u8 {
        match self {
            SessionState::Waiting => 0,
            SessionState::Working => 1,
            SessionState::Failed => 2,
            SessionState::Unread => 3,
            SessionState::Idle => 4,
            SessionState::Done => 5,
            SessionState::Untracked => 6,
        }
    }

    /// Whether the user may dismiss a row in this state (05 §12): never a
    /// live `working` / `waiting` / `idle` session.
    pub fn dismissable(self, ended: bool) -> bool {
        ended
            || matches!(
                self,
                SessionState::Done
                    | SessionState::Failed
                    | SessionState::Unread
                    | SessionState::Untracked
            )
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SessionState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        SessionState::ALL
            .iter()
            .copied()
            .find(|st| st.as_str() == s)
            .ok_or_else(|| format!("unknown session state `{s}`"))
    }
}

/// `sessions.waiting_kind` (05 §4.3; the data model's CHECK constraint).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitingKind {
    Permission,
    Question,
    IdlePrompt,
    Elicitation,
    AgentNeedsInput,
}

impl WaitingKind {
    pub const ALL: &[WaitingKind] = &[
        WaitingKind::Permission,
        WaitingKind::Question,
        WaitingKind::IdlePrompt,
        WaitingKind::Elicitation,
        WaitingKind::AgentNeedsInput,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            WaitingKind::Permission => "permission",
            WaitingKind::Question => "question",
            WaitingKind::IdlePrompt => "idle_prompt",
            WaitingKind::Elicitation => "elicitation",
            WaitingKind::AgentNeedsInput => "agent_needs_input",
        }
    }

    /// `permission` and `question` hold a decision the relay is waiting on;
    /// the other three can only be answered in the terminal.
    pub fn holds_decision(self) -> bool {
        matches!(self, WaitingKind::Permission | WaitingKind::Question)
    }
}

impl fmt::Display for WaitingKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WaitingKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        WaitingKind::ALL
            .iter()
            .copied()
            .find(|k| k.as_str() == s)
            .ok_or_else(|| format!("unknown waiting kind `{s}`"))
    }
}

/// `sessions.pending_json`: the display copy of the open card, exactly what
/// the overlay renders without a second call. `tool_input` is the 4 KB,
/// redacted storage copy (05 §3.5); `answerable` is true while the relay's
/// frame is open (a decision can still be written), false when only a
/// `Notification` was seen, after the relay's deadline, or when it was
/// answered in the terminal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    pub kind: WaitingKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// "Bash: npm test" / "Edit src/main.rs" / "asks: Which?" / the notification text.
    pub summary: String,
    /// `AskUserQuestion`'s `questions` array as received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub questions: Option<Value>,
    /// The `Notification` / `Elicitation` message, when that is the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub received_at: String,
    pub answerable: bool,
    /// The relay frame id the decision goes to (`None` for notification-only cards).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
    /// The relay gave up (deadline / cancel / EOF): "timed out — answer in the terminal".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
}

impl Pending {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    pub fn from_json(text: &str) -> Option<Pending> {
        serde_json::from_str(text).ok()
    }
}

/// An agent integration (05 §17). Claude Code is the only implementation in
/// v1 (ADR-0004); a second agent is a new implementation, not a UI change.
pub trait AgentAdapter: Send + Sync {
    /// Stable adapter id (`claude_code`).
    fn id(&self) -> &'static str;

    /// The full event → state table: what a hook event does to a session in
    /// the given snapshot (05 §4.2). The core owns `sessions.state`; the
    /// adapter only says what the event means.
    fn transition(
        &self,
        current: &SessionSnapshot,
        event: &HookEvent,
        occurred_at: &str,
    ) -> Transition;

    /// The state a hook event moves a `working` session to, or `None` when
    /// the event does not change the state on its own.
    fn state_after(&self, event: &HookEvent) -> Option<SessionState> {
        self.transition(
            &SessionSnapshot::working(),
            event,
            "9999-12-31T23:59:59.999Z",
        )
        .state
    }
}

/// The Claude Code adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeCode;

impl AgentAdapter for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude_code"
    }

    fn transition(
        &self,
        current: &SessionSnapshot,
        event: &HookEvent,
        occurred_at: &str,
    ) -> Transition {
        machine::transition(current, event, occurred_at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_round_trip_through_strings_and_serde() {
        for s in SessionState::ALL {
            assert_eq!(s.as_str().parse::<SessionState>().unwrap(), *s);
            assert_eq!(serde_json::to_string(s).unwrap(), format!("\"{}\"", s));
        }
        assert!("nope".parse::<SessionState>().is_err());
        assert!(SessionState::Waiting.needs_you());
        assert!(!SessionState::Working.needs_you());
        assert!(SessionState::Waiting.roster_rank() < SessionState::Done.roster_rank());
        for k in WaitingKind::ALL {
            assert_eq!(k.as_str().parse::<WaitingKind>().unwrap(), *k);
            assert_eq!(serde_json::to_string(k).unwrap(), format!("\"{}\"", k));
        }
        assert!(WaitingKind::Permission.holds_decision());
        assert!(!WaitingKind::IdlePrompt.holds_decision());
        assert!(!SessionState::Working.dismissable(false));
        assert!(SessionState::Working.dismissable(true));
        assert!(SessionState::Unread.dismissable(false));
    }

    #[test]
    fn claude_code_adapter_maps_the_obvious_events() {
        let a = ClaudeCode;
        assert_eq!(a.id(), "claude_code");
        let ev = |name: &str| HookEvent::new(name, "s");
        assert_eq!(a.state_after(&ev("SessionStart")), Some(SessionState::Idle));
        assert_eq!(
            a.state_after(&ev("UserPromptSubmit")),
            None,
            "a working session stays working on a prompt"
        );
        assert_eq!(
            a.state_after(&ev("PermissionRequest")),
            Some(SessionState::Waiting)
        );
        assert_eq!(a.state_after(&ev("Stop")), Some(SessionState::Unread));
        assert_eq!(
            a.state_after(&ev("StopFailure")),
            Some(SessionState::Failed)
        );
        assert_eq!(a.state_after(&ev("PreCompact")), None);

        let mut q = ev("PreToolUse");
        q.set("tool_name", "AskUserQuestion");
        assert_eq!(a.state_after(&q), Some(SessionState::Waiting));
        let mut n = ev("Notification");
        n.set("notification_type", "idle_prompt");
        assert_eq!(a.state_after(&n), Some(SessionState::Waiting));
        n.set("notification_type", "auth_success");
        assert_eq!(a.state_after(&n), None);
    }

    #[test]
    fn pending_round_trips_and_omits_empty_fields() {
        let p = Pending {
            kind: WaitingKind::Permission,
            tool_name: Some("Bash".into()),
            tool_input: Some(serde_json::json!({"command": "npm test"})),
            tool_use_id: None,
            summary: "Bash: npm test".into(),
            questions: None,
            message: None,
            received_at: "2026-10-21T12:00:00.000Z".into(),
            answerable: true,
            frame_id: Some("f1".into()),
            timed_out: false,
        };
        let json = p.to_json();
        assert!(!json.contains("questions"));
        assert!(!json.contains("timed_out"));
        assert_eq!(Pending::from_json(&json).unwrap(), p);
    }
}
