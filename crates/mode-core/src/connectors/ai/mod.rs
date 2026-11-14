//! AI backends (06 §10): one active backend for scoring, summaries, Chat
//! and titles. The Claude API with the user's own key ([`claude`]), Ollama
//! and LM Studio through one OpenAI-compatible client ([`openai`]), and a
//! fixture backend for tests ([`fixture`]). This module owns backend
//! selection, the local-only switch (§10.4: under it, only a loopback or
//! private-network backend is called), the address policy (§10.3), the
//! monthly token budget and the daily scoring pace cap (§10.5, 07 §3.4.5),
//! token accounting into `usage_samples` (tokens only; `cost_usd` stays
//! NULL — no price table exists in this build), and the health states of
//! §10.7. The API key is read from the keyring per call and never leaves
//! the core. [`providers`] adds the registry of endpoints and the per-feature
//! routes with backups (§10.1A, ADR-0059): [`resolve_with`] walks a route
//! into a [`Routed`] backend that tries the primary, then each backup, when a
//! call cannot be made.

pub mod claude;
pub mod fixture;
pub mod openai;
pub mod providers;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use super::http::HttpClient;
use crate::db::{Result as DbResult, format_timestamp};
use crate::keyring::Keyring;
use crate::notes::fields::LocalTz;

/// The client a call goes through: `ai.backend` names one of the first
/// three (the built-in backends); `usage_samples.source` records all four
/// for Mode's own calls. `OpenAiCompat` is a remote OpenAI-compatible
/// provider of the registry (§10.1A), never the selected backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    ClaudeApi,
    Ollama,
    LmStudio,
    OpenAiCompat,
}

impl BackendKind {
    /// The built-in backends `ai.backend` may name (06 §10.1).
    pub const ALL: [BackendKind; 3] = [
        BackendKind::ClaudeApi,
        BackendKind::Ollama,
        BackendKind::LmStudio,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::ClaudeApi => "claude_api",
            BackendKind::Ollama => "ollama",
            BackendKind::LmStudio => "lm_studio",
            BackendKind::OpenAiCompat => "openai_compat",
        }
    }

    /// A built-in backend by its `ai.backend` value.
    pub fn parse(s: &str) -> Option<BackendKind> {
        BackendKind::ALL.iter().copied().find(|b| b.as_str() == s)
    }

    pub fn is_local(self) -> bool {
        matches!(self, BackendKind::Ollama | BackendKind::LmStudio)
    }

    /// Counts toward the monthly token budget (06 §10.5): the paid, remote kinds.
    pub fn is_budgeted(self) -> bool {
        !self.is_local()
    }

    /// The name in user-facing text ("Can't reach Ollama").
    pub fn label(self) -> &'static str {
        match self {
            BackendKind::ClaudeApi => "Claude API",
            BackendKind::Ollama => "Ollama",
            BackendKind::LmStudio => "LM Studio",
            BackendKind::OpenAiCompat => "OpenAI-compatible",
        }
    }

    /// The default `ai.local_url` of a local backend.
    pub fn default_url(self) -> &'static str {
        match self {
            BackendKind::LmStudio => "http://127.0.0.1:1234",
            _ => "http://127.0.0.1:11434",
        }
    }
}

/// `usage_samples.purpose`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Scoring,
    Summary,
    Chat,
    Title,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Scoring => "scoring",
            Purpose::Summary => "summary",
            Purpose::Chat => "chat",
            Purpose::Title => "title",
        }
    }
}

/// `output_config.effort` (06 §10.2): `low` for scoring and titles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        }
    }
}

/// One non-streaming call.
#[derive(Debug, Clone, PartialEq)]
pub struct AiRequest {
    pub purpose: Purpose,
    pub model: String,
    /// The cached prefix (rubric + examples for scoring).
    pub system: String,
    /// The volatile part after the cache breakpoint.
    pub user: String,
    /// Structured output: the JSON schema the reply must match.
    pub json_schema: Option<Value>,
    pub max_tokens: u32,
    pub effort: Effort,
    pub timeout: Duration,
}

/// Tokens of one call (every column of `usage_samples`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    /// The part of `cache_write_tokens` written to the 1-hour cache (ADR-0051).
    pub cache_write_1h_tokens: i64,
}

