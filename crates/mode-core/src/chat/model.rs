//! The model side of a Chat step. [`ChatModel`] is what the turn loop
//! drives; [`EnvelopeModel`] implements it over the shared
//! [`AiBackend`](crate::connectors::ai::AiBackend) client (the one scoring
//! uses, 07 §3.4): that client has one non-streaming `complete` call with
//! structured output and no native tool use, so a step asks for one JSON
//! envelope (`tool_calls` | `answer` + `citations` + `actions`) validated
//! against [`envelope_schema`], and the reply text is then streamed to the
//! webview as `chat.delta` chunks. A backend with native streaming and
//! tools can implement [`ChatModel::step_stream`] directly.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::citations::RawCitation;
use crate::connectors::ai::{AiBackend, AiError, AiRequest, BackendKind, Effort, Purpose, Usage};

/// `max_tokens` for a Chat step (06 §10.2).
pub const MAX_TOKENS: u32 = 16_384;

/// One request to the model.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRequest {
    /// The fixed, cached prefix (prompt + tools + action shapes).
    pub system: String,
    /// The volatile part: history, data, the question.
    pub user: String,
    pub schema: Value,
    pub effort: Effort,
    pub max_tokens: u32,
    pub timeout: Duration,
}

/// A tool call in the envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    /// The input object (the envelope carries it as a JSON string).
    pub input: Value,
}

/// A proposed action in the envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawAction {
    pub kind: String,
    pub params: Value,
}

/// The reply envelope.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub tool_calls: Vec<ToolCall>,
    pub answer: String,
    pub citations: Vec<RawCitation>,
    pub actions: Vec<RawAction>,
}

/// One step's reply.
#[derive(Debug, Clone, PartialEq)]
pub struct StepReply {
    pub envelope: Envelope,
    pub usage: Option<Usage>,
    pub request_id: Option<String>,
    pub model: String,
    /// The client that answered (a route's backup may differ from the primary).
    pub source: Option<BackendKind>,
}

/// A failed step; tokens a backend charged for an unusable reply are kept.
#[derive(Debug, Clone, PartialEq)]
pub struct StepError {
    pub error: AiError,
    pub usage: Option<Usage>,
    pub request_id: Option<String>,
    pub source: Option<BackendKind>,
}

impl From<AiError> for StepError {
    fn from(error: AiError) -> Self {
        StepError {
            error,
            usage: None,
            request_id: None,
            source: None,
        }
    }
}

/// What the turn loop drives.
pub trait ChatModel: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn model(&self) -> &str;
    /// One request → one envelope.
    fn step(&self, req: &StepRequest) -> Result<StepReply, StepError>;
    /// One step with the answer streamed through `on_delta`; the default
    /// emits the finished answer in line-sized chunks, stopping when
    /// `cancel` is set (the partial text is what was emitted).
    fn step_stream(
        &self,
        req: &StepRequest,
        cancel: &AtomicBool,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<StepReply, StepError> {
        let r = self.step(req)?;
        if r.envelope.tool_calls.is_empty() {
            for chunk in chunks(&r.envelope.answer) {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                on_delta(&chunk);
            }
        }
        Ok(r)
    }
}

/// Split an answer into stream chunks: lines, long lines by words (~80 chars).
pub fn chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split_inclusive('\n') {
        if line.len() <= 96 {
            out.push(line.to_string());
            continue;
        }
        let mut cur = String::new();
        for word in line.split_inclusive(' ') {
            cur.push_str(word);
            if cur.len() >= 80 {
                out.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// The envelope's JSON schema (structured output; strict on Mode's side).
pub fn envelope_schema(tools: &[&str], actions: &[&str]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["tool_calls", "answer", "citations", "actions"],
        "properties": {
            "tool_calls": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "input_json"],
                    "properties": {
                        "name": { "type": "string", "enum": tools },
                        "input_json": { "type": "string" }
                    }
                }
            },
            "answer": { "type": "string" },
            "citations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["n", "kind", "id", "snippet"],
                    "properties": {
                        "n": { "type": "integer" },
                        "kind": { "type": "string", "enum": ["note", "message", "thread", "session", "calendar_event", "file"] },
                        "id": { "type": "string" },
                        "snippet": { "type": "string" }
                    }
                }
            },
            "actions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["kind", "params_json"],
                    "properties": {
                        "kind": { "type": "string", "enum": actions },
                        "params_json": { "type": "string" }
                    }
                }
            }
        }
    })
}

