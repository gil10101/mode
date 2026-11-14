//! Untracked sessions (05 §10): transcripts on disk with no `sessions` row
//! become `discovered` / `untracked` rows; ones that stop growing for 30
//! minutes get an inferred end.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use super::avatars;
use super::naming;
use crate::db::{Db, Result};
use crate::paths;
use crate::repo::{self, Session};
use crate::transcript::records;

/// Transcripts touched within this window are discovered (05 §10).
pub const DISCOVERY_WINDOW: Duration = Duration::from_secs(24 * 3600);
/// No growth for this long means the session ended (05 §4.2).
pub const INACTIVE_AFTER: Duration = Duration::from_secs(30 * 60);
/// Records read from the head of a file to identify it.
const HEAD_RECORDS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ScanReport {
    pub scanned: usize,
    pub discovered: usize,
    /// Known rows whose `last_event_at` moved to the file's mtime.
    pub refreshed: usize,
}

/// What the head of a transcript says about it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Head {
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub first_timestamp: Option<String>,
    pub title: Option<String>,
}

/// Read the first records of a transcript (05 §10: `sessionId` from the
/// first record, the file name as fallback).
pub fn read_head(path: &Path) -> Head {
    use std::io::BufRead;
    let mut head = Head::default();
    let Ok(file) = std::fs::File::open(path) else {
        return head;
    };
    let reader = std::io::BufReader::new(file);
    for line in reader.lines().take(HEAD_RECORDS) {
        let Ok(line) = line else { break };
        let Some(r) = records::parse_line(&line) else {
            continue;
        };
        if head.session_id.is_none() {
            head.session_id = r.session_id.clone();
        }
        if head.cwd.is_none() {
            head.cwd = r.cwd.clone();
        }
        if head.git_branch.is_none() {
            head.git_branch = r.git_branch.clone();
        }
        if head.first_timestamp.is_none() {
            head.first_timestamp = r.timestamp.clone();
        }
        if let Some(s) = r
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            head.title = Some(s.to_string());
        }
    }
    head
}

fn mtime_rfc3339(path: &Path) -> Option<String> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(crate::db::format_timestamp(m.into()))
}

fn minus(now: &str, d: Duration) -> String {
    let parsed = crate::db::parse_timestamp(now).unwrap_or_else(time::OffsetDateTime::now_utc);
    crate::db::format_timestamp(parsed - time::Duration::seconds(d.as_secs() as i64))
}

/// `<projects_root>/*/<session>.jsonl` (depth 2; subagent files live deeper).
pub fn list_transcripts(projects_root: &Path) -> Vec<PathBuf> {
    let Ok(projects) = std::fs::read_dir(projects_root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for project in projects.filter_map(|e| e.ok()).map(|e| e.path()) {
        if !project.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&project) else {
            continue;
        };
        for f in files.filter_map(|e| e.ok()).map(|e| e.path()) {
            if f.is_file() && f.extension().is_some_and(|x| x == "jsonl") {
                out.push(f);
            }
        }
    }
    out.sort();
    out
}

/// The columns of a known row the scan looks at.
struct Known {
    id: String,
    tracking: String,
    ended_at: Option<String>,
    last_event_at: String,
    end_reason: Option<String>,
}

