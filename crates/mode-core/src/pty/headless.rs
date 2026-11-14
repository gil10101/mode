//! Headless `stream-json` turns (05 §8.5): when a prompt goes to an embedded
//! session with no live PTY, Mode runs one `claude -p "<prompt>" --resume
//! <id> --output-format stream-json --verbose --include-partial-messages`
//! per prompt in the session's folder (same environment as a launch) and
//! consumes every event family of §8.5:
//!
//! | Event | What Mode does |
//! |---|---|
//! | `system/init` | confirms `session_id` (names a placeholder row), `model`; `mcp_servers` / `plugins` → `session_connectors` (§6A.1); one `headless.init` row (capabilities kept in its payload) |
//! | `assistant` / `user` | the same records the transcript holds: extracted and ingested like a transcript read (stable `tr:<uuid>` ids, so the tailer adds nothing twice); subagent messages (`parent_tool_use_id`) are left to the subagent transcript |
//! | `stream_event` text deltas | `turn.delta` (coalesced ~16 ms) for the streaming bubble |
//! | `system/api_retry` | an info line |
//! | `system/permission_denied` | an event row and a `denied` tool rule (origin `cli`) |
//! | `result` | usage / `num_turns` / `is_error` / `permission_denials` → event rows, `unread` or `failed`; no currency is read or stored |
//!
//! Hooks still run (non-bare), so approvals reach the same cards. "Stop
//! turn" is SIGINT; the CLI ends the turn and `result` still arrives.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rusqlite::Connection;
use serde_json::{Value, json};

use super::api::ApiError;
use super::driver::HandOff;
use super::launch::{self, ENV_REMOVE, Invocation, PromptVia};
use super::{Inner, TurnDelta, TurnFinished, TurnOutcome, TurnStarted, lock};
use crate::push;
use crate::redact;
use crate::repo::{self, Session};
use crate::sessions::embedded::{self, ProcessExit, ResumePrep, TurnResult};
use crate::sessions::naming;
use crate::sessions::writes::{EventIn, insert_event, upsert_connector};
use crate::settings;
use crate::transcript::records::{self, Record, Usage};

// ------------------------------------------------------------------- parser

/// An MCP server of `system/init`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub name: String,
    pub status: String,
}

/// A plugin of `system/init`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    pub name: String,
    pub version: Option<String>,
}

/// `system/init`.
#[derive(Debug, Clone, PartialEq)]
pub struct Init {
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub permission_mode: Option<String>,
    pub tools: Vec<String>,
    pub mcp_servers: Vec<McpServer>,
    pub plugins: Vec<Plugin>,
    pub capabilities: Option<Value>,
    pub cli_version: Option<String>,
}

/// One entry of `result.permission_denials` or a `system/permission_denied`.
#[derive(Debug, Clone, PartialEq)]
pub struct Denial {
    pub tool_name: String,
    pub tool_use_id: Option<String>,
    pub tool_input: Option<Value>,
    pub message: Option<String>,
}

/// The final `result`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultEv {
    pub subtype: String,
    pub is_error: bool,
    pub num_turns: Option<i64>,
    pub duration_ms: Option<i64>,
    pub usage: Option<Usage>,
    pub permission_denials: Vec<Denial>,
    pub session_id: Option<String>,
}

/// One stream-json line, by family.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Init(Box<Init>),
    /// `assistant` / `user`: the transcript record it duplicates.
    Message {
        record: Box<Record>,
        parent_tool_use_id: Option<String>,
    },
    /// `stream_event` / `content_block_delta` / `text_delta`.
    TextDelta {
        text: String,
        parent_tool_use_id: Option<String>,
    },
    /// Any other `stream_event` (message / block start and stop, thinking and
    /// tool-input deltas): nothing to store.
    Stream {
        kind: String,
    },
    ApiRetry {
        attempt: Option<i64>,
        max_retries: Option<i64>,
        retry_delay_ms: Option<i64>,
        error_status: Option<i64>,
        error: Option<String>,
    },
    PermissionDenied(Denial),
    Result(Box<ResultEv>),
    /// Another `system` subtype (`compact_boundary`, `status`, hook output).
    System {
        subtype: String,
    },
    Unknown {
        type_name: String,
    },
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn i(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(Value::as_i64)
}

fn denial(v: &Value) -> Denial {
    Denial {
        tool_name: s(v, "tool_name").unwrap_or_else(|| "tool".into()),
        tool_use_id: s(v, "tool_use_id"),
        tool_input: v.get("tool_input").cloned(),
        message: s(v, "message").or_else(|| s(v, "reason")),
    }
}

