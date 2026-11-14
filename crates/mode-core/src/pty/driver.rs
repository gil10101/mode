//! The single-driver invariant of ADR-0005 and 05 §14: a session has at most
//! one driver — `pty`, `headless` or `none` — and the "send a prompt" table
//! that decides which channel a prompt takes, or which sentence the box
//! shows instead. Both are pure; [`Drivers`] holds the per-session state
//! behind one lock so two prompts cannot both start a turn.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

use crate::sessions::{SessionState, WaitingKind};

/// Who drives a session (05 §14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Driver {
    #[default]
    None,
    Pty,
    Headless,
}

impl Driver {
    pub fn as_str(self) -> &'static str {
        match self {
            Driver::None => "none",
            Driver::Pty => "pty",
            Driver::Headless => "headless",
        }
    }
}

/// What moves the driver (05 §14 "Hand-off rule").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandOff {
    /// A new embedded session in a PTY.
    Launch,
    /// Resume (or "Open a terminal") in a PTY.
    Resume,
    /// A prompt while no PTY is alive: a headless turn.
    PromptWithoutPty,
    /// The PTY's process exited.
    PtyExited,
    /// The headless turn ended (`result`, or SIGINT) and its process is gone.
    TurnEnded,
}

/// Refusals of the hand-off table, verbatim for the UI.
pub const STOP_TURN_FIRST: &str = "stop the running turn first";
pub const TURN_RUNNING: &str = "a turn is running";
pub const TERMINAL_OPEN: &str = "a terminal is already open for this session";

/// 05 §14: `none → pty` on launch / resume, `none → headless` on a prompt
/// without a PTY, `pty → none` on exit, `headless → none` when the turn
/// ends; `pty` and `headless` never coexist. Resume while a PTY is alive is
/// a no-op (the terminal is already there).
pub fn hand_off(current: Driver, ev: HandOff) -> Result<Driver, &'static str> {
    use Driver::*;
    use HandOff::*;
    match (current, ev) {
        (None, Launch | Resume) => Ok(Pty),
        (Pty, Resume) => Ok(Pty),
        (Pty, Launch) => Err(TERMINAL_OPEN),
        (Headless, Launch | Resume) => Err(STOP_TURN_FIRST),
        (None, PromptWithoutPty) => Ok(Headless),
        (Headless, PromptWithoutPty) => Err(TURN_RUNNING),
        (Pty, PromptWithoutPty) => Err(TERMINAL_OPEN),
        (Pty, PtyExited) => Ok(None),
        (Headless, TurnEnded) => Ok(None),
        // A stale report (the other driver's process, or twice): unchanged.
        (d, PtyExited | TurnEnded) => Ok(d),
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Slot {
    driver: Driver,
    /// Bumped on every `none → pty | headless`; a delayed exit report for an
    /// older process checks it before closing the row.
    generation: u64,
}

/// The per-session driver state.
#[derive(Debug, Default)]
pub struct Drivers {
    slots: Mutex<HashMap<String, Slot>>,
}

impl Drivers {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Slot>> {
        self.slots.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn get(&self, session_id: &str) -> Driver {
        self.lock()
            .get(session_id)
            .map(|s| s.driver)
            .unwrap_or_default()
    }

    pub fn generation(&self, session_id: &str) -> u64 {
        self.lock().get(session_id).map_or(0, |s| s.generation)
    }

    /// Apply a hand-off atomically; the new driver and its generation.
    pub fn apply(&self, session_id: &str, ev: HandOff) -> Result<(Driver, u64), &'static str> {
        let mut map = self.lock();
        let slot = map.entry(session_id.to_string()).or_default();
        let next = hand_off(slot.driver, ev)?;
        if slot.driver == Driver::None && next != Driver::None {
            slot.generation += 1;
        }
        slot.driver = next;
        Ok((next, slot.generation))
    }

    /// Release only if `generation` still owns the slot (a new process may
    /// have taken over meanwhile).
    pub fn release(&self, session_id: &str, ev: HandOff, generation: u64) -> Driver {
        let mut map = self.lock();
        let slot = map.entry(session_id.to_string()).or_default();
        if slot.generation == generation {
            if let Ok(next) = hand_off(slot.driver, ev) {
                slot.driver = next;
            }
        }
        slot.driver
    }

    /// Move a PTY's slot to the row its launch now drives (`/clear`).
    pub fn rename(&self, from: &str, to: &str) {
        let mut map = self.lock();
        if let Some(slot) = map.remove(from) {
            map.insert(to.to_string(), slot);
        }
    }
}

// ------------------------------------------------------- send a prompt (§14)

/// What the UI may offer next to a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// **Jump to terminal**.
    Jump,
    /// **Resume** (opens the existing terminal when one is alive).
    Resume,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Jump => "jump",
            Action::Resume => "resume",
        }
    }
}

