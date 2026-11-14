//! The byte-offset tailer (05 §5.2 "Tail"), the database ingest of what a
//! batch of records contributes (05 §5.3) and the [`Tailers`] set that follows
//! every live session's transcript plus its subagents.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;

use super::records::{self, Extracted};
use crate::db::{Db, Result};
use crate::repo::{self, Session, SessionEvent};
use crate::sessions::naming;

/// Ended sessions are tailed this long after `ended_at` (final reads).
pub const ENDED_GRACE_SECS: i64 = 3600;

/// One transcript's read position. Complete lines only: a partial last line
/// waits for the next read; a shrink restarts from 0.
#[derive(Debug, Clone)]
pub struct Tailer {
    pub path: PathBuf,
    pub offset: u64,
    partial: Vec<u8>,
}

impl Tailer {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            partial: Vec::new(),
        }
    }

    /// The file's size differs from the offset (a stat; nothing is read).
    pub fn changed(&self) -> bool {
        match std::fs::metadata(&self.path) {
            Ok(m) => m.len() != self.offset,
            Err(_) => false,
        }
    }

    /// Every complete new line since the last read. A missing file reads as
    /// nothing; a file shorter than the offset restarts from 0.
    pub fn read_new(&mut self) -> io::Result<Vec<String>> {
        let mut file = match std::fs::File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let len = file.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
            self.partial.clear();
        }
        if len == self.offset {
            return Ok(Vec::new());
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::with_capacity((len - self.offset) as usize);
        let read = file.read_to_end(&mut buf)? as u64;
        self.offset += read;
        let mut data = std::mem::take(&mut self.partial);
        data.extend_from_slice(&buf);
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, b) in data.iter().enumerate() {
            if *b == b'\n' {
                let line = String::from_utf8_lossy(&data[start..i]).into_owned();
                if !line.trim().is_empty() {
                    lines.push(line);
                }
                start = i + 1;
            }
        }
        self.partial = data[start..].to_vec();
        Ok(lines)
    }

    /// Bytes held back as a partial last line.
    pub fn partial_len(&self) -> usize {
        self.partial.len()
    }
}

// ------------------------------------------------------------------- ingest

/// What one ingest wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct IngestReport {
    pub events: usize,
    pub samples: usize,
    pub title_changed: bool,
    /// Any session column changed (the roster should re-render).
    pub changed: bool,
}

