// The Calendar view (03 §6; behavior owner 06 §7, read-only): the layout
// (day / week / month) and the date it shows, the calendar events of the
// visible range (`agenda_list {from, to}`), the agenda rail (today and
// tomorrow), the selection that fills the detail panel, and the pure helpers
// the grids draw from: visible days, the header line, the hour window with
// "+ earlier / + later", overlap columns, agenda buckets, the now-line.
import { create } from 'zustand'
import { invoke, listen } from '../tauri'
import { cmdErrorText, EVENTS, type AgendaChanged } from './contract'
import type { Attendee, CalendarEvent, CoreEventRow, ThreadLink } from './contract-connectors'
import { useSessionsStore } from './sessions-store'

export type CalLayout = 'day' | 'week' | 'month'
export type WeekStart = 'monday' | 'sunday' | 'saturday'

/* ---- dates (local time, `YYYY-MM-DD`) ----------------------------------------------- */

export const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'] as const
export const WD_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'] as const
const MON_SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'] as const

export function localDate(ms: number): string {
  const d = new Date(ms)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

/** Local midnight of a `YYYY-MM-DD`. */
export function dateStart(date: string): Date {
  const [y, m, d] = date.split('-').map(Number)
  return new Date(y!, m! - 1, d!)
}

export function addDays(date: string, n: number): string {
  const d = dateStart(date)
  d.setDate(d.getDate() + n)
  return localDate(d.getTime())
}

const START_DOW: Record<WeekStart, number> = { sunday: 0, monday: 1, saturday: 6 }

export function weekStartOf(date: string, weekStart: WeekStart = 'monday'): string {
  const dow = dateStart(date).getDay()
  const back = (dow - START_DOW[weekStart] + 7) % 7
  return addDays(date, -back)
}

/** The weekday heads in the week's order. */
export function weekdayHeads(weekStart: WeekStart = 'monday'): string[] {
  const s = START_DOW[weekStart]
  return Array.from({ length: 7 }, (_, i) => WD_SHORT[(s + i) % 7]!)
}

/** Days drawn by a layout: one, seven, or the month's whole weeks. */
export function visibleDays(layout: CalLayout, anchor: string, weekStart: WeekStart = 'monday'): string[] {
  if (layout === 'day') return [anchor]
  if (layout === 'week') {
    const s = weekStartOf(anchor, weekStart)
    return Array.from({ length: 7 }, (_, i) => addDays(s, i))
  }
  const first = anchor.slice(0, 8) + '01'
  const start = weekStartOf(first, weekStart)
  const d = dateStart(first)
  const last = localDate(new Date(d.getFullYear(), d.getMonth() + 1, 0).getTime())
  const days: string[] = []
  for (let cur = start; ; cur = addDays(cur, 1)) {
    days.push(cur)
    if (cur >= last && days.length % 7 === 0) break
  }
  return days
}

/** `agenda_list` bounds: local midnight of the first day to local midnight after the last (UTC instants). */
export function rangeOf(days: readonly string[]): { from: string; to: string } {
  return { from: dateStart(days[0]!).toISOString(), to: dateStart(addDays(days[days.length - 1]!, 1)).toISOString() }
}

/** The header's left line (03 §6.1): "October 2026 · 19–25", "October 2026 · Wed 21", "October 2026". */
export function headerTitle(layout: CalLayout, anchor: string, weekStart: WeekStart = 'monday'): string {
  const a = dateStart(anchor)
  const month = `${MONTHS[a.getMonth()]} ${a.getFullYear()}`
  if (layout === 'month') return month
  if (layout === 'day') return `${month} · ${WD_SHORT[a.getDay()]} ${a.getDate()}`
  const days = visibleDays('week', anchor, weekStart)
  const s = dateStart(days[0]!)
  const e = dateStart(days[6]!)
  if (s.getMonth() === e.getMonth()) return `${MONTHS[s.getMonth()]} ${s.getFullYear()} · ${s.getDate()}–${e.getDate()}`
  const left = s.getFullYear() === e.getFullYear() ? MONTHS[s.getMonth()] : `${MONTHS[s.getMonth()]} ${s.getFullYear()}`
  return `${left} – ${MONTHS[e.getMonth()]} ${e.getFullYear()} · ${s.getDate()}–${e.getDate()}`
}

/** Anchor after ‹ / ›: one day, week or month. */
export function stepAnchor(layout: CalLayout, anchor: string, dir: 1 | -1): string {
  if (layout === 'day') return addDays(anchor, dir)
  if (layout === 'week') return addDays(anchor, 7 * dir)
  const d = dateStart(anchor)
  return localDate(new Date(d.getFullYear(), d.getMonth() + dir, 1).getTime())
}

/* ---- times -------------------------------------------------------------------------- */

function hm(d: Date): string {
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

/** "14:00" (agenda rail, mono). */
export function clock(iso: string): string {
  return hm(new Date(iso))
}

/** "14:00–14:30" (chips, detail panel). */
export function timeRange(e: Pick<CalendarEvent, 'starts_at' | 'ends_at'>): string {
  return `${clock(e.starts_at)}–${clock(e.ends_at)}`
}

/** "Wed 21 Oct · 11:00–12:00" plus the event's zone when it is not the device's (03 §6.6). */
export function detailTime(e: CalendarEvent, deviceZone?: string): string {
  const d = new Date(e.starts_at)
  const day = `${WD_SHORT[d.getDay()]} ${d.getDate()} ${MON_SHORT[d.getMonth()]}`
  if (e.all_day) return `${day} · all day`
  const zone = deviceZone ?? safeZone()
  return `${day} · ${timeRange(e)}${e.timezone && zone && e.timezone !== zone ? ` · ${e.timezone}` : ''}`
}

function safeZone(): string | undefined {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone
  } catch {
    return undefined
  }
}

/** Minutes from the day's local midnight, clipped to [0, 1440]. */
export function minutesIn(iso: string, date: string): number {
  const t = Date.parse(iso)
  const m = (t - dateStart(date).getTime()) / 60_000
  return Math.max(0, Math.min(1440, m))
}

/** All-day events of a day (Google's exclusive end) and timed events touching it. */
export function eventsOn(events: readonly CalendarEvent[], date: string, opts: { declined?: boolean } = {}): { allDay: CalendarEvent[]; timed: CalendarEvent[]; declined: CalendarEvent[] } {
  const start = dateStart(date).getTime()
  const end = dateStart(addDays(date, 1)).getTime()
  const allDay: CalendarEvent[] = []
  const timed: CalendarEvent[] = []
  const declined: CalendarEvent[] = []
  for (const e of events) {
    const s = Date.parse(e.starts_at)
    const en = Date.parse(e.ends_at)
    const touches = e.all_day ? s < end && en > start : s < end && (en > start || (en === s && s >= start))
    if (!touches) continue
    if (e.my_response === 'declined') {
      declined.push(e)
      if (!opts.declined) continue
    }
    if (e.all_day) allDay.push(e)
    else timed.push(e)
  }
  const byStart = (a: CalendarEvent, b: CalendarEvent) => Date.parse(a.starts_at) - Date.parse(b.starts_at) || Date.parse(b.ends_at) - Date.parse(a.ends_at)
  return { allDay: allDay.sort(byStart), timed: timed.sort(byStart), declined }
}

export interface Slot {
  /** Column inside the overlap group, 0-based. */
  col: number
  /** Columns the group shares the day width between. */
  cols: number
}

/**
 * Overlap layout (03 §6.2): events that overlap share the column width
 * equally, in `starts_at` order. Transitively overlapping events form one
 * group; each takes the first free column; the group's width is its column count.
 */
export function overlapSlots(timed: readonly CalendarEvent[], date: string): Map<string, Slot> {
  const items = timed
    .map((e) => ({ id: e.id, s: minutesIn(e.starts_at, date), e: Math.max(minutesIn(e.ends_at, date), minutesIn(e.starts_at, date) + 15) }))
    .sort((a, b) => a.s - b.s || b.e - a.e)
  const out = new Map<string, Slot>()
  let group: { id: string; col: number }[] = []
  let groupEnd = -1
  let colEnds: number[] = []
  const flush = () => {
    const cols = colEnds.length || 1
    for (const g of group) out.set(g.id, { col: g.col, cols })
    group = []
    colEnds = []
    groupEnd = -1
  }
  for (const it of items) {
    if (group.length && it.s >= groupEnd) flush()
    let col = colEnds.findIndex((end) => end <= it.s)
    if (col < 0) {
      col = colEnds.length
      colEnds.push(it.e)
    } else colEnds[col] = it.e
    group.push({ id: it.id, col })
    groupEnd = Math.max(groupEnd, it.e)
  }
  if (group.length) flush()
  return out
}

export interface HourWindow {
  start: number
  /** Last hour row drawn (inclusive). */
  end: number
  earlier: boolean
  later: boolean
}

/** 07:00–19:00 by default; widened to every event shown; "+ earlier / + later" open to 00 / 23 for the visit (03 §6.2). */
export function hourWindow(events: readonly CalendarEvent[], days: readonly string[], expanded: { earlier: boolean; later: boolean }, def = { start: 7, end: 19 }): HourWindow {
  let start = expanded.earlier ? 0 : def.start
  let end = expanded.later ? 23 : def.end
  for (const d of days) {
    for (const e of eventsOn(events, d).timed) {
      const s = minutesIn(e.starts_at, d)
      const en = minutesIn(e.ends_at, d)
      start = Math.min(start, Math.floor(s / 60))
      end = Math.max(end, Math.min(23, Math.ceil(en / 60) - 1))
    }
  }
  return { start, end, earlier: start > 0, later: end < 23 }
}

/* ---- the agenda rail (03 §6.4) ------------------------------------------------------- */

export interface AgendaBuckets {
  today: CalendarEvent[]
  tomorrow: CalendarEvent[]
}

/** Today and Tomorrow: all-day first, then by `starts_at`; declined events left out. */
export function agendaBuckets(events: readonly CalendarEvent[], now: number): AgendaBuckets {
  const today = localDate(now)
  const pick = (date: string) => {
    const { allDay, timed } = eventsOn(events, date)
    // A timed event belongs to the day it starts on.
    return [...allDay, ...timed.filter((e) => localDate(Date.parse(e.starts_at)) === date)]
  }
  return { today: pick(today), tomorrow: pick(addDays(today, 1)) }
}

/** The overlay's "next event": the first future row of Today (no extra mark, 03 §6.4). */
export function nextEvent(events: readonly CalendarEvent[], now: number): CalendarEvent | null {
  return agendaBuckets(events, now).today.find((e) => !e.all_day && Date.parse(e.starts_at) > now) ?? null
}

/** Timed events starting within two hours (the sidebar count when no push has said). */
export function next2h(events: readonly CalendarEvent[], now: number): number {
  return events.filter((e) => !e.all_day && e.my_response !== 'declined' && e.status !== 'cancelled' && Date.parse(e.starts_at) > now && Date.parse(e.starts_at) <= now + 2 * 3_600_000).length
}

/* ---- the core's EventRow → the UI's CalendarEvent ---------------------------------------- */

const RESPONSE: Record<string, Attendee['response']> = { accepted: 'accepted', tentative: 'tentative', declined: 'declined', needsAction: 'needs_action', needs_action: 'needs_action' }

/** Attendees as the UI keeps them, or as Google sends them (`displayName`, `responseStatus`, `organizer`, `self`). */
export function normalizeAttendees(v: unknown): Attendee[] {
  if (!Array.isArray(v)) return []
  return v.map((a) => {
    const o = (a ?? {}) as Record<string, unknown>
    const email = (o.email as string | undefined) ?? null
    return {
      name: String(o.name ?? o.displayName ?? email ?? ''),
      email,
      response: RESPONSE[String(o.response ?? o.responseStatus ?? 'needs_action')] ?? 'needs_action',
      organizer: !!o.organizer,
      self: !!o.self,
    }
  })
}

/** An `agenda_list` row: the UI's `CalendarEvent`, or the core's `EventRow` (`account_label`, raw attendees). */
export function normalizeEvent(raw: CalendarEvent | CoreEventRow): CalendarEvent {
  const core = raw as CoreEventRow & { account_name?: string | null }
  return {
    ...(raw as CalendarEvent),
    account_name: core.account_name ?? core.account_label ?? null,
    attendees: normalizeAttendees(core.attendees),
  }
}

/* ---- the store -------------------------------------------------------------------------- */

export interface CalendarState {
  layout: CalLayout
  anchor: string
  events: CalendarEvent[]
  /** The loaded range key (`from|to`). */
  rangeKey: string | null
  status: 'idle' | 'loading' | 'ready' | 'error'
  error: string | null
  agenda: CalendarEvent[]
  agendaLoaded: boolean
  selectedId: string | null
  expanded: { earlier: boolean; later: boolean }
  /** Days whose declined events are shown ("+n declined"). */
  declinedOpen: string[]
  /** `agenda.changed.next_2h_count` (the sidebar count). */
  next2h: number | null
  /** `event_links` per event (03 §6.6), loaded when the panel opens. */
  links: Record<string, ThreadLink[]>
  loadLinks(id: string): Promise<void>
  /** Link the event to a session, note or message by hand (`item_link_add`). */
  linkAdd(id: string, to: Pick<ThreadLink, 'kind' | 'id' | 'title'>): Promise<boolean>
  /** Remove a link or dismiss a suggestion (`item_link_remove`). */
  linkRemove(id: string, link: ThreadLink): Promise<boolean>
  setLayout(l: CalLayout): void
  setAnchor(date: string, layout?: CalLayout): void
  today(): void
  step(dir: 1 | -1): void
  select(id: string | null): void
  expand(which: 'earlier' | 'later' | 'both'): void
  toggleDeclined(date: string): void
  load(weekStart?: WeekStart): Promise<void>
  loadAgenda(): Promise<void>
  /** Prepare notes → the note (03 §6.7); returns its id. */
  prepare(id: string): Promise<string | null>
  join(id: string): Promise<void>
  applyPush(p: AgendaChanged): void
}

function errorText(e: unknown): string {
  return cmdErrorText(e)
}

let weekStartPref: WeekStart = 'monday'
let pushTimer: ReturnType<typeof setTimeout> | undefined
let loadSeq = 0

function targetOf(kind: ThreadLink['kind']): 'session' | 'note' | 'message' {
  return kind === 'thread' ? 'message' : kind
}

export const useCalendar = create<CalendarState>((set, get) => ({
  links: {},

  async loadLinks(id) {
    try {
      const out = await invoke<ThreadLink[]>('event_links', { id })
      if (Array.isArray(out)) set((s) => ({ links: { ...s.links, [id]: out } }))
    } catch {
      // The event is gone or the core is older: the panel keeps what the row carried.
    }
  },

  async linkAdd(id, to) {
    try {
      const out = await invoke<ThreadLink[]>('item_link_add', { kind: 'calendar_event', id, to_kind: targetOf(to.kind), to_id: to.id })
      if (Array.isArray(out)) set((s) => ({ links: { ...s.links, [id]: out } }))
      useSessionsStore.getState().showNotice(`Linked to ${to.title}`)
      return true
    } catch (e) {
      useSessionsStore.getState().showNotice(`Couldn't add the link: ${errorText(e)}`)
      return false
    }
  },

  async linkRemove(id, link) {
    const before = get().links[id]
    if (before) set((s) => ({ links: { ...s.links, [id]: before.filter((l) => !(l.kind === link.kind && l.id === link.id)) } }))
    try {
      const out = await invoke<ThreadLink[]>('item_link_remove', { kind: 'calendar_event', id, to_kind: targetOf(link.kind), to_id: link.id })
      if (Array.isArray(out)) set((s) => ({ links: { ...s.links, [id]: out } }))
      useSessionsStore.getState().showNotice(`Removed ${link.title}`)
      return true
    } catch (e) {
      if (before) set((s) => ({ links: { ...s.links, [id]: before } }))
      useSessionsStore.getState().showNotice(`Couldn't remove the link: ${errorText(e)}`)
      return false
    }
  },

  layout: 'week',
  anchor: localDate(Date.now()),
  events: [],
  rangeKey: null,
  status: 'idle',
  error: null,
  agenda: [],
  agendaLoaded: false,
  selectedId: null,
  expanded: { earlier: false, later: false },
  declinedOpen: [],
  next2h: null,

  setLayout(layout) {
    set({ layout })
    void get().load()
  },

  setAnchor(anchor, layout) {
    set({ anchor, layout: layout ?? get().layout, expanded: { earlier: false, later: false } })
    void get().load()
  },

  today() {
    get().setAnchor(localDate(Date.now()))
  },

  step(dir) {
    const { layout, anchor } = get()
    get().setAnchor(stepAnchor(layout, anchor, dir))
  },

  select(id) {
    set({ selectedId: id })
  },

  expand(which) {
    set((s) => ({ expanded: { earlier: which !== 'later' ? true : s.expanded.earlier, later: which !== 'earlier' ? true : s.expanded.later } }))
  },

  toggleDeclined(date) {
    set((s) => ({ declinedOpen: s.declinedOpen.includes(date) ? s.declinedOpen.filter((d) => d !== date) : [...s.declinedOpen, date] }))
  },

  async load(weekStart) {
    if (weekStart) weekStartPref = weekStart
    const { layout, anchor } = get()
    const range = rangeOf(visibleDays(layout, anchor, weekStartPref))
    const key = `${range.from}|${range.to}`
    const seq = ++loadSeq
    set({ status: 'loading', rangeKey: key })
    try {
      const events = await invoke<(CalendarEvent | CoreEventRow)[]>('agenda_list', range)
      if (seq !== loadSeq) return
      set({ events: (events ?? []).map(normalizeEvent), status: 'ready', error: null })
    } catch (e) {
      if (seq !== loadSeq) return
      set({ status: 'error', error: errorText(e) })
    }
  },

  async loadAgenda() {
    const today = localDate(Date.now())
    try {
      const rows = await invoke<(CalendarEvent | CoreEventRow)[]>('agenda_list', rangeOf([today, addDays(today, 1)]))
      set({ agenda: (rows ?? []).map(normalizeEvent), agendaLoaded: true })
    } catch {
      set({ agendaLoaded: true })
    }
  },

  async prepare(id) {
    try {
      const r = await invoke<{ note_id: string }>('event_prepare_notes', { id })
      const patch = (rows: CalendarEvent[]) => rows.map((e) => (e.id === id ? { ...e, prepare_note_id: r.note_id } : e))
      set((s) => ({ events: patch(s.events), agenda: patch(s.agenda) }))
      void get().loadLinks(id)
      return r.note_id
    } catch (e) {
      useSessionsStore.getState().showNotice(`Couldn't prepare notes: ${errorText(e)}`)
      return null
    }
  },

  async join(id) {
    try {
      const url = await invoke<string>('event_join', { id })
      if (url) await invoke('plugin:opener|open_url', { url })
    } catch (e) {
      useSessionsStore.getState().showNotice(`Couldn't open the call: ${errorText(e)}`)
    }
  },

  applyPush(p) {
    set({ next2h: p?.next_2h_count ?? 0 })
    clearTimeout(pushTimer)
    pushTimer = setTimeout(() => {
      if (get().rangeKey) void get().load()
      if (get().agendaLoaded) void get().loadAgenda()
    }, 250)
  },
}))

/** The selected event, from the grid's range or the rail. */
export function findEvent(s: Pick<CalendarState, 'events' | 'agenda'>, id: string | null): CalendarEvent | null {
  if (!id) return null
  return s.events.find((e) => e.id === id) ?? s.agenda.find((e) => e.id === id) ?? null
}

/** Subscribes the store to `agenda.changed`, then seeds the sidebar count once (`seedNext2h`). Returns the unsubscribe function. */
export async function connectCalendar(): Promise<() => void> {
  const off = await listen<AgendaChanged>(EVENTS.agenda, (p) => useCalendar.getState().applyPush(p))
  void seedNext2h()
  return off
}

/**
 * The sidebar's Calendar count before the first `agenda.changed` (the core pushes on change only):
 * one `agenda_list` over the next two hours at start. A push that arrived meanwhile wins.
 */
export async function seedNext2h(now = Date.now()): Promise<number | null> {
  try {
    const rows = await invoke<(CalendarEvent | CoreEventRow)[]>('agenda_list', { from: new Date(now).toISOString(), to: new Date(now + 2 * 3_600_000).toISOString() })
    if (useCalendar.getState().next2h !== null) return null
    const n = next2h((rows ?? []).map(normalizeEvent), now)
    useCalendar.setState({ next2h: n })
    return n
  } catch {
    return null
  }
}
