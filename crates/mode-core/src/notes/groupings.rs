//! Groupings, the saved views of 04 §6: the filter grammar of §6.1 with
//! validation that names the offending key, the sort grammar of §6.2, the
//! six premade groupings of §6.3 (seeded by migration `0001`, reused here),
//! custom groupings (§6.4) and the one-edit drop rule of §6.5.
//!
//! [`run`] is the single path from a `filter_json` to rows and a count: the
//! groupings, the Notes list, the overlay sections and the query-bar cards
//! all go through it (and through `queries::filter::compile`), so a card
//! and a grouping with equal filters return identical rows and counts.

use rusqlite::types::Value as SqlValue;
use rusqlite::{OptionalExtension, params_from_iter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ctx::{Cx, invalid};
use super::fields;
use super::write::FieldEdit;
use crate::db::{DbError, Result};
use crate::queries::filter::{self as qf, Filter, SortKey, WhenFilter};
use crate::repo::{self, Grouping};

/// The when buckets of 04 §5.3.
pub const BUCKETS: &[&str] = &["overdue", "today", "tomorrow", "scheduled", "none"];
/// `source_kind` values (09 §5.1).
pub const SOURCE_KINDS: &[&str] = &[
    "manual", "capture", "message", "session", "calendar", "import", "chat",
];
/// `sort_json` keys (04 §6.2).
pub const SORT_KEYS: &[&str] = &[
    "updated_at",
    "created_at",
    "when_date",
    "title",
    "urgent",
    "status",
];
/// The Today sort of §6.3, the default card sort (§8.4).
pub fn today_sort() -> Vec<SortKey> {
    vec![
        SortKey {
            by: "urgent".into(),
            dir: "desc".into(),
        },
        SortKey {
            by: "when_date".into(),
            dir: "asc".into(),
        },
        SortKey {
            by: "updated_at".into(),
            dir: "desc".into(),
        },
    ]
}

// ------------------------------------------------------------ validation

fn strings(v: &Value, key: &str) -> std::result::Result<Vec<String>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| format!("{key} must be a list"))?;
    arr.iter()
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| format!("{key} items must be names"))
        })
        .collect()
}

fn object<'a>(
    v: &'a Value,
    key: &str,
    allowed: &[&str],
) -> std::result::Result<&'a serde_json::Map<String, Value>, String> {
    let o = v
        .as_object()
        .ok_or_else(|| format!("{key} must be an object"))?;
    for k in o.keys() {
        if !allowed.contains(&k.as_str()) {
            return Err(format!("unknown key {key}.{k}"));
        }
    }
    Ok(o)
}

