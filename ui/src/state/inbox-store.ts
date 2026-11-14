// The Inbox (03 §4; behavior owners 06 and 07): the thread list as the core
// ranks it (`inbox_list`, never re-sorted here), its four sections (Urgent,
// Snoozed, the main list, Low priority), filters and search, the selected
// thread (`thread_get`, read on open after 1 s), and the triage actions with
// optimistic local state: the row changes at once, the command runs, a
// failure puts the row back and says so; a success shows the write-back
// notice ("Archived in Mode only · Slack has no archive") with Undo.
import { create } from 'zustand'
import { invoke, listen } from '../tauri'
import { cmdErrorText, EVENTS, type InboxChanged } from './contract'
import type {
  CoreInboxFilter,
  CoreThreadDetail,
  CoreThreadRow,
  CoreTriageResult,
  CoreVipRow,
  InboxFilter,
  InboxListResult,
  InboxSection,
  MessageRow,
  SnoozePreset,
  SnoozeResult,
  ThreadDetail,
  ThreadLink,
  ThreadRow,
  Tier,
  TriageResult,
  VipRow,
} from './contract-connectors'
import { useSessionsStore } from './sessions-store'
import { useAccounts } from './accounts-store'

/* ---- tiers and levels (07 §2.1, 10 §6.2) ---------------------------------------- */

export const URGENT_TIERS: readonly Tier[] = ['vip', 'direct', 'ai-urgent']
export const TIERS: readonly Tier[] = ['vip', 'direct', 'ai-urgent', 'ai-normal', 'low', 'unscored']
/** Tier menu entries (03 §4.5): the UI shows three levels; the menu names the tiers once. */
export const TIER_LABEL: Record<Tier, string> = {
  vip: 'important',
  direct: 'direct',
  'ai-urgent': 'urgent',
  'ai-normal': 'normal',
  low: 'low',
  unscored: 'not yet scored',
}

export type Level = 'urgent' | 'normal' | 'low'

export function levelOf(tier: Tier | null | undefined): Level {
  if (tier && URGENT_TIERS.includes(tier)) return 'urgent'
  if (tier === 'low') return 'low'
  return 'normal'
}

/** The 2-px accent bar (03 §4.2): `vip` / `direct` / `ai-urgent` only. */
export function isUrgentRow(r: Pick<ThreadRow, 'urgency_tier'>): boolean {
  return levelOf(r.urgency_tier) === 'urgent'
}

export function isSnoozed(r: Pick<ThreadRow, 'snoozed_until'>, now: number): boolean {
  if (!r.snoozed_until) return false
  const t = Date.parse(r.snoozed_until)
  return Number.isNaN(t) || t > now
}

/** The section a row belongs to: the core's when it sent one, else derived the way §13 Q3 / Q8 do. */
export function sectionOf(r: ThreadRow, now: number): InboxSection {
  if (r.section) return r.section
  if (isSnoozed(r, now)) return 'snoozed'
  if (r.urgency_tier === 'low') return 'low'
  if (isUrgentRow(r) && r.unread_count > 0) return 'urgent'
  return 'main'
}

export interface Sections {
  urgent: ThreadRow[]
  snoozed: ThreadRow[]
  main: ThreadRow[]
  low: ThreadRow[]
}

/**
 * Groups rows into the four sections, keeping the core's order inside each
 * (ranking is the core's, 07 §3.5). Archived rows are left out unless
 * `archived` is on. `stick` keeps the selected thread in the section it was
 * opened from until the selection moves (read on open must not move the row
 * the user is reading).
 */
export function groupSections(rows: readonly ThreadRow[], now: number, opts: { archived?: boolean; stick?: { id: string; section: InboxSection } | null } = {}): Sections {
  const out: Sections = { urgent: [], snoozed: [], main: [], low: [] }
  for (const r of rows) {
    if (r.is_archived && !opts.archived) continue
    const s = opts.stick && opts.stick.id === r.id ? opts.stick.section : sectionOf(r, now)
    out[s].push(r)
  }
  return out
}

/* ---- snooze presets (07 §6.2, resolved in local time) --------------------------- */

function at(d: Date, h: number, m = 0): Date {
  const x = new Date(d)
  x.setHours(h, m, 0, 0)
  return x
}

/** The preview / optimistic `until` of a preset; the core resolves the stored one. */
export function resolvePreset(preset: SnoozePreset, now: number, pick?: number): number {
  const d = new Date(now)
  switch (preset) {
    case 'later_today': {
      const six = at(d, 18)
      if (d.getHours() >= 18) {
        const t = at(d, 9)
        t.setDate(t.getDate() + 1)
        return t.getTime()
      }
      if (d.getHours() >= 15) return six.getTime()
      return Math.min(now + 3 * 3_600_000, six.getTime())
    }
    case 'tomorrow': {
      const t = at(d, 9)
      t.setDate(t.getDate() + 1)
      return t.getTime()
    }
    case 'next_week': {
      const t = at(d, 9)
      const dow = t.getDay() // 0 Sunday … 1 Monday
      const ahead = (8 - dow) % 7 || 7
      t.setDate(t.getDate() + ahead)
      return t.getTime()
    }
    case 'pick_date':
      return Math.max(pick ?? now, now + 5 * 60_000)
    case 'until_reply':
      return now + 30 * 86_400_000
  }
}

