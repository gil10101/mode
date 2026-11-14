//! Connectors (06): the shared adapter interface (§2.1) with capability
//! flags (§2.2), the poll scheduler and budgets (§2.3), the health model
//! (§2.4), the write-back queue (§2.5), accounts (§3), OAuth loopback with
//! PKCE (§4), the Gmail (§6), Google Calendar (§7) and Slack (§8) adapters,
//! Claude Code as a connector card (§9) and the AI backends (§10).
//!
//! Everything here is blocking and runs on the blocking pool; the service
//! ([`service::Connectors`]) owns the background loop. Provider-specific
//! code never leaks past the adapters: the Inbox, Calendar, Settings and
//! Diagnostics read the tables and the health interface only.

pub mod accounts;
pub mod ai;
pub mod api;
pub mod claude_code;
pub mod gcal;
pub mod gmail;
pub mod health;
pub mod http;
pub mod oauth;
pub mod scheduler;
pub mod service;
pub mod slack;
pub mod store;
pub mod writeback;

use std::cell::Cell;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub use service::Connectors;

use crate::db::{Db, DbError};
use crate::repo::{Account, AccountSource, ConnectorWriteback, Message};
use http::{HttpClient, HttpError, HttpRequest, HttpResponse};

// ------------------------------------------------------------- vocabulary

/// `accounts.provider`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Google,
    Slack,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Google => "google",
            Provider::Slack => "slack",
        }
    }

    pub fn parse(s: &str) -> Option<Provider> {
        match s {
            "google" | "gmail" | "calendar" | "gcal" => Some(Provider::Google),
            "slack" => Some(Provider::Slack),
            _ => None,
        }
    }

    /// The name used in user-facing text ("not synced to Gmail").
    pub fn label(self) -> &'static str {
        match self {
            Provider::Google => "Google",
            Provider::Slack => "Slack",
        }
    }
}

/// A service inside an account (`services_json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Gmail,
    Calendar,
    Slack,
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Service::Gmail => "gmail",
            Service::Calendar => "calendar",
            Service::Slack => "slack",
        }
    }

    pub fn parse(s: &str) -> Option<Service> {
        match s {
            "gmail" => Some(Service::Gmail),
            "calendar" => Some(Service::Calendar),
            "slack" => Some(Service::Slack),
            _ => None,
        }
    }
}

/// `account_sources.kind` (06 §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    GmailInbox,
    GmailLabel,
    SlackDm,
    SlackMpim,
    SlackMentions,
    SlackChannel,
    Calendar,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::GmailInbox => "gmail_inbox",
            SourceKind::GmailLabel => "gmail_label",
            SourceKind::SlackDm => "slack_dm",
            SourceKind::SlackMpim => "slack_mpim",
            SourceKind::SlackMentions => "slack_mentions",
            SourceKind::SlackChannel => "slack_channel",
            SourceKind::Calendar => "calendar",
        }
    }

    pub fn parse(s: &str) -> Option<SourceKind> {
        Some(match s {
            "gmail_inbox" => SourceKind::GmailInbox,
            "gmail_label" => SourceKind::GmailLabel,
            "slack_dm" => SourceKind::SlackDm,
            "slack_mpim" => SourceKind::SlackMpim,
            "slack_mentions" => SourceKind::SlackMentions,
            "slack_channel" => SourceKind::SlackChannel,
            "calendar" => SourceKind::Calendar,
            _ => return None,
        })
    }

    pub fn service(self) -> Service {
        match self {
            SourceKind::GmailInbox | SourceKind::GmailLabel => Service::Gmail,
            SourceKind::Calendar => Service::Calendar,
            _ => Service::Slack,
        }
    }

    /// Scheduler priority (06 §2.3): lower runs first. Slack DMs, group DMs
    /// and mentions before channels; Gmail inbox before labels; calendar last.
    pub fn priority(self) -> u8 {
        match self {
            SourceKind::SlackDm | SourceKind::SlackMpim | SourceKind::SlackMentions => 0,
            SourceKind::GmailInbox => 1,
            SourceKind::SlackChannel => 2,
            SourceKind::GmailLabel => 3,
            SourceKind::Calendar => 4,
        }
    }

    /// Always-on sources that cannot be unwatched (06 §8.4, ADR-0014).
    pub fn always_on(self) -> bool {
        matches!(
            self,
            SourceKind::SlackDm | SourceKind::SlackMpim | SourceKind::SlackMentions
        )
    }
}

