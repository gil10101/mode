//! The Chat contract at the edge (`ui/src/state/contract-chat.ts`): the
//! shapes the commands return and the three streams carry, built from the
//! stored rows of [`super::store`]. Field names follow the TypeScript file;
//! fields beyond it are extras the views may ignore.
//!
//! Kinds at the edge: a thread is a `message` (its route still opens the
//! thread) and a calendar event is an `event`; action kinds `set_note_fields`
//! / `snooze` / `archive` are `set_fields` / `snooze_message` /
//! `archive_message`; card states `proposed` / `confirmed` are `pending` /
//! `done`; a reply's `running` is `streaming` and a stopped reply is
//! `stopped`.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::actions::ActionView;
use super::citations::{self, SourceItem};
use super::store::{self, ActivityRow, ThreadDetail, TokenUsage, ToolCallRow, TurnView};
use super::tools::Sources;
use crate::db::Result;

/// `ChatConnectors` (`{gmail, slack, calendar}`).
pub type ChatConnectors = Sources;

/// The contract's item kind for a stored kind.
pub fn edge_kind(kind: &str) -> &str {
    match kind {
        "thread" => "message",
        "calendar_event" => "event",
        k => k,
    }
}

/// The contract's action kind for a stored kind.
pub fn edge_action_kind(kind: &str) -> &str {
    match kind {
        "set_note_fields" => "set_fields",
        "snooze" => "snooze_message",
        "archive" => "archive_message",
        k => k,
    }
}

/// The contract's card state for a stored one.
pub fn edge_action_state(state: &str) -> &str {
    match state {
        "proposed" => "pending",
        "confirmed" => "done",
        s => s,
    }
}

/// A file id without the `file://` scheme the picker may have added.
pub fn file_path(id: &str) -> &str {
    id.strip_prefix("file://").unwrap_or(id)
}

/// The stored kind and id of an attachment the view sends: `event` is a
/// calendar event, `message` is a thread when the id names one, a file
/// loses its `file://`.
pub fn attachment_kind(conn: &Connection, kind: &str, id: &str) -> Result<(String, String)> {
    Ok(match kind {
        "event" | "calendar" => ("calendar_event".into(), id.into()),
        "file" => ("file".into(), file_path(id).into()),
        "message" => {
            let is_message = conn
                .query_row("SELECT 1 FROM messages WHERE id = ?1", [id], |_| Ok(()))
                .optional()?
                .is_some();
            let is_thread = !is_message
                && conn
                    .query_row("SELECT 1 FROM threads WHERE id = ?1", [id], |_| Ok(()))
                    .optional()?
                    .is_some();
            (
                if is_thread { "thread" } else { "message" }.into(),
                id.into(),
            )
        }
        k => (k.into(), id.into()),
    })
}

fn clip(s: &str, max: usize) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        return t;
    }
    let mut out: String = t.chars().take(max - 1).collect();
    out = out.trim_end().to_string();
    out.push('…');
    out
}

fn route_of(kind: &str, id: &str) -> Option<String> {
    (kind != "file")
        .then(|| citations::route(kind, id))
        .filter(|r| !r.is_empty())
}

/// What a citation, a source item or an attachment points at (`ItemRef`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRef {
    pub kind: String,
    pub id: String,
    pub title: String,
    /// The Sources panel's second line (folder, sender, repo, time).
    #[serde(default)]
    pub detail: Option<String>,
    /// The app route that opens it (none for a file).
    #[serde(default)]
    pub route: Option<String>,
}

impl ItemRef {
    pub fn from_source(s: &SourceItem) -> ItemRef {
        let detail = s
            .meta
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| (!s.snippet.trim().is_empty()).then(|| clip(&s.snippet, 80)));
        ItemRef {
            kind: edge_kind(&s.kind).into(),
            id: s.id.clone(),
            title: s.title.clone(),
            detail,
            route: (s.kind != "file" && !s.route.is_empty()).then(|| s.route.clone()),
        }
    }

    pub fn from_attachment(a: &store::ItemRef) -> ItemRef {
        let title = a.title.clone().unwrap_or_else(|| {
            std::path::Path::new(&a.id)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| a.id.clone())
        });
        ItemRef {
            kind: edge_kind(&a.kind).into(),
            id: a.id.clone(),
            title,
            detail: (a.kind == "file").then(|| a.id.clone()),
            route: route_of(&a.kind, &a.id),
        }
    }
}

