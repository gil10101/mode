//! `mode-hook`: the Claude Code hook relay (05 §3.3).
//!
//! `mode-hook <Event> [timeout_seconds]` reads the hook JSON from stdin, sends
//! it to the running Mode instance as one NDJSON frame, waits for a `decision`
//! only for the two decision events, prints the decision's `output` to stdout
//! and exits 0 on **every** path. No Mode within 300 ms → exit 0 silently, so
//! Claude Code behaves exactly as if no hook were installed.
//!
//! `timeout_seconds` is the `timeout` the installer registered for the event
//! (05 §3.1); the relay gives up `2 s` before it so Claude Code never has to
//! kill it. `MODE_HOOK_TIMEOUT` is the fallback, then 120 s.
//!
//! `mode-hook --version` prints the relay version (the installer's status
//! check). Diagnostics go to stderr only when `MODE_HOOK_DEBUG=1`.

#![deny(unsafe_code)]

#[cfg(unix)]
#[path = "unix.rs"]
mod transport;
#[cfg(windows)]
#[path = "windows.rs"]
mod transport;

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use mode_wire::{
    CancelFrame, CancelReason, Endpoint, Frame, HookEvent, HookFrame, MAX_FRAME_BYTES,
    PROTOCOL_VERSION, scan, wants_decision,
};
use serde_json::Value;

/// Mode not running → give up after this (05 §3.3).
const CONNECT_TIMEOUT: Duration = Duration::from_millis(300);
/// Non-decision events wait this long for the `ack`, then exit regardless.
const ACK_WAIT: Duration = Duration::from_secs(1);
/// Claude Code's registered hook timeout when none is passed (05 §3.1).
const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// The relay exits this much before Claude Code's own timeout would kill it.
const DEADLINE_MARGIN: Duration = Duration::from_secs(2);
/// Never wait less than this for a decision, whatever the configured timeout.
const MIN_DECISION_WAIT: Duration = Duration::from_secs(1);
/// Environment variables forwarded to the core (05 §9.2 allow-list).
const ENV_ALLOW_LIST: &[&str] = &[
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "TERM_SESSION_ID",
    "ITERM_SESSION_ID",
    "WT_SESSION",
    "WT_PROFILE_ID",
    "KITTY_WINDOW_ID",
    "GHOSTTY_RESOURCES_DIR",
    "ALACRITTY_WINDOW_ID",
    "WEZTERM_PANE",
    "TMUX_PANE",
    "VSCODE_PID",
    "WSL_DISTRO_NAME",
    "WSL_INTEROP",
    "CLAUDE_CONFIG_DIR",
];

fn main() {
    // Exit 0 on every path, including a panic.
    std::panic::set_hook(Box::new(|info| debug(&format!("panic: {info}"))));
    let _ = std::panic::catch_unwind(run);
    // Kills the reader and drain threads too; nothing is left to flush.
    std::process::exit(0);
}

fn debug(msg: &str) {
    if std::env::var("MODE_HOOK_DEBUG").is_ok_and(|v| v == "1") {
        eprintln!("mode-hook: {msg}");
    }
}

/// The decision wait for a registered `timeout` (05 §3.3: `timeout − 2 s`).
fn decision_wait(timeout_secs: u64) -> Duration {
    Duration::from_secs(timeout_secs)
        .saturating_sub(DEADLINE_MARGIN)
        .max(MIN_DECISION_WAIT)
}

/// Read stdin up to the cap. Beyond it the rest is drained on a helper thread
/// (so Claude Code never sees a broken pipe) and `truncated` is set.
fn read_stdin() -> std::io::Result<(Vec<u8>, bool)> {
    let mut raw = Vec::with_capacity(64 * 1024);
    let mut stdin = std::io::stdin().lock();
    (&mut stdin)
        .take(MAX_FRAME_BYTES as u64 + 1)
        .read_to_end(&mut raw)?;
    let truncated = raw.len() > MAX_FRAME_BYTES;
    if truncated {
        raw.truncate(MAX_FRAME_BYTES);
        drop(stdin);
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin().lock(), &mut std::io::sink());
        });
    }
    Ok((raw, truncated))
}

/// The payload of a cut-off document: every top-level member that is still
/// complete (the common fields come first in Claude Code's payloads; the huge
/// `tool_response` comes last). `None` when not even `session_id` survived.
fn salvage(prefix: &[u8]) -> Option<HookEvent> {
    let open = scan::first_value(prefix)?;
    let obj = scan::scan_object(prefix, open)?;
    let mut map = serde_json::Map::new();
    for m in obj.members {
        if let Ok(v) = serde_json::from_slice::<Value>(&prefix[m.value.clone()]) {
            map.insert(m.key, v);
        }
    }
    serde_json::from_value(Value::Object(map)).ok()
}

