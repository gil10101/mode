import { Fragment, type CSSProperties, type ReactNode } from 'react'
import { cx } from './cx'
import { StateCol } from './lists'
import type { StateKind, TrackerLevel } from './types'

export function Stats({ children, className, style }: { children?: ReactNode; className?: string; style?: CSSProperties }) {
  return (
    <div className={cx('m-stats', className)} style={style}>
      {children}
    </div>
  )
}

export interface StatProps {
  value: ReactNode
  label: ReactNode
  /** The one figure a page leads with (`.is-hero`, 32 px). */
  hero?: boolean
  title?: string
}

export function Stat({ value, label, hero, title }: StatProps) {
  return (
    <div className={cx('m-stat', hero && 'is-hero')} title={title}>
      <div className="m-stat-n">{value}</div>
      <div className="m-stat-l">{label}</div>
    </div>
  )
}

/** A `.m-group-label` with a control at its right (the breakdown's toggle). */
export function StatsHead({ label, children, className, style }: { label: ReactNode; children?: ReactNode; className?: string; style?: CSSProperties }) {
  return (
    <div className={cx('m-stats-head', className)} style={style}>
      <div className="m-group-label">{label}</div>
      {children}
    </div>
  )
}

export interface StatsBarsProps {
  /** Heights, 0–100. */
  values: number[]
  todayIndex?: number
  /** Labels under the bars; an empty string leaves a slot blank. */
  days?: readonly string[]
  /** Seven bars (default), 24 local hours, or one per week. */
  variant?: 'days' | 'hours' | 'weeks'
  /** Hover titles, one per bar. */
  titles?: readonly string[]
}

/** Tokens per day as neutral bars, today in fg-2. */
export function StatsBars({ values, todayIndex, days, variant = 'days', titles }: StatsBarsProps) {
  const mod = variant === 'hours' ? 'is-hours' : variant === 'weeks' ? 'is-weeks' : undefined
  const n = variant === 'weeks' ? ({ '--n': values.length } as CSSProperties) : undefined
  return (
    <>
      <div className={cx('m-stats-bars', mod)} style={n}>
        {values.map((v, i) => (
          <div key={i} className={cx('m-stats-bar', i === todayIndex && 'is-today')} style={{ '--h': v ? `${v}%` : '0' } as CSSProperties} title={titles?.[i]} />
        ))}
      </div>
      {days && (
        <div className={cx('m-stats-days', mod)} style={n}>
          {days.map((d, i) => (
            <span key={i}>{d}</span>
          ))}
        </div>
      )}
    </>
  )
}

export interface HBarRow {
  label: ReactNode
  /** 0–100. */
  share: number
  figure: ReactNode
  title?: string
}

/** Horizontal share bars (the breakdown by model): label, bar, figure per row. */
export function HBars({ rows, className, style }: { rows: HBarRow[]; className?: string; style?: CSSProperties }) {
  return (
    <div className={cx('m-hbars', className)} style={style}>
      {rows.map((r, i) => (
        <Fragment key={i}>
          <span className="m-hbar-l" title={r.title}>
            {r.label}
          </span>
          <span className="m-hbar" title={r.title}>
            <i style={{ '--w': `${Math.max(0, Math.min(100, r.share))}%` } as CSSProperties} />
          </span>
          <span className="m-hbar-n">{r.figure}</span>
        </Fragment>
      ))}
    </div>
  )
}

export interface StatsColumn {
  label: ReactNode
  /** Right-aligned tabular figures. */
  num?: boolean
}

export interface StatsTableProps {
  columns: StatsColumn[]
  /** One array of cells per row, in column order. */
  rows: ReactNode[][]
  rowTitles?: readonly string[]
  className?: string
  style?: CSSProperties
  'aria-label'?: string
}

