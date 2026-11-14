// The app window's Sessions store (03 §3, 05 §11–§12): the roster pushed by
// `sessions.changed`, the history rows, the selection, filters, search and
// layout of the roster, the session-detail cache (`session_detail`), the
// decisions in flight and the one inline notice. The roster model (grouping,
// sorting, folding, filtering, agent nesting) is a set of pure functions
// below so it can be tested without React.
import { create } from 'zustand'
import { invoke, listen } from '../tauri'
import { cmdErrorText, EVENTS, type DecideResult, type Decision, type SessionsChanged } from './contract'
import type {
  AgentRow,
  AppHealth,
  AppRosterRow,
  LaunchArgs,
  LaunchResult,
  ResumeArgs,
  SessionDetail,
  SnoozePreset,
} from './contract-sessions'

/* ---- the roster model ----------------------------------------------------- */

/**
 * Group order of the roster. `unread` and `done` share the word "finished"
 * and one group (unread first, then acknowledged, then the fold), as the
 * overlay (02 §7.3), the kit (`SESSION_STATES`) and app-window.html draw it.
 */
export const GROUP_ORDER = ['waiting', 'working', 'failed', 'finished', 'idle', 'untracked'] as const
export type GroupKey = (typeof GROUP_ORDER)[number]

export const ALL_STATES = ['waiting', 'working', 'failed', 'unread', 'idle', 'done', 'untracked'] as const
export const ALL_KINDS = ['hooked', 'launched', 'discovered'] as const

export function groupOf(state: string): GroupKey | string {
  return state === 'unread' || state === 'done' ? 'finished' : state
}

function groupIndex(state: string): number {
  const i = (GROUP_ORDER as readonly string[]).indexOf(groupOf(state))
  return i < 0 ? GROUP_ORDER.length : i
}

/** Inside "finished": unread before acknowledged. */
function subIndex(state: string): number {
  return state === 'done' ? 1 : 0
}

/** Q2 order, made explicit: group, then `last_event_at` desc when the row carries it; otherwise the pushed order is kept (stable). */
export function sortRows<T extends Pick<AppRosterRow, 'state' | 'last_event_at'>>(rows: readonly T[]): T[] {
  return rows
    .map((row, i) => ({ row, i }))
    .sort((a, b) => {
      const g = groupIndex(a.row.state) - groupIndex(b.row.state)
      if (g) return g
      const s = subIndex(a.row.state) - subIndex(b.row.state)
      if (s) return s
      const ta = a.row.last_event_at ? Date.parse(a.row.last_event_at) : NaN
      const tb = b.row.last_event_at ? Date.parse(b.row.last_event_at) : NaN
      if (!Number.isNaN(ta) && !Number.isNaN(tb) && ta !== tb) return tb - ta
      return a.i - b.i
    })
    .map((x) => x.row)
}

export type DateFilter = 'today' | 'week'

export interface RosterFilters {
  /** `sessions.state` values; empty = all. */
  states: string[]
  /** `repo_root` values; empty = all. */
  repos: string[]
  /** `tracking` values; empty = all. */
  kinds: string[]
  date: DateFilter | null
}

export const EMPTY_FILTERS: RosterFilters = { states: [], repos: [], kinds: [], date: null }

export function hasFilters(f: RosterFilters): boolean {
  return f.states.length > 0 || f.repos.length > 0 || f.kinds.length > 0 || f.date !== null
}

/** In-memory search over `display_name`, `title`, `cwd`, `git_branch` (03 §3.3): prefix and substring, case-insensitive. */
export function matchesSearch(row: Pick<AppRosterRow, 'display_name' | 'title' | 'cwd' | 'git_branch'>, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true
  return [row.display_name, row.title, row.cwd, row.git_branch].some((v) => typeof v === 'string' && v.toLowerCase().includes(q))
}

function startOfDay(now: number): number {
  const d = new Date(now)
  d.setHours(0, 0, 0, 0)
  return d.getTime()
}

/** Monday 00:00 of the current week. */
function startOfWeek(now: number): number {
  const d = new Date(startOfDay(now))
  const back = (d.getDay() + 6) % 7
  d.setDate(d.getDate() - back)
  return d.getTime()
}

export function matchesFilters(row: AppRosterRow, f: RosterFilters, now: number): boolean {
  if (f.states.length && !f.states.includes(row.state)) return false
  if (f.repos.length && !(row.repo_root && f.repos.includes(row.repo_root))) return false
  if (f.kinds.length && !f.kinds.includes(row.tracking)) return false
  if (f.date) {
    const t = row.started_at ? Date.parse(row.started_at) : NaN
    if (Number.isNaN(t)) return false
    const from = f.date === 'today' ? startOfDay(now) : startOfWeek(now)
    if (t < from) return false
  }
  return true
}

