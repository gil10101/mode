// The command palette (10 §7.4.15, 03 §12 #35; proposed, not decided): one
// input over sessions, notes, messages, automations and settings; results
// from `palette_query` grouped by kind; Enter routes; recent items on an
// empty input. Opened with ⌘/Ctrl+P because ⌘/Ctrl+K is the floating query
// bar (03 §10.1); the key is printed only in the Shortcuts view (ADR-0038).
//
// Contract: `mode_core::palette::api` (package 11): `palette_query {text,
// limit?, kinds?}` → hits ranked by tier (exact, prefix, word, substring,
// fuzzy, keyword, body) plus recency; default 20 rows, at most 100. Row ids
// and routes as the core's index builds them: a message row carries the
// message id and opens `inbox/message/<id>`; a command is `view.<name>` or
// an action id (`note.new`) opening the view it acts in; a file is its
// absolute path and opens the session that touched it last
// (`sessions/<id>/files`).
import { create } from 'zustand'
import type { IconId } from '../kit/icons'
import { invoke } from '../tauri'

/* ---- contract ------------------------------------------------------------------------------- */

/** `command` (views and actions) and `file` (files the sessions touched) are the core's beyond the wireframe's list. */
export type PaletteKind = 'session' | 'note' | 'message' | 'automation' | 'setting' | 'chat' | 'event' | 'command' | 'file'

export interface PaletteResult {
  kind: PaletteKind
  id: string
  title: string
  /** The row's meta in `--m-fg-3` ("finished · mode", "done · Work/Mode", "09:41"). */
  detail: string
  /** The app route Enter opens (03 §2.5; a command: the view it acts in). */
  route: string
  /** The rank: tier score plus recency (higher first). */
  score: number
}

export interface PaletteQueryArgs {
  text: string
  limit?: number
  /** Beyond the list: restrict to some kinds (Chat's `+` → Attach note / message / session). */
  kinds?: PaletteKind[]
}

export interface PaletteCommands {
  palette_query: { args: PaletteQueryArgs; result: PaletteResult[] }
}

/* ---- grouping ------------------------------------------------------------------------------- */

/**
 * Group order, labels and icons: the wireframe's five (app-window.html `#palette`) with Calendar and Chat, and the
 * core's order around them (`palette::api::kind_order`: commands first, files last). Kit icons only.
 */
export const PALETTE_GROUPS: readonly { kind: PaletteKind; label: string; icon: IconId }[] = [
  { kind: 'command', label: 'Commands', icon: 'slash' },
  { kind: 'session', label: 'Sessions', icon: 'sessions' },
  { kind: 'note', label: 'Notes', icon: 'notes' },
  { kind: 'message', label: 'Inbox', icon: 'inbox' },
  { kind: 'event', label: 'Calendar', icon: 'calendar' },
  { kind: 'chat', label: 'Chat', icon: 'chat' },
  { kind: 'automation', label: 'Automations', icon: 'automations' },
  { kind: 'setting', label: 'Settings', icon: 'settings' },
  { kind: 'file', label: 'Files', icon: 'file' },
]

/** The view a route opens → its sidebar icon (a command shows the icon of the view it acts in). */
const VIEW_ICON: Record<string, IconId> = {
  sessions: 'sessions',
  inbox: 'inbox',
  notes: 'notes',
  calendar: 'calendar',
  chat: 'chat',
  automations: 'automations',
  integrations: 'integrations',
  shortcuts: 'shortcuts',
  stats: 'stats',
  settings: 'settings',
}

