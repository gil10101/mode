//! Settings → AI and the secrets (06 §10, 08 §8; `contract-connectors.ts`,
//! `contract-settings.ts`): the backend list and choice, the API key and
//! the local bearer in the keyring, health, the **Test call**, the
//! Integrations card status, and `secret_status` / `secret_set`.
//!
//! Secrets go through `mode_core::keyring` (the OS keyring; the explicit
//! file store `secrets-dev.json` on Linux): entry names of 06 §13
//! (`ai:anthropic_api_key`, `ai:local_token`, `ai:provider:<id>`; the
//! webview may also say `anthropic_api_key` / `local_token`). A value is never logged, returned,
//! pushed or exported: the webview sees `stored`, the last four characters
//! of an API key, and when it was written this run. `secret_status` also
//! answers for the OAuth client entries (`oauth_client:google`,
//! `oauth_client:slack[:<team>]`), which are written by `oauth_client_set`.

use std::sync::Arc;

use mode_core::connectors::ai::providers::{ProviderDraft, Route};
use mode_core::connectors::ai::{AiHealth, BackendKind};
use mode_core::connectors::api::{
    self, AiBackendSetArgs, AiBackendsStatus, AiHealthArgs, AiKeySetArgs, AiTestArgs, AiTestResult,
    BackendRow, IdArgs, ProviderKeyArgs, ProviderKindRow, ProviderRow, ProviderTestArgs,
    SecretSetArgs,
};
use mode_core::keyring::{SecretStatus, names};
use mode_core::repo::{self, Account};
use serde::Serialize;
use tauri::State;

use crate::cmd_error::{CmdError, CmdResult, blocking};
use crate::commands_connectors::with_connectors;
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// Longest secret accepted (an API key is ~110 characters).
pub const MAX_SECRET_CHARS: usize = 1_000;

/// What the webview may know about a secret (`SecretStatus` of the
/// settings contract, plus the API key's last four characters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SecretView {
    /// The name the webview asked with.
    pub name: String,
    pub stored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last4: Option<String>,
    /// Written this run (the keyring keeps no time).
    pub updated_at: Option<String>,
}

/// The keyring entry a webview name reads (status only for OAuth clients).
pub fn status_entry(name: &str) -> CmdResult<String> {
    match name {
        "anthropic_api_key" | names::AI_ANTHROPIC_API_KEY => {
            Ok(names::AI_ANTHROPIC_API_KEY.to_string())
        }
        "local_token" | names::AI_LOCAL_TOKEN => Ok(names::AI_LOCAL_TOKEN.to_string()),
        names::OAUTH_CLIENT_GOOGLE | names::OAUTH_CLIENT_SLACK => Ok(name.to_string()),
        n if n.starts_with("oauth_client:slack:") && n.len() <= 64 => Ok(n.to_string()),
        n if n.starts_with(names::AI_PROVIDER_PREFIX) && n.len() <= 96 && names::is_known(n) => {
            Ok(n.to_string())
        }
        _ => Err(CmdError::invalid(format!(
            "{name:?} is not a secret Mode keeps"
        ))),
    }
}

/// The keyring entry a webview name may write (`secret_set`): the AI ones.
pub fn set_entry(name: &str) -> CmdResult<&'static str> {
    match name {
        "anthropic_api_key" | names::AI_ANTHROPIC_API_KEY => Ok(names::AI_ANTHROPIC_API_KEY),
        "local_token" | names::AI_LOCAL_TOKEN => Ok(names::AI_LOCAL_TOKEN),
        _ => Err(CmdError::invalid(format!(
            "{name:?} is not a secret Settings may write"
        ))),
    }
}

fn view(core: &CoreState, name: &str, entry: &str, st: SecretStatus) -> SecretView {
    SecretView {
        name: name.to_string(),
        stored: st.stored,
        last4: st.last4,
        updated_at: CoreState::lock(&core.secret_updated).get(entry).cloned(),
    }
}

/// `secret_status` (blocking: the keyring may block on the OS).
pub fn secret_status_of(core: &CoreState, name: &str) -> CmdResult<SecretView> {
    let entry = status_entry(name)?;
    let st = core.keyring().status(&entry);
    Ok(view(core, name, &entry, st))
}

/// `secret_set`: store (or, for `None` / blank, delete) an AI secret. With
/// the connectors service running this is `ai_key_set` (scoring re-queued
/// and kicked); otherwise the keyring directly.
pub fn secret_set_of(core: &CoreState, name: &str, value: Option<&str>) -> CmdResult<SecretView> {
    let entry = set_entry(name)?;
    let value = value.map(str::trim).filter(|v| !v.is_empty());
    if let Some(v) = value
        && (v.chars().count() > MAX_SECRET_CHARS || v.chars().any(char::is_control))
    {
        return Err(CmdError::invalid(
            "That does not look like a key: too long or not one line.",
        ));
    }
    let st = match core.connectors.get() {
        Some(c) => api::secret_set(
            c,
            SecretSetArgs {
                name: entry.to_string(),
                value: value.map(str::to_string),
            },
        )?,
        None => {
            let kr = core.keyring();
            match value {
                Some(v) => kr.set(entry, v),
                None => kr.delete(entry).map(drop),
            }
            .map_err(|e| CmdError::new("unavailable", e.to_string()))?;
            kr.status(entry)
        }
    };
    tracing::info!(
        entry,
        stored = st.stored,
        store = core.keyring().kind(),
        "secret written"
    );
    CoreState::lock(&core.secret_updated).insert(entry.to_string(), mode_core::db::now_rfc3339());
    Ok(view(core, name, entry, st))
}

