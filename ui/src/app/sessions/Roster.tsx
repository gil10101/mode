// The roster (03 §3.1–§3.2): `.m-row` rows grouped by state with group
// labels, nested agent rows under their parent, the inline approval or
// question card of a waiting row, the "Recently completed (n)" fold and the
// history rows; or the same rows as cards in a responsive grid, or as cards
// in one column per state group (the Board layout).
import { Fragment, useEffect, type KeyboardEvent, type ReactNode } from 'react'
import { AgentAvatar } from '../../kit/avatars'
import { MenuButton } from '../../kit/button'
import { Card, CardHead } from '../../kit/cards'
import { Diff, GroupLabel, Row, StateCol } from '../../kit/lists'
import { joinDot } from '../../kit/text'
import { isLaunchPlaceholder, type AgentRow, type AppRosterRow } from '../../state/contract-sessions'
import { boardColumns, isExpanded, rosterItems, useSessionsStore, type RosterItem } from '../../state/sessions-store'
import { navigate } from '../router'
import { copyText, openMenu } from '../ui'
import { decide, jumpToTerminal, openSession, sessionMenu, useDialogs } from './actions'
import { PendingCard } from './cards'
import {
  agentsToggle,
  avatarHue,
  between,
  elapsed,
  elapsedWords,
  placeLabel,
  plural,
  rowState,
  secondsSince,
  stateDetail,
  statePhrase,
  tildePath,
} from './format'

/** Seconds in the current state, ticking from the push (`state_seconds` was exact when it arrived). */
export function liveSeconds(row: Pick<AppRosterRow, 'state_seconds'>, rowsAt: number, now: number): number {
  return row.state_seconds + Math.max(0, now - rowsAt) / 1000
}

export function agentsOf(row: AppRosterRow, details: Record<string, { agents: AgentRow[] }>): AgentRow[] | undefined {
  return row.agents ?? details[row.id]?.agents
}

/** "mode · feat/api · permission for pytest · ▾ 2 agents" (03 §3.2). */
export function detailLine(row: AppRosterRow, agents: { total: number; running: number; open: boolean; onToggle?: () => void } | null): ReactNode {
  const place = placeLabel(row)
  const detail = stateDetail(row)
  const parts: ReactNode[] = [place.mono ? <code>{place.text}</code> : place.text]
  if (row.git_branch) parts.push(<code>{row.git_branch}</code>)
  if (detail.diff) {
    const { add, del, files } = detail.diff
    // `+3 −1 · 1 file`: the diff and its file count are two parts of the line.
    if (add === 0 && del === 0) parts.push('no changes')
    else parts.push(<Diff key="diff" add={add} del={del} />, plural(files, 'file'))
  } else if (detail.text) parts.push(detail.text)
  if (agents && agents.total > 0) {
    const text = agentsToggle(agents.total, agents.running, agents.open)
    parts.push(
      <span
        className="ap-toggle"
        role="button"
        tabIndex={-1}
        aria-expanded={agents.open}
        onClick={(e) => {
          e.stopPropagation()
          agents.onToggle?.()
        }}
      >
        {text}
      </span>,
    )
  }
  return joinDot(...parts)
}

/** Screen-reader name of a session row (10 §11.4). */
export function rowLabel(row: AppRosterRow, seconds: number): string {
  const place = placeLabel(row)
  const parts = [`Session ${row.display_name}`, statePhrase(row.state), `repo ${place.text}${row.git_branch ? ` on ${row.git_branch}` : ''}`]
  if (row.current_tool) parts.push(`running ${row.current_tool}`)
  if (row.lines_added || row.lines_removed) parts.push(`plus ${row.lines_added} minus ${row.lines_removed}`)
  parts.push(elapsedWords(seconds))
  return parts.join(', ')
}

function rowTooltip(row: AppRosterRow): string {
  // The model is named once, in the session header (owner rule; 03 §3.2's tooltip list minus the model).
  return [row.title, isLaunchPlaceholder(row.claude_session_id) ? null : row.claude_session_id, tildePath(row.cwd), row.files_changed ? plural(row.files_changed, 'file') : null].filter(Boolean).join(' · ')
}