/// Validate a `filter_json` value (04 §6.1); errors name the key.
pub fn validate_filter(v: &Value) -> std::result::Result<Filter, String> {
    let o = v.as_object().ok_or("the filter must be an object")?;
    for (k, val) in o {
        match k.as_str() {
            "status" => {
                let s = object(val, "status", &["in", "not_in"])?;
                for (kk, list) in s {
                    for slug in strings(list, &format!("status.{kk}"))? {
                        if slug != "closed"
                            && !fields::is_slug(&slug)
                            && fields::fold_status(&slug) != slug
                        {
                            return Err(format!("status.{kk}: {slug:?} is not a status"));
                        }
                        if slug == "closed" && kk == "in" {
                            return Err("status.in cannot name \"closed\"".into());
                        }
                    }
                }
            }
            "urgent" | "archived" | "pinned" => {
                if !val.is_boolean() {
                    return Err(format!("{k} must be true or false"));
                }
            }
            "when" => {
                let w = object(val, "when", &["in", "from", "to"])?;
                let has_in = w.contains_key("in");
                let has_range = w.contains_key("from") || w.contains_key("to");
                if has_in && has_range {
                    return Err("when: buckets and a date range cannot be combined".into());
                }
                if !has_in && !has_range {
                    return Err("when needs \"in\" or \"from\" / \"to\"".into());
                }
                if let Some(list) = w.get("in") {
                    for b in strings(list, "when.in")? {
                        if !BUCKETS.contains(&b.as_str()) {
                            return Err(format!("when.in: {b:?} is not a bucket"));
                        }
                    }
                }
                for bound in ["from", "to"] {
                    if let Some(b) = w.get(bound) {
                        let s = b
                            .as_str()
                            .ok_or_else(|| format!("when.{bound} must be a date"))?;
                        if !fields::is_date_token(s) {
                            return Err(format!(
                                "when.{bound}: {s:?} is not a date or a relative token"
                            ));
                        }
                    }
                }
            }
            "tags" => {
                let t = object(val, "tags", &["all", "any", "none"])?;
                for (kk, list) in t {
                    for name in strings(list, &format!("tags.{kk}"))? {
                        if fields::tag_norm(&name).is_none() {
                            return Err(format!("tags.{kk}: {name:?} is not a tag"));
                        }
                    }
                }
            }
            "folder" => {
                let f = object(val, "folder", &["id", "recursive"])?;
                if !f.get("id").is_some_and(Value::is_string) {
                    return Err("folder.id must be a folder id or \"root\"".into());
                }
                if f.get("recursive").is_some_and(|r| !r.is_boolean()) {
                    return Err("folder.recursive must be true or false".into());
                }
            }
            "text" => {
                if !val.as_str().is_some_and(|s| !s.trim().is_empty()) {
                    return Err("text must be words to search for".into());
                }
            }
            "person" => {
                let p = object(val, "person", &["contact_id"])?;
                if !p.get("contact_id").is_some_and(Value::is_string) {
                    return Err("person.contact_id must be a contact id".into());
                }
            }
            "repo" => {
                let r = object(val, "repo", &["repo_root"])?;
                if !r.get("repo_root").is_some_and(Value::is_string) {
                    return Err("repo.repo_root must be a path".into());
                }
            }
            "source_kind" => {
                let s = object(val, "source_kind", &["in"])?;
                for kind in strings(s.get("in").unwrap_or(&Value::Null), "source_kind.in")? {
                    if !SOURCE_KINDS.contains(&kind.as_str()) {
                        return Err(format!("source_kind.in: {kind:?} is not a source"));
                    }
                }
            }
            other => return Err(format!("unknown key {other:?}")),
        }
    }
    // 04 §6.1: `folder.recursive` defaults to true.
    let mut v = v.clone();
    if let Some(f) = v.get_mut("folder").and_then(Value::as_object_mut) {
        f.entry("recursive").or_insert(Value::Bool(true));
    }
    serde_json::from_value::<Filter>(v).map_err(|e| e.to_string())
}

/// Validate a `sort_json` value (04 §6.2).
pub fn validate_sort(v: &Value) -> std::result::Result<Vec<SortKey>, String> {
    let arr = v.as_array().ok_or("the sort must be a list")?;
    let mut out = Vec::new();
    for item in arr {
        let o = object(item, "sort", &["by", "dir"])?;
        let by = o
            .get("by")
            .and_then(Value::as_str)
            .ok_or("sort.by is required")?;
        if !SORT_KEYS.contains(&by) {
            return Err(format!("sort.by: {by:?} cannot be sorted on"));
        }
        let dir = o.get("dir").and_then(Value::as_str).unwrap_or("asc");
        if dir != "asc" && dir != "desc" {
            return Err(format!("sort.dir: {dir:?} is not asc or desc"));
        }
        out.push(SortKey {
            by: by.into(),
            dir: dir.into(),
        });
    }
    Ok(out)
}

/// Resolve relative `from` / `to` tokens to literal dates with the user's
/// week start, so every caller compiles the same filter.
pub fn resolve_dates(cx: &Cx, f: &Filter) -> Filter {
    let mut f = f.clone();
    if let Some(WhenFilter::Range { from, to }) = &mut f.when {
        let today = cx.today();
        for b in [from, to].into_iter().flatten() {
            if let Some(d) = fields::resolve_date_token(b, today, cx.week_start) {
                *b = fields::fmt_date(d);
            }
        }
    }
    f
}

// ------------------------------------------------------------------ rows

/// A note in a list, a board column or a card (contract `NoteListRow`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteListRow {
    pub id: String,
    pub title: String,
    pub status: String,
    pub urgent: bool,
    pub when_date: Option<String>,
    /// `overdue` | `today` | `tomorrow` | `scheduled` | `none`.
    pub when_bucket: String,
    pub path: String,
    pub folder_id: Option<String>,
    pub excerpt: String,
    pub task_total: i64,
    pub task_done: i64,
    pub pinned: bool,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub source_kind: String,
    /// Display names in frontmatter order.
    pub tags: Vec<String>,
    pub has_links: bool,
    /// An active overlay snooze (04 §5.5).
    pub snoozed: bool,
}

/// What [`run`] needs besides the filter.
#[derive(Debug, Clone, Default)]
pub struct RunOpts {
    pub sort: Vec<SortKey>,
    pub limit: i64,
    pub offset: i64,
    /// "Show archived": archived and active notes together (03 §5.8).
    pub include_archived: bool,
    /// Restrict to one note (drop rule "already in").
    pub only_id: Option<String>,
}

