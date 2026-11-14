// `ui/state` for the Notes view (03 §5, 04 §9, §11): what is selected (folder,
// grouping, an expanded card's chips, the note), the list or board rows, the
// editor buffer with its save pipeline and conflict state, the query cards
// of every view, and the inline notice. Fed by `notes.changed`: every push
// re-reads what is on screen (the push says that something changed, not
// what), and the open note is compared against its `buffer_base_hash`.
import { create } from 'zustand'
import { EVENTS, type CaptureResult, type HealthChanged } from './contract'
import type {
  ConflictChoice,
  FilterJson,
  FolderRow,
  GroupingRow,
  ItemType,
  NoteDetail,
  NoteFieldEdit,
  NoteListRow,
  NoteWriteResult,
  NotesScope,
  QueryChip,
  RevisionRow,
  SortBy,
  SortKey,
  StatusDef,
  TagRow,
  WeekStart,
} from './contract-notes'
import { DEFAULT_STATUSES } from './contract-notes'
import { call } from '../notes/api'
import { localToday } from '../notes/dates'
import { rebaseOnto, withTitle } from '../notes/editor/markdown'
import { addTaskPlan, captureText, describeEdit, nextStatus, planDrop, sameChips, type DropNote } from '../notes/rules'
import { listen, type Unlisten } from '../tauri'

/* ---- shapes ---------------------------------------------------------------- */

export type Selection = { kind: 'folder'; id: string } | { kind: 'grouping'; slug: string } | { kind: 'query'; chips: QueryChip[] }

export type Layout = 'list' | 'board'

export type SaveState = 'clean' | 'dirty' | 'saving' | 'saved' | 'failed'

export interface EditorState {
  /** `null` with a draft: a New note not yet written (04 §7.1 path 7). */
  noteId: string | null
  draft: { folder_id: string | null } | null
  detail: NoteDetail | null
  /** The live buffer, frontmatter included. */
  text: string
  /** Bumped when the store replaces the buffer (reload, field write); the editor then takes `text`. */
  version: number
  baseHash: string
  baseRev: number
  save: SaveState
  error: string | null
  conflict: { disk_text: string; disk_hash: string; disk_rev: number } | null
  /** Raw source instead of live preview (04 §9.1). */
  raw: boolean
  /** The file was deleted outside Mode (04 §11.3). */
  gone: boolean
}

export interface BoardColumn {
  slug: string
  rows: NoteListRow[]
  count: number
}

export type GroupBy = 'none' | 'status' | 'when' | 'tag' | 'folder'

export interface CardState {
  id: string
  /** The view whose card layer shows it (03 §5.9.6). */
  view: string
  chips: QueryChip[]
  /** Chips typed into the card's own refine bar, AND-ed into this card only. */
  refine: QueryChip[]
  status: 'loading' | 'ready' | 'error'
  rows: NoteListRow[]
  count: number
  filter: FilterJson | null
  type: ItemType
  sort: SortBy
  groupBy: GroupBy
  /** From the content region's left / bottom until dragged, then left / top. */
  pos: { x: number; y: number; anchor: 'bottom' | 'top' }
  width: number
  z: number
  grouping: Pick<GroupingRow, 'id' | 'slug' | 'name' | 'glyph' | 'show_in_overlay'> | null
  error: string | null
}

export interface Notice {
  id: number
  text: string
  action?: { label: string; run: () => void }
}

export interface NotesData {
  ready: boolean
  today: string
  weekStart: WeekStart
  statuses: StatusDef[]
  vaultName: string
  rootCount: number
  folders: FolderRow[]
  groupings: GroupingRow[]
  board: string[]
  tags: TagRow[]
  expanded: string[]
  selection: Selection
  layout: Layout
  /** Per folder / grouping key (`settings['notes.sort']`, device). */
  sorts: Record<string, SortBy>
  showArchived: boolean
  rows: NoteListRow[]
  count: number
  listLoading: boolean
  columns: BoardColumn[]
  selectedId: string | null
  editor: EditorState
  detailOpen: boolean
  history: RevisionRow[]
  /** The docked bar's committed chips. */
  barChips: QueryChip[]
  cards: CardState[]
  notice: Notice | null
  health: Pick<HealthChanged, 'vault_missing' | 'read_only'> | null
  error: string | null
}