impl Usage {
    pub fn total(&self) -> i64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiResponse {
    pub text: String,
    pub usage: Option<Usage>,
    /// API `request-id` / message id (`usage_samples.turn_ref`).
    pub request_id: Option<String>,
    pub stop_reason: Option<String>,
    pub model: String,
    /// The client that answered (`usage_samples.source`): a backup of a
    /// route may differ from the primary's kind.
    pub source: BackendKind,
}

/// A call failure (06 §10.2 "Errors", §10.7 states).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AiError {
    #[error("Add your Anthropic API key")]
    NoKey,
    #[error("API key rejected")]
    InvalidKey,
    #[error("Can't reach {0}")]
    Unreachable(String),
    #[error("rate limited; retry in {retry_after_secs} s")]
    RateLimited { retry_after_secs: u64 },
    #[error("the model declined")]
    Refusal,
    #[error("the backend answered {0}")]
    Server(u16),
    #[error("the reply did not match the schema: {0}")]
    BadOutput(String),
    #[error("Local-only: add a local backend")]
    LocalOnlyBlocked,
    #[error("Monthly AI budget reached")]
    BudgetReached,
    #[error("{0}")]
    Config(String),
}

impl AiError {
    /// A backup on the route is worth trying (06 §10.1A): the endpoint or
    /// the key failed, not the request itself.
    pub fn tries_backup(&self) -> bool {
        match self {
            AiError::NoKey
            | AiError::InvalidKey
            | AiError::Unreachable(_)
            | AiError::RateLimited { .. }
            | AiError::LocalOnlyBlocked
            | AiError::Config(_) => true,
            AiError::Server(s) => *s >= 500,
            AiError::Refusal | AiError::BadOutput(_) | AiError::BudgetReached => false,
        }
    }
}

/// The backend interface.
pub trait AiBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn complete(&self, req: &AiRequest) -> Result<AiResponse, AiError>;
    /// `GET /v1/models` ids (the model picker; the local health check).
    fn list_models(&self) -> Result<Vec<String>, AiError>;
}

// --------------------------------------------------------------- settings

fn timeouts_default() -> Timeouts {
    Timeouts {
        scoring: 30,
        summary: 120,
        chat: 300,
    }
}

/// `ai.timeout_seconds`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timeouts {
    #[serde(default)]
    pub scoring: u64,
    #[serde(default)]
    pub summary: u64,
    #[serde(default)]
    pub chat: u64,
}

/// The `ai.*` settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiSettings {
    pub backend: BackendKind,
    pub local_only: bool,
    pub model_chat: String,
    pub model_scoring: String,
    pub model_summary: String,
    pub local_model_chat: Option<String>,
    pub local_model_scoring: Option<String>,
    pub local_model_summary: Option<String>,
    pub local_url: String,
    pub budget_tokens_month: i64,
    pub scoring_chars: i64,
    pub timeouts: Timeouts,
}

impl Default for AiSettings {
    fn default() -> Self {
        AiSettings {
            backend: BackendKind::ClaudeApi,
            local_only: false,
            model_chat: "claude-opus-5-5".into(),
            model_scoring: "claude-haiku-5-5".into(),
            model_summary: "claude-haiku-5-5".into(),
            local_model_chat: None,
            local_model_scoring: None,
            local_model_summary: None,
            local_url: BackendKind::Ollama.default_url().into(),
            budget_tokens_month: 2_000_000,
            scoring_chars: 500,
            timeouts: timeouts_default(),
        }
    }
}

impl AiSettings {
    pub fn load(conn: &Connection) -> DbResult<AiSettings> {
        use crate::settings::{get_as_in, get_bool_in, get_i64_in, get_string_in};
        let d = AiSettings::default();
        let s = |k: &str| get_string_in(conn, k);
        let t: Timeouts = get_as_in(conn, "ai.timeout_seconds").unwrap_or(d.timeouts);
        Ok(AiSettings {
            backend: s("ai.backend")?
                .and_then(|b| BackendKind::parse(&b))
                .unwrap_or(d.backend),
            local_only: get_bool_in(conn, "ai.local_only")?.unwrap_or(false),
            model_chat: s("ai.model_chat")?.unwrap_or(d.model_chat),
            model_scoring: s("ai.model_scoring")?.unwrap_or(d.model_scoring),
            model_summary: s("ai.model_summary")?.unwrap_or(d.model_summary),
            local_model_chat: s("ai.local_model_chat")?,
            local_model_scoring: s("ai.local_model_scoring")?,
            local_model_summary: s("ai.local_model_summary")?,
            local_url: s("ai.local_url")?
                .filter(|u| !u.trim().is_empty())
                .unwrap_or(d.local_url),
            budget_tokens_month: get_i64_in(conn, "ai.budget_tokens_month")?
                .filter(|n| *n > 0)
                .unwrap_or(d.budget_tokens_month),
            scoring_chars: get_i64_in(conn, "ai.scoring_chars")?
                .filter(|n| *n > 0)
                .unwrap_or(d.scoring_chars),
            timeouts: Timeouts {
                scoring: if t.scoring > 0 { t.scoring } else { 30 },
                summary: if t.summary > 0 { t.summary } else { 120 },
                chat: if t.chat > 0 { t.chat } else { 300 },
            },
        })
    }

