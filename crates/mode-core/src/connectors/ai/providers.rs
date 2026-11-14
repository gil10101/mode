//! Providers and routes (06 §10.1A, ADR-0059): the endpoints Mode's own
//! features and an agent's sessions may call. A **provider** is a row of
//! `ai_providers`: what it speaks ([`ProviderKind`]), where it is, how its
//! secret is sent and the models the user named. The three built-ins
//! (`claude_api`, `ollama`, `lm_studio`) are the backends of §10.1 and keep
//! their settings and keyring entries; a provider the user adds keeps its
//! secret under `ai:provider:<id>`. A **route** (`ai_routes`) names the
//! provider and model one feature calls and up to three backups tried in
//! order when a call cannot be made. An agent profile may name a
//! session-capable provider; [`session_env`] turns it into the environment
//! Claude Code reads (`ANTHROPIC_BASE_URL`, the cloud switches of 05 §8.1).

use std::collections::BTreeSet;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use super::{AddressClass, AiSettings, BackendKind, Purpose, address_class};
use crate::db::Result;
use crate::keyring::names;
use crate::repo::{self, AiProvider, AiRoute};
use crate::scheduler::actions::{FieldError, ferr};
use crate::scheduler::api::SaveError;

/// At most this many backups on a route.
pub const MAX_FALLBACKS: usize = 3;
pub const MAX_NAME: usize = 60;
pub const MAX_MODELS: usize = 20;

/// What a provider speaks (`ai_providers.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// The Anthropic Messages API: api.anthropic.com or a gateway speaking it.
    #[default]
    ClaudeApi,
    /// An OpenAI-compatible endpoint (OpenRouter, Groq, OpenAI, a proxy).
    OpenAiCompat,
    Ollama,
    LmStudio,
    /// Sessions only: Claude Code's Bedrock switch.
    Bedrock,
    /// Sessions only: Claude Code's Vertex AI switch.
    Vertex,
    /// Sessions only: Claude Code's Microsoft Foundry switch.
    Foundry,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 7] = [
        ProviderKind::ClaudeApi,
        ProviderKind::OpenAiCompat,
        ProviderKind::Ollama,
        ProviderKind::LmStudio,
        ProviderKind::Bedrock,
        ProviderKind::Vertex,
        ProviderKind::Foundry,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::ClaudeApi => "claude_api",
            ProviderKind::OpenAiCompat => "openai_compat",
            ProviderKind::Ollama => "ollama",
            ProviderKind::LmStudio => "lm_studio",
            ProviderKind::Bedrock => "bedrock",
            ProviderKind::Vertex => "vertex",
            ProviderKind::Foundry => "foundry",
        }
    }

    pub fn parse(s: &str) -> Option<ProviderKind> {
        ProviderKind::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    /// The kind's name in user-facing text.
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::ClaudeApi => "Anthropic API",
            ProviderKind::OpenAiCompat => "OpenAI-compatible",
            ProviderKind::Ollama => "Ollama",
            ProviderKind::LmStudio => "LM Studio",
            ProviderKind::Bedrock => "Amazon Bedrock",
            ProviderKind::Vertex => "Google Vertex AI",
            ProviderKind::Foundry => "Microsoft Foundry",
        }
    }

    /// The client Mode's own features use; `None` for a kind only a session can run on.
    pub fn backend(self) -> Option<BackendKind> {
        match self {
            ProviderKind::ClaudeApi => Some(BackendKind::ClaudeApi),
            ProviderKind::OpenAiCompat => Some(BackendKind::OpenAiCompat),
            ProviderKind::Ollama => Some(BackendKind::Ollama),
            ProviderKind::LmStudio => Some(BackendKind::LmStudio),
            ProviderKind::Bedrock | ProviderKind::Vertex | ProviderKind::Foundry => None,
        }
    }

    /// Runs on this computer or the private network (the address policy of §10.3).
    pub fn is_local(self) -> bool {
        matches!(self, ProviderKind::Ollama | ProviderKind::LmStudio)
    }

    /// Claude Code can run a session on it: the Anthropic protocol (which the
    /// local servers expose too) or one of the clouds the CLI switches to.
    pub fn session_capable(self) -> bool {
        !matches!(self, ProviderKind::OpenAiCompat)
    }

    /// The URL a row without `base_url` goes to (the built-in local kinds
    /// read `ai.local_url` first, [`Provider::url`]).
    pub fn default_url(self) -> Option<&'static str> {
        match self {
            ProviderKind::ClaudeApi => Some(super::claude::BASE_URL),
            ProviderKind::Ollama => Some(BackendKind::Ollama.default_url()),
            ProviderKind::LmStudio => Some(BackendKind::LmStudio.default_url()),
            _ => None,
        }
    }

    /// The ways a kind takes its secret, the first being the default.
    pub fn auths(self) -> &'static [Auth] {
        match self {
            ProviderKind::ClaudeApi => &[Auth::ApiKey, Auth::Bearer],
            ProviderKind::OpenAiCompat => &[Auth::Bearer, Auth::None],
            ProviderKind::Ollama | ProviderKind::LmStudio => &[Auth::Bearer, Auth::None],
            ProviderKind::Bedrock => &[Auth::Bearer, Auth::None],
            ProviderKind::Vertex => &[Auth::None, Auth::Bearer],
            ProviderKind::Foundry => &[Auth::ApiKey, Auth::Bearer],
        }
    }
}

