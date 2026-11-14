import type { CSSProperties, KeyboardEvent, MouseEvent, ReactNode } from 'react'
import { MenuButton } from './button'
import { cx } from './cx'
import { stateLabel, type StateKind } from './types'

interface BoxProps {
  children?: ReactNode
  className?: string
  style?: CSSProperties
}

/** Surface container for rows; `divided` draws a 1-px line between children (message lists). */
export function List({ divided, children, className, style }: BoxProps & { divided?: boolean }) {
  return (
    <div className={cx('m-list', divided && 'is-divided', className)} style={style}>
      {children}
    </div>
  )
}

/** Tiny 11-px label between groups (`waiting`, `working`, …). */
export function GroupLabel({ children, className, style }: BoxProps) {
  return (
    <div className={cx('m-group-label', className)} style={style}>
      {children}
    </div>
  )
}

export interface StateColProps {
  state: StateKind
  /** Overrides the state word (a relative time on automation rows, `loaded` on plugin rows). */
  children?: ReactNode
  /** The fixed 88-px column (`.m-col-state`), where states line up outside a row's grid; a row's grid cell already is one. */
  column?: boolean
  className?: string
}

/** Glyph box + lowercase word, as wide as its word; `column` fixes it at 88 px. `working` spins through the stylesheet animation. */
export function StateCol({ state, children, column, className }: StateColProps) {
  return <span className={cx('m-state', `is-${state}`, column && 'm-col-state', className)}>{children ?? stateLabel(state)}</span>
}

export interface RowProps {
  state: StateKind
  stateLabel?: ReactNode
  /** The 24-px avatar slot; omit it on avatar-less rows (`plain`, connector rows). */
  avatar?: ReactNode
  /** A 16-px brand mark in the avatar's place (the Connectors tab, 10 §7.4.16). */
  mark?: ReactNode
  name: ReactNode
  detail?: ReactNode
  meta?: ReactNode
  /** `false` drops the ⋮ cell. */
  menu?: boolean
  menuLabel?: string
  /** Receives the click so a caller can anchor its menu to the ⋮. */
  onMenu?: (e: MouseEvent<HTMLButtonElement>) => void
  onClick?: () => void
  selected?: boolean
  unread?: boolean
  /** 32-px overlay row. */
  dense?: boolean
  /** Subagent row (§7.4): 24-px indent, 16-px bot-head at 60 %. */
  agent?: boolean
  /** No avatar column (run history). */
  plain?: boolean
  /** Native tooltip on the row. */
  title?: string
  className?: string
  style?: CSSProperties
  testId?: string
}

function activate(onClick: (() => void) | undefined) {
  if (!onClick) return undefined
  return (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onClick()
    }
  }
}

/** `[state 88] [avatar 24] [name / detail] [meta] [⋮]`, 40 px (32 dense). */
export function Row({
  state,
  stateLabel: label,
  avatar,
  mark,
  name,
  detail,
  meta,
  menu = true,
  menuLabel,
  onMenu,
  onClick,
  selected,
  unread,
  dense,
  agent,
  plain,
  title,
  className,
  style,
  testId,
}: RowProps) {
  return (
    <div
      className={cx('m-row', selected && 'is-selected', unread && 'is-unread', dense && 'is-dense', agent && 'is-agent', plain && 'is-plain', mark !== undefined && 'has-mark', className)}
      style={style}
      title={title}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      aria-selected={onClick ? !!selected : undefined}
      onClick={onClick}
      onKeyDown={activate(onClick)}
      data-testid={testId}
    >
      <StateCol state={state}>{label}</StateCol>
      {mark ?? avatar}
      <div className="m-row-text">
        <div className="m-row-name">{name}</div>
        {detail !== undefined && <div className="m-row-detail">{detail}</div>}
      </div>
      {meta !== undefined && <span className="m-row-meta">{meta}</span>}
      {menu && (
        <MenuButton
          label={menuLabel}
          onClick={(e) => {
            e.stopPropagation()
            onMenu?.(e)
          }}
        />
      )}
    </div>
  )
}

