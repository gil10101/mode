// The thread of 03 §7.1: user bubbles, activity groups, the shimmer while a
// reply is pending, replies with inline citations and the list under them
// (§7.4), action cards that run only after Confirm (§7.5), and the inline
// error with Retry (§7.6). Nothing else sits in the column.
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Button } from '../../kit/button'
import { Activity, ActivityRow, BUBBLE_CLAMP_CHARS, BubbleUser, Shimmer } from '../../kit/chat'
import { Chip, Notice } from '../../kit/controls'
import { Icon, type IconId } from '../../kit/icons'
import type { ActivityGroup, ActivityRowSpec, ChatAction, ChatThread, ChatTurn, Citation, ItemKind, ItemRef } from '../../state/contract-chat'
import { canDecide, itemRoute, itemTitle, relTime, useChat } from '../../state/chat-store'
import { navigate } from '../router'
import { copyText, useNow } from '../ui'
import { ReplyMarkdown } from './markdown'

export const KIND_ICON: Record<ItemKind, IconId> = { note: 'notes', session: 'sessions', message: 'inbox', event: 'calendar', file: 'file' }

export function openItem(it: Pick<ItemRef, 'kind' | 'id' | 'route'>) {
  const route = itemRoute(it)
  if (route) navigate(route)
}

export function ItemChip({ item, n, onRemove }: { item: ItemRef; n?: number; onRemove?: () => void }) {
  return (
    <Chip removable={!!onRemove} onRemove={onRemove} onClick={onRemove ? undefined : () => openItem(item)}>
      {n !== undefined && <span className="ch-n">{n}</span>}
      <Icon id={KIND_ICON[item.kind] ?? 'file'} />
      <span className="m-ellipsis">{itemTitle(item)}</span>
    </Chip>
  )
}

function Group({ g, onRow }: { g: ActivityGroup; onRow: (r: ActivityRowSpec) => void }) {
  const [open, setOpen] = useState(false)
  const status = open && g.status ? g.status : undefined
  return (
    <Activity label={g.label} open={open} onToggle={setOpen} active={g.active} status={status}>
      {open && g.rows.length
        ? g.rows.map((r, i) => (
            <ActivityRow key={i} thumb={r.thumb ?? undefined} onClick={() => onRow(r)}>
              {r.text}
            </ActivityRow>
          ))
        : undefined}
    </Activity>
  )
}

function UserTurn({ turn }: { turn: ChatTurn }) {
  if (!turn.attachments?.length) return <BubbleUser>{turn.text}</BubbleUser>
  return (
    <BubbleUser clamp={turn.text.length > BUBBLE_CLAMP_CHARS}>
      {turn.text}
      <div className="ch-attach">
        {turn.attachments.map((a) => (
          <ItemChip key={`${a.kind}:${a.id}`} item={a} />
        ))}
      </div>
    </BubbleUser>
  )
}

function ActionCard({ turnId, action, thread }: { turnId: string; action: ChatAction; thread: ChatThread }) {
  const decide = useChat((s) => s.decide)
  const first = canDecide(thread, turnId, action.id)
  if (action.state === 'done' || action.state === 'rejected' || action.state === 'failed') {
    return (
      <div className="ch-result" data-testid={`action-result-${action.id}`} role="status">
        {action.state === 'done' && (
          <>
            <span>{action.result ?? `${action.title} done`}</span>
            {action.result_route && (
              <a
                href={`#/${action.result_route}`}
                onClick={(e) => {
                  e.preventDefault()
                  navigate(action.result_route!)
                }}
              >
                Open
              </a>
            )}
          </>
        )}
        {action.state === 'rejected' && <span className="m-fg-3">{`Rejected · ${action.title}`}</span>}
        {action.state === 'failed' && (
          <span>{`Couldn't ${action.title.toLowerCase()}: ${action.error ?? 'failed'}`}</span>
        )}
      </div>
    )
  }
  const running = action.state === 'running'
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget || running || !first) return
    if (e.key === 'Enter') {
      e.preventDefault()
      void decide(turnId, action.id, true)
    } else if (e.key === 'Escape') {
      e.preventDefault()
      e.stopPropagation()
      void decide(turnId, action.id, false)
    }
  }
  return (
    <div className="m-card ch-action" role="group" aria-label={`Proposed: ${action.title}`} tabIndex={0} onKeyDown={onKey} data-testid={`action-${action.id}`}>
      <div className="m-card-head">
        <span className="m-card-title">{action.title}</span>
        {!first && !running && <span className="m-t-meta">next</span>}
      </div>
      {action.fields.length > 0 && (
        <div className="ch-fields">
          {action.fields.map((f) => (
            <FieldLine key={f.label} label={f.label} value={f.value} before={f.before} />
          ))}
        </div>
      )}
      {action.preview && <pre className="ch-preview">{action.preview}</pre>}
      <div className="m-card-actions">
        <Button variant={first ? 'primary' : 'default'} disabled={!first || running} onClick={() => void decide(turnId, action.id, true)}>
          Confirm
        </Button>
        <Button disabled={!first || running} onClick={() => void decide(turnId, action.id, false)}>
          Reject
        </Button>
        {running && <span className="m-spinner" aria-label="Running" />}
      </div>
    </div>
  )
}

