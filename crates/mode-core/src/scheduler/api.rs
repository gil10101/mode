//! One plain function per automations command (03 §7B; shapes per
//! `ui/src/state/contract-automations.ts`, errors `{code, message}`):
//!
//! | Command | Args | Result |
//! |---|---|---|
//! | `automations_list` | `{query?, disabled_only?, sort?: position\|name\|next_run\|last_run}` | `AutomationRow[]` |
//! | `automation_get` | `{id}` | `AutomationRow` |
//! | `automation_save` | `{draft}` | `AutomationRow`, or `{code: invalid, message, fields[]}` |
//! | `automation_delete` | `{id}` | `{deleted}` |
//! | `automation_set_enabled` | `{id, enabled}` | `AutomationRow` |
//! | `automation_run_now` | `{id}` | `RunRow` (`busy` while one runs) |
//! | `automation_runs` | `{id, limit?}` | `RunRow[]` newest first |
//! | `automation_run_get` | `{id}` | `RunDetail {run, automation}` (the run page) |
//! | `automation_duplicate` | `{id}` | `AutomationRow` ("<name> copy", disabled) |
//! | `automation_examples` | — | `AutomationDraft[]` (03 §7B.5 cards) |
//! | `schedule_preview` | `{schedule_json}` | `SchedulePreview {fires, fires_local, echo, error}` |
//!
//! Rows carry `problem` (03 §7B.7). Every mutation requests
//! `automations.changed`, whose payload is the rows.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::actions::FieldError;
use super::engine::{RunNowError, Scheduler};
use super::schedule::{self, Schedule, ScheduleError, ScheduleKind};
use super::store::{self, AutomationDraft, AutomationRow, RunRow};
use crate::db::{DbError, format_timestamp};
use crate::repo::{self, Automation};

pub use crate::notes::api::{ApiError, ApiResult};

fn err(code: &str, message: impl Into<String>) -> ApiError {
    ApiError {
        code: code.into(),
        message: message.into(),
    }
}

