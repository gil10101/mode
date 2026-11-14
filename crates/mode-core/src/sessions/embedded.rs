//! The engine's side of embedded sessions (05 §8, §14; the non-hook inputs
//! of 05 §4.2): the `launched` row created before any hook arrives, the
//! `MODE_LAUNCH_ID` join, resume, the PTY / headless process exit, the
//! "output flowing" inference when hooks are absent, the headless `result`,
//! and the "no hook events" notice. The PTY host (`crate::pty`) drives these;
//! `sessions.state` is still written only from this module tree.
//!
//! A launched row carries a placeholder `claude_session_id`
//! (`mode-launch:<launch id>`) until the first frame carrying its launch id
//! (or a headless `system/init`) names the real one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

use super::avatars;
use super::engine::Engine;
use super::naming;
use super::writes::{EventIn, close_children, insert_event};
use super::{Pending, SessionState};
use crate::db::{DbError, Result};
use crate::push;
use crate::queries::ConnectorRow;
use crate::repo::{self, Session};

/// Prefix of the placeholder `claude_session_id` of a launched row.
pub const PLACEHOLDER_PREFIX: &str = "mode-launch:";

/// Whether a `claude_session_id` is a launch placeholder.
pub fn is_placeholder(claude_session_id: &str) -> bool {
    claude_session_id.starts_with(PLACEHOLDER_PREFIX)
}

/// A launch the engine expects frames from (05 §8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchReg {
    /// The Mode row the launch drives (moves on `/clear`, which starts a new
    /// Claude session in the same process).
    pub row_id: String,
    /// `embedded` (PTY) or `headless` (stream-json turn).
    pub launch_mode: String,
}

/// Answers which driver a row has (`pty` | `headless` | `none`, 05 §14).
pub type DriverFn = Arc<dyn Fn(&str) -> &'static str + Send + Sync>;

/// Launch registrations and the driver source, held by the engine.
#[derive(Default)]
pub struct Registry {
    launches: Mutex<HashMap<String, LaunchReg>>,
    driver: RwLock<Option<DriverFn>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("launches", &self.lock().len())
            .finish_non_exhaustive()
    }
}

impl Registry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, LaunchReg>> {
        self.launches.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Frames carrying `launch_id` belong to `row_id`.
    pub fn register(&self, launch_id: &str, row_id: &str, launch_mode: &str) {
        self.lock().insert(
            launch_id.to_string(),
            LaunchReg {
                row_id: row_id.to_string(),
                launch_mode: launch_mode.to_string(),
            },
        );
    }

    pub fn launch(&self, launch_id: &str) -> Option<LaunchReg> {
        self.lock().get(launch_id).cloned()
    }

    /// The process of this launch is gone; late frames no longer join.
    pub fn forget(&self, launch_id: &str) {
        self.lock().remove(launch_id);
    }

    /// A frame of this launch landed on another row (`/clear`).
    pub fn moved(&self, launch_id: &str, row_id: &str) {
        if let Some(r) = self.lock().get_mut(launch_id) {
            r.row_id = row_id.to_string();
        }
    }

    /// Install the PTY host's driver lookup (`SessionDetail.driver`).
    pub fn set_driver_source(&self, f: DriverFn) {
        *self.driver.write().unwrap_or_else(|p| p.into_inner()) = Some(f);
    }

    /// `pty` | `headless` | `none`.
    pub fn driver(&self, row_id: &str) -> &'static str {
        self.driver
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map_or("none", |f| f(row_id))
    }
}

/// What [`Engine::create_launched`] needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLaunch {
    pub launch_id: String,
    pub cwd: String,
    pub permission_mode: Option<String>,
    /// The agent profile the session runs as (03 §7E).
    pub profile_id: Option<String>,
}

/// What [`Engine::prepare_resume`] changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePrep {
    pub launch_mode: String,
    pub permission_mode: Option<String>,
    pub cwd: Option<String>,
    /// Hooks are not installed: no `SessionStart(resume)` will reopen the
    /// row, so it reopens now.
    pub reopen: bool,
}

