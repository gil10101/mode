// The pill model (02 §5): what `compact` draws for a Q1 candidate, built from
// `pill.changed` plus the pushed rows, and the local ladder used until the
// core has pushed a pill. Phrase parts are in drop order: the state word and
// the name are kept, later parts go first when the width runs out.
import type { AgendaRow, InboxRow, OverlayTab, PillCandidate, PillChanged, RosterRow } from '../state/contract'
import { tabOfCandidate, type Dictation, type Flash } from './machine'
import { elapsed, startsLabel, waitingPillDetail } from './phrases'

export type PillActionId = 'allow' | 'deny' | 'always' | 'answer' | 'jump' | 'open_in_app' | 'view' | 'dismiss' | 'open_source' | 'read' | 'snooze' | 'join' | 'prepare' | 'open_dictation' | 'open_flash'

export interface PillAction {
  id: PillActionId
  label: string
  /** The one accent use (Allow, Answer). */
  accent?: boolean
}

export interface PillModel {
  kind: PillCandidate['kind'] | 'none' | 'dictation' | 'flash'
  ref: string | null
  tab: OverlayTab
  /** Phrase parts in drop order; `parts[0]` and `parts[1]` are always kept. */
  parts: string[]
  /** Rendered as `.m-diff` in place of `parts[diffAt]`. */
  diff?: { add: number; del: number; at: number }
  primary: PillAction | null
  menu: PillAction[]
  count: number
  /** `assertive` for a waiting session (02 §13). */
  live: 'polite' | 'assertive'
  /** The screen-reader sentence. */
  label: string
  /** The row the pill is about, when it is a session. */
  row?: RosterRow
  /** Seconds behind the elapsed part, when the phrase has one: under an hour it carries seconds and the pill repaints once a second (02 §15). */
  seconds?: number
  /** Dictation (02 §5.6): the input level 0..1 drawn as the meter while listening. */
  meter?: number
  /** Dictation: the kit spinner while transcribing or thinking. */
  busy?: boolean
  /** Dictation: what `Open` opens (the captured note, or the Chat conversation that answered); a flash: its `open`. */
  open?: { view: string; id?: string }
  /** A flash from the app window: its kind, drawn as the kit's `.m-flash-glyph` before the phrase (02 §5.4). */
  glyph?: 'ok' | 'info' | 'bad'
}

export interface PillContext {
  rows: RosterRow[]
  inbox?: InboxRow[]
  agenda?: AgendaRow[]
  now: number
  /** When `rows` were pushed: `state_seconds` is as of then and the label runs on from it. */
  rowsAt?: number
  /** Dictation in progress or just finished: its pill comes first (02 §5.6). */
  dictation?: Dictation | null
}

/** A row's seconds in its state at `now` (the pushed snapshot plus the time since the push). */
function liveSeconds(row: RosterRow, ctx: PillContext): number {
  return row.state_seconds + (ctx.rowsAt ? Math.max(0, (ctx.now - ctx.rowsAt) / 1000) : 0)
}

function secondsSinceIso(iso: string | null | undefined, now: number): number | undefined {
  if (!iso) return undefined
  const t = Date.parse(iso)
  return Number.isNaN(t) ? undefined : Math.max(0, (now - t) / 1000)
}

/** "api-refactor, docs-site", then "+N" past two names (02 §5.2). */
export function namesList(names: string[]): string {
  const head = names.slice(0, 2).join(', ')
  return names.length > 2 ? `${head} +${names.length - 2}` : head
}

const JUMP: PillAction = { id: 'jump', label: 'Jump to terminal' }
const OPEN_APP: PillAction = { id: 'open_in_app', label: 'Open in app' }

