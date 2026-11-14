import { useState, type CSSProperties, type ReactNode } from 'react'
import { Button } from './button'
import { cx } from './cx'
import { Icon } from './icons'
import { runs } from './text'

interface BoxProps {
  children?: ReactNode
  className?: string
  style?: CSSProperties
}

/** One column, max 720 px. */
export function Chat({ children, className, style }: BoxProps) {
  return (
    <div className={cx('m-chat', className)} style={style}>
      {children}
    </div>
  )
}

export function ChatThread({ children, className, style }: BoxProps) {
  return (
    <div className={cx('m-chat-thread', className)} style={style}>
      {children}
    </div>
  )
}

/** Text longer than this clamps at eight lines with “Show more”. */
export const BUBBLE_CLAMP_CHARS = 600

export interface BubbleUserProps {
  children: ReactNode
  /** Overrides the length heuristic. */
  clamp?: boolean
  showMoreLabel?: ReactNode
  className?: string
}

export function BubbleUser({ children, clamp, showMoreLabel = 'Show more', className }: BubbleUserProps) {
  const [expanded, setExpanded] = useState(false)
  const auto = typeof children === 'string' && children.length > BUBBLE_CLAMP_CHARS
  const clamped = (clamp ?? auto) && !expanded
  return (
    <div className={cx('m-bubble-user', clamped && 'is-clamped', className)}>
      <div className="m-bubble-text">{children}</div>
      {clamped && (
        <span
          className="m-show-more"
          role="button"
          tabIndex={0}
          onClick={() => setExpanded(true)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              setExpanded(true)
            }
          }}
        >
          {showMoreLabel}
        </span>
      )}
    </div>
  )
}

export interface ReplyActionsProps {
  time?: ReactNode
  onCopy?: () => void
  onPin?: () => void
}

/** Copy · pin · relative time, nothing else. */
export function ReplyActions({ time, onCopy, onPin }: ReplyActionsProps) {
  return (
    <div className="m-reply-actions">
      <Button variant="icon" quiet aria-label="Copy" onClick={onCopy}>
        <Icon id="copy" />
      </Button>
      <Button variant="icon" quiet aria-label="Pin" onClick={onPin}>
        <Icon id="pin" />
      </Button>
      {time !== undefined && <span className="m-reply-time">{time}</span>}
    </div>
  )
}

export interface ReplyProps extends ReplyActionsProps {
  children: ReactNode
  /** Drops the footer (a reply still streaming). */
  actions?: boolean
  className?: string
}

/** Plain text on the canvas, 15/1.5, with the footer row. */
export function Reply({ children, time, onCopy, onPin, actions = true, className }: ReplyProps) {
  return (
    <div className={cx('m-reply', className)}>
      {children}
      {actions && <ReplyActions time={time} onCopy={onCopy} onPin={onPin} />}
    </div>
  )
}

export interface ActivityProps {
  /** `Ran 3 commands`, `Read 8 files`, `code-reviewer · 40 s`. */
  label: ReactNode
  open?: boolean
  defaultOpen?: boolean
  onToggle?: (open: boolean) => void
  /** The label shimmers while the group runs. */
  active?: boolean
  /** `4 background tasks completed`. */
  status?: ReactNode
  children?: ReactNode
  className?: string
}

/** One fg-2 row per tool group with a chevron; open = one row per action. */
export function Activity({ label, open, defaultOpen, onToggle, active, status, children, className }: ActivityProps) {
  const [own, setOwn] = useState(!!defaultOpen)
  const isOpen = open ?? own
  const toggle = () => (onToggle ? onToggle(!isOpen) : setOwn(!isOpen))
  const hasBody = (children !== undefined && children !== null && children !== false) || status !== undefined
  return (
    <div className={cx('m-activity', isOpen && 'is-open', active && 'is-active', className)}>
      <div
        className="m-activity-head"
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
        <span>{label}</span>
      </div>
      {hasBody && (
        <div className="m-activity-body">
          {children}
          {status !== undefined && <div className="m-activity-status">{status}</div>}
        </div>
      )}
    </div>
  )
}

