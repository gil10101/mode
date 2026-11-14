//! Mode's own writes (04 §3.4, §9.2–§9.4, §11.4, §12, §5.4): field edits,
//! editor saves with the conflict rule, new notes, archive / pin / move,
//! trash and its undo, restore from history, tag rename / merge / delete.
//!
//! Every file write is atomic (temp + rename) and indexes the new content
//! in the same writer job, so the watcher event that follows matches the
//! row and is skipped (09 §8.2). Every user-visible write sets
//! `updated = now` and `rev = rev + 1` in the file and the row together
//! (04 §3.4). A file that changed on disk since it was indexed is
//! reconciled first (04 §9.4: "retried from a fresh read").

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::body;
use super::ctx::{Cx, invalid};
use super::eol::{self, Eol};
use super::fields::{self, WhenRaw};
use super::frontmatter::{self, Canonical, Document, WriteOpts};
use super::fsio;
use super::reconcile::{self, ApplyOpts, FileState, FileView, Prepared};
use super::vault;
use crate::db::{DbError, Result};
use crate::repo::{self, Link, Note};

/// A one-field write (04 §9.2, §6.5; the overlay row actions of 02 §7.7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "field", content = "value", rename_all = "snake_case")]
pub enum FieldEdit {
    Status(String),
    Urgent(bool),
    /// `today`, `tomorrow`, a weekday, `YYYY-MM-DD`, a relative token; `None` clears.
    When(Option<String>),
    Tags(Vec<String>),
    Pinned(bool),
    FolderId(Option<String>),
    Archived(bool),
}

impl FieldEdit {
    /// The frontmatter key this edit writes, if it writes the file.
    pub fn key(&self) -> Option<&'static str> {
        match self {
            FieldEdit::Status(_) => Some("status"),
            FieldEdit::Urgent(_) => Some("urgent"),
            FieldEdit::When(_) => Some("when"),
            FieldEdit::Tags(_) => Some("tags"),
            _ => None,
        }
    }
}

/// The editor's three answers to a conflict (04 §11.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolve {
    Mine,
    Theirs,
    Both,
}

/// What a write did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The row and the file as they now are.
    Saved {
        note: Box<Note>,
        text: String,
        copy_id: Option<String>,
    },
    /// The file's hash is not the buffer's base: nothing was written.
    Conflict {
        disk_text: String,
        disk_hash: String,
        disk_rev: i64,
    },
}

// ---------------------------------------------------------------- helpers

/// Write `text` to `rel` and index it as Mode's write. `prior` is the row
/// this file belongs to (`None` for a new note).
pub fn commit(
    cx: &Cx,
    rel: &str,
    text: &str,
    prior: Option<&Note>,
    create: bool,
    origin: Option<&str>,
    source_kind: &str,
) -> Result<Note> {
    let abs = vault::abs_path(&cx.env.root, rel);
    if let Some(dir) = abs.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if create {
        fsio::atomic_create(&abs, text.as_bytes())?;
    } else {
        fsio::atomic_write(&abs, text.as_bytes())?;
    }
    let stat = fsio::stat(&abs)?;
    let hash = fsio::sha256_hex(text.as_bytes());
    let view = FileView::new(cx, rel, text.as_bytes(), &stat);
    let mut note = prior.cloned().unwrap_or_else(|| Note {
        id: view.fields.id.clone().unwrap_or_else(repo::new_id),
        source_kind: source_kind.to_string(),
        ..Default::default()
    });
    note.path = rel.to_string();
    note.folder_id = reconcile::ensure_folder(cx, vault::parent_dir(rel))?;
    match (view.fields.rev, &view.fields.updated) {
        (Some(rev), Some(updated)) if view.writable => {
            note.rev = rev;
            note.updated_at = updated.clone();
        }
        _ => {
            // A buffer whose frontmatter cannot be read is written as is:
            // the row runs ahead of the file (04 §3.3).
            note.rev = prior.map(|p| p.rev + 1).unwrap_or(1);
            note.updated_at = cx.now().to_string();
        }
    }
    note.created_at = view
        .fields
        .created
        .clone()
        .or_else(|| prior.map(|p| p.created_at.clone()))
        .unwrap_or_else(|| cx.now().to_string());
    view.fill(cx, &mut note, prior);
    note.device_id = cx.device().to_string();
    note.deleted_at = None;
    note.content_hash = Some(hash.clone());
    note.file_mtime = Some(stat.mtime.clone());
    note.file_size = Some(stat.size);
    note.indexed_at = Some(cx.now().to_string());
    note.frontmatter_dirty = view.writable
        && (view.fields.id.as_deref() != Some(note.id.as_str())
            || view.fields.rev != Some(note.rev));
    reconcile::store(cx, &mut note, prior.is_some(), &view.tags)?;
    if let Some(origin) = origin
        && text.len() as u64 <= vault::MAX_VERSIONED_BYTES
    {
        reconcile::insert_revision(cx, &note.id, note.rev, origin, text, &hash)?;
    }
    Ok(note)
}

