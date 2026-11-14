//! The `groupings.filter_json` grammar of 09 §5.1 and its compiler to a
//! `WHERE` clause over `notes n`, plus `sort_json`. The query bar chips of
//! 03 compile to the same object. Every key is optional; keys are AND-ed.

use rusqlite::types::Value as SqlValue;
use serde::{Deserialize, Serialize};

use crate::db::{DbError, Result};
use crate::repo::normalize_tag;

/// `{"in": [...]}` or `{"not_in": [...]}`; `"closed"` inside `not_in` expands
/// to every status whose `closed` flag is set in `settings['notes.statuses']`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusFilter {
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub r#in: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_in: Option<Vec<String>>,
}

/// `{"in": [buckets]}` or `{"from": date, "to": date}`; either bound is
/// optional and may be a relative token (`today`, `week_start`, `today+7`,
/// …; 04 §6.4) that the compiler resolves against today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WhenFilter {
    Buckets {
        #[serde(rename = "in")]
        r#in: Vec<String>,
    },
    Range {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagsFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub none: Option<Vec<String>>,
}

/// `{"id": "<folder id>" | "root", "recursive": true | false}`. A missing
/// `recursive` reads as false here (package 1); the notes layer applies
/// 04 §6.1's default (true) before parsing (`notes::groupings::validate_filter`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderFilter {
    pub id: String,
    #[serde(default)]
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonFilter {
    pub contact_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoFilter {
    pub repo_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InList {
    #[serde(rename = "in")]
    pub r#in: Vec<String>,
}

/// The whole `filter_json` object.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub urgent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<WhenFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<TagsFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<FolderFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<PersonFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<RepoFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<InList>,
}

impl Filter {
    pub fn parse(json: &str) -> Result<Filter> {
        serde_json::from_str(json).map_err(|e| DbError::Invalid {
            what: "filter_json",
            detail: e.to_string(),
        })
    }
}

/// `sort_json`: `[{"by": "updated_at", "dir": "desc"}]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SortKey {
    pub by: String,
    #[serde(default = "default_dir")]
    pub dir: String,
}

fn default_dir() -> String {
    "asc".into()
}

/// Columns `sort_json` may name.
pub const SORTABLE: &[&str] = &[
    "updated_at",
    "created_at",
    "title",
    "when_date",
    "status",
    "urgent",
    "pinned",
    "path",
    "word_count",
];

pub fn parse_sort(json: &str) -> Result<Vec<SortKey>> {
    serde_json::from_str(json).map_err(|e| DbError::Invalid {
        what: "sort_json",
        detail: e.to_string(),
    })
}

/// `ORDER BY` for a sort list (empty list = `updated_at DESC`). Pinned notes
/// always come first within a grouping view.
pub fn compile_sort(sort: &[SortKey]) -> Result<String> {
    compile_sort_with(sort, &[])
}

/// [`compile_sort`] with `status` ordered by position in
/// `settings['notes.statuses']` (04 §6.2); unknown statuses sort last.
/// An empty `status_order` sorts by slug.
pub fn compile_sort_with(sort: &[SortKey], status_order: &[String]) -> Result<String> {
    let mut parts = vec!["n.pinned DESC".to_string()];
    if sort.is_empty() {
        parts.push("n.updated_at DESC".into());
    }
    for key in sort {
        if !SORTABLE.contains(&key.by.as_str()) {
            return Err(DbError::Invalid {
                what: "sort_json",
                detail: format!("{} is not sortable", key.by),
            });
        }
        let dir = match key.dir.to_ascii_lowercase().as_str() {
            "asc" => "ASC",
            "desc" => "DESC",
            other => {
                return Err(DbError::Invalid {
                    what: "sort_json",
                    detail: format!("direction {other:?}"),
                });
            }
        };
        // NULL when_date ("none") sorts last either way.
        if key.by == "when_date" {
            parts.push("n.when_date IS NULL".into());
        }
        if key.by == "status" && !status_order.is_empty() {
            let arms: Vec<String> = status_order
                .iter()
                .enumerate()
                .map(|(i, s)| format!("WHEN '{}' THEN {i}", s.replace('\'', "''")))
                .collect();
            parts.push(format!(
                "CASE n.status {} ELSE {} END {dir}",
                arms.join(" "),
                status_order.len()
            ));
            continue;
        }
        parts.push(format!("n.{} {dir}", key.by));
    }
    parts.push("n.seq DESC".into());
    Ok(parts.join(", "))
}

/// What the compiler needs from the app: local dates and the closed statuses.
#[derive(Debug, Clone, Default)]
pub struct FilterContext {
    pub today: String,
    pub tomorrow: String,
    pub closed_statuses: Vec<String>,
}

