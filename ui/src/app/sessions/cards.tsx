// The approval and question cards (03 §3.7, 05 §7): the kit's
// PermissionCard / QuestionCard while open and answerable, wired to
// `session_decide`; their answered, timed-out and terminal-only states.
import { useState, type ReactElement, type ReactNode } from 'react'
import { PermissionCard, QuestionCard } from '../../kit/cards'
import { runs } from '../../kit/text'
import type { Pending, Question } from '../../state/contract'
import type { AnsweredWhere, CardStatus, TranscriptItem } from '../../state/contract-sessions'
import { useSessionsStore, type Decided } from '../../state/sessions-store'
import { answer, decide, jumpToTerminal } from './actions'
import { clock, pendingPreview } from './format'

/**
 * The answered line of a card (03 §3.7): "allowed", "answered in terminal".
 * The core records `app` for anything Mode answered (overlay or app window
 * alike), so only the terminal is named; this window's own answers say
 * "app window" (`decidedText`).
 */
export function statusText(status: CardStatus, where?: AnsweredWhere | 'app window' | null): string {
  const w = where === 'terminal' ? ' in terminal' : where === 'app window' ? ' · app window' : ''
  switch (status) {
    case 'allowed':
      return `allowed${w}`
    case 'denied':
      return `denied${w}`
    case 'always':
      return `always allowed${w}`
    case 'answered':
      return `answered${w}`
    case 'answered_in_terminal':
      return 'answered in terminal'
    case 'timed_out':
      return 'timed out — answer in the terminal'
    case 'session_ended':
      return 'session ended'
    default:
      return ''
  }
}

/** The line a card shows right after the user answered in this window. */
export function decidedText(d: Decided): string {
  switch (d.outcome) {
    case 'pending':
      return d.kind === 'deny' ? 'denying…' : d.kind === 'answer' ? 'sending…' : 'allowing…'
    case 'accepted':
      return statusText(d.kind === 'allow' ? 'allowed' : d.kind === 'deny' ? 'denied' : d.kind === 'always' ? 'always' : 'answered', 'app window')
    case 'already_answered':
      return 'already answered'
    case 'answered_in_terminal':
      return 'answered in terminal'
    default:
      return "couldn't send the answer"
  }
}

interface RowRef {
  id: string
  state: string
  launch_mode: string
  cwd: string
  ended_at?: string | null
  dismissed_at?: string | null
}

function isTimedOut(p: Pending): boolean {
  return (p as Pending & { timed_out?: boolean }).timed_out === true
}

/** A permission card that is not (or no longer) answerable here. */
function PermissionStatic({ tool, command, line, grey, action }: { tool: ReactNode; command: string; line: ReactNode; grey?: boolean; action?: ReactElement }) {
  return (
    <div className={`m-thread-permission is-done${grey ? ' is-grey' : ''}`} role="group" aria-label={`Approval: ${command}`}>
      <div className="m-thread-head">
        Permission
        <span className="m-t-meta">{tool}</span>
      </div>
      <code className="m-cmd">{command}</code>
      <div className="ap-decided">
        {runs([line, action && ' · '])}
        {action}
      </div>
    </div>
  )
}

/** The open card of a waiting session, answerable from the roster row or the Overview thread. */
export function PendingCard({ row, pending }: { row: RowRef; pending: Pending }) {
  const decided = useSessionsStore((s) => s.decided[row.id])
  const tool = `${pending.tool_name ?? 'Tool'} · ${clock(pending.received_at)}`
  const jump = (
    <a
      href="#"
      onClick={(e) => {
        e.preventDefault()
        jumpToTerminal(row)
      }}
    >
      Jump to terminal
    </a>
  )
  if (pending.kind === 'question') return <PendingQuestion row={row} pending={pending} />
  if (pending.kind !== 'permission') return null
  const command = pendingPreview(pending)
  if (decided && (decided.received_at === null || decided.received_at === pending.received_at))
    return <PermissionStatic tool={tool} command={command} line={<span data-testid="decided">{decidedText(decided)}</span>} />
  if (isTimedOut(pending)) return <PermissionStatic grey tool={tool} command={command} line="timed out — answer in the terminal" action={jump} />
  if (!pending.answerable) return <PermissionStatic tool={tool} command={command} line="answer in the terminal" action={jump} />
  return (
    <PermissionCard
      tool={tool}
      command={command}
      onAllow={() => void decide(row.id, 'allow', pending)}
      onDeny={() => void decide(row.id, 'deny', pending)}
      onAlways={() => void decide(row.id, 'always', pending)}
    />
  )
}

function questionHead(text: string, meta: string): ReactNode {
  return (
    <>
      <span className="m-ellipsis">{text}</span>
      <span className="m-t-meta">{meta}</span>
    </>
  )
}

