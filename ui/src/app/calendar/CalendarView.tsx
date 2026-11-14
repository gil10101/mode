// The Calendar view (03 §6; app-window.html data-view="calendar" and #month):
// a header of exactly four things (the month and range, Today, ‹ ›, Day /
// Week / Month), the week (default) / day / month grids, and one 240-px
// column on the right with the detail panel of the selected calendar event
// above the agenda rail (Today, Tomorrow). Below 1100 px the rail becomes a
// slide-over behind a ⋮ "Agenda" entry (10 §13). States of §6.8: no Google
// account with Calendar, Calendar not granted, the connector banner,
// offline, syncing. Keys (03 §10.5; printed only in Shortcuts): T, ← →,
// D W M, Enter, P J O C, +.
import { useEffect, useMemo, useRef, useState } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { Seg, SegItem } from '../../kit/controls'
import { MainBody } from '../../kit/layout'
import type { CalendarEvent } from '../../state/contract-connectors'
import { hasService, useAccounts } from '../../state/accounts-store'
import { MONTHS, agendaBuckets, clock, dateStart, findEvent, headerTitle, localDate, useCalendar, visibleDays, type CalLayout, type WeekStart } from '../../state/calendar-store'
import { useSetting } from '../../state/settings-store'
import { useShell } from '../shell-store'
import { navigate } from '../router'
import { copyText, openMenu, useNow, type MenuEntry } from '../ui'
import { ConnectorBanners } from '../inbox/banners'
import { LinkHost } from '../inbox/LinkDialog'
import { EventPanel, openInGoogle, openPrepare } from './EventPanel'
import { MonthGrid } from './MonthGrid'
import { WeekGrid } from './WeekGrid'
import './calendar.css'

const CAL_SERVICES = ['calendar'] as const
const LAYOUTS: readonly [CalLayout, string][] = [
  ['day', 'Day'],
  ['week', 'Week'],
  ['month', 'Month'],
]
const STEP_LABEL: Record<CalLayout, string> = { day: 'day', week: 'week', month: 'month' }
/** 10 §13: the rail sits beside the grid at ≥ 1100 px. */
const RAIL_MIN_WIDTH = 1100

function isTyping(el: EventTarget | null): boolean {
  const t = el as HTMLElement | null
  return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)
}