export const SNOOZE_PRESETS: readonly { preset: SnoozePreset; label: string }[] = [
  { preset: 'later_today', label: 'Later today' },
  { preset: 'tomorrow', label: 'Tomorrow' },
  { preset: 'next_week', label: 'Next week' },
  { preset: 'pick_date', label: 'Pick a date' },
  { preset: 'until_reply', label: 'Until reply' },
]

/* ---- filters (03 §4.4) ------------------------------------------------------------- */

export const EMPTY_FILTER: InboxFilter = {}

export function hasFilters(f: InboxFilter): boolean {
  return !!(f.tiers?.length || f.contact_id || f.query || f.source_id || f.unread_only || f.date || f.archived)
}

/** Local midnight of `now`'s day, as an instant. */
function dayStart(now: number): Date {
  const d = new Date(now)
  d.setHours(0, 0, 0, 0)
  return d
}

/** The filter sent to `inbox_list`: the chips plus the account chip, in the core's names (`CoreInboxFilter`). */
export function filterArgs(f: InboxFilter, account: string | null, now = Date.now()): CoreInboxFilter {
  const out: CoreInboxFilter = {}
  if (account) out.account_id = account
  if (f.tiers?.length) out.tiers = f.tiers
  if (f.contact_id) out.contact_id = f.contact_id
  if (f.query) out.topic = f.query
  if (f.source_id) out.source_id = f.source_id
  if (f.unread_only) out.unread_only = true
  if (f.date === 'today') out.from = dayStart(now).toISOString()
  else if (f.date === 'week') out.from = new Date(dayStart(now).getTime() - 6 * 86_400_000).toISOString()
  else if (f.date && typeof f.date === 'object') {
    out.from = f.date.from
    out.to = f.date.to
  }
  if (f.archived) out.show_archived = true
  return out
}

/* ---- the core's shapes → the UI's rows (contract-connectors.ts `Core*`) ---------------------- */

/** An `inbox_list` row: the UI's `ThreadRow`, or the core's `InboxThreadRow`. */
export function normalizeThread(raw: ThreadRow | CoreThreadRow): ThreadRow {
  if ('id' in raw && typeof raw.id === 'string') return raw
  const c = raw as CoreThreadRow
  const first = c.accounts?.[0]
  return {
    id: c.thread_id,
    account_id: first?.account_id ?? '',
    account_label: c.accounts?.length ? c.accounts.map((a) => a.label).join(', ') : null,
    provider: c.provider,
    kind: c.kind,
    title: c.title,
    channel_name: c.channel_name,
    source_id: null,
    sender_name: c.sender_name ?? '',
    sender_address: null,
    sender_avatar_url: c.sender_avatar_url,
    sender_contact_id: null,
    mentioned: c.mentioned,
    last_message_at: c.last_message_at ?? '',
    // Email rows show `title` (the subject); Slack rows the first line of this.
    last_snippet: c.line2,
    message_count: c.message_count,
    unread_count: c.unread_count,
    has_attachments: c.has_attachments,
    urgency_tier: c.urgency_tier,
    urgency_why: c.urgency_why,
    urgency_source: null,
    is_archived: c.is_archived,
    snoozed_until: c.snoozed_until,
    resurfaced: c.resurfaced,
    pending_writeback: c.pending_writeback,
    source_url: c.source_url,
    section: c.section,
  }
}

function participantsOf(v: unknown): ThreadDetail['participants'] {
  if (!Array.isArray(v)) return []
  return v.map((p) => {
    const o = (p ?? {}) as Record<string, unknown>
    return { name: String(o.name ?? o.address ?? ''), address: (o.address as string | undefined) ?? null, contact_id: (o.contact_id as string | undefined) ?? null }
  })
}

