// Stats (03 §7A): the spend totals at the top (estimates at list price, the
// one place currency appears, ADR-0051), the metrics of the window, then the
// breakdown: the year of Mode activity as a day tracker and the Models ·
// Agents · Days toggle whose panel holds the charts and the table of that
// dimension. Plain numbers, bars and one table at a time; the window picker in
// the toolbar scopes everything but the tracker and the streak. Until both calls answer, each
// band is its skeleton (StatsSkeleton.tsx); a window change dims the figures shown (.is-stale)
// until the new summary lands.
import { useEffect, useMemo, useRef, useState } from 'react'
import { Seg, SegItem } from '../../kit/controls'
import { cx } from '../../kit/cx'
import { MainBody, MainHead } from '../../kit/layout'
import { StateCol } from '../../kit/lists'
import { HBars, Stat, StatsBars, StatsHead, Tracker } from '../../kit/stats'
import { invoke } from '../../tauri'
import type { SessionStateCount, StatsDay, StatsSummary } from '../../state/contract-settings'
import { useSetting, errorText } from '../../state/settings-store'
import { useSessionsStore } from '../../state/sessions-store'
import { navigate } from '../router'
import { useNow } from '../ui'
import { MetricsSkeleton, PanelSkeleton, SpendSkeleton, TrackerSkeleton } from './StatsSkeleton'
import { SpecTable } from './table'
import { ACTIVITY_META, agentTable, dayBars, dayTable, HOURS_HEADING, hourSection, metricTiles, modelTable, moreDays, spendNote, spendTiles, stateCounts, streakLine, type Tile } from './text'
import { addDays, localDate, MAX_WEEKS, modelShares, statsRange, tipText, trackerModel, WINDOWS, windowLabel, type StatsWindow, type WeekStart } from './tracker'
import './stats.css'

type Panel = 'models' | 'agents' | 'days'
const PANELS: readonly [Panel, string][] = [
  ['models', 'Models'],
  ['agents', 'Agents'],
  ['days', 'Days'],
]

export function StatsView() {
  const now = useNow(60_000)
  const today = localDate(now)
  const [weekStart] = useSetting<WeekStart>('notes.week_start')
  const [resetDay] = useSetting<string | null>('usage.reset_weekday')
  const ws: WeekStart = weekStart === 'sunday' || weekStart === 'saturday' ? weekStart : 'monday'
  const usageWs: WeekStart = resetDay === 'sunday' || resetDay === 'saturday' || resetDay === 'monday' ? resetDay : ws
  const [win, setWin] = useState<StatsWindow>('this_week')
  const [panel, setPanel] = useState<Panel>('models')
  const [days, setDays] = useState<StatsDay[] | null>(null)
  const [summary, setSummary] = useState<StatsSummary | null>(null)
  // The window the shown summary belongs to: while a new one is fetched, the figures keep their labels.
  const [shownWin, setShownWin] = useState<StatsWindow>(win)
  // The first stats_summary has answered (a failure counts); a later one is in flight.
  const [answered, setAnswered] = useState(false)
  const [pending, setPending] = useState(false)
  const asked = useRef(0)
  const [failed, setFailed] = useState<string | null>(null)

  // The year of days once (the tracker and the streak).
  useEffect(() => {
    const from = addDays(today, -7 * MAX_WEEKS)
    invoke<StatsDay[]>('stats_days', { from, to: today }).then(
      (d) => setDays(Array.isArray(d) ? d : []),
      (e) => {
        setDays([])
        setFailed(errorText(e))
      },
    )
  }, [today])

  useEffect(() => {
    const r = statsRange(today, win, usageWs)
    const id = ++asked.current
    setPending(true)
    const land = (s: StatsSummary | null) => {
      // A slower answer to an earlier window never replaces a newer one.
      if (id !== asked.current) return
      setSummary(s)
      setShownWin(win)
      setAnswered(true)
      setPending(false)
    }
    invoke<StatsSummary>('stats_summary', r).then(land, () => land(null))
  }, [today, win, usageWs])

  // 03 §7A Loading: the skeleton until both calls have answered; empty is decided only then.
  const loading = days === null || !answered
  const stale = !loading && pending
  const empty = !loading && days.every((d) => d.sessions + d.files === 0) && (!summary || summary.tokens === 0)
  const label = windowLabel(loading ? win : shownWin)

  return (
    <section className="ap-view ss-view" aria-label="Stats" aria-busy={loading || stale} data-testid="stats-view" data-loading={loading ? 'true' : undefined}>
      <MainHead title="Stats" titleTestId="view-title">
        <Seg label="Window">
          {WINDOWS.map(([w, text]) => (
            <SegItem key={w} active={win === w} onClick={() => setWin(w)}>
              {text}
            </SegItem>
          ))}
        </Seg>
      </MainHead>
      <MainBody className="ss-body" testId="stats-body">
        {empty ? (
          <div className="ap-empty" data-testid="stats-empty">
            Nothing to count yet.
          </div>
        ) : (
          <>
            <div className="m-group-label">spend</div>
            {loading ? <SpendSkeleton /> : <SpendRow summary={summary} label={label} stale={stale} />}
            <div className="m-group-label">{label}</div>
            {loading ? <MetricsSkeleton /> : <Metrics summary={summary} stale={stale} />}
            <StatsHead label="breakdown">
              <Seg label="Breakdown">
                {PANELS.map(([p, text]) => (
                  <SegItem key={p} active={panel === p} onClick={() => setPanel(p)}>
                    {text}
                  </SegItem>
                ))}
              </Seg>
            </StatsHead>
            <div className="m-t-meta ss-sub" id="ss-activity-meta">
              {ACTIVITY_META}
            </div>
            {loading ? <TrackerSkeleton today={today} ws={ws} /> : <DayTracker loaded={days} today={today} ws={ws} />}
            {failed && <div className="m-t-meta ss-in">The activity could not be read.</div>}
            <div className={cx('ss-panel', stale && 'is-stale')} id="ss-panel" data-testid="breakdown" data-panel={panel}>
              {loading ? (
                <PanelSkeleton key={panel} panel={panel} today={today} win={win} ws={usageWs} />
              ) : (
                <div className="ss-panel ss-in" key={panel}>
                  {panel === 'models' && <ModelsPanel summary={summary} />}
                  {panel === 'agents' && <AgentsPanel summary={summary} />}
                  {panel === 'days' && <DaysPanel summary={summary} today={today} win={shownWin} ws={usageWs} />}
                </div>
              )}
            </div>
          </>
        )}
      </MainBody>
    </section>
  )
}

