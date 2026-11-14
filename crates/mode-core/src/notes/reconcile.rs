//! Reconcile (09 §8.2–§8.3, 04 §11): stat first, hash only when mtime or
//! size moved, parse only when the hash moved; identity by the frontmatter
//! `id` (missing → assigned, known elsewhere → move or duplicate, tombstoned
//! → restored); the revision rules; caches, tags and `note_revisions`;
//! folders and `.mode/folders.json`; the full scan with the S4 tombstone
//! pass; and the touch-up queue that writes Mode's keys back into files.
//!
//! [`prepare`] does the file I/O that needs no database (stat, read, hash)
//! so a scan reads files outside the writer; [`apply`] runs on the writer
//! connection and re-checks what it was handed, so a prepared snapshot that
//! went stale (Mode wrote the file meanwhile) is recognized and skipped.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::body;
use super::ctx::{Cx, invalid};
use super::fields;
use super::frontmatter::{self, Canonical, Document, Fields, WriteOpts};
use super::fsio::{self, Stat};
use super::vault;
use crate::db::{DbError, Result};
use crate::repo::{self, Folder, Note};

// ---------------------------------------------------------------- lookups

/// The reconcile bookkeeping of a live row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowStat {
    pub id: String,
    pub path: String,
    pub file_mtime: Option<String>,
    pub file_size: Option<i64>,
    pub content_hash: Option<String>,
}

/// Live rows by path, for [`prepare`].
pub fn row_stats(conn: &Connection) -> Result<HashMap<String, RowStat>> {
    let mut stmt = conn.prepare(
        "SELECT id, path, file_mtime, file_size, content_hash FROM notes WHERE deleted_at IS NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(RowStat {
            id: r.get(0)?,
            path: r.get(1)?,
            file_mtime: r.get(2)?,
            file_size: r.get(3)?,
            content_hash: r.get(4)?,
        })
    })?;
    let mut out = HashMap::new();
    for r in rows {
        let r = r?;
        out.insert(r.path.clone(), r);
    }
    Ok(out)
}

pub fn live_by_path(conn: &Connection, rel: &str) -> Result<Option<Note>> {
    Ok(
        repo::query::<Note>(conn, "path = ? AND deleted_at IS NULL", &[&rel])?
            .into_iter()
            .next(),
    )
}

pub fn by_id(conn: &Connection, id: &str) -> Result<Option<Note>> {
    repo::get_by_id::<Note>(conn, id)
}

pub fn live_note(conn: &Connection, id: &str) -> Result<Note> {
    repo::get_live::<Note>(conn, id)?.ok_or_else(|| DbError::NotFound {
        table: "notes",
        key: id.to_string(),
    })
}

// ---------------------------------------------------------------- prepare

/// A file as [`prepare`] found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    Missing,
    /// A directory, a symlink or something else that is not a note file.
    NotAFile,
    /// Stat equal to the row snapshot: nothing to read (09 §8.2 "Skip").
    Same(Stat),
    Read {
        stat: Stat,
        bytes: Vec<u8>,
        hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub rel: String,
    pub state: FileState,
}

fn stat_matches(stat: &Stat, row: &RowStat) -> bool {
    row.file_mtime.as_deref() == Some(stat.mtime.as_str()) && row.file_size == Some(stat.size)
}

/// Stat, and read + hash only when the stat moved (no database).
pub fn prepare(root: &Path, rel: &str, snapshot: Option<&RowStat>) -> Prepared {
    let abs = vault::abs_path(root, rel);
    let state = match std::fs::symlink_metadata(&abs) {
        Err(_) => FileState::Missing,
        Ok(m) if !m.is_file() || !vault::is_note_path(rel) => FileState::NotAFile,
        Ok(_) => match fsio::stat(&abs) {
            Err(_) => FileState::Missing,
            Ok(stat) if snapshot.is_some_and(|s| stat_matches(&stat, s)) => FileState::Same(stat),
            Ok(stat) => match std::fs::read(&abs) {
                Ok(bytes) => {
                    let hash = fsio::sha256_hex(&bytes);
                    FileState::Read { stat, bytes, hash }
                }
                Err(_) => FileState::Missing,
            },
        },
    };
    Prepared {
        rel: rel.to_string(),
        state,
    }
}

// ------------------------------------------------------------------ apply

/// What happened to one path.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Skipped,
    /// Same content, new mtime / size only.
    StatOnly,
    Created,
    Updated,
    Moved,
    Restored,
    /// A copy whose id belonged to another file: it got a new id.
    Duplicate,
    Tombstoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub rel: String,
    pub note_id: Option<String>,
    pub change: Change,
    /// Diagnostics → Notes entries for this path (rebuilt per reconcile).
    pub diagnostics: Vec<String>,
    /// Paths to reconcile next (the other copy of a duplicate).
    pub also: Vec<String>,
}

impl Outcome {
    fn new(rel: &str, change: Change, note_id: Option<String>) -> Outcome {
        Outcome {
            rel: rel.to_string(),
            note_id,
            change,
            diagnostics: Vec::new(),
            also: Vec::new(),
        }
    }

    pub fn changed(&self) -> bool {
        !matches!(self.change, Change::Skipped | Change::StatOnly)
    }
}