/** Distinct `repo_root` values, for the Repo filter. */
export function repoOptions(rows: readonly AppRosterRow[]): string[] {
  return Array.from(new Set(rows.map((r) => r.repo_root).filter((r): r is string => !!r))).sort()
}

/** The sidebar count: top-level `waiting` sessions (03 §2.3). */
export function waitingCount(rows: readonly Pick<AppRosterRow, 'state' | 'dismissed_at'>[]): number {
  return rows.filter((r) => r.state === 'waiting' && !r.dismissed_at).length
}

/** A finished agent stays in the roster nest for 10 s (03 §3.2.1). */
export const AGENT_HOLD_MS = 10_000

export function isAgentLive(a: Pick<AgentRow, 'ended_at'>): boolean {
  return !a.ended_at
}

/** The agent rows nested under an expanded parent: the running ones and those finished less than 10 s ago. */
export function nestedAgents(agents: readonly AgentRow[], now: number): AgentRow[] {
  return agents.filter((a) => {
    if (!a.ended_at) return true
    const t = Date.parse(a.ended_at)
    return !Number.isNaN(t) && now - t < AGENT_HOLD_MS
  })
}

export type RosterItem =
  | { kind: 'label'; key: string; text: string }
  | { kind: 'row'; row: AppRosterRow; history: boolean }
  | { kind: 'agent'; parent: string; agent: AgentRow }
  | { kind: 'fold'; count: number; open: boolean }

export interface RosterOptions {
  groupByRepo?: boolean
  foldOpen?: boolean
  /** Parent id → expanded; a missing entry means expanded while agents run. */
  expanded?: Record<string, boolean>
  agentsOf?: (id: string) => AgentRow[] | undefined
  now: number
}

/** Whether a parent's agent nest is open: the remembered toggle, else open while any agent runs. */
export function isExpanded(row: Pick<AppRosterRow, 'id' | 'live_agents'>, expanded: Record<string, boolean> | undefined): boolean {
  const v = expanded?.[row.id]
  return v ?? row.live_agents > 0
}

function pushRow(items: RosterItem[], row: AppRosterRow, history: boolean, o: RosterOptions) {
  items.push({ kind: 'row', row, history })
  if (history || !isExpanded(row, o.expanded)) return
  const agents = o.agentsOf?.(row.id) ?? row.agents
  if (!agents) return
  for (const agent of nestedAgents(agents, o.now)) items.push({ kind: 'agent', parent: row.id, agent })
}

/**
 * The roster as a flat list of items: group labels in GROUP_ORDER, rows,
 * nested agent rows and the "Recently completed (n)" fold at the end of the
 * finished group (05 §11.4, §12). History rows follow under their own label.
 * With `groupByRepo` the rows are bucketed by repository (label = repo name)
 * keeping the state order inside each bucket.
 */
export function rosterItems(rows: readonly AppRosterRow[], o: RosterOptions, history: readonly AppRosterRow[] = []): RosterItem[] {
  const sorted = sortRows(rows)
  const items: RosterItem[] = []
  if (o.groupByRepo) {
    const buckets = new Map<string, AppRosterRow[]>()
    for (const r of sorted) {
      const key = r.repo_root ?? r.cwd
      if (!buckets.has(key)) buckets.set(key, [])
      buckets.get(key)!.push(r)
    }
    for (const [key, list] of buckets) {
      items.push({ kind: 'label', key: `repo:${key}`, text: repoLabel(key) })
      for (const r of list) pushRow(items, r, false, o)
    }
  } else {
    let current: string | null = null
    const folded: AppRosterRow[] = []
    const flushFold = () => {
      if (folded.length) {
        if (o.foldOpen) for (const r of folded) pushRow(items, r, false, o)
        items.push({ kind: 'fold', count: folded.length, open: !!o.foldOpen })
        folded.length = 0
      }
    }
    for (const r of sorted) {
      const g = groupOf(r.state)
      if (g !== current) {
        if (current === 'finished') flushFold()
        current = g
        items.push({ kind: 'label', key: `state:${g}`, text: g })
      }
      // `unread` and `failed` never fold (ADR-0008).
      if (r.state === 'done' && r.folded) folded.push(r)
      else pushRow(items, r, false, o)
    }
    if (current === 'finished') flushFold()
    // A finished group made only of folded rows still gets its label (pushed above) and its fold row.
  }
  if (history.length) {
    items.push({ kind: 'label', key: 'history', text: 'history' })
    for (const r of history) items.push({ kind: 'row', row: r, history: true })
  }
  return items
}

