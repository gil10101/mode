//! `pty` (08 §3.3): the PTY host for embedded sessions (05 §8.1–§8.3),
//! launch and resume of `claude`, SIGINT semantics, the headless
//! `stream-json` driver (05 §8.5) and the single-driver hand-off with the
//! "send a prompt" table (05 §14).
//!
//! * [`host`]: one pseudo-terminal per embedded session, scrollback ring,
//!   coalesced output, the "output flowing" heuristic.
//! * [`launch`]: finding `claude`, its arguments and environment, launch and
//!   resume (the `launched` row, the `MODE_LAUNCH_ID` join, the 30 s notice).
//! * [`headless`]: `claude -p … --output-format stream-json` turns, the
//!   stream parser and what each event writes.
//! * [`driver`]: `pty | headless | none` and the prompt routing table.
//! * [`api`]: one plain function per command for `mode-app` to wrap.
//!
//! The service ([`Pty`]) owns the host, the drivers and the running turns;
//! it reports through two hooks: a [`push::SinkFn`] that delivers the
//! per-session `pty.*` / `turn.*` streams to the app window, and a
//! [`PublishFn`] that requests the debounced `sessions.changed` /
//! `pill.changed` pushes. Nothing here blocks on the async runtime: commands
//! run on the blocking pool, the PTY and headless I/O on their own threads.

pub mod api;
pub mod driver;
pub mod headless;
pub mod host;
pub mod launch;

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::push::{self, Push};
use crate::sessions::Engine;
use crate::sessions::embedded::ProcessExit;

pub use api::ApiError;
pub use driver::{Driver, Drivers, HandOff, PromptFacts, PromptRoute, Refusal};
pub use host::{ExitInfo, PtyHost, PtyProc, Scrollback};
pub use launch::ClaudeCommand;

/// Lines kept for replay (05 §8.3, 03 §3.8; `SCROLLBACK_LINES` in the UI).
pub const SCROLLBACK_LINES: usize = 10_000;
/// Byte cap of the replay ring.
pub const SCROLLBACK_BYTES: usize = 2 * 1024 * 1024;
/// Size of a PTY nobody has measured yet (launch from the dialog).
pub const DEFAULT_SIZE: (u16, u16) = (100, 30);

/// Requests the debounced area pushes (`mode-app`: `CoreState::publish`).
pub type PublishFn = Arc<dyn Fn(&[push::Event]) + Send + Sync>;

/// The host's clocks (05 §8.1, §8.3, §14; 08 §2.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timings {
    /// Output coalescing window (one chunk per frame).
    pub coalesce: Duration,
    /// "Output flowing" → `idle` after this much silence.
    pub idle_after: Duration,
    /// "no hook events — check hook install" after this long.
    pub no_hooks_after: Duration,
    /// Bracketed paste → `\r`.
    pub paste_cr_delay: Duration,
    /// PTY / headless exit → close the row if no `SessionEnd` arrived.
    pub session_end_grace: Duration,
    /// "Close terminal" / quit: SIGINT, then wait this long for the turn.
    pub turn_wait: Duration,
    /// Each later step of "Close terminal" (second SIGINT, EOF, hang-up).
    pub exit_step: Duration,
    /// Housekeeping cadence of the pump.
    pub tick: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Timings {
            coalesce: Duration::from_millis(16),
            idle_after: Duration::from_secs(2),
            no_hooks_after: Duration::from_secs(30),
            paste_cr_delay: Duration::from_millis(50),
            session_end_grace: Duration::from_secs(2),
            turn_wait: Duration::from_secs(10),
            exit_step: Duration::from_millis(1500),
            tick: Duration::from_millis(100),
        }
    }
}

/// Service configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub timings: Timings,
    pub scrollback_lines: usize,
    pub scrollback_bytes: usize,
    /// Skip discovery and run this (tests; `None` in the app).
    pub claude: Option<ClaudeCommand>,
    /// Extra environment for every `claude` (tests).
    pub extra_env: Vec<(String, String)>,
    /// Where an agent profile's files go (`<data_dir>/agents`): the system
    /// prompt file, an isolated `CLAUDE_CONFIG_DIR` (03 §7E); `None` passes
    /// the prompt inline and skips the isolation.
    pub agents_dir: Option<std::path::PathBuf>,
    /// The keyring an agent's session provider reads its secret from (06
    /// §10.1A); `None` launches such an agent without the provider's env.
    pub keyring: Option<std::sync::Arc<crate::keyring::Keyring>>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            timings: Timings::default(),
            scrollback_lines: SCROLLBACK_LINES,
            scrollback_bytes: SCROLLBACK_BYTES,
            claude: None,
            extra_env: Vec::new(),
            agents_dir: None,
            keyring: None,
        }
    }
}

// ------------------------------------------------------------------ payloads

/// `pty.output`: raw master bytes, base64 (RFC 4648 with padding).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyOutput {
    pub session_id: String,
    pub data: String,
    /// Stream offset of the first byte (bytes since the PTY spawned).
    pub offset: u64,
}