/// A PTY or headless process ended (05 §4.2 "PTY process exit").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessExit {
    /// `pty` or `headless` (the `session_events.source`).
    pub source: &'static str,
    pub code: Option<i32>,
    /// Signal name when the process was killed by one.
    pub signal: Option<String>,
    /// Mode sent the SIGINT that ended it (close, quit, Stop turn).
    pub interrupted_by_mode: bool,
    /// The process never started: the spawn error line (05 §16).
    pub error: Option<String>,
}

impl ProcessExit {
    /// Exit 0, or the SIGINT Mode sent (exit 130 / killed by SIGINT).
    pub fn clean(&self) -> bool {
        if self.error.is_some() {
            return false;
        }
        let by_sigint = self.interrupted_by_mode
            && (self.code == Some(130)
                || self
                    .signal
                    .as_deref()
                    .is_some_and(|s| s.contains("INT") || s.contains("Interrupt")));
        by_sigint || (self.signal.is_none() && self.code.unwrap_or(0) == 0)
    }

    fn summary(&self) -> String {
        if let Some(e) = &self.error {
            return format!("could not start · {}", naming::truncate_chars(e, 300));
        }
        match (&self.signal, self.code) {
            (Some(s), _) => format!("exited · signal {s}"),
            (None, Some(c)) => format!("exited · code {c}"),
            (None, None) => "exited".into(),
        }
    }
}

/// A headless turn's `result` (05 §4.2 "Headless turn result").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnResult {
    pub is_error: bool,
    /// No `UserPromptSubmit` hook counted this prompt (hooks absent).
    pub count_prompt: bool,
}

fn row(conn: &Connection, id: &str) -> Result<Session> {
    repo::get_by_id::<Session>(conn, id)?.ok_or_else(|| DbError::NotFound {
        table: "sessions",
        key: id.to_string(),
    })
}

/// Name the real Claude session of a placeholder row; `false` when the row
/// already has a real id or another row owns that id.
pub fn attach_claude_id(conn: &Connection, row_id: &str, claude_session_id: &str) -> Result<bool> {
    if claude_session_id.is_empty() || is_placeholder(claude_session_id) {
        return Ok(false);
    }
    let n = conn.execute(
        "UPDATE sessions SET claude_session_id = ?1, updated_at = ?3
          WHERE id = ?2 AND claude_session_id LIKE 'mode-launch:%'
            AND NOT EXISTS (SELECT 1 FROM sessions WHERE claude_session_id = ?1 AND agent_id IS NULL)",
        params![claude_session_id, row_id, crate::db::now_rfc3339()],
    )?;
    Ok(n > 0)
}

