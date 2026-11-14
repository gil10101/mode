import type { CSSProperties, HTMLAttributes, ReactElement, ReactNode } from 'react'
import { Mascot } from './avatars'
import { MenuButton } from './button'
import { Tab, Tabs } from './controls'
import { cx } from './cx'
import { runs } from './text'
import { ThemeProvider } from './theme'
import { PLACEMENT_CLASS, type OverlayState, type Placement, type SessionState } from './types'

export interface OverlayTabSpec {
  id: string
  label: string
}

export interface OverlayHeadProps {
  badge?: SessionState
  /** One phrase: `waiting · api-refactor`. */
  phrase: ReactNode
  count?: number
  onMore?: () => void
}

/** Notch top strip: wings either side of the camera housing. */
export function OverlayNotchHead({ badge, phrase, count, onMore }: OverlayHeadProps) {
  return (
    <div className="m-overlay-wings">
      <div className="m-overlay-wing">
        <Mascot size={18} badge={badge} />
        <span className="m-ellipsis">{phrase}</span>
      </div>
      <div className="m-overlay-housing" />
      <div className="m-overlay-wing is-right">
        {count !== undefined && <span className="m-count">{count}</span>}
        <MenuButton onClick={onMore} />
      </div>
    </div>
  )
}

/** Top-bar and right-edge head: mascot, phrase, count, ⋮. */
export function OverlayPanelHead({ badge, phrase, count, onMore }: OverlayHeadProps) {
  return (
    <div className="m-overlay-head">
      <Mascot size={18} badge={badge} />
      <span className="m-ellipsis" style={{ fontWeight: 500 }}>
        {phrase}
      </span>
      {count !== undefined && (
        <span className="m-count" style={{ marginLeft: 'auto' }}>
          {count}
        </span>
      )}
      <MenuButton onClick={onMore} style={count === undefined ? { marginLeft: 'auto' } : undefined} />
    </div>
  )
}

export interface OverlayProps extends OverlayHeadProps {
  placement: Placement
  /** Exposed as `data-state` for the window glue. */
  state?: OverlayState
  tabs?: OverlayTabSpec[]
  activeTab?: string
  onTab?: (id: string) => void
  /** Replaces the default head. */
  head?: ReactElement
  children?: ReactNode
  /** The capture box. */
  footer?: ReactElement
  className?: string
  style?: CSSProperties
  testId?: string
}

/**
 * The expanded overlay frame. The notch bezel is always the dark pack, so its
 * artwork is too; in the notch the frame sits in a `.m-notch` wrapper whose
 * top corners flare into the screen's edge (02 §6.1), and `className` and
 * `style` go on that wrapper, the outermost box (the one the window is sized to).
 */
export function Overlay({ placement, state, badge, phrase, count, onMore, tabs, activeTab, onTab, head, children, footer, className, style, testId }: OverlayProps) {
  const notch = placement === 'notch'
  const frame = (
    <div className={cx('m-overlay', PLACEMENT_CLASS[placement], !notch && className)} data-state={state} style={notch ? undefined : style} data-testid={testId}>
      {head ??
        (placement === 'notch' ? (
          <OverlayNotchHead badge={badge} phrase={phrase} count={count} onMore={onMore} />
        ) : (
          <OverlayPanelHead badge={badge} phrase={phrase} count={count} onMore={onMore} />
        ))}
      {tabs && (
        <Tabs>
          {tabs.map((t) => (
            <Tab key={t.id} active={t.id === activeTab} onClick={onTab ? () => onTab(t.id) : undefined}>
              {t.label}
            </Tab>
          ))}
        </Tabs>
      )}
      {children}
      {footer}
    </div>
  )
  return notch ? (
    <ThemeProvider theme="dark">
      <div className={cx('m-notch', className)} style={style}>
        {frame}
      </div>
    </ThemeProvider>
  ) : (
    frame
  )
}

export interface PillProps {
  badge?: SessionState
  phrase: ReactNode
  count?: number
  /** Hangs from the notch: dark pack, no border. */
  notch?: boolean
  /** The inline action and ⋮ after the count (02 §5.2). */
  actions?: ReactNode
  /** Replaces the mascot (`overlay.show_mascot = false`: the 6-px state dot). */
  mascot?: ReactNode
  /** Attributes of the phrase span (a live region, 02 §13). */
  phraseProps?: HTMLAttributes<HTMLSpanElement>
  onClick?: () => void
  className?: string
  style?: CSSProperties
  testId?: string
}

/** The compact pill: mascot artwork 18 px + one phrase + count. In the notch it sits in the `.m-notch` wrapper, which takes `className` and `style`. */
export function Pill({ badge, phrase, count, notch, actions, mascot, phraseProps, onClick, className, style, testId }: PillProps) {
  const pill = (
    <span className={cx('m-pill', notch && 'is-notch', !notch && className)} style={notch ? undefined : style} role={onClick ? 'button' : undefined} tabIndex={onClick ? 0 : undefined} onClick={onClick} data-testid={testId}>
      {mascot ?? <Mascot size={18} badge={badge} />}
      <span className="m-pill-text" {...phraseProps}>
        {phrase}
      </span>
      {count !== undefined && <span className="m-count">{count}</span>}
      {actions}
    </span>
  )
  return notch ? (
    <ThemeProvider theme="dark">
      <span className={cx('m-notch', className)} style={style}>
        {pill}
      </span>
    </ThemeProvider>
  ) : (
    pill
  )
}

export interface PeekProps {
  badge?: SessionState
  /** The top-bar strip instead of the right-edge tab. */
  top?: boolean
  /** Top-bar only: “3 items”. */
  label?: ReactNode
  /** Replaces the duck (the overlay's resting `.m-notch-bot`, or the 6-px state dot). */
  mascot?: ReactElement
  /** Now playing (02 §6.2, §6.3): the artwork or the equalizer, right of the mascot in the top-bar tab, under it in the right-edge tab. */
  eq?: ReactElement | null
  onClick?: () => void
  className?: string
}

export function Peek({ badge, top, label, mascot, eq, onClick, className }: PeekProps) {
  return (
    <span className={cx('m-peek', top && 'is-top', className)} role={onClick ? 'button' : undefined} onClick={onClick}>
      {mascot ?? <Mascot size={18} badge={badge} />}
      {eq}
      {label !== undefined && runs([' ', label])}
    </span>
  )
}
