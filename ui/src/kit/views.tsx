import type { CSSProperties, MouseEvent, ReactElement, ReactNode } from 'react'
import { Button, MenuButton } from './button'
import { Key, Switch } from './controls'
import { cx } from './cx'
import { Row, StateCol } from './lists'
import { joinDot } from './text'
import type { StateKind } from './types'

/* ---- connectors tab (§7.6) ------------------------------------------------ */

export interface ConnectorRowProps {
  /** The service's 16-px brand mark (an MCP server), or a kit icon. */
  mark?: ReactNode
  state: StateKind
  stateLabel?: ReactNode
  name: ReactNode
  detail?: ReactNode
  meta?: ReactNode
  /** Receives the click so a caller can anchor its menu to the ⋮. */
  onMore?: (e: MouseEvent<HTMLButtonElement>) => void
  /** Accessible name of the ⋮ ("Groq options"). */
  menuLabel?: string
  onClick?: () => void
  className?: string
}

/** `.m-row` without the avatar column; the status glyph sits in the state column. */
export function ConnectorRow({ state, stateLabel, mark, name, detail, meta, onMore, menuLabel, onClick, className }: ConnectorRowProps) {
  return <Row state={state} stateLabel={stateLabel} mark={mark} name={name} detail={detail} meta={meta} onMenu={onMore} menuLabel={menuLabel} onClick={onClick} className={cx('m-connector-row', className)} />
}

/* ---- shortcuts view (§7.7) ------------------------------------------------ */

export interface ShortcutProps {
  name: ReactNode
  keys: string[]
  /** `global`, `app`, `list`. */
  scope: ReactNode
  /** Conflict text under the name, in --m-bad. */
  conflict?: ReactNode
  /** The focus ring around the caps while recording. */
  editing?: boolean
  onEdit?: () => void
  onMore?: () => void
  className?: string
}

/** Action name, key caps that become editable on click, scope word, ⋮. */
export function Shortcut({ name, keys, scope, conflict, editing, onEdit, onMore, className }: ShortcutProps) {
  return (
    <div className={cx('m-shortcut', className)}>
      <div className="m-row-text">
        <div className="m-row-name">{name}</div>
        {conflict !== undefined && <div className="m-row-detail is-conflict">{conflict}</div>}
      </div>
      <span
        className="m-keys"
        role="button"
        tabIndex={0}
        aria-label="Edit shortcut"
        style={editing ? { boxShadow: '0 0 0 1px var(--m-line-strong)' } : undefined}
        onClick={onEdit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') onEdit?.()
        }}
      >
        {keys.map((k, i) => (
          <Key key={i}>{k}</Key>
        ))}
      </span>
      <span className="m-row-meta">{scope}</span>
      <MenuButton onClick={onMore} />
    </div>
  )
}

/* ---- integrations view (§7.8) --------------------------------------------- */

export function Integrations({ children, className, style }: { children?: ReactNode; className?: string; style?: CSSProperties }) {
  return (
    <div className={cx('m-integrations', className)} style={style}>
      {children}
    </div>
  )
}

export interface IntegrationCardProps {
  /** Brand mark, bot-head or mascot, 32 px. */
  icon: ReactNode
  name: ReactNode
  /** Status line: glyph + word, then the detail after a ` · `. */
  status?: { state: StateKind; label?: ReactNode; detail?: ReactNode }
  /** Count lines, 11 px. */
  lines?: ReactNode[]
  /** The one button; `primary` only where the user is needed (Reconnect). */
  action?: { label: ReactNode; primary?: boolean; onClick?: () => void }
  /** After the button (a switch). */
  extra?: ReactNode
  /** Dimmed, no ⋮: “Available later · reason”. */
  later?: boolean
  onMore?: () => void
  className?: string
}

export function IntegrationCard({ icon, name, status, lines, action, extra, later, onMore, className }: IntegrationCardProps) {
  return (
    <div className={cx('m-integration', later && 'is-later', className)}>
      {icon}
      <IntegrationHead name={name} status={later ? undefined : status} />
      {!later && <MenuButton onClick={onMore} />}
      <div className="m-integration-body">
        {lines?.map((line, i) => (
          <div key={i} className="m-meta">
            {line}
          </div>
        ))}
        {!later && (action || extra) && (
          <div className="m-card-actions">
            {action && (
              <Button variant={action.primary ? 'primary' : 'default'} onClick={action.onClick}>
                {action.label}
              </Button>
            )}
            {extra}
          </div>
        )}
      </div>
    </div>
  )
}

/* ---- automations view (§7.9) ---------------------------------------------- */

export interface AutomationRowProps {
  name: ReactNode
  on: boolean
  onToggle?: (on: boolean) => void
  /** Accessible name of the switch; defaults to `<name> on|off` when `name` is a string. */
  switchLabel?: string
  /** `Weekdays 08:00`. */
  trigger: ReactNode
  /** `Start session in mode · summarize inbox and calendar`. */
  action: ReactNode
  /** State glyph + relative time. */
  lastRun?: { state: StateKind; label: ReactNode }
  /** `tomorrow 08:00`, `paused`. */
  nextRun?: ReactNode
  onMore?: () => void
  onClick?: () => void
  className?: string
}

