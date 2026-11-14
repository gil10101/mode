//! The Chat retrieval tools (06 §10.6, 03 §7.3): read-only SQL over FTS5 and
//! structured filters, bounded rows, snippets only, ids the model must
//! cite. Names and input shapes are the 06 contract; `get_item {kind, id}`
//! is the generic getter (events and single messages have no dedicated
//! one). Each result is capped at about 8 K tokens (32 000 characters).

use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

use super::citations::{SourceItem, route};
use crate::db::{Result, format_timestamp, parse_timestamp};
use crate::notes::ctx::TzMode;
use crate::queries::fts::{Fts, build};

/// Largest serialized tool result sent to the model (≈ 8 K tokens).
pub const RESULT_MAX_CHARS: usize = 32_000;
/// Largest note body returned by `get_note`.
pub const NOTE_BODY_MAX: usize = 16_000;
/// Largest thread text returned by `get_thread`.
pub const THREAD_MAX: usize = 32_000;

/// A tool as declared to the backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// Which tools an effort offers (03 §7.2: Quick = search tools only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSet {
    All,
    Search,
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

fn str_list() -> Value {
    json!({ "type": "array", "items": { "type": "string" } })
}

/// Every tool, in a fixed order (the cached prefix must not change, 06 §10.6).
pub fn specs(set: ToolSet) -> Vec<ToolSpec> {
    let all = vec![
        ToolSpec {
            name: "now",
            description: "Current local date, time, weekday and UTC offset. Call it before reasoning about today, tomorrow or this week.",
            input_schema: obj(json!({}), &[]),
        },
        ToolSpec {
            name: "search_notes",
            description: "Search the user's notes by text and fields. Returns ids, titles, status, urgent, when_date and an excerpt.",
            input_schema: obj(
                json!({
                    "text": { "type": "string" },
                    "status": str_list(),
                    "urgent": { "type": "boolean" },
                    "when": { "type": "array", "items": { "type": "string", "enum": ["overdue", "today", "tomorrow", "scheduled", "none"] } },
                    "tags": str_list(),
                    "folder": { "type": "string" },
                    "archived": { "type": "boolean" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                }),
                &[],
            ),
        },
        ToolSpec {
            name: "get_note",
            description: "One note: title, path, fields and body (up to 16 000 characters).",
            input_schema: obj(json!({ "id": { "type": "string" } }), &["id"]),
        },
        ToolSpec {
            name: "search_messages",
            description: "Search the user's email and chat messages. Tiers: urgent, normal, low (or vip, direct, ai-urgent, ai-normal). since / until are ISO dates or times.",
            input_schema: obj(
                json!({
                    "text": { "type": "string" },
                    "account_id": { "type": "string" },
                    "tier": str_list(),
                    "contact_id": { "type": "string" },
                    "since": { "type": "string" },
                    "until": { "type": "string" },
                    "unread_only": { "type": "boolean" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                }),
                &[],
            ),
        },
        ToolSpec {
            name: "get_thread",
            description: "A message thread (by thread id or one of its message ids): its messages with bodies where Mode still keeps them.",
            input_schema: obj(json!({ "id": { "type": "string" } }), &["id"]),
        },
        ToolSpec {
            name: "list_sessions",
            description: "The user's coding-agent sessions: state, repository, model, lines changed, last activity. States: working, waiting, failed, unread, idle, done, untracked.",
            input_schema: obj(
                json!({
                    "state": str_list(),
                    "repo": { "type": "string" },
                    "since": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20 }
                }),
                &[],
            ),
        },
        ToolSpec {
            name: "get_session",
            description: "One session: header, the last assistant message preview, files changed and the last 20 activity summaries.",
            input_schema: obj(json!({ "id": { "type": "string" } }), &["id"]),
        },
        ToolSpec {
            name: "calendar_lookup",
            description: "Calendar events overlapping the local dates from … to (inclusive, ISO YYYY-MM-DD).",
            input_schema: obj(
                json!({
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                    "account_id": { "type": "string" }
                }),
                &["from", "to"],
            ),
        },
        ToolSpec {
            name: "get_item",
            description: "Any item by kind and id: note, message, thread, session or calendar_event.",
            input_schema: obj(
                json!({
                    "kind": { "type": "string", "enum": ["note", "message", "thread", "session", "calendar_event"] },
                    "id": { "type": "string" }
                }),
                &["kind", "id"],
            ),
        },
    ];
    match set {
        ToolSet::All => all,
        ToolSet::Search => all
            .into_iter()
            .filter(|t| {
                matches!(
                    t.name,
                    "now"
                        | "search_notes"
                        | "search_messages"
                        | "list_sessions"
                        | "calendar_lookup"
                )
            })
            .collect(),
    }
}

/// The per-conversation connector toggles of the `+` menu (03 §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sources {
    #[serde(default = "yes")]
    pub gmail: bool,
    #[serde(default = "yes")]
    pub slack: bool,
    #[serde(default = "yes")]
    pub calendar: bool,
}