export interface NotesActions {
  /** Subscribe to `notes.changed` / `health.changed` (idempotent) and load. */
  start(): Promise<void>
  stop(): void
  load(): Promise<void>
  refresh(): Promise<void>
  loadList(): Promise<void>
  selectFolder(id: string): Promise<void>
  selectGrouping(slug: string): Promise<void>
  /** ↗ on a card: the same chips in the view header (03 §5.9.5). */
  showQuery(chips: QueryChip[]): Promise<void>
  toggleFolder(id: string): void
  setLayout(layout: Layout): Promise<void>
  setSort(by: SortBy): Promise<void>
  setShowArchived(on: boolean): Promise<void>
  selectNote(id: string | null): Promise<void>
  newNote(): Promise<void>
  editorInput(text: string): void
  /** The head's title field: rewrites the first H1 of the buffer (03 §5.4; the file is not renamed). */
  setTitle(title: string): void
  saveNow(): Promise<void>
  /** Retry after a failed write (or a file deleted outside Mode): the buffer is written again. */
  retrySave(): Promise<void>
  resolveConflict(choice: ConflictChoice): Promise<void>
  setRaw(raw: boolean): void
  setField(noteId: string, edit: NoteFieldEdit, opts?: { undo?: boolean; quiet?: boolean }): Promise<boolean>
  cycleStatus(noteId: string): Promise<void>
  archive(noteId: string, archived: boolean): Promise<void>
  trash(noteId: string): Promise<void>
  dropOnGrouping(noteId: string, slug: string): Promise<'edit' | 'noop' | 'refuse'>
  addTask(title: string, target: { filter: FilterJson }): Promise<string | null>
  setDetailOpen(open: boolean): void
  loadHistory(): Promise<void>
  restore(rev: number): Promise<void>
  addUrl(url: string): Promise<void>
  setBarChips(chips: QueryChip[]): void
  /** Clicking a tag chip anywhere (03 §5.6): the chip into the bar and a card. */
  tagToBar(tag: string): Promise<void>
  spawnCard(chips: QueryChip[], opts?: { view?: string; force?: boolean }): Promise<string | null>
  runCard(id: string): Promise<void>
  closeCard(id: string): void
  raiseCard(id: string): void
  moveCard(id: string, pos: CardState['pos']): void
  resizeCard(id: string, width: number): void
  setCardRefine(id: string, chips: QueryChip[]): Promise<void>
  setCardSort(id: string, by: SortBy): Promise<void>
  setCardGroupBy(id: string, groupBy: GroupBy): void
  duplicateCard(id: string): Promise<void>
  closeOtherCards(id: string): void
  saveCardAsGrouping(id: string, name: string, glyph?: string): Promise<string | null>
  toggleCardOverlay(id: string): Promise<void>
  showNotice(text: string, action?: Notice['action']): void
  dismissNotice(): void
}

export type NotesState = NotesData & NotesActions

/* ---- constants ------------------------------------------------------------- */

/** `notes.autosave_delay_ms` (04 §9.3). */
export const AUTOSAVE_MS = 500
/** Inline notices stay 8 s (10 §12.3). */
export const NOTICE_MS = 8000
/** 03 §5.9.6. */
export const MAX_CARDS = 8
export const CARD_WIDTH = 360
export const CARD_PAGE = 50

const SORT_KEYS: Record<SortBy, SortKey[]> = {
  updated_at: [{ by: 'updated_at', dir: 'desc' }],
  created_at: [{ by: 'created_at', dir: 'desc' }],
  when_date: [
    { by: 'when_date', dir: 'asc' },
    { by: 'updated_at', dir: 'desc' },
  ],
  title: [{ by: 'title', dir: 'asc' }],
  urgent: [
    { by: 'urgent', dir: 'desc' },
    { by: 'updated_at', dir: 'desc' },
  ],
  status: [
    { by: 'status', dir: 'asc' },
    { by: 'updated_at', dir: 'desc' },
  ],
}

export function sortKeys(by: SortBy): SortKey[] {
  return SORT_KEYS[by]
}

const CARD_DEFAULT_SORT: SortKey[] = [
  { by: 'urgent', dir: 'desc' },
  { by: 'when_date', dir: 'asc' },
  { by: 'updated_at', dir: 'desc' },
]

const emptyEditor = (): EditorState => ({
  noteId: null,
  draft: null,
  detail: null,
  text: '',
  version: 0,
  baseHash: '',
  baseRev: 0,
  save: 'clean',
  error: null,
  conflict: null,
  raw: false,
  gone: false,
})

export function selectionKey(s: Selection): string {
  return s.kind === 'folder' ? `folder:${s.id}` : s.kind === 'grouping' ? `grouping:${s.slug}` : 'query'
}

export function dropNoteOf(r: NoteListRow): DropNote {
  return {
    status: r.status,
    urgent: r.urgent,
    when_bucket: r.when_bucket,
    when_date: r.when_date,
    tags: r.tags,
    folder_id: r.folder_id,
    pinned: r.pinned,
    archived_at: r.archived_at,
    source_kind: r.source_kind,
  }
}

/* ---- the store ------------------------------------------------------------- */

