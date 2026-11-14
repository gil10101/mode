//! Import and export (04 §13). Import copies a folder of `.md` files into
//! the vault (the source is never modified): ids kept unless taken, new
//! ones assigned, `tag:` (singular) and string `tags:` read as `tags`,
//! `date:` left alone, `source_kind = 'import'`, an `import` revision per
//! note; a preview counts everything first. Export of the Mode-only state
//! is `notes-state.json` for the settings bundle (§13.2).

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::ctx::{Cx, invalid};
use super::fields;
use super::frontmatter::{self, Canonical, Document, Value as YValue, WriteOpts};
use super::reconcile;
use super::vault;
use super::write;
use crate::db::Result;
use crate::repo;

/// The dialog's preview, and the report after applying (04 §13.1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub notes: usize,
    pub folders: usize,
    pub ids_kept: usize,
    pub ids_assigned: usize,
    /// Statuses neither premade nor in `notes.statuses`.
    pub unknown_statuses: Vec<String>,
    /// Target paths that existed and got a ` 2` suffix.
    pub collisions: Vec<String>,
    pub attachments: usize,
    /// The vault-relative destination folder.
    pub destination: String,
    /// Ids of the notes created (empty in a preview).
    pub created: Vec<String>,
}

fn walk_source(src: &Path) -> std::io::Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let mut md = Vec::new();
    let mut other = Vec::new();
    let mut dirs = Vec::new();
    let mut stack = vec![(src.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        for e in std::fs::read_dir(&dir)?.filter_map(|e| e.ok()) {
            let Some(name) = e.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if vault::ignored_name(&name) {
                continue;
            }
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                dirs.push(child.clone());
                stack.push((e.path(), child));
            } else if name.ends_with(".md") {
                md.push(child);
            } else {
                other.push(child);
            }
        }
    }
    md.sort();
    other.sort();
    dirs.sort();
    Ok((md, other, dirs))
}

/// `tags` from `tags:` and `tag:` in any accepted shape (04 §13.1).
fn import_tags(doc: &Document, cx: &Cx) -> Vec<String> {
    let mut items: Vec<String> = doc.fields(cx.tz()).tags;
    if let Some(e) = doc.entry("tag") {
        match &e.value {
            YValue::Scalar(s) => items.extend(fields::split_tag_string(s)),
            YValue::Seq(v) => items.extend(v.iter().filter_map(|x| match x {
                YValue::Scalar(s) => Some(s.clone()),
                _ => None,
            })),
            _ => {}
        }
    }
    fields::normalize_tags(items).0
}

