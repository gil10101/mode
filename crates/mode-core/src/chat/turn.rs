//! One Chat turn (03 §7, 08 §4.4): resolve the backend for the effort (the
//! local-only switch, the effort picker, the token budget), then the loop —
//! a step, the retrieval tools it asked for, the next step with their
//! results — until an answer; validate its citations (one repair step when
//! it cites nothing it retrieved), stream it as `chat.delta`, store the
//! proposed actions as cards (`chat.action`), record every step's tokens in
//! `usage_samples` (`purpose = 'chat'`) and finish with `chat.turn`. Stop
//! finishes the turn at once with the text streamed so far; a step already
//! in flight still has its tokens recorded when it returns.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Chat;
use super::actions::{self, ActionView};
use super::citations::{self, Citation, SourceItem};
use super::edge;
use super::model::{
    ChatModel, EnvelopeModel, MAX_TOKENS, StepError, StepReply, StepRequest, envelope_schema,
};
use super::prompt::{self, Exchange, ToolRecord, UserPart};
use super::store::{self, ItemRef, Record, TokenUsage, ToolCallRow, TurnMeta};
use super::tools::{self, Sources, ToolCtx, ToolSet};
use crate::connectors::ai::{
    self, AddressClass, AiConfig, AiError, AiSettings, BackendKind, Effort, Purpose,
};
use crate::db::format_timestamp;
use crate::push::Event;
use crate::repo::ChatTurn;

/// Steps per turn (each step is one backend call).
pub const MAX_STEPS: u32 = 6;
/// Tool calls honored per step.
pub const MAX_CALLS_PER_STEP: usize = 8;
/// Earlier exchanges sent as context.
pub const HISTORY_TURNS: usize = 10;

/// The effort picker (03 §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffortChoice {
    /// `ai.model_chat`, adaptive thinking, every tool.
    Auto,
    /// The scoring-class model, search tools only.
    Quick,
    /// `ai.model_chat` with `effort: high`.
    Thorough,
    /// The configured local backend for this conversation.
    LocalOnly,
}

impl EffortChoice {
    pub fn parse(s: &str) -> Option<EffortChoice> {
        match s {
            "auto" => Some(EffortChoice::Auto),
            "quick" => Some(EffortChoice::Quick),
            "thorough" => Some(EffortChoice::Thorough),
            "local_only" | "local-only" | "local" => Some(EffortChoice::LocalOnly),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EffortChoice::Auto => "auto",
            EffortChoice::Quick => "quick",
            EffortChoice::Thorough => "thorough",
            EffortChoice::LocalOnly => "local_only",
        }
    }

    /// `output_config.effort` (06 §10.2).
    pub fn effort(self) -> Effort {
        match self {
            EffortChoice::Quick => Effort::Low,
            EffortChoice::Thorough => Effort::High,
            _ => Effort::Medium,
        }
    }

    pub fn tool_set(self) -> ToolSet {
        match self {
            EffortChoice::Quick => ToolSet::Search,
            _ => ToolSet::All,
        }
    }
}

/// The backend and model a turn uses.
pub struct Resolved {
    pub model: Arc<dyn ChatModel>,
    pub settings: AiSettings,
    /// The primary provider's name and address class (the meta line, the privacy line).
    pub label: String,
    pub address: AddressClass,
    pub backups: usize,
}

/// Resolve the backend for an effort (06 §10.4: `ai.local_only` routes to the
/// local backend whatever the effort; Local-only does it per conversation;
/// §10.1A: the chat route, or the scoring route for Quick).
pub fn resolve(chat: &Chat, effort: EffortChoice) -> Result<Resolved, AiError> {
    let mut config = chat
        .db
        .read(AiConfig::load)
        .map_err(|e| AiError::Config(e.to_string()))?;
    if effort == EffortChoice::LocalOnly {
        config.settings.local_only = true;
    }
    let purpose = match effort {
        EffortChoice::Quick => Purpose::Scoring,
        _ => Purpose::Chat,
    };
    let r = ai::resolve_with(&config, &chat.keyring, chat.factory.as_ref(), purpose)?;
    Ok(Resolved {
        model: Arc::new(EnvelopeModel {
            backend: r.backend,
            model: r.model,
        }),
        settings: config.settings,
        label: r.label,
        address: r.address,
        backups: r.backups,
    })
}

