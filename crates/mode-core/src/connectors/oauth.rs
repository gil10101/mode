//! OAuth loopback with PKCE (06 §4, 08 §9): PKCE S256 and `state` from the
//! OS RNG, the per-provider parameters of §4.2, the loopback listener of
//! §4.3 on `connectors.oauth_ports` (127.0.0.1 only, one request whose
//! `state` matches, `/callback` only, loopback peers only, 5-minute hard
//! timeout, cancellable), the token exchange, refresh and revoke, and the
//! error mapping of §4.4. The app crate opens the browser; this module
//! returns the URL.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use super::http::{HttpClient, HttpRequest};
use super::{ConnectorError, Provider};
use crate::keyring::{Keyring, names};

/// `connectors.oauth_ports` default (06 §4.3): registered in the Slack manifest.
pub const DEFAULT_PORTS: [u16; 5] = [48311, 48312, 48313, 48314, 48315];
/// The flow's hard timeout (06 §4.3).
pub const FLOW_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Refresh Google tokens this long before expiry (06 §6.10).
pub const GOOGLE_REFRESH_MARGIN_SECS: i64 = 5 * 60;
/// Refresh rotated Slack tokens this long before expiry (06 §8.9).
pub const SLACK_REFRESH_MARGIN_SECS: i64 = 10 * 60;
/// Refresh any Slack token older than this at launch (30-day refresh-token expiry).
pub const SLACK_REFRESH_AGE_DAYS: i64 = 20;

pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
pub const SLACK_AUTH_URL: &str = "https://slack.com/oauth/v2/authorize";
pub const SLACK_TOKEN_URL: &str = "https://slack.com/api/oauth.v2.access";
pub const SLACK_REVOKE_URL: &str = "https://slack.com/api/auth.revoke";

pub const SCOPE_GMAIL_MODIFY: &str = "https://www.googleapis.com/auth/gmail.modify";
pub const SCOPE_GMAIL_READONLY: &str = "https://www.googleapis.com/auth/gmail.readonly";
pub const SCOPE_CALENDAR_READONLY: &str = "https://www.googleapis.com/auth/calendar.readonly";
pub const SCOPE_OPENID: &str = "openid";
pub const SCOPE_USERINFO_EMAIL: &str = "https://www.googleapis.com/auth/userinfo.email";

/// The Slack user scopes of 06 §8.2 (read side).
pub const SLACK_READ_SCOPES: &[&str] = &[
    "channels:history",
    "groups:history",
    "im:history",
    "mpim:history",
    "channels:read",
    "groups:read",
    "im:read",
    "mpim:read",
    "users:read",
    "search:read",
];
/// The optional write scopes, only for `conversations.mark` (06 §8.2).
pub const SLACK_WRITE_SCOPES: &[&str] =
    &["channels:write", "groups:write", "im:write", "mpim:write"];

// ------------------------------------------------------------------ PKCE

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // The OS RNG failing is unrecoverable for a security flow.
    getrandom::fill(&mut b).expect("OS random number generator");
    b
}

/// PKCE S256 (06 §4.3): a 64-byte verifier, base64url (86 chars).
#[derive(Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl std::fmt::Debug for Pkce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pkce")
            .field("challenge", &self.challenge)
            .finish_non_exhaustive()
    }
}

impl Pkce {
    pub fn new() -> Pkce {
        let verifier = URL_SAFE_NO_PAD.encode(random_bytes::<64>());
        let challenge = Pkce::challenge_for(&verifier);
        Pkce {
            verifier,
            challenge,
        }
    }

    /// `base64url(sha256(verifier))`, no padding.
    pub fn challenge_for(verifier: &str) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Pkce::new()
    }
}

/// A 32-byte random `state`, base64url (43 chars).
pub fn random_state() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

// ---------------------------------------------------- per-provider params

