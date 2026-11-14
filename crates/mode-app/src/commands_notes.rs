//! The notes commands (04; `ui/src/state/contract-notes.ts`, `note_capture`
//! of `contract.ts`): one thin `#[tauri::command]` per function of
//! `mode_core::notes::api`. Arguments are the snake_case keys the webview
//! sends, gathered into the api's args struct; the result is passed through;
//! an `ApiError` rejects as `{code, message}` ([`CmdError`]). Every call runs
//! on the blocking pool (the notes core waits on the database writer and
//! the file system); writes request `notes.changed` through the service.

use std::sync::Arc;

use mode_core::notes::Notes;
use mode_core::notes::api::{
    self, AddTaskArgs, ApiResult, Buffer, CaptureLinksArgs, CaptureResult, FromItemArgs,
    GroupingIdArgs, GroupingSaveArgs, GroupingsList, NoteArchiveArgs, NoteCaptureArgs,
    NoteCreateArgs, NoteDropArgs, NoteDropResult, NoteEditorArgs, NoteIdArgs, NoteLinkAddArgs,
    NoteRenameArgs, NoteRevArgs, NoteSaveArgs, NoteSetFieldArgs, NoteWriteResult, NotesImportArgs,
    NotesListArgs, NotesListResult, NotesScope, NotesStateArgs, NotesStateResult, QueryContext,
    QueryResolveArgs, QueryResolveResult, QueryRunArgs, QueryRunResult, ReindexResult,
    RevisionContent, StatusMoveArgs, StatusesSaveArgs, TagDeleteArgs, TagRenameArgs,
};
use mode_core::notes::capture::CreatedFrom;
use mode_core::notes::groupings::{AddTaskPlan, GroupingDraft, GroupingRow};
use mode_core::notes::import::ImportReport;
use mode_core::notes::query::Chip;
use mode_core::notes::views::{FoldersList, NoteDetail, NoteLink, RevisionRow, TagRow};
use mode_core::notes::write::{BatchReport, FieldEdit, Resolve};
use mode_core::push::{FlashKind, FlashOpen};
use mode_core::queries::filter::SortKey;
use mode_core::settings::StatusDef;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::cmd_error::{CmdError, CmdResult};
use crate::health;
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// Run one api function on the blocking pool.
pub async fn with_notes<T, F>(core: &CoreState, f: F) -> CmdResult<T>
where
    F: FnOnce(&Notes) -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    let notes = core.notes()?;
    tauri::async_runtime::spawn_blocking(move || f(&notes))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))?
        .map_err(CmdError::from)
}

// ------------------------------------------------------------- list / get

