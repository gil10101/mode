// The Notes view contract (03 §5, 04 §3–§12): commands the Notes view, the
// query bar and its cards invoke, and the rows they return.
//
// PROPOSED SHAPES. Written from the specs ahead of the Rust notes core (vault
// walker, frontmatter parser, groupings compiler, chip resolver) and to be
// reconciled with `mode_core::notes` when it lands. Field names are
// snake_case and mirror the `notes`, `folders`, `groupings`, `tags`,
// `note_revisions` and `links` columns of 09 §5.1 / §5.4 the way contract.ts
// mirrors `mode_core::queries`. `NoteListRow` is a superset of
// `mode_core::queries::NoteCard` (Q4) with `tags` as an array in place of
// the space-joined `tag_names` (a tag may contain a single space, 04 §5.4).
// `FilterJson` / `SortKey` are `mode_core::queries::filter::{Filter, SortKey}`.
//
// Every command is listed in `Commands` (contract.ts) under
// `// notes (package 7)`; the browser mock in `ui/src/notes/mock.ts` answers
// all of them. Args are snake_case objects, as for the session commands.
import type { CaptureResult } from './contract'

/* ---- vocabulary ---------------------------------------------------------- */

/** Derived at query time, never stored (04 §5.3). */
export type WhenBucket = 'overdue' | 'today' | 'tomorrow' | 'scheduled' | 'none'

export const WHEN_BUCKETS: readonly WhenBucket[] = ['overdue', 'today', 'tomorrow', 'scheduled', 'none']

/** One entry of `settings['notes.statuses']` (04 §4.6). */
export interface StatusDef {
  slug: string
  label: string
  glyph: string
  color: string | null
  closed: boolean
}

export const DEFAULT_STATUSES: readonly StatusDef[] = [
  { slug: 'todo', label: 'Todo', glyph: '○', color: null, closed: false },
  { slug: 'doing', label: 'Doing', glyph: '◑', color: null, closed: false },
  { slug: 'done', label: 'Done', glyph: '✓', color: null, closed: true },
]

export type SourceKind = 'manual' | 'capture' | 'message' | 'session' | 'calendar' | 'import' | 'chat'

export type WeekStart = 'monday' | 'sunday' | 'saturday'

/* ---- filter_json / sort_json (04 §6.1, §6.2) ----------------------------- */

export interface StatusFilter {
  in?: string[]
  /** May name `"closed"`: every closed status. */
  not_in?: string[]
}

/** `{in: buckets}` or `{from, to}` (literal dates or the relative tokens of 04 §6.4). */
export type WhenFilter = { in: WhenBucket[] } | { from?: string; to?: string }

export interface TagsFilter {
  all?: string[]
  any?: string[]
  none?: string[]
}

export interface FilterJson {
  status?: StatusFilter
  urgent?: boolean
  when?: WhenFilter
  tags?: TagsFilter
  /** `id` may be `"root"`; `recursive` defaults to true (04 §6.1). */
  folder?: { id: string; recursive?: boolean }
  text?: string
  person?: { contact_id: string }
  repo?: { repo_root: string }
  pinned?: true
  archived?: boolean
  source_kind?: { in: SourceKind[] }
}

export type SortBy = 'updated_at' | 'created_at' | 'when_date' | 'title' | 'urgent' | 'status'

export interface SortKey {
  by: SortBy
  dir: 'asc' | 'desc'
}

/* ---- rows ---------------------------------------------------------------- */

/** A note in a list, a board column or a card (Q4 + the row fields of 03 §5.3). */
export interface NoteListRow {
  id: string
  title: string
  status: string
  urgent: boolean
  when_date: string | null
  when_bucket: WhenBucket
  /** Vault-relative, with `.md`. */
  path: string
  folder_id: string | null
  excerpt: string
  task_total: number
  task_done: number
  pinned: boolean
  archived_at: string | null
  created_at: string
  updated_at: string
  source_kind: SourceKind | string
  /** Display names in frontmatter order. */
  tags: string[]
  has_links: boolean
  /** An active overlay snooze (04 §5.5): lists show a marker, nothing else changes. */
  snoozed?: boolean
}

export interface FolderRow {
  id: string
  parent_id: string | null
  /** Vault-relative, `/`-separated. */
  path: string
  name: string
  kind: 'normal' | 'capture' | 'templates'
  position: number
  /** Non-archived notes in this folder and below. */
  note_count: number
}

export interface FoldersList {
  /** The last segment of `notes.vault_path`: the root row's label. */
  vault_name: string
  vault_path: string
  /** Notes directly at the vault root (`folder_id IS NULL`). */
  root_count: number
  folders: FolderRow[]
}

export interface GroupingRow {
  id: string
  slug: string
  name: string
  kind: 'premade' | 'custom'
  filter_json: FilterJson
  sort_json: SortKey[]
  glyph: string | null
  color: string | null
  position: number
  show_in_overlay: boolean
  show_in_sidebar: boolean
  /** `COUNT(*)` of the compiled filter. */
  count: number
}