/* ---- 1 · spend --------------------------------------------------------------------- */

function TileRow({ tiles, className, id, testId, stale }: { tiles: Tile[]; className: string; id: string; testId: string; stale: boolean }) {
  return (
    <div className={cx(className, 'ss-in', stale && 'is-stale')} id={id} data-testid={testId}>
      {tiles.map((t, i) => (
        <Stat key={i} value={t.value} label={t.label} title={t.title} hero={t.hero} />
      ))}
    </div>
  )
}

function SpendRow({ summary, label, stale }: { summary: StatsSummary | null; label: string; stale: boolean }) {
  return (
    <>
      <TileRow tiles={spendTiles(summary, label)} className="m-stats is-spend" id="ss-spend" testId="spend" stale={stale} />
      <div className={cx('m-t-meta ss-note ss-in', stale && 'is-stale')} id="ss-spend-note">
        {spendNote(summary)}
      </div>
    </>
  )
}

/* ---- 2 · metrics ------------------------------------------------------------------- */

function Metrics({ summary, stale }: { summary: StatsSummary | null; stale: boolean }) {
  return <TileRow tiles={metricTiles(summary)} className="m-stats is-metrics" id="ss-metrics" testId="metrics" stale={stale} />
}

/* ---- 3 · breakdown ----------------------------------------------------------------- */

function ModelsPanel({ summary }: { summary: StatsSummary | null }) {
  return (
    <>
      <div className="ss-w" id="ss-shares" data-testid="shares">
        <HBars rows={modelShares(summary?.by_model ?? [])} />
      </div>
      <SpecTable spec={modelTable(summary)} className="ss-table" />
    </>
  )
}

function AgentsPanel({ summary }: { summary: StatsSummary | null }) {
  const ids = new Map((summary?.by_agent ?? []).map((r) => [r.name, r.id]))
  const open = (name: string) => {
    const id = ids.get(name)
    if (id) navigate({ view: 'sessions', id })
  }
  return (
    <>
      <SpecTable spec={agentTable(summary)} className="ss-table ss-agents" open={open} />
      <div className="m-group-label">sessions by state</div>
      <div className="ss-states" id="ss-states" data-testid="by-state">
        {stateCounts(summary).map(({ state, count: n }) => (
          <StateRow key={state} state={state} count={n} />
        ))}
      </div>
    </>
  )
}

/** `.m-stat-row` that opens the roster filtered to its state (03 §7A.3). */
function StateRow({ state, count: n }: { state: SessionStateCount; count: number }) {
  return (
    <div
      className="m-stat-row ss-link"
      role="button"
      tabIndex={0}
      aria-label={`${state}, ${n} sessions, open in Sessions`}
      onClick={() => navigate({ view: 'sessions', filter: state })}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          navigate({ view: 'sessions', filter: state })
        }
      }}
    >
      <StateCol state={state} />
      <span className="m-count">{n}</span>
    </div>
  )
}

