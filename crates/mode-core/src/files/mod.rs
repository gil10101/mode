//! Files changed (05 §6): rows derived from `PostToolUse` Edit / Write events
//! (`source = 'hook'`), the git pass that upgrades them to `'both'` with exact
//! counts and adds git-only rows ([`git`]), and the `+N −M` aggregates on the
//! session row. Every path here is canonical ([`crate::paths`]): a hook row
//! (`C:\Users\a\x.rs` from the payload) and a git row (`C:/Users/a/x.rs`)
//! for the same file are one row.

pub mod diff;
pub mod git;

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use mode_wire::{HookEvent, events};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{Db, Result};
use crate::paths;
use crate::repo::{self, FileChangeRow, Table};
use git::{ChangeKind, GitChange};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Hook,
    Git,
    Both,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Hook => "hook",
            Source::Git => "git",
            Source::Both => "both",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
    pub source: Source,
}

/// Tools whose `PostToolUse` means a file on disk changed.
pub const WRITING_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit"];

fn line_count(v: Option<&Value>) -> u32 {
    v.and_then(Value::as_str)
        .map(|s| s.lines().count() as u32)
        .unwrap_or(0)
}

/// A change row from a `PostToolUse` event of a writing tool; `None` otherwise.
pub fn from_post_tool_use(event: &HookEvent) -> Option<FileChange> {
    if event.hook_event_name != events::POST_TOOL_USE {
        return None;
    }
    let tool = event.tool_name()?;
    if !WRITING_TOOLS.contains(&tool) {
        return None;
    }
    let input = event.get("tool_input")?;
    let path = input
        .get("file_path")
        .or_else(|| input.get("notebook_path"))
        .and_then(Value::as_str)?
        .to_string();
    let (added, removed) = match tool {
        "Write" => (line_count(input.get("content")), 0),
        "Edit" => (
            line_count(input.get("new_string")),
            line_count(input.get("old_string")),
        ),
        "MultiEdit" => input
            .get("edits")
            .and_then(Value::as_array)
            .map(|edits| {
                edits.iter().fold((0, 0), |(a, r), e| {
                    (
                        a + line_count(e.get("new_string")),
                        r + line_count(e.get("old_string")),
                    )
                })
            })
            .unwrap_or((0, 0)),
        _ => (line_count(input.get("new_source")), 0),
    };
    Some(FileChange {
        path,
        added,
        removed,
        source: Source::Hook,
    })
}

/// `+N −M` over distinct files.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totals {
    pub files: usize,
    pub added: u32,
    pub removed: u32,
}

pub fn totals(changes: &[FileChange]) -> Totals {
    let files: BTreeSet<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    Totals {
        files: files.len(),
        added: changes.iter().map(|c| c.added).sum(),
        removed: changes.iter().map(|c| c.removed).sum(),
    }
}

impl fmt::Display for Totals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "+{} \u{2212}{}", self.added, self.removed)
    }
}

// ------------------------------------------------------------------ storage

/// What the row writers need to know about the session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionPaths {
    /// `sessions.id`.
    pub id: String,
    pub cwd: String,
    pub repo_root: Option<String>,
    pub started_at: String,
}

/// `file_changes.rel_path`: relative to `repo_root` when inside it (05 §6.1).
fn rel_path(path: &str, repo_root: Option<&str>) -> Option<String> {
    paths::relative(repo_root?, path)
}

/// The session's row for `path` (canonical). A Windows-shaped path is also
/// matched by [`paths::key`], so another drive-letter or directory case (or a
/// row stored before paths were canonical) is the same row.
fn get_row(conn: &Connection, session_id: &str, path: &str) -> Result<Option<FileChangeRow>> {
    let exact =
        repo::query::<FileChangeRow>(conn, "session_id = ? AND path = ?", &[&session_id, &path])?;
    if let Some(row) = exact.into_iter().next() {
        return Ok(Some(row));
    }
    let want = paths::key(path);
    if want == path && !path.starts_with("//") {
        return Ok(None);
    }
    let rows = repo::query::<FileChangeRow>(conn, "session_id = ?", &[&session_id])?;
    Ok(rows.into_iter().find(|r| paths::key(&r.path) == want))
}