/// The 03 §7.2 / §7.6 line for a backend error.
pub fn error_line(e: &AiError) -> String {
    match e {
        AiError::NoKey => "Add your Anthropic API key to use Chat".into(),
        AiError::LocalOnlyBlocked => "Local-only is on and no local backend is configured".into(),
        AiError::BudgetReached => "Monthly AI budget reached".into(),
        AiError::BadOutput(_) => "The reply could not be read".into(),
        other => other.to_string(),
    }
}

/// A running turn.
pub(crate) struct TurnHandle {
    pub turn_id: String,
    pub thread_id: String,
    /// The user's message that started it.
    pub user_id: String,
    pub cancel: AtomicBool,
    pub state: Mutex<HandleState>,
}

#[derive(Default)]
pub(crate) struct HandleState {
    pub finalized: bool,
    pub partial: String,
    pub seq: u64,
    pub meta: Option<TurnMeta>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What `chat_send` hands the worker.
#[derive(Debug, Clone)]
pub(crate) struct TurnRequest {
    pub question: String,
    pub effort: EffortChoice,
    pub attachments: Vec<ItemRef>,
    pub sources: Sources,
    pub allow_actions: bool,
}

/// How a turn ended.
pub(crate) enum Final {
    Finished {
        text: String,
        citations: Vec<Citation>,
        no_sources: bool,
        uncited: bool,
        dropped: usize,
    },
    Failed(String),
    Stopped,
}

impl Chat {
    /// `chat.turn` (`ChatTurnEvent`): `started` from [`begin`], then once
    /// `finished` / `failed` / `stopped` with the stored reply.
    pub(crate) fn emit_turn(&self, h: &TurnHandle, meta: &TurnMeta, citations: Vec<Citation>) {
        let started = meta.state == "running";
        let status = if started {
            "started"
        } else {
            edge::turn_status(&meta.state, meta.stopped)
        };
        let turn = if started {
            None
        } else {
            let (conv, id) = (h.thread_id.clone(), h.turn_id.clone());
            self.db
                .read(move |c| edge::reply(c, &conv, &id))
                .ok()
                .flatten()
        };
        let ev = edge::ChatTurnEvent {
            turn_id: h.turn_id.clone(),
            thread_id: h.thread_id.clone(),
            status: status.into(),
            usage: (!started).then_some(meta.usage),
            error: meta.error.clone(),
            turn,
            stopped: meta.stopped,
            citations: citations.iter().map(edge::Citation::from_stored).collect(),
            no_sources_label: (meta.no_sources && meta.state == "finished" && !meta.stopped)
                .then(|| citations::NO_SOURCES.to_string()),
            uncited: meta.uncited,
        };
        self.emit(
            Event::ChatTurn,
            serde_json::to_value(ev).unwrap_or(Value::Null),
        );
    }

    fn next_seq(&self, h: &TurnHandle, text: Option<&str>) -> u64 {
        let mut st = lock(&h.state);
        st.seq += 1;
        if let Some(t) = text {
            st.partial.push_str(t);
        }
        st.seq
    }

    /// `chat.delta` with reply text.
    fn delta_text(&self, h: &TurnHandle, text: &str) {
        let d = edge::ChatDelta {
            turn_id: h.turn_id.clone(),
            thread_id: h.thread_id.clone(),
            text: Some(text.to_string()),
            activity: None,
            seq: self.next_seq(h, Some(text)),
            kind: "text".into(),
        };
        self.emit(
            Event::ChatDelta,
            serde_json::to_value(d).unwrap_or(Value::Null),
        );
    }

