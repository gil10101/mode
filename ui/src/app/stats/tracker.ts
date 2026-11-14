// The day tracker and the window picker of Stats (03 §7A) as pure functions:
// local dates as `YYYY-MM-DD`, the year of whole weeks starting on
// `notes.week_start` and ending today, the five steps (quartiles of the
// non-zero days in the year), the streaks, month labels, the tooltip text,
// and the window ranges (this week, last week, 4 weeks, 12 months, all) with
// their bars, the hour bars and the model shares.
import type { HBarRow, TrackerDay, TrackerMonthLabel } from '../../kit/stats'
import type { TrackerLevel } from '../../kit/types'
import type { StatsDay, StatsModelRow, StatsSummaryDay } from '../../state/contract-settings'
import { tokens } from '../sessions/format'
import { count, hourLabel, shortModel, sourceLabel } from './format'

export type WeekStart = 'monday' | 'sunday' | 'saturday'

const DAY = 86_400_000
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

/** The device's local date of an instant. */
export function localDate(now: number): string {
  const d = new Date(now)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

function utc(date: string): number {
  const [y, m, d] = date.split('-').map(Number)
  return Date.UTC(y!, m! - 1, d!)
}

function fmt(t: number): string {
  return new Date(t).toISOString().slice(0, 10)
}

export function addDays(date: string, n: number): string {
  return fmt(utc(date) + n * DAY)
}

/** 0 = Monday … 6 = Sunday. */
export function isoWeekday(date: string): number {
  return (new Date(utc(date)).getUTCDay() + 6) % 7
}

const START: Record<WeekStart, number> = { monday: 0, sunday: 6, saturday: 5 }

export function weekStartOf(date: string, ws: WeekStart): string {
  const back = (isoWeekday(date) - START[ws] + 7) % 7
  return addDays(date, -back)
}

export function daysBetween(from: string, to: string): string[] {
  const out: string[] = []
  for (let t = utc(from); t <= utc(to); t += DAY) out.push(fmt(t))
  return out
}

/** A year: 52 whole weeks before the week of today, which ends at today (53 columns, as GitHub draws it). */
export const VISIBLE_WEEKS = 53
export const MAX_WEEKS = 53

/**
 * The tracker window: `weeks` columns ending with the week of today
 * (`offset` weeks earlier, for tests); the last column stops at today.
 */
export function trackerWindow(today: string, ws: WeekStart, weeks = VISIBLE_WEEKS, offset = 0): { from: string; to: string; days: string[] } {
  const lastStart = addDays(weekStartOf(today, ws), -7 * offset)
  const from = addDays(lastStart, -7 * (weeks - 1))
  const to = offset === 0 ? today : addDays(lastStart, 6)
  return { from, to, days: daysBetween(from, to) }
}

/** Nearest-rank quartiles of the non-zero values; null when every value is 0. */
export function quartiles(values: readonly number[]): [number, number, number] | null {
  const nz = values.filter((v) => v > 0).sort((a, b) => a - b)
  if (!nz.length) return null
  const at = (p: number) => nz[Math.max(0, Math.ceil(p * nz.length) - 1)]!
  return [at(0.25), at(0.5), at(0.75)]
}

export function levelOf(v: number, q: [number, number, number] | null): TrackerLevel {
  if (v <= 0 || !q) return 0
  if (v <= q[0]) return 1
  if (v <= q[1]) return 2
  if (v <= q[2]) return 3
  return 4
}

/** sessions started + files changed: one each (03 §7A.1). */
export function dayValue(d: Pick<StatsDay, 'sessions' | 'files'> | undefined): number {
  return d ? d.sessions + d.files : 0
}

/** Current streak ends today (or yesterday while today has none yet); longest is over the loaded days. */
export function streaks(byDate: ReadonlyMap<string, number>, today: string): { current: number; longest: number } {
  let current = 0
  let d = (byDate.get(today) ?? 0) > 0 ? today : addDays(today, -1)
  while ((byDate.get(d) ?? 0) > 0) {
    current++
    d = addDays(d, -1)
  }
  const dates = Array.from(byDate.keys()).sort()
  let longest = 0
  if (dates.length) {
    let run = 0
    for (const day of daysBetween(dates[0]!, dates[dates.length - 1]! > today ? dates[dates.length - 1]! : today)) {
      run = (byDate.get(day) ?? 0) > 0 ? run + 1 : 0
      longest = Math.max(longest, run)
    }
  }
  return { current, longest }
}

/** A label above the first column where most of the week is in a new month, two columns apart at least. */
export function monthLabels(days: readonly string[]): TrackerMonthLabel[] {
  const out: TrackerMonthLabel[] = []
  let last = -1
  let lastCol = -10
  for (let i = 0; i < days.length; i += 7) {
    const col = i / 7 + 1
    const mid = addDays(days[i]!, 3)
    const month = Number(mid.slice(5, 7)) - 1
    if (month !== last && col - lastCol >= 2) {
      out.push({ label: MONTHS[month]!, column: col })
      lastCol = col
    }
    last = month
  }
  return out
}

/** The weekday letters at the left, for a grid whose rows start on `ws`. */
export function weekdayLetters(ws: WeekStart): string[] {
  const letters = ['M', '', 'W', '', 'F', '', '']
  const s = START[ws]
  return Array.from({ length: 7 }, (_, i) => letters[(s + i) % 7]!)
}

const plural = (n: number, one: string) => `${n} ${n === 1 ? one : `${one}s`}`

/** "Fri 16 Oct · 3 sessions · 9 files · 210k tokens" (tokens in the tooltip only). */
export function tipText(date: string, d: StatsDay | undefined): string {
  const head = `${WEEKDAYS[isoWeekday(date)]} ${Number(date.slice(8, 10))} ${MONTHS[Number(date.slice(5, 7)) - 1]}`
  if (!d || (d.sessions === 0 && d.files === 0 && d.tokens === 0)) return `${head} · no activity`
  return `${head} · ${plural(d.sessions, 'session')} · ${plural(d.files, 'file')} · ${tokens(d.tokens)} tokens`
}

export interface TrackerModel {
  days: TrackerDay[]
  dates: string[]
  months: TrackerMonthLabel[]
  weekdays: string[]
  streak: { current: number; longest: number }
  /** Every square is `.lvl-0`. */
  empty: boolean
}

/** The visible window's squares from the loaded days (tips are added by the view, one open at a time). */
export function trackerModel(loaded: readonly StatsDay[], today: string, ws: WeekStart, offset = 0): TrackerModel {
  const byDate = new Map(loaded.map((d) => [d.date, d]))
  const win = trackerWindow(today, ws, VISIBLE_WEEKS, offset)
  const q = quartiles(win.days.map((d) => dayValue(byDate.get(d))))
  const days = win.days.map((date) => ({ date, level: levelOf(dayValue(byDate.get(date)), q) }))
  const values = new Map(loaded.map((d) => [d.date, dayValue(d)]))
  return {
    days,
    dates: win.days,
    months: monthLabels(win.days),
    weekdays: weekdayLetters(ws),
    streak: streaks(values, today),
    empty: days.every((d) => d.level === 0),
  }
}

/* ---- the window picker (03 §7A.2) -------------------------------------------------- */

export type StatsWindow = 'this_week' | 'last_week' | 'four_weeks' | 'year' | 'all'

/** The picker's labels, in order. */
export const WINDOWS: readonly [StatsWindow, string][] = [
  ['this_week', 'This week'],
  ['last_week', 'Last week'],
  ['four_weeks', '4 weeks'],
  ['year', '12 months'],
  ['all', 'All time'],
]

/** The lower-case name of a window for the group labels ("this week", "12 months"). */
export function windowLabel(w: StatsWindow): string {
  return (WINDOWS.find(([k]) => k === w)?.[1] ?? w).toLowerCase()
}

/** The roll-ups keep 400 days: the longest window ends today. */
export const ALL_DAYS = 400

export function statsRange(today: string, w: StatsWindow, ws: WeekStart): { from: string; to: string } {
  const start = weekStartOf(today, ws)
  if (w === 'last_week') return { from: addDays(start, -7), to: addDays(start, -1) }
  if (w === 'four_weeks') return { from: addDays(start, -21), to: addDays(start, 6) }
  if (w === 'year') return { from: addDays(start, -7 * 52), to: today }
  if (w === 'all') return { from: addDays(today, -(ALL_DAYS - 1)), to: today }
  return { from: start, to: addDays(start, 6) }
}

type BarDay = Pick<StatsSummaryDay, 'date' | 'tokens' | 'split'>

export interface Bars {
  /** Heights 0–100. */
  values: number[]
  labels: string[]
  todayIndex?: number
  /** For the tooltips: the tokens behind each bar and its split. */
  totals: number[]
  splits: StatsSummaryDay['split'][]
  /** Daily, or one bar per week. */
  variant: 'days' | 'weeks'
}

/** Seven daily bars; one per week for the longer windows; today in fg-2. */
export function bars(summary: { days: BarDay[] } | null, today: string, w: StatsWindow, ws: WeekStart): Bars {
  const { from, to } = statsRange(today, w, ws)
  const days = daysBetween(from, to)
  const byDate = new Map<string, BarDay>((summary?.days ?? []).map((d) => [d.date, d]))
  let totals: number[]
  let labels: string[]
  let splits: Bars['splits']
  let todayIndex: number | undefined
  let variant: Bars['variant'] = 'days'
  if (w === 'four_weeks') {
    variant = 'weeks'
    totals = [0, 1, 2, 3].map((i) => days.slice(i * 7, i * 7 + 7).reduce((n, d) => n + (byDate.get(d)?.tokens ?? 0), 0))
    labels = [0, 1, 2, 3].map((i) => `${Number(days[i * 7]!.slice(8, 10))} ${MONTHS[Number(days[i * 7]!.slice(5, 7)) - 1]}`)
    splits = [0, 1, 2, 3].map((i) => mergeSplits(days.slice(i * 7, i * 7 + 7).flatMap((d) => byDate.get(d)?.split ?? [])))
    const ti = days.indexOf(today)
    todayIndex = ti >= 0 ? Math.floor(ti / 7) : undefined
  } else if (w === 'year' || w === 'all') {
    // One bar per week from the window's first day; a month label where a week starts a new month.
    variant = 'weeks'
    const weeks = Math.ceil(days.length / 7)
    const chunk = (i: number) => days.slice(i * 7, i * 7 + 7)
    totals = Array.from({ length: weeks }, (_, i) => chunk(i).reduce((n, d) => n + (byDate.get(d)?.tokens ?? 0), 0))
    splits = Array.from({ length: weeks }, (_, i) => mergeSplits(chunk(i).flatMap((d) => byDate.get(d)?.split ?? [])))
    labels = []
    let last = -1
    let lastCol = -10
    for (let i = 0; i < weeks; i++) {
      const month = Number(chunk(i)[0]!.slice(5, 7)) - 1
      if (month !== last && i - lastCol >= 3) {
        labels.push(MONTHS[month]!)
        lastCol = i
      } else labels.push('')
      last = month
    }
    const ti = days.indexOf(today)
    todayIndex = ti >= 0 ? Math.floor(ti / 7) : undefined
  } else {
    totals = days.map((d) => byDate.get(d)?.tokens ?? 0)
    labels = days.map((d) => WEEKDAYS[isoWeekday(d)]!)
    splits = days.map((d) => byDate.get(d)?.split ?? [])
    const ti = days.indexOf(today)
    todayIndex = ti >= 0 ? ti : undefined
  }
  const max = Math.max(0, ...totals)
  return { values: totals.map((t) => (max > 0 ? Math.round((t / max) * 100) : 0)), labels, todayIndex, totals, splits, variant }
}

function mergeSplits(parts: StatsSummaryDay['split']): StatsSummaryDay['split'] {
  const m = new Map<string, { model: string; source: string; tokens: number }>()
  for (const p of parts) {
    const k = `${p.model}\u0000${p.source}`
    const cur = m.get(k)
    if (cur) cur.tokens += p.tokens
    else m.set(k, { ...p })
  }
  return Array.from(m.values()).sort((a, b) => b.tokens - a.tokens)
}

/** "1.2M tokens — claude-opus-5-5 · Claude Code 310k, claude-sonnet-5-5 · Mode 12k" for a bar's hover. */
export function splitText(split: StatsSummaryDay['split'], total: number): string {
  if (!total) return 'no tokens'
  const parts = split.map((p) => `${p.model} · ${sourceLabel(p.source)} ${tokens(p.tokens)}`)
  return `${tokens(total)} tokens${parts.length ? ` — ${parts.join(', ')}` : ''}`
}

/* ---- the breakdown (03 §7A.3) ------------------------------------------------------- */

export interface HourBars {
  /** Heights 0–100, 24 entries. */
  values: number[]
  /** `00:00`, `06:00`, `12:00`, `18:00`; blank elsewhere. */
  labels: string[]
  titles: string[]
  peak?: number
}

/** Messages per local hour as 24 bars, the peak hour in fg-2. */
export function hourBars(hours: readonly number[]): HourBars {
  const h = Array.from({ length: 24 }, (_, i) => hours[i] ?? 0)
  const max = Math.max(0, ...h)
  const peak = max > 0 ? h.indexOf(max) : undefined
  return {
    values: h.map((n) => (max > 0 ? Math.round((n / max) * 100) : 0)),
    labels: h.map((_, i) => (i % 6 === 0 ? hourLabel(i) : '')),
    titles: h.map((n, i) => `${hourLabel(i)} · ${count(n)} ${n === 1 ? 'message' : 'messages'}`),
    peak,
  }
}

/** The models by tokens as share bars: the family name (and the source when it is not Claude Code), the share of every token, the tokens. */
export function modelShares(rows: readonly StatsModelRow[], limit = 8): HBarRow[] {
  const total = rows.reduce((n, r) => n + r.tokens, 0)
  return rows.slice(0, limit).map((r) => ({
    label: r.source === 'claude_code' ? shortModel(r.model) : `${shortModel(r.model)} · ${sourceLabel(r.source)}`,
    share: total > 0 ? (r.tokens / total) * 100 : 0,
    figure: tokens(r.tokens),
    title: `${r.model} · ${sourceLabel(r.source)} · ${tokens(r.tokens)} tokens · ${count(r.messages)} messages`,
  }))
}
