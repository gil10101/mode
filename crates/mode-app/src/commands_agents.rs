//! The Agents commands (03 §7E; `ui/src/state/contract-agents.ts`): one thin
//! `#[tauri::command]` per function of `mode_core::agents` on the blocking
//! pool. `agent_save` and `agent_memory_save` reject an invalid draft as
//! `{code: "invalid", message, fields: [{field, message}]}`; every other
//! refusal is `{code, message}`. A save, a delete or a memory change
//! publishes `agents.changed` with the list.

use std::sync::Arc;

use mode_core::agents::{self, AgentDraft, AgentSummary, AgentView, MemoryRow};
use mode_core::push::Event;
use serde_json::Value;
use tauri::State;

use crate::cmd_error::{CmdError, CmdResult};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// The `agents.changed` payload: the list.
pub fn payload(core: &CoreState) -> Option<Value> {
    match core.db.read(agents::list) {
        Ok(rows) => serde_json::to_value(rows).ok(),
        Err(e) => {
            tracing::warn!(error = %e, "agents payload failed");
            None
        }
    }
}

async fn blocking<T, E, F>(f: F) -> CmdResult<T>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: Into<CmdError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(CmdError::internal)?
        .map_err(Into::into)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn agents_list(state: Core<'_>) -> CmdResult<Vec<AgentSummary>> {
    let db = Arc::clone(&state.db);
    blocking(move || db.read(agents::list)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn agent_get(state: Core<'_>, id: String) -> CmdResult<AgentView> {
    let db = Arc::clone(&state.db);
    blocking(move || -> CmdResult<AgentView> {
        db.read(|c| agents::get(c, &id, &mode_core::db::now_rfc3339()))?
            .ok_or_else(|| CmdError::not_found("no such agent"))
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn agent_save(state: Core<'_>, draft: AgentDraft) -> CmdResult<AgentView> {
    let db = Arc::clone(&state.db);
    let view = blocking(move || -> CmdResult<AgentView> {
        db.write(move |c| {
            let now = mode_core::db::now_rfc3339();
            Ok(agents::save(c, &draft, &now))
        })?
        .map_err(CmdError::from)
    })
    .await?;
    state.publish([Event::Agents]);
    Ok(view)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn agent_delete(state: Core<'_>, id: String) -> CmdResult<bool> {
    let db = Arc::clone(&state.db);
    let gone = blocking(move || db.write(move |c| agents::delete(c, &id))).await?;
    state.publish([Event::Agents, Event::Automations]);
    Ok(gone)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn agent_memory_save(
    state: Core<'_>,
    profile_id: String,
    id: Option<String>,
    title: String,
    body: String,
) -> CmdResult<MemoryRow> {
    let db = Arc::clone(&state.db);
    let row = blocking(move || -> CmdResult<MemoryRow> {
        db.write(move |c| {
            let now = mode_core::db::now_rfc3339();
            Ok(agents::memory_save(
                c,
                &profile_id,
                id.as_deref(),
                &title,
                &body,
                &now,
            ))
        })?
        .map_err(CmdError::from)
    })
    .await?;
    state.publish([Event::Agents]);
    Ok(row)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn agent_memory_delete(state: Core<'_>, id: String) -> CmdResult<bool> {
    let db = Arc::clone(&state.db);
    let gone = blocking(move || db.write(move |c| agents::memory_delete(c, &id))).await?;
    state.publish([Event::Agents]);
    Ok(gone)
}

/// The MCP servers Claude Code has configured, for the agent's folders (its
/// repositories and workspace): names, scopes and transports only (03 §7E.2
/// "Does", Connectors).
#[tauri::command(rename_all = "snake_case")]
pub async fn mcp_servers_list(
    folders: Option<Vec<String>>,
) -> CmdResult<Vec<agents::mcp::McpServer>> {
    blocking(move || -> CmdResult<Vec<agents::mcp::McpServer>> {
        let folders: Vec<std::path::PathBuf> = folders
            .unwrap_or_default()
            .iter()
            .filter_map(|f| agents::resolve_repo(f))
            .map(std::path::PathBuf::from)
            .collect();
        Ok(agents::mcp::configured(
            agents::mcp::claude_json().as_deref(),
            &folders,
        ))
    })
    .await
}

/// The skills Claude Code would load for a workspace (03 §7E.2 "Knows").
#[tauri::command(rename_all = "snake_case")]
pub async fn skills_list(workspace: Option<String>) -> CmdResult<Vec<String>> {
    blocking(move || -> CmdResult<Vec<String>> {
        Ok(agents::skills::installed(
            workspace.as_deref().map(std::path::Path::new),
        ))
    })
    .await
}