/// A triage write-back (06 §2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriageAction {
    MarkRead,
    MarkUnread,
    Archive,
    Unarchive,
}

impl TriageAction {
    pub fn as_str(self) -> &'static str {
        match self {
            TriageAction::MarkRead => "mark_read",
            TriageAction::MarkUnread => "mark_unread",
            TriageAction::Archive => "archive",
            TriageAction::Unarchive => "unarchive",
        }
    }

    pub fn parse(s: &str) -> Option<TriageAction> {
        Some(match s {
            "mark_read" => TriageAction::MarkRead,
            "mark_unread" => TriageAction::MarkUnread,
            "archive" => TriageAction::Archive,
            "unarchive" => TriageAction::Unarchive,
            _ => return None,
        })
    }

    pub fn opposite(self) -> TriageAction {
        match self {
            TriageAction::MarkRead => TriageAction::MarkUnread,
            TriageAction::MarkUnread => TriageAction::MarkRead,
            TriageAction::Archive => TriageAction::Unarchive,
            TriageAction::Unarchive => TriageAction::Archive,
        }
    }
}

/// The capability flags of 06 §2.2, refined per account from its scopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub read_messages: bool,
    pub read_events: bool,
    pub fetch_body_on_demand: bool,
    pub mark_read_writeback: bool,
    pub mark_unread_writeback: bool,
    pub archive_writeback: bool,
    /// `full` (Gmail), `mark_read` (Slack), `none`.
    pub thread_actions: String,
    /// `web` (Gmail), `html_link` (Calendar), `slack` (`slack://` + web fallback).
    pub deep_link: String,
    /// `history_id`, `sync_token`, `latest_ts`.
    pub incremental_cursor: String,
}

impl Capabilities {
    pub fn supports(&self, action: TriageAction) -> bool {
        match action {
            TriageAction::MarkRead => self.mark_read_writeback,
            TriageAction::MarkUnread => self.mark_unread_writeback,
            TriageAction::Archive | TriageAction::Unarchive => self.archive_writeback,
        }
    }
}

/// The capabilities of an account row (provider + granted scopes).
pub fn capabilities(account: &Account) -> Capabilities {
    let scopes = json_strings(&account.scopes_json);
    let services = json_strings(&account.services_json);
    match Provider::parse(&account.provider) {
        Some(Provider::Slack) => slack::capabilities(&scopes),
        _ => {
            let mut c = gmail::capabilities(&scopes);
            c.read_messages = services.iter().any(|s| s == "gmail");
            c.read_events = services.iter().any(|s| s == "calendar");
            c
        }
    }
}

/// A JSON array of strings (`[]` on anything else).
pub fn json_strings(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

// ----------------------------------------------------------------- errors

/// What went wrong talking to a provider; mapped onto `accounts.status`
/// and `status_detail` by [`health`]. Messages never carry a secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectorError {
    /// 401 after one refresh-and-retry.
    #[error("the provider rejected the token")]
    Unauthorized,
    /// `invalid_grant` / `invalid_refresh_token` / `token_expired`: re-authorize.
    #[error("needs to be re-authorized ({0})")]
    InvalidGrant(String),
    /// `token_revoked` / `account_inactive` / `invalid_auth`.
    #[error("access was revoked ({0})")]
    Revoked(String),
    #[error("rate limited; retry in {retry_after_secs} s")]
    RateLimited { retry_after_secs: u64 },
    /// 404 / 410 on a cursor: resync the source.
    #[error("the sync cursor expired")]
    CursorGone,
    #[error("no longer available at the source")]
    NotFound,
    /// A scope is missing for this call (`missing_scope`, `insufficientPermissions`).
    #[error("missing permission: {0}")]
    MissingScope(String),
    /// Project / app configuration (`accessNotConfigured`, admin approval).
    #[error("{0}")]
    Config(String),
    /// The action has no capability on this account.
    #[error("not supported by this account")]
    Unsupported,
    /// The per-minute budget ran out mid-sync (06 §2.3); not a failure.
    #[error("request budget used up for this minute")]
    BudgetExhausted,
    #[error("offline: {0}")]
    Offline(String),
    #[error("provider error {0}: {1}")]
    Server(u16, String),
    #[error("unexpected response: {0}")]
    Protocol(String),
    #[error("keyring: {0}")]
    Keyring(String),
    #[error("database: {0}")]
    Db(String),
    /// The OAuth client is not configured (06 §6.1, §8.1).
    #[error("{0}")]
    NotConfigured(String),
}

