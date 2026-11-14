// The Overview thread (03 §3.7) and the agent thread (03 §3.9A): one
// renderer for the item table. Items arrive paired and ordered from the core;
// this file sorts them (occurred_at, then seq), groups an agent's work into
// one activity group, and in agent threads folds runs of the same tool into
// one group ("Read 5 files").
import type { ReactNode } from 'react'
import { Activity } from '../../kit/chat'
import { Thread, ThreadAssistant, ThreadTool, ThreadUser } from '../../kit/cards'
import type { Pending } from '../../state/contract'
import type { TranscriptItem } from '../../state/contract-sessions'
import { navigate } from '../router'
import { AnsweredPermissionView, PendingCard, QuestionItemView } from './cards'
import { clock, elapsed, between, stateWord, tildePath } from './format'
import { Markdown } from './markdown'

type Item = TranscriptItem
type ToolItem = Extract<Item, { kind: 'tool' }>

export function sortItems(items: readonly Item[]): Item[] {
  return [...items].sort((a, b) => {
    const ta = Date.parse(a.at)
    const tb = Date.parse(b.at)
    if (!Number.isNaN(ta) && !Number.isNaN(tb) && ta !== tb) return ta - tb
    return a.seq - b.seq
  })
}

/**
 * The Overview's own items: everything without an `agent_id`, plus the one
 * `agent` item per agent (the core tags it with the agent's id too); an
 * agent's other items live in its activity group and its thread.
 */
export function overviewItems(items: readonly Item[]): Item[] {
  return sortItems(items.filter((i) => i.kind === 'agent' || !i.agent_id))
}

export function agentItems(items: readonly Item[], agentId: string): Item[] {
  return sortItems(items.filter((i) => i.kind !== 'agent' && i.agent_id === agentId))
}

export type Block = { kind: 'item'; item: Item } | { kind: 'tools'; tool: string; items: ToolItem[] }

/** Runs of three or more calls of the same tool become one group (agent threads). */
export function groupTools(items: readonly Item[], min = 3): Block[] {
  const out: Block[] = []
  let run: ToolItem[] = []
  const flush = () => {
    if (run.length >= min) out.push({ kind: 'tools', tool: run[0].tool_name, items: run })
    else for (const item of run) out.push({ kind: 'item', item })
    run = []
  }
  for (const item of items) {
    if (item.kind === 'tool' && !item.failed && (!run.length || run[0].tool_name === item.tool_name)) {
      run.push(item)
      continue
    }
    flush()
    if (item.kind === 'tool' && !item.failed) run.push(item)
    else out.push({ kind: 'item', item })
  }
  flush()
  return out
}

/** "Read 5 files", "Ran 3 commands" (the Chat activity wording, 10 §7.4.11). */
export function toolGroupLabel(tool: string, n: number): string {
  switch (tool) {
    case 'Read':
      return `Read ${n} files`
    case 'Bash':
      return `Ran ${n} commands`
    case 'Edit':
    case 'MultiEdit':
    case 'Write':
      return `Edited ${n} files`
    case 'Grep':
    case 'Glob':
      return `Searched ${n} times`
    default:
      return `${tool} · ${n} calls`
  }
}

function toolMeta(item: ToolItem): ReactNode {
  const parts: string[] = []
  if (item.failed) parts.push('failed')
  if (item.meta) parts.push(item.meta)
  else if (item.duration_ms != null && !(item.lines_added != null || item.lines_removed != null)) parts.push(elapsed(item.duration_ms / 1000))
  return parts.length ? parts.join(' · ') : undefined
}

export function ToolRow({ item }: { item: ToolItem }) {
  const hasDiff = !item.failed && !item.meta && (item.lines_added != null || item.lines_removed != null)
  return (
    <ThreadTool name={item.tool_name} arg={item.summary} meta={toolMeta(item)} diff={hasDiff ? { add: item.lines_added ?? 0, del: item.lines_removed ?? 0 } : undefined}>
      {item.body ?? undefined}
    </ThreadTool>
  )
}

function Divider({ children }: { children: ReactNode }) {
  return <div className="ap-line">{children}</div>
}

export interface ThreadContext {
  sessionId: string
  /** Row facts the open card needs (jump target). */
  row: { id: string; state: string; launch_mode: string; cwd: string; ended_at?: string | null; dismissed_at?: string | null }
  pending: Pending | null | undefined
  now: number
}