/// Upsert the `(session, path)` row for a writing tool's `PostToolUse`
/// (05 §6.1): `change_kind` is `added` when `tool_response.type = "create"`,
/// else `modified`; a `Write` over an existing file counts the old content
/// (`tool_response.originalFile`) as removed; `edit_count` increments and the
/// estimates accumulate until the git pass replaces them.
pub fn apply_hook_change(
    conn: &Connection,
    session: &SessionPaths,
    change: &FileChange,
    tool_use_id: Option<&str>,
    response: Option<&Value>,
    now: &str,
) -> Result<FileChangeRow> {
    let path = paths::join(&session.cwd, &change.path);
    let is_create = response.and_then(|r| r.get("type")).and_then(Value::as_str) == Some("create");
    let removed = match response
        .and_then(|r| r.get("originalFile"))
        .and_then(Value::as_str)
    {
        Some(original) if change.removed == 0 => original.lines().count() as u32,
        _ => change.removed,
    };
    let rel = rel_path(&path, session.repo_root.as_deref());
    match get_row(conn, &session.id, &path)? {
        Some(mut row) => {
            row.lines_added += i64::from(change.added);
            row.lines_removed += i64::from(removed);
            row.edit_count += 1;
            row.last_tool_use_id = tool_use_id.map(str::to_string);
            row.last_seen_at = now.to_string();
            row.rel_path = rel;
            row.path = path;
            if row.change_kind == "deleted" || row.change_kind == "untracked" {
                row.change_kind = if is_create { "added" } else { "modified" }.into();
            }
            if row.source == "git" {
                row.source = "both".into();
            }
            repo::replace(conn, &row)?;
            Ok(row)
        }
        None => {
            let row = FileChangeRow {
                id: repo::new_id(),
                session_id: session.id.clone(),
                path,
                rel_path: rel,
                change_kind: if is_create { "added" } else { "modified" }.into(),
                old_path: None,
                lines_added: i64::from(change.added),
                lines_removed: i64::from(removed),
                edit_count: 1,
                source: "hook".into(),
                last_tool_use_id: tool_use_id.map(str::to_string),
                first_seen_at: now.to_string(),
                last_seen_at: now.to_string(),
            };
            repo::insert(conn, &row)?;
            Ok(row)
        }
    }
}

/// What a git pass changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct MergeReport {
    /// Hook rows upgraded to `both` with exact counts.
    pub upgraded: usize,
    /// New `git` rows (mtime at or after `started_at`).
    pub inserted: usize,
    /// Git-reported paths skipped as predating the session.
    pub skipped: usize,
}

fn mtime_rfc3339(path: &str) -> Option<String> {
    let m = std::fs::metadata(paths::to_os(path))
        .ok()?
        .modified()
        .ok()?;
    Some(crate::db::format_timestamp(m.into()))
}

/// Fold the git pass into `file_changes` (05 §6.2): a git-reported path
/// upgrades its hook row to `both` with exact counts (`D` → `deleted`, `R` →
/// `renamed`); a path without a hook row becomes a `git` row only when its
/// mtime is at or after `started_at`. Rows git no longer reports are kept.
pub fn merge_git_pass(
    conn: &Connection,
    session: &SessionPaths,
    changes: &[GitChange],
    now: &str,
) -> Result<MergeReport> {
    let mut report = MergeReport::default();
    // Keyed by `paths::key`: the hook spelling and git's meet here.
    let existing: HashMap<String, FileChangeRow> =
        repo::query::<FileChangeRow>(conn, "session_id = ?", &[&session.id])?
            .into_iter()
            .map(|r| (paths::key(&r.path), r))
            .collect();
    for c in changes {
        let path = paths::canon(&c.path);
        let old_path = c.old_path.as_deref().map(paths::canon);
        let rel = rel_path(&path, session.repo_root.as_deref());
        match existing.get(&paths::key(&path)) {
            Some(row) => {
                let mut row = row.clone();
                row.path = path.clone();
                row.lines_added = i64::from(c.added);
                row.lines_removed = i64::from(c.removed);
                row.source = "both".into();
                row.last_seen_at = now.to_string();
                row.rel_path = rel;
                match c.kind {
                    ChangeKind::Deleted => row.change_kind = "deleted".into(),
                    ChangeKind::Renamed => {
                        row.change_kind = "renamed".into();
                        row.old_path = old_path;
                    }
                    ChangeKind::Added => row.change_kind = "added".into(),
                    // A hook-created file git has not seen yet stays `added`.
                    ChangeKind::Untracked if row.change_kind == "added" => {}
                    ChangeKind::Untracked => row.change_kind = "untracked".into(),
                    ChangeKind::Modified if row.change_kind == "added" => {}
                    ChangeKind::Modified => row.change_kind = "modified".into(),
                }
                repo::replace(conn, &row)?;
                report.upgraded += 1;
            }
            None => {
                if c.kind == ChangeKind::Deleted {
                    report.skipped += 1;
                    continue;
                }
                let in_window = mtime_rfc3339(&c.path)
                    .is_some_and(|m| m.as_str() >= session.started_at.as_str());
                if !in_window {
                    report.skipped += 1;
                    continue;
                }
                let row = FileChangeRow {
                    id: repo::new_id(),
                    session_id: session.id.clone(),
                    path,
                    rel_path: rel,
                    change_kind: c.kind.as_str().into(),
                    old_path,
                    lines_added: i64::from(c.added),
                    lines_removed: i64::from(c.removed),
                    edit_count: 0,
                    source: "git".into(),
                    last_tool_use_id: None,
                    first_seen_at: now.to_string(),
                    last_seen_at: now.to_string(),
                };
                repo::insert(conn, &row)?;
                report.inserted += 1;
            }
        }
    }
    Ok(report)
}

