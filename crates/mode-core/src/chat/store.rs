//! `chat_turns` persistence (09 §5.5, 03 §7.1). The table has one row per
//! user message (`role = 'user'`, attachments in `citations_json`), one per
//! reply (`role = 'assistant'`, id = the turn id, citations in
//! `citations_json`), and `role = 'tool'` rows whose `content` is a tagged
//! JSON record: the turn's state and tokens (`turn`), each retrieval call
//! with the items it returned (`tool_call`, the Sources panel), each
//! proposed action card (`action`), the conversation title (`title`) and
//! pins (`pin`). Everything a reopened conversation shows is read back from
//! these rows; retention deletes them all after 90 days.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::actions::ActionView;
use super::citations::{Citation, SourceItem};
use super::tools::Sources;
use crate::connectors::ai::Usage;
use crate::db::Result;
use crate::repo::ChatTurn;

/// Tokens of a turn (`usage_samples` holds one row per step).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub total: i64,
}

impl TokenUsage {
    pub fn add(&mut self, u: &Usage) {
        self.input_tokens += u.input_tokens;
        self.output_tokens += u.output_tokens;
        self.cache_read_tokens += u.cache_read_tokens;
        self.cache_write_tokens += u.cache_write_tokens;
        self.total += u.total();
    }
}

/// The `turn` record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnMeta {
    pub turn_id: String,
    pub user_id: String,
    /// `auto` | `quick` | `thorough` | `local_only`.
    pub effort: String,
    /// `running` | `finished` | `failed`.
    pub state: String,
    /// Stop pressed: the partial text was kept.
    #[serde(default)]
    pub stopped: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub usage: TokenUsage,
    /// Configuration value (`ai.model_chat` or the local model).
    #[serde(default)]
    pub model: Option<String>,
    /// `claude_api` | `ollama` | `lm_studio`.
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub steps: u32,
    #[serde(default)]
    pub no_sources: bool,
    #[serde(default)]
    pub uncited: bool,
    #[serde(default)]
    pub dropped_citations: usize,
    #[serde(default)]
    pub sources: Sources,
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
}

/// The `tool_call` record (one Sources panel entry).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRow {
    pub turn_id: String,
    pub call_id: String,
    pub name: String,
    /// "Searched notes".
    pub label: String,
    pub input: Value,
    pub items: Vec<SourceItem>,
    pub count: usize,
    #[serde(default)]
    pub error: Option<String>,
    pub at: String,
}

/// A `tool` row's content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Turn(TurnMeta),
    ToolCall(ToolCallRow),
    Action(ActionView),
    Title {
        title: String,
    },
    Pin {
        turn_id: String,
        pinned: bool,
    },
    /// The conversation's effort and connector toggles (`chat_thread_set`,
    /// and every send).
    Prefs {
        #[serde(default)]
        effort: Option<String>,
        #[serde(default)]
        sources: Option<Sources>,
    },
}

pub fn meta_id(turn_id: &str) -> String {
    format!("{turn_id}:turn")
}

pub fn title_id(conversation_id: &str) -> String {
    format!("{conversation_id}:title")
}

pub fn pin_id(turn_id: &str) -> String {
    format!("{turn_id}:pin")
}

pub fn prefs_id(conversation_id: &str) -> String {
    format!("{conversation_id}:prefs")
}

/// Insert or replace a row by id.
pub fn put(conn: &Connection, row: &ChatTurn) -> Result<()> {
    conn.execute(
        "INSERT INTO chat_turns (id, conversation_id, role, content, citations_json, model, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET content = excluded.content,
             citations_json = excluded.citations_json, model = excluded.model",
        params![
            row.id,
            row.conversation_id,
            row.role,
            row.content,
            row.citations_json,
            row.model,
            row.created_at
        ],
    )?;
    Ok(())
}

pub fn put_record(
    conn: &Connection,
    id: &str,
    conversation_id: &str,
    rec: &Record,
    at: &str,
) -> Result<()> {
    put(
        conn,
        &ChatTurn {
            id: id.into(),
            conversation_id: conversation_id.into(),
            role: "tool".into(),
            content: serde_json::to_string(rec)?,
            citations_json: "[]".into(),
            model: None,
            created_at: at.into(),
        },
    )
}

pub fn record(conn: &Connection, id: &str) -> Result<Option<(String, Record)>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT conversation_id, content FROM chat_turns WHERE id = ?1 AND role = 'tool'",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(c, content)| serde_json::from_str(&content).ok().map(|r| (c, r))))
}

pub fn rows(conn: &Connection, conversation_id: &str) -> Result<Vec<ChatTurn>> {
    crate::repo::query::<ChatTurn>(
        conn,
        "conversation_id = ?1 ORDER BY created_at, id",
        &[&conversation_id],
    )
}

pub fn conversation_of(conn: &Connection, turn_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT conversation_id FROM chat_turns WHERE id = ?1 OR id = ?2 LIMIT 1",
            params![turn_id, meta_id(turn_id)],
            |r| r.get(0),
        )
        .optional()?)
}

