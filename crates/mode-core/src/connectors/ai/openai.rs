//! Ollama and LM Studio (06 §10.3): one OpenAI-compatible client on
//! loopback (or a private address). `POST /v1/chat/completions` with
//! `response_format: json_schema`; a server that refuses it gets
//! `json_object` with the schema in the prompt, and Mode validates either
//! way. `GET /v1/models` is the model picker and the health check. An
//! optional bearer comes from `ai:local_token`. Tokens are recorded when
//! the server returns `usage`; local backends never count toward the budget.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{AiBackend, AiError, AiRequest, AiResponse, BackendKind, Usage};
use crate::connectors::http::{HttpClient, HttpRequest, HttpResponse};

pub struct OpenAiCompat {
    http: Arc<dyn HttpClient>,
    kind: BackendKind,
    base: String,
    token: Option<String>,
    /// The provider's name in "Can't reach …".
    label: String,
}

impl std::fmt::Debug for OpenAiCompat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompat")
            .field("kind", &self.kind)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl OpenAiCompat {
    pub fn new(
        http: Arc<dyn HttpClient>,
        kind: BackendKind,
        base: &str,
        token: Option<String>,
    ) -> OpenAiCompat {
        OpenAiCompat {
            http,
            kind,
            base: base.trim_end_matches('/').to_string(),
            token,
            label: kind.label().into(),
        }
    }

    pub fn with_label(mut self, label: &str) -> OpenAiCompat {
        if !label.trim().is_empty() {
            self.label = label.trim().to_string();
        }
        self
    }

    fn auth(&self, req: HttpRequest) -> HttpRequest {
        match &self.token {
            Some(t) if !t.is_empty() => req.bearer(t),
            _ => req,
        }
    }

    /// The request body; `strict_schema = false` is the `json_object` fallback.
    pub fn body(req: &AiRequest, strict_schema: bool) -> Value {
        let mut system = req.system.clone();
        let mut body = json!({
            "model": req.model,
            "stream": false,
            "max_tokens": req.max_tokens,
        });
        if let Some(schema) = &req.json_schema {
            if strict_schema {
                body["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": { "name": "mode_reply", "schema": schema, "strict": true }
                });
            } else {
                body["response_format"] = json!({ "type": "json_object" });
                system.push_str("\n\nReply with JSON matching this schema:\n");
                system.push_str(&schema.to_string());
            }
        }
        body["messages"] = json!([
            { "role": "system", "content": system },
            { "role": "user", "content": req.user },
        ]);
        body
    }

    fn post(&self, req: &AiRequest, strict: bool) -> Result<HttpResponse, AiError> {
        let r = self
            .auth(HttpRequest::post(format!(
                "{}/v1/chat/completions",
                self.base
            )))
            .json(&OpenAiCompat::body(req, strict))
            .timeout(req.timeout);
        self.http
            .send(&r)
            .map_err(|_| AiError::Unreachable(self.label.clone()))
    }
}

impl AiBackend for OpenAiCompat {
    fn kind(&self) -> BackendKind {
        self.kind
    }

    fn complete(&self, req: &AiRequest) -> Result<AiResponse, AiError> {
        let mut resp = self.post(req, true)?;
        if resp.status == 400 && req.json_schema.is_some() {
            resp = self.post(req, false)?;
        }
        if !resp.ok() {
            return Err(match resp.status {
                401 | 403 => AiError::InvalidKey,
                429 => AiError::RateLimited {
                    retry_after_secs: resp.retry_after_secs().unwrap_or(30),
                },
                s => AiError::Server(s),
            });
        }
        let v = resp.json_value();
        let text = v
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let usage = v.get("usage").map(|u| Usage {
            input_tokens: u.get("prompt_tokens").and_then(Value::as_i64).unwrap_or(0),
            output_tokens: u
                .get("completion_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cache_write_1h_tokens: 0,
        });
        Ok(AiResponse {
            text,
            usage,
            request_id: v.get("id").and_then(Value::as_str).map(str::to_string),
            stop_reason: v
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                .map(str::to_string),
            model: v
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&req.model)
                .to_string(),
            source: self.kind,
        })
    }

    fn list_models(&self) -> Result<Vec<String>, AiError> {
        let resp = self
            .http
            .send(&self.auth(HttpRequest::get(format!("{}/v1/models", self.base))))
            .map_err(|_| AiError::Unreachable(self.label.clone()))?;
        if !resp.ok() {
            return Err(match resp.status {
                401 | 403 => AiError::InvalidKey,
                _ => AiError::Unreachable(self.label.clone()),
            });
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
