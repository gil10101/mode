//! `automations` and `automation_runs` rows (09 §5.4A): the list row the
//! Automations view shows, the run rows, saving a draft with its validation,
//! the scheduler's bookkeeping columns and run pruning. Saves go through the
//! repository (`rev`, `change_log`); `last_run_at` / `next_run_at` are the
//! scheduler's derived state and are written without a `rev` bump, so they
//! never reach `change_log`.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use super::actions::{self, Action, FieldError, ferr};
use super::schedule::{self, Schedule};
use crate::db::{Result, format_timestamp, parse_timestamp};
use crate::repo::{self, Automation, AutomationRun};

/// Runs older than this are pruned whatever `keep_runs` says (09 §10).
pub const RUN_RETENTION_DAYS: i64 = 90;
/// Skip reasons of 03 §7B.3.
pub const SKIP_DISABLED: &str = "the automation was disabled at fire time";
pub const SKIP_CLOSED: &str = "Mode was closed past the catch-up window";
pub const SKIP_RUNNING: &str = "a previous run of the same automation was still running";

/// A session run's diff (`+add −del`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDiff {
    pub add: i64,
    pub del: i64,
}

/// One run (03 §7B.3; `RunRow` in `contract-automations.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRow {
    pub id: String,
    pub automation_id: String,
    /// `schedule` | `manual` (Run now).
    pub trigger_kind: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    /// `running` | `finished` | `failed` | `skipped`.
    pub state: String,
    /// `session` | `note`.
    pub result_kind: Option<String>,
    pub result_id: Option<String>,
    /// The failure text, or the skip reason.
    pub error: Option<String>,
    pub duration_ms: Option<i64>,
    /// The result's display name ("morning-brief", "Daily plan 14 Oct");
    /// `None` before there is one or once it is gone.
    #[serde(default)]
    pub result_title: Option<String>,
    /// A session result's diff.
    #[serde(default)]
    pub result_diff: Option<RunDiff>,
}

impl RunRow {
    pub fn from_row(r: &AutomationRun) -> RunRow {
        let duration_ms = match (
            parse_timestamp(&r.started_at),
            r.finished_at.as_deref().and_then(parse_timestamp),
        ) {
            (Some(a), Some(b)) => Some((b - a).whole_milliseconds() as i64),
            _ => None,
        };
        RunRow {
            id: r.id.clone(),
            automation_id: r.automation_id.clone(),
            trigger_kind: r.trigger_kind.clone(),
            started_at: r.started_at.clone(),
            finished_at: r.finished_at.clone(),
            state: r.state.clone(),
            result_kind: r.result_kind.clone(),
            result_id: r.result_id.clone(),
            error: r.error.clone(),
            duration_ms,
            result_title: None,
            result_diff: None,
        }
    }

    /// The row with its result's title and diff (`result_title`, `result_diff`).
    pub fn load(conn: &Connection, r: &AutomationRun) -> Result<RunRow> {
        let mut row = RunRow::from_row(r);
        match (row.result_kind.as_deref(), row.result_id.as_deref()) {
            (Some("session"), Some(sid)) => {
                if let Some((name, add, del)) = conn
                    .query_row(
                        "SELECT display_name, lines_added, lines_removed FROM sessions WHERE id = ?1",
                        [sid],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
                    )
                    .optional()?
                {
                    row.result_title = Some(name);
                    row.result_diff = Some(RunDiff { add, del });
                }
            }
            (Some("note"), Some(nid)) => {
                row.result_title = conn
                    .query_row(
                        "SELECT title FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                        [nid],
                        |r| r.get(0),
                    )
                    .optional()?;
            }
            _ => {}
        }
        Ok(row)
    }
}

/// A list row (03 §7B.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// `schedule` (v1).
    pub trigger_kind: String,
    pub schedule: Schedule,
    /// "Weekdays 08:00".
    pub trigger_phrase: String,
    /// `start_session` | `create_note` | `ask_chat`.
    pub action_kind: String,
    pub action: Value,
    /// "Start session in mode · summarize inbox and calendar".
    pub action_phrase: String,
    pub notify_on_failure: bool,
    pub keep_runs: i64,
    pub position: i64,
    pub last_run_at: Option<String>,
    /// UTC; `None` while disabled.
    pub next_run_at: Option<String>,
    /// `next_run_at` as wall time in the schedule's zone, `YYYY-MM-DD HH:MM`.
    pub next_run_local: Option<String>,
    pub last_run: Option<RunRow>,
    /// A run is in `running` (Run now is disabled).
    pub running: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Why the row shows `✕` before it runs (03 §7B.7): `claude_not_found` |
    /// `no_ai_backend` | `folder_missing`. The store sets `folder_missing`;
    /// the scheduler adds the dispatcher's ([`super::Scheduler::rows`]).
    #[serde(default)]
    pub problem: Option<String>,
}