    /// `chat.delta` with an activity group (upserted by id in the view).
    fn delta_activity(&self, h: &TurnHandle, group: edge::ActivityGroup) {
        let d = edge::ChatDelta {
            turn_id: h.turn_id.clone(),
            thread_id: h.thread_id.clone(),
            text: None,
            activity: Some(group),
            seq: self.next_seq(h, None),
            kind: "activity".into(),
        };
        self.emit(
            Event::ChatDelta,
            serde_json::to_value(d).unwrap_or(Value::Null),
        );
    }

    /// Record one step's tokens (06 §10.1: one `usage_samples` row per call).
    fn record_usage(
        &self,
        h: &TurnHandle,
        kind: BackendKind,
        model: &str,
        step: u32,
        usage: Option<ai::Usage>,
        request_id: Option<String>,
    ) {
        let Some(u) = usage else { return };
        let now = format_timestamp(self.now());
        let turn_ref = request_id.unwrap_or_else(|| format!("{}:{step}", h.turn_id));
        let model = model.to_string();
        if let Err(e) = self.db.write(move |c| {
            ai::record_usage(c, kind, Purpose::Chat, &model, &u, Some(&turn_ref), &now)
        }) {
            tracing::warn!("chat usage: {e}");
        }
        let rewrite = {
            let mut st = lock(&h.state);
            if let Some(m) = st.meta.as_mut() {
                m.usage.add(&u);
                m.steps = m.steps.max(step + 1);
            }
            st.finalized.then(|| st.meta.clone()).flatten()
        };
        // A step that returned after Stop: its tokens still count.
        if let Some(meta) = rewrite {
            let (id, conv, at) = (
                store::meta_id(&h.turn_id),
                h.thread_id.clone(),
                meta.started_at.clone(),
            );
            let _ = self
                .db
                .write(move |c| store::put_record(c, &id, &conv, &Record::Turn(meta), &at));
        }
    }