    /// Which local backend `ai.local_url` points at: `ai.backend` when it is
    /// local, else by port (1234 → LM Studio, otherwise Ollama).
    pub fn local_kind(&self) -> BackendKind {
        if self.backend.is_local() {
            return self.backend;
        }
        match url::Url::parse(&self.local_url).ok().and_then(|u| u.port()) {
            Some(1234) => BackendKind::LmStudio,
            _ => BackendKind::Ollama,
        }
    }

    pub fn timeout(&self, p: Purpose) -> Duration {
        Duration::from_secs(match p {
            Purpose::Scoring | Purpose::Title => self.timeouts.scoring,
            Purpose::Summary => self.timeouts.summary,
            Purpose::Chat => self.timeouts.chat,
        })
    }

    fn model_for(&self, p: Purpose, local: bool) -> Option<String> {
        if local {
            match p {
                Purpose::Chat => self.local_model_chat.clone(),
                Purpose::Summary => self.local_model_summary.clone(),
                Purpose::Scoring | Purpose::Title => self.local_model_scoring.clone(),
            }
        } else {
            Some(match p {
                Purpose::Chat => self.model_chat.clone(),
                Purpose::Summary => self.model_summary.clone(),
                Purpose::Scoring | Purpose::Title => self.model_scoring.clone(),
            })
        }
    }
}

// --------------------------------------------------------- address policy

/// 06 §10.3 address classes of `ai.local_url`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressClass {
    Loopback,
    /// LAN: "data leaves this computer but not your network".
    Private,
    /// Refused in Settings.
    Public,
    Invalid,
}

/// Classify a local backend URL (literal IPs and `localhost`; other host
/// names are refused because their address cannot be checked up front).
pub fn address_class(raw: &str) -> AddressClass {
    let Ok(u) = url::Url::parse(raw) else {
        return AddressClass::Invalid;
    };
    if !matches!(u.scheme(), "http" | "https") {
        return AddressClass::Invalid;
    }
    match u.host() {
        Some(url::Host::Domain(d)) if d.eq_ignore_ascii_case("localhost") => AddressClass::Loopback,
        Some(url::Host::Domain(_)) => AddressClass::Public,
        Some(url::Host::Ipv4(ip)) => classify_ip(IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => classify_ip(IpAddr::V6(ip)),
        None => AddressClass::Invalid,
    }
}

fn classify_ip(ip: IpAddr) -> AddressClass {
    match ip {
        IpAddr::V4(v) if v.is_loopback() => AddressClass::Loopback,
        IpAddr::V4(v) if v.is_private() || v.is_link_local() => AddressClass::Private,
        IpAddr::V6(v) if v.is_loopback() => AddressClass::Loopback,
        IpAddr::V6(v)
            if (v.segments()[0] & 0xffc0) == 0xfe80 || (v.segments()[0] & 0xfe00) == 0xfc00 =>
        {
            AddressClass::Private
        }
        _ => AddressClass::Public,
    }
}

// ------------------------------------------------------------- selection

/// The backend a call goes to, with its model.
#[derive(Clone)]
pub struct Resolved {
    pub backend: Arc<dyn AiBackend>,
    pub model: String,
    /// The primary's provider (`ai_providers.id`; a built-in's is its kind).
    pub provider_id: String,
    /// The primary's name for status lines ("OpenRouter", "Claude API").
    pub label: String,
    /// The primary's address class (the LAN notice).
    pub address: AddressClass,
    /// Backups behind the primary.
    pub backups: usize,
}

impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved")
            .field("kind", &self.backend.kind())
            .field("model", &self.model)
            .field("provider", &self.provider_id)
            .field("backups", &self.backups)
            .finish()
    }
}

/// Builds backends; tests swap in fixtures.
pub trait BackendFactory: Send + Sync {
    fn claude(&self, api_key: String) -> Arc<dyn AiBackend>;
    fn local(&self, kind: BackendKind, url: &str, token: Option<String>) -> Arc<dyn AiBackend>;
    /// The Anthropic protocol at another address (a gateway, §10.1A); a
    /// bearer token instead of `x-api-key` when `bearer`. Fixtures keep
    /// answering as the Claude API.
    fn claude_at(
        &self,
        api_key: String,
        _base: Option<&str>,
        _bearer: bool,
        _label: &str,
    ) -> Arc<dyn AiBackend> {
        self.claude(api_key)
    }
    /// A remote OpenAI-compatible endpoint. Fixtures answer as the local one.
    fn openai_compat(&self, base: &str, token: Option<String>, _label: &str) -> Arc<dyn AiBackend> {
        self.local(BackendKind::OpenAiCompat, base, token)
    }
}