/// `[n]` in the reply refers to `n` (`Citation`; `snippet` is beyond the contract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub n: u32,
    pub kind: String,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub route: Option<String>,
    #[serde(default)]
    pub snippet: String,
}

impl Citation {
    pub fn from_stored(c: &citations::Citation) -> Citation {
        Citation {
            n: c.n,
            kind: edge_kind(&c.kind).into(),
            id: c.id.clone(),
            title: c.title.clone(),
            detail: (!c.snippet.trim().is_empty()).then(|| clip(&c.snippet, 80)),
            route: (c.kind != "file" && !c.route.is_empty()).then(|| c.route.clone()),
            snippet: c.snippet.clone(),
        }
    }
}

/// One row of an activity group (`ActivityRowSpec`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityRowSpec {
    pub text: String,
    /// File kind for the 24 px thumbnail (`md`, `py`); files only.
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default)]
    pub call_id: Option<String>,
}

/// One tool call as an activity group (`ActivityGroup`): the label, one row
/// per item it returned (the inputs when it returned none). `tool`, `state`,
/// `count` and `error` are beyond the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityGroup {
    pub id: String,
    pub label: String,
    pub rows: Vec<ActivityRowSpec>,
    /// A line under the group: the failure, when the call failed.
    #[serde(default)]
    pub status: Option<String>,
    /// The call is running (the label shimmers).
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub tool: String,
    /// `running` | `done` | `failed`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub count: usize,
    #[serde(default)]
    pub error: Option<String>,
}

fn thumb(item: &SourceItem) -> Option<String> {
    (item.kind == "file")
        .then(|| {
            std::path::Path::new(&item.id)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
        })
        .flatten()
}

/// The inputs of a call as one line ("release · status todo").
pub fn input_line(input: &Value) -> String {
    fn word(v: &Value) -> Option<String> {
        match v {
            Value::Null => None,
            Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
            Value::Bool(b) => Some(b.to_string()),
            Value::Number(n) => Some(n.to_string()),
            Value::Array(a) => {
                let parts: Vec<String> = a.iter().filter_map(word).collect();
                (!parts.is_empty()).then(|| parts.join(", "))
            }
            Value::Object(_) => None,
        }
    }
    let line = match input {
        Value::Object(m) => m
            .iter()
            .filter_map(|(k, v)| {
                word(v).map(|w| match k.as_str() {
                    "query" | "text" | "id" | "note_id" | "thread_id" | "session_id" => w,
                    _ => format!("{} {w}", k.replace('_', " ")),
                })
            })
            .collect::<Vec<_>>()
            .join(" · "),
        other => word(other).unwrap_or_default(),
    };
    clip(&line, 120)
}

/// An activity group for a call: running (no items yet), done or failed.
pub fn group(
    call_id: &str,
    tool: &str,
    label: &str,
    input: &Value,
    items: &[SourceItem],
    state: &str,
    error: Option<&str>,
) -> ActivityGroup {
    let mut rows: Vec<ActivityRowSpec> = items
        .iter()
        .map(|it| ActivityRowSpec {
            text: it.title.clone(),
            thumb: thumb(it),
            call_id: Some(call_id.to_string()),
        })
        .collect();
    if rows.is_empty() && state != "running" {
        let line = input_line(input);
        rows.push(ActivityRowSpec {
            text: if line.is_empty() {
                tool.to_string()
            } else {
                format!("{tool} · {line}")
            },
            thumb: None,
            call_id: Some(call_id.to_string()),
        });
    }
    ActivityGroup {
        id: call_id.into(),
        label: label.into(),
        rows,
        status: error.map(str::to_string),
        active: state == "running",
        tool: tool.into(),
        state: state.into(),
        count: items.len(),
        error: error.map(str::to_string),
    }
}