export interface ActivityRowProps {
  /** 24-px file-type thumbnail text (`md`, `py`). */
  thumb?: ReactNode
  onClick?: () => void
  children: ReactNode
}

export function ActivityRow({ thumb, onClick, children }: ActivityRowProps) {
  return (
    <div className="m-activity-row" role={onClick ? 'button' : undefined} onClick={onClick}>
      {thumb !== undefined && (
        <span className="m-thumb" style={{ flex: 'none' }}>
          {thumb}
        </span>
      )}
      <span>{children}</span>
    </div>
  )
}

export function Thumb({ children, className }: BoxProps) {
  return <span className={cx('m-thumb', className)}>{children}</span>
}

/** Pending-reply skeleton: two or three sweeping lines. */
export function Shimmer({ lines = 3, className }: { lines?: 2 | 3; className?: string }) {
  return (
    <div className={cx('m-shimmer', className)} aria-hidden="true">
      {Array.from({ length: lines }, (_, i) => (
        <i key={i} />
      ))}
    </div>
  )
}

export interface ChatInputProps {
  value?: string
  placeholder?: string
  label?: string
  onChange?: (value: string) => void
  /** Enter (Shift+Enter newlines). */
  onSend?: (text: string) => void
  onPlus?: () => void
  /** `false` draws the field as static text (previews). */
  editable?: boolean
  className?: string
  style?: CSSProperties
}

/** Radius-24 sunken container: `+` left, the field, `↵` right (fg once there is text). */
export function ChatInput({
  value,
  placeholder = 'Ask about your notes, sessions and messages…',
  label = 'Message',
  onChange,
  onSend,
  onPlus,
  editable = true,
  className,
  style,
}: ChatInputProps) {
  const [text, setText] = useState('')
  const current = value ?? text
  const set = onChange ?? setText
  const hasText = current.length > 0
  function send() {
    const t = current.trim()
    if (!t) return
    onSend?.(t)
    set('')
  }
  return (
    <div className={cx('m-chat-input', hasText && 'has-text', className)} style={style}>
      <button type="button" className="m-plus" aria-label="Add" onClick={onPlus}>
        +
      </button>
      {editable ? (
        <input
          className="m-field"
          placeholder={placeholder}
          aria-label={label}
          value={current}
          onChange={(e) => set(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) {
              e.preventDefault()
              send()
            }
          }}
        />
      ) : (
        <div className={cx('m-field', !hasText && 'is-placeholder')}>{hasText ? current : placeholder}</div>
      )}
      <button type="button" className="m-send" aria-label="Send" onClick={send}>
        ↵
      </button>
    </div>
  )
}

export interface ChatMetaProps {
  /** Auto · Quick · Thorough · Local-only. */
  effort: ReactNode
  onEffort?: () => void
  /** The only place Chat names a model. */
  model: ReactNode
  /** A tiny spinner while a reply runs. */
  busy?: boolean
  className?: string
}

export function ChatMeta({ effort, onEffort, model, busy, className }: ChatMetaProps) {
  return (
    <div className={cx('m-chat-meta', className)}>
      <span className="m-effort" role="button" tabIndex={0} aria-haspopup="menu" onClick={onEffort}>
        {runs([effort, ' '])}
        <span className="m-caret">▾</span>
      </span>
      <span className="m-model">
        {runs([model, busy && ' '])}
        {busy && <span className="m-spinner" aria-label="Replying" />}
      </span>
    </div>
  )
}

export interface SlashCommand {
  name: string
  description: ReactNode
}

export interface SlashPopProps {
  commands: SlashCommand[]
  active?: string
  onSelect?: (name: string) => void
  className?: string
}

/** Slash-command popover: command + one-line description. */
export function SlashPop({ commands, active, onSelect, className }: SlashPopProps) {
  return (
    <div className={cx('m-slash-pop', className)} role="listbox">
      {commands.map((c) => (
        <div key={c.name} className={cx('m-slash-row', c.name === active && 'is-active')} role="option" aria-selected={c.name === active} onClick={() => onSelect?.(c.name)}>
          <code>{c.name}</code>
          <span>{c.description}</span>
        </div>
      ))}
    </div>
  )
}