fn hook_session_title(conn: &Connection, session_id: &str) -> Result<Option<String>> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM session_events
              WHERE session_id = ? AND source = 'hook' AND event = 'SessionStart'
              ORDER BY seq DESC LIMIT 1",
            [session_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(payload
        .and_then(|p| serde_json::from_str::<Value>(&p).ok())
        .and_then(|v| {
            v.get("session_title")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|t| !t.trim().is_empty()))
}

/// Write a batch of extracted records for a session row (05 §5.3):
/// `session_events` (`source = 'transcript'`, ids stable across re-reads),
/// `usage_samples` de-duplicated by `(source, turn_ref)`, token totals,
/// model, context size, title (unless a hook `session_title` or a custom name
/// wins), and for `discovered` rows `cwd`, `num_turns` and `last_event_at`.
pub fn ingest(
    conn: &Connection,
    session_id: &str,
    extracted: &Extracted,
    custom_named: bool,
    now: &str,
) -> Result<IngestReport> {
    let mut report = IngestReport::default();
    let Some(row) = repo::get_by_id::<Session>(conn, session_id)? else {
        return Ok(report);
    };
    let mut seq: i64 = conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM session_events WHERE session_id = ?",
        [session_id],
        |r| r.get(0),
    )?;
    for ev in &extracted.events {
        let id = format!("{session_id}:{}", ev.id);
        let exists: Option<i64> = conn
            .query_row("SELECT 1 FROM session_events WHERE id = ?", [&id], |r| {
                r.get(0)
            })
            .optional()?;
        if exists.is_some() {
            continue;
        }
        seq += 1;
        let payload_json = ev.payload.as_ref().map(|p| p.to_string());
        let row = SessionEvent {
            id,
            session_id: session_id.to_string(),
            seq,
            source: "transcript".into(),
            event: ev.event.to_string(),
            tool_name: ev.tool_name.clone(),
            tool_use_id: ev.tool_use_id.clone(),
            agent_id: ev.agent_id.clone(),
            state_before: None,
            state_after: None,
            summary: Some(ev.summary.clone()),
            payload_bytes: payload_json.as_ref().map(|p| p.len() as i64).unwrap_or(0),
            payload_json,
            compacted: false,
            occurred_at: if ev.occurred_at.is_empty() {
                now.to_string()
            } else {
                ev.occurred_at.clone()
            },
            received_at: now.to_string(),
        };
        repo::insert(conn, &row)?;
        report.events += 1;
    }

    for s in &extracted.samples {
        let n = conn.execute(
            "INSERT OR IGNORE INTO usage_samples
               (id, sampled_at, source, purpose, session_id, model, input_tokens, output_tokens,
                cache_read_tokens, cache_write_tokens, cost_usd, turn_ref, created_at, cache_write_1h_tokens)
             VALUES (?, ?, 'claude_code', 'session', ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)",
            params![
                repo::new_id(),
                if s.sampled_at.is_empty() {
                    now
                } else {
                    s.sampled_at.as_str()
                },
                session_id,
                s.model,
                s.usage.input,
                s.usage.output,
                s.usage.cache_read,
                s.usage.cache_write,
                s.turn_ref,
                now,
                s.usage.cache_write_1h,
            ],
        )?;
        report.samples += n;
    }
    if report.samples > 0 {
        conn.execute(
            "UPDATE sessions SET
               input_tokens = (SELECT COALESCE(SUM(input_tokens), 0) FROM usage_samples WHERE session_id = ?1),
               output_tokens = (SELECT COALESCE(SUM(output_tokens), 0) FROM usage_samples WHERE session_id = ?1),
               cache_read_tokens = (SELECT COALESCE(SUM(cache_read_tokens), 0) FROM usage_samples WHERE session_id = ?1),
               cache_write_tokens = (SELECT COALESCE(SUM(cache_write_tokens), 0) FROM usage_samples WHERE session_id = ?1)
             WHERE id = ?1",
            [session_id],
        )?;
        report.changed = true;
    }

    let mut sets: Vec<(&str, Box<dyn rusqlite::ToSql>)> = Vec::new();
    if let Some(m) = &extracted.model {
        if row.model.as_deref() != Some(m.as_str()) {
            sets.push(("model", Box::new(m.clone())));
        }
    }
    if let Some(ctx) = extracted.context_tokens {
        if row.context_tokens != Some(ctx) {
            sets.push(("context_tokens", Box::new(ctx)));
        }
    }
    if let Some(b) = &extracted.git_branch {
        if row.git_branch.as_deref() != Some(b.as_str()) {
            sets.push(("git_branch", Box::new(b.clone())));
        }
    }
    let discovered = row.tracking == "discovered";
    let mut cwd = row.cwd.clone();
    if discovered {
        if let Some(c) = &extracted.cwd {
            if row.cwd != *c {
                cwd = c.clone();
                sets.push(("cwd", Box::new(c.clone())));
            }
        }
        if extracted.prompts > 0 {
            sets.push((
                "num_turns",
                Box::new(row.num_turns + i64::from(extracted.prompts)),
            ));
        }
        if let Some(last) = &extracted.last_timestamp {
            if last.as_str() > row.last_event_at.as_str() {
                sets.push(("last_event_at", Box::new(last.clone())));
            }
        }
    }

    // Title precedence (05 §5.3): a hook `session_title` wins; else the
    // transcript `summary`; else the first prompt when previews are on.
    let hook_title = hook_session_title(conn, session_id)?;
    let candidate = extracted
        .title
        .clone()
        .or_else(|| row.title.clone())
        .or_else(|| extracted.first_prompt.clone());
    if hook_title.is_none() {
        if let Some(t) = candidate {
            if row.title.as_deref() != Some(t.as_str()) {
                sets.push(("title", Box::new(t.clone())));
                report.title_changed = true;
                if !custom_named {
                    let name = naming::display_name(
                        Some(&t),
                        row.repo_root.as_deref(),
                        &cwd,
                        &row.claude_session_id,
                    );
                    if name != row.display_name {
                        sets.push(("display_name", Box::new(name)));
                    }
                }
            }
        }
    }
    if !sets.is_empty() {
        sets.push(("updated_at", Box::new(now.to_string())));
        let assigns: Vec<String> = sets.iter().map(|(c, _)| format!("{c} = ?")).collect();
        let sql = format!("UPDATE sessions SET {} WHERE id = ?", assigns.join(", "));
        let mut values: Vec<&dyn rusqlite::ToSql> = sets.iter().map(|(_, v)| v.as_ref()).collect();
        values.push(&session_id);
        conn.execute(&sql, rusqlite::params_from_iter(values))?;
        report.changed = true;
    }
    if report.events > 0 {
        report.changed = true;
    }
    Ok(report)
}