/** A `thread_get` answer: the UI's `ThreadDetail`, or the core's flat `ThreadDetail` (merged over the list row when there is one). */
export function normalizeDetail(raw: ThreadDetail | CoreThreadDetail, listRow?: ThreadRow): ThreadDetail {
  if ('thread' in raw) return raw
  const c = raw as CoreThreadDetail
  const messages: MessageRow[] = c.messages.map((m) => ({
    id: m.id,
    thread_id: c.id,
    sender_name: m.sender_name,
    sender_address: m.sender_address,
    sender_avatar_url: m.sender_avatar_url,
    received_at: m.received_at,
    body_text: m.body_text,
    body_truncated: m.body_truncated,
    snippet: m.snippet,
    attachments: m.has_attachments ? 1 : 0,
    is_read: m.is_read,
    urgency_tier: m.urgency_tier,
    urgency_why: m.urgency_why,
    urgency_source: m.urgency_source,
    mentioned: false,
    source_url: m.source_url,
  }))
  const last = c.messages[c.messages.length - 1]
  const unread = messages.filter((m) => !m.is_read).length
  const fromDetail: ThreadRow = {
    id: c.id,
    account_id: c.account.account_id,
    account_label: c.account.label,
    provider: c.provider,
    kind: c.kind,
    title: c.title,
    channel_name: c.channel_name,
    source_id: null,
    sender_name: last?.sender_name ?? '',
    sender_address: last?.sender_address ?? null,
    sender_avatar_url: last?.sender_avatar_url ?? null,
    sender_contact_id: last?.sender_contact_id ?? null,
    sender_vip: last?.vip ?? false,
    mentioned: false,
    last_message_at: last?.received_at ?? '',
    last_snippet: last?.snippet ?? null,
    message_count: c.message_count,
    unread_count: unread,
    has_attachments: c.messages.some((m) => m.has_attachments),
    urgency_tier: c.urgency_tier,
    urgency_why: last?.urgency_why ?? null,
    urgency_source: last?.urgency_source ?? null,
    is_archived: c.messages.length > 0 && c.messages.every((m) => m.is_archived),
    snoozed_until: last?.snoozed_until ?? null,
    source_url: c.source_url,
  }
  // The list row knows the section and the mention; the detail knows the sender's contact.
  const thread: ThreadRow = listRow
    ? { ...listRow, sender_address: listRow.sender_address ?? fromDetail.sender_address, sender_contact_id: listRow.sender_contact_id ?? fromDetail.sender_contact_id, sender_vip: listRow.sender_vip ?? fromDetail.sender_vip }
    : fromDetail
  return {
    thread,
    participants: participantsOf(c.participants),
    messages,
    contact: last?.sender_contact_id ? { id: last.sender_contact_id, display_name: last.sender_name, vip: last.vip, vip_note: null, identities: [last.sender_address] } : null,
    links: c.links ?? [],
    source_name: null,
    body_error: null,
    app_local_actions: c.app_local_actions,
  }
}

/** The core's target kind of a link row (`thread` rows are messages). */
export function linkTarget(kind: ThreadLink['kind']): 'session' | 'note' | 'message' {
  return kind === 'thread' ? 'message' : kind
}

/** An important person row: the UI's, or the core's (`contact_id`, identity objects). */
export function normalizeVip(raw: VipRow | CoreVipRow): VipRow {
  if ('id' in raw) return raw
  return {
    id: raw.contact_id,
    display_name: raw.display_name,
    identities: raw.identities.map((i) => (i.kind === 'slack_user' ? `@${i.value}${i.scope ? ` · ${i.scope}` : ''}` : i.value)),
    vip_note: raw.vip_note,
    message_count: raw.message_count,
    last_seen_at: null,
  }
}

/** Why an action stays in Mode, when the core did not say (06 §2.5, §8.8). */
export function localReason(row: Pick<ThreadRow, 'provider' | 'account_id'>, action: 'mark_read' | 'mark_unread' | 'archive'): string | null {
  if (row.provider === 'slack') return action === 'archive' ? 'Slack has no archive' : action === 'mark_unread' ? 'Slack has no mark unread' : 'mark read is off for this workspace'
  const a = useAccounts.getState().accounts.find((x) => x.id === row.account_id)
  if (a && !a.scopes.some((s) => s.endsWith('/gmail.modify'))) return 'Gmail access is read-only'
  return null
}

/** A triage answer: the UI's `TriageResult`, or the core's (`app_local`, `queued`). */
export function normalizeTriage(raw: TriageResult | CoreTriageResult | null | undefined, reason?: string | null): TriageResult {
  if (!raw) return { write_back: 'none' }
  if ('write_back' in raw) return raw
  return { write_back: raw.app_local ? 'local_only' : raw.queued > 0 ? 'queued' : 'none', reason: raw.app_local ? (reason ?? null) : null }
}

export interface SavedFilter {
  name: string
  filter: InboxFilter
  account_id?: string | null
}

/* ---- the store ------------------------------------------------------------------------ */

export interface Undoable {
  label: string
  run: () => void
  at: number
}