/// The per-service toggles of `begin_connect` (06 §2.1, §6.3, §8.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectOptions {
    #[serde(default = "yes")]
    pub gmail: bool,
    #[serde(default = "yes")]
    pub calendar: bool,
    /// Gmail with `gmail.readonly` (no write-back, 06 §6.3).
    #[serde(default)]
    pub gmail_read_only: bool,
    /// Slack with the `*:write` scopes for `conversations.mark`.
    #[serde(default = "yes")]
    pub slack_writeback: bool,
    /// Re-authorize: Google `login_hint`.
    #[serde(default)]
    pub login_hint: Option<String>,
    /// Slack: which workspace app (`oauth_client:slack:<team_id>`).
    #[serde(default)]
    pub team_id: Option<String>,
}

fn yes() -> bool {
    true
}

impl Default for ConnectOptions {
    fn default() -> Self {
        ConnectOptions {
            gmail: true,
            calendar: true,
            gmail_read_only: false,
            slack_writeback: true,
            login_hint: None,
            team_id: None,
        }
    }
}

/// The scopes requested for `provider` with `opts`.
pub fn scopes(provider: Provider, opts: &ConnectOptions) -> Vec<String> {
    match provider {
        Provider::Google => {
            let mut s = vec![SCOPE_OPENID.to_string(), SCOPE_USERINFO_EMAIL.to_string()];
            if opts.gmail {
                s.push(if opts.gmail_read_only {
                    SCOPE_GMAIL_READONLY.into()
                } else {
                    SCOPE_GMAIL_MODIFY.into()
                });
            }
            if opts.calendar {
                s.push(SCOPE_CALENDAR_READONLY.into());
            }
            s
        }
        Provider::Slack => {
            let mut s: Vec<String> = SLACK_READ_SCOPES.iter().map(|x| x.to_string()).collect();
            if opts.slack_writeback {
                s.extend(SLACK_WRITE_SCOPES.iter().map(|x| x.to_string()));
            }
            s
        }
    }
}

/// The services a set of granted Google scopes covers (06 §4.4: never assumed).
pub fn google_services(granted: &[String]) -> Vec<String> {
    let mut s = Vec::new();
    if granted
        .iter()
        .any(|g| g == SCOPE_GMAIL_MODIFY || g == SCOPE_GMAIL_READONLY)
    {
        s.push("gmail".into());
    }
    if granted.iter().any(|g| g == SCOPE_CALENDAR_READONLY) {
        s.push("calendar".into());
    }
    s
}

/// The OAuth client (06 §5 `oauth_client:*`): the user's own Google
/// desktop client (path A, 06 §6.1) or the Slack workspace app's client id.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthClient {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient")
            .field("client_id", &self.client_id)
            .field("has_secret", &self.client_secret.is_some())
            .finish_non_exhaustive()
    }
}

/// The keyring entry holding a provider's client.
pub fn client_entry(provider: Provider, team_id: Option<&str>) -> String {
    match provider {
        Provider::Google => names::OAUTH_CLIENT_GOOGLE.to_string(),
        Provider::Slack => names::oauth_client_slack(team_id),
    }
}

/// Read the client; `NotConfigured` with the Integrations hint when absent.
pub fn load_client(
    keyring: &Keyring,
    provider: Provider,
    team_id: Option<&str>,
) -> Result<OAuthClient, ConnectorError> {
    let mut c: Option<OAuthClient> = keyring.get_json(&client_entry(provider, team_id))?;
    if c.is_none() && provider == Provider::Slack && team_id.is_some() {
        c = keyring.get_json(&client_entry(provider, None))?;
    }
    let c = c.ok_or_else(|| {
        ConnectorError::NotConfigured(match provider {
            Provider::Google => {
                "Add your Google OAuth client (client ID and secret) on the Gmail card first".into()
            }
            Provider::Slack => "Add your Slack app's client ID on the Slack card first".into(),
        })
    })?;
    if provider == Provider::Google && c.client_secret.as_deref().unwrap_or("").is_empty() {
        return Err(ConnectorError::NotConfigured(
            "The Google OAuth client needs its client secret".into(),
        ));
    }
    Ok(c)
}

