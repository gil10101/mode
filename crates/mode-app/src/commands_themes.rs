//! The theme commands (10 §4.5; 03 §8.5; `ui/src/state/contract-themes.ts`):
//! one thin `#[tauri::command]` per function of `mode_core::themes::api` on
//! the blocking pool. A change to the active pack (activation, import or
//! replace of the active pack, delete of the active pack) requests
//! `theme:changed` for both webviews through the service's publisher.
//! `theme_resolve` is also the overlay's (it applies the same token map).

use std::sync::Arc;

use mode_core::themes::Themes;
use mode_core::themes::api::{
    self, ApiResult, DeleteResult, DuplicateArgs, ExportArgs, ExportResult, ImportArgs, OnConflict,
    ResolveArgs, SetActiveArgs, ThemeDetail, ThemeIdArgs, ThemeImportResult, ThemeRow,
    ThemeUpdateArgs,
};
use mode_core::themes::resolve::ResolvedTheme;
use serde_json::Value;
use tauri::State;

use crate::cmd_error::{CmdError, CmdResult};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// Run one api function on the blocking pool.
pub async fn with_themes<T, F>(core: &CoreState, f: F) -> CmdResult<T>
where
    F: FnOnce(&Themes) -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    let t = core.themes()?;
    tauri::async_runtime::spawn_blocking(move || f(&t))
        .await
        .map_err(CmdError::internal)?
        .map_err(CmdError::from)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn themes_list(state: Core<'_>) -> CmdResult<Vec<ThemeRow>> {
    with_themes(&state, api::themes_list).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn theme_get(state: Core<'_>, id: String) -> CmdResult<ThemeDetail> {
    with_themes(&state, move |t| api::theme_get(t, ThemeIdArgs { id })).await
}

/// **Use** on a gallery card; `mode` sets `theme.appearance` too.
#[tauri::command(rename_all = "snake_case")]
pub async fn theme_set_active(
    state: Core<'_>,
    id: String,
    mode: Option<String>,
) -> CmdResult<ResolvedTheme> {
    with_themes(&state, move |t| {
        api::theme_set_active(t, SetActiveArgs { id, mode })
    })
    .await
}

/// `apply: false` (or absent) validates and previews; `apply: true` writes
/// the row (`on_conflict` when a user pack has the slug); `dry_run` is the
/// older spelling of `apply: false`.
#[tauri::command(rename_all = "snake_case")]
pub async fn theme_import(
    state: Core<'_>,
    path: Option<String>,
    text: Option<String>,
    apply: Option<bool>,
    dry_run: Option<bool>,
    on_conflict: Option<OnConflict>,
) -> CmdResult<ThemeImportResult> {
    let args = ImportArgs {
        path,
        text,
        apply,
        dry_run,
        on_conflict,
    };
    with_themes(&state, move |t| api::theme_import(t, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn theme_export(
    state: Core<'_>,
    id: String,
    path: Option<String>,
) -> CmdResult<ExportResult> {
    with_themes(&state, move |t| {
        api::theme_export(t, ExportArgs { id, path })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn theme_delete(state: Core<'_>, id: String) -> CmdResult<DeleteResult> {
    with_themes(&state, move |t| api::theme_delete(t, ThemeIdArgs { id })).await
}

/// The token map for a pack (`id`, default the active one) in `mode`
/// (default `theme.appearance`); both webviews apply it at start.
#[tauri::command(rename_all = "snake_case")]
pub async fn theme_resolve(
    state: Core<'_>,
    id: Option<String>,
    mode: Option<String>,
) -> CmdResult<ResolvedTheme> {
    with_themes(&state, move |t| {
        api::theme_resolve(t, ResolveArgs { id, mode })
    })
    .await
}

/// A `custom` copy of any pack (10 §4.5 **Duplicate**).
#[tauri::command(rename_all = "snake_case")]
pub async fn theme_duplicate(
    state: Core<'_>,
    id: String,
    name: Option<String>,
) -> CmdResult<ThemeRow> {
    with_themes(&state, move |t| {
        api::theme_duplicate(t, DuplicateArgs { id, name })
    })
    .await
}

/// The token editor's **Save** on an imported or custom pack.
#[tauri::command(rename_all = "snake_case")]
pub async fn theme_update(
    state: Core<'_>,
    id: String,
    document: Value,
) -> CmdResult<ThemeImportResult> {
    with_themes(&state, move |t| {
        api::theme_update(t, ThemeUpdateArgs { id, document })
    })
    .await
}
