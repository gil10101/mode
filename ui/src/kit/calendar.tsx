import { Fragment, type CSSProperties, type KeyboardEvent, type ReactNode } from 'react'
import { Button } from './button'
import { cx } from './cx'

/** One hour row of the week grid. */
export const HOUR_PX = 44

export interface CalendarDay {
  key: string
  /** Day of month. */
  num: number
  /** `Mon` … `Sun`. */
  label: string
  today?: boolean
}

export interface CalendarEvent {
  id: string
  /** Column, 0–6. */
  day: number
  /** Minutes from midnight. */
  start?: number
  end?: number
  allDay?: boolean
  title: ReactNode
  /** Overrides the computed time label. */
  time?: string
}

export function formatTime(minutes: number): string {
  const h = Math.floor(minutes / 60)
  const m = minutes % 60
  return `${h}:${String(m).padStart(2, '0')}`
}

/** `9:30` for half-hour events, `10:00–12:00` otherwise. */
export function eventTime(e: CalendarEvent): string | undefined {
  if (e.time !== undefined) return e.time
  if (e.start === undefined) return undefined
  const end = e.end ?? e.start + 30
  return end - e.start <= 30 ? formatTime(e.start) : `${formatTime(e.start)}–${formatTime(end)}`
}

/** Chip geometry inside the start hour's cell: 1 px in from the rule, 2 px short of the duration. */
export function eventBox(start: number, end: number, hourPx = HOUR_PX): { top: number; height: number } {
  const top = Math.max(1, Math.round(((start % 60) / 60) * hourPx))
  const height = Math.max(14, Math.round(((end - start) / 60) * hourPx) - 2)
  return { top, height }
}

export interface EventProps {
  title: ReactNode
  time?: ReactNode
  selected?: boolean
  /** Position inside an hour cell. */
  top?: number
  height?: number
  onClick?: () => void
  className?: string
  style?: CSSProperties
}

/** Flat sunken chip: title (500) and time (11 px fg-3); no bar, no dot, no calendar color (§7.2). */
export function Event({ title, time, selected, top, height, onClick, className, style }: EventProps) {
  const vars: Record<string, string> = {}
  if (top !== undefined) vars['--top'] = `${top}px`
  if (height !== undefined) vars['--h'] = `${height}px`
  return (
    <div
      className={cx('m-event', selected && 'is-selected', className)}
      style={{ ...vars, ...style } as CSSProperties}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      aria-pressed={onClick ? !!selected : undefined}
      onClick={onClick}
      onKeyDown={onClick ? press(onClick) : undefined}
    >
      <b>{title}</b>
      {time !== undefined && <small className="m-t-meta">{time}</small>}
    </div>
  )
}

function press(onClick: () => void) {
  return (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onClick()
    }
  }
}

export interface CalendarProps {
  days: CalendarDay[]
  events: CalendarEvent[]
  /** First and last hour drawn (default 7–19); the rest collapses behind the strips. */
  startHour?: number
  endHour?: number
  /** The now-line: column and minutes from midnight. */
  now?: { day: number; minutes: number }
  selectedId?: string
  onSelect?: (id: string) => void
  /** The “+ earlier” / “+ later” strips. */
  earlier?: boolean
  later?: boolean
  onEarlier?: () => void
  onLater?: () => void
  /** One hour row's height (default HOUR_PX); a smaller grid, as in the onboarding preview, sets `--m-cal-row` to match. */
  hourPx?: number
  className?: string
  style?: CSSProperties
}

