//! The session engine (05 §4, §7, §9, §12): turns relay frames into rows
//! through [`apply`](super::apply), answers decisions, reacts to relay
//! resolutions, probes process liveness and serves the roster, pill and
//! detail views. One instance per app; every method is synchronous and safe
//! to call from a blocking task.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use mode_wire::{Behavior, PermissionDecision, events, question_output};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::always::{self, Rules};
use super::apply::{self, ApplyInput, Queues};
use super::history;
use super::liveness::{self, Processes};
use super::naming;
use super::views::{self, PillPayload, SessionDetail, SessionItem};
use super::writes::{self, DecisionRecord};
use super::{Pending, SessionState, WaitingKind};
use crate::db::{Db, DbError, Result};
use crate::ipc::{Inbound, Resolved, ResolvedKind, Server};
use crate::push;
use crate::queries::Clock;
use crate::repo::{self, Session};
use crate::settings;

pub use super::apply::{Applied, AutoDecision};

/// Relay rows older than this on a live session read `failed` (05 §6A.1).
pub const RELAY_FRESH_SECS: i64 = 5 * 60;

/// [`Engine::jump_info`]: the inputs of the jump-to-terminal classifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JumpInfo {
    pub session_id: String,
    /// `external` | `embedded` | `headless`.
    pub launch_mode: String,
    /// `hooked` | `launched` | `discovered`.
    pub tracking: String,
    pub state: String,
    pub cwd: String,
    /// `terminal.app` | `iterm2` | `warp` | `ghostty` | `windows_terminal` |
    /// `wsl` | `powershell`, or another host's `TERM_PROGRAM`.
    pub terminal_app: Option<String>,
    pub terminal_ref: Option<String>,
    /// The `claude` process (the relay's parent), while it is alive.
    pub pid: Option<u32>,
    pub tty: Option<String>,
    pub ended: bool,
}

/// What the overlay sends to `session_decide` (`ui/src/state/contract.ts`
/// `Decision`).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Decision {
    Permission {
        behavior: Behavior,
        #[serde(default)]
        always: bool,
        #[serde(default)]
        message: Option<String>,
    },
    Answers {
        answers: Vec<Answer>,
    },
}

/// One answered question: the chosen label, or labels for `multiSelect`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Answer {
    pub question: String,
    pub answer: Value,
}

/// `session_decide`'s result (`DecideResult` in the contract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DecideOutcome {
    Accepted,
    AlreadyAnswered,
    AnsweredInTerminal,
    Failed { message: String },
}

impl DecideOutcome {
    fn failed(message: impl Into<String>) -> Self {
        DecideOutcome::Failed {
            message: message.into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DecideError {
    #[error(transparent)]
    Db(#[from] DbError),
}

/// A session the engine touched, for callers that publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub session_id: String,
    pub pushes: Vec<push::Event>,
}

/// A session reference the commands accept: the Mode id.
pub type SessionRef = String;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub struct Engine {
    db: Arc<Db>,
    server: RwLock<Option<Arc<Server>>>,
    always: Mutex<Rules>,
    processes: Mutex<Processes>,
    queued: Queues,
    /// Launch ids handed out by the PTY host (05 §8.1); a frame carrying one
    /// makes its session `launched` / `embedded`.
    launches: Mutex<std::collections::HashSet<String>>,
    /// Launch → row joins and the driver source of the PTY host (05 §8, §14).
    embedded: super::embedded::Registry,
    /// The machine boot the recorded pids belong to (`liveness::boot_id`),
    /// and when it happened, if known.
    boot: String,
    boot_time: Option<time::OffsetDateTime>,
}

/// What the startup reconciliation did (05 §4.2 "Mode starts").
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// Rows closed: Mode's own children, rows of a previous boot, external
    /// rows whose process is gone.
    pub closed: usize,
    /// External rows put back to `idle` (their process is alive or unknown).
    pub reset: usize,
    /// Live external processes registered for the liveness probe again.
    pub tracked: usize,
    pub pushes: Vec<push::Event>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("processes", &lock(&self.processes).len())
            .finish()
    }
}

impl Engine {
    pub fn new(db: Arc<Db>) -> Self {
        Self::with_boot(db, liveness::boot_id(), liveness::boot_time())
    }