/// The vault path of a folder id for the phrases (`None` = the capture folder).
pub fn folder_label(conn: &Connection, id: Option<&str>) -> String {
    let q = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> Option<String> {
        conn.query_row(sql, p, |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    };
    match id {
        Some("root") | Some("") => String::new(),
        Some(id) => q(
            "SELECT path FROM folders WHERE id = ?1 AND deleted_at IS NULL",
            &[&id],
        )
        .unwrap_or_else(|| "a missing folder".into()),
        None => q(
            "SELECT path FROM folders WHERE kind = 'capture' AND deleted_at IS NULL LIMIT 1",
            &[],
        )
        .unwrap_or_else(|| crate::notes::vault::CAPTURE_FOLDER.to_string()),
    }
}

pub fn load(conn: &Connection, id: &str) -> Result<Option<Automation>> {
    repo::get_live::<Automation>(conn, id)
}

pub fn schedule_of(a: &Automation) -> Schedule {
    serde_json::from_str(&a.schedule_json).unwrap_or(Schedule {
        kind: schedule::ScheduleKind::Daily,
        time: None,
        tz: None,
        weekday: None,
        cron: None,
    })
}

pub fn last_run(conn: &Connection, automation_id: &str) -> Result<Option<AutomationRun>> {
    Ok(repo::query::<AutomationRun>(
        conn,
        "automation_id = ?1 ORDER BY started_at DESC, created_at DESC LIMIT 1",
        &[&automation_id],
    )?
    .into_iter()
    .next())
}

pub fn has_running(conn: &Connection, automation_id: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM automation_runs WHERE automation_id = ?1 AND state = 'running' LIMIT 1",
            [automation_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

pub fn row(conn: &Connection, a: &Automation, device_tz: &str) -> Result<AutomationRow> {
    let sched = schedule_of(a);
    let action_json: Value = serde_json::from_str(&a.action_json).unwrap_or(Value::Null);
    let parsed = Action::parse(&a.action_kind, &action_json);
    let action_phrase = match &parsed {
        Ok(act) => actions::action_phrase(act, &|id| folder_label(conn, id)),
        Err(_) => a.action_kind.clone(),
    };
    let problem = match &parsed {
        Ok(act) => folder_problem(conn, act)?,
        Err(_) => None,
    };
    let next_run_local = match (&a.next_run_at, sched.plan(device_tz)) {
        (Some(n), Ok(p)) => parse_timestamp(n).map(|t| p.local(t)),
        _ => None,
    };
    let last = last_run(conn, &a.id)?;
    Ok(AutomationRow {
        id: a.id.clone(),
        name: a.name.clone(),
        enabled: a.enabled,
        trigger_kind: a.trigger_kind.clone(),
        trigger_phrase: schedule::trigger_phrase(&sched, device_tz),
        schedule: sched,
        action_kind: a.action_kind.clone(),
        action: action_json,
        action_phrase,
        notify_on_failure: a.notify_on_failure,
        keep_runs: a.keep_runs,
        position: a.position,
        last_run_at: a.last_run_at.clone(),
        next_run_at: a.next_run_at.clone(),
        next_run_local,
        running: has_running(conn, &a.id)?,
        last_run: match last.as_ref() {
            Some(r) => Some(RunRow::load(conn, r)?),
            None => None,
        },
        created_at: a.created_at.clone(),
        updated_at: a.updated_at.clone(),
        problem,
    })
}

/// `folder_missing` when the action's folder is gone (a deleted vault
/// folder, a session `cwd` that no longer exists).
fn folder_problem(conn: &Connection, act: &Action) -> Result<Option<String>> {
    let missing = match act {
        Action::StartSession(s) => !std::path::Path::new(&s.cwd).is_dir(),
        Action::CreateNote(n) => folder_gone(conn, n.folder_id.as_deref())?,
        Action::AskChat(c) => folder_gone(conn, c.folder_id.as_deref())?,
    };
    Ok(missing.then(|| "folder_missing".to_string()))
}

fn folder_gone(conn: &Connection, id: Option<&str>) -> Result<bool> {
    match id {
        None | Some("") | Some("root") => Ok(false),
        Some(id) => Ok(conn
            .query_row(
                "SELECT 1 FROM folders WHERE id = ?1 AND deleted_at IS NULL",
                [id],
                |_| Ok(()),
            )
            .optional()?
            .is_none()),
    }
}

pub fn list(conn: &Connection, device_tz: &str) -> Result<Vec<AutomationRow>> {
    repo::list_live::<Automation>(conn, "position, name COLLATE NOCASE")?
        .iter()
        .map(|a| row(conn, a, device_tz))
        .collect()
}

pub fn runs(conn: &Connection, automation_id: &str, limit: i64) -> Result<Vec<RunRow>> {
    repo::query::<AutomationRun>(
        conn,
        "automation_id = ?1 ORDER BY started_at DESC, created_at DESC LIMIT ?2",
        &[&automation_id, &limit],
    )?
    .iter()
    .map(|r| RunRow::load(conn, r))
    .collect()
}

pub fn run_by_id(conn: &Connection, id: &str) -> Result<Option<AutomationRun>> {
    repo::get_by_id::<AutomationRun>(conn, id)
}

/// A run row to insert.
#[derive(Debug, Clone, Copy)]
pub struct NewRun<'a> {
    pub automation_id: &'a str,
    /// `schedule` | `manual`.
    pub trigger_kind: &'a str,
    /// `running` | `skipped` | `failed`.
    pub state: &'a str,
    pub started_at: &'a str,
    pub finished_at: Option<&'a str>,
    pub error: Option<&'a str>,
}

impl<'a> NewRun<'a> {
    /// A run that ends as it starts (`skipped`, or `failed` up front).
    pub fn ended(
        automation_id: &'a str,
        trigger_kind: &'a str,
        state: &'a str,
        at: &'a str,
        why: &'a str,
    ) -> Self {
        NewRun {
            automation_id,
            trigger_kind,
            state,
            started_at: at,
            finished_at: Some(at),
            error: Some(why),
        }
    }
}

/// Insert a run row.
pub fn insert_run(conn: &Connection, n: &NewRun<'_>, now: &str) -> Result<AutomationRun> {
    let run = AutomationRun {
        id: repo::new_id(),
        automation_id: n.automation_id.into(),
        trigger_kind: n.trigger_kind.into(),
        started_at: n.started_at.into(),
        finished_at: n.finished_at.map(str::to_string),
        state: n.state.into(),
        result_kind: None,
        result_id: None,
        error: n.error.map(str::to_string),
        created_at: now.into(),
    };
    repo::insert(conn, &run)?;
    Ok(run)
}

/// `running` → `finished` / `failed`.
pub fn finish_run(
    conn: &Connection,
    run_id: &str,
    state: &str,
    result: Option<(&str, &str)>,
    error: Option<&str>,
    now: &str,
) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE automation_runs
            SET state = ?2, finished_at = ?3, error = ?4,
                result_kind = COALESCE(?5, result_kind), result_id = COALESCE(?6, result_id)
          WHERE id = ?1 AND state = 'running'",
        params![
            run_id,
            state,
            now,
            error,
            result.map(|r| r.0),
            result.map(|r| r.1)
        ],
    )? > 0)
}