/// `pty.started`: launch, resume, "Open a terminal".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyStarted {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

/// `pty.exited`: the driver went `pty → none`; `code` is `None` for a signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtyExited {
    pub session_id: String,
    pub code: Option<i32>,
}

/// `turn.started`: `none → headless`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnStarted {
    pub session_id: String,
    pub turn_id: String,
}

/// `turn.delta`: assistant text streamed while a headless turn runs (the
/// `stream_event` text deltas, coalesced per ~16 ms; appended by the UI to
/// the streaming bubble until the thread item lands).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnDelta {
    pub session_id: String,
    pub turn_id: String,
    pub text: String,
    /// Set for a subagent's message (its spawning `Task` / `Agent` tool use).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Finished,
    Stopped,
    Failed,
}

/// `turn.finished`: `headless → none` (the process is gone).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnFinished {
    pub session_id: String,
    pub turn_id: String,
    pub outcome: TurnOutcome,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Standard base64 with padding.
pub fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn b64_decode(text: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|e| format!("not base64: {e}"))
}

// ------------------------------------------------------------------- service

/// Options a launch dialog gave that the first headless turn of an
/// "Overview only" row must still pass (`--permission-mode`, `--model`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirstTurnOpts {
    pub permission_mode: Option<String>,
    pub model: Option<String>,
    /// The agent profile's flags (03 §7E), resolved at launch.
    pub flags: crate::agents::LaunchFlags,
}

pub(crate) struct Inner {
    pub engine: Arc<Engine>,
    pub sink: push::SinkFn,
    pub publish: PublishFn,
    pub config: Config,
    pub host: PtyHost,
    pub drivers: Arc<Drivers>,
    pub turns: headless::Turns,
    pub first_turn: Mutex<HashMap<String, FirstTurnOpts>>,
}

/// The PTY host service: one per app (`mode-app` keeps it in its state).
#[derive(Clone)]
pub struct Pty {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Pty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pty")
            .field("live", &self.inner.host.all_live().len())
            .finish_non_exhaustive()
    }
}