/// The real factory over an [`HttpClient`].
pub struct HttpBackends(pub Arc<dyn HttpClient>);

impl BackendFactory for HttpBackends {
    fn claude(&self, api_key: String) -> Arc<dyn AiBackend> {
        Arc::new(claude::ClaudeApi::new(Arc::clone(&self.0), api_key))
    }
    fn local(&self, kind: BackendKind, url: &str, token: Option<String>) -> Arc<dyn AiBackend> {
        Arc::new(openai::OpenAiCompat::new(
            Arc::clone(&self.0),
            kind,
            url,
            token,
        ))
    }
    fn claude_at(
        &self,
        api_key: String,
        base: Option<&str>,
        bearer: bool,
        label: &str,
    ) -> Arc<dyn AiBackend> {
        let mut c = claude::ClaudeApi::new(Arc::clone(&self.0), api_key).with_label(label);
        if let Some(b) = base {
            c = c.with_base(b);
        }
        if bearer {
            c = c.with_bearer();
        }
        Arc::new(c)
    }
    fn openai_compat(&self, base: &str, token: Option<String>, label: &str) -> Arc<dyn AiBackend> {
        Arc::new(
            openai::OpenAiCompat::new(Arc::clone(&self.0), BackendKind::OpenAiCompat, base, token)
                .with_label(label),
        )
    }
}

/// The settings, the providers and the routes, read together (one
/// `AiConfig::load` per batch or turn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiConfig {
    pub settings: AiSettings,
    pub providers: Vec<providers::Provider>,
    pub routes: Vec<providers::Route>,
}

impl AiConfig {
    pub fn load(conn: &Connection) -> DbResult<AiConfig> {
        Ok(AiConfig {
            settings: AiSettings::load(conn)?,
            providers: providers::list(conn)?,
            routes: providers::routes(conn)?,
        })
    }

    /// The three built-ins and no routes: the §10.1 behavior of `ai.backend`
    /// (tests, and callers that hold settings alone).
    pub fn builtins(settings: AiSettings) -> AiConfig {
        let mk = |kind: providers::ProviderKind, name: &str, auth: providers::Auth, pos: i64| {
            providers::Provider {
                id: kind.as_str().into(),
                kind,
                name: name.into(),
                base_url: None,
                auth,
                models: Vec::new(),
                extra: providers::ProviderExtra::default(),
                enabled: true,
                builtin: true,
                position: pos,
            }
        };
        AiConfig {
            settings,
            providers: vec![
                mk(
                    providers::ProviderKind::ClaudeApi,
                    "Claude API",
                    providers::Auth::ApiKey,
                    0,
                ),
                mk(
                    providers::ProviderKind::Ollama,
                    "Ollama",
                    providers::Auth::Bearer,
                    1,
                ),
                mk(
                    providers::ProviderKind::LmStudio,
                    "LM Studio",
                    providers::Auth::Bearer,
                    2,
                ),
            ],
            routes: Vec::new(),
        }
    }

    pub fn provider(&self, id: &str) -> Option<&providers::Provider> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// The route of a purpose when it names a provider.
    pub fn route(&self, purpose: Purpose) -> Option<&providers::Route> {
        let p = providers::route_purpose(purpose);
        self.routes
            .iter()
            .find(|r| r.purpose == p && r.provider_id.is_some())
    }

    /// The model a provider runs a purpose on when the route names none:
    /// the settings' per-feature model for a built-in, the first model the
    /// user named for an added provider (the Claude id for an Anthropic
    /// gateway), else `None` (the first listed model, at call time).
    pub fn default_model(&self, p: &providers::Provider, purpose: Purpose) -> Option<String> {
        if p.builtin {
            return self.settings.model_for(purpose, p.kind.is_local());
        }
        if let Some(m) = p.models.first() {
            return Some(m.clone());
        }
        (p.kind == providers::ProviderKind::ClaudeApi)
            .then(|| self.settings.model_for(purpose, false))
            .flatten()
    }
}

