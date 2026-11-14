//! Proposed actions (03 §7.5, ADR-0022): structured suggestions the model
//! returns with an answer, validated against the database, shown as cards
//! with a preview (fields, a diff, before → after), and executed only by
//! `chat_action_confirm` with `confirm = true`. Sending a message or a
//! reply, changing settings and deleting anything are never offered.

use std::sync::{Arc, Weak};

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::model::RawAction;
use crate::connectors::Connectors;
use crate::db::Result;
use crate::notes::Notes;
use crate::pty::Pty;
use crate::scheduler::Scheduler;

/// What the assistant may propose.
pub const ACTION_KINDS: [&str; 8] = [
    "create_note",
    "edit_note",
    "set_note_fields",
    "snooze",
    "archive",
    "resume_session",
    "session_prompt",
    "create_automation",
];

/// The card heading of a kind.
pub fn title_of(kind: &str) -> &'static str {
    match kind {
        "create_note" => "Create note",
        "edit_note" => "Edit note",
        "set_note_fields" => "Update note",
        "snooze" => "Snooze",
        "archive" => "Archive",
        "resume_session" => "Resume session",
        "session_prompt" => "Send prompt",
        "create_automation" => "Create automation",
        _ => "Action",
    }
}

/// A card (`chat.action` payload; stored in `chat_turns` as a `tool` row).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionView {
    pub action_id: String,
    pub turn_id: String,
    pub thread_id: String,
    pub kind: String,
    /// "Create note".
    pub title: String,
    /// One line: "Prepare for 1:1 in Unsorted/".
    pub summary: String,
    /// What the card shows (fields, a unified diff, before → after).
    pub preview: Value,
    /// The validated parameters executed on confirm.
    pub params: Value,
    /// `proposed` | `confirmed` | `rejected` | `failed`.
    pub state: String,
    /// The line that replaces the card ("Created note *X* in Unsorted/").
    pub result_line: Option<String>,
    pub error: Option<String>,
    /// Where the result opens.
    pub route: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

/// A validated proposal.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub kind: String,
    pub summary: String,
    pub preview: Value,
    pub params: Value,
}

/// `(title, status, urgent, when_raw, when_date)` of a note.
type FieldsRow = (String, String, bool, Option<String>, Option<String>);

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