/// An attachment as stored on the user row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRef {
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// The user's message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserView {
    pub id: String,
    pub text: String,
    pub attachments: Vec<ItemRef>,
    pub created_at: String,
}

/// The reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyView {
    pub id: String,
    pub text: String,
    pub citations: Vec<Citation>,
    pub model: Option<String>,
    pub created_at: String,
    pub pinned: bool,
}

/// One activity group row (03 §7.1 "Searched notes ›").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityRow {
    pub call_id: String,
    pub tool: String,
    pub label: String,
    pub count: usize,
    pub error: Option<String>,
    /// The call's inputs and what it returned (the group's rows).
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub items: Vec<SourceItem>,
}

/// One exchange of a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnView {
    pub turn_id: String,
    pub started_at: String,
    pub user: UserView,
    pub reply: Option<ReplyView>,
    /// `running` | `finished` | `failed`.
    pub state: String,
    pub stopped: bool,
    pub error: Option<String>,
    pub effort: String,
    pub usage: TokenUsage,
    /// "no sources — answered from the question alone" when nothing was retrieved.
    pub no_sources_label: Option<String>,
    pub uncited: bool,
    pub activity: Vec<ActivityRow>,
    pub actions: Vec<ActionView>,
}

/// A conversation as the thread shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadDetail {
    pub id: String,
    pub title: String,
    /// The first and the last row of the conversation.
    pub created_at: String,
    pub updated_at: String,
    /// The effort the next turn uses by default.
    pub effort: String,
    pub sources: Sources,
    pub turns: Vec<TurnView>,
}

/// A pinned reply in the conversations list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinnedReply {
    pub turn_id: String,
    pub excerpt: String,
}

/// A conversations-list row (`ChatThreadRow` in `contract-chat.ts`;
/// `created_at` is beyond the contract).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub created_at: String,
    pub updated_at: String,
    /// The user messages in the conversation.
    pub turn_count: i64,
    pub pinned: Vec<PinnedReply>,
}

fn first_line(s: &str, max: usize) -> String {
    let l = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    if l.chars().count() <= max {
        l.to_string()
    } else {
        let mut out: String = l.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

/// Assemble a conversation from its rows.
pub fn thread(conn: &Connection, conversation_id: &str) -> Result<Option<ThreadDetail>> {
    let all = rows(conn, conversation_id)?;
    if all.is_empty() {
        return Ok(None);
    }
    let mut users = std::collections::HashMap::new();
    let mut replies = std::collections::HashMap::new();
    let mut metas: Vec<TurnMeta> = Vec::new();
    let mut calls: Vec<ToolCallRow> = Vec::new();
    let mut actions: Vec<ActionView> = Vec::new();
    let mut pins = std::collections::HashMap::new();
    let mut title = None;
    let mut prefs: (Option<String>, Option<Sources>) = (None, None);
    for r in &all {
        match r.role.as_str() {
            "user" => {
                users.insert(r.id.clone(), r.clone());
            }
            "assistant" => {
                replies.insert(r.id.clone(), r.clone());
            }
            _ => match serde_json::from_str::<Record>(&r.content) {
                Ok(Record::Turn(m)) => metas.push(m),
                Ok(Record::ToolCall(c)) => calls.push(c),
                Ok(Record::Action(a)) => actions.push(a),
                Ok(Record::Title { title: t }) => title = Some(t),
                Ok(Record::Pin { turn_id, pinned }) => {
                    pins.insert(turn_id, pinned);
                }
                Ok(Record::Prefs { effort, sources }) => prefs = (effort, sources),
                Err(_) => {}
            },
        }
    }
    metas.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then(a.turn_id.cmp(&b.turn_id))
    });
    let mut turns = Vec::new();
    for m in &metas {
        let Some(u) = users.get(&m.user_id) else {
            continue;
        };
        let attachments: Vec<ItemRef> = serde_json::from_str(&u.citations_json).unwrap_or_default();
        let reply = replies.get(&m.turn_id).map(|r| ReplyView {
            id: r.id.clone(),
            text: r.content.clone(),
            citations: serde_json::from_str(&r.citations_json).unwrap_or_default(),
            model: r.model.clone(),
            created_at: r.created_at.clone(),
            pinned: pins.get(&m.turn_id).copied().unwrap_or(false),
        });
        turns.push(TurnView {
            turn_id: m.turn_id.clone(),
            started_at: m.started_at.clone(),
            user: UserView {
                id: u.id.clone(),
                text: u.content.clone(),
                attachments,
                created_at: u.created_at.clone(),
            },
            reply,
            state: m.state.clone(),
            stopped: m.stopped,
            error: m.error.clone(),
            effort: m.effort.clone(),
            usage: m.usage,
            no_sources_label: (m.no_sources && m.state != "running")
                .then(|| super::citations::NO_SOURCES.to_string()),
            uncited: m.uncited,
            activity: calls
                .iter()
                .filter(|c| c.turn_id == m.turn_id)
                .map(|c| ActivityRow {
                    call_id: c.call_id.clone(),
                    tool: c.name.clone(),
                    label: c.label.clone(),
                    count: c.count,
                    error: c.error.clone(),
                    input: c.input.clone(),
                    items: c.items.clone(),
                })
                .collect(),
            actions: actions
                .iter()
                .filter(|a| a.turn_id == m.turn_id)
                .cloned()
                .collect(),
        });
    }
    let title = title.unwrap_or_else(|| {
        turns
            .first()
            .map(|t| first_line(&t.user.text, 60))
            .unwrap_or_else(|| "New conversation".into())
    });
    let last = metas.last();
    let created_at = all
        .iter()
        .map(|r| r.created_at.as_str())
        .min()
        .unwrap_or_default()
        .to_string();
    let updated_at = all
        .iter()
        .map(|r| r.created_at.as_str())
        .max()
        .unwrap_or_default()
        .to_string();
    Ok(Some(ThreadDetail {
        id: conversation_id.into(),
        title,
        created_at,
        updated_at,
        effort: prefs
            .0
            .or_else(|| last.map(|m| m.effort.clone()))
            .unwrap_or_else(|| "auto".into()),
        sources: prefs
            .1
            .or_else(|| last.map(|m| m.sources))
            .unwrap_or_default(),
        turns,
    }))
}