/// One link of a route: a provider's client and the model it runs.
#[derive(Clone)]
pub struct Candidate {
    pub provider_id: String,
    pub label: String,
    pub kind: BackendKind,
    /// `None` = the first model the endpoint lists, found at call time.
    pub model: Option<String>,
    pub backend: Arc<dyn AiBackend>,
}

impl Candidate {
    fn model_now(&self) -> Result<String, AiError> {
        match &self.model {
            Some(m) => Ok(m.clone()),
            None => self
                .backend
                .list_models()?
                .into_iter()
                .next()
                .ok_or_else(|| AiError::Unreachable(self.label.clone())),
        }
    }
}

/// A route as one backend (06 §10.1A): the primary answers; when it cannot
/// be called ([`AiError::tries_backup`]) each backup is tried in order with
/// its own model, and the reply says which client answered (`source`).
pub struct Routed {
    chain: Vec<Candidate>,
}

impl Routed {
    pub fn new(chain: Vec<Candidate>) -> Routed {
        debug_assert!(!chain.is_empty());
        Routed { chain }
    }

    pub fn primary(&self) -> &Candidate {
        &self.chain[0]
    }

    pub fn chain(&self) -> &[Candidate] {
        &self.chain
    }
}

impl std::fmt::Debug for Routed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Routed")
            .field(
                "chain",
                &self
                    .chain
                    .iter()
                    .map(|c| (&c.provider_id, c.kind, &c.model))
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl AiBackend for Routed {
    fn kind(&self) -> BackendKind {
        self.chain[0].kind
    }

    fn complete(&self, req: &AiRequest) -> Result<AiResponse, AiError> {
        let mut last: Option<AiError> = None;
        for (i, c) in self.chain.iter().enumerate() {
            let model = if i == 0 && !req.model.is_empty() {
                req.model.clone()
            } else {
                match c.model_now() {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(provider = %c.provider_id, error = %e, "backup has no model");
                        last = Some(e);
                        continue;
                    }
                }
            };
            let r = if model == req.model {
                c.backend.complete(req)
            } else {
                let mut own = req.clone();
                own.model = model;
                c.backend.complete(&own)
            };
            match r {
                Ok(mut resp) => {
                    resp.source = c.kind;
                    if i > 0 {
                        tracing::info!(provider = %c.provider_id, step = i, "a backup answered");
                    }
                    return Ok(resp);
                }
                Err(e) if e.tries_backup() && i + 1 < self.chain.len() => {
                    tracing::warn!(provider = %c.provider_id, error = %e, "trying the next backup");
                    last = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| AiError::Config("no provider on the route".into())))
    }

    fn list_models(&self) -> Result<Vec<String>, AiError> {
        self.chain[0].backend.list_models()
    }
}

/// The client of one provider, or why it cannot be built.
fn candidate(
    config: &AiConfig,
    keyring: &Keyring,
    factory: &dyn BackendFactory,
    p: &providers::Provider,
    model: Option<String>,
    purpose: Purpose,
) -> Result<Candidate, AiError> {
    let settings = &config.settings;
    let Some(kind) = p.backend() else {
        return Err(AiError::Config(format!(
            "{} runs sessions, not Mode's own calls",
            p.name
        )));
    };
    if !p.enabled {
        return Err(AiError::Config(format!("{} is turned off", p.name)));
    }
    let url = p.url(settings);
    let class = p.address(settings);
    let local_address = matches!(class, AddressClass::Loopback | AddressClass::Private);
    if settings.local_only && !local_address {
        return Err(AiError::LocalOnlyBlocked);
    }
    if kind.is_local() && !local_address {
        return Err(AiError::Config(
            "the local backend address is refused".into(),
        ));
    }
    let key = keyring
        .get(&p.key_entry())
        .map_err(|e| AiError::Config(e.to_string()))?
        .filter(|k| !k.trim().is_empty());
    let backend: Arc<dyn AiBackend> = match kind {
        BackendKind::ClaudeApi => {
            let key = key.ok_or(AiError::NoKey)?;
            let base = url.as_deref().filter(|u| *u != claude::BASE_URL);
            factory.claude_at(key, base, p.auth == providers::Auth::Bearer, &p.name)
        }
        BackendKind::OpenAiCompat => {
            let Some(u) = url.as_deref() else {
                return Err(AiError::Config(format!("{} has no address", p.name)));
            };
            factory.openai_compat(u, key, &p.name)
        }
        BackendKind::Ollama | BackendKind::LmStudio => {
            let Some(u) = url.as_deref() else {
                return Err(AiError::Config(format!("{} has no address", p.name)));
            };
            factory.local(kind, u, key)
        }
    };
    let model = model
        .filter(|m| !m.trim().is_empty())
        .or_else(|| config.default_model(p, purpose))
        .filter(|m| !m.trim().is_empty());
    Ok(Candidate {
        provider_id: p.id.clone(),
        label: p.name.clone(),
        kind,
        model,
        backend,
    })
}

/// The providers and models a purpose walks, in order: the route's primary
/// and backups when the route names a provider, else the selected backend of
/// `ai.backend` (under local-only, the local backend regardless, §10.4).
fn targets(config: &AiConfig, purpose: Purpose) -> Vec<providers::Target> {
    let mut out: Vec<providers::Target> = Vec::new();
    if let Some(r) = config.route(purpose)
        && let Some(id) = &r.provider_id
    {
        out.push(providers::Target {
            provider_id: id.clone(),
            model: r.model.clone(),
        });
        out.extend(r.fallbacks.iter().cloned());
    }
    let s = &config.settings;
    let legacy = if s.local_only || s.backend.is_local() {
        s.local_kind()
    } else {
        s.backend
    };
    let legacy_id = legacy.as_str().to_string();
    if !out.iter().any(|t| t.provider_id == legacy_id) && (out.is_empty() || s.local_only) {
        // Nothing routed, or local-only must still find the local backend.
        out.push(providers::Target {
            provider_id: legacy_id,
            model: None,
        });
    }
    out
}

/// Pick the clients for `purpose` (06 §10.1A, §10.4): the route's chain, or
/// the selected backend; every provider that cannot be called is skipped and
/// the first reason is the error when none remains.
pub fn resolve_with(
    config: &AiConfig,
    keyring: &Keyring,
    factory: &dyn BackendFactory,
    purpose: Purpose,
) -> Result<Resolved, AiError> {
    let mut chain: Vec<Candidate> = Vec::new();
    let mut first_error: Option<AiError> = None;
    let mut skipped_remote = false;
    for t in targets(config, purpose) {
        let Some(p) = config.provider(&t.provider_id) else {
            first_error.get_or_insert(AiError::Config(format!("no provider {}", t.provider_id)));
            continue;
        };
        match candidate(config, keyring, factory, p, t.model.clone(), purpose) {
            Ok(c) => chain.push(c),
            Err(AiError::LocalOnlyBlocked) => skipped_remote = true,
            Err(e) => {
                tracing::debug!(provider = %p.id, error = %e, "provider skipped");
                first_error.get_or_insert(e);
            }
        }
    }
    if chain.is_empty() {
        return Err(
            if config.settings.local_only && (skipped_remote || first_error.is_none()) {
                AiError::LocalOnlyBlocked
            } else {
                match first_error {
                    Some(AiError::Unreachable(_)) if config.settings.local_only => {
                        AiError::LocalOnlyBlocked
                    }
                    Some(e) => e,
                    None => AiError::Config("no provider to call".into()),
                }
            },
        );
    }
    let primary = &chain[0];
    let model = match primary.model_now() {
        Ok(m) => m,
        Err(AiError::Unreachable(_)) if config.settings.local_only => {
            return Err(AiError::LocalOnlyBlocked);
        }
        Err(e) => return Err(e),
    };
    let p = config.provider(&primary.provider_id);
    Ok(Resolved {
        model,
        provider_id: primary.provider_id.clone(),
        label: primary.label.clone(),
        address: p
            .map(|p| p.address(&config.settings))
            .unwrap_or(AddressClass::Invalid),
        backups: chain.len() - 1,
        backend: Arc::new(Routed::new(chain)),
    })
}

/// [`resolve_with`] over the built-ins alone (06 §10.4: under
/// `ai.local_only` the local backend regardless of `ai.backend`).
pub fn resolve(
    settings: &AiSettings,
    keyring: &Keyring,
    factory: &dyn BackendFactory,
    purpose: Purpose,
) -> Result<Resolved, AiError> {
    resolve_with(
        &AiConfig::builtins(settings.clone()),
        keyring,
        factory,
        purpose,
    )
}

// ------------------------------------------------------------- the budget

/// Tokens used against the monthly budget and the daily scoring pace cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub used_month: i64,
    pub limit_month: i64,
    pub used_today_scoring: i64,
    /// 10 % of the monthly budget per local day (07 §3.4.5).
    pub pace_cap: i64,
}

