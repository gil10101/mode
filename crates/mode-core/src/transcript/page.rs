//! The Transcript tab (03 §3.10): a read-only rendering of
//! `transcript_path`, one block per JSONL line, paged backwards from the end
//! (500 lines a page), with the subagent files of
//! `<session-id>/subagents/*.jsonl` as nested sections on the first page.
//! Shapes: `contract-sessions.ts` `TranscriptLine` / `TranscriptPage`.
//!
//! Line numbers are 1-based and absolute. One pass over the file records
//! the line offsets; only the page's lines are parsed. Raw lines are capped
//! at [`RAW_CAP`] bytes and expanded content at [`CONTENT_CAP`]. The CLI's
//! `cost-state` bookkeeping lines are left out and cost keys are dropped
//! from the raw JSON (no currency anywhere, ADR-0032).

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};

/// Lines per page (03 §3.10) and the largest page a caller may ask for.
pub const PAGE_LINES: usize = 500;
pub const MAX_PAGE_LINES: usize = 2_000;
/// The raw JSON kept per line ("Raw JSON" toggle).
pub const RAW_CAP: usize = 64 * 1024;
/// The expanded content kept per line.
pub const CONTENT_CAP: usize = 16 * 1024;
/// The one-line text.
const TEXT_CHARS: usize = 240;
/// Subagent files rendered on the first page, and their lines each.
const MAX_SUBAGENT_FILES: usize = 50;
const SUBAGENT_LINES: usize = 200;
/// Lines longer than this are not parsed (the block says so).
const PARSE_CAP: u64 = 8 * 1024 * 1024;

/// Per-turn usage (`TranscriptLine.usage`): tokens only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LineUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
}

/// One block of the Transcript tab.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptLine {
    pub line: u64,
    pub at: Option<String>,
    /// The record `type`.
    #[serde(rename = "type")]
    pub kind: String,
    /// `user`, `assistant`, `tool_use · Bash`, `tool_result`, `summary`, …
    pub label: String,
    /// One line of content.
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<LineUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

/// `session_transcript`'s answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptPage {
    pub path: Option<String>,
    pub size_bytes: Option<u64>,
    pub exists: bool,
    pub lines: Vec<TranscriptLine>,
    /// Lines exist before the first one returned.
    pub has_more: bool,
    pub subagent_files: usize,
}

fn first_line(s: &str) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(TEXT_CHARS).collect();
    if line.chars().count() > TEXT_CHARS {
        out.push('…');
    }
    out
}

fn cap(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [truncated {} bytes]", &s[..end], s.len() - end)
}

const COST_KEYS: &[&str] = &["costUSD", "totalCostUSD", "cost_usd", "total_cost_usd"];

fn strip_cost(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for k in COST_KEYS {
                m.remove(*k);
            }
            for x in m.values_mut() {
                strip_cost(x);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(strip_cost),
        _ => {}
    }
}

/// The argument a tool call is known by (path, command, pattern, …).
fn tool_arg(input: &Value) -> String {
    for key in [
        "file_path",
        "notebook_path",
        "command",
        "pattern",
        "path",
        "url",
        "query",
        "description",
        "prompt",
    ] {
        if let Some(s) = input.get(key).and_then(Value::as_str) {
            return first_line(s);
        }
    }
    if let Some(q) = input
        .get("questions")
        .and_then(Value::as_array)
        .and_then(|q| q.first())
        .and_then(|q| q.get("question"))
        .and_then(Value::as_str)
    {
        return first_line(q);
    }
    first_line(&serde_json::to_string(input).unwrap_or_default())
}

