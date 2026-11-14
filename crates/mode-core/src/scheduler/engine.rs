//! The scheduler (03 §7B.4, 08 §3.2 `scheduler`): the one-minute tick over
//! S8 (`automations.next_run_at`), recomputed in `schedule_json.tz` after
//! every run; the catch-up window at launch and on wake
//! (`automations.catch_up_minutes`, 30: a firing missed by less runs at once,
//! older misses become one `skipped` run); one running run per automation;
//! Run now; `automation_runs` `running` → `finished` / `failed`; pruning per
//! `keep_runs`; the three actions dispatched through [`Dispatch`] to
//! `sessions` (launch + prompt), `notes` (create from a template / pattern)
//! and `chat` (one turn saved as a note).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::mpsc;

use super::actions::{Action, AskChat, CreateNote, StartSession};
use super::schedule::{self, Schedule};
use super::store::{self, AutomationRow, RunRow};
use crate::db::{Db, DbError, Result, format_timestamp, parse_timestamp};
use crate::push::Event;
use crate::repo::Automation;
use crate::settings;

/// Where an automation's work goes. The app's implementation is
/// [`super::dispatch::LiveDispatch`]; tests use stubs.
pub trait Dispatch: Send + Sync {
    /// A skip reason when the action cannot run now (03 §7B.3: `claude` not
    /// found; local-only with no local backend; the monthly AI budget).
    fn preflight(&self, action: &Action) -> Option<String>;
    /// Launch per 03 §3.5 and send the prompt as the first turn; the session id.
    fn start_session(
        &self,
        a: &StartSession,
        ctx: &RunContext,
    ) -> std::result::Result<String, String>;
    /// SIGINT the session's turn (the token cap).
    fn stop_session(&self, session_id: &str) -> std::result::Result<(), String>;
    /// Write the note; its id.
    fn create_note(&self, a: &CreateNote, ctx: &RunContext) -> std::result::Result<String, String>;
    /// One Chat turn saved as a note; the note's id.
    fn ask_chat(&self, a: &AskChat, ctx: &RunContext) -> std::result::Result<String, String>;
}

/// What a dispatched action knows about its run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunContext {
    pub automation_id: String,
    pub automation_name: String,
    pub run_id: String,
    pub fired_at: OffsetDateTime,
    /// The schedule's IANA zone (titles and dates are rendered in it).
    pub tz: String,
}

impl RunContext {
    /// The fire time as wall time in the schedule's zone.
    pub fn local(&self) -> jiff::civil::DateTime {
        let tz = jiff::tz::TimeZone::get(&self.tz).unwrap_or(jiff::tz::TimeZone::UTC);
        jiff::Timestamp::from_second(self.fired_at.unix_timestamp())
            .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
            .to_zoned(tz)
            .datetime()
    }
}

/// A failed run with `notify_on_failure` on: the app raises the
/// `automation.failed` alert (07 §8.1, premade rule "Automation failed").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedRun {
    pub automation_id: String,
    pub automation_name: String,
    pub run_id: String,
    pub error: String,
}

/// Why the run loop wakes early.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Wake {
    /// The OS woke from sleep: catch up.
    Resumed,
    /// An automation or `automations.catch_up_minutes` changed.
    Changed,
}

/// What one tick did (tests and diagnostics).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickReport {
    /// Runs created in `running`.
    pub fired: Vec<String>,
    /// Runs created in `skipped`.
    pub skipped: Vec<String>,
    /// Runs moved to `finished` / `failed`.
    pub finished: Vec<String>,
}

impl TickReport {
    pub fn is_empty(&self) -> bool {
        self.fired.is_empty() && self.skipped.is_empty() && self.finished.is_empty()
    }
}

pub type Clock = Arc<dyn Fn() -> OffsetDateTime + Send + Sync>;
type PublishFn = Arc<dyn Fn(Event) + Send + Sync>;
type FailureFn = Arc<dyn Fn(&FailedRun) + Send + Sync>;

/// Options for [`Scheduler::new`].
#[derive(Clone, Default)]
pub struct SchedulerOptions {
    /// A fixed clock (tests).
    pub clock: Option<Clock>,
    /// The device's IANA zone; default the OS zone.
    pub device_tz: Option<String>,
    /// Run actions on the calling thread (tests) instead of a worker thread.
    pub inline: bool,
}