/// The live note and its file, reconciled first when the file changed
/// since it was indexed.
pub fn fresh(cx: &Cx, note_id: &str) -> Result<(Note, String, String)> {
    let note = reconcile::live_note(cx.conn, note_id)?;
    let (text, stat, hash) = match reconcile::read_writable(cx, &note.path) {
        Ok(v) => v,
        Err(DbError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(invalid("note", "the file was deleted outside Mode"));
        }
        Err(e) => return Err(e),
    };
    if note.content_hash.as_deref() == Some(hash.as_str()) {
        return Ok((note, text, hash));
    }
    let p = Prepared {
        rel: note.path.clone(),
        state: FileState::Read {
            stat,
            bytes: text.clone().into_bytes(),
            hash: hash.clone(),
        },
    };
    reconcile::apply(cx, &p, &ApplyOpts::default())?;
    let note = reconcile::live_note(cx.conn, note_id)?;
    Ok((note, text, hash))
}

fn parse_writable(text: &str) -> Result<Document> {
    let doc = Document::parse(text);
    if let Some(reason) = doc.error() {
        return Err(invalid(
            "frontmatter",
            format!("Frontmatter could not be read ({reason}); fix it in raw source"),
        ));
    }
    Ok(doc)
}

/// The keys a user-visible write sets on top of `extra`.
fn edited_keys(doc: &Document, note: &Note, cx: &Cx, extra: &[&'static str]) -> Vec<&'static str> {
    let f = doc.fields(cx.tz());
    let mut keys: Vec<&'static str> = vec!["rev", "updated"];
    if f.id.as_deref() != Some(note.id.as_str()) {
        keys.push("id");
    }
    if f.created.is_none() {
        keys.push("created");
    }
    keys.extend_from_slice(extra);
    keys
}

/// Resolve a `when` value read from a buffer: the row's frozen date while
/// the text is unchanged, else against today.
fn when_from(
    cx: &Cx,
    w: Option<&WhenRaw>,
    raw: Option<&str>,
    note: Option<&Note>,
) -> Option<String> {
    let w = w?;
    if let Some(n) = note
        && n.when_raw.as_deref() == raw
        && n.when_date.is_some()
    {
        return n.when_date.clone();
    }
    Some(fields::fmt_date(w.resolve(cx.today())))
}

fn validate_status(s: &str) -> Result<String> {
    let s = fields::fold_status(s);
    if s.is_empty() || s.chars().count() > 64 {
        return Err(invalid("status", "a status needs a name"));
    }
    Ok(s)
}

/// Apply a frontmatter edit to `c`; returns the key it set.
fn apply_edit(cx: &Cx, c: &mut Canonical, edit: &FieldEdit) -> Result<()> {
    match edit {
        FieldEdit::Status(s) => c.status = validate_status(s)?,
        FieldEdit::Urgent(b) => c.urgent = *b,
        FieldEdit::When(v) => {
            c.when = match v {
                None => None,
                Some(v) => fields::resolve_when_input(v, cx.today(), cx.week_start)
                    .map_err(|e| invalid("when", e))?
                    .map(fields::fmt_date),
            }
        }
        FieldEdit::Tags(list) => {
            let (tags, bad) = fields::normalize_tags(list);
            if let Some(b) = bad.first() {
                return Err(invalid("tags", format!("{b:?} is not a valid tag")));
            }
            c.tags = tags;
        }
        _ => {}
    }
    Ok(())
}

// ------------------------------------------------------------ field edits

/// One field write (04 §9.2). `buffer` is the editor's unsaved text and the
/// hash it was based on: it is written first, in the same atomic write.
pub fn set_field(
    cx: &Cx,
    note_id: &str,
    edit: &FieldEdit,
    buffer: Option<(&str, &str)>,
) -> Result<Outcome> {
    match edit {
        FieldEdit::Pinned(b) => {
            return flag(
                cx,
                note_id,
                "pinned",
                rusqlite::types::Value::Integer(i64::from(*b)),
            );
        }
        FieldEdit::Archived(b) => {
            let v = if *b {
                rusqlite::types::Value::Text(cx.now().to_string())
            } else {
                rusqlite::types::Value::Null
            };
            return flag(cx, note_id, "archived_at", v);
        }
        FieldEdit::FolderId(f) => return move_to(cx, note_id, f.as_deref()),
        _ => {}
    }
    let (note, disk_text, disk_hash) = fresh(cx, note_id)?;
    let text = match buffer {
        Some((_, base)) if base != disk_hash => {
            return Ok(conflict(cx, &note, disk_text, disk_hash));
        }
        // The editor's buffer goes in the file's own line ending (04 §9.4).
        Some((buf, _)) => eol::normalize(buf, eol::of_existing(&disk_text)).into_owned(),
        // Frontmatter only: the body bytes stay verbatim (04 §3.4).
        None => disk_text,
    };
    let doc = parse_writable(&text)?;
    let f = doc.fields(cx.tz());
    let mut c = reconcile::canonical_from_row(&note, f.tags.clone(), true);
    if buffer.is_some() {
        // The buffer may carry raw-source edits of the frontmatter.
        c.status = f.status.clone().unwrap_or_else(|| "todo".into());
        c.urgent = f.urgent.unwrap_or(false);
        c.when = when_from(cx, f.when.as_ref(), f.when_raw.as_deref(), Some(&note));
    }
    apply_edit(cx, &mut c, edit)?;
    c.rev = note.rev.max(f.rev.unwrap_or(0)) + 1;
    c.updated = cx.now().to_string();
    let key = edit.key().into_iter().collect::<Vec<_>>();
    let edited = edited_keys(&doc, &note, cx, &key);
    let new_text = frontmatter::rewrite(&doc, &c, &WriteOpts::editing(&edited))
        .ok_or_else(|| invalid("frontmatter", "Frontmatter could not be read"))?;
    let note = commit(
        cx,
        &note.path,
        &new_text,
        Some(&note),
        false,
        Some("mode"),
        &note.source_kind,
    )?;
    Ok(Outcome::Saved {
        note: Box::new(note),
        text: new_text,
        copy_id: None,
    })
}

fn conflict(cx: &Cx, note: &Note, disk_text: String, disk_hash: String) -> Outcome {
    let disk_rev = Document::parse(&disk_text)
        .fields(cx.tz())
        .rev
        .unwrap_or(note.rev);
    Outcome::Conflict {
        disk_text,
        disk_hash,
        disk_rev,
    }
}

/// A SQLite-only flag (pin, archive; 04 §12): `rev + 1`, `updated = now`,
/// and a touch-up writes the new `rev` (04 §11.2).
fn flag(cx: &Cx, note_id: &str, column: &str, value: rusqlite::types::Value) -> Result<Outcome> {
    let n = cx.conn.execute(
        &format!(
            "UPDATE notes SET {column} = ?1, rev = rev + 1, updated_at = ?2, device_id = ?3, frontmatter_dirty = 1
              WHERE id = ?4 AND deleted_at IS NULL"
        ),
        params![value, cx.now(), cx.device(), note_id],
    )?;
    if n == 0 {
        return Err(DbError::NotFound {
            table: "notes",
            key: note_id.to_string(),
        });
    }
    saved_from_disk(cx, note_id)
}

fn saved_from_disk(cx: &Cx, note_id: &str) -> Result<Outcome> {
    let note = reconcile::live_note(cx.conn, note_id)?;
    let text =
        std::fs::read_to_string(vault::abs_path(&cx.env.root, &note.path)).unwrap_or_default();
    Ok(Outcome::Saved {
        note: Box::new(note),
        text,
        copy_id: None,
    })
}

/// Move a note into another folder (`None` = the vault root), keeping its
/// file name (` 2` on a collision); `rev + 1`, touch-up scheduled (04 §11.3).
pub fn move_to(cx: &Cx, note_id: &str, folder_id: Option<&str>) -> Result<Outcome> {
    let note = reconcile::live_note(cx.conn, note_id)?;
    let dir = match folder_id {
        None | Some("root") | Some("") => String::new(),
        Some(id) => reconcile::folder_path(cx.conn, id)?,
    };
    if vault::parent_dir(&note.path) == dir {
        return saved_from_disk(cx, note_id);
    }
    let stem = vault::title_from_path(&note.path);
    std::fs::create_dir_all(vault::abs_path(&cx.env.root, &dir))?;
    let rel = vault::unique_name(&cx.env.root, &dir, &stem)?;
    fsio::move_file(
        &vault::abs_path(&cx.env.root, &note.path),
        &vault::abs_path(&cx.env.root, &rel),
    )?;
    let folder = reconcile::ensure_folder(cx, &dir)?;
    cx.conn.execute(
        "UPDATE notes SET path = ?1, folder_id = ?2, rev = rev + 1, updated_at = ?3, device_id = ?4,
                          frontmatter_dirty = 1
          WHERE id = ?5",
        params![rel, folder, cx.now(), cx.device(), note_id],
    )?;
    saved_from_disk(cx, note_id)
}

/// Rename a note's file (the editor's Rename, 04 §2.4): `title` becomes the
/// file name; the H1 is not touched.
pub fn rename(cx: &Cx, note_id: &str, title: &str) -> Result<Outcome> {
    let note = reconcile::live_note(cx.conn, note_id)?;
    let dir = vault::parent_dir(&note.path).to_string();
    let stem = vault::stem_for_title(title);
    let current = vault::title_from_path(&note.path);
    if current == stem {
        return saved_from_disk(cx, note_id);
    }
    let from = vault::abs_path(&cx.env.root, &note.path);
    let rel = if current.to_lowercase() == stem.to_lowercase() {
        // A case-only rename goes through a temporary name so it also works
        // on case-insensitive volumes (macOS, Windows).
        let rel = if dir.is_empty() {
            format!("{stem}.md")
        } else {
            format!("{dir}/{stem}.md")
        };
        let tmp = from.with_file_name(format!(".mode-rename-{}", note.id));
        std::fs::rename(&from, &tmp)?;
        std::fs::rename(&tmp, vault::abs_path(&cx.env.root, &rel))?;
        rel
    } else {
        let rel = vault::unique_name(&cx.env.root, &dir, &stem)?;
        fsio::move_file(&from, &vault::abs_path(&cx.env.root, &rel))?;
        rel
    };
    cx.conn.execute(
        "UPDATE notes SET path = ?1, rev = rev + 1, updated_at = ?2, device_id = ?3, frontmatter_dirty = 1
          WHERE id = ?4",
        params![rel, cx.now(), cx.device(), note_id],
    )?;
    saved_from_disk(cx, note_id)
}

/// Note level: todo → doing → done → todo; any other status → todo (03 §5.3).
pub fn next_status(status: &str) -> &'static str {
    match status {
        "todo" => "doing",
        "doing" => "done",
        _ => "todo",
    }
}

// ------------------------------------------------------------- editor save

/// The editor's save (04 §9.3) with the conflict rule of §11.4: the buffer
/// is written only over a file whose hash is `base_hash`, unless the user
/// chose how to resolve. `record_revision` is the 60 s coalescing decision
/// (always true on blur, close and conflict answers).
pub fn save(
    cx: &Cx,
    note_id: &str,
    text: &str,
    base_hash: &str,
    resolve: Option<Resolve>,
    record_revision: bool,
) -> Result<Outcome> {
    let row = reconcile::by_id(cx.conn, note_id)?.ok_or_else(|| DbError::NotFound {
        table: "notes",
        key: note_id.to_string(),
    })?;
    let abs = vault::abs_path(&cx.env.root, &row.path);
    let disk = match std::fs::read(&abs) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let Some(disk) = disk else {
        // Deleted outside Mode: "Save again" recreates the file, same id (§11.3).
        let text = eol::normalize(text, eol::NEW_FILE);
        return write_buffer(cx, &row, &text, true, true);
    };
    // The buffer is written in the file's own line ending (04 §9.4).
    let text = eol::normalize(text, eol::of_existing(&String::from_utf8_lossy(&disk)));
    let text = text.as_ref();
    let disk_hash = fsio::sha256_hex(&disk);
    let mut note = reconcile::live_note(cx.conn, note_id)?;
    if disk_hash != base_hash {
        if fsio::sha256_hex(text.as_bytes()) == disk_hash {
            let (note, text, _) = fresh(cx, note_id)?;
            return Ok(Outcome::Saved {
                note: Box::new(note),
                text,
                copy_id: None,
            });
        }
        let disk_text = String::from_utf8_lossy(&disk).to_string();
        let stat = fsio::stat(&abs)?;
        let disk_prepared = Prepared {
            rel: note.path.clone(),
            state: FileState::Read {
                stat,
                bytes: disk.clone(),
                hash: disk_hash.clone(),
            },
        };
        match resolve {
            None => return Ok(conflict(cx, &note, disk_text, disk_hash)),
            Some(Resolve::Mine) => {
                // The disk version becomes an `external` revision first.
                reconcile::apply(cx, &disk_prepared, &ApplyOpts::default())?;
                note = reconcile::live_note(cx.conn, note_id)?;
            }
            Some(Resolve::Theirs) => {
                let disk_rev = Document::parse(&disk_text).fields(cx.tz()).rev.unwrap_or(0);
                let buffer_rev = note.rev.max(disk_rev) + 1;
                reconcile::insert_revision(
                    cx,
                    note_id,
                    buffer_rev,
                    "mode",
                    text,
                    &fsio::sha256_hex(text.as_bytes()),
                )?;
                let opts = ApplyOpts {
                    min_rev: Some(buffer_rev + 1),
                    ..ApplyOpts::default()
                };
                reconcile::apply(cx, &disk_prepared, &opts)?;
                let note = reconcile::live_note(cx.conn, note_id)?;
                return Ok(Outcome::Saved {
                    note: Box::new(note),
                    text: disk_text,
                    copy_id: None,
                });
            }
            Some(Resolve::Both) => {
                let copy = conflict_copy(cx, &note, text)?;
                link(
                    cx,
                    ("note", &note.id),
                    ("note", &copy.id),
                    "related",
                    None,
                    "system",
                )?;
                reconcile::apply(cx, &disk_prepared, &ApplyOpts::default())?;
                let note = reconcile::live_note(cx.conn, note_id)?;
                return Ok(Outcome::Saved {
                    note: Box::new(note),
                    text: disk_text,
                    copy_id: Some(copy.id),
                });
            }
        }
        return write_buffer(cx, &note, text, true, false);
    }
    write_buffer(cx, &note, text, record_revision, false)
}

/// Write an editor buffer as note `note`: Mode's keys regenerated (`id`
/// kept, `rev + 1`, `updated = now`), the body verbatim.
fn write_buffer(cx: &Cx, note: &Note, text: &str, record: bool, create: bool) -> Result<Outcome> {
    let doc = Document::parse(text);
    let new_text = if doc.error().is_some() {
        text.to_string()
    } else {
        let f = doc.fields(cx.tz());
        let c = Canonical {
            id: note.id.clone(),
            status: f.status.clone().unwrap_or_else(|| "todo".into()),
            write_status: true,
            urgent: f.urgent.unwrap_or(false),
            when: when_from(cx, f.when.as_ref(), f.when_raw.as_deref(), Some(note)),
            tags: f.tags.clone(),
            created: f.created.clone().unwrap_or_else(|| note.created_at.clone()),
            updated: cx.now().to_string(),
            rev: note.rev.max(f.rev.unwrap_or(0)) + 1,
        };
        let edited = edited_keys(&doc, note, cx, &[]);
        frontmatter::rewrite(&doc, &c, &WriteOpts::editing(&edited))
            .unwrap_or_else(|| text.to_string())
    };
    if create && note.deleted_at.is_some() {
        cx.conn.execute(
            "UPDATE notes SET deleted_at = NULL WHERE id = ?1",
            [&note.id],
        )?;
    }
    let mut prior = note.clone();
    prior.deleted_at = None;
    let rel = if create && vault::abs_path(&cx.env.root, &note.path).exists() {
        vault::unique_name(
            &cx.env.root,
            vault::parent_dir(&note.path),
            &vault::title_from_path(&note.path),
        )?
    } else {
        note.path.clone()
    };
    let origin = record.then_some("mode");
    let saved = commit(
        cx,
        &rel,
        &new_text,
        Some(&prior),
        create,
        origin,
        &note.source_kind,
    )?;
    Ok(Outcome::Saved {
        note: Box::new(saved),
        text: new_text,
        copy_id: None,
    })
}

/// "Keep both" (04 §11.4): the buffer as `<Title> (conflict YYYY-MM-DD HHMM).md`
/// next to the original, a new id, `rev = 1`, same status / urgent / when / tags.
fn conflict_copy(cx: &Cx, note: &Note, text: &str) -> Result<Note> {
    let local = cx
        .tz()
        .to_local(crate::db::parse_timestamp(cx.now()).unwrap_or(time::OffsetDateTime::UNIX_EPOCH));
    let stamp = format!(
        "{} {:02}{:02}",
        fields::fmt_date(local.date()),
        local.hour(),
        local.minute()
    );
    let text = eol::normalize(text, eol::NEW_FILE);
    let text = text.as_ref();
    let doc = Document::parse(text);
    let body_text = doc.body();
    let title = body::first_h1(body_text).unwrap_or_else(|| note.title.clone());
    let stem = vault::stem_for_title(&format!("{title} (conflict {stamp})"));
    let rel = vault::unique_name(&cx.env.root, vault::parent_dir(&note.path), &stem)?;
    let f = if doc.error().is_none() {
        doc.fields(cx.tz())
    } else {
        Default::default()
    };
    let c = Canonical {
        id: repo::new_id(),
        status: f.status.clone().unwrap_or_else(|| note.status.clone()),
        write_status: true,
        urgent: f.urgent.unwrap_or(note.urgent),
        when: when_from(cx, f.when.as_ref(), f.when_raw.as_deref(), Some(note))
            .or_else(|| note.when_date.clone()),
        tags: if f.tags.is_empty() {
            tags_of(cx, &note.id)?
        } else {
            f.tags.clone()
        },
        created: cx.now().to_string(),
        updated: cx.now().to_string(),
        rev: 1,
    };
    let all: Vec<&'static str> = frontmatter::MODE_KEYS.to_vec();
    let new_text = match doc.error() {
        None => frontmatter::rewrite(&doc, &c, &WriteOpts::editing(&all)).unwrap_or_default(),
        Some(_) => frontmatter::new_file(&c, text),
    };
    commit(
        cx,
        &rel,
        &new_text,
        None,
        true,
        Some("mode"),
        &note.source_kind,
    )
}

/// Tag display names of a note, in frontmatter order.
pub fn tags_of(cx: &Cx, note_id: &str) -> Result<Vec<String>> {
    let mut stmt = cx.conn.prepare(
        "SELECT t.name FROM note_tags nt JOIN tags t ON t.id = nt.tag_id WHERE nt.note_id = ?1 ORDER BY nt.position",
    )?;
    let rows = stmt.query_map([note_id], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

// --------------------------------------------------------------- creation

/// A new note's values: `todo`, not urgent, no `when`, no tags, `rev = 1`.
pub fn new_canonical(cx: &Cx) -> Canonical {
    Canonical {
        id: repo::new_id(),
        status: "todo".into(),
        write_status: true,
        urgent: false,
        when: None,
        tags: Vec::new(),
        created: cx.now().to_string(),
        updated: cx.now().to_string(),
        rev: 1,
    }
}

/// Create `<dir>/<title>.md` from values and a body (capture, new note,
/// templates): the file name follows the title (04 §2.4).
pub fn create_file(
    cx: &Cx,
    dir: &str,
    title: &str,
    c: &Canonical,
    body_text: &str,
    source_kind: &str,
) -> Result<Note> {
    std::fs::create_dir_all(vault::abs_path(&cx.env.root, dir))?;
    let rel = vault::unique_name(&cx.env.root, dir, &vault::stem_for_title(title))?;
    let text = frontmatter::new_file(c, body_text);
    commit(cx, &rel, &text, None, true, Some("mode"), source_kind)
}

/// "New note" (04 §7.1 path 7): called on the first saved keystroke with the
/// editor's text, in `folder_id` (`None` = the vault root).
pub fn create_note(cx: &Cx, folder_id: Option<&str>, text: &str) -> Result<Note> {
    let dir = match folder_id {
        None | Some("root") | Some("") => String::new(),
        Some(id) => reconcile::folder_path(cx.conn, id)?,
    };
    let text = eol::normalize(text, eol::NEW_FILE);
    let text = text.as_ref();
    let doc = Document::parse(text);
    let title = body::first_h1(doc.body()).unwrap_or_else(|| "Untitled".to_string());
    let mut c = new_canonical(cx);
    if doc.has_block() {
        let f = doc.fields(cx.tz());
        c.status = f.status.clone().unwrap_or_else(|| "todo".into());
        c.urgent = f.urgent.unwrap_or(false);
        c.when = when_from(cx, f.when.as_ref(), f.when_raw.as_deref(), None);
        c.tags = f.tags.clone();
        std::fs::create_dir_all(vault::abs_path(&cx.env.root, &dir))?;
        let rel = vault::unique_name(&cx.env.root, &dir, &vault::stem_for_title(&title))?;
        let all: Vec<&'static str> = frontmatter::MODE_KEYS.to_vec();
        let new_text = frontmatter::rewrite(&doc, &c, &WriteOpts::editing(&all))
            .unwrap_or_else(|| text.to_string());
        return commit(cx, &rel, &new_text, None, true, Some("mode"), "manual");
    }
    create_file(cx, &dir, &title, &c, text, "manual")
}

/// A `links` row (09 §5.5), stamped; an identical live link is reused.
pub fn link(
    cx: &Cx,
    from: (&str, &str),
    to: (&str, &str),
    link_type: &str,
    label: Option<&str>,
    created_by: &str,
) -> Result<Link> {
    if let Some(l) = repo::query::<Link>(
        cx.conn,
        "from_kind = ? AND from_id = ? AND to_kind = ? AND to_id = ? AND link_type = ? AND deleted_at IS NULL",
        &[&from.0, &from.1, &to.0, &to.1, &link_type],
    )?
    .into_iter()
    .next()
    {
        return Ok(l);
    }
    let mut l = Link {
        id: repo::new_id(),
        from_kind: from.0.into(),
        from_id: from.1.into(),
        to_kind: to.0.into(),
        to_id: to.1.into(),
        link_type: link_type.into(),
        label: label.map(str::to_string),
        created_by: created_by.into(),
        ..Default::default()
    };
    repo::create(cx.conn, cx.device(), &mut l)?;
    Ok(l)
}

// ------------------------------------------------------------ trash / undo

/// The bytes a trash took away, for the 8 s Undo (04 §12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trashed {
    pub note_id: String,
    pub rel: String,
    pub bytes: Vec<u8>,
}

/// Delete a note (04 §12): the file goes to the OS trash (never `unlink`),
/// the row is tombstoned and its last content kept as a revision.
pub fn trash(cx: &Cx, note_id: &str) -> Result<Trashed> {
    let note = reconcile::live_note(cx.conn, note_id)?;
    let abs = vault::abs_path(&cx.env.root, &note.path);
    let bytes = std::fs::read(&abs).unwrap_or_default();
    if abs.exists() {
        cx.env.trash.trash(&abs)?;
    }
    reconcile::tombstone(cx, note_id)?;
    let rev: i64 = cx
        .conn
        .query_row("SELECT rev FROM notes WHERE id = ?1", [note_id], |r| {
            r.get(0)
        })?;
    if let Ok(text) = std::str::from_utf8(&bytes)
        && bytes.len() as u64 <= vault::MAX_VERSIONED_BYTES
    {
        reconcile::insert_revision(cx, note_id, rev, "mode", text, &fsio::sha256_hex(&bytes))?;
    }
    Ok(Trashed {
        note_id: note_id.to_string(),
        rel: note.path,
        bytes,
    })
}

/// Undo a trash: the OS trash puts the file back where the platform can,
/// else the saved bytes (or the last revision) are written again; the row
/// is un-tombstoned (`rev + 1`, caches rebuilt, tags re-linked).
pub fn untrash(cx: &Cx, note_id: &str, saved: Option<&Trashed>) -> Result<Outcome> {
    let note = reconcile::by_id(cx.conn, note_id)?.ok_or_else(|| DbError::NotFound {
        table: "notes",
        key: note_id.to_string(),
    })?;
    if note.deleted_at.is_none() {
        return saved_from_disk(cx, note_id);
    }
    let mut rel = note.path.clone();
    let mut abs = vault::abs_path(&cx.env.root, &rel);
    let restored = !abs.exists() && cx.env.trash.restore(&abs).unwrap_or(false);
    if !restored {
        let bytes = match saved.filter(|s| s.note_id == note_id) {
            Some(s) => s.bytes.clone(),
            None => cx
                .conn
                .query_row(
                    "SELECT content FROM note_revisions WHERE note_id = ?1 ORDER BY rev DESC LIMIT 1",
                    [note_id],
                    |r| r.get::<_, String>(0),
                )
                .map(String::into_bytes)
                .map_err(|_| invalid("note", "nothing to restore"))?,
        };
        if abs.exists() {
            rel = vault::unique_name(
                &cx.env.root,
                vault::parent_dir(&rel),
                &vault::title_from_path(&rel),
            )?;
            abs = vault::abs_path(&cx.env.root, &rel);
        }
        if let Some(dir) = abs.parent() {
            std::fs::create_dir_all(dir)?;
        }
        fsio::atomic_create(&abs, &bytes)?;
    }
    if reconcile::live_by_path(cx.conn, &rel)?.is_some() {
        return Err(invalid("note", "another note holds that path"));
    }
    cx.conn.execute(
        "UPDATE notes SET deleted_at = NULL, path = ?1, content_hash = NULL, rev = rev + 1, updated_at = ?2, device_id = ?3
          WHERE id = ?4",
        params![rel, cx.now(), cx.device(), note_id],
    )?;
    let p = reconcile::prepare(&cx.env.root, &rel, None);
    reconcile::apply(cx, &p, &ApplyOpts::default())?;
    saved_from_disk(cx, note_id)
}

// ------------------------------------------------------------ history

/// Restore a revision (03 §5.8, 09 §5.1): its content is written as a new
/// revision (`origin = 'restore'`); `rev` moves forward, never back.
pub fn restore_revision(cx: &Cx, note_id: &str, rev: i64) -> Result<Outcome> {
    let (note, current, _) = fresh(cx, note_id)?;
    let target: Eol = eol::of_existing(&current);
    let content: String = cx
        .conn
        .query_row(
            "SELECT content FROM note_revisions WHERE note_id = ?1 AND rev = ?2",
            params![note_id, rev],
            |r| r.get(0),
        )
        .map_err(|_| DbError::NotFound {
            table: "note_revisions",
            key: format!("{note_id}@{rev}"),
        })?;
    let content = eol::normalize(&content, target).into_owned();
    let doc = Document::parse(&content);
    let new_text = if doc.error().is_some() {
        content.clone()
    } else {
        let f = doc.fields(cx.tz());
        let c = Canonical {
            id: note.id.clone(),
            status: f.status.clone().unwrap_or_else(|| "todo".into()),
            write_status: true,
            urgent: f.urgent.unwrap_or(false),
            when: f
                .when
                .as_ref()
                .map(|w| fields::fmt_date(w.resolve(cx.today()))),
            tags: f.tags.clone(),
            created: f.created.clone().unwrap_or_else(|| note.created_at.clone()),
            updated: cx.now().to_string(),
            rev: note.rev + 1,
        };
        let edited = edited_keys(&doc, &note, cx, &[]);
        frontmatter::rewrite(&doc, &c, &WriteOpts::editing(&edited)).unwrap_or(content.clone())
    };
    let saved = commit(
        cx,
        &note.path,
        &new_text,
        Some(&note),
        false,
        Some("restore"),
        &note.source_kind,
    )?;
    Ok(Outcome::Saved {
        note: Box::new(saved),
        text: new_text,
        copy_id: None,
    })
}

// ------------------------------------------------------------- tag batches

/// A batch rewrite's report (04 §5.4).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct BatchReport {
    pub written: usize,
    /// The file that stopped the batch and why; already-written files stay.
    pub stopped: Option<(String, String)>,
}

fn notes_with_tag(cx: &Cx, norm: &str) -> Result<Vec<String>> {
    let mut stmt = cx.conn.prepare(
        "SELECT n.id FROM notes n JOIN note_tags nt ON nt.note_id = n.id JOIN tags t ON t.id = nt.tag_id
          WHERE t.name_norm = ?1 AND n.deleted_at IS NULL ORDER BY n.path",
    )?;
    let rows = stmt.query_map([norm], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Rename a tag in every note that carries it; renaming onto an existing
/// tag merges (the survivor keeps its color, the loser is tombstoned).
pub fn tag_rename(cx: &Cx, from: &str, to: &str) -> Result<BatchReport> {
    let from_norm =
        fields::tag_norm(from).ok_or_else(|| invalid("tag", format!("{from:?} is not a tag")))?;
    let to_display = fields::tag_display(to)
        .ok_or_else(|| invalid("tag", format!("{to:?} is not a valid tag")))?;
    let to_norm = to_display.to_lowercase();
    let survivor: Option<String> = cx
        .conn
        .query_row(
            "SELECT id FROM tags WHERE name_norm = ?1 AND deleted_at IS NULL",
            [&to_norm],
            |r| r.get(0),
        )
        .ok();
    let loser: Option<String> = cx
        .conn
        .query_row(
            "SELECT id FROM tags WHERE name_norm = ?1 AND deleted_at IS NULL",
            [&from_norm],
            |r| r.get(0),
        )
        .ok();
    let merge = survivor.is_some() && from_norm != to_norm;
    if !merge && let Some(id) = &loser {
        // A plain rename keeps the row (and its color).
        cx.conn.execute(
            "UPDATE tags SET name = ?1, name_norm = ?2, rev = rev + 1, updated_at = ?3, device_id = ?4 WHERE id = ?5",
            params![to_display, to_norm, cx.now(), cx.device(), id],
        )?;
    }
    let mut report = BatchReport::default();
    // A merge rewrites the loser's notes; a rename already renamed the row.
    let ids = notes_with_tag(cx, if merge { &from_norm } else { &to_norm })?;
    for id in ids {
        let res = (|| -> Result<()> {
            let (_, text, _) = fresh(cx, &id)?;
            let doc = parse_writable(&text)?;
            let mut tags: Vec<String> = Vec::new();
            for t in doc.fields(cx.tz()).tags {
                let n = t.to_lowercase();
                let name = if n == from_norm {
                    to_display.clone()
                } else {
                    t
                };
                if !tags.iter().any(|x| x.to_lowercase() == name.to_lowercase()) {
                    tags.push(name);
                }
            }
            set_field(cx, &id, &FieldEdit::Tags(tags), None)?;
            Ok(())
        })();
        match res {
            Ok(()) => report.written += 1,
            Err(e) => {
                let path = reconcile::live_note(cx.conn, &id)
                    .map(|n| n.path)
                    .unwrap_or(id.clone());
                report.stopped = Some((path, e.to_string()));
                return Ok(report);
            }
        }
    }
    if merge && let Some(id) = loser {
        repo::soft_delete::<repo::Tag>(cx.conn, cx.device(), &id)?;
    }
    Ok(report)
}

/// Remove a tag from every note (same write path), then tombstone it.
pub fn tag_delete(cx: &Cx, name: &str) -> Result<BatchReport> {
    let norm =
        fields::tag_norm(name).ok_or_else(|| invalid("tag", format!("{name:?} is not a tag")))?;
    let mut report = BatchReport::default();
    for id in notes_with_tag(cx, &norm)? {
        let res = (|| -> Result<()> {
            let (_, text, _) = fresh(cx, &id)?;
            let doc = parse_writable(&text)?;
            let tags: Vec<String> = doc
                .fields(cx.tz())
                .tags
                .into_iter()
                .filter(|t| t.to_lowercase() != norm)
                .collect();
            set_field(cx, &id, &FieldEdit::Tags(tags), None)?;
            Ok(())
        })();
        match res {
            Ok(()) => report.written += 1,
            Err(e) => {
                report.stopped = Some((id.clone(), e.to_string()));
                return Ok(report);
            }
        }
    }
    cx.conn.execute(
        "UPDATE tags SET deleted_at = ?1, updated_at = ?1, rev = rev + 1, device_id = ?2 WHERE name_norm = ?3 AND deleted_at IS NULL",
        params![cx.now(), cx.device(), norm],
    )?;
    Ok(report)
}

/// "Move n notes to <status>" (04 §4.6): every note with status `from`
/// gets `to` through the normal field write (rev + 1, file rewritten).
pub fn move_status(cx: &Cx, from: &str, to: &str) -> Result<BatchReport> {
    let to = validate_status(to)?;
    let ids: Vec<String> = {
        let mut stmt = cx.conn.prepare(
            "SELECT id FROM notes WHERE status = ?1 AND deleted_at IS NULL ORDER BY path",
        )?;
        stmt.query_map([from], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut report = BatchReport::default();
    for id in ids {
        match set_field(cx, &id, &FieldEdit::Status(to.clone()), None) {
            Ok(_) => report.written += 1,
            Err(e) => {
                report.stopped = Some((id, e.to_string()));
                return Ok(report);
            }
        }
    }
    Ok(report)
}