fn yes() -> bool {
    true
}

impl Default for Sources {
    fn default() -> Self {
        Sources {
            gmail: true,
            slack: true,
            calendar: true,
        }
    }
}

/// What a tool runs with.
#[derive(Debug, Clone, Copy)]
pub struct ToolCtx {
    pub now: OffsetDateTime,
    pub tz: TzMode,
    pub sources: Sources,
}

/// A tool's answer: the data for the model, the chips for the panel.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub data: Value,
    pub items: Vec<SourceItem>,
    pub error: Option<String>,
}

impl ToolOutput {
    fn error(msg: impl Into<String>) -> ToolOutput {
        let m = msg.into();
        ToolOutput {
            data: json!({ "error": m }),
            items: Vec::new(),
            error: Some(m),
        }
    }
}

/// The Sources panel / activity label of a tool (03 §7.1).
pub fn label(name: &str) -> &'static str {
    match name {
        "search_notes" => "Searched notes",
        "get_note" => "Read a note",
        "search_messages" => "Searched messages",
        "get_thread" => "Read a thread",
        "list_sessions" => "Listed sessions",
        "get_session" => "Read a session",
        "calendar_lookup" => "Looked up the calendar",
        "now" => "Checked the time",
        "get_item" => "Read an item",
        _ => "Used a tool",
    }
}

// ----------------------------------------------------------------- helpers

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

fn b(v: &Value, k: &str) -> Option<bool> {
    v.get(k).and_then(Value::as_bool)
}

fn list(v: &Value, k: &str) -> Vec<String> {
    match v.get(k) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => Vec::new(),
    }
}

fn limit(v: &Value) -> i64 {
    v.get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(10)
        .clamp(1, 20)
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

struct Local {
    today: String,
    tomorrow: String,
}

fn local_days(conn: &Connection, ctx: &ToolCtx) -> Local {
    crate::urgency::with_tz(conn, ctx.tz, |z| {
        let d = z.local_date(ctx.now);
        let t = d.next_day().unwrap_or(d);
        Local {
            today: crate::notes::fields::fmt_date(d),
            tomorrow: crate::notes::fields::fmt_date(t),
        }
    })
}

/// `YYYY-MM-DD HH:MM` local of a stored UTC timestamp.
fn local_time(conn: &Connection, ctx: &ToolCtx, ts: &str) -> String {
    let Some(t) = parse_timestamp(ts) else {
        return ts.to_string();
    };
    crate::urgency::with_tz(conn, ctx.tz, |z| {
        let l = z.to_local(t);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            l.year(),
            u8::from(l.month()),
            l.day(),
            l.hour(),
            l.minute()
        )
    })
}

/// A local date (`YYYY-MM-DD`) or an instant → the UTC timestamp it starts at.
fn bound(conn: &Connection, ctx: &ToolCtx, raw: &str, end_of_day: bool) -> Option<String> {
    if raw.len() > 10 {
        return parse_timestamp(raw).map(format_timestamp);
    }
    let d = crate::notes::fields::parse_date(raw)?;
    let d = if end_of_day { d.next_day()? } else { d };
    Some(crate::urgency::with_tz(conn, ctx.tz, |z| {
        format_timestamp(z.local_midnight(d))
    }))
}

fn rows_of(
    conn: &Connection,
    sql: &str,
    params: Vec<Sql>,
    mut f: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Value>,
) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql)?;
    let out = stmt
        .query_map(params_from_iter(params), |r| f(r))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(out)
}

/// Shrink an array result until it fits [`RESULT_MAX_CHARS`].
fn cap(mut data: Value, key: Option<&str>) -> Value {
    loop {
        let len = data.to_string().len();
        if len <= RESULT_MAX_CHARS {
            return data;
        }
        let arr = match key {
            Some(k) => data.get_mut(k).and_then(Value::as_array_mut),
            None => data.as_array_mut(),
        };
        match arr {
            Some(a) if a.len() > 1 => {
                a.pop();
            }
            _ => {
                let t = data.to_string();
                return json!({ "truncated": clip(&t, RESULT_MAX_CHARS - 100) });
            }
        }
    }
}

// ------------------------------------------------------------------- tools

