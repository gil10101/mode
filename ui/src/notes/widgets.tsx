// Local building blocks of the Notes view that the kit does not have yet
// (kit candidates, listed in the package report): a fixed-position menu that
// escapes clipped panels, menu rows that always carry `.m-label` (glyph /
// hint / reason / current), the note task row with its glyph cycle, hover
// control, tags and drag source, and the inline notice.
import { useEffect, useLayoutEffect, useRef, useState, type DragEvent, type KeyboardEvent, type ReactNode } from 'react'
import './notes.css'
import { Button, MenuButton } from '../kit/button'
import { cx } from '../kit/cx'
import type { NoteListRow, StatusDef } from '../state/contract-notes'
import type { Notice } from '../state/notes-store'
import { whenLabel } from './dates'
import { isDone, rowGlyph, statusLabel } from './rules'

/* ---- menus ------------------------------------------------------------------ */

export interface FloatMenuProps {
  /** The element the menu hangs from (below it, flipped above when there is no room). */
  anchor: HTMLElement
  onClose: () => void
  label: string
  align?: 'left' | 'right'
  children: ReactNode
  testId?: string
  className?: string
}

export function FloatMenu({ anchor, onClose, label, align = 'left', children, testId, className }: FloatMenuProps) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null)
  const close = useRef(onClose)
  close.current = onClose

  useLayoutEffect(() => {
    const a = anchor.getBoundingClientRect()
    const el = ref.current
    const w = el?.offsetWidth ?? 200
    const h = el?.offsetHeight ?? 0
    let top = a.bottom + 4
    if (top + h > window.innerHeight - 8 && a.top - h - 4 > 8) top = a.top - h - 4
    let left = align === 'right' ? a.right - w : a.left
    left = Math.max(8, Math.min(left, window.innerWidth - w - 8))
    setPos({ top, left })
  }, [anchor, align])

  useEffect(() => {
    const down = (e: MouseEvent) => {
      const t = e.target as Node
      if (ref.current?.contains(t) || anchor.contains(t)) return
      close.current()
    }
    const key = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') close.current()
    }
    document.addEventListener('mousedown', down)
    document.addEventListener('keydown', key)
    return () => {
      document.removeEventListener('mousedown', down)
      document.removeEventListener('keydown', key)
    }
  }, [anchor])

  return (
    <div
      ref={ref}
      className={cx('m-menu nt-menu nt-float', className)}
      role="menu"
      aria-label={label}
      data-testid={testId}
      style={pos ? { top: pos.top, left: pos.left } : { visibility: 'hidden' }}
    >
      {children}
    </div>
  )
}

export interface ItemProps {
  label: ReactNode
  /** `''` keeps an empty glyph column (the "Search for" row). */
  glyph?: ReactNode
  hint?: ReactNode
  reason?: ReactNode
  /** Keyboard highlight (`--m-hover`). */
  active?: boolean
  /** The current choice (`--m-selected`). */
  current?: boolean
  dimmed?: boolean
  danger?: boolean
  trailing?: ReactNode
  onSelect?: () => void
  testId?: string
}

/** A menu row as the query-cards wireframe draws it: `[glyph] [label (+ reason)] [hint]`. */
export function Item({ label, glyph, hint, reason, active, current, dimmed, danger, trailing, onSelect, testId }: ItemProps) {
  return (
    <div
      className={cx('m-menu-item', active && 'is-active', current && 'is-current', dimmed && 'is-dimmed', danger && 'is-danger')}
      role="menuitem"
      tabIndex={dimmed ? -1 : 0}
      aria-disabled={dimmed || undefined}
      aria-current={current || undefined}
      data-testid={testId}
      onMouseDown={(e) => e.preventDefault()}
      onClick={dimmed ? undefined : onSelect}
      onKeyDown={(e) => {
        if (!dimmed && (e.key === 'Enter' || e.key === ' ')) {
          e.preventDefault()
          onSelect?.()
        }
      }}
    >
      {glyph !== undefined && <span className="m-glyph">{glyph}</span>}
      <span className="m-label">
        {label}
        {reason !== undefined && <span className="m-reason">{reason}</span>}
      </span>
      {hint !== undefined && <span className="m-hint">{hint}</span>}
      {trailing}
    </div>
  )
}

export function Sep() {
  return <div className="m-menu-sep" role="separator" />
}

/** One menu, several pages (Sort ›, Group by ›, When ›): `page` picks the content. */
export function useMenu<P extends string>() {
  const [state, setState] = useState<{ anchor: HTMLElement; page: P } | null>(null)
  return {
    open: state !== null,
    anchor: state?.anchor ?? null,
    page: state?.page ?? null,
    show: (anchor: HTMLElement, page: P) => setState({ anchor, page }),
    go: (page: P) => setState((s) => (s ? { ...s, page } : s)),
    close: () => setState(null),
  }
}