fn run() {
    let args: Vec<String> = std::env::args().collect();
    if matches!(args.get(1).map(String::as_str), Some("--version" | "-V")) {
        println!("mode-hook {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let event_arg = args.get(1).filter(|s| !s.is_empty()).cloned();
    let timeout_secs = args
        .get(2)
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| {
            std::env::var("MODE_HOOK_TIMEOUT")
                .ok()
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    // 1. stdin, capped at 8 MB (05 §3.3: forwarded truncated beyond).
    let (raw, truncated) = match read_stdin() {
        Ok(r) => r,
        Err(e) => {
            debug(&format!("stdin: {e}"));
            return;
        }
    };
    let payload: HookEvent = match serde_json::from_slice(&raw) {
        Ok(p) => p,
        Err(e) if truncated => match salvage(&raw) {
            Some(p) => {
                debug("payload over 8 MB: forwarding the complete fields only");
                p
            }
            None => {
                debug(&format!("payload over 8 MB and nothing to salvage ({e})"));
                return;
            }
        },
        Err(e) => {
            debug(&format!("payload is not a hook event ({e})"));
            return;
        }
    };
    let event = event_arg.unwrap_or_else(|| payload.hook_event_name.clone());
    let wants_decision = wants_decision(&event, payload.tool_name());
    let wait = decision_wait(timeout_secs);
    let id = mode_wire::id::uuid_v7();

    let frame = HookFrame {
        v: PROTOCOL_VERSION,
        id: id.clone(),
        event: event.clone(),
        relay_version: env!("CARGO_PKG_VERSION").to_string(),
        wants_decision,
        sent_at: mode_wire::time::now_rfc3339(),
        ppid: transport::parent_pid(),
        tty: transport::tty_name(),
        env: allow_listed_env(),
        launch_id: std::env::var("MODE_LAUNCH_ID")
            .ok()
            .filter(|s| !s.is_empty()),
        truncated,
        decision_wait_ms: wants_decision.then_some(wait.as_millis() as u64),
        payload,
    };

    // 2. connect, 300 ms budget.
    let endpoint = Endpoint::from_env();
    let started = Instant::now();
    let Some(mut conn) = transport::connect(&endpoint, CONNECT_TIMEOUT) else {
        debug(&format!(
            "no Mode at {} after {:?}",
            endpoint.describe(),
            started.elapsed()
        ));
        return;
    };
    debug(&format!(
        "connected to {} in {:?}",
        endpoint.describe(),
        started.elapsed()
    ));

    // 3. send the hook frame.
    let line = match Frame::hook(frame).to_line() {
        Ok(l) => l,
        Err(e) => {
            debug(&format!("serialize: {e}"));
            return;
        }
    };
    if let Err(e) = conn.write_all(line.as_bytes()).and_then(|_| conn.flush()) {
        debug(&format!("write: {e}"));
        return;
    }

    // 4. frames from the core arrive on a reader thread; `None` means EOF.
    let reader = match conn.try_clone() {
        Ok(r) => r,
        Err(e) => {
            debug(&format!("clone: {e}"));
            return;
        }
    };
    let (tx, rx) = mpsc::channel::<Option<Frame>>();
    std::thread::spawn(move || {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next() {
                Some(Ok(l)) => match Frame::from_line(&l) {
                    Ok(f) => {
                        if tx.send(Some(f)).is_err() {
                            return;
                        }
                    }
                    Err(e) => debug(&format!("bad frame from core: {e}")),
                },
                _ => {
                    let _ = tx.send(None);
                    return;
                }
            }
        }
    });

    if !wants_decision {
        match rx.recv_timeout(ACK_WAIT) {
            Ok(Some(Frame::Ack(a))) if a.id == id => debug("acked"),
            other => debug(&format!("no ack: {other:?}")),
        }
        return;
    }

    // 5. decision wait, bounded so the relay always exits before Claude Code's timeout.
    let deadline = Instant::now() + wait;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(Some(Frame::Ack(a))) if a.id == id => debug("acked"),
            Ok(Some(Frame::Decision(d))) if d.id == id => {
                let mut out = std::io::stdout().lock();
                let _ = serde_json::to_writer(&mut out, &d.output);
                let _ = out.write_all(b"\n");
                let _ = out.flush();
                debug("decision printed");
                return;
            }
            Ok(Some(Frame::NoOpinion(n))) if n.id == id => {
                debug("no opinion from core");
                return;
            }
            Ok(Some(other)) => debug(&format!("ignored frame: {other:?}")),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                debug("core closed the connection before a decision");
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                debug(&format!("deadline after {wait:?}; canceling"));
                if let Ok(l) =
                    Frame::Cancel(CancelFrame::new(id.clone(), CancelReason::Deadline)).to_line()
                {
                    let _ = conn.write_all(l.as_bytes()).and_then(|_| conn.flush());
                }
                return;
            }
        }
    }
}

fn allow_listed_env() -> BTreeMap<String, String> {
    ENV_ALLOW_LIST
        .iter()
        .filter_map(|k| {
            std::env::var(k)
                .ok()
                .filter(|v| !v.is_empty())
                .map(|v| (k.to_string(), v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_wait_keeps_the_two_second_margin_with_a_floor() {
        assert_eq!(decision_wait(120), Duration::from_secs(118));
        assert_eq!(decision_wait(3), Duration::from_secs(1));
        assert_eq!(decision_wait(2), MIN_DECISION_WAIT);
        assert_eq!(decision_wait(0), MIN_DECISION_WAIT);
    }

    #[test]
    fn salvage_keeps_the_complete_top_level_fields() {
        let full = r#"{"session_id":"s1","transcript_path":"/t","cwd":"/c","hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"cat big"},"tool_use_id":"toolu_1","tool_response":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;
        let cut = &full.as_bytes()[..full.len() - 30];
        let ev = salvage(cut).unwrap();
        assert_eq!(ev.session_id, "s1");
        assert_eq!(ev.hook_event_name, "PostToolUse");
        assert_eq!(ev.tool_name(), Some("Bash"));
        assert_eq!(ev.tool_use_id(), Some("toolu_1"));
        assert_eq!(ev.get("tool_input").unwrap()["command"], "cat big");
        assert!(
            ev.get("tool_response").is_none(),
            "the cut field is dropped"
        );
        assert!(
            salvage(b"{\"hook_event_name\": \"Stop\", \"x\": \"aaaa").is_none(),
            "no session_id"
        );
        assert!(salvage(b"garbage").is_none());
    }
}