/** A column of the Board layout: one state group (or history) and the items under it. */
export interface BoardColumn {
  key: string
  text: string
  items: RosterItem[]
}

/**
 * The Board layout's columns (03 §3.1): the roster items cut at their state
 * labels, so one column per non-empty group in GROUP_ORDER (finished =
 * `unread` + `done`, its fold row last) and history after them. Agent rows are
 * left out, as on the cards; items built with `groupByRepo` are cut at their
 * repo labels the same way, so callers pass state-grouped items.
 */
export function boardColumns(items: readonly RosterItem[]): BoardColumn[] {
  const cols: BoardColumn[] = []
  for (const item of items) {
    if (item.kind === 'label') cols.push({ key: item.key, text: item.text, items: [] })
    else if (item.kind !== 'agent' && cols.length) cols[cols.length - 1].items.push(item)
  }
  return cols.filter((c) => c.items.length > 0)
}

function repoLabel(path: string): string {
  const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/)
  return parts[parts.length - 1] || path
}

/** Which actions a row offers (03 §3.4, 05 §12). */
export function canDismiss(row: Pick<AppRosterRow, 'state' | 'ended_at'>): boolean {
  return row.state === 'done' || row.state === 'failed' || row.state === 'unread' || row.state === 'untracked' || !!row.ended_at
}

export function canResume(row: Pick<AppRosterRow, 'ended_at' | 'dismissed_at' | 'state'>, history = false): boolean {
  return history || !!row.ended_at || !!row.dismissed_at || row.state === 'untracked'
}

export function canSnooze(row: Pick<AppRosterRow, 'state'>): boolean {
  return row.state === 'waiting' || row.state === 'unread' || row.state === 'failed'
}

export type PrimaryAction = 'allow' | 'answer' | 'jump' | 'open' | 'resume'

/** The one visible action of a row (03 §3.4). */
export function primaryAction(row: AppRosterRow, history = false): PrimaryAction {
  if (history || row.dismissed_at) return 'resume'
  if (row.state === 'waiting' && row.pending?.answerable) return row.pending.kind === 'question' ? 'answer' : 'allow'
  if (row.state === 'done' || row.state === 'unread' || row.state === 'failed') return 'open'
  if (row.ended_at) return 'resume'
  return 'jump'
}

/* ---- the store ------------------------------------------------------------ */

export type Layout = 'list' | 'cards' | 'board'

export type DecisionKind = 'allow' | 'deny' | 'always' | 'answer'

export interface Decided {
  kind: DecisionKind
  outcome: DecideResult['outcome'] | 'pending'
  /** The `pending.received_at` the decision answered; a newer request clears it. */
  received_at: string | null
}

export interface Notice {
  id: number
  text: string
  action?: { label: string; run: () => void }
}

export type DetailStatus = 'loading' | 'ready' | 'error'

export interface SessionsState {
  rows: AppRosterRow[]
  /** When the last push arrived (elapsed labels tick from it). */
  rowsAt: number
  /** The first roster (list or push) arrived. */
  loaded: boolean
  history: AppRosterRow[]
  showHistory: boolean
  health: AppHealth | null
  filters: RosterFilters
  search: string
  layout: Layout
  groupByRepo: boolean
  selectedId: string | null
  expanded: Record<string, boolean>
  foldOpen: boolean
  details: Record<string, SessionDetail>
  detailStatus: Record<string, DetailStatus>
  decided: Record<string, Decided>
  notice: Notice | null

  applySessions(rows: AppRosterRow[], now?: number): void
  applyHealth(h: AppHealth): void
  select(id: string | null): void
  toggleExpanded(id: string, open?: boolean): void
  setFoldOpen(open: boolean): void
  setLayout(layout: Layout): void
  setGroupByRepo(on: boolean): void
  setSearch(q: string): void
  setFilters(patch: Partial<RosterFilters>): void
  clearFilters(): void
  setShowHistory(on: boolean): Promise<void>
  loadDetail(id: string): Promise<SessionDetail | null>
  acknowledge(id: string): Promise<void>
  dismiss(id: string): Promise<void>
  undismiss(id: string): Promise<void>
  snooze(id: string, preset: SnoozePreset): Promise<void>
  decide(id: string, kind: DecisionKind, decision: Decision, received_at: string | null): Promise<DecideResult['outcome']>
  launch(args: LaunchArgs): Promise<LaunchResult | null>
  resume(args: ResumeArgs): Promise<LaunchResult | null>
  showNotice(text: string, action?: Notice['action']): void
  clearNotice(id?: number): void
  reset(): void
}