interface RosterProps {
  rows: AppRosterRow[]
  history: AppRosterRow[]
  now: number
}

export function Roster({ rows, history, now }: RosterProps) {
  const layout = useSessionsStore((s) => s.layout)
  const groupByRepo = useSessionsStore((s) => s.groupByRepo)
  const foldOpen = useSessionsStore((s) => s.foldOpen)
  const expanded = useSessionsStore((s) => s.expanded)
  const details = useSessionsStore((s) => s.details)
  // The Board's columns are the state groups, so "Group by repo" does not apply to it.
  const items = rosterItems(rows, { groupByRepo: groupByRepo && layout !== 'board', foldOpen, expanded, agentsOf: (id) => details[id]?.agents, now }, history)
  useNeededAgents(rows, expanded)
  if (layout === 'cards') return <RosterCards items={items} now={now} />
  if (layout === 'board') return <RosterBoard items={items} now={now} />
  // The kit's `.m-list` (as a plain element so it can carry the list role; labels stay direct children for the stylesheet).
  return (
    <div className="m-list" role="list" aria-label="Sessions" data-testid="roster">
      {items.map((item) => (
        <RosterItemView key={itemKey(item)} item={item} now={now} />
      ))}
    </div>
  )
}

function itemKey(item: RosterItem): string {
  switch (item.kind) {
    case 'label':
      return item.key
    case 'row':
      return `${item.history ? 'h' : 'r'}:${item.row.id}`
    case 'agent':
      return `a:${item.parent}:${item.agent.id}`
    case 'fold':
      return 'fold'
  }
}

/** Expanded parents whose push carried no agent rows get them from `session_detail`. */
function useNeededAgents(rows: AppRosterRow[], expanded: Record<string, boolean>) {
  const details = useSessionsStore((s) => s.details)
  const status = useSessionsStore((s) => s.detailStatus)
  const load = useSessionsStore((s) => s.loadDetail)
  useEffect(() => {
    for (const r of rows) {
      if (r.agents || r.live_agents <= 0 || !isExpanded(r, expanded)) continue
      if (!details[r.id] && !status[r.id]) void load(r.id)
    }
  }, [rows, expanded, details, status, load])
}

function RosterItemView({ item, now }: { item: RosterItem; now: number }) {
  switch (item.kind) {
    case 'label':
      return <GroupLabel>{item.text}</GroupLabel>
    case 'row':
      return <SessionRow row={item.row} history={item.history} now={now} />
    case 'agent':
      return <AgentNestRow agent={item.agent} parent={item.parent} now={now} />
    case 'fold':
      return <FoldRow count={item.count} open={item.open} />
  }
}

function FoldRow({ count, open }: { count: number; open: boolean }) {
  const setFoldOpen = useSessionsStore((s) => s.setFoldOpen)
  return (
    <div
      className="aw-fold"
      role="button"
      tabIndex={0}
      aria-expanded={open}
      aria-label={`Recently completed, ${plural(count, 'session')}, ${open ? 'expanded' : 'collapsed'}`}
      onClick={() => setFoldOpen(!open)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          setFoldOpen(!open)
        }
      }}
      data-testid="fold-row"
    >
      {`${open ? '▾' : '▸'} Recently completed (${count})`}
    </div>
  )
}

function useRowBits(row: AppRosterRow, now: number) {
  const rowsAt = useSessionsStore((s) => s.rowsAt)
  const expanded = useSessionsStore((s) => s.expanded)
  const details = useSessionsStore((s) => s.details)
  const toggle = useSessionsStore((s) => s.toggleExpanded)
  const agents = agentsOf(row, details)
  const total = agents ? agents.length : row.live_agents
  const running = agents ? agents.filter((a) => !a.ended_at).length : row.live_agents
  const open = isExpanded(row, expanded)
  const seconds = liveSeconds(row, rowsAt, now)
  return { seconds, agentInfo: total > 0 ? { total, running, open, onToggle: () => toggle(row.id) } : null }
}