/// Compile a filter (dates resolved, archived optionally included) to a
/// `WHERE` fragment.
pub fn compile(cx: &Cx, filter: &Filter, include_archived: bool) -> Result<qf::Compiled> {
    let ctx = cx.filter_context();
    let f = resolve_dates(cx, filter);
    if include_archived && f.archived != Some(true) {
        let active = qf::compile(&f, &ctx)?;
        let mut archived = f.clone();
        archived.archived = Some(true);
        let arch = qf::compile(&archived, &ctx)?;
        let mut params = active.params;
        params.extend(arch.params);
        return Ok(qf::Compiled {
            where_sql: format!("(({}) OR ({}))", active.where_sql, arch.where_sql),
            params,
        });
    }
    qf::compile(&f, &ctx)
}

pub(crate) const ROW_SELECT: &str = "n.id, n.title, n.status, n.urgent, n.when_date, {bucket} AS when_bucket, n.path, n.folder_id,
       n.excerpt, n.task_total, n.task_done, n.pinned, n.archived_at, n.created_at, n.updated_at, n.source_kind,
       (SELECT group_concat(t.name, char(31) ORDER BY nt.position) FROM note_tags nt JOIN tags t ON t.id = nt.tag_id
         WHERE nt.note_id = n.id AND t.deleted_at IS NULL) AS tags,
       EXISTS (SELECT 1 FROM links l WHERE l.deleted_at IS NULL
                 AND ((l.from_kind = 'note' AND l.from_id = n.id) OR (l.to_kind = 'note' AND l.to_id = n.id))) AS has_links,
       EXISTS (SELECT 1 FROM snoozes z WHERE z.target_kind = 'note' AND z.target_id = n.id AND z.fired_at IS NULL
                 AND z.cancelled_at IS NULL AND z.deleted_at IS NULL AND z.until > ?) AS snoozed";

pub(crate) fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<NoteListRow> {
    let tags: Option<String> = r.get(16)?;
    Ok(NoteListRow {
        id: r.get(0)?,
        title: r.get(1)?,
        status: r.get(2)?,
        urgent: r.get(3)?,
        when_date: r.get(4)?,
        when_bucket: r.get(5)?,
        path: r.get(6)?,
        folder_id: r.get(7)?,
        excerpt: r.get(8)?,
        task_total: r.get(9)?,
        task_done: r.get(10)?,
        pinned: r.get(11)?,
        archived_at: r.get(12)?,
        created_at: r.get(13)?,
        updated_at: r.get(14)?,
        source_kind: r.get(15)?,
        tags: tags
            .map(|s| s.split('\u{1f}').map(str::to_string).collect())
            .unwrap_or_default(),
        has_links: r.get(17)?,
        snoozed: r.get(18)?,
    })
}

/// The status order of `settings['notes.statuses']` for `sort by status`.
fn status_order(cx: &Cx) -> Vec<String> {
    cx.statuses().iter().map(|s| s.slug.clone()).collect()
}

/// Rows and `COUNT(*)` of a filter: the one query behind groupings, lists,
/// the overlay and cards (09 §13 Q4).
pub fn run(cx: &Cx, filter: &Filter, opts: &RunOpts) -> Result<(Vec<NoteListRow>, i64)> {
    let compiled = compile(cx, filter, opts.include_archived)?;
    let mut where_sql = compiled.where_sql.clone();
    let mut where_params = compiled.params.clone();
    if let Some(id) = &opts.only_id {
        where_sql = format!("({where_sql}) AND n.id = ?");
        where_params.push(SqlValue::Text(id.clone()));
    }
    let count: i64 = cx.conn.query_row(
        &format!("SELECT COUNT(*) FROM notes n WHERE {where_sql}"),
        params_from_iter(where_params.iter()),
        |r| r.get(0),
    )?;
    if opts.limit == 0 {
        return Ok((Vec::new(), count));
    }
    let ctx = cx.filter_context();
    let mut params: Vec<SqlValue> = Vec::new();
    let bucket = qf::when_bucket_case(&ctx, &mut params);
    params.push(SqlValue::Text(cx.now().to_string()));
    params.extend(where_params);
    params.push(SqlValue::Integer(if opts.limit < 0 {
        -1
    } else {
        opts.limit
    }));
    params.push(SqlValue::Integer(opts.offset.max(0)));
    let order = qf::compile_sort_with(&opts.sort, &status_order(cx))?;
    let sql = format!(
        "SELECT {} FROM notes n WHERE {where_sql} ORDER BY {order} LIMIT ? OFFSET ?",
        ROW_SELECT.replace("{bucket}", &bucket)
    );
    let mut stmt = cx.conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(params), row_from)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok((rows, count))
}

