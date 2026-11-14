//! One plain function per Terminal / prompt command
//! (`ui/src/state/contract-terminal.ts`, `session_launch` / `session_resume`
//! of `contract-sessions.ts`): an args struct, a result struct, snake_case,
//! so `mode-app` wraps each in a `#[tauri::command]` mechanically:
//!
//! ```ignore
//! #[tauri::command(rename_all = "snake_case")]
//! pub async fn pty_open(state: State<'_, Arc<CoreState>>, session_id: String, cols: u16, rows: u16)
//!     -> Result<PtyOpenResult, ApiError> {
//!     let pty = state.pty.clone();
//!     tauri::async_runtime::spawn_blocking(move || api::pty_open(&pty, PtyOpenArgs { session_id, cols, rows }))
//!         .await.map_err(ApiError::internal)?
//! }
//! ```
//!
//! Every function may block (a PTY write, git for a new row, the 50 ms
//! paste delay): run them on the blocking pool.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::driver::{self, Action, HandOff, PromptFacts, PromptRoute};
use super::{Pty, b64, b64_decode};
use crate::db::DbError;
use crate::sessions::{SessionState, liveness};

// ------------------------------------------------------------------ errors

/// A command error the webview shows: `code` is `refused` (the prompt box
/// rules of 05 §14; `action` names the button to offer), `not_found`,
/// `invalid`, `claude_missing`, `cwd_missing`, `transcript_missing`, `io`,
/// `read_only` or `internal`; `message` is the sentence to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    /// `jump` | `resume`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        ApiError {
            code: code.into(),
            message: message.into(),
            action: None,
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    pub fn claude_missing(message: impl Into<String>) -> Self {
        Self::new("claude_missing", message)
    }

    pub fn io(e: impl std::fmt::Display) -> Self {
        Self::new("io", e.to_string())
    }

    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::new("internal", e.to_string())
    }

    pub fn refused(message: &str, action: Option<Action>) -> Self {
        ApiError {
            code: "refused".into(),
            message: message.into(),
            action: action.map(|a| a.as_str().to_string()),
        }
    }

    pub fn refused_plain(message: &str) -> Self {
        Self::refused(message, None)
    }
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        let code = match &e {
            DbError::NotFound { .. } => "not_found",
            DbError::Invalid { .. } => "invalid",
            DbError::ReadOnly(_) => "read_only",
            DbError::Io(_) => "io",
            _ => "internal",
        };
        let message = match &e {
            DbError::Invalid { detail, .. } => detail.clone(),
            other => other.to_string(),
        };
        Self::new(code, message)
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

// -------------------------------------------------------------------- args

/// Commands that name only a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionArgs {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PtyOpenArgs {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