pub struct Scheduler {
    /// A weak handle to itself for worker threads.
    me: std::sync::Weak<Scheduler>,
    db: Arc<Db>,
    dispatch: RwLock<Arc<dyn Dispatch>>,
    clock: Option<Clock>,
    device_tz: String,
    inline: bool,
    publish: Mutex<Option<PublishFn>>,
    failure: Mutex<Option<FailureFn>>,
    /// Runs whose action is executing on a worker thread.
    workers: Mutex<HashSet<String>>,
    /// Quit: nothing fires any more.
    stopping: AtomicBool,
}

impl std::fmt::Debug for Scheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scheduler")
            .field("device_tz", &self.device_tz)
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

enum Outcome {
    /// The run is over with this result.
    Done(&'static str, String),
    /// The run keeps running until the session finishes.
    Session(String),
}

impl Scheduler {
    pub fn new(db: Arc<Db>, dispatch: Arc<dyn Dispatch>, opts: SchedulerOptions) -> Arc<Scheduler> {
        Arc::new_cyclic(|me| Scheduler {
            me: me.clone(),
            db,
            dispatch: RwLock::new(dispatch),
            clock: opts.clock,
            device_tz: opts.device_tz.unwrap_or_else(schedule::device_zone),
            inline: opts.inline,
            publish: Mutex::new(None),
            failure: Mutex::new(None),
            workers: Mutex::new(HashSet::new()),
            stopping: AtomicBool::new(false),
        })
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    pub fn device_tz(&self) -> &str {
        &self.device_tz
    }

    pub fn now(&self) -> OffsetDateTime {
        match &self.clock {
            Some(c) => c(),
            None => OffsetDateTime::now_utc(),
        }
    }

    pub fn dispatch(&self) -> Arc<dyn Dispatch> {
        Arc::clone(&self.dispatch.read().unwrap_or_else(|p| p.into_inner()))
    }

    /// Swap the dispatcher (the app builds Chat after the scheduler).
    pub fn set_dispatch(&self, d: Arc<dyn Dispatch>) {
        *self.dispatch.write().unwrap_or_else(|p| p.into_inner()) = d;
    }

    /// Where `automations.changed` requests go.
    pub fn set_publisher(&self, f: impl Fn(Event) + Send + Sync + 'static) {
        *lock(&self.publish) = Some(Arc::new(f));
    }

    /// Where failed runs with `notify_on_failure` go (the alert pipeline).
    pub fn set_failure_sink(&self, f: impl Fn(&FailedRun) + Send + Sync + 'static) {
        *lock(&self.failure) = Some(Arc::new(f));
    }

    pub fn changed(&self) {
        let f = lock(&self.publish).clone();
        if let Some(f) = f {
            f(Event::Automations);
        }
    }

    /// The `automations.changed` payload: the list rows (with `problem`).
    pub fn payload(&self) -> Option<serde_json::Value> {
        self.rows().ok().and_then(|l| serde_json::to_value(l).ok())
    }

    /// The list rows in position order, each with its `problem`.
    pub fn rows(&self) -> Result<Vec<AutomationRow>> {
        let tz = self.device_tz.clone();
        let mut rows = self.db.read(|c| store::list(c, &tz))?;
        self.fill_problems(&mut rows);
        Ok(rows)
    }

    /// One row by id (live automations only), with its `problem`.
    pub fn row(&self, id: &str) -> Result<Option<AutomationRow>> {
        let (tz, id) = (self.device_tz.clone(), id.to_string());
        let row = self.db.read(move |c| match store::load(c, &id)? {
            Some(a) => Ok(Some(store::row(c, &a, &tz)?)),
            None => Ok(None),
        })?;
        Ok(row.map(|mut r| {
            self.fill_problems(std::slice::from_mut(&mut r));
            r
        }))
    }

    /// `problem` from the dispatcher's preflight (03 §7B.7): `claude` not
    /// found, no AI backend for *Ask Chat*, the notes folder unavailable.
    /// The store already set `folder_missing` for a missing folder or cwd.
    pub fn fill_problems(&self, rows: &mut [AutomationRow]) {
        let d = self.dispatch();
        let mut memo: HashMap<String, Option<String>> = HashMap::new();
        for r in rows.iter_mut().filter(|r| r.problem.is_none()) {
            let Ok(action) = Action::parse(&r.action_kind, &r.action) else {
                continue;
            };
            let key = match &action {
                Action::StartSession(_) => "start_session".to_string(),
                Action::CreateNote(_) => "create_note".to_string(),
                Action::AskChat(a) => format!("ask_chat:{}", a.effort),
            };
            let reason = memo
                .entry(key)
                .or_insert_with(|| d.preflight(&action))
                .clone();
            r.problem = reason.as_deref().and_then(problem_code).map(str::to_string);
        }
    }

    /// Quit: stop firing, and mark the runs whose note or Chat turn is still
    /// executing `failed` (a session run keeps being watched next launch).
    pub fn shutdown(&self) -> usize {
        self.stopping.store(true, Ordering::SeqCst);
        let busy: Vec<String> = lock(&self.workers).drain().collect();
        if busy.is_empty() {
            return 0;
        }
        let now = format_timestamp(self.now());
        let n = self
            .db
            .write(move |c| {
                let mut n = 0;
                for id in &busy {
                    if store::finish_run(c, id, "failed", None, Some(QUIT_REASON), &now)? {
                        n += 1;
                    }
                }
                Ok(n)
            })
            .unwrap_or(0);
        if n > 0 {
            self.changed();
        }
        n
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    fn catch_up(&self) -> Result<time::Duration> {
        let m = self
            .db
            .read(|c| settings::get_i64_in(c, "automations.catch_up_minutes"))?
            .filter(|m| *m >= 0)
            .unwrap_or(30);
        Ok(time::Duration::minutes(m))
    }

    /// Runs left `running` by a previous process whose work cannot be
    /// resumed (a note or a Chat turn) become `failed`; session runs keep
    /// being watched.
    pub fn recover(&self) -> Result<usize> {
        let now = format_timestamp(self.now());
        let busy: Vec<String> = lock(&self.workers).iter().cloned().collect();
        let n = self.db.write(move |c| {
            let ids: Vec<String> = c
                .prepare(
                    "SELECT id FROM automation_runs
                      WHERE state = 'running' AND (result_kind IS NULL OR result_kind <> 'session')",
                )?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let mut n = 0;
            for id in ids.iter().filter(|i| !busy.contains(i)) {
                if store::finish_run(c, id, "failed", None, Some(QUIT_REASON), &now)? {
                    n += 1;
                }
            }
            Ok(n)
        })?;
        if n > 0 {
            self.changed();
        }
        Ok(n)
    }

    /// One tick at the current time.
    pub fn tick(&self) -> Result<TickReport> {
        self.tick_at(self.now())
    }

    /// Watch session runs, then fire what is due (S8) with the catch-up
    /// rule. Launch and wake call this too: the window applies to every
    /// late firing, however it came to be late.
    pub fn tick_at(&self, now: OffsetDateTime) -> Result<TickReport> {
        let mut report = TickReport::default();
        if self.is_stopping() {
            return Ok(report);
        }
        self.monitor_sessions(now, &mut report)?;
        let now_s = format_timestamp(now);
        let due: Vec<Automation> = self.db.read(|c| {
            crate::repo::query::<Automation>(
                c,
                "enabled = 1 AND deleted_at IS NULL AND next_run_at IS NOT NULL AND next_run_at <= ?1
                 ORDER BY next_run_at, position",
                &[&now_s],
            )
        })?;
        let window = self.catch_up()?;
        for a in due {
            self.process_due(&a, now, window, &mut report)?;
        }
        if !report.is_empty() {
            self.changed();
        }
        Ok(report)
    }

    fn process_due(
        &self,
        a: &Automation,
        now: OffsetDateTime,
        window: time::Duration,
        report: &mut TickReport,
    ) -> Result<()> {
        let now_s = format_timestamp(now);
        let sched: Schedule = store::schedule_of(a);
        let plan = match sched.plan(&self.device_tz) {
            Ok(p) => p,
            Err(e) => {
                let (id, msg) = (
                    a.id.clone(),
                    format!("the schedule is invalid: {}", e.message),
                );
                let n2 = now_s.clone();
                let run = self.db.write(move |c| {
                    store::set_times(c, &id, Some(&n2), None)?;
                    store::insert_run(
                        c,
                        &store::NewRun::ended(&id, "schedule", "failed", &n2, &msg),
                        &n2,
                    )
                })?;
                report.finished.push(run.id);
                return Ok(());
            }
        };
        let Some(first) = a.next_run_at.as_deref().and_then(parse_timestamp) else {
            return Ok(());
        };
        let mut missed = vec![first];
        missed.extend(plan.between(first, now, 10_000));
        let latest = *missed.last().unwrap_or(&first);
        let in_window = now - latest <= window;
        let older = if in_window {
            missed.len() - 1
        } else {
            missed.len()
        };
        let next = plan.next_after(now).map(format_timestamp);
        let (id, n2) = (a.id.clone(), now_s.clone());
        let skipped = self.db.write(move |c| {
            let skipped = if older > 0 {
                let reason = if older == 1 {
                    SKIP_CLOSED_ONE.to_string()
                } else {
                    format!("{} ({older} missed)", store::SKIP_CLOSED)
                };
                let first = format_timestamp(missed[0]);
                Some(store::insert_run(
                    c,
                    &store::NewRun {
                        automation_id: &id,
                        trigger_kind: "schedule",
                        state: "skipped",
                        started_at: &first,
                        finished_at: Some(&n2),
                        error: Some(&reason),
                    },
                    &n2,
                )?)
            } else {
                None
            };
            store::set_times(c, &id, Some(&n2), next.as_deref())?;
            Ok(skipped)
        })?;
        if let Some(s) = skipped {
            report.skipped.push(s.id);
        }
        if in_window {
            let run = self.fire(a, "schedule", now)?;
            match run.state.as_str() {
                "skipped" => report.skipped.push(run.id),
                "running" => report.fired.push(run.id),
                _ => report.finished.push(run.id),
            }
        }
        Ok(())
    }

    /// Create the run and dispatch the action (03 §7B.4 "Firing"). A run is
    /// `skipped` when another run of the automation is still running or the
    /// dispatcher reports a skip reason.
    pub fn fire(&self, a: &Automation, trigger_kind: &str, now: OffsetDateTime) -> Result<RunRow> {
        let now_s = format_timestamp(now);
        let action_json: serde_json::Value =
            serde_json::from_str(&a.action_json).unwrap_or(serde_json::Value::Null);
        let action = Action::parse(&a.action_kind, &action_json);
        let reason = match &action {
            Ok(act) => self.dispatch().preflight(act),
            Err(e) => Some(format!("the action is invalid: {}", e.message)),
        };
        let (id, trig, n2) = (a.id.clone(), trigger_kind.to_string(), now_s.clone());
        let enabled = a.enabled;
        let run = self.db.write(move |c| {
            c.execute_batch("SAVEPOINT fire")?;
            let out = (|| {
                let skip = |why: &str| {
                    store::insert_run(
                        c,
                        &store::NewRun::ended(&id, &trig, "skipped", &n2, why),
                        &n2,
                    )
                };
                if store::has_running(c, &id)? {
                    return skip(store::SKIP_RUNNING);
                }
                if trig == "schedule" && !enabled {
                    return skip(store::SKIP_DISABLED);
                }
                if let Some(r) = &reason {
                    return skip(r);
                }
                store::insert_run(
                    c,
                    &store::NewRun {
                        automation_id: &id,
                        trigger_kind: &trig,
                        state: "running",
                        started_at: &n2,
                        finished_at: None,
                        error: None,
                    },
                    &n2,
                )
            })();
            match out {
                Ok(r) => {
                    c.execute_batch("RELEASE fire")?;
                    Ok(r)
                }
                Err(e) => {
                    let _ = c.execute_batch("ROLLBACK TO fire; RELEASE fire");
                    Err(e)
                }
            }
        })?;
        let row = RunRow::from_row(&run);
        if run.state != "running" {
            self.after_run(a, &row)?;
            return Ok(row);
        }
        if self.is_stopping() {
            let (id, now_s) = (run.id.clone(), format_timestamp(now));
            self.db.write(move |c| {
                store::finish_run(c, &id, "failed", None, Some(QUIT_REASON), &now_s)
            })?;
            return Ok(RunRow {
                state: "failed".into(),
                error: Some(QUIT_REASON.into()),
                ..row
            });
        }
        let Ok(action) = action else {
            return Ok(row);
        };
        let ctx = RunContext {
            automation_id: a.id.clone(),
            automation_name: a.name.clone(),
            run_id: run.id.clone(),
            fired_at: now,
            tz: store::schedule_of(a)
                .tz
                .unwrap_or_else(|| self.device_tz.clone()),
        };
        if self.inline {
            self.execute(a.clone(), action, ctx);
        } else {
            lock(&self.workers).insert(run.id.clone());
            let me = self.me.clone();
            let a = a.clone();
            std::thread::Builder::new()
                .name("mode-automation".into())
                .spawn(move || {
                    let run_id = ctx.run_id.clone();
                    if let Some(s) = me.upgrade() {
                        s.execute(a, action, ctx);
                        lock(&s.workers).remove(&run_id);
                    }
                })
                .map_err(DbError::Io)?;
        }
        let id = run.id.clone();
        Ok(self
            .db
            .read(move |c| match store::run_by_id(c, &id)? {
                Some(r) => Ok(Some(RunRow::load(c, &r)?)),
                None => Ok(None),
            })?
            .unwrap_or(row))
    }

    fn execute(&self, a: Automation, action: Action, ctx: RunContext) {
        let d = self.dispatch();
        let out = match &action {
            Action::StartSession(s) => d.start_session(s, &ctx).map(Outcome::Session),
            Action::CreateNote(n) => d.create_note(n, &ctx).map(|id| Outcome::Done("note", id)),
            Action::AskChat(q) => d.ask_chat(q, &ctx).map(|id| Outcome::Done("note", id)),
        };
        let now = format_timestamp(self.now());
        let run_id = ctx.run_id.clone();
        let res = self.db.write(move |c| {
            match out {
                Ok(Outcome::Session(sid)) => {
                    store::set_run_result(c, &run_id, "session", &sid)?;
                }
                Ok(Outcome::Done(kind, id)) => {
                    store::finish_run(c, &run_id, "finished", Some((kind, &id)), None, &now)?;
                }
                Err(e) => {
                    store::finish_run(c, &run_id, "failed", None, Some(&e), &now)?;
                }
            }
            store::run_by_id(c, &run_id)
        });
        match res {
            Ok(Some(run)) => {
                let row = RunRow::from_row(&run);
                if row.state != "running" {
                    let _ = self.after_run(&a, &row);
                }
                self.changed();
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("automation run {}: {e}", ctx.run_id),
        }
    }

    /// Prune, raise the failure alert, publish.
    fn after_run(&self, a: &Automation, run: &RunRow) -> Result<()> {
        let (id, keep, now) = (a.id.clone(), a.keep_runs, self.now());
        self.db.write(move |c| store::prune(c, &id, keep, now))?;
        if run.state == "failed" && a.notify_on_failure {
            let f = lock(&self.failure).clone();
            if let Some(f) = f {
                f(&FailedRun {
                    automation_id: a.id.clone(),
                    automation_name: a.name.clone(),
                    run_id: run.id.clone(),
                    error: run.error.clone().unwrap_or_default(),
                });
            }
        }
        Ok(())
    }

    /// Session runs: finished when the session's turn is over, failed when
    /// the session failed, stopped with SIGINT past the token cap.
    fn monitor_sessions(&self, now: OffsetDateTime, report: &mut TickReport) -> Result<()> {
        type Watched = (
            String,
            String,
            Option<i64>,
            Option<(String, Option<String>, i64, i64, bool)>,
        );
        let rows: Vec<Watched> = self.db.read(|c| {
            let mut stmt = c.prepare(
                "SELECT r.id, r.automation_id, a.action_json,
                        s.state, s.state_reason,
                        s.input_tokens + s.output_tokens + s.cache_read_tokens + s.cache_write_tokens,
                        s.num_turns, s.ended_at IS NOT NULL, s.id IS NOT NULL
                   FROM automation_runs r
                   JOIN automations a ON a.id = r.automation_id
                   LEFT JOIN sessions s ON s.id = r.result_id
                  WHERE r.state = 'running' AND r.result_kind = 'session'",
            )?;
            let out = stmt
                .query_map([], |r| {
                    let action: String = r.get(2)?;
                    let cap = serde_json::from_str::<serde_json::Value>(&action)
                        .ok()
                        .and_then(|v| v.get("token_cap").and_then(serde_json::Value::as_i64));
                    let present: bool = r.get(8)?;
                    let session = if present {
                        Some((r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
                    } else {
                        None
                    };
                    Ok((r.get(0)?, r.get(1)?, cap, session))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(out)
        })?;
        for (run_id, automation_id, cap, session) in rows {
            let verdict: Option<(&str, Option<String>)> = match session {
                None => Some(("finished", None)),
                Some((state, reason, tokens, turns, ended)) => {
                    if let Some(cap) = cap.filter(|c| tokens > *c) {
                        let sid = self.db.read(|c| {
                            Ok(store::run_by_id(c, &run_id)?.and_then(|r| r.result_id))
                        })?;
                        if let Some(sid) = sid {
                            let _ = self.dispatch().stop_session(&sid);
                        }
                        Some((
                            "failed",
                            Some(format!("stopped at the token cap of {cap} tokens")),
                        ))
                    } else if state == "failed" {
                        Some((
                            "failed",
                            Some(reason.unwrap_or_else(|| "the session failed".into())),
                        ))
                    } else if ended
                        || state == "unread"
                        || state == "done"
                        || (state == "idle" && turns > 0)
                    {
                        Some(("finished", None))
                    } else {
                        None
                    }
                }
            };
            let Some((state, error)) = verdict else {
                continue;
            };
            let now_s = format_timestamp(now);
            let (rid, st) = (run_id.clone(), state.to_string());
            let done = self.db.write(move |c| {
                store::finish_run(c, &rid, &st, None, error.as_deref(), &now_s)?;
                Ok((store::run_by_id(c, &rid)?, store::load(c, &automation_id)?))
            })?;
            if let (Some(run), Some(a)) = done {
                self.after_run(&a, &RunRow::from_row(&run))?;
            }
            report.finished.push(run_id);
        }
        Ok(())
    }

    /// Run now (03 §7B.4): a `manual` run regardless of `enabled`; refused
    /// while a run of the automation is running.
    pub fn run_now(&self, id: &str) -> std::result::Result<RunRow, RunNowError> {
        let key = id.to_string();
        let a = self
            .db
            .read(move |c| store::load(c, &key))?
            .ok_or(RunNowError::NotFound)?;
        let key = id.to_string();
        if self.db.read(move |c| store::has_running(c, &key))? {
            return Err(RunNowError::Busy);
        }
        let run = self.fire(&a, "manual", self.now())?;
        let (aid, now) = (a.id.clone(), format_timestamp(self.now()));
        self.db.write(move |c| {
            c.execute(
                "UPDATE automations SET last_run_at = ?2 WHERE id = ?1",
                rusqlite::params![aid, now],
            )?;
            Ok(())
        })?;
        self.changed();
        Ok(run)
    }

    /// The background task: catch up at launch, then tick at the top of
    /// every minute or when woken (OS wake, an automation saved).
    pub async fn run(self: Arc<Self>, mut wake: mpsc::UnboundedReceiver<Wake>) {
        let me = Arc::clone(&self);
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(e) = me.recover() {
                tracing::warn!("scheduler recover: {e}");
            }
            if let Err(e) = me.tick() {
                tracing::warn!("scheduler launch catch-up: {e}");
            }
        })
        .await;
        loop {
            let now = self.now();
            let secs = 60 - u64::from(now.second()).min(59);
            let sleep = Duration::from_secs(secs) + Duration::from_millis(50);
            tokio::select! {
                _ = tokio::time::sleep(sleep) => {}
                w = wake.recv() => if w.is_none() { return },
            }
            let me = Arc::clone(&self);
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(e) = me.tick() {
                    tracing::warn!("scheduler tick: {e}");
                }
            })
            .await;
        }
    }
}

const SKIP_CLOSED_ONE: &str = store::SKIP_CLOSED;
/// The failure of a run cut short by quitting.
pub const QUIT_REASON: &str = "Mode quit before the run finished";

/// The 03 §7B.7 problem word for a preflight skip reason.
pub fn problem_code(reason: &str) -> Option<&'static str> {
    match reason {
        super::dispatch::CLAUDE_MISSING => Some("claude_not_found"),
        crate::chat::NO_BACKEND | crate::chat::LOCAL_ONLY_BLOCKED | crate::chat::BUDGET_REACHED => {
            Some("no_ai_backend")
        }
        super::dispatch::NOTES_MISSING => Some("folder_missing"),
        _ => None,
    }
}

/// Run now's refusals.
#[derive(Debug, thiserror::Error)]
pub enum RunNowError {
    #[error("No automation with that id.")]
    NotFound,
    #[error("A run of this automation is still running.")]
    Busy,
    #[error(transparent)]
    Db(#[from] DbError),
}