export interface GroupingsList {
  groupings: GroupingRow[]
  /** `settings['notes.board_groupings']`, by slug. */
  board: string[]
  statuses: StatusDef[]
  /** The device's local date, recomputed by the core at midnight (04 §5.3). */
  today: string
  week_start: WeekStart
}

export interface TagRow {
  id: string
  name: string
  name_norm: string
  color: string | null
  /** Non-archived notes carrying the tag. */
  count: number
}

export type LinkKind = 'note' | 'message' | 'thread' | 'session' | 'calendar_event' | 'contact' | 'url'
export type LinkType = 'created_from' | 'references' | 'prepare_for' | 'related' | 'attachment'

/** A `links` row seen from the note (either direction), with the other end's display title. */
export interface NoteLink {
  id: string
  link_type: LinkType
  direction: 'from' | 'to'
  kind: LinkKind
  target_id: string
  label: string | null
  title: string | null
  /** False once the other end was purged ("unavailable"). */
  available: boolean
}

/** `note_get`: the whole file plus the row (04 §9). */
export interface NoteDetail {
  row: NoteListRow
  /** The file as on disk, frontmatter included; the editor hides the frontmatter. */
  text: string
  /** `buffer_base_hash` of 04 §11.4. */
  content_hash: string
  rev: number
  /** `''` for the vault root. */
  folder_path: string
  read_only: boolean
  read_only_reason: string | null
  /** Frontmatter present but not valid YAML (04 §3.3). */
  frontmatter_error: boolean
  /** `status` is neither premade nor in `notes.statuses` (04 §3.1 assumption). */
  unknown_status: boolean
  links: NoteLink[]
}

/** A `note_revisions` row (S7) plus the device name for the History list. */
export interface RevisionRow {
  rev: number
  origin: 'mode' | 'external' | 'restore' | 'import'
  created_at: string
  device_id: string
  device_name?: string
  bytes: number
}

/* ---- command args and results -------------------------------------------- */

export type NotesScope =
  | { kind: 'folder'; folder_id: string; recursive?: boolean }
  | { kind: 'grouping'; slug: string }
  | { kind: 'filter'; filter: FilterJson }

export interface NotesListArgs {
  scope: NotesScope
  sort?: SortKey[]
  show_archived?: boolean
  limit?: number
  offset?: number
}

export interface NotesListResult {
  rows: NoteListRow[]
  count: number
}

/** The one-field writes of the header strip, the Fields panel, rows and board drops (04 §9.2, §6.5). */
export type NoteFieldEdit =
  | { field: 'status'; value: string }
  | { field: 'urgent'; value: boolean }
  /** `today` / `tomorrow` are resolved by the core; `null` removes the key. */
  | { field: 'when'; value: string | null }
  | { field: 'tags'; value: string[] }
  | { field: 'pinned'; value: boolean }
  | { field: 'folder_id'; value: string | null }

export type ConflictChoice = 'mine' | 'theirs' | 'both'

/**
 * Every write returns the file as it now is, so an open editor can take the
 * new frontmatter without touching the body. `conflict`: the file's hash is
 * not `base_hash` (04 §11.4); nothing was written.
 */
export type NoteWriteResult =
  | { outcome: 'saved'; row: NoteListRow; text: string; content_hash: string; rev: number; updated_at: string; copy_id?: string }
  | { outcome: 'conflict'; disk_text: string; disk_hash: string; disk_rev: number }
  | { outcome: 'failed'; message: string }

export interface NoteSaveArgs {
  note_id: string
  /** The whole buffer, frontmatter included (04 §9.3). */
  text: string
  base_hash: string
  base_rev: number
  /** Only after a conflict: Keep mine / Keep theirs / Keep both (04 §11.4). */
  resolve?: ConflictChoice
}

export interface NoteCreateArgs {
  /** `null` = the vault root. */
  folder_id: string | null
  text: string
}

export interface NoteSetFieldArgs {
  note_id: string
  edit: NoteFieldEdit
  /** The editor's unsaved buffer: written first, in the same atomic write (04 §9.2). */
  buffer?: { text: string; base_hash: string }
}

export interface QueryContextArgs {
  /** Chips already committed in this bar (a card's refine bar: its refine chips only). */
  chips: QueryChip[]
  /** `true` on Enter: resolve the whole text (04 §5.9.3); `false` per keystroke. */
  final: boolean
  surface: 'bar' | 'refine' | 'floating'
  /** The floating bar's default type (03 §5.9.1). */
  view_type?: ItemType
}

export interface QueryResolveArgs {
  text: string
  context: QueryContextArgs
}