/// A `tool_result` block's content as text.
fn result_text(block: &Value) -> String {
    match block.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn usage_of(message: &Value) -> Option<LineUsage> {
    let u = message.get("usage")?.as_object()?;
    let get = |k: &str| u.get(k).and_then(Value::as_i64).unwrap_or(0);
    Some(LineUsage {
        input_tokens: get("input_tokens"),
        output_tokens: get("output_tokens"),
        cache_read_tokens: get("cache_read_input_tokens"),
        cache_write_tokens: get("cache_creation_input_tokens"),
    })
}

/// Render one JSONL line. `None` for blank lines and `cost-state` lines.
pub fn render_line(line: u64, raw: &str) -> Option<TranscriptLine> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut v: Value = match serde_json::from_str(trimmed) {
        Ok(v @ Value::Object(_)) => v,
        _ => {
            return Some(TranscriptLine {
                line,
                at: None,
                kind: "unreadable".into(),
                label: "unreadable".into(),
                text: "This line is not a JSON object.".into(),
                content: None,
                usage: None,
                raw: Some(cap(trimmed, RAW_CAP)),
                agent_id: None,
                agent_type: None,
            });
        }
    };
    let kind = v
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    if kind == "cost-state" {
        return None;
    }
    let at = v
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    let raw_out = if trimmed.contains("ost") {
        strip_cost(&mut v);
        serde_json::to_string(&v).unwrap_or_default()
    } else {
        trimmed.to_string()
    };
    let empty = Value::Object(Map::new());
    let message = v.get("message").unwrap_or(&empty);
    let mut usage = None;
    let (label, text, content) = match kind.as_str() {
        "user" | "assistant" => {
            if kind == "assistant" {
                usage = usage_of(message);
            }
            match message.get("content") {
                Some(Value::String(s)) => (kind.clone(), first_line(s), Some(s.clone())),
                Some(Value::Array(blocks)) => {
                    let tool_use = blocks
                        .iter()
                        .find(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"));
                    let tool_result = blocks
                        .iter()
                        .find(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"));
                    let texts: Vec<&str> = blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(Value::as_str))
                        .collect();
                    if let Some(t) = tool_use {
                        let name = t.get("name").and_then(Value::as_str).unwrap_or("tool");
                        let input = t.get("input").cloned().unwrap_or(Value::Null);
                        let mut body = texts.join("\n");
                        if !body.is_empty() {
                            body.push_str("\n\n");
                        }
                        body.push_str(&serde_json::to_string_pretty(&input).unwrap_or_default());
                        (format!("tool_use · {name}"), tool_arg(&input), Some(body))
                    } else if let Some(r) = tool_result {
                        let body = result_text(r);
                        let failed = r.get("is_error").and_then(Value::as_bool) == Some(true);
                        let text = if failed {
                            format!("error · {}", first_line(&body))
                        } else {
                            first_line(&body)
                        };
                        ("tool_result".to_string(), text, Some(body))
                    } else if !texts.is_empty() {
                        let body = texts.join("\n");
                        (kind.clone(), first_line(&body), Some(body))
                    } else if blocks
                        .iter()
                        .any(|b| b.get("type").and_then(Value::as_str) == Some("thinking"))
                    {
                        ("thinking".to_string(), "thinking".to_string(), None)
                    } else {
                        (kind.clone(), String::new(), None)
                    }
                }
                _ => (kind.clone(), String::new(), None),
            }
        }
        "summary" => {
            let s = v.get("summary").and_then(Value::as_str).unwrap_or("");
            ("summary".to_string(), first_line(s), Some(s.to_string()))
        }
        "system" => {
            let subtype = v.get("subtype").and_then(Value::as_str);
            let body = v.get("content").and_then(Value::as_str).unwrap_or("");
            let text = match subtype {
                Some("compact_boundary") => "compacted".to_string(),
                _ if !body.is_empty() => first_line(body),
                Some(s) => s.replace('_', " "),
                None => String::new(),
            };
            let label = match subtype {
                Some(s) => format!("system · {}", s.replace('_', " ")),
                None => "system".into(),
            };
            (label, text, (!body.is_empty()).then(|| body.to_string()))
        }
        "attachment" => {
            let a = v.get("attachment").unwrap_or(&empty);
            let text = ["filename", "path", "type"]
                .iter()
                .find_map(|k| a.get(*k).and_then(Value::as_str))
                .map(first_line)
                .unwrap_or_default();
            ("attachment".to_string(), text, None)
        }
        other => (other.to_string(), String::new(), None),
    };
    Some(TranscriptLine {
        line,
        at,
        kind,
        label,
        text,
        content: content
            .filter(|c| !c.is_empty())
            .map(|c| cap(&c, CONTENT_CAP)),
        usage,
        raw: Some(cap(&raw_out, RAW_CAP)),
        agent_id: None,
        agent_type: None,
    })
}

/// Byte offsets of every line start, and the file size.
fn line_starts(file: &mut File) -> std::io::Result<(Vec<u64>, u64)> {
    let mut starts = vec![0u64];
    let mut buf = vec![0u8; 64 * 1024];
    let mut pos = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for (i, b) in buf[..n].iter().enumerate() {
            if *b == b'\n' {
                starts.push(pos + i as u64 + 1);
            }
        }
        pos += n as u64;
    }
    if starts.last() == Some(&pos) {
        starts.pop();
    }
    Ok((starts, pos))
}