/// One note as a list row (any state, archived included).
pub fn row_for(cx: &Cx, note_id: &str) -> Result<Option<NoteListRow>> {
    let ctx = cx.filter_context();
    let mut params: Vec<SqlValue> = Vec::new();
    let bucket = qf::when_bucket_case(&ctx, &mut params);
    params.push(SqlValue::Text(cx.now().to_string()));
    params.push(SqlValue::Text(note_id.to_string()));
    let sql = format!(
        "SELECT {} FROM notes n WHERE n.id = ?",
        ROW_SELECT.replace("{bucket}", &bucket)
    );
    Ok(cx
        .conn
        .query_row(&sql, params_from_iter(params), row_from)
        .optional()?)
}

// ------------------------------------------------------------- groupings

/// A grouping with its live count (contract `GroupingRow`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupingRow {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub kind: String,
    pub filter_json: Value,
    pub sort_json: Value,
    pub glyph: Option<String>,
    pub color: Option<String>,
    pub position: i64,
    pub show_in_overlay: bool,
    pub show_in_sidebar: bool,
    pub count: i64,
    /// False when the stored filter no longer validates (shown grayed out
    /// with "Edit filter", 04 §6.1).
    pub valid: bool,
    pub error: Option<String>,
}

/// A stored grouping's filter and sort, validated.
pub fn parsed(g: &Grouping) -> std::result::Result<(Filter, Vec<SortKey>), String> {
    let fv: Value = serde_json::from_str(&g.filter_json).map_err(|e| e.to_string())?;
    let f = validate_filter(&fv)?;
    let sv: Value = serde_json::from_str(&g.sort_json).map_err(|e| e.to_string())?;
    let s = validate_sort(&sv)?;
    Ok((f, s))
}

pub fn grouping_row(cx: &Cx, g: &Grouping) -> Result<GroupingRow> {
    let (count, valid, error) = match parsed(g) {
        Ok((f, _)) => match run(cx, &f, &RunOpts::default()) {
            Ok((_, n)) => (n, true, None),
            Err(e) => (0, false, Some(e.to_string())),
        },
        Err(e) => (0, false, Some(e)),
    };
    Ok(GroupingRow {
        id: g.id.clone(),
        slug: g.slug.clone(),
        name: g.name.clone(),
        kind: g.kind.clone(),
        filter_json: serde_json::from_str(&g.filter_json).unwrap_or(Value::Null),
        sort_json: serde_json::from_str(&g.sort_json).unwrap_or(Value::Null),
        glyph: g.glyph.clone(),
        color: g.color.clone(),
        position: g.position,
        show_in_overlay: g.show_in_overlay,
        show_in_sidebar: g.show_in_sidebar,
        count,
        valid,
        error,
    })
}

/// Live groupings in `position` order.
pub fn live(cx: &Cx) -> Result<Vec<Grouping>> {
    repo::list_live::<Grouping>(cx.conn, "position, name")
}

pub fn by_slug(cx: &Cx, slug: &str) -> Result<Grouping> {
    repo::query::<Grouping>(cx.conn, "slug = ? AND deleted_at IS NULL", &[&slug])?
        .into_iter()
        .next()
        .ok_or_else(|| DbError::NotFound {
            table: "groupings",
            key: slug.to_string(),
        })
}

/// A grouping as the settings editor or "Save as grouping" sends it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GroupingDraft {
    /// Absent: create a custom grouping.
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub glyph: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    pub filter_json: Value,
    #[serde(default)]
    pub sort_json: Option<Value>,
    #[serde(default)]
    pub show_in_sidebar: Option<bool>,
    #[serde(default)]
    pub show_in_overlay: Option<bool>,
    #[serde(default)]
    pub position: Option<i64>,
}

/// `[a-z0-9-]` from a name.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() { "grouping".into() } else { s }
}