/// Parse one stream-json line; `None` for blank or non-JSON lines.
pub fn parse(line: &str) -> Option<StreamEvent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    v.as_object()?;
    let type_name = s(&v, "type").unwrap_or_default();
    let parent = s(&v, "parent_tool_use_id");
    Some(match type_name.as_str() {
        "system" => {
            let subtype = s(&v, "subtype").unwrap_or_default();
            match subtype.as_str() {
                "init" => StreamEvent::Init(Box::new(Init {
                    session_id: s(&v, "session_id"),
                    model: s(&v, "model"),
                    cwd: s(&v, "cwd"),
                    permission_mode: s(&v, "permissionMode").or_else(|| s(&v, "permission_mode")),
                    tools: v
                        .get("tools")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect()
                        })
                        .unwrap_or_default(),
                    mcp_servers: v
                        .get("mcp_servers")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|m| {
                                    Some(McpServer {
                                        name: s(m, "name")?,
                                        status: s(m, "status").unwrap_or_else(|| "pending".into()),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    plugins: v
                        .get("plugins")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|p| match p {
                                    Value::String(n) => Some(Plugin {
                                        name: n.clone(),
                                        version: None,
                                    }),
                                    _ => Some(Plugin {
                                        name: s(p, "name")?,
                                        version: s(p, "version"),
                                    }),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    capabilities: v.get("capabilities").cloned(),
                    cli_version: s(&v, "claude_code_version"),
                })),
                "api_retry" => StreamEvent::ApiRetry {
                    attempt: i(&v, "attempt"),
                    max_retries: i(&v, "max_retries"),
                    retry_delay_ms: i(&v, "retry_delay_ms"),
                    error_status: i(&v, "error_status"),
                    error: s(&v, "error"),
                },
                "permission_denied" => StreamEvent::PermissionDenied(denial(&v)),
                _ => StreamEvent::System { subtype },
            }
        }
        "assistant" | "user" => {
            let mut record = records::parse_line(line)?;
            if record.session_id.is_none() {
                record.session_id = s(&v, "session_id");
            }
            StreamEvent::Message {
                record: Box::new(record),
                parent_tool_use_id: parent,
            }
        }
        "stream_event" => {
            let ev = v.get("event").cloned().unwrap_or(Value::Null);
            let kind = s(&ev, "type").unwrap_or_default();
            let delta = ev.get("delta");
            let text = (kind == "content_block_delta")
                .then_some(delta)
                .flatten()
                .filter(|d| d.get("type").and_then(Value::as_str) == Some("text_delta"))
                .and_then(|d| s(d, "text"));
            match text {
                Some(text) => StreamEvent::TextDelta {
                    text,
                    parent_tool_use_id: parent,
                },
                None => StreamEvent::Stream { kind },
            }
        }
        "result" => StreamEvent::Result(Box::new(ResultEv {
            subtype: s(&v, "subtype").unwrap_or_else(|| "success".into()),
            is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
            num_turns: i(&v, "num_turns"),
            duration_ms: i(&v, "duration_ms"),
            usage: v.get("usage").map(|u| Usage {
                input: i(u, "input_tokens").unwrap_or(0),
                output: i(u, "output_tokens").unwrap_or(0),
                cache_read: i(u, "cache_read_input_tokens").unwrap_or(0),
                cache_write: i(u, "cache_creation_input_tokens").unwrap_or(0),
                cache_write_1h: records::cache_write_1h_of(
                    u,
                    i(u, "cache_creation_input_tokens").unwrap_or(0),
                ),
            }),
            permission_denials: v
                .get("permission_denials")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(denial).collect())
                .unwrap_or_default(),
            session_id: s(&v, "session_id"),
        })),
        _ => StreamEvent::Unknown { type_name },
    })
}

// ------------------------------------------------------------------ effects

/// `system/init` `mcp_servers[].status` → the tab's three words (§6A.1).
pub fn mcp_status(raw: &str) -> &'static str {
    match raw {
        "connected" => "connected",
        "pending" | "connecting" => "pending",
        _ => "failed",
    }
}