impl Pty {
    /// `sink` delivers the `pty.*` / `turn.*` streams (to the app window);
    /// `publish` requests `sessions.changed` / `pill.changed`. Installs the
    /// engine's driver source so `SessionDetail.driver` is live.
    pub fn new(engine: Arc<Engine>, sink: push::SinkFn, publish: PublishFn, config: Config) -> Pty {
        let drivers = Arc::new(Drivers::default());
        let d = Arc::clone(&drivers);
        engine
            .embedded()
            .set_driver_source(Arc::new(move |id: &str| d.get(id).as_str()));
        Pty {
            inner: Arc::new(Inner {
                engine,
                sink,
                publish,
                config,
                host: PtyHost::default(),
                drivers,
                turns: headless::Turns::default(),
                first_turn: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn engine(&self) -> &Arc<Engine> {
        &self.inner.engine
    }

    /// The driver of a session (05 §14).
    pub fn driver(&self, session_id: &str) -> Driver {
        self.inner.drivers.get(session_id)
    }

    /// The session's PTY (live, else the last exited one kept for replay).
    pub fn proc(&self, session_id: &str) -> Option<Arc<PtyProc>> {
        self.inner.host.by_row(session_id)
    }

    /// Drop the replay buffers of a dismissed row.
    pub fn forget(&self, session_id: &str) -> usize {
        self.inner.host.forget(session_id)
    }
}

impl Inner {
    pub(crate) fn emit<T: Serialize>(&self, event: push::Event, payload: &T) {
        match serde_json::to_value(payload) {
            Ok(payload) => (self.sink)(&Push { event, payload }),
            Err(e) => tracing::warn!(target: "pty", error = %e, "payload did not serialize"),
        }
    }

    pub(crate) fn publish(&self, events: &[push::Event]) {
        if !events.is_empty() {
            (self.publish)(events);
        }
    }

    /// Publish what an engine call changed; log a failure (the PTY keeps
    /// running whatever the database does).
    pub(crate) fn engine_pushes(&self, what: &str, r: crate::db::Result<Vec<push::Event>>) {
        match r {
            Ok(events) => self.publish(&events),
            Err(e) => tracing::warn!(target: "pty", error = %e, "{what} failed"),
        }
    }

    fn proc_opts(&self, infer: bool) -> host::ProcOpts {
        host::ProcOpts {
            scrollback_lines: self.config.scrollback_lines,
            scrollback_bytes: self.config.scrollback_bytes,
            timings: self.config.timings.clone(),
            infer,
        }
    }
}

/// The host's callbacks: output to the attached Terminal tab, the "without
/// hooks" inference, the 30 s notice, `/clear` following, and the exit.
struct Events(Arc<Inner>);

impl host::HostEvents for Events {
    fn output(&self, proc: &Arc<PtyProc>, offset: u64, bytes: &[u8]) {
        if proc.attached.load(Ordering::SeqCst) {
            self.0.emit(
                push::Event::PtyOutput,
                &PtyOutput {
                    session_id: proc.row_id(),
                    data: b64(bytes),
                    offset,
                },
            );
        }
    }

    fn activity(&self, proc: &Arc<PtyProc>, working: bool) {
        if !proc.infer.load(Ordering::SeqCst) || proc.closing.load(Ordering::SeqCst) {
            return;
        }
        let row = proc.row_id();
        // Hooks arrived after all: the state machine owns the state again.
        if self
            .0
            .engine
            .hook_event_since(&row, &proc.spawned_ts, None)
            .unwrap_or(false)
        {
            proc.infer.store(false, Ordering::SeqCst);
            return;
        }
        self.0
            .engine_pushes("state inference", self.0.engine.infer_state(&row, working));
    }

    fn tick(&self, proc: &Arc<PtyProc>) {
        let inner = &self.0;
        // `/clear` starts a new Claude session in the same process: follow it.
        if let Some(reg) = inner.engine.embedded().launch(&proc.launch_id) {
            let row = proc.row_id();
            if reg.row_id != row {
                proc.set_row_id(&reg.row_id);
                inner.drivers.rename(&row, &reg.row_id);
                let (cols, rows) = proc.size();
                inner.emit(
                    push::Event::PtyStarted,
                    &PtyStarted {
                        session_id: reg.row_id.clone(),
                        cols,
                        rows,
                    },
                );
            }
        }
        if !proc.hooks_checked.load(Ordering::SeqCst)
            && proc.spawned_at.elapsed() >= inner.config.timings.no_hooks_after
        {
            proc.hooks_checked.store(true, Ordering::SeqCst);
            let row = proc.row_id();
            let seen = inner
                .engine
                .hook_event_since(&row, &proc.spawned_ts, None)
                .unwrap_or(true);
            if !seen {
                // Inference is switched on before the notice is written, so output that
                // follows the notice is never dropped, and it adopts the output state of
                // this moment: a burst already flowing produces no further `activity`
                // edge, so waiting for one would leave the row idle until the next pause.
                proc.infer.store(true, Ordering::SeqCst);
                inner.engine_pushes("no-hook notice", inner.engine.mark_no_hook_events(&row));
                let working_now = proc.silent_for() < inner.config.timings.idle_after;
                inner.engine_pushes(
                    "state inference",
                    inner.engine.infer_state(&row, working_now),
                );
            }
        }
    }

    fn exited(&self, proc: &Arc<PtyProc>, exit: &ExitInfo) {
        let inner = Arc::clone(&self.0);
        let row = proc.row_id();
        tracing::info!(target: "pty", session = %row, code = ?exit.code, signal = ?exit.signal, "terminal exited");
        inner.engine.forget_process(&row);
        inner.engine.embedded().forget(&proc.launch_id);
        let generation = inner.drivers.generation(&row);
        inner.drivers.release(&row, HandOff::PtyExited, generation);
        inner.emit(
            push::Event::PtyExited,
            &PtyExited {
                session_id: row.clone(),
                code: exit.code,
            },
        );
        inner.publish(&[push::Event::Sessions]);
        // 05 §4.2: close the row from the exit only if no `SessionEnd`
        // arrived within the grace, and only if no new process took over.
        // The wait runs off the pump so the process reads as exited as soon
        // as the announcement above is out.
        let pe = ProcessExit {
            source: "pty",
            code: exit.code,
            signal: exit.signal.clone(),
            interrupted_by_mode: proc.sigint_sent.load(Ordering::SeqCst),
            error: None,
        };
        let grace = inner.config.timings.session_end_grace;
        let spawned = std::thread::Builder::new()
            .name("mode-pty-grace".into())
            .spawn(move || {
                std::thread::sleep(grace);
                if inner.drivers.generation(&row) == generation
                    && inner.drivers.get(&row) == Driver::None
                {
                    inner.engine_pushes("exit record", inner.engine.record_process_exit(&row, &pe));
                }
            });
        if let Err(e) = spawned {
            tracing::warn!(target: "pty", error = %e, "exit record thread did not start");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips_raw_bytes() {
        let raw = [0u8, 0x1b, b'[', 0xff, 0xe2, 0x82];
        assert_eq!(b64_decode(&b64(&raw)).unwrap(), raw);
        assert_eq!(b64(b"hi"), "aGk=");
        assert!(b64_decode("***").is_err());
    }

    #[test]
    fn payloads_use_the_contract_field_names() {
        let v = serde_json::to_value(TurnDelta {
            session_id: "s".into(),
            turn_id: "t".into(),
            text: "x".into(),
            parent_tool_use_id: None,
        })
        .unwrap();
        assert!(v.get("parent_tool_use_id").is_none());
        let v = serde_json::to_value(TurnFinished {
            session_id: "s".into(),
            turn_id: "t".into(),
            outcome: TurnOutcome::Stopped,
        })
        .unwrap();
        assert_eq!(v["outcome"], "stopped");
        let v = serde_json::to_value(PtyExited {
            session_id: "s".into(),
            code: None,
        })
        .unwrap();
        assert!(v["code"].is_null());
    }
}
