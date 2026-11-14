//! The PTY host as commands (package 8; `ui/src/state/contract-terminal.ts`,
//! `session_launch` / `session_resume` of `contract-sessions.ts`): one thin
//! `#[tauri::command]` per function of `mode_core::pty::api`, run on the
//! blocking pool. An `ApiError {code, message, action?}` rejects as
//! [`CmdError`] with `action` kept (05 §14: `jump` | `resume`). Granted to
//! the app window only. Plus `app_quit`, the quit confirmation of 03 §2.1
//! ([`crate::quit`]).

use std::sync::Arc;

use mode_core::pty::Pty;
use mode_core::pty::api::{
    self, ApiResult, ClaudeLocation, LaunchArgs, LaunchResult, LiveSession, PromptArgs,
    PromptDelivered, PtyOpenArgs, PtyOpenResult, PtyResizeArgs, PtyWriteArgs, QuitAllArgs,
    QuitAllResult, ResumeArgs, SessionArgs,
};
use mode_core::push::{FlashKind, OverlayFlash};
use tauri::{AppHandle, State};

use crate::cmd_error::{CmdError, CmdResult};
use crate::quit::{self, QuitAnswer, QuitStep};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// Run one api function on the blocking pool.
pub async fn with_pty<T, F>(core: &CoreState, f: F) -> CmdResult<T>
where
    F: FnOnce(&Pty) -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    let pty = core.pty()?;
    tauri::async_runtime::spawn_blocking(move || f(&pty))
        .await
        .map_err(CmdError::internal)?
        .map_err(CmdError::from)
}

// ---------------------------------------------------------------- terminal

/// Attach the Terminal tab: resize, then the replay (`pty.output` follows).
#[tauri::command(rename_all = "snake_case")]
pub async fn pty_open(
    state: Core<'_>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> CmdResult<PtyOpenResult> {
    let args = PtyOpenArgs {
        session_id,
        cols,
        rows,
    };
    with_pty(&state, move |p| api::pty_open(p, args)).await
}

/// The Terminal tab went away: stop streaming its output (the PTY stays).
#[tauri::command(rename_all = "snake_case")]
pub async fn pty_detach(state: Core<'_>, session_id: String) -> CmdResult<()> {
    with_pty(&state, move |p| {
        api::pty_detach(p, SessionArgs { session_id })
    })
    .await
}

