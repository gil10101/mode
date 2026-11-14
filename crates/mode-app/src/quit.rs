//! Quit with live embedded sessions (03 §2.1, 08 §2.6): when a quit is
//! requested (`RunEvent::ExitRequested` from ⌘Q / Ctrl+Q / the OS, or the
//! profile row's Quit through `app_quit`) and `pty_list` is not empty, the
//! exit is held and the app window gets `app:quit-confirm {sessions}`; the
//! webview answers `app_quit {confirm: true}` (SIGINT every PTY and headless
//! turn, wait up to 10 s, answer pending decisions `no_opinion`, then exit)
//! or `app_quit {confirm: false}` (stay). With nothing live the app exits at
//! once. [`QuitGate`] is the pure state machine; the rest is glue.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mode_core::pty::api::{self, LiveSession, QuitAllArgs};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::state::CoreState;

/// The event the app window gets when a quit needs confirmation.
pub const QUIT_CONFIRM_EVENT: &str = "app:quit-confirm";

/// How long quit waits for the turns to end (08 §2.6).
pub const QUIT_WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Running,
    /// `app:quit-confirm` was sent; waiting for `app_quit {confirm}`.
    Confirming,
    /// Confirmed (or nothing was live): the next exit request goes through.
    Quitting,
}

/// What to do with an `ExitRequested`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitDecision {
    Allow,
    /// Hold the exit and ask about these sessions.
    Confirm(Vec<LiveSession>),
}

/// What `app_quit` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuitStep {
    /// Nothing live: exit now.
    Exit,
    /// Confirmed: stop the sessions, then exit.
    StopThenExit,
    /// Ask first.
    Confirm(Vec<LiveSession>),
    /// The user chose to stay.
    Cancelled,
}

#[derive(Debug, Default)]
pub struct QuitGate {
    phase: Mutex<Phase>,
}

impl QuitGate {
    pub fn phase(&self) -> Phase {
        *CoreState::lock(&self.phase)
    }

    fn set(&self, p: Phase) {
        *CoreState::lock(&self.phase) = p;
    }

    /// An exit was requested by the OS or the menu. `live` is read only
    /// when the answer depends on it.
    pub fn on_exit_requested(&self, live: impl FnOnce() -> Vec<LiveSession>) -> ExitDecision {
        if self.phase() == Phase::Quitting {
            return ExitDecision::Allow;
        }
        let sessions = live();
        if sessions.is_empty() {
            self.set(Phase::Quitting);
            ExitDecision::Allow
        } else {
            self.set(Phase::Confirming);
            ExitDecision::Confirm(sessions)
        }
    }

    /// `app_quit {confirm?}` from the webview.
    pub fn on_app_quit(
        &self,
        confirm: Option<bool>,
        live: impl FnOnce() -> Vec<LiveSession>,
    ) -> QuitStep {
        match confirm {
            Some(false) => {
                self.set(Phase::Running);
                QuitStep::Cancelled
            }
            Some(true) => {
                self.set(Phase::Quitting);
                QuitStep::StopThenExit
            }
            None => {
                let sessions = live();
                if sessions.is_empty() {
                    self.set(Phase::Quitting);
                    QuitStep::Exit
                } else {
                    self.set(Phase::Confirming);
                    QuitStep::Confirm(sessions)
                }
            }
        }
    }
}

/// `app:quit-confirm`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuitConfirm {
    pub sessions: Vec<LiveSession>,
}

/// `app_quit`'s answer: `quitting` | `confirm` | `canceled`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuitAnswer {
    pub status: &'static str,
    pub sessions: Vec<LiveSession>,
}

/// The sessions a quit would interrupt (none before the PTY host runs).
pub fn live_sessions(core: &CoreState) -> Vec<LiveSession> {
    match core.pty.get() {
        Some(pty) => api::pty_list(pty).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "quit: pty_list failed");
            Vec::new()
        }),
        None => Vec::new(),
    }
}

