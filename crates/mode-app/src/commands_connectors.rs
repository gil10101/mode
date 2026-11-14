//! The connectors commands (06; `ui/src/state/contract-connectors.ts`,
//! `contract-settings.ts`): accounts and the OAuth loopback (06 §3–§4), the
//! Integrations cards (06 §9, §9A), the user's OAuth clients, the Slack
//! manifest, sources and the Slack channel picker (06 §3.4, §8.4). One thin
//! `#[tauri::command]` per function of `mode_core::connectors::api`;
//! arguments are the snake_case keys the webview sends; an `ApiError`
//! rejects as `{code, message}` ([`CmdError`]); every call runs on the
//! blocking pool (the database writer, the network, the loopback wait).
//! App window only.
//!
//! `account_add_start` / `account_reauthorise` also open `auth_url` in the
//! default browser through the opener plugin (06 §4.3) and say whether that
//! worked (`opened`), so the dialog can offer Copy link otherwise; the URL
//! is never logged. `account_add_finish` waits for the loopback (≤ 5 min).

use std::sync::Arc;

use mode_core::connectors::accounts::{AccountRow, Removed};
use mode_core::connectors::api::{
    self, AccountAddStartArgs, ApiResult, ClaudeCodeCardArgs, IdArgs, ManifestArgs,
    OAuthClientArgs, SlackChannels, SlackChannelsArgs, SlackChannelsSetArgs, SourceRow,
    SourceSetArgs, SourcesListArgs, SourcesRefreshArgs, StateArgs,
};
use mode_core::connectors::claude_code::ClaudeCodeCard;
use mode_core::connectors::health::{CardStatus, HealthReport};
use mode_core::connectors::oauth::ConnectOptions;
use mode_core::connectors::service::ConnectStart;
use mode_core::connectors::{Connectors, Provider};
use mode_core::keyring::SecretStatus;
use mode_core::push::{FlashKind, FlashOpen};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::cmd_error::{CmdError, CmdResult};
use crate::state::CoreState;

type Core<'a> = State<'a, Arc<CoreState>>;

/// Run one api function on the blocking pool.
pub async fn with_connectors<T, F>(core: &CoreState, f: F) -> CmdResult<T>
where
    F: FnOnce(&Connectors) -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    let c = core.connectors()?;
    tauri::async_runtime::spawn_blocking(move || f(&c))
        .await
        .map_err(CmdError::internal)?
        .map_err(CmdError::from)
}

/// The schemes the core opens for the webviews (the app capability's
/// `opener:allow-open-url` list: http, https, mailto, slack).
pub fn external_url_ok(url: &str) -> bool {
    let lower = url.trim_start().to_ascii_lowercase();
    ["http://", "https://", "mailto:", "slack://"]
        .iter()
        .any(|p| lower.starts_with(p))
        && !url.chars().any(char::is_control)
}

/// Open a URL in its default handler (the browser for http / https).
pub fn open_external(app: &AppHandle, url: &str) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    if !external_url_ok(url) {
        return Err(CmdError::invalid(
            "Mode opens only web, mail and Slack links.",
        ));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| CmdError::new("unavailable", format!("the link could not be opened: {e}")))
}

/// `url_open { url }`: open a link in its default handler (the onboarding's
/// Star on GitHub, 03 §2.8 row 7). A thin wrapper over [`open_external`], so
/// the same guard applies and a refused scheme rejects with its message.
/// App window only.
#[tauri::command(rename_all = "snake_case")]
pub async fn url_open(app: AppHandle, url: String) -> CmdResult<()> {
    open_external(&app, &url)
}

/// `account_add_start` / `account_reauthorise`: the flow plus whether the
/// browser was opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthStarted {
    #[serde(flatten)]
    pub start: ConnectStart,
    /// The default browser was asked to open `auth_url`.
    pub opened: bool,
}

fn launch_browser(app: &AppHandle, start: ConnectStart) -> AuthStarted {
    let opened = match open_external(app, &start.auth_url) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(target: "mode::connectors", code = %e.code, "the browser did not open for sign-in");
            false
        }
    };
    AuthStarted { start, opened }
}

// ----------------------------------------------------------------- accounts

#[tauri::command(rename_all = "snake_case")]
pub async fn accounts_list(state: Core<'_>) -> CmdResult<Vec<AccountRow>> {
    with_connectors(&state, api::accounts_list).await
}

/// Start a connect (06 §4.1): PKCE, the loopback listener, then the browser.
#[tauri::command(rename_all = "snake_case")]
pub async fn account_add_start(
    app: AppHandle,
    state: Core<'_>,
    provider: String,
    options: Option<ConnectOptions>,
) -> CmdResult<AuthStarted> {
    let start = with_connectors(&state, move |c| {
        api::account_add_start(c, AccountAddStartArgs { provider, options })
    })
    .await?;
    Ok(launch_browser(&app, start))
}