const INITIAL = {
  rows: [] as AppRosterRow[],
  rowsAt: 0,
  loaded: false,
  history: [] as AppRosterRow[],
  showHistory: false,
  health: null as AppHealth | null,
  filters: EMPTY_FILTERS,
  search: '',
  layout: 'list' as Layout,
  groupByRepo: false,
  selectedId: null as string | null,
  expanded: {} as Record<string, boolean>,
  foldOpen: false,
  details: {} as Record<string, SessionDetail>,
  detailStatus: {} as Record<string, DetailStatus>,
  decided: {} as Record<string, Decided>,
  notice: null as Notice | null,
}

/** "Snoozed until 14:00" / "Snoozed until Thu 09:00" from `session_snooze`'s end time (local). */
export function snoozedText(until: string | null | undefined, now = Date.now()): string {
  const t = until ? Date.parse(until) : NaN
  if (Number.isNaN(t)) return 'Snoozed'
  const d = new Date(t)
  const hm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
  const today = new Date(now)
  const sameDay = d.getFullYear() === today.getFullYear() && d.getMonth() === today.getMonth() && d.getDate() === today.getDate()
  if (sameDay) return `Snoozed until ${hm}`
  const wd = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'][d.getDay()]
  return `Snoozed until ${wd} ${hm}`
}

/** How long an inline notice stays (10 §12.3). */
export const NOTICE_MS = 8000

let noticeSeq = 0
let noticeTimer: ReturnType<typeof setTimeout> | undefined

export const useSessionsStore = create<SessionsState>((set, get) => ({
  ...INITIAL,

  applySessions(rows, now = Date.now()) {
    set((s) => {
      // A newer request (a different `received_at`) drops the decision for that row; a cleared
      // card keeps it so the thread can say how it ended until the detail catches up.
      const decided: Record<string, Decided> = {}
      for (const r of rows) {
        const d = s.decided[r.id]
        if (d && (!r.pending || d.received_at === null || d.received_at === r.pending.received_at)) decided[r.id] = d
      }
      // History rows are dismissed ones (`sessions_history`); a row back on the roster (undismissed, resumed) leaves it.
      const ids = new Set(rows.map((r) => r.id))
      const history = s.history.some((h) => ids.has(h.id)) ? s.history.filter((h) => !ids.has(h.id)) : s.history
      return { rows, rowsAt: now, loaded: true, decided, history }
    })
  },

  applyHealth(h) {
    set({ health: h })
  },

  select(id) {
    set({ selectedId: id })
  },

  toggleExpanded(id, open) {
    set((s) => {
      const row = s.rows.find((r) => r.id === id)
      const current = row ? isExpanded(row, s.expanded) : !!s.expanded[id]
      return { expanded: { ...s.expanded, [id]: open ?? !current } }
    })
  },

  setFoldOpen(open) {
    set({ foldOpen: open })
  },

  setLayout(layout) {
    set({ layout })
  },

  setGroupByRepo(on) {
    set({ groupByRepo: on })
  },

  setSearch(q) {
    set({ search: q })
  },

  setFilters(patch) {
    set((s) => ({ filters: { ...s.filters, ...patch } }))
  },

  clearFilters() {
    set({ filters: EMPTY_FILTERS, search: '' })
  },

  async setShowHistory(on) {
    set({ showHistory: on })
    if (!on) return
    try {
      const history = await invoke<AppRosterRow[]>('sessions_history')
      set({ history: Array.isArray(history) ? history : [] })
    } catch {
      set({ history: [] })
    }
  },

  async loadDetail(id) {
    set((s) => ({ detailStatus: { ...s.detailStatus, [id]: s.details[id] ? 'ready' : 'loading' } }))
    try {
      const detail = await invoke<SessionDetail | null>('session_detail', { session_id: id })
      if (!detail) {
        set((s) => ({ detailStatus: { ...s.detailStatus, [id]: 'error' } }))
        return null
      }
      set((s) => ({ details: { ...s.details, [id]: detail }, detailStatus: { ...s.detailStatus, [id]: 'ready' } }))
      return detail
    } catch {
      set((s) => ({ detailStatus: { ...s.detailStatus, [id]: 'error' } }))
      return null
    }
  },

  async acknowledge(id) {
    const row = get().rows.find((r) => r.id === id)
    if (row && row.state !== 'unread') return
    await invoke('session_acknowledge', { session_id: id }).catch(() => {})
  },

  async dismiss(id) {
    try {
      await invoke('session_dismiss', { session_id: id })
    } catch (e) {
      get().showNotice(`Couldn't dismiss the session: ${cmdErrorText(e)}`)
      return
    }
    if (get().selectedId === id) set({ selectedId: null })
    get().showNotice('Dismissed', { label: 'Undo', run: () => void get().undismiss(id) })
    // The history list (dismissed rows only) gains it.
    if (get().showHistory) void get().setShowHistory(true)
  },

  async undismiss(id) {
    try {
      await invoke('session_undismiss', { session_id: id })
      set((s) => ({ history: s.history.filter((h) => h.id !== id) }))
      get().clearNotice()
    } catch (e) {
      get().showNotice(`Couldn't undo: ${cmdErrorText(e)}`)
    }
  },

  async snooze(id, preset) {
    try {
      const until = await invoke<string | null>('session_snooze', { session_id: id, preset })
      get().showNotice(snoozedText(until), { label: 'Undo', run: () => void invoke('session_snooze', { session_id: id, preset: null }).catch(() => {}) })
    } catch (e) {
      get().showNotice(`Couldn't snooze the session: ${cmdErrorText(e)}`)
    }
  },

  async decide(id, kind, decision, received_at) {
    set((s) => ({ decided: { ...s.decided, [id]: { kind, outcome: 'pending', received_at } } }))
    let outcome: DecideResult['outcome']
    try {
      const r = await invoke<DecideResult>('session_decide', { session_id: id, decision })
      outcome = r?.outcome ?? 'failed'
    } catch {
      outcome = 'failed'
    }
    set((s) => ({ decided: { ...s.decided, [id]: { kind, outcome, received_at } } }))
    if (outcome === 'already_answered') get().showNotice('Already answered')
    return outcome
  },

  async launch(args) {
    try {
      return await invoke<LaunchResult>('session_launch', args as unknown as Record<string, unknown>)
    } catch (e) {
      get().showNotice(`Couldn't launch the session: ${cmdErrorText(e)}`)
      return null
    }
  },

  async resume(args) {
    try {
      return await invoke<LaunchResult>('session_resume', args as unknown as Record<string, unknown>)
    } catch (e) {
      get().showNotice(`Couldn't resume the session: ${cmdErrorText(e)}`)
      return null
    }
  },

  showNotice(text, action) {
    const id = ++noticeSeq
    set({ notice: { id, text, action } })
    clearTimeout(noticeTimer)
    noticeTimer = setTimeout(() => get().clearNotice(id), NOTICE_MS)
  },

  clearNotice(id) {
    set((s) => (id === undefined || s.notice?.id === id ? { notice: null } : {}))
  },

  reset() {
    clearTimeout(noticeTimer)
    set({ ...INITIAL })
  },
}))