/// Show the app window and ask.
pub fn ask(app: &AppHandle, sessions: Vec<LiveSession>) {
    tracing::info!(live = sessions.len(), "quit: asking to confirm");
    if let Some(w) = app.get_webview_window(crate::APP_WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    if let Err(e) = app.emit_to(
        crate::APP_WINDOW,
        QUIT_CONFIRM_EVENT,
        QuitConfirm { sessions },
    ) {
        tracing::warn!(error = %e, "emit app:quit-confirm failed");
    }
}

/// `RunEvent::ExitRequested` (main thread).
pub fn on_exit_requested(app: &AppHandle, api: &tauri::ExitRequestApi, code: Option<i32>) {
    if code == Some(tauri::RESTART_EXIT_CODE) {
        return;
    }
    let Some(core) = app.try_state::<Arc<CoreState>>() else {
        return;
    };
    let core = Arc::clone(core.inner());
    match core.quit.on_exit_requested(|| live_sessions(&core)) {
        ExitDecision::Allow => tracing::info!("quit: exiting"),
        ExitDecision::Confirm(sessions) => {
            api.prevent_exit();
            ask(app, sessions);
        }
    }
}

/// Confirmed: SIGINT every live PTY and headless turn, wait for them (up
/// to [`QUIT_WAIT`]), answer pending decisions `no_opinion` so the relays
/// exit at once, then exit.
pub fn stop_then_exit(app: AppHandle, core: Arc<CoreState>) {
    tauri::async_runtime::spawn(async move {
        let c2 = Arc::clone(&core);
        let _ = tauri::async_runtime::spawn_blocking(move || crate::services::stop(&c2)).await;
        if let Some(pty) = core.pty.get().cloned() {
            let job = tauri::async_runtime::spawn_blocking(move || {
                api::pty_quit_all(
                    &pty,
                    QuitAllArgs {
                        wait_ms: Some(QUIT_WAIT.as_millis() as u64),
                    },
                )
            })
            .await;
            match job {
                Ok(Ok(r)) => tracing::info!(
                    signalled = r.signalled.len(),
                    remaining = r.remaining.len(),
                    "quit: sessions stopped"
                ),
                Ok(Err(e)) => tracing::warn!(error = %e, "quit: pty_quit_all failed"),
                Err(e) => tracing::warn!(error = %e, "quit: pty_quit_all task failed"),
            }
        }
        if let Some(server) = core.server() {
            for id in server.pending_ids().await {
                server.no_opinion(&id).await;
            }
        }
        app.exit(0);
    });
}

/// `RunEvent::Exit`: stop the running Chat turns and automation runs
/// (partial text kept, runs marked failed), then fold the WAL back (08 §2.6).
pub fn on_exit(app: &AppHandle) {
    if let Some(core) = app.try_state::<Arc<CoreState>>() {
        crate::services::stop(&core);
    }
    if let Some(core) = app.try_state::<Arc<CoreState>>()
        && !core.is_db_newer()
        && let Err(e) = core.db.checkpoint()
    {
        tracing::warn!(error = %e, "quit: WAL checkpoint failed");
    }
    tracing::info!("quit: bye");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(n: usize) -> Vec<LiveSession> {
        (0..n)
            .map(|i| LiveSession {
                session_id: format!("s{i}"),
                display_name: format!("proj{i}"),
                driver: "pty".into(),
                state: "working".into(),
            })
            .collect()
    }

    #[test]
    fn nothing_live_exits_at_once() {
        let g = QuitGate::default();
        assert_eq!(g.phase(), Phase::Running);
        assert_eq!(g.on_exit_requested(|| live(0)), ExitDecision::Allow);
        assert_eq!(g.phase(), Phase::Quitting);
        let g = QuitGate::default();
        assert_eq!(g.on_app_quit(None, || live(0)), QuitStep::Exit);
        assert_eq!(g.phase(), Phase::Quitting);
    }

    #[test]
    fn live_sessions_hold_the_exit_until_confirmed() {
        let g = QuitGate::default();
        // ⌘Q with two terminals open: held, the window is asked.
        assert_eq!(
            g.on_exit_requested(|| live(2)),
            ExitDecision::Confirm(live(2))
        );
        assert_eq!(g.phase(), Phase::Confirming);
        // A second ⌘Q while the question is open asks again.
        assert!(matches!(
            g.on_exit_requested(|| live(2)),
            ExitDecision::Confirm(_)
        ));
        // Confirm: stop the sessions, then the exit goes through without
        // reading the list again.
        assert_eq!(
            g.on_app_quit(Some(true), || live(2)),
            QuitStep::StopThenExit
        );
        assert_eq!(g.phase(), Phase::Quitting);
        assert_eq!(
            g.on_exit_requested(|| panic!("not read once quitting")),
            ExitDecision::Allow
        );
    }

    #[test]
    fn cancel_returns_to_running_and_the_profile_quit_asks_too() {
        let g = QuitGate::default();
        assert_eq!(g.on_app_quit(None, || live(1)), QuitStep::Confirm(live(1)));
        assert_eq!(g.phase(), Phase::Confirming);
        assert_eq!(g.on_app_quit(Some(false), || live(1)), QuitStep::Cancelled);
        assert_eq!(g.phase(), Phase::Running);
        // The sessions ended meanwhile: the next quit goes straight through.
        assert_eq!(g.on_exit_requested(|| live(0)), ExitDecision::Allow);
    }

    #[test]
    fn payloads_name_the_sessions() {
        let v = serde_json::to_value(QuitConfirm { sessions: live(1) }).unwrap();
        assert_eq!(v["sessions"][0]["session_id"], "s0");
        assert_eq!(v["sessions"][0]["driver"], "pty");
        let v = serde_json::to_value(QuitAnswer {
            status: "confirm",
            sessions: live(1),
        })
        .unwrap();
        assert_eq!(v["status"], "confirm");
        assert_eq!(
            serde_json::to_value(Phase::Confirming).unwrap(),
            "confirming"
        );
    }
}