export interface MsgRowProps {
  avatar: ReactNode
  sender: ReactNode
  /** Slack channel with its `#`; omitted = the `Inbox` tag (Gmail). */
  channel?: string
  /** Draws the `@` glyph; only when the user is mentioned. */
  mentioned?: boolean
  time: ReactNode
  /** Subject (email) or the first text line, one line. */
  preview: ReactNode
  /** The 2-px accent bar on the left edge; nothing else marks urgency on a row. */
  urgent?: boolean
  unread?: boolean
  selected?: boolean
  onClick?: () => void
  className?: string
  style?: CSSProperties
  testId?: string
}

export function MsgRow({ avatar, sender, channel, mentioned, time, preview, urgent, unread, selected, onClick, className, style, testId }: MsgRowProps) {
  return (
    <div
      className={cx('m-msg', urgent && 'is-urgent', unread && 'is-unread', selected && 'is-selected', className)}
      style={style}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onClick={onClick}
      onKeyDown={activate(onClick)}
      data-testid={testId}
    >
      {avatar}
      <div className="m-msg-body">
        <div className="m-msg-l1">
          <span className="m-msg-name">{sender}</span>
          <span className="m-tag">{channel ?? 'Inbox'}</span>
          {mentioned && <span className="m-at">@</span>}
          <span className="m-msg-time">{time}</span>
        </div>
        <div className="m-msg-l2">{preview}</div>
      </div>
    </div>
  )
}

export interface DiffProps {
  add: number
  del: number
  className?: string
}

/** `+N −M`, mono 12, always together. */
export function Diff({ add, del, className }: DiffProps) {
  return (
    <span className={cx('m-diff', className)}>
      <span className="add">{`+${add}`}</span> <span className="del">{`−${del}`}</span>
    </span>
  )
}

export interface UsageProps {
  /** Formatted, e.g. `1.2M` or `137k`. */
  tokens: string
  turns: number
  /** Soft weekly budget, shown only with the bar. */
  budget?: string
  /** 0–100; given, the 4-px bar is drawn and the element is a block. */
  pct?: number
  className?: string
  style?: CSSProperties
}

/** Tokens and turns, never currency. */
export function Usage({ tokens, turns, budget, pct, className, style }: UsageProps) {
  const text = `${tokens} tokens · ${turns} turns`
  if (pct === undefined) {
    return (
      <span className={cx('m-usage', className)} style={style}>
        {text}
      </span>
    )
  }
  return (
    <div className={cx('m-usage', className)} style={style}>
      {text}
      {budget !== undefined && <span className="m-t-meta">{` · soft budget ${budget}`}</span>}
      <div className="m-usage-bar">
        <i style={{ '--pct': `${pct}%` } as CSSProperties} />
      </div>
    </div>
  )
}

export interface TaskProps {
  /** ○ todo · ◑ doing · ✱ urgent · ✓ done; file rows use ~ / + / −. */
  glyph?: ReactNode
  title: ReactNode
  meta?: ReactNode
  done?: boolean
  selected?: boolean
  /** Anything after the meta (a diff, a chevron). */
  trailing?: ReactNode
  onClick?: () => void
  className?: string
  testId?: string
}

/** 32-px task row of query cards and the overlay Notes tab. */
export function Task({ glyph, title, meta, done, selected, trailing, onClick, className, testId }: TaskProps) {
  return (
    <div
      className={cx('m-task', done && 'is-done', selected && 'is-selected', className)}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onClick={onClick}
      onKeyDown={activate(onClick)}
      data-testid={testId}
    >
      {glyph !== undefined && <span className="m-task-glyph">{glyph}</span>}
      <span className="m-task-title">{title}</span>
      {meta !== undefined && <span className="m-t-meta">{meta}</span>}
      {trailing}
    </div>
  )
}