fn info_row(
    conn: &Connection,
    row_id: &str,
    event: &str,
    summary: String,
    tool: Option<(&str, Option<&str>)>,
    payload: Value,
    now: &str,
) -> crate::db::Result<i64> {
    insert_event(
        conn,
        EventIn {
            id: repo::new_id(),
            session_id: row_id,
            source: "headless",
            event,
            tool_name: tool.map(|(n, _)| n.to_string()),
            tool_use_id: tool.and_then(|(_, id)| id.map(str::to_string)),
            agent_id: None,
            state_before: None,
            state_after: None,
            summary: redact::scrub(&summary).into_owned(),
            payload_bytes: payload.to_string().len(),
            payload: Some(payload),
            occurred_at: now,
            received_at: now,
        },
    )
}

/// `system/init` into rows: model, the transcript path a placeholder row
/// lacked, connectors, one info row.
pub fn write_init(
    conn: &Connection,
    row_id: &str,
    init: &Init,
    now: &str,
) -> crate::db::Result<()> {
    let Some(row) = repo::get_by_id::<Session>(conn, row_id)? else {
        return Ok(());
    };
    if let Some(m) = &init.model {
        conn.execute(
            "UPDATE sessions SET model = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![row_id, m, now],
        )?;
    }
    // A row no hook named a transcript for (hooks absent) gets the CLI's
    // path once that file exists; a guess never blocks a later Resume.
    if row.transcript_path.is_none()
        && let (Some(home), Some(sid)) = (dirs::home_dir(), init.session_id.as_deref())
    {
        let cwd = init.cwd.as_deref().unwrap_or(&row.cwd);
        let path = crate::transcript::transcript_path(&home, cwd, sid);
        if path.is_file() {
            conn.execute(
                "UPDATE sessions SET transcript_path = ?2 WHERE id = ?1 AND transcript_path IS NULL",
                rusqlite::params![row_id, crate::paths::from_os(&path)],
            )?;
        }
    }
    for m in &init.mcp_servers {
        let prefix = format!("mcp__{}__", m.name);
        let tools = init.tools.iter().filter(|t| t.starts_with(&prefix)).count();
        upsert_connector(
            conn,
            row_id,
            "mcp_server",
            &m.name,
            mcp_status(&m.status),
            Some(json!({"tools": tools, "status": m.status, "source": "init"})),
            now,
        )?;
    }
    for p in &init.plugins {
        upsert_connector(
            conn,
            row_id,
            "plugin",
            &p.name,
            "connected",
            Some(json!({"version": p.version, "source": "init"})),
            now,
        )?;
    }
    info_row(
        conn,
        row_id,
        "headless.init",
        format!(
            "headless turn · {} tools · {} MCP servers",
            init.tools.len(),
            init.mcp_servers.len()
        ),
        None,
        json!({
            "session_id": init.session_id,
            "model": init.model,
            "permission_mode": init.permission_mode,
            "tools": init.tools.len(),
            "mcp_servers": init.mcp_servers.iter().map(|m| json!({"name": m.name, "status": m.status})).collect::<Vec<_>>(),
            "plugins": init.plugins.iter().map(|p| json!({"name": p.name, "version": p.version})).collect::<Vec<_>>(),
            "capabilities": init.capabilities,
            "cli_version": init.cli_version,
        }),
        now,
    )?;
    Ok(())
}

/// A denial (system event or `result.permission_denials`): one event row per
/// tool use, and the tool's `denied` rule (§6A.1, origin `cli`).
pub fn write_denial(
    conn: &Connection,
    row_id: &str,
    d: &Denial,
    base: Option<&str>,
    now: &str,
) -> crate::db::Result<bool> {
    if let Some(id) = &d.tool_use_id {
        let seen: Option<i64> = rusqlite::OptionalExtension::optional(conn.query_row(
            "SELECT 1 FROM session_events WHERE session_id = ? AND event = 'headless.permission_denied' AND tool_use_id = ?",
            rusqlite::params![row_id, id],
            |r| r.get(0),
        ))?;
        if seen.is_some() {
            return Ok(false);
        }
    }
    let what = naming::tool_summary(&d.tool_name, d.tool_input.as_ref(), base);
    let summary = match &d.message {
        Some(m) if !m.trim().is_empty() => {
            format!("denied · {what} · {}", naming::truncate_chars(m, 200))
        }
        _ => format!("denied · {what}"),
    };
    info_row(
        conn,
        row_id,
        "headless.permission_denied",
        summary,
        Some((&d.tool_name, d.tool_use_id.as_deref())),
        json!({
            "tool_input": d.tool_input.as_ref().map(redact::tool_field_for_storage),
            "message": d.message,
        }),
        now,
    )?;
    upsert_connector(
        conn,
        row_id,
        "tool_rule",
        &d.tool_name,
        "denied",
        Some(json!({"origin": "cli"})),
        now,
    )?;
    Ok(true)
}