/// "Delete all Mode data" (03 §8.15): remove every account (revoked
/// upstream first, best effort, when the connectors service runs) and
/// every keyring entry Mode writes. Returns how many entries existed.
pub fn forget_all_secrets(core: &CoreState) -> usize {
    let accounts = core
        .db
        .read(|c| repo::query::<Account>(c, "1", &[]))
        .unwrap_or_default();
    if let Some(c) = core.connectors.get() {
        for a in &accounts {
            if let Err(e) = api::account_remove(c, api::IdArgs { id: a.id.clone() }) {
                tracing::warn!(code = %e.code, "account not removed upstream");
            }
        }
    }
    let mut entries: Vec<String> = vec![
        names::AI_ANTHROPIC_API_KEY.into(),
        names::AI_LOCAL_TOKEN.into(),
        names::OAUTH_CLIENT_GOOGLE.into(),
        names::OAUTH_CLIENT_SLACK.into(),
    ];
    for p in core
        .db
        .read(mode_core::connectors::ai::providers::list)
        .unwrap_or_default()
    {
        entries.push(p.key_entry());
    }
    for a in &accounts {
        entries.push(a.keyring_ref.clone());
        entries.push(names::account(&a.id));
        if a.provider == "slack" {
            entries.push(names::oauth_client_slack(a.team_id.as_deref()));
        }
    }
    entries.sort();
    entries.dedup();
    let kr = core.keyring();
    let n = entries
        .iter()
        .filter(|e| names::is_known(e) && kr.delete(e).unwrap_or(false))
        .count();
    CoreState::lock(&core.secret_updated).clear();
    n
}

// ----------------------------------------------------------------- secrets

/// Whether a secret is stored (never its value).
#[tauri::command(rename_all = "snake_case")]
pub async fn secret_status(state: Core<'_>, name: String) -> CmdResult<SecretView> {
    let core = Arc::clone(&state);
    blocking(move || secret_status_of(&core, &name)).await
}

/// Store a secret in the keyring; `value: null` (or blank) deletes it.
#[tauri::command(rename_all = "snake_case")]
pub async fn secret_set(
    state: Core<'_>,
    name: String,
    value: Option<String>,
) -> CmdResult<SecretView> {
    let core = Arc::clone(&state);
    blocking(move || secret_set_of(&core, &name, value.as_deref())).await
}

// ---------------------------------------------------------------- backends

#[tauri::command(rename_all = "snake_case")]
pub async fn ai_backends_list(state: Core<'_>) -> CmdResult<Vec<BackendRow>> {
    with_connectors(&state, api::ai_backends_list).await
}

/// Choose the backend, the local URL (loopback or private only, 06 §10.3),
/// local-only, and the models (configuration values).
#[allow(clippy::too_many_arguments)]
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_backend_set(
    state: Core<'_>,
    backend: Option<BackendKind>,
    local_url: Option<String>,
    local_only: Option<bool>,
    model_scoring: Option<String>,
    model_summary: Option<String>,
    model_chat: Option<String>,
    local_model_scoring: Option<String>,
    local_model_summary: Option<String>,
    local_model_chat: Option<String>,
) -> CmdResult<AiHealth> {
    let args = AiBackendSetArgs {
        backend,
        local_url,
        local_only,
        model_scoring,
        model_summary,
        model_chat,
        local_model_scoring,
        local_model_summary,
        local_model_chat,
    };
    with_connectors(&state, move |c| api::ai_backend_set(c, args)).await
}

/// The Claude API key or the local bearer; only `stored` and the last four
/// characters come back.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_key_set(
    state: Core<'_>,
    backend: BackendKind,
    key: Option<String>,
) -> CmdResult<SecretStatus> {
    with_connectors(&state, move |c| {
        api::ai_key_set(c, AiKeySetArgs { backend, key })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn ai_health(state: Core<'_>, probe: Option<bool>) -> CmdResult<AiHealth> {
    let args = AiHealthArgs {
        probe: probe.unwrap_or(false),
    };
    with_connectors(&state, move |c| api::ai_health(c, args)).await
}

/// Settings → AI **Test call**: the selected backend, or `backend`.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_test_call(
    state: Core<'_>,
    backend: Option<BackendKind>,
) -> CmdResult<AiTestResult> {
    with_connectors(&state, move |c| {
        api::ai_test_call_with(
            c,
            AiTestArgs {
                backend,
                provider: None,
                model: None,
            },
        )
    })
    .await
}

/// The Integrations / Settings card: the backend, the key's presence, the
/// local backends' reachability and models, this month's tokens.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_backends_status(state: Core<'_>) -> CmdResult<AiBackendsStatus> {
    with_connectors(&state, api::ai_backends_status).await
}

// --------------------------------------------------------------- providers

/// The provider registry (06 §10.1A): built-ins first, then the user's.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_providers_list(state: Core<'_>) -> CmdResult<Vec<ProviderRow>> {
    with_connectors(&state, api::ai_providers_list).await
}

