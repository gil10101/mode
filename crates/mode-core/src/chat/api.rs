//! One plain function per Chat command (03 §7; errors `{code, message}`),
//! shapes per `ui/src/state/contract-chat.ts` ([`super::edge`]):
//!
//! | Command | Args | Result |
//! |---|---|---|
//! | `chat_threads_list` | — | `ChatThreadRow[]` newest first |
//! | `chat_thread_get` | `{id}` | `ChatThread {id, title, created_at, updated_at, effort, connectors, turns[]}` (flat turns) |
//! | `chat_send` | `{thread_id?, text, effort?, attachments?: [{kind, id}], connectors?: {gmail, slack, calendar}}` | `{turn_id, thread_id, user_turn_id}` |
//! | `chat_stop` | `{turn_id}` | `{stopped}` |
//! | `chat_action_confirm` | `{turn_id, action_id, confirm}` | `ChatAction` |
//! | `chat_thread_delete` | `{id}` | `{deleted}` |
//! | `chat_sources` | `{turn_id}` | `ChatSources {turn_id, calls[], tokens}` |
//! | `chat_status` | `{effort?}` | `ChatStatus` (the meta line and input state) |
//! | `chat_thread_rename` | `{id, title}` | — |
//! | `chat_pin` | `{turn_id, pinned}` | — |
//! | `chat_thread_set` | `{id, effort?, connectors?}` | — |
//!
//! `chat_pick_file` (the OS picker) is the app's. Streams: `chat.delta`,
//! `chat.turn`, `chat.action` (see [`super::turn`]).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Chat;
use super::citations::KINDS;
use super::edge::{self, AccountsPresent, ChatAction, ChatSources, ChatThread};
use super::store::{self, ItemRef, Record, ThreadSummary};
use super::tools::Sources;
use super::turn::{self, EffortChoice, TurnRequest};
use crate::connectors::ai::{AddressClass, AiError, AiSettings, BackendKind};
use crate::db::format_timestamp;
use crate::push::Event;

pub use crate::notes::api::{ApiError, ApiResult};

/// Longest question.
pub const TEXT_MAX: usize = 32_000;
/// Most attachments per message.
pub const ATTACHMENTS_MAX: usize = 10;

fn err(code: &str, message: impl Into<String>) -> ApiError {
    ApiError {
        code: code.into(),
        message: message.into(),
    }
}