/** "0:04", "1:12": the time held while listening. */
export function heldLabel(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`
}

function wordsLabel(n: number): string {
  return `${n} ${n === 1 ? 'word' : 'words'}`
}

const OPEN_DICTATION: PillAction = { id: 'open_dictation', label: 'Open' }

/** The dictation pill (02 §5.6): listening (with the meter), transcribing, thinking, then where the words went, or the reason it failed. */
export function dictationPill(d: Dictation): PillModel {
  const base = { kind: 'dictation' as const, ref: null, tab: 'notes' as OverlayTab, count: 0, menu: [], live: 'polite' as const, primary: null }
  switch (d.state) {
    case 'listening': {
      const parts = ['listening', heldLabel(d.seconds)]
      return { ...base, parts, meter: d.level, label: `Dictation listening, ${d.seconds} seconds` }
    }
    case 'transcribing':
      return { ...base, parts: ['transcribing'], busy: true, label: 'Dictation transcribing' }
    case 'thinking':
    case 'answering':
      return { ...base, parts: ['thinking', 'Mode'], busy: true, label: 'Mode is working on the request' }
    case 'done': {
      if (d.target === 'chat') {
        const parts = ['Mode answered', d.text?.trim() || 'in Chat']
        return { ...base, tab: 'sessions', parts, primary: d.threadId ? OPEN_DICTATION : null, open: d.threadId ? { view: 'chat', id: d.threadId } : undefined, label: `Mode answered: ${parts[1]}` }
      }
      if (d.target === 'note') {
        return { ...base, parts: ['captured', 'note'], primary: d.noteId ? OPEN_DICTATION : null, open: d.noteId ? { view: 'notes', id: d.noteId } : undefined, label: 'Dictation captured as a note' }
      }
      if (d.target === 'field') {
        const parts = ['typed', wordsLabel(d.words ?? 0)]
        return { ...base, parts, label: `Dictation typed ${parts[1]}` }
      }
      return { ...base, parts: ['dictation', 'nothing to type into'], label: 'Dictation: nothing to type into' }
    }
    case 'error': {
      const reason = d.error?.trim() || 'something went wrong'
      return { ...base, parts: ['dictation', reason], label: `Dictation failed: ${reason}` }
    }
  }
}

const OPEN_FLASH: PillAction = { id: 'open_flash', label: 'Open' }

/**
 * A flash from the app window (02 §5.4), drawn over whatever the overlay shows for its hold: the
 * kind's glyph, the text as phrase parts ("captured · note"), Open when it names an item, and the
 * spinner while it holds for the pill that follows ("thinking · Mode", "launching · …").
 */
export function flashPill(f: Flash): PillModel | null {
  if (!f.app) return null
  const parts = f.text.split(' · ').filter(Boolean)
  const open = f.app.open
  return {
    kind: 'flash',
    ref: null,
    tab: 'sessions',
    parts: parts.length ? parts : [f.text],
    primary: open ? OPEN_FLASH : null,
    menu: [],
    count: 0,
    live: 'polite',
    label: f.text,
    glyph: f.app.kind,
    ...(open ? { open } : {}),
    ...(f.keepPill && f.app.kind === 'info' ? { busy: true } : {}),
  }
}

export function buildPill(p: PillChanged | null, ctx: PillContext): PillModel | null {
  if (ctx.dictation) return dictationPill(ctx.dictation)
  const c = p?.candidate
  if (!c) return null
  const count = p?.count ?? 0
  const base = { kind: c.kind, ref: c.ref, tab: tabOfCandidate(c), count }
  switch (c.kind) {
    case 'session_waiting': {
      const row = ctx.rows.find((r) => r.id === c.ref)
      const detail = row ? waitingPillDetail(row) : null
      const parts = ['waiting', row?.display_name ?? c.label]
      if (detail) parts.push(detail)
      const kind = row?.pending?.kind ?? row?.waiting_kind
      const seconds = row ? liveSeconds(row, ctx) : secondsSinceIso(c.at, ctx.now)
      if (seconds !== undefined) parts.push(elapsed(seconds))
      const question = kind === 'question' ? (row?.pending?.questions?.[0]?.question ?? row?.pending?.summary ?? null) : null
      const answerable = row?.pending?.answerable !== false
      let primary: PillAction | null
      let menu: PillAction[]
      if (kind === 'permission' && answerable) {
        primary = { id: 'allow', label: 'Allow', accent: true }
        menu = [{ id: 'deny', label: 'Deny' }, { id: 'always', label: 'Always' }, JUMP, OPEN_APP]
      } else if (kind === 'question' && answerable) {
        primary = { id: 'answer', label: 'Answer', accent: true }
        menu = [JUMP, OPEN_APP]
      } else {
        primary = JUMP
        menu = [OPEN_APP]
      }
      const said = question ? [question, ...parts.slice(3)] : parts.slice(2)
      return { ...base, parts, primary, menu, live: 'assertive', label: `Session ${parts[1]}, waiting for you, ${said.join(', ')}`, row, seconds }
    }
    case 'urgent_message': {
      const m = ctx.inbox?.find((r) => r.id === c.ref)
      const seconds = m ? secondsSinceIso(m.received_at, ctx.now) : undefined
      const parts = m
        ? ['urgent', m.sender_name, m.channel_name ? `#${m.channel_name}` : 'Inbox', (m.subject ?? m.snippet ?? '').slice(0, 40), seconds === undefined ? '' : elapsed(seconds)]
        : ['urgent', c.label]
      return {
        seconds,
        ...base,
        parts: parts.filter(Boolean),
        primary: { id: 'open_source', label: 'Open' },
        menu: [{ id: 'read', label: 'Mark read' }, { id: 'snooze', label: 'Snooze' }, OPEN_APP],
        live: 'polite',
        label: `Urgent message, ${parts.slice(1).join(', ')}`,
      }
    }
    case 'session_finished': {
      const row = ctx.rows.find((r) => r.id === c.ref)
      const failed = row ? row.state === 'failed' : c.at === '0000'
      const name = row?.display_name ?? c.label
      if (failed) {
        const seconds = row ? liveSeconds(row, ctx) : secondsSinceIso(c.at, ctx.now)
        const since = seconds === undefined ? '' : elapsed(seconds)
        const parts = ['failed', name, row?.end_reason || row?.state_reason || 'ended with an error', since].filter(Boolean)
        return { ...base, parts, primary: { id: 'view', label: 'View' }, menu: [{ id: 'dismiss', label: 'Dismiss' }], live: 'polite', label: `Session ${name} failed, ${parts.slice(2).join(', ')}`, row, seconds }
      }
      // "finished · api-refactor · +86 −4 · 4 files": the file count, not a clock (02 §5.2).
      const files = row?.files_changed ?? 0
      const parts = ['finished', name, '']
      if (files > 0) parts.push(`${files} ${files === 1 ? 'file' : 'files'}`)
      return {
        ...base,
        parts,
        diff: { add: row?.lines_added ?? 0, del: row?.lines_removed ?? 0, at: 2 },
        primary: { id: 'view', label: 'View' },
        menu: [{ id: 'dismiss', label: 'Dismiss' }],
        live: 'polite',
        label: `Session ${name} finished, +${row?.lines_added ?? 0} −${row?.lines_removed ?? 0}${files > 0 ? `, ${parts[3]}` : ''}`,
        row,
      }
    }
    case 'active_count': {
      // One session: "working · api-refactor · pytest · 2m 04s"; several:
      // "2 working · api-refactor, docs-site". Idle sessions are named only
      // when none is working (they still make the candidate qualify, 02 §5.1).
      const working = ctx.rows.filter((r) => r.state === 'working')
      const group = working.length ? working : ctx.rows.filter((r) => r.state === 'idle')
      const word = working.length ? 'working' : 'idle'
      if (!group.length) {
        const parts = [`${Number(c.label) || 0} working`]
        return { ...base, parts, primary: null, menu: [], live: 'polite', label: parts.join(', ') }
      }
      if (group.length === 1) {
        const r = group[0]
        const seconds = liveSeconds(r, ctx)
        const parts = [word, r.display_name]
        if (r.state === 'working' && r.current_tool) parts.push(r.current_tool)
        parts.push(elapsed(seconds))
        return { ...base, parts, primary: null, menu: [], live: 'polite', label: `Session ${r.display_name} ${word}, ${parts.slice(2).join(', ')}`, row: r, seconds }
      }
      const parts = [`${group.length} ${word}`, namesList(group.map((r) => r.display_name))]
      return { ...base, parts, primary: null, menu: [], live: 'polite', label: `${parts[0]}: ${group.map((r) => r.display_name).join(', ')}` }
    }
    case 'next_event': {
      const e = ctx.agenda?.find((r) => r.id === c.ref)
      const when = c.at ? startsLabel(c.at, ctx.now) : ''
      const parts = [when || 'next', e?.title ?? c.label]
      const primary: PillAction = e?.join_url ? { id: 'join', label: 'Join' } : { id: 'open_source', label: 'Open' }
      return { ...base, parts, primary, menu: [{ id: 'prepare', label: 'Prepare notes' }, OPEN_APP], live: 'polite', label: `Event ${parts[1]}, ${parts[0]}` }
    }
    default:
      return { ...base, parts: [c.label], primary: null, menu: [], live: 'polite', label: c.label }
  }
}