function DaysPanel({ summary, today, win, ws }: { summary: StatsSummary | null; today: string; win: StatsWindow; ws: WeekStart }) {
  const d = dayBars(summary, today, win, ws)
  const h = hourSection(summary)
  const { table, more } = dayTable(summary, today)
  return (
    <>
      <div className="ss-w" id="ss-bars" data-testid="bars">
        <div className="m-t-meta">{d.heading}</div>
        <StatsBars values={d.bars.values} todayIndex={d.bars.todayIndex} days={d.bars.labels} variant={d.bars.variant} titles={d.titles} />
      </div>
      <div className="ss-w" id="ss-hours" data-testid="hours">
        <div className="m-t-meta">{HOURS_HEADING}</div>
        <StatsBars values={h.values} todayIndex={h.peak} days={h.labels} variant="hours" titles={h.titles} />
      </div>
      <SpecTable spec={table} className="ss-table" />
      {more > 0 && <div className="m-t-meta ss-more">{moreDays(more)}</div>}
    </>
  )
}

/* ---- the day tracker (03 §7A.3) ---------------------------------------------------- */

/**
 * A year of Mode activity as the kit's Tracker with the behavior 03 §7A asks
 * of it: `role="grid"`, each square labeled with its tooltip text, the
 * tooltip open on hover or focus, arrow keys between days, Enter or click
 * opens the Sessions view on that day with history on.
 */
function DayTracker({ loaded, today, ws }: { loaded: StatsDay[]; today: string; ws: WeekStart }) {
  const [open, setOpen] = useState<number | undefined>(undefined)
  const ref = useRef<HTMLDivElement>(null)
  const model = useMemo(() => trackerModel(loaded, today, ws), [loaded, today, ws])
  const byDate = useMemo(() => new Map(loaded.map((d) => [d.date, d])), [loaded])
  const tips = useMemo(() => model.dates.map((d) => tipText(d, byDate.get(d))), [model, byDate])
  const days = model.days.map((d, i) => (i === open ? { ...d, tip: tips[i] } : d))

  useEffect(() => {
    const squares = ref.current?.querySelectorAll<HTMLElement>('.m-tracker-grid > i') ?? []
    squares.forEach((el, i) => {
      el.setAttribute('role', 'gridcell')
      el.setAttribute('aria-label', tips[i] ?? '')
      el.dataset.date = model.dates[i] ?? ''
    })
  }, [tips, model])

  const indexOf = (el: EventTarget | null) => {
    const squares = Array.from(ref.current?.querySelectorAll<HTMLElement>('.m-tracker-grid > i') ?? [])
    return squares.indexOf(el as HTMLElement)
  }

  const openDay = (i: number) => {
    const date = model.dates[i]
    if (!date) return
    const s = useSessionsStore.getState()
    void s.setShowHistory(true)
    if (date === today) s.setFilters({ date: 'today' })
    navigate({ view: 'sessions' })
  }

  return (
    <div
      ref={ref}
      className="ss-tracker ss-in"
      role="grid"
      aria-label={`Mode activity, ${model.streak.current} day streak`}
      data-testid="tracker"
      onMouseOver={(e) => {
        const i = indexOf(e.target)
        if (i >= 0 && i !== open) setOpen(i)
      }}
      onMouseLeave={() => setOpen(undefined)}
      onClick={(e) => {
        const i = indexOf(e.target)
        if (i >= 0) openDay(i)
      }}
      onKeyDown={(e) => {
        const i = indexOf(e.target)
        if (i < 0) return
        const n = model.days.length
        let next: number | null = null
        if (e.key === 'ArrowUp') next = i % 7 === 0 ? null : i - 1
        else if (e.key === 'ArrowDown') next = i % 7 === 6 || i + 1 >= n ? null : i + 1
        else if (e.key === 'ArrowLeft') next = i - 7 >= 0 ? i - 7 : null
        else if (e.key === 'ArrowRight') next = i + 7 < n ? i + 7 : null
        else if (e.key === 'Enter') {
          e.preventDefault()
          openDay(i)
          return
        }
        if (next !== null) {
          e.preventDefault()
          const squares = ref.current?.querySelectorAll<HTMLElement>('.m-tracker-grid > i')
          squares?.[next]?.focus()
        }
      }}
    >
      <Tracker
        days={days}
        months={model.months}
        weekdays={model.weekdays}
        foot={streakLine(model.streak, model.empty)}
        openIndex={open}
        onOpen={setOpen}
      />
    </div>
  )
}