/// The `launched` row of 05 §8.1, created before any hook arrives.
pub fn insert_launched(
    conn: &Connection,
    new: &NewLaunch,
    repo: &naming::RepoInfo,
    now: &str,
) -> Result<Session> {
    let placeholder = format!("{PLACEHOLDER_PREFIX}{}", new.launch_id);
    let row = Session {
        id: repo::new_id(),
        claude_session_id: placeholder.clone(),
        tracking: "launched".into(),
        launch_mode: "embedded".into(),
        state: SessionState::Idle.as_str().into(),
        state_since: now.to_string(),
        state_reason: Some("launch".into()),
        display_name: naming::display_name(None, None, &new.cwd, &placeholder),
        avatar_color: naming::avatar_color(repo.repo_root.as_deref(), &new.cwd),
        avatar_variant: avatars::next_variant(conn)?,
        cwd: new.cwd.clone(),
        repo_root: repo.repo_root.clone(),
        git_branch: repo.git_branch.clone(),
        permission_mode: new.permission_mode.clone(),
        profile_id: new.profile_id.clone(),
        started_at: now.to_string(),
        last_event_at: now.to_string(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        ..Default::default()
    };
    repo::insert(conn, &row)?;
    Ok(row)
}

/// `tracking = launched`, the launch mode, dismissal undone; reopened at
/// once when no `SessionStart(resume)` will do it (05 §8.2, §4.2).
pub fn prepare_resume(conn: &Connection, id: &str, prep: &ResumePrep, now: &str) -> Result<bool> {
    let r = row(conn, id)?;
    let reopen = prep.reopen && r.ended_at.is_some();
    conn.execute(
        "UPDATE sessions SET tracking = 'launched', launch_mode = ?2, dismissed_at = NULL,
                permission_mode = COALESCE(?3, permission_mode), cwd = COALESCE(?4, cwd), updated_at = ?5
          WHERE id = ?1",
        params![id, prep.launch_mode, prep.permission_mode, prep.cwd, now],
    )?;
    if reopen || (prep.reopen && r.state == SessionState::Untracked.as_str()) {
        conn.execute(
            "UPDATE sessions SET ended_at = NULL, end_reason = NULL, state = 'idle', state_since = ?2,
                    state_reason = 'resume', waiting_kind = NULL, pending_json = NULL,
                    acknowledged_at = NULL
              WHERE id = ?1",
            params![id, now],
        )?;
    }
    Ok(true)
}

/// `launch_mode` while a headless turn runs and after it (05 §8.5).
pub fn set_launch_mode(conn: &Connection, id: &str, mode: &str, now: &str) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE sessions SET launch_mode = ?2, updated_at = ?3 WHERE id = ?1 AND launch_mode <> ?2",
        params![id, mode, now],
    )? > 0)
}

/// 05 §4.2 "PTY process exit": closes a row no `SessionEnd` closed. Exit 0
/// (or Mode's own SIGINT) → `unread` (`done` when acknowledged); otherwise
/// `failed`. `failed` and `done` are kept. `None` when nothing changed.
pub fn record_exit(
    conn: &Connection,
    id: &str,
    exit: &ProcessExit,
    now: &str,
) -> Result<Option<SessionState>> {
    let Some(r) = repo::get_by_id::<Session>(conn, id)? else {
        return Ok(None);
    };
    if r.ended_at.is_some() && exit.error.is_none() {
        return Ok(None);
    }
    let before: SessionState = r.state.parse().unwrap_or(SessionState::Idle);
    let after = if !exit.clean() {
        SessionState::Failed
    } else {
        match before {
            SessionState::Failed => SessionState::Failed,
            SessionState::Done => SessionState::Done,
            _ if r.acknowledged_at.is_some() && before != SessionState::Waiting => {
                SessionState::Done
            }
            _ => SessionState::Unread,
        }
    };
    let reason = if exit.signal.is_some() && !exit.clean() {
        "signal"
    } else {
        "exit"
    };
    let source = if exit.source.is_empty() {
        "pty"
    } else {
        exit.source
    };
    conn.execute(
        "UPDATE sessions SET ended_at = COALESCE(ended_at, ?2), end_reason = COALESCE(end_reason, ?3),
                state = ?4, state_since = CASE WHEN state = ?4 THEN state_since ELSE ?2 END,
                state_reason = ?5, waiting_kind = NULL, pending_json = NULL,
                current_tool = NULL, current_tool_use_id = NULL, current_mcp_server = NULL,
                last_event_at = MAX(last_event_at, ?2), updated_at = ?2
          WHERE id = ?1",
        params![id, now, reason, after.as_str(), format!("{source}:exit")],
    )?;
    close_children(conn, id, now)?;
    insert_event(
        conn,
        EventIn {
            id: repo::new_id(),
            session_id: id,
            source,
            event: if source == "headless" {
                "headless.exit"
            } else {
                "pty.exit"
            },
            tool_name: None,
            tool_use_id: None,
            agent_id: None,
            state_before: Some(before),
            state_after: (after != before).then_some(after),
            summary: exit.summary(),
            payload: Some(serde_json::json!({
                "code": exit.code,
                "signal": exit.signal,
                "interrupted_by_mode": exit.interrupted_by_mode,
                "error": exit.error,
            })),
            payload_bytes: 0,
            occurred_at: now,
            received_at: now,
        },
    )?;
    Ok(Some(after))
}