    /// Finish the turn once (Stop and the worker may race).
    pub(crate) fn finalize(&self, h: &TurnHandle, end: Final) -> bool {
        let (meta, text, citations) = {
            let mut st = lock(&h.state);
            if st.finalized {
                return false;
            }
            st.finalized = true;
            let partial = st.partial.clone();
            let Some(meta) = st.meta.as_mut() else {
                return false;
            };
            meta.finished_at = Some(format_timestamp(self.now()));
            let (text, citations) = match end {
                Final::Finished {
                    text,
                    citations,
                    no_sources,
                    uncited,
                    dropped,
                } => {
                    meta.state = "finished".into();
                    meta.no_sources = no_sources;
                    meta.uncited = uncited;
                    meta.dropped_citations = dropped;
                    (text, citations)
                }
                Final::Failed(e) => {
                    meta.state = "failed".into();
                    meta.error = Some(e);
                    (partial, Vec::new())
                }
                Final::Stopped => {
                    meta.state = "finished".into();
                    meta.stopped = true;
                    (partial, Vec::new())
                }
            };
            (meta.clone(), text, citations)
        };
        let (h_turn, conv) = (h.turn_id.clone(), h.thread_id.clone());
        let model = meta.model.clone();
        let cites_json = serde_json::to_string(&citations).unwrap_or_else(|_| "[]".into());
        let meta2 = meta.clone();
        let now = meta.finished_at.clone().unwrap_or_default();
        let res = self.db.write(move |c| {
            if !text.is_empty() || meta2.state == "finished" {
                store::put(
                    c,
                    &ChatTurn {
                        id: h_turn.clone(),
                        conversation_id: conv.clone(),
                        role: "assistant".into(),
                        content: text,
                        citations_json: cites_json,
                        model,
                        created_at: now,
                    },
                )?;
            }
            store::put_record(
                c,
                &store::meta_id(&h_turn),
                &conv,
                &Record::Turn(meta2.clone()),
                &meta2.started_at,
            )
        });
        if let Err(e) = res {
            tracing::warn!("chat turn {}: {e}", h.turn_id);
        }
        self.emit_turn(h, &meta, citations);
        true
    }
}

fn history(chat: &Chat, thread_id: &str, before_turn: &str) -> Vec<Exchange> {
    let Ok(Some(t)) = chat.db.read(|c| store::thread(c, thread_id)) else {
        return Vec::new();
    };
    let prior: Vec<_> = t
        .turns
        .into_iter()
        .filter(|x| x.turn_id != before_turn && x.state != "running")
        .collect();
    let skip = prior.len().saturating_sub(HISTORY_TURNS);
    prior
        .into_iter()
        .skip(skip)
        .map(|x| Exchange {
            user: x.user.text,
            assistant: x.reply.as_ref().map(|r| r.text.clone()),
            cited: x
                .reply
                .map(|r| {
                    r.citations
                        .iter()
                        .map(|c| format!("[{}] {} {} \"{}\"", c.n, c.kind, c.id, c.title))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

fn sources_line(s: &Sources) -> Option<String> {
    let off: Vec<&str> = [
        ("Gmail", s.gmail),
        ("Slack", s.slack),
        ("Calendar", s.calendar),
    ]
    .iter()
    .filter(|(_, on)| !on)
    .map(|(n, _)| *n)
    .collect();
    (!off.is_empty()).then(|| {
        format!(
            "The user turned off {} for this conversation; do not search there.",
            off.join(", ")
        )
    })
}

/// Run a turn to its end (on the worker thread, or inline).
pub(crate) fn run(chat: &Chat, h: &TurnHandle, req: TurnRequest) {
    let resolved = match resolve(chat, req.effort) {
        Ok(r) => r,
        Err(e) => {
            chat.finalize(h, Final::Failed(error_line(&e)));
            return;
        }
    };
    let model = resolved.model;
    let kind = model.kind();
    {
        let mut st = lock(&h.state);
        if let Some(m) = st.meta.as_mut() {
            m.model = Some(model.model().to_string());
            m.backend = Some(kind.as_str().to_string());
        }
    }
    let budget = |chat: &Chat| -> Option<ai::Budget> {
        if !kind.is_budgeted() {
            return None;
        }
        let (now, tz, limit) = (chat.now(), chat.tz, resolved.settings.budget_tokens_month);
        chat.db
            .read(move |c| crate::urgency::with_tz(c, tz, |z| ai::budget(c, now, z, limit)))
            .ok()
    };
    let tool_specs = tools::specs(req.effort.tool_set());
    let tool_names: Vec<&str> = tool_specs.iter().map(|t| t.name).collect();
    let system = prompt::system(&tool_specs);
    let schema = envelope_schema(&tool_names, &actions::ACTION_KINDS);
    let ctx = ToolCtx {
        now: chat.now(),
        tz: chat.tz,
        sources: req.sources,
    };

    // Attachments: retrieved context the user chose (bodies within retention).
    let mut retrieved: Vec<SourceItem> = Vec::new();
    let mut attached: Vec<Value> = Vec::new();
    for a in &req.attachments {
        let (kind, id) = (a.kind.clone(), a.id.clone());
        if let Ok(out) = chat
            .db
            .read(move |c| tools::attachment(c, &ctx, &kind, &id))
        {
            if out.error.is_none() {
                retrieved.extend(out.items);
                attached.push(json!({ "kind": a.kind, "id": a.id, "data": out.data }));
            }
        }
    }
    let past = history(chat, &h.thread_id, &h.turn_id);
    let mut records: Vec<ToolRecord> = Vec::new();
    let mut repair = false;
    let mut repaired = false;
    let mut call_n = 0u32;
    for step in 0..MAX_STEPS {
        if h.cancel.load(Ordering::SeqCst) {
            chat.finalize(h, Final::Stopped);
            return;
        }
        let last_step = step + 1 == MAX_STEPS;
        let user = prompt::user(&UserPart {
            history: &past,
            attachments: &attached,
            tools: &records,
            question: &req.question,
            sources_line: sources_line(&req.sources),
            allow_actions: req.allow_actions,
            last_step,
            repair,
        });
        if let Some(b) = budget(chat) {
            let est = ai::estimate_tokens(user.len(), system.len(), 2_000);
            if !b.allows(est, false) {
                chat.finalize(h, Final::Failed(error_line(&AiError::BudgetReached)));
                return;
            }
        }
        let step_req = StepRequest {
            system: system.clone(),
            user,
            schema: schema.clone(),
            effort: req.effort.effort(),
            max_tokens: MAX_TOKENS,
            timeout: resolved.settings.timeout(Purpose::Chat),
        };
        // The answer is streamed after its citations are validated (markers
        // renumbered); an answer that needs the repair step is never shown.
        let reply: Result<StepReply, StepError> = model.step(&step_req);
        match reply {
            Err(e) => {
                chat.record_usage(
                    h,
                    e.source.unwrap_or(kind),
                    model.model(),
                    step,
                    e.usage,
                    e.request_id,
                );
                chat.finalize(h, Final::Failed(error_line(&e.error)));
                return;
            }
            Ok(r) => {
                chat.record_usage(
                    h,
                    r.source.unwrap_or(kind),
                    model.model(),
                    step,
                    r.usage,
                    r.request_id.clone(),
                );
                if h.cancel.load(Ordering::SeqCst) {
                    chat.finalize(h, Final::Stopped);
                    return;
                }
                let env = r.envelope;
                if !env.tool_calls.is_empty() && !last_step {
                    for call in env.tool_calls.into_iter().take(MAX_CALLS_PER_STEP) {
                        call_n += 1;
                        let call_id = format!("{}:call:{call_n:03}", h.turn_id);
                        let label = tools::label(&call.name).to_string();
                        chat.delta_activity(
                            h,
                            edge::group(
                                &call_id,
                                &call.name,
                                &label,
                                &call.input,
                                &[],
                                "running",
                                None,
                            ),
                        );
                        let (name, input) = (call.name.clone(), call.input.clone());
                        let out = chat
                            .db
                            .read(move |c| tools::run(c, &ctx, &name, &input))
                            .unwrap_or_else(|e| tools::ToolOutput {
                                data: json!({ "error": e.to_string() }),
                                items: Vec::new(),
                                error: Some(e.to_string()),
                            });
                        for it in &out.items {
                            if !retrieved.iter().any(|x| x.kind == it.kind && x.id == it.id) {
                                retrieved.push(it.clone());
                            }
                        }
                        let row = ToolCallRow {
                            turn_id: h.turn_id.clone(),
                            call_id: call_id.clone(),
                            name: call.name.clone(),
                            label: label.clone(),
                            input: call.input.clone(),
                            count: out.items.len(),
                            items: out.items.clone(),
                            error: out.error.clone(),
                            at: format_timestamp(chat.now()),
                        };
                        let conv = h.thread_id.clone();
                        let rid = call_id.clone();
                        let _ = chat.db.write(move |c| {
                            let at = row.at.clone();
                            store::put_record(c, &rid, &conv, &Record::ToolCall(row), &at)
                        });
                        chat.delta_activity(
                            h,
                            edge::group(
                                &call_id,
                                &call.name,
                                &label,
                                &call.input,
                                &out.items,
                                if out.error.is_some() {
                                    "failed"
                                } else {
                                    "done"
                                },
                                out.error.as_deref(),
                            ),
                        );
                        records.push(ToolRecord {
                            name: call.name,
                            input: call.input,
                            data: out.data,
                        });
                    }
                    continue;
                }
                let answer = env.answer.trim().to_string();
                if answer.is_empty() {
                    if last_step {
                        chat.finalize(
                            h,
                            Final::Failed(format!("No answer after {MAX_STEPS} steps")),
                        );
                        return;
                    }
                    repair = false;
                    continue;
                }
                let checked = citations::validate(&answer, &env.citations, &retrieved);
                if checked.uncited && !repaired && !last_step {
                    repaired = true;
                    repair = true;
                    continue;
                }
                // Stream the validated text (renumbered markers).
                for chunk in super::model::chunks(&checked.text) {
                    if h.cancel.load(Ordering::SeqCst) {
                        chat.finalize(h, Final::Stopped);
                        return;
                    }
                    chat.delta_text(h, &chunk);
                }
                if req.allow_actions {
                    propose(chat, h, &env.actions);
                }
                chat.finalize(
                    h,
                    Final::Finished {
                        text: checked.text,
                        citations: checked.citations,
                        no_sources: checked.no_sources,
                        uncited: checked.uncited,
                        dropped: checked.dropped.len(),
                    },
                );
                return;
            }
        }
    }
    chat.finalize(
        h,
        Final::Failed(format!("No answer after {MAX_STEPS} steps")),
    );
}

/// Validate and store each proposed action as a card; emit `chat.action`.
fn propose(chat: &Chat, h: &TurnHandle, raw: &[super::model::RawAction]) {
    for a in raw.iter().take(8) {
        let a2 = a.clone();
        let prepared = match chat.db.read(move |c| actions::prepare(c, &a2)) {
            Ok(Ok(p)) => p,
            Ok(Err(reason)) => {
                tracing::info!("chat: dropped a proposed {}: {reason}", a.kind);
                continue;
            }
            Err(e) => {
                tracing::warn!("chat: proposal check failed: {e}");
                continue;
            }
        };
        let view = ActionView {
            action_id: crate::repo::new_id(),
            turn_id: h.turn_id.clone(),
            thread_id: h.thread_id.clone(),
            title: actions::title_of(&prepared.kind).into(),
            kind: prepared.kind,
            summary: prepared.summary,
            preview: prepared.preview,
            params: prepared.params,
            state: "proposed".into(),
            result_line: None,
            error: None,
            route: None,
            created_at: format_timestamp(chat.now()),
            decided_at: None,
        };
        let v2 = view.clone();
        let conv = h.thread_id.clone();
        if chat
            .db
            .write(move |c| {
                let (id, at) = (v2.action_id.clone(), v2.created_at.clone());
                store::put_record(c, &id, &conv, &Record::Action(v2), &at)
            })
            .is_ok()
        {
            chat.emit(
                Event::ChatAction,
                serde_json::to_value(edge::ChatActionEvent::of(&view)).unwrap_or(Value::Null),
            );
        }
    }
}

/// Start a turn: the user row, the `running` meta row, `chat.turn started`.
pub(crate) fn begin(
    chat: &Chat,
    thread_id: &str,
    req: &TurnRequest,
) -> crate::db::Result<Arc<TurnHandle>> {
    let now = format_timestamp(chat.now());
    let turn_id = crate::repo::new_id();
    let user_id = crate::repo::new_id();
    let meta = TurnMeta {
        turn_id: turn_id.clone(),
        user_id: user_id.clone(),
        effort: req.effort.as_str().into(),
        state: "running".into(),
        stopped: false,
        error: None,
        usage: TokenUsage::default(),
        model: None,
        backend: None,
        steps: 0,
        no_sources: false,
        uncited: false,
        dropped_citations: 0,
        sources: req.sources,
        started_at: now.clone(),
        finished_at: None,
    };
    let (conv, q, m2) = (thread_id.to_string(), req.question.clone(), meta.clone());
    let atts = req.attachments.clone();
    chat.db.write(move |c| {
        store::put(
            c,
            &ChatTurn {
                id: user_id,
                conversation_id: conv.clone(),
                role: "user".into(),
                content: q,
                citations_json: serde_json::to_string(&atts)?,
                model: None,
                created_at: now.clone(),
            },
        )?;
        store::put_record(
            c,
            &store::meta_id(&m2.turn_id),
            &conv,
            &Record::Turn(m2.clone()),
            &now,
        )
    })?;
    let h = Arc::new(TurnHandle {
        turn_id,
        thread_id: thread_id.to_string(),
        user_id: meta.user_id.clone(),
        cancel: AtomicBool::new(false),
        state: Mutex::new(HandleState {
            meta: Some(meta.clone()),
            ..Default::default()
        }),
    });
    chat.emit_turn(&h, &meta, Vec::new());
    Ok(h)
}
