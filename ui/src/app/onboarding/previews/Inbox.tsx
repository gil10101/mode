// Inbox and calendar step preview: the Inbox with two urgent threads on top, each with its why line,
// then the recent mail and Slack; the pointer clicks the Calendar tab and the week grid shows Mon–Fri
// from 09:00 (now marked) with its hour row sized so the grid fills the visible part of the frame down
// to the panel's bottom edge; the pointer clicks "1:1 Priya" and its detail opens as the Calendar view
// draws it below 1100 px (a card over the grid: title, time, the join link as text, Join); 7 s a loop,
// then Inbox again. Sample people.
import { useLayoutEffect, useRef, useState, type RefObject } from 'react'
import { BrandMark } from '../../../kit/brands'
import { Button, ButtonGroup } from '../../../kit/button'
import { PersonAvatar } from '../../../kit/avatars'
import { Calendar, type CalendarDay, type CalendarEvent } from '../../../kit/calendar'
import { Tab, Tabs } from '../../../kit/controls'
import { DetailPanel } from '../../../kit/layout'
import { GroupLabel, List, MsgRow, Task } from '../../../kit/lists'
import { Cursor } from './Cursor'
import { Frame } from './Frame'
import { useLoop } from './loop'

const PERIOD = 7000
const MOVE_AT = 1000
const PRESS_AT = 2400
const CALENDAR_AT = 2560
const EVENT_MOVE_AT = 3200
const EVENT_PRESS_AT = 4000
const SELECTED_AT = 4160
export const MEET_LINK = 'meet.google.com/abc-defg-hij'

const DAYS: CalendarDay[] = [
  { key: 'mon', num: 2, label: 'Mon', today: true },
  { key: 'tue', num: 3, label: 'Tue' },
  { key: 'wed', num: 4, label: 'Wed' },
  { key: 'thu', num: 5, label: 'Thu' },
  { key: 'fri', num: 6, label: 'Fri' },
]
const h = (hour: number, min = 0) => hour * 60 + min
// Today is the first column, so the detail card (bottom right) never covers today's events.
const EVENTS: CalendarEvent[] = [
  { id: 'design', day: 0, start: h(11), end: h(12), title: 'Design sync' },
  { id: 'priya', day: 0, start: h(15), end: h(16), title: '1:1 Priya' },
  { id: 'retry', day: 0, start: h(17, 30), end: h(18), title: 'Retry policy review' },
  { id: 'lunch', day: 1, start: h(11, 30), end: h(12, 30), title: 'Lunch with Sam' },
  { id: 'interview', day: 2, start: h(10, 30), end: h(11, 30), title: 'Interview: backend' },
  { id: 'release', day: 3, start: h(10), end: h(11), title: 'Release planning' },
  { id: 'freeze', day: 4, allDay: true, title: 'Docs freeze' },
  { id: 'demo', day: 4, start: h(11), end: h(12), title: 'Sprint demo' },
]
/** The grid's hours (09:00–17:00) and its smallest hour row. */
const START_HOUR = 9
const END_HOUR = 17
const MIN_ROW = 35

/** Top of `el` inside `box`, from layout offsets (a transform, such as a step's slide, does not move it). */
function offsetIn(el: HTMLElement, box: HTMLElement): number {
  let y = 0
  for (let n: HTMLElement | null = el; n && n !== box; n = n.offsetParent as HTMLElement | null) y += n.offsetTop + (n !== el ? n.clientTop : 0)
  return y
}

/**
 * The hour row that makes the grid reach the panel's bottom edge (03 §2.8 step 4): the visible part of the
 * preview below the first hour's top, over the hours drawn, rounded up so no surface shows under the last
 * row, and never under 35 px. Follows the stage's size.
 */
function useHourRow(view: RefObject<HTMLDivElement | null>, active: boolean): number {
  const [px, setPx] = useState(MIN_ROW)
  useLayoutEffect(() => {
    const el = view.current
    const preview = el?.closest<HTMLElement>('.m-ob-preview')
    const stage = el?.closest<HTMLElement>('.m-ob-stage')
    if (!active || !el || !preview || !stage) return
    const measure = () => {
      const first = el.querySelector<HTMLElement>(`.m-cal-cell[data-hour="${START_HOUR}"]`)
      if (!first) return
      const visible = stage.clientHeight - preview.offsetTop - preview.clientTop - offsetIn(first, preview)
      const next = Math.max(MIN_ROW, Math.ceil(visible / (END_HOUR - START_HOUR + 1)))
      setPx((p) => (p === next ? p : next))
    }
    measure()
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure)
    ro?.observe(stage)
    return () => ro?.disconnect()
  }, [view, active])
  return px
}

