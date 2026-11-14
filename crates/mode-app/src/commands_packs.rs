//! The Marketplace commands (03 §7C.6; `ui/src/state/contract-packs.ts`):
//! one thin `#[tauri::command]` per function of `mode_core::packs::api` on
//! the blocking pool. An install that wrote an agent publishes
//! `agents.changed`; one that wrote an automation wakes the scheduler's
//! loop (the scheduler publishes `automations.changed` itself); a theme
//! goes through `theme_import`, which pushes `theme.changed` when it is the
//! active pack.

use std::sync::Arc;

use mode_core::packs::ItemKind;
use mode_core::packs::api::{
    self, ApiResult, CatalogueArgs, CatalogueResult, ExportArgs, FolderOverride, ImportArgs,
    ItemOutcome, PackConflict, PackExportResult, PackImportResult, Packs,
};
use mode_core::push::Event;
use mode_core::scheduler::Wake;
use tauri::State;

use crate::cmd_error::{CmdError, CmdResult};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// Run one api function on the blocking pool.
pub async fn with_packs<T, F>(core: &CoreState, f: F) -> CmdResult<T>
where
    F: FnOnce(&Packs) -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    let p = core.packs()?;
    tauri::async_runtime::spawn_blocking(move || f(&p))
        .await
        .map_err(CmdError::internal)?
        .map_err(CmdError::from)
}

/// **Export pack** on a row, **Create pack…** on the Marketplace page.
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn pack_export(
    state: Core<'_>,
    agents: Option<Vec<String>>,
    automations: Option<Vec<String>>,
    themes: Option<Vec<String>>,
    name: Option<String>,
    slug: Option<String>,
    author: Option<String>,
    description: Option<String>,
    path: Option<String>,
) -> CmdResult<PackExportResult> {
    let args = ExportArgs {
        agents: agents.unwrap_or_default(),
        automations: automations.unwrap_or_default(),
        themes: themes.unwrap_or_default(),
        name,
        slug,
        author,
        description,
        path,
    };
    with_packs(&state, move |p| api::pack_export(p, args)).await
}

/// `apply: false` (or absent) validates and previews; `apply: true`
/// installs the items not in `skip` (`on_conflict` for names and slugs
/// already here; `folders` for session automations whose folder is not).
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn pack_import(
    state: Core<'_>,
    text: Option<String>,
    path: Option<String>,
    url: Option<String>,
    apply: Option<bool>,
    on_conflict: Option<PackConflict>,
    skip: Option<Vec<String>>,
    folders: Option<Vec<FolderOverride>>,
) -> CmdResult<PackImportResult> {
    let args = ImportArgs {
        text,
        path,
        url,
        apply,
        on_conflict,
        skip: skip.unwrap_or_default(),
        folders: folders.unwrap_or_default(),
    };
    let r = with_packs(&state, move |p| api::pack_import(p, args)).await?;
    let wrote = |kind: ItemKind| {
        r.installed.iter().any(|i| {
            i.kind == kind && matches!(i.outcome, ItemOutcome::Created | ItemOutcome::Replaced)
        })
    };
    if wrote(ItemKind::Agent) {
        state.publish([Event::Agents]);
    }
    if wrote(ItemKind::Automation) {
        state.wake_scheduler(Wake::Changed);
    }
    Ok(r)
}

/// The catalog (`marketplace.index_url`), fetched once per run or on
/// `refresh`.
#[tauri::command(rename_all = "snake_case")]
pub async fn catalogue_get(state: Core<'_>, refresh: Option<bool>) -> CmdResult<CatalogueResult> {
    let args = CatalogueArgs {
        refresh: refresh.unwrap_or(false),
    };
    with_packs(&state, move |p| api::catalogue_get(p, args)).await
}
