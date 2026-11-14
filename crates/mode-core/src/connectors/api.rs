//! One plain function per connectors / inbox / calendar / alerts / AI
//! command, with args and result structs (`serde` snake_case), so
//! `mode-app` wraps each in a `#[tauri::command]` mechanically, like
//! `notes::api`. Errors are the same `ApiError {code, message}`. Functions
//! that touch the network or wait on the browser are blocking: the app runs
//! every call on the blocking pool. Mutations request the matching
//! `*.changed` pushes through the service.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::accounts::{self, AccountRow, Removed};
use super::ai::providers::{self, Provider as AiProvider, ProviderDraft, ProviderKind, Route};
use super::ai::{self, AiConfig, AiHealth, AiSettings, BackendKind, HealthState};
use super::health::{self, CardStatus, HealthReport};
use super::oauth::{self, ConnectOptions, OAuthClient, OAuthError};
use super::scheduler;
use super::service::{AlertCommand, ConnectStart, Connectors, OsState, PollReport};
use super::store::{self, TriageResult};
use super::{ConnectorError, Provider, SourceKind, TriageAction};
use crate::calendar::{self, EventRow};
use crate::db::{Result as DbResult, format_timestamp};
use crate::keyring::{SecretStatus, names};
use crate::links::{self, ItemLink};
use crate::notes::Notes;
use crate::push::Event;
use crate::queries::Clock;
use crate::repo::{self, Account, AccountSource};
use crate::settings;
use crate::urgency::alerts::{self, AlertOutcome, QuietChanged, QuietHours};
use crate::urgency::route::{self, InboxFilter, InboxListResult, ThreadDetail};
use crate::urgency::snooze::{self, Preset, SnoozeRequest};
use crate::urgency::vips::{self, VipAdd, VipChange, VipRow};
use crate::urgency::{self, Tier, with_tz};

pub use crate::notes::api::{ApiError, ApiResult};

fn err(code: &str, message: impl Into<String>) -> ApiError {
    ApiError {
        code: code.into(),
        message: message.into(),
    }
}

/// A connector failure as `{code, message}`: `not_configured`, `auth`,
/// `rate_limited`, `offline`, `not_found`, `unsupported`, `provider`.
pub fn connector_error(e: ConnectorError) -> ApiError {
    let code = match &e {
        ConnectorError::NotConfigured(_) => "not_configured",
        ConnectorError::Unauthorized
        | ConnectorError::InvalidGrant(_)
        | ConnectorError::Revoked(_) => "auth",
        ConnectorError::RateLimited { .. } => "rate_limited",
        ConnectorError::Offline(_) => "offline",
        ConnectorError::NotFound => "not_found",
        ConnectorError::Unsupported | ConnectorError::MissingScope(_) => "unsupported",
        ConnectorError::Keyring(_) => "keyring",
        ConnectorError::Db(_) => "internal",
        _ => "provider",
    };
    err(code, crate::redact::scrub(&e.to_string()).into_owned())
}

/// An OAuth flow failure (06 §4.4 texts).
pub fn oauth_error(e: OAuthError) -> ApiError {
    let code = match &e {
        OAuthError::AccessDenied => "access_denied",
        OAuthError::AdminApproval => "admin_approval",
        OAuthError::Timeout => "timeout",
        OAuthError::Cancelled => "cancelled",
        OAuthError::PortsBusy(_) => "ports_busy",
        OAuthError::UnknownFlow => "not_found",
        OAuthError::Exchange(_) => "exchange",
        OAuthError::Connector(_) => "provider",
    };
    err(code, e.to_string())
}

fn clock(c: &Connectors) -> Clock {
    Clock {
        now: format_timestamp(c.now()),
        today: String::new(),
        tomorrow: String::new(),
    }
}

/// The UTC start of the device's local day (the "resurfaced" marker).
fn day_start(c: &Connectors) -> ApiResult<String> {
    let (now, tz) = (c.now(), c.tz());
    Ok(c.db().read(move |conn| {
        Ok(with_tz(conn, tz, |z| {
            let d = z.to_local(now).date();
            format_timestamp(z.to_utc(time::PrimitiveDateTime::new(d, time::Time::MIDNIGHT)))
        }))
    })?)
}

fn provider(s: &str) -> ApiResult<Provider> {
    Provider::parse(s).ok_or_else(|| err("invalid", format!("unknown provider {s}")))
}

// ----------------------------------------------------------------- accounts

