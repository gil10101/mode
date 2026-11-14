//! Transcript records (05 §5.2 "Record shape", §5.3 "Extraction"): one JSON
//! object per line, every field optional, unknown types skipped. Observed on
//! Claude Code 2.1.305 (`fixtures/transcripts/2.1.305/`): `user`,
//! `assistant`, `summary`, `system` (`compact_boundary`,
//! `stop_hook_summary`), `attachment`, plus bookkeeping lines
//! (`queue-operation`, `last-prompt`, `atis-latch`, `cost-state`, `mode`).
//! Subagent files under `<session>/subagents/agent-<id>.jsonl` carry
//! `agentId` and `isSidechain: true`.

use serde_json::{Map, Value};

use crate::redact;

/// Token figures of one assistant message (`message.usage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    /// The part of `cache_write` written to the 1-hour cache
    /// (`usage.cache_creation.ephemeral_1h_input_tokens`), priced apart (ADR-0051).
    pub cache_write_1h: i64,
}

impl Usage {
    /// Input + cache read + cache write: what the model had in context.
    pub fn context(&self) -> i64 {
        self.input + self.cache_read + self.cache_write
    }

    pub fn total(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_write
    }
}

/// A `tool_use` content block.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolUse {
    pub id: Option<String>,
    pub name: String,
    pub input: Value,
}

/// A `tool_result` content block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub tool_use_id: Option<String>,
    pub is_error: bool,
    pub bytes: usize,
}

/// `message` of a `user` / `assistant` record.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Message {
    pub role: String,
    /// The API message id (`msg_…`); several records share one when a
    /// response has several content blocks, and they repeat its `usage`.
    pub id: Option<String>,
    pub model: Option<String>,
    /// Text blocks joined (a `user` record's plain string content included).
    pub text: String,
    pub tool_uses: Vec<ToolUse>,
    pub tool_results: Vec<ToolResult>,
    pub has_thinking: bool,
    pub usage: Option<Usage>,
    pub stop_reason: Option<String>,
}

/// What kind of line this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    User,
    Assistant,
    Summary,
    System,
    Attachment,
    /// Bookkeeping (`queue-operation`, `cost-state`, …) or unknown.
    Other,
}

/// One parsed line.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub kind: Kind,
    /// The raw `type`.
    pub type_name: String,
    pub subtype: Option<String>,
    pub uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub timestamp: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub version: Option<String>,
    pub slug: Option<String>,
    pub is_meta: bool,
    pub is_sidechain: bool,
    pub message: Option<Message>,
    /// `summary` records: the CLI's own session title.
    pub summary: Option<String>,
    /// `system` / `compact_boundary`: `(preTokens, postTokens)`.
    pub compact: Option<(i64, i64)>,
    /// `cost-state`: `totalCostUSD` (stored, never shown; 05 §5.3).
    pub total_cost_usd: Option<f64>,
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn i64_of(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn parse_usage(v: &Value) -> Option<Usage> {
    let u = v.as_object()?;
    if u.is_empty() {
        return None;
    }
    let cache_write = i64_of(v, "cache_creation_input_tokens");
    Some(Usage {
        input: i64_of(v, "input_tokens"),
        output: i64_of(v, "output_tokens"),
        cache_read: i64_of(v, "cache_read_input_tokens"),
        cache_write,
        cache_write_1h: cache_write_1h_of(v, cache_write),
    })
}

/// `usage.cache_creation.ephemeral_1h_input_tokens`, never more than the cache writes themselves.
pub fn cache_write_1h_of(usage: &Value, cache_write: i64) -> i64 {
    usage
        .get("cache_creation")
        .map(|c| i64_of(c, "ephemeral_1h_input_tokens"))
        .unwrap_or(0)
        .clamp(0, cache_write.max(0))
}

fn parse_message(m: &Value) -> Message {
    let mut out = Message {
        role: str_of(m, "role").unwrap_or_default(),
        id: str_of(m, "id"),
        model: str_of(m, "model"),
        usage: m.get("usage").and_then(parse_usage),
        stop_reason: str_of(m, "stop_reason"),
        ..Default::default()
    };
    match m.get("content") {
        Some(Value::String(s)) => out.text = s.clone(),
        Some(Value::Array(blocks)) => {
            let mut texts: Vec<&str> = Vec::new();
            for b in blocks {
                match b.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = b.get("text").and_then(Value::as_str) {
                            texts.push(t);
                        }
                    }
                    Some("thinking") => out.has_thinking = true,
                    Some("tool_use") => out.tool_uses.push(ToolUse {
                        id: str_of(b, "id"),
                        name: str_of(b, "name").unwrap_or_else(|| "tool".into()),
                        input: b.get("input").cloned().unwrap_or(Value::Null),
                    }),
                    Some("tool_result") => {
                        let content = b.get("content");
                        let bytes = match content {
                            Some(Value::String(s)) => s.len(),
                            Some(other) => serde_json::to_vec(other).map(|v| v.len()).unwrap_or(0),
                            None => 0,
                        };
                        out.tool_results.push(ToolResult {
                            tool_use_id: str_of(b, "tool_use_id"),
                            is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                            bytes,
                        });
                    }
                    _ => {}
                }
            }
            out.text = texts.join("\n");
        }
        _ => {}
    }
    out
}