/// Run one tool. Unknown names and bad inputs answer `{error}` so the model
/// can correct itself; database failures are errors.
pub fn run(conn: &Connection, ctx: &ToolCtx, name: &str, input: &Value) -> Result<ToolOutput> {
    let out = match name {
        "now" => now(conn, ctx),
        "search_notes" => search_notes(conn, ctx, input)?,
        "get_note" => match s(input, "id") {
            Some(id) => get_note(conn, ctx, &id)?,
            None => ToolOutput::error("get_note needs an id"),
        },
        "search_messages" => search_messages(conn, ctx, input)?,
        "get_thread" => match s(input, "id") {
            Some(id) => get_thread(conn, ctx, &id)?,
            None => ToolOutput::error("get_thread needs an id"),
        },
        "list_sessions" => list_sessions(conn, ctx, input)?,
        "get_session" => match s(input, "id") {
            Some(id) => get_session(conn, ctx, &id)?,
            None => ToolOutput::error("get_session needs an id"),
        },
        "calendar_lookup" => calendar_lookup(conn, ctx, input)?,
        "get_item" => match (s(input, "kind"), s(input, "id")) {
            (Some(k), Some(id)) => get_item(conn, ctx, &super::citations::normalise_kind(&k), &id)?,
            _ => ToolOutput::error("get_item needs a kind and an id"),
        },
        other => ToolOutput::error(format!("there is no tool named {other}")),
    };
    Ok(out)
}

fn now(conn: &Connection, ctx: &ToolCtx) -> ToolOutput {
    let (local, weekday, offset) = crate::urgency::with_tz(conn, ctx.tz, |z| {
        let l = z.to_local(ctx.now);
        let off = z.to_utc(l) - ctx.now;
        let mins = -(off.whole_minutes());
        (
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}",
                l.year(),
                u8::from(l.month()),
                l.day(),
                l.hour(),
                l.minute()
            ),
            l.weekday().to_string(),
            format!(
                "{}{:02}:{:02}",
                if mins < 0 { '-' } else { '+' },
                mins.abs() / 60,
                mins.abs() % 60
            ),
        )
    });
    ToolOutput {
        data: json!({
            "now_utc": format_timestamp(ctx.now),
            "local": local,
            "date": &local[..10],
            "weekday": weekday,
            "utc_offset": offset,
        }),
        items: Vec::new(),
        error: None,
    }
}

fn note_item(id: &str, title: &str, snippet: &str, meta: String) -> SourceItem {
    SourceItem {
        kind: "note".into(),
        id: id.into(),
        title: title.into(),
        snippet: clip(snippet, 200),
        route: route("note", id),
        meta: Some(meta),
    }
}

fn search_notes(conn: &Connection, ctx: &ToolCtx, v: &Value) -> Result<ToolOutput> {
    let text = s(v, "text");
    let fts = text.as_deref().and_then(|t| build(t, Fts::Notes, true));
    let match_expr = fts.as_ref().and_then(|q| q.match_expr.clone());
    let mut sql = String::from(
        "SELECT n.id, n.title, n.status, n.urgent, n.when_date, n.excerpt, n.path, n.updated_at",
    );
    let mut params: Vec<Sql> = Vec::new();
    if match_expr.is_some() {
        sql.push_str(", snippet(notes_fts, 1, '', '', '…', 16) FROM notes n JOIN notes_fts ON notes_fts.rowid = n.seq");
    } else {
        sql.push_str(", NULL FROM notes n");
    }
    sql.push_str(" WHERE n.deleted_at IS NULL");
    if let Some(m) = &match_expr {
        sql.push_str(" AND notes_fts MATCH ?");
        params.push(Sql::Text(m.clone()));
    } else if text.is_some() && fts.as_ref().and_then(|q| q.in_scope.clone()).is_none() {
        return Ok(ToolOutput {
            data: json!({ "notes": [] }),
            items: Vec::new(),
            error: None,
        });
    }
    match b(v, "archived") {
        Some(true) => {}
        _ => sql.push_str(" AND n.archived_at IS NULL"),
    }
    let status = list(v, "status");
    if !status.is_empty() {
        sql.push_str(&format!(
            " AND n.status IN ({})",
            placeholders(status.len())
        ));
        params.extend(status.into_iter().map(|x| Sql::Text(x.to_lowercase())));
    }
    if let Some(u) = b(v, "urgent") {
        sql.push_str(" AND n.urgent = ?");
        params.push(Sql::Integer(i64::from(u)));
    }
    let when = list(v, "when");
    if !when.is_empty() {
        let days = local_days(conn, ctx);
        let closed = crate::settings::closed_statuses_in(conn)?;
        let mut ors = Vec::new();
        for w in when {
            match w.as_str() {
                "overdue" => {
                    let mut c = String::from("(n.when_date < ?");
                    params.push(Sql::Text(days.today.clone()));
                    if !closed.is_empty() {
                        c.push_str(&format!(
                            " AND n.status NOT IN ({})",
                            placeholders(closed.len())
                        ));
                        params.extend(closed.iter().map(|x| Sql::Text(x.clone())));
                    }
                    c.push(')');
                    ors.push(c);
                }
                "today" => {
                    ors.push("n.when_date = ?".into());
                    params.push(Sql::Text(days.today.clone()));
                }
                "tomorrow" => {
                    ors.push("n.when_date = ?".into());
                    params.push(Sql::Text(days.tomorrow.clone()));
                }
                "scheduled" => {
                    ors.push("n.when_date > ?".into());
                    params.push(Sql::Text(days.tomorrow.clone()));
                }
                "none" => ors.push("n.when_date IS NULL".into()),
                _ => {}
            }
        }
        if !ors.is_empty() {
            sql.push_str(&format!(" AND ({})", ors.join(" OR ")));
        }
    }
    for tag in list(v, "tags") {
        sql.push_str(
            " AND EXISTS (SELECT 1 FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
                          WHERE nt.note_id = n.id AND t.deleted_at IS NULL AND t.name_norm = ?)",
        );
        params.push(Sql::Text(crate::repo::normalize_tag(
            tag.trim_start_matches('#'),
        )));
    }
    let scope = s(v, "folder").or_else(|| fts.as_ref().and_then(|q| q.in_scope.clone()));
    if let Some(f) = scope {
        let f = f.trim_matches('/').to_string();
        sql.push_str(
            " AND EXISTS (SELECT 1 FROM folders fo WHERE fo.deleted_at IS NULL
                            AND (lower(fo.path) = lower(?) OR lower(fo.name) = lower(?))
                            AND (n.path LIKE fo.path || '/%'))",
        );
        params.push(Sql::Text(f.clone()));
        params.push(Sql::Text(f));
    }
    if match_expr.is_some() {
        sql.push_str(" ORDER BY bm25(notes_fts, 4.0, 1.0, 2.0, 0.0), n.updated_at DESC");
    } else {
        sql.push_str(" ORDER BY n.urgent DESC, n.updated_at DESC");
    }
    sql.push_str(" LIMIT ?");
    params.push(Sql::Integer(limit(v)));
    let mut items = Vec::new();
    let rows = rows_of(conn, &sql, params, |r| {
        let id: String = r.get(0)?;
        let title: String = r.get(1)?;
        let status: String = r.get(2)?;
        let urgent: bool = r.get(3)?;
        let when: Option<String> = r.get(4)?;
        let excerpt: String = r.get(5)?;
        let hit: Option<String> = r.get(8)?;
        let snippet = hit.filter(|h| !h.trim().is_empty()).unwrap_or(excerpt);
        let meta = match &when {
            Some(w) => format!("{status} · {w}"),
            None => status.clone(),
        };
        items.push(note_item(&id, &title, &snippet, meta));
        Ok(json!({
            "id": id, "title": title, "status": status, "urgent": urgent,
            "when_date": when, "excerpt": clip(&snippet, 300),
        }))
    })?;
    Ok(ToolOutput {
        data: cap(json!({ "notes": rows }), Some("notes")),
        items,
        error: None,
    })
}

