//! The Sessions view's commands beyond the roster (03 §3.3–§3.10;
//! `SessionsCommands` in `ui/src/state/contract-sessions.ts`): history,
//! undismiss, snooze, the Files tab's git refresh and diff pane, the
//! Transcript tab's pages, the launch dialog's recent folders and folder
//! picker (launch,
//! resume and the prompt box are the PTY host's, `commands_pty.rs`). Plus
//! `health_get` for windows that open after the last `health.changed`.
//! Rejections are `{code, message}`.

use std::sync::Arc;
use std::time::Instant;

use mode_core::files::{self, diff::DiffResult};
use mode_core::queries::{self, SessionFile};
use mode_core::repo::{self, Session};
use mode_core::sessions::history::{HistoryItem, HistoryQuery, RECENT_FOLDERS};
use mode_core::transcript::page::{self, PAGE_LINES, TranscriptPage};
use tauri::{AppHandle, State};

use crate::cmd_error::{CmdError, CmdResult, blocking};
use crate::dialogs::{self, Picked};
use crate::health::{self, HealthChanged};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

fn session_row(core: &CoreState, session_id: &str) -> CmdResult<Session> {
    let sid = session_id.to_string();
    core.db
        .read(|c| repo::get_by_id::<Session>(c, &sid))?
        .ok_or_else(|| CmdError::not_found(format!("no session {session_id}")))
}

/// History rows (03 §3.3): dismissed sessions within
/// `retention.sessions_days`, most recently dismissed first. Every argument
/// is optional (`limit` 200, `before` = the last row's `dismissed_at`,
/// `query` over name / title / folder / branch).
#[tauri::command(rename_all = "snake_case")]
pub async fn sessions_history(
    state: Core<'_>,
    limit: Option<i64>,
    before: Option<String>,
    query: Option<String>,
) -> CmdResult<Vec<HistoryItem>> {
    let engine = Arc::clone(&state.engine);
    let q = HistoryQuery {
        limit,
        before,
        query,
    };
    blocking(move || Ok(engine.history(&q)?)).await
}

/// Undo a dismiss (05 §12): the row returns to the roster.
#[tauri::command(rename_all = "snake_case")]
pub async fn session_undismiss(state: Core<'_>, session_id: String) -> CmdResult<()> {
    let engine = Arc::clone(&state.engine);
    let pushes = blocking(move || Ok(engine.undismiss(&session_id)?)).await?;
    state.publish(pushes);
    Ok(())
}

/// Snooze a `waiting` / `unread` / `failed` session (03 §3.4): `preset` is
/// `later_today` | `tomorrow` | `next_week` (03 §4.7 times, local), or
/// `until` an explicit UTC time; neither cancels the active snooze (the
/// Undo). Returns when the snooze ends (`null` when canceled).
#[tauri::command(rename_all = "snake_case")]
pub async fn session_snooze(
    state: Core<'_>,
    session_id: String,
    preset: Option<String>,
    until: Option<String>,
) -> CmdResult<Option<String>> {
    let engine = Arc::clone(&state.engine);
    let (end, pushes) =
        blocking(move || Ok(engine.snooze(&session_id, preset.as_deref(), until.as_deref())?))
            .await?;
    state.publish(pushes);
    Ok(end)
}

/// "Refresh git status" (03 §3.9, 05 §6.2): run the git pass now and
/// answer the Files tab's rows. A session without a repository answers its
/// hook rows unchanged.
#[tauri::command(rename_all = "snake_case")]
pub async fn session_files_refresh(
    state: Core<'_>,
    session_id: String,
) -> CmdResult<Vec<SessionFile>> {
    let core = Arc::clone(&state);
    let (rows, changed) = blocking(move || {
        session_row(&core, &session_id)?;
        let pass = files::run_git_pass(&core.db, &session_id)?;
        CoreState::lock(&core.git_passes)
            .last
            .insert(session_id.clone(), Instant::now());
        let changed = pass.is_some_and(|(r, _)| r.upgraded + r.inserted > 0);
        let sid = session_id.clone();
        let rows = core.db.read(|c| queries::session_files(c, &sid))?;
        Ok((rows, changed))
    })
    .await?;
    if changed {
        state.publish([mode_core::push::Event::Sessions]);
    }
    Ok(rows)
}

/// The diff pane of one Files-tab row (05 §6.3).
#[tauri::command(rename_all = "snake_case")]
pub async fn session_diff(
    state: Core<'_>,
    session_id: String,
    file_change_id: String,
) -> CmdResult<DiffResult> {
    let db = Arc::clone(&state.db);
    blocking(move || {
        files::diff::diff_for(&db, &session_id, &file_change_id)?
            .ok_or_else(|| CmdError::not_found(format!("no file change {file_change_id}")))
    })
    .await
}

/// One page of the Transcript tab (03 §3.10): `limit` lines (500) before
/// `before_line` (the first page when absent; `before_seq` is accepted as
/// an alias), counted from the end of `transcript_path`.
#[tauri::command(rename_all = "snake_case")]
pub async fn session_transcript(
    state: Core<'_>,
    session_id: String,
    before_line: Option<u64>,
    before_seq: Option<u64>,
    limit: Option<usize>,
) -> CmdResult<TranscriptPage> {
    let core = Arc::clone(&state);
    blocking(move || {
        let row = session_row(&core, &session_id)?;
        Ok(page::page(
            row.transcript_path.as_deref(),
            before_line.or(before_seq),
            limit.unwrap_or(PAGE_LINES),
        ))
    })
    .await
}

/// The launch dialog's recent folders (03 §3.5): distinct `sessions.cwd`,
/// newest first, only folders that still exist.
#[tauri::command(rename_all = "snake_case")]
pub async fn sessions_recent_folders(state: Core<'_>) -> CmdResult<Vec<String>> {
    let engine = Arc::clone(&state.engine);
    blocking(move || {
        Ok(engine
            .recent_folders(RECENT_FOLDERS * 2)?
            .into_iter()
            .filter(|p| std::path::Path::new(p).is_dir())
            .take(RECENT_FOLDERS as usize)
            .collect())
    })
    .await
}

/// The launch dialog's "Choose…" (03 §3.5): the OS folder picker, starting
/// in `start` when it exists (the core shows pickers; the webview never gets
/// `dialog:*`). `null` when the picker was canceled.
#[tauri::command(rename_all = "snake_case")]
pub async fn sessions_pick_folder(
    app: AppHandle,
    start: Option<String>,
) -> CmdResult<Option<String>> {
    blocking(move || {
        let start = start.as_deref().map(dialogs::expand_home);
        match dialogs::pick_folder(&app, "Choose the session folder", start.as_deref()) {
            Picked::Path(p) => Ok(Some(p.to_string_lossy().into_owned())),
            Picked::Cancelled => Ok(None),
            Picked::Unavailable => Err(CmdError::new(
                "unavailable",
                "The folder picker is not available; type the path instead.",
            )),
        }
    })
    .await
}

// ------------------------------------------------------------------ shell

/// The current `health.changed` payload, recomputed, for a window that
/// opened after the last push.
#[tauri::command(rename_all = "snake_case")]
pub async fn health_get(state: Core<'_>) -> CmdResult<HealthChanged> {
    let core = Arc::clone(&state);
    let h = blocking(move || Ok(health::compute(&core))).await?;
    *CoreState::lock(&state.last_health) = Some(h.clone());
    Ok(h)
}