impl FilterContext {
    /// The closed statuses as SQL literals list, `'done'` when none is configured.
    fn closed(&self) -> Vec<String> {
        if self.closed_statuses.is_empty() {
            vec!["done".to_string()]
        } else {
            self.closed_statuses.clone()
        }
    }
}

/// A compiled `WHERE` fragment with positional parameters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Compiled {
    pub where_sql: String,
    pub params: Vec<SqlValue>,
}

fn placeholders(n: usize) -> String {
    (0..n).map(|_| "?").collect::<Vec<_>>().join(", ")
}

/// The `when_bucket` CASE of 09 Q4 (generalized to the configured closed statuses).
pub fn when_bucket_case(ctx: &FilterContext, params: &mut Vec<SqlValue>) -> String {
    let closed = ctx.closed();
    params.push(SqlValue::Text(ctx.today.clone()));
    for c in &closed {
        params.push(SqlValue::Text(c.clone()));
    }
    params.push(SqlValue::Text(ctx.today.clone()));
    params.push(SqlValue::Text(ctx.tomorrow.clone()));
    format!(
        "CASE WHEN n.when_date < ? AND n.status NOT IN ({}) THEN 'overdue'
              WHEN n.when_date = ? THEN 'today'
              WHEN n.when_date = ? THEN 'tomorrow'
              WHEN n.when_date IS NOT NULL THEN 'scheduled' ELSE 'none' END",
        placeholders(closed.len())
    )
}