    /// An engine that believes the machine booted as given (tests).
    pub fn with_boot(
        db: Arc<Db>,
        boot: impl Into<String>,
        boot_time: Option<time::OffsetDateTime>,
    ) -> Self {
        Self {
            db,
            server: RwLock::new(None),
            always: Mutex::new(Rules::default()),
            processes: Mutex::new(Processes::default()),
            queued: Arc::new(Mutex::new(HashMap::new())),
            launches: Mutex::new(Default::default()),
            embedded: super::embedded::Registry::default(),
            boot: boot.into(),
            boot_time,
        }
    }

    /// The boot identity recorded pids are tagged with.
    pub fn boot(&self) -> &str {
        &self.boot
    }

    /// Persist a session's process with the boot it belongs to (05 §9.1), so
    /// the next start can probe it.
    fn persist_process(&self, session_id: &str, pid: u32) -> Result<()> {
        let (sid, boot) = (session_id.to_string(), self.boot.clone());
        self.db.write(move |c| {
            c.execute(
                "UPDATE sessions SET pid = ?1, boot_id = ?2
                  WHERE id = ?3 AND (pid IS NOT ?1 OR boot_id IS NOT ?2)",
                params![i64::from(pid), boot, sid],
            )?;
            Ok(())
        })
    }

    /// Launch registrations and the driver source (`crate::pty`).
    pub fn embedded(&self) -> &super::embedded::Registry {
        &self.embedded
    }

    /// The PTY host spawned `claude` with this pid (liveness, jump info).
    pub fn record_process(&self, session_id: &str, pid: u32) {
        if lock(&self.processes).record(session_id, Some(pid), None)
            && let Err(e) = self.persist_process(session_id, pid)
        {
            tracing::warn!(error = %e, session = session_id, "pid not persisted");
        }
    }