fn group_of(a: &ActivityRow) -> ActivityGroup {
    let state = if a.error.is_some() { "failed" } else { "done" };
    group(
        &a.call_id,
        &a.tool,
        &a.label,
        &a.input,
        &a.items,
        state,
        a.error.as_deref(),
    )
}

/// One tool call in the Sources panel (`ToolCall`; `label`, `error` and
/// `input` are beyond the contract).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub tool: String,
    pub input_line: String,
    pub items: Vec<ItemRef>,
    /// `running` | `done` | `failed`.
    pub state: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub input: Value,
}

impl ToolCall {
    pub fn from_row(r: &ToolCallRow) -> ToolCall {
        ToolCall {
            id: r.call_id.clone(),
            tool: r.name.clone(),
            input_line: input_line(&r.input),
            items: r.items.iter().map(ItemRef::from_source).collect(),
            state: if r.error.is_some() { "failed" } else { "done" }.into(),
            label: r.label.clone(),
            error: r.error.clone(),
            input: r.input.clone(),
        }
    }
}

/// A label · value line of an action card; `before` draws "before → after".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionField {
    pub label: String,
    pub value: String,
    #[serde(default)]
    pub before: Option<String>,
}

fn field(label: &str, value: impl Into<String>) -> ActionField {
    ActionField {
        label: label.into(),
        value: value.into(),
        before: None,
    }
}

/// A proposal card (`ChatAction`). `turn_id`, `thread_id`, `summary`,
/// `created_at` and `decided_at` are beyond the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatAction {
    pub id: String,
    /// `create_note` | `edit_note` | `set_fields` | `snooze_message` |
    /// `archive_message` | `resume_session`, and the core's
    /// `session_prompt` / `create_automation`.
    pub kind: String,
    pub title: String,
    pub fields: Vec<ActionField>,
    #[serde(default)]
    pub preview: Option<String>,
    /// `pending` | `rejected` | `running` | `done` | `failed`.
    pub state: String,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub result_route: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub decided_at: Option<String>,
}

/// A JSON value as a field value ("todo", "true", "a, b").
fn shown(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()).filter(|s| !s.trim().is_empty()),
        Value::Bool(true) => Some("yes".into()),
        Value::Bool(false) => Some("no".into()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().filter_map(shown).collect();
            Some(if parts.is_empty() {
                "none".into()
            } else {
                parts.join(", ")
            })
        }
        Value::Object(_) => None,
    }
}

fn label_of(field: &str) -> String {
    let mut c = field.chars();
    match c.next() {
        Some(f) => f
            .to_uppercase()
            .chain(c)
            .collect::<String>()
            .replace('_', " "),
        None => String::new(),
    }
}