/// How the keyring secret reaches the endpoint (`ai_providers.auth`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Auth {
    None,
    /// `x-api-key` (the Anthropic API), `ANTHROPIC_API_KEY` for a session.
    #[default]
    ApiKey,
    /// `Authorization: Bearer`, `ANTHROPIC_AUTH_TOKEN` for a session.
    Bearer,
}

impl Auth {
    pub fn as_str(self) -> &'static str {
        match self {
            Auth::None => "none",
            Auth::ApiKey => "api_key",
            Auth::Bearer => "bearer",
        }
    }

    pub fn parse(s: &str) -> Option<Auth> {
        [Auth::None, Auth::ApiKey, Auth::Bearer]
            .into_iter()
            .find(|a| a.as_str() == s)
    }
}

/// Kind-specific fields (`ai_providers.extra_json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderExtra {
    /// Bedrock `AWS_REGION`, Vertex `CLOUD_ML_REGION`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Vertex `ANTHROPIC_VERTEX_PROJECT_ID`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Foundry `ANTHROPIC_FOUNDRY_RESOURCE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    /// `ANTHROPIC_CUSTOM_HEADERS` for a session (`Name: value` lines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_headers: Option<String>,
}

/// A provider as the core reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub kind: ProviderKind,
    pub name: String,
    /// The row's own address; `None` = the kind's default ([`Provider::url`]).
    pub base_url: Option<String>,
    pub auth: Auth,
    /// The models the user named; the first is the default.
    pub models: Vec<String>,
    pub extra: ProviderExtra,
    pub enabled: bool,
    pub builtin: bool,
    pub position: i64,
}

impl Provider {
    fn of(r: &AiProvider) -> Provider {
        Provider {
            id: r.id.clone(),
            kind: ProviderKind::parse(&r.kind).unwrap_or_default(),
            name: r.name.clone(),
            base_url: r.base_url.clone(),
            auth: Auth::parse(&r.auth).unwrap_or_default(),
            models: serde_json::from_str(&r.models_json).unwrap_or_default(),
            extra: serde_json::from_str(&r.extra_json).unwrap_or_default(),
            enabled: r.enabled,
            builtin: r.builtin,
            position: r.position,
        }
    }

    /// The keyring entry holding its secret (06 §5): the built-ins keep the
    /// two entries of §10.2 / §10.3, every other provider its own.
    pub fn key_entry(&self) -> String {
        if self.builtin {
            return match self.kind {
                ProviderKind::ClaudeApi => names::AI_ANTHROPIC_API_KEY.to_string(),
                _ => names::AI_LOCAL_TOKEN.to_string(),
            };
        }
        names::ai_provider(&self.id)
    }

