// Week and day layouts (03 §6.2–§6.3) composed from the kit's calendar
// classes and its flat `Event` chip: the all-day row (three chips, then
// "+n"), the hour grid 07–19 with "+ earlier / + later" (opened for the
// visit; widened to any event outside it), the today tint, the now-line in
// the today column, overlapping events sharing the column width, declined
// events behind a "+n declined" line. The kit's `Calendar` draws one chip
// per hour cell at full width; overlap columns and glyphs are this file's
// (kit candidate).
import { Fragment, type CSSProperties } from 'react'
import { Event, HOUR_PX, eventBox } from '../../kit/calendar'
import { cx } from '../../kit/cx'
import type { CalendarEvent } from '../../state/contract-connectors'
import { dateStart, eventsOn, hourWindow, localDate, minutesIn, overlapSlots, timeRange, WD_SHORT } from '../../state/calendar-store'
import { JoinGlyph, NoteGlyph } from './glyphs'

/** Chips under 32 px drop the time (03 §6.3). */
const TIME_MIN_PX = 32

export function chipClass(e: CalendarEvent): string | undefined {
  return cx(e.status === 'cancelled' && 'cv-cancelled', e.status === 'tentative' && 'cv-tentative', e.my_response === 'declined' && 'cv-declined') || undefined
}

export function ChipTitle({ e, glyphs = true }: { e: CalendarEvent; glyphs?: boolean }) {
  return (
    <>
      {e.title}
      {glyphs && e.join_url && <JoinGlyph conference={e.conference} />}
      {glyphs && e.prepare_note_id && <NoteGlyph />}
    </>
  )
}

export interface WeekGridProps {
  days: string[]
  events: CalendarEvent[]
  now: number
  selectedId: string | null
  expanded: { earlier: boolean; later: boolean }
  declinedOpen: string[]
  onSelect: (id: string) => void
  onExpand: (which: 'earlier' | 'later') => void
  onToggleDeclined: (date: string) => void
  onDay?: (date: string) => void
}

export function WeekGrid({ days, events, now, selectedId, expanded, declinedOpen, onSelect, onExpand, onToggleDeclined, onDay }: WeekGridProps) {
  const today = localDate(now)
  const win = hourWindow(events, days, expanded)
  const hours: number[] = []
  for (let h = win.start; h <= win.end; h++) hours.push(h)
  const perDay = days.map((d) => {
    const on = eventsOn(events, d, { declined: declinedOpen.includes(d) })
    return { date: d, ...on, slots: overlapSlots(on.timed, d) }
  })
  const nowMin = minutesIn(new Date(now).toISOString(), today)
  const chip = (e: CalendarEvent, date: string, slot: { col: number; cols: number } | undefined) => {
    const s = minutesIn(e.starts_at, date)
    const en = Math.max(minutesIn(e.ends_at, date), s + 15)
    const box = eventBox(Math.round(s), Math.round(en))
    const style: CSSProperties | undefined =
      slot && slot.cols > 1
        ? { left: `calc(3px + (100% - 6px) * ${slot.col} / ${slot.cols})`, width: `calc((100% - 6px) / ${slot.cols} - 2px)`, right: 'auto' }
        : undefined
    return (
      <Event
        key={e.id}
        title={<ChipTitle e={e} />}
        time={box.height >= TIME_MIN_PX ? timeRange(e) : undefined}
        selected={e.id === selectedId}
        top={box.top}
        height={box.height}
        onClick={() => onSelect(e.id)}
        className={chipClass(e)}
        style={style}
      />
    )
  }
  return (
    <div className={cx('m-calendar', days.length === 1 && 'cv-day')} role="grid" aria-label="Calendar events" data-testid="week-grid">
      <div className="m-cal-corner" />
      {days.map((d) => {
        const dt = dateStart(d)
        return (
          <div key={d} className={cx('m-cal-day', d === today && 'is-today')} data-date={d} onClick={onDay ? () => onDay(d) : undefined}>
            <b>{dt.getDate()}</b>
            {WD_SHORT[dt.getDay()]}
          </div>
        )
      })}
      <div className="m-cal-allday">
        <div className="m-cal-hour">all day</div>
        {perDay.map((p) => (
          <div key={p.date} className={cx('m-cal-cell', p.date === today && 'is-today')}>
            {p.allDay.slice(0, 3).map((e) => (
              <Event key={e.id} title={<ChipTitle e={e} />} selected={e.id === selectedId} onClick={() => onSelect(e.id)} className={chipClass(e)} />
            ))}
            {p.allDay.length > 3 && (
              <div className="cv-allday-more" role="button" tabIndex={0} onClick={() => onDay?.(p.date)}>
                {`+${p.allDay.length - 3}`}
              </div>
            )}
          </div>
        ))}
      </div>
      {win.earlier && (
        <div className="m-cal-more" role="button" tabIndex={0} onClick={() => onExpand('earlier')} onKeyDown={(e) => e.key === 'Enter' && onExpand('earlier')}>
          + earlier
        </div>
      )}
      {hours.map((h) => (
        <Fragment key={h}>
          <div className="m-cal-hour">{`${h}:00`}</div>
          {perDay.map((p) => (
            <div key={p.date} className={cx('m-cal-cell', p.date === today && 'is-today')} data-hour={h} data-date={p.date}>
              {p.timed.filter((e) => Math.floor(minutesIn(e.starts_at, p.date) / 60) === h).map((e) => chip(e, p.date, p.slots.get(e.id)))}
              {p.date === today && Math.floor(nowMin / 60) === h && <i className="m-cal-now" style={{ '--top': `${Math.round(((nowMin % 60) / 60) * HOUR_PX)}px` } as CSSProperties} data-testid="now-line" />}
              {h === win.end && p.declined.length > 0 && (
                <div className="cv-declined-line" role="button" tabIndex={0} onClick={() => onToggleDeclined(p.date)} onKeyDown={(e) => e.key === 'Enter' && onToggleDeclined(p.date)} data-testid={`declined-${p.date}`}>
                  {declinedOpen.includes(p.date) ? 'hide declined' : `+${p.declined.length} declined`}
                </div>
              )}
            </div>
          ))}
        </Fragment>
      ))}
      {win.later && (
        <div className="m-cal-more" role="button" tabIndex={0} onClick={() => onExpand('later')} onKeyDown={(e) => e.key === 'Enter' && onExpand('later')}>
          + later
        </div>
      )}
    </div>
  )
}