/// The authorization URL (06 §4.2).
pub fn authorization_url(
    provider: Provider,
    client: &OAuthClient,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
    opts: &ConnectOptions,
) -> String {
    let scopes = scopes(provider, opts);
    let mut u = match provider {
        Provider::Google => url::Url::parse(GOOGLE_AUTH_URL),
        Provider::Slack => url::Url::parse(SLACK_AUTH_URL),
    }
    .expect("static URL");
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("client_id", &client.client_id);
        q.append_pair("redirect_uri", redirect_uri);
        match provider {
            Provider::Google => {
                q.append_pair("response_type", "code");
                q.append_pair("scope", &scopes.join(" "));
                q.append_pair("access_type", "offline");
                q.append_pair("prompt", "consent");
                q.append_pair("include_granted_scopes", "true");
                if let Some(h) = &opts.login_hint {
                    q.append_pair("login_hint", h);
                }
            }
            Provider::Slack => {
                // User scopes only; no `scope` (bot scopes are refused on PKCE apps).
                q.append_pair("user_scope", &scopes.join(","));
            }
        }
        q.append_pair("state", state);
        q.append_pair("code_challenge", challenge);
        q.append_pair("code_challenge_method", "S256");
    }
    u.to_string()
}

// ---------------------------------------------------------------- tokens

/// The keyring entry `account:<id>` (06 §5).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default = "bearer")]
    pub token_type: String,
    /// 09 §4.1 timestamp; `None` for non-expiring tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub scope: String,
    /// When the refresh token was issued (Slack's 30-day lifetime).
    pub rotated_at: String,
}

fn bearer() -> String {
    "Bearer".into()
}

impl std::fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSet")
            .field("expires_at", &self.expires_at)
            .field("has_refresh", &self.refresh_token.is_some())
            .finish_non_exhaustive()
    }
}

impl TokenSet {
    /// Whether the access token should be refreshed before use.
    pub fn needs_refresh(&self, provider: Provider, now: OffsetDateTime) -> bool {
        let Some(exp) = self
            .expires_at
            .as_deref()
            .and_then(crate::db::parse_timestamp)
        else {
            return false;
        };
        let margin = match provider {
            Provider::Google => GOOGLE_REFRESH_MARGIN_SECS,
            Provider::Slack => SLACK_REFRESH_MARGIN_SECS,
        };
        (exp - now).whole_seconds() < margin
    }

    /// Slack: the refresh token is older than 20 days (refresh at launch).
    pub fn refresh_token_aging(&self, now: OffsetDateTime) -> bool {
        self.refresh_token.is_some()
            && crate::db::parse_timestamp(&self.rotated_at)
                .is_some_and(|t| (now - t).whole_days() >= SLACK_REFRESH_AGE_DAYS)
    }

    pub fn scopes(&self) -> Vec<String> {
        split_scopes(&self.scope)
    }
}

/// Google separates scopes with spaces, Slack with commas.
pub fn split_scopes(s: &str) -> Vec<String> {
    s.split([' ', ','])
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect()
}

/// What the token endpoint returned.
#[derive(Clone, PartialEq, Eq)]
pub struct Exchanged {
    pub tokens: TokenSet,
    pub scopes: Vec<String>,
    /// Google `id_token` claims (`sub`, `email`), unverified until userinfo agrees.
    pub id_sub: Option<String>,
    pub id_email: Option<String>,
    /// Slack `authed_user.id`, `team.id`, `team.name`.
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub team_name: Option<String>,
}

impl std::fmt::Debug for Exchanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Exchanged")
            .field("scopes", &self.scopes)
            .field("team_id", &self.team_id)
            .finish_non_exhaustive()
    }
}

/// A flow failure (06 §4.4), with the text the UI shows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OAuthError {
    #[error("Not connected")]
    AccessDenied,
    #[error("Your workspace needs an admin to approve the Mode app")]
    AdminApproval,
    #[error("the browser did not come back")]
    Timeout,
    #[error("Connect was canceled")]
    Cancelled,
    #[error("ports {0} are in use")]
    PortsBusy(String),
    #[error("Could not finish connecting — try again")]
    Exchange(String),
    #[error("{0}")]
    Connector(String),
    #[error("no connect flow is waiting for this sign-in")]
    UnknownFlow,
}

impl From<ConnectorError> for OAuthError {
    fn from(e: ConnectorError) -> Self {
        OAuthError::Connector(e.to_string())
    }
}