fn seconds(ms: Option<i64>) -> String {
    match ms {
        Some(ms) if ms >= 0 => naming::elapsed_text(ms / 1000),
        _ => "?".into(),
    }
}

/// `result` into rows (the state change is the engine's,
/// [`crate::sessions::Engine::record_turn_result`]).
pub fn write_result(
    conn: &Connection,
    row_id: &str,
    r: &ResultEv,
    base: Option<&str>,
    now: &str,
) -> crate::db::Result<()> {
    let summary = if r.is_error {
        format!("turn failed · {}", r.subtype)
    } else {
        format!(
            "turn finished · {} turns · {}",
            r.num_turns.unwrap_or(0),
            seconds(r.duration_ms)
        )
    };
    info_row(
        conn,
        row_id,
        "headless.result",
        summary,
        None,
        json!({
            "subtype": r.subtype,
            "is_error": r.is_error,
            "num_turns": r.num_turns,
            "duration_ms": r.duration_ms,
            "usage": r.usage.map(|u| json!({
                "input_tokens": u.input,
                "output_tokens": u.output,
                "cache_read_tokens": u.cache_read,
                "cache_write_tokens": u.cache_write,
            })),
            "permission_denials": r.permission_denials.len(),
        }),
        now,
    )?;
    for d in &r.permission_denials {
        write_denial(conn, row_id, d, base, now)?;
    }
    Ok(())
}

/// `assistant` / `user` records into the thread, exactly as a transcript
/// read would write them.
pub fn write_records(
    conn: &Connection,
    row_id: &str,
    recs: &[Record],
    now: &str,
) -> crate::db::Result<bool> {
    let Some(row) = repo::get_by_id::<Session>(conn, row_id)? else {
        return Ok(false);
    };
    let keep = settings::get_bool_in(conn, "sessions.store_prompt_previews")?.unwrap_or(false);
    let custom: HashMap<String, String> =
        settings::get_as_in(conn, "sessions.custom_names").unwrap_or_default();
    let mut recs = recs.to_vec();
    for r in &mut recs {
        r.timestamp.get_or_insert_with(|| now.to_string());
    }
    let extracted = records::extract(&recs, keep);
    let report = crate::transcript::ingest(
        conn,
        row_id,
        &extracted,
        custom.contains_key(&row.claude_session_id),
        now,
    )?;
    Ok(report.changed)
}

// ------------------------------------------------------------------- turns

/// A running headless turn.
#[derive(Debug)]
pub struct Turn {
    pub turn_id: String,
    pub session_id: String,
    pub launch_id: String,
    pub pid: u32,
    pub generation: u64,
    pub started_at: String,
    pub stopped: AtomicBool,
}

/// The running turns, by session.
#[derive(Debug, Default)]
pub struct Turns {
    map: Mutex<HashMap<String, Arc<Turn>>>,
}

impl Turns {
    pub fn get(&self, session_id: &str) -> Option<Arc<Turn>> {
        lock(&self.map).get(session_id).cloned()
    }

    pub fn running(&self) -> Vec<String> {
        lock(&self.map).keys().cloned().collect()
    }

    fn insert(&self, t: Arc<Turn>) {
        lock(&self.map).insert(t.session_id.clone(), t);
    }

    fn remove(&self, session_id: &str, turn_id: &str) {
        let mut m = lock(&self.map);
        if m.get(session_id).is_some_and(|t| t.turn_id == turn_id) {
            m.remove(session_id);
        }
    }
}

/// SIGINT to the headless `claude` (unix); CTRL_BREAK to its process group
/// on Windows, where a console-less child cannot receive Ctrl+C. Never
/// SIGTERM.
fn interrupt(pid: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        // SAFETY: plain kill(2) on the child's pid.
        let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGINT) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, GenerateConsoleCtrlEvent};
        // SAFETY: plain Win32 call with a process-group id.
        let ok = unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        Err(std::io::Error::other("no interrupt on this platform"))
    }
}

