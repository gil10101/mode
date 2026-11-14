// The detail panel on selection (03 §6.6; app-window.html #c-detail): title,
// time (the event's zone when it is not the device's), the calendar's name as
// text (the only place a calendar is identified), attendees with response
// glyphs (organizer first), the join link, location and description, the
// linked sessions, notes and messages with why (`event_links`, loaded on
// open; app/links.tsx), Prepare notes as the single primary (Open prepare note
// once it exists) with ⋮ (Join, Open in Google Calendar, Copy join link), and
// the read-only footer (ADR-0015).
import { useEffect } from 'react'
import { MenuButton, Button, ButtonGroup } from '../../kit/button'
import { DetailPanel } from '../../kit/layout'
import { BrandMark, brandForConference } from '../../kit/brands'
import { GroupLabel, Task } from '../../kit/lists'
import type { Attendee, CalendarEvent, ThreadLink } from '../../state/contract-connectors'
import { LinkRows } from '../links'
import { openLinkPicker } from '../inbox/LinkDialog'
import { detailTime, useCalendar } from '../../state/calendar-store'
import { invoke } from '../../tauri'
import { navigate } from '../router'
import { copyText, openMenu, type MenuEntry } from '../ui'

const RESPONSE_GLYPH: Record<Attendee['response'], string> = { accepted: '✓', tentative: '?', declined: '×', needs_action: '○' }
const CONFERENCE: Record<string, string> = { meet: 'Meet', zoom: 'Zoom', teams: 'Teams', other: 'Call' }

export function joinLabel(url: string): string {
  return url.replace(/^https?:\/\//, '').replace(/\/$/, '')
}

export function orderedAttendees(list: readonly Attendee[]): Attendee[] {
  return [...list.filter((a) => a.organizer), ...list.filter((a) => !a.organizer)]
}

export async function openPrepare(e: CalendarEvent) {
  if (e.prepare_note_id) {
    navigate({ view: 'notes', rest: e.prepare_note_id })
    return
  }
  const id = await useCalendar.getState().prepare(e.id)
  if (id) navigate({ view: 'notes', rest: id })
}

export async function openInGoogle(e: CalendarEvent) {
  if (e.html_link) await invoke('plugin:opener|open_url', { url: e.html_link }).catch(() => {})
}

export function eventMenu(e: CalendarEvent): MenuEntry[] {
  const out: MenuEntry[] = []
  if (e.join_url) out.push({ id: 'join', label: 'Join', run: () => void useCalendar.getState().join(e.id) })
  if (e.html_link) out.push({ id: 'google', label: 'Open in Google Calendar', run: () => void openInGoogle(e) })
  if (e.join_url) out.push({ id: 'copy', label: 'Copy join link', run: () => copyText(e.join_url!, 'Join link copied') })
  out.push({ id: 'link', label: 'Link…', run: () => openLinkPicker({ kind: 'calendar_event', id: e.id }) })
  return out
}

/** The rows of "linked": the core's `event_links`, else what the row carried; the prepare note always first. */
export function linkedRows(e: CalendarEvent, loaded: readonly ThreadLink[] | undefined): ThreadLink[] {
  const base = [...(loaded ?? e.linked ?? [])]
  const prep = e.prepare_note_id
  if (!prep || base.some((l) => l.kind === 'note' && l.id === prep)) return base
  return [{ kind: 'note', id: prep, title: e.title, why: 'prepare note', by_user: false }, ...base]
}

export function EventPanel({ event: e }: { event: CalendarEvent }) {
  const attendees = orderedAttendees(e.attendees ?? [])
  const loaded = useCalendar((s) => s.links[e.id])
  useEffect(() => {
    void useCalendar.getState().loadLinks(e.id)
  }, [e.id])
  const linked = linkedRows(e, loaded)
  const where = [e.calendar_name, e.account_name].filter(Boolean).join(' · ')
  const menu = eventMenu(e)
  return (
    <DetailPanel label={`${e.title} details`} className="cv-panel">
      <div className="m-t-title m-ellipsis" title={e.title} data-testid="event-title">
        {e.title}
      </div>
      <div className="m-meta cv-gap4">{detailTime(e)}</div>
      {where && <div className="m-meta">{where}</div>}
      {e.location && <div className="m-meta">{e.location}</div>}
      {attendees.length > 0 && (
        <>
          <GroupLabel>{`attendees · ${attendees.length}`}</GroupLabel>
          {attendees.map((a) => (
            <Task key={`${a.email ?? a.name}`} glyph={RESPONSE_GLYPH[a.response]} title={a.name} meta={a.organizer ? 'organizer' : undefined} />
          ))}
        </>
      )}
      {e.join_url && (
        <>
          <GroupLabel>join</GroupLabel>
          <Task
            glyph={(() => {
              const id = brandForConference(e.conference)
              return id ? <BrandMark id={id} size={12} label="" /> : '▶'
            })()}
            title={
              <a
                href={e.join_url}
                onClick={(ev) => {
                  ev.preventDefault()
                  void useCalendar.getState().join(e.id)
                }}
              >
                {joinLabel(e.join_url)}
              </a>
            }
            meta={CONFERENCE[e.conference ?? 'other'] ?? 'Call'}
          />
        </>
      )}
      {e.description && (
        <>
          <GroupLabel>description</GroupLabel>
          <div className="cv-desc">{e.description.slice(0, 2048)}</div>
        </>
      )}
      {linked.length > 0 && (
        <>
          <GroupLabel>linked</GroupLabel>
          <LinkRows links={linked} onRemove={(l) => void useCalendar.getState().linkRemove(e.id, l)} />
        </>
      )}
      <ButtonGroup className="cv-actions">
        <Button variant="primary" onClick={() => void openPrepare(e)}>
          {e.prepare_note_id ? 'Open prepare note' : 'Prepare notes'}
        </Button>
        {menu.length > 0 && <MenuButton label={`${e.title} options`} onClick={(ev) => openMenu(ev.currentTarget, menu, `${e.title} options`)} />}
      </ButtonGroup>
      <div className="m-t-meta cv-foot">Mode reads your calendar and never writes to it</div>
    </DetailPanel>
  )
}
