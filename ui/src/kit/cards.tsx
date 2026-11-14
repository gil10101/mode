import { useId, useState, type CSSProperties, type ReactNode } from 'react'
import { Button, MenuButton } from './button'
import { cx } from './cx'
import { Diff, StateCol } from './lists'
import { joinDot, runs, type Runs } from './text'
import type { StateKind } from './types'

interface BoxProps {
  children?: ReactNode
  className?: string
  style?: CSSProperties
}

/** Surface, 1-px line, radius 10, no shadow; never inside another bordered box. */
export function Card({ selected, onClick, children, className, style, testId }: BoxProps & { selected?: boolean; onClick?: () => void; testId?: string }) {
  return (
    <div className={cx('m-card', selected && 'is-selected', className)} style={style} onClick={onClick} data-testid={testId}>
      {children}
    </div>
  )
}

export interface CardHeadProps {
  avatar?: ReactNode
  title: ReactNode
  /** Plain count after the title (query cards). */
  count?: number
  /** Icon buttons at the right: ↗ · ⋮ · ×. */
  actions?: ReactNode
  className?: string
}

export function CardHead({ avatar, title, count, actions, className }: CardHeadProps) {
  return (
    <div className={cx('m-card-head', className)}>
      {avatar}
      <span className="m-card-title">{title}</span>
      {count !== undefined && <span className="m-n">{count}</span>}
      {actions && <span className="m-actions">{actions}</span>}
    </div>
  )
}

export function CardBody({ children, className, style }: BoxProps) {
  return (
    <div className={cx('m-card-body', className)} style={style}>
      {children}
    </div>
  )
}

export function CardActions({ children, className }: BoxProps) {
  return <div className={cx('m-card-actions', className)}>{children}</div>
}

export interface SessionHeadProps {
  /** 32-px artwork avatar. */
  avatar: ReactNode
  title: ReactNode
  /** Meta line parts, joined by ` · `: repo (or a mono path), branch, `external`, the model (its only mention), elapsed. */
  repo?: string
  path?: string
  branch?: string
  external?: boolean
  model: string
  elapsed: string
  /** Replaces the composed meta line. */
  meta?: ReactNode
  /** Replaces the default buttons (Resume + ⋮). */
  actions?: ReactNode
  primaryLabel?: ReactNode
  /** Accent on the primary button; off for Open on external sessions. */
  primary?: boolean
  onPrimary?: () => void
  onMore?: () => void
  /** The sub line: state, diff, files, usage. */
  state?: StateKind
  stateLabel?: ReactNode
  diff?: { add: number; del: number }
  files?: number
  usage?: ReactNode
  sub?: ReactNode
  className?: string
}

export function sessionMeta({ repo, path, branch, external, model, elapsed }: Pick<SessionHeadProps, 'repo' | 'path' | 'branch' | 'external' | 'model' | 'elapsed'>): Runs {
  const parts: ReactNode[] = []
  if (repo !== undefined) parts.push(repo)
  else if (path !== undefined) parts.push(<code key="path">{path}</code>)
  if (branch !== undefined) parts.push(<code key="branch">{branch}</code>)
  if (external) parts.push('external')
  parts.push(model, elapsed)
  return joinDot(...parts)
}

/** Session detail header: avatar, one-line title, one meta line, Resume + ⋮, then the sub line. */
export function SessionHead(props: SessionHeadProps) {
  const { avatar, title, meta, actions, primaryLabel = 'Resume', primary = true, onPrimary, onMore, state, stateLabel, diff, files, usage, sub, className } = props
  const hasSub = state !== undefined || diff !== undefined || files !== undefined || usage !== undefined || sub !== undefined
  return (
    <>
      <div className={cx('m-session-head', className)}>
        {avatar}
        <div className="m-ellipsis">
          <div className="m-session-title">{title}</div>
          <div className="m-meta">{meta ?? sessionMeta(props)}</div>
        </div>
        <div className="m-btn-group">
          {actions ?? (
            <>
              <Button variant={primary ? 'primary' : 'default'} onClick={onPrimary}>
                {primaryLabel}
              </Button>
              <MenuButton onClick={onMore} />
            </>
          )}
        </div>
      </div>
      {hasSub && (
        <div className="m-session-sub">
          {state !== undefined && <StateCol state={state}>{stateLabel}</StateCol>}
          {diff && <Diff add={diff.add} del={diff.del} />}
          {files !== undefined && <span className="m-t-meta">{`· ${files} files`}</span>}
          {usage !== undefined && <span className="m-usage">{usage}</span>}
          {sub}
        </div>
      )}
    </>
  )
}

/* ---- thread -------------------------------------------------------------- */

export function Thread({ children, className, style }: BoxProps) {
  return (
    <div className={cx('m-thread', className)} style={style}>
      {children}
    </div>
  )
}

/** The user's prompt: right-aligned, sunken. */
export function ThreadUser({ children, className }: BoxProps) {
  return <div className={cx('m-thread-user', className)}>{children}</div>
}