impl ConnectorError {
    /// Auth failures move the account to `needs_reauth` / `revoked`.
    pub fn is_auth(&self) -> bool {
        matches!(
            self,
            ConnectorError::Unauthorized
                | ConnectorError::InvalidGrant(_)
                | ConnectorError::Revoked(_)
        )
    }

    /// Counted toward the five consecutive failures of 06 §2.4.
    pub fn counts_as_failure(&self) -> bool {
        matches!(
            self,
            ConnectorError::Server(..)
                | ConnectorError::Protocol(_)
                | ConnectorError::Config(_)
                | ConnectorError::MissingScope(_)
                | ConnectorError::Db(_)
                | ConnectorError::Keyring(_)
        )
    }
}

impl From<DbError> for ConnectorError {
    fn from(e: DbError) -> Self {
        ConnectorError::Db(e.to_string())
    }
}

impl From<HttpError> for ConnectorError {
    fn from(e: HttpError) -> Self {
        match e {
            HttpError::Offline(m) => ConnectorError::Offline(m),
            HttpError::Timeout => ConnectorError::Offline("timed out".into()),
            HttpError::Other(m) => ConnectorError::Protocol(m),
        }
    }
}

impl From<crate::keyring::KeyringError> for ConnectorError {
    fn from(e: crate::keyring::KeyringError) -> Self {
        ConnectorError::Keyring(e.to_string())
    }
}