/// Read lines `[from, to)` (0-based indexes into `starts`) and render them.
fn read_range(
    file: &mut File,
    starts: &[u64],
    size: u64,
    from: usize,
    to: usize,
) -> std::io::Result<Vec<TranscriptLine>> {
    let mut out = Vec::with_capacity(to.saturating_sub(from));
    if from >= to {
        return Ok(out);
    }
    file.seek(SeekFrom::Start(starts[from]))?;
    let mut reader = BufReader::new(file);
    for (i, start) in starts.iter().enumerate().take(to).skip(from) {
        let end = starts.get(i + 1).copied().unwrap_or(size);
        let len = end.saturating_sub(*start);
        let n = i as u64 + 1;
        if len > PARSE_CAP {
            let mut skipped = Vec::new();
            (&mut reader).take(len).read_to_end(&mut skipped)?;
            out.push(TranscriptLine {
                line: n,
                at: None,
                kind: "oversized".into(),
                label: "oversized".into(),
                text: format!("A {} MB line, not shown.", len / (1024 * 1024)),
                content: None,
                usage: None,
                raw: None,
                agent_id: None,
                agent_type: None,
            });
            continue;
        }
        let mut bytes = Vec::with_capacity(len as usize);
        reader.read_until(b'\n', &mut bytes)?;
        let text = String::from_utf8_lossy(&bytes);
        let last_unterminated = i + 1 == starts.len() && !text.ends_with('\n');
        match render_line(n, &text) {
            // A partial last line is still being written: leave it for the next page load.
            Some(l) if last_unterminated && l.kind == "unreadable" => {}
            Some(l) => out.push(l),
            None => {}
        }
    }
    Ok(out)
}

/// The last `limit` lines before line `before` (1-based; `None` = the end).
pub fn tail(
    path: &Path,
    before: Option<u64>,
    limit: usize,
) -> std::io::Result<(Vec<TranscriptLine>, bool, u64)> {
    let mut file = File::open(path)?;
    let (starts, size) = line_starts(&mut file)?;
    let end = match before {
        Some(b) => (b.saturating_sub(1) as usize).min(starts.len()),
        None => starts.len(),
    };
    let from = end.saturating_sub(limit.max(1));
    let lines = read_range(&mut file, &starts, size, from, end)?;
    Ok((lines, from > 0, size))
}