#[tauri::command(rename_all = "snake_case")]
pub async fn notes_list(
    state: Core<'_>,
    scope: NotesScope,
    sort: Option<Vec<SortKey>>,
    show_archived: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> CmdResult<NotesListResult> {
    let args = NotesListArgs {
        scope,
        sort,
        show_archived,
        limit,
        offset,
    };
    with_notes(&state, move |n| api::notes_list(n, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_get(state: Core<'_>, note_id: String) -> CmdResult<NoteDetail> {
    with_notes(&state, move |n| api::note_get(n, NoteIdArgs { note_id })).await
}

/// The editor opened or closed a note (no touch-up while open, 04 §11.2).
#[tauri::command(rename_all = "snake_case")]
pub async fn note_editor(state: Core<'_>, note_id: String, open: bool) -> CmdResult<()> {
    with_notes(&state, move |n| {
        api::note_editor(n, NoteEditorArgs { note_id, open })
    })
    .await
}

// ----------------------------------------------------------------- writes

#[tauri::command(rename_all = "snake_case")]
pub async fn note_create(
    state: Core<'_>,
    folder_id: Option<String>,
    text: String,
) -> CmdResult<NoteDetail> {
    with_notes(&state, move |n| {
        api::note_create(n, NoteCreateArgs { folder_id, text })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_save(
    state: Core<'_>,
    note_id: String,
    text: String,
    base_hash: String,
    base_rev: Option<i64>,
    resolve: Option<Resolve>,
    checkpoint: Option<bool>,
) -> CmdResult<NoteWriteResult> {
    let args = NoteSaveArgs {
        note_id,
        text,
        base_hash,
        base_rev,
        resolve,
        checkpoint,
    };
    with_notes(&state, move |n| api::note_save(n, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_set_field(
    state: Core<'_>,
    note_id: String,
    edit: FieldEdit,
    buffer: Option<Buffer>,
) -> CmdResult<NoteWriteResult> {
    let args = NoteSetFieldArgs {
        note_id,
        edit,
        buffer,
    };
    with_notes(&state, move |n| api::note_set_field(n, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_cycle_status(state: Core<'_>, note_id: String) -> CmdResult<NoteWriteResult> {
    with_notes(&state, move |n| {
        api::note_cycle_status(n, NoteIdArgs { note_id })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_archive(
    state: Core<'_>,
    note_id: String,
    archived: bool,
) -> CmdResult<NoteWriteResult> {
    with_notes(&state, move |n| {
        api::note_archive(n, NoteArchiveArgs { note_id, archived })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_rename(
    state: Core<'_>,
    note_id: String,
    title: String,
) -> CmdResult<NoteWriteResult> {
    with_notes(&state, move |n| {
        api::note_rename(n, NoteRenameArgs { note_id, title })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_trash(state: Core<'_>, note_id: String) -> CmdResult<()> {
    with_notes(&state, move |n| api::note_trash(n, NoteIdArgs { note_id })).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_untrash(state: Core<'_>, note_id: String) -> CmdResult<NoteWriteResult> {
    with_notes(&state, move |n| {
        api::note_untrash(n, NoteIdArgs { note_id })
    })
    .await
}

// ------------------------------------------------- folders and groupings

#[tauri::command(rename_all = "snake_case")]
pub async fn folders_list(state: Core<'_>) -> CmdResult<FoldersList> {
    with_notes(&state, api::folders_list).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn groupings_list(state: Core<'_>) -> CmdResult<GroupingsList> {
    with_notes(&state, api::groupings_list).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn grouping_save(state: Core<'_>, grouping: GroupingDraft) -> CmdResult<GroupingRow> {
    with_notes(&state, move |n| {
        api::grouping_save(n, GroupingSaveArgs { grouping })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn grouping_delete(state: Core<'_>, id: String) -> CmdResult<()> {
    with_notes(&state, move |n| {
        api::grouping_delete(n, GroupingIdArgs { id })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_drop(
    state: Core<'_>,
    note_id: String,
    grouping: String,
    apply: Option<bool>,
) -> CmdResult<NoteDropResult> {
    let args = NoteDropArgs {
        note_id,
        grouping,
        apply: apply.unwrap_or(false),
    };
    with_notes(&state, move |n| api::note_drop(n, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn add_task_plan(state: Core<'_>, filter: Value) -> CmdResult<AddTaskPlan> {
    with_notes(&state, move |n| {
        api::add_task_plan(n, AddTaskArgs { filter })
    })
    .await
}

// -------------------------------------------------------------------- tags

#[tauri::command(rename_all = "snake_case")]
pub async fn tags_list(state: Core<'_>) -> CmdResult<Vec<TagRow>> {
    with_notes(&state, api::tags_list).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn tag_rename(state: Core<'_>, from: String, to: String) -> CmdResult<BatchReport> {
    with_notes(&state, move |n| {
        api::tag_rename(n, TagRenameArgs { from, to })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn tag_delete(state: Core<'_>, name: String) -> CmdResult<BatchReport> {
    with_notes(&state, move |n| api::tag_delete(n, TagDeleteArgs { name })).await
}

// --------------------------------------------------------------- query bar

#[tauri::command(rename_all = "snake_case")]
pub async fn query_resolve(
    state: Core<'_>,
    text: String,
    context: QueryContext,
) -> CmdResult<QueryResolveResult> {
    with_notes(&state, move |n| {
        api::query_resolve(n, QueryResolveArgs { text, context })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn query_run(
    state: Core<'_>,
    chips: Vec<Chip>,
    sort: Option<Vec<SortKey>>,
    limit: Option<i64>,
) -> CmdResult<QueryRunResult> {
    with_notes(&state, move |n| {
        api::query_run(n, QueryRunArgs { chips, sort, limit })
    })
    .await
}

// ----------------------------------------------------------------- history

#[tauri::command(rename_all = "snake_case")]
pub async fn note_history(state: Core<'_>, note_id: String) -> CmdResult<Vec<RevisionRow>> {
    with_notes(&state, move |n| {
        api::note_history(n, NoteIdArgs { note_id })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_revision_get(
    state: Core<'_>,
    note_id: String,
    rev: i64,
) -> CmdResult<RevisionContent> {
    with_notes(&state, move |n| {
        api::note_revision_get(n, NoteRevArgs { note_id, rev })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_restore(
    state: Core<'_>,
    note_id: String,
    rev: i64,
) -> CmdResult<NoteWriteResult> {
    with_notes(&state, move |n| {
        api::note_restore(n, NoteRevArgs { note_id, rev })
    })
    .await
}

// ------------------------------------------------------------------- links

#[tauri::command(rename_all = "snake_case")]
pub async fn note_link_add(
    state: Core<'_>,
    note_id: String,
    kind: String,
    target_id: String,
    link_type: String,
    label: Option<String>,
) -> CmdResult<NoteLink> {
    let args = NoteLinkAddArgs {
        note_id,
        kind,
        target_id,
        link_type,
        label,
    };
    with_notes(&state, move |n| api::note_link_add(n, args)).await
}

// ----------------------------------------------------------------- capture

/// Quick capture (04 §7.1 paths 1, 2, 8): the capture box, the hotkey and
/// ⊕ Add task. Replaces the package 5 stub.
#[tauri::command(rename_all = "snake_case")]
pub async fn note_capture(
    window: tauri::Window,
    state: Core<'_>,
    text: String,
    created_from: Option<CreatedFrom>,
) -> CmdResult<CaptureResult> {
    let r = with_notes(&state, move |n| {
        api::note_capture(n, NoteCaptureArgs { text, created_from })
    })
    .await?;
    if window.label() == crate::APP_WINDOW {
        flash_captured(&state, &r.note_id);
    }
    Ok(r)
}

/// "captured · note" in the notch (02 §5.4) for a capture made away from
/// the overlay (the app window's ⊕ Add task, dictation with no field); the
/// overlay's own capture box confirms in place.
pub fn flash_captured(core: &CoreState, note_id: &str) {
    core.flash(
        "captured · note",
        FlashKind::Ok,
        Some(FlashOpen {
            view: "notes".into(),
            id: Some(note_id.to_string()),
        }),
    );
}

/// URLs or files dropped on the overlay (04 §7.1 path 3).
#[tauri::command(rename_all = "snake_case")]
pub async fn note_capture_links(state: Core<'_>, items: Vec<String>) -> CmdResult<CaptureResult> {
    with_notes(&state, move |n| {
        api::note_capture_links(n, CaptureLinksArgs { items })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_from_session(state: Core<'_>, id: String) -> CmdResult<CaptureResult> {
    with_notes(&state, move |n| {
        api::note_from_session(n, FromItemArgs { id })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_from_message(state: Core<'_>, id: String) -> CmdResult<CaptureResult> {
    with_notes(&state, move |n| {
        api::note_from_message(n, FromItemArgs { id })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn note_prepare(state: Core<'_>, id: String) -> CmdResult<CaptureResult> {
    with_notes(&state, move |n| api::note_prepare(n, FromItemArgs { id })).await
}

// -------------------------------------------------- diagnostics, settings

/// Diagnostics → Reindex: a full scan now; the vault may have come back
/// (or gone), so `health.changed` follows.
#[tauri::command(rename_all = "snake_case")]
pub async fn notes_reindex(app: AppHandle, state: Core<'_>) -> CmdResult<ReindexResult> {
    let r = with_notes(&state, api::notes_reindex).await;
    if r.is_ok() {
        *CoreState::lock(&state.last_full_scan_at) = Some(mode_core::db::now_rfc3339());
    }
    health::emit(app, Arc::clone(&state)).await;
    r
}

#[tauri::command(rename_all = "snake_case")]
pub async fn notes_import(
    state: Core<'_>,
    source: String,
    destination: Option<String>,
    include_attachments: Option<bool>,
    apply: Option<bool>,
) -> CmdResult<ImportReport> {
    let args = NotesImportArgs {
        source,
        destination,
        include_attachments: include_attachments.unwrap_or(false),
        apply: apply.unwrap_or(false),
    };
    with_notes(&state, move |n| api::notes_import(n, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn notes_state_export(state: Core<'_>) -> CmdResult<Value> {
    with_notes(&state, api::notes_state_export).await
}

/// `notes-state.json` from a settings bundle (04 §13.2). The webview's key
/// is `state`, so the managed state goes by `core` here.
#[tauri::command(rename_all = "snake_case")]
pub async fn notes_state_import(core: Core<'_>, state: Value) -> CmdResult<NotesStateResult> {
    with_notes(&core, move |n| {
        api::notes_state_import(n, NotesStateArgs { state })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn statuses_save(
    app: AppHandle,
    state: Core<'_>,
    statuses: Vec<StatusDef>,
) -> CmdResult<Vec<StatusDef>> {
    let saved = with_notes(&state, move |n| {
        api::statuses_save(n, StatusesSaveArgs { statuses })
    })
    .await?;
    // `notes.statuses` is a setting: the app window's settings store follows.
    crate::commands::emit_app_settings(&app, &state, Some(vec!["notes.statuses".into()])).await;
    Ok(saved)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn status_move(state: Core<'_>, from: String, to: String) -> CmdResult<BatchReport> {
    with_notes(&state, move |n| {
        api::status_move(n, StatusMoveArgs { from, to })
    })
    .await
}