/** The assistant's turn: surface with a 1-px line. */
export function ThreadAssistant({ children, className }: BoxProps) {
  return <div className={cx('m-thread-assistant', className)}>{children}</div>
}

export interface ThreadToolProps {
  name: string
  arg?: ReactNode
  /** Right-aligned 11-px meta (`84 lines`). */
  meta?: ReactNode
  /** Or a diff in its place. */
  diff?: { add: number; del: number }
  open?: boolean
  defaultOpen?: boolean
  onToggle?: (open: boolean) => void
  /** The expanded body (mono, pre-wrap). */
  children?: ReactNode
  className?: string
}

/** One-line tool row; expanded, its body follows as a sibling (the stylesheet joins them). */
export function ThreadTool({ name, arg, meta, diff, open, defaultOpen, onToggle, children, className }: ThreadToolProps) {
  const [own, setOwn] = useState(!!defaultOpen)
  const isOpen = open ?? own
  const toggle = () => (onToggle ? onToggle(!isOpen) : setOwn(!isOpen))
  return (
    <>
      <div
        className={cx('m-thread-tool', isOpen && 'is-open', className)}
        role="button"
        aria-expanded={isOpen}
        tabIndex={0}
        onClick={toggle}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            toggle()
          }
        }}
      >
        <span className="m-tool-name">{name}</span>
        {arg !== undefined && <span className="m-tool-arg">{arg}</span>}
        {diff ? <Diff className="m-tool-meta" add={diff.add} del={diff.del} /> : meta !== undefined && <span className="m-tool-meta">{meta}</span>}
      </div>
      {isOpen && children !== undefined && <div className="m-thread-tool-body">{children}</div>}
    </>
  )
}

export interface PermissionCardProps {
  title?: ReactNode
  /** The tool name, right of the title. */
  tool: ReactNode
  command: ReactNode
  onAllow?: () => void
  onDeny?: () => void
  onAlways?: () => void
  className?: string
}

/** Tool + command line + Allow (primary) / Deny / Always. */
export function PermissionCard({ title = 'Permission', tool, command, onAllow, onDeny, onAlways, className }: PermissionCardProps) {
  return (
    <div className={cx('m-thread-permission', className)}>
      <div className="m-thread-head">
        {title}
        <span className="m-t-meta">{tool}</span>
      </div>
      <code className="m-cmd">{command}</code>
      <div className="m-btn-group">
        <Button variant="primary" onClick={onAllow}>
          Allow
        </Button>
        <Button onClick={onDeny}>Deny</Button>
        <Button onClick={onAlways}>Always</Button>
      </div>
    </div>
  )
}

export interface QuestionOption {
  id: string
  label: ReactNode
  checked?: boolean
}

export interface QuestionGroup {
  id: string
  legend?: ReactNode
  /** Checkboxes instead of radios. */
  multiple?: boolean
  options: QuestionOption[]
}

export interface QuestionCardProps {
  question: ReactNode
  groups: QuestionGroup[]
  onChange?: (groupId: string, optionId: string, checked: boolean) => void
  onSend?: () => void
  sendLabel?: ReactNode
  className?: string
}

/** Radio and checkbox lists + Send. */
export function QuestionCard({ question, groups, onChange, onSend, sendLabel = 'Send', className }: QuestionCardProps) {
  // Radio groups are document-wide by name; scope them so two cards never share one.
  const uid = useId()
  return (
    <div className={cx('m-thread-question', className)}>
      <div className="m-thread-head">{question}</div>
      {groups.map((g) => (
        <fieldset key={g.id}>
          {g.legend !== undefined && <legend>{g.legend}</legend>}
          {g.options.map((o) => (
            <label key={o.id} className="m-choice">
              <input type={g.multiple ? 'checkbox' : 'radio'} name={uid + g.id} checked={!!o.checked} onChange={(e) => onChange?.(g.id, o.id, e.target.checked)} />
              {runs([' ', o.label])}
            </label>
          ))}
        </fieldset>
      ))}
      <div className="m-card-actions">
        <Button variant="primary" onClick={onSend}>
          {sendLabel}
        </Button>
      </div>
    </div>
  )
}

export interface ComposerProps {
  placeholder?: string
  label?: string
  value?: string
  onChange?: (value: string) => void
  onSend?: (text: string) => void
  sendLabel?: ReactNode
  className?: string
}

/** `.m-input` + Send primary, no helper text. */
export function Composer({ placeholder, label = 'Reply', value, onChange, onSend, sendLabel = 'Send', className }: ComposerProps) {
  const [text, setText] = useState('')
  const current = value ?? text
  const set = onChange ?? setText
  function send() {
    const t = current.trim()
    if (!t) return
    onSend?.(t)
    set('')
  }
  return (
    <div className={cx('m-composer', className)}>
      <input
        className="m-input is-lg"
        placeholder={placeholder}
        aria-label={label}
        value={current}
        onChange={(e) => set(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            send()
          }
        }}
      />
      <Button variant="primary" onClick={send}>
        {sendLabel}
      </Button>
    </div>
  )
}
