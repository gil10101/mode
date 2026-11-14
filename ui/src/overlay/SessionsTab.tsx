// The Sessions tab (02 §7.3–§7.5, §10, §14): rows grouped by state with the
// group labels, the approval and question cards under their waiting row, the
// fold row, the overflow row, the hook-health banner and the empty states.
import { useId, useState, type ReactNode } from 'react'
import { AgentAvatar } from '../kit/avatars'
import { Button } from '../kit/button'
import { Diff, GroupLabel, Row } from '../kit/lists'
import { joinDot } from '../kit/text'
import type { RosterRow } from '../state/contract'
import { useOverlayStore, type Decided } from '../state/overlay-store'
import { menuFor } from './actions'
import { liveSeconds, useOverlay } from './context'
import { groupLabel, layoutSessions, listBudget, type SessionItem } from './layout'
import { agentsSuffix, avatarHue, elapsed, pendingPreview, placeLabel, rowState, stateDetail } from './phrases'

export function SessionsTab() {
  const { placement, act } = useOverlay()
  const rows = useOverlayStore((s) => s.rows)
  const health = useOverlayStore((s) => s.health)
  const settings = useOverlayStore((s) => s.settings)
  const foldOpen = useOverlayStore((s) => s.foldOpen)
  const setFoldOpen = useOverlayStore((s) => s.setFoldOpen)
  const workArea = useOverlayStore((s) => s.display.work_area)
  const hooks = health?.hooks ?? 'none'
  const banner = hooks === 'stale' ? 'Hooks need updating' : hooks === 'missing' ? 'Hooks were removed' : hooks === 'conflict' ? 'Another hook is registered for Claude Code' : null
  const layout = layoutSessions(rows, { maxRows: settings.max_rows, maxHeight: listBudget(placement, workArea?.height), foldOpen })

  return (
    <div className="m-list" role="list" aria-label="Sessions" data-testid="sessions-list">
      {banner && (
        <div className="ov-line" role="status" data-testid="hooks-banner">
          <span className="m-dot ov-health" />
          {banner}
          <Button onClick={() => act('fix_hooks')}>Fix</Button>
        </div>
      )}
      {rows.length === 0 ? (
        <EmptySessions installed={hooks === 'installed'} />
      ) : (
        layout.items.map((item, i) => <Item key={i} item={item} onFold={() => setFoldOpen(!foldOpen)} />)
      )}
    </div>
  )
}

function EmptySessions({ installed }: { installed: boolean }) {
  const { act } = useOverlay()
  if (installed) {
    return (
      <div className="ov-empty" data-testid="empty-sessions">
        No sessions running. Start <code>claude</code> in any terminal, or launch one from the app.
        <br />
        <Button onClick={() => act('launch')}>Launch</Button>
      </div>
    )
  }
  return (
    <div className="ov-empty" data-testid="empty-sessions">
      No sessions yet. Install the hook relay to see Claude Code sessions here.
      <br />
      <Button onClick={() => act('install_hooks')}>Install hooks</Button>
    </div>
  )
}

function Item({ item, onFold }: { item: SessionItem; onFold: () => void }) {
  const { act } = useOverlay()
  switch (item.kind) {
    case 'label':
      return <GroupLabel>{groupLabel(item.state)}</GroupLabel>
    case 'row':
      return <SessionRowView row={item.row} card={item.card} />
    case 'fold':
      return (
        <div className="ov-line ov-action" role="button" aria-expanded={item.open} onClick={onFold} data-testid="fold-row">
          <span className="ov-chev">{item.open ? '▾' : '▸'}</span>
          {`Recently completed · ${item.count}`}
        </div>
      )
    case 'overflow':
      return (
        <div className="ov-line ov-action" role="button" onClick={() => act('open_tab', undefined, { tab: 'sessions' })} data-testid="overflow-row">
          {`+${item.count} more · open in app`}
        </div>
      )
  }
}

export function detailLine(row: RosterRow): ReactNode {
  const place = placeLabel(row)
  const detail = stateDetail(row)
  const parts: ReactNode[] = [place.mono ? <code>{place.text}</code> : place.text]
  if (row.git_branch) parts.push(<code>{row.git_branch}</code>)
  if (detail.diff) {
    const { add, del, files } = detail.diff
    if (add === 0 && del === 0) parts.push('no changes')
    else parts.push(<Diff key="diff" add={add} del={del} />, `${files} ${files === 1 ? 'file' : 'files'}`)
  } else if (detail.text) parts.push(detail.text)
  const agents = agentsSuffix(row.live_agents)
  if (agents) parts.push(agents)
  return joinDot(...parts)
}