/// Keystrokes and pastes (base64), in order.
#[tauri::command(rename_all = "snake_case")]
pub async fn pty_write(state: Core<'_>, session_id: String, data: String) -> CmdResult<()> {
    with_pty(&state, move |p| {
        api::pty_write(p, PtyWriteArgs { session_id, data })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn pty_resize(
    state: Core<'_>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> CmdResult<()> {
    let args = PtyResizeArgs {
        session_id,
        cols,
        rows,
    };
    with_pty(&state, move |p| api::pty_resize(p, args)).await
}

/// "Close terminal": SIGINT, wait for the turn, let `claude` exit; never
/// SIGTERM (05 §8.3). `pty.exited` follows.
#[tauri::command(rename_all = "snake_case")]
pub async fn pty_close(state: Core<'_>, session_id: String) -> CmdResult<()> {
    with_pty(&state, move |p| {
        api::pty_close(p, SessionArgs { session_id })
    })
    .await
}

// ---------------------------------------------------------------- sessions

/// Launch dialog (03 §3.5, 05 §8.1).
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn session_launch(
    state: Core<'_>,
    cwd: String,
    permission_mode: Option<String>,
    model: Option<String>,
    profile_id: Option<String>,
    open: Option<String>,
    overview_only: Option<bool>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> CmdResult<LaunchResult> {
    let args = LaunchArgs {
        cwd,
        permission_mode,
        model,
        profile_id,
        open,
        overview_only,
        cols,
        rows,
    };
    let r = with_pty(&state, move |p| api::session_launch(p, args)).await?;
    flash_launching(&state, &r);
    Ok(r)
}

/// "launching · <name>" in the notch (02 §5.4), held (`keep_pill`) until
/// the session's own pill (working, waiting) takes over. Nothing when the terminal did
/// not start (the Terminal tab says why) or was already open.
pub fn flash_launching(core: &CoreState, r: &LaunchResult) {
    if r.error.is_some() || r.already_open {
        return;
    }
    let sid = r.session_id.clone();
    let name = core
        .db
        .read(|c| mode_core::repo::get_by_id::<mode_core::repo::Session>(c, &sid))
        .ok()
        .flatten()
        .map(|s| s.display_name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "session".into());
    core.send_flash(
        &OverlayFlash::new(format!("launching · {name}"), FlashKind::Info)
            .opening("sessions", Some(&r.session_id))
            .keeping_pill(),
    );
}

/// Resume dialog (03 §3.5, 05 §8.2).
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
pub async fn session_resume(
    state: Core<'_>,
    session_id: String,
    cwd: Option<String>,
    permission_mode: Option<String>,
    model: Option<String>,
    open: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> CmdResult<LaunchResult> {
    let args = ResumeArgs {
        session_id,
        cwd,
        permission_mode,
        model,
        open,
        cols,
        rows,
    };
    let r = with_pty(&state, move |p| api::session_resume(p, args)).await?;
    flash_launching(&state, &r);
    Ok(r)
}

/// The prompt box (05 §14): PTY stdin or a headless turn; a refusal
/// rejects with `{code: "refused", message, action?}`.
#[tauri::command(rename_all = "snake_case")]
pub async fn session_prompt(
    state: Core<'_>,
    session_id: String,
    text: String,
) -> CmdResult<PromptDelivered> {
    with_pty(&state, move |p| {
        api::session_prompt(p, PromptArgs { session_id, text })
    })
    .await
}

/// "Stop turn" (headless SIGINT, or SIGINT to a working terminal).
#[tauri::command(rename_all = "snake_case")]
pub async fn turn_stop(state: Core<'_>, session_id: String) -> CmdResult<()> {
    with_pty(&state, move |p| {
        api::turn_stop(p, SessionArgs { session_id })
    })
    .await
}

/// SIGINT every live PTY and headless turn and wait (`wait_ms`, 10 s).
#[tauri::command(rename_all = "snake_case")]
pub async fn pty_quit_all(state: Core<'_>, wait_ms: Option<u64>) -> CmdResult<QuitAllResult> {
    with_pty(&state, move |p| {
        api::pty_quit_all(p, QuitAllArgs { wait_ms })
    })
    .await
}

/// The sessions a quit would interrupt.
#[tauri::command(rename_all = "snake_case")]
pub async fn pty_list(state: Core<'_>) -> CmdResult<Vec<LiveSession>> {
    with_pty(&state, api::pty_list).await
}

/// Where `claude` resolves to now (setting, login `PATH`, `where`).
#[tauri::command(rename_all = "snake_case")]
pub async fn claude_locate(state: Core<'_>) -> CmdResult<ClaudeLocation> {
    with_pty(&state, api::claude_locate).await
}

// -------------------------------------------------------------------- quit

/// Quit (03 §2.1; the profile row's Quit). Without `confirm`: exit at once
/// when no embedded session is live, else send the app window
/// `app:quit-confirm {sessions}` and answer `{status: "confirm"}`;
/// `confirm: true` stops the sessions (SIGINT, 10 s) and exits;
/// `confirm: false` stays.
#[tauri::command(rename_all = "snake_case")]
pub async fn app_quit(
    app: AppHandle,
    state: Core<'_>,
    confirm: Option<bool>,
) -> CmdResult<QuitAnswer> {
    let core = Arc::clone(&state);
    let live = crate::cmd_error::blocking(move || Ok(quit::live_sessions(&core))).await?;
    let step = state.quit.on_app_quit(confirm, || live);
    tracing::info!(step = ?step, "app_quit");
    Ok(match step {
        QuitStep::Exit => {
            app.exit(0);
            QuitAnswer {
                status: "quitting",
                sessions: Vec::new(),
            }
        }
        QuitStep::StopThenExit => {
            quit::stop_then_exit(app, Arc::clone(&state));
            QuitAnswer {
                status: "quitting",
                sessions: Vec::new(),
            }
        }
        QuitStep::Confirm(sessions) => {
            quit::ask(&app, sessions.clone());
            QuitAnswer {
                status: "confirm",
                sessions,
            }
        }
        QuitStep::Cancelled => QuitAnswer {
            status: "cancelled",
            sessions: Vec::new(),
        },
    })
}
