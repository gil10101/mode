//! The core's background work (08 §5.2, §5.3): the hook-frame pump, the
//! relay resolutions, the transcript tailer, the liveness probe, the
//! untracked scan, the debounced git pass, and the `*.changed` sink that
//! emits every push to both windows.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use mode_core::files;
use mode_core::ipc::{Inbound, Server};
use mode_core::push::{Event, Push, SinkFn};
use mode_core::sessions::untracked;
use mode_core::transcript::tailer::Polled;
use mode_core::transcript::{self, WatchMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;

use crate::state::{CoreState, RecentFrame};

/// Git pass debounce after a writing tool's `PostToolUse` (05 §6.2).
const GIT_PASS_DEBOUNCE: Duration = Duration::from_secs(2);
/// At most one git pass per session per this interval (05 §6.2).
const GIT_PASS_MIN_INTERVAL: Duration = Duration::from_secs(10);
/// Transcript stat cadence for live sessions (05 §5.2).
const TAIL_INTERVAL: Duration = Duration::from_secs(5);
/// Liveness probe cadence (05 §9.1).
const PROBE_INTERVAL: Duration = Duration::from_secs(15);
/// Untracked discovery cadence (05 §10).
const SCAN_INTERVAL: Duration = Duration::from_secs(600);
/// The no-UI safety answers `no_opinion` this long before the relay's deadline.
const SAFETY_MARGIN: Duration = Duration::from_secs(2);
/// How long a decision frame that arrives before the overlay was ever drawn
/// waits for it to appear (the frame's own pill usually draws it) before the
/// core answers `no_opinion`.
pub const UI_GRACE: Duration = Duration::from_millis(1_500);

#[derive(Debug, Clone, Serialize)]
pub struct CoreChanged {
    /// `ipc_listening`, `ipc_error`, `hook`.
    pub reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub ipc_listening: bool,
    pub hook_frames_received: u64,
}

impl CoreChanged {
    pub fn reason(reason: &'static str, state: &CoreState) -> Self {
        Self {
            reason,
            event: None,
            session_id: None,
            ipc_listening: state.ipc_listening.load(Ordering::Relaxed),
            hook_frames_received: state.frames_received.load(Ordering::Relaxed),
        }
    }
}

pub fn emit_core_changed(app: &AppHandle, payload: CoreChanged) {
    if let Err(e) = app.emit(Event::Core.wire_name(), payload) {
        tracing::warn!(error = %e, "emit core.changed failed");
    }
}

/// The publisher's sink: every push goes to both windows as `<area>:changed`
/// (the Tauri spelling of `<area>.changed`).
pub fn sink(app: AppHandle) -> SinkFn {
    let seen: std::sync::Mutex<std::collections::HashSet<Event>> = Default::default();
    Arc::new(move |push: &Push| {
        if let Err(e) = app.emit(push.event.wire_name(), &push.payload) {
            tracing::warn!(event = push.event.name(), error = %e, "emit failed");
        } else if CoreState::lock(&seen).insert(push.event) {
            tracing::info!(event = push.event.name(), "first push");
        }
    })
}

/// Drain the IPC channel for the life of the app: every frame goes through
/// the engine on the blocking pool; decision frames the engine did not answer
/// get `no_opinion` at once when no webview can show a card, or shortly
/// before the relay's own deadline otherwise.
pub async fn pump(
    app: AppHandle,
    server: Arc<Server>,
    mut rx: mpsc::Receiver<Inbound>,
    state: Arc<CoreState>,
) {
    while let Some(inbound) = rx.recv().await {
        let n = state.frames_received.fetch_add(1, Ordering::Relaxed) + 1;
        let frame = &inbound.frame;
        let (id, event, session) = (
            frame.id.clone(),
            frame.event.clone(),
            frame.payload.session_id.clone(),
        );
        let wants_decision = frame.wants_decision;
        let wait = frame.decision_wait_ms.map(Duration::from_millis);
        let sent_at = frame.sent_at.clone();
        tracing::info!(id = %id, event = %event, session = %session, n, "hook frame");

        let engine = Arc::clone(&state.engine);
        let applied =
            match tauri::async_runtime::spawn_blocking(move || engine.apply(&inbound)).await {
                Ok(Ok(applied)) => applied,
                Ok(Err(e)) => {
                    tracing::error!(id = %id, error = %e, "engine could not apply the frame");
                    None
                }
                Err(e) => {
                    tracing::error!(id = %id, error = %e, "engine task failed");
                    None
                }
            };

        state.record_frame(recent_frame(&event, &sent_at));

        let auto_answered = applied
            .as_ref()
            .and_then(|a| a.auto_decision.as_ref())
            .is_some_and(|d| d.sent);
        if wants_decision && !auto_answered && server.is_pending(&id) {
            if state.smoke {
                server.no_opinion(&id).await;
                tracing::info!(id = %id, "MODE_SMOKE: answered no_opinion at once");
            } else {
                let wait = wait
                    .unwrap_or(mode_core::ipc::DEFAULT_DECISION_WAIT)
                    .saturating_sub(SAFETY_MARGIN)
                    .max(Duration::from_secs(1));
                tauri::async_runtime::spawn(hold_for_card(
                    Arc::clone(&state),
                    Arc::clone(&server),
                    id.clone(),
                    wait,
                ));
            }
        }

        if let Some(a) = &applied {
            state.publish(a.pushes.iter().copied());
            if a.git_pass.is_some() {
                schedule_git_pass(&state, a.session_id.clone());
            }
        }
        emit_core_changed(
            &app,
            CoreChanged {
                reason: "hook",
                event: Some(event),
                session_id: Some(session),
                ipc_listening: true,
                hook_frames_received: n,
            },
        );
    }
    tracing::warn!("hook IPC channel closed");
}

/// A frame for Diagnostics: its event, now, and the relay's `sent_at` →
/// applied latency (`None` when the stamp is unreadable or implausible).
pub fn recent_frame(event: &str, sent_at: &str) -> RecentFrame {
    let now = time::OffsetDateTime::now_utc();
    let latency_ms = mode_core::db::parse_timestamp(sent_at)
        .map(|t| (now - t).whole_milliseconds())
        .and_then(|ms| u64::try_from(ms).ok())
        .filter(|ms| *ms < 3_600_000);
    RecentFrame {
        event: event.chars().take(64).collect(),
        at: mode_core::db::format_timestamp(now),
        latency_ms,
    }
}

/// The PTY host's sink (package 8): `pty.*` / `turn.*` streams go straight
/// to the app window, every chunk in order, never through the debounced
/// publisher (it coalesces latest-wins and would lose bytes).
pub fn pty_sink(app: AppHandle) -> SinkFn {
    Arc::new(move |push: &Push| {
        if let Err(e) = app.emit_to(crate::APP_WINDOW, push.event.wire_name(), &push.payload) {
            tracing::warn!(event = push.event.name(), error = %e, "emit failed");
        }
    })
}

/// Keep a decision frame open for the user (08 §5.2): when the overlay was
/// never drawn, wait up to [`UI_GRACE`] for it (the frame's pill normally
/// draws it) and answer `no_opinion` at once if it does not appear; then
/// hold the card until shortly before the relay's deadline.
async fn hold_for_card(state: Arc<CoreState>, server: Arc<Server>, id: String, wait: Duration) {
    let started = Instant::now();
    if !state.is_ui_ready() {
        let notified = state.ui_ready_notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !state.is_ui_ready() {
            let _ = tokio::time::timeout(UI_GRACE, notified).await;
        }
        if !state.is_ui_ready() {
            if server.no_opinion(&id).await {
                tracing::info!(id = %id, "the overlay was never drawn: answered no_opinion");
            }
            return;
        }
    }
    tokio::time::sleep(wait.saturating_sub(started.elapsed())).await;
    if server.no_opinion(&id).await {
        tracing::warn!(id = %id, "card unanswered before the relay deadline: no_opinion");
    }
}

/// Relay-side resolutions (deadline / cancel / EOF gray the card; 05 §7.4).
pub async fn resolutions_task(state: Arc<CoreState>, server: Arc<Server>) {
    let mut rx = server.resolutions();
    loop {
        match rx.recv().await {
            Ok(r) => {
                let engine = Arc::clone(&state.engine);
                match tauri::async_runtime::spawn_blocking(move || engine.resolved(&r)).await {
                    Ok(Ok(pushes)) => state.publish(pushes),
                    Ok(Err(e)) => tracing::warn!(error = %e, "resolution not recorded"),
                    Err(e) => tracing::warn!(error = %e, "resolution task failed"),
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!(n, "resolutions lagged");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

async fn watcher_recv(w: &mut Option<Watcher>) -> Option<PathBuf> {
    match w {
        Some(w) => w.recv().await,
        None => std::future::pending().await,
    }
}

/// Transcript tailing (05 §5.2): a native watcher on the projects root with
/// a 250 ms per-file debounce, plus a 5 s stat of every tailed file.
pub async fn tailer_task(state: Arc<CoreState>) {
    let root = transcript::default_projects_root();
    let mut watcher = root.as_ref().filter(|r| r.is_dir()).and_then(|r| {
        match Watcher::start(r, WatchMode::Native, transcript::watch::DEBOUNCE) {
            Ok(w) => {
                tracing::info!(root = %r.display(), "transcript watcher running");
                Some(w)
            }
            Err(e) => {
                tracing::warn!(root = %r.display(), error = %e, "transcript watcher unavailable; polling only");
                None
            }
        }
    });
    let mut last_sync = Instant::now() - TAIL_INTERVAL;
    loop {
        let due_in = TAIL_INTERVAL.saturating_sub(last_sync.elapsed());
        let path = match tokio::time::timeout(due_in, watcher_recv(&mut watcher)).await {
            Ok(Some(p)) => Some(p),
            Ok(None) => {
                watcher = None;
                None
            }
            Err(_) => None,
        };
        let (db, tailers) = (Arc::clone(&state.db), Arc::clone(&state.tailers));
        let sync_now = path.is_none() || last_sync.elapsed() >= TAIL_INTERVAL;
        if sync_now {
            last_sync = Instant::now();
        }
        let job =
            tauri::async_runtime::spawn_blocking(move || -> mode_core::db::Result<Vec<Polled>> {
                let mut t = CoreState::lock(&tailers);
                if sync_now {
                    t.sync(&db)?;
                }
                match path {
                    Some(p) => Ok(t.poll_path(&db, &p)?.into_iter().collect()),
                    None => t.poll_all(&db),
                }
            })
            .await;
        match job {
            Ok(Ok(polled)) => {
                if polled.iter().any(|p| p.report.changed) {
                    state.publish([Event::Sessions]);
                }
            }
            Ok(Err(e)) => tracing::warn!(error = %e, "transcript poll failed"),
            Err(e) => tracing::warn!(error = %e, "transcript task failed"),
        }
    }
}

/// The 15 s liveness probe (05 §9.1).
pub async fn liveness_task(state: Arc<CoreState>) {
    loop {
        tokio::time::sleep(PROBE_INTERVAL).await;
        let engine = Arc::clone(&state.engine);
        match tauri::async_runtime::spawn_blocking(move || engine.probe_liveness()).await {
            Ok(Ok(pushes)) => state.publish(pushes),
            Ok(Err(e)) => tracing::warn!(error = %e, "liveness probe failed"),
            Err(e) => tracing::warn!(error = %e, "liveness task failed"),
        }
    }
}

/// Untracked discovery at launch and every 10 minutes (05 §10), with the
/// 30-minute inactivity rule.
pub async fn scan_task(state: Arc<CoreState>) {
    let root = transcript::default_projects_root();
    loop {
        if let Some(root) = root.clone().filter(|r| r.is_dir()) {
            let db = Arc::clone(&state.db);
            let job = tauri::async_runtime::spawn_blocking(move || {
                let now = mode_core::db::now_rfc3339();
                let report = untracked::scan(&db, &root, &now, untracked::DISCOVERY_WINDOW)?;
                let closed = untracked::mark_inactive(&db, &now, untracked::INACTIVE_AFTER)?;
                Ok::<_, mode_core::db::DbError>((report, closed))
            })
            .await;
            match job {
                Ok(Ok((report, closed))) => {
                    tracing::info!(
                        scanned = report.scanned,
                        discovered = report.discovered,
                        refreshed = report.refreshed,
                        closed,
                        "untracked scan"
                    );
                    if report.discovered + report.refreshed + closed > 0 {
                        state.publish([Event::Sessions, Event::Pill]);
                    }
                }
                Ok(Err(e)) => tracing::warn!(error = %e, "untracked scan failed"),
                Err(e) => tracing::warn!(error = %e, "scan task failed"),
            }
        }
        tokio::time::sleep(SCAN_INTERVAL).await;
    }
}

/// Run a git pass for the session 2 s from now, at most once per 10 s per
/// session (05 §6.2); the roster re-renders when rows changed.
pub fn schedule_git_pass(state: &Arc<CoreState>, session_id: String) {
    let wait = {
        let mut g = CoreState::lock(&state.git_passes);
        if !g.scheduled.insert(session_id.clone()) {
            return;
        }
        let since_last = g
            .last
            .get(&session_id)
            .map(|t| GIT_PASS_MIN_INTERVAL.saturating_sub(t.elapsed()))
            .unwrap_or(Duration::ZERO);
        GIT_PASS_DEBOUNCE.max(since_last)
    };
    let state = Arc::clone(state);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(wait).await;
        let db = Arc::clone(&state.db);
        let sid = session_id.clone();
        let result =
            tauri::async_runtime::spawn_blocking(move || files::run_git_pass(&db, &sid)).await;
        {
            let mut g = CoreState::lock(&state.git_passes);
            g.scheduled.remove(&session_id);
            g.last.insert(session_id.clone(), Instant::now());
        }
        match result {
            Ok(Ok(Some((report, totals)))) => {
                tracing::debug!(session = %session_id, upgraded = report.upgraded, inserted = report.inserted, files = totals.files, "git pass");
                state.publish([Event::Sessions]);
            }
            Ok(Ok(None)) => {}
            Ok(Err(e)) => tracing::warn!(session = %session_id, error = %e, "git pass failed"),
            Err(e) => tracing::warn!(error = %e, "git pass task failed"),
        }
    });
}