/**
 * Subscribes the store to the core: `sessions.changed`, `health.changed`
 * and the first `sessions_list`. Returns the unsubscribe function.
 */
export async function connectSessions(): Promise<() => void> {
  const store = useSessionsStore.getState()
  const offs = await Promise.all([
    listen<SessionsChanged>(EVENTS.sessions, (p) => {
      const before = useSessionsStore.getState().rows
      const rows = (p?.rows ?? []) as AppRosterRow[]
      useSessionsStore.getState().applySessions(rows)
      refreshChangedDetails(before, rows)
    }),
    listen<AppHealth>(EVENTS.health, (p) => useSessionsStore.getState().applyHealth(p)),
  ])
  try {
    const rows = await invoke<AppRosterRow[]>('sessions_list')
    // A push that raced the list wins.
    if (!useSessionsStore.getState().loaded) store.applySessions(Array.isArray(rows) ? rows : [])
  } catch {
    if (!useSessionsStore.getState().loaded) store.applySessions([])
  }
  return () => offs.forEach((off) => off())
}

/** Ids whose roster row differs between two pushes (or is new). */
export function changedIds(before: readonly AppRosterRow[], after: readonly AppRosterRow[]): string[] {
  const prev = new Map(before.map((r) => [r.id, JSON.stringify(r)]))
  return after.filter((r) => prev.get(r.id) !== JSON.stringify(r)).map((r) => r.id)
}

/** Refetch the details already loaded whose roster row changed with this push. */
function refreshChangedDetails(before: readonly AppRosterRow[], after: readonly AppRosterRow[]) {
  const s = useSessionsStore.getState()
  for (const id of changedIds(before, after)) if (s.details[id]) void s.loadDetail(id)
}
