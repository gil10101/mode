// Inbox, Notes and Calendar tabs (02 §7.6–§7.8, §14): rows from their pushes
// when a connector exists, otherwise the empty state that says what to set up.
import { Fragment } from 'react'
import { Button, MenuButton } from '../kit/button'
import { PersonAvatar } from '../kit/avatars'
import { GroupLabel, MsgRow, Task } from '../kit/lists'
import type { AgendaRow, NoteRow } from '../state/contract'
import { useOverlayStore } from '../state/overlay-store'
import { useOverlay } from './context'
import { sinceIso, startsLabel } from './phrases'

function initials(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((w) => w[0]!.toUpperCase())
    .join('')
}

function Overflow({ count, tab }: { count: number; tab: 'inbox' | 'notes' | 'calendar' }) {
  const { act } = useOverlay()
  if (count <= 0) return null
  return (
    <div className="ov-line ov-action" role="button" onClick={() => act('open_tab', undefined, { tab })} data-testid="overflow-row">
      {`+${count} more · open in app`}
    </div>
  )
}

export function InboxTab() {
  const { act, m } = useOverlay()
  const inbox = useOverlayStore((s) => s.inbox)
  const health = useOverlayStore((s) => s.health)
  const max = useOverlayStore((s) => s.settings.max_rows)
  const broken = health?.broken_accounts ?? []
  const rows = inbox?.rows ?? []
  const shown = rows.slice(0, max)
  return (
    <div className={`m-list${shown.length ? ' is-divided' : ''}`} role="list" aria-label="Inbox" data-testid="inbox-list">
      {broken.map((label) => (
        <div className="ov-line" role="status" key={label}>
          <span className="m-dot ov-health" />
          {`${label} needs sign-in`}
          <Button onClick={() => act('connect')}>Re-authorize</Button>
        </div>
      ))}
      {!inbox ? (
        <div className="ov-empty" data-testid="empty-inbox">
          No accounts connected. Connect Gmail or Slack to see urgent messages here.
          <br />
          <Button onClick={() => act('connect')}>Connect</Button>
        </div>
      ) : shown.length === 0 ? (
        <div className="ov-empty" data-testid="empty-inbox">
          {inbox.unread_in_app > 0 ? 'No urgent messages. ' : 'No urgent messages.'}
          {inbox.unread_in_app > 0 && (
            <a
              href="#"
              onClick={(e) => {
                e.preventDefault()
                act('open_tab', undefined, { tab: 'inbox' })
              }}
            >
              {`${inbox.unread_in_app} unread in app`}
            </a>
          )}
        </div>
      ) : (
        shown.map((msg) => (
          <MsgRow
            key={msg.id}
            avatar={<PersonAvatar initials={initials(msg.sender_name)} src={msg.sender_avatar_url ?? undefined} source={msg.provider} />}
            sender={msg.sender_name}
            channel={msg.channel_name ? `#${msg.channel_name}` : undefined}
            mentioned={msg.mentioned}
            time={sinceIso(msg.received_at, m.pillShownAt || Date.now())}
            preview={msg.subject ?? (msg.snippet ?? '').split('\n')[0]}
            urgent={msg.urgency_tier === 'vip' || msg.urgency_tier === 'direct' || msg.urgency_tier === 'ai-urgent'}
            unread={!msg.is_read}
            selected={m.focusRow === msg.id}
            onClick={() => act('open_source', undefined, { tab: 'inbox', id: msg.id })}
            testId={`msg-${msg.id}`}
          />
        ))
      )}
      <Overflow count={rows.length - shown.length} tab="inbox" />
    </div>
  )
}

function noteGlyph(n: NoteRow): string {
  if (n.urgent) return '✱'
  if (n.glyph) return n.glyph
  return n.status === 'done' ? '✓' : n.status === 'doing' ? '◑' : '○'
}

function noteMeta(n: NoteRow): string | undefined {
  if (n.task_total > 0) return `${n.task_done}/${n.task_total}`
  if (n.tags.length) return `#${n.tags[0]}${n.tags.length > 2 ? ` +${n.tags.length - 2}` : n.tags.length === 2 ? ` #${n.tags[1]}` : ''}`
  if (n.when_bucket === 'overdue') return `overdue · ${n.when_date ?? ''}`.trim()
  if (n.when_bucket === 'today' || n.when_bucket === 'tomorrow') return n.when_bucket
  if (n.when_bucket === 'date' && n.when_date) return n.when_date
  return undefined
}

