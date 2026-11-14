//! Redaction before storage and before logging (09 §11; 08 §12).
//!
//! * [`scrub`] replaces the secret patterns of 09 §11 (`AKIA…`, `sk-…`,
//!   `ghp_…`, `-----BEGIN … PRIVATE KEY`, `Bearer …`, `password=`) with
//!   `[redacted]`.
//! * [`for_storage`] is what goes into `session_events.payload_json`:
//!   `tool_input` and `tool_response` truncated to 4 KB per string field and
//!   scrubbed, the prompt dropped unless previews are on.
//! * The IPC logger never logs `tool_input` values at all, only sizes
//!   ([`sizes`]).
//!
//! No regex crate: the patterns are few and a hand-written scanner keeps the
//! core's dependency list short.

use std::borrow::Cow;

use mode_wire::HookEvent;
use serde_json::{Map, Value};

/// What replaces a secret.
pub const REDACTED: &str = "[redacted]";
/// Per-field cap for `tool_input` / `tool_response` string values (09 §11).
pub const FIELD_CAP_BYTES: usize = 4 * 1024;
/// Hard cap for either field as a whole; beyond it only the identity keys
/// (`file_path`, `command`, …) survive.
pub const TOTAL_CAP_BYTES: usize = 16 * 1024;

/// Keys that identify what a tool did and are kept when the rest is dropped.
const IDENTITY_KEYS: &[&str] = &[
    "file_path",
    "notebook_path",
    "command",
    "description",
    "pattern",
    "path",
    "url",
    "query",
    "type",
    "filePath",
];

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Length of the run of token bytes starting at `i`.
fn token_len(s: &[u8], i: usize) -> usize {
    s[i..].iter().take_while(|b| is_token_byte(**b)).count()
}

fn starts_with_ci(s: &[u8], i: usize, pat: &str) -> bool {
    s.len() >= i + pat.len() && s[i..i + pat.len()].eq_ignore_ascii_case(pat.as_bytes())
}

/// How many bytes from `i` form a secret, if any.
fn secret_len(s: &[u8], i: usize) -> Option<usize> {
    // Only at a token boundary.
    if i > 0 && is_token_byte(s[i - 1]) {
        return None;
    }
    let rest = &s[i..];
    // AWS access key id: AKIA + 16 upper-case alphanumerics.
    if rest.starts_with(b"AKIA") {
        let n = token_len(s, i);
        if n == 20
            && rest[4..20]
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Some(n);
        }
    }
    // OpenAI / Anthropic style keys: sk-… (at least 8 more token bytes).
    if rest.starts_with(b"sk-") {
        let n = token_len(s, i);
        if n >= 11 {
            return Some(n);
        }
    }
    // GitHub tokens: ghp_, gho_, ghu_, ghs_, ghr_ (+36) and github_pat_ (+ many).
    if rest.len() > 4 && rest.starts_with(b"gh") && rest[3] == b'_' && b"pousr".contains(&rest[2]) {
        let n = token_len(s, i);
        if n >= 20 {
            return Some(n);
        }
    }
    if rest.starts_with(b"github_pat_") {
        let n = token_len(s, i);
        if n >= 30 {
            return Some(n);
        }
    }
    // PEM private key block, up to and including the END line (or the end of the text).
    if rest.starts_with(b"-----BEGIN") {
        if let Some(head_end) = rest.iter().position(|b| *b == b'\n') {
            let head = &rest[..head_end];
            if head.windows(11).any(|w| w == b"PRIVATE KEY") {
                let tail = rest[head_end..]
                    .windows(8)
                    .position(|w| w == b"-----END")
                    .map(|p| head_end + p);
                let end = match tail {
                    Some(t) => rest[t..]
                        .iter()
                        .position(|b| *b == b'\n')
                        .map(|p| t + p)
                        .unwrap_or(rest.len()),
                    None => rest.len(),
                };
                return Some(end);
            }
        } else if rest.windows(11).any(|w| w == b"PRIVATE KEY") {
            return Some(rest.len());
        }
    }
    None
}