/// The card's fields and preview text for a stored proposal.
fn card(a: &ActionView) -> (Vec<ActionField>, Option<String>) {
    let p = &a.preview;
    let s = |k: &str| p.get(k).and_then(shown);
    let mut fields = Vec::new();
    let mut preview = None;
    match a.kind.as_str() {
        "create_note" => {
            if let Some(f) = s("folder") {
                fields.push(field("Folder", f));
            }
            if let Some(t) = s("title") {
                fields.push(field("Title", t));
            }
            let f = p.get("fields").cloned().unwrap_or(Value::Null);
            if let Some(st) = f.get("status").and_then(shown) {
                fields.push(field("Status", st));
            }
            if f.get("urgent").and_then(Value::as_bool) == Some(true) {
                fields.push(field("Urgent", "yes"));
            }
            if let Some(w) = f.get("when").and_then(shown) {
                fields.push(field("When", w));
            }
            if let Some(t) = f
                .get("tags")
                .filter(|t| t.as_array().is_some_and(|a| !a.is_empty()))
                .and_then(shown)
            {
                fields.push(field("Tags", t));
            }
            if let Some(n) = p.get("links").and_then(Value::as_u64).filter(|n| *n > 0) {
                fields.push(field("Links", n.to_string()));
            }
            if let Some(n) = s("notice") {
                fields.push(field("Note", n));
            }
            preview = s("body_preview");
        }
        "edit_note" => {
            if let Some(t) = s("title") {
                fields.push(field("Note", t));
            }
            preview = s("diff");
        }
        "set_note_fields" => {
            if let Some(t) = s("title") {
                fields.push(field("Note", t));
            }
            for ch in p
                .get("changes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let name = ch.get("field").and_then(Value::as_str).unwrap_or("field");
                fields.push(ActionField {
                    label: label_of(name),
                    value: ch
                        .get("after")
                        .and_then(shown)
                        .unwrap_or_else(|| "none".into()),
                    before: Some(
                        ch.get("before")
                            .and_then(shown)
                            .unwrap_or_else(|| "none".into()),
                    ),
                });
            }
        }
        "snooze" => {
            let what = if a.params.get("target_kind").and_then(Value::as_str) == Some("message") {
                "Message"
            } else {
                "Thread"
            };
            if let Some(t) = s("title") {
                fields.push(field(what, t));
            }
            if let Some(u) = s("until") {
                fields.push(field("Until", u));
            }
        }
        "archive" => {
            let what = if a.params.get("target_kind").and_then(Value::as_str) == Some("message") {
                "Message"
            } else {
                "Thread"
            };
            if let Some(t) = s("title") {
                fields.push(field(what, t));
            }
            if let Some(w) = s("write_back") {
                fields.push(field("Source", w));
            }
        }
        "resume_session" | "session_prompt" => {
            if let Some(n) = s("name") {
                fields.push(field("Session", n));
            }
            if let Some(c) = s("cwd") {
                fields.push(field("Folder", c));
            }
            if let Some(st) = s("state") {
                fields.push(field("State", st));
            }
            preview = s("prompt");
        }
        "create_automation" => {
            if let Some(n) = s("name") {
                fields.push(field("Name", n));
            }
            if let Some(t) = s("trigger_phrase") {
                fields.push(field("When", t));
            }
            if let Some(x) = s("action_phrase") {
                fields.push(field("Does", x));
            }
            if let Some(e) = p.get("enabled").and_then(Value::as_bool) {
                fields.push(field("Enabled", if e { "yes" } else { "no" }));
            }
        }
        _ => {
            if let Value::Object(m) = p {
                for (k, v) in m {
                    if let Some(x) = shown(v) {
                        fields.push(field(&label_of(k), x));
                    }
                }
            }
        }
    }
    (fields, preview)
}

/// A result line without the Markdown emphasis the core writes.
fn plain(line: &str) -> String {
    line.replace('*', "")
}

impl ChatAction {
    pub fn from_view(a: &ActionView) -> ChatAction {
        let (fields, preview) = card(a);
        ChatAction {
            id: a.action_id.clone(),
            kind: edge_action_kind(&a.kind).into(),
            title: a.title.clone(),
            fields,
            preview,
            state: edge_action_state(&a.state).into(),
            result: a.result_line.as_deref().map(plain),
            result_route: a.route.clone(),
            error: a.error.clone(),
            turn_id: a.turn_id.clone(),
            thread_id: a.thread_id.clone(),
            summary: a.summary.clone(),
            created_at: a.created_at.clone(),
            decided_at: a.decided_at.clone(),
        }
    }
}

/// One turn of the flat list (`ChatTurn`): the user's message, or the reply
/// whose `id` is the turn id. `model`, `effort`, `usage`, `uncited` and
/// `no_sources_label` are beyond the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatTurn {
    pub id: String,
    /// `user` | `assistant`.
    pub role: String,
    pub text: String,
    pub created_at: String,
    /// Replies: `streaming` | `finished` | `failed` | `stopped`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<Vec<ActivityGroup>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citations: Option<Vec<Citation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ChatAction>>,
    /// User turns: the items passed as retrieved context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<ItemRef>>,
    /// Replies: every token of the turn's steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_sources: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uncited: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_sources_label: Option<String>,
}

/// The reply's status word.
pub fn turn_status(state: &str, stopped: bool) -> &'static str {
    match (state, stopped) {
        ("running", _) => "streaming",
        (_, true) => "stopped",
        ("failed", _) => "failed",
        _ => "finished",
    }
}

