//! Chat over my data (ADR-0022; 03 §7; 06 §10.5–§10.6; 08 §3.2 `chat`,
//! §4.4): an assistant over the user's own notes, sessions, messages and
//! calendar that retrieves with tools, cites every item it used and
//! proposes actions that run only after confirmation.
//!
//! * [`tools`]: the retrieval tools of 06 §10.6 (FTS5 + structured filters).
//! * [`citations`]: validation of the model's citations against what the
//!   turn retrieved (03 §7.4).
//! * [`actions`]: proposals, their cards and the confirmed execution.
//! * [`model`] / [`prompt`]: the backend step (the reply envelope over the
//!   shared AI client) and the vendor-free prompts.
//! * [`turn`]: the agentic loop, streaming, usage accounting, Stop.
//! * [`store`]: `chat_turns` rows, the thread view, the Sources panel.
//! * [`api`]: one function per command.
//! * [`edge`]: the shapes of `ui/src/state/contract-chat.ts` the commands
//!   and streams carry.
//!
//! Streams go to the app window through [`Chat::set_sink`] (`chat.delta`,
//! `chat.turn`, `chat.action`), never through the debounced publisher.

pub mod actions;
pub mod api;
pub mod citations;
pub mod edge;
pub mod model;
pub mod prompt;
pub mod store;
pub mod tools;
pub mod turn;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use serde_json::Value;
use time::OffsetDateTime;

use crate::connectors::ai::{AiError, BackendFactory};
use crate::db::Db;
use crate::keyring::Keyring;
use crate::notes::ctx::TzMode;
use crate::push::{Event, Push};
use actions::{Effects, NoEffects};
use citations::Citation;
use turn::{EffortChoice, TurnHandle};

/// The 03 §7B.3 skip reason when Chat has no backend.
pub const NO_BACKEND: &str = "no AI backend";
/// Skip reason under the local-only switch with no local backend.
pub const LOCAL_ONLY_BLOCKED: &str =
    "the local-only switch is on and no local backend is configured";
/// Skip reason at the monthly budget.
pub const BUDGET_REACHED: &str = "the monthly AI token budget was reached";

pub type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;
type SinkFn = Arc<dyn Fn(&Push) + Send + Sync>;

/// Options for [`Chat::new`].
#[derive(Clone, Default)]
pub struct ChatOptions {
    /// A fixed clock (tests).
    pub clock: Option<Clock>,
    /// The device zone for local dates (the tools' `now`, calendar days).
    pub tz: TzMode,
    /// Run turns on the calling thread (tests, automations).
    pub inline: bool,
}

/// The Chat service: one per app.
pub struct Chat {
    me: Weak<Chat>,
    pub(crate) db: Arc<Db>,
    pub(crate) keyring: Arc<Keyring>,
    pub(crate) factory: Arc<dyn BackendFactory>,
    clock: Option<Clock>,
    pub(crate) tz: TzMode,
    inline: bool,
    effects: RwLock<Arc<dyn Effects>>,
    sink: Mutex<Option<SinkFn>>,
    turns: Mutex<HashMap<String, Arc<TurnHandle>>>,
    /// The app's reachability verdict (`chat_status` reads `offline`).
    offline: AtomicBool,
}

impl std::fmt::Debug for Chat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chat")
            .field("inline", &self.inline)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// One answer for an automation (03 §7B.4 *Ask Chat*).
#[derive(Debug, Clone, PartialEq)]
pub struct OneShot {
    pub thread_id: String,
    pub turn_id: String,
    pub text: String,
    pub citations: Vec<Citation>,
}

impl Chat {
    /// `factory` is the AI backend factory the connectors service uses
    /// (`Connectors::ai_factory()`), `keyring` the same keyring.
    pub fn new(
        db: Arc<Db>,
        keyring: Arc<Keyring>,
        factory: Arc<dyn BackendFactory>,
        opts: ChatOptions,
    ) -> Arc<Chat> {
        Arc::new_cyclic(|me| Chat {
            me: me.clone(),
            db,
            keyring,
            factory,
            clock: opts.clock,
            tz: opts.tz,
            inline: opts.inline,
            effects: RwLock::new(Arc::new(NoEffects)),
            sink: Mutex::new(None),
            turns: Mutex::new(HashMap::new()),
            offline: AtomicBool::new(false),
        })
    }