impl Inner {
    /// Start a headless turn (the driver is already `headless` with
    /// `generation`); `turn.started` is emitted before this returns.
    pub(crate) fn start_turn(
        self: &Arc<Self>,
        row: &Session,
        prompt: &str,
        generation: u64,
    ) -> Result<String, ApiError> {
        let fail = |inner: &Arc<Self>, e: ApiError| {
            inner
                .drivers
                .release(&row.id, HandOff::TurnEnded, generation);
            e
        };
        let cwd =
            launch::check_dir(&row.cwd).map_err(|e| fail(self, ApiError::new("cwd_missing", e)))?;
        let hooks = self.engine.hooks_installed().unwrap_or(false);
        let placeholder = embedded::is_placeholder(&row.claude_session_id);
        let first = if placeholder {
            lock(&self.first_turn)
                .get(&row.id)
                .cloned()
                .unwrap_or_default()
        } else {
            Default::default()
        };
        // The agent profile's flags (03 §7E): a placeholder row keeps its
        // launch's, a session that ran gets its agent's current ones.
        let flags = if placeholder {
            first.flags.clone()
        } else {
            self.flags_for(row.profile_id.as_deref())
                .map_err(|e| fail(self, e))?
        }
        .for_cwd(&cwd);
        let claude = self.claude_for(&flags).map_err(|e| fail(self, e))?;
        let via = launch::prompt_via(prompt, claude.is_shim());
        let args = launch::build_args_with(
            &Invocation::Headless {
                prompt: (via == PromptVia::Arg).then_some(prompt),
                resume: (!placeholder).then_some(row.claude_session_id.as_str()),
                permission_mode: first.permission_mode.as_deref(),
                model: first.model.as_deref(),
            },
            &flags,
        );
        let launch_id = uuid::Uuid::now_v7().to_string();
        self.engine
            .embedded()
            .register(&launch_id, &row.id, "headless");
        self.engine_pushes(
            "headless start",
            self.engine.prepare_resume(
                &row.id,
                &ResumePrep {
                    launch_mode: "headless".into(),
                    permission_mode: None,
                    cwd: None,
                    reopen: !hooks || placeholder,
                },
            ),
        );
        let mut cmd = Command::new(&claude.program);
        cmd.args(&claude.prefix)
            .args(&args)
            .current_dir(&cwd)
            .stdin(if via == PromptVia::Stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for k in ENV_REMOVE {
            cmd.env_remove(k);
        }
        cmd.envs(self.env_for(&launch_id));
        cmd.envs(flags.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                self.engine.embedded().forget(&launch_id);
                self.engine_pushes(
                    "headless start",
                    self.engine.set_launch_mode(&row.id, "embedded"),
                );
                return Err(fail(
                    self,
                    ApiError::io(format!("claude did not start: {e}")),
                ));
            }
        };
        if via == PromptVia::Stdin {
            if let Some(mut stdin) = child.stdin.take() {
                let text = prompt.to_string();
                std::thread::spawn(move || {
                    let _ = stdin.write_all(text.as_bytes());
                });
            }
        }
        lock(&self.first_turn).remove(&row.id);
        let turn = Arc::new(Turn {
            turn_id: uuid::Uuid::now_v7().to_string(),
            session_id: row.id.clone(),
            launch_id,
            pid: child.id(),
            generation,
            started_at: crate::db::now_rfc3339(),
            stopped: AtomicBool::new(false),
        });
        self.turns.insert(Arc::clone(&turn));
        self.emit(
            push::Event::TurnStarted,
            &TurnStarted {
                session_id: row.id.clone(),
                turn_id: turn.turn_id.clone(),
            },
        );
        self.publish(&[push::Event::Sessions, push::Event::Pill]);
        let inner = Arc::clone(self);
        let t = Arc::clone(&turn);
        let base = row.repo_root.clone().or_else(|| Some(row.cwd.clone()));
        std::thread::Builder::new()
            .name("mode-headless".into())
            .spawn(move || run(inner, t, child, base))
            .map_err(ApiError::io)?;
        tracing::info!(target: "pty", session = %row.id, turn = %turn.turn_id, "headless turn started");
        Ok(turn.turn_id.clone())
    }

    /// "Stop turn": SIGINT; `turn.finished` with `stopped` follows.
    pub(crate) fn stop_turn(&self, session_id: &str) -> Result<(), ApiError> {
        let t = self
            .turns
            .get(session_id)
            .ok_or_else(|| ApiError::not_found("no turn is running"))?;
        t.stopped.store(true, Ordering::SeqCst);
        interrupt(t.pid).map_err(ApiError::io)
    }
}

/// Text deltas waiting for the next ~16 ms frame.
struct Deltas {
    text: String,
    parent: Option<String>,
    since: Option<Instant>,
}