export function NotesTab() {
  const { act, m } = useOverlay()
  const notes = useOverlayStore((s) => s.notes)
  const health = useOverlayStore((s) => s.health)
  const max = useOverlayStore((s) => s.settings.max_rows)
  const sections = notes?.sections ?? []
  const total = sections.reduce((n, s) => n + s.rows.length, 0)
  let budget = max
  return (
    <div className="m-list" role="list" aria-label="Notes" style={{ padding: '4px 8px' }} data-testid="notes-list">
      {health?.vault_missing && (
        <div className="ov-line" role="status">
          <span className="m-dot ov-health" />
          Vault not found
          <Button onClick={() => act('settings')}>Choose vault</Button>
        </div>
      )}
      {total === 0 ? (
        <div className="ov-empty" data-testid="empty-notes">
          Nothing due. Capture a note below.
        </div>
      ) : (
        sections.map((section) => {
          if (budget <= 0) return null
          const rows = section.rows.slice(0, budget)
          budget -= rows.length
          if (!rows.length) return null
          return (
            <Fragment key={section.grouping}>
              <GroupLabel style={{ paddingLeft: 4 }}>{section.grouping.toLowerCase()}</GroupLabel>
              {rows.map((n) => (
                <Task
                  key={n.id}
                  glyph={noteGlyph(n)}
                  title={n.title}
                  meta={noteMeta(n)}
                  done={n.status === 'done'}
                  selected={m.focusRow === n.id}
                  onClick={() => act('open_tab', undefined, { tab: 'notes', id: n.id })}
                  testId={`note-${n.id}`}
                />
              ))}
            </Fragment>
          )
        })
      )}
      <Overflow count={total - (max - budget)} tab="notes" />
    </div>
  )
}

const SECTION_LABEL: Record<AgendaRow['section'], string> = { now: 'now', next: 'next', later: 'later today', tomorrow: 'tomorrow' }

function clock(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? '' : `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

function timeLabel(e: AgendaRow, now: number): string {
  if (e.all_day) return 'all day'
  if (e.section === 'now') {
    const left = Math.max(0, Math.round((Date.parse(e.ends_at) - now) / 60_000))
    return `${left} min left`
  }
  if (e.section === 'next') return startsLabel(e.starts_at, now)
  return clock(e.starts_at)
}

export function CalendarTab() {
  const { act, m, now } = useOverlay()
  const agenda = useOverlayStore((s) => s.agenda)
  const max = useOverlayStore((s) => s.settings.max_rows)
  const rows = agenda?.rows ?? []
  const shown = rows.slice(0, max)
  const sections = (['now', 'next', 'later', 'tomorrow'] as const).map((s) => ({ s, rows: shown.filter((e) => e.section === s) })).filter((g) => g.rows.length)
  return (
    <div className="m-list" role="list" aria-label="Calendar" data-testid="calendar-list">
      {!agenda ? (
        <div className="ov-empty" data-testid="empty-calendar">
          No calendar connected.
          <br />
          <Button onClick={() => act('connect')}>Connect Google</Button>
        </div>
      ) : shown.length === 0 ? (
        <div className="ov-empty" data-testid="empty-calendar">
          Nothing scheduled today.
        </div>
      ) : (
        sections.map(({ s, rows: list }) => (
          <Fragment key={s}>
            <GroupLabel>{SECTION_LABEL[s]}</GroupLabel>
            {list.map((e) => (
              <div
                key={e.id}
                className={`m-row is-dense${m.focusRow === e.id ? ' is-selected' : ''}`}
                role="button"
                aria-label={`${e.title}, ${timeLabel(e, now)}`}
                onClick={() => act(e.join_url ? 'join' : 'open_source', undefined, { tab: 'calendar', id: e.id })}
                data-testid={`event-${e.id}`}
              >
                <span className="ov-time">{timeLabel(e, now)}</span>
                <span className="ov-account" style={e.account_color ? { background: e.account_color } : undefined} title={e.calendar_name ?? undefined} />
                <div className="m-row-text">
                  <div className="m-row-name" style={e.status === 'cancelled' ? { textDecoration: 'line-through' } : undefined}>
                    {`${e.title}${e.my_response === 'tentative' || e.my_response === 'needs_action' ? ' ?' : ''}`}
                  </div>
                  <div className="m-row-detail">
                    {`${e.all_day ? 'all day' : `${clock(e.starts_at)}–${clock(e.ends_at)}`}${e.conference ? ` · ${e.conference === 'meet' ? 'Meet' : e.conference === 'zoom' ? 'Zoom' : e.conference === 'teams' ? 'Teams' : 'call'}` : e.calendar_name ? ` · ${e.calendar_name}` : ''}${e.location ? ` · ${e.location}` : ''}`}
                  </div>
                </div>
                {e.join_url ? (
                  <Button
                    onClick={(ev) => {
                      ev.stopPropagation()
                      act('join', undefined, { tab: 'calendar', id: e.id })
                    }}
                  >
                    Join
                  </Button>
                ) : (
                  <span className="m-row-meta" />
                )}
                <MenuButton label={`More for ${e.title}`} onClick={(ev) => ev.stopPropagation()} />
              </div>
            ))}
          </Fragment>
        ))
      )}
      <Overflow count={rows.length - shown.length} tab="calendar" />
    </div>
  )
}
