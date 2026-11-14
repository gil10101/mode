// Every string the Stats view draws (03 §7A), as data: the tiles, the note,
// the table columns and cells. StatsView renders them with the kit; the
// wireframe generator (wireframe.ts) renders the same data as HTML, so the
// app and docs/wireframes/app-window.html cannot drift apart.
import type { StatsColumn } from '../../kit/stats'
import type { SessionStateCount, StatsSummary } from '../../state/contract-settings'
import { tokens } from '../sessions/format'
import { count, duration, hourLabel, money, pct, shortModel, sourceLabel } from './format'
import { bars, hourBars, MAX_WEEKS, splitText, type Bars, type HourBars, type StatsWindow, type WeekStart } from './tracker'

export interface Tile {
  value: string
  label: string
  title?: string
  hero?: boolean
}

/** The spend row (03 §7A.1): the window as the hero figure, then today, this month, all time. */
export function spendTiles(s: StatsSummary | null, windowLabel: string): Tile[] {
  const sp = s?.spend
  return [
    { value: money(sp ? sp.window : 0), label: `${windowLabel} · estimated at list price`, hero: true },
    { value: money(sp ? sp.today : 0), label: 'today' },
    { value: money(sp ? sp.month : 0), label: 'this month' },
    { value: money(sp ? sp.all : 0), label: 'all time' },
  ]
}

/** The line under the spend row. */
export function spendNote(s: StatsSummary | null): string {
  const sp = s?.spend
  const unpriced = sp?.unpriced_models ?? []
  const base = `From token counts at the list prices of ${sp?.prices_as_of ?? '—'}: cache writes at the 5-minute rate unless the transcript says 1-hour; plan usage is included in a subscription.`
  return unpriced.length ? `${base} Not priced: ${unpriced.map(shortModel).join(', ')}.` : base
}

/** The twelve metric tiles (03 §7A.2). */
export function metricTiles(s: StatsSummary | null): Tile[] {
  const sessions = s?.sessions ?? 0
  const perSession = s && sessions > 0 ? Math.round(s.tokens / sessions) : 0
  const fav = s?.favorite_model
  const peak = s?.peak_hour
  const streak = s?.streak ?? { current: 0, longest: 0 }
  const split = s?.tokens_split
  return [
    {
      value: tokens(s?.tokens ?? 0),
      label: `tokens · ${tokens(split?.output ?? 0)} out`,
      title: split ? `${count(split.input)} in · ${count(split.output)} out · ${count(split.cache_read)} cache read · ${count(split.cache_write)} cache write` : undefined,
    },
    { value: count(s?.messages ?? 0), label: 'messages' },
    { value: count(sessions), label: 'sessions started' },
    { value: count(s?.agents ?? 0), label: 'agents spawned' },
    { value: fav ? shortModel(fav.model) : '—', label: fav ? `favorite model · ${pct(fav.share)}` : 'favorite model', title: fav?.model },
    { value: peak ? hourLabel(peak.hour) : '—', label: peak ? `peak hour · ${count(peak.messages)} msgs` : 'peak hour', title: peak ? `${count(peak.messages)} messages in that hour` : undefined },
    { value: `${streak.current} ${streak.current === 1 ? 'day' : 'days'}`, label: `streak · longest ${streak.longest}`, title: 'consecutive days with activity, ending today or yesterday' },
    { value: `${s?.active_days ?? 0} of ${s?.window_days ?? 0}`, label: 'active days' },
    { value: s && s.longest_session_s > 0 ? duration(s.longest_session_s) : '—', label: 'longest session' },
    { value: count(s?.files ?? 0), label: `files · +${count(s?.lines_added ?? 0)} −${count(s?.lines_removed ?? 0)}`, title: 'files changed, lines added and removed' },
    { value: pct(s?.cache_hit), label: 'of input from cache' },
    { value: tokens(perSession), label: 'tokens per session' },
  ]
}

export const ACTIVITY_META = `Mode activity over the last ${MAX_WEEKS} weeks · sessions started + files changed per day`

export type Cell =
  | { text: string; code?: boolean; dim?: boolean }
  | { text: string; agent: { name: string; variant: string; untracked: boolean; suffix?: string } }

export interface TableSpec {
  label: string
  columns: StatsColumn[]
  rows: Cell[][]
  rowTitles?: string[]
}

function spendCell(v: number | null | undefined): Cell {
  return v === null || v === undefined ? { text: '—', dim: true } : { text: money(v) }
}