impl Budget {
    pub fn month_reached(&self) -> bool {
        self.used_month >= self.limit_month
    }

    pub fn pace_reached(&self) -> bool {
        self.used_today_scoring >= self.pace_cap
    }

    /// Whether a call estimated at `estimate` tokens may run: refused when
    /// it would cross a limit by more than 1 % of the monthly budget
    /// (06 §10.5 "Estimation"); `scoring` also checks the pace cap.
    pub fn allows(&self, estimate: i64, scoring: bool) -> bool {
        let slack = self.limit_month / 100;
        if self.month_reached() || self.used_month + estimate > self.limit_month + slack {
            return false;
        }
        if scoring
            && (self.pace_reached() || self.used_today_scoring + estimate > self.pace_cap + slack)
        {
            return false;
        }
        true
    }

    /// Percent of the monthly budget used (the 80 % warning, `usage.threshold`).
    pub fn percent(&self) -> i64 {
        if self.limit_month <= 0 {
            0
        } else {
            self.used_month * 100 / self.limit_month
        }
    }
}

/// The local month start and day start as UTC timestamps.
pub fn windows(now: OffsetDateTime, tz: &dyn LocalTz) -> (String, String) {
    let today = tz.local_date(now);
    let month_start = today.replace_day(1).unwrap_or(today);
    (
        format_timestamp(tz.local_midnight(month_start)),
        format_timestamp(tz.local_midnight(today)),
    )
}