/// A running run gets its result while it keeps running (a session).
pub fn set_run_result(conn: &Connection, run_id: &str, kind: &str, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE automation_runs SET result_kind = ?2, result_id = ?3 WHERE id = ?1",
        params![run_id, kind, id],
    )?;
    Ok(())
}

/// The scheduler's derived columns (no `rev` bump, no `change_log`).
pub fn set_times(
    conn: &Connection,
    id: &str,
    last_run_at: Option<&str>,
    next_run_at: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE automations SET last_run_at = COALESCE(?2, last_run_at), next_run_at = ?3 WHERE id = ?1",
        params![id, last_run_at, next_run_at],
    )?;
    Ok(())
}

/// Keep the newest `keep` finished / failed / skipped runs and nothing older
/// than 90 days (03 §7B.4 "Pruning"); a running run is never pruned.
pub fn prune(
    conn: &Connection,
    automation_id: &str,
    keep: i64,
    now: OffsetDateTime,
) -> Result<usize> {
    let cutoff = format_timestamp(now - time::Duration::days(RUN_RETENTION_DAYS));
    let mut n = conn.execute(
        "DELETE FROM automation_runs WHERE automation_id = ?1 AND state <> 'running' AND started_at < ?2",
        params![automation_id, cutoff],
    )?;
    n += conn.execute(
        "DELETE FROM automation_runs
          WHERE automation_id = ?1 AND state <> 'running' AND id NOT IN (
                SELECT id FROM automation_runs WHERE automation_id = ?1 AND state <> 'running'
                 ORDER BY started_at DESC, created_at DESC LIMIT ?2)",
        params![automation_id, keep.max(1)],
    )?;
    Ok(n)
}

// -------------------------------------------------------------------- save

/// What the editor saves (`automation_save {draft}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationDraft {
    /// `None` creates a new automation.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    pub schedule: Schedule,
    pub action_kind: String,
    pub action: Value,
    #[serde(default)]
    pub notify_on_failure: Option<bool>,
    #[serde(default)]
    pub keep_runs: Option<i64>,
    #[serde(default)]
    pub position: Option<i64>,
}