/** The open question card: all questions stacked, each after the first under its header; Send when every one has a choice (05 §7.3). */
function PendingQuestion({ row, pending }: { row: RowRef; pending: Pending }) {
  const decided = useSessionsStore((s) => s.decided[row.id])
  const questions = pending.questions ?? []
  const [picked, setPicked] = useState<Record<number, string[]>>({})
  if (!questions.length) return null
  const meta = clock(pending.received_at)
  if (decided && (decided.received_at === null || decided.received_at === pending.received_at)) {
    return <AnsweredQuestionView questions={questions} answers={answersFrom(questions, picked)} meta={decidedText(decided)} />
  }
  if (!pending.answerable || isTimedOut(pending)) {
    return (
      <AnsweredQuestionView
        grey={isTimedOut(pending)}
        questions={questions}
        answers={{}}
        meta={isTimedOut(pending) ? 'timed out — answer in the terminal' : 'answer in the terminal'}
      />
    )
  }
  const complete = questions.every((_, i) => (picked[i]?.length ?? 0) > 0)
  return (
    <QuestionCard
      question={questionHead(questions[0].question, meta)}
      groups={questions.map((q, i) => ({
        id: String(i),
        legend: i === 0 ? undefined : (q.header ?? q.question),
        multiple: !!q.multiSelect,
        options: (q.options ?? []).map((o) => ({ id: o.label, label: o.label, checked: picked[i]?.includes(o.label) })),
      }))}
      onChange={(g, o, checked) => {
        const i = Number(g)
        setPicked((p) => {
          const cur = p[i] ?? []
          const next = questions[i].multiSelect ? (checked ? [...cur.filter((x) => x !== o), o] : cur.filter((x) => x !== o)) : [o]
          return { ...p, [i]: next }
        })
      }}
      onSend={() => {
        if (!complete) return
        void answer(
          row.id,
          questions.map((q, i) => ({ question: q.question, answer: (picked[i] ?? []).join(', ') })),
          pending,
        )
      }}
    />
  )
}

function answersFrom(questions: Question[], picked: Record<number, string[]>): Record<string, string[]> {
  const out: Record<string, string[]> = {}
  questions.forEach((q, i) => (out[q.question] = picked[i] ?? []))
  return out
}

/** A question card after it was answered: chosen options filled, controls disabled, no Send (session-detail.html callout 7). */
export function AnsweredQuestionView({ questions, answers, meta, grey }: { questions: Question[]; answers: Record<string, string | string[]>; meta: string; grey?: boolean }) {
  return (
    <div className={`m-thread-question${grey ? ' is-grey' : ''}`} role="group" aria-label={`Question: ${questions[0]?.question ?? ''}`}>
      <div className="m-thread-head">{questionHead(questions[0]?.question ?? '', meta)}</div>
      {questions.map((q, i) => {
        const chosen = answers[q.question]
        const set = new Set(Array.isArray(chosen) ? chosen : chosen ? chosen.split(', ') : [])
        return (
          <fieldset key={i}>
            {i > 0 && <legend>{q.header ?? q.question}</legend>}
            {(q.options ?? []).map((o) => (
              <label key={o.label} className="m-choice">
                {/* design: the question card's native choice controls (10 §7.4.9); the kit has no checkbox or radio yet */}
                <input type={q.multiSelect ? 'checkbox' : 'radio'} name={`answered-${i}-${q.question}`} checked={set.has(o.label)} disabled readOnly />
                {` ${o.label}`}
              </label>
            ))}
          </fieldset>
        )
      })}
    </div>
  )
}

type PermissionItem = Extract<TranscriptItem, { kind: 'permission' }>
type QuestionItem = Extract<TranscriptItem, { kind: 'question' }>

/** A permission item of the thread that is no longer open (or whose card the roster already cleared). */
export function AnsweredPermissionView({ item, sessionId }: { item: PermissionItem; sessionId: string }) {
  const decided = useSessionsStore((s) => s.decided[sessionId])
  const tool = `${item.tool_name} · ${clock(item.at)}`
  const base = item.status === 'open' ? (decided ? decidedText(decided) : '') : statusText(item.status, item.answered_where)
  const text = base + (item.rule ? ` · rule: ${item.rule.replace(/^[^:]+:/, '')}` : '')
  return <PermissionStatic grey={item.status === 'timed_out'} tool={tool} command={item.command} line={text} />
}

export function QuestionItemView({ item }: { item: QuestionItem }) {
  const when = item.answered_at ? clock(item.answered_at) : ''
  const meta = item.status === 'answered' ? `answered ${when}`.trim() + (item.answered_where === 'terminal' ? ' in terminal' : '') : statusText(item.status, item.answered_where)
  return <AnsweredQuestionView grey={item.status === 'timed_out'} questions={item.questions} answers={item.answers ?? {}} meta={meta} />
}