    /// Where its calls go: the row's address; for a built-in local kind
    /// `ai.local_url` when that kind is the selected local backend; else the
    /// kind's default. `None` for a cloud kind without an address.
    pub fn url(&self, settings: &AiSettings) -> Option<String> {
        if let Some(u) = self
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|u| !u.is_empty())
        {
            return Some(u.trim_end_matches('/').to_string());
        }
        if self.builtin
            && self.kind.is_local()
            && settings.local_kind().as_str() == self.kind.as_str()
        {
            return Some(settings.local_url.trim_end_matches('/').to_string());
        }
        self.kind.default_url().map(str::to_string)
    }

    /// The address class of [`Provider::url`] (`Invalid` without one).
    pub fn address(&self, settings: &AiSettings) -> AddressClass {
        self.url(settings)
            .map(|u| address_class(&u))
            .unwrap_or(AddressClass::Invalid)
    }

    /// On this computer or the private network: allowed under local-only.
    pub fn is_local_address(&self, settings: &AiSettings) -> bool {
        matches!(
            self.address(settings),
            AddressClass::Loopback | AddressClass::Private
        )
    }

    /// Mode's own features can call it (a session-only kind cannot).
    pub fn backend(&self) -> Option<BackendKind> {
        self.kind.backend()
    }
}

/// What the editor saves (`ai_provider_save`); `id` empty on a new provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderDraft {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub kind: ProviderKind,
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub extra: ProviderExtra,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl Default for ProviderDraft {
    /// An Anthropic-protocol provider, on, taking an API key.
    fn default() -> Self {
        ProviderDraft {
            id: None,
            kind: ProviderKind::ClaudeApi,
            name: String::new(),
            base_url: None,
            auth: Auth::ApiKey,
            models: Vec::new(),
            extra: ProviderExtra::default(),
            enabled: true,
        }
    }
}

impl ProviderDraft {
    /// The draft of a stored provider.
    pub fn of(p: &Provider) -> ProviderDraft {
        ProviderDraft {
            id: Some(p.id.clone()),
            kind: p.kind,
            name: p.name.clone(),
            base_url: p.base_url.clone(),
            auth: p.auth,
            models: p.models.clone(),
            extra: p.extra.clone(),
            enabled: p.enabled,
        }
    }
}