/// Knobs for [`apply`].
#[derive(Debug, Clone)]
pub struct ApplyOpts {
    /// `note_revisions.origin` for content Mode did not write.
    pub origin: &'static str,
    /// `source_kind` of a row this creates.
    pub source_kind: &'static str,
    /// A floor for the resulting `rev` (conflict "Keep theirs").
    pub min_rev: Option<i64>,
}

impl Default for ApplyOpts {
    fn default() -> Self {
        ApplyOpts {
            origin: "external",
            source_kind: "manual",
            min_rev: None,
        }
    }
}

/// How the file relates to the rows (09 §8.2 G → L).
#[derive(Debug, Clone, PartialEq)]
enum Ident {
    /// No row, or a new id for a duplicate (`reassigned`).
    New {
        id: String,
        reassigned: bool,
    },
    Same(Box<Note>),
    Moved(Box<Note>),
    /// The newer of two files with one id takes the row from the older
    /// (04 §3.3): revision rules as for the same note, no move bump.
    TakeOver(Box<Note>),
    Restored(Box<Note>),
}

/// Reconcile one prepared path on the writer.
pub fn apply(cx: &Cx, p: &Prepared, opts: &ApplyOpts) -> Result<Outcome> {
    let conn = cx.conn;
    let rel = p.rel.as_str();
    let row_at = live_by_path(conn, rel)?;
    let (stat, bytes, hash) = match &p.state {
        FileState::Missing | FileState::NotAFile => {
            return Ok(match row_at {
                Some(r) => {
                    tombstone(cx, &r.id)?;
                    Outcome::new(rel, Change::Tombstoned, Some(r.id))
                }
                None => Outcome::new(rel, Change::Skipped, None),
            });
        }
        FileState::Same(stat) => {
            // Re-check against the row as it is now.
            let snap = row_at.as_ref().map(row_stat);
            if snap.as_ref().is_some_and(|s| stat_matches(stat, s)) {
                return Ok(Outcome::new(rel, Change::Skipped, row_at.map(|r| r.id)));
            }
            let fresh = prepare(&cx.env.root, rel, snap.as_ref());
            if matches!(fresh.state, FileState::Same(_)) {
                return Ok(Outcome::new(rel, Change::Skipped, row_at.map(|r| r.id)));
            }
            return apply(cx, &fresh, opts);
        }
        FileState::Read { stat, bytes, hash } => (stat, bytes, hash),
    };

    // 09 §8.2 C → F: same content, only the stat moved.
    if let Some(r) = &row_at
        && r.content_hash.as_deref() == Some(hash.as_str())
    {
        if r.file_mtime.as_deref() == Some(stat.mtime.as_str()) && r.file_size == Some(stat.size) {
            return Ok(Outcome::new(rel, Change::Skipped, Some(r.id.clone())));
        }
        conn.execute(
            "UPDATE notes SET file_mtime = ?1, file_size = ?2 WHERE id = ?3",
            params![stat.mtime, stat.size, r.id],
        )?;
        return Ok(Outcome::new(rel, Change::StatOnly, Some(r.id.clone())));
    }

    let file = FileView::new(cx, rel, bytes, stat);
    let mut also = Vec::new();

    // Identity (09 §8.2 G → L).
    let ident = match file.fields.id.clone() {
        None => match &row_at {
            Some(r) => Ident::Same(Box::new(r.clone())),
            None => Ident::New {
                id: repo::new_id(),
                reassigned: false,
            },
        },
        Some(x) => match by_id(conn, &x)? {
            Some(k) if k.deleted_at.is_some() => Ident::Restored(Box::new(k)),
            Some(k) if k.path == rel => Ident::Same(Box::new(k)),
            Some(k) => {
                let other = vault::abs_path(&cx.env.root, &k.path);
                match read_other(cx, &other) {
                    Some((other_fields, other_stat))
                        if other_fields.id.as_deref() == Some(x.as_str()) =>
                    {
                        // Duplicate (04 §3.3): the newer file keeps the id;
                        // newer = greater frontmatter `updated`, then mtime;
                        // a tie keeps the id where the row is.
                        let this_key = (file.fields.updated.clone(), stat.mtime.clone());
                        let other_key = (other_fields.updated.clone(), other_stat.mtime.clone());
                        if this_key > other_key {
                            also.push(k.path.clone());
                            Ident::TakeOver(Box::new(k))
                        } else {
                            Ident::New {
                                id: repo::new_id(),
                                reassigned: true,
                            }
                        }
                    }
                    _ => Ident::Moved(Box::new(k)),
                }
            }
            None => Ident::New {
                id: x,
                reassigned: false,
            },
        },
    };

    let (id, prior, change) = match &ident {
        Ident::New { id, reassigned } => (
            id.clone(),
            None,
            if *reassigned {
                Change::Duplicate
            } else {
                Change::Created
            },
        ),
        Ident::Same(n) | Ident::TakeOver(n) => {
            (n.id.clone(), Some(n.as_ref().clone()), Change::Updated)
        }
        Ident::Moved(n) => (n.id.clone(), Some(n.as_ref().clone()), Change::Moved),
        Ident::Restored(n) => (n.id.clone(), Some(n.as_ref().clone()), Change::Restored),
    };

    // The path now belongs to `id`: a different live row there is gone.
    if let Some(r) = &row_at
        && r.id != id
    {
        tombstone(cx, &r.id)?;
    }

    // Revision rules (09 §8.3, 04 §11.2 / §11.3).
    let f_rev = file.fields.rev;
    let latest_file_time = max_opt(file.fields.updated.clone(), Some(stat.mtime.clone()));
    let (mut rev, mut updated_at) = match &prior {
        None => (
            f_rev.unwrap_or(1),
            file.fields
                .updated
                .clone()
                .unwrap_or_else(|| stat.mtime.clone()),
        ),
        Some(p) => {
            let r = p.rev;
            let content_changed = p.content_hash.as_deref() != Some(hash.as_str());
            match f_rev {
                Some(f) if f > r => (
                    f,
                    file.fields
                        .updated
                        .clone()
                        .unwrap_or_else(|| stat.mtime.clone()),
                ),
                _ if !content_changed => (r, p.updated_at.clone()),
                Some(f) if f < r && p.frontmatter_dirty => (
                    r,
                    max_opt(latest_file_time.clone(), Some(p.updated_at.clone()))
                        .unwrap_or_default(),
                ),
                _ => (
                    r + 1,
                    latest_file_time
                        .clone()
                        .unwrap_or_else(|| stat.mtime.clone()),
                ),
            }
        }
    };
    match &ident {
        Ident::Moved(p) => {
            rev = rev.max(p.rev + 1);
            updated_at = cx.now().to_string();
        }
        Ident::Restored(p) => rev = rev.max(p.rev + 1),
        _ => {}
    }
    if let Some(min) = opts.min_rev {
        rev = rev.max(min);
    }
    let mut created_at = file
        .fields
        .created
        .clone()
        .or_else(|| prior.as_ref().map(|p| p.created_at.clone()))
        .unwrap_or_else(|| birth_or_mtime(stat));
    let mut diagnostics = file.diagnostics.clone();
    if created_at > updated_at {
        diagnostics.push("created is later than updated; updated set to created".into());
        updated_at = created_at.clone();
    }
    if created_at.is_empty() {
        created_at = cx.now().to_string();
    }

    let mut note = prior.clone().unwrap_or_else(|| Note {
        seq: None,
        id: id.clone(),
        pinned: false,
        archived_at: None,
        source_kind: opts.source_kind.to_string(),
        ..Default::default()
    });
    note.id = id.clone();
    note.path = rel.to_string();
    note.folder_id = ensure_folder(cx, vault::parent_dir(rel))?;
    note.created_at = created_at;
    note.updated_at = updated_at;
    note.rev = rev;
    note.device_id = cx.device().to_string();
    note.deleted_at = None;
    file.fill(cx, &mut note, prior.as_ref());
    note.content_hash = Some(hash.clone());
    note.file_mtime = Some(stat.mtime.clone());
    note.file_size = Some(stat.size);
    note.indexed_at = Some(cx.now().to_string());
    // A touch-up is due when the file does not say what the row holds.
    note.frontmatter_dirty = file.writable
        && (file.fields.id.as_deref() != Some(note.id.as_str())
            || file.fields.rev != Some(note.rev)
            || file.fields.created.as_deref() != Some(note.created_at.as_str())
            || file.fields.updated.as_deref() != Some(note.updated_at.as_str()));

    store(cx, &mut note, prior.is_some(), &file.tags)?;
    let content_changed = prior
        .as_ref()
        .is_none_or(|p| p.content_hash.as_deref() != Some(hash.as_str()));
    if content_changed && let Some(text) = file.versionable() {
        insert_revision(cx, &note.id, note.rev, opts.origin, text, hash)?;
    }
    let mut out = Outcome::new(rel, change, Some(note.id.clone()));
    out.diagnostics = diagnostics;
    out.also = also;
    Ok(out)
}