/** Row keys of 03 §10.2 (printed only in the Shortcuts view). */
function rowKeys(row: AppRosterRow, history: boolean) {
  return (e: KeyboardEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement
    if (!(target.classList.contains('m-row') || target.classList.contains('m-card')) || e.metaKey || e.ctrlKey || e.altKey) return
    const s = useSessionsStore.getState()
    const key = e.key
    const handled = () => e.preventDefault()
    if (key === 'Enter') return void (handled(), openSession(row.id))
    if (key === 'ArrowDown' || key === 'ArrowUp') {
      handled()
      const all = Array.from(document.querySelectorAll<HTMLElement>('[data-roster-row]'))
      const i = all.indexOf(e.currentTarget)
      const next = all[key === 'ArrowDown' ? i + 1 : i - 1]
      if (next) {
        next.querySelector<HTMLElement>('.m-row, .m-card')?.focus()
        s.select(next.dataset.rosterRow ?? null)
      }
      return
    }
    if (key === 'ArrowRight' || key === 'ArrowLeft') return void (handled(), s.toggleExpanded(row.id, key === 'ArrowRight'))
    const lower = key.toLowerCase()
    if (lower === 'e' && row.state === 'unread') return void (handled(), s.acknowledge(row.id))
    if (lower === 'x') return void (handled(), s.dismiss(row.id))
    if (lower === 'j' && !history) return void (handled(), jumpToTerminal(row))
    if (lower === 'r' && (history || row.ended_at)) return void (handled(), useDialogs.getState().openResume(row.id))
    if (lower === 'n') return void (handled(), useDialogs.getState().openNote(row.id))
    if (row.state === 'waiting' && row.pending?.answerable && row.pending.kind === 'permission') {
      if (key === 'A') return void (handled(), decide(row.id, 'always', row.pending))
      if (key === 'a') return void (handled(), decide(row.id, 'allow', row.pending))
      if (key === 'd') return void (handled(), decide(row.id, 'deny', row.pending))
    }
  }
}

export function SessionRow({ row, history, now }: { row: AppRosterRow; history: boolean; now: number }) {
  const selectedId = useSessionsStore((s) => s.selectedId)
  const select = useSessionsStore((s) => s.select)
  const { seconds, agentInfo } = useRowBits(row, now)
  const selected = selectedId === row.id
  const showCard = !history && row.state === 'waiting' && !!row.pending && (row.pending.kind === 'permission' || row.pending.kind === 'question')
  return (
    <div className="ap-row-wrap" role="listitem" aria-label={rowLabel(row, seconds)} data-roster-row={row.id} onKeyDown={rowKeys(row, history)}>
      <Row
        state={rowState(row.state)}
        unread={row.state === 'unread'}
        selected={selected}
        className={history ? 'is-history' : undefined}
        avatar={<AgentAvatar hue={avatarHue(row.avatar_color)} variant={row.avatar_variant} untracked={row.state === 'untracked'} />}
        name={row.display_name}
        title={rowTooltip(row)}
        detail={detailLine(row, history ? null : agentInfo)}
        meta={elapsed(seconds)}
        menuLabel={`More for ${row.display_name}`}
        onMenu={(e) => openMenu(e.currentTarget, sessionMenu(row, { history }), `Actions for ${row.display_name}`)}
        onClick={() => select(row.id)}
        testId={`row-${row.id}`}
      />
      {showCard && <PendingCard row={row} pending={row.pending!} />}
    </div>
  )
}

/** A subagent nested under its parent (03 §3.2.1): no avatar (16-px bot-head at 60 %), the type as the name. */
function AgentNestRow({ agent, parent, now }: { agent: AgentRow; parent: string; now: number }) {
  const seconds = agent.ended_at ? between(agent.started_at, agent.ended_at, now) : secondsSince(agent.started_at, now)
  const detail = agent.current_tool ? (
    <>
      <code>{agent.current_tool}</code>
      {agent.current_tool_arg ? ` ${agent.current_tool_arg}` : ''}
    </>
  ) : (
    (agent.last_assistant_preview ?? '')
  )
  const type = agent.agent_type ?? 'agent'
  return (
    <div role="listitem" aria-label={`Agent ${type}, ${statePhrase(agent.state)}, ${elapsedWords(seconds)}${agent.current_tool ? `, ${agent.current_tool} ${agent.current_tool_arg ?? ''}` : ''}`}>
      <Row
        agent
        state={rowState(agent.state)}
        avatar={<AgentAvatar />}
        name={type}
        detail={detail}
        meta={elapsed(seconds)}
        menuLabel={`More for ${type}`}
        onMenu={(e) =>
          openMenu(e.currentTarget, [
            { id: 'thread', label: 'Open agent thread', run: () => openAgent(parent, agent) },
            { id: 'copy', label: 'Copy agent id', run: () => copyText(agent.agent_id ?? agent.id, 'Agent id copied') },
          ])
        }
        onClick={() => openAgent(parent, agent)}
        testId={`agent-${agent.id}`}
      />
    </div>
  )
}