/// Every provider, built-ins first, then by position.
pub fn list(conn: &Connection) -> Result<Vec<Provider>> {
    let rows = repo::query::<AiProvider>(conn, "1 ORDER BY builtin DESC, position, name", &[])?;
    Ok(rows.iter().map(Provider::of).collect())
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Provider>> {
    Ok(repo::get_by_id::<AiProvider>(conn, id)?
        .as_ref()
        .map(Provider::of))
}

fn clean(s: &Option<String>) -> Option<String> {
    s.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// The inline errors of a draft (03 §8.14 "Providers"); empty when it can be saved.
pub fn validate(d: &ProviderDraft, existing: Option<&Provider>) -> Vec<FieldError> {
    let mut out = Vec::new();
    let name = d.name.trim();
    if name.is_empty() {
        out.push(ferr("name", "Give the provider a name."));
    } else if name.chars().count() > MAX_NAME {
        out.push(ferr("name", format!("At most {MAX_NAME} characters.")));
    }
    if let Some(e) = existing
        && e.kind != d.kind
    {
        out.push(ferr("kind", "A provider keeps its kind; add another one."));
    }
    if !d.kind.auths().contains(&d.auth) {
        out.push(ferr(
            "auth",
            format!("{} takes {}.", d.kind.label(), auths_text(d.kind)),
        ));
    }
    let builtin = existing.is_some_and(|e| e.builtin);
    match clean(&d.base_url) {
        Some(u) if !builtin => match address_class(&u) {
            AddressClass::Invalid => out.push(ferr("base_url", "Expects an http:// or https:// address.")),
            AddressClass::Public if d.kind.is_local() => out.push(ferr(
                "base_url",
                "A local server must be on this computer or your private network.",
            )),
            AddressClass::Public if u.starts_with("http://") => out.push(ferr(
                "base_url",
                "A remote endpoint needs https:// (http:// only on this computer or your private network).",
            )),
            _ => {}
        },
        Some(_) => {}
        None => {
            if d.kind == ProviderKind::OpenAiCompat && !builtin {
                out.push(ferr("base_url", "Give the endpoint's address."));
            }
        }
    }
    match d.kind {
        ProviderKind::Bedrock if clean(&d.extra.region).is_none() => {
            out.push(ferr("extra.region", "Give the AWS region."));
        }
        ProviderKind::Vertex => {
            if clean(&d.extra.region).is_none() {
                out.push(ferr("extra.region", "Give the Vertex region."));
            }
            if clean(&d.extra.project_id).is_none() {
                out.push(ferr(
                    "extra.project_id",
                    "Give the Google Cloud project id.",
                ));
            }
        }
        ProviderKind::Foundry
            if clean(&d.extra.resource).is_none() && clean(&d.base_url).is_none() =>
        {
            out.push(ferr(
                "extra.resource",
                "Give the Foundry resource name or an address.",
            ));
        }
        _ => {}
    }
    if d.models.len() > MAX_MODELS {
        out.push(ferr("models", format!("At most {MAX_MODELS} models.")));
    }
    let mut seen = BTreeSet::new();
    for m in &d.models {
        let m = m.trim();
        if m.is_empty() {
            out.push(ferr("models", "A model is empty."));
        } else if !seen.insert(m.to_string()) {
            out.push(ferr("models", format!("{m} is listed twice.")));
        }
    }
    if let Some(h) = clean(&d.extra.custom_headers)
        && h.lines().any(|l| !l.trim().is_empty() && !l.contains(':'))
    {
        out.push(ferr("extra.custom_headers", "One `Name: value` per line."));
    }
    out
}

fn auths_text(kind: ProviderKind) -> String {
    kind.auths()
        .iter()
        .map(|a| match a {
            Auth::None => "no secret",
            Auth::ApiKey => "an API key",
            Auth::Bearer => "a bearer token",
        })
        .collect::<Vec<_>>()
        .join(" or ")
}

fn invalid(fields: Vec<FieldError>) -> SaveError {
    SaveError {
        code: "invalid".into(),
        message: "The provider cannot be saved as it is.".into(),
        fields,
    }
}

/// Insert or update (`id` known) a provider; the saved row back. A built-in
/// keeps its kind and address (its address is `ai.local_url`, 06 §10.3).
pub fn save(
    conn: &Connection,
    d: &ProviderDraft,
    now: &str,
) -> std::result::Result<Provider, SaveError> {
    let existing = match d.id.as_deref().map(str::trim).filter(|i| !i.is_empty()) {
        Some(id) => get(conn, id)?,
        None => None,
    };
    if d.id.as_deref().is_some_and(|i| !i.trim().is_empty()) && existing.is_none() {
        return Err(SaveError {
            code: "not_found".into(),
            message: "The provider is gone.".into(),
            fields: Vec::new(),
        });
    }
    let errors = validate(d, existing.as_ref());
    if !errors.is_empty() {
        return Err(invalid(errors));
    }
    let builtin = existing.as_ref().is_some_and(|e| e.builtin);
    let id = existing
        .as_ref()
        .map(|e| e.id.clone())
        .unwrap_or_else(repo::new_id);
    let position = match &existing {
        Some(e) => e.position,
        None => next_position(conn)?,
    };
    let models: Vec<String> = d
        .models
        .iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect();
    let extra = ProviderExtra {
        region: clean(&d.extra.region),
        project_id: clean(&d.extra.project_id),
        resource: clean(&d.extra.resource),
        custom_headers: clean(&d.extra.custom_headers),
    };
    let row = AiProvider {
        id: id.clone(),
        kind: existing
            .as_ref()
            .map(|e| e.kind)
            .unwrap_or(d.kind)
            .as_str()
            .into(),
        name: d.name.trim().to_string(),
        base_url: if builtin {
            None
        } else {
            clean(&d.base_url).map(|u| u.trim_end_matches('/').to_string())
        },
        auth: d.auth.as_str().into(),
        models_json: serde_json::to_string(&models).unwrap_or_else(|_| "[]".into()),
        extra_json: serde_json::to_string(&extra).unwrap_or_else(|_| "{}".into()),
        enabled: d.enabled,
        builtin,
        position,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    write_row(conn, &row, existing.is_some())?;
    Ok(Provider::of(&row))
}

fn next_position(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM ai_providers",
        [],
        |r| r.get::<_, i64>(0),
    )?)
}

fn write_row(conn: &Connection, row: &AiProvider, update: bool) -> Result<()> {
    if update {
        conn.execute(
            "UPDATE ai_providers SET name = ?2, base_url = ?3, auth = ?4, models_json = ?5, extra_json = ?6,
                    enabled = ?7, updated_at = ?8 WHERE id = ?1",
            params![
                row.id,
                row.name,
                row.base_url,
                row.auth,
                row.models_json,
                row.extra_json,
                row.enabled,
                row.updated_at
            ],
        )?;
    } else {
        repo::insert(conn, row)?;
    }
    Ok(())
}

/// Remove a provider the user added: the routes drop it (a primary becomes
/// the selected backend, a backup disappears), the agents that ran on it run
/// on Claude Code's own sign-in. `Ok(false)` for an unknown id; a built-in
/// is refused.
pub fn delete(conn: &Connection, id: &str) -> std::result::Result<bool, SaveError> {
    let Some(p) = get(conn, id)? else {
        return Ok(false);
    };
    if p.builtin {
        return Err(SaveError {
            code: "invalid".into(),
            message: format!("{} is built in and stays.", p.name),
            fields: Vec::new(),
        });
    }
    let now = crate::db::now_rfc3339();
    for mut r in routes(conn)? {
        let before = r.fallbacks.len();
        r.fallbacks.retain(|t| t.provider_id != id);
        let primary = r.provider_id.as_deref() == Some(id);
        if primary {
            r.provider_id = None;
            r.model = None;
        }
        if primary || r.fallbacks.len() != before {
            write_route(conn, &r, &now)?;
        }
    }
    Ok(remove_row(conn, id)?)
}

fn remove_row(conn: &Connection, id: &str) -> Result<bool> {
    conn.execute(
        "UPDATE agent_profiles SET provider_id = NULL WHERE provider_id = ?1",
        params![id],
    )?;
    repo::delete_by_id::<AiProvider>(conn, id)
}

// ------------------------------------------------------------------ routes

/// One provider and model to try (`ai_routes.fallbacks_json` entries).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub provider_id: String,
    /// `None` = the provider's default model for the purpose.
    #[serde(default)]
    pub model: Option<String>,
}