/// Compile a filter to `WHERE` conditions over `notes n` (joined to nothing;
/// sub-selects reach the other tables). Always includes `n.deleted_at IS NULL`
/// and excludes notes inside the `templates` folder.
pub fn compile(filter: &Filter, ctx: &FilterContext) -> Result<Compiled> {
    let mut conds: Vec<String> = vec!["n.deleted_at IS NULL".into()];
    let mut params: Vec<SqlValue> = Vec::new();
    let text = |s: &str| SqlValue::Text(s.to_string());

    match filter.archived {
        Some(true) => conds.push("n.archived_at IS NOT NULL".into()),
        _ => conds.push("n.archived_at IS NULL".into()),
    }
    conds.push(
        "(n.folder_id IS NULL OR n.folder_id NOT IN (SELECT id FROM folders WHERE kind = 'templates'))"
            .into(),
    );

    if let Some(status) = &filter.status {
        if let Some(list) = &status.r#in
            && !list.is_empty()
        {
            conds.push(format!("n.status IN ({})", placeholders(list.len())));
            params.extend(list.iter().map(|s| text(s)));
        }
        if let Some(list) = &status.not_in {
            let mut expanded: Vec<String> = Vec::new();
            for s in list {
                if s == "closed" {
                    expanded.extend(ctx.closed());
                } else {
                    expanded.push(s.clone());
                }
            }
            expanded.dedup();
            if !expanded.is_empty() {
                conds.push(format!(
                    "n.status NOT IN ({})",
                    placeholders(expanded.len())
                ));
                params.extend(expanded.iter().map(|s| text(s)));
            }
        }
    }

    if let Some(urgent) = filter.urgent {
        conds.push(format!("n.urgent = {}", i32::from(urgent)));
    }

    match &filter.when {
        Some(WhenFilter::Buckets { r#in }) => {
            let mut ors = Vec::new();
            for bucket in r#in {
                let closed = ctx.closed();
                match bucket.as_str() {
                    "overdue" => {
                        ors.push(format!(
                            "(n.when_date < ? AND n.status NOT IN ({}))",
                            placeholders(closed.len())
                        ));
                        params.push(text(&ctx.today));
                        params.extend(closed.iter().map(|s| text(s)));
                    }
                    "today" => {
                        ors.push("n.when_date = ?".into());
                        params.push(text(&ctx.today));
                    }
                    "tomorrow" => {
                        ors.push("n.when_date = ?".into());
                        params.push(text(&ctx.tomorrow));
                    }
                    "scheduled" => {
                        ors.push(format!(
                            "(n.when_date IS NOT NULL AND n.when_date <> ? AND n.when_date <> ? \
                             AND NOT (n.when_date < ? AND n.status NOT IN ({})))",
                            placeholders(closed.len())
                        ));
                        params.push(text(&ctx.today));
                        params.push(text(&ctx.tomorrow));
                        params.push(text(&ctx.today));
                        params.extend(closed.iter().map(|s| text(s)));
                    }
                    "none" => ors.push("n.when_date IS NULL".into()),
                    other => {
                        return Err(DbError::Invalid {
                            what: "filter_json",
                            detail: format!("unknown when bucket {other:?}"),
                        });
                    }
                }
            }
            if !ors.is_empty() {
                conds.push(format!("({})", ors.join(" OR ")));
            }
        }
        Some(WhenFilter::Range { from, to }) => {
            let today = crate::notes::fields::parse_date(&ctx.today);
            let resolve = |bound: &str| -> Result<String> {
                today
                    .and_then(|t| {
                        crate::notes::fields::resolve_date_token(
                            bound,
                            t,
                            crate::notes::fields::WeekStart::Monday,
                        )
                    })
                    .or_else(|| crate::notes::fields::parse_date(bound))
                    .map(crate::notes::fields::fmt_date)
                    .ok_or_else(|| DbError::Invalid {
                        what: "filter_json",
                        detail: format!("when bound {bound:?} is not a date"),
                    })
            };
            if from.is_none() && to.is_none() {
                return Err(DbError::Invalid {
                    what: "filter_json",
                    detail: "when needs \"in\", \"from\" or \"to\"".into(),
                });
            }
            conds.push("n.when_date IS NOT NULL".into());
            if let Some(f) = from {
                conds.push("n.when_date >= ?".into());
                params.push(text(&resolve(f)?));
            }
            if let Some(t) = to {
                conds.push("n.when_date <= ?".into());
                params.push(text(&resolve(t)?));
            }
        }
        None => {}
    }

    if let Some(tags) = &filter.tags {
        let norm =
            |v: &Vec<String>| -> Vec<String> { v.iter().map(|t| normalize_tag(t)).collect() };
        if let Some(any) = &tags.any
            && !any.is_empty()
        {
            let any = norm(any);
            conds.push(format!(
                "EXISTS (SELECT 1 FROM note_tags nt JOIN tags t ON t.id = nt.tag_id \
                 WHERE nt.note_id = n.id AND t.name_norm IN ({}))",
                placeholders(any.len())
            ));
            params.extend(any.iter().map(|s| text(s)));
        }
        if let Some(all) = &tags.all {
            for tag in norm(all) {
                conds.push(
                    "EXISTS (SELECT 1 FROM note_tags nt JOIN tags t ON t.id = nt.tag_id \
                     WHERE nt.note_id = n.id AND t.name_norm = ?)"
                        .into(),
                );
                params.push(text(&tag));
            }
        }
        if let Some(none) = &tags.none
            && !none.is_empty()
        {
            let none = norm(none);
            conds.push(format!(
                "NOT EXISTS (SELECT 1 FROM note_tags nt JOIN tags t ON t.id = nt.tag_id \
                 WHERE nt.note_id = n.id AND t.name_norm IN ({}))",
                placeholders(none.len())
            ));
            params.extend(none.iter().map(|s| text(s)));
        }
    }

    if let Some(folder) = filter.folder.as_ref().filter(|f| f.id == "root") {
        // The vault root: every note when recursive, else the root's own notes.
        if !folder.recursive {
            conds.push("n.folder_id IS NULL".into());
        }
    } else if let Some(folder) = &filter.folder {
        if folder.recursive {
            conds.push(
                "n.folder_id IN (SELECT f.id FROM folders f, folders r \
                 WHERE r.id = ? AND f.deleted_at IS NULL AND (f.id = r.id OR f.path LIKE r.path || '/%'))"
                    .into(),
            );
        } else {
            conds.push("n.folder_id = ?".into());
        }
        params.push(text(&folder.id));
    }

    if let Some(t) = &filter.text
        && let Some(q) = super::fts::build(t, super::fts::Fts::Notes, false)
    {
        if let Some(m) = q.match_expr {
            conds.push("n.seq IN (SELECT rowid FROM notes_fts WHERE notes_fts MATCH ?)".into());
            params.push(text(&m));
        }
        if let Some(scope) = q.in_scope {
            conds.push("n.path LIKE ? || '%'".into());
            params.push(text(&scope));
        }
    }

    if let Some(person) = &filter.person {
        conds.push(
            "EXISTS (SELECT 1 FROM links l WHERE l.deleted_at IS NULL AND (\
               (l.from_kind = 'note' AND l.from_id = n.id AND (\
                 (l.to_kind = 'contact' AND l.to_id = ?) OR \
                 (l.to_kind = 'message' AND l.to_id IN (SELECT id FROM messages WHERE sender_contact_id = ?)))) OR \
               (l.to_kind = 'note' AND l.to_id = n.id AND (\
                 (l.from_kind = 'contact' AND l.from_id = ?) OR \
                 (l.from_kind = 'message' AND l.from_id IN (SELECT id FROM messages WHERE sender_contact_id = ?))))))"
                .into(),
        );
        for _ in 0..4 {
            params.push(text(&person.contact_id));
        }
    }

    if let Some(repo) = &filter.repo {
        conds.push(
            "EXISTS (SELECT 1 FROM links l WHERE l.deleted_at IS NULL AND (\
               (l.from_kind = 'note' AND l.from_id = n.id AND l.to_kind = 'session' \
                  AND l.to_id IN (SELECT id FROM sessions WHERE repo_root = ?)) OR \
               (l.to_kind = 'note' AND l.to_id = n.id AND l.from_kind = 'session' \
                  AND l.from_id IN (SELECT id FROM sessions WHERE repo_root = ?))))"
                .into(),
        );
        params.push(text(&repo.repo_root));
        params.push(text(&repo.repo_root));
    }

    if filter.pinned == Some(true) {
        conds.push("n.pinned = 1".into());
    }

    if let Some(kinds) = &filter.source_kind
        && !kinds.r#in.is_empty()
    {
        conds.push(format!(
            "n.source_kind IN ({})",
            placeholders(kinds.r#in.len())
        ));
        params.extend(kinds.r#in.iter().map(|s| text(s)));
    }

    Ok(Compiled {
        where_sql: conds.join("\n   AND "),
        params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> FilterContext {
        FilterContext {
            today: "2026-10-21".into(),
            tomorrow: "2026-10-22".into(),
            closed_statuses: vec!["done".into()],
        }
    }

    #[test]
    fn premade_filters_parse_and_compile() {
        for (slug, json) in [
            ("todo", r#"{"status":{"in":["todo"]}}"#),
            (
                "urgent",
                r#"{"urgent":true,"status":{"not_in":["closed"]}}"#,
            ),
            (
                "today",
                r#"{"when":{"in":["overdue","today"]},"status":{"not_in":["closed"]}}"#,
            ),
            (
                "tomorrow",
                r#"{"when":{"in":["tomorrow"]},"status":{"not_in":["closed"]}}"#,
            ),
        ] {
            let f = Filter::parse(json).unwrap_or_else(|e| panic!("{slug}: {e}"));
            let c = compile(&f, &ctx()).unwrap();
            assert!(c.where_sql.contains("n.deleted_at IS NULL"), "{slug}");
            assert!(c.where_sql.contains("n.archived_at IS NULL"), "{slug}");
        }
        let today =
            Filter::parse(r#"{"when":{"in":["overdue","today"]},"status":{"not_in":["closed"]}}"#)
                .unwrap();
        let c = compile(&today, &ctx()).unwrap();
        assert!(c.where_sql.contains("n.status NOT IN (?)"));
        assert!(c.where_sql.contains("n.when_date = ?"));
        assert_eq!(
            c.params,
            vec![
                SqlValue::Text("done".into()),
                SqlValue::Text("2026-10-21".into()),
                SqlValue::Text("done".into()),
                SqlValue::Text("2026-10-21".into()),
            ]
        );
    }

    #[test]
    fn unknown_keys_and_buckets_are_rejected() {
        assert!(Filter::parse(r#"{"colour":"red"}"#).is_err());
        let f = Filter::parse(r#"{"when":{"in":["yesterday"]}}"#).unwrap();
        assert!(compile(&f, &ctx()).is_err());
        assert!(Filter::parse(r#"{"when":{"from":"2026-01-01","to":"2026-01-31"}}"#).is_ok());
        assert!(parse_sort(r#"[{"by":"updated_at","dir":"desc"}]"#).is_ok());
        assert!(compile_sort(&parse_sort(r#"[{"by":"body","dir":"desc"}]"#).unwrap()).is_err());
        assert!(
            compile_sort(&parse_sort(r#"[{"by":"title","dir":"sideways"}]"#).unwrap()).is_err()
        );
        assert_eq!(
            compile_sort(&[]).unwrap(),
            "n.pinned DESC, n.updated_at DESC, n.seq DESC"
        );
    }

    #[test]
    fn tags_are_normalised_and_folders_recurse() {
        let f = Filter::parse(r#"{"tags":{"any":["  Groceries ","Gym"],"all":["x"],"none":["y"]},"folder":{"id":"f1","recursive":true},"archived":true,"pinned":true,"source_kind":{"in":["capture"]}}"#).unwrap();
        let c = compile(&f, &ctx()).unwrap();
        assert!(c.where_sql.contains("n.archived_at IS NOT NULL"));
        assert!(c.where_sql.contains("n.pinned = 1"));
        assert!(c.where_sql.contains("f.path LIKE r.path || '/%'"));
        assert!(c.params.contains(&SqlValue::Text("groceries".into())));
        assert!(c.params.contains(&SqlValue::Text("gym".into())));
        assert_eq!(c.where_sql.matches("EXISTS").count(), 3);
        assert!(c.where_sql.contains("NOT EXISTS"));
    }
}