/// Parse one line. `None` for blank lines and anything that is not a JSON
/// object (a partial last line is the caller's business).
pub fn parse_line(line: &str) -> Option<Record> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    let obj: &Map<String, Value> = v.as_object()?;
    let type_name = obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let kind = match type_name.as_str() {
        "user" => Kind::User,
        "assistant" => Kind::Assistant,
        "summary" => Kind::Summary,
        "system" => Kind::System,
        "attachment" => Kind::Attachment,
        _ => Kind::Other,
    };
    let message = match kind {
        Kind::User | Kind::Assistant => v.get("message").map(parse_message),
        _ => None,
    };
    let compact = if type_name == "system" {
        v.get("compactMetadata")
            .map(|c| (i64_of(c, "preTokens"), i64_of(c, "postTokens")))
    } else {
        None
    };
    Some(Record {
        kind,
        subtype: str_of(&v, "subtype"),
        uuid: str_of(&v, "uuid"),
        parent_uuid: str_of(&v, "parentUuid"),
        session_id: str_of(&v, "sessionId"),
        agent_id: str_of(&v, "agentId"),
        timestamp: str_of(&v, "timestamp"),
        // Canonical at ingress (`crate::paths`): rows compare it with the
        // hook's `cwd`.
        cwd: str_of(&v, "cwd").map(|c| crate::paths::canon(&c)),
        git_branch: str_of(&v, "gitBranch"),
        version: str_of(&v, "version"),
        slug: str_of(&v, "slug"),
        is_meta: v.get("isMeta").and_then(Value::as_bool).unwrap_or(false),
        is_sidechain: v
            .get("isSidechain")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        message,
        summary: if kind == Kind::Summary {
            str_of(&v, "summary")
        } else {
            None
        },
        compact,
        total_cost_usd: if type_name == "cost-state" {
            v.get("totalCostUSD").and_then(Value::as_f64)
        } else {
            None
        },
        type_name,
    })
}

impl Record {
    /// A real prompt typed by the user (not a tool result, not a meta line).
    pub fn is_user_prompt(&self) -> bool {
        self.kind == Kind::User
            && !self.is_meta
            && self
                .message
                .as_ref()
                .is_some_and(|m| m.tool_results.is_empty() && !m.text.trim().is_empty())
    }
}

// ---------------------------------------------------------------- extraction

/// An Overview item derived from the transcript (`session_events` with
/// `source = 'transcript'`; the content stays in the file, 05 §5.3).
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptEvent {
    /// Stable across re-reads: `tr:<uuid>` or `tr:<uuid>:<block>`.
    pub id: String,
    /// `transcript.user`, `transcript.assistant`, `transcript.tool_use`,
    /// `transcript.tool_result`, `transcript.compacted`, `transcript.subagent`.
    pub event: &'static str,
    pub tool_name: Option<String>,
    pub tool_use_id: Option<String>,
    pub agent_id: Option<String>,
    pub summary: String,
    pub occurred_at: String,
    /// Small, redacted; `None` for most items.
    pub payload: Option<Value>,
}

/// A `usage_samples` row to write (de-duplicated by `turn_ref`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageIn {
    /// The API message id, else the record uuid.
    pub turn_ref: String,
    pub model: String,
    pub sampled_at: String,
    pub usage: Usage,
}

/// A subagent the transcript shows being spawned (`Agent` / `Task` tool use).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentSpawn {
    pub tool_use_id: Option<String>,
    pub agent_type: Option<String>,
    pub description: Option<String>,
    pub occurred_at: String,
}