/// The "without hooks" inference of 05 §8.1: `working` while output flows,
/// `idle` after the silence. Never touches `waiting`, `failed` or ended rows.
pub fn infer_state(conn: &Connection, id: &str, working: bool, now: &str) -> Result<bool> {
    let n = if working {
        conn.execute(
            "UPDATE sessions SET state = 'working', state_since = ?2, state_reason = 'pty:output',
                    last_event_at = MAX(last_event_at, ?2), updated_at = ?2
              WHERE id = ?1 AND ended_at IS NULL AND state IN ('idle', 'unread', 'done')",
            params![id, now],
        )?
    } else {
        conn.execute(
            "UPDATE sessions SET state = 'idle', state_since = ?2, state_reason = 'pty:silence',
                    updated_at = ?2
              WHERE id = ?1 AND ended_at IS NULL AND state = 'working'",
            params![id, now],
        )?
    };
    Ok(n > 0)
}

/// A headless `result` (05 §4.2): `unread`, or `failed` when `is_error`;
/// `num_turns + 1` when no hook counted the prompt. A success leaves an
/// already-ended row alone (its `SessionEnd` decided).
pub fn record_turn_result(
    conn: &Connection,
    id: &str,
    res: &TurnResult,
    now: &str,
) -> Result<bool> {
    let r = row(conn, id)?;
    let before: SessionState = r.state.parse().unwrap_or(SessionState::Idle);
    let target = if res.is_error {
        SessionState::Failed
    } else {
        SessionState::Unread
    };
    let mut changed = false;
    if res.count_prompt {
        conn.execute(
            "UPDATE sessions SET num_turns = num_turns + 1, updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        changed = true;
    }
    let move_state = before != target
        && before != SessionState::Failed
        && (r.ended_at.is_none() || res.is_error);
    if move_state {
        conn.execute(
            "UPDATE sessions SET state = ?2, state_since = ?3, state_reason = 'headless:result',
                    waiting_kind = NULL, pending_json = NULL, current_tool = NULL,
                    current_tool_use_id = NULL, current_mcp_server = NULL,
                    last_event_at = MAX(last_event_at, ?3), updated_at = ?3
              WHERE id = ?1",
            params![id, target.as_str(), now],
        )?;
        changed = true;
    }
    Ok(changed)
}

/// 05 §8.1: no `SessionStart` within 30 s of the launch. Written once per
/// launch; the row stays.
pub fn mark_no_hook_events(conn: &Connection, id: &str, now: &str) -> Result<bool> {
    let r = row(conn, id)?;
    let state: SessionState = r.state.parse().unwrap_or(SessionState::Idle);
    conn.execute(
        "UPDATE sessions SET state_reason = 'no_hook_events', updated_at = ?2
          WHERE id = ?1 AND ended_at IS NULL",
        params![id, now],
    )?;
    insert_event(
        conn,
        EventIn {
            id: repo::new_id(),
            session_id: id,
            source: "mode",
            event: "mode.no_hook_events",
            tool_name: None,
            tool_use_id: None,
            agent_id: None,
            state_before: Some(state),
            state_after: None,
            summary: "no hook events — check hook install".into(),
            payload: None,
            payload_bytes: 0,
            occurred_at: now,
            received_at: now,
        },
    )?;
    Ok(true)
}

/// Whether a hook frame for the row arrived at or after `since` (optionally
/// only `event`).
pub fn hook_event_since(
    conn: &Connection,
    id: &str,
    since: &str,
    event: Option<&str>,
) -> Result<bool> {
    let hit: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM session_events
              WHERE session_id = ?1 AND source = 'hook' AND received_at >= ?2
                AND (?3 IS NULL OR event = ?3)
              LIMIT 1",
            params![id, since, event],
            |r| r.get(0),
        )
        .optional()?;
    Ok(hit.is_some())
}

/// `hook_installs.status = 'installed'` for some settings file (05 §8.1).
pub fn hooks_installed(conn: &Connection) -> Result<bool> {
    let hit: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM hook_installs WHERE status = 'installed' AND removed_at IS NULL LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(hit.is_some())
}