/** The resolved-flash text of an inline action (02 §5.5). */
export function flashText(id: PillActionId | 'acknowledge'): string | null {
  switch (id) {
    case 'allow':
      return 'Allowed ✓'
    case 'deny':
      return 'Denied'
    case 'always':
      return 'Always allowed ✓'
    case 'answer':
      return 'Answered ✓'
    case 'read':
      return 'Read ✓'
    case 'snooze':
      return 'Snoozed until tomorrow'
    case 'view':
    case 'acknowledge':
      return 'Marked done ✓'
    case 'dismiss':
      return 'Dismissed'
    default:
      return null
  }
}

const WAITING_RANK: Record<string, number> = { permission: 0, question: 1 }

/** Q1 computed in the webview from the pushed rows: the fallback until the core pushes `pill.changed` (02 §5.1). */
export function localLadder(rows: RosterRow[], now = Date.now(), inbox: InboxRow[] = [], agenda: AgendaRow[] = []): PillChanged {
  const top = rows.filter((r) => r.state === 'waiting')
  if (top.length) {
    const sorted = [...top].sort((a, b) => b.state_seconds - a.state_seconds || (WAITING_RANK[a.waiting_kind ?? ''] ?? 2) - (WAITING_RANK[b.waiting_kind ?? ''] ?? 2))
    return { candidate: candidateOf(1, 'session_waiting', sorted[0], now), count: sorted.length - 1 }
  }
  const urgent = inbox.filter((m) => !m.is_read && (m.urgency_tier === 'vip' || m.urgency_tier === 'direct' || m.urgency_tier === 'ai-urgent'))
  if (urgent.length) {
    const m = [...urgent].sort((a, b) => a.received_at.localeCompare(b.received_at))[0]
    return { candidate: { priority: 2, kind: 'urgent_message', ref: m.id, label: `${m.sender_name}: ${m.subject ?? m.snippet ?? ''}`, at: m.received_at }, count: urgent.length - 1 }
  }
  const finished = rows.filter((r) => r.state === 'failed' || r.state === 'unread')
  if (finished.length) {
    const sorted = [...finished].sort((a, b) => Number(b.state === 'failed') - Number(a.state === 'failed') || b.state_seconds - a.state_seconds)
    const c = candidateOf(3, 'session_finished', sorted[0], now)
    return { candidate: sorted[0].state === 'failed' ? { ...c, at: '0000' } : c, count: finished.length - 1 }
  }
  const live = rows.filter((r) => r.state === 'working' || r.state === 'idle')
  if (live.length) return { candidate: { priority: 4, kind: 'active_count', ref: null, label: String(live.length), at: null }, count: 0 }
  const soon = agenda.filter((e) => e.status !== 'cancelled' && e.my_response !== 'declined' && within2h(e.starts_at, now))
  if (soon.length) {
    const e = [...soon].sort((a, b) => a.starts_at.localeCompare(b.starts_at))[0]
    return { candidate: { priority: 5, kind: 'next_event', ref: e.id, label: e.title, at: e.starts_at }, count: soon.length - 1 }
  }
  return { candidate: null, count: 0 }
}

function within2h(iso: string, now: number): boolean {
  const t = Date.parse(iso)
  return !Number.isNaN(t) && t > now && t <= now + 2 * 3_600_000
}

function candidateOf(priority: number, kind: string, row: RosterRow, now: number): PillCandidate {
  return { priority, kind, ref: row.id, label: row.display_name, at: new Date(now - row.state_seconds * 1000).toISOString() }
}