/** A row's icon: its kind's, except a command, which takes its view's (`Go to Inbox` → inbox; unknown → slash). */
export function rowIcon(r: Pick<PaletteResult, 'kind' | 'route'>): IconId {
  if (r.kind === 'command') return VIEW_ICON[(r.route ?? '').replace(/^#?\/?/, '').split('/')[0] ?? ''] ?? 'slash'
  return PALETTE_GROUPS.find((g) => g.kind === r.kind)?.icon ?? 'notes'
}

export interface PaletteGroupOf {
  kind: PaletteKind | 'recent'
  label: string
  rows: PaletteResult[]
}

/** Results by kind in the fixed group order; within a group by score (desc), then title. Unknown kinds go last under their own name. */
export function groupResults(results: PaletteResult[]): PaletteGroupOf[] {
  const by = new Map<string, PaletteResult[]>()
  for (const r of results) {
    if (!by.has(r.kind)) by.set(r.kind, [])
    by.get(r.kind)!.push(r)
  }
  const order = PALETTE_GROUPS.map((g) => g.kind as string)
  const kinds = Array.from(by.keys()).sort((a, b) => {
    const ia = order.indexOf(a)
    const ib = order.indexOf(b)
    return (ia < 0 ? order.length : ia) - (ib < 0 ? order.length : ib) || a.localeCompare(b)
  })
  return kinds.map((k) => ({
    kind: k as PaletteKind,
    label: PALETTE_GROUPS.find((g) => g.kind === k)?.label ?? k,
    rows: [...by.get(k)!].sort((a, b) => b.score - a.score || a.title.localeCompare(b.title)),
  }))
}

/** The rows in display order (what ↑ ↓ walk). */
export function flatRows(groups: PaletteGroupOf[]): PaletteResult[] {
  return groups.flatMap((g) => g.rows)
}

export function rowKey(r: Pick<PaletteResult, 'kind' | 'id'>): string {
  return `${r.kind}:${r.id}`
}

/* ---- recent --------------------------------------------------------------------------------- */

/** Rows shown per kind; the best matches first. */
export const GROUP_MAX = 5

export const RECENT_KEY = 'mode.palette.recent'
export const RECENT_MAX = 8

export function readRecent(): PaletteResult[] {
  try {
    const v = JSON.parse(window.localStorage.getItem(RECENT_KEY) ?? '[]') as unknown
    return Array.isArray(v) ? (v.filter((r) => r && typeof r === 'object' && 'route' in r && 'title' in r) as PaletteResult[]).slice(0, RECENT_MAX) : []
  } catch {
    return []
  }
}

export function pushRecent(list: PaletteResult[], r: PaletteResult): PaletteResult[] {
  return [r, ...list.filter((x) => rowKey(x) !== rowKey(r))].slice(0, RECENT_MAX)
}

function writeRecent(list: PaletteResult[]) {
  try {
    window.localStorage.setItem(RECENT_KEY, JSON.stringify(list))
  } catch {
    /* recent items last for this window only */
  }
}

/* ---- the store ------------------------------------------------------------------------------- */

export interface PaletteState {
  open: boolean
  text: string
  results: PaletteResult[]
  recent: PaletteResult[]
  /** `rowKey` of the active row. */
  active: string | null
  loading: boolean
  error: string | null
  show(): void
  hide(): void
  setText(text: string): void
  /** Runs `palette_query` for the current text (debounced by the view). */
  query(): Promise<void>
  groups(): PaletteGroupOf[]
  move(delta: number): void
  setActive(key: string | null): void
  /** The chosen row: remembered as recent; the caller routes. */
  choose(r: PaletteResult): PaletteResult
}

let seq = 0

export const usePalette = create<PaletteState>((set, get) => ({
  open: false,
  text: '',
  results: [],
  recent: [],
  active: null,
  loading: false,
  error: null,
  show() {
    const recent = readRecent()
    set({ open: true, text: '', results: [], recent, active: recent[0] ? rowKey(recent[0]) : null, error: null })
  },
  hide() {
    seq++
    set({ open: false, loading: false })
  },
  setText(text) {
    set({ text })
    if (!text.trim()) {
      const recent = get().recent
      set({ results: [], active: recent[0] ? rowKey(recent[0]) : null, loading: false })
    }
  },
  async query() {
    const text = get().text
    if (!text.trim()) return
    const mine = ++seq
    set({ loading: true })
    try {
      const results = await invoke<PaletteResult[]>('palette_query', { text, limit: 30 })
      if (mine !== seq) return
      const list = Array.isArray(results) ? results : []
      const first = flatRows(groupResults(list))[0]
      set({ results: list, loading: false, error: null, active: first ? rowKey(first) : null })
    } catch (e) {
      if (mine !== seq) return
      set({ results: [], loading: false, error: e && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : String(e) })
    }
  },
  groups() {
    const s = get()
    if (!s.text.trim()) return s.recent.length ? [{ kind: 'recent', label: 'Recent', rows: s.recent }] : []
    return groupResults(s.results).map((g) => ({ ...g, rows: g.rows.slice(0, GROUP_MAX) }))
  },
  move(delta) {
    const rows = flatRows(get().groups())
    if (!rows.length) return
    const i = rows.findIndex((r) => rowKey(r) === get().active)
    const next = rows[(i < 0 ? (delta > 0 ? -1 : 0) : i) + delta]
    const wrapped = next ?? (delta > 0 ? rows[0] : rows[rows.length - 1])
    set({ active: rowKey(wrapped!) })
  },
  setActive(active) {
    set({ active })
  },
  choose(r) {
    const recent = pushRecent(get().recent, r)
    writeRecent(recent)
    set({ recent, open: false })
    return r
  },
}))