/// `pty_open`: the size the PTY now has and the replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PtyOpenResult {
    pub cols: u16,
    pub rows: u16,
    /// base64 of the buffered bytes (may be empty).
    pub data: String,
    /// Stream offset of the first replayed byte.
    pub offset: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PtyWriteArgs {
    pub session_id: String,
    /// base64 of the keystroke / paste bytes.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PtyResizeArgs {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

/// `session_launch` (`LaunchArgs` of `contract-sessions.ts` plus the
/// Terminal tab's size and "Overview only").
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LaunchArgs {
    /// Empty: the agent's first repository (03 §7E.2), else refused.
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// The agent profile to launch as (03 §7E): its flags are added, its
    /// model and permission mode used where this launch names none.
    #[serde(default)]
    pub profile_id: Option<String>,
    /// `overview` | `terminal`: which tab the UI opens; the core ignores it.
    #[serde(default)]
    pub open: Option<String>,
    /// No terminal: the first prompt runs a headless turn (05 §8.5).
    #[serde(default)]
    pub overview_only: Option<bool>,
    /// Initial PTY size (default 100 × 30; `pty_open` resizes).
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

/// `session_resume` (`ResumeArgs` plus the size).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResumeArgs {
    pub session_id: String,
    /// A replacement folder when the original `cwd` is gone.
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    /// A model for the resumed session (03 §3.5): the backup the agent
    /// offers when it asks before switching (03 §7E.3).
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub open: Option<String>,
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

/// `session_launch` / `session_resume` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LaunchResult {
    /// The roster row the launch created or reused.
    pub session_id: String,
    /// `false`: "approvals will appear in the Terminal tab only" (03 §3.5).
    pub hooks_installed: bool,
    /// The terminal did not start (05 §16): the row is `failed` with this
    /// line; the Terminal tab shows it inline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Resume found the terminal already open (nothing was spawned).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub already_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PromptArgs {
    pub session_id: String,
    pub text: String,
}

/// How `session_prompt` delivered the text (05 §14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PromptDelivered {
    /// `pty` | `headless`.
    pub delivered_via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct QuitAllArgs {
    /// How long to wait for the turns to end (default 10 s, 08 §2.6).
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct QuitAllResult {
    /// Sessions that were sent SIGINT.
    pub signalled: Vec<String>,
    /// Sessions whose process had not exited when the wait ended.
    pub remaining: Vec<String>,
}

/// A session with a live process (the quit confirmation's list, 03 §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LiveSession {
    pub session_id: String,
    pub display_name: String,
    /// `pty` | `headless`.
    pub driver: String,
    pub state: String,
}

/// Where `claude` was found (the Claude Code card, 03 §2.7 empty states).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ClaudeLocation {
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `setting` | `path` | `where` | `fallback` | `config`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

// --------------------------------------------------------------- commands

/// Attach the Terminal tab: resize to its size, then return the replay
/// buffer; live `pty.output` follows from here on. An exited PTY still
/// replays (until the row is dismissed).
pub fn pty_open(pty: &Pty, args: PtyOpenArgs) -> ApiResult<PtyOpenResult> {
    let proc = pty
        .proc(&args.session_id)
        .ok_or_else(|| ApiError::not_found("this session has no terminal"))?;
    if proc.is_alive() {
        let (c, r) = proc.size();
        if (c, r) != (args.cols, args.rows) && args.cols >= 2 && args.rows >= 1 {
            proc.resize(args.cols, args.rows).map_err(ApiError::io)?;
        }
    }
    proc.attached
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (offset, bytes) = proc.snapshot();
    let (cols, rows) = proc.size();
    let exit = proc.exit_info();
    Ok(PtyOpenResult {
        cols,
        rows,
        data: b64(&bytes),
        offset,
        exited: exit.is_some(),
        exit_code: exit.and_then(|e| e.code),
    })
}

/// The Terminal tab went away: stop emitting its output (the PTY and its
/// buffer stay, 08 §5.4 "Detach").
pub fn pty_detach(pty: &Pty, args: SessionArgs) -> ApiResult<()> {
    if let Some(p) = pty.proc(&args.session_id) {
        p.attached.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    Ok(())
}

/// Keystrokes and pastes (base64), in order.
pub fn pty_write(pty: &Pty, args: PtyWriteArgs) -> ApiResult<()> {
    let bytes = b64_decode(&args.data).map_err(ApiError::invalid)?;
    let proc = pty
        .inner
        .host
        .live(&args.session_id)
        .ok_or_else(|| ApiError::not_found("no terminal is open for this session"))?;
    proc.write(&bytes).map_err(ApiError::io)
}

pub fn pty_resize(pty: &Pty, args: PtyResizeArgs) -> ApiResult<()> {
    if !(2..=1000).contains(&args.cols) || !(1..=1000).contains(&args.rows) {
        return Err(ApiError::invalid("terminal size out of range"));
    }
    let proc = pty
        .inner
        .host
        .live(&args.session_id)
        .ok_or_else(|| ApiError::not_found("no terminal is open for this session"))?;
    proc.resize(args.cols, args.rows).map_err(ApiError::io)
}

/// "Close terminal": SIGINT, the turn ends, `claude` exits; `pty.exited`
/// follows. Never SIGTERM.
pub fn pty_close(pty: &Pty, args: SessionArgs) -> ApiResult<()> {
    pty.close(&args.session_id)
}

pub fn session_launch(pty: &Pty, args: LaunchArgs) -> ApiResult<LaunchResult> {
    pty.launch(&args)
}

pub fn session_resume(pty: &Pty, args: ResumeArgs) -> ApiResult<LaunchResult> {
    pty.resume(&args)
}

/// The facts the §14 table reads for a session.
pub fn prompt_facts(pty: &Pty, session_id: &str) -> ApiResult<PromptFacts> {
    let inner = &pty.inner;
    let row = inner
        .engine
        .session_row(session_id)?
        .ok_or_else(|| ApiError::not_found("no such session"))?;
    let mut drv = inner.drivers.get(&row.id);
    let live = inner.host.live(&row.id);
    if drv == driver::Driver::Pty && live.is_none() {
        drv = driver::Driver::None;
    }
    let foreign = drv == driver::Driver::None
        && row.ended_at.is_none()
        && inner
            .engine
            .process(&row.id)
            .is_some_and(liveness::pid_alive);
    Ok(PromptFacts {
        tracking: row.tracking.clone(),
        state: row.state.parse().unwrap_or(SessionState::Idle),
        waiting_kind: row.waiting_kind.as_deref().and_then(|k| k.parse().ok()),
        ended: row.ended_at.is_some(),
        is_subagent: row.parent_session_id.is_some(),
        driver: drv,
        pty_closing: live
            .as_ref()
            .is_some_and(|p| p.closing.load(std::sync::atomic::Ordering::SeqCst)),
        foreign_process: foreign,
    })
}

/// The prompt box (05 §14): PTY stdin through bracketed paste with `\r`
/// after 50 ms, or a headless turn; refused (`code: refused`) with the
/// box's sentence otherwise.
pub fn session_prompt(pty: &Pty, args: PromptArgs) -> ApiResult<PromptDelivered> {
    if args.text.trim().is_empty() {
        return Err(ApiError::invalid("nothing to send"));
    }
    let facts = prompt_facts(pty, &args.session_id)?;
    match driver::route_prompt(&facts) {
        PromptRoute::Refused(r) => Err(ApiError::refused(r.message, r.action)),
        PromptRoute::Pty => {
            let proc = pty
                .inner
                .host
                .live(&args.session_id)
                .ok_or_else(|| ApiError::refused_plain(driver::say::CLOSING))?;
            proc.paste(&args.text, pty.inner.config.timings.paste_cr_delay)
                .map_err(ApiError::io)?;
            Ok(PromptDelivered {
                delivered_via: "pty".into(),
                turn_id: None,
            })
        }
        PromptRoute::Headless => {
            let (_, generation) = pty
                .inner
                .drivers
                .apply(&args.session_id, HandOff::PromptWithoutPty)
                .map_err(ApiError::refused_plain)?;
            let row = pty
                .inner
                .engine
                .session_row(&args.session_id)?
                .ok_or_else(|| ApiError::not_found("no such session"))?;
            let turn_id = pty.inner.start_turn(&row, &args.text, generation)?;
            Ok(PromptDelivered {
                delivered_via: "headless".into(),
                turn_id: Some(turn_id),
            })
        }
    }
}

/// "Stop turn": SIGINT to the headless turn (`turn.finished` with `stopped`
/// follows); for a terminal session that is `working`, SIGINT to its
/// `claude` (the turn is interrupted, the terminal stays).
pub fn turn_stop(pty: &Pty, args: SessionArgs) -> ApiResult<()> {
    if pty.inner.turns.get(&args.session_id).is_some() {
        return pty.stop_turn(&args.session_id);
    }
    match pty.inner.host.live(&args.session_id) {
        // A lone SIGINT at the idle prompt does nothing useful (the CLI
        // reads it as the first half of "exit").
        Some(p) if !pty.inner.engine.turn_over(&args.session_id)? => {
            p.sigint().map_err(ApiError::io)
        }
        _ => Err(ApiError::not_found("no turn is running")),
    }
}

/// Quit with live sessions (03 §2.1, 08 §2.6): SIGINT every live PTY and
/// headless turn, wait up to `wait_ms` (10 s) for them to end.
pub fn pty_quit_all(pty: &Pty, args: QuitAllArgs) -> ApiResult<QuitAllResult> {
    let wait = Duration::from_millis(
        args.wait_ms
            .unwrap_or(pty.inner.config.timings.turn_wait.as_millis() as u64),
    );
    let (signalled, remaining) = pty.quit_all(wait);
    Ok(QuitAllResult {
        signalled,
        remaining,
    })
}

/// The sessions a quit would interrupt.
pub fn pty_list(pty: &Pty) -> ApiResult<Vec<LiveSession>> {
    let inner = &pty.inner;
    let mut out = Vec::new();
    let mut add = |id: String, drv: &str| -> ApiResult<()> {
        if let Some(r) = inner.engine.session_row(&id)? {
            out.push(LiveSession {
                session_id: r.id,
                display_name: r.display_name,
                driver: drv.into(),
                state: r.state,
            });
        }
        Ok(())
    };
    for p in inner.host.all_live() {
        add(p.row_id(), "pty")?;
    }
    for id in inner.turns.running() {
        add(id, "headless")?;
    }
    Ok(out)
}

/// Where `claude` resolves to now (setting, login `PATH`, `where`).
pub fn claude_locate(pty: &Pty) -> ApiResult<ClaudeLocation> {
    Ok(match pty.inner.claude() {
        Ok(c) => ClaudeLocation {
            found: true,
            path: Some(c.path.to_string_lossy().into_owned()),
            source: Some(c.source.into()),
            message: None,
        },
        Err(e) => ClaudeLocation {
            found: false,
            path: None,
            source: None,
            message: Some(e.message),
        },
    })
}

impl Pty {
    /// "Stop turn" on a headless run.
    pub fn stop_turn(&self, session_id: &str) -> ApiResult<()> {
        self.inner.stop_turn(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_serialise_for_the_webview() {
        let e = ApiError::refused("a turn is running", None);
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "refused");
        assert_eq!(v["message"], "a turn is running");
        assert!(v.get("action").is_none());
        let v = serde_json::to_value(ApiError::refused("x", Some(Action::Jump))).unwrap();
        assert_eq!(v["action"], "jump");
        let e: ApiError = DbError::NotFound {
            table: "sessions",
            key: "k".into(),
        }
        .into();
        assert_eq!(e.code, "not_found");
    }

    #[test]
    fn args_accept_the_contract_shapes() {
        let a: LaunchArgs =
            serde_json::from_str(r#"{"cwd":"/x","permission_mode":"plan","open":"terminal"}"#)
                .unwrap();
        assert_eq!(a.permission_mode.as_deref(), Some("plan"));
        assert_eq!(a.overview_only, None);
        let r: ResumeArgs =
            serde_json::from_str(r#"{"session_id":"s","open":"overview"}"#).unwrap();
        assert_eq!(r.cwd, None);
        let o = PtyOpenResult {
            cols: 80,
            rows: 24,
            data: String::new(),
            offset: 0,
            exited: false,
            exit_code: None,
        };
        let v = serde_json::to_value(o).unwrap();
        assert!(v.get("exited").is_none());
        let d = PromptDelivered {
            delivered_via: "pty".into(),
            turn_id: None,
        };
        assert_eq!(
            serde_json::to_string(&d).unwrap(),
            r#"{"delivered_via":"pty"}"#
        );
    }
}