/* ---- the note task row ------------------------------------------------------- */

/** The note `dataTransfer` type for drags onto folders and board columns. */
export const NOTE_DRAG = 'application/x-mode-note'

/** The row being dragged (dragover cannot read `dataTransfer`). */
export const dragState: { row: NoteListRow | null } = { row: null }

export interface NoteTaskProps {
  row: NoteListRow
  statuses: readonly StatusDef[]
  today: string
  selected?: boolean
  /**
   * `list` and `board`: glyph, title, the when label or else the task progress (app-window.html
   * #n-list, #n-board); `card`: when, the first two tags and the progress (04 §8.5).
   */
  variant?: 'list' | 'board' | 'card'
  /** A card's own chips are not repeated on its rows (query-cards assumption 5). */
  hideWhen?: boolean
  hideTags?: readonly string[]
  onOpen?: () => void
  onCycle?: () => void
  onToggleUrgent?: () => void
  onMenu?: (anchor: HTMLElement) => void
  onTag?: (tag: string) => void
  draggable?: boolean
  onKeyDown?: (e: KeyboardEvent<HTMLDivElement>) => void
  testId?: string
}

export function NoteTask({ row, statuses, today, selected, variant = 'list', hideWhen, hideTags, onOpen, onCycle, onToggleUrgent, onMenu, onTag, draggable, onKeyDown, testId }: NoteTaskProps) {
  const [hover, setHover] = useState(false)
  const when = hideWhen ? null : whenLabel(row.when_bucket, row.when_date, today)
  const shownTags = variant === 'card' ? row.tags.filter((t) => !hideTags?.includes(t.toLowerCase())).slice(0, 2) : []
  const progress = row.task_total > 0 && (variant === 'card' || !when) ? `${row.task_done}/${row.task_total}` : null
  const glyph = rowGlyph(row, statuses)
  const status = `${statusLabel(row.status, statuses)}${row.urgent ? ', urgent' : ''}`
  return (
    <div
      className={cx('m-task', isDone(row) && 'is-done', selected && 'is-selected')}
      role="option"
      aria-selected={!!selected}
      tabIndex={-1}
      title={row.excerpt ? `${row.title} · ${row.excerpt}` : row.title}
      data-testid={testId}
      data-note={row.id}
      draggable={draggable}
      onDragStart={(e: DragEvent<HTMLDivElement>) => {
        dragState.row = row
        e.dataTransfer.setData(NOTE_DRAG, row.id)
        e.dataTransfer.setData('text/plain', row.title)
        e.dataTransfer.effectAllowed = 'move'
      }}
      onDragEnd={() => {
        dragState.row = null
      }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onClick={onOpen}
      onKeyDown={onKeyDown}
    >
      <span
        className="m-task-glyph nt-cycle"
        role="button"
        aria-label={`${status}, change status`}
        onClick={(e) => {
          e.stopPropagation()
          onCycle?.()
        }}
      >
        {glyph}
      </span>
      <span className="m-task-title">{row.title}</span>
      {row.pinned && (
        <span className="m-t-meta" aria-label="Pinned">
          ⌖
        </span>
      )}
      {when && <span className="m-t-meta">{when}</span>}
      {shownTags.map((t) => (
        <span
          key={t}
          className="m-tag"
          role="button"
          onClick={(e) => {
            e.stopPropagation()
            onTag?.(t)
          }}
        >
          {`#${t}`}
        </span>
      ))}
      {progress && <span className="m-t-meta">{progress}</span>}
      {hover && (onToggleUrgent || onMenu) && (
        <span className="nt-row-ctl">
          {onToggleUrgent && (
            <Button
              variant="icon"
              quiet
              aria-label={row.urgent ? 'Clear urgent' : 'Mark urgent'}
              aria-pressed={row.urgent}
              onClick={(e) => {
                e.stopPropagation()
                onToggleUrgent()
              }}
            >
              ✱
            </Button>
          )}
          {onMenu && (
            <MenuButton
              onClick={(e) => {
                e.stopPropagation()
                onMenu(e.currentTarget)
              }}
            />
          )}
        </span>
      )}
    </div>
  )
}

/* ---- inline notice ------------------------------------------------------------- */

/** Bottom-left, 8 s, one action, announced (10 §12.3). */
export function NoticeView({ notice, onDismiss }: { notice: Notice | null; onDismiss: () => void }) {
  if (!notice) return null
  return (
    <div className="nt-notice" role="status" data-testid="notes-notice">
      <span className="m-ellipsis">{notice.text}</span>
      {notice.action && (
        <Button
          onClick={() => {
            notice.action!.run()
            onDismiss()
          }}
        >
          {notice.action.label}
        </Button>
      )}
    </div>
  )
}
