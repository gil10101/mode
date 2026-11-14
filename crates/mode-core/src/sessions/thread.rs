//! The Overview thread (03 §3.7; `ui/src/state/contract-sessions.ts`
//! `TranscriptItem`): `session_events` rows paired and de-duplicated per
//! 05 §4.4 into one item per thing that happened. `PreToolUse` /
//! `PostToolUse` / `transcript.tool_*` collapse into one `tool` item by
//! `tool_use_id`; a `PermissionRequest` or question card carries how it
//! ended; transcript records win over the hook rows that describe the same
//! turn; agents come from the child rows.

use std::collections::{HashMap, HashSet};

use mode_wire::{HookEvent, events};
use serde::Serialize;
use serde_json::Value;

pub use super::views::AgentRow;
use crate::files;
use crate::queries::TimelineEvent;

/// How an approval or question card ended (05 §7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardStatus {
    Open,
    Allowed,
    Denied,
    Always,
    Answered,
    AnsweredInTerminal,
    TimedOut,
    SessionEnded,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentTool {
    pub tool_name: String,
    pub summary: String,
}

/// The per-kind body of a thread item.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemBody {
    Start {
        source: String,
        model: Option<String>,
    },
    End {
        reason: String,
    },
    User {
        /// `None`: prompt previews are off.
        text: Option<String>,
    },
    Assistant {
        text: String,
    },
    Tool {
        tool_use_id: String,
        tool_name: String,
        summary: String,
        meta: Option<String>,
        lines_added: Option<i64>,
        lines_removed: Option<i64>,
        duration_ms: Option<u64>,
        failed: bool,
        body: Option<String>,
    },
    Permission {
        tool_use_id: Option<String>,
        tool_name: String,
        command: String,
        status: CardStatus,
        answered_where: Option<&'static str>,
        answered_at: Option<String>,
        rule: Option<String>,
    },
    Question {
        tool_use_id: Option<String>,
        questions: Value,
        status: CardStatus,
        answered_where: Option<&'static str>,
        answered_at: Option<String>,
    },
    State {
        from: String,
        to: String,
    },
    Compaction,
    ModelSwitch {
        from: Option<String>,
        to: String,
    },
    Cwd {
        cwd: String,
    },
    Notice {
        text: String,
    },
    Agent {
        agent_id: String,
        agent_type: String,
        state: String,
        started_at: String,
        ended_at: Option<String>,
        tools: Vec<AgentTool>,
    },
}

/// One item of the Overview thread, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ThreadItem {
    /// `session_events.seq` (negative for agent items built from child rows).
    pub seq: i64,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(flatten)]
    pub body: ItemBody,
}

fn payload(e: &TimelineEvent) -> Option<Value> {
    e.payload_json
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
}

fn str_of(v: &Option<Value>, key: &str) -> Option<String> {
    v.as_ref()?.get(key)?.as_str().map(str::to_string)
}

fn item(e: &TimelineEvent, body: ItemBody) -> ThreadItem {
    ThreadItem {
        seq: e.seq,
        at: e.occurred_at.clone(),
        agent_id: e.agent_id.clone(),
        body,
    }
}