impl From<serde_json::Error> for ConnectorError {
    fn from(e: serde_json::Error) -> Self {
        ConnectorError::Protocol(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ConnectorError>;

// ------------------------------------------------------------- settings

/// The `polling.*`, `gmail.*`, `slack.*` keys an adapter or the scheduler reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectorSettings {
    pub gmail_seconds: i64,
    pub slack_seconds: i64,
    pub calendar_seconds: i64,
    pub slack_budget_per_minute: i64,
    pub gmail_initial_sync_days: i64,
    pub gmail_initial_sync_max: i64,
    /// `auto` | `always` | `never` (06 §6.7).
    pub gmail_body_fetch: String,
    pub slack_initial_sync_days: i64,
    pub slack_search_mentions_seconds: i64,
    pub message_body_days: i64,
    pub oauth_ports: Vec<u16>,
}

impl Default for ConnectorSettings {
    fn default() -> Self {
        ConnectorSettings {
            gmail_seconds: 60,
            slack_seconds: 30,
            calendar_seconds: 300,
            slack_budget_per_minute: 40,
            gmail_initial_sync_days: 14,
            gmail_initial_sync_max: 500,
            gmail_body_fetch: "auto".into(),
            slack_initial_sync_days: 7,
            slack_search_mentions_seconds: 60,
            message_body_days: 7,
            oauth_ports: oauth::DEFAULT_PORTS.to_vec(),
        }
    }
}

impl ConnectorSettings {
    pub fn load(conn: &Connection) -> crate::db::Result<ConnectorSettings> {
        use crate::settings::{get_as_in, get_i64_in, get_string_in};
        let d = ConnectorSettings::default();
        let int = |k: &str, dflt: i64| -> crate::db::Result<i64> {
            Ok(get_i64_in(conn, k)?.filter(|n| *n > 0).unwrap_or(dflt))
        };
        Ok(ConnectorSettings {
            gmail_seconds: int("polling.gmail_seconds", d.gmail_seconds)?,
            slack_seconds: int("polling.slack_seconds", d.slack_seconds)?,
            calendar_seconds: int("polling.calendar_seconds", d.calendar_seconds)?,
            slack_budget_per_minute: int(
                "polling.slack_budget_per_minute",
                d.slack_budget_per_minute,
            )?,
            gmail_initial_sync_days: int("gmail.initial_sync_days", d.gmail_initial_sync_days)?,
            gmail_initial_sync_max: int("gmail.initial_sync_max", d.gmail_initial_sync_max)?,
            gmail_body_fetch: get_string_in(conn, "gmail.body_fetch")?
                .filter(|s| matches!(s.as_str(), "auto" | "always" | "never"))
                .unwrap_or(d.gmail_body_fetch),
            slack_initial_sync_days: int("slack.initial_sync_days", d.slack_initial_sync_days)?,
            slack_search_mentions_seconds: int(
                "slack.search_mentions_seconds",
                d.slack_search_mentions_seconds,
            )?,
            message_body_days: int("retention.message_body_days", d.message_body_days)?,
            oauth_ports: get_as_in::<Vec<u16>>(conn, "connectors.oauth_ports")
                .ok()
                .filter(|v| !v.is_empty())
                .unwrap_or(d.oauth_ports),
        })
    }
}

// --------------------------------------------------------------- the api

/// Where an adapter gets its bearer token, and how it refreshes it once on
/// a 401 (06 §2.1 "401 → refresh then retry once").
pub trait TokenSource: Send + Sync {
    fn access_token(&self) -> Result<String>;
    /// Refresh now (after a 401) and return the new access token.
    fn refresh(&self) -> Result<String>;
}

/// A fixed token (tests, and calls made right after the token exchange).
pub struct StaticToken(pub String);

impl TokenSource for StaticToken {
    fn access_token(&self) -> Result<String> {
        Ok(self.0.clone())
    }
    fn refresh(&self) -> Result<String> {
        Err(ConnectorError::Unauthorized)
    }
}

/// An authenticated, budgeted HTTP caller for one sync. Every request adds
/// the bearer token, counts against the budget, and on a 401 refreshes
/// once and retries; a second 401 is [`ConnectorError::Unauthorized`].
pub struct Api<'a> {
    pub http: &'a dyn HttpClient,
    tokens: &'a dyn TokenSource,
    budget: u32,
    used: Cell<u32>,
}

impl<'a> Api<'a> {
    pub fn new(http: &'a dyn HttpClient, tokens: &'a dyn TokenSource, budget: u32) -> Api<'a> {
        Api {
            http,
            tokens,
            budget,
            used: Cell::new(0),
        }
    }

    pub fn used(&self) -> u32 {
        self.used.get()
    }

    pub fn remaining(&self) -> u32 {
        self.budget.saturating_sub(self.used.get())
    }

    /// Send with the bearer token; the caller classifies non-2xx statuses
    /// other than 401 (with [`classify_status`] or its own rules).
    pub fn send(&self, req: HttpRequest) -> Result<HttpResponse> {
        if self.used.get() >= self.budget {
            return Err(ConnectorError::BudgetExhausted);
        }
        self.used.set(self.used.get() + 1);
        let token = self.tokens.access_token()?;
        let resp = self.http.send(&req.clone().bearer(&token))?;
        if resp.status != 401 {
            return Ok(resp);
        }
        let token = self.tokens.refresh()?;
        self.used.set(self.used.get() + 1);
        let resp = self.http.send(&req.bearer(&token))?;
        if resp.status == 401 {
            return Err(ConnectorError::Unauthorized);
        }
        Ok(resp)
    }

