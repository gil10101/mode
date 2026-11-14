// The month layout (03 §6.5; app-window.html #month): weekday heads, then a
// cell per day with its number (a click opens the Day layout), up to three
// one-line chips (the title, then the join mark when the event has a join
// link; a click selects the event) and a "+n" line
// that opens the day; today tinted, days outside the month in fg-3. The
// kit's `Month` draws the heads; cells are composed here because the kit's
// `MonthCell` number is not clickable (kit candidate).
import { Event, Month } from '../../kit/calendar'
import { cx } from '../../kit/cx'
import type { CalendarEvent } from '../../state/contract-connectors'
import { dateStart, eventsOn, localDate, weekdayHeads, type WeekStart } from '../../state/calendar-store'
import { chipClass } from './WeekGrid'
import { JoinGlyph } from './glyphs'

export const MONTH_CHIPS = 3

/** A month chip's title: the title ellipsizes, the join mark (03 §6.5) stays whole after it. */
export function MonthChipTitle({ e }: { e: CalendarEvent }) {
  if (!e.join_url) return <>{e.title}</>
  return (
    <>
      <span className="m-ellipsis">{e.title}</span>
      <JoinGlyph conference={e.conference} />
    </>
  )
}

export function MonthGrid({ days, anchor, events, now, selectedId, weekStart, onSelect, onDay }: { days: string[]; anchor: string; events: CalendarEvent[]; now: number; selectedId: string | null; weekStart: WeekStart; onSelect: (id: string) => void; onDay: (date: string) => void }) {
  const today = localDate(now)
  const month = anchor.slice(0, 7)
  return (
    <Month weekdays={weekdayHeads(weekStart)} className="cv-month">
      {days.map((d) => {
        const { allDay, timed } = eventsOn(events, d)
        const list = [...allDay, ...timed.filter((e) => localDate(Date.parse(e.starts_at)) === d)]
        const shown = list.slice(0, MONTH_CHIPS)
        const hidden = list.length - shown.length
        return (
          <div key={d} className={cx('m-month-cell', d === today && 'is-today', d.slice(0, 7) !== month && 'cv-month-out')} role="gridcell" data-date={d}>
            <b role="button" tabIndex={0} aria-label={`Open ${d}`} onClick={() => onDay(d)} onKeyDown={(e) => e.key === 'Enter' && onDay(d)}>
              {dateStart(d).getDate()}
            </b>
            {shown.map((e) => (
              <Event key={e.id} title={<MonthChipTitle e={e} />} selected={e.id === selectedId} onClick={() => onSelect(e.id)} className={cx(chipClass(e), e.join_url && 'cv-join')} />
            ))}
            {hidden > 0 && (
              <div className="m-t-meta cv-more-n" role="button" tabIndex={0} onClick={() => onDay(d)} onKeyDown={(e) => e.key === 'Enter' && onDay(d)}>
                {`+${hidden}`}
              </div>
            )}
          </div>
        )
      })}
    </Month>
  )
}