// ------------------------------------------------------------------ tailers

#[derive(Debug)]
struct Tracked {
    tailer: Tailer,
    session_id: String,
    custom_named: bool,
}

/// A session whose transcript was read, and what the read wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Polled {
    pub session_id: String,
    pub path: PathBuf,
    pub report: IngestReport,
}

/// What a `sync` found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct SyncReport {
    pub tracked: usize,
    pub added: usize,
    pub removed: usize,
    /// Child rows created for subagent transcripts no hook announced.
    pub subagents_created: usize,
}

#[derive(Debug, Clone)]
struct Followed {
    id: String,
    claude_session_id: String,
    parent_session_id: Option<String>,
    agent_id: Option<String>,
    transcript_path: Option<String>,
    tracking: String,
    launch_mode: String,
    cwd: String,
    repo_root: Option<String>,
    avatar_color: String,
    avatar_variant: String,
}

fn followed(conn: &Connection, cutoff: &str) -> Result<Vec<Followed>> {
    let mut stmt = conn.prepare(
        "SELECT id, claude_session_id, parent_session_id, agent_id, transcript_path, tracking,
                launch_mode, cwd, repo_root, avatar_color, avatar_variant
           FROM sessions
          WHERE dismissed_at IS NULL AND (ended_at IS NULL OR ended_at > ?)",
    )?;
    let rows = stmt
        .query_map([cutoff], |r| {
            Ok(Followed {
                id: r.get(0)?,
                claude_session_id: r.get(1)?,
                parent_session_id: r.get(2)?,
                agent_id: r.get(3)?,
                transcript_path: r.get(4)?,
                tracking: r.get(5)?,
                launch_mode: r.get(6)?,
                cwd: r.get(7)?,
                repo_root: r.get(8)?,
                avatar_color: r.get(9)?,
                avatar_variant: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn cutoff_rfc3339(secs: i64) -> String {
    let t = time::OffsetDateTime::now_utc() - time::Duration::seconds(secs);
    crate::db::format_timestamp(t)
}

/// The set of tailed transcripts: every live session (and sessions ended in
/// the last hour) with a `transcript_path`, plus the subagent transcripts
/// under each one, which get child rows when no hook announced them.
#[derive(Debug, Default)]
pub struct Tailers {
    tracked: HashMap<PathBuf, Tracked>,
    keep_prompts: bool,
}

impl Tailers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.tracked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tracked.is_empty()
    }

    pub fn is_tracked(&self, path: &Path) -> bool {
        self.tracked.contains_key(path)
    }

    /// The session a tracked path feeds.
    pub fn session_for(&self, path: &Path) -> Option<&str> {
        self.tracked.get(path).map(|t| t.session_id.as_str())
    }

    /// Re-read the roster: start tailing new transcripts, stop tailing gone
    /// ones, create child rows for subagent transcripts without one.
    pub fn sync(&mut self, db: &Db) -> Result<SyncReport> {
        let cutoff = cutoff_rfc3339(ENDED_GRACE_SECS);
        let (rows, keep_prompts, custom): (Vec<Followed>, bool, HashSet<String>) =
            db.read(|c| {
                let rows = followed(c, &cutoff)?;
                let keep = crate::settings::get_bool_in(c, "sessions.store_prompt_previews")?
                    .unwrap_or(false);
                let names: HashMap<String, String> =
                    crate::settings::get_as_in(c, "sessions.custom_names").unwrap_or_default();
                Ok((rows, keep, names.into_keys().collect()))
            })?;
        self.keep_prompts = keep_prompts;
        let mut report = SyncReport::default();
        let mut wanted: HashMap<PathBuf, (String, bool)> = HashMap::new();
        for r in &rows {
            if let Some(p) = &r.transcript_path {
                wanted.insert(
                    crate::paths::to_os(p),
                    (r.id.clone(), custom.contains(&r.claude_session_id)),
                );
            }
        }
        // Subagent transcripts under each top-level session.
        for parent in rows.iter().filter(|r| r.parent_session_id.is_none()) {
            let Some(path) = &parent.transcript_path else {
                continue;
            };
            let Some(dir) = super::subagents_dir_of(&crate::paths::to_os(path)) else {
                continue;
            };
            for (agent_id, file) in super::list_subagents(&dir) {
                if wanted.contains_key(&file) {
                    continue;
                }
                let child = rows.iter().find(|r| {
                    r.parent_session_id.as_deref() == Some(parent.id.as_str())
                        && r.agent_id.as_deref() == Some(agent_id.as_str())
                });
                let child_id = match child {
                    Some(c) => {
                        if c.transcript_path.is_none() {
                            let (id, p) = (c.id.clone(), crate::paths::from_os(&file));
                            db.write(move |conn| {
                                conn.execute(
                                    "UPDATE sessions SET transcript_path = ? WHERE id = ?",
                                    params![p, id],
                                )?;
                                Ok(())
                            })?;
                        }
                        c.id.clone()
                    }
                    // A child that ended before the grace period is not among the followed
                    // rows, but its transcript is still listed: find it, never create it twice.
                    None => match existing_child(db, &parent.id, &agent_id)? {
                        Some(id) => id,
                        None => {
                            let id = create_child_row(db, parent, &agent_id, &file)?;
                            report.subagents_created += 1;
                            id
                        }
                    },
                };
                wanted.insert(file, (child_id, false));
            }
        }
        let gone: Vec<PathBuf> = self
            .tracked
            .keys()
            .filter(|p| !wanted.contains_key(*p))
            .cloned()
            .collect();
        for p in gone {
            self.tracked.remove(&p);
            report.removed += 1;
        }
        for (path, (session_id, custom_named)) in wanted {
            match self.tracked.get_mut(&path) {
                Some(t) => {
                    t.session_id = session_id;
                    t.custom_named = custom_named;
                }
                None => {
                    self.tracked.insert(
                        path.clone(),
                        Tracked {
                            tailer: Tailer::new(path),
                            session_id,
                            custom_named,
                        },
                    );
                    report.added += 1;
                }
            }
        }
        report.tracked = self.tracked.len();
        Ok(report)
    }

    /// Read every transcript that grew; returns the sessions that changed.
    pub fn poll_all(&mut self, db: &Db) -> Result<Vec<Polled>> {
        let paths: Vec<PathBuf> = self
            .tracked
            .iter()
            .filter(|(_, t)| t.tailer.changed())
            .map(|(p, _)| p.clone())
            .collect();
        let mut out = Vec::new();
        for p in paths {
            if let Some(polled) = self.poll_path(db, &p)? {
                out.push(polled);
            }
        }
        Ok(out)
    }

    /// Read one tracked transcript now (a watcher event). `None` when the path
    /// is not tracked or nothing new was written.
    pub fn poll_path(&mut self, db: &Db, path: &Path) -> Result<Option<Polled>> {
        let keep_prompts = self.keep_prompts;
        let Some(t) = self.tracked.get_mut(path) else {
            return Ok(None);
        };
        let lines = match t.tailer.read_new() {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(target: "transcript", path = %path.display(), error = %e, "transcript read failed");
                return Ok(None);
            }
        };
        if lines.is_empty() {
            return Ok(None);
        }
        let records: Vec<records::Record> = lines
            .iter()
            .filter_map(|l| records::parse_line(l))
            .collect();
        let extracted = records::extract(&records, keep_prompts);
        let (session_id, custom_named) = (t.session_id.clone(), t.custom_named);
        let sid = session_id.clone();
        let report = db.write(move |c| {
            let now = crate::db::now_rfc3339();
            repo::tx(c, |tx| ingest(tx, &sid, &extracted, custom_named, &now))
        })?;
        Ok(Some(Polled {
            session_id,
            path: path.to_path_buf(),
            report,
        }))
    }
}

fn file_time(path: &Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| crate::db::format_timestamp(t.into()))
        .unwrap_or_else(|_| crate::db::now_rfc3339())
}

/// The id of the parent's child row for `agent_id`, followed or not.
fn existing_child(db: &Db, parent_id: &str, agent_id: &str) -> Result<Option<String>> {
    let (parent_id, agent_id) = (parent_id.to_string(), agent_id.to_string());
    db.read(move |c| {
        Ok(c.query_row(
            "SELECT id FROM sessions WHERE parent_session_id = ? AND agent_id = ?",
            params![parent_id, agent_id],
            |r| r.get(0),
        )
        .optional()?)
    })
}

/// A child row for a subagent transcript the hooks never announced
/// (05 §4.5): `agent_type` from `.meta.json`, `done` when it says `finished`.
fn create_child_row(db: &Db, parent: &Followed, agent_id: &str, file: &Path) -> Result<String> {
    let meta = super::read_meta(file).unwrap_or_default();
    let mtime = file_time(file);
    let now = crate::db::now_rfc3339();
    let id = repo::new_id();
    let row = Session {
        id: id.clone(),
        claude_session_id: parent.claude_session_id.clone(),
        parent_session_id: Some(parent.id.clone()),
        agent_id: Some(agent_id.to_string()),
        agent_type: meta.agent_type.clone(),
        tracking: parent.tracking.clone(),
        launch_mode: parent.launch_mode.clone(),
        state: if meta.finished { "done" } else { "working" }.into(),
        state_since: if meta.finished {
            mtime.clone()
        } else {
            now.clone()
        },
        state_reason: Some("transcript:subagent".into()),
        display_name: meta
            .agent_type
            .clone()
            .unwrap_or_else(|| format!("agent {agent_id}")),
        title: meta.description.clone(),
        avatar_color: parent.avatar_color.clone(),
        avatar_variant: parent.avatar_variant.clone(),
        cwd: parent.cwd.clone(),
        repo_root: parent.repo_root.clone(),
        transcript_path: Some(crate::paths::from_os(file)),
        started_at: mtime.clone(),
        last_event_at: mtime.clone(),
        ended_at: meta.finished.then(|| mtime.clone()),
        end_reason: meta.finished.then(|| "inferred".to_string()),
        acknowledged_at: meta.finished.then(|| now.clone()),
        created_at: now.clone(),
        updated_at: now,
        ..Default::default()
    };
    db.write(move |c| {
        repo::insert(c, &row)?;
        Ok(())
    })?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn tailer_holds_partial_lines_and_restarts_on_shrink() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jsonl");
        let mut t = Tailer::new(&p);
        assert!(
            t.read_new().unwrap().is_empty(),
            "missing file reads as nothing"
        );
        assert!(!t.changed());
        std::fs::write(&p, "{\"a\":1}\n{\"b\":").unwrap();
        assert!(t.changed());
        let lines = t.read_new().unwrap();
        assert_eq!(lines, vec!["{\"a\":1}".to_string()]);
        assert_eq!(t.partial_len(), 5);
        assert!(
            !t.changed(),
            "the partial bytes are consumed from the file's view"
        );
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        write!(f, "2}}\n\n{{\"c\":3}}\n").unwrap();
        let lines = t.read_new().unwrap();
        assert_eq!(
            lines,
            vec!["{\"b\":2}".to_string(), "{\"c\":3}".to_string()]
        );
        assert_eq!(t.partial_len(), 0);
        std::fs::write(&p, "{\"d\":4}\n").unwrap();
        let lines = t.read_new().unwrap();
        assert_eq!(
            lines,
            vec!["{\"d\":4}".to_string()],
            "shrink restarts from 0"
        );
        assert_eq!(t.offset, 8);
    }
}
