//! Read models for the Notes view and the editor (03 §5; contract
//! `contract-notes.ts`): the folder tree with counts, the tags list, the
//! note detail (file text, `buffer_base_hash`, read-only reasons, links),
//! and the history list (09 §13 S7).

use std::collections::HashMap;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::ctx::Cx;
use super::frontmatter::Document;
use super::groupings::{self, NoteListRow};
use super::vault;
use crate::db::{DbError, Result};

/// A folder in the tree (contract `FolderRow`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderRow {
    pub id: String,
    pub parent_id: Option<String>,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub position: i64,
    /// Non-archived notes in this folder and below.
    pub note_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldersList {
    pub vault_name: String,
    pub vault_path: String,
    /// Notes directly at the vault root.
    pub root_count: i64,
    pub folders: Vec<FolderRow>,
}

pub fn folders(cx: &Cx) -> Result<FoldersList> {
    let mut stmt = cx.conn.prepare(
        "SELECT id, parent_id, path, name, kind, position FROM folders WHERE deleted_at IS NULL ORDER BY position, name",
    )?;
    let mut rows = stmt
        .query_map([], |r| {
            Ok(FolderRow {
                id: r.get(0)?,
                parent_id: r.get(1)?,
                path: r.get(2)?,
                name: r.get(3)?,
                kind: r.get(4)?,
                position: r.get(5)?,
                note_count: 0,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut per_folder: HashMap<Option<String>, i64> = HashMap::new();
    let mut stmt = cx.conn.prepare(
        "SELECT folder_id, COUNT(*) FROM notes WHERE deleted_at IS NULL AND archived_at IS NULL GROUP BY folder_id",
    )?;
    for r in stmt.query_map([], |r| {
        Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
    })? {
        let (k, v) = r?;
        per_folder.insert(k, v);
    }
    let paths: Vec<(String, String)> = rows
        .iter()
        .map(|f| (f.id.clone(), f.path.clone()))
        .collect();
    for f in &mut rows {
        let prefix = format!("{}/", f.path);
        f.note_count = paths
            .iter()
            .filter(|(_, p)| *p == f.path || p.starts_with(&prefix))
            .map(|(id, _)| per_folder.get(&Some(id.clone())).copied().unwrap_or(0))
            .sum();
    }
    let root = &cx.env.root;
    Ok(FoldersList {
        vault_name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        vault_path: root.to_string_lossy().to_string(),
        root_count: per_folder.get(&None).copied().unwrap_or(0),
        folders: rows,
    })
}

/// A tag with its count (contract `TagRow`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRow {
    pub id: String,
    pub name: String,
    pub name_norm: String,
    pub color: Option<String>,
    /// Non-archived notes carrying the tag.
    pub count: i64,
    /// Archived notes carrying it (04 §5.4: counts include archived, labeled).
    pub archived_count: i64,
}

pub fn tags(cx: &Cx) -> Result<Vec<TagRow>> {
    let mut stmt = cx.conn.prepare(
        "SELECT t.id, t.name, t.name_norm, t.color,
                COUNT(CASE WHEN n.archived_at IS NULL THEN 1 END),
                COUNT(CASE WHEN n.archived_at IS NOT NULL THEN 1 END)
           FROM tags t
           LEFT JOIN note_tags nt ON nt.tag_id = t.id
           LEFT JOIN notes n ON n.id = nt.note_id AND n.deleted_at IS NULL
          WHERE t.deleted_at IS NULL
          GROUP BY t.id
          ORDER BY 5 DESC, t.name_norm",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
                name_norm: r.get(2)?,
                color: r.get(3)?,
                count: r.get(4)?,
                archived_count: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// A `links` row seen from the note (contract `NoteLink`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteLink {
    pub id: String,
    pub link_type: String,
    /// `from`: the note is the link's source; `to`: its target.
    pub direction: String,
    pub kind: String,
    pub target_id: String,
    pub label: Option<String>,
    pub title: Option<String>,
    /// False once the other end is gone ("unavailable").
    pub available: bool,
}

fn other_title(cx: &Cx, kind: &str, id: &str) -> Result<(Option<String>, bool)> {
    let sql = match kind {
        "note" => "SELECT title, deleted_at IS NULL FROM notes WHERE id = ?1",
        "session" => "SELECT display_name, 1 FROM sessions WHERE id = ?1",
        "message" => "SELECT COALESCE(subject, snippet), 1 FROM messages WHERE id = ?1",
        "thread" => "SELECT title, 1 FROM threads WHERE id = ?1",
        "calendar_event" => "SELECT title, 1 FROM calendar_events WHERE id = ?1",
        "contact" => "SELECT display_name, deleted_at IS NULL FROM contacts WHERE id = ?1",
        _ => return Ok((Some(id.to_string()), true)),
    };
    Ok(cx
        .conn
        .query_row(sql, [id], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, bool>(1)?))
        })
        .optional()?
        .unwrap_or((None, false)))
}

pub fn links(cx: &Cx, note_id: &str) -> Result<Vec<NoteLink>> {
    let rows = crate::queries::links_for(cx.conn, "note", note_id)?;
    let mut stmt = cx
        .conn
        .prepare("SELECT from_kind, from_id FROM links WHERE id = ?1")?;
    let mut out = Vec::new();
    for l in rows {
        let (from_kind, from_id): (String, String) =
            stmt.query_row([&l.id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let direction = if from_kind == "note" && from_id == note_id {
            "from"
        } else {
            "to"
        };
        let (title, available) = other_title(cx, &l.other_kind, &l.other_id)?;
        out.push(NoteLink {
            id: l.id,
            link_type: l.link_type,
            direction: direction.into(),
            kind: l.other_kind,
            target_id: l.other_id,
            label: l.label,
            title,
            available,
        });
    }
    Ok(out)
}

/// `note_get` (contract `NoteDetail`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteDetail {
    pub row: NoteListRow,
    /// The file as on disk, frontmatter included.
    pub text: String,
    /// `buffer_base_hash` (04 §11.4).
    pub content_hash: String,
    pub rev: i64,
    /// `""` for the vault root.
    pub folder_path: String,
    pub read_only: bool,
    pub read_only_reason: Option<String>,
    /// Frontmatter present but not valid YAML (04 §3.3).
    pub frontmatter_error: bool,
    /// `status` is neither premade nor in `notes.statuses`.
    pub unknown_status: bool,
    pub links: Vec<NoteLink>,
    /// The file is gone from disk (04 §11.3 banner); `text` is the last revision.
    pub missing: bool,
}

/// The editor's view of a note: the file as it is now (reconciled first
/// when it changed), with the reasons Mode will not write it.
pub fn detail(cx: &Cx, note_id: &str, vault_read_only: Option<&str>) -> Result<NoteDetail> {
    let note = super::reconcile::by_id(cx.conn, note_id)?.ok_or_else(|| DbError::NotFound {
        table: "notes",
        key: note_id.to_string(),
    })?;
    let abs = vault::abs_path(&cx.env.root, &note.path);
    let (text, hash, missing, mut reason) = match std::fs::read(&abs) {
        Ok(bytes) => {
            let hash = super::fsio::sha256_hex(&bytes);
            if note.deleted_at.is_none() && note.content_hash.as_deref() != Some(hash.as_str()) {
                let p = super::reconcile::prepare(&cx.env.root, &note.path, None);
                super::reconcile::apply(cx, &p, &super::reconcile::ApplyOpts::default())?;
            }
            let large = bytes.len() as u64 > vault::MAX_INDEX_BYTES;
            match String::from_utf8(bytes) {
                Ok(t) if !large => (t, hash, false, None),
                Ok(_) => (
                    String::new(),
                    hash,
                    false,
                    Some("The file is larger than 10 MB".to_string()),
                ),
                Err(e) => (
                    String::from_utf8_lossy(e.as_bytes()).to_string(),
                    hash,
                    false,
                    Some("The file is not valid UTF-8".to_string()),
                ),
            }
        }
        Err(_) => {
            let last: Option<String> = cx
                .conn
                .query_row(
                    "SELECT content FROM note_revisions WHERE note_id = ?1 ORDER BY rev DESC LIMIT 1",
                    [note_id],
                    |r| r.get(0),
                )
                .optional()?;
            (last.unwrap_or_default(), String::new(), true, None)
        }
    };
    let note = super::reconcile::by_id(cx.conn, note_id)?.unwrap_or(note);
    let doc = Document::parse(&text);
    let in_templates: bool = match &note.folder_id {
        Some(f) => cx
            .conn
            .query_row(
                "SELECT kind = 'templates' FROM folders WHERE id = ?1",
                [f],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(false),
        None => false,
    };
    if reason.is_none() && in_templates {
        reason = Some("Template".into());
    }
    if reason.is_none() {
        reason = vault_read_only.map(str::to_string);
    }
    let row = groupings::row_for(cx, note_id)?.ok_or_else(|| DbError::NotFound {
        table: "notes",
        key: note_id.to_string(),
    })?;
    Ok(NoteDetail {
        unknown_status: !cx.status_known(&row.status),
        row,
        content_hash: hash,
        rev: note.rev,
        folder_path: vault::parent_dir(&note.path).to_string(),
        read_only: reason.is_some(),
        read_only_reason: reason,
        frontmatter_error: doc.error().is_some(),
        links: links(cx, note_id)?,
        missing,
        text,
    })
}

/// A history row (09 §13 S7) with the device's name when it is this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionRow {
    pub rev: i64,
    pub origin: String,
    pub created_at: String,
    pub device_id: String,
    pub device_name: Option<String>,
    pub bytes: i64,
}

pub fn history(cx: &Cx, note_id: &str) -> Result<Vec<RevisionRow>> {
    let local_name = crate::settings::get_string_in(cx.conn, "device.name")?;
    Ok(crate::queries::note_history(cx.conn, note_id, 50)?
        .into_iter()
        .map(|r| RevisionRow {
            device_name: (r.device_id == cx.device())
                .then(|| local_name.clone())
                .flatten(),
            rev: r.rev,
            origin: r.origin,
            created_at: r.created_at,
            device_id: r.device_id,
            bytes: r.bytes,
        })
        .collect())
}

pub fn revision(cx: &Cx, note_id: &str, rev: i64) -> Result<String> {
    cx.conn
        .query_row(
            "SELECT content FROM note_revisions WHERE note_id = ?1 AND rev = ?2",
            rusqlite::params![note_id, rev],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| DbError::NotFound {
            table: "note_revisions",
            key: format!("{note_id}@{rev}"),
        })
}