/// The user turn of an exchange.
pub fn user_turn(t: &TurnView) -> ChatTurn {
    ChatTurn {
        id: t.user.id.clone(),
        role: "user".into(),
        text: t.user.text.clone(),
        created_at: t.user.created_at.clone(),
        status: None,
        error: None,
        activity: None,
        citations: None,
        actions: None,
        attachments: Some(
            t.user
                .attachments
                .iter()
                .map(ItemRef::from_attachment)
                .collect(),
        ),
        tokens: None,
        pinned: None,
        no_sources: None,
        model: None,
        effort: None,
        usage: None,
        uncited: None,
        no_sources_label: None,
    }
}

/// The reply of an exchange (`id` = the turn id).
pub fn assistant_turn(t: &TurnView) -> ChatTurn {
    let reply = t.reply.as_ref();
    ChatTurn {
        id: t.turn_id.clone(),
        role: "assistant".into(),
        text: reply.map(|r| r.text.clone()).unwrap_or_default(),
        created_at: reply
            .map(|r| r.created_at.clone())
            .unwrap_or_else(|| t.started_at.clone()),
        status: Some(turn_status(&t.state, t.stopped).into()),
        error: t.error.clone(),
        activity: Some(t.activity.iter().map(group_of).collect()),
        citations: Some(
            reply
                .map(|r| r.citations.iter().map(Citation::from_stored).collect())
                .unwrap_or_default(),
        ),
        actions: Some(t.actions.iter().map(ChatAction::from_view).collect()),
        attachments: None,
        tokens: Some(t.usage.total),
        pinned: Some(reply.is_some_and(|r| r.pinned)),
        no_sources: Some(t.no_sources_label.is_some()),
        model: reply.and_then(|r| r.model.clone()),
        effort: Some(t.effort.clone()),
        usage: Some(t.usage),
        uncited: Some(t.uncited),
        no_sources_label: t.no_sources_label.clone(),
    }
}

/// A conversation as the thread shows it (`ChatThread`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatThread {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    /// `auto` | `quick` | `thorough` | `local_only`.
    pub effort: String,
    pub connectors: ChatConnectors,
    pub turns: Vec<ChatTurn>,
}

impl ChatThread {
    pub fn from_detail(d: &ThreadDetail) -> ChatThread {
        let mut turns = Vec::with_capacity(d.turns.len() * 2);
        for t in &d.turns {
            turns.push(user_turn(t));
            turns.push(assistant_turn(t));
        }
        ChatThread {
            id: d.id.clone(),
            title: d.title.clone(),
            created_at: d.created_at.clone(),
            updated_at: d.updated_at.clone(),
            effort: d.effort.clone(),
            connectors: d.sources,
            turns,
        }
    }
}

/// The stored reply of a turn as the contract shows it.
pub fn reply(conn: &Connection, thread_id: &str, turn_id: &str) -> Result<Option<ChatTurn>> {
    Ok(store::thread(conn, thread_id)?.and_then(|d| {
        d.turns
            .iter()
            .find(|t| t.turn_id == turn_id)
            .map(assistant_turn)
    }))
}

/// The Sources panel of a turn (`ChatSources`; `thread_id`, `attachments`,
/// `usage` and `items` are beyond the contract).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatSources {
    pub turn_id: String,
    pub calls: Vec<ToolCall>,
    pub tokens: Option<i64>,
    #[serde(default)]
    pub thread_id: String,
    #[serde(default)]
    pub attachments: Vec<ItemRef>,
    #[serde(default)]
    pub usage: TokenUsage,
    #[serde(default)]
    pub items: Vec<ItemRef>,
}

impl ChatSources {
    pub fn from_stored(s: &store::TurnSources) -> ChatSources {
        ChatSources {
            turn_id: s.turn_id.clone(),
            calls: s.entries.iter().map(ToolCall::from_row).collect(),
            tokens: Some(s.usage.total),
            thread_id: s.thread_id.clone(),
            attachments: s.attachments.iter().map(ItemRef::from_attachment).collect(),
            usage: s.usage,
            items: s.items.iter().map(ItemRef::from_source).collect(),
        }
    }
}

/// `chat.delta`: append `text`; upsert `activity` by id. `seq` and `kind`
/// (`text` | `activity`) are beyond the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatDelta {
    pub turn_id: String,
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<ActivityGroup>,
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub kind: String,
}