export function InboxPreview() {
  const t = useLoop(PERIOD)
  const calendar = t >= CALENDAR_AT
  const selected = t >= SELECTED_AT
  const view = useRef<HTMLDivElement>(null)
  const hourPx = useHourRow(view, calendar)
  const target = t >= MOVE_AT && t < EVENT_MOVE_AT ? '.m-tabs .m-tab:nth-child(2)' : t >= EVENT_MOVE_AT && t < SELECTED_AT + 400 ? '.m-cal-cell[data-day="0"][data-hour="15"] .m-event' : null
  return (
    <Frame step="accounts">
      <Tabs label="Inbox and calendar">
        <Tab active={!calendar}>Inbox</Tab>
        <Tab active={calendar}>Calendar</Tab>
      </Tabs>
      {calendar ? (
        <div className="m-ob-view m-ob-enter" data-testid="preview-calendar" ref={view}>
          <Calendar days={DAYS} events={EVENTS} startHour={START_HOUR} endHour={END_HOUR} hourPx={hourPx} now={{ day: 0, minutes: h(14, 20) }} selectedId={selected ? 'priya' : undefined} earlier={false} later={false} />
        </div>
      ) : (
        <div className="m-ob-view">
          <List>
            <GroupLabel>urgent</GroupLabel>
            <MsgRow urgent unread avatar={<PersonAvatar initials="PR" source="slack" />} sender="Priya" channel="#release" mentioned time="09:41" preview="Can you approve the retry change before the 4 pm train?" />
            <div className="m-ob-why">urgent · important person · mentioned you in #release</div>
            <MsgRow urgent unread avatar={<PersonAvatar initials="FI" source="gmail" />} sender="Finance" time="09:20" preview="Invoice 4471 is overdue. Please approve the payment today." />
            <div className="m-ob-why">urgent · deadline today</div>
            <GroupLabel>recent</GroupLabel>
            <MsgRow unread avatar={<PersonAvatar initials="SO" source="gmail" />} sender="Sam" time="09:12" preview="Re: Q4 roadmap, notes from Tuesday" />
            <MsgRow unread avatar={<PersonAvatar initials="LM" source="slack" />} sender="Lena" channel="#api" time="08:50" preview="PR 412 is ready for another look" />
            <MsgRow avatar={<PersonAvatar initials="MA" source="slack" />} sender="Marco" channel="#design" time="08:31" preview="The new empty states are up for review" />
            <MsgRow unread avatar={<PersonAvatar initials="DK" source="gmail" />} sender="Dana" time="08:05" preview="Offsite agenda: a draft for Thursday" />
            <MsgRow avatar={<PersonAvatar initials="RV" source="slack" />} sender="Ravi" channel="#ops" time="07:48" preview="Staging is back up after the migration" />
            <MsgRow avatar={<PersonAvatar initials="AT" source="gmail" />} sender="Aiko" time="Mon" preview="Contract renewal: two questions before we sign" />
          </List>
        </div>
      )}
      {calendar && selected && (
        <DetailPanel label="1:1 Priya details" className="m-ob-detail m-ob-enter">
          <div className="m-t-title m-ellipsis">1:1 Priya</div>
          <div className="m-meta">Mon 2 Nov · 15:00–16:00</div>
          <GroupLabel>join</GroupLabel>
          <Task glyph={<BrandMark id="googlemeet" size={12} label="" />} title={MEET_LINK} meta="Meet" />
          <ButtonGroup>
            <Button>Join</Button>
          </ButtonGroup>
        </DetailPanel>
      )}
      <Cursor to={target} pressed={(t >= PRESS_AT && t < PRESS_AT + 160) || (t >= EVENT_PRESS_AT && t < EVENT_PRESS_AT + 160)} visible={t >= 500 && t < PERIOD - 500} />
    </Frame>
  )
}