function SessionRowView({ row, card }: { row: RosterRow; card: boolean }) {
  const ctx = useOverlay()
  const { m, act, runner, openMenu } = ctx
  const tooltip = [row.title, row.cwd, row.git_branch].filter(Boolean).join(' · ')
  return (
    <>
      <Row
        dense
        state={rowState(row.state)}
        unread={row.state === 'unread'}
        selected={m.focusRow === row.id}
        avatar={<AgentAvatar hue={avatarHue(row.avatar_color)} variant={row.avatar_variant} untracked={row.state === 'untracked'} />}
        name={row.display_name}
        title={tooltip}
        detail={detailLine(row)}
        meta={elapsed(liveSeconds(row, ctx))}
        menuLabel={`More for ${row.display_name}`}
        onMenu={(e) => openMenu(e.currentTarget, menuFor(row), (id) => act(id as never, row))}
        onClick={() => {
          runner.dispatch({ type: 'focus_row', row: row.id })
          act('primary', row)
        }}
        testId={`row-${row.id}`}
      />
      {card && row.pending?.kind === 'permission' && <PermissionCardView row={row} />}
      {card && row.pending?.kind === 'question' && <QuestionCardView row={row} />}
    </>
  )
}

export function decidedText(d: Decided): string {
  switch (d.outcome) {
    case 'pending':
      return ''
    case 'accepted':
      return `${d.kind === 'allow' ? 'allowed' : d.kind === 'deny' ? 'denied' : d.kind === 'always' ? 'always allowed' : 'answered'} · overlay`
    case 'already_answered':
      return 'already answered'
    case 'answered_in_terminal':
      return 'answered in terminal'
    default:
      return "couldn't do that"
  }
}

function DecidedLine({ d }: { d: Decided | undefined }) {
  if (!d || d.outcome === 'pending') return null
  return (
    <div className="ov-decided" role="status" data-testid="decided">
      {decidedText(d)}
      {d.kind === 'always' && d.outcome === 'accepted' && <span className="m-chip ov-rule">rule</span>}
    </div>
  )
}

function PermissionCardView({ row }: { row: RosterRow }) {
  const { act, decidedFor } = useOverlay()
  const p = row.pending!
  const d = decidedFor(row.id)
  const busy = !!d
  return (
    <div className="m-thread-permission" role="group" aria-label={`Approval: ${p.summary}`} data-testid={`card-${row.id}`}>
      <code className="m-cmd" title={p.summary}>
        {pendingPreview(p)}
      </code>
      <div className="m-btn-group">
        <Button variant="primary" disabled={busy} onClick={() => act('allow', row)}>
          Allow
        </Button>
        <Button disabled={busy} onClick={() => act('deny', row)}>
          Deny
        </Button>
        <Button disabled={busy} onClick={() => act('always', row)}>
          Always
        </Button>
      </div>
      <DecidedLine d={d} />
    </div>
  )
}

function QuestionCardView({ row }: { row: RosterRow }) {
  const { act, decidedFor } = useOverlay()
  const p = row.pending!
  const qs = p.questions?.length ? p.questions : [{ question: p.summary, options: [] }]
  const [page, setPage] = useState(0)
  const [answers, setAnswers] = useState<Record<number, string[]>>({})
  const uid = useId()
  const d = decidedFor(row.id)
  const busy = !!d
  const i = Math.min(page, qs.length - 1)
  const q = qs[i]
  const chosen = answers[i] ?? []
  const allAnswered = qs.every((_, k) => (answers[k] ?? []).length > 0)

  const send = (final: Record<number, string[]>) => act('answer', row, { answers: qs.map((qq, k) => ({ question: qq.question, answer: (final[k] ?? []).join(', ') })) })
  const choose = (label: string) => {
    if (busy) return
    if (q.multiSelect) {
      setAnswers({ ...answers, [i]: chosen.includes(label) ? chosen.filter((l) => l !== label) : [...chosen, label] })
      return
    }
    const next = { ...answers, [i]: [label] }
    setAnswers(next)
    if (i < qs.length - 1) setPage(i + 1)
    else send(next)
  }

  return (
    <div className="m-thread-question" role="group" aria-label={`Question: ${q.question}`} data-testid={`card-${row.id}`}>
      <div className="m-thread-head">
        <span className="m-ellipsis" title={q.question}>
          {`${q.header ? `${q.header}: ` : ''}${q.question}`}
        </span>
        {qs.length > 1 && (
          <span className="ov-paging">
            <Button variant="icon" quiet aria-label="Previous question" disabled={i === 0} onClick={() => setPage(i - 1)}>
              ‹
            </Button>
            {`${i + 1} of ${qs.length}`}
            <Button variant="icon" quiet aria-label="Next question" disabled={i === qs.length - 1} onClick={() => setPage(i + 1)}>
              ›
            </Button>
          </span>
        )}
      </div>
      <fieldset>
        {q.options?.map((o) => (
          <label key={o.label} className="m-choice" title={o.description}>
            {/* design: the question card's native choice controls (10 §7.4.9); the kit has no checkbox or radio yet */}
            <input type={q.multiSelect ? 'checkbox' : 'radio'} name={uid + i} checked={chosen.includes(o.label)} disabled={busy} onChange={() => choose(o.label)} />
            {` ${o.label}`}
          </label>
        ))}
      </fieldset>
      <div className="m-btn-group">
        <Button variant="primary" disabled={busy || !allAnswered} onClick={() => send(answers)}>
          Send
        </Button>
        <Button disabled={busy} onClick={() => act('answer_in_app', row)}>
          Answer in app
        </Button>
      </div>
      <DecidedLine d={d} />
    </div>
  )
}