/// Secrets introduced by a keyword: `Bearer <token>` and `password=<value>`
/// (also `password:`). Returns the span of the value to replace.
fn keyed_secret(s: &[u8], i: usize) -> Option<(usize, usize)> {
    if i > 0 && is_token_byte(s[i - 1]) {
        return None;
    }
    if starts_with_ci(s, i, "bearer ") {
        let start = i + 7;
        let n = s[start..]
            .iter()
            .take_while(|b| !b.is_ascii_whitespace() && !b"\"'`,;)]}".contains(b))
            .count();
        if n > 0 {
            return Some((start, start + n));
        }
    }
    for key in ["password=", "password:", "passwd=", "pwd="] {
        if starts_with_ci(s, i, key) {
            let mut start = i + key.len();
            while start < s.len() && s[start] == b' ' {
                start += 1;
            }
            let n = s[start..]
                .iter()
                .take_while(|b| !b.is_ascii_whitespace() && !b"\"'`,;&)]}".contains(b))
                .count();
            if n > 0 {
                return Some((start, start + n));
            }
        }
    }
    None
}

/// Replace every secret pattern of 09 §11 with `[redacted]`.
pub fn scrub(text: &str) -> Cow<'_, str> {
    let s = text.as_bytes();
    let mut out: Option<String> = None;
    let mut i = 0;
    let mut copied = 0;
    while i < s.len() {
        if !text.is_char_boundary(i) {
            i += 1;
            continue;
        }
        if let Some(n) = secret_len(s, i) {
            let o = out.get_or_insert_with(|| String::with_capacity(text.len()));
            o.push_str(&text[copied..i]);
            o.push_str(REDACTED);
            i += n;
            copied = i;
            continue;
        }
        if let Some((start, end)) = keyed_secret(s, i) {
            let o = out.get_or_insert_with(|| String::with_capacity(text.len()));
            o.push_str(&text[copied..start]);
            o.push_str(REDACTED);
            i = end;
            copied = i;
            continue;
        }
        i += 1;
    }
    match out {
        Some(mut o) => {
            o.push_str(&text[copied..]);
            Cow::Owned(o)
        }
        None => Cow::Borrowed(text),
    }
}

/// Cut a string to `cap` bytes on a char boundary, marking the cut.
fn truncate_str(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut end = cap;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [truncated {} bytes]", &s[..end], s.len() - end)
}