/// Insert or update a provider; an invalid draft rejects with `fields`.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_provider_save(state: Core<'_>, draft: ProviderDraft) -> CmdResult<ProviderRow> {
    let c = state.connectors()?;
    let d = draft.clone();
    // The field errors of the draft reach the webview as `fields`.
    let fields = blocking(
        move || -> CmdResult<Vec<mode_core::scheduler::actions::FieldError>> {
            let existing = match d.id.as_deref().filter(|i| !i.trim().is_empty()) {
                Some(id) => c
                    .db()
                    .read(|conn| mode_core::connectors::ai::providers::get(conn, id))?,
                None => None,
            };
            Ok(mode_core::connectors::ai::providers::validate(
                &d,
                existing.as_ref(),
            ))
        },
    )
    .await?;
    if !fields.is_empty() {
        return Err(CmdError::from(mode_core::scheduler::api::SaveError {
            code: "invalid".into(),
            message: "The provider cannot be saved as it is.".into(),
            fields,
        }));
    }
    with_connectors(&state, move |c| api::ai_provider_save(c, draft)).await
}

/// Remove a provider the user added (a built-in is refused).
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_provider_delete(state: Core<'_>, id: String) -> CmdResult<bool> {
    with_connectors(&state, move |c| api::ai_provider_delete(c, IdArgs { id })).await
}

/// A provider's secret into the keyring; only `stored` and the last four come back.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_provider_key_set(
    state: Core<'_>,
    id: String,
    key: Option<String>,
) -> CmdResult<SecretStatus> {
    with_connectors(&state, move |c| {
        api::ai_provider_key_set(c, ProviderKeyArgs { id, key })
    })
    .await
}

/// The Test call against one provider.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_provider_test(
    state: Core<'_>,
    id: String,
    model: Option<String>,
) -> CmdResult<AiTestResult> {
    with_connectors(&state, move |c| {
        api::ai_provider_test(c, ProviderTestArgs { id, model })
    })
    .await
}

/// The models a provider lists (the named ones first).
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_provider_models(state: Core<'_>, id: String) -> CmdResult<Vec<String>> {
    with_connectors(&state, move |c| api::ai_provider_models(c, IdArgs { id })).await
}

/// The kinds a provider may be (the add dialog).
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_provider_kinds() -> CmdResult<Vec<ProviderKindRow>> {
    Ok(api::ai_provider_kinds())
}

/// One route per feature (chat, scoring, summary).
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_routes_list(state: Core<'_>) -> CmdResult<Vec<Route>> {
    with_connectors(&state, api::ai_routes_list).await
}

/// Store a route; an invalid one rejects with `fields`.
#[tauri::command(rename_all = "snake_case")]
pub async fn ai_route_set(state: Core<'_>, route: Route) -> CmdResult<Route> {
    let c = state.connectors()?;
    let r = route.clone();
    let fields = blocking(
        move || -> CmdResult<Vec<mode_core::scheduler::actions::FieldError>> {
            let providers = c.db().read(mode_core::connectors::ai::providers::list)?;
            Ok(mode_core::connectors::ai::providers::validate_route(
                &r, &providers,
            ))
        },
    )
    .await?;
    if !fields.is_empty() {
        return Err(CmdError::from(mode_core::scheduler::api::SaveError {
            code: "invalid".into(),
            message: "The route cannot be saved as it is.".into(),
            fields,
        }));
    }
    with_connectors(&state, move |c| api::ai_route_set(c, route)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_to_06_entries_and_unknown_names_are_refused() {
        assert_eq!(
            status_entry("anthropic_api_key").unwrap(),
            "ai:anthropic_api_key"
        );
        assert_eq!(status_entry("ai:local_token").unwrap(), "ai:local_token");
        assert_eq!(
            status_entry("oauth_client:google").unwrap(),
            "oauth_client:google"
        );
        assert_eq!(
            status_entry("oauth_client:slack:T0ACME").unwrap(),
            "oauth_client:slack:T0ACME"
        );
        assert_eq!(status_entry("account:x").unwrap_err().code, "invalid");
        assert_eq!(
            set_entry("oauth_client:google").unwrap_err().code,
            "invalid"
        );
        assert_eq!(set_entry("local_token").unwrap(), "ai:local_token");
    }
}