/// Create or edit a grouping (04 §6.3, §6.4). Premade rows keep their
/// name, filter and sort; glyph, color, visibility and position change.
pub fn save(cx: &Cx, draft: &GroupingDraft) -> Result<Grouping> {
    let filter = validate_filter(&draft.filter_json).map_err(|e| invalid("filter_json", e))?;
    let sort = match &draft.sort_json {
        Some(v) => validate_sort(v).map_err(|e| invalid("sort_json", e))?,
        None => vec![SortKey {
            by: "updated_at".into(),
            dir: "desc".into(),
        }],
    };
    let name = draft.name.trim();
    let existing = match &draft.id {
        Some(id) => {
            Some(
                repo::get_live::<Grouping>(cx.conn, id)?.ok_or_else(|| DbError::NotFound {
                    table: "groupings",
                    key: id.clone(),
                })?,
            )
        }
        None => None,
    };
    if let Some(mut g) = existing.clone().filter(|g| g.kind == "premade") {
        let (f0, s0) = parsed(&g).map_err(|e| invalid("filter_json", e))?;
        if f0 != filter || name != g.name || (draft.sort_json.is_some() && s0 != sort) {
            return Err(invalid(
                "grouping",
                "a premade grouping keeps its name, filter and sort; its glyph, color and visibility can change",
            ));
        }
        apply_presentation(&mut g, draft);
        repo::save(cx.conn, cx.device(), &mut g)?;
        return Ok(g);
    }
    if name.is_empty() {
        return Err(invalid("name", "a grouping needs a name"));
    }
    let self_id = existing.as_ref().map(|g| g.id.clone()).unwrap_or_default();
    let clash: Option<String> = cx
        .conn
        .query_row(
            "SELECT id FROM groupings WHERE deleted_at IS NULL AND lower(name) = lower(?1) AND id <> ?2",
            rusqlite::params![name, self_id],
            |r| r.get(0),
        )
        .optional()?;
    if clash.is_some() {
        return Err(invalid("name", format!("a grouping named {name:?} exists")));
    }
    let filter_json = serde_json::to_string(&draft.filter_json)?;
    let sort_json = serde_json::to_string(&sort)?;
    match existing {
        Some(mut g) => {
            g.name = name.to_string();
            g.filter_json = filter_json;
            g.sort_json = sort_json;
            apply_presentation(&mut g, draft);
            repo::save(cx.conn, cx.device(), &mut g)?;
            Ok(g)
        }
        None => {
            let base = slugify(name);
            let mut slug = base.clone();
            let mut n = 2;
            while cx.conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM groupings WHERE slug = ?1 AND deleted_at IS NULL)",
                [&slug],
                |r| r.get::<_, bool>(0),
            )? {
                slug = format!("{base}-{n}");
                n += 1;
            }
            let position: i64 = cx.conn.query_row(
                "SELECT COALESCE(MAX(position), -1) + 1 FROM groupings WHERE deleted_at IS NULL",
                [],
                |r| r.get(0),
            )?;
            let mut g = Grouping {
                id: repo::new_id(),
                name: name.to_string(),
                slug,
                kind: "custom".into(),
                filter_json,
                sort_json,
                glyph: Some("\u{25A3}".into()),
                color: None,
                position,
                show_in_overlay: false,
                show_in_sidebar: true,
                ..Default::default()
            };
            apply_presentation(&mut g, draft);
            let _ = filter;
            repo::create(cx.conn, cx.device(), &mut g)?;
            Ok(g)
        }
    }
}

fn apply_presentation(g: &mut Grouping, d: &GroupingDraft) {
    if let Some(glyph) = d.glyph.as_ref().filter(|s| !s.trim().is_empty()) {
        g.glyph = Some(glyph.clone());
    }
    if d.color.is_some() {
        g.color = d.color.clone().filter(|c| !c.is_empty());
    }
    if let Some(b) = d.show_in_sidebar {
        g.show_in_sidebar = b;
    }
    if let Some(b) = d.show_in_overlay {
        g.show_in_overlay = b;
    }
    if let Some(p) = d.position {
        g.position = p;
    }
}

/// Delete a custom grouping (tombstone); premade ones are never deletable.
pub fn delete(cx: &Cx, id: &str) -> Result<()> {
    let g = repo::get_live::<Grouping>(cx.conn, id)?.ok_or_else(|| DbError::NotFound {
        table: "groupings",
        key: id.to_string(),
    })?;
    if g.kind == "premade" {
        return Err(invalid(
            "grouping",
            "premade groupings can be hidden, not deleted",
        ));
    }
    repo::soft_delete::<Grouping>(cx.conn, cx.device(), id)?;
    Ok(())
}

// --------------------------------------------------------- the drop rule

/// The fields the drop rule reads (04 §6.5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropNote {
    pub id: Option<String>,
    pub status: String,
    pub urgent: bool,
    pub when_date: Option<String>,
    pub tags: Vec<String>,
    pub folder_id: Option<String>,
    pub pinned: bool,
    pub archived: bool,
    pub source_kind: String,
}

