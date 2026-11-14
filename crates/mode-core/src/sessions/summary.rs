//! The one-line `session_events.summary` of a hook event (05 §4.2 "summary"
//! column): "started · resume · claude-opus-5-5", "Edit src/main.rs (+12 −3)",
//! "turn failed · …". Secrets are scrubbed; nothing here is longer than a line.

use mode_wire::{HookEvent, events};
use serde_json::Value;

use super::naming;
use crate::redact;

fn first_line(s: &str, max: usize) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    redact::scrub(&naming::truncate_chars(line, max)).into_owned()
}

fn str_field(ev: &HookEvent, key: &str) -> Option<String> {
    ev.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Milliseconds as "12ms", "1.2s", "2m 5s".
pub fn duration_text(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let s = ms / 1000;
        format!("{}m {}s", s / 60, s % 60)
    }
}

/// The summary of a hook event. `base` is the repo root (or cwd) paths are
/// shown relative to; `file_delta` is the `(+N −M)` of a writing tool's
/// `PostToolUse` once the row was written.
pub fn event_summary(
    ev: &HookEvent,
    base: Option<&str>,
    keep_prompts: bool,
    file_delta: Option<(u32, u32)>,
) -> String {
    let tool = ev.tool_name().unwrap_or("tool");
    let input = ev.get("tool_input");
    match ev.hook_event_name.as_str() {
        events::SESSION_START => {
            let mut s = format!("started · {}", ev.source.as_deref().unwrap_or("startup"));
            if let Some(m) = str_field(ev, "model") {
                s.push_str(" · ");
                s.push_str(&m);
            }
            s
        }
        events::USER_PROMPT_SUBMIT => {
            if keep_prompts {
                str_field(ev, "prompt")
                    .map(|p| first_line(&p, 200))
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| "prompt".into())
            } else {
                "prompt".into()
            }
        }
        events::PRE_TOOL_USE => naming::tool_summary(tool, input, base),
        events::PERMISSION_REQUEST => {
            format!("permission · {}", naming::tool_summary(tool, input, base))
        }
        events::POST_TOOL_USE => {
            let mut s = naming::tool_summary(tool, input, base);
            if let Some((a, r)) = file_delta {
                s.push_str(&format!(
                    " ({})",
                    naming::diff_text(i64::from(a), i64::from(r))
                ));
            }
            if let Some(ms) = ev.get("duration_ms").and_then(Value::as_u64) {
                s.push_str(" · ");
                s.push_str(&duration_text(ms));
            }
            s
        }
        events::POST_TOOL_USE_FAILURE => {
            let mut s = format!("failed · {}", naming::tool_summary(tool, input, base));
            if let Some(err) = str_field(ev, "error").filter(|e| !e.trim().is_empty()) {
                s.push_str(" · ");
                s.push_str(&first_line(&err, 120));
            }
            s
        }
        events::NOTIFICATION => {
            let kind = ev.notification_type.as_deref().unwrap_or("notification");
            match ev.message.as_deref().filter(|m| !m.trim().is_empty()) {
                Some(m) => format!("{kind} · {}", first_line(m, 160)),
                None => kind.to_string(),
            }
        }
        events::STOP => match ev.last_assistant_message.as_deref() {
            Some(m) if !m.trim().is_empty() => format!("finished · {}", first_line(m, 120)),
            _ => "turn finished".into(),
        },
        events::STOP_FAILURE => match str_field(ev, "error") {
            Some(e) if !e.trim().is_empty() => format!("turn failed · {}", first_line(&e, 160)),
            _ => "turn failed".into(),
        },
        events::SUBAGENT_START => {
            format!("{} started", ev.agent_type.as_deref().unwrap_or("agent"))
        }
        events::SUBAGENT_STOP => {
            format!("{} finished", ev.agent_type.as_deref().unwrap_or("agent"))
        }
        events::PRE_COMPACT => match str_field(ev, "trigger").or_else(|| ev.reason.clone()) {
            Some(t) => format!("compacting · {t}"),
            None => "compacting".into(),
        },
        events::POST_COMPACT => "context compacted".into(),
        events::CWD_CHANGED => {
            let cwd = str_field(ev, "new_cwd").unwrap_or_else(|| ev.cwd.clone());
            format!("cwd → {}", naming::basename(&cwd).unwrap_or(cwd))
        }
        events::PRE_MODEL_SWITCH | events::POST_MODEL_SWITCH => format!(
            "model: {} → {}",
            str_field(ev, "from_model").unwrap_or_else(|| "?".into()),
            str_field(ev, "to_model").unwrap_or_else(|| "?".into())
        ),
        events::ELICITATION => match ev.message.as_deref() {
            Some(m) if !m.trim().is_empty() => {
                format!("needs input in the terminal · {}", first_line(m, 120))
            }
            _ => "needs input in the terminal".into(),
        },
        events::SESSION_END => format!("ended · {}", ev.reason.as_deref().unwrap_or("other")),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_follow_the_table() {
        let mut start = HookEvent::new("SessionStart", "s");
        start.set("source", "resume");
        start.set("model", "claude-opus-5-5");
        assert_eq!(
            event_summary(&start, None, false, None),
            "started · resume · claude-opus-5-5"
        );
        let mut prompt = HookEvent::new("UserPromptSubmit", "s");
        prompt.set("prompt", "Fix the sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA key\nsecond");
        assert_eq!(event_summary(&prompt, None, false, None), "prompt");
        let kept = event_summary(&prompt, None, true, None);
        assert!(kept.starts_with("Fix the [redacted]"), "{kept}");
        let mut post = HookEvent::new("PostToolUse", "s");
        post.set("tool_name", "Edit");
        post.set(
            "tool_input",
            serde_json::json!({"file_path": "/r/src/a.rs"}),
        );
        post.set("duration_ms", 1234);
        assert_eq!(
            event_summary(&post, Some("/r"), false, Some((12, 3))),
            "Edit src/a.rs (+12 \u{2212}3) · 1.2s"
        );
        let mut fail = HookEvent::new("PostToolUseFailure", "s");
        fail.set("tool_name", "Bash");
        fail.set("tool_input", serde_json::json!({"command": "cargo test"}));
        fail.set("error", "exit 101\nmore");
        assert_eq!(
            event_summary(&fail, None, false, None),
            "failed · Bash: cargo test · exit 101"
        );
        let mut end = HookEvent::new("SessionEnd", "s");
        end.set("reason", "clear");
        assert_eq!(event_summary(&end, None, false, None), "ended · clear");
        let mut n = HookEvent::new("Notification", "s");
        n.set("notification_type", "idle_prompt");
        n.set("message", "Claude is waiting for your input");
        assert_eq!(
            event_summary(&n, None, false, None),
            "idle_prompt · Claude is waiting for your input"
        );
        let mut sw = HookEvent::new("PostModelSwitch", "s");
        sw.set("from_model", "a");
        sw.set("to_model", "b");
        assert_eq!(event_summary(&sw, None, false, None), "model: a → b");
        assert_eq!(duration_text(12), "12ms");
        assert_eq!(duration_text(125_000), "2m 5s");
        assert_eq!(
            event_summary(&HookEvent::new("Mystery", "s"), None, false, None),
            "Mystery"
        );
    }
}