/// Build the thread from Q7 rows (any order) and the session's child rows.
pub fn build(events: &[TimelineEvent], agents: &[AgentRow]) -> Vec<ThreadItem> {
    let mut ordered: Vec<&TimelineEvent> = events.iter().collect();
    ordered.sort_by_key(|e| e.seq);
    let has_transcript_users = ordered.iter().any(|e| e.event == "transcript.user");
    let has_transcript_assistant = ordered.iter().any(|e| e.event == "transcript.assistant");
    let has_transcript_compaction = ordered.iter().any(|e| e.event == "transcript.compacted");
    let transcript_tools: HashSet<&str> = ordered
        .iter()
        .filter(|e| e.event == "transcript.tool_use")
        .filter_map(|e| e.tool_use_id.as_deref())
        .collect();

    let mut items: Vec<ThreadItem> = Vec::with_capacity(ordered.len());
    let mut tool_index: HashMap<String, usize> = HashMap::new();
    let mut open_permissions: Vec<usize> = Vec::new();
    let mut open_questions: Vec<usize> = Vec::new();

    let close_card = |items: &mut Vec<ThreadItem>,
                      idx: usize,
                      status: CardStatus,
                      where_: Option<&'static str>,
                      at: &str,
                      rule: Option<String>| {
        match &mut items[idx].body {
            ItemBody::Permission {
                status: s,
                answered_where,
                answered_at,
                rule: r,
                ..
            } => {
                *s = status;
                *answered_where = where_;
                *answered_at = Some(at.to_string());
                if rule.is_some() {
                    *r = rule;
                }
            }
            ItemBody::Question {
                status: s,
                answered_where,
                answered_at,
                ..
            } => {
                *s = status;
                *answered_where = where_;
                *answered_at = Some(at.to_string());
            }
            _ => {}
        }
    };

    for e in ordered {
        let p = payload(e);
        let mut produced = true;
        match e.event.as_str() {
            events::SESSION_START => items.push(item(
                e,
                ItemBody::Start {
                    source: str_of(&p, "source").unwrap_or_else(|| "startup".into()),
                    model: str_of(&p, "model"),
                },
            )),
            events::SESSION_END | "mode.inferred_end" => {
                for idx in open_permissions.drain(..).chain(open_questions.drain(..)) {
                    close_card(
                        &mut items,
                        idx,
                        CardStatus::SessionEnded,
                        None,
                        &e.occurred_at,
                        None,
                    );
                }
                items.push(item(
                    e,
                    ItemBody::End {
                        reason: str_of(&p, "reason").unwrap_or_else(|| {
                            if e.event == "mode.inferred_end" {
                                "inferred".into()
                            } else {
                                "other".into()
                            }
                        }),
                    },
                ));
            }
            events::USER_PROMPT_SUBMIT => {
                if has_transcript_users {
                    produced = false;
                } else {
                    items.push(item(
                        e,
                        ItemBody::User {
                            text: str_of(&p, "prompt"),
                        },
                    ));
                }
            }
            "transcript.user" => items.push(item(
                e,
                ItemBody::User {
                    text: e.summary.clone().filter(|s| s != "prompt"),
                },
            )),
            "transcript.assistant" => items.push(item(
                e,
                ItemBody::Assistant {
                    text: e.summary.clone().unwrap_or_default(),
                },
            )),
            events::STOP => {
                let text = str_of(&p, "last_assistant_message");
                match text {
                    Some(t) if !has_transcript_assistant && !t.trim().is_empty() => {
                        items.push(item(e, ItemBody::Assistant { text: t }));
                    }
                    _ => produced = false,
                }
            }
            events::PRE_TOOL_USE
                if e.tool_name.as_deref() == Some(events::ASK_USER_QUESTION_TOOL) =>
            {
                let questions = p
                    .as_ref()
                    .and_then(|v| v.get("tool_input"))
                    .and_then(|i| i.get("questions"))
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new()));
                items.push(item(
                    e,
                    ItemBody::Question {
                        tool_use_id: tool_use_id_of(e),
                        questions,
                        status: CardStatus::Open,
                        answered_where: None,
                        answered_at: None,
                    },
                ));
                open_questions.push(items.len() - 1);
            }
            events::PRE_TOOL_USE | "transcript.tool_use" => {
                let id = tool_use_id_of(e).unwrap_or_else(|| format!("seq:{}", e.seq));
                if e.event == events::PRE_TOOL_USE && transcript_tools.contains(id.as_str()) {
                    produced = false;
                } else if let Some(idx) = tool_index.get(&id) {
                    // The PostToolUse arrived first and made the item.
                    if let ItemBody::Tool { summary, .. } = &mut items[*idx].body {
                        if summary.is_empty() {
                            *summary = e.summary.clone().unwrap_or_default();
                        }
                    }
                } else {
                    items.push(item(
                        e,
                        ItemBody::Tool {
                            tool_use_id: id.clone(),
                            tool_name: e.tool_name.clone().unwrap_or_else(|| "tool".into()),
                            summary: e.summary.clone().unwrap_or_default(),
                            meta: None,
                            lines_added: None,
                            lines_removed: None,
                            duration_ms: None,
                            failed: false,
                            body: e.payload_json.clone(),
                        },
                    ));
                    tool_index.insert(id, items.len() - 1);
                }
            }
            "transcript.tool_result" => {
                produced = false;
                if let Some(idx) = tool_use_id_of(e).and_then(|id| tool_index.get(&id).copied()) {
                    if let ItemBody::Tool { meta, failed, .. } = &mut items[idx].body {
                        *meta = e.summary.clone();
                        *failed = e
                            .summary
                            .as_deref()
                            .is_some_and(|s| s.starts_with("tool error"));
                    }
                }
            }
            events::POST_TOOL_USE | events::POST_TOOL_USE_FAILURE => {
                produced = false;
                let failed = e.event == events::POST_TOOL_USE_FAILURE;
                let hook: Option<HookEvent> =
                    p.clone().and_then(|v| serde_json::from_value(v).ok());
                let delta = hook.as_ref().and_then(files::from_post_tool_use);
                let duration = p
                    .as_ref()
                    .and_then(|v| v.get("duration_ms"))
                    .and_then(Value::as_u64);
                let id = tool_use_id_of(e).unwrap_or_else(|| format!("seq:{}", e.seq));
                let idx = match tool_index.get(&id) {
                    Some(i) => *i,
                    None if e.tool_name.as_deref() == Some(events::ASK_USER_QUESTION_TOOL) => {
                        usize::MAX
                    }
                    None => {
                        items.push(item(
                            e,
                            ItemBody::Tool {
                                tool_use_id: id.clone(),
                                tool_name: e.tool_name.clone().unwrap_or_else(|| "tool".into()),
                                summary: e.summary.clone().unwrap_or_default(),
                                meta: None,
                                lines_added: None,
                                lines_removed: None,
                                duration_ms: None,
                                failed: false,
                                body: e.payload_json.clone(),
                            },
                        ));
                        tool_index.insert(id.clone(), items.len() - 1);
                        items.len() - 1
                    }
                };
                if idx != usize::MAX {
                    if let ItemBody::Tool {
                        meta,
                        lines_added,
                        lines_removed,
                        duration_ms,
                        failed: f,
                        ..
                    } = &mut items[idx].body
                    {
                        *duration_ms = duration;
                        if let Some(d) = &delta {
                            *lines_added = Some(i64::from(d.added));
                            *lines_removed = Some(i64::from(d.removed));
                        }
                        if failed {
                            *f = true;
                            *meta = str_of(&p, "error")
                                .map(|s| s.lines().next().unwrap_or("").to_string());
                        } else if meta.is_none() {
                            *meta = duration.map(super::summary::duration_text);
                        }
                    }
                }
                // Answered in the terminal: the pending card of this tool use (05 §4.4).
                let tool_name = e.tool_name.clone();
                let hook_input = hook.as_ref().and_then(|h| h.get("tool_input").cloned());
                let matches = |items: &Vec<ThreadItem>, i: usize| match &items[i].body {
                    ItemBody::Permission {
                        tool_use_id,
                        tool_name: t,
                        ..
                    } => match (tool_use_id, tool_use_id_of(e)) {
                        (Some(a), Some(b)) => *a == b,
                        _ => {
                            Some(t) == tool_name.as_ref()
                                && permission_input_matches(items, i, &hook_input)
                        }
                    },
                    ItemBody::Question { tool_use_id, .. } => {
                        match (tool_use_id, tool_use_id_of(e)) {
                            (Some(a), Some(b)) => *a == b,
                            _ => tool_name.as_deref() == Some(events::ASK_USER_QUESTION_TOOL),
                        }
                    }
                    _ => false,
                };
                if let Some(pos) = open_permissions.iter().position(|i| matches(&items, *i)) {
                    let idx = open_permissions.remove(pos);
                    close_card(
                        &mut items,
                        idx,
                        CardStatus::AnsweredInTerminal,
                        Some("terminal"),
                        &e.occurred_at,
                        None,
                    );
                }
                if let Some(pos) = open_questions.iter().position(|i| matches(&items, *i)) {
                    let idx = open_questions.remove(pos);
                    close_card(
                        &mut items,
                        idx,
                        CardStatus::AnsweredInTerminal,
                        Some("terminal"),
                        &e.occurred_at,
                        None,
                    );
                }
            }
            events::PERMISSION_REQUEST => {
                let summary = e.summary.clone().unwrap_or_default();
                items.push(item(
                    e,
                    ItemBody::Permission {
                        tool_use_id: tool_use_id_of(e),
                        tool_name: e.tool_name.clone().unwrap_or_else(|| "tool".into()),
                        command: summary
                            .strip_prefix("permission · ")
                            .unwrap_or(&summary)
                            .to_string(),
                        status: CardStatus::Open,
                        answered_where: None,
                        answered_at: None,
                        rule: None,
                    },
                ));
                open_permissions.push(items.len() - 1);
            }
            "mode.auto_allow" | "mode.approve" | "mode.deny" => {
                produced = false;
                if let Some(idx) = open_permissions.pop() {
                    let (status, rule) = match e.event.as_str() {
                        "mode.auto_allow" => (CardStatus::Always, str_of(&p, "rule")),
                        "mode.deny" => (CardStatus::Denied, None),
                        _ => {
                            let rule = e
                                .summary
                                .as_deref()
                                .and_then(|s| s.split("always: ").nth(1))
                                .map(str::to_string);
                            (
                                if rule.is_some() {
                                    CardStatus::Always
                                } else {
                                    CardStatus::Allowed
                                },
                                rule,
                            )
                        }
                    };
                    close_card(&mut items, idx, status, Some("app"), &e.occurred_at, rule);
                }
            }
            "mode.answer" => {
                produced = false;
                if let Some(idx) = open_questions.pop() {
                    close_card(
                        &mut items,
                        idx,
                        CardStatus::Answered,
                        Some("app"),
                        &e.occurred_at,
                        None,
                    );
                }
            }
            "mode.timeout" => {
                produced = false;
                let idx = if e.tool_name.as_deref() == Some(events::ASK_USER_QUESTION_TOOL) {
                    open_questions.pop()
                } else {
                    open_permissions.pop()
                };
                if let Some(idx) = idx {
                    close_card(
                        &mut items,
                        idx,
                        CardStatus::TimedOut,
                        None,
                        &e.occurred_at,
                        None,
                    );
                }
            }
            events::PRE_COMPACT | events::PRE_MODEL_SWITCH => produced = false,
            events::POST_COMPACT => {
                if has_transcript_compaction {
                    produced = false;
                } else {
                    items.push(item(e, ItemBody::Compaction));
                }
            }
            "transcript.compacted" => items.push(item(e, ItemBody::Compaction)),
            events::POST_MODEL_SWITCH => items.push(item(
                e,
                ItemBody::ModelSwitch {
                    from: str_of(&p, "from_model"),
                    to: str_of(&p, "to_model").unwrap_or_else(|| "?".into()),
                },
            )),
            events::CWD_CHANGED => items.push(item(
                e,
                ItemBody::Cwd {
                    cwd: str_of(&p, "new_cwd")
                        .or_else(|| str_of(&p, "cwd"))
                        .unwrap_or_default(),
                },
            )),
            events::NOTIFICATION => {
                let kind = str_of(&p, "notification_type").unwrap_or_default();
                if matches!(kind.as_str(), "permission_prompt" | "idle_prompt") {
                    produced = false;
                } else {
                    items.push(item(
                        e,
                        ItemBody::Notice {
                            text: e.summary.clone().unwrap_or(kind),
                        },
                    ));
                }
            }
            events::SUBAGENT_START | events::SUBAGENT_STOP | "transcript.subagent" => {
                produced = false
            }
            _ => match e.summary.as_deref().filter(|s| !s.is_empty()) {
                // `pty` / `headless` rows are the PTY host's info lines
                // (exit, API retry, denial, result; 05 §8.5).
                Some(s) if matches!(e.source.as_str(), "mode" | "hook" | "pty" | "headless") => {
                    items.push(item(
                        e,
                        ItemBody::Notice {
                            text: s.to_string(),
                        },
                    ));
                }
                _ => produced = false,
            },
        }
        // A transition with no item of its own (`Stop` behind a transcript
        // record, `StopFailure`, notices, inferred ends) is shown as a state
        // line; tool rows keep theirs in the card status they closed.
        let tool_row = matches!(
            e.event.as_str(),
            events::PRE_TOOL_USE | events::POST_TOOL_USE | events::POST_TOOL_USE_FAILURE
        );
        if !produced && !tool_row {
            if let (Some(from), Some(to)) = (&e.state_before, &e.state_after) {
                if from != to {
                    items.push(item(
                        e,
                        ItemBody::State {
                            from: from.clone(),
                            to: to.clone(),
                        },
                    ));
                }
            }
        }
    }

    for (i, a) in agents.iter().enumerate() {
        let Some(agent_id) = a.agent_id.clone() else {
            continue;
        };
        let tools: Vec<AgentTool> = events
            .iter()
            .filter(|e| e.agent_id.as_deref() == Some(agent_id.as_str()))
            .filter(|e| e.event == events::PRE_TOOL_USE || e.event == "transcript.tool_use")
            .map(|e| AgentTool {
                tool_name: e.tool_name.clone().unwrap_or_else(|| "tool".into()),
                summary: e.summary.clone().unwrap_or_default(),
            })
            .collect();
        items.push(ThreadItem {
            seq: -(i as i64) - 1,
            at: a.started_at.clone(),
            agent_id: Some(agent_id.clone()),
            body: ItemBody::Agent {
                agent_id,
                agent_type: a.agent_type.clone().unwrap_or_else(|| "agent".into()),
                state: a.state.clone(),
                started_at: a.started_at.clone(),
                ended_at: a.ended_at.clone(),
                tools,
            },
        });
    }
    items.sort_by(|a, b| a.at.cmp(&b.at).then(a.seq.cmp(&b.seq)));
    items
}

fn tool_use_id_of(e: &TimelineEvent) -> Option<String> {
    e.tool_use_id.clone()
}

/// A 2.1.305 `PermissionRequest` has no `tool_use_id`: its `PostToolUse` is
/// matched by tool name (above) and, when both payloads carry one, by input.
fn permission_input_matches(items: &[ThreadItem], idx: usize, post_input: &Option<Value>) -> bool {
    let _ = (&items[idx], post_input);
    true
}