export function AutomationRow({ name, on, onToggle, switchLabel, trigger, action, lastRun, nextRun, onMore, onClick, className }: AutomationRowProps) {
  const label = switchLabel ?? (typeof name === 'string' ? `${name} ${on ? 'on' : 'off'}` : undefined)
  return (
    <div className={cx('m-automation', className)} role={onClick ? 'button' : undefined} tabIndex={onClick ? 0 : undefined} onClick={onClick}>
      <Switch on={on} onChange={onToggle} label={label} />
      <div className="m-row-text">
        <div className="m-row-name" style={{ fontWeight: 500 }}>
          {name}
        </div>
        <div className="m-row-detail">{joinDot(trigger, action)}</div>
      </div>
      {lastRun ? <StateCol state={lastRun.state}>{lastRun.label}</StateCol> : <span />}
      <span className="m-row-meta">{nextRun}</span>
      <MenuButton
        onClick={(e) => {
          e.stopPropagation()
          onMore?.()
        }}
      />
    </div>
  )
}

/* ---- forms ---------------------------------------------------------------- */

/** Label · control rows (the automation editor). Deprecated: the label-column form gives way to the Settings rows (`SettingGroup` / `SettingRow`, kit/settings.tsx); it keeps working until its last callers move. */
export function Form({ label, children, className, style }: { label?: string; children?: ReactNode; className?: string; style?: CSSProperties }) {
  return (
    <div className={cx('m-form', className)} style={style} role="form" aria-label={label}>
      {children}
    </div>
  )
}

export interface FormFieldProps {
  /** Omitted = an empty label cell (the Save row). */
  label?: ReactNode
  htmlFor?: string
  children: ReactNode
}

/** Deprecated with `Form`. */
export function FormField({ label, htmlFor, children }: FormFieldProps) {
  return (
    <>
      {label !== undefined ? (
        <label className="m-t-dense m-fg-2" htmlFor={htmlFor}>
          {label}
        </label>
      ) : (
        <span />
      )}
      {children}
    </>
  )
}

/** The head of an integration card or page (03 §7C.1): the name over the status line. */
export function IntegrationHead({ name, status }: { name: ReactNode; status?: IntegrationCardProps['status'] }) {
  return (
    <div className="m-integration-head">
      <span className="m-card-title">{name}</span>
      {status && <IntegrationStatus status={status} />}
    </div>
  )
}

/** The status line: the state word, then the detail; the " · " between them is the stylesheet's (`.m-integration-detail::before`). */
export function IntegrationStatus({ status, className }: { status: NonNullable<IntegrationCardProps['status']>; className?: string }) {
  return (
    <div className={cx('m-integration-status', className)}>
      <StateCol state={status.state}>{status.label}</StateCol>
      {status.detail && (
        <span className="m-integration-detail" title={typeof status.detail === 'string' ? status.detail : undefined}>
          {status.detail}
        </span>
      )}
    </div>
  )
}

export interface SectionProps {
  title: ReactNode
  /** Right of the title, `--m-fg-3`. */
  meta?: ReactNode
  /** The body holds rows (4 / 8 px padding instead of 12 / 16). */
  list?: boolean
  /** Buttons under the body. */
  foot?: ReactNode
  children?: ReactNode
  className?: string
  testId?: string
  /** An icon button after the meta, at the head's right (the catalog's refresh beside its "fetched" stamp); an element, so the head's text stays one node. */
  actions?: ReactElement | null
}

/** A page section (03 §7C.3): a bordered card with a head, a body and a foot for its buttons. */
export function Section({ title, meta, list, foot, actions, children, className, testId }: SectionProps) {
  return (
    <section className={cx('m-section', className)} aria-label={typeof title === 'string' ? title : undefined} data-testid={testId}>
      <div className="m-section-head">
        {title}
        {meta !== undefined && <span className="m-t-meta">{meta}</span>}
        {actions}
      </div>
      {children !== undefined && children !== null && <div className={cx('m-section-body', list && 'is-list')}>{children}</div>}
      {foot && <div className="m-section-foot">{foot}</div>}
    </section>
  )
}

export interface SourceRowProps {
  name: ReactNode
  /** After the name, `--m-fg-3` ("always watched"). */
  note?: ReactNode
  /** One switch per column, right-aligned under the head's labels. */
  cells: ReactNode[]
}

/** The head of a watched-rows list: a blank name column then one label per switch column. */
export function SourceHead({ labels }: { labels: string[] }) {
  return (
    <div className={cx('m-source-head', labels.length === 1 && 'is-two')}>
      <span />
      {labels.map((l) => (
        <span key={l}>{l}</span>
      ))}
    </div>
  )
}

export function SourceRow({ name, note, cells }: SourceRowProps) {
  return (
    <div className={cx('m-source-row', cells.length === 1 && 'is-two')}>
      <span className="m-source-name">
        {name}
        {note ? <span className="m-t-meta">{note}</span> : null}
      </span>
      {cells.map((c, i) => (
        <span key={i} className="m-source-cell">
          {c}
        </span>
      ))}
    </div>
  )
}
