// The Inbox detail panel (03 §4.1; app-window.html #i-dhead, #i-why, #i-msgs):
// the sender head with Open (the thread view, the drill-in rule's full page),
// Snooze and ⋮; the level and why line (a menu of levels, 03 §4.5); the last
// messages; links (sessions, notes and messages the thread is about, with
// why; app/links.tsx); then the contact card (important-person toggle, identities), the
// account and source, attachments, the snooze state and whether triage
// reaches the source (06 §2.5).
import { PersonAvatar } from '../../kit/avatars'
import { Button, ButtonGroup, MenuButton } from '../../kit/button'
import { Switch } from '../../kit/controls'
import { DetailPanel } from '../../kit/layout'
import { GroupLabel } from '../../kit/lists'
import type { ThreadRow } from '../../state/contract-connectors'
import { backLabel, chatPill, initials, isSnoozed, levelOf, rowTime, sourceOf, useInbox } from '../../state/inbox-store'
import { LinkRows } from '../links'
import { openMenu } from '../ui'
import { levelEntries, openInSource, openThread, showThreadMenu } from './actions'
import { PanelMessage } from './rows'
import { openSnooze } from './SnoozeMenu'

/** How many of the latest messages the panel shows (the thread view has all). */
export const PANEL_MESSAGES = 5

/** "urgent · Important person: Sam Okafor (your manager)" (03 §4.1). A read thread has no tier (07 §3.5); its latest message's level stands in. */
export function whyLine(row: Pick<ThreadRow, 'urgency_tier' | 'urgency_why'>, fallback?: ThreadRow['urgency_tier']): string {
  const tier = row.urgency_tier ?? fallback ?? null
  const level = levelOf(tier)
  if (tier === 'unscored') return `${level} · scoring…`
  return row.urgency_why ? `${level} · ${row.urgency_why}` : level
}

export function metaLine(row: ThreadRow, now: number): string {
  const chat = chatPill(row) ?? (row.kind === 'dm' ? 'DM' : row.kind === 'group_dm' ? 'Group DM' : null)
  return [chat, row.account_label, rowTime(row.last_message_at, now)].filter(Boolean).join(' · ')
}

export function ThreadPanel({ row, now }: { row: ThreadRow; now: number }) {
  const detail = useInbox((s) => s.details[row.id])
  const error = useInbox((s) => s.detailError[row.id])
  const local = useInbox((s) => s.localOnly[row.id])
  const memory = useInbox((s) => s.tierMemory[row.id])
  const slack = row.provider === 'slack'
  const snoozed = isSnoozed(row, now)
  const messages = detail?.messages.slice(-PANEL_MESSAGES) ?? []
  const hidden = (detail?.messages.length ?? 0) - messages.length
  const attachments = detail?.messages.reduce((n, m) => n + m.attachments, 0) ?? (row.has_attachments ? 1 : 0)
  const open = () => openThread(row.id)

  return (
    <DetailPanel label={`${row.sender_name} summary`} className="ib-panel">
      <div className="m-session-head" data-testid="thread-head">
        <PersonAvatar size={32} initials={initials(row.sender_name)} src={row.sender_avatar_url ?? undefined} source={sourceOf(row.provider)} />
        <div className="m-ellipsis">
          <div className="m-session-title">{row.sender_name}</div>
          <div className="m-meta">{metaLine(row, now)}</div>
        </div>
        <ButtonGroup>
          <Button variant="primary" onClick={open}>
            Open
          </Button>
          {snoozed ? (
            <Button onClick={() => void useInbox.getState().unsnooze(row.id)}>Cancel snooze</Button>
          ) : (
            <Button onClick={(e) => openSnooze(e.currentTarget, [row.id], detail?.messages[detail.messages.length - 1]?.id)}>Snooze</Button>
          )}
          <MenuButton label={`${row.sender_name} options`} onClick={(e) => showThreadMenu(e.currentTarget, row, detail)} />
        </ButtonGroup>
      </div>
      <div
        className="m-meta ib-why"
        role="button"
        tabIndex={0}
        aria-haspopup="menu"
        title="Change the level"
        data-testid="why-line"
        onClick={(e) => openMenu(e.currentTarget, levelEntries(row), 'Level')}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            openMenu(e.currentTarget, levelEntries(row), 'Level')
          }
        }}
      >
        {whyLine(row, memory ?? detail?.messages[detail.messages.length - 1]?.urgency_tier)}
      </div>
      <hr className="m-divider ib-rule" />
      <div className="ib-msgs" data-testid="panel-messages">
        {hidden > 0 && (
          <div className="ib-more">
            {`${hidden} earlier ${hidden === 1 ? 'message' : 'messages'} · `}
            <a href={`#/inbox/thread/${row.id}`} onClick={(e) => (e.preventDefault(), open())}>
              Open
            </a>
          </div>
        )}
        {messages.map((m) => (
          <PanelMessage key={m.id} m={m} slack={slack} now={now} />
        ))}
        {!detail && !error && <div className="ib-more">Loading…</div>}
      </div>
      {error && (
        <div className="ib-error" role="alert">
          <span>{error}</span>
          <Button onClick={() => void useInbox.getState().loadDetail(row.id)}>Retry</Button>
          <Button quiet onClick={() => void openInSource(row.id)}>
            Open in source
          </Button>
        </div>
      )}
      {detail?.links && detail.links.length > 0 && (
        <>
          <GroupLabel>links</GroupLabel>
          <LinkRows links={detail.links} onRemove={(l) => void useInbox.getState().linkRemove(row.id, l)} />
        </>
      )}
      <GroupLabel>sender</GroupLabel>
      <div className="ib-vip">
        <span title={detail?.contact?.identities.join(', ') ?? row.sender_address ?? undefined}>
          {`${detail?.contact?.display_name ?? row.sender_name}${(detail?.contact?.identities.length ?? 0) > 0 ? ` · ${detail!.contact!.identities.join(', ')}` : row.sender_address ? ` · ${row.sender_address}` : ''}`}
        </span>
        <Switch on={!!row.sender_vip} label="Important person" onChange={(on) => void useInbox.getState().setVip(row.id, on)} />
        <span className="m-t-meta">Important person</span>
      </div>
      <GroupLabel>details</GroupLabel>
      <div className="ib-facts" data-testid="thread-facts">
        <span>account</span>
        <span>{row.account_label ?? '—'}</span>
        <span>source</span>
        <span>{detail?.source_name ?? (slack ? 'Slack' : 'Gmail inbox')}</span>
        <span>messages</span>
        <span>{`${row.message_count} · ${row.unread_count} unread`}</span>
        <span>attachments</span>
        <span>{attachments ? `${attachments}` : 'none'}</span>
        <span>snooze</span>
        <span>{snoozed && row.snoozed_until ? `until ${backLabel(row.snoozed_until, now)}` : row.resurfaced ? 'resurfaced' : 'not snoozed'}</span>
        {detail?.app_local_actions && (
          <>
            <span>write-back</span>
            <span title="Triage that stays in Mode for this account">{detail.app_local_actions.length ? `in Mode only: ${detail.app_local_actions.map((a) => a.replace(/_/g, ' ')).join(', ')}` : `to ${slack ? 'Slack' : 'Gmail'}`}</span>
          </>
        )}
      </div>
      {local && <div className="ib-local">{local.charAt(0).toUpperCase() + local.slice(1)}</div>}
    </DetailPanel>
  )
}