impl DropNote {
    /// A blank note: `todo`, not urgent, no `when`, no tags (⊕ Add task).
    pub fn blank() -> DropNote {
        DropNote {
            status: "todo".into(),
            source_kind: "manual".into(),
            ..Default::default()
        }
    }

    pub fn of(cx: &Cx, id: &str) -> Result<DropNote> {
        let n = super::reconcile::live_note(cx.conn, id)?;
        Ok(DropNote {
            id: Some(n.id.clone()),
            status: n.status,
            urgent: n.urgent,
            when_date: n.when_date,
            tags: super::write::tags_of(cx, id)?,
            folder_id: n.folder_id,
            pinned: n.pinned,
            archived: n.archived_at.is_some(),
            source_kind: n.source_kind,
        })
    }
}

/// One field edit the drop rule may perform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "key", content = "value", rename_all = "snake_case")]
pub enum DropEdit {
    Status(String),
    Urgent(bool),
    /// `today` | `tomorrow` | `None` (clear).
    When(Option<String>),
    TagAdd(String),
    TagRemove(String),
}

impl DropEdit {
    fn key(&self) -> &'static str {
        match self {
            DropEdit::Status(_) => "status",
            DropEdit::Urgent(_) => "urgent",
            DropEdit::When(_) => "when",
            DropEdit::TagAdd(_) | DropEdit::TagRemove(_) => "tags",
        }
    }
}

/// The per-key evaluation of §6.5.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DropEval {
    pub edits: Vec<DropEdit>,
    /// Keys that no single deterministic edit satisfies.
    pub refused: Vec<String>,
}

/// The decision for a drop.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DropPlan {
    /// The note already matches ("already in Today").
    Noop { hint: String },
    /// Exactly one field edit.
    Edit { edit: FieldEdit, hint: String },
    /// Refused; `needs` names the keys ("needs status and when").
    Refuse { needs: Vec<String>, hint: String },
}

fn refuse(e: &mut DropEval, key: &str) {
    if !e.refused.iter().any(|k| k == key) {
        e.refused.push(key.to_string());
    }
}

/// Whether `note` satisfies the filter keys that are not field edits
/// (folder, text, person, repo, source, pinned, archived), by SQL.
fn key_satisfied(cx: &Cx, note: &DropNote, f: Filter) -> Result<bool> {
    let Some(id) = &note.id else {
        return Ok(false);
    };
    let mut f = f;
    f.archived = Some(note.archived);
    let opts = RunOpts {
        limit: 0,
        only_id: Some(id.clone()),
        ..RunOpts::default()
    };
    Ok(run(cx, &f, &opts)?.1 > 0)
}