/// What one feature calls (`ai_routes`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub purpose: Purpose,
    /// `None` = the selected backend of `ai.backend` with its `ai.model_*`.
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Tried in order when the primary cannot be called.
    #[serde(default)]
    pub fallbacks: Vec<Target>,
}

/// The purposes a route exists for (titles ride on scoring).
pub const ROUTED: [Purpose; 3] = [Purpose::Chat, Purpose::Scoring, Purpose::Summary];

/// The purpose a route stands for.
pub fn route_purpose(p: Purpose) -> Purpose {
    match p {
        Purpose::Title => Purpose::Scoring,
        other => other,
    }
}

/// One route per purpose: the stored row, else the default (the selected backend).
pub fn routes(conn: &Connection) -> Result<Vec<Route>> {
    let rows = repo::query::<AiRoute>(conn, "1", &[])?;
    Ok(ROUTED
        .iter()
        .map(|p| {
            rows.iter()
                .find(|r| r.purpose == p.as_str())
                .map(|r| Route {
                    purpose: *p,
                    provider_id: r.provider_id.clone(),
                    model: r.model.clone(),
                    fallbacks: serde_json::from_str(&r.fallbacks_json).unwrap_or_default(),
                })
                .unwrap_or(Route {
                    purpose: *p,
                    provider_id: None,
                    model: None,
                    fallbacks: Vec::new(),
                })
        })
        .collect())
}

fn write_route(conn: &Connection, r: &Route, now: &str) -> Result<()> {
    let row = AiRoute {
        purpose: r.purpose.as_str().into(),
        provider_id: r.provider_id.clone(),
        model: r.model.clone(),
        fallbacks_json: serde_json::to_string(&r.fallbacks).unwrap_or_else(|_| "[]".into()),
        updated_at: now.to_string(),
    };
    repo::upsert(conn, &row, &["purpose"])?;
    Ok(())
}