/** A plain table of figures (`.m-table`): numbers right-aligned, nothing else. */
export function StatsTable({ columns, rows, rowTitles, className, style, 'aria-label': label }: StatsTableProps) {
  return (
    <table className={cx('m-table', className)} style={style} aria-label={label}>
      <thead>
        <tr>
          {columns.map((c, i) => (
            <th key={i} className={cx(c.num && 'is-num')}>
              {c.label}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((cells, r) => (
          <tr key={r} title={rowTitles?.[r]}>
            {cells.map((cell, i) => (
              <td key={i} className={cx(columns[i]?.num && 'is-num')}>
                {cell}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  )
}

export interface StatRowProps {
  state?: StateKind
  label?: ReactNode
  count: ReactNode
}

/** Sessions by state: the state column and a count. */
export function StatRow({ state, label, count }: StatRowProps) {
  return (
    <div className="m-stat-row">
      {state ? <StateCol state={state}>{label}</StateCol> : <span>{label}</span>}
      <span className="m-count">{count}</span>
    </div>
  )
}

export interface TrackerDay {
  /** ISO date (`2026-10-16`). */
  date: string
  level: TrackerLevel
  /** `Fri 16 Oct · 3 sessions · 9 files · 210k tokens`. */
  tip?: ReactNode
}

export interface TrackerMonthLabel {
  label: string
  /** 1-based week column. */
  column: number
  span?: number
}

export interface TrackerProps {
  /** Oldest first, starting on a Monday; one square each, column-major. */
  days: TrackerDay[]
  /** Overrides the labels computed from the dates. */
  months?: TrackerMonthLabel[]
  weekdays?: readonly string[]
  streak?: { current: number; longest: number }
  /** Replaces the streak line. */
  foot?: ReactNode
  /** The square whose tooltip is open. */
  openIndex?: number
  onOpen?: (index: number | undefined) => void
  className?: string
  style?: CSSProperties
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** One label per month change, at the week column where it starts, two columns apart at least. */
export function trackerMonths(days: TrackerDay[]): TrackerMonthLabel[] {
  const out: TrackerMonthLabel[] = []
  let last = -1
  let lastCol = -10
  for (let i = 0; i < days.length; i += 7) {
    const col = i / 7 + 1
    const month = new Date(days[i]!.date).getUTCMonth()
    if (month !== last && col - lastCol >= 2) {
      out.push({ label: MONTHS[month]!, column: col })
      lastCol = col
    }
    last = month
  }
  return out
}

/** GitHub-style day tracker of Mode activity (§7.3): 53 weeks × 7 days (a year ending today), five steps of --m-ok. */
export function Tracker({ days, months, weekdays = ['M', '', 'W', '', 'F', '', ''], streak, foot, openIndex, onOpen, className, style }: TrackerProps) {
  const labels = months ?? trackerMonths(days)
  const weeks = Math.max(1, Math.ceil(days.length / 7))
  return (
    <div className={cx('m-tracker', className)} style={{ '--weeks': weeks, ...style } as CSSProperties}>
      <div className="m-tracker-months">
        {labels.map((m) => (
          <span key={m.label + m.column} style={{ gridColumn: `${m.column}/span ${m.span ?? 2}` }}>
            {m.label}
          </span>
        ))}
      </div>
      <div className="m-tracker-days">
        {weekdays.map((w, i) => (
          <span key={i}>{w}</span>
        ))}
      </div>
      <div className="m-tracker-grid">
        {days.map((d, i) => (
          <i
            key={d.date}
            className={cx(`lvl-${d.level}`, i === openIndex && 'is-open')}
            tabIndex={0}
            style={{ gridArea: `${(i % 7) + 1}/${Math.floor(i / 7) + 1}` }}
            onFocus={onOpen ? () => onOpen(i) : undefined}
            onBlur={onOpen ? () => onOpen(undefined) : undefined}
          >
            {d.tip !== undefined && <span className="m-tracker-tip">{d.tip}</span>}
          </i>
        ))}
      </div>
      <div className="m-tracker-foot">
        <span>{foot ?? (streak ? `current streak ${streak.current} days · longest ${streak.longest}` : null)}</span>
        <span className="m-tracker-legend">
          less&nbsp;
          <i className="lvl-0" />
          <i className="lvl-1" />
          <i className="lvl-2" />
          <i className="lvl-3" />
          <i className="lvl-4" />
          &nbsp;more
        </span>
      </div>
    </div>
  )
}