/// Scrub and cap every string leaf of a JSON value.
fn scrub_value(v: &Value, cap: usize) -> Value {
    match v {
        Value::String(s) => Value::String(scrub(&truncate_str(s, cap)).into_owned()),
        Value::Array(a) => Value::Array(a.iter().map(|x| scrub_value(x, cap)).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| (k.clone(), scrub_value(x, cap)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `tool_input` / `tool_response` as stored: every string capped at 4 KB and
/// scrubbed; when the whole field still exceeds 16 KB, only the identity keys
/// are kept next to a `truncated` marker.
pub fn tool_field_for_storage(v: &Value) -> Value {
    let scrubbed = scrub_value(v, FIELD_CAP_BYTES);
    let size = serde_json::to_vec(&scrubbed).map(|b| b.len()).unwrap_or(0);
    if size <= TOTAL_CAP_BYTES {
        return scrubbed;
    }
    let mut kept = Map::new();
    if let Value::Object(m) = &scrubbed {
        for key in IDENTITY_KEYS {
            if let Some(x) = m.get(*key) {
                kept.insert((*key).to_string(), x.clone());
            }
        }
    }
    kept.insert("truncated".into(), Value::Bool(true));
    kept.insert("bytes".into(), Value::from(size));
    Value::Object(kept)
}

/// The payload as written to `session_events.payload_json` (09 §11, 05 §3.5):
/// tool fields truncated and scrubbed, `prompt` dropped unless
/// `keep_prompt` (`sessions.store_prompt_previews`), the assistant preview
/// scrubbed. Everything else is kept as sent.
pub fn for_storage(event: &HookEvent, keep_prompt: bool) -> Value {
    let mut v = serde_json::to_value(event).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        for key in ["tool_input", "tool_response"] {
            if let Some(field) = m.get(key).cloned() {
                m.insert(key.to_string(), tool_field_for_storage(&field));
            }
        }
        if keep_prompt {
            if let Some(Value::String(p)) = m.get("prompt") {
                let p = scrub(&truncate_str(p, FIELD_CAP_BYTES)).into_owned();
                m.insert("prompt".into(), Value::String(p));
            }
        } else {
            m.remove("prompt");
        }
        for key in ["last_assistant_message", "message"] {
            if let Some(Value::String(s)) = m.get(key) {
                let s = scrub(&truncate_str(s, FIELD_CAP_BYTES)).into_owned();
                m.insert(key.to_string(), Value::String(s));
            }
        }
    }
    v
}

/// Sizes of the fields the logger must never print (08 §12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sizes {
    pub tool_input: usize,
    pub tool_response: usize,
    pub prompt: usize,
}

/// Byte sizes of `tool_input`, `tool_response` and `prompt` for log lines.
pub fn sizes(event: &HookEvent) -> Sizes {
    let len = |key: &str| {
        event
            .get(key)
            .map(|v| match v {
                Value::String(s) => s.len(),
                other => serde_json::to_vec(other).map(|b| b.len()).unwrap_or(0),
            })
            .unwrap_or(0)
    };
    Sizes {
        tool_input: len("tool_input"),
        tool_response: len("tool_response"),
        prompt: len("prompt"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_replaces_each_pattern_of_the_data_model() {
        let cases = [
            ("key AKIAIOSFODNN7EXAMPLE here", "key [redacted] here"),
            ("sk-ant-api03-abcdefghijklmnop", "[redacted]"),
            (
                "token=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef1234",
                "token=[redacted]",
            ),
            (
                "github_pat_11ABCDEFG0123456789_abcdefghijklmnop",
                "[redacted]",
            ),
            (
                "Authorization: Bearer eyJhbGciOi.xxx.yyy\n",
                "Authorization: Bearer [redacted]\n",
            ),
            (
                "curl -u x --data password=hunter2&next=1",
                "curl -u x --data password=[redacted]&next=1",
            ),
            ("PASSWORD: s3cret", "PASSWORD: [redacted]"),
            (
                "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----\nafter",
                "[redacted]\nafter",
            ),
            ("-----BEGIN OPENSSH PRIVATE KEY-----\nabc", "[redacted]"),
            (
                "plain text, nothing to hide; task-123 skip",
                "plain text, nothing to hide; task-123 skip",
            ),
            ("AKIA short", "AKIA short"),
            ("masked-sk-abcdefghijk", "masked-sk-abcdefghijk"),
            ("ünïcode sk-abcdefghijklmnop ok", "ünïcode [redacted] ok"),
        ];
        for (input, expected) in cases {
            assert_eq!(scrub(input), expected, "{input:?}");
        }
        assert!(matches!(scrub("nothing"), Cow::Borrowed(_)));
    }

    #[test]
    fn storage_form_caps_and_scrubs_tool_fields_and_drops_the_prompt() {
        let mut ev = HookEvent::new("PostToolUse", "s");
        ev.set("tool_name", "Write");
        ev.set(
            "tool_input",
            serde_json::json!({"file_path": "/x/.env", "content": format!("TOKEN=sk-abcdefghijklmnop\n{}", "x".repeat(10_000))}),
        );
        ev.set("tool_response", serde_json::json!({"type": "create", "filePath": "/x/.env", "content": "y".repeat(40_000)}));
        ev.set("prompt", "please password=abc");
        let stored = for_storage(&ev, false);
        assert!(stored.get("prompt").is_none(), "prompts are opt-in");
        let content = stored["tool_input"]["content"].as_str().unwrap();
        assert!(content.starts_with("TOKEN=[redacted]\n"));
        assert!(content.len() < FIELD_CAP_BYTES + 64, "{}", content.len());
        assert!(content.contains("[truncated"));
        assert_eq!(stored["tool_input"]["file_path"], "/x/.env");
        // 40 KB in one string → capped at 4 KB; the whole field then fits.
        assert!(stored["tool_response"]["content"].as_str().unwrap().len() < FIELD_CAP_BYTES + 64);
        assert_eq!(stored["tool_name"], "Write");

        let with_prompt = for_storage(&ev, true);
        assert_eq!(with_prompt["prompt"], "please password=[redacted]");

        // Many small strings beyond the total cap: identity keys only.
        let big: Vec<Value> = (0..3000)
            .map(|i| Value::from(format!("line {i} of output")))
            .collect();
        let v = serde_json::json!({"command": "ls", "lines": big});
        let stored = tool_field_for_storage(&v);
        assert_eq!(stored["command"], "ls");
        assert_eq!(stored["truncated"], true);
        assert!(stored.get("lines").is_none());
        assert!(stored["bytes"].as_u64().unwrap() > TOTAL_CAP_BYTES as u64);
    }

    #[test]
    fn sizes_report_bytes_without_contents() {
        let mut ev = HookEvent::new("PreToolUse", "s");
        ev.set("tool_input", serde_json::json!({"command": "npm test"}));
        ev.set("prompt", "hello");
        let s = sizes(&ev);
        assert_eq!(s.tool_input, r#"{"command":"npm test"}"#.len());
        assert_eq!(s.prompt, 5);
        assert_eq!(s.tool_response, 0);
    }
}
