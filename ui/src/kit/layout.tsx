import { Fragment, useEffect, useRef, type CSSProperties, type ReactNode } from 'react'
import './panel.css'
import { Mascot } from './avatars'
import { cx } from './cx'
import { Icon, type IconId } from './icons'

interface BoxProps {
  children?: ReactNode
  className?: string
  style?: CSSProperties
}

/** The app window chrome: sidebar left, main right. */
/** `inert`: the window stays drawn but takes no input or focus (behind the onboarding layer, 03 §2.8). */
export function Window({ children, className, style, testId, inert }: BoxProps & { testId?: string; inert?: boolean }) {
  return (
    <div className={cx('m-window', className)} style={style} data-testid={testId} inert={inert || undefined}>
      {children}
    </div>
  )
}

export interface SidebarItemSpec {
  id: string
  label: string
  icon: IconId
  count?: number
  /** The 6-px accent dot: only when something waits for the user. */
  waiting?: boolean
  /** A nested list under the entry, open while it is the active view (10 §7.4.7: the Chat conversations). */
  sub?: ReactNode
}

export interface SidebarProps {
  items: SidebarItemSpec[]
  /** After the 1-px divider (Stats, Settings). */
  bottom?: SidebarItemSpec[]
  active?: string
  onSelect?: (id: string) => void
  /** 56 px, icons only; counts hide, the waiting dot stays. */
  collapsed?: boolean
  /** The title-bar slot; defaults to the traffic lights. */
  top?: ReactNode
  /** The profile row under the bottom group. */
  profile?: ReactNode
  /** A row of the active entry's nested list is the selection (an open conversation): the entry itself is not drawn selected, its list stays open. */
  subActive?: boolean
  label?: string
  className?: string
}

export function Sidebar({ items, bottom, active, onSelect, collapsed, top, profile, subActive, label = 'Views', className }: SidebarProps) {
  const render = ({ sub, ...it }: SidebarItemSpec) => {
    const isActive = it.id === active
    const item = <SidebarItem {...it} active={isActive && !(subActive && sub !== undefined)} current={isActive} onClick={() => onSelect?.(it.id)} />
    if (sub === undefined) return <Fragment key={it.id}>{item}</Fragment>
    // The nested list opens with the view: its row animates from 0fr to 1fr (reduced motion: at once).
    return (
      <Fragment key={it.id}>
        {item}
        <div className={cx('m-sidebar-sub', isActive && 'is-open')} aria-hidden={!isActive}>
          <div className="m-sidebar-sub-in">{isActive ? sub : null}</div>
        </div>
      </Fragment>
    )
  }
  return (
    <nav className={cx('m-sidebar', collapsed && 'is-collapsed', className)} aria-label={label}>
      {/* The title-bar strip drags the window (Tauri); the dots hide in the real window, where the OS draws its own. */}
      <div className="m-sidebar-top" data-tauri-drag-region="">
        {top ?? <Traffic />}
      </div>
      {items.map(render)}
      {(bottom || profile) && (
        <div className="m-sidebar-bottom">
          {bottom?.map(render)}
          {profile}
        </div>
      )}
    </nav>
  )
}

/** The three neutral window dots. */
export function Traffic() {
  return (
    <span className="m-traffic">
      <i />
      <i />
      <i />
    </span>
  )
}

export interface SidebarItemProps extends Omit<SidebarItemSpec, 'sub'> {
  active?: boolean
  /** The entry is the current view (`aria-current`), drawn selected unless a row of its nested list is. */
  current?: boolean
  onClick?: () => void
}

export interface SidebarSubItemProps {
  active?: boolean
  onClick?: () => void
  /** The full text, for the tooltip of a truncated row. */
  title?: string
  /** The hover `⋮` (a `MenuButton`). */
  trailing?: ReactNode
  children: ReactNode
  testId?: string
}

/** A row of the nested list (10 §7.4.7): 28 px, indented to the entry's label, `⋮` on hover. */
export function SidebarSubItem({ active, onClick, title, trailing, children, testId }: SidebarSubItemProps) {
  return (
    <div
      className={cx('m-sidebar-subitem', active && 'is-active')}
      role="button"
      tabIndex={0}
      aria-current={active ? 'page' : undefined}
      title={title}
      data-testid={testId}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' && e.target === e.currentTarget) onClick?.()
      }}
    >
      <span className="m-label">{children}</span>
      {trailing}
    </div>
  )
}

