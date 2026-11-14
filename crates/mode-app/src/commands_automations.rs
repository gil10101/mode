//! The Automations commands (03 §7B; `ui/src/state/contract-automations.ts`):
//! one thin `#[tauri::command]` per function of `mode_core::scheduler::api`
//! on the blocking pool. `automation_save` rejects an invalid draft as
//! `{code: "invalid", message, fields: [{field, message}]}`; every other
//! refusal is `{code, message}`. A save, a toggle, a delete or a duplicate
//! wakes the scheduler's loop (`Wake::Changed`) so a new `next_run_at` is
//! honored at once; the scheduler requests `automations:changed` itself.

use std::sync::Arc;

use mode_core::push::{FlashKind, FlashOpen};
use mode_core::scheduler::api::{
    self, DeleteResult, IdArgs, ListArgs, PreviewArgs, RunDetail, RunsArgs, SaveArgs, SaveError,
    SchedulePreview, SetEnabledArgs,
};
use mode_core::scheduler::store::{AutomationDraft, AutomationRow, RunRow};
use mode_core::scheduler::{Scheduler, Wake};
use serde_json::Value;
use tauri::State;

use crate::cmd_error::{CmdError, CmdResult};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// Run one api function on the blocking pool (any error that converts).
pub async fn with_scheduler<T, E, F>(core: &CoreState, f: F) -> CmdResult<T>
where
    F: FnOnce(&Scheduler) -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: Into<CmdError> + Send + 'static,
{
    let s = core.scheduler()?;
    tauri::async_runtime::spawn_blocking(move || f(&s))
        .await
        .map_err(CmdError::internal)?
        .map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn automations_list(
    state: Core<'_>,
    query: Option<String>,
    disabled_only: Option<bool>,
    sort: Option<String>,
) -> CmdResult<Vec<AutomationRow>> {
    let args = ListArgs {
        query,
        disabled_only,
        sort,
    };
    with_scheduler(&state, move |s| api::automations_list(s, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn automation_get(state: Core<'_>, id: String) -> CmdResult<AutomationRow> {
    with_scheduler(&state, move |s| api::automation_get(s, IdArgs { id })).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn automation_save(state: Core<'_>, draft: AutomationDraft) -> CmdResult<AutomationRow> {
    let row = with_scheduler(&state, move |s| -> Result<_, SaveError> {
        api::automation_save(s, SaveArgs { draft })
    })
    .await?;
    state.wake_scheduler(Wake::Changed);
    Ok(row)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn automation_delete(state: Core<'_>, id: String) -> CmdResult<DeleteResult> {
    let r = with_scheduler(&state, move |s| api::automation_delete(s, IdArgs { id })).await?;
    state.wake_scheduler(Wake::Changed);
    Ok(r)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn automation_set_enabled(
    state: Core<'_>,
    id: String,
    enabled: bool,
) -> CmdResult<AutomationRow> {
    let row = with_scheduler(&state, move |s| {
        api::automation_set_enabled(s, SetEnabledArgs { id, enabled })
    })
    .await?;
    state.wake_scheduler(Wake::Changed);
    Ok(row)
}

/// Run now (03 §7B.4): `busy` while a run of it is running.
#[tauri::command(rename_all = "snake_case")]
pub async fn automation_run_now(state: Core<'_>, id: String) -> CmdResult<RunRow> {
    let run = with_scheduler(&state, move |s| api::automation_run_now(s, IdArgs { id })).await?;
    flash_running(&state, &run);
    Ok(run)
}

/// "running · <name>" in the notch (02 §5.4) for Run now; a run that was
/// skipped or failed at once says nothing here (the Runs list says why).
pub fn flash_running(core: &CoreState, run: &RunRow) {
    if run.state != "running" {
        return;
    }
    let aid = run.automation_id.clone();
    let name = core
        .db
        .read(|c| mode_core::repo::get_by_id::<mode_core::repo::Automation>(c, &aid))
        .ok()
        .flatten()
        .map_or_else(|| "automation".to_string(), |a| a.name);
    core.flash(
        format!("running · {name}"),
        FlashKind::Info,
        Some(FlashOpen {
            view: "automations".into(),
            id: Some(run.automation_id.clone()),
        }),
    );
}

#[tauri::command(rename_all = "snake_case")]
pub async fn automation_runs(
    state: Core<'_>,
    id: String,
    limit: Option<i64>,
) -> CmdResult<Vec<RunRow>> {
    with_scheduler(&state, move |s| {
        api::automation_runs(s, RunsArgs { id, limit })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn automation_run_get(state: Core<'_>, id: String) -> CmdResult<RunDetail> {
    with_scheduler(&state, move |s| api::automation_run_get(s, IdArgs { id })).await
}

/// The row's `⋮` → Duplicate: "<name> copy", disabled.
#[tauri::command(rename_all = "snake_case")]
pub async fn automation_duplicate(state: Core<'_>, id: String) -> CmdResult<AutomationRow> {
    let row = with_scheduler(&state, move |s| api::automation_duplicate(s, IdArgs { id })).await?;
    state.wake_scheduler(Wake::Changed);
    Ok(row)
}

/// The empty state's example cards (03 §7B.5) as drafts.
#[tauri::command(rename_all = "snake_case")]
pub async fn automation_examples(state: Core<'_>) -> CmdResult<Vec<AutomationDraft>> {
    with_scheduler(&state, api::automation_examples).await
}

/// The editor's live preview: the next five fire times and the echo, or
/// the field the schedule fails on.
#[tauri::command(rename_all = "snake_case")]
pub async fn schedule_preview(state: Core<'_>, schedule_json: Value) -> CmdResult<SchedulePreview> {
    with_scheduler(&state, move |s| {
        api::schedule_preview(s, PreviewArgs { schedule_json })
    })
    .await
}