/// `sessions.lines_added / lines_removed / files_changed` from the rows.
pub fn recompute_totals(conn: &Connection, session_id: &str) -> Result<Totals> {
    let (files, added, removed): (i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(lines_added), 0), COALESCE(SUM(lines_removed), 0)
           FROM file_changes WHERE session_id = ?",
        [session_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    conn.execute(
        "UPDATE sessions SET lines_added = ?, lines_removed = ?, files_changed = ?, updated_at = ?
          WHERE id = ?",
        params![added, removed, files, crate::db::now_rfc3339(), session_id],
    )?;
    Ok(Totals {
        files: files as usize,
        added: added as u32,
        removed: removed as u32,
    })
}

/// The session's path context from its row.
pub fn session_paths(conn: &Connection, session_id: &str) -> Result<Option<SessionPaths>> {
    Ok(
        repo::get_by_id::<repo::Session>(conn, session_id)?.map(|s| SessionPaths {
            id: s.id,
            cwd: s.cwd,
            repo_root: s.repo_root,
            started_at: s.started_at,
        }),
    )
}

/// One git pass for a session (05 §6.2): `git status` + `numstat` in its
/// `repo_root`, merged into `file_changes`, totals recomputed. `None` when
/// the session has no repository (or `git` failed); the caller debounces.
pub fn run_git_pass(db: &Db, session_id: &str) -> Result<Option<(MergeReport, Totals)>> {
    let Some(session) = db.read(|c| session_paths(c, session_id))? else {
        return Ok(None);
    };
    let Some(root) = session.repo_root.clone() else {
        return Ok(None);
    };
    let Some(changes) = git::status(&paths::to_os(&root)) else {
        return Ok(None);
    };
    let session_id = session_id.to_string();
    db.write(move |c| {
        let now = crate::db::now_rfc3339();
        let report = repo::tx(c, |tx| merge_git_pass(tx, &session, &changes, &now))?;
        let totals = recompute_totals(c, &session_id)?;
        Ok(Some((report, totals)))
    })
}

/// The `file_changes` table name, for callers that count rows.
pub const TABLE: &str = FileChangeRow::TABLE;

#[cfg(test)]
mod tests {
    use super::*;

    fn post_tool_use(tool: &str, input: Value) -> HookEvent {
        let mut ev = HookEvent::new("PostToolUse", "s");
        ev.set("tool_name", tool);
        ev.set("tool_input", input);
        ev.set("tool_response", Value::Null);
        ev
    }

    #[test]
    fn derives_rows_from_writing_tools_only() {
        let edit = post_tool_use(
            "Edit",
            serde_json::json!({"file_path": "src/a.rs", "old_string": "a\nb", "new_string": "a\nb\nc"}),
        );
        assert_eq!(
            from_post_tool_use(&edit),
            Some(FileChange {
                path: "src/a.rs".into(),
                added: 3,
                removed: 2,
                source: Source::Hook
            })
        );
        let write = post_tool_use(
            "Write",
            serde_json::json!({"file_path": "README.md", "content": "one\ntwo\n"}),
        );
        assert_eq!(from_post_tool_use(&write).unwrap().added, 2);
        let bash = post_tool_use("Bash", serde_json::json!({"command": "ls"}));
        assert_eq!(from_post_tool_use(&bash), None);
        let mut pre = edit.clone();
        pre.hook_event_name = "PreToolUse".into();
        assert_eq!(
            from_post_tool_use(&pre),
            None,
            "only PostToolUse means the file changed"
        );
    }

    #[test]
    fn totals_count_distinct_files() {
        let rows = vec![
            FileChange {
                path: "a".into(),
                added: 1,
                removed: 0,
                source: Source::Hook,
            },
            FileChange {
                path: "a".into(),
                added: 2,
                removed: 1,
                source: Source::Hook,
            },
            FileChange {
                path: "b".into(),
                added: 0,
                removed: 4,
                source: Source::Git,
            },
        ];
        let t = totals(&rows);
        assert_eq!(
            t,
            Totals {
                files: 2,
                added: 3,
                removed: 5
            }
        );
        assert_eq!(t.to_string(), "+3 \u{2212}5");
        assert_eq!(
            rel_path("/u/mode/src/a.rs", Some("/u/mode")).as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(rel_path("/elsewhere/a.rs", Some("/u/mode")), None);
        assert_eq!(rel_path("/u/mode/src/a.rs", None), None);
    }
}