/// A file's creation time when its frontmatter has none (04 §3.1): the
/// birth time, but never later than the mtime (a copied vault keeps old
/// mtimes and gets new birth times).
pub fn birth_or_mtime(stat: &Stat) -> String {
    match &stat.birth {
        Some(b) if *b < stat.mtime => b.clone(),
        _ => stat.mtime.clone(),
    }
}

fn row_stat(n: &Note) -> RowStat {
    RowStat {
        id: n.id.clone(),
        path: n.path.clone(),
        file_mtime: n.file_mtime.clone(),
        file_size: n.file_size,
        content_hash: n.content_hash.clone(),
    }
}

fn max_opt(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a >= b { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn read_other(cx: &Cx, abs: &Path) -> Option<(Fields, Stat)> {
    let stat = fsio::stat(abs).ok()?;
    let text = std::fs::read_to_string(abs).ok()?;
    let doc = Document::parse(&text);
    Some((doc.fields(cx.tz()), stat))
}

/// A file's content as the indexer sees it.
pub struct FileView<'a> {
    rel: &'a str,
    bytes: &'a [u8],
    stat: &'a Stat,
    pub text: Option<String>,
    pub doc: Option<Document>,
    pub fields: Fields,
    pub tags: Vec<String>,
    /// Mode may write this file (UTF-8, ≤ 10 MB, readable frontmatter).
    pub writable: bool,
    pub diagnostics: Vec<String>,
}

impl<'a> FileView<'a> {
    pub fn new(cx: &Cx, rel: &'a str, bytes: &'a [u8], stat: &'a Stat) -> FileView<'a> {
        let mut diagnostics = Vec::new();
        let too_large = bytes.len() as u64 > vault::MAX_INDEX_BYTES;
        let text = if too_large {
            diagnostics.push("larger than 10 MB: indexed by title only".to_string());
            None
        } else {
            match std::str::from_utf8(bytes) {
                Ok(t) => Some(t.to_string()),
                Err(_) => {
                    diagnostics
                        .push("not valid UTF-8: indexed by title only, read-only".to_string());
                    None
                }
            }
        };
        let doc = text.as_deref().map(Document::parse);
        let fields = match &doc {
            Some(d) if d.error().is_none() => d.fields(cx.tz()),
            _ => Fields::default(),
        };
        if let Some(reason) = doc.as_ref().and_then(|d| d.error()) {
            diagnostics.push(format!("Frontmatter could not be read: {reason}"));
        }
        for (k, why) in &fields.invalid {
            diagnostics.push(format!("{k}: {why}"));
        }
        let writable = doc.as_ref().is_some_and(|d| d.error().is_none());
        let tags = fields.tags.clone();
        FileView {
            rel,
            bytes,
            stat,
            text,
            doc,
            fields,
            tags,
            writable,
            diagnostics,
        }
    }

    /// The text to keep as a revision (UTF-8, ≤ 1 MB; 09 §10).
    pub fn versionable(&self) -> Option<&str> {
        (self.bytes.len() as u64 <= vault::MAX_VERSIONED_BYTES)
            .then_some(self.text.as_deref())
            .flatten()
    }

    /// Fill the file-derived columns: fields (file wins) and caches.
    pub fn fill(&self, cx: &Cx, note: &mut Note, prior: Option<&Note>) {
        let f = &self.fields;
        note.status = f.status.clone().unwrap_or_else(|| "todo".to_string());
        note.urgent = f.urgent.unwrap_or(false);
        // `when` (04 §5.3): a relative word resolves once and is frozen.
        note.when_raw = f.when_raw.clone();
        note.when_date = match &f.when {
            None => None,
            Some(w) => {
                let frozen = prior
                    .filter(|p| p.when_raw == f.when_raw && p.when_date.is_some())
                    .and_then(|p| p.when_date.clone());
                frozen.or_else(|| {
                    let anchor = f
                        .updated
                        .as_deref()
                        .and_then(crate::db::parse_timestamp)
                        .or_else(|| crate::db::parse_timestamp(&self.stat.mtime))
                        .map(|t| cx.tz().local_date(t))
                        .unwrap_or_else(|| cx.today());
                    Some(fields::fmt_date(w.resolve(anchor)))
                })
            }
        };
        let body_text = match &self.doc {
            Some(d) => d.body().to_string(),
            None => String::new(),
        };
        note.title = body::first_h1(&body_text).unwrap_or_else(|| vault::title_from_path(self.rel));
        let (total, done) = body::counts(&body_text);
        note.task_total = total;
        note.task_done = done;
        note.excerpt = body::excerpt(&body_text);
        note.word_count = body::word_count(&body_text);
        note.body = body_text;
    }
}

/// Insert or replace the row, then its tags (`note_tags`, `tags_text`).
pub fn store(cx: &Cx, note: &mut Note, exists: bool, tags: &[String]) -> Result<()> {
    note.tags_text = tags.join(" ");
    if exists {
        repo::replace(cx.conn, note)?;
    } else {
        note.seq = None;
        let seq = repo::insert(cx.conn, note)?;
        note.seq = Some(seq);
    }
    sync_tags(cx, &note.id, tags)
}

/// Upsert `tags` by `name_norm` (first spelling wins) and rewrite `note_tags`
/// in frontmatter order (09 §5.1).
pub fn sync_tags(cx: &Cx, note_id: &str, tags: &[String]) -> Result<()> {
    let conn = cx.conn;
    conn.execute("DELETE FROM note_tags WHERE note_id = ?1", [note_id])?;
    for (i, name) in tags.iter().enumerate() {
        let Some(norm) = fields::tag_norm(name) else {
            continue;
        };
        let tag_id: Option<String> = conn
            .query_row(
                "SELECT id FROM tags WHERE name_norm = ?1 AND deleted_at IS NULL",
                [&norm],
                |r| r.get(0),
            )
            .optional()?;
        let tag_id = match tag_id {
            Some(id) => id,
            None => {
                let mut t = repo::Tag {
                    id: repo::new_id(),
                    name: name.clone(),
                    name_norm: norm.clone(),
                    ..Default::default()
                };
                repo::create(conn, cx.device(), &mut t)?;
                t.id
            }
        };
        conn.execute(
            "INSERT OR IGNORE INTO note_tags (note_id, tag_id, position) VALUES (?1, ?2, ?3)",
            params![note_id, tag_id, i as i64],
        )?;
    }
    Ok(())
}

/// A `note_revisions` row; a rev already recorded is kept (unique per rev).
pub fn insert_revision(
    cx: &Cx,
    note_id: &str,
    rev: i64,
    origin: &str,
    content: &str,
    hash: &str,
) -> Result<bool> {
    let n = cx.conn.execute(
        "INSERT OR IGNORE INTO note_revisions (id, note_id, rev, origin, content, content_hash, device_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![repo::new_id(), note_id, rev, origin, content, hash, cx.device(), cx.now()],
    )?;
    Ok(n > 0)
}

/// Tombstone a note (09 S4 for one row): `deleted_at`, caches cleared,
/// `rev + 1`. `note_tags` stay until the purge (04 §12).
pub fn tombstone(cx: &Cx, id: &str) -> Result<bool> {
    let n = cx.conn.execute(
        "UPDATE notes SET deleted_at = ?1, body = '', excerpt = '', tags_text = '',
                          rev = rev + 1, device_id = ?2, updated_at = ?1
          WHERE id = ?3 AND deleted_at IS NULL",
        params![cx.now(), cx.device(), id],
    )?;
    Ok(n > 0)
}

// ---------------------------------------------------------------- folders

/// The live folder row at a vault-relative directory, created (with its
/// ancestors and `folders.json` entries) when missing. `""` = the root (`None`).
pub fn ensure_folder(cx: &Cx, dir: &str) -> Result<Option<String>> {
    if dir.is_empty() {
        return Ok(None);
    }
    if let Some(id) = folder_id_at(cx.conn, dir)? {
        return Ok(Some(id));
    }
    let parent = ensure_folder(cx, vault::parent_dir(dir))?;
    let mut json = vault::read_folders_json(&cx.env.root);
    // An id persisted for this path (another device, a rebuilt index) wins.
    let known = json
        .iter()
        .find(|(_, p)| p.as_str() == dir)
        .map(|(id, _)| id.clone());
    let id = known.unwrap_or_else(repo::new_id);
    upsert_folder(cx, &id, dir, parent)?;
    if json.get(&id).map(String::as_str) != Some(dir) {
        json.insert(id.clone(), dir.to_string());
        vault::write_folders_json(&cx.env.root, &json)?;
    }
    Ok(Some(id))
}

pub fn folder_id_at(conn: &Connection, dir: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT id FROM folders WHERE path = ?1 AND deleted_at IS NULL",
            [dir],
            |r| r.get(0),
        )
        .optional()?)
}