/// The inline errors of a route; empty when it can be saved.
pub fn validate_route(r: &Route, providers: &[Provider]) -> Vec<FieldError> {
    let mut out = Vec::new();
    if !ROUTED.contains(&r.purpose) {
        out.push(ferr(
            "purpose",
            "Routes exist for chat, scoring and summaries.",
        ));
    }
    let callable = |id: &str| -> std::result::Result<(), String> {
        let Some(p) = providers.iter().find(|p| p.id == id) else {
            return Err("No such provider.".into());
        };
        if p.backend().is_none() {
            return Err(format!(
                "{} can run sessions, not Mode's own calls.",
                p.name
            ));
        }
        if !p.enabled {
            return Err(format!("{} is turned off.", p.name));
        }
        Ok(())
    };
    if let Some(id) = r.provider_id.as_deref()
        && let Err(e) = callable(id)
    {
        out.push(ferr("provider_id", e));
    }
    if r.fallbacks.len() > MAX_FALLBACKS {
        out.push(ferr(
            "fallbacks",
            format!("At most {MAX_FALLBACKS} backups."),
        ));
    }
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    if let Some(id) = &r.provider_id {
        seen.insert((id.clone(), r.model.clone().unwrap_or_default()));
    }
    for t in &r.fallbacks {
        if let Err(e) = callable(&t.provider_id) {
            out.push(ferr("fallbacks", e));
        } else if !seen.insert((t.provider_id.clone(), t.model.clone().unwrap_or_default())) {
            out.push(ferr("fallbacks", "A backup is listed twice."));
        }
    }
    out
}

/// Store a route (`ai_route_set`); the stored route back.
pub fn route_set(conn: &Connection, r: &Route, now: &str) -> std::result::Result<Route, SaveError> {
    let providers = list(conn)?;
    let errors = validate_route(r, &providers);
    if !errors.is_empty() {
        return Err(SaveError {
            code: "invalid".into(),
            message: "The route cannot be saved as it is.".into(),
            fields: errors,
        });
    }
    let clean_model = |m: &Option<String>| clean(m);
    let route = Route {
        purpose: r.purpose,
        provider_id: clean(&r.provider_id),
        model: if r.provider_id.is_some() {
            clean_model(&r.model)
        } else {
            None
        },
        fallbacks: r
            .fallbacks
            .iter()
            .map(|t| Target {
                provider_id: t.provider_id.trim().to_string(),
                model: clean_model(&t.model),
            })
            .collect(),
    };
    write_route(conn, &route, now)?;
    Ok(route)
}

// ------------------------------------------------------------ session env

/// The environment a session on `p` gets (05 §8.1 "Environment"; Claude
/// Code's own variables): the Anthropic protocol through `ANTHROPIC_BASE_URL`
/// with the secret as `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN`, or one of
/// the cloud switches with its region, project or resource. `url` is
/// [`Provider::url`]; `key` the keyring value, never logged.
pub fn session_env(p: &Provider, url: Option<&str>, key: Option<&str>) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = Vec::new();
    let mut set = |k: &str, v: &str| env.push((k.to_string(), v.to_string()));
    let key = key.map(str::trim).filter(|k| !k.is_empty());
    match p.kind {
        ProviderKind::ClaudeApi => {
            if let Some(u) = url
                && u != super::claude::BASE_URL
            {
                set("ANTHROPIC_BASE_URL", u);
            }
            if let Some(k) = key {
                match p.auth {
                    Auth::Bearer => set("ANTHROPIC_AUTH_TOKEN", k),
                    _ => set("ANTHROPIC_API_KEY", k),
                }
            }
        }
        ProviderKind::Ollama | ProviderKind::LmStudio => {
            if let Some(u) = url {
                set("ANTHROPIC_BASE_URL", u);
            }
            // The CLI wants some credential even when the server checks none.
            set("ANTHROPIC_AUTH_TOKEN", key.unwrap_or("local"));
        }
        ProviderKind::Bedrock => {
            set("CLAUDE_CODE_USE_BEDROCK", "1");
            if let Some(r) = p.extra.region.as_deref() {
                set("AWS_REGION", r);
            }
            if let Some(u) = url {
                set("ANTHROPIC_BEDROCK_BASE_URL", u);
            }
            match key {
                Some(k) => set("AWS_BEARER_TOKEN_BEDROCK", k),
                None if url.is_some() => set("CLAUDE_CODE_SKIP_BEDROCK_AUTH", "1"),
                None => {}
            }
        }
        ProviderKind::Vertex => {
            set("CLAUDE_CODE_USE_VERTEX", "1");
            if let Some(r) = p.extra.region.as_deref() {
                set("CLOUD_ML_REGION", r);
            }
            if let Some(pr) = p.extra.project_id.as_deref() {
                set("ANTHROPIC_VERTEX_PROJECT_ID", pr);
            }
            if let Some(u) = url {
                set("ANTHROPIC_VERTEX_BASE_URL", u);
            }
            if let Some(k) = key {
                set("ANTHROPIC_AUTH_TOKEN", k);
                set("CLAUDE_CODE_SKIP_VERTEX_AUTH", "1");
            } else if url.is_some() {
                set("CLAUDE_CODE_SKIP_VERTEX_AUTH", "1");
            }
        }
        ProviderKind::Foundry => {
            set("CLAUDE_CODE_USE_FOUNDRY", "1");
            if let Some(r) = p.extra.resource.as_deref() {
                set("ANTHROPIC_FOUNDRY_RESOURCE", r);
            }
            if let Some(u) = url {
                set("ANTHROPIC_FOUNDRY_BASE_URL", u);
            }
            if let Some(k) = key {
                match p.auth {
                    Auth::Bearer => set("ANTHROPIC_FOUNDRY_AUTH_TOKEN", k),
                    _ => set("ANTHROPIC_FOUNDRY_API_KEY", k),
                }
            }
        }
        ProviderKind::OpenAiCompat => {}
    }
    if let Some(h) = p.extra.custom_headers.as_deref()
        && p.kind != ProviderKind::OpenAiCompat
    {
        set("ANTHROPIC_CUSTOM_HEADERS", h);
    }
    env
}