/** The breakdown by model: one row per model and source, tokens high to low. */
export function modelTable(s: StatsSummary | null): TableSpec {
  const rows = s?.by_model ?? []
  return {
    label: 'By model',
    columns: [{ label: 'Model' }, { label: 'Source' }, { label: 'Messages', num: true }, { label: 'Input', num: true }, { label: 'Output', num: true }, { label: 'Cache read', num: true }, { label: 'Cache write', num: true }, { label: 'Spend', num: true }],
    rows: rows.map((r) => [{ text: r.model, code: true }, { text: sourceLabel(r.source) }, { text: count(r.messages) }, { text: tokens(r.input) }, { text: tokens(r.output) }, { text: tokens(r.cache_read) }, { text: tokens(r.cache_write) }, spendCell(r.spend_usd)]),
  }
}

/** The breakdown by agent: the sessions with usage in the window, tokens high to low. */
export function agentTable(s: StatsSummary | null): TableSpec {
  const rows = s?.by_agent ?? []
  const names = new Map(rows.map((r) => [r.id, r.name]))
  return {
    label: 'By agent',
    columns: [{ label: 'Agent' }, { label: 'Model' }, { label: 'Messages', num: true }, { label: 'Tokens', num: true }, { label: 'Duration', num: true }, { label: 'Spend', num: true }],
    rows: rows.map((r) => [
      { text: r.name, agent: { name: r.name, variant: r.avatar_variant, untracked: r.state === 'untracked', suffix: r.parent_id ? ` · agent of ${names.get(r.parent_id) ?? r.parent_id}` : undefined } },
      r.model ? { text: r.model, code: true } : { text: '—', dim: true },
      { text: count(r.messages) },
      { text: tokens(r.tokens) },
      { text: duration(r.duration_s) },
      spendCell(r.spend_usd),
    ]),
  }
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

/** `Wed 21 Oct`. */
export function dayLabel(date: string): string {
  const dow = (new Date(`${date}T00:00:00Z`).getUTCDay() + 6) % 7
  return `${WEEKDAYS[dow]} ${Number(date.slice(8, 10))} ${MONTHS[Number(date.slice(5, 7)) - 1]}`
}

/** The breakdown by day: the elapsed days newest first, 14 at a time. */
export function dayTable(s: StatsSummary | null, today: string, limit = 14): { table: TableSpec; more: number } {
  const elapsed = (s?.days ?? []).filter((d) => d.date <= today).reverse()
  const shown = elapsed.slice(0, limit)
  return {
    table: {
      label: 'By day',
      columns: [{ label: 'Day' }, { label: 'Sessions', num: true }, { label: 'Messages', num: true }, { label: 'Tokens', num: true }, { label: 'Spend', num: true }],
      rows: shown.map((d) => [{ text: dayLabel(d.date) }, { text: count(d.sessions) }, { text: count(d.messages) }, { text: tokens(d.tokens) }, spendCell(d.spend_usd)]),
      rowTitles: shown.map((d) => splitText(d.split, d.tokens)),
    },
    more: elapsed.length - shown.length,
  }
}

/** `3 more days`. */
export function moreDays(n: number): string {
  return `${n} more ${n === 1 ? 'day' : 'days'}`
}

/** The tokens bars of the window with their heading and hover titles. */
export function dayBars(s: StatsSummary | null, today: string, win: StatsWindow, ws: WeekStart): { heading: string; bars: Bars; titles: string[] } {
  const b = bars(s, today, win, ws)
  return {
    heading: `tokens per ${b.variant === 'weeks' ? 'week' : 'day'}`,
    bars: b,
    titles: b.labels.map((l, i) => `${l || `week ${i + 1}`} · ${splitText(b.splits[i] ?? [], b.totals[i] ?? 0)}`),
  }
}

export const HOURS_HEADING = 'messages by hour'

export function hourSection(s: StatsSummary | null): HourBars {
  return hourBars(s?.hours ?? [])
}

/** The state-column order (03 §7A.3). */
export const STATES: readonly SessionStateCount[] = ['waiting', 'working', 'failed', 'finished', 'idle', 'untracked']

export function stateCounts(s: StatsSummary | null): { state: SessionStateCount; count: number }[] {
  return STATES.map((state) => ({ state, count: s?.by_state[state] ?? 0 }))
}

/** The streak line under the tracker. */
export function streakLine(streak: { current: number; longest: number }, empty: boolean): string {
  return empty ? 'no activity yet' : `current streak ${streak.current} days · longest ${streak.longest}`
}