/// Validate a draft against the database: the schedule compiles, the
/// action's fields are present, its folder / template / grouping exist.
pub fn check_draft(
    conn: &Connection,
    d: &AutomationDraft,
    device_tz: &str,
) -> Result<std::result::Result<(Schedule, Action), Vec<FieldError>>> {
    let mut errors = Vec::new();
    if d.name.trim().is_empty() {
        errors.push(ferr("name", "Name the automation."));
    } else if d.name.chars().count() > 120 {
        errors.push(ferr("name", "Keep the name under 120 characters."));
    }
    let sched = d.schedule.normalised(device_tz);
    if let Err(e) = sched.plan(device_tz) {
        errors.push(ferr(&e.field, e.message));
    }
    if let Some(k) = d.keep_runs
        && !(1..=500).contains(&k)
    {
        errors.push(ferr("keep_runs", "Keep between 1 and 500 runs."));
    }
    let action = match Action::parse(&d.action_kind, &d.action) {
        Ok(a) => {
            errors.extend(a.check());
            let folder_exists = |id: &str| -> Result<bool> {
                Ok(id == "root"
                    || conn
                        .query_row(
                            "SELECT 1 FROM folders WHERE id = ?1 AND deleted_at IS NULL",
                            [id],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_some())
            };
            let folder = match &a {
                Action::CreateNote(n) => n.folder_id.clone(),
                Action::AskChat(c) => c.folder_id.clone(),
                Action::StartSession(_) => None,
            };
            if let Some(f) = folder.filter(|f| !f.is_empty())
                && !folder_exists(&f)?
            {
                errors.push(ferr("action.folder_id", "That folder no longer exists."));
            }
            if let Action::CreateNote(n) = &a {
                if let Some(t) = n.template_note_id.as_deref().filter(|t| !t.is_empty())
                    && conn
                        .query_row(
                            "SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                            [t],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_none()
                {
                    errors.push(ferr("action.template_note_id", "That template is gone."));
                }
                if let Some(g) = n.grouping_slug.as_deref().filter(|g| !g.is_empty())
                    && conn
                        .query_row(
                            "SELECT 1 FROM groupings WHERE slug = ?1 AND deleted_at IS NULL",
                            [g],
                            |_| Ok(()),
                        )
                        .optional()?
                        .is_none()
                {
                    errors.push(ferr("action.grouping_slug", "That grouping is gone."));
                }
            }
            Some(a)
        }
        Err(e) => {
            errors.push(e);
            None
        }
    };
    Ok(match action {
        Some(a) if errors.is_empty() => Ok((sched, a)),
        _ => Err(errors),
    })
}

/// Insert or update an automation from a valid draft; `next_run_at` is
/// computed from `now` when enabled, NULL when disabled.
pub fn save(
    conn: &Connection,
    device_id: &str,
    d: &AutomationDraft,
    sched: &Schedule,
    action: &Action,
    now: OffsetDateTime,
    device_tz: &str,
) -> Result<Automation> {
    let next = |enabled: bool| -> Option<String> {
        if !enabled {
            return None;
        }
        sched
            .plan(device_tz)
            .ok()
            .and_then(|p| p.next_after(now))
            .map(format_timestamp)
    };
    let schedule_json = serde_json::to_string(sched)?;
    let action_json = action.to_json().to_string();
    let existing = match d.id.as_deref() {
        Some(id) => load(conn, id)?,
        None => None,
    };
    if d.id.is_some() && existing.is_none() {
        return Err(crate::db::DbError::NotFound {
            table: "automations",
            key: d.id.clone().unwrap_or_default(),
        });
    }
    match existing {
        Some(mut a) => {
            let enabled = d.enabled.unwrap_or(a.enabled);
            a.name = d.name.trim().to_string();
            a.enabled = enabled;
            a.schedule_json = schedule_json;
            a.action_kind = action.kind().into();
            a.action_json = action_json;
            a.notify_on_failure = d.notify_on_failure.unwrap_or(a.notify_on_failure);
            a.keep_runs = d.keep_runs.unwrap_or(a.keep_runs);
            a.position = d.position.unwrap_or(a.position);
            a.next_run_at = next(enabled);
            repo::save(conn, device_id, &mut a)?;
            Ok(a)
        }
        None => {
            let position = match d.position {
                Some(p) => p,
                None => conn.query_row(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM automations WHERE deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )?,
            };
            let enabled = d.enabled.unwrap_or(true);
            let mut a = Automation {
                id: repo::new_id(),
                name: d.name.trim().to_string(),
                enabled,
                trigger_kind: "schedule".into(),
                schedule_json,
                action_kind: action.kind().into(),
                action_json,
                notify_on_failure: d.notify_on_failure.unwrap_or(true),
                keep_runs: d.keep_runs.unwrap_or(20),
                last_run_at: None,
                next_run_at: next(enabled),
                position,
                ..Default::default()
            };
            repo::create(conn, device_id, &mut a)?;
            Ok(a)
        }
    }
}