fn not_found(id: &str) -> ApiError {
    err("not_found", format!("No automation {id}."))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdArgs {
    pub id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListArgs {
    /// Name, trigger and action phrases.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub disabled_only: Option<bool>,
    /// `position` (default) | `name` | `next_run` | `last_run`.
    #[serde(default)]
    pub sort: Option<String>,
}

/// `automations_list` (03 §7B.1): disabled rows sort last ("Off" group).
pub fn automations_list(s: &Scheduler, args: ListArgs) -> ApiResult<Vec<AutomationRow>> {
    let mut rows = s.rows()?;
    if let Some(q) = args
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    {
        let q = q.to_lowercase();
        rows.retain(|r| {
            [&r.name, &r.trigger_phrase, &r.action_phrase]
                .iter()
                .any(|f| f.to_lowercase().contains(&q))
        });
    }
    if args.disabled_only.unwrap_or(false) {
        rows.retain(|r| !r.enabled);
    }
    match args.sort.as_deref() {
        Some("name") => rows.sort_by_key(|r| r.name.to_lowercase()),
        Some("next_run") => rows.sort_by(|a, b| {
            (a.next_run_at.is_none(), &a.next_run_at)
                .cmp(&(b.next_run_at.is_none(), &b.next_run_at))
        }),
        Some("last_run") => rows.sort_by(|a, b| b.last_run_at.cmp(&a.last_run_at)),
        _ => {}
    }
    rows.sort_by_key(|r| !r.enabled);
    Ok(rows)
}

/// `automation_get {id}`: the row (the detail panel loads its runs with
/// `automation_runs`).
pub fn automation_get(s: &Scheduler, args: IdArgs) -> ApiResult<AutomationRow> {
    s.row(&args.id)?.ok_or_else(|| not_found(&args.id))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveArgs {
    pub draft: AutomationDraft,
}

/// `automation_save`'s refusal (`SaveError`): `invalid` with the inline
/// field errors, or another `{code, message}` with no fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub fields: Vec<FieldError>,
}

impl From<ApiError> for SaveError {
    fn from(e: ApiError) -> Self {
        SaveError {
            code: e.code,
            message: e.message,
            fields: Vec::new(),
        }
    }
}

impl From<DbError> for SaveError {
    fn from(e: DbError) -> Self {
        ApiError::from(e).into()
    }
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `automation_save {draft}` (03 §7B.2 **Save**): validates, inserts or
/// updates, recomputes `next_run_at` in the schedule's zone; the saved row
/// (its `problem` says when `claude` or an AI backend is missing).
pub fn automation_save(s: &Scheduler, args: SaveArgs) -> Result<AutomationRow, SaveError> {
    let tz = s.device_tz().to_string();
    let draft = args.draft;
    let d2 = draft.clone();
    let checked = s.db().read(move |c| store::check_draft(c, &d2, &tz))?;
    let (sched, action) = match checked {
        Ok(v) => v,
        Err(fields) => {
            return Err(SaveError {
                code: "invalid".into(),
                message: fields
                    .first()
                    .map(|f| f.message.clone())
                    .unwrap_or_else(|| "Check the automation.".into()),
                fields,
            });
        }
    };
    let (device, now, tz) = (
        s.db().device_id().to_string(),
        s.now(),
        s.device_tz().to_string(),
    );
    let saved = s.db().write(move |c| {
        let a = store::save(c, &device, &draft, &sched, &action, now, &tz)?;
        store::row(c, &a, &tz)
    });
    let mut row = match saved {
        Ok(r) => r,
        Err(DbError::NotFound { key, .. }) => return Err(not_found(&key).into()),
        Err(e) => return Err(e.into()),
    };
    s.fill_problems(std::slice::from_mut(&mut row));
    s.changed();
    Ok(row)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResult {
    pub deleted: bool,
}

/// `automation_delete {id}`: tombstones it (its runs stay until the purge).
pub fn automation_delete(s: &Scheduler, args: IdArgs) -> ApiResult<DeleteResult> {
    let device = s.db().device_id().to_string();
    let id = args.id.clone();
    let deleted = s.db().write(move |c| {
        let ok = repo::soft_delete::<Automation>(c, &device, &id)?;
        if ok {
            store::set_times(c, &id, None, None)?;
        }
        Ok(ok)
    })?;
    if !deleted {
        return Err(not_found(&args.id));
    }
    s.changed();
    Ok(DeleteResult { deleted })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetEnabledArgs {
    pub id: String,
    pub enabled: bool,
}

/// `automation_set_enabled {id, enabled}`: the row's switch (undoable by
/// calling it again).
pub fn automation_set_enabled(s: &Scheduler, args: SetEnabledArgs) -> ApiResult<AutomationRow> {
    let (device, now, tz) = (
        s.db().device_id().to_string(),
        s.now(),
        s.device_tz().to_string(),
    );
    let id = args.id.clone();
    let row = s.db().write(move |c| {
        let Some(a) = store::load(c, &id)? else {
            return Ok(None);
        };
        let next = if args.enabled {
            store::schedule_of(&a)
                .plan(&tz)
                .ok()
                .and_then(|p| p.next_after(now))
                .map(format_timestamp)
        } else {
            None
        };
        repo::patch::<Automation>(
            c,
            &device,
            &id,
            &[("enabled", &args.enabled), ("next_run_at", &next)],
        )?;
        let a = store::load(c, &id)?.ok_or(crate::db::DbError::NotFound {
            table: "automations",
            key: id.clone(),
        })?;
        Ok(Some(store::row(c, &a, &tz)?))
    })?;
    let mut row = row.ok_or_else(|| not_found(&args.id))?;
    s.fill_problems(std::slice::from_mut(&mut row));
    s.changed();
    Ok(row)
}

/// `automation_run_now {id}` (03 §7B.4).
pub fn automation_run_now(s: &Scheduler, args: IdArgs) -> ApiResult<RunRow> {
    s.run_now(&args.id).map_err(|e| match e {
        RunNowError::NotFound => not_found(&args.id),
        RunNowError::Busy => err("busy", e.to_string()),
        RunNowError::Db(d) => d.into(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunsArgs {
    pub id: String,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// `automation_runs {id, limit?}` (03 §7B.3), newest first.
pub fn automation_runs(s: &Scheduler, args: RunsArgs) -> ApiResult<Vec<RunRow>> {
    let limit = args.limit.unwrap_or(100).clamp(1, 500);
    Ok(s.db().read(|c| store::runs(c, &args.id, limit))?)
}

/// The run page (03 §7B.3; `RunDetail {run, automation}`). The phrases,
/// the result's title and route are beyond the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunDetail {
    pub run: RunRow,
    /// The automation (a deleted one keeps its last row).
    pub automation: AutomationRow,
    #[serde(default)]
    pub automation_id: String,
    #[serde(default)]
    pub automation_name: String,
    #[serde(default)]
    pub trigger_phrase: String,
    #[serde(default)]
    pub action_phrase: String,
    /// The result's title ("mode", "Daily plan 14 Oct"); `None` once pruned.
    #[serde(default)]
    pub result_title: Option<String>,
    /// `sessions/<id>` or `notes/<id>`.
    #[serde(default)]
    pub result_route: Option<String>,
}

/// `automation_run_get {id}`.
pub fn automation_run_get(s: &Scheduler, args: IdArgs) -> ApiResult<RunDetail> {
    let tz = s.device_tz().to_string();
    let id = args.id.clone();
    let detail = s.db().read(move |c| {
        let Some(run) = store::run_by_id(c, &id)? else {
            return Ok(None);
        };
        let Some(a) = repo::get_by_id::<Automation>(c, &run.automation_id)? else {
            return Ok(None);
        };
        let automation = store::row(c, &a, &tz)?;
        let run = RunRow::load(c, &run)?;
        let result_route = match (run.result_kind.as_deref(), &run.result_id) {
            (Some("session"), Some(id)) => Some(format!("sessions/{id}")),
            (Some("note"), Some(id)) => Some(format!("notes/{id}")),
            _ => None,
        };
        Ok(Some(RunDetail {
            automation_id: automation.id.clone(),
            automation_name: automation.name.clone(),
            trigger_phrase: automation.trigger_phrase.clone(),
            action_phrase: automation.action_phrase.clone(),
            result_title: run.result_title.clone(),
            result_route,
            run,
            automation,
        }))
    })?;
    let mut d = detail.ok_or_else(|| err("not_found", format!("No run {}.", args.id)))?;
    s.fill_problems(std::slice::from_mut(&mut d.automation));
    Ok(d)
}

/// `automation_duplicate {id}`: "<name> copy", disabled (03 §7B.1).
pub fn automation_duplicate(s: &Scheduler, args: IdArgs) -> ApiResult<AutomationRow> {
    let (device, now, tz) = (
        s.db().device_id().to_string(),
        s.now(),
        s.device_tz().to_string(),
    );
    let id = args.id.clone();
    let row = s.db().write(move |c| {
        let Some(a) = store::load(c, &id)? else {
            return Ok(None);
        };
        let draft = AutomationDraft {
            id: None,
            name: format!("{} copy", a.name),
            enabled: Some(false),
            schedule: store::schedule_of(&a),
            action_kind: a.action_kind.clone(),
            action: serde_json::from_str(&a.action_json).unwrap_or(Value::Null),
            notify_on_failure: Some(a.notify_on_failure),
            keep_runs: Some(a.keep_runs),
            position: None,
        };
        let action =
            super::actions::Action::parse(&draft.action_kind, &draft.action).map_err(|e| {
                crate::db::DbError::Invalid {
                    what: "action",
                    detail: e.message,
                }
            })?;
        let sched = draft.schedule.normalised(&tz);
        let saved = store::save(c, &device, &draft, &sched, &action, now, &tz)?;
        Ok(Some(store::row(c, &saved, &tz)?))
    })?;
    let mut row = row.ok_or_else(|| not_found(&args.id))?;
    s.fill_problems(std::slice::from_mut(&mut row));
    s.changed();
    Ok(row)
}

/// The two example cards of 03 §7B.5 as drafts (created disabled; the
/// session example needs its repository chosen in the editor).
pub fn automation_examples(s: &Scheduler) -> ApiResult<Vec<AutomationDraft>> {
    let tz = s.device_tz().to_string();
    let last_repo: Option<String> = s.db().read(|c| {
        Ok(c.query_row(
            "SELECT COALESCE(repo_root, cwd) FROM sessions WHERE parent_session_id IS NULL
              ORDER BY started_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok())
    })?;
    Ok(vec![
        AutomationDraft {
            id: None,
            name: "Morning brief".into(),
            enabled: Some(false),
            schedule: Schedule {
                kind: ScheduleKind::Weekdays,
                time: Some("08:00".into()),
                tz: Some(tz.clone()),
                weekday: None,
                cron: None,
            },
            action_kind: "ask_chat".into(),
            action: json!({"prompt": "Summarize today's inbox and calendar", "folder_id": null, "effort": "auto"}),
            notify_on_failure: Some(true),
            keep_runs: Some(20),
            position: None,
        },
        AutomationDraft {
            id: None,
            name: "Nightly test run".into(),
            enabled: Some(false),
            schedule: Schedule {
                kind: ScheduleKind::Daily,
                time: Some("23:00".into()),
                tz: Some(tz),
                weekday: None,
                cron: None,
            },
            action_kind: "start_session".into(),
            action: json!({
                "cwd": last_repo.unwrap_or_default(),
                "prompt": "Run the test suite and summarize the failures",
                "permission_mode": "default",
                "mode": "headless",
                "token_cap": null
            }),
            notify_on_failure: Some(true),
            keep_runs: Some(20),
            position: None,
        },
    ])
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreviewArgs {
    /// The editor's schedule (an object, or its JSON text).
    pub schedule_json: Value,
}

/// `schedule_preview` (03 §7B.2: the cron echo and the next fire times;
/// `SchedulePreview {fires, fires_local, echo, error}`). `valid`,
/// `trigger_phrase` and `tz` are beyond the contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulePreview {
    /// The next five fire times, UTC.
    pub fires: Vec<String>,
    /// The same as wall times in the schedule's zone, `YYYY-MM-DD HH:MM`.
    #[serde(default)]
    pub fires_local: Vec<String>,
    /// "every weekday at 08:00"; `None` when the schedule does not compile.
    pub echo: Option<String>,
    /// The field the schedule fails on.
    pub error: Option<ScheduleError>,
    #[serde(default)]
    pub valid: bool,
    /// "Weekdays 08:00 · New York".
    #[serde(default)]
    pub trigger_phrase: String,
    /// The zone the times are computed in.
    #[serde(default)]
    pub tz: String,
}

/// `schedule_preview {schedule_json}` → the next five fire times.
pub fn schedule_preview(s: &Scheduler, args: PreviewArgs) -> ApiResult<SchedulePreview> {
    let v = match args.schedule_json {
        Value::String(t) => serde_json::from_str(&t)?,
        v => v,
    };
    let sched: Schedule = match serde_json::from_value(v) {
        Ok(x) => x,
        Err(e) => {
            return Ok(SchedulePreview {
                fires: Vec::new(),
                fires_local: Vec::new(),
                echo: None,
                error: Some(ScheduleError {
                    field: "schedule".into(),
                    message: format!("The schedule could not be read: {e}"),
                }),
                valid: false,
                trigger_phrase: String::new(),
                tz: s.device_tz().to_string(),
            });
        }
    };
    let tz = s.device_tz();
    let sched = sched.normalised(tz);
    let trigger_phrase = schedule::trigger_phrase(&sched, tz);
    Ok(match sched.plan(tz) {
        Ok(p) => {
            let next = p.next_n(s.now(), 5);
            SchedulePreview {
                fires_local: next.iter().map(|t| p.local(*t)).collect(),
                fires: next.into_iter().map(format_timestamp).collect(),
                echo: Some(schedule::describe(&sched)),
                error: None,
                valid: true,
                trigger_phrase,
                tz: p.tz_name.clone(),
            }
        }
        Err(e) => SchedulePreview {
            fires: Vec::new(),
            fires_local: Vec::new(),
            echo: None,
            error: Some(e),
            valid: false,
            trigger_phrase,
            tz: sched.tz.clone().unwrap_or_default(),
        },
    })
}