/// One page of the Transcript tab for a session's `transcript_path`.
/// `before_line` pages backwards; the first page (`None`) also carries the
/// subagent files' last lines.
pub fn page(path: Option<&str>, before_line: Option<u64>, limit: usize) -> TranscriptPage {
    let limit = limit.clamp(1, MAX_PAGE_LINES);
    let Some(p) = path.filter(|p| !p.is_empty()) else {
        return TranscriptPage {
            path: None,
            size_bytes: None,
            exists: false,
            lines: Vec::new(),
            has_more: false,
            subagent_files: 0,
        };
    };
    let file_os = crate::paths::to_os(p);
    let file = file_os.as_path();
    let subagents = super::subagents_dir_of(file)
        .map(|d| super::list_subagents(&d))
        .unwrap_or_default();
    let (mut lines, has_more, size) = match tail(file, before_line, limit) {
        Ok(r) => r,
        Err(_) => {
            return TranscriptPage {
                path: Some(p.to_string()),
                size_bytes: None,
                exists: false,
                lines: Vec::new(),
                has_more: false,
                subagent_files: subagents.len(),
            };
        }
    };
    if before_line.is_none() {
        for (agent_id, sub) in subagents.iter().take(MAX_SUBAGENT_FILES) {
            let agent_type = super::read_meta(sub).and_then(|m| m.agent_type);
            if let Ok((sub_lines, _, _)) = tail(sub, None, SUBAGENT_LINES.min(limit)) {
                lines.extend(sub_lines.into_iter().map(|mut l| {
                    l.agent_id = Some(agent_id.clone());
                    l.agent_type = agent_type.clone();
                    l
                }));
            }
        }
    }
    TranscriptPage {
        path: Some(p.to_string()),
        size_bytes: Some(size),
        exists: true,
        lines,
        has_more,
        subagent_files: subagents.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_lines(path: &Path, n: usize) {
        let mut s = String::new();
        for i in 1..=n {
            s.push_str(&format!(
                r#"{{"type":"user","timestamp":"2026-10-21T12:00:{:02}.000Z","message":{{"role":"user","content":"prompt {i}"}}}}"#,
                i % 60
            ));
            s.push('\n');
        }
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn pages_backwards_with_absolute_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        write_lines(&p, 12);
        let first = page(Some(p.to_str().unwrap()), None, 5);
        assert!(first.exists && first.has_more);
        let nums: Vec<u64> = first.lines.iter().map(|l| l.line).collect();
        assert_eq!(nums, [8, 9, 10, 11, 12]);
        assert_eq!(first.lines[4].text, "prompt 12");
        assert_eq!(first.lines[4].label, "user");
        let second = page(Some(p.to_str().unwrap()), Some(8), 5);
        let nums: Vec<u64> = second.lines.iter().map(|l| l.line).collect();
        assert_eq!(nums, [3, 4, 5, 6, 7]);
        assert!(second.has_more);
        let last = page(Some(p.to_str().unwrap()), Some(3), 5);
        assert_eq!(last.lines.len(), 2);
        assert!(!last.has_more);
        let missing = page(
            Some(dir.path().join("gone.jsonl").to_str().unwrap()),
            None,
            5,
        );
        assert!(!missing.exists && missing.lines.is_empty());
        assert!(!page(None, None, 5).exists);
    }

    #[test]
    fn labels_tool_calls_results_and_usage_and_drops_cost() {
        let tool = r#"{"type":"assistant","timestamp":"t","message":{"role":"assistant","usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":5},"content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"npm test"}}]}}"#;
        let l = render_line(4, tool).unwrap();
        assert_eq!(l.label, "tool_use · Bash");
        assert_eq!(l.text, "npm test");
        assert_eq!(l.usage.unwrap().cache_read_tokens, 5);
        let result = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","is_error":true,"content":"1 failed\nmore"}]}}"#;
        let l = render_line(5, result).unwrap();
        assert_eq!(
            (l.label.as_str(), l.text.as_str()),
            ("tool_result", "error · 1 failed")
        );
        assert!(render_line(6, r#"{"type":"cost-state","totalCostUSD":1.5}"#).is_none());
        let l = render_line(7, r#"{"type":"result","total_cost_usd":0.2,"usage":{}}"#).unwrap();
        assert!(!l.raw.unwrap().contains("cost"));
        assert!(render_line(8, "   ").is_none());
        assert_eq!(render_line(9, "{not json").unwrap().kind, "unreadable");
    }
}