/// Create, restore or re-path folder row `id`.
fn upsert_folder(cx: &Cx, id: &str, dir: &str, parent: Option<String>) -> Result<()> {
    let name = vault::file_name(dir).to_string();
    match repo::get_by_id::<Folder>(cx.conn, id)? {
        Some(mut f) => {
            let was_deleted = f.deleted_at.is_some();
            if f.path != dir || f.parent_id != parent || was_deleted || f.name != name {
                let old_path = f.path.clone();
                f.path = dir.to_string();
                f.name = name;
                f.parent_id = parent;
                f.deleted_at = None;
                repo::save(cx.conn, cx.device(), &mut f)?;
                if !was_deleted && old_path != dir {
                    repath_children(cx, &old_path, dir)?;
                }
            }
        }
        None => {
            let mut f = Folder {
                id: id.to_string(),
                parent_id: parent,
                path: dir.to_string(),
                name,
                kind: "normal".into(),
                ..Default::default()
            };
            repo::create(cx.conn, cx.device(), &mut f)?;
        }
    }
    Ok(())
}

/// A directory moved (04 §11.3): every child folder and note path follows
/// in one transaction; no file is rewritten (the notes' `rev` moves so the
/// new paths reach `change_log`, without a touch-up).
fn repath_children(cx: &Cx, old: &str, new: &str) -> Result<()> {
    let prefix = format!("{old}/");
    let n = prefix.chars().count() as i64;
    for table in ["folders", "notes"] {
        cx.conn.execute(
            &format!(
                "UPDATE {table} SET path = ?1 || substr(path, ?2), updated_at = ?3, rev = rev + 1, device_id = ?4
                  WHERE deleted_at IS NULL AND substr(path, 1, ?2) = ?5"
            ),
            params![new, n, cx.now(), cx.device(), prefix],
        )?;
    }
    Ok(())
}