fn decode_jwt_claims(jwt: &str) -> Option<Value> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn expires_at(now: OffsetDateTime, secs: Option<i64>) -> Option<String> {
    secs.filter(|s| *s > 0)
        .map(|s| crate::db::format_timestamp(now + time::Duration::seconds(s)))
}

fn parse_google(v: &Value, now: OffsetDateTime, prev_refresh: Option<&str>) -> Option<Exchanged> {
    let access = v.get("access_token")?.as_str()?.to_string();
    let scope = v
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let refresh = v
        .get("refresh_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| prev_refresh.map(str::to_string));
    let claims = v
        .get("id_token")
        .and_then(Value::as_str)
        .and_then(decode_jwt_claims);
    Some(Exchanged {
        scopes: split_scopes(&scope),
        tokens: TokenSet {
            access_token: access,
            refresh_token: refresh,
            token_type: v
                .get("token_type")
                .and_then(Value::as_str)
                .unwrap_or("Bearer")
                .to_string(),
            expires_at: expires_at(now, v.get("expires_in").and_then(Value::as_i64)),
            scope,
            rotated_at: crate::db::format_timestamp(now),
        },
        id_sub: claims
            .as_ref()
            .and_then(|c| c.get("sub"))
            .and_then(Value::as_str)
            .map(str::to_string),
        id_email: claims
            .as_ref()
            .and_then(|c| c.get("email"))
            .and_then(Value::as_str)
            .map(str::to_string),
        user_id: None,
        team_id: None,
        team_name: None,
    })
}

fn parse_slack(v: &Value, now: OffsetDateTime) -> Option<Exchanged> {
    // The exchange answers with `authed_user.{…}`; a refresh answers flat.
    let u = v
        .get("authed_user")
        .filter(|u| u.get("access_token").is_some())
        .unwrap_or(v);
    let access = u.get("access_token")?.as_str()?.to_string();
    let scope = u
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(Exchanged {
        scopes: split_scopes(&scope),
        tokens: TokenSet {
            access_token: access,
            refresh_token: u
                .get("refresh_token")
                .and_then(Value::as_str)
                .map(str::to_string),
            token_type: "Bearer".into(),
            expires_at: expires_at(now, u.get("expires_in").and_then(Value::as_i64)),
            scope,
            rotated_at: crate::db::format_timestamp(now),
        },
        id_sub: None,
        id_email: None,
        user_id: u
            .get("id")
            .or_else(|| v.get("user_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        team_id: v
            .pointer("/team/id")
            .and_then(Value::as_str)
            .map(str::to_string),
        team_name: v
            .pointer("/team/name")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

/// Exchange the code (06 §4.2 token endpoint). A network failure is
/// retried once (§4.4); the code is single use, so nothing else retries.
pub fn exchange_code(
    http: &dyn HttpClient,
    provider: Provider,
    client: &OAuthClient,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    now: OffsetDateTime,
) -> Result<Exchanged, OAuthError> {
    let mut form: Vec<(&str, &str)> = vec![
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", &client.client_id),
        ("code_verifier", verifier),
    ];
    let url = match provider {
        Provider::Google => {
            form.push(("grant_type", "authorization_code"));
            form.push((
                "client_secret",
                client.client_secret.as_deref().unwrap_or(""),
            ));
            GOOGLE_TOKEN_URL
        }
        Provider::Slack => SLACK_TOKEN_URL,
    };
    let req = HttpRequest::post(url).form(&form);
    let resp = match http.send(&req) {
        Ok(r) => r,
        Err(_) => http
            .send(&req)
            .map_err(|e| OAuthError::Exchange(e.to_string()))?,
    };
    let v = resp.json_value();
    match provider {
        Provider::Google => {
            if !resp.ok() {
                return Err(match v.get("error").and_then(Value::as_str) {
                    Some("access_denied") => OAuthError::AccessDenied,
                    Some(e) => OAuthError::Exchange(e.to_string()),
                    None => OAuthError::Exchange(format!("status {}", resp.status)),
                });
            }
            parse_google(&v, now, None).ok_or_else(|| OAuthError::Exchange("no token".into()))
        }
        Provider::Slack => {
            if v.get("ok").and_then(Value::as_bool) != Some(true) {
                return Err(slack_oauth_error(&v));
            }
            parse_slack(&v, now).ok_or_else(|| OAuthError::Exchange("no user token".into()))
        }
    }
}

fn slack_oauth_error(v: &Value) -> OAuthError {
    match v.get("error").and_then(Value::as_str).unwrap_or("") {
        "access_denied" => OAuthError::AccessDenied,
        "invalid_scope"
        | "app_approval_required"
        | "restricted_action"
        | "admin_approval_required"
        | "team_access_not_granted" => OAuthError::AdminApproval,
        "" => OAuthError::Exchange("unknown".into()),
        e => OAuthError::Exchange(e.to_string()),
    }
}

/// Refresh (06 §4.2): Google keeps the refresh token; Slack's rotation
/// returns a new one that replaces the old at once (single use).
pub fn refresh(
    http: &dyn HttpClient,
    provider: Provider,
    client: &OAuthClient,
    refresh_token: &str,
    now: OffsetDateTime,
) -> Result<Exchanged, ConnectorError> {
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", &client.client_id),
    ];
    let url = match provider {
        Provider::Google => {
            form.push((
                "client_secret",
                client.client_secret.as_deref().unwrap_or(""),
            ));
            GOOGLE_TOKEN_URL
        }
        Provider::Slack => SLACK_TOKEN_URL,
    };
    let resp = http.send(&HttpRequest::post(url).form(&form))?;
    let v = resp.json_value();
    match provider {
        Provider::Google => {
            if !resp.ok() {
                let e = v.get("error").and_then(Value::as_str).unwrap_or("");
                return Err(match e {
                    "invalid_grant" => {
                        ConnectorError::InvalidGrant("refresh token expired or revoked".into())
                    }
                    "invalid_client" | "unauthorized_client" => {
                        ConnectorError::Config("the Google OAuth client was rejected".into())
                    }
                    _ if resp.status >= 500 => ConnectorError::Server(resp.status, e.into()),
                    _ => ConnectorError::InvalidGrant(e.to_string()),
                });
            }
            parse_google(&v, now, Some(refresh_token))
                .ok_or_else(|| ConnectorError::Protocol("no access token".into()))
        }
        Provider::Slack => {
            if v.get("ok").and_then(Value::as_bool) != Some(true) {
                let e = v.get("error").and_then(Value::as_str).unwrap_or("");
                return Err(slack_auth_error(e));
            }
            parse_slack(&v, now).ok_or_else(|| ConnectorError::Protocol("no access token".into()))
        }
    }
}

/// Slack auth error codes → status (06 §8.9).
pub fn slack_auth_error(code: &str) -> ConnectorError {
    match code {
        "invalid_refresh_token" | "token_expired" => ConnectorError::InvalidGrant(code.into()),
        "token_revoked" | "account_inactive" | "invalid_auth" | "not_authed" => {
            ConnectorError::Revoked(code.into())
        }
        "ratelimited" => ConnectorError::RateLimited {
            retry_after_secs: 60,
        },
        other => ConnectorError::Protocol(other.to_string()),
    }
}

/// Revoke upstream, best effort (06 §3.3): Google revokes the whole grant
/// by refresh token; Slack `auth.revoke` with the user token.
pub fn revoke(
    http: &dyn HttpClient,
    provider: Provider,
    tokens: &TokenSet,
) -> Result<(), ConnectorError> {
    let resp = match provider {
        Provider::Google => {
            let t = tokens
                .refresh_token
                .as_deref()
                .unwrap_or(&tokens.access_token);
            http.send(&HttpRequest::post(GOOGLE_REVOKE_URL).form(&[("token", t)]))?
        }
        Provider::Slack => http.send(
            &HttpRequest::post(SLACK_REVOKE_URL)
                .bearer(&tokens.access_token)
                .form::<&str, &str>(&[]),
        )?,
    };
    if resp.ok() {
        Ok(())
    } else {
        Err(ConnectorError::Server(resp.status, "revoke".into()))
    }
}

// --------------------------------------------------------------- loopback

/// The loopback listener of one flow (06 §4.3).
#[derive(Debug)]
pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

/// What the browser brought back.
#[derive(Clone, PartialEq, Eq)]
pub enum Callback {
    Code(String),
    Denied,
    Error(String),
}

impl std::fmt::Debug for Callback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Callback::Code(_) => f.write_str("Code(…)"),
            Callback::Denied => f.write_str("Denied"),
            Callback::Error(e) => write!(f, "Error({e})"),
        }
    }
}

impl Loopback {
    /// Bind `127.0.0.1` on the first free port of `ports`; Google may fall
    /// back to an ephemeral port, Slack fails (its redirect URLs are fixed).
    pub fn bind(ports: &[u16], allow_ephemeral: bool) -> Result<Loopback, OAuthError> {
        for p in ports {
            if let Ok(l) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, *p))) {
                return Loopback::from_listener(l);
            }
        }
        if allow_ephemeral {
            let l = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                .map_err(|e| OAuthError::PortsBusy(e.kind().to_string()))?;
            return Loopback::from_listener(l);
        }
        let list = match (ports.first(), ports.last()) {
            (Some(a), Some(b)) if a != b => format!("{a}–{b}"),
            (Some(a), _) => a.to_string(),
            _ => "(none)".into(),
        };
        Err(OAuthError::PortsBusy(list))
    }

    fn from_listener(l: TcpListener) -> Result<Loopback, OAuthError> {
        l.set_nonblocking(true)
            .map_err(|e| OAuthError::PortsBusy(e.kind().to_string()))?;
        let port = l
            .local_addr()
            .map_err(|e| OAuthError::PortsBusy(e.kind().to_string()))?
            .port();
        Ok(Loopback { listener: l, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// `http://127.0.0.1:<port>/callback` (never `localhost` by name).
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.port)
    }

    /// Wait for the one request whose `state` matches; anything else gets
    /// 404 and is ignored. Ends at `deadline` (Timeout) or when `cancel` is
    /// set (Canceled). Consumes the listener: the port closes on return.
    pub fn wait(
        self,
        state: &str,
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<Callback, OAuthError> {
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(OAuthError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(OAuthError::Timeout);
            }
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    if let Some(cb) = handle_connection(stream, peer, state) {
                        return Ok(cb);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    }
}

fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

/// The callback page (06 §4.3): ≈1 KB inline HTML, Mode's tokens inlined,
/// no scripts, no external requests.
pub fn callback_page(ok: bool, message: &str) -> String {
    let title = if ok { "Connected to Mode" } else { "Mode" };
    let escaped = message
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{title}</title>\
<meta name=\"viewport\" content=\"width=device-width\">\
<style>:root{{--bg:#f7f6f3;--ink:#1d1c1a;--muted:#6b6862;--accent:#3b6ff5}}\
@media (prefers-color-scheme:dark){{:root{{--bg:#151514;--ink:#ecebe8;--muted:#a3a09a}}}}\
body{{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--bg);color:var(--ink);\
font:15px/1.5 -apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif}}\
main{{max-width:28rem;padding:2rem;text-align:center}}h1{{font-size:1.1rem;margin:0 0 .5rem}}\
p{{margin:0;color:var(--muted)}}b{{color:var(--accent)}}</style></head>\
<body><main><h1>{escaped}</h1><p>You can close this tab.</p></main></body></html>"
    )
}

/// Read one request; `Some` only for a matching `/callback` from a loopback peer.
fn handle_connection(mut stream: TcpStream, peer: SocketAddr, state: &str) -> Option<Callback> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    if !peer.ip().is_loopback() {
        respond(&mut stream, "404 Not Found", "");
        return None;
    }
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    while buf.len() < 8192 {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.windows(2).any(|w| w == b"\n\n")
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let line = head.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let Ok(u) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
        respond(&mut stream, "404 Not Found", "");
        return None;
    };
    if method != "GET" || u.path() != "/callback" {
        respond(&mut stream, "404 Not Found", "");
        return None;
    }
    let q: Vec<(String, String)> = u.query_pairs().into_owned().collect();
    let get = |k: &str| q.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    if get("state") != Some(state) {
        respond(&mut stream, "404 Not Found", "");
        return None;
    }
    if let Some(err) = get("error") {
        let cb = if err == "access_denied" {
            Callback::Denied
        } else {
            Callback::Error(err.chars().take(80).collect())
        };
        let msg = if cb == Callback::Denied {
            "Not connected"
        } else {
            "Mode could not connect"
        };
        respond(&mut stream, "200 OK", &callback_page(false, msg));
        return Some(cb);
    }
    match get("code") {
        Some(code) if !code.is_empty() => {
            respond(
                &mut stream,
                "200 OK",
                &callback_page(true, "Connected to Mode — you can close this tab"),
            );
            Some(Callback::Code(code.to_string()))
        }
        _ => {
            respond(&mut stream, "404 Not Found", "");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_is_s256_and_in_range() {
        let p = Pkce::new();
        assert_eq!(p.verifier.len(), 86);
        assert!(
            p.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        // RFC 7636 appendix B
        assert_eq!(
            Pkce::challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(random_state().len(), 43);
        assert_ne!(random_state(), random_state());
    }

    #[test]
    fn scopes_per_toggle() {
        let g = scopes(Provider::Google, &ConnectOptions::default());
        assert!(g.contains(&SCOPE_GMAIL_MODIFY.to_string()));
        assert!(g.contains(&SCOPE_CALENDAR_READONLY.to_string()));
        let ro = scopes(
            Provider::Google,
            &ConnectOptions {
                gmail_read_only: true,
                calendar: false,
                ..Default::default()
            },
        );
        assert!(ro.contains(&SCOPE_GMAIL_READONLY.to_string()));
        assert!(!ro.contains(&SCOPE_CALENDAR_READONLY.to_string()));
        let s = scopes(
            Provider::Slack,
            &ConnectOptions {
                slack_writeback: false,
                ..Default::default()
            },
        );
        assert!(!s.iter().any(|x| x.ends_with(":write")));
        assert!(s.contains(&"search:read".to_string()));
        assert_eq!(
            google_services(&[SCOPE_CALENDAR_READONLY.to_string()]),
            vec!["calendar".to_string()]
        );
    }

    #[test]
    fn authorization_urls_carry_the_spec_parameters() {
        let c = OAuthClient {
            client_id: "cid".into(),
            client_secret: Some("sec".into()),
            project_id: None,
            team_id: None,
        };
        let u = authorization_url(
            Provider::Google,
            &c,
            "http://127.0.0.1:48311/callback",
            "st",
            "ch",
            &ConnectOptions::default(),
        );
        for p in [
            "response_type=code",
            "access_type=offline",
            "prompt=consent",
            "code_challenge=ch",
            "code_challenge_method=S256",
            "state=st",
        ] {
            assert!(u.contains(p), "{p} in {u}");
        }
        assert!(!u.contains("sec"), "the secret never goes in the URL");
        let s = authorization_url(
            Provider::Slack,
            &c,
            "http://127.0.0.1:48311/callback",
            "st",
            "ch",
            &ConnectOptions::default(),
        );
        assert!(s.starts_with(SLACK_AUTH_URL));
        assert!(s.contains("user_scope="));
        assert!(!s.contains("&scope="));
    }

    #[test]
    fn callback_page_is_small_and_static() {
        let p = callback_page(true, "Connected to Mode — you can close this tab");
        assert!(p.len() < 1500);
        assert!(!p.contains("<script"));
        assert!(!p.contains("http"));
    }

    #[test]
    fn token_refresh_margins() {
        let now = crate::db::parse_timestamp("2026-10-22T10:00:00Z").unwrap();
        let t = TokenSet {
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            token_type: "Bearer".into(),
            expires_at: Some("2026-10-22T10:04:00.000Z".into()),
            scope: String::new(),
            rotated_at: "2026-10-01T00:00:00.000Z".into(),
        };
        assert!(t.needs_refresh(Provider::Google, now));
        let later = TokenSet {
            expires_at: Some("2026-10-22T10:08:00.000Z".into()),
            ..t.clone()
        };
        assert!(!later.needs_refresh(Provider::Google, now));
        assert!(later.needs_refresh(Provider::Slack, now));
        assert!(t.refresh_token_aging(now));
    }
}