/// The variable names [`session_env`] may set (the relay's allow-list of 05
/// §9.2 never forwards them).
pub const SESSION_ENV_NAMES: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_CUSTOM_HEADERS",
    "CLAUDE_CODE_USE_BEDROCK",
    "AWS_REGION",
    "ANTHROPIC_BEDROCK_BASE_URL",
    "AWS_BEARER_TOKEN_BEDROCK",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "CLAUDE_CODE_USE_VERTEX",
    "CLOUD_ML_REGION",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "ANTHROPIC_VERTEX_BASE_URL",
    "CLAUDE_CODE_SKIP_VERTEX_AUTH",
    "CLAUDE_CODE_USE_FOUNDRY",
    "ANTHROPIC_FOUNDRY_RESOURCE",
    "ANTHROPIC_FOUNDRY_BASE_URL",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_FOUNDRY_AUTH_TOKEN",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(kind: ProviderKind, auth: Auth) -> Provider {
        Provider {
            id: "p1".into(),
            kind,
            name: "Gateway".into(),
            base_url: None,
            auth,
            models: vec![],
            extra: ProviderExtra::default(),
            enabled: true,
            builtin: false,
            position: 0,
        }
    }

    #[test]
    fn kinds_round_trip_and_say_what_they_can_do() {
        for k in ProviderKind::ALL {
            assert_eq!(ProviderKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(ProviderKind::Bedrock.backend(), None);
        assert_eq!(
            ProviderKind::OpenAiCompat.backend(),
            Some(BackendKind::OpenAiCompat)
        );
        assert!(!ProviderKind::OpenAiCompat.session_capable());
        assert!(ProviderKind::Ollama.session_capable());
        assert!(ProviderKind::LmStudio.is_local());
    }

    #[test]
    fn the_session_environment_per_kind() {
        let p = provider(ProviderKind::ClaudeApi, Auth::Bearer);
        let env = session_env(&p, Some("https://openrouter.ai/api"), Some("sk-or-1"));
        assert_eq!(
            env,
            vec![
                (
                    "ANTHROPIC_BASE_URL".to_string(),
                    "https://openrouter.ai/api".to_string()
                ),
                ("ANTHROPIC_AUTH_TOKEN".to_string(), "sk-or-1".to_string()),
            ]
        );
        // The Claude API itself at its own address sets only the key.
        let p = provider(ProviderKind::ClaudeApi, Auth::ApiKey);
        assert_eq!(
            session_env(&p, Some(super::super::claude::BASE_URL), Some("sk-ant-1")),
            vec![("ANTHROPIC_API_KEY".to_string(), "sk-ant-1".to_string())]
        );
        // A local server: the base URL and a placeholder credential.
        let p = provider(ProviderKind::Ollama, Auth::None);
        assert_eq!(
            session_env(&p, Some("http://127.0.0.1:11434"), None),
            vec![
                (
                    "ANTHROPIC_BASE_URL".to_string(),
                    "http://127.0.0.1:11434".to_string()
                ),
                ("ANTHROPIC_AUTH_TOKEN".to_string(), "local".to_string()),
            ]
        );
        let mut p = provider(ProviderKind::Bedrock, Auth::Bearer);
        p.extra.region = Some("us-east-1".into());
        let env = session_env(&p, None, Some("abt-1"));
        assert!(env.contains(&("CLAUDE_CODE_USE_BEDROCK".to_string(), "1".to_string())));
        assert!(env.contains(&("AWS_REGION".to_string(), "us-east-1".to_string())));
        assert!(env.contains(&("AWS_BEARER_TOKEN_BEDROCK".to_string(), "abt-1".to_string())));
        let mut p = provider(ProviderKind::Vertex, Auth::None);
        p.extra.region = Some("us-east5".into());
        p.extra.project_id = Some("acme-ai".into());
        let env = session_env(&p, None, None);
        assert!(env.contains(&("CLAUDE_CODE_USE_VERTEX".to_string(), "1".to_string())));
        assert!(env.contains(&(
            "ANTHROPIC_VERTEX_PROJECT_ID".to_string(),
            "acme-ai".to_string()
        )));
        assert!(!env.iter().any(|(k, _)| k == "CLAUDE_CODE_SKIP_VERTEX_AUTH"));
        let mut p = provider(ProviderKind::Foundry, Auth::ApiKey);
        p.extra.resource = Some("acme-foundry".into());
        let env = session_env(&p, None, Some("fk-1"));
        assert!(env.contains(&(
            "ANTHROPIC_FOUNDRY_RESOURCE".to_string(),
            "acme-foundry".to_string()
        )));
        assert!(env.contains(&("ANTHROPIC_FOUNDRY_API_KEY".to_string(), "fk-1".to_string())));
        assert!(
            session_env(
                &provider(ProviderKind::OpenAiCompat, Auth::Bearer),
                Some("https://x"),
                Some("k")
            )
            .is_empty()
        );
        for (k, _) in session_env(&p, Some("https://g"), Some("k")) {
            assert!(SESSION_ENV_NAMES.contains(&k.as_str()), "{k} is listed");
        }
    }

    #[test]
    fn drafts_are_validated_per_kind() {
        let d = ProviderDraft {
            kind: ProviderKind::OpenAiCompat,
            name: "OpenRouter".into(),
            auth: Auth::Bearer,
            ..Default::default()
        };
        assert_eq!(
            validate(&d, None)
                .iter()
                .map(|e| e.field.as_str())
                .collect::<Vec<_>>(),
            ["base_url"]
        );
        let d = ProviderDraft {
            base_url: Some("http://api.example.com/v1".into()),
            ..d
        };
        assert_eq!(
            validate(&d, None)[0].field,
            "base_url",
            "http on a public host"
        );
        let d = ProviderDraft {
            base_url: Some("https://openrouter.ai/api/v1".into()),
            models: vec!["a".into(), "a".into()],
            ..d
        };
        assert_eq!(validate(&d, None)[0].message, "a is listed twice.");
        let d = ProviderDraft {
            kind: ProviderKind::Ollama,
            name: "Box".into(),
            base_url: Some("http://203.0.113.9:11434".into()),
            auth: Auth::None,
            ..Default::default()
        };
        assert!(validate(&d, None)[0].message.contains("private network"));
        let d = ProviderDraft {
            kind: ProviderKind::Vertex,
            name: "Vertex".into(),
            auth: Auth::None,
            ..Default::default()
        };
        assert_eq!(
            validate(&d, None)
                .iter()
                .map(|e| e.field.as_str())
                .collect::<Vec<_>>(),
            ["extra.region", "extra.project_id"]
        );
        let d = ProviderDraft {
            kind: ProviderKind::ClaudeApi,
            name: "Second key".into(),
            auth: Auth::None,
            ..Default::default()
        };
        assert_eq!(validate(&d, None)[0].field, "auth");
    }
}
