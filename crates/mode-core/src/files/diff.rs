//! The diff viewer of 05 §6.3 (03 §3.9 Files tab → diff pane): read-only,
//! one file change row at a time.
//!
//! | Row | Diff | How |
//! |---|---|---|
//! | in the repository, still changed | working tree vs `HEAD` | `git diff HEAD -- <path>` (3 lines of context) |
//! | in the repository, committed since | the session's commits touching the path | `git log -p --since=<started_at> -- <path>` |
//! | untracked / new | the whole file as additions | file read, 1 MB cap |
//! | deleted | the `HEAD` version as removals | `git diff HEAD` (else `git show HEAD:<path>`) |
//! | hook only (no repository, outside it) | the last edit's `old_string` → `new_string` | the paired `PostToolUse` payload (4 KB per field) |
//!
//! Every diff comes back as typed lines (`hunk` / `ctx` / `add` / `del`, the
//! `contract-sessions.ts` `DiffLine`) whose `text` is the unified-diff line
//! itself (`+…`, `-…`, ` …`, `@@ … @@`), so joining the texts gives a valid
//! unified diff body ("Copy diff"). Output is capped at [`MAX_LINES`].

use std::path::Path;

use rusqlite::OptionalExtension;
use serde::Serialize;
use serde_json::Value;

use super::git::{self, GIT_TIMEOUT};
use crate::db::{Db, Result};
use crate::paths;
use crate::redact;
use crate::repo::{self, FileChangeRow, Session};

/// Lines shown before "diff truncated; open the file".
pub const MAX_LINES: usize = 3_000;
/// Whole-file reads (untracked / new files) stop here (05 §6.3: 1 MB).
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// One line of the diff pane (`DiffLine`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    /// `hunk` | `ctx` | `add` | `del`.
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_no: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_no: Option<u32>,
    pub text: String,
}

/// `session_diff`'s answer (`DiffResult`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffResult {
    /// "working tree vs HEAD", "commits in this session", "last edit",
    /// "new file", "deleted", "binary file", or "no diff available".
    pub basis: String,
    pub lines: Vec<DiffLine>,
    pub truncated: bool,
}

impl DiffResult {
    fn empty(basis: &str) -> Self {
        DiffResult {
            basis: basis.into(),
            lines: Vec::new(),
            truncated: false,
        }
    }
}

/// Parse unified diff text (`git diff`, `git log -p`, `similar`) into typed
/// lines. File headers (`diff --git`, `index`, `---`, `+++`, modes) are
/// dropped; `commit <sha> <subject>` headers of `git log -p
/// --format='commit %h %s'` become `hunk` lines. Returns the lines and
/// whether [`MAX_LINES`] cut them.
pub fn parse_unified(text: &str) -> (Vec<DiffLine>, bool) {
    let mut out = Vec::new();
    let (mut old_no, mut new_no) = (0u32, 0u32);
    let mut in_hunk = false;
    for line in text.lines() {
        if out.len() >= MAX_LINES {
            return (out, true);
        }
        if let Some(rest) = line.strip_prefix("@@") {
            if let Some((o, n)) = hunk_starts(rest) {
                old_no = o;
                new_no = n;
            }
            in_hunk = true;
            out.push(DiffLine {
                kind: "hunk",
                old_no: None,
                new_no: None,
                text: line.to_string(),
            });
            continue;
        }
        if !in_hunk || line.starts_with("diff --git ") {
            in_hunk = false;
            if line.starts_with("commit ") {
                out.push(DiffLine {
                    kind: "hunk",
                    old_no: None,
                    new_no: None,
                    text: line.to_string(),
                });
            }
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                out.push(DiffLine {
                    kind: "add",
                    old_no: None,
                    new_no: Some(new_no),
                    text: line.to_string(),
                });
                new_no += 1;
            }
            Some(b'-') => {
                out.push(DiffLine {
                    kind: "del",
                    old_no: Some(old_no),
                    new_no: None,
                    text: line.to_string(),
                });
                old_no += 1;
            }
            Some(b'\\') => {}
            _ => {
                out.push(DiffLine {
                    kind: "ctx",
                    old_no: Some(old_no),
                    new_no: Some(new_no),
                    text: if line.is_empty() {
                        " ".into()
                    } else {
                        line.to_string()
                    },
                });
                old_no += 1;
                new_no += 1;
            }
        }
    }
    (out, false)
}

