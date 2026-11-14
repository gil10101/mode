// The Stats view's loading state (03 §7A Loading; 10 §3.7, §7.4.21): each band that waits on
// stats_days or stats_summary drawn as kit Skeleton blocks at its final size, with what the view
// already knows (the tracker's labels and legend, the tables' headings, the Days panel's
// headings and labels) drawn as itself. StatsView swaps these for the bands as the figures land.
// The geometry is skeleton.ts's, which the "Stats · loading" wireframe figure draws too.
import { Fragment, type CSSProperties } from 'react'
import { Skeleton } from '../../kit/skeleton'
import { Stat } from '../../kit/stats'
import { cellWidth, DAY_BARS, HOUR_BARS, METRICS, NOTE, SHARES, SPEND, STREAK, TABLE_ROWS } from './skeleton'
import { agentTable, dayBars, dayTable, HOURS_HEADING, hourSection, modelTable, type TableSpec } from './text'
import { trackerModel, type StatsWindow, type WeekStart } from './tracker'

/** The four spend tiles and the note line under them. */
export function SpendSkeleton() {
  return (
    <>
      <div className="m-stats is-spend" data-testid="spend-skeleton">
        {SPEND.map((t, i) => (
          <Stat key={i} hero={i === 0} value={<Skeleton variant="n" width={t.n} />} label={<Skeleton width={t.l} />} />
        ))}
      </div>
      <div className="m-t-meta ss-note">
        <Skeleton width={NOTE} />
      </div>
    </>
  )
}

/** The twelve metric tiles. */
export function MetricsSkeleton() {
  return (
    <div className="m-stats is-metrics" data-testid="metrics-skeleton">
      {METRICS.map((t, i) => (
        <Stat key={i} value={<Skeleton variant="n" width={t.n} />} label={<Skeleton width={t.l} />} />
      ))}
    </div>
  )
}

/** The year tracker: its month and weekday labels, one block per square, the streak line as a block, the legend. */
export function TrackerSkeleton({ today, ws }: { today: string; ws: WeekStart }) {
  const model = trackerModel([], today, ws)
  return (
    <div className="ss-tracker" data-testid="tracker-skeleton">
      <div className="m-tracker" style={{ '--weeks': Math.max(1, Math.ceil(model.days.length / 7)) } as CSSProperties}>
        <div className="m-tracker-months">
          {model.months.map((m) => (
            <span key={m.label + m.column} style={{ gridColumn: `${m.column}/span ${m.span ?? 2}` }}>
              {m.label}
            </span>
          ))}
        </div>
        <div className="m-tracker-days">
          {model.weekdays.map((w, i) => (
            <span key={i}>{w}</span>
          ))}
        </div>
        <div className="m-tracker-grid">
          {model.days.map((d, i) => (
            <Skeleton key={d.date} variant="square" column={Math.floor(i / 7)} />
          ))}
        </div>
        <div className="m-tracker-foot">
          <span>
            <Skeleton width={STREAK} />
          </span>
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
    </div>
  )
}

/** A table's real headings over rows of blocks. */
function TableSkeleton({ spec }: { spec: TableSpec }) {
  return (
    <table className="m-table ss-table" aria-label={spec.label}>
      <thead>
        <tr>
          {spec.columns.map((c, i) => (
            <th key={i} className={c.num ? 'is-num' : undefined}>
              {c.label}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {Array.from({ length: TABLE_ROWS }, (_, r) => (
          <tr key={r}>
            {spec.columns.map((c, i) => (
              <td key={i} className={c.num ? 'is-num' : undefined}>
                <Skeleton width={cellWidth(i, Boolean(c.num), r)} />
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  )
}

/** The panel of the selected toggle: Models (share bars and table), Agents (table), Days (bars and table). */
export function PanelSkeleton({ panel, today, win, ws }: { panel: 'models' | 'agents' | 'days'; today: string; win: StatsWindow; ws: WeekStart }) {
  if (panel === 'agents') return <TableSkeleton spec={agentTable(null)} />
  if (panel === 'days') {
    const d = dayBars(null, today, win, ws)
    const h = hourSection(null)
    const weeks = d.bars.variant === 'weeks'
    const n = weeks ? ({ '--n': d.bars.labels.length } as CSSProperties) : undefined
    return (
      <>
        <div className="ss-w">
          <div className="m-t-meta">{d.heading}</div>
          <div className={weeks ? 'm-stats-bars is-weeks' : 'm-stats-bars'} style={n}>
            {d.bars.labels.map((_, i) => (
              <Skeleton key={i} variant="bar" height={DAY_BARS[i % DAY_BARS.length]} />
            ))}
          </div>
          <BarLabels labels={d.bars.labels} className={weeks ? 'm-stats-days is-weeks' : 'm-stats-days'} style={n} />
        </div>
        <div className="ss-w">
          <div className="m-t-meta">{HOURS_HEADING}</div>
          <div className="m-stats-bars is-hours">
            {HOUR_BARS.map((v, i) => (
              <Skeleton key={i} variant="bar" height={v} />
            ))}
          </div>
          <BarLabels labels={h.labels} className="m-stats-days is-hours" />
        </div>
        <TableSkeleton spec={dayTable(null, today).table} />
      </>
    )
  }
  return (
    <>
      <div className="ss-w" data-testid="shares-skeleton">
        <div className="m-hbars">
          {SHARES.map((r, i) => (
            <Fragment key={i}>
              <span className="m-hbar-l">
                <Skeleton width={r.l} />
              </span>
              <span className="m-hbar" />
              <span className="m-hbar-n">
                <Skeleton width={r.n} />
              </span>
            </Fragment>
          ))}
        </div>
      </div>
      <TableSkeleton spec={modelTable(null)} />
    </>
  )
}

/** The labels under a row of bars (known before the figures). */
function BarLabels({ labels, className, style }: { labels: readonly string[]; className: string; style?: CSSProperties }) {
  return (
    <div className={className} style={style}>
      {labels.map((l, i) => (
        <span key={i}>{l}</span>
      ))}
    </div>
  )
}