/// Preview (`apply = false`) or run an import of `src` into vault folder
/// `dest` (default: a new folder named after the source).
pub fn import(
    cx: &Cx,
    src: &Path,
    dest: Option<&str>,
    include_attachments: bool,
    apply: bool,
) -> Result<ImportReport> {
    if !src.is_dir() {
        return Err(invalid(
            "import",
            format!("{} is not a folder", src.display()),
        ));
    }
    if src.starts_with(&cx.env.root) || cx.env.root.starts_with(src) {
        return Err(invalid("import", "the source overlaps the vault"));
    }
    let dest = match dest {
        Some(d) => d.trim_matches('/').to_string(),
        None => vault::stem_for_title(
            &src.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Import".into()),
        ),
    };
    if !dest.is_empty() && !vault::is_folder_path(&dest) {
        return Err(invalid("import", format!("{dest:?} cannot be a folder")));
    }
    let (md, other, dirs) = walk_source(src)?;
    let mut report = ImportReport {
        notes: md.len(),
        folders: dirs.len(),
        attachments: if include_attachments { other.len() } else { 0 },
        destination: dest.clone(),
        ..Default::default()
    };
    let mut taken: HashSet<String> = {
        let mut s = cx.conn.prepare("SELECT id FROM notes")?;
        s.query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<_>>>()?
    };
    let known: Vec<String> = cx.statuses().iter().map(|s| s.slug.clone()).collect();
    let join = |rel: &str| {
        if dest.is_empty() {
            rel.to_string()
        } else {
            format!("{dest}/{rel}")
        }
    };
    for rel in &md {
        let bytes = std::fs::read(src.join(rel))?;
        let text = String::from_utf8(bytes.clone()).ok();
        let doc = text.as_deref().map(Document::parse);
        let fields = match &doc {
            Some(d) if d.error().is_none() => d.fields(cx.tz()),
            _ => Default::default(),
        };
        if let Some(s) = &fields.status
            && !known.contains(s)
            && !report.unknown_statuses.contains(s)
        {
            report.unknown_statuses.push(s.clone());
        }
        let id = match fields.id.clone().filter(|i| !taken.contains(i)) {
            Some(i) => {
                report.ids_kept += 1;
                i
            }
            None => {
                report.ids_assigned += 1;
                repo::new_id()
            }
        };
        taken.insert(id.clone());
        let target = join(rel);
        let dir = vault::parent_dir(&target).to_string();
        let stem = vault::title_from_path(&target);
        let free = vault::unique_name(&cx.env.root, &dir, &stem)?;
        if free != target {
            report.collisions.push(target.clone());
        }
        if !apply {
            continue;
        }
        let writable = doc.as_ref().is_some_and(|d| d.error().is_none());
        let out: Vec<u8> = match (&doc, writable) {
            (Some(d), true) => {
                let c = Canonical {
                    id: id.clone(),
                    status: fields.status.clone().unwrap_or_else(|| "todo".into()),
                    write_status: d.entry("status").is_some(),
                    urgent: fields.urgent.unwrap_or(false),
                    when: fields.when.as_ref().map(|w| {
                        let anchor = fields
                            .updated
                            .as_deref()
                            .and_then(crate::db::parse_timestamp)
                            .map(|t| cx.tz().local_date(t))
                            .unwrap_or_else(|| cx.today());
                        fields::fmt_date(w.resolve(anchor))
                    }),
                    tags: import_tags(d, cx),
                    created: fields
                        .created
                        .clone()
                        .unwrap_or_else(|| cx.now().to_string()),
                    updated: fields
                        .updated
                        .clone()
                        .unwrap_or_else(|| cx.now().to_string()),
                    rev: fields.rev.unwrap_or(1),
                };
                let mut edited: Vec<&'static str> = vec!["id"];
                if c.tags != fields.tags {
                    edited.push("tags");
                }
                frontmatter::rewrite(d, &c, &WriteOpts::editing(&edited))
                    .map(String::into_bytes)
                    .unwrap_or(bytes.clone())
            }
            _ => bytes.clone(),
        };
        std::fs::create_dir_all(vault::abs_path(&cx.env.root, &dir))?;
        let note = match String::from_utf8(out.clone()) {
            Ok(t) if writable => {
                write::commit(cx, &free, &t, None, true, Some("import"), "import")?
            }
            _ => {
                // Copied verbatim; indexed like any file found in the vault.
                super::fsio::atomic_create(&vault::abs_path(&cx.env.root, &free), &out)?;
                let p = reconcile::prepare(&cx.env.root, &free, None);
                let opts = reconcile::ApplyOpts {
                    origin: "import",
                    source_kind: "import",
                    min_rev: None,
                };
                let o = reconcile::apply(cx, &p, &opts)?;
                let id2 = o.note_id.unwrap_or_default();
                repo::get_by_id::<repo::Note>(cx.conn, &id2)?.unwrap_or_default()
            }
        };
        cx.conn.execute(
            "UPDATE notes SET source_kind = 'import' WHERE id = ?1",
            params![note.id],
        )?;
        report.created.push(note.id);
    }
    if apply && include_attachments {
        for rel in &other {
            let target = vault::abs_path(&cx.env.root, &join(rel));
            if target.exists() {
                continue;
            }
            if let Some(d) = target.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::copy(src.join(rel), &target)?;
        }
    }
    if apply {
        for d in &dirs {
            reconcile::ensure_folder(cx, &join(d))?;
        }
        if !dest.is_empty() {
            std::fs::create_dir_all(vault::abs_path(&cx.env.root, &dest))?;
            reconcile::ensure_folder(cx, &dest)?;
        }
    }
    Ok(report)
}