/// `" -12,9 +12,19 @@ ctx"` → `(12, 12)`.
fn hunk_starts(rest: &str) -> Option<(u32, u32)> {
    let mut parts = rest.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let start = |s: &str| s.split(',').next().and_then(|n| n.parse::<u32>().ok());
    Some((start(old)?, start(new)?))
}

/// Shift every line number by `by` and rewrite the hunk headers to match
/// (a hook edit's snippet placed at its line in the file).
fn shift(lines: &mut [DiffLine], by: u32) {
    if by == 0 {
        return;
    }
    for l in lines.iter_mut() {
        l.old_no = l.old_no.map(|n| n + by);
        l.new_no = l.new_no.map(|n| n + by);
        if l.kind == "hunk"
            && let Some(rest) = l.text.strip_prefix("@@ ")
        {
            let mut parts = rest.splitn(3, ' ');
            let (Some(old), Some(new)) = (parts.next(), parts.next()) else {
                continue;
            };
            let tail = parts.next().unwrap_or("@@");
            let bump = |r: &str, sign: char| -> String {
                let r = r.trim_start_matches(sign);
                let mut it = r.splitn(2, ',');
                let start = it.next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
                match it.next() {
                    Some(len) => format!("{sign}{},{len}", start + by),
                    None => format!("{sign}{}", start + by),
                }
            };
            l.text = format!("@@ {} {} {tail}", bump(old, '-'), bump(new, '+'));
        }
    }
}

/// A whole file as additions (untracked / new; 05 §6.3, 1 MB cap).
fn whole_file(path: &Path, basis: &str) -> DiffResult {
    let Ok(meta) = std::fs::metadata(path) else {
        return DiffResult::empty("no diff available");
    };
    let truncated_bytes = meta.len() > MAX_FILE_BYTES;
    let mut bytes = Vec::new();
    {
        use std::io::Read;
        let Ok(f) = std::fs::File::open(path) else {
            return DiffResult::empty("no diff available");
        };
        if f.take(MAX_FILE_BYTES).read_to_end(&mut bytes).is_err() {
            return DiffResult::empty("no diff available");
        }
    }
    if bytes.contains(&0) {
        return DiffResult::empty("binary file");
    }
    let text = String::from_utf8_lossy(&bytes);
    let count = text.lines().count();
    let mut lines = vec![DiffLine {
        kind: "hunk",
        old_no: None,
        new_no: None,
        text: format!("@@ -0,0 +1,{count} @@"),
    }];
    let mut truncated = truncated_bytes;
    for (i, l) in text.lines().enumerate() {
        if lines.len() >= MAX_LINES {
            truncated = true;
            break;
        }
        lines.push(DiffLine {
            kind: "add",
            old_no: None,
            new_no: Some(i as u32 + 1),
            text: format!("+{l}"),
        });
    }
    DiffResult {
        basis: basis.into(),
        lines,
        truncated,
    }
}

/// `YYYY-MM-DD HH:MM:SS +0000` for `git --since`.
fn git_date(ts: &str) -> Option<String> {
    let t = crate::db::parse_timestamp(ts)?;
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} +0000",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    ))
}

fn git_text(root: &Path, args: &[&str]) -> Option<String> {
    git::run_git(args, root, GIT_TIMEOUT).map(|o| String::from_utf8_lossy(&o).into_owned())
}

fn is_binary_diff(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("Binary files ") && l.ends_with(" differ"))
}