/// `chat.turn`: `started` | `finished` | `failed` | `stopped`; `turn` is the
/// stored reply once it ended. `stopped`, `citations`, `no_sources_label`
/// and `uncited` are beyond the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatTurnEvent {
    pub turn_id: String,
    pub thread_id: String,
    pub status: String,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<ChatTurn>,
    #[serde(default)]
    pub stopped: bool,
    #[serde(default)]
    pub citations: Vec<Citation>,
    #[serde(default)]
    pub no_sources_label: Option<String>,
    #[serde(default)]
    pub uncited: bool,
}

/// `chat.action`: a proposal, later its result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatActionEvent {
    pub turn_id: String,
    pub thread_id: String,
    pub action: ChatAction,
}

impl ChatActionEvent {
    pub fn of(a: &ActionView) -> ChatActionEvent {
        ChatActionEvent {
            turn_id: a.turn_id.clone(),
            thread_id: a.thread_id.clone(),
            action: ChatAction::from_view(a),
        }
    }
}

/// Which connectors have an account (the `+` menu's reasons).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountsPresent {
    pub gmail: bool,
    pub slack: bool,
    pub calendar: bool,
}

/// The accounts behind the Gmail, Slack and Calendar toggles.
pub fn accounts_present(conn: &Connection) -> Result<AccountsPresent> {
    let mut out = AccountsPresent::default();
    let mut stmt = conn.prepare("SELECT provider, services_json FROM accounts")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (provider, services) = row?;
        let services: Vec<String> = serde_json::from_str(&services).unwrap_or_default();
        match provider.as_str() {
            "google" => {
                out.gmail |= services.is_empty() || services.iter().any(|s| s == "gmail");
                out.calendar |= services.is_empty() || services.iter().any(|s| s == "calendar");
            }
            "slack" => out.slack = true,
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kinds_and_states_follow_the_contract() {
        assert_eq!(edge_kind("thread"), "message");
        assert_eq!(edge_kind("calendar_event"), "event");
        assert_eq!(edge_kind("note"), "note");
        assert_eq!(edge_action_kind("set_note_fields"), "set_fields");
        assert_eq!(edge_action_kind("snooze"), "snooze_message");
        assert_eq!(edge_action_kind("archive"), "archive_message");
        assert_eq!(edge_action_state("proposed"), "pending");
        assert_eq!(edge_action_state("confirmed"), "done");
        assert_eq!(turn_status("running", false), "streaming");
        assert_eq!(turn_status("finished", true), "stopped");
        assert_eq!(turn_status("failed", false), "failed");
        assert_eq!(file_path("file:///a/b.md"), "/a/b.md");
    }

    #[test]
    fn inputs_become_one_line() {
        assert_eq!(
            input_line(&json!({"query": "release", "status": "todo"})),
            "release · status todo"
        );
        assert_eq!(input_line(&json!({})), "");
        assert_eq!(
            input_line(&json!({"tags": ["a", "b"], "limit": 5})),
            "limit 5 · tags a, b"
        );
    }

    #[test]
    fn a_set_fields_card_draws_before_and_after() {
        let v = ActionView {
            action_id: "a1".into(),
            turn_id: "t1".into(),
            thread_id: "c1".into(),
            kind: "set_note_fields".into(),
            title: "Update note".into(),
            summary: "Plan".into(),
            preview: json!({"title": "Plan", "changes": [{"field": "status", "before": "todo", "after": "doing"}]}),
            params: json!({}),
            state: "confirmed".into(),
            result_line: Some("Set status doing on *Plan*".into()),
            error: None,
            route: Some("notes/n1".into()),
            created_at: "2026-10-21T12:00:00.000Z".into(),
            decided_at: None,
        };
        let a = ChatAction::from_view(&v);
        assert_eq!(a.kind, "set_fields");
        assert_eq!(a.state, "done");
        assert_eq!(a.result.as_deref(), Some("Set status doing on Plan"));
        assert_eq!(
            a.fields,
            vec![
                field("Note", "Plan"),
                ActionField {
                    label: "Status".into(),
                    value: "doing".into(),
                    before: Some("todo".into())
                }
            ]
        );
    }
}