/// `(id, title, path, status, urgent, when_date, when_raw, body, folder path)`.
type NoteRow = (
    String,
    String,
    String,
    String,
    bool,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
);

fn get_note(conn: &Connection, _ctx: &ToolCtx, id: &str) -> Result<ToolOutput> {
    let row: Option<NoteRow> = conn
        .query_row(
            "SELECT n.id, n.title, n.path, n.status, n.urgent, n.when_date, n.when_raw, n.body,
                    (SELECT f.path FROM folders f WHERE f.id = n.folder_id)
               FROM notes n WHERE n.id = ?1 AND n.deleted_at IS NULL",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .optional()?;
    let Some((id, title, path, status, urgent, when_date, when_raw, body, folder)) = row else {
        return Ok(ToolOutput::error(format!("no note with id {id}")));
    };
    let tags: Vec<String> = conn
        .prepare(
            "SELECT t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
              WHERE nt.note_id = ?1 AND t.deleted_at IS NULL ORDER BY nt.position",
        )?
        .query_map([&id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let item = note_item(&id, &title, &clip(&body, 200), status.clone());
    Ok(ToolOutput {
        data: json!({
            "id": id, "title": title, "path": path,
            "fields": { "status": status, "urgent": urgent, "when": when_raw.or(when_date), "tags": tags, "folder": folder },
            "body": clip(&body, NOTE_BODY_MAX),
        }),
        items: vec![item],
        error: None,
    })
}

fn tiers(raw: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for t in raw {
        let t = t.to_ascii_lowercase();
        let add: &[&str] = match t.as_str() {
            "urgent" => &["vip", "direct", "ai-urgent"],
            "normal" => &["ai-normal", "unscored"],
            "low" => &["low"],
            "vip" => &["vip"],
            "direct" => &["direct"],
            "ai-urgent" => &["ai-urgent"],
            "ai-normal" => &["ai-normal"],
            "unscored" => &["unscored"],
            _ => &[],
        };
        for a in add {
            if !out.iter().any(|x: &String| x == a) {
                out.push((*a).to_string());
            }
        }
    }
    out
}

fn provider_filter(ctx: &ToolCtx, alias: &str) -> String {
    let mut off = Vec::new();
    if !ctx.sources.gmail {
        off.push("'google'");
    }
    if !ctx.sources.slack {
        off.push("'slack'");
    }
    if off.is_empty() {
        String::new()
    } else {
        format!(" AND {alias}.provider NOT IN ({})", off.join(", "))
    }
}

fn tier_word(t: &str) -> &'static str {
    match t {
        "vip" | "direct" | "ai-urgent" => "urgent",
        "low" => "low",
        _ => "normal",
    }
}

fn search_messages(conn: &Connection, ctx: &ToolCtx, v: &Value) -> Result<ToolOutput> {
    let text = s(v, "text");
    let fts = text.as_deref().and_then(|t| build(t, Fts::Messages, true));
    let match_expr = fts.as_ref().and_then(|q| q.match_expr.clone());
    let mut params: Vec<Sql> = Vec::new();
    let mut sql = String::from(
        "SELECT m.id, m.thread_id, m.sender_name, m.subject, m.snippet, m.received_at, m.urgency_tier, m.urgency_why",
    );
    if match_expr.is_some() {
        sql.push_str(", snippet(messages_fts, -1, '', '', '…', 16) FROM messages m JOIN messages_fts ON messages_fts.rowid = m.seq");
    } else {
        sql.push_str(", NULL FROM messages m");
    }
    sql.push_str(" WHERE 1 = 1");
    sql.push_str(&provider_filter(ctx, "m"));
    if let Some(m) = &match_expr {
        sql.push_str(" AND messages_fts MATCH ?");
        params.push(Sql::Text(m.clone()));
    }
    if let Some(scope) = fts.as_ref().and_then(|q| q.in_scope.clone()) {
        sql.push_str(" AND m.channel_name = ?");
        params.push(Sql::Text(scope));
    }
    if let Some(a) = s(v, "account_id") {
        sql.push_str(" AND m.account_id = ?");
        params.push(Sql::Text(a));
    }
    let t = tiers(list(v, "tier"));
    if !t.is_empty() {
        sql.push_str(&format!(
            " AND m.urgency_tier IN ({})",
            placeholders(t.len())
        ));
        params.extend(t.into_iter().map(Sql::Text));
    }
    if let Some(c) = s(v, "contact_id") {
        sql.push_str(" AND m.sender_contact_id = ?");
        params.push(Sql::Text(c));
    }
    if let Some(since) = s(v, "since").and_then(|x| bound(conn, ctx, &x, false)) {
        sql.push_str(" AND m.received_at >= ?");
        params.push(Sql::Text(since));
    }
    if let Some(until) = s(v, "until").and_then(|x| bound(conn, ctx, &x, true)) {
        sql.push_str(" AND m.received_at < ?");
        params.push(Sql::Text(until));
    }
    if b(v, "unread_only") == Some(true) {
        sql.push_str(" AND m.is_read = 0");
    }
    if match_expr.is_some() {
        sql.push_str(" ORDER BY bm25(messages_fts, 3.0, 1.0, 1.0, 2.0), m.received_at DESC");
    } else {
        sql.push_str(" ORDER BY m.received_at DESC");
    }
    sql.push_str(" LIMIT ?");
    params.push(Sql::Integer(limit(v)));
    let mut items = Vec::new();
    let rows = rows_of(conn, &sql, params, |r| {
        let id: String = r.get(0)?;
        let thread_id: Option<String> = r.get(1)?;
        let sender: String = r.get(2)?;
        let subject: Option<String> = r.get(3)?;
        let snippet: String = r.get(4)?;
        let received: String = r.get(5)?;
        let tier: String = r.get(6)?;
        let why: Option<String> = r.get(7)?;
        let hit: Option<String> = r.get(8)?;
        let shown = hit.filter(|h| !h.trim().is_empty()).unwrap_or(snippet);
        let title = subject
            .clone()
            .filter(|x| !x.trim().is_empty())
            .unwrap_or_else(|| format!("{sender}: {}", clip(&shown, 60)));
        items.push(SourceItem {
            kind: "message".into(),
            id: id.clone(),
            title,
            snippet: clip(&shown, 200),
            route: route("message", &id),
            meta: Some(format!("{sender} · {}", local_time(conn, ctx, &received))),
        });
        Ok(json!({
            "id": id, "thread_id": thread_id, "sender_name": sender, "subject": subject,
            "snippet": clip(&shown, 300), "received_at": received,
            "received_local": local_time(conn, ctx, &received),
            "urgency": tier_word(&tier), "urgency_tier": tier, "urgency_why": why,
        }))
    })?;
    Ok(ToolOutput {
        data: cap(json!({ "messages": rows }), Some("messages")),
        items,
        error: None,
    })
}

fn get_thread(conn: &Connection, ctx: &ToolCtx, id: &str) -> Result<ToolOutput> {
    let pf = provider_filter(ctx, "t");
    let thread: Option<(String, String)> = conn
        .query_row(
            &format!(
                "SELECT t.id, t.title FROM threads t
                  WHERE (t.id = ?1 OR t.id = (SELECT thread_id FROM messages WHERE id = ?1)){pf}"
            ),
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((tid, title)) = thread else {
        return Ok(ToolOutput::error(format!("no thread with id {id}")));
    };
    let mut msgs: Vec<(String, String, String, Option<String>, String)> = conn
        .prepare(
            "SELECT id, sender_name, received_at, body_text, snippet FROM messages
              WHERE thread_id = ?1 ORDER BY received_at",
        )?
        .query_map([&tid], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    // Keep the newest messages within the budget.
    let mut total = 0usize;
    let mut kept = Vec::new();
    while let Some((mid, sender, at, body, snippet)) = msgs.pop() {
        let text = body.filter(|b| !b.trim().is_empty()).unwrap_or(snippet);
        let text = clip(&text, THREAD_MAX / 2);
        total += text.len() + sender.len() + 40;
        if total > THREAD_MAX && !kept.is_empty() {
            break;
        }
        kept.push((mid, sender, at, text));
    }
    kept.reverse();
    let mut items = vec![SourceItem {
        kind: "thread".into(),
        id: tid.clone(),
        title: title.clone(),
        snippet: kept.last().map(|k| clip(&k.3, 200)).unwrap_or_default(),
        route: route("thread", &tid),
        meta: Some(format!("{} messages", kept.len())),
    }];
    let messages: Vec<Value> = kept
        .iter()
        .map(|(mid, sender, at, text)| {
            items.push(SourceItem {
                kind: "message".into(),
                id: mid.clone(),
                title: format!("{sender}: {}", clip(&title, 60)),
                snippet: clip(text, 200),
                route: route("message", mid),
                meta: Some(local_time(conn, ctx, at)),
            });
            json!({ "id": mid, "sender_name": sender, "received_at": at,
                    "received_local": local_time(conn, ctx, at), "text": text })
        })
        .collect();
    Ok(ToolOutput {
        data: json!({ "id": tid, "title": title, "messages": messages }),
        items,
        error: None,
    })
}

fn session_item(id: &str, name: &str, state: &str, snippet: &str) -> SourceItem {
    SourceItem {
        kind: "session".into(),
        id: id.into(),
        title: name.into(),
        snippet: clip(snippet, 200),
        route: route("session", id),
        meta: Some(state.into()),
    }
}

fn list_sessions(conn: &Connection, ctx: &ToolCtx, v: &Value) -> Result<ToolOutput> {
    let mut sql = String::from(
        "SELECT s.id, s.display_name, s.state, s.repo_root, s.model, s.lines_added, s.lines_removed,
                s.last_event_at, s.title, s.cwd, s.git_branch
           FROM sessions s WHERE s.parent_session_id IS NULL",
    );
    let mut params: Vec<Sql> = Vec::new();
    let states = list(v, "state");
    if !states.is_empty() {
        sql.push_str(&format!(" AND s.state IN ({})", placeholders(states.len())));
        params.extend(states.into_iter().map(|x| Sql::Text(x.to_lowercase())));
    }
    if let Some(repo) = s(v, "repo") {
        sql.push_str(
            " AND (s.repo_root LIKE '%' || ? || '%' OR s.cwd LIKE '%' || ? || '%' OR s.display_name LIKE '%' || ? || '%')",
        );
        params.extend([
            Sql::Text(repo.clone()),
            Sql::Text(repo.clone()),
            Sql::Text(repo),
        ]);
    }
    if let Some(since) = s(v, "since").and_then(|x| bound(conn, ctx, &x, false)) {
        sql.push_str(" AND s.last_event_at >= ?");
        params.push(Sql::Text(since));
    }
    sql.push_str(" ORDER BY s.last_event_at DESC LIMIT ?");
    params.push(Sql::Integer(limit(v)));
    let mut items = Vec::new();
    let rows = rows_of(conn, &sql, params, |r| {
        let id: String = r.get(0)?;
        let name: String = r.get(1)?;
        let state: String = r.get(2)?;
        let last: String = r.get(7)?;
        let title: Option<String> = r.get(8)?;
        items.push(session_item(
            &id,
            &name,
            &state,
            title.as_deref().unwrap_or(""),
        ));
        Ok(json!({
            "id": id, "display_name": name, "state": state, "repo_root": r.get::<_, Option<String>>(3)?,
            "model": r.get::<_, Option<String>>(4)?, "lines_added": r.get::<_, i64>(5)?,
            "lines_removed": r.get::<_, i64>(6)?, "last_event_at": last,
            "last_event_local": local_time(conn, ctx, &last), "title": title,
            "branch": r.get::<_, Option<String>>(10)?,
        }))
    })?;
    Ok(ToolOutput {
        data: cap(json!({ "sessions": rows }), Some("sessions")),
        items,
        error: None,
    })
}

fn get_session(conn: &Connection, ctx: &ToolCtx, id: &str) -> Result<ToolOutput> {
    type Head = (
        String,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        i64,
        i64,
        i64,
        i64,
        Option<String>,
    );
    let head: Option<Head> = conn
        .query_row(
            "SELECT id, display_name, title, state, cwd, repo_root, git_branch, model, started_at, ended_at,
                    lines_added, lines_removed, files_changed, num_turns, last_assistant_preview
               FROM sessions WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
                    r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?,
                    r.get(14)?,
                ))
            },
        )
        .optional()?;
    let Some(h) = head else {
        return Ok(ToolOutput::error(format!("no session with id {id}")));
    };
    let files: Vec<Value> = crate::queries::session_files(conn, &h.0)?
        .into_iter()
        .take(50)
        .map(|f| json!({ "path": f.rel_path.unwrap_or(f.path), "change": f.change_kind, "added": f.lines_added, "removed": f.lines_removed }))
        .collect();
    let summaries: Vec<Value> = conn
        .prepare(
            "SELECT occurred_at, event, tool_name, summary FROM session_events
              WHERE session_id = ?1 AND summary IS NOT NULL ORDER BY seq DESC LIMIT 20",
        )?
        .query_map([&h.0], |r| {
            Ok(
                json!({ "at": r.get::<_, String>(0)?, "event": r.get::<_, String>(1)?,
                       "tool": r.get::<_, Option<String>>(2)?, "summary": r.get::<_, String>(3)? }),
            )
        })?
        .collect::<rusqlite::Result<_>>()?;
    let preview = h.14.clone().unwrap_or_default();
    let item = session_item(&h.0, &h.1, &h.3, &preview);
    Ok(ToolOutput {
        data: cap(
            json!({
                "id": h.0, "display_name": h.1, "title": h.2, "state": h.3, "cwd": h.4,
                "repo_root": h.5, "branch": h.6, "model": h.7, "started_at": h.8,
                "started_local": local_time(conn, ctx, &h.8), "ended_at": h.9,
                "lines_added": h.10, "lines_removed": h.11, "files_changed": h.12, "turns": h.13,
                "last_assistant_preview": clip(&preview, 2000), "files": files, "recent": summaries,
            }),
            Some("recent"),
        ),
        items: vec![item],
        error: None,
    })
}

fn event_item(
    conn: &Connection,
    ctx: &ToolCtx,
    id: &str,
    title: &str,
    starts: &str,
    location: Option<&str>,
) -> SourceItem {
    SourceItem {
        kind: "calendar_event".into(),
        id: id.into(),
        title: title.into(),
        snippet: location.map(str::to_string).unwrap_or_default(),
        route: route("calendar_event", id),
        meta: Some(local_time(conn, ctx, starts)),
    }
}

const EVENT_COLUMNS: &str = "e.id, e.title, e.starts_at, e.ends_at, e.location, e.join_url, e.my_response, e.all_day, e.status";

fn event_json(
    conn: &Connection,
    ctx: &ToolCtx,
    r: &rusqlite::Row<'_>,
    items: &mut Vec<SourceItem>,
) -> rusqlite::Result<Value> {
    let id: String = r.get(0)?;
    let title: String = r.get(1)?;
    let starts: String = r.get(2)?;
    let ends: String = r.get(3)?;
    let location: Option<String> = r.get(4)?;
    items.push(event_item(
        conn,
        ctx,
        &id,
        &title,
        &starts,
        location.as_deref(),
    ));
    Ok(json!({
        "id": id, "title": title, "starts_at": starts, "ends_at": ends,
        "starts_local": local_time(conn, ctx, &starts), "ends_local": local_time(conn, ctx, &ends),
        "location": location, "join_url": r.get::<_, Option<String>>(5)?,
        "my_response": r.get::<_, Option<String>>(6)?, "all_day": r.get::<_, bool>(7)?,
        "status": r.get::<_, String>(8)?,
    }))
}

fn calendar_lookup(conn: &Connection, ctx: &ToolCtx, v: &Value) -> Result<ToolOutput> {
    if !ctx.sources.calendar {
        return Ok(ToolOutput::error("Calendar is off for this conversation"));
    }
    let (Some(from), Some(to)) = (s(v, "from"), s(v, "to")) else {
        return Ok(ToolOutput::error(
            "calendar_lookup needs from and to (YYYY-MM-DD)",
        ));
    };
    let (Some(start), Some(end)) = (bound(conn, ctx, &from, false), bound(conn, ctx, &to, true))
    else {
        return Ok(ToolOutput::error("from and to must be dates (YYYY-MM-DD)"));
    };
    let mut sql = format!(
        "SELECT {EVENT_COLUMNS} FROM calendar_events e
          WHERE e.status <> 'cancelled' AND e.ends_at > ? AND e.starts_at < ?"
    );
    let mut params = vec![Sql::Text(start), Sql::Text(end)];
    if let Some(a) = s(v, "account_id") {
        sql.push_str(" AND e.account_id = ?");
        params.push(Sql::Text(a));
    }
    sql.push_str(" ORDER BY e.all_day DESC, e.starts_at LIMIT 100");
    let mut items = Vec::new();
    let rows = rows_of(conn, &sql, params, |r| event_json(conn, ctx, r, &mut items))?;
    Ok(ToolOutput {
        data: cap(json!({ "events": rows }), Some("events")),
        items,
        error: None,
    })
}

fn get_item(conn: &Connection, ctx: &ToolCtx, kind: &str, id: &str) -> Result<ToolOutput> {
    match kind {
        "note" => get_note(conn, ctx, id),
        "thread" => get_thread(conn, ctx, id),
        "session" => get_session(conn, ctx, id),
        "message" => {
            let mut items = Vec::new();
            let pf = provider_filter(ctx, "m");
            let row = conn
                .query_row(
                    &format!(
                        "SELECT m.id, m.thread_id, m.sender_name, m.subject, COALESCE(m.body_text, m.snippet),
                                m.received_at, m.urgency_tier, m.urgency_why
                           FROM messages m WHERE m.id = ?1{pf}"
                    ),
                    [id],
                    |r| {
                        let mid: String = r.get(0)?;
                        let sender: String = r.get(2)?;
                        let subject: Option<String> = r.get(3)?;
                        let text: String = r.get(4)?;
                        let at: String = r.get(5)?;
                        items.push(SourceItem {
                            kind: "message".into(),
                            id: mid.clone(),
                            title: subject.clone().unwrap_or_else(|| format!("{sender}: {}", clip(&text, 60))),
                            snippet: clip(&text, 200),
                            route: route("message", &mid),
                            meta: Some(format!("{sender} · {}", local_time(conn, ctx, &at))),
                        });
                        Ok(json!({ "id": mid, "thread_id": r.get::<_, Option<String>>(1)?, "sender_name": sender,
                                   "subject": subject, "text": clip(&text, THREAD_MAX / 2), "received_at": at,
                                   "urgency_tier": r.get::<_, String>(6)?, "urgency_why": r.get::<_, Option<String>>(7)? }))
                    },
                )
                .optional()?;
            Ok(match row {
                Some(d) => ToolOutput {
                    data: d,
                    items,
                    error: None,
                },
                None => ToolOutput::error(format!("no message with id {id}")),
            })
        }
        "calendar_event" => {
            if !ctx.sources.calendar {
                return Ok(ToolOutput::error("Calendar is off for this conversation"));
            }
            let mut items = Vec::new();
            let row = conn
                .query_row(
                    &format!("SELECT {EVENT_COLUMNS}, e.description FROM calendar_events e WHERE e.id = ?1"),
                    [id],
                    |r| {
                        let mut v = event_json(conn, ctx, r, &mut items)?;
                        v["description"] = json!(r.get::<_, Option<String>>(9)?.map(|d| clip(&d, 2000)));
                        Ok(v)
                    },
                )
                .optional()?;
            Ok(match row {
                Some(d) => ToolOutput {
                    data: d,
                    items,
                    error: None,
                },
                None => ToolOutput::error(format!("no event with id {id}")),
            })
        }
        other => Ok(ToolOutput::error(format!("{other} is not an item kind"))),
    }
}

/// The context an attachment contributes (03 §7.2: bodies within retention),
/// or a `file://` link cited only (never read).
pub fn attachment(conn: &Connection, ctx: &ToolCtx, kind: &str, id: &str) -> Result<ToolOutput> {
    if kind == "file" {
        let name = std::path::Path::new(id)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.to_string());
        return Ok(ToolOutput {
            data: json!({ "file": id, "note": "a file link; its contents are not available" }),
            items: vec![SourceItem {
                kind: "file".into(),
                id: id.into(),
                title: name,
                snippet: String::new(),
                route: route("file", id),
                meta: None,
            }],
            error: None,
        });
    }
    get_item(conn, ctx, &super::citations::normalise_kind(kind), id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_offers_search_tools_only() {
        let names: Vec<&str> = specs(ToolSet::Search).iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                "now",
                "search_notes",
                "search_messages",
                "list_sessions",
                "calendar_lookup"
            ]
        );
        assert_eq!(specs(ToolSet::All).len(), 9);
        for t in specs(ToolSet::All) {
            assert_eq!(t.input_schema["additionalProperties"], false, "{}", t.name);
        }
    }

    #[test]
    fn tier_words_expand() {
        assert_eq!(tiers(vec!["urgent".into()]), ["vip", "direct", "ai-urgent"]);
        assert_eq!(tiers(vec!["low".into(), "LOW".into()]), ["low"]);
    }
}