/// `SessionDetail.connectors_source`: `init` once a headless `system/init`
/// filled the tab (05 §6A.2), `hooks` otherwise.
pub fn connectors_source(rows: &[ConnectorRow]) -> &'static str {
    let from_init = rows.iter().any(|r| {
        r.detail_json
            .as_deref()
            .and_then(|d| serde_json::from_str::<Value>(d).ok())
            .and_then(|v| v.get("source").and_then(Value::as_str).map(|s| s == "init"))
            .unwrap_or(false)
    });
    if from_init { "init" } else { "hooks" }
}

fn pushes(changed: bool) -> Vec<push::Event> {
    if changed {
        vec![push::Event::Sessions, push::Event::Pill]
    } else {
        Vec::new()
    }
}

impl Engine {
    /// Create the `launched` row of 05 §8.1 (`idle`, the folder's name, the
    /// repository resolved first) and expect frames carrying `launch_id`.
    /// Runs git: call it off the async runtime.
    pub fn create_launched(&self, new: &NewLaunch) -> Result<(Session, Vec<push::Event>)> {
        let repo = naming::repo_info(&new.cwd);
        let n = new.clone();
        let row = self.db().write(move |c| {
            repo::tx(c, |tx| {
                insert_launched(tx, &n, &repo, &crate::db::now_rfc3339())
            })
        })?;
        self.embedded()
            .register(&new.launch_id, &row.id, "embedded");
        tracing::info!(target: "pty", session = %row.id, "launched row created");
        Ok((row, pushes(true)))
    }

    /// Resume bookkeeping before the process starts (05 §8.2).
    pub fn prepare_resume(&self, session_id: &str, prep: &ResumePrep) -> Result<Vec<push::Event>> {
        let sid = session_id.to_string();
        let p = prep.clone();
        let changed = self.db().write(move |c| {
            repo::tx(c, |tx| {
                prepare_resume(tx, &sid, &p, &crate::db::now_rfc3339())
            })
        })?;
        Ok(pushes(changed))
    }

    /// Name the Claude session of a placeholder row (headless `system/init`).
    pub fn attach_claude_session(&self, session_id: &str, claude_session_id: &str) -> Result<bool> {
        let (sid, cid) = (session_id.to_string(), claude_session_id.to_string());
        self.db().write(move |c| attach_claude_id(c, &sid, &cid))
    }

    pub fn set_launch_mode(&self, session_id: &str, mode: &str) -> Result<Vec<push::Event>> {
        let (sid, m) = (session_id.to_string(), mode.to_string());
        let changed = self
            .db()
            .write(move |c| set_launch_mode(c, &sid, &m, &crate::db::now_rfc3339()))?;
        Ok(if changed {
            vec![push::Event::Sessions]
        } else {
            Vec::new()
        })
    }

    /// The process of an embedded session ended (05 §4.2, §16). Call it
    /// after the 2 s `SessionEnd` grace; a row a `SessionEnd` already closed
    /// is left alone.
    pub fn record_process_exit(
        &self,
        session_id: &str,
        exit: &ProcessExit,
    ) -> Result<Vec<push::Event>> {
        self.forget_process(session_id);
        let sid = session_id.to_string();
        let e = exit.clone();
        let after = self.db().write(move |c| {
            repo::tx(c, |tx| record_exit(tx, &sid, &e, &crate::db::now_rfc3339()))
        })?;
        Ok(pushes(after.is_some()))
    }

    /// 05 §8.1 "Without hooks": `working` while output flows, `idle` after
    /// the silence.
    pub fn infer_state(&self, session_id: &str, working: bool) -> Result<Vec<push::Event>> {
        let sid = session_id.to_string();
        let changed = self
            .db()
            .write(move |c| infer_state(c, &sid, working, &crate::db::now_rfc3339()))?;
        Ok(pushes(changed))
    }