/// Read the budget (the remote kinds' tokens; local backends are uncounted).
pub fn budget(
    conn: &Connection,
    now: OffsetDateTime,
    tz: &dyn LocalTz,
    limit_month: i64,
) -> DbResult<Budget> {
    let (month, day) = windows(now, tz);
    Ok(Budget {
        used_month: crate::queries::ai_tokens_used(conn, &month, None)?,
        limit_month,
        used_today_scoring: crate::queries::ai_tokens_used(conn, &day, Some("scoring"))?,
        pace_cap: limit_month / 10,
    })
}

/// About 4 characters per token, plus the system prompt and the output.
pub fn estimate_tokens(input_chars: usize, system_chars: usize, expected_output: i64) -> i64 {
    ((input_chars + system_chars) / 4) as i64 + expected_output
}

/// One `usage_samples` row per call (06 §10.1): tokens only, `cost_usd`
/// NULL, de-duplicated on `(source, turn_ref)`.
pub fn record_usage(
    conn: &Connection,
    kind: BackendKind,
    purpose: Purpose,
    model: &str,
    usage: &Usage,
    turn_ref: Option<&str>,
    now: &str,
) -> DbResult<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO usage_samples
           (id, sampled_at, source, purpose, session_id, model, input_tokens, output_tokens,
            cache_read_tokens, cache_write_tokens, cost_usd, turn_ref, created_at, cache_write_1h_tokens)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?2, ?11)",
        params![
            crate::repo::new_id(),
            now,
            kind.as_str(),
            purpose.as_str(),
            model,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
            turn_ref,
            usage.cache_write_1h_tokens
        ],
    )?;
    Ok(n > 0)
}

// ------------------------------------------------------------------ health

/// 06 §10.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Ready,
    NoKey,
    Unreachable,
    InvalidKey,
    BudgetReached,
    LocalOnlyBlocked,
}

/// What Settings → AI and the Chat indicator show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiHealth {
    pub backend: BackendKind,
    /// The provider scoring goes to (`ai_providers.id`) and its name.
    pub provider: String,
    pub label: String,
    pub local_only: bool,
    pub state: HealthState,
    pub message: String,
    /// `loopback` | `private` for a local backend (the LAN notice).
    pub address: Option<AddressClass>,
    pub models: Vec<String>,
    pub tokens_used_month: i64,
    pub budget_tokens_month: i64,
    /// Backups behind the primary on the scoring route.
    pub backups: usize,
}