export function CalendarView({ rest }: { rest?: string }) {
  const now = useNow(60_000)
  const layout = useCalendar((s) => s.layout)
  const anchor = useCalendar((s) => s.anchor)
  const events = useCalendar((s) => s.events)
  const agenda = useCalendar((s) => s.agenda)
  const status = useCalendar((s) => s.status)
  const error = useCalendar((s) => s.error)
  const selectedId = useCalendar((s) => s.selectedId)
  const expanded = useCalendar((s) => s.expanded)
  const declinedOpen = useCalendar((s) => s.declinedOpen)
  const accounts = useAccounts((s) => s.accounts)
  const accountsLoaded = useAccounts((s) => s.loaded)
  const width = useShell((s) => s.width)
  const [weekStartSetting] = useSetting<string>('notes.week_start')
  const weekStart: WeekStart = weekStartSetting === 'sunday' || weekStartSetting === 'saturday' ? weekStartSetting : 'monday'
  const [syncing, setSyncing] = useState(false)
  const [slide, setSlide] = useState(false)
  const narrow = width < RAIL_MIN_WIDTH

  useEffect(() => {
    void useCalendar.getState().load(weekStart)
    void useCalendar.getState().loadAgenda()
    if (!useAccounts.getState().loaded) void useAccounts.getState().load()
  }, [weekStart])

  // `calendar/<id>`: that calendar event selected with its detail panel (03 §2.5).
  useEffect(() => {
    if (rest) useCalendar.getState().select(rest)
  }, [rest])

  // "syncing…" only when a load takes a moment, so a fast one never flashes.
  useEffect(() => {
    if (status !== 'loading') {
      setSyncing(false)
      return
    }
    const t = setTimeout(() => setSyncing(true), 400)
    return () => clearTimeout(t)
  }, [status])

  const days = useMemo(() => visibleDays(layout, anchor, weekStart), [layout, anchor, weekStart])
  const selected = findEvent({ events, agenda }, selectedId)
  const buckets = agendaBuckets(agenda, now)
  const calAccounts = accounts.filter((a) => hasService(a, 'calendar'))
  const google = accounts.filter((a) => a.provider === 'google')
  const select = (id: string) => useCalendar.getState().select(id)

  // Keys of 03 §10.5.
  const keyRef = useRef<(e: KeyboardEvent) => void>(() => {})
  keyRef.current = (e: KeyboardEvent) => {
    if (e.metaKey || e.ctrlKey || e.altKey || isTyping(e.target) || e.defaultPrevented) return
    const cal = useCalendar.getState()
    const sel = findEvent(cal, cal.selectedId)
    const k = e.key
    if (k === 't' || k === 'T') cal.today()
    else if (k === 'ArrowLeft') cal.step(-1)
    else if (k === 'ArrowRight') cal.step(1)
    else if (k === 'd' || k === 'D') cal.setLayout('day')
    else if (k === 'w' || k === 'W') cal.setLayout('week')
    else if (k === 'm' || k === 'M') cal.setLayout('month')
    else if (k === '+') cal.expand('both')
    else if ((k === 'p' || k === 'P') && sel) void openPrepare(sel)
    else if ((k === 'j' || k === 'J') && sel?.join_url) void cal.join(sel.id)
    else if ((k === 'o' || k === 'O') && sel) void openInGoogle(sel)
    else if ((k === 'c' || k === 'C') && sel?.join_url) copyText(sel.join_url, 'Join link copied')
    else if (k === 'Escape' && cal.selectedId) cal.select(null)
    else return
    e.preventDefault()
  }
  useEffect(() => {
    const on = (e: KeyboardEvent) => keyRef.current(e)
    window.addEventListener('keydown', on)
    return () => window.removeEventListener('keydown', on)
  }, [])

  const monthPicker = (el: Element) => {
    const a = dateStart(anchor)
    const entries: MenuEntry[] = [
      { id: 'prev-year', label: `‹ ${a.getFullYear() - 1}`, run: () => useCalendar.getState().setAnchor(localDate(new Date(a.getFullYear() - 1, a.getMonth(), 1).getTime()), 'month') },
      ...MONTHS.map((m, i) => ({ id: `m-${i}`, label: `${m} ${a.getFullYear()}`, checked: i === a.getMonth(), run: () => useCalendar.getState().setAnchor(localDate(new Date(a.getFullYear(), i, 1).getTime())) })),
      { id: 'next-year', label: `${a.getFullYear() + 1} ›`, run: () => useCalendar.getState().setAnchor(localDate(new Date(a.getFullYear() + 1, a.getMonth(), 1).getTime()), 'month') },
    ]
    openMenu(el, entries, 'Pick a month')
  }

  const rail = (
    <div className="m-agenda" data-testid="agenda">
      <AgendaGroup label="Today" items={buckets.today} onSelect={select} />
      <AgendaGroup label="Tomorrow" items={buckets.tomorrow} onSelect={select} />
    </div>
  )

  let body: React.ReactNode
  if (accountsLoaded && calAccounts.length === 0) {
    body =
      google.length === 0 ? (
        <div className="ap-empty cv-empty" data-testid="calendar-empty">
          Connect a Google account to see your calendar.
          <Button variant="primary" onClick={() => navigate({ view: 'integrations', rest: 'google-calendar' })}>
            Connect account
          </Button>
        </div>
      ) : (
        <div className="ap-empty cv-empty" data-testid="calendar-not-granted">
          {`Calendar access was not granted for ${google[0]!.display_name}.`}
          <Button variant="primary" onClick={() => void useAccounts.getState().startReauth(google[0]!.id, { services: ['gmail', 'calendar'] })}>
            Re-authorise with Calendar
          </Button>
        </div>
      )
  } else {
    body = (
      <>
        <div className="cv-banners">
          <ConnectorBanners services={CAL_SERVICES} />
          {status === 'error' && (
            <div className="ap-banner" role="alert">
              <span>{error ?? "Couldn't read the calendar."}</span>
              <Button onClick={() => void useCalendar.getState().load()}>Retry</Button>
              <Button quiet onClick={() => navigate({ view: 'settings', rest: 'advanced/diagnostics' })}>
                Open Diagnostics
              </Button>
            </div>
          )}
        </div>
        {layout === 'month' ? (
          <MonthGrid days={days} anchor={anchor} events={events} now={now} selectedId={selectedId} weekStart={weekStart} onSelect={select} onDay={(d) => useCalendar.getState().setAnchor(d, 'day')} />
        ) : (
          <WeekGrid
            days={days}
            events={events}
            now={now}
            selectedId={selectedId}
            expanded={expanded}
            declinedOpen={declinedOpen}
            onSelect={select}
            onExpand={(w) => useCalendar.getState().expand(w)}
            onToggleDeclined={(d) => useCalendar.getState().toggleDeclined(d)}
            onDay={layout === 'week' ? (d) => useCalendar.getState().setAnchor(d, 'day') : undefined}
          />
        )}
        {!narrow && (
          <div className="m-stack">
            {selected && <EventPanel event={selected} />}
            {rail}
          </div>
        )}
      </>
    )
  }

  return (
    <section className="ap-view cv-view" aria-label="Calendar" data-testid="calendar-view">
      <div className="m-main-head">
        <span
          className="m-t-title cv-title"
          role="button"
          tabIndex={0}
          aria-haspopup="menu"
          data-testid="view-title"
          title={syncing ? `last synced ${calAccounts[0]?.last_sync_at ? clock(calAccounts[0].last_sync_at) : 'not yet'}` : undefined}
          onClick={(e) => monthPicker(e.currentTarget)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') monthPicker(e.currentTarget)
          }}
        >
          {headerTitle(layout, anchor, weekStart)}
          {syncing && <span className="cv-syncing"> syncing…</span>}
        </span>
        <Button onClick={() => useCalendar.getState().today()}>Today</Button>
        <Button variant="icon" aria-label={`Previous ${STEP_LABEL[layout]}`} onClick={() => useCalendar.getState().step(-1)}>
          ‹
        </Button>
        <Button variant="icon" aria-label={`Next ${STEP_LABEL[layout]}`} onClick={() => useCalendar.getState().step(1)}>
          ›
        </Button>
        <Seg label="Layout">
          {LAYOUTS.map(([l, label]) => (
            <SegItem key={l} active={layout === l} onClick={() => useCalendar.getState().setLayout(l)}>
              {label}
            </SegItem>
          ))}
        </Seg>
        {narrow && <MenuButton label="Calendar options" onClick={(e) => openMenu(e.currentTarget, [{ id: 'agenda', label: 'Agenda', checked: slide, run: () => setSlide(!slide) }], 'Calendar options')} />}
      </div>
      <MainBody className={`cv-body${narrow ? ' is-narrow' : ''}`} testId="calendar-body">
        {body}
      </MainBody>
      {/* Below 1100 px the detail panel and the rail are a slide-over (10 §13); Esc closes the selection. */}
      {narrow && (selected || slide) && (
        <div className="cv-slide m-stack" role="dialog" aria-label={selected ? `${selected.title} details` : 'Agenda'} data-testid="calendar-slide">
          {selected && <EventPanel event={selected} />}
          {slide && rail}
        </div>
      )}
      <LinkHost />
    </section>
  )
}

function AgendaGroup({ label, items, onSelect }: { label: string; items: CalendarEvent[]; onSelect: (id: string) => void }) {
  return (
    <>
      <div className="m-group-label cv-agenda-label">{label}</div>
      {items.length === 0 && <div className="cv-nothing">Nothing scheduled.</div>}
      {items.map((e) => (
        <div
          key={e.id}
          className="m-task"
          role="button"
          tabIndex={0}
          data-event={e.id}
          onClick={() => onSelect(e.id)}
          onKeyDown={(ev) => {
            if (ev.key === 'Enter') {
              ev.preventDefault()
              onSelect(e.id)
            }
          }}
        >
          <span className="m-t-meta">{e.all_day ? 'all day' : clock(e.starts_at)}</span>
          <span className="m-task-title">{e.title}</span>
          {e.join_url && (
            <Button
              onClick={(ev) => {
                ev.stopPropagation()
                void useCalendar.getState().join(e.id)
              }}
            >
              Join
            </Button>
          )}
        </div>
      ))}
    </>
  )
}