const initial = (): NotesData => ({
  ready: false,
  today: localToday(),
  weekStart: 'monday',
  statuses: DEFAULT_STATUSES.map((s) => ({ ...s })),
  vaultName: 'Notes',
  rootCount: 0,
  folders: [],
  groupings: [],
  board: ['todo', 'doing', 'urgent', 'done'],
  tags: [],
  expanded: [],
  selection: { kind: 'grouping', slug: 'today' },
  layout: 'list',
  sorts: {},
  showArchived: false,
  rows: [],
  count: 0,
  listLoading: false,
  columns: [],
  selectedId: null,
  editor: emptyEditor(),
  detailOpen: false,
  history: [],
  barChips: [],
  cards: [],
  notice: null,
  health: null,
  error: null,
})

let saveTimer: ReturnType<typeof setTimeout> | undefined
let refreshTimer: ReturnType<typeof setTimeout> | undefined
let noticeTimer: ReturnType<typeof setTimeout> | undefined
let unlisten: Unlisten[] = []
let started = false
let listSeq = 0
let cardSeq = 0
let noticeSeq = 0
let topZ = 1
/** The save in flight: later saves and field writes wait for it (04 §9.3). */
let inFlight: Promise<void> | null = null
/** The first load, shared by every `start()` (StrictMode runs effects twice). */
let loading: Promise<void> | null = null

/** Commands reject with a plain `{ code, message }` object (mode-app `cmd_error`), not an `Error`. */
function message(e: unknown): string {
  if (e instanceof Error) return e.message
  if (e && typeof e === 'object' && typeof (e as { message?: unknown }).message === 'string') return (e as { message: string }).message
  return String(e)
}