/// Wait for the loopback, exchange the code, store the account (blocking pool).
#[tauri::command(rename_all = "snake_case")]
pub async fn account_add_finish(core: Core<'_>, state: String) -> CmdResult<AccountRow> {
    let provider = core.connectors.get().and_then(|c| c.flow_provider(&state));
    let r = with_connectors(&core, move |c| {
        api::account_add_finish(c, StateArgs { state })
    })
    .await;
    flash_connect(&core, provider, &r);
    r
}

/// "connected · Gmail" or "Gmail · sign-in failed" in the notch (02 §5.4),
/// opening the Integrations card; a canceled or forgotten flow says
/// nothing. A Google account is named by its card: Gmail, or Google
/// Calendar for a calendar-only account (a failed Google sign-in names Gmail).
pub fn flash_connect(core: &CoreState, provider: Option<Provider>, r: &CmdResult<AccountRow>) {
    let card = |provider: Option<Provider>, services: &[String]| match provider {
        Some(Provider::Slack) => ("Slack", "slack"),
        Some(Provider::Google)
            if !services.is_empty() && !services.iter().any(|s| s == "gmail") =>
        {
            ("Google Calendar", "google-calendar")
        }
        _ => ("Gmail", "gmail"),
    };
    let open = |slug: &str| {
        Some(FlashOpen {
            view: "integrations".into(),
            id: Some(slug.to_string()),
        })
    };
    match r {
        Ok(row) => {
            let (label, slug) = card(Provider::parse(&row.provider), &row.services);
            core.flash(format!("connected · {label}"), FlashKind::Ok, open(slug));
        }
        Err(e) if matches!(e.code.as_str(), "cancelled" | "not_found") => {}
        Err(_) => {
            let (label, slug) = card(provider, &[]);
            core.flash(
                format!("{label} · sign-in failed"),
                FlashKind::Bad,
                open(slug),
            );
        }
    }
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_add_cancel(core: Core<'_>, state: String) -> CmdResult<bool> {
    with_connectors(&core, move |c| {
        api::account_add_cancel(c, StateArgs { state })
    })
    .await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_pause(state: Core<'_>, id: String) -> CmdResult<AccountRow> {
    with_connectors(&state, move |c| api::account_pause(c, IdArgs { id })).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_resume(state: Core<'_>, id: String) -> CmdResult<AccountRow> {
    with_connectors(&state, move |c| api::account_resume(c, IdArgs { id })).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_retry(state: Core<'_>, id: String) -> CmdResult<AccountRow> {
    with_connectors(&state, move |c| api::account_retry(c, IdArgs { id })).await
}

/// Re-authorize (06 §3.3): a fresh consent in the browser; finish with
/// `account_add_finish { state }`.
#[tauri::command(rename_all = "snake_case")]
pub async fn account_reauthorise(
    app: AppHandle,
    state: Core<'_>,
    id: String,
) -> CmdResult<AuthStarted> {
    let start =
        with_connectors(&state, move |c| api::account_reauthorise(c, IdArgs { id })).await?;
    Ok(launch_browser(&app, start))
}

#[tauri::command(rename_all = "snake_case")]
pub async fn account_remove(state: Core<'_>, id: String) -> CmdResult<Removed> {
    with_connectors(&state, move |c| api::account_remove(c, IdArgs { id })).await
}

// -------------------------------------------------------- cards and clients

/// One account of `connectors_status`: the core's health report plus the
/// `AccountSummary` fields of `contract-settings.ts` (`id`, `services`,
/// `status_detail`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountStatusRow {
    pub id: String,
    pub services: Vec<String>,
    pub status_detail: Option<String>,
    #[serde(flatten)]
    pub report: HealthReport,
}

/// `connectors_status`: the card lines (06 §9A) and one row per account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectorsStatusView {
    pub shipped: bool,
    pub offline: bool,
    pub gmail: CardStatus,
    pub calendar: CardStatus,
    pub slack: CardStatus,
    pub accounts: Vec<AccountStatusRow>,
}

/// The core's `connectors_status` with the account summary fields merged.
pub fn connectors_status_view(c: &Connectors) -> ApiResult<ConnectorsStatusView> {
    let st = api::connectors_status(c)?;
    let rows = api::accounts_list(c)?;
    let accounts = st
        .accounts
        .into_iter()
        .map(|report| {
            let row = rows.iter().find(|r| r.id == report.account_id);
            AccountStatusRow {
                id: report.account_id.clone(),
                services: row.map(|r| r.services.clone()).unwrap_or_default(),
                status_detail: row.and_then(|r| r.status_detail.clone()),
                report,
            }
        })
        .collect();
    Ok(ConnectorsStatusView {
        shipped: st.shipped,
        offline: st.offline,
        gmail: st.gmail,
        calendar: st.calendar,
        slack: st.slack,
        accounts,
    })
}

#[tauri::command(rename_all = "snake_case")]
pub async fn connectors_status(state: Core<'_>) -> CmdResult<ConnectorsStatusView> {
    with_connectors(&state, connectors_status_view).await
}

/// The Claude Code card (06 §9); the binary probe is the app's own unless
/// the caller says otherwise.
#[tauri::command(rename_all = "snake_case")]
pub async fn claude_code_card(
    state: Core<'_>,
    binary_found: Option<bool>,
) -> CmdResult<ClaudeCodeCard> {
    let core = Arc::clone(&state);
    let found = match binary_found {
        Some(f) => f,
        None => {
            tauri::async_runtime::spawn_blocking(move || crate::claude::probe(&core).path.is_some())
                .await
                .map_err(CmdError::internal)?
        }
    };
    with_connectors(&state, move |c| {
        api::claude_code_card(
            c,
            ClaudeCodeCardArgs {
                binary_found: found,
            },
        )
    })
    .await
}

/// The user's own OAuth client (06 §6.1, §8.1): stored in the keyring;
/// only `{name, stored}` comes back.
#[tauri::command(rename_all = "snake_case")]
pub async fn oauth_client_set(
    state: Core<'_>,
    provider: String,
    client_id: String,
    client_secret: Option<String>,
    project_id: Option<String>,
    team_id: Option<String>,
) -> CmdResult<SecretStatus> {
    let args = OAuthClientArgs {
        provider,
        client_id,
        client_secret,
        project_id,
        team_id,
    };
    with_connectors(&state, move |c| api::oauth_client_set(c, args)).await
}

/// The Slack app manifest to paste at api.slack.com (06 §8.1).
#[tauri::command(rename_all = "snake_case")]
pub async fn slack_manifest(state: Core<'_>, writeback: Option<bool>) -> CmdResult<String> {
    let args = ManifestArgs {
        writeback: writeback.unwrap_or(true),
    };
    with_connectors(&state, move |c| api::slack_manifest(c, args)).await
}

// ------------------------------------------------------------------ sources

#[tauri::command(rename_all = "snake_case")]
pub async fn sources_list(
    state: Core<'_>,
    account_id: Option<String>,
) -> CmdResult<Vec<SourceRow>> {
    with_connectors(&state, move |c| {
        api::sources_list(c, SourcesListArgs { account_id })
    })
    .await
}

/// Rediscover labels, calendars or DMs, then list (the card's refresh).
#[tauri::command(rename_all = "snake_case")]
pub async fn sources_refresh(state: Core<'_>, account_id: String) -> CmdResult<Vec<SourceRow>> {
    with_connectors(&state, move |c| {
        api::sources_refresh(c, SourcesRefreshArgs { account_id })
    })
    .await
}

/// `source_set { id, enabled?, vip?, poll_seconds? }`; `poll_seconds ≤ 0`
/// clears the per-source interval.
#[tauri::command(rename_all = "snake_case")]
pub async fn source_set(
    state: Core<'_>,
    id: String,
    enabled: Option<bool>,
    vip: Option<bool>,
    poll_seconds: Option<i64>,
) -> CmdResult<SourceRow> {
    let args = SourceSetArgs {
        id,
        enabled,
        vip,
        poll_seconds: poll_seconds.map(Some),
    };
    with_connectors(&state, move |c| api::source_set(c, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn slack_channels_list(
    state: Core<'_>,
    account_id: String,
    refresh: Option<bool>,
) -> CmdResult<SlackChannels> {
    let args = SlackChannelsArgs {
        account_id,
        refresh: refresh.unwrap_or(false),
    };
    with_connectors(&state, move |c| api::slack_channels_list(c, args)).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn slack_channels_set(
    state: Core<'_>,
    account_id: String,
    channel_ids: Vec<String>,
) -> CmdResult<Vec<SourceRow>> {
    let args = SlackChannelsSetArgs {
        account_id,
        channel_ids,
    };
    with_connectors(&state, move |c| api::slack_channels_set(c, args)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_mail_and_slack_links_open() {
        assert!(external_url_ok(
            "https://accounts.google.com/o/oauth2/v2/auth?x=1"
        ));
        assert!(external_url_ok("http://127.0.0.1:53682/callback"));
        assert!(external_url_ok("mailto:ana@example.com"));
        assert!(external_url_ok("slack://channel?team=T1&id=C1"));
        assert!(external_url_ok("HTTPS://MEET.GOOGLE.COM/abc"));
        assert!(!external_url_ok("file:///etc/passwd"));
        assert!(!external_url_ok("javascript:alert(1)"));
        assert!(!external_url_ok("zoommtg://zoom.us/join"));
        assert!(!external_url_ok("https://x.example/\nnext"));
    }
}