export interface InboxState {
  rows: ThreadRow[]
  nextCursor: string | null
  loaded: boolean
  loading: boolean
  error: string | null
  account: string | null
  filter: InboxFilter
  /** Toolbar search (03 §4.8): results replace the list. */
  search: string
  searchRows: ThreadRow[] | null
  selectedId: string | null
  /** The section the selected row was opened from (see `groupSections`). */
  stick: { id: string; section: InboxSection } | null
  details: Record<string, ThreadDetail>
  detailError: Record<string, string>
  /** "marked read in Mode only · Gmail access is read-only" per thread (06 §2.5). */
  localOnly: Record<string, string>
  expanded: { snoozed: boolean; low: boolean }
  multi: boolean
  picked: string[]
  urgentCount: number
  unreadInApp: number
  /** Message id → thread id, from the pushed rows (`InboxRow.thread_id`) and resolved deep links. */
  messageThreads: Record<string, string>
  lastUndo: Undoable | null
  /** Tiers seen before a thread was marked read (mark unread brings the bar back). */
  tierMemory: Record<string, Tier | null>
  load(): Promise<void>
  loadMore(): Promise<void>
  setAccount(id: string | null): void
  setFilter(f: InboxFilter): void
  clearFilters(): void
  setSearch(q: string): void
  runSearch(): Promise<void>
  select(id: string | null, opts?: { markRead?: boolean }): void
  loadDetail(id: string): Promise<void>
  /** Link the thread to a session, note or message by hand (`item_link_add`); the panel's links update. */
  linkAdd(id: string, to: Pick<ThreadLink, 'kind' | 'id' | 'title'>): Promise<boolean>
  /** Remove a link or dismiss a suggestion (`item_link_remove`); remembered by the core. */
  linkRemove(id: string, link: ThreadLink): Promise<boolean>
  toggleSection(s: 'snoozed' | 'low'): void
  setMulti(on: boolean): void
  togglePick(id: string): void
  pickAll(): void
  markRead(id: string, read: boolean, opts?: { silent?: boolean }): Promise<boolean>
  archive(id: string, archived?: boolean): Promise<boolean>
  snooze(id: string, preset: SnoozePreset, pick?: number): Promise<boolean>
  unsnooze(id: string): Promise<boolean>
  setTier(id: string, tier: Tier): Promise<boolean>
  /**
   * `inbox/message/<id>` → the thread to open: the pushed rows' `thread_id`, else `thread_get` with the
   * message id once the core resolves it (the answer names its thread), else the id itself (the fallback).
   */
  resolveMessage(id: string): Promise<string>
  setVip(id: string, vip: boolean): Promise<boolean>
  markAllRead(): Promise<void>
  bulk(action: 'read' | 'archive' | 'snooze', preset?: SnoozePreset): Promise<void>
  undo(): void
  applyPush(p: InboxChanged): void
}

export const PAGE = 100
const UNDO_MS = 8000

function notice(text: string, action?: { label: string; run: () => void }) {
  useSessionsStore.getState().showNotice(text, action)
}

function errorText(e: unknown): string {
  return cmdErrorText(e)
}

/** An `inbox.changed` has arrived: counts come from pushes from then on. */
let pushedOnce = false

let readTimer: ReturnType<typeof setTimeout> | undefined
let pushTimer: ReturnType<typeof setTimeout> | undefined
let loadSeq = 0

export const READ_ON_OPEN_MS = 1000