/// The conversations list (03 §7.1), newest first.
pub fn threads(conn: &Connection) -> Result<Vec<ThreadSummary>> {
    let base: Vec<(String, String, String, i64)> = conn
        .prepare(
            "SELECT conversation_id, MIN(created_at), MAX(created_at), SUM(role = 'user')
               FROM chat_turns GROUP BY conversation_id ORDER BY MAX(created_at) DESC",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::with_capacity(base.len());
    for (id, created, updated, turns) in base {
        if turns == 0 {
            continue;
        }
        let title: Option<String> = conn
            .query_row(
                "SELECT content FROM chat_turns WHERE id = ?1",
                [title_id(&id)],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|c| match serde_json::from_str::<Record>(&c) {
                Ok(Record::Title { title }) => Some(title),
                _ => None,
            });
        let title = match title {
            Some(t) => t,
            None => conn
                .query_row(
                    "SELECT content FROM chat_turns WHERE conversation_id = ?1 AND role = 'user'
                      ORDER BY created_at, id LIMIT 1",
                    [&id],
                    |r| r.get::<_, String>(0),
                )
                .map(|c| first_line(&c, 60))
                .unwrap_or_else(|_| "New conversation".into()),
        };
        let mut pinned = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT content FROM chat_turns WHERE conversation_id = ?1 AND role = 'tool' AND id LIKE '%:pin'",
        )?;
        for c in stmt.query_map([&id], |r| r.get::<_, String>(0))? {
            if let Ok(Record::Pin {
                turn_id,
                pinned: true,
            }) = serde_json::from_str(&c?)
            {
                let excerpt: Option<String> = conn
                    .query_row(
                        "SELECT content FROM chat_turns WHERE id = ?1 AND role = 'assistant'",
                        [&turn_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(e) = excerpt {
                    pinned.push(PinnedReply {
                        turn_id,
                        excerpt: first_line(&e, 80),
                    });
                }
            }
        }
        out.push(ThreadSummary {
            id,
            title,
            created_at: created,
            updated_at: updated,
            turn_count: turns,
            pinned,
        });
    }
    Ok(out)
}

/// The Sources panel of a turn (03 §7.3): every tool call with its inputs
/// and items, the attachments, the turn's tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnSources {
    pub turn_id: String,
    pub thread_id: String,
    pub entries: Vec<ToolCallRow>,
    pub attachments: Vec<ItemRef>,
    pub usage: TokenUsage,
    /// Every distinct item retrieved, in call order (the icon rail count).
    pub items: Vec<SourceItem>,
}

pub fn sources(conn: &Connection, turn_id: &str) -> Result<Option<TurnSources>> {
    let Some((conv, Record::Turn(meta))) = record(conn, &meta_id(turn_id))? else {
        return Ok(None);
    };
    let attachments: Vec<ItemRef> = conn
        .query_row(
            "SELECT citations_json FROM chat_turns WHERE id = ?1",
            [&meta.user_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default();
    let mut entries = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT content FROM chat_turns WHERE conversation_id = ?1 AND role = 'tool' AND id LIKE ?2
          ORDER BY created_at, id",
    )?;
    for c in stmt.query_map(params![conv, format!("{turn_id}:call:%")], |r| {
        r.get::<_, String>(0)
    })? {
        if let Ok(Record::ToolCall(t)) = serde_json::from_str(&c?) {
            entries.push(t);
        }
    }
    entries.sort_by(|a, b| a.at.cmp(&b.at).then(a.call_id.cmp(&b.call_id)));
    let mut items: Vec<SourceItem> = Vec::new();
    for e in &entries {
        for it in &e.items {
            if !items.iter().any(|x| x.kind == it.kind && x.id == it.id) {
                items.push(it.clone());
            }
        }
    }
    Ok(Some(TurnSources {
        turn_id: turn_id.into(),
        thread_id: conv,
        entries,
        attachments,
        usage: meta.usage,
        items,
    }))
}