/// Why the prompt box is disabled, in the words the box shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    pub message: &'static str,
    pub action: Option<Action>,
}

/// The sentences of 05 §14 (and two this module adds: subagent rows, and an
/// embedded session some other terminal is driving).
pub mod say {
    pub const WORKING: &str = "Claude is working…";
    pub const CARD_FIRST: &str = "answer the pending card first";
    pub const IN_TERMINAL: &str = "this needs input in the terminal";
    pub const TURN_RUNNING: &str = super::TURN_RUNNING;
    pub const FAILED: &str = "this session failed; Resume to see why in the terminal";
    pub const EXTERNAL: &str = "send prompts from your terminal";
    pub const UNTRACKED: &str = "Resume to track this session";
    pub const SUBAGENT: &str = "prompts go to the parent session";
    pub const CLOSING: &str = "the terminal is closing";
    pub const ELSEWHERE: &str = "this session is running in another terminal";
}

/// The facts the table reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFacts {
    /// `hooked` | `launched` | `discovered`.
    pub tracking: String,
    pub state: SessionState,
    pub waiting_kind: Option<WaitingKind>,
    pub ended: bool,
    pub is_subagent: bool,
    pub driver: Driver,
    /// The PTY is being closed ("Close terminal" / quit).
    pub pty_closing: bool,
    /// A live `claude` this host did not start drives the row.
    pub foreign_process: bool,
}

/// Where a prompt goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptRoute {
    Pty,
    Headless,
    Refused(Refusal),
}

fn refuse(message: &'static str, action: Option<Action>) -> PromptRoute {
    PromptRoute::Refused(Refusal { message, action })
}

fn waiting_route(kind: Option<WaitingKind>, open: PromptRoute) -> PromptRoute {
    match kind {
        Some(WaitingKind::IdlePrompt) => open,
        Some(WaitingKind::Elicitation | WaitingKind::AgentNeedsInput) => {
            refuse(say::IN_TERMINAL, Some(Action::Jump))
        }
        Some(WaitingKind::Permission | WaitingKind::Question) | None => {
            refuse(say::CARD_FIRST, None)
        }
    }
}

/// The "send a prompt" table of 05 §14, row by row.
pub fn route_prompt(f: &PromptFacts) -> PromptRoute {
    if f.is_subagent {
        return refuse(say::SUBAGENT, None);
    }
    if f.tracking == "discovered" || f.state == SessionState::Untracked {
        return refuse(say::UNTRACKED, Some(Action::Resume));
    }
    if f.tracking != "launched" {
        return refuse(say::EXTERNAL, Some(Action::Jump));
    }
    match f.driver {
        Driver::Headless => refuse(say::TURN_RUNNING, None),
        Driver::Pty => {
            if f.pty_closing {
                return refuse(say::CLOSING, None);
            }
            match f.state {
                SessionState::Failed => refuse(say::FAILED, Some(Action::Resume)),
                SessionState::Working => refuse(say::WORKING, None),
                SessionState::Waiting => waiting_route(f.waiting_kind, PromptRoute::Pty),
                SessionState::Idle | SessionState::Unread | SessionState::Done => PromptRoute::Pty,
                SessionState::Untracked => refuse(say::UNTRACKED, Some(Action::Resume)),
            }
        }
        Driver::None => {
            if f.state == SessionState::Failed {
                return refuse(say::FAILED, Some(Action::Resume));
            }
            if f.ended {
                return PromptRoute::Headless;
            }
            if f.foreign_process {
                return refuse(say::ELSEWHERE, Some(Action::Jump));
            }
            match f.state {
                SessionState::Working => refuse(say::WORKING, None),
                SessionState::Waiting => waiting_route(f.waiting_kind, PromptRoute::Headless),
                _ => PromptRoute::Headless,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generations_guard_late_releases() {
        let d = Drivers::default();
        assert_eq!(d.get("s"), Driver::None);
        let (drv, g1) = d.apply("s", HandOff::PromptWithoutPty).unwrap();
        assert_eq!(drv, Driver::Headless);
        assert_eq!(d.release("s", HandOff::TurnEnded, g1), Driver::None);
        let (_, g2) = d.apply("s", HandOff::Resume).unwrap();
        assert!(g2 > g1);
        assert_eq!(
            d.release("s", HandOff::TurnEnded, g1),
            Driver::Pty,
            "an old turn's late release leaves the new PTY alone"
        );
        d.rename("s", "t");
        assert_eq!(d.get("t"), Driver::Pty);
        assert_eq!(d.get("s"), Driver::None);
    }
}