fn one<T: rusqlite::types::FromSql>(conn: &Connection, sql: &str, id: &str) -> Result<Option<T>> {
    Ok(conn.query_row(sql, [id], |r| r.get(0)).optional()?)
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

fn when_ok(w: &str) -> bool {
    matches!(w, "today" | "tomorrow") || crate::notes::fields::parse_date(w).is_some()
}

fn capture_folder(conn: &Connection) -> Result<(Option<String>, String)> {
    Ok(conn
        .query_row(
            "SELECT id, path FROM folders WHERE kind = 'capture' AND deleted_at IS NULL LIMIT 1",
            [],
            |r| Ok((Some(r.get::<_, String>(0)?), r.get::<_, String>(1)?)),
        )
        .optional()?
        .unwrap_or((None, crate::notes::vault::CAPTURE_FOLDER.to_string())))
}

fn known_statuses(conn: &Connection) -> Vec<String> {
    crate::settings::statuses_in(conn)
        .map(|v| v.into_iter().map(|s| s.slug).collect())
        .unwrap_or_else(|_| vec!["todo".into(), "doing".into(), "done".into()])
}

fn tags_of(conn: &Connection, note_id: &str) -> Result<Vec<String>> {
    Ok(conn
        .prepare(
            "SELECT t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
              WHERE nt.note_id = ?1 AND t.deleted_at IS NULL ORDER BY nt.position",
        )?
        .query_map([note_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

fn str_list(v: &Value, k: &str) -> Option<Vec<String>> {
    v.get(k).and_then(Value::as_array).map(|a| {
        a.iter()
            .filter_map(|x| {
                x.as_str()
                    .map(|s| s.trim().trim_start_matches('#').to_string())
            })
            .filter(|s| !s.is_empty())
            .collect()
    })
}

fn item_exists(conn: &Connection, kind: &str, id: &str) -> Result<bool> {
    let sql = match kind {
        "note" => "SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL",
        "message" => "SELECT 1 FROM messages WHERE id = ?1",
        "thread" => "SELECT 1 FROM threads WHERE id = ?1",
        "session" => "SELECT 1 FROM sessions WHERE id = ?1",
        "calendar_event" => "SELECT 1 FROM calendar_events WHERE id = ?1",
        _ => return Ok(false),
    };
    Ok(one::<i64>(conn, sql, id)?.is_some())
}

/// Validate a proposal and build its card; `Err` is the reason it was dropped.
pub fn prepare(
    conn: &Connection,
    raw: &RawAction,
) -> Result<std::result::Result<Prepared, String>> {
    let p = &raw.params;
    let kind = raw.kind.trim().to_ascii_lowercase();
    if !ACTION_KINDS.contains(&kind.as_str()) {
        return Ok(Err(format!(
            "{kind:?} is not something Chat can do (it never sends messages, changes settings or deletes anything)"
        )));
    }
    let out = match kind.as_str() {
        "create_note" => {
            let Some(title) = s(p, "title") else {
                return Ok(Err("a note needs a title".into()));
            };
            let title = clip(&title.replace(['\n', '\r'], " "), 200);
            let (cap_id, cap_path) = capture_folder(conn)?;
            let (folder_id, folder_path, missing) = match s(p, "folder") {
                Some(f) => {
                    let f = f.trim_matches('/').to_string();
                    let hit: Option<(String, String)> = conn
                        .query_row(
                            "SELECT id, path FROM folders WHERE deleted_at IS NULL
                               AND (lower(path) = lower(?1) OR lower(name) = lower(?1))
                             ORDER BY length(path) LIMIT 1",
                            [&f],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional()?;
                    match hit {
                        Some((id, path)) => (Some(id), path, None),
                        None => (cap_id.clone(), cap_path.clone(), Some(f)),
                    }
                }
                None => (cap_id.clone(), cap_path.clone(), None),
            };
            let status = s(p, "status")
                .map(|x| x.to_lowercase())
                .filter(|x| known_statuses(conn).contains(x))
                .unwrap_or_else(|| "todo".into());
            let urgent = p.get("urgent").and_then(Value::as_bool).unwrap_or(false);
            let when = s(p, "when").filter(|w| when_ok(w));
            let tags = str_list(p, "tags").unwrap_or_default();
            let body = s(p, "body").unwrap_or_default();
            let mut links = Vec::new();
            if let Some(a) = p.get("links").and_then(Value::as_array) {
                for l in a {
                    let k = super::citations::normalise_kind(&s(l, "kind").unwrap_or_default());
                    let Some(id) = s(l, "id") else { continue };
                    if item_exists(conn, &k, &id)? {
                        links.push(json!({ "kind": k, "id": id }));
                    }
                }
            }
            let mut preview = json!({
                "folder": format!("{folder_path}/"),
                "title": title,
                "fields": { "status": status, "urgent": urgent, "when": when, "tags": tags },
                "body_preview": clip(&body, 400),
                "links": links.len(),
            });
            if let Some(m) = missing {
                preview["notice"] =
                    json!(format!("No folder {m}; the note goes to {folder_path}/"));
            }
            Prepared {
                summary: format!("{title} in {folder_path}/"),
                params: json!({
                    "title": title, "folder_id": folder_id, "folder_path": folder_path,
                    "status": status, "urgent": urgent, "when": when, "tags": tags,
                    "body": body, "links": links,
                }),
                preview,
                kind,
            }
        }
        "edit_note" => {
            let (Some(note_id), Some(body)) =
                (s(p, "note_id"), p.get("body").and_then(Value::as_str))
            else {
                return Ok(Err("an edit needs note_id and body".into()));
            };
            let row: Option<(String, String, Option<String>)> = conn
                .query_row(
                    "SELECT title, body, content_hash FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                    [&note_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((title, old, hash)) = row else {
                return Ok(Err(format!("no note {note_id}")));
            };
            let new = format!("{}\n", body.trim_end_matches(['\n', '\r']));
            if new.trim_end() == old.trim_end() {
                return Ok(Err("the proposed body is the same".into()));
            }
            let diff = similar::TextDiff::from_lines(old.as_str(), new.as_str())
                .unified_diff()
                .context_radius(3)
                .header("before", "after")
                .to_string();
            Prepared {
                summary: title.clone(),
                preview: json!({ "title": title, "diff": clip(&diff, 8000) }),
                params: json!({ "note_id": note_id, "title": title, "body": new, "base_hash": hash }),
                kind,
            }
        }
        "set_note_fields" => {
            let Some(note_id) = s(p, "note_id") else {
                return Ok(Err("set_note_fields needs note_id".into()));
            };
            let row: Option<FieldsRow> = conn
                .query_row(
                    "SELECT title, status, urgent, when_raw, when_date FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                    [&note_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?;
            let Some((title, status, urgent, when_raw, when_date)) = row else {
                return Ok(Err(format!("no note {note_id}")));
            };
            let mut changes = Vec::new();
            let mut params = json!({ "note_id": note_id, "title": title });
            if let Some(st) = s(p, "status").map(|x| x.to_lowercase())
                && st != status
                && known_statuses(conn).contains(&st)
            {
                changes.push(json!({ "field": "status", "before": status, "after": st }));
                params["status"] = json!(st);
            }
            if let Some(u) = p.get("urgent").and_then(Value::as_bool)
                && u != urgent
            {
                changes.push(json!({ "field": "urgent", "before": urgent, "after": u }));
                params["urgent"] = json!(u);
            }
            if let Some(w) = p.get("when") {
                let after = w
                    .as_str()
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                    .map(str::to_string);
                let before = when_raw.or(when_date);
                if after.as_deref().is_none_or(when_ok) && after != before {
                    changes.push(json!({ "field": "when", "before": before, "after": after }));
                    params["when"] = json!(after);
                }
            }
            if let Some(t) = str_list(p, "tags") {
                let before = tags_of(conn, &note_id)?;
                if t != before {
                    changes.push(json!({ "field": "tags", "before": before, "after": t }));
                    params["tags"] = json!(t);
                }
            }
            if changes.is_empty() {
                return Ok(Err("nothing would change".into()));
            }
            Prepared {
                summary: title.clone(),
                preview: json!({ "title": title, "changes": changes }),
                params,
                kind,
            }
        }
        "snooze" | "archive" => {
            let target = s(p, "target_kind").unwrap_or_else(|| "thread".into());
            if !matches!(target.as_str(), "thread" | "message") {
                return Ok(Err(
                    "only a thread or a message can be snoozed or archived".into()
                ));
            }
            let Some(id) = s(p, "id") else {
                return Ok(Err("which thread?".into()));
            };
            let row: Option<(String, String)> = if target == "thread" {
                conn.query_row(
                    "SELECT title, provider FROM threads WHERE id = ?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
            } else {
                conn.query_row(
                    "SELECT COALESCE(subject, sender_name || ': ' || substr(snippet, 1, 60)), provider FROM messages WHERE id = ?1",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
            };
            let Some((title, provider)) = row else {
                return Ok(Err(format!("no {target} {id}")));
            };
            if kind == "snooze" {
                let until = s(p, "until").filter(|u| crate::db::parse_timestamp(u).is_some());
                let preset = if until.is_some() {
                    None
                } else {
                    Some(
                        s(p, "preset")
                            .filter(|x| {
                                matches!(x.as_str(), "later_today" | "tomorrow" | "next_week")
                            })
                            .unwrap_or_else(|| "tomorrow".into()),
                    )
                };
                let label = match (preset.as_deref(), until.as_deref()) {
                    (Some("later_today"), _) => "later today".to_string(),
                    (Some("tomorrow"), _) => "tomorrow".to_string(),
                    (Some("next_week"), _) => "next week".to_string(),
                    (_, Some(u)) => u.to_string(),
                    _ => "tomorrow".to_string(),
                };
                Prepared {
                    summary: format!("{title} until {label}"),
                    preview: json!({ "title": title, "until": label }),
                    params: json!({ "target_kind": target, "id": id, "title": title, "preset": preset, "until": until }),
                    kind,
                }
            } else {
                let note = if provider == "google" {
                    "Gmail archives it at the source too."
                } else {
                    "Archived in Mode; Slack keeps the message."
                };
                Prepared {
                    summary: title.clone(),
                    preview: json!({ "title": title, "write_back": note }),
                    params: json!({ "target_kind": target, "id": id, "title": title }),
                    kind,
                }
            }
        }
        "resume_session" | "session_prompt" => {
            let Some(sid) = s(p, "session_id") else {
                return Ok(Err("which session?".into()));
            };
            let row: Option<(String, String, String)> = conn
                .query_row(
                    "SELECT display_name, cwd, state FROM sessions WHERE id = ?1",
                    [&sid],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((name, cwd, state)) = row else {
                return Ok(Err(format!("no session {sid}")));
            };
            if kind == "resume_session" {
                Prepared {
                    summary: format!("{name} · {cwd}"),
                    preview: json!({ "name": name, "cwd": cwd, "state": state }),
                    params: json!({ "session_id": sid, "name": name }),
                    kind,
                }
            } else {
                let Some(text) = s(p, "text") else {
                    return Ok(Err("the prompt is empty".into()));
                };
                Prepared {
                    summary: format!("{name} · {}", clip(&text, 60)),
                    preview: json!({ "name": name, "cwd": cwd, "prompt": text }),
                    params: json!({ "session_id": sid, "name": name, "text": text }),
                    kind,
                }
            }
        }
        "create_automation" => {
            let tz = crate::scheduler::schedule::device_zone();
            let mut draft_v = p.clone();
            if draft_v.get("enabled").is_none() {
                draft_v["enabled"] = json!(true);
            }
            let draft: crate::scheduler::store::AutomationDraft =
                match serde_json::from_value(draft_v) {
                    Ok(d) => d,
                    Err(e) => return Ok(Err(format!("the automation is incomplete: {e}"))),
                };
            match crate::scheduler::store::check_draft(conn, &draft, &tz)? {
                Err(errors) => {
                    return Ok(Err(errors
                        .iter()
                        .map(|e| format!("{}: {}", e.field, e.message))
                        .collect::<Vec<_>>()
                        .join("; ")));
                }
                Ok((sched, action)) => {
                    let trig = crate::scheduler::schedule::trigger_phrase(&sched, &tz);
                    let act = crate::scheduler::actions::action_phrase(&action, &|id| {
                        crate::scheduler::store::folder_label(conn, id)
                    });
                    Prepared {
                        summary: format!("{} · {trig}", draft.name),
                        preview: json!({ "name": draft.name, "trigger_phrase": trig, "action_phrase": act, "enabled": draft.enabled }),
                        params: serde_json::to_value(&draft).unwrap_or(Value::Null),
                        kind,
                    }
                }
            }
        }
        _ => return Ok(Err("unknown action".into())),
    };
    Ok(Ok(out))
}

/// The result of an executed action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applied {
    /// "Created note *Prepare for 1:1* in Unsorted/".
    pub line: String,
    pub route: Option<String>,
}

/// Executes confirmed actions; the app's is [`LiveEffects`], tests stub it.
pub trait Effects: Send + Sync {
    fn apply(&self, kind: &str, params: &Value) -> std::result::Result<Applied, String>;
}

/// No services: every action reports what is missing.
#[derive(Debug, Default)]
pub struct NoEffects;

impl Effects for NoEffects {
    fn apply(&self, kind: &str, _params: &Value) -> std::result::Result<Applied, String> {
        Err(format!("{} is not available right now", title_of(kind)))
    }
}

/// The app's executor: the same operations as the UI (ADR-0022).
#[derive(Default, Clone)]
pub struct LiveEffects {
    pub notes: Option<Arc<Notes>>,
    pub connectors: Option<Arc<Connectors>>,
    pub pty: Option<Arc<Pty>>,
    pub scheduler: Option<Weak<Scheduler>>,
}

impl std::fmt::Debug for LiveEffects {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveEffects")
            .field("notes", &self.notes.is_some())
            .field("connectors", &self.connectors.is_some())
            .field("pty", &self.pty.is_some())
            .finish()
    }
}

fn need<'a, T>(o: &'a Option<Arc<T>>, what: &str) -> std::result::Result<&'a Arc<T>, String> {
    o.as_ref()
        .ok_or_else(|| format!("{what} is not available right now"))
}

fn write_ok(
    r: crate::notes::api::ApiResult<crate::notes::api::NoteWriteResult>,
) -> std::result::Result<(), String> {
    use crate::notes::api::NoteWriteResult;
    match r.map_err(|e| e.message)? {
        NoteWriteResult::Saved { .. } => Ok(()),
        NoteWriteResult::Conflict { .. } => Err("The note changed on disk; ask again.".into()),
        NoteWriteResult::Failed { message } => Err(message),
    }
}

/// Create a note as ⌘/Ctrl+N + field edits, links `created_by = 'ai'`.
pub fn create_note(notes: &Notes, p: &Value) -> std::result::Result<Applied, String> {
    use crate::notes::api::{self, NoteCreateArgs, NoteSetFieldArgs};
    use crate::notes::write::FieldEdit;
    let title = s(p, "title").unwrap_or_else(|| "Untitled".into());
    let body = s(p, "body").unwrap_or_default();
    let folder_id = match s(p, "folder_id") {
        Some(f) => Some(f),
        None => Some(
            notes
                .write(crate::notes::reconcile::capture_folder)
                .map_err(|e| e.to_string())?
                .0,
        ),
    };
    let text = if body.is_empty() {
        format!("# {title}\n")
    } else {
        format!("# {title}\n\n{}\n", body.trim_end())
    };
    let d = api::note_create(notes, NoteCreateArgs { folder_id, text }).map_err(|e| e.message)?;
    let id = d.row.id.clone();
    notes.set_open(&id, false);
    let mut edits = Vec::new();
    if let Some(st) = s(p, "status").filter(|x| x != "todo") {
        edits.push(FieldEdit::Status(st));
    }
    if p.get("urgent").and_then(Value::as_bool) == Some(true) {
        edits.push(FieldEdit::Urgent(true));
    }
    if let Some(w) = s(p, "when") {
        edits.push(FieldEdit::When(Some(w)));
    }
    if let Some(t) = str_list(p, "tags").filter(|t| !t.is_empty()) {
        edits.push(FieldEdit::Tags(t));
    }
    for edit in edits {
        write_ok(api::note_set_field(
            notes,
            NoteSetFieldArgs {
                note_id: id.clone(),
                edit,
                buffer: None,
            },
        ))?;
    }
    if let Some(links) = p
        .get("links")
        .and_then(Value::as_array)
        .filter(|l| !l.is_empty())
    {
        let links = links.clone();
        let nid = id.clone();
        notes
            .write(move |cx| {
                for l in &links {
                    if let (Some(k), Some(t)) = (s(l, "kind"), s(l, "id")) {
                        crate::notes::write::link(
                            cx,
                            ("note", &nid),
                            (&k, &t),
                            "references",
                            None,
                            "ai",
                        )?;
                    }
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        notes.changed();
    }
    let folder = s(p, "folder_path").unwrap_or_default();
    Ok(Applied {
        line: format!("Created note *{title}* in {folder}/"),
        route: Some(format!("notes/{id}")),
    })
}

/// Replace a note's body, keeping its frontmatter; refused when the file
/// changed since the proposal.
pub fn edit_note(notes: &Notes, p: &Value) -> std::result::Result<Applied, String> {
    use crate::notes::api::{self, NoteIdArgs, NoteSaveArgs};
    let note_id = s(p, "note_id").ok_or("no note")?;
    let d = api::note_get(
        notes,
        NoteIdArgs {
            note_id: note_id.clone(),
        },
    )
    .map_err(|e| e.message)?;
    if let Some(base) = s(p, "base_hash")
        && base != d.content_hash
    {
        return Err("The note changed since the proposal; ask again.".into());
    }
    let doc = crate::notes::frontmatter::Document::parse(&d.text);
    let prefix_len = d.text.len() - doc.body().len();
    let body = p.get("body").and_then(Value::as_str).unwrap_or("");
    let text = format!("{}{}", &d.text[..prefix_len], body);
    write_ok(api::note_save(
        notes,
        NoteSaveArgs {
            note_id: note_id.clone(),
            text,
            base_hash: d.content_hash.clone(),
            base_rev: Some(d.rev),
            resolve: None,
            checkpoint: Some(true),
        },
    ))?;
    Ok(Applied {
        line: format!("Updated *{}*", d.row.title),
        route: Some(format!("notes/{note_id}")),
    })
}

/// Set fields one frontmatter rewrite at a time (03 §7.5).
pub fn set_note_fields(notes: &Notes, p: &Value) -> std::result::Result<Applied, String> {
    use crate::notes::api::{self, NoteSetFieldArgs};
    use crate::notes::write::FieldEdit;
    let note_id = s(p, "note_id").ok_or("no note")?;
    let mut edits = Vec::new();
    let mut words = Vec::new();
    if let Some(st) = s(p, "status") {
        words.push(format!("status {st}"));
        edits.push(FieldEdit::Status(st));
    }
    if let Some(u) = p.get("urgent").and_then(Value::as_bool) {
        words.push(if u {
            "urgent".into()
        } else {
            "not urgent".into()
        });
        edits.push(FieldEdit::Urgent(u));
    }
    if let Some(w) = p.get("when") {
        let when = w.as_str().map(str::to_string);
        words.push(match &when {
            Some(x) => format!("when {x}"),
            None => "no date".into(),
        });
        edits.push(FieldEdit::When(when));
    }
    if let Some(t) = str_list(p, "tags") {
        words.push(format!("tags {}", t.join(", ")));
        edits.push(FieldEdit::Tags(t));
    }
    for edit in edits {
        write_ok(api::note_set_field(
            notes,
            NoteSetFieldArgs {
                note_id: note_id.clone(),
                edit,
                buffer: None,
            },
        ))?;
    }
    Ok(Applied {
        line: format!(
            "Set {} on *{}*",
            words.join(", "),
            s(p, "title").unwrap_or_default()
        ),
        route: Some(format!("notes/{note_id}")),
    })
}

impl Effects for LiveEffects {
    fn apply(&self, kind: &str, p: &Value) -> std::result::Result<Applied, String> {
        match kind {
            "create_note" => create_note(need(&self.notes, "Notes")?, p),
            "edit_note" => edit_note(need(&self.notes, "Notes")?, p),
            "set_note_fields" => set_note_fields(need(&self.notes, "Notes")?, p),
            "snooze" => {
                use crate::connectors::api::{SnoozeArgs, thread_snooze};
                let c = need(&self.connectors, "Inbox")?;
                let preset = p
                    .get("preset")
                    .filter(|v| !v.is_null())
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                let r = thread_snooze(
                    c,
                    SnoozeArgs {
                        id: s(p, "id").unwrap_or_default(),
                        kind: s(p, "target_kind"),
                        preset,
                        until: s(p, "until"),
                        wake_on_reply: None,
                    },
                )
                .map_err(|e| e.message)?;
                Ok(Applied {
                    line: format!(
                        "Snoozed *{}* until {}",
                        s(p, "title").unwrap_or_default(),
                        r.until
                    ),
                    route: None,
                })
            }
            "archive" => {
                use crate::connectors::api::{ArchiveArgs, thread_archive};
                let c = need(&self.connectors, "Inbox")?;
                let id = s(p, "id").unwrap_or_default();
                let kind = s(p, "target_kind");
                thread_archive(
                    c,
                    ArchiveArgs {
                        id: id.clone(),
                        kind: kind.clone(),
                        archived: true,
                    },
                )
                .map_err(|e| e.message)?;
                Ok(Applied {
                    line: format!("Archived *{}*", s(p, "title").unwrap_or_default()),
                    route: Some(format!(
                        "inbox/{}/{id}",
                        if kind.as_deref() == Some("message") {
                            "message"
                        } else {
                            "thread"
                        }
                    )),
                })
            }
            "resume_session" => {
                let pty = need(&self.pty, "The terminal host")?;
                let sid = s(p, "session_id").unwrap_or_default();
                let r = crate::pty::api::session_resume(
                    pty,
                    crate::pty::api::ResumeArgs {
                        session_id: sid,
                        ..Default::default()
                    },
                )
                .map_err(|e| e.message)?;
                if let Some(e) = r.error {
                    return Err(e);
                }
                Ok(Applied {
                    line: format!("Resumed *{}*", s(p, "name").unwrap_or_default()),
                    route: Some(format!("sessions/{}/terminal", r.session_id)),
                })
            }
            "session_prompt" => {
                let pty = need(&self.pty, "The terminal host")?;
                let sid = s(p, "session_id").unwrap_or_default();
                crate::pty::api::session_prompt(
                    pty,
                    crate::pty::api::PromptArgs {
                        session_id: sid.clone(),
                        text: s(p, "text").unwrap_or_default(),
                    },
                )
                .map_err(|e| e.message)?;
                Ok(Applied {
                    line: format!("Sent the prompt to *{}*", s(p, "name").unwrap_or_default()),
                    route: Some(format!("sessions/{sid}")),
                })
            }
            "create_automation" => {
                let sched = self
                    .scheduler
                    .as_ref()
                    .and_then(Weak::upgrade)
                    .ok_or("Automations are not available right now")?;
                let draft: crate::scheduler::store::AutomationDraft =
                    serde_json::from_value(p.clone()).map_err(|e| e.to_string())?;
                let a = crate::scheduler::api::automation_save(
                    &sched,
                    crate::scheduler::api::SaveArgs { draft },
                )
                .map_err(|e| {
                    if e.fields.is_empty() {
                        e.message
                    } else {
                        e.fields
                            .iter()
                            .map(|f| f.message.clone())
                            .collect::<Vec<_>>()
                            .join(" ")
                    }
                })?;
                Ok(Applied {
                    line: format!("Created automation *{}* ({})", a.name, a.trigger_phrase),
                    route: Some(format!("automations/{}", a.id)),
                })
            }
            other => Err(format!("{other} is not something Chat can do")),
        }
    }
}