/// Evaluate §6.5 for one (note, filter) pair. For `when.in` the assignable
/// buckets are `today`, `tomorrow` and `none`: exactly one of them in the
/// list is the edit (Today's `[overdue, today]` assigns `today`, as the
/// worked case "a done note → Today needs status and when" reads).
pub fn evaluate(cx: &Cx, note: &DropNote, filter: &Filter, for_add: bool) -> Result<DropEval> {
    let mut e = DropEval::default();
    let norm = |t: &str| fields::tag_norm(t).unwrap_or_default();
    let tags: Vec<String> = note.tags.iter().map(|t| norm(t)).collect();
    let closed = cx.closed_statuses();
    if let Some(s) = &filter.status {
        if let Some(list) = s.r#in.as_ref().filter(|l| !l.is_empty())
            && !list.contains(&note.status)
        {
            if list.len() == 1 {
                e.edits.push(DropEdit::Status(list[0].clone()));
            } else {
                refuse(&mut e, "status");
            }
        }
        if let Some(list) = &s.not_in {
            let banned: Vec<String> = list
                .iter()
                .flat_map(|x| {
                    if x == "closed" {
                        closed.clone()
                    } else {
                        vec![x.clone()]
                    }
                })
                .collect();
            let pending = e.edits.iter().find_map(|d| match d {
                DropEdit::Status(s) => Some(s.clone()),
                _ => None,
            });
            let current = pending.clone().unwrap_or_else(|| note.status.clone());
            if banned.contains(&current) {
                if pending.is_some() || banned.iter().any(|b| b == "todo") {
                    refuse(&mut e, "status");
                } else {
                    e.edits.push(DropEdit::Status("todo".into()));
                }
            }
        }
    }
    if let Some(b) = filter.urgent
        && note.urgent != b
    {
        e.edits.push(DropEdit::Urgent(b));
    }
    match &filter.when {
        Some(WhenFilter::Buckets { r#in }) => {
            let bucket = fields::when_bucket(
                note.when_date.as_deref(),
                closed.contains(&note.status),
                &cx.clock.today,
                &cx.clock.tomorrow,
            );
            if !r#in.iter().any(|b| b == bucket) {
                let assignable: Vec<&String> = r#in
                    .iter()
                    .filter(|b| matches!(b.as_str(), "today" | "tomorrow" | "none"))
                    .collect();
                if assignable.len() == 1 {
                    let v = assignable[0];
                    e.edits
                        .push(DropEdit::When((v != "none").then(|| v.clone())));
                } else {
                    refuse(&mut e, "when");
                }
            }
        }
        Some(WhenFilter::Range { .. }) => {
            let f = resolve_dates(cx, filter);
            let inside = match (&f.when, &note.when_date) {
                (Some(WhenFilter::Range { from, to }), Some(d)) => {
                    from.as_ref().is_none_or(|x| d >= x) && to.as_ref().is_none_or(|x| d <= x)
                }
                _ => false,
            };
            if !inside {
                refuse(&mut e, "when");
            }
        }
        None => {}
    }
    if let Some(t) = &filter.tags {
        for want in t.all.iter().flatten() {
            if !tags.contains(&norm(want)) {
                e.edits.push(DropEdit::TagAdd(norm(want)));
            }
        }
        if let Some(any) = t.any.as_ref().filter(|a| !a.is_empty())
            && !any.iter().any(|a| tags.contains(&norm(a)))
        {
            if any.len() == 1 {
                e.edits.push(DropEdit::TagAdd(norm(&any[0])));
            } else {
                refuse(&mut e, "tags");
            }
        }
        for not in t.none.iter().flatten() {
            if tags.contains(&norm(not)) {
                e.edits.push(DropEdit::TagRemove(norm(not)));
            }
        }
    }
    // Keys that are not field edits: satisfied already, or refused.
    if let Some(folder) = &filter.folder
        && !for_add
        && !key_satisfied(
            cx,
            note,
            Filter {
                folder: Some(folder.clone()),
                ..Default::default()
            },
        )?
    {
        refuse(&mut e, "folder");
    }
    for (key, sub) in [
        (
            "text",
            filter.text.as_ref().map(|t| Filter {
                text: Some(t.clone()),
                ..Default::default()
            }),
        ),
        (
            "person",
            filter.person.as_ref().map(|p| Filter {
                person: Some(p.clone()),
                ..Default::default()
            }),
        ),
        (
            "repo",
            filter.repo.as_ref().map(|r| Filter {
                repo: Some(r.clone()),
                ..Default::default()
            }),
        ),
    ] {
        if let Some(sub) = sub {
            if for_add && key == "person" {
                continue;
            }
            if for_add || !key_satisfied(cx, note, sub)? {
                refuse(&mut e, key);
            }
        }
    }
    if let Some(s) = &filter.source_kind
        && !s.r#in.contains(&note.source_kind)
        && !(for_add && s.r#in.iter().any(|k| k == "capture" || k == "manual"))
    {
        refuse(&mut e, "source");
    }
    if filter.pinned == Some(true) && !note.pinned {
        refuse(&mut e, "pinned");
    }
    if filter.archived.unwrap_or(false) != note.archived {
        refuse(&mut e, "archived");
    }
    Ok(e)
}

fn join_words(words: &[String]) -> String {
    match words.len() {
        0 => String::new(),
        1 => words[0].clone(),
        n => format!("{} and {}", words[..n - 1].join(", "), words[n - 1]),
    }
}

/// The drop decision (04 §6.5): one deterministic edit, a no-op, or a
/// refusal whose hint names what is missing.
pub fn plan_drop(cx: &Cx, note: &DropNote, name: &str, filter: &Filter) -> Result<DropPlan> {
    if let Some(id) = &note.id {
        let opts = RunOpts {
            limit: 0,
            only_id: Some(id.clone()),
            include_archived: note.archived,
            ..RunOpts::default()
        };
        let mut f = filter.clone();
        if note.archived && f.archived.is_none() {
            f.archived = Some(false);
        }
        if run(cx, &f, &opts)?.1 > 0 {
            return Ok(DropPlan::Noop {
                hint: format!("already in {name}"),
            });
        }
    }
    let e = evaluate(cx, note, filter, false)?;
    if !e.refused.is_empty() || e.edits.len() > 1 {
        let mut needs = e.refused.clone();
        for d in &e.edits {
            if !needs.iter().any(|k| k == d.key()) {
                needs.push(d.key().to_string());
            }
        }
        return Ok(DropPlan::Refuse {
            hint: format!("needs {}", join_words(&needs)),
            needs,
        });
    }
    let Some(edit) = e.edits.into_iter().next() else {
        return Ok(DropPlan::Noop {
            hint: format!("already in {name}"),
        });
    };
    let (edit, hint) = match edit {
        DropEdit::Status(s) => (FieldEdit::Status(s.clone()), format!("status set to {s}")),
        DropEdit::Urgent(b) => (
            FieldEdit::Urgent(b),
            if b {
                "marked urgent".into()
            } else {
                "no longer urgent".into()
            },
        ),
        DropEdit::When(w) => (
            FieldEdit::When(w.clone()),
            match w {
                Some(w) => format!("date set to {w}"),
                None => "date cleared".into(),
            },
        ),
        DropEdit::TagAdd(t) => {
            let mut tags = note.tags.clone();
            tags.push(t.clone());
            (FieldEdit::Tags(tags), format!("tagged #{t}"))
        }
        DropEdit::TagRemove(t) => {
            let tags = note
                .tags
                .iter()
                .filter(|x| fields::tag_norm(x).as_deref() != Some(t.as_str()))
                .cloned()
                .collect();
            (FieldEdit::Tags(tags), format!("#{t} removed"))
        }
    };
    Ok(DropPlan::Edit { edit, hint })
}

/// ⊕ Add task (04 §6.5 last paragraph): shown when every key can be
/// assigned to a blank note; the note is created with every edit applied,
/// in the filter's folder when it names one.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AddTaskPlan {
    pub show: bool,
    pub status: String,
    pub urgent: bool,
    /// `today` | `tomorrow` | `None`.
    pub when: Option<String>,
    pub tags: Vec<String>,
    /// `None` = the capture folder (`"root"` = the vault root).
    pub folder_id: Option<String>,
    /// A person chip becomes a `references` link (03 §5.9.5).
    pub person: Option<String>,
}