/// `accounts_list`.
pub fn accounts_list(c: &Connectors) -> ApiResult<Vec<AccountRow>> {
    let now = c.now();
    Ok(c.db().read(|conn| accounts::list(conn, now))?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountAddStartArgs {
    /// `google` (Gmail + Calendar) | `slack`.
    pub provider: String,
    #[serde(default)]
    pub options: Option<ConnectOptions>,
}

/// `account_add_start { provider } → { auth_url, state }`: the app opens
/// `auth_url` in the system browser (or shows Copy link).
pub fn account_add_start(c: &Connectors, args: AccountAddStartArgs) -> ApiResult<ConnectStart> {
    let p = provider(&args.provider)?;
    c.begin_connect(p, args.options.unwrap_or_default())
        .map_err(oauth_error)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateArgs {
    pub state: String,
}

/// `account_add_finish { state } → AccountRow`: waits for the loopback
/// (≤ 5 min), exchanges the code, stores the account. Blocking.
pub fn account_add_finish(c: &Connectors, args: StateArgs) -> ApiResult<AccountRow> {
    c.finish_connect(&args.state).map_err(oauth_error)
}

/// `account_add_cancel { state }`.
pub fn account_add_cancel(c: &Connectors, args: StateArgs) -> ApiResult<bool> {
    Ok(c.cancel_connect(&args.state))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdArgs {
    pub id: String,
}

fn account_row(c: &Connectors, id: &str) -> ApiResult<AccountRow> {
    let now = c.now();
    Ok(c.db().read(|conn| {
        let a = repo::must_get::<Account>(conn, id)?;
        accounts::row(conn, &a, now)
    })?)
}

/// `account_pause { id }`.
pub fn account_pause(c: &Connectors, args: IdArgs) -> ApiResult<AccountRow> {
    let now = c.now();
    let id = args.id.clone();
    c.db().write(move |conn| accounts::pause(conn, &id, now))?;
    c.changed(&[Event::Accounts, Event::Health]);
    account_row(c, &args.id)
}

/// `account_resume { id }`: active again with an immediate catch-up poll.
pub fn account_resume(c: &Connectors, args: IdArgs) -> ApiResult<AccountRow> {
    let now = c.now();
    let id = args.id.clone();
    c.db().write(move |conn| accounts::resume(conn, &id, now))?;
    c.wake();
    c.changed(&[Event::Accounts, Event::Health]);
    account_row(c, &args.id)
}

/// `account_retry { id }`: an `error` account polls again now.
pub fn account_retry(c: &Connectors, args: IdArgs) -> ApiResult<AccountRow> {
    let now = c.now();
    let id = args.id.clone();
    c.db()
        .write(move |conn| health::transition(conn, &id, &health::Transition::UserRetry, now))?;
    c.wake();
    c.changed(&[Event::Accounts, Event::Health]);
    account_row(c, &args.id)
}

/// `account_reauthorise { id } → { auth_url, state }`; finish with
/// `account_add_finish` (the same identity updates in place).
pub fn account_reauthorise(c: &Connectors, args: IdArgs) -> ApiResult<ConnectStart> {
    c.begin_reauthorise(&args.id).map_err(oauth_error)
}

/// `account_remove { id }`: revoke upstream (best effort), delete the
/// keyring entry and the rows (06 §3.3).
pub fn account_remove(c: &Connectors, args: IdArgs) -> ApiResult<Removed> {
    let r = accounts::remove(c.db(), c.http().as_ref(), c.keyring(), &args.id)
        .map_err(connector_error)?;
    c.changed(&[
        Event::Accounts,
        Event::Health,
        Event::Inbox,
        Event::Agenda,
        Event::Pill,
    ]);
    Ok(r)
}

/// The Integrations card status lines (06 §9A) and the Diagnostics lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorsStatus {
    pub shipped: bool,
    pub offline: bool,
    pub gmail: CardStatus,
    pub calendar: CardStatus,
    pub slack: CardStatus,
    pub accounts: Vec<HealthReport>,
}

/// `connectors_status` (the settings contract's proposed command).
pub fn connectors_status(c: &Connectors) -> ApiResult<ConnectorsStatus> {
    let now = c.now();
    let offline = c.is_offline();
    let settings = c.db().read(super::ConnectorSettings::load)?;
    let accounts = c
        .db()
        .read(|conn| repo::query::<Account>(conn, "1 ORDER BY position", &[]))?;
    let reports: Vec<HealthReport> = accounts
        .iter()
        .map(|a| Ok(health::report(a, &c.runtime(a, &settings)?, now)))
        .collect::<crate::db::Result<_>>()?;
    let has = |a: &Account, s: &str| super::json_strings(&a.services_json).iter().any(|x| x == s);
    let pick = |s: &str| -> Vec<HealthReport> {
        accounts
            .iter()
            .zip(reports.iter())
            .filter(|(a, _)| has(a, s))
            .map(|(_, r)| r.clone())
            .collect()
    };
    Ok(ConnectorsStatus {
        shipped: true,
        offline,
        gmail: health::card_status(&pick("gmail"), now),
        calendar: health::card_status(&pick("calendar"), now),
        slack: health::card_status(&pick("slack"), now),
        accounts: reports,
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeCodeCardArgs {
    /// The app's probe for the `claude` binary.
    #[serde(default)]
    pub binary_found: bool,
}

/// `claude_code_card { binary_found }`: the Integrations card of 06 §9.
pub fn claude_code_card(
    c: &Connectors,
    args: ClaudeCodeCardArgs,
) -> ApiResult<super::claude_code::ClaudeCodeCard> {
    let since = day_start(c)?;
    Ok(c.db()
        .read(move |conn| super::claude_code::card(conn, args.binary_found, &since))?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthClientArgs {
    /// `google` | `slack`.
    pub provider: String,
    pub client_id: String,
    /// Google only (the user's own desktop client, 06 §6.1).
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Slack: the workspace this app belongs to (`oauth_client:slack:<team_id>`).
    #[serde(default)]
    pub team_id: Option<String>,
}

/// `oauth_client_set`: Integrations → Gmail / Slack card → OAuth client.
/// Stored in the keyring; only "stored" comes back.
pub fn oauth_client_set(c: &Connectors, args: OAuthClientArgs) -> ApiResult<SecretStatus> {
    let p = provider(&args.provider)?;
    let id = args.client_id.trim().to_string();
    if id.is_empty() {
        return Err(err("invalid", "the client ID is empty"));
    }
    if p == Provider::Google
        && args
            .client_secret
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err(err("invalid", "the Google client needs its client secret"));
    }
    let entry = oauth::client_entry(p, args.team_id.as_deref());
    let client = OAuthClient {
        client_id: id,
        client_secret: if p == Provider::Google {
            args.client_secret.map(|s| s.trim().to_string())
        } else {
            None
        },
        project_id: args.project_id,
        team_id: args.team_id,
    };
    c.keyring()
        .set_json(&entry, &client)
        .map_err(|e| err("keyring", e.to_string()))?;
    Ok(c.keyring().status(&entry))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestArgs {
    #[serde(default = "yes")]
    pub writeback: bool,
}

fn yes() -> bool {
    true
}

/// `slack_manifest`: the manifest to paste at api.slack.com (06 §8.1).
pub fn slack_manifest(c: &Connectors, args: ManifestArgs) -> ApiResult<String> {
    let s = c.db().read(super::ConnectorSettings::load)?;
    Ok(super::slack::manifest(&s.oauth_ports, args.writeback))
}

// ------------------------------------------------------------------ sources

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRow {
    pub id: String,
    pub account_id: String,
    pub kind: String,
    pub external_id: String,
    pub name: String,
    pub enabled: bool,
    pub vip: bool,
    pub poll_seconds: Option<i64>,
    pub last_polled_at: Option<String>,
    /// DMs, group DMs and mentions cannot be unwatched.
    pub always_on: bool,
}

fn source_row(s: AccountSource) -> SourceRow {
    SourceRow {
        always_on: SourceKind::parse(&s.kind).is_some_and(|k| k.always_on()),
        id: s.id,
        account_id: s.account_id,
        kind: s.kind,
        external_id: s.external_id,
        name: s.name,
        enabled: s.enabled,
        vip: s.vip,
        poll_seconds: s.poll_seconds,
        last_polled_at: s.last_polled_at,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcesListArgs {
    #[serde(default)]
    pub account_id: Option<String>,
}

/// `sources_list { account_id? }`.
pub fn sources_list(c: &Connectors, args: SourcesListArgs) -> ApiResult<Vec<SourceRow>> {
    let rows = c.db().read(|conn| match &args.account_id {
        Some(a) => accounts::sources(conn, a),
        None => repo::query::<AccountSource>(conn, "1 ORDER BY account_id, kind, name", &[]),
    })?;
    Ok(rows.into_iter().map(source_row).collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcesRefreshArgs {
    pub account_id: String,
}

/// `sources_refresh { account_id } → [SourceRow]`: rediscover labels,
/// calendars or DMs (the Integrations card's refresh), then list.
pub fn sources_refresh(c: &Connectors, args: SourcesRefreshArgs) -> ApiResult<Vec<SourceRow>> {
    let added = c.discover(&args.account_id).map_err(connector_error)?;
    if added > 0 {
        c.changed(&[Event::Accounts]);
    }
    sources_list(
        c,
        SourcesListArgs {
            account_id: Some(args.account_id),
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSetArgs {
    pub id: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Important channel (07 §5).
    #[serde(default)]
    pub vip: Option<bool>,
    /// Slow a source down (never below the provider default... the
    /// scheduler treats it as the base interval).
    #[serde(default)]
    pub poll_seconds: Option<Option<i64>>,
}

/// `source_set { id, enabled }` (+ `vip`, `poll_seconds`).
pub fn source_set(c: &Connectors, args: SourceSetArgs) -> ApiResult<SourceRow> {
    let now = c.now();
    let tz = c.tz();
    let a = args.clone();
    let row = c.db().write(move |conn| {
        repo::tx(conn, |tx| {
            let now_s = format_timestamp(now);
            if let Some(e) = a.enabled {
                accounts::set_source_enabled(tx, &a.id, e, &now_s)?;
            }
            if let Some(p) = a.poll_seconds {
                tx.execute(
                    "UPDATE account_sources SET poll_seconds = ?2, updated_at = ?3 WHERE id = ?1",
                    rusqlite::params![a.id, p.filter(|x| *x > 0), now_s],
                )?;
            }
            if let Some(v) = a.vip {
                let ctx = urgency::Ctx::load(tx, now, tz)?;
                let days = settings::retention_in(tx)?.messages_days;
                vips::set_source_vip(tx, &a.id, v, &ctx, days)?;
            }
            repo::must_get::<AccountSource>(tx, &a.id)
        })
    })?;
    if args.vip.is_some() {
        c.changed(&[Event::Inbox, Event::Pill]);
    }
    c.changed(&[Event::Accounts]);
    Ok(source_row(row))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackChannelsArgs {
    pub account_id: String,
    #[serde(default)]
    pub refresh: bool,
}

/// One row of the channel picker (06 §8.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRow {
    pub channel_id: String,
    pub name: String,
    pub private: bool,
    pub member_count: Option<i64>,
    pub watched: bool,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackChannels {
    pub channels: Vec<ChannelRow>,
    /// "n channels · ≈ m requests/min at your interval".
    pub budget_line: String,
    /// The bucket cannot serve every source at its interval (warn, refuse nothing).
    pub over_budget: bool,
}

fn budget_line(c: &Connectors, account_id: &str) -> ApiResult<(String, bool)> {
    let s = c.db().read(super::ConnectorSettings::load)?;
    let srcs = c.db().read(|conn| accounts::sources(conn, account_id))?;
    let mut per_min = 0.0f64;
    let mut channels = 0;
    for src in srcs.iter().filter(|x| x.enabled) {
        let Some(k) = SourceKind::parse(&src.kind) else {
            continue;
        };
        if k == SourceKind::SlackChannel {
            channels += 1;
        }
        let i = src
            .poll_seconds
            .unwrap_or_else(|| scheduler::base_interval(k, &s))
            .max(1);
        per_min += 60.0 / i as f64;
    }
    let line = format!(
        "{channels} channel{} · ≈ {} requests/min at your interval",
        if channels == 1 { "" } else { "s" },
        per_min.round() as i64
    );
    Ok((line, per_min > s.slack_budget_per_minute as f64))
}

/// `slack_channels_list { account_id }`: the picker list (cached 10 min).
pub fn slack_channels_list(c: &Connectors, args: SlackChannelsArgs) -> ApiResult<SlackChannels> {
    let list = c
        .slack_channels(&args.account_id, args.refresh)
        .map_err(connector_error)?;
    let srcs = c
        .db()
        .read(|conn| accounts::sources(conn, &args.account_id))?;
    let channels = list
        .into_iter()
        .map(|ch| {
            let s = srcs
                .iter()
                .find(|s| s.kind == "slack_channel" && s.external_id == ch.external_id);
            ChannelRow {
                channel_id: ch.external_id,
                name: ch.name,
                private: ch.private,
                member_count: ch.member_count,
                watched: s.is_some_and(|s| s.enabled),
                source_id: s.map(|s| s.id.clone()),
            }
        })
        .collect();
    let (line, over) = budget_line(c, &args.account_id)?;
    Ok(SlackChannels {
        channels,
        budget_line: line,
        over_budget: over,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackChannelsSetArgs {
    pub account_id: String,
    /// The full watched set (channel ids); others are unwatched (`enabled = 0`,
    /// the row and its cursor stay so re-watching resumes).
    pub channel_ids: Vec<String>,
}

/// `slack_channels_set { account_id, channel_ids }`.
pub fn slack_channels_set(c: &Connectors, args: SlackChannelsSetArgs) -> ApiResult<Vec<SourceRow>> {
    let known = c
        .slack_channels(&args.account_id, false)
        .unwrap_or_default();
    let now = c.now();
    let back_days = c
        .db()
        .read(super::ConnectorSettings::load)?
        .slack_initial_sync_days;
    let a = args.clone();
    let rows = c.db().write(move |conn| {
        repo::tx(conn, |tx| {
            let now_s = format_timestamp(now);
            tx.execute(
                "UPDATE account_sources SET enabled = 0, updated_at = ?2
                  WHERE account_id = ?1 AND kind = 'slack_channel'",
                rusqlite::params![a.account_id, now_s],
            )?;
            for ch in &a.channel_ids {
                let name = known
                    .iter()
                    .find(|k| &k.external_id == ch)
                    .map(|k| k.name.clone())
                    .unwrap_or_else(|| format!("#{ch}"));
                let cursor = format!("{}.000000", (now - time::Duration::days(back_days.max(1))).unix_timestamp());
                tx.execute(
                    "INSERT INTO account_sources (id, account_id, kind, external_id, name, enabled, vip,
                            poll_seconds, cursor, last_polled_at, created_at, updated_at)
                     VALUES (?1, ?2, 'slack_channel', ?3, ?4, 1, 0, NULL, ?5, NULL, ?6, ?6)
                     ON CONFLICT(account_id, kind, external_id) DO UPDATE SET enabled = 1, updated_at = excluded.updated_at",
                    rusqlite::params![repo::new_id(), a.account_id, ch, name, cursor, now_s],
                )?;
            }
            accounts::sources(tx, &a.account_id)
        })
    })?;
    c.wake();
    c.changed(&[Event::Accounts]);
    Ok(rows.into_iter().map(source_row).collect())
}

// -------------------------------------------------------------------- inbox

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxListArgs {
    #[serde(default)]
    pub filter: InboxFilter,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// `inbox_list { filter, cursor?, limit? }`.
pub fn inbox_list(c: &Connectors, args: InboxListArgs) -> ApiResult<InboxListResult> {
    let ck = clock(c);
    let ds = day_start(c)?;
    Ok(c.db().read(|conn| {
        route::inbox_list(
            conn,
            &ck,
            &ds,
            &args.filter,
            args.cursor.as_deref(),
            args.limit,
        )
    })?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadGetArgs {
    pub thread_id: String,
}

/// `thread_get { thread_id }` (03 §4.5). `thread_id` may also be a message
/// id (the UI's `inbox/message/<id>` route): the message's thread answers.
pub fn thread_get(c: &Connectors, args: ThreadGetArgs) -> ApiResult<ThreadDetail> {
    let ck = clock(c);
    c.db()
        .read(|conn| {
            if let Some(d) = route::thread_detail(conn, &ck, &args.thread_id)? {
                return Ok(Some(d));
            }
            use rusqlite::OptionalExtension;
            let thread: Option<Option<String>> = conn
                .query_row(
                    "SELECT thread_id FROM messages WHERE id = ?1",
                    [&args.thread_id],
                    |r| r.get(0),
                )
                .optional()?;
            match thread.flatten() {
                Some(t) => route::thread_detail(conn, &ck, &t),
                None => Ok(None),
            }
        })?
        .ok_or_else(|| err("not_found", "the thread is gone"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkReadArgs {
    /// A message id, or a thread id with `kind = "thread"`.
    pub id: String,
    #[serde(default)]
    pub kind: Option<String>,
    /// `false` = mark unread.
    #[serde(default = "yes")]
    pub read: bool,
}

fn triage(c: &Connectors, kind: &str, id: &str, action: TriageAction) -> ApiResult<TriageResult> {
    let now = c.now();
    let device = c.db().device_id().to_string();
    let (k, i) = (kind.to_string(), id.to_string());
    let r = c
        .db()
        .write(move |conn| repo::tx(conn, |tx| store::triage(tx, &device, &k, &i, action, now)))?;
    if matches!(action, TriageAction::MarkRead | TriageAction::Archive) {
        c.withdraw(kind, id);
    }
    c.changed(&[Event::Inbox, Event::Pill]);
    Ok(r)
}

/// `message_mark_read { id, kind?, read? }`: local first, queued write-back
/// where the scope allows (06 §2.5).
pub fn message_mark_read(c: &Connectors, args: MarkReadArgs) -> ApiResult<TriageResult> {
    let kind = args.kind.as_deref().unwrap_or("message");
    let action = if args.read {
        TriageAction::MarkRead
    } else {
        TriageAction::MarkUnread
    };
    triage(c, kind, &args.id, action)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveArgs {
    /// A thread id, or a message id with `kind = "message"`.
    pub id: String,
    #[serde(default)]
    pub kind: Option<String>,
    /// `false` = unarchive.
    #[serde(default = "yes")]
    pub archived: bool,
}

/// `thread_archive { id, kind?, archived? }`: archive cancels a snooze.
pub fn thread_archive(c: &Connectors, args: ArchiveArgs) -> ApiResult<TriageResult> {
    let kind = args.kind.as_deref().unwrap_or("thread");
    let action = if args.archived {
        TriageAction::Archive
    } else {
        TriageAction::Unarchive
    };
    triage(c, kind, &args.id, action)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnoozeArgs {
    pub id: String,
    /// `thread` (default, whole thread) | `message` | `calendar_event` | `note` | `session`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub preset: Option<Preset>,
    #[serde(default)]
    pub until: Option<String>,
    #[serde(default)]
    pub wake_on_reply: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnoozeResult {
    pub until: String,
    pub preset: Option<String>,
    /// Rows created (the target plus its twins).
    pub targets: usize,
}

/// `thread_snooze { id, preset | until }` (07 §6; app-local).
pub fn thread_snooze(c: &Connectors, args: SnoozeArgs) -> ApiResult<SnoozeResult> {
    let now = c.now();
    let tz = c.tz();
    let device = c.db().device_id().to_string();
    let req = SnoozeRequest {
        target_kind: args.kind.clone().unwrap_or_else(|| "thread".into()),
        target_id: args.id.clone(),
        preset: args.preset,
        until: args.until.clone(),
        wake_on_reply: args.wake_on_reply,
    };
    let rows = c.db().write(move |conn| {
        repo::tx(conn, |tx| {
            let wake =
                settings::get_bool_in(tx, "notifications.snooze_wake_on_reply")?.unwrap_or(true);
            with_tz(tx, tz, |z| snooze::snooze(tx, &device, &req, now, z, wake))
        })
    })?;
    c.withdraw(args.kind.as_deref().unwrap_or("thread"), &args.id);
    c.changed(&[Event::Inbox, Event::Pill, Event::Agenda]);
    let first = rows
        .first()
        .ok_or_else(|| err("internal", "nothing was snoozed"))?;
    Ok(SnoozeResult {
        until: first.until.clone(),
        preset: first.preset.clone(),
        targets: rows.len(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsnoozeArgs {
    pub id: String,
    #[serde(default)]
    pub kind: Option<String>,
}

/// `thread_unsnooze { id, kind? }` (Cancel snooze).
pub fn thread_unsnooze(c: &Connectors, args: UnsnoozeArgs) -> ApiResult<usize> {
    let now = c.now();
    let device = c.db().device_id().to_string();
    let kind = args.kind.clone().unwrap_or_else(|| "thread".into());
    let id = args.id.clone();
    let n = c
        .db()
        .write(move |conn| repo::tx(conn, |tx| snooze::cancel(tx, &device, &kind, &id, now)))?;
    c.changed(&[Event::Inbox, Event::Pill, Event::Agenda]);
    Ok(n)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenInSource {
    /// The deep link (`slack://…` for Slack, the Gmail web URL, `html_link`).
    pub url: String,
    /// The web fallback when no handler is registered (Slack).
    pub fallback_url: Option<String>,
}

/// provider, external id, web URL, team id, account cursor, thread external id.
type SourceRow6 = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// `open_in_source { id } → url` for a message, a thread or a calendar event.
pub fn open_in_source(c: &Connectors, args: IdArgs) -> ApiResult<OpenInSource> {
    let id = args.id.clone();
    let found: Option<SourceRow6> =
        c.db().read(move |conn| {
            use rusqlite::OptionalExtension;
            let m = conn
                .query_row(
                    "SELECT m.provider, m.external_id, m.source_url, a.team_id, a.cursor_json,
                            (SELECT t.external_id FROM threads t WHERE t.id = m.thread_id)
                       FROM messages m JOIN accounts a ON a.id = m.account_id
                      WHERE m.id = ?1
                         OR m.id = (SELECT id FROM messages WHERE thread_id = ?1 ORDER BY received_at DESC LIMIT 1)
                      LIMIT 1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
                )
                .optional()?;
            Ok(m)
        })?;
    if let Some((prov, ext, web, team, cursor, thread_ext)) = found {
        if prov == "slack" {
            let (channel, ts) = ext.split_once(':').unwrap_or(("", ""));
            let thread_ts = thread_ext
                .as_deref()
                .and_then(|t| t.split_once(':'))
                .map(|(_, ts)| ts.to_string());
            let team_url = cursor
                .and_then(|c| serde_json::from_str::<Value>(&c).ok())
                .and_then(|v| {
                    v.get("team_url")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
            let (deep, web2) = super::slack::links(
                team.as_deref().unwrap_or(""),
                team_url.as_deref(),
                channel,
                ts,
                thread_ts.as_deref(),
            );
            return Ok(OpenInSource {
                url: deep,
                fallback_url: Some(if web.is_empty() { web2 } else { web }),
            });
        }
        return Ok(OpenInSource {
            url: web,
            fallback_url: None,
        });
    }
    let ev = c.db().read(|conn| calendar::event(conn, &args.id))?;
    match ev.and_then(|e| e.html_link) {
        Some(l) => Ok(OpenInSource {
            url: l,
            fallback_url: None,
        }),
        None => Err(err("not_found", "nothing to open")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetTierArgs {
    pub id: String,
    pub tier: Tier,
}

/// `message_set_tier { id, tier }`: the badge menu override (07 §3.5, AW-36).
pub fn message_set_tier(c: &Connectors, args: SetTierArgs) -> ApiResult<usize> {
    let now = format_timestamp(c.now());
    let n = c.db().write(move |conn| {
        repo::tx(conn, |tx| {
            urgency::set_override(tx, &args.id, args.tier, &now)
        })
    })?;
    c.changed(&[Event::Inbox, Event::Pill]);
    Ok(n)
}

/// `message_body { id }`: the body on demand (06 §2.1 `fetch_body`).
pub fn message_body(c: &Connectors, args: IdArgs) -> ApiResult<super::FetchedBody> {
    c.fetch_body(&args.id).map_err(connector_error)
}

/// `keyword_rules_list` (Settings → Urgency, 07 §3.3): the stored
/// rules; the first read of a never-written key seeds the three premade
/// rules, disabled, so the page has examples to switch on.
pub fn keyword_rules_list(c: &Connectors) -> ApiResult<Vec<urgency::rules::KeywordRule>> {
    const KEY: &str = "urgency.keyword_rules";
    let (rev, rules) = c.db().read(|conn| {
        Ok((
            settings::rev_in(conn, KEY)?,
            settings::get_as_in::<Vec<urgency::rules::KeywordRule>>(conn, KEY).unwrap_or_default(),
        ))
    })?;
    if rules.is_empty() && rev.is_none_or(|r| r <= 1) {
        let seeded = urgency::rules::premade();
        settings::set(c.db(), KEY, &serde_json::to_value(&seeded)?)?;
        return Ok(seeded);
    }
    Ok(rules)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeywordRulesSetArgs {
    pub rules: Vec<urgency::rules::KeywordRule>,
}

/// `keyword_rules_set { rules }`: validated (name ≤ 40, regex ≤ 512 and
/// compiling, horizon 1–30), new rules get an id, positions follow the
/// list order. Applies to new messages; Re-score re-applies the last 24 h.
pub fn keyword_rules_set(
    c: &Connectors,
    args: KeywordRulesSetArgs,
) -> ApiResult<Vec<urgency::rules::KeywordRule>> {
    let mut rules = args.rules;
    for (i, r) in rules.iter_mut().enumerate() {
        r.name = r.name.trim().to_string();
        if r.id.trim().is_empty() {
            r.id = repo::new_id();
        }
        r.position = i as i64;
        urgency::rules::validate(r).map_err(|m| err("invalid", format!("{}: {m}", r.name)))?;
    }
    settings::set(
        c.db(),
        "urgency.keyword_rules",
        &serde_json::to_value(&rules)?,
    )?;
    Ok(rules)
}

/// `urgency_rescore`: Settings → Urgency → Re-score (07 §3.4.8).
pub fn urgency_rescore(c: &Connectors) -> ApiResult<usize> {
    let (now, tz) = (c.now(), c.tz());
    let n = c.db().write(move |conn| {
        repo::tx(conn, |tx| {
            let ctx = urgency::Ctx::load(tx, now, tz)?;
            urgency::rescore(tx, &ctx)
        })
    })?;
    c.kick_scoring();
    c.changed(&[Event::Inbox, Event::Pill]);
    Ok(n)
}

/// `inbox_refresh`: poll every source now.
pub fn inbox_refresh(c: &Connectors) -> ApiResult<PollReport> {
    c.wake();
    Ok(c.poll_once()?)
}

// --------------------------------------------------------------------- important people

/// `vips_list`.
pub fn vips_list(c: &Connectors) -> ApiResult<Vec<VipRow>> {
    Ok(c.db().read(vips::list)?)
}

/// `vip_add { contact_id | message_id | email | slack_user, vip_note? }`.
pub fn vip_add(c: &Connectors, args: VipAdd) -> ApiResult<VipChange> {
    let (now, tz) = (c.now(), c.tz());
    let device = c.db().device_id().to_string();
    let r = c.db().write(move |conn| {
        repo::tx(conn, |tx| {
            let ctx = urgency::Ctx::load(tx, now, tz)?;
            let days = settings::retention_in(tx)?.messages_days;
            vips::add(tx, &device, &args, &ctx, days)
        })
    })?;
    c.changed(&[Event::Inbox, Event::Pill]);
    Ok(r)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactArgs {
    pub contact_id: String,
}

/// `vip_remove { contact_id }`.
pub fn vip_remove(c: &Connectors, args: ContactArgs) -> ApiResult<VipChange> {
    let (now, tz) = (c.now(), c.tz());
    let device = c.db().device_id().to_string();
    let r = c.db().write(move |conn| {
        repo::tx(conn, |tx| {
            let ctx = urgency::Ctx::load(tx, now, tz)?;
            let days = settings::retention_in(tx)?.messages_days;
            vips::remove(tx, &device, &args.contact_id, &ctx, days)
        })
    })?;
    c.kick_scoring();
    c.changed(&[Event::Inbox, Event::Pill]);
    Ok(r)
}

// ----------------------------------------------------------------- calendar

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgendaArgs {
    pub from: String,
    pub to: String,
}

/// `agenda_list { from, to }`.
pub fn agenda_list(c: &Connectors, args: AgendaArgs) -> ApiResult<Vec<EventRow>> {
    Ok(c.db()
        .read(|conn| calendar::agenda_list(conn, &args.from, &args.to))?)
}

/// `event_join { id } → url`.
pub fn event_join(c: &Connectors, args: IdArgs) -> ApiResult<String> {
    Ok(c.db().read(|conn| calendar::join_url(conn, &args.id))?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteIdResult {
    pub note_id: String,
}

/// `event_prepare_notes { id } → note id` (calls `notes::api::note_prepare`
/// unless the event already has its note).
pub fn event_prepare_notes(c: &Connectors, notes: &Notes, args: IdArgs) -> ApiResult<NoteIdResult> {
    let note_id = calendar::prepare_notes(notes, &args.id)?;
    c.changed(&[Event::Agenda]);
    Ok(NoteIdResult { note_id })
}

// -------------------------------------------------------------------- links

/// `event_links { id }`: the event panel's linked sessions, notes and
/// messages (03 §6.6; `crate::links`), found on open.
pub fn event_links(c: &Connectors, args: IdArgs) -> ApiResult<Vec<ItemLink>> {
    c.db()
        .read(|conn| links::event_links(conn, &args.id))?
        .ok_or_else(|| err("not_found", "the event is no longer in the calendar"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemLinkArgs {
    /// `thread` | `calendar_event`.
    pub kind: String,
    /// The thread (or one of its messages) or the event.
    pub id: String,
    /// `session` | `note` | `message`.
    pub to_kind: String,
    pub to_id: String,
}

/// The anchor of a link command; a message id stands for its thread.
fn anchor_of(
    conn: &rusqlite::Connection,
    kind: &str,
    id: &str,
) -> DbResult<Option<(bool, String)>> {
    use rusqlite::OptionalExtension;
    match kind {
        "thread" => {
            let t: Option<String> = conn
                .query_row("SELECT id FROM threads WHERE id = ?1", [id], |r| r.get(0))
                .optional()?;
            if t.is_some() {
                return Ok(t.map(|t| (true, t)));
            }
            let m: Option<Option<String>> = conn
                .query_row("SELECT thread_id FROM messages WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            Ok(m.flatten().map(|t| (true, t)))
        }
        "calendar_event" | "event" => Ok(Some((false, id.to_string()))),
        _ => Ok(None),
    }
}

fn link_change(c: &Connectors, args: ItemLinkArgs, add: bool) -> ApiResult<Vec<ItemLink>> {
    if !links::TARGET_KINDS.contains(&args.to_kind.as_str()) {
        return Err(err(
            "invalid",
            format!("{:?} is not a link target", args.to_kind),
        ));
    }
    let device = c.db().device_id().to_string();
    let out = c.db().write(move |conn| {
        let Some((is_thread, id)) = anchor_of(conn, &args.kind, &args.id)? else {
            return Err(crate::db::DbError::Invalid {
                what: "link",
                detail: format!("{:?} is not a thread or an event", args.kind),
            });
        };
        let anchor = if is_thread {
            links::Anchor::Thread(&id)
        } else {
            links::Anchor::Event(&id)
        };
        repo::tx(conn, |tx| {
            if add {
                links::add(tx, &device, anchor, &args.to_kind, &args.to_id)?;
            } else {
                links::remove(tx, &device, anchor, &args.to_kind, &args.to_id)?;
            }
            links::links_of(tx, anchor)
        })
    })?;
    c.changed(&[Event::Notes]);
    Ok(out)
}

/// `item_link_add { kind, id, to_kind, to_id }`: link by hand (a
/// `references` row, `created_by = 'user'`); returns the anchor's links.
pub fn item_link_add(c: &Connectors, args: ItemLinkArgs) -> ApiResult<Vec<ItemLink>> {
    link_change(c, args, true)
}

/// `item_link_remove { kind, id, to_kind, to_id }`: tombstone the explicit
/// rows and remember the dismissal; returns the anchor's links.
pub fn item_link_remove(c: &Connectors, args: ItemLinkArgs) -> ApiResult<Vec<ItemLink>> {
    link_change(c, args, false)
}

// ------------------------------------------------------------------- alerts

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertsRecentArgs {
    #[serde(default)]
    pub limit: Option<i64>,
}

/// One `alert_log` row with its rule's name (Diagnostics / Settings → Alerts).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertLogRow {
    pub id: String,
    pub rule_id: String,
    pub rule_name: Option<String>,
    pub event: String,
    pub item_key: String,
    pub channels: Vec<String>,
    pub suppressed_reason: Option<String>,
    pub fired_at: Option<String>,
    pub created_at: String,
}

/// `alerts_recent { limit? }`: the newest `alert_log` rows (7-day class L).
pub fn alerts_recent(c: &Connectors, args: AlertsRecentArgs) -> ApiResult<Vec<AlertLogRow>> {
    let limit = args.limit.unwrap_or(50).clamp(1, 500);
    Ok(c.db().read(move |conn| {
        let mut st = conn.prepare(
            "SELECT l.id, l.rule_id, r.name, l.event, l.item_key, l.channels_json, l.suppressed_reason,
                    l.fired_at, l.created_at
               FROM alert_log l LEFT JOIN alert_rules r ON r.id = l.rule_id
              ORDER BY l.created_at DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit], |r| {
            let ch: String = r.get(5)?;
            Ok(AlertLogRow {
                id: r.get(0)?,
                rule_id: r.get(1)?,
                rule_name: r.get(2)?,
                event: r.get(3)?,
                item_key: r.get(4)?,
                channels: serde_json::from_str(&ch).unwrap_or_default(),
                suppressed_reason: r.get(6)?,
                fired_at: r.get(7)?,
                created_at: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertDismissArgs {
    /// The notification identifier (= item key), or an item to resolve.
    #[serde(default)]
    pub item_key: Option<String>,
    #[serde(default)]
    pub target_kind: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
}

/// `alert_dismiss`: withdraw the OS notification(s) of an item. Nothing is
/// re-alerted afterwards (the `alert_log` row stays).
pub fn alert_dismiss(c: &Connectors, args: AlertDismissArgs) -> ApiResult<Vec<String>> {
    let mut ids = Vec::new();
    if let Some(k) = args.item_key {
        ids.push(k);
    }
    if let (Some(k), Some(i)) = (args.target_kind.as_deref(), args.target_id.as_deref()) {
        ids.extend(c.db().read(|conn| alerts::withdraw_keys(conn, k, i))?);
    }
    ids.dedup();
    if !ids.is_empty() {
        c.send_alert(AlertCommand::Withdraw {
            identifiers: ids.clone(),
        });
    }
    Ok(ids)
}

/// One alert rule as Settings → Alerts lists it (07 §8.2): the trigger
/// split into `event` and `filters`, the channels as a list, and `edited`
/// when a premade rule's definition differs from its seed
/// ([`alerts::edited`]: compared by value, so a Reset clears it; custom
/// rules are never edited).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRuleRow {
    pub id: String,
    pub name: String,
    /// `premade` | `custom`.
    pub kind: String,
    pub enabled: bool,
    pub event: String,
    pub filters: alerts::Filters,
    /// ⊆ `overlay`, `os_notification`, `sound`; `overlay` always.
    pub channels: Vec<String>,
    pub sound: Option<String>,
    pub respect_dnd: bool,
    pub quiet_hours: bool,
    pub cooldown_seconds: i64,
    pub position: i64,
    pub last_fired_at: Option<String>,
    pub edited: bool,
}

fn rule_row(r: &repo::AlertRule) -> AlertRuleRow {
    let t = alerts::parse_trigger(&r.trigger_json).ok();
    AlertRuleRow {
        id: r.id.clone(),
        name: r.name.clone(),
        kind: r.kind.clone(),
        enabled: r.enabled,
        event: t
            .as_ref()
            .map(|t| t.event.clone())
            .unwrap_or_else(|| "unknown".into()),
        filters: t.map(|t| t.filters).unwrap_or_default(),
        channels: serde_json::from_str(&r.channels_json).unwrap_or_else(|_| vec!["overlay".into()]),
        sound: r.sound.clone(),
        respect_dnd: r.respect_dnd,
        quiet_hours: r.quiet_hours,
        cooldown_seconds: r.cooldown_seconds,
        position: r.position,
        last_fired_at: r.last_fired_at.clone(),
        edited: alerts::edited(r),
    }
}

/// A rule name: trimmed, 1–60 characters.
fn rule_name(raw: &str) -> Result<String, ApiError> {
    let n = raw.trim();
    if n.is_empty() || n.chars().count() > 60 {
        return Err(err("invalid", "a rule name is 1–60 characters"));
    }
    Ok(n.to_string())
}

/// Another live rule already has this name (case-insensitive): premade
/// rules are matched by name at every start (ADR-0048), so names stay
/// unique.
fn name_taken(conn: &rusqlite::Connection, name: &str, self_id: &str) -> crate::db::Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM alert_rules
                         WHERE deleted_at IS NULL AND lower(name) = lower(?1) AND id <> ?2)",
        rusqlite::params![name, self_id],
        |r| r.get(0),
    )?)
}

/// The channels as stored: `overlay` first and always, no repeats.
fn rule_channels(ch: &[String]) -> Vec<String> {
    let mut out = vec!["overlay".to_string()];
    for c in ch {
        if !out.contains(c) {
            out.push(c.clone());
        }
    }
    out
}

/// A cue id; blank clears it.
fn rule_sound(snd: &str) -> Option<String> {
    Some(snd.trim().to_string()).filter(|s| !s.is_empty())
}

fn check_cooldown(cd: i64) -> Result<i64, ApiError> {
    if (0..=86_400).contains(&cd) {
        Ok(cd)
    } else {
        Err(err("invalid", "the cooldown is 0–86400 seconds"))
    }
}

/// `alert_rules_list`: the live rules by position (the premade ones are
/// upserted by name at every start, 07 §8.2).
pub fn alert_rules_list(c: &Connectors) -> ApiResult<Vec<AlertRuleRow>> {
    Ok(c.db().read(|conn| {
        Ok(
            repo::query::<repo::AlertRule>(
                conn,
                "deleted_at IS NULL ORDER BY position, name",
                &[],
            )?
            .iter()
            .map(rule_row)
            .collect(),
        )
    })?)
}

/// `alert_rule_set {id, enabled?, …}`: every field optional; `reset`
/// restores a premade rule's seed first (the row then reads `edited:
/// false`). Validated as the Settings editor validates (07 §8.2):
/// `overlay` stays, `respect_dnd = false` only for session rules, filters
/// of the event's schema ([`alerts::check_filters`]), a unique name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRuleSetArgs {
    pub id: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub channels: Option<Vec<String>>,
    /// A cue id; `""` clears it.
    #[serde(default)]
    pub sound: Option<String>,
    #[serde(default)]
    pub cooldown_seconds: Option<i64>,
    #[serde(default)]
    pub respect_dnd: Option<bool>,
    #[serde(default)]
    pub quiet_hours: Option<bool>,
    #[serde(default)]
    pub filters: Option<alerts::Filters>,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub reset: Option<bool>,
}

pub fn alert_rule_set(c: &Connectors, args: AlertRuleSetArgs) -> ApiResult<AlertRuleRow> {
    let device = c.db().device_id().to_string();
    c.db().write(move |conn| {
        let Some(mut r) = repo::get_live::<repo::AlertRule>(conn, &args.id)? else {
            return Ok(Err(err("not_found", "no such rule")));
        };
        if args.reset == Some(true) {
            let Some(seed) = alerts::premade_seed(&r) else {
                return Ok(Err(err("invalid", "only a premade rule can be reset")));
            };
            alerts::reset_to_seed(&mut r, seed);
        }
        if let Some(e) = args.enabled {
            r.enabled = e;
        }
        if let Some(n) = &args.name {
            let n = match rule_name(n) {
                Ok(n) => n,
                Err(e) => return Ok(Err(e)),
            };
            if n != r.name && name_taken(conn, &n, &r.id)? {
                return Ok(Err(err("invalid", format!("a rule named {n:?} exists"))));
            }
            r.name = n;
        }
        if let Some(ch) = &args.channels {
            r.channels_json = serde_json::to_string(&rule_channels(ch))?;
        }
        if let Some(snd) = &args.sound {
            r.sound = rule_sound(snd);
        }
        if let Some(cd) = args.cooldown_seconds {
            match check_cooldown(cd) {
                Ok(cd) => r.cooldown_seconds = cd,
                Err(e) => return Ok(Err(e)),
            }
        }
        if let Some(d) = args.respect_dnd {
            r.respect_dnd = d;
        }
        if let Some(q) = args.quiet_hours {
            r.quiet_hours = q;
        }
        if let Some(f) = &args.filters {
            let mut t = match alerts::parse_trigger(&r.trigger_json) {
                Ok(t) => t,
                Err(e) => return Ok(Err(err("invalid", e))),
            };
            t.filters = f.clone();
            r.trigger_json = serde_json::to_string(&t)?;
        }
        if let Some(p) = args.position {
            r.position = p;
        }
        if let Err(e) = alerts::validate_rule(&r) {
            return Ok(Err(err("invalid", e)));
        }
        repo::save(conn, &device, &mut r)?;
        Ok(Ok(rule_row(&r)))
    })?
}

/// `alert_rule_add {name, event, filters, channels, sound?, respect_dnd?,
/// quiet_hours?, cooldown_seconds?}`: a custom rule from the Alerts page's
/// event picker (03 §8.11), enabled and placed after the last rule.
/// Validated as `alert_rule_set` validates (07 §8.2): a known event,
/// filters of that event's schema, `overlay` kept, `respect_dnd = false`
/// only for `session.waiting` / `session.failed`, a cooldown of 0–86400 s,
/// a unique 1–60 character name. Refusals are `invalid`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRuleAddArgs {
    pub name: String,
    pub event: String,
    pub filters: alerts::Filters,
    /// ⊆ `overlay`, `os_notification`, `sound`; `overlay` is added.
    pub channels: Vec<String>,
    /// A cue id; absent or `""` = none.
    #[serde(default)]
    pub sound: Option<String>,
    /// Default `true`.
    #[serde(default)]
    pub respect_dnd: Option<bool>,
    /// Default `true`.
    #[serde(default)]
    pub quiet_hours: Option<bool>,
    /// Default 0.
    #[serde(default)]
    pub cooldown_seconds: Option<i64>,
}

pub fn alert_rule_add(c: &Connectors, args: AlertRuleAddArgs) -> ApiResult<AlertRuleRow> {
    let device = c.db().device_id().to_string();
    c.db().write(move |conn| {
        let name = match rule_name(&args.name) {
            Ok(n) => n,
            Err(e) => return Ok(Err(e)),
        };
        if name_taken(conn, &name, "")? {
            return Ok(Err(err("invalid", format!("a rule named {name:?} exists"))));
        }
        let cooldown_seconds = match check_cooldown(args.cooldown_seconds.unwrap_or(0)) {
            Ok(cd) => cd,
            Err(e) => return Ok(Err(e)),
        };
        let trigger = alerts::Trigger {
            event: args.event.trim().to_string(),
            filters: args.filters,
        };
        let position: i64 = conn.query_row(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM alert_rules WHERE deleted_at IS NULL",
            [],
            |r| r.get(0),
        )?;
        let mut r = repo::AlertRule {
            id: repo::new_id(),
            name,
            kind: "custom".into(),
            enabled: true,
            trigger_json: serde_json::to_string(&trigger)?,
            channels_json: serde_json::to_string(&rule_channels(&args.channels))?,
            sound: args.sound.as_deref().and_then(rule_sound),
            respect_dnd: args.respect_dnd.unwrap_or(true),
            quiet_hours: args.quiet_hours.unwrap_or(true),
            cooldown_seconds,
            position,
            ..Default::default()
        };
        if let Err(e) = alerts::validate_rule(&r) {
            return Ok(Err(err("invalid", e)));
        }
        repo::create(conn, &device, &mut r)?;
        Ok(Ok(rule_row(&r)))
    })?
}

/// `alert_rule_delete {id} → {deleted}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertRuleDeleted {
    pub deleted: bool,
}

/// `alert_rule_delete {id}`: tombstones a custom rule (its `alert_log`
/// rows stay). A premade rule refuses with `premade` (it can be turned
/// off or reset, not deleted); an unknown or deleted id is `not_found`.
pub fn alert_rule_delete(c: &Connectors, args: IdArgs) -> ApiResult<AlertRuleDeleted> {
    let device = c.db().device_id().to_string();
    c.db().write(move |conn| {
        let Some(r) = repo::get_live::<repo::AlertRule>(conn, &args.id)? else {
            return Ok(Err(err("not_found", "no such rule")));
        };
        if r.kind == "premade" {
            return Ok(Err(err(
                "premade",
                "premade rules can be turned off or reset, not deleted",
            )));
        }
        let deleted = repo::soft_delete::<repo::AlertRule>(conn, &device, &r.id)?;
        Ok(Ok(AlertRuleDeleted { deleted }))
    })?
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertTestArgs {
    pub rule_id: String,
}

/// `alert_rule_test { rule_id }`: Settings → Alerts → Test (07 §8.2): a
/// sample event through the pipeline; suppression reported, not applied;
/// nothing written; delivered to the rule's channels.
pub fn alert_rule_test(c: &Connectors, args: AlertTestArgs) -> ApiResult<AlertOutcome> {
    let rule = c
        .db()
        .read(|conn| repo::get_by_id::<repo::AlertRule>(conn, &args.rule_id))?
        .ok_or_else(|| err("not_found", "no such rule"))?;
    let t = alerts::parse_trigger(&rule.trigger_json).map_err(|e| err("invalid", e))?;
    let ev = sample_event(&t, c.now());
    let env = c.environment()?;
    let (now, tz) = (c.now(), c.tz());
    let mut out = c.db().read(move |conn| {
        with_tz(conn, tz, |z| {
            alerts::evaluate(conn, &ev, &env, now, z, true)
        })
    })?;
    out.rules.retain(|r| r.rule_id == args.rule_id);
    c.send_alert(AlertCommand::Fired(out.clone()));
    Ok(out)
}

fn sample_event(t: &alerts::Trigger, now: time::OffsetDateTime) -> alerts::AlertEvent {
    use alerts::*;
    let now_s = format_timestamp(now);
    match t.event.as_str() {
        "session.waiting" => AlertEvent::SessionWaiting(SessionWaiting {
            session_id: "test".into(),
            claude_session_id: format!("test-{now_s}"),
            display_name: "api-refactor".into(),
            repo_root: Some("/work/mode".into()),
            waiting_kind: t
                .filters
                .waiting_kind
                .as_ref()
                .and_then(|w| w.first().cloned())
                .unwrap_or_else(|| "permission".into()),
            tool_name: Some("Bash".into()),
            tool_use_id: Some(format!("test-{now_s}")),
            summary: Some("npm test".into()),
            state_since: now_s.clone(),
        }),
        "session.failed" => AlertEvent::SessionFailed(SessionFailed {
            session_id: "test".into(),
            claude_session_id: "test".into(),
            display_name: "api-refactor".into(),
            repo_root: Some("/work/mode".into()),
            reason: Some("the process exited with status 1".into()),
            ended_at: now_s.clone(),
        }),
        "session.finished" => AlertEvent::SessionFinished(SessionFinished {
            session_id: "test".into(),
            claude_session_id: "test".into(),
            display_name: "api-refactor".into(),
            repo_root: Some("/work/mode".into()),
            lines_added: 12,
            lines_removed: 3,
            files_changed: 2,
            duration_seconds: 300,
            ended_at: now_s.clone(),
            last_assistant_preview: Some("All tests pass.".into()),
        }),
        "calendar.starting" => AlertEvent::CalendarStarting(CalendarStarting {
            event_id: "test".into(),
            title: "Design review".into(),
            starts_at: format_timestamp(
                now + time::Duration::minutes(t.filters.minutes_before.unwrap_or(5)),
            ),
            join_url: Some("https://meet.google.com/abc-defg-hij".into()),
            conference: Some("meet".into()),
            location: None,
            account_id: "test".into(),
            calendar_id: "primary".into(),
            external_id: format!("test-{now_s}"),
            ical_uid: None,
            attendee_count: 4,
            minutes_before: t.filters.minutes_before.unwrap_or(5),
        }),
        "note.due" => AlertEvent::NoteDue(NoteDue {
            date: now_s[..10].to_string(),
            notes: vec![DueNote {
                id: "test".into(),
                title: "Send the contract".into(),
                urgent: true,
                when_date: now_s[..10].to_string(),
            }],
        }),
        "automation.failed" => AlertEvent::AutomationFailed(AutomationFailed {
            automation_id: t
                .filters
                .automation_id
                .clone()
                .unwrap_or_else(|| "test".into()),
            automation_name: "Morning brief".into(),
            run_id: format!("test-{now_s}"),
            error: "claude not found".into(),
        }),
        "usage.threshold" => AlertEvent::UsageThreshold(UsageThreshold {
            source: "claude_api".into(),
            percent: t.filters.percent.unwrap_or(80),
            used: 1_600_000,
            limit: 2_000_000,
            period: format!("test-{now_s}"),
        }),
        _ => AlertEvent::MessageReceived(MessageReceived {
            message_id: "test".into(),
            thread_id: None,
            dedup_key: format!("test:{now_s}"),
            account_id: "test".into(),
            provider: "google".into(),
            kind: "email".into(),
            sender_contact_id: None,
            sender_name: "Ana Ruiz".into(),
            channel_name: None,
            subject: Some("Contract v3 — need your sign-off".into()),
            snippet: "Can you sign off by Friday?".into(),
            received_at: now_s.clone(),
            urgency_tier: t
                .filters
                .tier
                .as_ref()
                .and_then(|x| x.first().cloned())
                .unwrap_or_else(|| "vip".into()),
            urgency_why: Some("Important person: Ana Ruiz (your manager)".into()),
            resurfaced: false,
        }),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuietSetArgs {
    /// "Pause alerts for 1 h": minutes from now (0 = resume).
    #[serde(default)]
    pub pause_minutes: Option<i64>,
    /// `notifications.quiet_hours`.
    #[serde(default)]
    pub quiet_hours: Option<QuietHours>,
    /// From the app's overlay controller (02 §6.6).
    #[serde(default)]
    pub presentation: Option<bool>,
    /// Detected Focus / DND from the app (Windows), `null` = unknown.
    #[serde(default)]
    pub focus: Option<Option<bool>>,
    #[serde(default)]
    pub screen_locked: Option<bool>,
    #[serde(default)]
    pub output_muted: Option<bool>,
    #[serde(default)]
    pub os_permission: Option<alerts::OsPermission>,
}

/// `quiet_set`: the manual pause, quiet hours, and the OS facts the app
/// sees (presentation, Focus, lock, mute, permission). Returns the merged
/// `quiet.changed` payload and pushes it.
pub fn quiet_set(c: &Connectors, args: QuietSetArgs) -> ApiResult<QuietChanged> {
    if let Some(m) = args.pause_minutes {
        let v = if m > 0 {
            Value::String(format_timestamp(c.now() + time::Duration::minutes(m)))
        } else {
            Value::Null
        };
        settings::set(c.db(), "notifications.pause_until", &v)?;
    }
    if let Some(q) = &args.quiet_hours {
        if alerts::parse_hm(&q.from).is_none() || alerts::parse_hm(&q.to).is_none() {
            return Err(err("invalid", "quiet hours are HH:MM"));
        }
        settings::set(
            c.db(),
            "notifications.quiet_hours",
            &serde_json::to_value(q)?,
        )?;
    }
    let mut os: OsState = c.os_state();
    if let Some(p) = args.presentation {
        os.presentation = p;
    }
    if let Some(f) = args.focus {
        os.focus = f;
    }
    if let Some(l) = args.screen_locked {
        os.screen_locked = l;
    }
    if let Some(m) = args.output_muted {
        os.output_muted = m;
    }
    if let Some(p) = args.os_permission {
        os.os_permission = p;
    }
    c.set_os_state(os);
    c.changed(&[Event::Quiet]);
    Ok(c.quiet_state()?)
}

// ----------------------------------------------------------------------- AI

/// One row of Settings → AI's backend list (06 §10.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendRow {
    pub backend: BackendKind,
    pub label: String,
    pub selected: bool,
    pub local: bool,
    /// Claude API: the key is stored (never the key itself).
    pub key_stored: Option<bool>,
    pub key_last4: Option<String>,
    pub url: Option<String>,
    pub address: Option<ai::AddressClass>,
}

/// `ai_backends_list`.
pub fn ai_backends_list(c: &Connectors) -> ApiResult<Vec<BackendRow>> {
    let s = c.db().read(AiSettings::load)?;
    let key = c.keyring().status(names::AI_ANTHROPIC_API_KEY);
    Ok(BackendKind::ALL
        .iter()
        .map(|b| BackendRow {
            backend: *b,
            label: b.label().into(),
            selected: s.backend == *b,
            local: b.is_local(),
            key_stored: (!b.is_local()).then_some(key.stored),
            key_last4: if b.is_local() {
                None
            } else {
                key.last4.clone()
            },
            url: b.is_local().then(|| {
                if s.backend == *b {
                    s.local_url.clone()
                } else {
                    b.default_url().to_string()
                }
            }),
            address: b.is_local().then(|| {
                ai::address_class(if s.backend == *b {
                    &s.local_url
                } else {
                    b.default_url()
                })
            }),
        })
        .collect())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiBackendSetArgs {
    #[serde(default)]
    pub backend: Option<BackendKind>,
    #[serde(default)]
    pub local_url: Option<String>,
    #[serde(default)]
    pub local_only: Option<bool>,
    #[serde(default)]
    pub model_scoring: Option<String>,
    #[serde(default)]
    pub model_summary: Option<String>,
    #[serde(default)]
    pub model_chat: Option<String>,
    #[serde(default)]
    pub local_model_scoring: Option<String>,
    #[serde(default)]
    pub local_model_summary: Option<String>,
    #[serde(default)]
    pub local_model_chat: Option<String>,
}

/// `ai_backend_set`: choose the backend, the local URL (address policy of
/// 06 §10.3: loopback or private only), the local-only switch and models.
/// Flipping the switch re-queues scoring at once (§10.4).
pub fn ai_backend_set(c: &Connectors, args: AiBackendSetArgs) -> ApiResult<AiHealth> {
    let db = c.db();
    if let Some(u) = &args.local_url {
        match ai::address_class(u) {
            ai::AddressClass::Loopback | ai::AddressClass::Private => {}
            _ => {
                return Err(err(
                    "invalid",
                    "a local backend must be on this computer or your private network",
                ));
            }
        }
        settings::set(
            db,
            "ai.local_url",
            &Value::String(u.trim_end_matches('/').into()),
        )?;
    } else if let Some(b) = args.backend
        && b.is_local()
    {
        let cur = db.read(AiSettings::load)?;
        if cur.backend != b && cur.local_url == cur.local_kind().default_url() {
            settings::set(db, "ai.local_url", &Value::String(b.default_url().into()))?;
        }
    }
    if let Some(b) = args.backend {
        settings::set(db, "ai.backend", &Value::String(b.as_str().into()))?;
    }
    if let Some(l) = args.local_only {
        settings::set(db, "ai.local_only", &Value::Bool(l))?;
        let now = format_timestamp(c.now());
        db.write(move |conn| crate::urgency::scoring::requeue_local_only(conn, &now))?;
    }
    for (k, v) in [
        ("ai.model_scoring", &args.model_scoring),
        ("ai.model_summary", &args.model_summary),
        ("ai.model_chat", &args.model_chat),
        ("ai.local_model_scoring", &args.local_model_scoring),
        ("ai.local_model_summary", &args.local_model_summary),
        ("ai.local_model_chat", &args.local_model_chat),
    ] {
        if let Some(m) = v {
            settings::set(db, k, &Value::String(m.clone()))?;
        }
    }
    c.kick_scoring();
    ai_health(c, AiHealthArgs { probe: false })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiKeySetArgs {
    /// `claude_api` (the Anthropic key) or `ollama` / `lm_studio` (the optional bearer).
    pub backend: BackendKind,
    /// `null` removes the entry.
    pub key: Option<String>,
}

/// `ai_key_set { backend, key }`: stored in the keyring; only "stored" and
/// the last four characters come back, never the key.
pub fn ai_key_set(c: &Connectors, args: AiKeySetArgs) -> ApiResult<SecretStatus> {
    let entry = if args.backend.is_local() {
        names::AI_LOCAL_TOKEN
    } else {
        names::AI_ANTHROPIC_API_KEY
    };
    match args.key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => c
            .keyring()
            .set(entry, k)
            .map_err(|e| err("keyring", e.to_string()))?,
        None => {
            c.keyring()
                .delete(entry)
                .map_err(|e| err("keyring", e.to_string()))?;
        }
    }
    if entry == names::AI_ANTHROPIC_API_KEY {
        let now = format_timestamp(c.now());
        c.db()
            .write(move |conn| crate::urgency::scoring::requeue_no_key(conn, &now))?;
        c.kick_scoring();
    }
    Ok(c.keyring().status(entry))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiHealthArgs {
    /// Also call `GET /v1/models` on the Claude API (no tokens billed).
    #[serde(default)]
    pub probe: bool,
}

/// `ai_health` (06 §10.7).
pub fn ai_health(c: &Connectors, args: AiHealthArgs) -> ApiResult<AiHealth> {
    let cfg = c.db().read(AiConfig::load)?;
    let (now, tz) = (c.now(), c.tz());
    let b = c.db().read(|conn| {
        with_tz(conn, tz, |z| {
            ai::budget(conn, now, z, cfg.settings.budget_tokens_month)
        })
    })?;
    let h = ai::health(&cfg, c.keyring(), c.ai_factory().as_ref(), &b, args.probe);
    if h.state == HealthState::Ready {
        c.kick_scoring();
    }
    Ok(h)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiTestResult {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

/// `ai_test_call`: the Settings → AI **Test call** (a minimal message,
/// `max_tokens` 8; recorded in `usage_samples` as a title-sized call).
pub fn ai_test_call(c: &Connectors) -> ApiResult<AiTestResult> {
    ai_test_call_with(c, AiTestArgs::default())
}

/// `ai_test_call { backend?, provider?, model? }` (the settings contract's argument).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiTestArgs {
    /// Test this backend instead of the selected one (a local backend at
    /// its stored URL when it is the configured local kind, else its
    /// default URL). Nothing is stored.
    #[serde(default)]
    pub backend: Option<BackendKind>,
    /// Test this provider of the registry (06 §10.1A) on `model` or its
    /// default model; without a route, so no backup answers for it.
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// [`ai_test_call`] against the selected backend, `args.backend` or `args.provider`.
pub fn ai_test_call_with(c: &Connectors, args: AiTestArgs) -> ApiResult<AiTestResult> {
    let mut cfg = c.db().read(AiConfig::load)?;
    let purpose = ai::Purpose::Title;
    if let Some(id) = args
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|i| !i.is_empty())
    {
        if cfg.provider(id).is_none() {
            return Err(err("not_found", "no such provider"));
        }
        // One route to this provider alone, for the test's purpose.
        cfg.routes = vec![Route {
            purpose: providers::route_purpose(purpose),
            provider_id: Some(id.to_string()),
            model: args.model.clone().filter(|m| !m.trim().is_empty()),
            fallbacks: Vec::new(),
        }];
        cfg.settings.local_only = false;
    } else {
        cfg.routes.clear();
        if let Some(b) = args.backend {
            if b.is_local() && cfg.settings.local_kind() != b {
                cfg.settings.local_url = b.default_url().to_string();
            }
            cfg.settings.local_only = b.is_local() && cfg.settings.local_only;
            cfg.settings.backend = b;
        }
    }
    let r = match ai::resolve_with(&cfg, c.keyring(), c.ai_factory().as_ref(), purpose) {
        Ok(r) => r,
        Err(e) => {
            return Ok(AiTestResult {
                ok: false,
                latency_ms: None,
                error: Some(e.to_string()),
            });
        }
    };
    let started = std::time::Instant::now();
    let req = ai::AiRequest {
        purpose,
        model: r.model.clone(),
        system: "Reply with the single word: ok".into(),
        user: "ping".into(),
        json_schema: None,
        max_tokens: 8,
        effort: ai::Effort::Low,
        timeout: cfg.settings.timeout(purpose),
    };
    match r.backend.complete(&req) {
        Ok(resp) => {
            if let Some(u) = resp.usage {
                let (kind, model, turn, now) = (
                    resp.source,
                    resp.model.clone(),
                    resp.request_id.clone(),
                    format_timestamp(c.now()),
                );
                c.db().write(move |conn| {
                    ai::record_usage(
                        conn,
                        kind,
                        ai::Purpose::Title,
                        &model,
                        &u,
                        turn.as_deref(),
                        &now,
                    )
                })?;
            }
            Ok(AiTestResult {
                ok: true,
                latency_ms: Some(started.elapsed().as_millis() as u64),
                error: None,
            })
        }
        Err(e) => Ok(AiTestResult {
            ok: false,
            latency_ms: Some(started.elapsed().as_millis() as u64),
            error: Some(e.to_string()),
        }),
    }
}

/// `ai_backends_status` (the settings contract's shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiBackendsStatus {
    pub backend: BackendKind,
    pub claude_api: Value,
    pub ollama: Value,
    pub lm_studio: Value,
    pub tokens_used_month: i64,
}

pub fn ai_backends_status(c: &Connectors) -> ApiResult<AiBackendsStatus> {
    let s = c.db().read(AiSettings::load)?;
    let (now, tz) = (c.now(), c.tz());
    let b = c.db().read(|conn| {
        with_tz(conn, tz, |z| {
            ai::budget(conn, now, z, s.budget_tokens_month)
        })
    })?;
    let local = |kind: BackendKind| -> Value {
        let url = if s.local_kind() == kind {
            s.local_url.clone()
        } else {
            kind.default_url().to_string()
        };
        let backend = c.ai_factory().local(kind, &url, None);
        match backend.list_models() {
            Ok(m) => serde_json::json!({"url": url, "reachable": true, "models": m, "error": null}),
            Err(e) => {
                serde_json::json!({"url": url, "reachable": false, "models": [], "error": e.to_string()})
            }
        }
    };
    Ok(AiBackendsStatus {
        backend: s.backend,
        claude_api: serde_json::json!({"key_stored": c.keyring().present(names::AI_ANTHROPIC_API_KEY)}),
        ollama: local(BackendKind::Ollama),
        lm_studio: local(BackendKind::LmStudio),
        tokens_used_month: b.used_month,
    })
}

// ---------------------------------------------------------- providers

/// One row of Settings → AI's provider list (06 §10.1A).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRow {
    #[serde(flatten)]
    pub provider: AiProvider,
    /// The address its calls go to ([`AiProvider::url`]).
    pub url: Option<String>,
    pub address: ai::AddressClass,
    /// A secret is stored (never the secret itself) and its last four characters.
    pub key_stored: bool,
    pub key_last4: Option<String>,
    /// Mode's own features can call it (a route may name it).
    pub callable: bool,
    /// An agent's sessions can run on it.
    pub session_capable: bool,
    /// The routes whose primary or backup it is.
    pub routed: Vec<ai::Purpose>,
    /// Agent profiles whose sessions run on it.
    pub agents: i64,
}

fn provider_row(c: &Connectors, cfg: &AiConfig, p: &AiProvider, agents: i64) -> ProviderRow {
    let st = c.keyring().status(&p.key_entry());
    ProviderRow {
        provider: p.clone(),
        url: p.url(&cfg.settings),
        address: p.address(&cfg.settings),
        key_stored: st.stored,
        key_last4: st.last4,
        callable: p.backend().is_some(),
        session_capable: p.kind.session_capable(),
        routed: cfg
            .routes
            .iter()
            .filter(|r| {
                r.provider_id.as_deref() == Some(&p.id)
                    || r.fallbacks.iter().any(|t| t.provider_id == p.id)
            })
            .map(|r| r.purpose)
            .collect(),
        agents,
    }
}

fn agents_per_provider(conn: &rusqlite::Connection) -> DbResult<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT provider_id, COUNT(*) FROM agent_profiles WHERE provider_id IS NOT NULL GROUP BY provider_id",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `ai_providers_list`: built-ins first, then the user's, by position.
pub fn ai_providers_list(c: &Connectors) -> ApiResult<Vec<ProviderRow>> {
    let (cfg, agents) = c
        .db()
        .read(|conn| Ok((AiConfig::load(conn)?, agents_per_provider(conn)?)))?;
    Ok(cfg
        .providers
        .iter()
        .map(|p| {
            let n = agents
                .iter()
                .find(|(id, _)| *id == p.id)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            provider_row(c, &cfg, p, n)
        })
        .collect())
}

/// `ai_provider_save { draft }`: insert or update; an invalid draft is
/// `{code: "invalid", fields}` ([`providers::validate`]). The row back.
pub fn ai_provider_save(c: &Connectors, draft: ProviderDraft) -> ApiResult<ProviderRow> {
    let now = format_timestamp(c.now());
    let saved = c
        .db()
        .write(move |conn| Ok(providers::save(conn, &draft, &now)))?
        .map_err(|e| ApiError {
            code: e.code,
            message: match e.fields.first() {
                Some(f) => format!("{} ({})", e.message, f.message),
                None => e.message,
            },
        })?;
    let (cfg, agents) = c
        .db()
        .read(|conn| Ok((AiConfig::load(conn)?, agents_per_provider(conn)?)))?;
    let n = agents
        .iter()
        .find(|(id, _)| *id == saved.id)
        .map(|(_, n)| *n)
        .unwrap_or(0);
    c.kick_scoring();
    Ok(provider_row(c, &cfg, &saved, n))
}

/// `ai_provider_delete { id }`: the user's providers only; its secret goes
/// with it, the routes and agents that named it fall back (06 §10.1A).
pub fn ai_provider_delete(c: &Connectors, args: IdArgs) -> ApiResult<bool> {
    let id = args.id.clone();
    let entry = c
        .db()
        .read(|conn| providers::get(conn, &id))?
        .map(|p| p.key_entry());
    let id = args.id.clone();
    let gone = c
        .db()
        .write(move |conn| Ok(providers::delete(conn, &id)))?
        .map_err(|e| ApiError {
            code: e.code,
            message: e.message,
        })?;
    if gone
        && let Some(e) = entry
        && e.starts_with(names::AI_PROVIDER_PREFIX)
    {
        let _ = c.keyring().delete(&e);
    }
    c.kick_scoring();
    Ok(gone)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderKeyArgs {
    pub id: String,
    /// `null` removes the entry.
    pub key: Option<String>,
}

/// `ai_provider_key_set { id, key }`: the provider's secret into the
/// keyring (a built-in's is the entry of §10.2 / §10.3); only "stored" and
/// the last four characters come back.
pub fn ai_provider_key_set(c: &Connectors, args: ProviderKeyArgs) -> ApiResult<SecretStatus> {
    let id = args.id.clone();
    let Some(p) = c.db().read(|conn| providers::get(conn, &id))? else {
        return Err(err("not_found", "no such provider"));
    };
    if p.builtin {
        return ai_key_set(
            c,
            AiKeySetArgs {
                backend: p.backend().unwrap_or(BackendKind::ClaudeApi),
                key: args.key,
            },
        );
    }
    let entry = p.key_entry();
    match args.key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => c
            .keyring()
            .set(&entry, k)
            .map_err(|e| err("keyring", e.to_string()))?,
        None => {
            c.keyring()
                .delete(&entry)
                .map_err(|e| err("keyring", e.to_string()))?;
        }
    }
    c.kick_scoring();
    Ok(c.keyring().status(&entry))
}

/// `ai_provider_test { id, model? }`: the Test call against one provider.
pub fn ai_provider_test(c: &Connectors, args: ProviderTestArgs) -> ApiResult<AiTestResult> {
    let id = args.id.clone();
    let Some(p) = c.db().read(|conn| providers::get(conn, &id))? else {
        return Err(err("not_found", "no such provider"));
    };
    if p.backend().is_none() {
        return Ok(AiTestResult {
            ok: false,
            latency_ms: None,
            error: Some(format!(
                "{} runs sessions; launch an agent on it to try it",
                p.kind.label()
            )),
        });
    }
    ai_test_call_with(
        c,
        AiTestArgs {
            backend: None,
            provider: Some(args.id),
            model: args.model,
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTestArgs {
    pub id: String,
    #[serde(default)]
    pub model: Option<String>,
}

/// `ai_provider_models { id }`: what the endpoint lists (`GET /v1/models`),
/// for the model pickers; the user's named models come first.
pub fn ai_provider_models(c: &Connectors, args: IdArgs) -> ApiResult<Vec<String>> {
    let mut cfg = c.db().read(AiConfig::load)?;
    let Some(p) = cfg.provider(&args.id).cloned() else {
        return Err(err("not_found", "no such provider"));
    };
    let mut out = p.models.clone();
    if p.backend().is_some() {
        cfg.routes = vec![Route {
            purpose: ai::Purpose::Scoring,
            provider_id: Some(p.id.clone()),
            model: None,
            fallbacks: Vec::new(),
        }];
        cfg.settings.local_only = false;
        if let Ok(r) = ai::resolve_with(
            &cfg,
            c.keyring(),
            c.ai_factory().as_ref(),
            ai::Purpose::Scoring,
        ) && let Ok(listed) = r.backend.list_models()
        {
            for m in listed {
                if !out.contains(&m) {
                    out.push(m);
                }
            }
        }
    }
    Ok(out)
}

/// `ai_routes_list`: one route per purpose (chat, scoring, summary).
pub fn ai_routes_list(c: &Connectors) -> ApiResult<Vec<Route>> {
    Ok(c.db().read(providers::routes)?)
}

/// `ai_route_set { route }`: store a route; an invalid one is
/// `{code: "invalid", fields}`. Scoring is kicked (the route may have
/// changed what it can call).
pub fn ai_route_set(c: &Connectors, route: Route) -> ApiResult<Route> {
    let now = format_timestamp(c.now());
    let saved = c
        .db()
        .write(move |conn| Ok(providers::route_set(conn, &route, &now)))?
        .map_err(|e| ApiError {
            code: e.code,
            message: match e.fields.first() {
                Some(f) => format!("{} ({})", e.message, f.message),
                None => e.message,
            },
        })?;
    c.kick_scoring();
    Ok(saved)
}

/// The kinds a provider may be, for the add dialog: kind, label, whether
/// Mode's features can call it, whether sessions can run on it, the
/// auths it takes and its default address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderKindRow {
    pub kind: ProviderKind,
    pub label: String,
    pub callable: bool,
    pub session_capable: bool,
    pub auths: Vec<providers::Auth>,
    pub default_url: Option<String>,
}

/// `ai_provider_kinds`.
pub fn ai_provider_kinds() -> Vec<ProviderKindRow> {
    ProviderKind::ALL
        .iter()
        .map(|k| ProviderKindRow {
            kind: *k,
            label: k.label().into(),
            callable: k.backend().is_some(),
            session_capable: k.session_capable(),
            auths: k.auths().to_vec(),
            default_url: k.default_url().map(str::to_string),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretNameArgs {
    /// `anthropic_api_key` | `local_token` (the settings contract's names).
    pub name: String,
}

fn secret_entry(name: &str) -> ApiResult<&'static str> {
    match name {
        "anthropic_api_key" | "ai:anthropic_api_key" => Ok(names::AI_ANTHROPIC_API_KEY),
        "local_token" | "ai:local_token" => Ok(names::AI_LOCAL_TOKEN),
        _ => Err(err(
            "invalid",
            format!("{name} is not a secret Mode stores"),
        )),
    }
}

/// `secret_status { name }`.
pub fn secret_status(c: &Connectors, args: SecretNameArgs) -> ApiResult<SecretStatus> {
    Ok(c.keyring().status(secret_entry(&args.name)?))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretSetArgs {
    pub name: String,
    pub value: Option<String>,
}

/// `secret_set { name, value }` (`null` removes it).
pub fn secret_set(c: &Connectors, args: SecretSetArgs) -> ApiResult<SecretStatus> {
    let entry = secret_entry(&args.name)?;
    ai_key_set(
        c,
        AiKeySetArgs {
            backend: if entry == names::AI_LOCAL_TOKEN {
                BackendKind::Ollama
            } else {
                BackendKind::ClaudeApi
            },
            key: args.value,
        },
    )
}

/// `connectors_diagnostics`: scorer counters and queue depth (07 §11.6;
/// counts only, never text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrgencyDiagnostics {
    pub counters: crate::urgency::scoring::Counters,
    pub queue_depth: i64,
    pub writebacks_queued: i64,
}

pub fn urgency_diagnostics(c: &Connectors) -> ApiResult<UrgencyDiagnostics> {
    let (q, w) = c.db().read(|conn| {
        let q = crate::urgency::scoring::queue_depth(conn)?;
        let w: i64 = conn.query_row("SELECT COUNT(*) FROM connector_writebacks", [], |r| {
            r.get(0)
        })?;
        Ok((q, w))
    })?;
    Ok(UrgencyDiagnostics {
        counters: c.scorer_counters(),
        queue_depth: q,
        writebacks_queued: w,
    })
}