/// Check the backend scoring goes to (06 §10.7): the key's presence for a
/// remote one (no tokens spent unless `probe_remote`), `/v1/models` for a
/// local backend.
pub fn health(
    config: &AiConfig,
    keyring: &Keyring,
    factory: &dyn BackendFactory,
    budget: &Budget,
    probe_remote: bool,
) -> AiHealth {
    let settings = &config.settings;
    let mut h = AiHealth {
        backend: BackendKind::ClaudeApi,
        provider: BackendKind::ClaudeApi.as_str().into(),
        label: BackendKind::ClaudeApi.label().into(),
        local_only: settings.local_only,
        state: HealthState::Ready,
        message: "Ready".into(),
        address: None,
        models: Vec::new(),
        tokens_used_month: budget.used_month,
        budget_tokens_month: budget.limit_month,
        backups: 0,
    };
    let set = |h: &mut AiHealth, s: HealthState, m: String| {
        h.state = s;
        h.message = m;
    };
    // The primary of the scoring route, named even when it cannot be called.
    let primary_id = targets(config, Purpose::Scoring)
        .into_iter()
        .next()
        .map(|t| t.provider_id);
    let primary = primary_id.as_deref().and_then(|id| config.provider(id));
    if let Some(p) = primary {
        h.provider = p.id.clone();
        h.label = p.name.clone();
        if let Some(k) = p.backend() {
            h.backend = k;
        }
        if p.kind.is_local() || !p.builtin {
            h.address = Some(p.address(settings));
        }
    }
    let r = match resolve_with(config, keyring, factory, Purpose::Scoring) {
        Ok(r) => r,
        Err(AiError::LocalOnlyBlocked) => {
            set(
                &mut h,
                HealthState::LocalOnlyBlocked,
                AiError::LocalOnlyBlocked.to_string(),
            );
            return h;
        }
        Err(AiError::NoKey) => {
            set(&mut h, HealthState::NoKey, AiError::NoKey.to_string());
            return h;
        }
        Err(AiError::InvalidKey) => {
            set(
                &mut h,
                HealthState::InvalidKey,
                AiError::InvalidKey.to_string(),
            );
            return h;
        }
        Err(e) => {
            set(&mut h, HealthState::Unreachable, e.to_string());
            return h;
        }
    };
    h.backend = r.backend.kind();
    h.provider = r.provider_id.clone();
    h.label = r.label.clone();
    h.backups = r.backups;
    let local = r.backend.kind().is_local();
    // The LAN notice: a local backend's class; an added provider's too (a
    // gateway on the private network); the Claude API's own address says nothing.
    h.address = config
        .provider(&r.provider_id)
        .filter(|p| local || !p.builtin)
        .map(|_| r.address);
    if !local {
        if budget.month_reached() {
            set(
                &mut h,
                HealthState::BudgetReached,
                AiError::BudgetReached.to_string(),
            );
            return h;
        }
        if probe_remote {
            match r.backend.list_models() {
                Ok(m) => h.models = m,
                Err(AiError::InvalidKey) => set(
                    &mut h,
                    HealthState::InvalidKey,
                    AiError::InvalidKey.to_string(),
                ),
                Err(e) => set(&mut h, HealthState::Unreachable, e.to_string()),
            }
        }
        return h;
    }
    match r.backend.list_models() {
        Ok(m) => h.models = m,
        Err(_) if settings.local_only => set(
            &mut h,
            HealthState::LocalOnlyBlocked,
            AiError::LocalOnlyBlocked.to_string(),
        ),
        Err(_) => set(
            &mut h,
            HealthState::Unreachable,
            AiError::Unreachable(r.label.clone()).to_string(),
        ),
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_policy() {
        assert_eq!(
            address_class("http://127.0.0.1:11434"),
            AddressClass::Loopback
        );
        assert_eq!(
            address_class("http://localhost:1234"),
            AddressClass::Loopback
        );
        assert_eq!(address_class("http://[::1]:1234"), AddressClass::Loopback);
        assert_eq!(
            address_class("http://192.168.1.20:11434"),
            AddressClass::Private
        );
        assert_eq!(address_class("http://10.0.0.2"), AddressClass::Private);
        assert_eq!(address_class("http://169.254.1.1"), AddressClass::Private);
        assert_eq!(address_class("http://8.8.8.8"), AddressClass::Public);
        assert_eq!(address_class("https://example.com"), AddressClass::Public);
        assert_eq!(address_class("ftp://127.0.0.1"), AddressClass::Invalid);
    }

    #[test]
    fn budget_rules() {
        let b = Budget {
            used_month: 1_000_000,
            limit_month: 2_000_000,
            used_today_scoring: 150_000,
            pace_cap: 200_000,
        };
        assert!(b.allows(5_000, true));
        assert!(b.allows(60_000, false));
        assert!(
            !b.allows(80_000, true),
            "crosses the pace cap by more than 1 %"
        );
        let full = Budget {
            used_month: 2_000_000,
            ..b
        };
        assert!(full.month_reached());
        assert!(!full.allows(1, false));
        assert_eq!(b.percent(), 50);
        assert_eq!(estimate_tokens(4000, 2000, 600), 2100);
    }

    #[test]
    fn local_kind_by_setting_or_port() {
        let mut s = AiSettings::default();
        assert_eq!(s.local_kind(), BackendKind::Ollama);
        s.local_url = "http://127.0.0.1:1234".into();
        assert_eq!(s.local_kind(), BackendKind::LmStudio);
        s.backend = BackendKind::Ollama;
        assert_eq!(s.local_kind(), BackendKind::Ollama);
    }
}
