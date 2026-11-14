//! The Chat commands (03 §7; `ui/src/state/contract-chat.ts`): one thin
//! `#[tauri::command]` per function of `mode_core::chat::api`, plus
//! `chat_pick_file` (the OS file picker: a file is cited, never uploaded).
//! Arguments are the snake_case keys the webview sends; an `ApiError`
//! rejects as `{code, message}`. Every call runs on the blocking pool. The
//! streams (`chat:delta`, `chat:turn`, `chat:action`) go to the app window
//! through the service's sink (see [`crate::services`]).

use std::sync::Arc;

use mode_core::chat::Chat;
use mode_core::chat::api::{
    self, ApiResult, AttachmentArg, ChatStatus, ConfirmArgs, DeleteResult, PinArgs, RenameArgs,
    SendArgs, SendResult, StatusArgs, StopResult, ThreadIdArgs, ThreadSetArgs, TurnIdArgs,
};
use mode_core::chat::edge::{ChatAction, ChatSources, ChatThread, ItemRef};
use mode_core::chat::store::ThreadSummary;
use mode_core::chat::tools::Sources;
use tauri::{AppHandle, State};

use crate::cmd_error::{CmdError, CmdResult};
use crate::dialogs::{self, Picked};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// Run one api function on the blocking pool.
pub async fn with_chat<T, F>(core: &CoreState, f: F) -> CmdResult<T>
where
    F: FnOnce(&Chat) -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    let chat = core.chat()?;
    tauri::async_runtime::spawn_blocking(move || f(&chat))
        .await
        .map_err(CmdError::internal)?
        .map_err(CmdError::from)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_threads_list(state: Core<'_>) -> CmdResult<Vec<ThreadSummary>> {
    with_chat(&state, api::chat_threads_list).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_thread_get(state: Core<'_>, id: String) -> CmdResult<ChatThread> {
    with_chat(&state, move |c| {
        api::chat_thread_get(c, ThreadIdArgs { id })
    })
    .await
}

/// Store the question and start the turn; the reply streams as `chat:delta`
/// / `chat:turn` (a new conversation joins the palette).
#[tauri::command(rename_all = "snake_case")]
pub async fn chat_send(
    state: Core<'_>,
    thread_id: Option<String>,
    text: String,
    effort: Option<String>,
    attachments: Option<Vec<AttachmentArg>>,
    connectors: Option<Sources>,
) -> CmdResult<SendResult> {
    let args = SendArgs {
        thread_id,
        text,
        effort,
        attachments,
        connectors,
    };
    let r = with_chat(&state, move |c| api::chat_send(c, args)).await?;
    state.invalidate_palette();
    Ok(r)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_stop(state: Core<'_>, turn_id: String) -> CmdResult<StopResult> {
    with_chat(&state, move |c| api::chat_stop(c, TurnIdArgs { turn_id })).await
}

/// The only way a proposed action runs (03 §7.5).
#[tauri::command(rename_all = "snake_case")]
pub async fn chat_action_confirm(
    state: Core<'_>,
    turn_id: String,
    action_id: String,
    confirm: bool,
) -> CmdResult<ChatAction> {
    let args = ConfirmArgs {
        turn_id,
        action_id,
        confirm,
    };
    with_chat(&state, move |c| api::chat_action_confirm(c, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_thread_delete(state: Core<'_>, id: String) -> CmdResult<DeleteResult> {
    let r = with_chat(&state, move |c| {
        api::chat_thread_delete(c, ThreadIdArgs { id })
    })
    .await?;
    state.invalidate_palette();
    Ok(r)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_sources(state: Core<'_>, turn_id: String) -> CmdResult<ChatSources> {
    with_chat(&state, move |c| {
        api::chat_sources(c, TurnIdArgs { turn_id })
    })
    .await
}

/// The meta line, the disabled input and the `+` menu reasons; `offline`
/// follows the connectors' reachability check.
#[tauri::command(rename_all = "snake_case")]
pub async fn chat_status(state: Core<'_>, effort: Option<String>) -> CmdResult<ChatStatus> {
    if let (Ok(chat), Some(c)) = (state.chat(), state.connectors.get()) {
        chat.set_offline(c.is_offline());
    }
    with_chat(&state, move |c| api::chat_status(c, StatusArgs { effort })).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_thread_rename(state: Core<'_>, id: String, title: String) -> CmdResult<()> {
    with_chat(&state, move |c| {
        api::chat_thread_rename(c, RenameArgs { id, title })
    })
    .await?;
    state.invalidate_palette();
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn chat_pin(state: Core<'_>, turn_id: String, pinned: bool) -> CmdResult<()> {
    with_chat(&state, move |c| {
        api::chat_pin(c, PinArgs { turn_id, pinned })
    })
    .await
}

/// The effort picker and the connector toggles, stored per conversation.
#[tauri::command(rename_all = "snake_case")]
pub async fn chat_thread_set(
    state: Core<'_>,
    id: String,
    effort: Option<String>,
    connectors: Option<Sources>,
) -> CmdResult<()> {
    let args = ThreadSetArgs {
        id,
        effort,
        connectors,
    };
    with_chat(&state, move |c| api::chat_thread_set(c, args)).await
}

/// `+` → Add file: the OS picker; the answer is an `ItemRef` of kind
/// `file` (its id the absolute path), `null` when canceled. The file is
/// cited, never read into the prompt or uploaded.
#[tauri::command(rename_all = "snake_case")]
pub async fn chat_pick_file(app: AppHandle) -> CmdResult<Option<ItemRef>> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        dialogs::pick_any_file(&app, "Add a file to the conversation")
    })
    .await
    .map_err(CmdError::internal)?;
    match picked {
        Picked::Path(p) => Ok(Some(file_ref(&p))),
        Picked::Cancelled => Ok(None),
        Picked::Unavailable => Err(CmdError::new(
            "unavailable",
            "No file picker is available here.",
        )),
    }
}

/// The citation chip of a picked file: its name, the folder (home as `~`).
pub fn file_ref(p: &std::path::Path) -> ItemRef {
    let path = p.to_string_lossy().into_owned();
    let title = p
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    let dir = p
        .parent()
        .map(|d| {
            let d = d.to_string_lossy().into_owned();
            match dirs::home_dir().map(|h| h.to_string_lossy().into_owned()) {
                Some(h) if d == h => "~".to_string(),
                Some(h) if d.starts_with(&format!("{h}/")) => format!("~{}", &d[h.len()..]),
                _ => d,
            }
        })
        .unwrap_or_default();
    ItemRef {
        kind: "file".into(),
        id: path,
        title,
        detail: Some(dir),
        route: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picked_file_is_a_file_citation() {
        let r = file_ref(std::path::Path::new("/srv/code/mode/README.md"));
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            serde_json::json!({"kind": "file", "id": "/srv/code/mode/README.md", "title": "README.md",
                               "detail": "/srv/code/mode", "route": null})
        );
        if let Some(h) = dirs::home_dir() {
            let r = file_ref(&h.join("code").join("spec.pdf"));
            assert_eq!(r.detail.as_deref(), Some("~/code"));
        }
    }
}