function openAgent(parent: string, agent: AgentRow) {
  navigate({ view: 'sessions', id: parent, tab: 'agents', sub: agent.agent_id ?? agent.id })
}

/* ---- cards layout ---------------------------------------------------------- */

function RosterCards({ items, now }: { items: RosterItem[]; now: number }) {
  return (
    <div className="ap-cards" role="list" aria-label="Sessions">
      {items.map((item) => {
        if (item.kind === 'label')
          return (
            <GroupLabel key={item.key} className="ap-cards-label">
              {item.text}
            </GroupLabel>
          )
        if (item.kind === 'row') return <SessionCard key={itemKey(item)} row={item.row} history={item.history} now={now} />
        if (item.kind === 'fold')
          return (
            <Fragment key="fold">
              <FoldRow count={item.count} open={item.open} />
            </Fragment>
          )
        return null
      })}
    </div>
  )
}

/* ---- board layout ---------------------------------------------------------- */

/** One column per non-empty state group in roster order, the same cards inside (03 §3.1). */
function RosterBoard({ items, now }: { items: RosterItem[]; now: number }) {
  return (
    <div className="ap-board" role="group" aria-label="Sessions" data-testid="roster-board">
      {boardColumns(items).map((col) => (
        <div key={col.key} className="ap-board-col" role="list" aria-label={col.text} data-testid={`board-col-${col.key}`}>
          <GroupLabel className="ap-board-label">{col.text}</GroupLabel>
          {col.items.map((item) =>
            item.kind === 'row' ? (
              <SessionCard key={itemKey(item)} row={item.row} history={item.history} now={now} />
            ) : item.kind === 'fold' ? (
              <FoldRow key="fold" count={item.count} open={item.open} />
            ) : null,
          )}
        </div>
      ))}
    </div>
  )
}

function SessionCard({ row, history, now }: { row: AppRosterRow; history: boolean; now: number }) {
  const selectedId = useSessionsStore((s) => s.selectedId)
  const select = useSessionsStore((s) => s.select)
  const { seconds, agentInfo } = useRowBits(row, now)
  const showCard = !history && row.state === 'waiting' && !!row.pending
  return (
    <div role="listitem" aria-label={rowLabel(row, seconds)} data-roster-row={row.id} onKeyDown={rowKeys(row, history)}>
      <Card className="ap-card" selected={selectedId === row.id} onClick={() => select(row.id)} testId={`card-${row.id}`}>
        <CardHead
          avatar={<AgentAvatar size={32} hue={avatarHue(row.avatar_color)} variant={row.avatar_variant} untracked={row.state === 'untracked'} />}
          title={<span className={row.state === 'unread' ? 'ap-unread' : undefined}>{row.display_name}</span>}
          actions={
            <MenuButton
              label={`More for ${row.display_name}`}
              onClick={(e) => {
                e.stopPropagation()
                openMenu(e.currentTarget, sessionMenu(row, { history }))
              }}
            />
          }
        />
        <div className="ap-card-line">
          <StateCol state={rowState(row.state)} />
          <span className="m-t-meta">{elapsed(seconds)}</span>
        </div>
        <div className="m-meta" title={rowTooltip(row)}>
          {detailLine(row, history ? null : agentInfo)}
        </div>
        {showCard && <PendingCard row={row} pending={row.pending!} />}
      </Card>
    </div>
  )
}
