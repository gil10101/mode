// The thread view (03 §4.5; the drill-in rule's full page, `inbox/thread/<id>`
// and `inbox/message/<id>`): the header (title, which chat, participants,
// account, source, message count), every message in order with its level and
// why line, the body (Slack mrkdwn-light, email plain; past retention a fetch
// or the snippet offline), attachments, and the sticky action bar: Open in
// source as the primary and ⋮ for the rest. Read on open after 1 s; Shift+Enter
// opened it without marking (03 §10.3).
import { useEffect, useRef } from 'react'
import { PersonAvatar } from '../../kit/avatars'
import { Button, MenuButton } from '../../kit/button'
import { MainHead } from '../../kit/layout'
import type { MessageRow, ThreadRow } from '../../state/contract-connectors'
import { chatPill, initials, levelOf, rowTime, useInbox } from '../../state/inbox-store'
import { useSessionsStore } from '../../state/sessions-store'
import { navigate } from '../router'
import { useNow } from '../ui'
import { openInSource, showThreadMenu } from './actions'
import { MessageBody } from './rows'

function messageWhy(m: MessageRow): string {
  const level = levelOf(m.urgency_tier)
  if (m.urgency_tier === 'unscored') return `${level} · scoring…`
  return m.urgency_why ? `${level} · ${m.urgency_why}` : level
}

export function ThreadPage({ id, messageId, markRead }: { id: string; messageId?: string; markRead: boolean }) {
  const now = useNow(30_000)
  const detail = useInbox((s) => s.details[id])
  const listRow = useInbox((s) => s.rows.find((r) => r.id === id))
  const error = useInbox((s) => s.detailError[id])
  const local = useInbox((s) => s.localOnly[id])
  const offline = useSessionsStore((s) => !!s.health?.offline)
  const scroller = useRef<HTMLDivElement>(null)
  const row: ThreadRow | undefined = listRow ?? detail?.thread

  useEffect(() => {
    useInbox.getState().select(id, { markRead })
    return () => {
      if (useInbox.getState().selectedId === id) useInbox.getState().select(null)
    }
  }, [id, markRead])

  // `inbox/message/<id>`: scrolled to that message.
  useEffect(() => {
    if (!messageId || !detail) return
    scroller.current?.querySelector(`[data-message="${messageId}"]`)?.scrollIntoView?.({ block: 'center' })
  }, [messageId, detail])

  const back = () => navigate({ view: 'inbox' })

  // Esc goes up one level (03 §2.4); the shell's handler only knows Sessions.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented) return
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
      e.preventDefault()
      back()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const slack = row?.provider === 'slack'
  const chat = row ? (chatPill(row) ?? (row.kind === 'dm' ? 'DM' : row.kind === 'group_dm' ? 'Group DM' : '')) : ''
  return (
    <section className="ap-view ib-view" aria-label="Thread" data-testid="thread-page">
      <MainHead title={row?.title ?? 'Thread'} titleTestId="view-title">
        <Button quiet aria-label="Back to Inbox" onClick={back}>
          ‹ Inbox
        </Button>
      </MainHead>
      <div className="ib-page">
        {row && (
          <div className="ib-page-head">
            <div className="m-meta">{[chat, `${row.message_count} ${row.message_count === 1 ? 'message' : 'messages'}`, row.account_label, slack ? 'Slack' : 'Gmail'].filter(Boolean).join(' · ')}</div>
            {detail && detail.participants.length > 0 && (
              <div className="m-meta" title={detail.participants.map((p) => p.address ?? p.name).join(', ')}>
                {detail.participants.map((p) => p.name).join(', ')}
              </div>
            )}
          </div>
        )}
        <div className="ib-page-msgs" ref={scroller} data-testid="thread-messages">
          {detail?.messages.map((m) => (
            <div key={m.id} className={`m-msg${m.id === messageId ? ' is-target' : ''}`} data-message={m.id}>
              <PersonAvatar initials={initials(m.sender_name)} src={m.sender_avatar_url ?? undefined} source={slack ? 'slack' : 'gmail'} />
              <div className="m-msg-body">
                <div className="m-msg-l1">
                  <span className="m-msg-name" title={m.sender_address}>
                    {m.sender_name}
                  </span>
                  {!m.is_read && <span className="m-tag">unread</span>}
                  <span className="m-msg-time">{rowTime(m.received_at, now)}</span>
                </div>
                <div className="m-meta ib-msg-why">{messageWhy(m)}</div>
                <MessageBody m={m} slack={slack} offline={offline} />
                {m.attachments > 0 && <div className="m-t-meta">{`${m.attachments === 1 ? '1 attachment' : `${m.attachments} attachments`} · open in source to download`}</div>}
              </div>
            </div>
          ))}
          {!detail && !error && <div className="ib-more">Loading…</div>}
          {error && (
            <div className="ib-error" role="alert">
              <span>{error}</span>
              <Button onClick={() => void useInbox.getState().loadDetail(id)}>Retry</Button>
              <Button quiet onClick={() => void openInSource(id)}>
                Open in source
              </Button>
            </div>
          )}
        </div>
        {row && (
          <div className="ib-actionbar">
            <Button variant="primary" onClick={() => void openInSource(row.id)}>
              Open in source
            </Button>
            <span className="ib-grow">{local ? local.charAt(0).toUpperCase() + local.slice(1) : rowTime(row.last_message_at, now)}</span>
            <MenuButton label="Thread options" onClick={(e) => showThreadMenu(e.currentTarget, row, detail, { page: true })} />
          </div>
        )}
      </div>
    </section>
  )
}