export interface QueryResolveResult {
  /** The bar's whole chip list after this resolution (merged per 04 §8.3). */
  chips: QueryChip[]
  /** What stays typed in the field. */
  rest: string
  /** At most 6, ranked per 04 §8.7; the last is "Search for '…'" when text is pending. */
  suggestions: QuerySuggestion[]
}

export interface QueryRunArgs {
  chips: QueryChip[]
  sort?: SortKey[]
  limit?: number
}

export interface QueryRunResult {
  rows: NoteListRow[]
  count: number
  /** The compiled filter (04 §8.4), with relative date tokens where chips were relative. */
  filter: FilterJson
  type: ItemType
}

export interface GroupingDraft {
  /** Absent: create (custom). */
  id?: string
  name: string
  glyph?: string | null
  color?: string | null
  filter_json: FilterJson
  sort_json?: SortKey[]
  show_in_sidebar?: boolean
  show_in_overlay?: boolean
}

export interface NoteLinkAddArgs {
  note_id: string
  kind: LinkKind
  /** A row id, or the URL itself when `kind = 'url'`. */
  target_id: string
  link_type: LinkType
  label?: string
}

/* ---- query chips (04 §8.1) ----------------------------------------------- */

export type ChipKind = 'person' | 'type' | 'status' | 'when' | 'tag' | 'repo' | 'folder' | 'text' | 'flag'
export type ChipOrigin = 'typed' | 'autocomplete' | 'refine' | 'saved'
export type ItemType = 'notes' | 'tasks' | 'messages' | 'sessions' | 'calendar_events'

interface ChipBase {
  label: string
  glyph: string | null
  origin: ChipOrigin
}

export type QueryChip =
  | (ChipBase & { kind: 'person'; value: { contact_id: string }; initials: string })
  | (ChipBase & { kind: 'type'; value: ItemType })
  /** A status slug, or `urgent` / `not-urgent`. */
  | (ChipBase & { kind: 'status'; value: string })
  | (ChipBase & { kind: 'when'; value: WhenFilter })
  /** A tag `name_norm` (→ `tags.all`), or the `or` group (→ `tags.any`). */
  | (ChipBase & { kind: 'tag'; value: string | { any: string[] } })
  | (ChipBase & { kind: 'repo'; value: { repo_root: string } })
  | (ChipBase & { kind: 'folder'; value: { id: string } })
  | (ChipBase & { kind: 'text'; value: string })
  | (ChipBase & { kind: 'flag'; value: 'pinned' | 'archived' | `source:${SourceKind}` })

export interface QuerySuggestion {
  id: string
  label: string
  glyph: string | null
  /** The kind, or a count ("2 notes"), as a muted hint. */
  hint: string
  /** Ghost text after the caret when the label completes the typed prefix. */
  completion: string | null
  /** The bar after accepting: its chips and what stays typed. */
  apply: { chips: QueryChip[]; rest: string }
}

/* ---- the command map ----------------------------------------------------- */

/** The notes commands as `Commands` entries (spread into contract.ts's `Commands`). */
export interface NotesCommands {
  notes_list: { args: NotesListArgs; result: NotesListResult }
  note_get: { args: { note_id: string }; result: NoteDetail }
  /** New note (04 §7.1 path 7): called on the first saved keystroke, never on "New note". */
  note_create: { args: NoteCreateArgs; result: NoteDetail }
  note_save: { args: NoteSaveArgs; result: NoteWriteResult }
  note_set_field: { args: NoteSetFieldArgs; result: NoteWriteResult }
  /** Note level: todo → doing → done → todo (03 §5.3); a custom status goes to todo. */
  note_cycle_status: { args: { note_id: string }; result: NoteWriteResult }
  note_archive: { args: { note_id: string; archived: boolean }; result: NoteWriteResult }
  /** To the OS trash (04 §12). */
  note_trash: { args: { note_id: string }; result: void }
  /** The 8 s Undo of a trash. */
  note_untrash: { args: { note_id: string }; result: NoteWriteResult }
  folders_list: { args: undefined; result: FoldersList }
  groupings_list: { args: undefined; result: GroupingsList }
  grouping_save: { args: { grouping: GroupingDraft }; result: GroupingRow }
  tags_list: { args: undefined; result: TagRow[] }
  query_resolve: { args: QueryResolveArgs; result: QueryResolveResult }
  query_run: { args: QueryRunArgs; result: QueryRunResult }
  note_history: { args: { note_id: string }; result: RevisionRow[] }
  note_revision_get: { args: { note_id: string; rev: number }; result: { rev: number; content: string } }
  /** Writes that revision's content as a new revision, `origin = 'restore'` (03 §5.8). */
  note_restore: { args: { note_id: string; rev: number }; result: NoteWriteResult }
  note_link_add: { args: NoteLinkAddArgs; result: NoteLink }
}

/** `note_capture` (contract.ts) is reused for ⊕ Add task: the capture token grammar of 04 §7.2 carries the fields. */
export type AddTaskResult = CaptureResult