/// Hand everything that points at folder `from` to folder `to`, then
/// tombstone `from` (two rows claimed one directory).
fn merge_folder(cx: &Cx, from: &str, to: &str) -> Result<()> {
    cx.conn.execute(
        "UPDATE notes SET folder_id = ?1 WHERE folder_id = ?2",
        params![to, from],
    )?;
    cx.conn.execute(
        "UPDATE folders SET parent_id = ?1 WHERE parent_id = ?2",
        params![to, from],
    )?;
    repo::soft_delete::<Folder>(cx.conn, cx.device(), from)?;
    Ok(())
}

/// What a folder pass changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderReport {
    pub created: usize,
    pub moved: usize,
    pub tombstoned: usize,
}

/// Bring `folders` and `.mode/folders.json` in line with the directories on
/// disk (04 §2.3, §11.3): an entry whose path is gone tombstones its row; a
/// directory without an entry gets one (reusing the live row's id when
/// there is one); a row whose id maps to a new path in `folders.json` (a
/// move made by Mode or another device) is re-pathed with its children.
pub fn sync_folders(cx: &Cx, dirs: &[String]) -> Result<FolderReport> {
    let mut report = FolderReport::default();
    let on_disk: HashSet<&str> = dirs.iter().map(String::as_str).collect();
    let mut json = vault::read_folders_json(&cx.env.root);
    let before = json.clone();
    json.retain(|_, p| on_disk.contains(p.as_str()));
    // Rows by id.
    let rows: Vec<Folder> = repo::query::<Folder>(cx.conn, "1", &[])?;
    let by_id: HashMap<&str, &Folder> = rows.iter().map(|f| (f.id.as_str(), f)).collect();
    // Entries first: ids that moved (another device, or Mode).
    let mut entries: Vec<(String, String)> =
        json.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    entries.sort_by(|a, b| a.1.cmp(&b.1));
    for (id, path) in &entries {
        let parent = parent_folder_id(cx, path)?;
        match by_id.get(id.as_str()) {
            Some(f) if f.path == *path && f.deleted_at.is_none() && f.parent_id == parent => {}
            Some(f) => {
                if f.deleted_at.is_none() && f.path != *path {
                    report.moved += 1;
                }
                // Free the path from another live row first.
                let other = folder_id_at(cx.conn, path)?.filter(|o| o != id);
                upsert_folder_free(cx, id, path, parent, other)?;
            }
            None => {
                let other = folder_id_at(cx.conn, path)?.filter(|o| o != id);
                upsert_folder_free(cx, id, path, parent, other)?;
                report.created += 1;
            }
        }
    }
    // Directories without an entry.
    for dir in dirs {
        if json.values().any(|p| p == dir) {
            continue;
        }
        let id = match folder_id_at(cx.conn, dir)? {
            Some(id) => id,
            None => {
                let parent = parent_folder_id(cx, dir)?;
                let id = repo::new_id();
                upsert_folder(cx, &id, dir, parent)?;
                report.created += 1;
                id
            }
        };
        json.insert(id, dir.clone());
    }
    // Live rows whose directory is gone.
    let live: Vec<Folder> = repo::query::<Folder>(cx.conn, "deleted_at IS NULL", &[])?;
    for f in live {
        if !on_disk.contains(f.path.as_str()) {
            repo::soft_delete::<Folder>(cx.conn, cx.device(), &f.id)?;
            report.tombstoned += 1;
        }
    }
    if json != before {
        vault::write_folders_json(&cx.env.root, &json)?;
    }
    Ok(report)
}

