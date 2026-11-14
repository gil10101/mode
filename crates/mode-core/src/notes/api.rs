//! One plain function per notes command (`ui/src/state/contract-notes.ts`
//! and `note_capture` of `contract.ts`): an args struct, a result struct,
//! `serde` snake_case, so `mode-app` can wrap each in a
//! `#[tauri::command]` mechanically:
//!
//! ```ignore
//! #[tauri::command(rename_all = "snake_case")]
//! pub fn notes_list(state: State<'_, Arc<CoreState>>, scope: NotesScope, sort: Option<Vec<SortKey>>,
//!                   show_archived: Option<bool>, limit: Option<i64>, offset: Option<i64>)
//!     -> Result<NotesListResult, ApiError> {
//!     api::notes_list(&state.notes, NotesListArgs { scope, sort, show_archived, limit, offset })
//! }
//! ```
//!
//! Every mutating function requests `notes.changed` through the service's
//! publisher hook.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::capture::{self, CreatedFrom};
use super::fields::WeekStart;
use super::groupings::{
    self, AddTaskPlan, DropNote, DropPlan, GroupingDraft, GroupingRow, NoteListRow, RunOpts,
};
use super::query::{self, Chip, Suggestion};
use super::service::Notes;
use super::views::{self, FoldersList, NoteDetail, NoteLink, RevisionRow, TagRow};
use super::write::{self, BatchReport, FieldEdit, Outcome, Resolve};
use crate::db::DbError;
use crate::queries::filter::{Filter, FolderFilter, SortKey};
use crate::settings::StatusDef;

// ------------------------------------------------------------------ errors

/// A command error the webview can show: `code` is `not_found`, `invalid`,
/// `read_only`, `io` or `internal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        let code = match &e {
            DbError::NotFound { .. } => "not_found",
            DbError::Invalid { .. } => "invalid",
            DbError::ReadOnly(_) => "read_only",
            DbError::Io(_) => "io",
            _ => "internal",
        };
        let message = match &e {
            DbError::Invalid { detail, .. } => detail.clone(),
            other => other.to_string(),
        };
        ApiError {
            code: code.into(),
            message,
        }
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError {
            code: "invalid".into(),
            message: e.to_string(),
        }
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

fn bad(message: impl Into<String>) -> ApiError {
    ApiError {
        code: "invalid".into(),
        message: message.into(),
    }
}

// ------------------------------------------------------------------- rows

/// Every write returns the file as it now is (contract `NoteWriteResult`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum NoteWriteResult {
    Saved {
        row: Box<NoteListRow>,
        text: String,
        content_hash: String,
        rev: i64,
        updated_at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        copy_id: Option<String>,
    },
    /// The file's hash is not `base_hash`: nothing was written (04 §11.4).
    Conflict {
        disk_text: String,
        disk_hash: String,
        disk_rev: i64,
    },
    Failed {
        message: String,
    },
}

fn write_result(notes: &Notes, out: Outcome) -> ApiResult<NoteWriteResult> {
    Ok(match out {
        Outcome::Saved {
            note,
            text,
            copy_id,
        } => {
            let id = note.id.clone();
            let row = notes
                .read(|cx| groupings::row_for(cx, &id))?
                .ok_or_else(|| bad("the note vanished"))?;
            NoteWriteResult::Saved {
                row: Box::new(row),
                content_hash: super::fsio::sha256_hex(text.as_bytes()),
                text,
                rev: note.rev,
                updated_at: note.updated_at.clone(),
                copy_id,
            }
        }
        Outcome::Conflict {
            disk_text,
            disk_hash,
            disk_rev,
        } => NoteWriteResult::Conflict {
            disk_text,
            disk_hash,
            disk_rev,
        },
    })
}

/// A failed write the editor shows as a banner (04 §9.3) instead of an error.
fn or_failed(r: ApiResult<NoteWriteResult>) -> ApiResult<NoteWriteResult> {
    match r {
        Err(e) if e.code == "io" || e.code == "read_only" => {
            Ok(NoteWriteResult::Failed { message: e.message })
        }
        other => other,
    }
}