    /// The app reports the network state (`chat_status.ready = offline`).
    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::SeqCst);
    }

    pub fn is_offline(&self) -> bool {
        self.offline.load(Ordering::SeqCst)
    }

    /// Stop every running turn, keeping the text streamed so far (quit).
    /// Returns the turns stopped.
    pub fn stop_all(&self) -> usize {
        let running: Vec<Arc<TurnHandle>> = lock(&self.turns).values().cloned().collect();
        let mut n = 0;
        for h in running {
            h.cancel.store(true, Ordering::SeqCst);
            if self.finalize(&h, turn::Final::Stopped) {
                n += 1;
            }
        }
        n
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    pub fn now(&self) -> OffsetDateTime {
        match &self.clock {
            Some(c) => c(),
            None => OffsetDateTime::now_utc(),
        }
    }

    /// What executes confirmed actions ([`actions::LiveEffects`] in the app).
    pub fn set_effects(&self, e: Arc<dyn Effects>) {
        *self.effects.write().unwrap_or_else(|p| p.into_inner()) = e;
    }

    pub(crate) fn effects(&self) -> Arc<dyn Effects> {
        Arc::clone(&self.effects.read().unwrap_or_else(|p| p.into_inner()))
    }

    /// Where `chat.delta` / `chat.turn` / `chat.action` go (the app emits
    /// them to the app window directly, in order).
    pub fn set_sink(&self, f: impl Fn(&Push) + Send + Sync + 'static) {
        *lock(&self.sink) = Some(Arc::new(f));
    }

    pub(crate) fn emit(&self, event: Event, payload: Value) {
        let f = lock(&self.sink).clone();
        if let Some(f) = f {
            f(&Push { event, payload });
        }
    }

    /// Turns still running.
    pub fn running(&self) -> Vec<String> {
        lock(&self.turns).keys().cloned().collect()
    }

    pub(crate) fn handle(&self, turn_id: &str) -> Option<Arc<TurnHandle>> {
        lock(&self.turns).get(turn_id).cloned()
    }

    pub(crate) fn running_in(&self, thread_id: &str) -> Option<Arc<TurnHandle>> {
        lock(&self.turns)
            .values()
            .find(|h| h.thread_id == thread_id)
            .cloned()
    }

    /// Wait until a turn's worker is done (tests, quit).
    pub fn wait(&self, turn_id: &str, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if !lock(&self.turns).contains_key(turn_id) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    /// Start a turn's worker (inline in tests and automations).
    pub(crate) fn start(&self, h: Arc<TurnHandle>, req: turn::TurnRequest) -> std::io::Result<()> {
        lock(&self.turns).insert(h.turn_id.clone(), Arc::clone(&h));
        if self.inline {
            turn::run(self, &h, req);
            lock(&self.turns).remove(&h.turn_id);
            return Ok(());
        }
        let me = self.me.clone();
        let id = h.turn_id.clone();
        let spawned = std::thread::Builder::new()
            .name("mode-chat-turn".into())
            .spawn(move || {
                if let Some(chat) = me.upgrade() {
                    turn::run(&chat, &h, req);
                    lock(&chat.turns).remove(&h.turn_id);
                }
            });
        if let Err(e) = spawned {
            lock(&self.turns).remove(&id);
            return Err(e);
        }
        Ok(())
    }

    /// Why an *Ask Chat* run cannot start (03 §7B.3), `None` when it can.
    pub fn preflight(&self, effort: &str) -> Option<String> {
        let e = EffortChoice::parse(effort).unwrap_or(EffortChoice::Auto);
        match turn::resolve(self, e) {
            Err(AiError::LocalOnlyBlocked) => Some(LOCAL_ONLY_BLOCKED.into()),
            Err(AiError::NoKey) | Err(AiError::Config(_)) => Some(NO_BACKEND.into()),
            Err(_) => None,
            Ok(r) => {
                if !r.model.kind().is_budgeted() {
                    return None;
                }
                let (now, tz, limit) = (self.now(), self.tz, r.settings.budget_tokens_month);
                let reached = self
                    .db
                    .read(move |c| {
                        crate::urgency::with_tz(c, tz, |z| {
                            crate::connectors::ai::budget(c, now, z, limit)
                        })
                    })
                    .map(|b| b.month_reached())
                    .unwrap_or(false);
                reached.then(|| BUDGET_REACHED.into())
            }
        }
    }

    /// One turn in a new conversation, run on this thread, no actions
    /// (automations: nobody is there to confirm one).
    pub fn ask_once(&self, prompt: &str, effort: &str) -> Result<OneShot, String> {
        let effort = EffortChoice::parse(effort).unwrap_or(EffortChoice::Auto);
        let thread_id = crate::repo::new_id();
        let req = turn::TurnRequest {
            question: prompt.to_string(),
            effort,
            attachments: Vec::new(),
            sources: tools::Sources::default(),
            allow_actions: false,
        };
        let h = turn::begin(self, &thread_id, &req).map_err(|e| e.to_string())?;
        let turn_id = h.turn_id.clone();
        lock(&self.turns).insert(turn_id.clone(), Arc::clone(&h));
        turn::run(self, &h, req);
        lock(&self.turns).remove(&turn_id);
        let t = self
            .db
            .read(|c| store::thread(c, &thread_id))
            .map_err(|e| e.to_string())?
            .and_then(|t| t.turns.into_iter().find(|x| x.turn_id == turn_id))
            .ok_or("the turn vanished")?;
        if t.state == "failed" {
            return Err(t.error.unwrap_or_else(|| "Chat failed".into()));
        }
        let reply = t.reply.ok_or("no answer")?;
        Ok(OneShot {
            thread_id,
            turn_id,
            text: reply.text,
            citations: reply.citations,
        })
    }
}