export const useInbox = create<InboxState>((set, get) => {
  /** Patches one row everywhere it is shown (list, search results, detail). */
  const patchRow = (id: string, patch: Partial<ThreadRow>) =>
    set((s) => ({
      rows: s.rows.map((r) => (r.id === id ? { ...r, ...patch } : r)),
      searchRows: s.searchRows?.map((r) => (r.id === id ? { ...r, ...patch } : r)) ?? null,
      details: s.details[id] ? { ...s.details, [id]: { ...s.details[id]!, thread: { ...s.details[id]!.thread, ...patch } } } : s.details,
    }))
  const rowOf = (id: string): ThreadRow | undefined => get().rows.find((r) => r.id === id) ?? get().searchRows?.find((r) => r.id === id) ?? get().details[id]?.thread
  /** Puts a snapshot back (the optimistic change failed). */
  const restore = (row: ThreadRow) => patchRow(row.id, row)
  const offerUndo = (text: string, label: string, run: () => void) => {
    const u: Undoable = { label, run, at: Date.now() }
    set({ lastUndo: u })
    notice(text, { label: 'Undo', run: () => get().undo() })
  }
  const writeBack = (id: string, r: TriageResult | undefined, done: string, opts?: { silent?: boolean; undo?: () => void }) => {
    const local = r?.write_back === 'local_only'
    if (local) set((s) => ({ localOnly: { ...s.localOnly, [id]: `${done.toLowerCase()} in Mode only${r?.reason ? ` · ${r.reason}` : ''}` } }))
    else set((s) => {
      if (!s.localOnly[id]) return {}
      const next = { ...s.localOnly }
      delete next[id]
      return { localOnly: next }
    })
    if (opts?.silent) return
    const text = local ? `${done} in Mode only${r?.reason ? ` · ${r.reason}` : ''}` : done
    if (opts?.undo) offerUndo(text, 'Undo', opts.undo)
    else notice(text)
  }

  return {
    rows: [],
    nextCursor: null,
    loaded: false,
    loading: false,
    error: null,
    account: null,
    filter: EMPTY_FILTER,
    search: '',
    searchRows: null,
    selectedId: null,
    stick: null,
    details: {},
    detailError: {},
    localOnly: {},
    expanded: { snoozed: false, low: false },
    multi: false,
    picked: [],
    urgentCount: 0,
    unreadInApp: 0,
    messageThreads: {},
    lastUndo: null,
    tierMemory: {},

    async load() {
      const seq = ++loadSeq
      const { filter, account, rows } = get()
      set({ loading: true })
      try {
        const res = await invoke<InboxListResult>('inbox_list', { filter: filterArgs(filter, account), limit: Math.max(PAGE, rows.length) })
        if (seq !== loadSeq) return
        set({ rows: (res?.rows ?? []).map(normalizeThread), nextCursor: res?.next_cursor ?? null, loaded: true, loading: false, error: null })
      } catch (e) {
        if (seq !== loadSeq) return
        set({ loaded: true, loading: false, error: errorText(e) })
      }
    },

    async loadMore() {
      const { nextCursor, filter, account, loading } = get()
      if (!nextCursor || loading) return
      const seq = loadSeq
      set({ loading: true })
      try {
        const res = await invoke<InboxListResult>('inbox_list', { filter: filterArgs(filter, account), cursor: nextCursor, limit: PAGE })
        if (seq !== loadSeq) return
        const more = (res?.rows ?? []).map(normalizeThread)
        set((s) => ({ rows: [...s.rows, ...more.filter((r) => !s.rows.some((x) => x.id === r.id))], nextCursor: res?.next_cursor ?? null, loading: false }))
      } catch (e) {
        set({ loading: false, error: errorText(e) })
      }
    },

    setAccount(id) {
      set({ account: id, rows: [], nextCursor: null, loaded: false })
      void get().load()
      if (get().search) void get().runSearch()
    },

    setFilter(f) {
      set({ filter: f, rows: [], nextCursor: null, loaded: false })
      void get().load()
    },

    clearFilters() {
      get().setFilter(EMPTY_FILTER)
    },

    setSearch(q) {
      set({ search: q })
      if (!q.trim()) set({ searchRows: null })
    },

    async runSearch() {
      const q = get().search.trim()
      if (!q) {
        set({ searchRows: null })
        return
      }
      try {
        const res = await invoke<InboxListResult>('inbox_list', { filter: { ...filterArgs(get().filter, get().account), topic: q, show_archived: true }, limit: PAGE })
        if (get().search.trim() === q) set({ searchRows: (res?.rows ?? []).map(normalizeThread) })
      } catch (e) {
        set({ searchRows: [], error: errorText(e) })
      }
    },

    select(id, opts) {
      clearTimeout(readTimer)
      if (!id) {
        set({ selectedId: null, stick: null })
        return
      }
      const row = rowOf(id)
      set({ selectedId: id, stick: row ? { id, section: sectionOf(row, Date.now()) } : null })
      // Read on open (03 §4.5): after 1 s of visibility, written back per 06.
      const schedule = (r: ThreadRow) => {
        if (opts?.markRead === false || r.unread_count === 0) return
        clearTimeout(readTimer)
        readTimer = setTimeout(() => {
          if (get().selectedId !== id) return
          if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return
          void get().markRead(id, true, { silent: true })
        }, READ_ON_OPEN_MS)
      }
      if (row) schedule(row)
      void get()
        .loadDetail(id)
        .then(() => {
          // A deep link can open a thread the list has not loaded yet.
          if (row || get().selectedId !== id) return
          const r = rowOf(id)
          if (r) schedule(r)
        })
    },

    async loadDetail(id) {
      try {
        const raw = await invoke<ThreadDetail | CoreThreadDetail>('thread_get', { thread_id: id })
        if (!raw) return
        set((s) => {
          const errs = { ...s.detailError }
          delete errs[id]
          // Keep the optimistic row state over the (possibly older) detail answer.
          const local = s.rows.find((r) => r.id === id)
          const d = normalizeDetail(raw, local)
          return { details: { ...s.details, [id]: local ? { ...d, thread: { ...d.thread, ...local, sender_contact_id: local.sender_contact_id ?? d.thread.sender_contact_id, sender_address: local.sender_address ?? d.thread.sender_address } } : d }, detailError: errs }
        })
      } catch (e) {
        set((s) => ({ detailError: { ...s.detailError, [id]: errorText(e) } }))
      }
    },

    async linkAdd(id, to) {
      const setLinks = (links: ThreadLink[]) =>
        set((s) => {
          const d = s.details[id]
          return d ? { details: { ...s.details, [id]: { ...d, links } } } : {}
        })
      try {
        const out = await invoke<ThreadLink[]>('item_link_add', { kind: 'thread', id, to_kind: linkTarget(to.kind), to_id: to.id })
        if (Array.isArray(out)) setLinks(out)
        else {
          const links = get().details[id]?.links ?? []
          if (!links.some((l) => l.kind === to.kind && l.id === to.id)) setLinks([...links, { ...to, why: 'linked by you', by_user: true }])
        }
        notice(`Linked to ${to.title}`)
        return true
      } catch (e) {
        notice(`Couldn't add the link: ${errorText(e)}`)
        return false
      }
    },

    async linkRemove(id, link) {
      const d = get().details[id]
      const before = d?.links ?? []
      if (d) set((s) => ({ details: { ...s.details, [id]: { ...d, links: before.filter((l) => !(l.kind === link.kind && l.id === link.id)) } } }))
      try {
        const out = await invoke<ThreadLink[]>('item_link_remove', { kind: 'thread', id, to_kind: linkTarget(link.kind), to_id: link.id })
        if (Array.isArray(out))
          set((s) => {
            const cur = s.details[id]
            return cur ? { details: { ...s.details, [id]: { ...cur, links: out } } } : {}
          })
        notice(`Removed ${link.title}`)
        return true
      } catch (e) {
        set((s) => {
          const cur = s.details[id]
          return cur ? { details: { ...s.details, [id]: { ...cur, links: before } } } : {}
        })
        notice(`Couldn't remove the link: ${errorText(e)}`)
        return false
      }
    },

    toggleSection(sec) {
      set((s) => ({ expanded: { ...s.expanded, [sec]: !s.expanded[sec] } }))
    },

    setMulti(on) {
      set({ multi: on, picked: [] })
    },

    togglePick(id) {
      set((s) => ({ multi: true, picked: s.picked.includes(id) ? s.picked.filter((x) => x !== id) : [...s.picked, id] }))
    },

    pickAll() {
      const now = Date.now()
      const { rows, filter } = get()
      set({ multi: true, picked: rows.filter((r) => (!r.is_archived || filter.archived) && sectionOf(r, now) !== 'snoozed').map((r) => r.id) })
    },

    async markRead(id, read, opts) {
      const before = rowOf(id)
      if (!before) return false
      if (read) set((s) => ({ tierMemory: { ...s.tierMemory, [id]: before.urgency_tier ?? s.tierMemory[id] ?? null } }))
      const patch: Partial<ThreadRow> = read
        ? { unread_count: 0, urgency_tier: null, section: undefined }
        : { unread_count: Math.max(1, before.unread_count), urgency_tier: before.urgency_tier ?? get().tierMemory[id] ?? null, section: undefined }
      patchRow(id, patch)
      try {
        const raw = await invoke<TriageResult | CoreTriageResult>('message_mark_read', { id, kind: 'thread', read })
        const r = normalizeTriage(raw, localReason(before, read ? 'mark_read' : 'mark_unread'))
        writeBack(id, r, read ? 'Marked read' : 'Marked unread', { silent: opts?.silent, undo: () => void get().markRead(id, !read, { silent: true }) })
        return true
      } catch (e) {
        restore(before)
        notice(`Couldn't mark ${read ? 'read' : 'unread'}: ${errorText(e)}`)
        return false
      }
    },

    async archive(id, archived = true) {
      const before = rowOf(id)
      if (!before) return false
      // Archive cancels a snooze (07 §6.1).
      patchRow(id, archived ? { is_archived: true, snoozed_until: null, section: undefined } : { is_archived: false, section: undefined })
      if (archived && get().selectedId === id) set({ selectedId: null, stick: null })
      try {
        const raw = await invoke<TriageResult | CoreTriageResult>('thread_archive', { id, archived })
        const r = normalizeTriage(raw, localReason(before, 'archive'))
        writeBack(id, r, archived ? 'Archived' : 'Moved back to the inbox', { undo: archived ? () => void get().archive(id, false) : undefined })
        return true
      } catch (e) {
        restore(before)
        notice(`Couldn't ${archived ? 'archive' : 'unarchive'}: ${errorText(e)}`)
        return false
      }
    },

    async snooze(id, preset, pick) {
      const before = rowOf(id)
      if (!before) return false
      const until = new Date(resolvePreset(preset, Date.now(), pick)).toISOString()
      patchRow(id, { snoozed_until: until, snooze_preset: preset, resurfaced: false, section: undefined })
      if (get().selectedId === id) set({ stick: null })
      try {
        const args: Record<string, unknown> = { id, preset }
        if (preset === 'pick_date') args.until = until
        const r = await invoke<SnoozeResult>('thread_snooze', args)
        if (r?.until) patchRow(id, { snoozed_until: r.until })
        offerUndo(`Snoozed until ${backLabel(r?.until ?? until, Date.now())}`, 'Undo', () => void get().unsnooze(id))
        return true
      } catch (e) {
        restore(before)
        notice(`Couldn't snooze: ${errorText(e)}`)
        return false
      }
    },

    async unsnooze(id) {
      const before = rowOf(id)
      if (!before) return false
      patchRow(id, { snoozed_until: null, snooze_preset: null, section: undefined })
      if (get().stick?.id === id) set({ stick: null })
      try {
        await invoke('thread_unsnooze', { id })
        return true
      } catch (e) {
        restore(before)
        notice(`Couldn't cancel the snooze: ${errorText(e)}`)
        return false
      }
    },

    async setTier(id, tier) {
      const before = rowOf(id)
      if (!before) return false
      patchRow(id, { urgency_tier: before.unread_count > 0 ? tier : before.urgency_tier, urgency_source: 'user', urgency_why: 'set by you', section: undefined })
      // A deliberate change of level moves the row, unlike read on open.
      if (get().stick?.id === id) set({ stick: null })
      set((s) => ({ tierMemory: { ...s.tierMemory, [id]: tier } }))
      try {
        // The level is read from the latest message (07 §3.5); the thread id stands in until the detail is loaded.
        const msgs = get().details[id]?.messages
        await invoke('message_set_tier', { id: msgs?.length ? msgs[msgs.length - 1]!.id : id, tier })
        notice(`Set to ${levelOf(tier)}`)
        return true
      } catch (e) {
        restore(before)
        notice(`Couldn't change the level: ${errorText(e)}`)
        return false
      }
    },

    async setVip(id, vip) {
      const before = rowOf(id)
      if (!before) return false
      const sameSender = (r: ThreadRow) => (before.sender_contact_id ? r.sender_contact_id === before.sender_contact_id : r.sender_address === before.sender_address)
      const flip = (on: boolean) =>
        set((s) => ({
          rows: s.rows.map((r) => (sameSender(r) ? { ...r, sender_vip: on } : r)),
          details: Object.fromEntries(
            Object.entries(s.details).map(([k, d]) => [k, sameSender(d.thread) ? { ...d, thread: { ...d.thread, sender_vip: on }, contact: d.contact ? { ...d.contact, vip: on } : d.contact } : d]),
          ),
        }))
      flip(vip)
      try {
        const contactId = before.sender_contact_id ?? get().details[id]?.contact?.id ?? null
        const name = get().details[id]?.contact?.display_name ?? before.sender_name
        if (vip) {
          const args = contactId
            ? { contact_id: contactId }
            : before.provider === 'slack'
              ? { slack_user: before.sender_address ?? undefined, display_name: before.sender_name }
              : { email: before.sender_address ?? undefined, display_name: before.sender_name }
          await invoke('vip_add', args)
          notice(`${name} is an important person`)
        } else {
          if (!contactId) throw new Error('the sender has no contact yet')
          await invoke('vip_remove', { contact_id: contactId })
          notice(`${name} is no longer an important person`)
        }
        return true
      } catch (e) {
        flip(!vip)
        notice(`Couldn't change the important people list: ${errorText(e)}`)
        return false
      }
    },

    async markAllRead() {
      const ids = get()
        .rows.filter((r) => r.unread_count > 0 && !r.is_archived && !isSnoozed(r, Date.now()))
        .map((r) => r.id)
      if (!ids.length) return
      const results = await Promise.all(ids.map((id) => get().markRead(id, true, { silent: true })))
      const n = results.filter(Boolean).length
      offerUndo(`Marked ${n} read`, 'Undo', () => ids.forEach((id) => void get().markRead(id, false, { silent: true })))
    },

    async bulk(action, preset) {
      const ids = get().picked
      if (!ids.length) return
      if (action === 'read') await Promise.all(ids.map((id) => get().markRead(id, true, { silent: true })))
      else if (action === 'archive') await Promise.all(ids.map((id) => get().archive(id)))
      else await Promise.all(ids.map((id) => get().snooze(id, preset ?? 'tomorrow')))
      set({ picked: [], multi: false })
      if (action === 'read') offerUndo(`Marked ${ids.length} read`, 'Undo', () => ids.forEach((id) => void get().markRead(id, false, { silent: true })))
    },

    undo() {
      const u = get().lastUndo
      if (!u || Date.now() - u.at > UNDO_MS) return
      set({ lastUndo: null })
      u.run()
      notice('Undone')
    },

    async resolveMessage(id) {
      const known = get().messageThreads[id]
      if (known) return known
      try {
        const raw = await invoke<ThreadDetail | CoreThreadDetail>('thread_get', { thread_id: id })
        if (raw) {
          const d = normalizeDetail(raw)
          const thread = d.thread.id
          if (thread && thread !== id) {
            set((s) => ({ messageThreads: { ...s.messageThreads, [id]: thread }, details: { ...s.details, [thread]: s.details[thread] ?? d } }))
            return thread
          }
        }
      } catch {
        /* the core does not answer a message id yet: the fallback below */
      }
      return id
    },

    applyPush(p) {
      pushedOnce = true
      const pairs = (p?.rows ?? []).filter((r) => r.thread_id).map((r) => [r.id, r.thread_id as string] as const)
      set((s) => ({ urgentCount: p?.urgent_count ?? 0, unreadInApp: p?.unread_in_app ?? 0, ...(pairs.length ? { messageThreads: { ...s.messageThreads, ...Object.fromEntries(pairs) } } : {}) }))
      // The push carries Q3 rows; the list itself is re-read (debounced) so the core's order stays the source.
      clearTimeout(pushTimer)
      pushTimer = setTimeout(() => {
        if (!get().loaded) return
        void get().load()
        const sel = get().selectedId
        if (sel) void get().loadDetail(sel)
      }, 250)
    },
  }
})