    /// Send and require 2xx (classifying anything else), returning JSON.
    pub fn json(&self, req: HttpRequest) -> Result<serde_json::Value> {
        let resp = self.send(req)?;
        if !resp.ok() {
            return Err(classify_status(&resp));
        }
        Ok(resp.json_value())
    }
}

/// The generic status mapping (adapters refine provider error bodies).
pub fn classify_status(resp: &HttpResponse) -> ConnectorError {
    match resp.status {
        401 => ConnectorError::Unauthorized,
        404 | 410 => ConnectorError::NotFound,
        429 => ConnectorError::RateLimited {
            retry_after_secs: resp.retry_after_secs().unwrap_or(60),
        },
        599 => ConnectorError::Protocol(resp.text()),
        s => ConnectorError::Server(s, short_detail(&resp.json_value())),
    }
}

/// A provider error body reduced to a short, secret-free line.
pub fn short_detail(v: &serde_json::Value) -> String {
    let s = v
        .pointer("/error/message")
        .or_else(|| v.pointer("/error_description"))
        .or_else(|| v.pointer("/error"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    crate::redact::scrub(&s.chars().take(200).collect::<String>()).into_owned()
}

// ------------------------------------------------------------ the adapter

/// Who connected (06 §3.1), from the identity call(s) after the exchange.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Identity {
    pub external_id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub handle: Option<String>,
    pub team_id: Option<String>,
    pub team_name: Option<String>,
    /// Slack: the workspace URL from `auth.test` (deep-link fallback domain).
    pub team_url: Option<String>,
    pub avatar_url: Option<String>,
    pub services: Vec<String>,
    pub scopes: Vec<String>,
}

/// A candidate `account_sources` row (06 §2.1 `list_sources`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCandidate {
    pub kind: SourceKind,
    pub external_id: String,
    pub name: String,
    /// Seeded enabled (Calendar: `primary` and `selected`).
    pub enabled: bool,
    pub private: bool,
    pub member_count: Option<i64>,
}

/// What one `sync` did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOutcome {
    /// New `messages.id`s (for the urgency engine and alerts).
    pub inserted: Vec<String>,
    pub updated: usize,
    pub deleted: usize,
    pub events_changed: usize,
    pub requests: u32,
    /// The budget ran out; the source stays due.
    pub more: bool,
    /// A full resync ran (cursor lost or first sync).
    pub full_sync: bool,
    /// The newest message time seen (Slack inactivity ladder).
    pub latest_message_at: Option<String>,
    /// Alert events produced at ingest (deterministic tiers).
    #[serde(skip)]
    pub alerts: Vec<crate::urgency::alerts::AlertEvent>,
}

/// A body fetched on demand (06 §2.1 `fetch_body`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchedBody {
    pub text: String,
    pub truncated: bool,
}

/// What every sync call gets.
pub struct SyncCx<'a> {
    pub db: &'a Db,
    pub api: Api<'a>,
    pub account: &'a Account,
    pub settings: &'a ConnectorSettings,
    pub now: OffsetDateTime,
    pub tz: crate::notes::ctx::TzMode,
}

impl SyncCx<'_> {
    pub fn now_str(&self) -> String {
        crate::db::format_timestamp(self.now)
    }
}