/** The "Show n more" / "Show less" row of a nested list. */
export function SidebarMore({ less, onClick, children }: { less?: boolean; onClick?: () => void; children: ReactNode }) {
  return (
    <button type="button" className="m-sidebar-more" onClick={onClick}>
      <span className="m-sidebar-more-glyph" aria-hidden="true">
        {less ? '▴' : '▾'}
      </span>
      {children}
    </button>
  )
}

/** A quiet line under a nested list (an empty state, the retention note). */
export function SidebarSubNote({ children }: { children: ReactNode }) {
  return <div className="m-sidebar-subnote">{children}</div>
}

export function SidebarItem({ id, label, icon, count, waiting, active, current = active, onClick }: SidebarItemProps) {
  return (
    <button
      type="button"
      className={cx('m-sidebar-item', active && 'is-active')}
      data-view={id}
      aria-current={current ? 'page' : undefined}
      onClick={onClick}
    >
      <Icon id={icon} />
      <span className="m-label">{label}</span>
      {waiting && <span className="m-dot" />}
      {count !== undefined && <span className="m-count">{count}</span>}
    </button>
  )
}

export interface ProfileRowProps {
  name: ReactNode
  /** Second line: “1.2M tokens this week”. */
  detail: ReactNode
  /** Replaces the mascot. */
  src?: string
  alt?: string
  onClick?: () => void
  className?: string
  style?: CSSProperties
  testId?: string
}

/** Picture · name / tokens, at the bottom of the sidebar (§7.1). */
export function ProfileRow({ name, detail, src, alt = 'Profile picture', onClick, className, style, testId }: ProfileRowProps) {
  return (
    <div
      className={cx('m-profile', className)}
      style={style}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onClick={onClick}
      onKeyDown={
        onClick
          ? (e) => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault()
                onClick()
              }
            }
          : undefined
      }
      data-testid={testId}
    >
      <Mascot size={24} src={src} picture={Boolean(src)} alt={alt} />
      <div className="m-row-text">
        <div className="m-row-name">{name}</div>
        <div className="m-row-detail">{detail}</div>
      </div>
    </div>
  )
}

export function Main({ children, className, style }: BoxProps) {
  return (
    <main className={cx('m-main', className)} style={style}>
      {children}
    </main>
  )
}

export interface MainHeadProps extends BoxProps {
  title: ReactNode
  titleTestId?: string
}

/** 44-px head: the title, then whatever sits right (a segmented control, ⋮). */
export function MainHead({ title, titleTestId, children, className, style }: MainHeadProps) {
  return (
    <div className={cx('m-main-head', className)} style={style} data-tauri-drag-region="">
      <span className="m-t-title" data-testid={titleTestId}>
        {title}
      </span>
      {children}
    </div>
  )
}

export function MainBody({ children, className, style, testId }: BoxProps & { testId?: string }) {
  return (
    <div className={cx('m-main-body', className)} style={style} data-testid={testId}>
      {children}
    </div>
  )
}

/**
 * List on the left, a 440-px detail panel on the right. With `open` given the split stays mounted and its
 * panel column opens and closes by a transition (10 §3.7); without it the split is always open.
 */
export function Split({ children, className, style, open }: BoxProps & { open?: boolean }) {
  return (
    <div className={cx('m-split', open === undefined ? undefined : open ? 'is-open' : 'is-closed', className)} style={style}>
      {children}
    </div>
  )
}

/** The detail panel of the drill-in rule (§7.5): summary and one primary action. */
export function DetailPanel({ label, children, className, style, swapKey }: BoxProps & { label?: string; swapKey?: string }) {
  const ref = useRef<HTMLElement>(null)
  const first = useRef(true)
  // Another item in a panel already open: its content fades in (10 §3.7, content swap); the first mount slides (panel.css).
  useEffect(() => {
    if (first.current) {
      first.current = false
      return
    }
    const el = ref.current
    if (!el || typeof el.animate !== 'function') return
    const reduced = !!el.closest('[data-motion="reduced"]') || (typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches)
    if (!reduced) el.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 120, easing: 'ease-out' })
  }, [swapKey])
  return (
    <aside ref={ref} className={cx('m-panel', className)} style={style} aria-label={label}>
      {children}
    </aside>
  )
}