/// One discovery pass (05 §10): every transcript whose mtime is within
/// `window` and whose session has no row becomes a `discovered` /
/// `untracked` row; known `discovered` rows get `last_event_at` = mtime.
pub fn scan(db: &Db, projects_root: &Path, now: &str, window: Duration) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    let since = minus(now, window);
    for path in list_transcripts(projects_root) {
        let Some(mtime) = mtime_rfc3339(&path) else {
            continue;
        };
        if mtime < since {
            continue;
        }
        report.scanned += 1;
        let head = read_head(&path);
        let claude_id = head
            .session_id
            .clone()
            .or_else(|| crate::transcript::session_id_from_path(&path));
        let Some(claude_id) = claude_id else {
            continue;
        };
        let path_text = paths::from_os(&path);
        let now_s = now.to_string();
        let mtime_c = mtime.clone();
        let cid = claude_id.clone();
        let outcome: u8 = db.write(move |c| {
            let existing: Option<Known> = c
                .query_row(
                    "SELECT id, tracking, ended_at, last_event_at, end_reason FROM sessions
                      WHERE claude_session_id = ? AND agent_id IS NULL",
                    [&cid],
                    |r| {
                        Ok(Known {
                            id: r.get(0)?,
                            tracking: r.get(1)?,
                            ended_at: r.get(2)?,
                            last_event_at: r.get(3)?,
                            end_reason: r.get(4)?,
                        })
                    },
                )
                .optional()?;
            match existing {
                Some(Known {
                    id,
                    tracking,
                    ended_at,
                    last_event_at,
                    end_reason,
                }) => {
                    if tracking == "discovered" && mtime_c > last_event_at {
                        let reopen = ended_at.is_some() && end_reason.as_deref() == Some("inferred");
                        c.execute(
                            "UPDATE sessions SET last_event_at = ?, transcript_path = COALESCE(transcript_path, ?),
                                    ended_at = CASE WHEN ?3 THEN NULL ELSE ended_at END,
                                    end_reason = CASE WHEN ?3 THEN NULL ELSE end_reason END,
                                    updated_at = ?
                              WHERE id = ?",
                            params![mtime_c, path_text, reopen, now_s, id],
                        )?;
                        return Ok(1);
                    }
                    Ok(0)
                }
                None => {
                    // The transcript's own `cwd` record (05 §10), canonical
                    // since `records::parse_line`.
                    let cwd = head.cwd.clone().unwrap_or_default();
                    let repo = if paths::to_os(&cwd).is_dir() {
                        naming::repo_info(&cwd)
                    } else {
                        naming::RepoInfo::default()
                    };
                    let started = head.first_timestamp.clone().unwrap_or_else(|| mtime_c.clone());
                    let row = Session {
                        id: repo::new_id(),
                        claude_session_id: cid.clone(),
                        tracking: "discovered".into(),
                        launch_mode: "external".into(),
                        state: "untracked".into(),
                        state_since: mtime_c.clone(),
                        state_reason: Some("transcript:discovered".into()),
                        display_name: naming::display_name(
                            head.title.as_deref(),
                            repo.repo_root.as_deref(),
                            &cwd,
                            &cid,
                        ),
                        title: head.title.clone(),
                        avatar_color: naming::avatar_color(repo.repo_root.as_deref(), &cwd),
                        avatar_variant: avatars::next_variant(c)?,
                        cwd,
                        repo_root: repo.repo_root,
                        git_branch: head.git_branch.clone().or(repo.git_branch),
                        transcript_path: Some(path_text),
                        started_at: started,
                        last_event_at: mtime_c,
                        created_at: now_s.clone(),
                        updated_at: now_s,
                        ..Default::default()
                    };
                    repo::insert(c, &row)?;
                    Ok(2)
                }
            }
        })?;
        match outcome {
            1 => report.refreshed += 1,
            2 => report.discovered += 1,
            _ => {}
        }
    }
    Ok(report)
}

/// `discovered` rows with no growth for `after` get `ended_at` = their last
/// mtime and `end_reason = 'inferred'` (05 §4.2). Returns rows closed.
pub fn mark_inactive(db: &Db, now: &str, after: Duration) -> Result<usize> {
    let cutoff = minus(now, after);
    let now = now.to_string();
    db.write(move |c| {
        Ok(c.execute(
            "UPDATE sessions SET ended_at = last_event_at, end_reason = 'inferred', updated_at = ?
              WHERE tracking = 'discovered' AND ended_at IS NULL AND last_event_at < ?",
            params![now, cutoff],
        )?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_head_and_lists_depth_two_only() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("-home-ana-code-x");
        std::fs::create_dir_all(proj.join("s1/subagents")).unwrap();
        std::fs::write(
            proj.join("s1.jsonl"),
            "{\"type\":\"summary\",\"summary\":\"Title here\",\"leafUuid\":\"x\"}\n{\"type\":\"user\",\"sessionId\":\"s1\",\"cwd\":\"/home/ana/code/x\",\"gitBranch\":\"main\",\"timestamp\":\"2026-10-21T10:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
        )
        .unwrap();
        std::fs::write(proj.join("s1/subagents/agent-a.jsonl"), "").unwrap();
        std::fs::write(proj.join("notes.txt"), "").unwrap();
        let listed = list_transcripts(dir.path());
        assert_eq!(listed, vec![proj.join("s1.jsonl")]);
        let head = read_head(&proj.join("s1.jsonl"));
        assert_eq!(head.session_id.as_deref(), Some("s1"));
        assert_eq!(head.cwd.as_deref(), Some("/home/ana/code/x"));
        assert_eq!(head.title.as_deref(), Some("Title here"));
        assert_eq!(
            head.first_timestamp.as_deref(),
            Some("2026-10-21T10:00:00.000Z")
        );
        // A transcript written on Windows: its `cwd` record is canonical
        // here, so it matches the hook's `cwd` and the row's.
        std::fs::write(
            proj.join("s2.jsonl"),
            "{\"type\":\"user\",\"sessionId\":\"s2\",\"cwd\":\"c:\\\\Users\\\\ana\\\\code\\\\x\\\\\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
        )
        .unwrap();
        assert_eq!(
            read_head(&proj.join("s2.jsonl")).cwd.as_deref(),
            Some("C:/Users/ana/code/x")
        );
        assert_eq!(
            minus("2026-10-21T12:00:00.000Z", Duration::from_secs(1800)),
            "2026-10-21T11:30:00.000Z"
        );
    }
}