export const useNotesStore = create<NotesState>((set, get) => {
  /** Applies a write result to every list, the board, the cards and the open note. */
  function applyRow(row: NoteListRow) {
    const patch = (rows: NoteListRow[]) => rows.map((r) => (r.id === row.id ? row : r))
    set((s) => ({
      rows: patch(s.rows),
      columns: s.columns.map((c) => ({ ...c, rows: patch(c.rows) })),
      cards: s.cards.map((c) => ({ ...c, rows: patch(c.rows) })),
    }))
  }

  /**
   * A write the core made to the open note: lay its frontmatter onto the live
   * buffer (the body of `sent`, or of the last known file, is what the core
   * wrote; anything typed since stays and is saved next).
   */
  function applyToEditor(res: Extract<NoteWriteResult, { outcome: 'saved' }>, sent: string | null) {
    const ed = get().editor
    if (ed.noteId !== res.row.id || !ed.detail) return
    const detail = { ...ed.detail, row: res.row, text: res.text, content_hash: res.content_hash, rev: res.rev }
    if (ed.conflict) {
      set({ editor: { ...ed, detail } })
      return
    }
    const text = rebaseOnto(ed.text, sent ?? ed.detail.text, res.text)
    const dirty = text !== res.text
    set({
      editor: {
        ...ed,
        text,
        version: ed.version + 1,
        baseHash: res.content_hash,
        baseRev: res.rev,
        detail,
        save: dirty ? 'dirty' : sent !== null ? 'saved' : ed.save === 'dirty' ? 'saved' : ed.save,
      },
    })
    if (dirty) schedule()
  }

  function schedule() {
    clearTimeout(saveTimer)
    saveTimer = setTimeout(() => void get().saveNow(), AUTOSAVE_MS)
  }

  function lookupRow(id: string): NoteListRow | undefined {
    const s = get()
    if (s.editor.detail?.row.id === id) return s.editor.detail.row
    return s.rows.find((r) => r.id === id) ?? s.columns.flatMap((c) => c.rows).find((r) => r.id === id) ?? s.cards.flatMap((c) => c.rows).find((r) => r.id === id)
  }

  async function checkOpenNote() {
    const ed = get().editor
    if (!ed.noteId || ed.conflict || ed.save === 'saving' || inFlight) return
    let d: NoteDetail
    try {
      d = await call('note_get', { note_id: ed.noteId })
    } catch {
      // Gone from disk (04 §11.3): keep the buffer, offer Save again / Close.
      if (get().editor.noteId === ed.noteId) set({ editor: { ...get().editor, gone: true } })
      return
    }
    const now = get().editor
    if (now.noteId !== d.row.id || now.save === 'saving' || inFlight) return
    if (d.content_hash === now.baseHash) {
      set({ editor: { ...now, detail: d, gone: false } })
      return
    }
    if (now.save === 'dirty') {
      set({ editor: { ...now, conflict: { disk_text: d.text, disk_hash: d.content_hash, disk_rev: d.rev }, detail: d } })
    } else {
      // A clean buffer reloads silently; the caret stays (the editor maps it).
      set({ editor: { ...now, detail: d, text: d.text, version: now.version + 1, baseHash: d.content_hash, baseRev: d.rev, gone: false } })
    }
  }

  async function runCardQuery(card: CardState): Promise<Partial<CardState>> {
    const chips = [...card.chips, ...card.refine]
    if (!chips.length) return { status: 'ready', rows: [], count: 0, filter: {}, error: null }
    try {
      const res = await call('query_run', { chips, sort: card.sort === 'urgent' ? CARD_DEFAULT_SORT : sortKeys(card.sort), limit: CARD_PAGE })
      return { status: 'ready', rows: res.rows, count: res.count, filter: res.filter, type: res.type, error: null }
    } catch (e) {
      return { status: 'error', error: message(e) }
    }
  }

  function patchCard(id: string, patch: Partial<CardState>) {
    set((s) => ({ cards: s.cards.map((c) => (c.id === id ? { ...c, ...patch } : c)) }))
  }

  async function write(noteId: string, p: Promise<NoteWriteResult>): Promise<NoteWriteResult | null> {
    try {
      const res = await p
      if (res.outcome === 'saved') {
        applyRow(res.row)
        applyToEditor(res, null)
      } else if (res.outcome === 'failed') get().showNotice(res.message)
      else if (get().editor.noteId === noteId) set({ editor: { ...get().editor, conflict: { disk_text: res.disk_text, disk_hash: res.disk_hash, disk_rev: res.disk_rev } } })
      return res
    } catch (e) {
      get().showNotice(message(e))
      return null
    }
  }

  return {
    ...initial(),

    async start() {
      if (!started) {
        started = true
        const subs = await Promise.all([
          listen(EVENTS.notes, () => {
            clearTimeout(refreshTimer)
            refreshTimer = setTimeout(() => void get().refresh(), 30)
          }),
          listen<HealthChanged>(EVENTS.health, (h) => set({ health: { vault_missing: h.vault_missing, read_only: h.read_only } })),
        ])
        unlisten.push(...subs)
      }
      if (!get().ready) await (loading ??= get().load().finally(() => (loading = null)))
    },

    stop() {
      for (const u of unlisten) u()
      unlisten = []
      started = false
      clearTimeout(saveTimer)
      clearTimeout(refreshTimer)
      clearTimeout(noticeTimer)
    },

    async load() {
      try {
        const [g, f, tags] = await Promise.all([call('groupings_list'), call('folders_list'), call('tags_list')])
        set({
          ready: true,
          error: null,
          today: g.today,
          weekStart: g.week_start,
          statuses: g.statuses,
          groupings: g.groupings,
          board: g.board,
          folders: f.folders,
          vaultName: f.vault_name,
          rootCount: f.root_count,
          tags,
        })
      } catch (e) {
        set({ error: message(e), ready: true })
        return
      }
      await get().loadList()
    },

    async refresh() {
      try {
        const [g, f, tags] = await Promise.all([call('groupings_list'), call('folders_list'), call('tags_list')])
        set({ today: g.today, weekStart: g.week_start, statuses: g.statuses, groupings: g.groupings, board: g.board, folders: f.folders, rootCount: f.root_count, vaultName: f.vault_name, tags })
      } catch (e) {
        set({ error: message(e) })
      }
      await get().loadList()
      const cards = get().cards
      const results = new Map(await Promise.all(cards.map(async (c) => [c.id, await runCardQuery(c)] as const)))
      set((s) => ({ cards: s.cards.map((c) => ({ ...c, ...results.get(c.id) })) }))
      await checkOpenNote()
      if (get().detailOpen && get().editor.noteId) await get().loadHistory()
    },

    async loadList() {
      const s = get()
      const seq = ++listSeq
      set({ listLoading: true })
      try {
        if (s.layout === 'board') {
          const cols = await Promise.all(
            s.board
              .filter((slug) => s.groupings.some((g) => g.slug === slug))
              .map(async (slug) => {
                const r = await call('notes_list', { scope: { kind: 'grouping', slug }, limit: 50 })
                return { slug, rows: r.rows, count: r.count }
              }),
          )
          if (seq === listSeq) set({ columns: cols, listLoading: false })
          return
        }
        let scope: NotesScope
        if (s.selection.kind === 'folder') scope = { kind: 'folder', folder_id: s.selection.id }
        else if (s.selection.kind === 'grouping') scope = { kind: 'grouping', slug: s.selection.slug }
        else {
          const res = await call('query_run', { chips: s.selection.chips, sort: s.sorts.query ? sortKeys(s.sorts.query) : undefined, limit: 500 })
          if (seq === listSeq) set({ rows: res.rows, count: res.count, listLoading: false })
          return
        }
        const by = s.sorts[selectionKey(s.selection)]
        const res = await call('notes_list', { scope, sort: by ? sortKeys(by) : undefined, show_archived: s.showArchived })
        if (seq === listSeq) set({ rows: res.rows, count: res.count, listLoading: false })
      } catch (e) {
        if (seq === listSeq) set({ listLoading: false, error: message(e) })
      }
    },

    async selectFolder(id) {
      set((s) => ({ selection: { kind: 'folder', id }, layout: 'list', expanded: s.expanded.includes(id) ? s.expanded : [...s.expanded, id] }))
      await get().loadList()
    },

    async selectGrouping(slug) {
      set({ selection: { kind: 'grouping', slug } })
      await get().loadList()
    },

    async showQuery(chips) {
      set({ selection: { kind: 'query', chips }, layout: 'list' })
      await get().loadList()
    },

    toggleFolder(id) {
      set((s) => ({ expanded: s.expanded.includes(id) ? s.expanded.filter((x) => x !== id) : [...s.expanded, id] }))
    },

    async setLayout(layout) {
      set({ layout })
      await get().loadList()
    },

    async setSort(by) {
      set((s) => ({ sorts: { ...s.sorts, [selectionKey(s.selection)]: by } }))
      await get().loadList()
    },

    async setShowArchived(on) {
      set({ showArchived: on })
      await get().loadList()
    },

    async selectNote(id) {
      const cur = get().editor
      if (cur.noteId === id && id !== null) {
        set({ selectedId: id })
        return
      }
      await get().saveNow()
      if (id === null) {
        set({ selectedId: null, editor: emptyEditor(), history: [] })
        return
      }
      set({ selectedId: id })
      try {
        const d = await call('note_get', { note_id: id })
        if (get().selectedId !== id) return
        set((s) => ({
          editor: { ...emptyEditor(), noteId: id, detail: d, text: d.text, version: s.editor.version + 1, baseHash: d.content_hash, baseRev: d.rev, raw: false },
          history: [],
        }))
        if (get().detailOpen) await get().loadHistory()
      } catch (e) {
        get().showNotice(message(e))
      }
    },

    async newNote() {
      await get().saveNow()
      const s = get()
      const capture = s.folders.find((f) => f.kind === 'capture')
      const folder_id = s.selection.kind === 'folder' ? (s.selection.id === 'root' ? null : s.selection.id) : capture?.id ?? null
      set((st) => ({ selectedId: null, history: [], editor: { ...emptyEditor(), draft: { folder_id }, text: '', version: st.editor.version + 1 } }))
    },

    editorInput(text) {
      const ed = get().editor
      if (text === ed.text) return
      if (!ed.noteId && !ed.draft) return
      set({ editor: { ...ed, text, save: ed.conflict ? ed.save : 'dirty' } })
      if (!ed.conflict) schedule()
    },

    setTitle(title) {
      const ed = get().editor
      if ((!ed.noteId && !ed.draft) || ed.detail?.read_only) return
      const text = withTitle(ed.text, title)
      if (text === ed.text) return
      set({ editor: { ...ed, text, version: ed.version + 1, save: ed.conflict ? ed.save : 'dirty' } })
      if (!ed.conflict) schedule()
    },

    async saveNow() {
      clearTimeout(saveTimer)
      if (inFlight) await inFlight
      const ed = get().editor
      if (ed.conflict || ed.save !== 'dirty' || ed.detail?.read_only) return
      const sent = ed.text
      set({ editor: { ...ed, save: 'saving' } })
      const run = async () => {
        try {
          if (ed.draft && !ed.noteId) {
            const d = await call('note_create', { folder_id: ed.draft.folder_id, text: sent })
            const now = get().editor
            const text = rebaseOnto(now.text, sent, d.text)
            set({
              selectedId: d.row.id,
              editor: { ...now, noteId: d.row.id, draft: null, detail: d, text, version: now.version + 1, baseHash: d.content_hash, baseRev: d.rev, save: text === d.text ? 'saved' : 'dirty', error: null },
            })
            if (text !== d.text) schedule()
            return
          }
          const res = await call('note_save', { note_id: ed.noteId!, text: sent, base_hash: ed.baseHash, base_rev: ed.baseRev })
          const now = get().editor
          if (now.noteId !== ed.noteId) return
          if (res.outcome === 'saved') {
            applyRow(res.row)
            const text = rebaseOnto(now.text, sent, res.text)
            set({
              editor: {
                ...now,
                text,
                version: now.version + 1,
                baseHash: res.content_hash,
                baseRev: res.rev,
                detail: now.detail ? { ...now.detail, row: res.row, text: res.text, content_hash: res.content_hash, rev: res.rev } : now.detail,
                save: text === res.text ? 'saved' : 'dirty',
                error: null,
              },
            })
            if (text !== res.text) schedule()
          } else if (res.outcome === 'conflict') {
            set({ editor: { ...now, save: 'dirty', conflict: { disk_text: res.disk_text, disk_hash: res.disk_hash, disk_rev: res.disk_rev } } })
          } else {
            set({ editor: { ...now, save: 'failed', error: res.message } })
          }
        } catch (e) {
          set({ editor: { ...get().editor, save: 'failed', error: message(e) } })
        }
      }
      inFlight = run()
      try {
        await inFlight
      } finally {
        inFlight = null
      }
    },

    async retrySave() {
      const ed = get().editor
      if (!ed.noteId && !ed.draft) return
      set({ editor: { ...ed, save: 'dirty', error: null, gone: false } })
      await get().saveNow()
    },

    async resolveConflict(choice) {
      const ed = get().editor
      if (!ed.noteId || !ed.conflict) return
      const sent = ed.text
      try {
        const res = await call('note_save', { note_id: ed.noteId, text: sent, base_hash: ed.conflict.disk_hash, base_rev: ed.conflict.disk_rev, resolve: choice })
        if (res.outcome !== 'saved') {
          get().showNotice(res.outcome === 'failed' ? res.message : 'This note changed again on disk.')
          return
        }
        applyRow(res.row)
        const now = get().editor
        const text = choice === 'mine' ? rebaseOnto(now.text, sent, res.text) : res.text
        set({
          editor: {
            ...now,
            conflict: null,
            text,
            version: now.version + 1,
            baseHash: res.content_hash,
            baseRev: res.rev,
            detail: now.detail ? { ...now.detail, row: res.row, text: res.text, content_hash: res.content_hash, rev: res.rev } : now.detail,
            save: text === res.text ? 'saved' : 'dirty',
          },
        })
        if (choice === 'both') get().showNotice('Your version was kept as a copy')
        if (text !== res.text) schedule()
      } catch (e) {
        get().showNotice(message(e))
      }
    },

    setRaw(raw) {
      set((s) => ({ editor: { ...s.editor, raw } }))
    },

    async setField(noteId, edit, opts = {}) {
      const before = lookupRow(noteId)
      const ed = get().editor
      // The open note's unsaved body goes first, in the same write (04 §9.2).
      const buffer = ed.noteId === noteId && ed.save === 'dirty' && !ed.conflict ? { text: ed.text, base_hash: ed.baseHash } : undefined
      if (buffer) {
        clearTimeout(saveTimer)
        if (inFlight) await inFlight
      }
      let res: NoteWriteResult
      try {
        res = await call('note_set_field', { note_id: noteId, edit, ...(buffer ? { buffer } : {}) })
      } catch (e) {
        get().showNotice(message(e))
        return false
      }
      if (res.outcome !== 'saved') {
        if (res.outcome === 'conflict' && get().editor.noteId === noteId) set({ editor: { ...get().editor, conflict: { disk_text: res.disk_text, disk_hash: res.disk_hash, disk_rev: res.disk_rev } } })
        else get().showNotice(res.outcome === 'failed' ? res.message : 'This note changed on disk.')
        return false
      }
      applyRow(res.row)
      applyToEditor(res, buffer ? buffer.text : null)
      if (!opts.quiet && before) {
        const undo = inverseEdit(before, edit)
        get().showNotice(describeEdit(edit, get().statuses), opts.undo === false || !undo ? undefined : { label: 'Undo', run: () => void get().setField(noteId, undo, { quiet: true }).then((ok) => ok && get().showNotice('Undone')) })
      }
      return true
    },

    async cycleStatus(noteId) {
      const row = lookupRow(noteId)
      if (!row) {
        await write(noteId, call('note_cycle_status', { note_id: noteId }))
        return
      }
      await get().setField(noteId, { field: 'status', value: nextStatus(row.status) })
    },

    async archive(noteId, archived) {
      const res = await write(noteId, call('note_archive', { note_id: noteId, archived }))
      if (res?.outcome === 'saved') get().showNotice(archived ? 'Archived' : 'Unarchived', { label: 'Undo', run: () => void get().archive(noteId, !archived) })
    },

    async trash(noteId) {
      try {
        await call('note_trash', { note_id: noteId })
      } catch (e) {
        get().showNotice(message(e))
        return
      }
      if (get().editor.noteId === noteId) set({ editor: emptyEditor(), selectedId: null })
      set((s) => ({ rows: s.rows.filter((r) => r.id !== noteId) }))
      get().showNotice('Deleted', {
        label: 'Undo',
        run: () =>
          void call('note_untrash', { note_id: noteId })
            .then(() => get().selectNote(noteId))
            .catch((e: unknown) => get().showNotice(message(e))),
      })
    },

    async dropOnGrouping(noteId, slug) {
      const s = get()
      const g = s.groupings.find((x) => x.slug === slug)
      const row = lookupRow(noteId)
      if (!g || !row) return 'refuse'
      const column = s.columns.find((c) => c.slug === slug)
      const plan = planDrop(dropNoteOf(row), { name: g.name, filter: g.filter_json }, { statuses: s.statuses, today: s.today, weekStart: s.weekStart, folders: s.folders, alreadyIn: column?.rows.some((r) => r.id === noteId) })
      if (plan.kind !== 'edit') {
        get().showNotice(plan.hint)
        return plan.kind
      }
      const ok = await get().setField(noteId, plan.edit)
      return ok ? 'edit' : 'refuse'
    },

    async addTask(title, target) {
      const s = get()
      const plan = addTaskPlan(target.filter, { statuses: s.statuses, today: s.today, weekStart: s.weekStart, folders: s.folders })
      if (!plan.show || !title.trim()) return null
      const folder = plan.folderId ? s.folders.find((f) => f.id === plan.folderId) ?? null : null
      let res: CaptureResult
      try {
        res = await call('note_capture', { text: captureText(title, plan, folder) })
        if (folder && /\s/.test(folder.path)) await call('note_set_field', { note_id: res.note_id, edit: { field: 'folder_id', value: folder.id } })
        if (plan.personId) await call('note_link_add', { note_id: res.note_id, kind: 'contact', target_id: plan.personId, link_type: 'references' })
      } catch (e) {
        get().showNotice(message(e))
        return null
      }
      return res.note_id
    },

    setDetailOpen(open) {
      set({ detailOpen: open })
      if (open) void get().loadHistory()
    },

    async loadHistory() {
      const id = get().editor.noteId
      if (!id) {
        set({ history: [] })
        return
      }
      try {
        const history = await call('note_history', { note_id: id })
        if (get().editor.noteId === id) set({ history })
      } catch {
        set({ history: [] })
      }
    },

    async restore(rev) {
      const ed = get().editor
      if (!ed.noteId) return
      await get().saveNow()
      const res = await write(ed.noteId, call('note_restore', { note_id: ed.noteId, rev }))
      if (res?.outcome === 'saved') {
        const now = get().editor
        set({ editor: { ...now, text: res.text, version: now.version + 1, save: 'saved' } })
        get().showNotice(`Restored revision ${rev}`)
        await get().loadHistory()
      }
    },

    async addUrl(url) {
      const ed = get().editor
      const u = url.trim()
      if (!ed.noteId || !ed.detail || !u) return
      try {
        const link = await call('note_link_add', { note_id: ed.noteId, kind: 'url', target_id: u, link_type: 'attachment' })
        const now = get().editor
        if (now.detail) set({ editor: { ...now, detail: { ...now.detail, links: [...now.detail.links, link], row: { ...now.detail.row, has_links: true } } } })
      } catch (e) {
        get().showNotice(message(e))
      }
    },

    setBarChips(chips) {
      set({ barChips: chips })
    },

    async tagToBar(tag) {
      const norm = tag.trim().replace(/^#/, '').toLowerCase()
      const known = get().tags.find((t) => t.name_norm === norm)
      const chip: QueryChip = { kind: 'tag', value: norm, label: known?.name ?? tag.replace(/^#/, ''), glyph: '#', origin: 'typed' }
      const chips = get().barChips.some((c) => c.kind === 'tag' && c.value === norm) ? get().barChips : [...get().barChips, chip]
      set({ barChips: [] })
      await get().spawnCard(chips)
    },

    async spawnCard(chips, opts = {}) {
      if (!chips.length) return null
      const view = opts.view ?? 'notes'
      const s = get()
      const same = s.cards.find((c) => c.view === view && sameChips([...c.chips, ...c.refine], chips))
      if (same && !opts.force) {
        get().raiseCard(same.id)
        return same.id
      }
      const mine = s.cards.filter((c) => c.view === view)
      let cards = s.cards
      if (mine.length >= MAX_CARDS) {
        const oldest = mine[0]
        cards = cards.filter((c) => c.id !== oldest.id)
        get().showNotice('Closed the oldest card')
      }
      const n = cards.filter((c) => c.view === view).length
      const card: CardState = {
        id: `card-${++cardSeq}`,
        view,
        chips,
        refine: [],
        status: 'loading',
        rows: [],
        count: 0,
        filter: null,
        type: 'notes',
        sort: 'urgent',
        groupBy: 'none',
        pos: { x: 16 + 24 * n, y: 16 + 24 * n, anchor: 'bottom' },
        width: CARD_WIDTH,
        z: ++topZ,
        grouping: null,
        error: null,
      }
      set({ cards: [...cards, card] })
      await get().runCard(card.id)
      return card.id
    },

    async runCard(id) {
      const card = get().cards.find((c) => c.id === id)
      if (!card) return
      patchCard(id, { status: card.rows.length ? card.status : 'loading' })
      const res = await runCardQuery(card)
      patchCard(id, res)
    },

    closeCard(id) {
      set((s) => ({ cards: s.cards.filter((c) => c.id !== id) }))
    },

    raiseCard(id) {
      const card = get().cards.find((c) => c.id === id)
      if (!card || card.z === topZ) return
      patchCard(id, { z: ++topZ })
    },

    moveCard(id, pos) {
      patchCard(id, { pos })
    },

    resizeCard(id, width) {
      patchCard(id, { width: Math.max(320, Math.min(640, Math.round(width))) })
    },

    async setCardRefine(id, chips) {
      patchCard(id, { refine: chips.map((c) => ({ ...c, origin: 'refine' as const })), status: 'loading' })
      await get().runCard(id)
    },

    async setCardSort(id, by) {
      patchCard(id, { sort: by })
      await get().runCard(id)
    },

    setCardGroupBy(id, groupBy) {
      patchCard(id, { groupBy })
    },

    async duplicateCard(id) {
      const card = get().cards.find((c) => c.id === id)
      if (!card) return
      const copy = await get().spawnCard([...card.chips], { view: card.view, force: true })
      if (copy && card.refine.length) await get().setCardRefine(copy, card.refine)
    },

    closeOtherCards(id) {
      const card = get().cards.find((c) => c.id === id)
      if (!card) return
      set((s) => ({ cards: s.cards.filter((c) => c.id === id || c.view !== card.view) }))
    },

    async saveCardAsGrouping(id, name, glyph = '▣') {
      const card = get().cards.find((c) => c.id === id)
      if (!card) return 'No such card.'
      if (!card.filter) await get().runCard(id)
      const filter = get().cards.find((c) => c.id === id)?.filter
      if (!filter) return "Couldn't run this query."
      try {
        const g = await call('grouping_save', { grouping: { name, glyph, filter_json: filter, sort_json: card.sort === 'urgent' ? CARD_DEFAULT_SORT : sortKeys(card.sort), show_in_sidebar: true, show_in_overlay: false } })
        patchCard(id, { grouping: { id: g.id, slug: g.slug, name: g.name, glyph: g.glyph, show_in_overlay: g.show_in_overlay } })
        set((s) => ({ groupings: [...s.groupings.filter((x) => x.id !== g.id), g].sort((a, b) => a.position - b.position) }))
        get().showNotice(`Saved as ${g.name}`)
        return null
      } catch (e) {
        return message(e)
      }
    },

    async toggleCardOverlay(id) {
      const card = get().cards.find((c) => c.id === id)
      const g = card?.grouping && get().groupings.find((x) => x.id === card.grouping!.id)
      if (!card || !g) return
      try {
        const saved = await call('grouping_save', { grouping: { id: g.id, name: g.name, filter_json: g.filter_json, show_in_overlay: !g.show_in_overlay } })
        patchCard(id, { grouping: { ...card.grouping!, show_in_overlay: saved.show_in_overlay } })
        set((s) => ({ groupings: s.groupings.map((x) => (x.id === saved.id ? saved : x)) }))
      } catch (e) {
        get().showNotice(message(e))
      }
    },

    showNotice(text, action) {
      clearTimeout(noticeTimer)
      const notice = { id: ++noticeSeq, text, action }
      set({ notice })
      noticeTimer = setTimeout(() => {
        if (get().notice?.id === notice.id) set({ notice: null })
      }, NOTICE_MS)
    },

    dismissNotice() {
      clearTimeout(noticeTimer)
      set({ notice: null })
    },
  }
})

function inverseEdit(row: NoteListRow, edit: NoteFieldEdit): NoteFieldEdit | null {
  switch (edit.field) {
    case 'status':
      return { field: 'status', value: row.status }
    case 'urgent':
      return { field: 'urgent', value: row.urgent }
    case 'when':
      return { field: 'when', value: row.when_date }
    case 'tags':
      return { field: 'tags', value: row.tags }
    case 'pinned':
      return { field: 'pinned', value: row.pinned }
    case 'folder_id':
      return { field: 'folder_id', value: row.folder_id }
  }
}

/** Test helper: back to the initial state, timers and subscriptions dropped. */
export function resetNotesStore(): void {
  useNotesStore.getState().stop()
  inFlight = null
  loading = null
  cardSeq = 0
  topZ = 1
  useNotesStore.setState(initial())
}