/// `notes-state.json` (04 §13.2): pinned / archived per note id, plus the
/// links whose both ends are notes or URLs.
pub fn export_state(cx: &Cx) -> Result<Value> {
    let mut notes: BTreeMap<String, Value> = BTreeMap::new();
    let mut s = cx.conn.prepare(
        "SELECT id, pinned, archived_at FROM notes WHERE deleted_at IS NULL AND (pinned = 1 OR archived_at IS NOT NULL) ORDER BY id",
    )?;
    for r in s.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })? {
        let (id, pinned, archived_at) = r?;
        notes.insert(id, json!({"pinned": pinned, "archived_at": archived_at}));
    }
    let links = repo::query::<repo::Link>(
        cx.conn,
        "deleted_at IS NULL AND from_kind = 'note' AND to_kind IN ('note','url') ORDER BY created_at",
        &[],
    )?;
    let links: Vec<Value> = links
        .into_iter()
        .map(|l| {
            json!({
                "id": l.id, "from_kind": l.from_kind, "from_id": l.from_id, "to_kind": l.to_kind,
                "to_id": l.to_id, "link_type": l.link_type, "label": l.label, "created_by": l.created_by,
            })
        })
        .collect();
    Ok(json!({"notes": notes, "links": links}))
}

/// Apply a `notes-state.json` to the notes this vault has (by id);
/// returns how many notes and links changed.
pub fn import_state(cx: &Cx, state: &Value) -> Result<(usize, usize)> {
    let mut n = 0;
    if let Some(map) = state.get("notes").and_then(Value::as_object) {
        for (id, v) in map {
            let pinned = v.get("pinned").and_then(Value::as_i64).unwrap_or(0) == 1;
            let archived = v
                .get("archived_at")
                .and_then(Value::as_str)
                .map(str::to_string);
            n += cx.conn.execute(
                "UPDATE notes SET pinned = ?1, archived_at = ?2, rev = rev + 1, updated_at = ?3, device_id = ?4,
                                  frontmatter_dirty = 1
                  WHERE id = ?5 AND deleted_at IS NULL AND (pinned <> ?1 OR archived_at IS NOT ?2)",
                params![pinned, archived, cx.now(), cx.device(), id],
            )?;
        }
    }
    let mut l = 0;
    for link in state
        .get("links")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let s = |k: &str| {
            link.get(k)
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        let (from_id, to_kind, to_id) = (s("from_id"), s("to_kind"), s("to_id"));
        let exists = |id: &str| -> Result<bool> {
            Ok(cx.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL)",
                [id],
                |r| r.get(0),
            )?)
        };
        if !exists(&from_id)? || (to_kind == "note" && !exists(&to_id)?) {
            continue;
        }
        let label = link.get("label").and_then(Value::as_str);
        let by = s("created_by");
        let by = if matches!(by.as_str(), "user" | "ai" | "system") {
            by
        } else {
            "user".into()
        };
        let kind = s("link_type");
        if !matches!(
            kind.as_str(),
            "created_from" | "references" | "prepare_for" | "related" | "attachment"
        ) || !matches!(to_kind.as_str(), "note" | "url")
        {
            continue;
        }
        write::link(
            cx,
            ("note", &from_id),
            (&to_kind, &to_id),
            &kind,
            label,
            &by,
        )?;
        l += 1;
    }
    Ok((n, l))
}

/// A source folder for [`import`] from a path string.
pub fn source(path: &str) -> PathBuf {
    vault::expand(path)
}