    pub fn record_turn_result(
        &self,
        session_id: &str,
        res: &TurnResult,
    ) -> Result<Vec<push::Event>> {
        let sid = session_id.to_string();
        let r = res.clone();
        let changed = self.db().write(move |c| {
            repo::tx(c, |tx| {
                record_turn_result(tx, &sid, &r, &crate::db::now_rfc3339())
            })
        })?;
        Ok(pushes(changed))
    }

    pub fn mark_no_hook_events(&self, session_id: &str) -> Result<Vec<push::Event>> {
        let sid = session_id.to_string();
        self.db().write(move |c| {
            repo::tx(c, |tx| {
                mark_no_hook_events(tx, &sid, &crate::db::now_rfc3339())
            })
        })?;
        Ok(vec![push::Event::Sessions])
    }

    pub fn hook_event_since(
        &self,
        session_id: &str,
        since: &str,
        event: Option<&str>,
    ) -> Result<bool> {
        self.db()
            .read(|c| hook_event_since(c, session_id, since, event))
    }

    pub fn hooks_installed(&self) -> Result<bool> {
        self.db().read(hooks_installed)
    }

    /// A row by Mode id.
    pub fn session_row(&self, session_id: &str) -> Result<Option<Session>> {
        self.db()
            .read(|c| repo::get_by_id::<Session>(c, session_id))
    }

    /// The open card of a row, if any.
    pub fn pending_of(&self, session_id: &str) -> Result<Option<Pending>> {
        Ok(self
            .session_row(session_id)?
            .and_then(|r| r.pending_json.as_deref().and_then(Pending::from_json)))
    }