fn run(inner: Arc<Inner>, turn: Arc<Turn>, mut child: Child, base: Option<String>) {
    let row = turn.session_id.clone();
    let stderr_tail = Arc::new(Mutex::new(Vec::<String>::new()));
    let tail = Arc::clone(&stderr_tail);
    let err_thread = child.stderr.take().map(|e| {
        std::thread::spawn(move || {
            for line in BufReader::new(e).lines().map_while(Result::ok) {
                let mut t = lock(&tail);
                t.push(line);
                if t.len() > 20 {
                    t.remove(0);
                }
            }
        })
    });
    let mut result: Option<ResultEv> = None;
    let mut deltas = Deltas {
        text: String::new(),
        parent: None,
        since: None,
    };
    let coalesce = inner.config.timings.coalesce;
    let flush = |d: &mut Deltas| {
        if !d.text.is_empty() {
            inner.emit(
                push::Event::TurnDelta,
                &TurnDelta {
                    session_id: row.clone(),
                    turn_id: turn.turn_id.clone(),
                    text: std::mem::take(&mut d.text),
                    parent_tool_use_id: d.parent.take(),
                },
            );
        }
        d.since = None;
    };
    if let Some(out) = child.stdout.take() {
        let mut reader = BufReader::new(out);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader
                .by_ref()
                .take(64 * 1024 * 1024)
                .read_until(b'\n', &mut buf)
            {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = String::from_utf8_lossy(&buf);
            let Some(ev) = parse(&line) else { continue };
            if let StreamEvent::TextDelta {
                text,
                parent_tool_use_id,
            } = &ev
            {
                if !deltas.text.is_empty() && deltas.parent != *parent_tool_use_id {
                    flush(&mut deltas);
                }
                deltas.text.push_str(text);
                deltas.parent.clone_from(parent_tool_use_id);
                let since = *deltas.since.get_or_insert_with(Instant::now);
                if since.elapsed() >= coalesce {
                    flush(&mut deltas);
                }
                continue;
            }
            flush(&mut deltas);
            if let StreamEvent::Result(r) = &ev {
                result = Some((**r).clone());
            }
            apply(&inner, &turn, &row, base.as_deref(), ev);
        }
    }
    flush(&mut deltas);
    let status = child.wait();
    if let Some(t) = err_thread {
        let _ = t.join();
    }
    let exit = match &status {
        Ok(s) => exit_of(s),
        Err(_) => (None, Some("unknown".to_string())),
    };
    let stopped = turn.stopped.load(Ordering::SeqCst);
    let outcome = if stopped {
        TurnOutcome::Stopped
    } else if result.as_ref().map_or(exit.0 != Some(0), |r| r.is_error) {
        TurnOutcome::Failed
    } else {
        TurnOutcome::Finished
    };
    if result.is_none() && !stopped {
        // No `result`: the stderr line is the only explanation (05 §16).
        let last = lock(&stderr_tail).last().cloned().unwrap_or_default();
        let r = row.clone();
        let now = crate::db::now_rfc3339();
        let summary = if last.trim().is_empty() {
            "the turn ended without a result".to_string()
        } else {
            format!(
                "the turn ended without a result · {}",
                naming::truncate_chars(last.trim(), 300)
            )
        };
        let written = inner.engine.db().write(move |c| {
            info_row(
                c,
                &r,
                "headless.error",
                summary,
                None,
                json!({"code": exit.0}),
                &now,
            )
        });
        if let Err(e) = written {
            tracing::warn!(target: "pty", error = %e, "headless error row failed");
        }
        inner.engine_pushes(
            "headless failure",
            inner.engine.record_turn_result(
                &row,
                &TurnResult {
                    is_error: true,
                    count_prompt: false,
                },
            ),
        );
    }
    inner.turns.remove(&row, &turn.turn_id);
    inner.engine.embedded().forget(&turn.launch_id);
    inner.engine_pushes(
        "headless end",
        inner.engine.set_launch_mode(&row, "embedded"),
    );
    inner
        .drivers
        .release(&row, HandOff::TurnEnded, turn.generation);
    inner.emit(
        push::Event::TurnFinished,
        &TurnFinished {
            session_id: row.clone(),
            turn_id: turn.turn_id.clone(),
            outcome,
        },
    );
    inner.publish(&[push::Event::Sessions, push::Event::Pill]);
    tracing::info!(target: "pty", session = %row, turn = %turn.turn_id, ?outcome, "headless turn finished");
    // Close the row like a PTY exit when no `SessionEnd` did (hooks absent),
    // unless another turn or a terminal took over meanwhile.
    std::thread::sleep(inner.config.timings.session_end_grace);
    if inner.drivers.generation(&row) == turn.generation
        && inner.drivers.get(&row) == super::Driver::None
    {
        // "Stop turn": the turn ends cleanly (05 §8.5), whatever the code.
        let pe = ProcessExit {
            source: "headless",
            code: if stopped { Some(0) } else { exit.0 },
            signal: if stopped { None } else { exit.1 },
            interrupted_by_mode: stopped,
            error: None,
        };
        inner.engine_pushes("headless exit", inner.engine.record_process_exit(&row, &pe));
    }
}