/* ---- labels ------------------------------------------------------------------------------ */

const WD = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'] as const
const MON = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'] as const

function hm(d: Date): string {
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

function dayDiff(a: Date, b: Date): number {
  const x = new Date(a.getFullYear(), a.getMonth(), a.getDate()).getTime()
  const y = new Date(b.getFullYear(), b.getMonth(), b.getDate()).getTime()
  return Math.round((x - y) / 86_400_000)
}

/** Row time (03 §4.2): `09:41` today, `yesterday`, `Mon` this week, `12 Oct` before. */
export function rowTime(iso: string, now: number): string {
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  const diff = dayDiff(new Date(now), d)
  if (diff <= 0) return hm(d)
  if (diff === 1) return 'yesterday'
  if (diff < 7) return WD[d.getDay()]!
  return `${d.getDate()} ${MON[d.getMonth()]}`
}

/** "back Tue 09:00" (Snoozed rows) / "Tue 21 Oct, 09:00" (the snooze preview). */
export function backLabel(iso: string, now: number): string {
  const d = new Date(iso)
  const diff = dayDiff(d, new Date(now))
  if (diff === 0) return `today ${hm(d)}`
  if (diff === 1) return `tomorrow ${hm(d)}`
  if (diff < 7) return `${WD[d.getDay()]} ${hm(d)}`
  return `${d.getDate()} ${MON[d.getMonth()]} ${hm(d)}`
}

export function previewLabel(ms: number): string {
  const d = new Date(ms)
  return `back ${WD[d.getDay()]} ${d.getDate()} ${MON[d.getMonth()]}, ${hm(d)}`
}

/** Neutral initials of `sender_name` (01 §6.2): first letters of the first and last word. */
export function initials(name: string): string {
  const words = name
    .replace(/[^\p{L}\p{N}\s]/gu, ' ')
    .split(/\s+/)
    .filter(Boolean)
  if (!words.length) return '?'
  const first = words[0]![0]!
  const last = words.length > 1 ? words[words.length - 1]![0]! : ''
  return (first + last).toUpperCase()
}

/** The source mark of a row (`google` → Gmail). */
export function sourceOf(provider: ThreadRow['provider']): 'gmail' | 'slack' {
  return provider === 'slack' ? 'slack' : 'gmail'
}

/** Line 1's chat pill: `#channel` for Slack channels and threads, "Inbox" for email, nothing for DMs (03 §4.2). */
export function chatPill(r: Pick<ThreadRow, 'kind' | 'channel_name'>): string | null {
  if (r.kind === 'email') return 'Inbox'
  if (r.kind === 'channel' || r.kind === 'channel_thread') return r.channel_name ? (r.channel_name.startsWith('#') ? r.channel_name : `#${r.channel_name}`) : null
  return null
}

/** Senders the Inbox has seen, for the contact filter and the important-people picker (no contacts command yet). */
export function knownSenders(rows: readonly ThreadRow[]): { key: string; contact_id: string | null; name: string; address: string | null }[] {
  const seen = new Map<string, { key: string; contact_id: string | null; name: string; address: string | null }>()
  for (const r of rows) {
    const key = r.sender_contact_id ?? r.sender_address ?? r.sender_name
    if (!seen.has(key)) seen.set(key, { key, contact_id: r.sender_contact_id ?? null, name: r.sender_name, address: r.sender_address ?? null })
  }
  return Array.from(seen.values())
}

/**
 * Subscribes the store to `inbox.changed` (counts at once, the list re-read
 * after a short debounce). Returns the unsubscribe function.
 */
export async function connectInbox(): Promise<() => void> {
  const off = await listen<InboxChanged>(EVENTS.inbox, (p) => useInbox.getState().applyPush(p))
  void seedUrgentCount()
  return off
}

/**
 * The sidebar's Inbox count before the first `inbox.changed` (the core pushes on change only): one
 * `inbox_list` at start; its `counts.urgent`, or the urgent rows it returned. A push that arrived meanwhile wins.
 */
export async function seedUrgentCount(now = Date.now()): Promise<number | null> {
  try {
    const res = await invoke<InboxListResult>('inbox_list', { filter: {}, limit: PAGE })
    if (pushedOnce) return null
    const n = typeof res?.counts?.urgent === 'number' ? res.counts.urgent : (res?.rows ?? []).map(normalizeThread).filter((r) => sectionOf(r, now) === 'urgent').length
    useInbox.setState({ urgentCount: n })
    return n
  } catch {
    return null
  }
}

/** Test hook: forget that a push arrived. */
export function resetInboxPushes() {
  pushedOnce = false
}