function FieldLine({ label, value, before }: { label: string; value: string; before?: string | null }) {
  return (
    <>
      <span className="m-t-meta">{label}</span>
      <span>
        {before ? (
          <>
            <span className="ch-before">{before}</span>
            {` → ${value}`}
          </>
        ) : (
          value
        )}
      </span>
    </>
  )
}

function AssistantTurn({ turn, thread, onRow }: { turn: ChatTurn; thread: ChatThread; onRow: (r: ActivityRowSpec) => void }) {
  const now = useNow(30_000)
  const pin = useChat((s) => s.pin)
  const retry = useChat((s) => s.retry)
  const streaming = turn.status === 'streaming'
  const citations = turn.citations ?? []
  const onCite = (c: Citation) => openItem(c)
  return (
    <>
      {(turn.activity ?? []).map((g) => (
        <Group key={g.id} g={g} onRow={onRow} />
      ))}
      {streaming && !turn.text && <Shimmer lines={2} />}
      {turn.text && (
        <div className="m-reply" data-testid={`reply-${turn.id}`}>
          <ReplyMarkdown text={turn.text} citations={citations} onOpen={onCite} onCopyCode={(code) => copyText(code, 'Copied code')} />
          {!streaming && citations.length > 0 && (
            <div className="ch-cites" aria-label="Sources">
              {citations.map((c) => (
                <ItemChip key={c.n} item={c} n={c.n} />
              ))}
            </div>
          )}
          {!streaming && turn.no_sources && <div className="m-t-meta ch-nosrc">no sources — answered from the question alone</div>}
          {turn.status === 'stopped' && <div className="m-t-meta ch-stopped">stopped</div>}
          {!streaming && (
            <div className="m-reply-actions">
              <Button variant="icon" quiet aria-label="Copy" onClick={() => copyText(turn.text)}>
                <Icon id="copy" />
              </Button>
              <Button variant="icon" quiet aria-label={turn.pinned ? 'Unpin' : 'Pin'} aria-pressed={!!turn.pinned} onClick={() => void pin(turn.id, !turn.pinned)}>
                <Icon id="pin" />
              </Button>
              <span className="m-reply-time" title={new Date(turn.created_at).toLocaleString()}>
                {relTime(turn.created_at, now)}
              </span>
            </div>
          )}
        </div>
      )}
      {!turn.text && !streaming && turn.status !== 'failed' && turn.no_sources && <div className="m-t-meta ch-nosrc">no sources — answered from the question alone</div>}
      {(turn.actions ?? []).map((a) => (
        <ActionCard key={a.id} turnId={turn.id} action={a} thread={thread} />
      ))}
      {turn.status === 'failed' && (
        <Notice
          state="bad"
          className="ch-error"
          testId={`turn-error-${turn.id}`}
          action={
            <Button quiet onClick={() => void retry(turn.id)}>
              Retry
            </Button>
          }
        >
          {turn.error ?? 'failed'}
        </Notice>
      )}
    </>
  )
}

export function Conversation({ thread, onRow }: { thread: ChatThread; onRow: (r: ActivityRowSpec) => void }) {
  const ref = useRef<HTMLDivElement>(null)
  const stick = useRef(true)
  const streaming = thread.turns.some((t) => t.status === 'streaming')
  const size = thread.turns.length + thread.turns.reduce((n, t) => n + t.text.length + (t.actions?.length ?? 0), 0)

  // A new conversation or a new turn scrolls to the end; a streaming reply keeps it there unless the user scrolled up.
  useLayoutEffect(() => {
    stick.current = true
    const el = ref.current
    if (el) el.scrollTop = el.scrollHeight
  }, [thread.id, thread.turns.length])
  useLayoutEffect(() => {
    const el = ref.current
    if (el && stick.current) el.scrollTop = el.scrollHeight
  }, [size])
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const on = () => {
      stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48
    }
    el.addEventListener('scroll', on)
    return () => el.removeEventListener('scroll', on)
  }, [])

  return (
    <div className="m-chat-thread" ref={ref} aria-label="Conversation" aria-busy={streaming} data-testid="chat-thread">
      {thread.turns.map((t) => (t.role === 'user' ? <UserTurn key={t.id} turn={t} /> : <AssistantTurn key={t.id} turn={t} thread={thread} onRow={onRow} />))}
    </div>
  )
}
