//! The Claude API backend (06 §10.2): `POST /v1/messages` from the core
//! with reqwest + rustls; `x-api-key` from the keyring entry
//! `ai:anthropic_api_key`; `anthropic-version: 2026-10-21`; the system
//! prompt marked `cache_control: ephemeral` (the batch goes after the
//! breakpoint); `output_config.effort` and, for structured replies,
//! `output_config.format` with a JSON schema; no sampling parameters, no
//! `thinking` (adaptive is the default), no prefill. `GET /v1/models` is the
//! key check (no tokens). The model id is configuration (`ai.model_*`).

use std::sync::Arc;

use serde_json::{Value, json};

use super::{AiBackend, AiError, AiRequest, AiResponse, BackendKind, Usage};
use crate::connectors::http::{HttpClient, HttpError, HttpRequest};

pub const BASE_URL: &str = "https://api.anthropic.com";
pub const API_VERSION: &str = "2026-10-21";

pub struct ClaudeApi {
    http: Arc<dyn HttpClient>,
    key: String,
    base: String,
    /// `Authorization: Bearer` instead of `x-api-key` (a gateway, 06 §10.1A).
    bearer: bool,
    /// The provider's name in "Can't reach …".
    label: String,
}

impl std::fmt::Debug for ClaudeApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeApi")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

/// The schema as the API's structured outputs accept it: numeric and
/// string-length constraints are removed (Mode validates them itself).
pub fn api_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(m) => Value::Object(
            m.iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "minimum"
                            | "maximum"
                            | "exclusiveMinimum"
                            | "exclusiveMaximum"
                            | "multipleOf"
                            | "minLength"
                            | "maxLength"
                            | "minItems"
                            | "maxItems"
                    )
                })
                .map(|(k, v)| (k.clone(), api_schema(v)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(api_schema).collect()),
        other => other.clone(),
    }
}

impl ClaudeApi {
    pub fn new(http: Arc<dyn HttpClient>, key: String) -> ClaudeApi {
        ClaudeApi {
            http,
            key,
            base: BASE_URL.into(),
            bearer: false,
            label: BackendKind::ClaudeApi.label().into(),
        }
    }

    pub fn with_base(mut self, base: &str) -> ClaudeApi {
        self.base = base.trim_end_matches('/').to_string();
        self
    }

    /// Send the key as a bearer token (gateways that take one).
    pub fn with_bearer(mut self) -> ClaudeApi {
        self.bearer = true;
        self
    }

    pub fn with_label(mut self, label: &str) -> ClaudeApi {
        if !label.trim().is_empty() {
            self.label = label.trim().to_string();
        }
        self
    }

    fn headers(&self, req: HttpRequest) -> HttpRequest {
        let req = if self.bearer {
            req.bearer(&self.key)
        } else {
            req.header("x-api-key", self.key.clone())
        };
        req.header("anthropic-version", API_VERSION)
    }

    fn transport(&self, e: HttpError) -> AiError {
        match e {
            HttpError::Offline(_) | HttpError::Timeout => AiError::Unreachable(self.label.clone()),
            HttpError::Other(m) => AiError::Unreachable(format!("{} ({m})", self.label)),
        }
    }

    /// The request body (public for the contract tests).
    pub fn body(req: &AiRequest) -> Value {
        let mut output_config = json!({ "effort": req.effort.as_str() });
        if let Some(schema) = &req.json_schema {
            output_config["format"] =
                json!({ "type": "json_schema", "schema": api_schema(schema) });
        }
        json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "system": [{
                "type": "text",
                "text": req.system,
                "cache_control": { "type": "ephemeral" }
            }],
            "messages": [{ "role": "user", "content": req.user }],
            "output_config": output_config,
        })
    }
}

fn status_error(status: u16, retry_after: Option<u64>) -> AiError {
    match status {
        401 | 403 => AiError::InvalidKey,
        429 => AiError::RateLimited {
            retry_after_secs: retry_after.unwrap_or(30),
        },
        s => AiError::Server(s),
    }
}

impl AiBackend for ClaudeApi {
    fn kind(&self) -> BackendKind {
        BackendKind::ClaudeApi
    }

    fn complete(&self, req: &AiRequest) -> Result<AiResponse, AiError> {
        let http_req = self
            .headers(HttpRequest::post(format!("{}/v1/messages", self.base)))
            .json(&ClaudeApi::body(req))
            .timeout(req.timeout);
        let resp = self.http.send(&http_req).map_err(|e| self.transport(e))?;
        if !resp.ok() {
            return Err(status_error(resp.status, resp.retry_after_secs()));
        }
        let v = resp.json_value();
        let stop = v
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_string);
        let u = v.get("usage");
        let n = |k: &str| {
            u.and_then(|u| u.get(k))
                .and_then(Value::as_i64)
                .unwrap_or(0)
        };
        let usage = u.map(|_| Usage {
            input_tokens: n("input_tokens"),
            output_tokens: n("output_tokens"),
            cache_read_tokens: n("cache_read_input_tokens"),
            cache_write_tokens: n("cache_creation_input_tokens"),
            cache_write_1h_tokens: u
                .and_then(|u| u.get("cache_creation"))
                .and_then(|c| c.get("ephemeral_1h_input_tokens"))
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .clamp(0, n("cache_creation_input_tokens").max(0)),
        });
        let request_id = resp
            .header("request-id")
            .map(str::to_string)
            .or_else(|| v.get("id").and_then(Value::as_str).map(str::to_string));
        if stop.as_deref() == Some("refusal") {
            return Err(AiError::Refusal);
        }
        let text: String = v
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        Ok(AiResponse {
            text,
            usage,
            request_id,
            stop_reason: stop,
            model: v
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&req.model)
                .to_string(),
            source: BackendKind::ClaudeApi,
        })
    }

    fn list_models(&self) -> Result<Vec<String>, AiError> {
        let resp = self
            .http
            .send(&self.headers(HttpRequest::get(format!("{}/v1/models", self.base))))
            .map_err(|e| self.transport(e))?;
        if !resp.ok() {
            return Err(status_error(resp.status, resp.retry_after_secs()));
        }
        Ok(resp
            .json_value()
            .get("data")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_constraints_are_stripped_for_the_api() {
        let s = json!({"type":"object","properties":{"i":{"type":"integer","minimum":0},
            "why":{"type":"string","minLength":1,"maxLength":120}},"additionalProperties":false});
        let a = api_schema(&s);
        assert!(a["properties"]["i"].get("minimum").is_none());
        assert!(a["properties"]["why"].get("maxLength").is_none());
        assert_eq!(a["additionalProperties"], false);
    }
}