/// The git half of 05 §6.3. `None` when git has nothing to say (the caller
/// falls back to the hook payload).
fn git_diff(root: &Path, row: &FileChangeRow, started_at: &str) -> Option<DiffResult> {
    let rel = row.rel_path.as_deref()?;
    let abs_os = paths::to_os(&row.path);
    let abs = abs_os.as_path();
    if row.change_kind == "untracked" {
        return Some(whole_file(abs, "new file"));
    }
    if !git::has_head(root) {
        return abs.is_file().then(|| whole_file(abs, "new file"));
    }
    let root_c = paths::from_os(root);
    let old_rel = row
        .old_path
        .as_deref()
        .map(|p| paths::display(p, Some(&root_c)));
    let mut args = vec![
        "diff",
        "--no-color",
        "--no-ext-diff",
        "-U3",
        "-M",
        "HEAD",
        "--",
        rel,
    ];
    if let Some(o) = old_rel.as_deref().filter(|o| *o != rel) {
        args.push(o);
    }
    let text = git_text(root, &args)?;
    if is_binary_diff(&text) {
        return Some(DiffResult::empty("binary file"));
    }
    if !text.trim().is_empty() {
        let (lines, truncated) = parse_unified(&text);
        let basis = match row.change_kind.as_str() {
            "deleted" => "deleted",
            "added" => "new file",
            _ => "working tree vs HEAD",
        };
        return Some(DiffResult {
            basis: basis.into(),
            lines,
            truncated,
        });
    }
    // Nothing versus HEAD: untracked (not in the index), or committed since.
    let tracked = git::run_git(
        &["ls-files", "--error-unmatch", "--", rel],
        root,
        GIT_TIMEOUT,
    )
    .is_some();
    if !tracked {
        return abs.is_file().then(|| whole_file(abs, "new file"));
    }
    let since = git_date(started_at)
        .map(|d| format!("--since={d}"))
        .unwrap_or_else(|| "--since=1970-01-01".into());
    let text = git_text(
        root,
        &[
            "log",
            "-p",
            "--no-color",
            "--no-ext-diff",
            "-U3",
            "--format=commit %h %s",
            &since,
            "--",
            rel,
        ],
    )?;
    if is_binary_diff(&text) {
        return Some(DiffResult::empty("binary file"));
    }
    let (lines, truncated) = parse_unified(&text);
    (!lines.is_empty()).then(|| DiffResult {
        basis: "commits in this session".into(),
        lines,
        truncated,
    })
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn cut(s: &str) -> bool {
    s.contains("… [truncated ")
}

/// The line (0-based) where `needle` starts in `hay`, when it occurs once.
fn line_of(hay: &str, needle: &str) -> Option<u32> {
    if needle.is_empty() {
        return None;
    }
    let at = hay.find(needle)?;
    if hay[at + needle.len()..].contains(needle) {
        return None;
    }
    Some(hay[..at].bytes().filter(|b| *b == b'\n').count() as u32)
}

/// The hook half of 05 §6.3: the paired `PostToolUse` payload (as stored:
/// 4 KB per field, redacted) diffed with `similar`, placed at its line in
/// the current file when the new text occurs there once.
pub fn hook_diff(payload: &Value, current_file: Option<&str>) -> DiffResult {
    let tool = str_field(payload, "tool_name");
    let input = payload.get("tool_input").cloned().unwrap_or(Value::Null);
    let response = payload.get("tool_response").cloned().unwrap_or(Value::Null);
    let mut truncated = input.get("truncated").and_then(Value::as_bool) == Some(true);
    let pairs: Vec<(String, String)> = match tool {
        "Edit" => vec![(
            str_field(&input, "old_string").to_string(),
            str_field(&input, "new_string").to_string(),
        )],
        "MultiEdit" => input
            .get("edits")
            .and_then(Value::as_array)
            .map(|edits| {
                edits
                    .iter()
                    .map(|e| {
                        (
                            str_field(e, "old_string").to_string(),
                            str_field(e, "new_string").to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        "Write" => vec![(
            str_field(&response, "originalFile").to_string(),
            str_field(&input, "content").to_string(),
        )],
        "NotebookEdit" => vec![(String::new(), str_field(&input, "new_source").to_string())],
        _ => Vec::new(),
    };
    if pairs.is_empty() {
        return DiffResult::empty("no diff available");
    }
    let mut lines = Vec::new();
    for (old, new) in &pairs {
        truncated |= cut(old) || cut(new);
        let text = similar::TextDiff::from_lines(old.as_str(), new.as_str())
            .unified_diff()
            .context_radius(3)
            .to_string();
        let (mut part, cap) = parse_unified(&text);
        truncated |= cap;
        if tool != "Write"
            && let Some(at) = current_file.and_then(|f| line_of(f, new))
        {
            shift(&mut part, at);
        }
        lines.extend(part);
        if lines.len() >= MAX_LINES {
            lines.truncate(MAX_LINES);
            truncated = true;
            break;
        }
    }
    let basis = if tool == "Write" && str_field(&response, "originalFile").is_empty() {
        "new file"
    } else {
        "last edit"
    };
    DiffResult {
        basis: basis.into(),
        lines,
        truncated,
    }
}

/// The stored `PostToolUse` payload of the row's last edit: by
/// `last_tool_use_id` (the session or one of its subagents), else the
/// latest writing-tool event naming the path.
fn last_edit_payload(
    conn: &rusqlite::Connection,
    session_id: &str,
    row: &FileChangeRow,
) -> Result<Option<Value>> {
    let scope = "(e.session_id = ?1 OR e.session_id IN (SELECT id FROM sessions WHERE parent_session_id = ?1))";
    let mut payload: Option<String> = None;
    if let Some(tid) = row.last_tool_use_id.as_deref() {
        payload = conn
            .query_row(
                &format!(
                    "SELECT e.payload_json FROM session_events e
                      WHERE {scope} AND e.event = 'PostToolUse' AND e.tool_use_id = ?2
                        AND e.payload_json IS NOT NULL
                      ORDER BY e.seq DESC LIMIT 1"
                ),
                rusqlite::params![session_id, tid],
                |r| r.get(0),
            )
            .optional()?;
    }
    if payload.is_none() {
        let rel = row.rel_path.clone().unwrap_or_default();
        payload = conn
            .query_row(
                &format!(
                    "SELECT e.payload_json FROM session_events e
                      WHERE {scope} AND e.event = 'PostToolUse' AND e.payload_json IS NOT NULL
                        AND e.tool_name IN ('Edit', 'MultiEdit', 'Write', 'NotebookEdit')
                        AND COALESCE(json_extract(e.payload_json, '$.tool_input.file_path'),
                                     json_extract(e.payload_json, '$.tool_input.notebook_path')) IN (?2, ?3)
                      ORDER BY e.seq DESC LIMIT 1"
                ),
                rusqlite::params![session_id, row.path, rel],
                |r| r.get(0),
            )
            .optional()?;
    }
    Ok(payload.and_then(|p| serde_json::from_str(&p).ok()))
}

/// The diff pane for one Files-tab row (`session_diff`). `None` when the
/// row is not one of the session's.
pub fn diff_for(db: &Db, session_id: &str, file_change_id: &str) -> Result<Option<DiffResult>> {
    let (sid, fid) = (session_id.to_string(), file_change_id.to_string());
    let found = db.read(|c| {
        let Some(row) =
            repo::query::<FileChangeRow>(c, "id = ? AND session_id = ?", &[&fid, &sid])?
                .into_iter()
                .next()
        else {
            return Ok(None);
        };
        let Some(session) = repo::get_by_id::<Session>(c, &sid)? else {
            return Ok(None);
        };
        let payload = last_edit_payload(c, &sid, &row)?;
        Ok(Some((row, session, payload)))
    })?;
    let Some((row, session, payload)) = found else {
        return Ok(None);
    };
    if let (Some(root), Some(_)) = (session.repo_root.as_deref(), row.rel_path.as_deref())
        && paths::to_os(root).is_dir()
        && let Some(d) = git_diff(&paths::to_os(root), &row, &session.started_at)
        && (!d.lines.is_empty() || d.basis == "binary file")
    {
        return Ok(Some(d));
    }
    if let Some(p) = payload {
        let current = std::fs::metadata(paths::to_os(&row.path))
            .ok()
            .filter(|m| m.len() <= MAX_FILE_BYTES)
            .and_then(|_| std::fs::read_to_string(paths::to_os(&row.path)).ok())
            .map(|s| redact::scrub(&s).into_owned());
        let d = hook_diff(&p, current.as_deref());
        if !d.lines.is_empty() {
            return Ok(Some(d));
        }
    }
    let path = paths::to_os(&row.path);
    if matches!(row.change_kind.as_str(), "added" | "untracked") && path.is_file() {
        return Ok(Some(whole_file(&path, "new file")));
    }
    Ok(Some(DiffResult::empty("no diff available")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_output_with_numbers_and_drops_headers() {
        let text = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -12,4 +12,5 @@ fn main()\n ctx one\n-old\n+new\n+added\n ctx two\n\\ No newline at end of file\n";
        let (lines, cut) = parse_unified(text);
        assert!(!cut);
        let kinds: Vec<&str> = lines.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, ["hunk", "ctx", "del", "add", "add", "ctx"]);
        assert_eq!(lines[1].old_no, Some(12));
        assert_eq!(lines[2].old_no, Some(13));
        assert_eq!(lines[3].new_no, Some(13));
        assert_eq!(lines[4].new_no, Some(14));
        assert_eq!((lines[5].old_no, lines[5].new_no), (Some(14), Some(15)));
        assert_eq!(lines[2].text, "-old");
        assert_eq!(lines[0].text, "@@ -12,4 +12,5 @@ fn main()");
    }

    #[test]
    fn log_output_keeps_commit_headers_as_hunk_lines() {
        let text = "commit abc1234 Pin the clock\ndiff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n";
        let (lines, _) = parse_unified(text);
        assert_eq!(lines[0].kind, "hunk");
        assert_eq!(lines[0].text, "commit abc1234 Pin the clock");
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn hook_edits_diff_and_land_at_their_line() {
        let payload = serde_json::json!({
            "tool_name": "Edit",
            "tool_input": {"file_path": "/r/a.txt", "old_string": "two\n", "new_string": "TWO\n"},
            "tool_response": {}
        });
        let file = "one\nTWO\nthree\n";
        let d = hook_diff(&payload, Some(file));
        assert_eq!(d.basis, "last edit");
        assert!(!d.truncated);
        let del = d.lines.iter().find(|l| l.kind == "del").unwrap();
        let add = d.lines.iter().find(|l| l.kind == "add").unwrap();
        assert_eq!((del.text.as_str(), del.old_no), ("-two", Some(2)));
        assert_eq!((add.text.as_str(), add.new_no), ("+TWO", Some(2)));
        assert_eq!(d.lines[0].text, "@@ -2 +2 @@");

        let cut = serde_json::json!({
            "tool_name": "Write",
            "tool_input": {"file_path": "/r/b.txt", "content": "x\n… [truncated 9000 bytes]"},
            "tool_response": {}
        });
        let d = hook_diff(&cut, None);
        assert_eq!(d.basis, "new file");
        assert!(d.truncated);
        assert!(d.lines.iter().all(|l| l.kind != "del"));
    }

    #[test]
    fn whole_files_are_additions_and_binaries_are_named() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("n.txt");
        std::fs::write(&p, "a\nb\n").unwrap();
        let d = whole_file(&p, "new file");
        assert_eq!(d.lines.len(), 3);
        assert_eq!(d.lines[0].text, "@@ -0,0 +1,2 @@");
        assert_eq!(d.lines[2].text, "+b");
        let b = dir.path().join("b.bin");
        std::fs::write(&b, [0u8, 1, 2]).unwrap();
        assert_eq!(whole_file(&b, "new file").basis, "binary file");
        assert_eq!(
            git_date("2026-10-21T12:00:00.000Z").unwrap(),
            "2026-10-21 12:00:00 +0000"
        );
    }
}