/// Everything a batch of records contributes (05 §5.3 table).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extracted {
    pub events: Vec<TranscriptEvent>,
    pub samples: Vec<UsageIn>,
    pub spawns: Vec<SubagentSpawn>,
    /// The latest `summary` record's text.
    pub title: Option<String>,
    /// `message.model` of the latest assistant record.
    pub model: Option<String>,
    /// Context size after the latest assistant record.
    pub context_tokens: Option<i64>,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub cli_version: Option<String>,
    pub slug: Option<String>,
    /// User prompts in the batch.
    pub prompts: u32,
    /// The first user prompt's text (first 60 chars), for the title fallback.
    pub first_prompt: Option<String>,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
    pub total_cost_usd: Option<f64>,
    /// Lines that did not parse (diagnostics; never fatal).
    pub skipped: u32,
}

/// Tools whose `tool_use` spawns a subagent.
pub const SUBAGENT_TOOLS: &[&str] = &["Agent", "Task"];

fn preview(text: &str, max: usize) -> String {
    let one = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let cut: String = one.chars().take(max).collect();
    let cut = if one.chars().count() > max {
        format!("{cut}…")
    } else {
        cut
    };
    redact::scrub(&cut).into_owned()
}

/// Extract from already-parsed records. `keep_prompts` is
/// `sessions.store_prompt_previews`: without it user prompts become a bare
/// "prompt" item and no first-prompt title is produced.
pub fn extract(records: &[Record], keep_prompts: bool) -> Extracted {
    let mut out = Extracted::default();
    let mut seen_usage: std::collections::HashSet<String> = std::collections::HashSet::new();
    for r in records {
        if out.session_id.is_none() {
            out.session_id = r.session_id.clone();
        }
        if out.agent_id.is_none() {
            out.agent_id = r.agent_id.clone();
        }
        if let Some(c) = &r.cwd {
            out.cwd = Some(c.clone());
        }
        if let Some(b) = &r.git_branch {
            out.git_branch = Some(b.clone());
        }
        if let Some(v) = &r.version {
            out.cli_version = Some(v.clone());
        }
        if let Some(s) = &r.slug {
            out.slug = Some(s.clone());
        }
        if let Some(ts) = &r.timestamp {
            if out.first_timestamp.is_none() {
                out.first_timestamp = Some(ts.clone());
            }
            out.last_timestamp = Some(ts.clone());
        }
        if let Some(c) = r.total_cost_usd {
            out.total_cost_usd = Some(c);
        }
        let at = r.timestamp.clone().unwrap_or_default();
        let agent_id = r.agent_id.clone();
        match r.kind {
            Kind::Summary => {
                if let Some(s) = r
                    .summary
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    out.title = Some(s.to_string());
                }
            }
            Kind::System => {
                if r.subtype.as_deref() == Some("compact_boundary") {
                    let (pre, post) = r.compact.unwrap_or((0, 0));
                    out.events.push(TranscriptEvent {
                        id: format!("tr:{}", r.uuid.clone().unwrap_or_default()),
                        event: "transcript.compacted",
                        tool_name: None,
                        tool_use_id: None,
                        agent_id,
                        summary: if pre > 0 {
                            format!("context compacted · {pre} → {post} tokens")
                        } else {
                            "context compacted".into()
                        },
                        occurred_at: at,
                        payload: Some(serde_json::json!({"pre_tokens": pre, "post_tokens": post})),
                    });
                    if post > 0 {
                        out.context_tokens = Some(post);
                    }
                }
            }
            Kind::User => {
                let Some(m) = &r.message else { continue };
                let uuid = r.uuid.clone().unwrap_or_default();
                if r.is_user_prompt() {
                    out.prompts += 1;
                    if out.first_prompt.is_none() && keep_prompts {
                        out.first_prompt = Some(preview(&m.text, 60));
                    }
                    out.events.push(TranscriptEvent {
                        id: format!("tr:{uuid}"),
                        event: "transcript.user",
                        tool_name: None,
                        tool_use_id: None,
                        agent_id,
                        summary: if keep_prompts {
                            preview(&m.text, 200)
                        } else {
                            "prompt".into()
                        },
                        occurred_at: at,
                        payload: None,
                    });
                } else {
                    for (i, tr) in m.tool_results.iter().enumerate() {
                        out.events.push(TranscriptEvent {
                            id: format!("tr:{uuid}:{i}"),
                            event: "transcript.tool_result",
                            tool_name: None,
                            tool_use_id: tr.tool_use_id.clone(),
                            agent_id: agent_id.clone(),
                            summary: if tr.is_error {
                                format!("tool error · {} bytes", tr.bytes)
                            } else {
                                format!("tool result · {} bytes", tr.bytes)
                            },
                            occurred_at: at.clone(),
                            payload: None,
                        });
                    }
                }
            }
            Kind::Assistant => {
                let Some(m) = &r.message else { continue };
                let uuid = r.uuid.clone().unwrap_or_default();
                if let Some(model) = &m.model {
                    out.model = Some(model.clone());
                }
                if let Some(u) = m.usage {
                    let turn_ref = m.id.clone().unwrap_or_else(|| uuid.clone());
                    if !turn_ref.is_empty() && seen_usage.insert(turn_ref.clone()) {
                        out.samples.push(UsageIn {
                            turn_ref,
                            model: m.model.clone().unwrap_or_else(|| "unknown".into()),
                            sampled_at: at.clone(),
                            usage: u,
                        });
                    }
                    out.context_tokens = Some(u.context());
                }
                if !m.text.trim().is_empty() {
                    out.events.push(TranscriptEvent {
                        id: format!("tr:{uuid}"),
                        event: "transcript.assistant",
                        tool_name: None,
                        tool_use_id: None,
                        agent_id: agent_id.clone(),
                        summary: preview(&m.text, 200),
                        occurred_at: at.clone(),
                        payload: None,
                    });
                }
                for (i, tu) in m.tool_uses.iter().enumerate() {
                    let summary =
                        crate::sessions::naming::tool_summary(&tu.name, Some(&tu.input), None);
                    if SUBAGENT_TOOLS.contains(&tu.name.as_str()) {
                        let agent_type = tu
                            .input
                            .get("subagent_type")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        let description = tu
                            .input
                            .get("description")
                            .and_then(Value::as_str)
                            .map(|d| preview(d, 120));
                        out.spawns.push(SubagentSpawn {
                            tool_use_id: tu.id.clone(),
                            agent_type: agent_type.clone(),
                            description: description.clone(),
                            occurred_at: at.clone(),
                        });
                        out.events.push(TranscriptEvent {
                            id: format!("tr:{uuid}:{i}"),
                            event: "transcript.subagent",
                            tool_name: Some(tu.name.clone()),
                            tool_use_id: tu.id.clone(),
                            agent_id: agent_id.clone(),
                            summary: format!(
                                "{} · {}",
                                agent_type.as_deref().unwrap_or("agent"),
                                description.as_deref().unwrap_or("")
                            )
                            .trim_end_matches(" · ")
                            .to_string(),
                            occurred_at: at.clone(),
                            payload: Some(serde_json::json!({
                                "agent_type": agent_type,
                                "description": description,
                            })),
                        });
                        continue;
                    }
                    out.events.push(TranscriptEvent {
                        id: format!("tr:{uuid}:{i}"),
                        event: "transcript.tool_use",
                        tool_name: Some(tu.name.clone()),
                        tool_use_id: tu.id.clone(),
                        agent_id: agent_id.clone(),
                        summary: redact::scrub(&summary).into_owned(),
                        occurred_at: at.clone(),
                        payload: Some(redact::tool_field_for_storage(&identity_only(&tu.input))),
                    });
                }
            }
            Kind::Attachment | Kind::Other => {}
        }
    }
    out
}

/// Only the keys that identify what a tool did (never file contents).
fn identity_only(input: &Value) -> Value {
    const KEYS: &[&str] = &[
        "file_path",
        "notebook_path",
        "command",
        "description",
        "pattern",
        "path",
        "url",
        "query",
        "subagent_type",
    ];
    let mut out = Map::new();
    if let Value::Object(m) = input {
        for k in KEYS {
            if let Some(v) = m.get(*k) {
                out.insert((*k).to_string(), v.clone());
            }
        }
    }
    Value::Object(out)
}

/// Parse and extract a whole text (tests, catch-up reads).
pub fn extract_text(text: &str, keep_prompts: bool) -> Extracted {
    let mut records = Vec::new();
    let mut skipped = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_line(line) {
            Some(r) => records.push(r),
            None => skipped += 1,
        }
    }
    let mut out = extract(&records, keep_prompts);
    out.skipped = skipped;
    out
}
