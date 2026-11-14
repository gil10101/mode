//! The notes core (04; 08 §3.2 row `notes`; 09 §8): the vault on disk, the
//! frontmatter parser / writer that never destroys bytes it did not write,
//! task lines, field semantics, reconcile (mtime → hash → parse), touch-up,
//! revisions, conflicts, archive / trash, groupings and the `filter_json`
//! compiler, the query-bar chip resolver, capture, the vault watcher and
//! the `notes.changed` push. [`api`] has one plain function per command.
//!
//! The vocabulary below (one `status` per note, an `urgent` flag, `when`,
//! free `tags[]`, the glyphs `○ ◑ ✱ ✓`) is package 0's, kept as is.

pub mod api;
pub mod body;
pub mod capture;
pub mod ctx;
pub mod eol;
pub mod fields;
pub mod frontmatter;
pub mod fsio;
pub mod groupings;
pub mod import;
pub mod push;
pub mod query;
pub mod reconcile;
pub mod service;
pub mod vault;
pub mod views;
pub mod watcher;
pub mod write;

pub use service::{Notes, NotesOptions, VaultStatus};

use serde::{Deserialize, Serialize};

/// `status` ∈ {todo, doing, done, custom…}; custom statuses are user-defined slugs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Todo,
    Doing,
    Done,
    #[serde(untagged)]
    Custom(String),
}

impl Status {
    pub fn from_slug(slug: &str) -> Status {
        match slug {
            "todo" => Status::Todo,
            "doing" => Status::Doing,
            "done" => Status::Done,
            other => Status::Custom(other.to_string()),
        }
    }

    pub fn slug(&self) -> &str {
        match self {
            Status::Todo => "todo",
            Status::Doing => "doing",
            Status::Done => "done",
            Status::Custom(s) => s,
        }
    }

    /// Done-like statuses render with a strikethrough and never show `✱`.
    pub fn closed(&self) -> bool {
        matches!(self, Status::Done)
    }

    pub fn glyph(&self) -> &'static str {
        match self {
            Status::Todo | Status::Custom(_) => "\u{25CB}", // ○
            Status::Doing => "\u{25D1}",                    // ◑
            Status::Done => "\u{2713}",                     // ✓
        }
    }
}

/// `✱`: the urgent marker, shown instead of the status glyph on open tasks.
pub const URGENT_GLYPH: &str = "\u{2731}";

/// The glyph a task line shows (`○` todo, `◑` doing, `✱` urgent, `✓` done).
pub fn task_glyph(status: &Status, urgent: bool) -> &'static str {
    if urgent && !status.closed() {
        URGENT_GLYPH
    } else {
        status.glyph()
    }
}

/// `when` ∈ {today, tomorrow, date, none}; "overdue" is derived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind", content = "date")]
pub enum When {
    Today,
    Tomorrow,
    /// An ISO date, `YYYY-MM-DD`.
    Date(String),
    None,
}

impl When {
    /// A dated task before `today` (ISO date) is overdue.
    pub fn is_overdue(&self, today: &str) -> bool {
        matches!(self, When::Date(d) if d.as_str() < today)
    }
}

/// Premade groupings (saved views) every vault starts with.
pub const PREMADE_GROUPINGS: &[&str] = &["Todo", "Doing", "Urgent", "Done", "Today", "Tomorrow"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_follow_the_brief() {
        assert_eq!(task_glyph(&Status::Todo, false), "○");
        assert_eq!(task_glyph(&Status::Doing, false), "◑");
        assert_eq!(task_glyph(&Status::Todo, true), "✱");
        assert_eq!(
            task_glyph(&Status::Done, true),
            "✓",
            "done never shows urgent"
        );
        assert_eq!(task_glyph(&Status::Custom("groceries".into()), false), "○");
    }

    #[test]
    fn statuses_serialise_as_slugs_including_custom() {
        assert_eq!(serde_json::to_string(&Status::Doing).unwrap(), "\"doing\"");
        assert_eq!(
            serde_json::to_string(&Status::Custom("gym".into())).unwrap(),
            "\"gym\""
        );
        assert_eq!(
            serde_json::from_str::<Status>("\"gym\"").unwrap(),
            Status::Custom("gym".into())
        );
        assert_eq!(
            serde_json::from_str::<Status>("\"done\"").unwrap(),
            Status::Done
        );
        assert_eq!(Status::from_slug("todo"), Status::Todo);
        assert_eq!(Status::from_slug("gym").slug(), "gym");
    }

    #[test]
    fn overdue_is_derived_from_when() {
        assert!(When::Date("2026-10-20".into()).is_overdue("2026-10-21"));
        assert!(!When::Date("2026-10-21".into()).is_overdue("2026-10-21"));
        assert!(!When::Today.is_overdue("2026-10-21"));
        assert_eq!(
            serde_json::to_string(&When::Date("2026-10-21".into())).unwrap(),
            r#"{"kind":"date","date":"2026-10-21"}"#
        );
        assert_eq!(PREMADE_GROUPINGS.len(), 6);
    }
}