    /// The process is gone and its exit is handled by the PTY host; the
    /// liveness probe must not infer an end meanwhile.
    pub fn forget_process(&self, session_id: &str) {
        lock(&self.processes).forget(session_id);
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    /// The relay server, once bound; decisions go through it.
    pub fn set_server(&self, server: Arc<Server>) {
        *self.server.write().unwrap_or_else(|p| p.into_inner()) = Some(server);
    }

    pub fn server(&self) -> Option<Arc<Server>> {
        self.server
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Expect a frame with this `launch_id` (05 §8.1).
    pub fn register_launch(&self, launch_id: &str) {
        lock(&self.launches).insert(launch_id.to_string());
    }

    /// The `always` rules of a session (05 §7.2).
    pub fn rules(&self, session_id: &str) -> Vec<String> {
        lock(&self.always).list(session_id)
    }

    pub fn remove_rule(&self, session_id: &str, key: &str) -> bool {
        lock(&self.always).remove(session_id, key)
    }

    /// Cards queued behind the open one.
    pub fn queued(&self, session_id: &str) -> usize {
        lock(&self.queued).get(session_id).map_or(0, |q| q.len())
    }

    /// The known process of a session (the relay's `ppid`).
    pub fn process(&self, session_id: &str) -> Option<u32> {
        lock(&self.processes).get(session_id).map(|p| p.pid)
    }

    fn base_of(row: &Session) -> Option<String> {
        row.repo_root
            .clone()
            .or_else(|| (!row.cwd.is_empty()).then(|| row.cwd.clone()))
    }

    // ------------------------------------------------------------- frames

    /// Apply one relay frame. `None` for the installer's round trip and for
    /// a frame already stored (`session_events.id = frame.id`). Runs git for
    /// `SessionStart` / `CwdChanged` / first frames: call it off the async
    /// runtime.
    pub fn apply(&self, inbound: &Inbound) -> Result<Option<Applied>> {
        // Paths are canonical from here on (`crate::paths`); `tool_input` is
        // left verbatim because answers echo it back to Claude Code.
        let mut canonical = inbound.frame.clone();
        naming::canonical_paths(&mut canonical.payload);
        let frame = &canonical;
        let ev = &frame.payload;
        if ev.is_round_trip() {
            return Ok(None);
        }
        let name = ev.hook_event_name.as_str();
        let claude_id = ev.session_id.clone();
        let existing = self.db.read(|c| writes::find_top_level(c, &claude_id))?;
        let cwd_for_repo = if name == events::CWD_CHANGED {
            ev.get("new_cwd")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| ev.cwd.clone())
        } else {
            ev.cwd.clone()
        };
        let needs_repo = matches!(name, events::SESSION_START | events::CWD_CHANGED)
            || existing
                .as_ref()
                .is_none_or(|r| r.repo_root.is_none() && !ev.cwd.is_empty() && r.cwd != ev.cwd);
        let repo =
            (needs_repo && !cwd_for_repo.is_empty()).then(|| naming::repo_info(&cwd_for_repo));
        let terminal = naming::terminal_from_env(&frame.env, frame.tty.as_deref());
        let auto_rule = if name == events::PERMISSION_REQUEST {
            existing.as_ref().and_then(|r| {
                lock(&self.always).matching(
                    &r.id,
                    ev.tool_name().unwrap_or("tool"),
                    ev.get("tool_input"),
                    Self::base_of(r).as_deref(),
                )
            })
        } else {
            None
        };
        // 05 §8.1: a frame carrying a launch id joins the row the PTY host
        // created before any hook arrived (its placeholder id is replaced).
        let join = frame
            .launch_id
            .as_deref()
            .and_then(|l| self.embedded.launch(l).map(|r| (l.to_string(), r)));
        let launched = join.is_some()
            || frame
                .launch_id
                .as_deref()
                .is_some_and(|l| lock(&self.launches).remove(l));
        let input = ApplyInput {
            frame: frame.clone(),
            relay_stale: inbound.relay_stale,
            repo,
            terminal,
            auto_rule,
            launched,
            launch_mode: join.as_ref().map(|(_, r)| r.launch_mode.clone()),
            now: crate::db::now_rfc3339(),
        };
        let queued = Arc::clone(&self.queued);
        let server = self.server();
        let join_row = join.as_ref().map(|(_, r)| r.row_id.clone());
        let applied = self.db.write(move |c| {
            repo::tx(c, |tx| {
                if let Some(row) = &join_row {
                    super::embedded::attach_claude_id(tx, row, &input.frame.payload.session_id)?;
                }
                apply::apply_in_tx(tx, &input, &queued, server.as_deref())
            })
        })?;
        let Some(mut applied) = applied else {
            return Ok(None);
        };
        if let Some((launch_id, reg)) = &join {
            if reg.row_id != applied.session_id {
                self.embedded.moved(launch_id, &applied.session_id);
            }
        }
        if let Some(pid) = frame.ppid
            && lock(&self.processes).record(&applied.session_id, Some(pid), frame.tty.as_deref())
        {
            self.persist_process(&applied.session_id, pid)?;
        }
        if name == events::SESSION_END {
            lock(&self.processes).forget(&applied.session_id);
            lock(&self.always).clear(&applied.session_id);
            lock(&self.queued).remove(&applied.session_id);
        }
        if let Some(auto) = &mut applied.auto_decision {
            auto.sent = match self.server() {
                Some(s) => s.try_decide(&auto.frame_id, auto.output.clone()),
                None => false,
            };
            tracing::info!(target: "sessions", session = %applied.session_id, rule = %auto.rule, sent = auto.sent, "always rule answered a PermissionRequest");
        }
        tracing::debug!(target: "sessions", session = %applied.session_id, event = %applied.event, before = %applied.state_before, after = %applied.state_after, note = applied.note, "frame applied");
        Ok(Some(applied))
    }

    // ---------------------------------------------------------- decisions

    /// Answer the open card of a session (05 §7.1, §7.3, §7.4): the first
    /// decision for a pending item wins.
    pub fn decide(&self, session_id: &str, decision: Decision) -> Result<DecideOutcome> {
        let sid = session_id.to_string();
        let Some(row) = self.db.read(|c| repo::get_by_id::<Session>(c, &sid))? else {
            return Ok(DecideOutcome::failed("no such session"));
        };
        let Some(pending) = row.pending_json.as_deref().and_then(Pending::from_json) else {
            return Ok(DecideOutcome::AlreadyAnswered);
        };
        if pending.timed_out {
            return Ok(DecideOutcome::failed("timed out; answer in the terminal"));
        }
        let Some(frame_id) = pending.frame_id.clone().filter(|_| pending.answerable) else {
            return Ok(DecideOutcome::failed(
                "this prompt can only be answered in the terminal",
            ));
        };
        let base = Self::base_of(&row);
        let (output, rec) = match (pending.kind, &decision) {
            (
                WaitingKind::Permission,
                Decision::Permission {
                    behavior,
                    always,
                    message,
                },
            ) => {
                let tool = pending.tool_name.clone().unwrap_or_else(|| "tool".into());
                match behavior {
                    Behavior::Allow => {
                        let rule = always.then(|| {
                            always::rule_key(&tool, pending.tool_input.as_ref(), base.as_deref())
                        });
                        let summary = match &rule {
                            Some(r) => format!("allowed in Mode · always: {r}"),
                            None => "allowed in Mode".into(),
                        };
                        (
                            PermissionDecision::allow().output(),
                            DecisionRecord {
                                event: "mode.approve",
                                summary,
                                tool_rule: rule.clone().map(|r| (r, "allowed", "always rule")),
                                next: None,
                            },
                        )
                    }
                    Behavior::Deny => (
                        PermissionDecision::deny(message.clone()).output(),
                        DecisionRecord {
                            event: "mode.deny",
                            summary: match message {
                                Some(m) if !m.trim().is_empty() => format!("denied in Mode · {m}"),
                                _ => "denied in Mode".into(),
                            },
                            tool_rule: Some((tool, "denied", "denied in Mode")),
                            next: None,
                        },
                    ),
                }
            }
            (WaitingKind::Question, Decision::Answers { answers }) => {
                let mut map = Map::new();
                for a in answers {
                    map.insert(a.question.clone(), a.answer.clone());
                }
                let input = pending.tool_input.clone().unwrap_or(Value::Null);
                (
                    question_output(&input, &map),
                    DecisionRecord {
                        event: "mode.answer",
                        summary: format!("answered in Mode · {} answer(s)", map.len()),
                        tool_rule: None,
                        next: None,
                    },
                )
            }
            _ => {
                return Ok(DecideOutcome::failed(
                    "the decision does not match the open card",
                ));
            }
        };
        let sent = match self.server() {
            Some(s) => s.try_decide(&frame_id, output),
            None => true,
        };
        if !sent {
            return Ok(DecideOutcome::AnsweredInTerminal);
        }
        if let Some((rule, _, _)) = &rec.tool_rule {
            if rec.event == "mode.approve" {
                lock(&self.always).add(&row.id, rule.clone());
            }
        }
        let next = apply::pop_open_queued(&self.queued, self.server().as_deref(), &row.id);
        let rec = DecisionRecord { next, ..rec };
        let pending_c = pending.clone();
        self.db.write(move |c| {
            repo::tx(c, |tx| {
                writes::record_decision(tx, &row, &pending_c, rec, &crate::db::now_rfc3339())
            })
        })?;
        Ok(DecideOutcome::Accepted)
    }

    /// A pending decision was resolved by the relay side (05 §7.4):
    /// deadline / cancel / EOF gray the card and keep the state; the other
    /// kinds are already reflected by the frame that caused them.
    pub fn resolved(&self, r: &Resolved) -> Result<Vec<push::Event>> {
        {
            let mut q = lock(&self.queued);
            for list in q.values_mut() {
                list.retain(|p| p.frame_id.as_deref() != Some(r.id.as_str()));
            }
        }
        if !matches!(
            r.kind,
            ResolvedKind::Timeout | ResolvedKind::Cancelled | ResolvedKind::Eof
        ) {
            return Ok(Vec::new());
        }
        let claude_id = r.session_id.clone();
        let Some(row) = self.db.read(|c| writes::find_top_level(c, &claude_id))? else {
            return Ok(Vec::new());
        };
        let Some(pending) = row.pending_json.as_deref().and_then(Pending::from_json) else {
            return Ok(Vec::new());
        };
        if pending.frame_id.as_deref() != Some(r.id.as_str()) || pending.timed_out {
            return Ok(Vec::new());
        }
        self.db.write(move |c| {
            repo::tx(c, |tx| {
                writes::record_timeout(tx, &row, &pending, &crate::db::now_rfc3339())
            })
        })?;
        Ok(vec![push::Event::Sessions, push::Event::Pill])
    }

    // ------------------------------------------------------ user actions

    /// `unread` → `done` (05 §12 "Acknowledge").
    pub fn acknowledge(&self, session_id: &str) -> Result<Vec<push::Event>> {
        let sid = session_id.to_string();
        let changed = self.db.write(move |c| {
            let now = crate::db::now_rfc3339();
            let n = c.execute(
                "UPDATE sessions SET acknowledged_at = COALESCE(acknowledged_at, ?1),
                        state = CASE WHEN state = 'unread' THEN 'done' ELSE state END,
                        state_since = CASE WHEN state = 'unread' THEN ?1 ELSE state_since END,
                        state_reason = CASE WHEN state = 'unread' THEN 'acknowledge' ELSE state_reason END,
                        updated_at = ?1
                  WHERE id = ?2",
                rusqlite::params![now, sid],
            )?;
            Ok(n > 0)
        })?;
        Ok(if changed {
            vec![push::Event::Sessions, push::Event::Pill]
        } else {
            Vec::new()
        })
    }

    /// Hide a row from the roster (05 §12 "Dismiss"); refused for a live
    /// `working` / `waiting` / `idle` session.
    pub fn dismiss(&self, session_id: &str) -> Result<Vec<push::Event>> {
        let sid = session_id.to_string();
        let Some(row) = self.db.read(|c| repo::get_by_id::<Session>(c, &sid))? else {
            return Err(DbError::NotFound {
                table: "sessions",
                key: session_id.to_string(),
            });
        };
        let state: SessionState = row.state.parse().unwrap_or(SessionState::Idle);
        if !state.dismissable(row.ended_at.is_some()) {
            return Err(DbError::Invalid {
                what: "dismiss",
                detail: format!("a {state} session cannot be dismissed while it is live"),
            });
        }
        let sid = session_id.to_string();
        self.db.write(move |c| {
            let now = crate::db::now_rfc3339();
            c.execute(
                "UPDATE sessions SET dismissed_at = COALESCE(dismissed_at, ?1), updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, sid],
            )?;
            Ok(())
        })?;
        Ok(vec![push::Event::Sessions, push::Event::Pill])
    }

    /// A user-chosen name (`sessions.custom_names`), kept across restarts and
    /// never overwritten by a title; `None` restores the computed name.
    pub fn rename(&self, session_id: &str, name: Option<&str>) -> Result<Vec<push::Event>> {
        let sid = session_id.to_string();
        let Some(row) = self.db.read(|c| repo::get_by_id::<Session>(c, &sid))? else {
            return Err(DbError::NotFound {
                table: "sessions",
                key: session_id.to_string(),
            });
        };
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        let device = self.db.device_id().to_string();
        self.db.write(move |c| {
            let mut names: HashMap<String, String> =
                settings::get_as_in(c, "sessions.custom_names").unwrap_or_default();
            let display = match &name {
                Some(n) => {
                    names.insert(row.claude_session_id.clone(), n.clone());
                    n.clone()
                }
                None => {
                    names.remove(&row.claude_session_id);
                    naming::display_name(
                        row.title.as_deref(),
                        row.repo_root.as_deref(),
                        &row.cwd,
                        &row.claude_session_id,
                    )
                }
            };
            settings::set_in(
                c,
                &device,
                "sessions.custom_names",
                &serde_json::to_value(&names)?,
            )?;
            c.execute(
                "UPDATE sessions SET display_name = ?, updated_at = ? WHERE id = ?",
                rusqlite::params![display, crate::db::now_rfc3339(), row.id],
            )?;
            Ok(())
        })?;
        Ok(vec![push::Event::Sessions, push::Event::Pill])
    }

    // ----------------------------------------------------------- liveness

    /// Once at start (05 §4.2 "Mode starts"): settle what the previous run
    /// left open. Mode's own children (embedded and headless rows) died with
    /// it; a row whose last event predates this boot, or whose pid belongs to
    /// another boot, outlived nothing; an external row whose persisted pid is
    /// alive is tracked again, one whose pid is gone is closed, one with no
    /// pid on record is kept; in the two kept cases `waiting` and `working`
    /// go back to `idle` (the pending decision died with the relay
    /// connection) until the next frame. Discovered rows close only on a
    /// reboot; the inactivity rule covers the rest.
    pub fn reconcile_startup(&self) -> Result<Reconciled> {
        self.reconcile_startup_with(liveness::pid_alive, &crate::db::now_rfc3339())
    }

    pub fn reconcile_startup_with(
        &self,
        alive: impl Fn(u32) -> bool,
        now: &str,
    ) -> Result<Reconciled> {
        use super::writes::InferredEnd;
        enum Act {
            Close(InferredEnd),
            CloseDiscovered,
            Reset { track: Option<u32> },
            Leave,
        }
        let open = self
            .db
            .read(|c| repo::query::<Session>(c, "ended_at IS NULL", &[]))?;
        let acts: Vec<(String, Act)> = open
            .iter()
            .map(|row| {
                let predates_boot = self.boot_time.is_some_and(|bt| {
                    crate::db::parse_timestamp(&row.last_event_at).is_some_and(|t| {
                        t < bt - time::Duration::seconds(liveness::BOOT_TOLERANCE_SECS)
                    })
                });
                let other_boot = row
                    .boot_id
                    .as_deref()
                    .is_some_and(|b| !liveness::same_boot(b, &self.boot));
                let rebooted = predates_boot || other_boot;
                let act = if row.tracking == "discovered" {
                    if rebooted {
                        Act::CloseDiscovered
                    } else {
                        Act::Leave
                    }
                } else if rebooted {
                    Act::Close(InferredEnd::Reboot)
                } else if row.launch_mode != "external" {
                    Act::Close(InferredEnd::Restart)
                } else {
                    match (row.pid, row.boot_id.is_some()) {
                        (Some(pid), true) => {
                            let pid = u32::try_from(pid).unwrap_or(0);
                            if pid != 0 && alive(pid) {
                                Act::Reset { track: Some(pid) }
                            } else {
                                Act::Close(InferredEnd::Restart)
                            }
                        }
                        _ => Act::Reset { track: None },
                    }
                };
                (row.id.clone(), act)
            })
            .collect();
        let mut report = Reconciled::default();
        let mut tracked = Vec::new();
        for (id, act) in &acts {
            if let Act::Reset { track: Some(pid) } = act {
                tracked.push((id.clone(), *pid));
            }
        }
        let now_s = now.to_string();
        let (closed, reset) = self.db.write(move |c| {
            let (mut closed, mut reset) = (0usize, 0usize);
            repo::tx(c, |tx| {
                for (id, act) in &acts {
                    match act {
                        Act::Close(why) => {
                            if super::writes::close_inferred_as(tx, id, &now_s, *why)?.is_some() {
                                closed += 1;
                            }
                        }
                        Act::CloseDiscovered => {
                            closed += tx.execute(
                                "UPDATE sessions SET ended_at = last_event_at, end_reason = 'inferred',
                                        state_reason = 'reboot:inferred', updated_at = ?1
                                  WHERE id = ?2 AND ended_at IS NULL",
                                params![now_s, id],
                            )?;
                        }
                        Act::Reset { .. } => {
                            if super::writes::reset_after_restart(tx, id, &now_s)? {
                                reset += 1;
                            }
                        }
                        Act::Leave => {}
                    }
                }
                Ok(())
            })?;
            Ok((closed, reset))
        })?;
        {
            let mut processes = lock(&self.processes);
            for (id, pid) in &tracked {
                processes.record(id, Some(*pid), None);
            }
        }
        report.closed = closed;
        report.reset = reset;
        report.tracked = tracked.len();
        if closed + reset > 0 {
            report.pushes = vec![push::Event::Sessions, push::Event::Pill];
        }
        Ok(report)
    }

    /// Every 15 s (05 §9.1): sessions whose `claude` process is gone get an
    /// inferred end; relay rows of live sessions go `failed` after 5 minutes
    /// without a frame (05 §6A.1).
    pub fn probe_liveness(&self) -> Result<Vec<push::Event>> {
        self.probe_liveness_with(liveness::pid_alive)
    }

    pub fn probe_liveness_with(&self, alive: impl Fn(u32) -> bool) -> Result<Vec<push::Event>> {
        let dead = lock(&self.processes).reap(alive);
        for id in &dead {
            lock(&self.always).clear(id);
            lock(&self.queued).remove(id);
        }
        let changed = self.db.write(move |c| {
            let now = crate::db::now_rfc3339();
            let mut changed = false;
            repo::tx(c, |tx| {
                for id in &dead {
                    if writes::close_inferred(tx, id, &now)?.is_some() {
                        changed = true;
                    }
                }
                let stale = crate::db::format_timestamp(
                    time::OffsetDateTime::now_utc() - time::Duration::seconds(RELAY_FRESH_SECS),
                );
                let n = tx.execute(
                    "UPDATE session_connectors SET status = 'failed'
                      WHERE kind = 'relay' AND status = 'connected' AND seen_at < ?
                        AND session_id IN (SELECT id FROM sessions WHERE ended_at IS NULL AND tracking <> 'discovered')",
                    [stale],
                )?;
                Ok(changed || n > 0)
            })
        })?;
        Ok(if changed {
            vec![push::Event::Sessions, push::Event::Pill]
        } else {
            Vec::new()
        })
    }

    // -------------------------------------------------------------- views

    /// What jump to terminal needs about a session (05 §9.3): the row's
    /// launch mode, tracking, state, `cwd` and terminal columns (filled from
    /// the first frame's allow-listed environment and tty, 05 §9.1–§9.2),
    /// plus the live process the relay reported. A subagent row answers with
    /// its parent's terminal. `None` for an unknown id.
    pub fn jump_info(&self, session_id: &str) -> Result<Option<JumpInfo>> {
        let sid = session_id.to_string();
        let Some(mut row) = self.db.read(|c| repo::get_by_id::<Session>(c, &sid))? else {
            return Ok(None);
        };
        if let Some(parent) = row.parent_session_id.clone()
            && let Some(p) = self.db.read(|c| repo::get_by_id::<Session>(c, &parent))?
        {
            row = p;
        }
        let process = lock(&self.processes).get(&row.id).cloned();
        let tty = process.as_ref().and_then(|p| p.tty.clone()).or_else(|| {
            row.terminal_ref
                .as_deref()
                .and_then(|r| r.strip_prefix("terminal.app:"))
                .filter(|t| t.starts_with("/dev/"))
                .map(str::to_string)
        });
        Ok(Some(JumpInfo {
            session_id: row.id,
            launch_mode: row.launch_mode,
            tracking: row.tracking,
            state: row.state,
            cwd: row.cwd,
            terminal_app: row.terminal_app,
            terminal_ref: row.terminal_ref,
            pid: process.map(|p| p.pid),
            tty,
            ended: row.ended_at.is_some(),
        }))
    }

    /// The `sessions.changed` rows.
    pub fn roster(&self) -> Result<Vec<SessionItem>> {
        self.db.read(|c| {
            let clock = Clock::current(c)?;
            let fold = settings::get_i64_in(c, "sessions.fold_after_minutes")?.unwrap_or(10);
            views::roster(c, &clock, fold)
        })
    }

    /// The `pill.changed` payload.
    pub fn pill(&self) -> Result<PillPayload> {
        self.db.read(|c| {
            let clock = Clock::current(c)?;
            let max_age =
                settings::get_i64_in(c, "urgency.pill_message_max_age_hours")?.unwrap_or(24);
            views::pill(c, &clock, max_age)
        })
    }

    /// The payload of a push this engine owns (`sessions.changed`,
    /// `pill.changed`); `None` for the other areas.
    pub fn payload(&self, event: push::Event) -> Result<Option<Value>> {
        Ok(match event {
            push::Event::Sessions => Some(serde_json::json!({ "rows": self.roster()? })),
            push::Event::Pill => Some(serde_json::to_value(self.pill()?)?),
            _ => None,
        })
    }

    /// Session detail with the `always` rules in memory.
    pub fn detail(&self, session_id: &str) -> Result<Option<SessionDetail>> {
        let sid = session_id.to_string();
        let pids: HashMap<String, u32> = {
            let p = lock(&self.processes);
            p.get(&sid)
                .map(|r| (sid.clone(), r.pid))
                .into_iter()
                .collect()
        };
        let mut detail = self.db.read(|c| {
            views::detail(c, &sid, views::TIMELINE_PAGE, |s| {
                pids.get(&s.id).is_none_or(|pid| liveness::pid_alive(*pid))
            })
        })?;
        if let Some(d) = &mut detail {
            d.driver = self.embedded.driver(&sid);
            d.connectors_source = super::embedded::connectors_source(&d.connectors);
        }
        Ok(detail)
    }

    // ------------------------------------------------- history and snooze

    /// The History rows of the Sessions view (03 §3.3; dismissed rows within
    /// `retention.sessions_days`).
    pub fn history(&self, q: &history::HistoryQuery) -> Result<Vec<history::HistoryItem>> {
        self.db.read(|c| {
            let clock = Clock::current(c)?;
            let fold = settings::get_i64_in(c, "sessions.fold_after_minutes")?.unwrap_or(10);
            let days = settings::get_i64_in(c, "retention.sessions_days")?.unwrap_or(90);
            history::history(c, &clock, fold, days, q)
        })
    }

    /// Undo a dismiss (05 §12): the row returns to the roster.
    pub fn undismiss(&self, session_id: &str) -> Result<Vec<push::Event>> {
        let sid = session_id.to_string();
        let changed = self
            .db
            .write(move |c| history::undismiss(c, &sid, &crate::db::now_rfc3339()))?;
        Ok(if changed {
            vec![push::Event::Sessions, push::Event::Pill]
        } else {
            Vec::new()
        })
    }

    /// The launch dialog's recent folders (03 §3.5).
    pub fn recent_folders(&self, limit: i64) -> Result<Vec<String>> {
        self.db.read(|c| history::recent_folders(c, limit))
    }

    /// Snooze a session (03 §3.4) until `until` (UTC) or a preset's time;
    /// both `None` cancels the active snooze (the Undo). Returns the snooze
    /// end, `None` when canceled.
    pub fn snooze(
        &self,
        session_id: &str,
        preset: Option<&str>,
        until: Option<&str>,
    ) -> Result<(Option<String>, Vec<push::Event>)> {
        let sid = session_id.to_string();
        let preset = preset.map(str::to_string);
        let until = until.map(str::to_string);
        let device = self.db.device_id().to_string();
        let end = self.db.write(move |c| {
            let now = crate::db::now_rfc3339();
            let end = match (&until, &preset) {
                (None, None) => {
                    let exists: Option<i64> = c
                        .query_row("SELECT 1 FROM sessions WHERE id = ?", [&sid], |r| r.get(0))
                        .optional()?;
                    if exists.is_none() {
                        return Err(DbError::NotFound {
                            table: "sessions",
                            key: sid,
                        });
                    }
                    history::cancel_snooze(c, &device, &sid)?;
                    return Ok(None);
                }
                (Some(u), _) => {
                    let at = crate::db::parse_timestamp(u).ok_or_else(|| DbError::Invalid {
                        what: "until",
                        detail: format!("{u:?} is not a timestamp"),
                    })?;
                    let at = crate::db::format_timestamp(at);
                    if at <= now {
                        return Err(DbError::Invalid {
                            what: "until",
                            detail: "the snooze must end in the future".into(),
                        });
                    }
                    at
                }
                (None, Some(p)) => {
                    history::preset_until(c, &now, p)?.ok_or_else(|| DbError::Invalid {
                        what: "preset",
                        detail: format!("{p:?} is not one of {}", history::PRESETS.join(", ")),
                    })?
                }
            };
            let preset = preset.as_deref().or(until.as_ref().map(|_| "pick_date"));
            if let Some(p) = preset
                && !history::PRESETS.contains(&p)
                && !matches!(p, "pick_date" | "until_reply")
            {
                return Err(DbError::Invalid {
                    what: "preset",
                    detail: format!("{p:?} is not a snooze preset"),
                });
            }
            repo::tx(c, |tx| history::snooze(tx, &device, &sid, &end, preset))?;
            Ok(Some(end))
        })?;
        Ok((end, vec![push::Event::Sessions, push::Event::Pill]))
    }

    /// The time sweep of snoozes (S2): fire what is due; the areas to push.
    pub fn fire_due_snoozes(&self) -> Result<Vec<push::Event>> {
        let device = self.db.device_id().to_string();
        let fired = self.db.write(move |c| {
            let clock = Clock::current(c)?;
            history::fire_due(c, &device, &clock)
        })?;
        let mut out: Vec<push::Event> = Vec::new();
        for f in &fired {
            let events: &[push::Event] = match f.target_kind.as_str() {
                "session" => &[push::Event::Sessions, push::Event::Pill],
                "note" => &[push::Event::Notes],
                "message" | "thread" => &[push::Event::Inbox, push::Event::Pill],
                "calendar_event" => &[push::Event::Agenda, push::Event::Pill],
                _ => &[],
            };
            for e in events {
                if !out.contains(e) {
                    out.push(*e);
                }
            }
        }
        Ok(out)
    }
}