fn exit_of(s: &std::process::ExitStatus) -> (Option<i32>, Option<String>) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = s.signal() {
            let name = if sig == libc::SIGINT {
                "SIGINT".to_string()
            } else {
                format!("signal {sig}")
            };
            return (None, Some(name));
        }
    }
    (s.code(), None)
}

/// What one non-delta event writes.
fn apply(inner: &Arc<Inner>, turn: &Turn, row: &str, base: Option<&str>, ev: StreamEvent) {
    let db = inner.engine.db();
    let now = crate::db::now_rfc3339();
    let r = row.to_string();
    let base = base.map(str::to_string);
    let res: crate::db::Result<Vec<push::Event>> = match ev {
        StreamEvent::Init(init) => {
            if let Some(sid) = init.session_id.as_deref() {
                if let Err(e) = inner.engine.attach_claude_session(row, sid) {
                    tracing::warn!(target: "pty", error = %e, "attach session id failed");
                }
            }
            db.write(move |c| {
                repo::tx(c, |tx| write_init(tx, &r, &init, &now))?;
                Ok(vec![push::Event::Sessions])
            })
        }
        StreamEvent::Message {
            record,
            parent_tool_use_id,
        } => {
            if parent_tool_use_id.is_some() {
                // The subagent's own transcript feeds its activity group.
                Ok(Vec::new())
            } else {
                db.write(move |c| {
                    let changed = repo::tx(c, |tx| write_records(tx, &r, &[*record], &now))?;
                    Ok(if changed {
                        vec![push::Event::Sessions]
                    } else {
                        Vec::new()
                    })
                })
            }
        }
        StreamEvent::ApiRetry {
            attempt,
            max_retries,
            retry_delay_ms,
            error_status,
            error,
        } => db.write(move |c| {
            let mut summary = format!(
                "API retry · attempt {} of {}",
                attempt.map_or("?".into(), |a| a.to_string()),
                max_retries.map_or("?".into(), |a| a.to_string())
            );
            if let Some(st) = error_status {
                summary.push_str(&format!(" · {st}"));
            }
            if let Some(e) = &error {
                summary.push_str(&format!(" {e}"));
            }
            if let Some(ms) = retry_delay_ms {
                summary.push_str(&format!(" · in {}", seconds(Some(ms.max(1000)))));
            }
            info_row(
                c,
                &r,
                "headless.api_retry",
                summary,
                None,
                json!({"attempt": attempt, "max_retries": max_retries, "retry_delay_ms": retry_delay_ms, "error_status": error_status, "error": error}),
                &now,
            )?;
            Ok(vec![push::Event::Sessions])
        }),
        StreamEvent::PermissionDenied(d) => db.write(move |c| {
            repo::tx(c, |tx| write_denial(tx, &r, &d, base.as_deref(), &now))?;
            Ok(vec![push::Event::Sessions])
        }),
        StreamEvent::Result(res) => {
            let res2 = (*res).clone();
            let written = db.write(move |c| {
                repo::tx(c, |tx| write_result(tx, &r, &res2, base.as_deref(), &now))
            });
            if let Err(e) = written {
                tracing::warn!(target: "pty", error = %e, "result rows failed");
            }
            let counted = inner
                .engine
                .hook_event_since(row, &turn.started_at, Some(mode_wire::events::USER_PROMPT_SUBMIT))
                .unwrap_or(false);
            // A turn the user stopped is not a failure, whatever the CLI
            // reports for the interrupted turn.
            let stopped = turn.stopped.load(Ordering::SeqCst);
            inner.engine.record_turn_result(
                row,
                &TurnResult {
                    is_error: res.is_error && !stopped,
                    count_prompt: !counted,
                },
            )
        }
        StreamEvent::TextDelta { .. }
        | StreamEvent::Stream { .. }
        | StreamEvent::System { .. }
        | StreamEvent::Unknown { .. } => Ok(Vec::new()),
    };
    inner.engine_pushes("headless event", res);
}

