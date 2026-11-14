//! The `notes.changed` payload (02 §7.3, §7.7, §16; contract
//! `NotesChanged`): one section per grouping with `show_in_overlay = 1` in
//! `position` order, rows sorted `urgent DESC, when_date, updated_at DESC`,
//! a note shown in the first section that selects it only, snoozed notes
//! hidden (04 §5.5), and the overdue count for the tab.

use serde::{Deserialize, Serialize};

use super::ctx::Cx;
use super::groupings::{self, NoteListRow, RunOpts};
use crate::db::Result;
use crate::queries::filter::{Filter, StatusFilter, WhenFilter};

/// Rows per overlay section (the overlay shows `overlay.max_rows` of them).
pub const SECTION_ROWS: i64 = 50;

/// An overlay Notes row (contract `NoteRow`, 02 §7.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRow {
    pub id: String,
    pub title: String,
    pub status: String,
    /// The status glyph from `notes.statuses` (`?` for an unknown status);
    /// the overlay draws `✱` over it when `urgent`.
    pub glyph: Option<String>,
    pub urgent: bool,
    /// `overdue` | `today` | `tomorrow` | `date` | `none`.
    pub when_bucket: String,
    pub when_date: Option<String>,
    pub tags: Vec<String>,
    pub task_done: i64,
    pub task_total: i64,
    pub has_links: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotesSection {
    /// The grouping's name.
    pub grouping: String,
    pub glyph: Option<String>,
    pub rows: Vec<NoteRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotesChanged {
    pub sections: Vec<NotesSection>,
    pub overdue_count: i64,
}

fn overlay_row(cx: &Cx, r: NoteListRow) -> NoteRow {
    let glyph = super::body::note_glyph(&r.status, cx.statuses()).to_string();
    NoteRow {
        glyph: Some(glyph),
        when_bucket: if r.when_bucket == "scheduled" {
            "date".into()
        } else {
            r.when_bucket
        },
        id: r.id,
        title: r.title,
        status: r.status,
        urgent: r.urgent,
        when_date: r.when_date,
        tags: r.tags,
        task_done: r.task_done,
        task_total: r.task_total,
        has_links: r.has_links,
    }
}

/// The overlay's sort (02 §7.3).
fn overlay_sort() -> Vec<crate::queries::SortKey> {
    groupings::today_sort()
}

/// Compute `notes.changed`.
pub fn payload(cx: &Cx) -> Result<NotesChanged> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut sections = Vec::new();
    for g in groupings::live(cx)?
        .into_iter()
        .filter(|g| g.show_in_overlay)
    {
        let Ok((filter, _)) = groupings::parsed(&g) else {
            continue;
        };
        let opts = RunOpts {
            sort: overlay_sort(),
            limit: SECTION_ROWS,
            ..RunOpts::default()
        };
        let (rows, _) = groupings::run(cx, &filter, &opts)?;
        let rows: Vec<NoteRow> = rows
            .into_iter()
            .filter(|r| !r.snoozed && seen.insert(r.id.clone()))
            .map(|r| overlay_row(cx, r))
            .collect();
        sections.push(NotesSection {
            grouping: g.name.clone(),
            glyph: g.glyph.clone(),
            rows,
        });
    }
    let overdue = Filter {
        when: Some(WhenFilter::Buckets {
            r#in: vec!["overdue".into()],
        }),
        status: Some(StatusFilter {
            r#in: None,
            not_in: Some(vec!["closed".into()]),
        }),
        ..Default::default()
    };
    let (rows, _) = groupings::run(
        cx,
        &overdue,
        &RunOpts {
            limit: -1,
            ..RunOpts::default()
        },
    )?;
    Ok(NotesChanged {
        sections,
        overdue_count: rows.iter().filter(|r| !r.snoozed).count() as i64,
    })
}
