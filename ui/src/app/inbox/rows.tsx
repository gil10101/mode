// Thread rows and message items (03 §4.2, §4.5; 10 §7.4.1). A thread row is
// the kit's `.m-msg` with one difference the kit's MsgRow cannot draw: DMs
// carry no chat pill, only the `@` (03 §4.2 "nothing for DMs and group DMs
// beyond the @ mark"); kit candidate. Urgent = the 2-px accent bar; the why
// line, the account and the snooze state live in the tooltip and the panel.
import type { KeyboardEvent, MouseEvent, ReactNode } from 'react'
import { PersonAvatar } from '../../kit/avatars'
import { cx } from '../../kit/cx'
import type { MessageRow, ThreadRow } from '../../state/contract-connectors'
import { backLabel, chatPill, initials, isSnoozed, isUrgentRow, levelOf, rowTime, sourceOf } from '../../state/inbox-store'
import { highlight, renderMrkdwn } from './mrkdwn'

export function rowTooltip(r: ThreadRow, now: number): string {
  const parts = [r.account_label, levelOf(r.urgency_tier)]
  if (r.urgency_why) parts.push(r.urgency_why)
  if (isSnoozed(r, now) && r.snoozed_until) parts.push(`snoozed until ${backLabel(r.snoozed_until, now)}`)
  else if (r.resurfaced) parts.push('resurfaced')
  if (r.has_attachments) parts.push('attachments')
  if (r.pending_writeback) parts.push('waiting for the source to confirm')
  return parts.filter(Boolean).join(' · ')
}

export interface ThreadItemProps {
  row: ThreadRow
  now: number
  selected?: boolean
  /** Search results: the FTS hit replaces line 2. */
  query?: string
  onClick?: (e: MouseEvent<HTMLDivElement>) => void
  onKeyDown?: (e: KeyboardEvent<HTMLDivElement>) => void
}

export function ThreadItem({ row, now, selected, query, onClick, onKeyDown }: ThreadItemProps) {
  const pill = chatPill(row)
  const snoozed = isSnoozed(row, now)
  const line2: ReactNode = snoozed && row.snoozed_until ? `back ${backLabel(row.snoozed_until, now)}` : query && row.hit ? <span className="ib-hit">{highlight(row.hit, query)}</span> : row.kind === 'email' ? row.title : (row.last_snippet ?? '').split('\n')[0]
  return (
    <div
      className={cx('m-msg', isUrgentRow(row) && 'is-urgent', row.unread_count > 0 && 'is-unread', selected && 'is-selected')}
      role="option"
      aria-selected={!!selected}
      tabIndex={0}
      title={rowTooltip(row, now)}
      data-testid={`thread-${row.id}`}
      data-thread={row.id}
      onClick={onClick}
      onKeyDown={onKeyDown}
    >
      <PersonAvatar initials={initials(row.sender_name)} src={row.sender_avatar_url ?? undefined} source={sourceOf(row.provider)} />
      <div className="m-msg-body">
        <div className="m-msg-l1">
          <span className="m-msg-name">{row.sender_name}</span>
          {pill && <span className="m-tag">{pill}</span>}
          {row.mentioned && <span className="m-at">@</span>}
          {row.pending_writeback && (
            <span className="ib-pending" title="Waiting for the source to confirm" aria-label="pending">
              ↻
            </span>
          )}
          <span className="m-msg-time">{rowTime(row.last_message_at, now)}</span>
        </div>
        <div className="m-msg-l2">{line2}</div>
      </div>
    </div>
  )
}

/** Message body: Markdown-light for Slack, plain for email (03 §4.5). */
export function MessageBody({ m, slack, offline }: { m: MessageRow; slack: boolean; offline?: boolean }) {
  if (m.body_text === null) {
    return <div className="aw-mtext">{offline ? `Body not available offline — snippet shown: ${m.snippet}` : `Loading from ${slack ? 'Slack' : 'Gmail'}…`}</div>
  }
  return (
    <div className="aw-mtext">
      {slack ? renderMrkdwn(m.body_text) : m.body_text}
      {m.body_truncated && <span className="m-t-meta"> · truncated at 32 KB · open in source</span>}
    </div>
  )
}

/** A message of the panel (the wireframe's `#i-msgs`): avatar, sender, time, body. */
export function PanelMessage({ m, slack, now }: { m: MessageRow; slack: boolean; now: number }) {
  return (
    <div className="m-msg" data-message={m.id}>
      <PersonAvatar initials={initials(m.sender_name)} src={m.sender_avatar_url ?? undefined} source={slack ? 'slack' : 'gmail'} />
      <div className="m-msg-body">
        <div className="m-msg-l1">
          <span className="m-msg-name" title={m.sender_address}>
            {m.sender_name}
          </span>
          <span className="m-msg-time">{rowTime(m.received_at, now)}</span>
        </div>
        <MessageBody m={m} slack={slack} />
      </div>
    </div>
  )
}