function ItemView({ item, ctx }: { item: Item; ctx: ThreadContext }) {
  switch (item.kind) {
    case 'start':
      return <Divider>{`started · ${item.source}`}</Divider>
    case 'end':
      return <Divider>{endText(item.reason)}</Divider>
    case 'user':
      if (item.text === null)
        return (
          <div className="ap-hidden-prompt">
            {'prompt · hidden by setting · '}
            <a
              href="#/settings/general"
              onClick={(e) => {
                e.preventDefault()
                navigate('settings/general')
              }}
            >
              Settings
            </a>
          </div>
        )
      return <ThreadUser>{item.text}</ThreadUser>
    case 'assistant':
      return (
        <ThreadAssistant>
          <Markdown text={item.text} />
        </ThreadAssistant>
      )
    case 'tool':
      return <ToolRow item={item} />
    case 'permission':
      if (item.status === 'open' && ctx.pending?.kind === 'permission') return <PendingCard row={ctx.row} pending={ctx.pending} />
      return <AnsweredPermissionView item={item} sessionId={ctx.sessionId} />
    case 'question':
      if (item.status === 'open' && ctx.pending?.kind === 'question') return <PendingCard row={ctx.row} pending={ctx.pending} />
      return <QuestionItemView item={item} />
    case 'state':
      return (
        <Divider>{`${stateWord(item.from)} → ${stateWord(item.to)} · ${clock(item.at)}`}</Divider>
      )
    case 'compaction':
      return <Divider>context compacted</Divider>
    case 'model_switch':
      // The model is named once, in the header (owner rule); the divider only marks the switch.
      return <Divider>{`model changed · ${clock(item.at)}`}</Divider>
    case 'cwd':
      return <Divider>{`cwd: ${tildePath(item.cwd)}`}</Divider>
    case 'notice':
      return <div className="ap-info">{item.text}</div>
    case 'agent': {
      const running = !item.ended_at
      const secs = between(item.started_at, item.ended_at, ctx.now)
      return (
        <Activity label={`${item.agent_type} · ${elapsed(secs)}`} active={running}>
          {item.tools.map((t, i) => (
            <div key={i} className="m-activity-row">
              <span>
                <code>{t.tool_name}</code>
                {` ${t.summary}`}
              </span>
            </div>
          ))}
          <div
            className="m-activity-row"
            role="button"
            tabIndex={0}
            onClick={() => navigate({ view: 'sessions', id: ctx.sessionId, tab: 'agents', sub: item.agent_id })}
            onKeyDown={(e) => {
              if (e.key === 'Enter') navigate({ view: 'sessions', id: ctx.sessionId, tab: 'agents', sub: item.agent_id })
            }}
          >
            <span>Open agent thread</span>
          </div>
        </Activity>
      )
    }
  }
}

/** The Overview thread; an open card with no item of its own is appended at the end. */
export function OverviewThread({ items, ctx }: { items: readonly Item[]; ctx: ThreadContext }) {
  const list = overviewItems(items)
  const hasOpenCard = list.some((i) => (i.kind === 'permission' || i.kind === 'question') && i.status === 'open')
  return (
    <Thread>
      {list.map((item) => (
        <ItemView key={`${item.kind}:${item.seq}`} item={item} ctx={ctx} />
      ))}
      {!hasOpenCard && ctx.pending && (ctx.pending.kind === 'permission' || ctx.pending.kind === 'question') && <PendingCard row={ctx.row} pending={ctx.pending} />}
    </Thread>
  )
}

/** An agent's own thread: read-only, same items, runs of one tool grouped. */
export function AgentThread({ items, ctx, running }: { items: readonly Item[]; ctx: ThreadContext; running: boolean }) {
  const blocks = groupTools(items)
  return (
    <Thread>
      {blocks.map((b, i) =>
        b.kind === 'item' ? (
          <ItemView key={`${b.item.kind}:${b.item.seq}`} item={b.item} ctx={{ ...ctx, pending: null }} />
        ) : (
          <Activity key={`g${i}`} label={toolGroupLabel(b.tool, b.items.length)} defaultOpen={running}>
            {b.items.map((t) => (
              <div key={t.seq} className="m-activity-row">
                <span>
                  <code>{t.tool_name}</code>
                  {` ${t.summary}${t.meta ? ` · ${t.meta}` : ''}`}
                </span>
              </div>
            ))}
          </Activity>
        ),
      )}
    </Thread>
  )
}

/** The end divider (05 §4.2): Claude Code's `SessionEnd` reason as is; Mode's inferred ends in words. */
export function endText(reason: string): string {
  switch (reason) {
    case 'restart':
      return 'ended · Mode was not running'
    case 'reboot':
      return 'ended · the machine restarted'
    default:
      return `ended · ${reason}`
  }
}