/// The adapter interface of 06 §2.1. `begin_connect` / `complete_connect`
/// / `refresh` / `revoke` are generic over [`oauth::ProviderParams`]
/// ([`accounts`], [`oauth`]); `health` is derived ([`health`]).
pub trait ConnectorAdapter: Send + Sync {
    fn provider(&self) -> Provider;
    /// §2.2, refined from the granted scopes.
    fn capabilities(&self, account: &Account) -> Capabilities;
    /// The identity call(s) of `complete_connect` (userinfo / `auth.test`).
    fn identify(&self, api: &Api<'_>, granted_scopes: &[String]) -> Result<Identity>;
    /// Candidate sources (labels, calendars, conversations).
    fn list_sources(&self, api: &Api<'_>, account: &Account) -> Result<Vec<SourceCandidate>>;
    /// One source, from its cursor, within the api's budget.
    fn sync(&self, cx: &SyncCx<'_>, source: &AccountSource) -> Result<SyncOutcome>;
    /// The plain-text body (≤ 32 KB).
    fn fetch_body(&self, cx: &SyncCx<'_>, message: &Message) -> Result<FetchedBody>;
    /// One queued write-back (06 §2.5).
    fn write_back(&self, cx: &SyncCx<'_>, item: &ConnectorWriteback) -> Result<()>;
    /// Open in source.
    fn source_url(&self, account: &Account, message: &Message) -> String;
}

/// `messages.body_text` is capped at 32 KB (09 §5.3).
pub const BODY_CAP_BYTES: usize = 32 * 1024;

/// Cut `text` to `cap` bytes at a char boundary; `(text, truncated)`.
pub fn cap_body(text: &str, cap: usize) -> (String, bool) {
    if text.len() <= cap {
        return (text.to_string(), false);
    }
    let mut end = cap;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

/// The first 500 characters of plain text, whitespace collapsed (09 §5.3 `snippet`).
pub fn snippet_of(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(500).collect()
}

/// Seconds since the epoch → a 09 §4.1 timestamp.
pub fn ts_from_epoch_f64(secs: f64) -> Option<String> {
    let nanos = (secs * 1e9) as i128;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .map(crate::db::format_timestamp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_kinds_round_trip_and_order() {
        for k in [
            SourceKind::GmailInbox,
            SourceKind::GmailLabel,
            SourceKind::SlackDm,
            SourceKind::SlackMpim,
            SourceKind::SlackMentions,
            SourceKind::SlackChannel,
            SourceKind::Calendar,
        ] {
            assert_eq!(SourceKind::parse(k.as_str()), Some(k));
        }
        assert!(SourceKind::SlackDm.priority() < SourceKind::SlackChannel.priority());
        assert!(SourceKind::GmailInbox.priority() < SourceKind::GmailLabel.priority());
        assert!(SourceKind::Calendar.priority() > SourceKind::GmailLabel.priority());
    }

    #[test]
    fn body_caps_at_a_char_boundary() {
        let (t, cut) = cap_body("héllo", 2);
        assert_eq!(t, "h");
        assert!(cut);
        assert_eq!(snippet_of("a \n\n b\tc"), "a b c");
        assert_eq!(snippet_of(&"x".repeat(900)).len(), 500);
    }

    #[test]
    fn epoch_seconds_format() {
        assert_eq!(
            ts_from_epoch_f64(1_760_700_000.123_456).as_deref(),
            Some("2025-10-17T11:20:00.123Z")
        );
    }

    struct Flaky {
        calls: std::sync::Mutex<u32>,
    }
    impl HttpClient for Flaky {
        fn send(&self, req: &HttpRequest) -> std::result::Result<HttpResponse, HttpError> {
            let mut c = self.calls.lock().unwrap();
            *c += 1;
            let auth = req.header_value("authorization").unwrap_or("").to_string();
            Ok(HttpResponse {
                status: if auth.ends_with("new") { 200 } else { 401 },
                headers: vec![],
                body: b"{}".to_vec(),
            })
        }
    }
    struct Refreshing;
    impl TokenSource for Refreshing {
        fn access_token(&self) -> Result<String> {
            Ok("old".into())
        }
        fn refresh(&self) -> Result<String> {
            Ok("new".into())
        }
    }

    #[test]
    fn a_401_refreshes_once_and_retries() {
        let http = Flaky {
            calls: std::sync::Mutex::new(0),
        };
        let api = Api::new(&http, &Refreshing, 10);
        let r = api.send(HttpRequest::get("https://x/y")).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(api.used(), 2);
        let fixed = StaticToken("old".into());
        let api = Api::new(&http, &fixed, 10);
        assert_eq!(
            api.send(HttpRequest::get("https://x/y")),
            Err(ConnectorError::Unauthorized)
        );
        let api = Api::new(&http, &Refreshing, 0);
        assert_eq!(
            api.send(HttpRequest::get("https://x/y")),
            Err(ConnectorError::BudgetExhausted)
        );
    }
}
