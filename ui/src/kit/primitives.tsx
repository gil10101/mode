import type { CSSProperties, ReactNode } from 'react'
import { cx } from './cx'

interface BoxProps {
  children?: ReactNode
  className?: string
  style?: CSSProperties
}

/** Neutral count pill. */
export function Count({ children, className, style }: BoxProps) {
  return (
    <span className={cx('m-count', className)} style={style}>
      {children}
    </span>
  )
}

/** The 6-px accent dot: only for `waiting`. */
export function Dot({ className }: { className?: string }) {
  return <span className={cx('m-dot', className)} />
}

/** Sunken pill: `#channel` or `Inbox` on a message row. */
export function Tag({ children, className }: BoxProps) {
  return <span className={cx('m-tag', className)}>{children}</span>
}

/** 11-px fg-3 text; `block` makes it a line of its own (section labels). */
export function Meta({ block, children, className, style }: BoxProps & { block?: boolean }) {
  const Tag = block ? 'div' : 'span'
  return (
    <Tag className={cx('m-t-meta', className)} style={style}>
      {children}
    </Tag>
  )
}

/** 11-px fg-3 one-line block with an ellipsis (card and header meta lines). */
export function MetaLine({ children, className, style }: BoxProps) {
  return (
    <div className={cx('m-meta', className)} style={style}>
      {children}
    </div>
  )
}

export function Ellipsis({ children, className, style }: BoxProps) {
  return (
    <span className={cx('m-ellipsis', className)} style={style}>
      {children}
    </span>
  )
}

export function Stack({ children, className, style }: BoxProps) {
  return (
    <div className={cx('m-stack', className)} style={style}>
      {children}
    </div>
  )
}

export function Inline({ children, className, style }: BoxProps) {
  return (
    <div className={cx('m-inline', className)} style={style}>
      {children}
    </div>
  )
}

export function Divider({ className }: { className?: string }) {
  return <hr className={cx('m-divider', className)} />
}

/** The braille spinner, as a standalone glyph. */
export function Spinner({ className }: { className?: string }) {
  return <span className={cx('m-spinner', className)} aria-hidden="true" />
}