/// Upsert folder `id` at `path`, merging a live row `other` that held it.
fn upsert_folder_free(
    cx: &Cx,
    id: &str,
    path: &str,
    parent: Option<String>,
    other: Option<String>,
) -> Result<()> {
    if let Some(other) = &other {
        // Move the other row aside so the unique path index allows `id`.
        cx.conn.execute(
            "UPDATE folders SET path = path || '#' || id WHERE id = ?1",
            [other],
        )?;
    }
    upsert_folder(cx, id, path, parent)?;
    if let Some(other) = other {
        merge_folder(cx, &other, id)?;
        cx.conn.execute(
            "UPDATE folders SET path = ?1 WHERE id = ?2",
            params![path, other],
        )?;
    }
    Ok(())
}

fn parent_folder_id(cx: &Cx, dir: &str) -> Result<Option<String>> {
    let parent = vault::parent_dir(dir);
    if parent.is_empty() {
        return Ok(None);
    }
    folder_id_at(cx.conn, parent)
}

/// The capture folder (04 §2.2): the live `kind = 'capture'` row, else a
/// tombstoned one restored, else `Unsorted/`; the directory is recreated
/// when it disappeared. Returns `(id, path)`.
pub fn capture_folder(cx: &Cx) -> Result<(String, String)> {
    let live: Option<Folder> = repo::query::<Folder>(
        cx.conn,
        "kind = 'capture' AND deleted_at IS NULL ORDER BY updated_at DESC LIMIT 1",
        &[],
    )?
    .into_iter()
    .next();
    let folder = match live {
        Some(f) => f,
        None => {
            let gone: Option<Folder> = repo::query::<Folder>(
                cx.conn,
                "kind = 'capture' ORDER BY updated_at DESC LIMIT 1",
                &[],
            )?
            .into_iter()
            .next();
            let path = gone
                .as_ref()
                .map(|f| f.path.clone())
                .unwrap_or_else(|| vault::CAPTURE_FOLDER.to_string());
            std::fs::create_dir_all(vault::abs_path(&cx.env.root, &path))?;
            let id = match (gone, folder_id_at(cx.conn, &path)?) {
                (_, Some(existing)) => existing,
                (Some(g), None) => {
                    let parent = ensure_folder(cx, vault::parent_dir(&path))?;
                    upsert_folder(cx, &g.id, &path, parent)?;
                    let mut json = vault::read_folders_json(&cx.env.root);
                    json.insert(g.id.clone(), path.clone());
                    vault::write_folders_json(&cx.env.root, &json)?;
                    g.id
                }
                (None, None) => ensure_folder(cx, &path)?.unwrap_or_default(),
            };
            cx.conn.execute(
                "UPDATE folders SET kind = 'capture' WHERE id = ?1 AND kind <> 'capture'",
                [&id],
            )?;
            // At most one capture folder.
            cx.conn.execute(
                "UPDATE folders SET kind = 'normal' WHERE kind = 'capture' AND id <> ?1",
                [&id],
            )?;
            repo::must_get::<Folder>(cx.conn, &id)?
        }
    };
    let abs = vault::abs_path(&cx.env.root, &folder.path);
    if !abs.is_dir() {
        std::fs::create_dir_all(&abs)?;
    }
    Ok((folder.id, folder.path))
}

pub fn folder_path(conn: &Connection, id: &str) -> Result<String> {
    Ok(repo::get_live::<Folder>(conn, id)?
        .ok_or_else(|| DbError::NotFound {
            table: "folders",
            key: id.to_string(),
        })?
        .path)
}