    /// "The engine says when" the turn ended (05 §8.3 "Close terminal"): the
    /// row is no longer `working`, or it ended.
    pub fn turn_over(&self, session_id: &str) -> Result<bool> {
        Ok(match self.session_row(session_id)? {
            Some(r) => r.ended_at.is_some() || r.state != SessionState::Working.as_str(),
            None => true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(code: Option<i32>, signal: Option<&str>, by_mode: bool) -> ProcessExit {
        ProcessExit {
            source: "pty",
            code,
            signal: signal.map(str::to_string),
            interrupted_by_mode: by_mode,
            error: None,
        }
    }

    #[test]
    fn exit_cleanliness_follows_the_table() {
        assert!(exit(Some(0), None, false).clean());
        assert!(!exit(Some(1), None, false).clean());
        assert!(
            !exit(Some(130), None, false).clean(),
            "SIGINT nobody in Mode sent"
        );
        assert!(exit(Some(130), None, true).clean(), "Mode's own SIGINT");
        assert!(exit(None, Some("Interrupt"), true).clean());
        assert!(!exit(None, Some("Killed"), true).clean());
        assert!(!exit(None, Some("Hangup"), false).clean());
        let mut e = exit(Some(0), None, false);
        e.error = Some("No such file or directory".into());
        assert!(!e.clean());
        assert!(e.summary().starts_with("could not start · "));
        assert_eq!(exit(Some(2), None, false).summary(), "exited · code 2");
        assert_eq!(
            exit(None, Some("Killed"), false).summary(),
            "exited · signal Killed"
        );
    }

    #[test]
    fn placeholders_are_recognised() {
        assert!(is_placeholder("mode-launch:0190"));
        assert!(!is_placeholder("b4e66983-4e8e"));
    }

    #[test]
    fn registry_follows_moves_and_answers_drivers() {
        let r = Registry::default();
        assert_eq!(r.driver("x"), "none");
        r.register("L1", "row-a", "embedded");
        assert_eq!(r.launch("L1").unwrap().row_id, "row-a");
        r.moved("L1", "row-b");
        assert_eq!(r.launch("L1").unwrap().row_id, "row-b");
        r.forget("L1");
        assert!(r.launch("L1").is_none());
        r.set_driver_source(Arc::new(|id: &str| if id == "p" { "pty" } else { "none" }));
        assert_eq!(r.driver("p"), "pty");
        assert_eq!(r.driver("q"), "none");
    }

    #[test]
    fn connectors_source_is_init_only_with_an_init_row() {
        let row = |detail: Option<&str>| ConnectorRow {
            kind: "mcp_server".into(),
            name: "github".into(),
            status: "connected".into(),
            detail_json: detail.map(str::to_string),
            seen_at: "t".into(),
        };
        assert_eq!(connectors_source(&[]), "hooks");
        assert_eq!(connectors_source(&[row(Some(r#"{"tools":3}"#))]), "hooks");
        assert_eq!(
            connectors_source(&[row(None), row(Some(r#"{"tools":3,"source":"init"}"#))]),
            "init"
        );
    }

    fn engine() -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::db::open(&dir.path().join("mode.db")).unwrap());
        (dir, Engine::new(db))
    }

    fn launched(e: &Engine, dir: &tempfile::TempDir) -> Session {
        e.create_launched(&NewLaunch {
            launch_id: uuid::Uuid::now_v7().to_string(),
            cwd: dir.path().to_string_lossy().into_owned(),
            permission_mode: Some("plan".into()),
            profile_id: None,
        })
        .unwrap()
        .0
    }

    fn set(e: &Engine, id: &str, sql: &'static str) {
        let id = id.to_string();
        e.db()
            .write(move |c| {
                c.execute(sql, [&id])?;
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn launched_rows_wait_for_their_claude_session() {
        let (dir, e) = engine();
        let row = launched(&e, &dir);
        let launch = row
            .claude_session_id
            .strip_prefix(PLACEHOLDER_PREFIX)
            .unwrap();
        assert_eq!(row.state, "idle");
        assert_eq!(row.permission_mode.as_deref(), Some("plan"));
        assert_eq!(e.embedded().launch(launch).unwrap().row_id, row.id);
        assert!(e.attach_claude_session(&row.id, "real-1").unwrap());
        assert!(
            !e.attach_claude_session(&row.id, "real-2").unwrap(),
            "a real id is never replaced"
        );
        assert_eq!(
            e.session_row(&row.id).unwrap().unwrap().claude_session_id,
            "real-1"
        );
        let other = launched(&e, &dir);
        assert!(
            !e.attach_claude_session(&other.id, "real-1").unwrap(),
            "an id another row owns is not taken"
        );
        assert!(!e.hooks_installed().unwrap());
    }

    #[test]
    fn process_exits_follow_the_non_hook_table() {
        let (dir, e) = engine();
        let ok = |code, by_mode| ProcessExit {
            source: "pty",
            code: Some(code),
            interrupted_by_mode: by_mode,
            ..Default::default()
        };
        let r = launched(&e, &dir);
        assert!(
            !e.record_process_exit(&r.id, &ok(0, false))
                .unwrap()
                .is_empty()
        );
        let row = e.session_row(&r.id).unwrap().unwrap();
        assert_eq!(
            (row.state.as_str(), row.end_reason.as_deref()),
            ("unread", Some("exit"))
        );
        assert!(
            e.record_process_exit(&r.id, &ok(1, false))
                .unwrap()
                .is_empty(),
            "an ended row (a SessionEnd came first) is left alone"
        );

        let r = launched(&e, &dir);
        set(
            &e,
            &r.id,
            "UPDATE sessions SET acknowledged_at = '2026-10-22T00:00:00.000Z' WHERE id = ?",
        );
        e.record_process_exit(&r.id, &ok(0, false)).unwrap();
        assert_eq!(e.session_row(&r.id).unwrap().unwrap().state, "done");

        let r = launched(&e, &dir);
        e.record_process_exit(&r.id, &ok(130, true)).unwrap();
        assert_eq!(
            e.session_row(&r.id).unwrap().unwrap().state,
            "unread",
            "Mode's SIGINT"
        );

        let r = launched(&e, &dir);
        e.record_process_exit(&r.id, &ok(2, false)).unwrap();
        assert_eq!(e.session_row(&r.id).unwrap().unwrap().state, "failed");

        let r = launched(&e, &dir);
        let sig = ProcessExit {
            source: "pty",
            signal: Some("Killed".into()),
            ..Default::default()
        };
        e.record_process_exit(&r.id, &sig).unwrap();
        let row = e.session_row(&r.id).unwrap().unwrap();
        assert_eq!(
            (row.state.as_str(), row.end_reason.as_deref()),
            ("failed", Some("signal"))
        );
    }

    #[test]
    fn inference_and_results_respect_the_live_states() {
        let (dir, e) = engine();
        let r = launched(&e, &dir);
        assert!(
            e.infer_state(&r.id, false).unwrap().is_empty(),
            "idle stays idle"
        );
        assert!(!e.infer_state(&r.id, true).unwrap().is_empty());
        assert_eq!(e.session_row(&r.id).unwrap().unwrap().state, "working");
        assert!(!e.turn_over(&r.id).unwrap());
        assert!(!e.infer_state(&r.id, false).unwrap().is_empty());
        assert_eq!(e.session_row(&r.id).unwrap().unwrap().state, "idle");
        assert!(e.turn_over(&r.id).unwrap());
        set(
            &e,
            &r.id,
            "UPDATE sessions SET state = 'waiting', waiting_kind = 'permission' WHERE id = ?",
        );
        assert!(
            e.infer_state(&r.id, true).unwrap().is_empty(),
            "waiting is never inferred away"
        );

        let r = launched(&e, &dir);
        e.record_turn_result(
            &r.id,
            &TurnResult {
                is_error: false,
                count_prompt: true,
            },
        )
        .unwrap();
        let row = e.session_row(&r.id).unwrap().unwrap();
        assert_eq!((row.state.as_str(), row.num_turns), ("unread", 1));
        e.record_turn_result(
            &r.id,
            &TurnResult {
                is_error: true,
                count_prompt: false,
            },
        )
        .unwrap();
        assert_eq!(e.session_row(&r.id).unwrap().unwrap().state, "failed");
        e.record_turn_result(
            &r.id,
            &TurnResult {
                is_error: false,
                count_prompt: false,
            },
        )
        .unwrap();
        assert_eq!(
            e.session_row(&r.id).unwrap().unwrap().state,
            "failed",
            "failed is sticky"
        );

        e.mark_no_hook_events(&r.id).unwrap();
        assert_eq!(
            e.session_row(&r.id)
                .unwrap()
                .unwrap()
                .state_reason
                .as_deref(),
            Some("no_hook_events")
        );
        assert!(
            !e.hook_event_since(&r.id, "2000-01-01T00:00:00.000Z", None)
                .unwrap()
        );
    }

    #[test]
    fn resume_reopens_only_without_hooks() {
        let (dir, e) = engine();
        let r = launched(&e, &dir);
        e.record_process_exit(
            &r.id,
            &ProcessExit {
                source: "pty",
                code: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        set(
            &e,
            &r.id,
            "UPDATE sessions SET tracking = 'hooked', launch_mode = 'external', dismissed_at = '2026-10-22T00:00:00.000Z' WHERE id = ?",
        );
        let prep = |reopen| ResumePrep {
            launch_mode: "embedded".into(),
            permission_mode: Some("acceptEdits".into()),
            cwd: None,
            reopen,
        };
        e.prepare_resume(&r.id, &prep(false)).unwrap();
        let row = e.session_row(&r.id).unwrap().unwrap();
        assert_eq!(
            (row.tracking.as_str(), row.launch_mode.as_str()),
            ("launched", "embedded")
        );
        assert_eq!(row.permission_mode.as_deref(), Some("acceptEdits"));
        assert!(row.dismissed_at.is_none(), "a resumed history row is back");
        assert!(row.ended_at.is_some(), "SessionStart(resume) reopens it");
        e.prepare_resume(&r.id, &prep(true)).unwrap();
        let row = e.session_row(&r.id).unwrap().unwrap();
        assert!(row.ended_at.is_none());
        assert_eq!(row.state, "idle");
        assert!(!e.set_launch_mode(&r.id, "headless").unwrap().is_empty());
        assert!(e.set_launch_mode(&r.id, "headless").unwrap().is_empty());
    }
}
