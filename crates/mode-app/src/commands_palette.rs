//! The command palette (10 §7.4.15; `ui/src/state/palette-store.ts`):
//! `palette_query {text, limit?, kinds?}` over the in-memory index the app
//! invalidates on sessions / notes / inbox / agenda / automations pushes
//! and Chat changes (rebuilt on a background thread). Synchronous and
//! cheap, so it runs on the blocking pool only to keep the IPC thread free.
//! The overlay may query it too.

use std::sync::Arc;

use mode_core::palette::{Hit, QueryArgs, palette_query as query};
use tauri::State;

use crate::cmd_error::{CmdError, CmdResult};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

#[tauri::command(rename_all = "snake_case")]
pub async fn palette_query(
    state: Core<'_>,
    text: String,
    limit: Option<usize>,
    kinds: Option<Vec<String>>,
) -> CmdResult<Vec<Hit>> {
    let p = state.palette()?;
    let args = QueryArgs { text, limit, kinds };
    tauri::async_runtime::spawn_blocking(move || query(&p, args))
        .await
        .map_err(CmdError::internal)?
        .map_err(CmdError::from)
}
