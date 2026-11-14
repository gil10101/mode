// Local dates as `YYYY-MM-DD` strings (04 §5.3: `when_date` carries no zone;
// "today" is the device's local date, which the core hands the UI in
// `groupings_list`). Arithmetic runs in UTC so no zone can shift a day.
import type { WeekStart, WhenBucket } from '../state/contract-notes'

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']
const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']

export const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/

function toUtc(d: string): Date {
  const [y, m, day] = d.split('-').map(Number)
  return new Date(Date.UTC(y, m - 1, day))
}

function fromUtc(dt: Date): string {
  return dt.toISOString().slice(0, 10)
}

/** The device's local date. */
export function localToday(now = new Date()): string {
  const m = String(now.getMonth() + 1).padStart(2, '0')
  const d = String(now.getDate()).padStart(2, '0')
  return `${now.getFullYear()}-${m}-${d}`
}

export function isValidDate(d: string): boolean {
  if (!ISO_DATE.test(d)) return false
  return fromUtc(toUtc(d)) === d
}

export function addDays(d: string, n: number): string {
  const dt = toUtc(d)
  dt.setUTCDate(dt.getUTCDate() + n)
  return fromUtc(dt)
}

/** 0 = Sunday. */
export function weekday(d: string): number {
  return toUtc(d).getUTCDay()
}

const WEEK_START_DAY: Record<WeekStart, number> = { sunday: 0, monday: 1, saturday: 6 }

export function weekStartOf(d: string, ws: WeekStart = 'monday'): string {
  const back = (weekday(d) - WEEK_START_DAY[ws] + 7) % 7
  return addDays(d, -back)
}

export function monthStart(d: string): string {
  return `${d.slice(0, 7)}-01`
}

export function monthEnd(d: string): string {
  const dt = toUtc(monthStart(d))
  dt.setUTCMonth(dt.getUTCMonth() + 1)
  dt.setUTCDate(0)
  return fromUtc(dt)
}

/**
 * A `from` / `to` bound of `filter_json` (04 §6.4): a literal date or one of
 * `today`, `tomorrow`, `week_start`, `week_end`, `month_start`, `month_end`,
 * `today+N`, `today-N`. Unknown text → null.
 */
export function resolveDateToken(token: string, today: string, ws: WeekStart = 'monday'): string | null {
  if (isValidDate(token)) return token
  const rel = /^today([+-])(\d{1,4})$/.exec(token)
  if (rel) return addDays(today, (rel[1] === '-' ? -1 : 1) * Number(rel[2]))
  switch (token) {
    case 'today':
      return today
    case 'tomorrow':
      return addDays(today, 1)
    case 'week_start':
      return weekStartOf(today, ws)
    case 'week_end':
      return addDays(weekStartOf(today, ws), 6)
    case 'month_start':
      return monthStart(today)
    case 'month_end':
      return monthEnd(today)
    default:
      return null
  }
}

/** `Oct 24`. */
export function shortDate(d: string): string {
  const dt = toUtc(d)
  return `${MONTHS[dt.getUTCMonth()]} ${dt.getUTCDate()}`
}

export function weekdayName(d: string): string {
  return WEEKDAYS[weekday(d)]
}

/** `Oct 19 – 25`, `Oct 26 – Nov 1`, `Oct 19`, `from Oct 19`, `until Oct 19`. */
export function rangeLabel(from: string | null, to: string | null): string {
  if (from && to) {
    if (from === to) return shortDate(from)
    const a = toUtc(from)
    const b = toUtc(to)
    if (a.getUTCMonth() === b.getUTCMonth() && a.getUTCFullYear() === b.getUTCFullYear()) return `${shortDate(from)} – ${b.getUTCDate()}`
    return `${shortDate(from)} – ${shortDate(to)}`
  }
  if (from) return `from ${shortDate(from)}`
  if (to) return `until ${shortDate(to)}`
  return 'any date'
}

/**
 * The meta text of a row (03 §5.3): `overdue`, `today`, `tomorrow`, a weekday
 * within the coming week (`Fri`), else `Oct 24`.
 */
export function whenLabel(bucket: WhenBucket, date: string | null, today: string): string | null {
  switch (bucket) {
    case 'overdue':
      return 'overdue'
    case 'today':
      return 'today'
    case 'tomorrow':
      return 'tomorrow'
    case 'none':
      return null
    default: {
      if (!date) return null
      const ahead = (toUtc(date).getTime() - toUtc(today).getTime()) / 86_400_000
      if (ahead >= 2 && ahead <= 6) return weekdayName(date)
      return shortDate(date)
    }
  }
}

/** Rows show relative times (10 §12.1): `now`, `4m`, `2h`, `3d`, then the date. */
export function relativeTime(at: string, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - Date.parse(at)) / 1000))
  if (s < 60) return 'now'
  if (s < 3600) return `${Math.floor(s / 60)}m`
  if (s < 86_400) return `${Math.floor(s / 3600)}h`
  if (s < 86_400 * 30) return `${Math.floor(s / 86_400)}d`
  return shortDate(at.slice(0, 10))
}