/// Parse a whole recorded stream (tests, diagnostics).
pub fn parse_all(text: &str) -> Vec<StreamEvent> {
    text.lines().filter_map(parse).collect()
}

/// Feed recorded events for a row as a turn would (tests, replays): every
/// non-delta event written, deltas returned joined.
pub fn replay(
    conn: &Connection,
    row_id: &str,
    events: &[StreamEvent],
    now: &str,
) -> crate::db::Result<String> {
    let base: Option<String> =
        repo::get_by_id::<Session>(conn, row_id)?.map(|r| r.repo_root.unwrap_or(r.cwd));
    let mut text = String::new();
    for ev in events {
        match ev {
            StreamEvent::Init(init) => {
                if let Some(sid) = init.session_id.as_deref() {
                    embedded::attach_claude_id(conn, row_id, sid)?;
                }
                write_init(conn, row_id, init, now)?;
            }
            StreamEvent::Message {
                record,
                parent_tool_use_id: None,
            } => {
                write_records(conn, row_id, std::slice::from_ref(record.as_ref()), now)?;
            }
            StreamEvent::TextDelta {
                text: t,
                parent_tool_use_id: None,
            } => text.push_str(t),
            StreamEvent::PermissionDenied(d) => {
                write_denial(conn, row_id, d, base.as_deref(), now)?;
            }
            StreamEvent::Result(r) => {
                write_result(conn, row_id, r, base.as_deref(), now)?;
                embedded::record_turn_result(
                    conn,
                    row_id,
                    &TurnResult {
                        is_error: r.is_error,
                        count_prompt: true,
                    },
                    now,
                )?;
            }
            StreamEvent::ApiRetry {
                attempt,
                max_retries,
                ..
            } => {
                info_row(
                    conn,
                    row_id,
                    "headless.api_retry",
                    format!(
                        "API retry · attempt {} of {}",
                        attempt.unwrap_or(0),
                        max_retries.unwrap_or(0)
                    ),
                    None,
                    json!({}),
                    now,
                )?;
            }
            _ => {}
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_parse() {
        let init = parse(r#"{"type":"system","subtype":"init","session_id":"s1","tools":["Bash","mcp__gh__issue","mcp__gh__pr"],"mcp_servers":[{"name":"gh","status":"connected"},{"name":"db","status":"needs-auth"}],"plugins":[{"name":"p","version":"1.0"},"q"],"permissionMode":"default"}"#).unwrap();
        let StreamEvent::Init(init) = init else {
            panic!("init")
        };
        assert_eq!(init.session_id.as_deref(), Some("s1"));
        assert_eq!(init.mcp_servers.len(), 2);
        assert_eq!(init.plugins[1].name, "q");
        assert_eq!(mcp_status(&init.mcp_servers[1].status), "failed");
        let d = parse(r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}},"parent_tool_use_id":null}"#).unwrap();
        assert_eq!(
            d,
            StreamEvent::TextDelta {
                text: "Hi".into(),
                parent_tool_use_id: None
            }
        );
        let t = parse(r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{"}}}"#).unwrap();
        assert_eq!(
            t,
            StreamEvent::Stream {
                kind: "content_block_delta".into()
            }
        );
        let r = parse(r#"{"type":"result","subtype":"error_max_turns","is_error":true,"num_turns":9,"total_cost_usd":1.5,"usage":{"input_tokens":3,"output_tokens":4},"permission_denials":[{"tool_name":"Bash","tool_use_id":"tu1","tool_input":{"command":"rm"}}]}"#).unwrap();
        let StreamEvent::Result(r) = r else {
            panic!("result")
        };
        assert!(r.is_error);
        assert_eq!(r.num_turns, Some(9));
        assert_eq!(r.usage.unwrap().output, 4);
        assert_eq!(r.permission_denials[0].tool_use_id.as_deref(), Some("tu1"));
        assert!(matches!(
            parse(r#"{"type":"system","subtype":"api_retry","attempt":2,"max_retries":10}"#),
            Some(StreamEvent::ApiRetry {
                attempt: Some(2),
                ..
            })
        ));
        assert!(matches!(
            parse(r#"{"type":"mystery"}"#),
            Some(StreamEvent::Unknown { .. })
        ));
        assert_eq!(parse("not json"), None);
        assert_eq!(parse("   "), None);
    }
}