// ------------------------------------------------------------------- scan

/// What a full scan did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub files: usize,
    pub changed: Vec<String>,
    pub tombstoned: usize,
    pub folders: FolderReport,
    /// Diagnostics → Notes, rebuilt per scan: `(path, message)`.
    pub diagnostics: Vec<(String, String)>,
}

impl ScanReport {
    pub fn any_change(&self) -> bool {
        !self.changed.is_empty() || self.tombstoned > 0 || self.folders != FolderReport::default()
    }
}

/// A full scan on the writer: folders, every prepared file, the duplicate
/// follow-ups, then S4 for rows whose file is gone (09 §8.2, §13 S4).
pub fn scan(cx: &Cx, walk: &vault::Walk, prepared: &[Prepared]) -> Result<ScanReport> {
    let mut report = ScanReport {
        files: walk.files.len(),
        folders: sync_folders(cx, &walk.dirs)?,
        diagnostics: walk.diagnostics.clone(),
        ..Default::default()
    };
    let mut queue: Vec<String> = Vec::new();
    for p in prepared {
        let out = apply(cx, p, &ApplyOpts::default())?;
        absorb(&mut report, out, &mut queue);
    }
    follow_up(cx, &mut report, queue)?;
    let paths: Vec<String> = walk.files.iter().map(|f| f.rel.clone()).collect();
    report.tombstoned +=
        crate::queries::tombstone_missing_notes(cx.conn, cx.device(), &cx.clock, &paths)?;
    Ok(report)
}

fn absorb(report: &mut ScanReport, out: Outcome, queue: &mut Vec<String>) {
    for d in &out.diagnostics {
        report.diagnostics.push((out.rel.clone(), d.clone()));
    }
    if out.changed()
        && let Some(id) = &out.note_id
    {
        report.changed.push(id.clone());
    }
    if out.change == Change::Tombstoned {
        report.tombstoned += 1;
    }
    queue.extend(out.also);
}

fn follow_up(cx: &Cx, report: &mut ScanReport, mut queue: Vec<String>) -> Result<()> {
    let mut rounds = 0;
    while !queue.is_empty() && rounds < 4 {
        rounds += 1;
        let batch = std::mem::take(&mut queue);
        for rel in batch {
            let snap = live_by_path(cx.conn, &rel)?.map(|n| row_stat(&n));
            let p = prepare(&cx.env.root, &rel, snap.as_ref());
            let out = apply(cx, &p, &ApplyOpts::default())?;
            absorb(report, out, &mut queue);
        }
    }
    Ok(())
}

/// Reconcile watcher paths (debounced): existing files first so a rename
/// reads as a move, then the vanished ones (09 §8.2).
pub fn reconcile_paths(cx: &Cx, prepared: &[Prepared]) -> Result<ScanReport> {
    let mut report = ScanReport::default();
    let mut queue = Vec::new();
    let (present, gone): (Vec<&Prepared>, Vec<&Prepared>) = prepared
        .iter()
        .partition(|p| !matches!(p.state, FileState::Missing | FileState::NotAFile));
    for p in present.into_iter().chain(gone) {
        let out = apply(cx, p, &ApplyOpts::default())?;
        absorb(&mut report, out, &mut queue);
    }
    follow_up(cx, &mut report, queue)?;
    Ok(report)
}

// --------------------------------------------------------------- touch-up

/// Notes due for a touch-up (04 §11.2): dirty, live, quiet for `delay_s`,
/// not open in the editor.
pub fn due_touchups(cx: &Cx, delay_s: i64, open: &HashSet<String>) -> Result<Vec<Note>> {
    let cutoff = crate::db::parse_timestamp(cx.now())
        .map(|t| fields::fmt_instant(t - time::Duration::seconds(delay_s)))
        .unwrap_or_default();
    let rows = repo::query::<Note>(
        cx.conn,
        "frontmatter_dirty = 1 AND deleted_at IS NULL AND (file_mtime IS NULL OR file_mtime <= ?) ORDER BY updated_at",
        &[&cutoff],
    )?;
    Ok(rows.into_iter().filter(|n| !open.contains(&n.id)).collect())
}

/// What a touch-up did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchUp {
    Written,
    /// The file changed since it was indexed; the next reconcile handles it.
    Deferred,
    /// Unparseable, not UTF-8, too large or read-only: never written (§3.3).
    Skipped(String),
}

/// The values a row holds, in the shape Mode writes (04 §3.4).
pub fn canonical_from_row(note: &Note, tags: Vec<String>, write_status: bool) -> Canonical {
    Canonical {
        id: note.id.clone(),
        status: note.status.clone(),
        write_status,
        urgent: note.urgent,
        when: note.when_date.clone(),
        tags,
        created: note.created_at.clone(),
        updated: note.updated_at.clone(),
        rev: note.rev,
    }
}