pub fn add_task_plan(cx: &Cx, filter: &Filter) -> Result<AddTaskPlan> {
    let e = evaluate(cx, &DropNote::blank(), filter, true)?;
    let mut p = AddTaskPlan {
        show: e.refused.is_empty(),
        status: "todo".into(),
        folder_id: filter.folder.as_ref().map(|f| f.id.clone()),
        person: filter.person.as_ref().map(|p| p.contact_id.clone()),
        ..Default::default()
    };
    for d in e.edits {
        match d {
            DropEdit::Status(s) => p.status = s,
            DropEdit::Urgent(b) => p.urgent = b,
            DropEdit::When(w) => p.when = w,
            DropEdit::TagAdd(t) => p.tags.push(t),
            DropEdit::TagRemove(_) => {}
        }
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validation_names_the_key() {
        assert!(
            validate_filter(&json!({"status":{"in":["todo"]},"when":{"in":["overdue","today"]}}))
                .is_ok()
        );
        let e = validate_filter(&json!({"colour":"red"})).unwrap_err();
        assert!(e.contains("colour"), "{e}");
        assert!(
            validate_filter(&json!({"when":{"in":["today"],"from":"today"}}))
                .unwrap_err()
                .contains("combined")
        );
        assert!(validate_filter(&json!({"when":{"from":"week_start","to":"today+6"}})).is_ok());
        assert!(
            validate_filter(&json!({"when":{"from":"next tuesday"}}))
                .unwrap_err()
                .contains("when.from")
        );
        assert!(
            validate_filter(&json!({"when":{"in":["yesterday"]}}))
                .unwrap_err()
                .contains("bucket")
        );
        assert!(
            validate_filter(&json!({"tags":{"any":["a,b"]}}))
                .unwrap_err()
                .contains("tags.any")
        );
        assert!(validate_filter(&json!({"source_kind":{"in":["fax"]}})).is_err());
        assert!(validate_filter(&json!({"status":{"in":["closed"]}})).is_err());
        assert!(validate_filter(&json!({"folder":{"id":"root"}})).is_ok());
        assert!(
            validate_filter(&json!({"folder":{"id":"x"}}))
                .unwrap()
                .folder
                .unwrap()
                .recursive,
            "recursive defaults to true"
        );
        assert!(validate_sort(&json!([{"by":"status","dir":"asc"}])).is_ok());
        assert!(
            validate_sort(&json!([{"by":"body"}]))
                .unwrap_err()
                .contains("body")
        );
        assert_eq!(slugify("Gym this week!"), "gym-this-week");
        assert_eq!(slugify("!!"), "grouping");
    }
}