fn json_field(v: &Value, string_key: &str, object_key: &str) -> Value {
    match v.get(string_key) {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|_| json!({})),
        Some(o @ Value::Object(_)) => o.clone(),
        _ => v.get(object_key).cloned().unwrap_or_else(|| json!({})),
    }
}

/// Parse a reply (lenient: a fenced block, a missing key, an object where
/// the schema says string are all accepted; anything else is `BadOutput`).
pub fn parse_envelope(text: &str) -> Result<Envelope, AiError> {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .map(|x| x.trim_end().trim_end_matches("```").trim())
        .unwrap_or(t);
    let start = t.find('{');
    let end = t.rfind('}');
    let body = match (start, end) {
        (Some(a), Some(b)) if b > a => &t[a..=b],
        _ => return Err(AiError::BadOutput("the reply is not a JSON object".into())),
    };
    let v: Value =
        serde_json::from_str(body).map_err(|e| AiError::BadOutput(format!("reply JSON: {e}")))?;
    let tool_calls = v
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    let name = c.get("name")?.as_str()?.to_string();
                    Some(ToolCall {
                        input: json_field(c, "input_json", "input"),
                        name,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let citations = v
        .get("citations")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| serde_json::from_value::<RawCitation>(c.clone()).ok())
                .collect()
        })
        .unwrap_or_default();
    let actions = v
        .get("actions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| {
                    Some(RawAction {
                        kind: c.get("kind")?.as_str()?.to_string(),
                        params: json_field(c, "params_json", "params"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Envelope {
        tool_calls,
        answer: v
            .get("answer")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        citations,
        actions,
    })
}

/// [`ChatModel`] over an [`AiBackend`].
pub struct EnvelopeModel {
    pub backend: Arc<dyn AiBackend>,
    pub model: String,
}

impl ChatModel for EnvelopeModel {
    fn kind(&self) -> BackendKind {
        self.backend.kind()
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn step(&self, req: &StepRequest) -> Result<StepReply, StepError> {
        let r = self.backend.complete(&AiRequest {
            purpose: Purpose::Chat,
            model: self.model.clone(),
            system: req.system.clone(),
            user: req.user.clone(),
            json_schema: Some(req.schema.clone()),
            max_tokens: req.max_tokens,
            effort: req.effort,
            timeout: req.timeout,
        })?;
        // Tokens are spent even when the reply does not parse.
        match parse_envelope(&r.text) {
            Ok(envelope) => Ok(StepReply {
                envelope,
                usage: r.usage,
                request_id: r.request_id,
                model: r.model,
                source: Some(r.source),
            }),
            Err(error) => Err(StepError {
                error,
                usage: r.usage,
                request_id: r.request_id,
                source: Some(r.source),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelopes_parse_leniently() {
        let e = parse_envelope(
            "```json\n{\"tool_calls\":[{\"name\":\"search_notes\",\"input_json\":\"{\\\"text\\\":\\\"plan\\\"}\"}],\"answer\":\"\",\"citations\":[],\"actions\":[]}\n```",
        )
        .unwrap();
        assert_eq!(e.tool_calls[0].input["text"], "plan");
        let e = parse_envelope(r#"{"answer":"Hi [1]","citations":[{"n":1,"kind":"note","id":"a","snippet":"x"}],"actions":[{"kind":"create_note","params":{"title":"T"}}]}"#).unwrap();
        assert_eq!(e.citations[0].id, "a");
        assert_eq!(e.actions[0].params["title"], "T");
        assert!(parse_envelope("no json here").is_err());
    }

    #[test]
    fn chunks_cover_the_text() {
        let t = "First line.\nA much longer second line that keeps going and going well past the eighty character mark for sure.\n";
        let c = chunks(t);
        assert!(c.len() >= 3);
        assert_eq!(c.concat(), t);
    }
}