/// Rewrite Mode's keys from the row, body verbatim (04 §11.2): the file
/// must still hash to the row's `content_hash` and parse; `rev` and
/// `updated` are written as the row holds them (no bump).
pub fn touch_up(cx: &Cx, note: &Note) -> Result<TouchUp> {
    let abs = vault::abs_path(&cx.env.root, &note.path);
    let bytes = match std::fs::read(&abs) {
        Ok(b) => b,
        Err(e) => return Ok(TouchUp::Skipped(e.to_string())),
    };
    if note.content_hash.as_deref() != Some(fsio::sha256_hex(&bytes).as_str()) {
        return Ok(TouchUp::Deferred);
    }
    if bytes.len() as u64 > vault::MAX_INDEX_BYTES {
        clear_dirty(cx, &note.id)?;
        return Ok(TouchUp::Skipped("larger than 10 MB".into()));
    }
    let Ok(text) = String::from_utf8(bytes) else {
        clear_dirty(cx, &note.id)?;
        return Ok(TouchUp::Skipped("not UTF-8".into()));
    };
    let doc = Document::parse(&text);
    if let Some(reason) = doc.error() {
        clear_dirty(cx, &note.id)?;
        return Ok(TouchUp::Skipped(format!(
            "frontmatter could not be read: {reason}"
        )));
    }
    let fields = doc.fields(cx.tz());
    let write_status = doc.entry("status").is_some() || note.status != "todo";
    let c = canonical_from_row(note, fields.tags.clone(), write_status);
    let mut edited: Vec<&'static str> = vec!["id", "rev", "created", "updated"];
    edited.retain(|k| match *k {
        "id" => fields.id.as_deref() != Some(note.id.as_str()),
        "rev" => fields.rev != Some(note.rev),
        "created" => fields.created.as_deref() != Some(note.created_at.as_str()),
        _ => fields.updated.as_deref() != Some(note.updated_at.as_str()),
    });
    let Some(new_text) = frontmatter::rewrite(&doc, &c, &WriteOpts::editing(&edited)) else {
        return Ok(TouchUp::Skipped("frontmatter could not be read".into()));
    };
    if new_text == text {
        clear_dirty(cx, &note.id)?;
        return Ok(TouchUp::Written);
    }
    if let Err(e) = fsio::atomic_write(&abs, new_text.as_bytes()) {
        return Ok(TouchUp::Skipped(e.to_string()));
    }
    let stat = fsio::stat(&abs)?;
    let hash = fsio::sha256_hex(new_text.as_bytes());
    let new_doc = Document::parse(&new_text);
    let new_fields = new_doc.fields(cx.tz());
    cx.conn.execute(
        "UPDATE notes SET content_hash = ?1, file_mtime = ?2, file_size = ?3, frontmatter_dirty = 0,
                          indexed_at = ?4, when_raw = ?5
          WHERE id = ?6",
        params![hash, stat.mtime, stat.size, cx.now(), new_fields.when_raw, note.id],
    )?;
    if new_text.len() as u64 <= vault::MAX_VERSIONED_BYTES {
        insert_revision(cx, &note.id, note.rev, "mode", &new_text, &hash)?;
    }
    Ok(TouchUp::Written)
}

fn clear_dirty(cx: &Cx, id: &str) -> Result<()> {
    cx.conn
        .execute("UPDATE notes SET frontmatter_dirty = 0 WHERE id = ?1", [id])?;
    Ok(())
}

// ------------------------------------------------------------ vault index

/// `notes.indexed_vault_id` (device): the vault the index was built from.
pub const INDEXED_VAULT_KEY: &str = "notes.indexed_vault_id";

/// Drop the notes index (04 §2.2: vault path changed): `notes`, `folders`,
/// `note_tags`, `note_revisions`; groupings, tag colors and settings stay.
pub fn drop_index(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DELETE FROM note_tags; DELETE FROM note_revisions; DELETE FROM notes; DELETE FROM folders;",
    )?;
    Ok(())
}

/// Check `.mode/vault.json` against the index: create it when missing,
/// rebuild the index when it names another vault. Returns the vault's
/// `frontmatter` version (higher than [`vault::FRONTMATTER_VERSION`] makes
/// the Notes view read-only).
pub fn check_vault(cx: &Cx) -> Result<vault::VaultJson> {
    std::fs::create_dir_all(&cx.env.root)?;
    let v = vault::ensure_vault_json(&cx.env.root, cx.device(), cx.now())?;
    let indexed = crate::settings::get_string_in(cx.conn, INDEXED_VAULT_KEY)?;
    if indexed.as_deref() != Some(v.vault_id.as_str()) {
        if indexed.is_some() {
            tracing::info!(vault = %v.vault_id, "vault changed: rebuilding the notes index");
            drop_index(cx.conn)?;
        }
        crate::settings::set_scoped_in(
            cx.conn,
            cx.device(),
            INDEXED_VAULT_KEY,
            &serde_json::Value::String(v.vault_id.clone()),
            crate::settings::Scope::Device,
        )?;
    }
    Ok(v)
}

/// Read a file that must be indexable and writable (Mode writes).
pub fn read_writable(cx: &Cx, rel: &str) -> Result<(String, Stat, String)> {
    let abs = vault::abs_path(&cx.env.root, rel);
    let bytes = std::fs::read(&abs)?;
    let stat = fsio::stat(&abs)?;
    let hash = fsio::sha256_hex(&bytes);
    if bytes.len() as u64 > vault::MAX_INDEX_BYTES {
        return Err(invalid(
            "note",
            "the file is larger than 10 MB and is read-only in Mode",
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| {
        invalid(
            "note",
            "the file is not valid UTF-8 and is read-only in Mode",
        )
    })?;
    Ok((text, stat, hash))
}