/** Week grid: all-day row, hours 7–19, today tint, now-line, flat event chips. */
export function Calendar({ days, events, startHour = 7, endHour = 19, now, selectedId, onSelect, earlier = true, later = true, onEarlier, onLater, hourPx = HOUR_PX, className, style }: CalendarProps) {
  const hours: number[] = []
  for (let h = startHour; h <= endHour; h++) hours.push(h)
  const cell = (d: CalendarDay) => cx('m-cal-cell', d.today && 'is-today')
  const chip = (e: CalendarEvent, box?: { top: number; height: number }) => (
    <Event
      key={e.id}
      title={e.title}
      time={e.allDay ? undefined : eventTime(e)}
      selected={e.id === selectedId}
      top={box?.top}
      height={box?.height}
      onClick={onSelect ? () => onSelect(e.id) : undefined}
    />
  )
  return (
    <div className={cx('m-calendar', className)} style={hourPx === HOUR_PX ? style : ({ '--m-cal-row': `${hourPx}px`, ...style } as CSSProperties)} role="grid">
      <div className="m-cal-corner" />
      {days.map((d) => (
        <div key={d.key} className={cx('m-cal-day', d.today && 'is-today')}>
          <b>{d.num}</b>
          {d.label}
        </div>
      ))}
      <div className="m-cal-allday">
        <div className="m-cal-hour">all day</div>
        {days.map((d, i) => (
          <div key={d.key} className={cell(d)}>
            {events.filter((e) => e.allDay && e.day === i).map((e) => chip(e))}
          </div>
        ))}
      </div>
      {earlier && (
        <div className="m-cal-more" role="button" tabIndex={0} onClick={onEarlier} onKeyDown={onEarlier ? press(onEarlier) : undefined}>
          + earlier
        </div>
      )}
      {hours.map((h) => (
        <Fragment key={h}>
          <div className="m-cal-hour">{`${h}:00`}</div>
          {days.map((d, i) => (
            <div key={d.key} className={cell(d)} data-hour={h} data-day={i}>
              {events
                .filter((e) => !e.allDay && e.day === i && e.start !== undefined && Math.floor(e.start / 60) === h)
                .map((e) => chip(e, eventBox(e.start!, e.end ?? e.start! + 30, hourPx)))}
              {now && now.day === i && Math.floor(now.minutes / 60) === h && (
                <i className="m-cal-now" style={{ '--top': `${Math.round(((now.minutes % 60) / 60) * hourPx)}px` } as CSSProperties} />
              )}
            </div>
          ))}
        </Fragment>
      ))}
      {later && (
        <div className="m-cal-more" role="button" tabIndex={0} onClick={onLater} onKeyDown={onLater ? press(onLater) : undefined}>
          + later
        </div>
      )}
    </div>
  )
}

export interface AgendaItemSpec {
  id: string
  time: ReactNode
  title: ReactNode
  /** A Join button, only when a link exists. */
  join?: boolean
}

export interface AgendaGroup {
  label: string
  items: AgendaItemSpec[]
}

export interface AgendaProps {
  groups: AgendaGroup[]
  onSelect?: (id: string) => void
  onJoin?: (id: string) => void
  className?: string
  style?: CSSProperties
}

/** The 240-px rail: Today and Tomorrow as plain rows. */
export function Agenda({ groups, onSelect, onJoin, className, style }: AgendaProps) {
  return (
    <div className={cx('m-agenda', className)} style={style}>
      {groups.map((g) => (
        <Fragment key={g.label}>
          <div className="m-group-label" style={{ paddingLeft: 4 }}>
            {g.label}
          </div>
          {g.items.map((it) => (
            <AgendaItem key={it.id} time={it.time} title={it.title} join={it.join} onClick={onSelect ? () => onSelect(it.id) : undefined} onJoin={onJoin ? () => onJoin(it.id) : undefined} />
          ))}
        </Fragment>
      ))}
    </div>
  )
}

export interface AgendaItemProps {
  time: ReactNode
  title: ReactNode
  join?: boolean
  onClick?: () => void
  onJoin?: () => void
}

export function AgendaItem({ time, title, join, onClick, onJoin }: AgendaItemProps) {
  return (
    <div className="m-task" role={onClick ? 'button' : undefined} onClick={onClick}>
      <span className="m-t-meta">{time}</span>
      <span className="m-task-title">{title}</span>
      {join && (
        <Button
          onClick={(e) => {
            e.stopPropagation()
            onJoin?.()
          }}
        >
          Join
        </Button>
      )}
    </div>
  )
}

export const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'] as const

/** Month grid: weekday heads, then `MonthCell`s. */
export function Month({ weekdays = WEEKDAYS, children, className, style }: { weekdays?: readonly string[]; children?: ReactNode; className?: string; style?: CSSProperties }) {
  return (
    <div className={cx('m-month', className)} style={style} role="grid">
      {weekdays.map((w) => (
        <div key={w} className="m-cal-day">
          {w}
        </div>
      ))}
      {children}
    </div>
  )
}

export interface MonthCellProps {
  day: number
  today?: boolean
  events?: { id: string; title: ReactNode }[]
  /** Chips shown before the “+n” line. */
  max?: number
  selectedId?: string
  onSelect?: (id: string) => void
  onMore?: () => void
  className?: string
}

export function MonthCell({ day, today, events = [], max = 3, selectedId, onSelect, onMore, className }: MonthCellProps) {
  const shown = events.slice(0, max)
  const hidden = events.length - shown.length
  return (
    <div className={cx('m-month-cell', today && 'is-today', className)} role="gridcell">
      <b>{day}</b>
      {shown.map((e) => (
        <Event key={e.id} title={e.title} selected={e.id === selectedId} onClick={onSelect ? () => onSelect(e.id) : undefined} />
      ))}
      {hidden > 0 && (
        <div className="m-t-meta" style={{ padding: '2px 6px' }} role={onMore ? 'button' : undefined} onClick={onMore}>
          {`+${hidden}`}
        </div>
      )}
    </div>
  )
}