// -------------------------------------------------------------- notes_list

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotesScope {
    Folder {
        folder_id: String,
        #[serde(default)]
        recursive: Option<bool>,
    },
    Grouping {
        slug: String,
    },
    Filter {
        filter: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NotesListArgs {
    pub scope: NotesScope,
    #[serde(default)]
    pub sort: Option<Vec<SortKey>>,
    #[serde(default)]
    pub show_archived: Option<bool>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotesListResult {
    pub rows: Vec<NoteListRow>,
    pub count: i64,
}

pub fn notes_list(notes: &Notes, args: NotesListArgs) -> ApiResult<NotesListResult> {
    let (rows, count) = notes.read(|cx| {
        let (filter, default_sort) = match &args.scope {
            NotesScope::Folder {
                folder_id,
                recursive,
            } => (
                Filter {
                    folder: Some(FolderFilter {
                        id: folder_id.clone(),
                        recursive: recursive.unwrap_or(folder_id != "root"),
                    }),
                    ..Default::default()
                },
                Vec::new(),
            ),
            NotesScope::Grouping { slug } => {
                let g = groupings::by_slug(cx, slug)?;
                groupings::parsed(&g).map_err(|e| super::ctx::invalid("filter_json", e))?
            }
            NotesScope::Filter { filter } => (
                groupings::validate_filter(filter)
                    .map_err(|e| super::ctx::invalid("filter_json", e))?,
                Vec::new(),
            ),
        };
        let opts = RunOpts {
            sort: args.sort.clone().unwrap_or(default_sort),
            limit: args.limit.unwrap_or(500),
            offset: args.offset.unwrap_or(0),
            include_archived: args.show_archived.unwrap_or(false),
            only_id: None,
        };
        groupings::run(cx, &filter, &opts)
    })?;
    Ok(NotesListResult { rows, count })
}

// ---------------------------------------------------------------- note_get

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteIdArgs {
    pub note_id: String,
}

/// The editor's note (reconciles the file first when it changed).
pub fn note_get(notes: &Notes, args: NoteIdArgs) -> ApiResult<NoteDetail> {
    let ro = notes.vault_status().read_only;
    let d = notes.write(move |cx| views::detail(cx, &args.note_id, ro.as_deref()))?;
    Ok(d)
}

/// The editor opened or closed a note (no touch-up while open, 04 §11.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteEditorArgs {
    pub note_id: String,
    pub open: bool,
}

pub fn note_editor(notes: &Notes, args: NoteEditorArgs) -> ApiResult<()> {
    notes.set_open(&args.note_id, args.open);
    Ok(())
}

// ------------------------------------------------------------- note_create

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteCreateArgs {
    /// `None` = the vault root.
    #[serde(default)]
    pub folder_id: Option<String>,
    pub text: String,
}

pub fn note_create(notes: &Notes, args: NoteCreateArgs) -> ApiResult<NoteDetail> {
    notes.ensure_writable()?;
    let ro = notes.vault_status().read_only;
    let d = notes.write(move |cx| {
        let n = write::create_note(cx, args.folder_id.as_deref(), &args.text)?;
        views::detail(cx, &n.id, ro.as_deref())
    })?;
    notes.set_open(&d.row.id, true);
    notes.changed();
    Ok(d)
}

// --------------------------------------------------------------- note_save

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteSaveArgs {
    pub note_id: String,
    /// The whole buffer, frontmatter included.
    pub text: String,
    pub base_hash: String,
    #[serde(default)]
    pub base_rev: Option<i64>,
    /// Only after a conflict: Keep mine / Keep theirs / Keep both.
    #[serde(default)]
    pub resolve: Option<Resolve>,
    /// Blur, close, switching notes, quit: always a history entry (04 §9.3).
    #[serde(default)]
    pub checkpoint: Option<bool>,
}

pub fn note_save(notes: &Notes, args: NoteSaveArgs) -> ApiResult<NoteWriteResult> {
    notes.ensure_writable()?;
    let record = args.resolve.is_some()
        || notes.take_revision_slot(&args.note_id, args.checkpoint.unwrap_or(false));
    let out = notes.write(move |cx| {
        write::save(
            cx,
            &args.note_id,
            &args.text,
            &args.base_hash,
            args.resolve,
            record,
        )
    });
    let r = or_failed(
        out.map_err(ApiError::from)
            .and_then(|o| write_result(notes, o)),
    );
    if matches!(r, Ok(NoteWriteResult::Saved { .. })) {
        notes.changed();
    }
    r
}

// ---------------------------------------------------------- note_set_field

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Buffer {
    pub text: String,
    pub base_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteSetFieldArgs {
    pub note_id: String,
    pub edit: FieldEdit,
    /// The editor's unsaved buffer: written first, in the same atomic write.
    #[serde(default)]
    pub buffer: Option<Buffer>,
}

pub fn note_set_field(notes: &Notes, args: NoteSetFieldArgs) -> ApiResult<NoteWriteResult> {
    notes.ensure_writable()?;
    let out = notes.write(move |cx| {
        let buf = args
            .buffer
            .as_ref()
            .map(|b| (b.text.as_str(), b.base_hash.as_str()));
        write::set_field(cx, &args.note_id, &args.edit, buf)
    });
    let r = or_failed(
        out.map_err(ApiError::from)
            .and_then(|o| write_result(notes, o)),
    );
    if matches!(r, Ok(NoteWriteResult::Saved { .. })) {
        notes.changed();
    }
    r
}

/// Note level: todo → doing → done → todo; another status → todo (03 §5.3).
pub fn note_cycle_status(notes: &Notes, args: NoteIdArgs) -> ApiResult<NoteWriteResult> {
    let status =
        notes.read(|cx| Ok(super::reconcile::live_note(cx.conn, &args.note_id)?.status))?;
    note_set_field(
        notes,
        NoteSetFieldArgs {
            note_id: args.note_id,
            edit: FieldEdit::Status(write::next_status(&status).into()),
            buffer: None,
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteArchiveArgs {
    pub note_id: String,
    pub archived: bool,
}

pub fn note_archive(notes: &Notes, args: NoteArchiveArgs) -> ApiResult<NoteWriteResult> {
    note_set_field(
        notes,
        NoteSetFieldArgs {
            note_id: args.note_id,
            edit: FieldEdit::Archived(args.archived),
            buffer: None,
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRenameArgs {
    pub note_id: String,
    pub title: String,
}

/// The editor's Rename (04 §2.4): the file name follows `title`.
pub fn note_rename(notes: &Notes, args: NoteRenameArgs) -> ApiResult<NoteWriteResult> {
    notes.ensure_writable()?;
    let out = notes.write(move |cx| write::rename(cx, &args.note_id, &args.title))?;
    notes.changed();
    write_result(notes, out)
}

// ------------------------------------------------------------ trash / undo

/// To the OS trash (04 §12); the row is tombstoned.
pub fn note_trash(notes: &Notes, args: NoteIdArgs) -> ApiResult<()> {
    notes.ensure_writable()?;
    let t = notes.write(move |cx| write::trash(cx, &args.note_id))?;
    notes.set_open(&t.note_id, false);
    notes.remember_trash(t);
    notes.changed();
    Ok(())
}

/// The Undo of a trash.
pub fn note_untrash(notes: &Notes, args: NoteIdArgs) -> ApiResult<NoteWriteResult> {
    notes.ensure_writable()?;
    let saved = notes.take_trash(&args.note_id);
    let out = notes.write(move |cx| write::untrash(cx, &args.note_id, saved.as_ref()))?;
    notes.changed();
    write_result(notes, out)
}

// ---------------------------------------------------------------- folders

pub fn folders_list(notes: &Notes) -> ApiResult<FoldersList> {
    Ok(notes.read(views::folders)?)
}

// --------------------------------------------------------------- groupings

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupingsList {
    pub groupings: Vec<GroupingRow>,
    /// `settings['notes.board_groupings']`, by slug.
    pub board: Vec<String>,
    pub statuses: Vec<StatusDef>,
    /// The device's local date (04 §5.3).
    pub today: String,
    pub week_start: WeekStart,
}

pub fn groupings_list(notes: &Notes) -> ApiResult<GroupingsList> {
    Ok(notes.read(|cx| {
        let rows = groupings::live(cx)?
            .iter()
            .map(|g| groupings::grouping_row(cx, g))
            .collect::<crate::db::Result<Vec<_>>>()?;
        Ok(GroupingsList {
            groupings: rows,
            board: crate::settings::get_as_in(cx.conn, "notes.board_groupings")?,
            statuses: cx.statuses().to_vec(),
            today: cx.clock.today.clone(),
            week_start: cx.week_start,
        })
    })?)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupingSaveArgs {
    pub grouping: GroupingDraft,
}

pub fn grouping_save(notes: &Notes, args: GroupingSaveArgs) -> ApiResult<GroupingRow> {
    let row = notes.write(move |cx| {
        let g = groupings::save(cx, &args.grouping)?;
        groupings::grouping_row(cx, &g)
    })?;
    notes.changed();
    Ok(row)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupingIdArgs {
    pub id: String,
}

/// Delete a custom grouping (premade ones are hidden instead).
pub fn grouping_delete(notes: &Notes, args: GroupingIdArgs) -> ApiResult<()> {
    notes.write(move |cx| groupings::delete(cx, &args.id))?;
    notes.changed();
    Ok(())
}

/// A board drop (04 §6.5): `apply = false` evaluates while the drag hovers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteDropArgs {
    pub note_id: String,
    /// The target grouping's slug.
    pub grouping: String,
    #[serde(default)]
    pub apply: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoteDropResult {
    pub plan: DropPlan,
    /// The write when `apply` performed the edit.
    pub result: Option<NoteWriteResult>,
}

pub fn note_drop(notes: &Notes, args: NoteDropArgs) -> ApiResult<NoteDropResult> {
    let (id, slug) = (args.note_id.clone(), args.grouping.clone());
    let plan = notes.read(move |cx| {
        let g = groupings::by_slug(cx, &slug)?;
        let (f, _) = groupings::parsed(&g).map_err(|e| super::ctx::invalid("filter_json", e))?;
        groupings::plan_drop(cx, &DropNote::of(cx, &id)?, &g.name, &f)
    })?;
    let result = match (&plan, args.apply) {
        (DropPlan::Edit { edit, .. }, true) => Some(note_set_field(
            notes,
            NoteSetFieldArgs {
                note_id: args.note_id,
                edit: edit.clone(),
                buffer: None,
            },
        )?),
        _ => None,
    };
    Ok(NoteDropResult { plan, result })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AddTaskArgs {
    pub filter: Value,
}

/// Whether ⊕ Add task shows for a filter, and the fields it assigns.
pub fn add_task_plan(notes: &Notes, args: AddTaskArgs) -> ApiResult<AddTaskPlan> {
    let f = groupings::validate_filter(&args.filter).map_err(bad)?;
    Ok(notes.read(|cx| groupings::add_task_plan(cx, &f))?)
}

// -------------------------------------------------------------------- tags

pub fn tags_list(notes: &Notes) -> ApiResult<Vec<TagRow>> {
    Ok(notes.read(views::tags)?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagRenameArgs {
    pub from: String,
    pub to: String,
}

/// Settings → Notes → Tags → Rename (merges onto an existing tag, 04 §5.4).
pub fn tag_rename(notes: &Notes, args: TagRenameArgs) -> ApiResult<BatchReport> {
    notes.ensure_writable()?;
    let r = notes.write(move |cx| write::tag_rename(cx, &args.from, &args.to))?;
    notes.changed();
    Ok(r)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagDeleteArgs {
    pub name: String,
}

pub fn tag_delete(notes: &Notes, args: TagDeleteArgs) -> ApiResult<BatchReport> {
    notes.ensure_writable()?;
    let r = notes.write(move |cx| write::tag_delete(cx, &args.name))?;
    notes.changed();
    Ok(r)
}

// ------------------------------------------------------------- query bar

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryContext {
    /// Chips already committed in this bar.
    #[serde(default)]
    pub chips: Vec<Chip>,
    /// `true` on Enter: resolve the whole text.
    #[serde(rename = "final", default)]
    pub final_: bool,
    /// `bar` | `refine` | `floating`.
    #[serde(default)]
    pub surface: Option<String>,
    /// The floating bar's default type.
    #[serde(default)]
    pub view_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResolveArgs {
    pub text: String,
    pub context: QueryContext,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResolveResult {
    pub chips: Vec<Chip>,
    pub rest: String,
    pub suggestions: Vec<Suggestion>,
    /// The card title for `chips` (04 §8.5).
    pub title: String,
}

pub fn query_resolve(notes: &Notes, args: QueryResolveArgs) -> ApiResult<QueryResolveResult> {
    let origin = if args.context.surface.as_deref() == Some("refine") {
        "refine"
    } else {
        "typed"
    };
    let mut bar = args.context.chips.clone();
    if let Some(t) = args.context.view_type.as_deref().filter(|t| *t != "notes")
        && !bar.iter().any(|c| c.kind() == "type")
    {
        bar.insert(0, query::type_chip(t));
    }
    let r = notes.read(|cx| query::resolve(cx, &args.text, &bar, args.context.final_, origin))?;
    Ok(QueryResolveResult {
        title: query::card_title(&r.chips),
        chips: r.chips,
        rest: r.rest,
        suggestions: r.suggestions,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryRunArgs {
    pub chips: Vec<Chip>,
    #[serde(default)]
    pub sort: Option<Vec<SortKey>>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryRunResult {
    pub rows: Vec<NoteListRow>,
    pub count: i64,
    /// The compiled filter with relative date tokens where chips were relative.
    pub filter: Value,
    /// The filter that ran (literal dates).
    pub resolved_filter: Value,
    #[serde(rename = "type")]
    pub item_type: String,
    pub title: String,
}

pub fn query_run(notes: &Notes, args: QueryRunArgs) -> ApiResult<QueryRunResult> {
    let r = notes
        .read(|cx| query::run(cx, &args.chips, args.sort.clone(), args.limit.unwrap_or(50)))?;
    Ok(QueryRunResult {
        rows: r.rows,
        count: r.count,
        filter: serde_json::to_value(&r.filter)?,
        resolved_filter: serde_json::to_value(&r.resolved_filter)?,
        item_type: r.item_type,
        title: r.title,
    })
}

// ----------------------------------------------------------------- history

pub fn note_history(notes: &Notes, args: NoteIdArgs) -> ApiResult<Vec<RevisionRow>> {
    Ok(notes.read(|cx| views::history(cx, &args.note_id))?)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRevArgs {
    pub note_id: String,
    pub rev: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionContent {
    pub rev: i64,
    pub content: String,
}

pub fn note_revision_get(notes: &Notes, args: NoteRevArgs) -> ApiResult<RevisionContent> {
    let content = notes.read(|cx| views::revision(cx, &args.note_id, args.rev))?;
    Ok(RevisionContent {
        rev: args.rev,
        content,
    })
}

/// Restore a revision as a new revision (`origin = 'restore'`).
pub fn note_restore(notes: &Notes, args: NoteRevArgs) -> ApiResult<NoteWriteResult> {
    notes.ensure_writable()?;
    let out = notes.write(move |cx| write::restore_revision(cx, &args.note_id, args.rev))?;
    notes.changed();
    write_result(notes, out)
}

// ------------------------------------------------------------------- links

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteLinkAddArgs {
    pub note_id: String,
    /// `note` | `message` | `thread` | `session` | `calendar_event` | `contact` | `url`.
    pub kind: String,
    /// A row id, or the URL itself when `kind = 'url'`.
    pub target_id: String,
    /// `created_from` | `references` | `prepare_for` | `related` | `attachment`.
    pub link_type: String,
    #[serde(default)]
    pub label: Option<String>,
}

pub fn note_link_add(notes: &Notes, args: NoteLinkAddArgs) -> ApiResult<NoteLink> {
    const KINDS: &[&str] = &[
        "note",
        "message",
        "thread",
        "session",
        "calendar_event",
        "contact",
        "url",
    ];
    const TYPES: &[&str] = &[
        "created_from",
        "references",
        "prepare_for",
        "related",
        "attachment",
    ];
    if !KINDS.contains(&args.kind.as_str()) {
        return Err(bad(format!("{:?} is not a link kind", args.kind)));
    }
    if !TYPES.contains(&args.link_type.as_str()) {
        return Err(bad(format!("{:?} is not a link type", args.link_type)));
    }
    let link = notes.write(move |cx| {
        super::reconcile::live_note(cx.conn, &args.note_id)?;
        let l = write::link(
            cx,
            ("note", &args.note_id),
            (&args.kind, &args.target_id),
            &args.link_type,
            args.label.as_deref(),
            "user",
        )?;
        let all = views::links(cx, &args.note_id)?;
        all.into_iter()
            .find(|x| x.id == l.id)
            .ok_or_else(|| super::ctx::invalid("link", "the link was not stored"))
    })?;
    notes.changed();
    Ok(link)
}

// ----------------------------------------------------------------- capture

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteCaptureArgs {
    pub text: String,
    #[serde(default)]
    pub created_from: Option<CreatedFrom>,
}

/// `note_capture` (contract `CaptureResult` plus the path, title and hints).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureResult {
    pub note_id: String,
    pub path: String,
    pub title: String,
    /// Refused tokens (`@2026-02-30`), shown under the capture box.
    pub hints: Vec<String>,
}

fn capture_result(c: capture::Captured) -> CaptureResult {
    CaptureResult {
        note_id: c.note_id,
        path: c.path,
        title: c.title,
        hints: c.hints,
    }
}

/// Capture box, hotkey and ⊕ Add task (04 §7.1 paths 1, 2, 8).
pub fn note_capture(notes: &Notes, args: NoteCaptureArgs) -> ApiResult<CaptureResult> {
    notes.ensure_writable()?;
    let c =
        notes.write(move |cx| capture::capture_text(cx, &args.text, args.created_from.as_ref()))?;
    notes.changed();
    Ok(capture_result(c))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureLinksArgs {
    /// URLs or file paths dropped on the overlay.
    pub items: Vec<String>,
}

/// Drag-to-overlay with URLs or files (04 §7.1 path 3).
pub fn note_capture_links(notes: &Notes, args: CaptureLinksArgs) -> ApiResult<CaptureResult> {
    notes.ensure_writable()?;
    let c = notes.write(move |cx| capture::capture_links(cx, &args.items))?;
    notes.changed();
    Ok(capture_result(c))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FromItemArgs {
    /// The session, message or calendar event id.
    pub id: String,
}

/// Note from session (04 §7.1 path 4).
pub fn note_from_session(notes: &Notes, args: FromItemArgs) -> ApiResult<CaptureResult> {
    notes.ensure_writable()?;
    let c = notes.write(move |cx| capture::note_from_session(cx, &args.id))?;
    notes.changed();
    Ok(capture_result(c))
}

/// Note from message (04 §7.1 path 5, v0.2).
pub fn note_from_message(notes: &Notes, args: FromItemArgs) -> ApiResult<CaptureResult> {
    notes.ensure_writable()?;
    let c = notes.write(move |cx| capture::note_from_message(cx, &args.id))?;
    notes.changed();
    Ok(capture_result(c))
}

/// Prepare notes for a calendar event (04 §7.1 path 6, v0.2).
pub fn note_prepare(notes: &Notes, args: FromItemArgs) -> ApiResult<CaptureResult> {
    notes.ensure_writable()?;
    let c = notes.write(move |cx| capture::prepare_notes(cx, &args.id))?;
    notes.changed();
    Ok(capture_result(c))
}

// ------------------------------------------------------------ diagnostics

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReindexResult {
    pub files: usize,
    pub changed: usize,
    pub tombstoned: usize,
    pub diagnostics: Vec<(String, String)>,
}

/// Diagnostics → Reindex: a full scan now.
pub fn notes_reindex(notes: &Notes) -> ApiResult<ReindexResult> {
    let r = notes.scan()?;
    Ok(ReindexResult {
        files: r.files,
        changed: r.changed.len(),
        tombstoned: r.tombstoned,
        diagnostics: notes.diagnostics(),
    })
}

// ------------------------------------------------------- import / export

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotesImportArgs {
    /// A folder of `.md` files (never modified).
    pub source: String,
    /// The vault folder to copy into; default a new folder named after the source.
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub include_attachments: bool,
    /// `false`: the preview counts only.
    #[serde(default)]
    pub apply: bool,
}

/// Settings → Notes → Import (04 §13.1, v1.0): preview, then apply.
pub fn notes_import(
    notes: &Notes,
    args: NotesImportArgs,
) -> ApiResult<super::import::ImportReport> {
    if args.apply {
        notes.ensure_writable()?;
    }
    let src = super::import::source(&args.source);
    let apply = args.apply;
    let r = notes.write(move |cx| {
        super::import::import(
            cx,
            &src,
            args.destination.as_deref(),
            args.include_attachments,
            apply,
        )
    })?;
    if apply {
        notes.changed();
    }
    Ok(r)
}

/// `notes-state.json` for the settings bundle (04 §13.2).
pub fn notes_state_export(notes: &Notes) -> ApiResult<Value> {
    Ok(notes.read(super::import::export_state)?)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotesStateArgs {
    pub state: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotesStateResult {
    pub notes: usize,
    pub links: usize,
}

pub fn notes_state_import(notes: &Notes, args: NotesStateArgs) -> ApiResult<NotesStateResult> {
    let (n, l) = notes.write(move |cx| super::import::import_state(cx, &args.state))?;
    notes.changed();
    Ok(NotesStateResult { notes: n, links: l })
}

// ---------------------------------------------------------------- statuses

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusesSaveArgs {
    pub statuses: Vec<StatusDef>,
}

/// Settings → Notes → statuses (04 §4.6): validated, then stored in
/// `settings['notes.statuses']`; refused while a removed status is used.
pub fn statuses_save(notes: &Notes, args: StatusesSaveArgs) -> ApiResult<Vec<StatusDef>> {
    let list = args.statuses;
    let saved = notes.write(move |cx| {
        let mut stmt = cx.conn.prepare(
            "SELECT status, COUNT(*) FROM notes WHERE deleted_at IS NULL GROUP BY status",
        )?;
        let in_use = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        super::fields::validate_statuses(&list, &in_use)
            .map_err(|e| super::ctx::invalid("statuses", e))?;
        crate::settings::set_in(
            cx.conn,
            cx.device(),
            "notes.statuses",
            &serde_json::to_value(&list)?,
        )?;
        Ok(list)
    })?;
    notes.changed();
    Ok(saved)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusMoveArgs {
    pub from: String,
    pub to: String,
}

/// "Move n notes to <status>" before deleting a custom status.
pub fn status_move(notes: &Notes, args: StatusMoveArgs) -> ApiResult<BatchReport> {
    notes.ensure_writable()?;
    let r = notes.write(move |cx| write::move_status(cx, &args.from, &args.to))?;
    notes.changed();
    Ok(r)
}