/// `chat_threads_list`.
pub fn chat_threads_list(chat: &Chat) -> ApiResult<Vec<ThreadSummary>> {
    Ok(chat.db.read(store::threads)?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadIdArgs {
    pub id: String,
}

/// `chat_thread_get {id}`: a reopened conversation keeps its citations,
/// activity, cards and tokens; turns alternate user / reply.
pub fn chat_thread_get(chat: &Chat, args: ThreadIdArgs) -> ApiResult<ChatThread> {
    let id = args.id.clone();
    chat.db
        .read(move |c| store::thread(c, &id))?
        .map(|d| ChatThread::from_detail(&d))
        .ok_or_else(|| err("not_found", format!("No conversation {}.", args.id)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentArg {
    /// `note` | `message` | `session` | `event` | `file` (the contract's
    /// kinds; the stored `thread` / `calendar_event` are accepted too).
    pub kind: String,
    /// A row id (a message or a thread id for `message`), or a file path
    /// for `file` (`file://` allowed; cited, never uploaded).
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendArgs {
    /// `None` starts a new conversation.
    #[serde(default)]
    pub thread_id: Option<String>,
    pub text: String,
    /// `auto` | `quick` | `thorough` | `local_only`; default the conversation's.
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub attachments: Option<Vec<AttachmentArg>>,
    /// The `+` menu's Connectors toggles for this conversation.
    #[serde(default)]
    pub connectors: Option<Sources>,
}

/// `turn_id` is the reply that streams; `user_turn_id` the stored question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendResult {
    pub turn_id: String,
    pub thread_id: String,
    #[serde(default)]
    pub user_turn_id: String,
}

/// `chat_send`: store the question, start the turn, return at once; the
/// reply arrives as `chat.delta` / `chat.turn`.
pub fn chat_send(chat: &Chat, args: SendArgs) -> ApiResult<SendResult> {
    let text = args.text.trim().to_string();
    if text.is_empty() {
        return Err(err("invalid", "Ask something first."));
    }
    if text.chars().count() > TEXT_MAX {
        return Err(err("invalid", "That message is too long."));
    }
    let raw = args.attachments.clone().unwrap_or_default();
    if raw.len() > ATTACHMENTS_MAX {
        return Err(err(
            "invalid",
            format!("Attach at most {ATTACHMENTS_MAX} items."),
        ));
    }
    let atts: Vec<AttachmentArg> = chat.db.read(move |c| {
        raw.iter()
            .map(|a| {
                edge::attachment_kind(c, &a.kind, &a.id)
                    .map(|(kind, id)| AttachmentArg { kind, id })
            })
            .collect()
    })?;
    if let Some(a) = atts.iter().find(|a| !KINDS.contains(&a.kind.as_str())) {
        return Err(err("invalid", format!("{:?} cannot be attached.", a.kind)));
    }
    let existing = match &args.thread_id {
        Some(id) => {
            let id2 = id.clone();
            let t = chat.db.read(move |c| store::thread(c, &id2))?;
            if t.is_none() {
                return Err(err("not_found", format!("No conversation {id}.")));
            }
            t
        }
        None => None,
    };
    let thread_id = args.thread_id.clone().unwrap_or_else(crate::repo::new_id);
    if chat.running_in(&thread_id).is_some() {
        return Err(err(
            "busy",
            "A reply is still running in this conversation.",
        ));
    }
    let effort = match args.effort.as_deref() {
        Some(e) => EffortChoice::parse(e).ok_or_else(|| {
            err(
                "invalid",
                format!("{e:?} is not Auto, Quick, Thorough or Local-only"),
            )
        })?,
        None => existing
            .as_ref()
            .and_then(|t| EffortChoice::parse(&t.effort))
            .unwrap_or(EffortChoice::Auto),
    };
    let sources = args
        .connectors
        .or_else(|| existing.as_ref().map(|t| t.sources))
        .unwrap_or_default();
    // Titles of the attachments for the chips.
    let refs: Vec<ItemRef> = {
        let atts = atts.clone();
        chat.db.read(move |c| {
            Ok(atts
                .iter()
                .map(|a| {
                    let title: Option<String> = match a.kind.as_str() {
                        "note" => c
                            .query_row("SELECT title FROM notes WHERE id = ?1", [&a.id], |r| {
                                r.get(0)
                            })
                            .ok(),
                        "thread" => c
                            .query_row("SELECT title FROM threads WHERE id = ?1", [&a.id], |r| {
                                r.get(0)
                            })
                            .ok(),
                        "message" => c
                            .query_row(
                                "SELECT COALESCE(subject, sender_name) FROM messages WHERE id = ?1",
                                [&a.id],
                                |r| r.get(0),
                            )
                            .ok(),
                        "session" => c
                            .query_row(
                                "SELECT display_name FROM sessions WHERE id = ?1",
                                [&a.id],
                                |r| r.get(0),
                            )
                            .ok(),
                        "calendar_event" => c
                            .query_row(
                                "SELECT title FROM calendar_events WHERE id = ?1",
                                [&a.id],
                                |r| r.get(0),
                            )
                            .ok(),
                        _ => std::path::Path::new(&a.id)
                            .file_name()
                            .map(|f| f.to_string_lossy().into_owned()),
                    };
                    ItemRef {
                        kind: a.kind.clone(),
                        id: a.id.clone(),
                        title,
                    }
                })
                .collect())
        })?
    };
    let req = TurnRequest {
        question: text,
        effort,
        attachments: refs,
        sources,
        allow_actions: true,
    };
    let h = turn::begin(chat, &thread_id, &req)?;
    let (turn_id, user_turn_id) = (h.turn_id.clone(), h.user_id.clone());
    write_prefs(
        chat,
        &thread_id,
        Some(effort.as_str().into()),
        Some(sources),
    )?;
    chat.start(h, req)
        .map_err(|e| err("internal", e.to_string()))?;
    Ok(SendResult {
        turn_id,
        thread_id,
        user_turn_id,
    })
}

/// The conversation's effort / connectors (merged with what is stored).
fn write_prefs(
    chat: &Chat,
    thread_id: &str,
    effort: Option<String>,
    sources: Option<Sources>,
) -> ApiResult<()> {
    let now = format_timestamp(chat.now());
    let conv = thread_id.to_string();
    chat.db.write(move |c| {
        let id = store::prefs_id(&conv);
        let (old_e, old_s) = match store::record(c, &id)? {
            Some((_, Record::Prefs { effort, sources })) => (effort, sources),
            _ => (None, None),
        };
        store::put_record(
            c,
            &id,
            &conv,
            &Record::Prefs {
                effort: effort.or(old_e),
                sources: sources.or(old_s),
            },
            &now,
        )
    })?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnIdArgs {
    pub turn_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopResult {
    /// `false` when the turn had already ended.
    pub stopped: bool,
}

/// `chat_stop {turn_id}`: aborts and keeps the partial text (03 §7.6).
pub fn chat_stop(chat: &Chat, args: TurnIdArgs) -> ApiResult<StopResult> {
    let Some(h) = chat.handle(&args.turn_id) else {
        return Ok(StopResult { stopped: false });
    };
    h.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(StopResult {
        stopped: chat.finalize(&h, turn::Final::Stopped),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfirmArgs {
    pub turn_id: String,
    pub action_id: String,
    /// `true` = Confirm, `false` = Reject.
    pub confirm: bool,
}

/// `chat_action_confirm`: the only way an action runs (03 §7.5).
pub fn chat_action_confirm(chat: &Chat, args: ConfirmArgs) -> ApiResult<ChatAction> {
    let id = args.action_id.clone();
    let found = chat.db.read(move |c| store::record(c, &id))?;
    let Some((conv, Record::Action(mut view))) = found else {
        return Err(err("not_found", "No such proposal."));
    };
    if view.turn_id != args.turn_id {
        return Err(err("invalid", "That proposal belongs to another turn."));
    }
    if view.state != "proposed" {
        return Err(err(
            "invalid",
            format!("This proposal was already {}.", view.state),
        ));
    }
    view.decided_at = Some(format_timestamp(chat.now()));
    if args.confirm {
        match chat.effects().apply(&view.kind, &view.params) {
            Ok(applied) => {
                view.state = "confirmed".into();
                view.result_line = Some(applied.line);
                view.route = applied.route;
            }
            Err(e) => {
                view.state = "failed".into();
                view.error = Some(e);
            }
        }
    } else {
        view.state = "rejected".into();
    }
    let v2 = view.clone();
    chat.db.write(move |c| {
        let (id, at) = (v2.action_id.clone(), v2.created_at.clone());
        store::put_record(c, &id, &conv, &Record::Action(v2), &at)
    })?;
    let ev = edge::ChatActionEvent::of(&view);
    chat.emit(
        Event::ChatAction,
        serde_json::to_value(&ev).unwrap_or(Value::Null),
    );
    Ok(ev.action)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResult {
    /// Rows removed.
    pub deleted: usize,
}

/// `chat_thread_delete {id}` (stops a running turn first).
pub fn chat_thread_delete(chat: &Chat, args: ThreadIdArgs) -> ApiResult<DeleteResult> {
    if let Some(h) = chat.running_in(&args.id) {
        h.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        chat.finalize(&h, turn::Final::Stopped);
    }
    let id = args.id.clone();
    let deleted = chat.db.write(move |c| {
        Ok(c.execute("DELETE FROM chat_turns WHERE conversation_id = ?1", [&id])?)
    })?;
    if deleted == 0 {
        return Err(err("not_found", format!("No conversation {}.", args.id)));
    }
    Ok(DeleteResult { deleted })
}

/// `chat_sources {turn_id}`: the Sources panel (03 §7.3).
pub fn chat_sources(chat: &Chat, args: TurnIdArgs) -> ApiResult<ChatSources> {
    let id = args.turn_id.clone();
    chat.db
        .read(move |c| store::sources(c, &id))?
        .map(|s| ChatSources::from_stored(&s))
        .ok_or_else(|| err("not_found", format!("No turn {}.", args.turn_id)))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameArgs {
    pub id: String,
    pub title: String,
}

/// `chat_thread_rename {id, title}` (the list's `⋮` → Rename).
pub fn chat_thread_rename(chat: &Chat, args: RenameArgs) -> ApiResult<()> {
    let title = args.title.trim().to_string();
    if title.is_empty() {
        return Err(err("invalid", "Give the conversation a title."));
    }
    let now = format_timestamp(chat.now());
    let id = args.id.clone();
    let ok = chat.db.write(move |c| {
        let exists: bool = c
            .query_row(
                "SELECT 1 FROM chat_turns WHERE conversation_id = ?1 LIMIT 1",
                [&id],
                |_| Ok(()),
            )
            .is_ok();
        if exists {
            store::put_record(
                c,
                &store::title_id(&id),
                &id,
                &Record::Title { title },
                &now,
            )?;
        }
        Ok(exists)
    })?;
    if !ok {
        return Err(err("not_found", format!("No conversation {}.", args.id)));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinArgs {
    pub turn_id: String,
    pub pinned: bool,
}

/// `chat_pin {turn_id, pinned}` (a reply's footer pin, 03 §7.1).
pub fn chat_pin(chat: &Chat, args: PinArgs) -> ApiResult<()> {
    let now = format_timestamp(chat.now());
    let tid = args.turn_id.clone();
    let ok = chat.db.write(move |c| {
        let conv: Option<String> = c
            .query_row(
                "SELECT conversation_id FROM chat_turns WHERE id = ?1 AND role = 'assistant'",
                [&tid],
                |r| r.get(0),
            )
            .ok();
        if let Some(conv) = &conv {
            store::put_record(
                c,
                &store::pin_id(&tid),
                conv,
                &Record::Pin {
                    turn_id: tid.clone(),
                    pinned: args.pinned,
                },
                &now,
            )?;
        }
        Ok(conv.is_some())
    })?;
    if !ok {
        return Err(err("not_found", format!("No reply {}.", args.turn_id)));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSetArgs {
    pub id: String,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub connectors: Option<Sources>,
}

/// `chat_thread_set {id, effort?, connectors?}`: the effort picker and the
/// `+` menu's connector toggles, stored per conversation.
pub fn chat_thread_set(chat: &Chat, args: ThreadSetArgs) -> ApiResult<()> {
    let effort = match args.effort.as_deref() {
        Some(e) => Some(
            EffortChoice::parse(e)
                .ok_or_else(|| {
                    err(
                        "invalid",
                        format!("{e:?} is not Auto, Quick, Thorough or Local-only"),
                    )
                })?
                .as_str()
                .to_string(),
        ),
        None => None,
    };
    let id = args.id.clone();
    let exists = chat.db.read(move |c| {
        Ok(c.query_row(
            "SELECT 1 FROM chat_turns WHERE conversation_id = ?1 LIMIT 1",
            [&id],
            |_| Ok(()),
        )
        .is_ok())
    })?;
    if !exists {
        return Err(err("not_found", format!("No conversation {}.", args.id)));
    }
    write_prefs(chat, &args.id, effort, args.connectors)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusArgs {
    #[serde(default)]
    pub effort: Option<String>,
}

/// The meta line and the input's state (03 §7.2 "Backend state";
/// `ChatStatus` in the contract). `state`, `message`, `input_disabled`,
/// `effort`, `model_short`, `privacy_line` and `hint` are beyond it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatStatus {
    /// `claude_api` | `ollama` | `lm_studio`.
    pub backend: String,
    /// "Claude API", "Ollama".
    pub backend_label: String,
    /// The model the next turn uses (configuration, shown once in the meta line).
    pub model: Option<String>,
    pub local: bool,
    /// A local backend is configured (Local-only is offered).
    pub local_available: bool,
    /// `ai.local_only` is on.
    pub local_only: bool,
    /// `ok` | `no_key` | `local_only_no_backend` | `budget` | `unreachable` | `offline`.
    pub ready: String,
    /// The line for a backend that answered with an error.
    #[serde(default)]
    pub error: Option<String>,
    /// Which connectors have an account (the `+` menu reasons).
    pub accounts: AccountsPresent,
    /// `ready` | `no_key` | `local_only_blocked` | `budget_reached` | `unreachable` | `invalid_key` | `offline`.
    #[serde(default)]
    pub state: String,
    /// The disabled input's line, or "Ready".
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub input_disabled: bool,
    #[serde(default)]
    pub effort: String,
    /// Its short form (`opus-5-5`).
    #[serde(default)]
    pub model_short: Option<String>,
    /// "Your question and the retrieved snippets are sent to …" / "Nothing leaves this machine".
    #[serde(default)]
    pub privacy_line: Option<String>,
    /// "local models may handle actions less reliably" for a local backend.
    #[serde(default)]
    pub hint: Option<String>,
}

fn short(model: &str) -> String {
    model.strip_prefix("claude-").unwrap_or(model).to_string()
}

/// `chat_status {effort?}`.
pub fn chat_status(chat: &Chat, args: StatusArgs) -> ApiResult<ChatStatus> {
    let effort = args
        .effort
        .as_deref()
        .and_then(EffortChoice::parse)
        .unwrap_or(EffortChoice::Auto);
    let (settings, accounts) = chat
        .db
        .read(|c| Ok((AiSettings::load(c)?, edge::accounts_present(c)?)))?;
    let local_available =
        settings.backend != BackendKind::ClaudeApi || settings.local_model_chat.is_some();
    // The configured backend, for the meta line when it cannot be reached.
    let configured = if settings.local_only || effort == EffortChoice::LocalOnly {
        if settings.backend == BackendKind::ClaudeApi {
            BackendKind::Ollama
        } else {
            settings.backend
        }
    } else {
        BackendKind::ClaudeApi
    };
    let configured_model = if configured == BackendKind::ClaudeApi {
        Some(settings.model_chat.clone())
    } else {
        settings.local_model_chat.clone()
    };
    let base = ChatStatus {
        backend: configured.as_str().into(),
        backend_label: configured.label().into(),
        model_short: configured_model.as_deref().map(short),
        model: configured_model,
        local: configured != BackendKind::ClaudeApi,
        local_available,
        local_only: settings.local_only,
        ready: "ok".into(),
        error: None,
        accounts,
        state: "ready".into(),
        message: "Ready".into(),
        input_disabled: false,
        effort: effort.as_str().into(),
        privacy_line: None,
        hint: None,
    };
    let disabled = |ready: &str, state: &str, e: &AiError| ChatStatus {
        ready: ready.into(),
        state: state.into(),
        message: turn::error_line(e),
        error: Some(turn::error_line(e)),
        input_disabled: true,
        ..base.clone()
    };
    match turn::resolve(chat, effort) {
        Err(e @ AiError::NoKey) => Ok(disabled("no_key", "no_key", &e)),
        Err(e @ AiError::LocalOnlyBlocked) => {
            Ok(disabled("local_only_no_backend", "local_only_blocked", &e))
        }
        Err(e @ AiError::InvalidKey) => Ok(disabled("no_key", "invalid_key", &e)),
        Err(e) => Ok(ChatStatus {
            input_disabled: false,
            ..disabled("unreachable", "unreachable", &e)
        }),
        Ok(r) => {
            let kind = r.model.kind();
            let local = kind.is_local();
            let model = r.model.model().to_string();
            let mut st = ChatStatus {
                backend: kind.as_str().into(),
                backend_label: r.label.clone(),
                model_short: Some(short(&model)),
                model: Some(model),
                local,
                privacy_line: Some(if !local {
                    format!(
                        "Your question and the retrieved snippets are sent to {}",
                        r.label
                    )
                } else if r.address == AddressClass::Private {
                    "LAN backend — data leaves this computer but not your network".into()
                } else {
                    "Nothing leaves this machine".into()
                }),
                hint: local.then(|| "local models may handle actions less reliably".into()),
                ..base.clone()
            };
            if !local && chat.preflight(effort.as_str()).as_deref() == Some(super::BUDGET_REACHED) {
                st.ready = "budget".into();
                st.state = "budget_reached".into();
                st.message = turn::error_line(&AiError::BudgetReached);
                st.input_disabled = true;
            } else if !local && chat.is_offline() {
                st.ready = "offline".into();
                st.state = "offline".into();
                st.message = "You're offline".into();
                st.error = Some("offline".into());
            }
            Ok(st)
        }
    }
}
