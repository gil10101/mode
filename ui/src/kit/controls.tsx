import {
  Fragment,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type InputHTMLAttributes,
  type KeyboardEvent,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
  type Ref,
  type TextareaHTMLAttributes,
} from 'react'
import { PersonAvatar } from './avatars'
import { cx } from './cx'
import { Icon, type IconId } from './icons'
import { runs } from './text'

interface BoxProps {
  children?: ReactNode
  className?: string
  style?: CSSProperties
}

/* ---- menu ---------------------------------------------------------------- */

export interface MenuProps extends BoxProps {
  /** Controlled: `false` renders nothing. */
  open?: boolean
  /** The chat `+` menu: 14-px rows, icon + label, hint right. */
  plus?: boolean
  label?: string
  /** Escape. */
  onClose?: () => void
}

export function Menu({ open = true, plus, label, onClose, children, className, style }: MenuProps) {
  if (!open) return null
  return (
    <div
      className={cx('m-menu', plus && 'is-plus', className)}
      style={style}
      role="menu"
      aria-label={label}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          onClose?.()
          return
        }
        // ↑ ↓ move between the items with wrap, Home / End to the ends (03 §10.7, 10 §11.2).
        if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp' && e.key !== 'Home' && e.key !== 'End') return
        const items = Array.from(e.currentTarget.querySelectorAll<HTMLElement>('[role^="menuitem"]:not([aria-disabled="true"])'))
        if (items.length === 0) return
        e.preventDefault()
        const at = items.indexOf(document.activeElement as HTMLElement)
        const last = items.length - 1
        const next =
          e.key === 'Home' ? 0 : e.key === 'End' ? last : e.key === 'ArrowDown' ? (at >= last ? 0 : at + 1) : at <= 0 ? last : at - 1
        items[next]?.focus()
      }}
    >
      {children}
    </div>
  )
}

export interface MenuItemProps {
  icon?: IconId
  /** A brand mark in the icon's place (the chat connectors, 10 §7.4.16). */
  mark?: ReactElement
  /** Plain muted letter at the right; the stylesheet keeps it a letter, not a cap, inside menus. */
  shortcut?: string
  hint?: ReactNode
  /** One-line reason under the label (unavailable items). */
  reason?: ReactNode
  danger?: boolean
  dimmed?: boolean
  /** An element after the label (a switch). */
  trailing?: ReactElement
  onSelect?: () => void
  children: ReactNode
  className?: string
}

export function MenuItem({ icon, mark, shortcut, hint, reason, danger, dimmed, trailing, onSelect, children, className }: MenuItemProps) {
  const wrapped = icon !== undefined || mark !== undefined || reason !== undefined
  return (
    <div
      className={cx('m-menu-item', danger && 'is-danger', dimmed && 'is-dimmed', className)}
      role="menuitem"
      tabIndex={dimmed ? -1 : 0}
      aria-disabled={dimmed || undefined}
      onClick={dimmed ? undefined : onSelect}
      onKeyDown={(e) => {
        if (!dimmed && (e.key === 'Enter' || e.key === ' ')) {
          e.preventDefault()
          onSelect?.()
        }
      }}
    >
      {mark ?? (icon && <Icon id={icon} />)}
      {wrapped ? (
        <span className="m-label">
          {children}
          {reason !== undefined && <span className="m-reason">{reason}</span>}
        </span>
      ) : (
        children
      )}
      {shortcut !== undefined && <span className="m-key">{shortcut}</span>}
      {hint !== undefined && <span className="m-hint">{hint}</span>}
      {trailing}
    </div>
  )
}

export function MenuSep() {
  return <div className="m-menu-sep" role="separator" />
}

/* ---- inputs -------------------------------------------------------------- */

export interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  /** 32 px (composer, palette). */
  lg?: boolean
}

export function Input({ lg, className, ...rest }: InputProps) {
  return <input className={cx('m-input', lg && 'is-lg', className)} {...rest} />
}

export function TextArea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={cx('m-input', className)} {...rest} />
}

export interface StaticInputProps extends BoxProps {
  value?: ReactNode
  placeholder?: ReactNode
  focused?: boolean
  lg?: boolean
}

/** A non-editable input drawing (focused state with a caret, or a placeholder); for previews and static states. */
export function StaticInput({ value, placeholder, focused, lg, className, style }: StaticInputProps) {
  return (
    <div className={cx('m-input', lg && 'is-lg', focused && 'is-focused', className)} style={style}>
      {value !== undefined ? <span className="m-ellipsis">{value}</span> : <span className="m-placeholder">{placeholder}</span>}
      {focused && <span className="m-caret" />}
    </div>
  )
}

export interface PickerProps extends BoxProps {
  value: ReactNode
  label?: string
  onClick?: () => void
}

/** A select-like control: the value and a ▾ in an input frame. Deprecated: `Select` replaces it (10 §7.4.5); it stays until its last callers move. */
export function Picker({ value, label, onClick, className, style }: PickerProps) {
  return (
    <div
      className={cx('m-input', className)}
      style={{ width: 'auto', gap: 8, ...style }}
      role="button"
      tabIndex={0}
      aria-label={label}
      aria-haspopup="menu"
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onClick?.()
        }
      }}
    >
      {value}
      <span className="m-t-meta">▾</span>
    </div>
  )
}

/* ---- select and combobox (10 §7.4.5) ------------------------------------ */

/** One menu entry as the window's menu host draws it (the shape of `MenuEntry` in app/ui.tsx). */
export interface MenuOpenerEntry {
  id: string
  label: ReactNode
  checked?: boolean
  run: () => void
}

export type MenuOpener = (anchor: Element, entries: MenuOpenerEntry[], label: string | undefined, options: { align?: 'start' | 'end'; minWidth?: number }) => void

let menuOpener: MenuOpener | null = null

/** The window's popover host registers how a menu opens (app/ui.tsx `openMenu`); `Select` and `Combo` open theirs through it. */
export function setMenuOpener(open: MenuOpener | null) {
  menuOpener = open
}

function openOptions(anchor: HTMLElement, entries: MenuOpenerEntry[], label: string) {
  menuOpener?.(anchor, entries, label, { align: 'start', minWidth: anchor.offsetWidth })
}

export interface SelectProps<V extends string | number> {
  value: V | undefined
  /** `[value, label]` pairs, in menu order. */
  options: readonly (readonly [V, ReactNode])[]
  onChange: (value: V) => void
  /** The accessible name ("Model"); the button shows the chosen option's label. */
  label: string
  /** Shown in fg-3 while no option matches the value. */
  placeholder?: ReactNode
  /** The button's label in place of the chosen option's (a custom id behind "Other…"). */
  display?: ReactNode
  disabled?: boolean
  /** Full width of its column (a stacked row, a dialog field). */
  wide?: boolean
  id?: string
  className?: string
  testId?: string
}

/** A 28-px button with the value and a chevron; opens the window's `.m-menu` under it with a check on the current option. Enter, Space and ArrowDown open it. */
export function Select<V extends string | number>({ value, options, onChange, label, placeholder, display, disabled, wide, id, className, testId }: SelectProps<V>) {
  const current = options.find(([v]) => v === value)
  const open = (anchor: HTMLElement) =>
    openOptions(
      anchor,
      options.map(([v, text]) => ({ id: String(v), label: text, checked: v === value, run: () => onChange(v) })),
      label,
    )
  return (
    <button
      type="button"
      id={id}
      className={cx('m-select', wide && 'is-wide', !current && 'is-placeholder', className)}
      aria-haspopup="menu"
      aria-label={label}
      data-testid={testId}
      disabled={disabled}
      onClick={(e) => open(e.currentTarget)}
      onKeyDown={(e) => {
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault()
          open(e.currentTarget)
        }
      }}
    >
      <span className="m-select-label">{current ? (display ?? current[1]) : placeholder}</span>
      <Icon id="chevron" className="m-select-chev" />
    </button>
  )
}

export interface ComboProps {
  value: string
  onChange: (value: string) => void
  /** The menu's entries; picking one fills the field. */
  suggestions: readonly string[]
  /** An entry's label in the menu (a path shows its last segment); the value itself by default. */
  render?: (suggestion: string) => ReactNode
  /** The accessible name of the field. */
  label: string
  id?: string
  placeholder?: string
  /** Enter, blur or a pick: the value to keep. */
  onCommit?: (value: string) => void
  disabled?: boolean
  /** The field itself (a form that focuses it, a test that types in it). */
  inputRef?: Ref<HTMLInputElement>
  inputTestId?: string
  onFocus?: () => void
  /** Runs before the field's own keys; `preventDefault()` skips them (Esc restoring a stored value). */
  onKeyDown?: (e: KeyboardEvent<HTMLInputElement>) => void
  className?: string
  testId?: string
}

/** Free text with suggestions: an `.m-input` with a trailing chevron that opens the same menu as `Select`. */
export function Combo({ value, onChange, suggestions, render, label, id, placeholder, onCommit, disabled, inputRef, inputTestId, onFocus, onKeyDown, className, testId }: ComboProps) {
  const field = useRef<HTMLDivElement>(null)
  const pick = (anchor: HTMLElement) =>
    openOptions(
      anchor,
      suggestions.map((s) => ({
        id: s,
        label: render ? render(s) : s,
        checked: s === value,
        run: () => {
          onChange(s)
          onCommit?.(s)
        },
      })),
      label,
    )
  return (
    <div className={cx('m-combo', className)} ref={field} data-testid={testId}>
      <input
        ref={inputRef}
        className="m-input"
        id={id}
        value={value}
        placeholder={placeholder}
        aria-label={label}
        disabled={disabled}
        data-testid={inputTestId}
        onChange={(e) => onChange(e.target.value)}
        onFocus={onFocus}
        onBlur={() => onCommit?.(value)}
        onKeyDown={(e) => {
          onKeyDown?.(e)
          if (e.defaultPrevented) return
          if (e.key === 'Enter') {
            e.preventDefault()
            onCommit?.(value)
          } else if (e.key === 'ArrowDown' && suggestions.length && field.current) {
            e.preventDefault()
            pick(field.current)
          }
        }}
      />
      <button
        type="button"
        className="m-btn m-btn-icon is-quiet m-combo-chev"
        aria-label={`Choose ${label.toLowerCase()}`}
        aria-haspopup="menu"
        disabled={disabled || !suggestions.length}
        onClick={() => field.current && pick(field.current)}
      >
        <Icon id="chevron" />
      </button>
    </div>
  )
}

export interface CaptureProps {
  placeholder?: string
  label?: string
  value?: string
  onChange?: (value: string) => void
  /** Enter with text. */
  onSave?: (text: string) => void
  /** The `Open` link of the saved state. */
  onOpen?: () => void
  /** Forces the saved state (otherwise it shows for `savedMs` after a save). */
  saved?: boolean
  savedMs?: number
  className?: string
  /** Replaces the input (static drawings). */
  children?: ReactNode
}

/** One input; after a save “Saved · Open” replaces the placeholder for 3 s. */
export function Capture({
  placeholder = 'Capture a note…',
  label = 'Capture a note',
  value,
  onChange,
  onSave,
  onOpen,
  saved,
  savedMs = 3000,
  className,
  children,
}: CaptureProps) {
  const [text, setText] = useState('')
  const [savedAt, setSavedAt] = useState(0)
  useEffect(() => {
    if (!savedAt) return
    const t = setTimeout(() => setSavedAt(0), savedMs)
    return () => clearTimeout(t)
  }, [savedAt, savedMs])

  const current = value ?? text
  const set = onChange ?? setText
  const isSaved = saved ?? savedAt > 0

  function submit() {
    const t = current.trim()
    if (!t) return
    onSave?.(t)
    set('')
    setSavedAt(Date.now())
  }

  return (
    <div className={cx('m-capture', className)}>
      {children ??
        (isSaved ? (
          <div className="m-input">
            <span className="m-capture-saved">
              {'Saved · '}
              <a
                href="#"
                onClick={(e) => {
                  e.preventDefault()
                  onOpen?.()
                }}
              >
                Open
              </a>
            </span>
          </div>
        ) : (
          <input
            className="m-input"
            placeholder={placeholder}
            aria-label={label}
            value={current}
            onChange={(e) => set(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                submit()
              }
            }}
          />
        ))}
    </div>
  )
}

/* ---- tabs, segmented control, chips, query ------------------------------- */

/** WAI-ARIA tabs: the active tab alone is in the tab order; ArrowLeft / ArrowRight / Home / End move to a tab and choose it. */
export function Tabs({ label, children, className, style }: BoxProps & { label?: string }) {
  return (
    <div className={cx('m-tabs', className)} style={style} role="tablist" aria-label={label} onKeyDown={rove}>
      {children}
    </div>
  )
}

const ROVE_KEYS = new Set(['ArrowLeft', 'ArrowRight', 'Home', 'End'])

function rove(e: KeyboardEvent<HTMLElement>) {
  if (!ROVE_KEYS.has(e.key) || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return
  const tabs = Array.from(e.currentTarget.querySelectorAll<HTMLElement>(':scope > [role="tab"][tabindex]'))
  const i = tabs.indexOf(e.target as HTMLElement)
  if (i < 0) return
  const n = tabs.length
  const next = tabs[e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : (i + (e.key === 'ArrowRight' ? 1 : n - 1)) % n]
  e.preventDefault()
  e.stopPropagation()
  next.focus()
  if (next !== tabs[i]) next.click()
}

export interface TabProps {
  active?: boolean
  /** Plain muted number after the label. */
  count?: number
  onClick?: () => void
  /** The id of the tab panel it shows (the panel carries `role="tabpanel"`). */
  controls?: string
  id?: string
  children: ReactNode
  className?: string
}

export function Tab({ active, count, onClick, controls, id, children, className }: TabProps) {
  return (
    <span
      className={cx('m-tab', active && 'is-active', className)}
      role="tab"
      id={id}
      aria-selected={!!active}
      aria-controls={controls}
      tabIndex={onClick ? (active ? 0 : -1) : undefined}
      onClick={onClick}
      onKeyDown={onClick ? pressable(onClick) : undefined}
    >
      {runs([children, count !== undefined && ' '])}
      {count !== undefined && <span className="m-n">{count}</span>}
    </span>
  )
}

function pressable(onClick: () => void) {
  return (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onClick()
    }
  }
}

export function Seg({ label, children, className, style }: BoxProps & { label?: string }) {
  return (
    <span className={cx('m-seg', className)} style={style} role="group" aria-label={label}>
      {children}
    </span>
  )
}

export interface SegItemProps {
  active?: boolean
  /** A ▾ after the label (grouping pickers). */
  caret?: boolean
  onClick?: () => void
  children: ReactNode
  className?: string
}

export function SegItem({ active, caret, onClick, children, className }: SegItemProps) {
  return (
    <span
      className={cx('m-seg-item', active && 'is-active', className)}
      role="button"
      aria-pressed={!!active}
      tabIndex={onClick ? 0 : undefined}
      onClick={onClick}
      onKeyDown={onClick ? pressable(onClick) : undefined}
    >
      {runs([children, caret && ' '])}
      {caret && <span className="m-caret">▾</span>}
    </span>
  )
}

export interface ChipProps {
  /** The one accented chip: a person in the query bar. */
  person?: { initials?: string; src?: string }
  removable?: boolean
  onRemove?: () => void
  onClick?: () => void
  children: ReactNode
  className?: string
}

export function Chip({ person, removable, onRemove, onClick, children, className }: ChipProps) {
  return (
    <span className={cx('m-chip', person && 'is-person', className)} onClick={onClick}>
      {person && <PersonAvatar initials={person.initials} src={person.src} />}
      {runs([person && ' ', children, removable && ' '])}
      {removable && (
        <span
          className="m-x"
          role="button"
          aria-label="Remove"
          onClick={(e: MouseEvent) => {
            e.stopPropagation()
            onRemove?.()
          }}
        >
          ×
        </span>
      )}
    </span>
  )
}

export interface ChipsFieldProps {
  values: string[]
  onChange: (values: string[]) => void
  /** The accessible name of the input ("Backup models"). */
  label: string
  placeholder?: string
  /** No input once the list is full. */
  max?: number
  /** Chip text; the value itself by default (a path shows its last segment). */
  render?: (value: string) => ReactNode
  /** Native tooltip per chip (the full path). */
  title?: (value: string) => string | undefined
  className?: string
  style?: CSSProperties
}

/** 03 §7E.2: removable chips and a bare input in one sunken frame; Enter or a comma adds, Backspace on an empty input removes the last. */
export function ChipsField({ values, onChange, label, placeholder, max, render, title, className, style }: ChipsFieldProps) {
  const [text, setText] = useState('')
  const full = max !== undefined && values.length >= max
  const add = () => {
    const v = text.trim().replace(/,$/, '').trim()
    if (!v) return
    setText('')
    if (full || values.includes(v)) return
    onChange([...values, v])
  }
  return (
    <div className={cx('m-chips', className)} style={style}>
      {values.map((v, i) => (
        <Chip key={`${v}-${i}`} removable onRemove={() => onChange(values.filter((_, j) => j !== i))} className={undefined}>
          <span title={title?.(v)}>{render ? render(v) : v}</span>
        </Chip>
      ))}
      {!full && (
        <input
          aria-label={label}
          placeholder={values.length ? undefined : placeholder}
          value={text}
          onChange={(e) => {
            if (e.target.value.endsWith(',')) {
              setText(e.target.value)
              setTimeout(add, 0)
            } else setText(e.target.value)
          }}
          onBlur={add}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault()
              add()
            } else if (e.key === 'Backspace' && !text && values.length) {
              e.preventDefault()
              onChange(values.slice(0, -1))
            }
          }}
        />
      )}
    </div>
  )
}

/** The Notes query bar: chips and a placeholder in a sunken frame. */
export function Query({ placeholder, children, className, style }: BoxProps & { placeholder?: ReactNode }) {
  return (
    <div className={cx('m-query', className)} style={style}>
      {children}
      {placeholder !== undefined && <span className="m-placeholder">{placeholder}</span>}
    </div>
  )
}

/* ---- switch, key caps, breadcrumb ---------------------------------------- */

export interface SwitchProps {
  on: boolean
  onChange?: (on: boolean) => void
  label?: string
  disabled?: boolean
  className?: string
  style?: CSSProperties
}

/** 36 × 20 track, 16-px white knob: --m-switch-off off, --m-ok on, never accent (10 §7.4.14). */
export function Switch({ on, onChange, label, disabled, className, style }: SwitchProps) {
  return (
    <button
      type="button"
      className={cx('m-switch', on && 'is-on', className)}
      style={style}
      role="switch"
      aria-checked={on}
      aria-label={label}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation()
        onChange?.(!on)
      }}
    />
  )
}

/** Key cap; the Shortcuts view is the only place keys appear (ADR-0032). */
export function Key({ children, className }: BoxProps) {
  return <kbd className={cx('m-key', className)}>{children}</kbd>
}

export interface CrumbProps {
  items: ReactNode[]
  className?: string
  style?: CSSProperties
}

/** Breadcrumb at the top of every sub-detail page (§7.5). */
export function Crumb({ items, className, style }: CrumbProps) {
  return (
    <nav className={cx('m-crumb', className)} style={style} aria-label="Breadcrumb">
      {items.map((item, i) => (
        <span key={i}>{item}</span>
      ))}
    </nav>
  )
}

/* ---- notice, swatch grid ------------------------------------------------- */

export interface NoticeProps {
  /** `bad`: an error, its glyph in --m-bad; `info`: a neutral line, its glyph in --m-fg-3. */
  state?: 'bad' | 'info'
  /** A quiet button at the right ("Retry", "Open AI settings"). */
  action?: ReactElement
  children: ReactNode
  className?: string
  testId?: string
}

/** An inline notice (10 §7.4.20): sunken, a 12-px state glyph, one line, an optional quiet action; it rises in. */
export function Notice({ state = 'info', action, children, className, testId }: NoticeProps) {
  return (
    <div className={cx('m-notice', `is-${state}`, className)} role="status" data-testid={testId}>
      <i className="m-notice-glyph" aria-hidden="true" />
      <span>{children}</span>
      {action}
    </div>
  )
}

export interface SwatchOption {
  id: string
  /** The accessible name and tooltip; never printed. */
  label: string
  /** The art: a 40-px avatar, a theme dot. */
  node: ReactNode
}

export interface SwatchGridProps {
  options: readonly SwatchOption[]
  value: string
  onPick: (id: string) => void
  /** Escape. */
  onClose?: () => void
  columns?: number
  label?: string
  className?: string
  testId?: string
}

/** A popover's grid of picks (10 §7.4.19): five to a row, the chosen one ringed; arrows move in the grid, Enter or Space picks. */
export function SwatchGrid({ options, value, onPick, onClose, columns = 5, label, className, testId }: SwatchGridProps) {
  const focusable = options.some((o) => o.id === value) ? value : options[0]?.id
  return (
    <div
      className={cx('m-swatches', className)}
      role="menu"
      aria-label={label}
      data-testid={testId}
      style={columns !== 5 ? ({ '--m-cols': columns } as CSSProperties) : undefined}
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          onClose?.()
          return
        }
        const step = {
          ArrowLeft: -1,
          ArrowRight: 1,
          ArrowUp: -columns,
          ArrowDown: columns,
        }[e.key]
        if (step === undefined && e.key !== 'Home' && e.key !== 'End') return
        const picks = Array.from(e.currentTarget.querySelectorAll<HTMLElement>('[role="menuitemradio"]'))
        const i = picks.indexOf(e.target as HTMLElement)
        if (i < 0) return
        e.preventDefault()
        const j = e.key === 'Home' ? 0 : e.key === 'End' ? picks.length - 1 : Math.min(picks.length - 1, Math.max(0, i + step!))
        picks[j].focus()
      }}
    >
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          className={cx('m-swatch-cell', o.id === value && 'is-on')}
          role="menuitemradio"
          aria-checked={o.id === value}
          aria-label={o.label}
          title={o.label}
          tabIndex={o.id === focusable ? 0 : -1}
          onClick={() => onPick(o.id)}
        >
          {o.node}
        </button>
      ))}
    </div>
  )
}

/* ---- folder tree --------------------------------------------------------- */

export interface TreeNode {
  id: string
  label: ReactNode
  count?: number
  /** Task glyph on view rows; `''` keeps the column without a glyph. */
  glyph?: ReactNode
  children?: TreeNode[]
}

export interface TreeProps extends BoxProps {
  nodes?: TreeNode[]
  /** Ids of open branches. */
  expanded?: Iterable<string>
  onToggle?: (id: string) => void
  activeId?: string
  onSelect?: (id: string) => void
  label?: string
}

export function Tree({ nodes, expanded, onToggle, activeId, onSelect, label, children, className, style }: TreeProps) {
  const open = new Set(expanded ?? [])
  return (
    <div className={cx('m-tree', className)} style={style} role="tree" aria-label={label}>
      {nodes?.map((node) => (
        <TreeBranch key={node.id} node={node} open={open} onToggle={onToggle} activeId={activeId} onSelect={onSelect} />
      ))}
      {children}
    </div>
  )
}

function TreeBranch({
  node,
  open,
  onToggle,
  activeId,
  onSelect,
}: {
  node: TreeNode
  open: Set<string>
  onToggle?: (id: string) => void
  activeId?: string
  onSelect?: (id: string) => void
}) {
  const hasChildren = node.children !== undefined
  const isOpen = hasChildren && open.has(node.id)
  return (
    <>
      <TreeItem
        open={isOpen}
        leaf={!hasChildren}
        active={activeId === node.id}
        glyph={node.glyph}
        count={node.count}
        onClick={() => {
          if (hasChildren) onToggle?.(node.id)
          onSelect?.(node.id)
        }}
      >
        {node.label}
      </TreeItem>
      {isOpen && (
        <TreeChildren>
          {node.children!.map((child) => (
            <TreeBranch key={child.id} node={child} open={open} onToggle={onToggle} activeId={activeId} onSelect={onSelect} />
          ))}
        </TreeChildren>
      )}
    </>
  )
}

export interface TreeItemProps {
  open?: boolean
  leaf?: boolean
  active?: boolean
  glyph?: ReactNode
  count?: number
  onClick?: () => void
  children: ReactNode
  className?: string
}

export function TreeItem({ open, leaf, active, glyph, count, onClick, children, className }: TreeItemProps) {
  return (
    <div
      className={cx('m-tree-item', open && 'is-open', leaf && 'is-leaf', active && 'is-active', className)}
      role="treeitem"
      aria-expanded={leaf ? undefined : !!open}
      aria-selected={!!active}
      tabIndex={onClick ? 0 : undefined}
      onClick={onClick}
      onKeyDown={onClick ? pressable(onClick) : undefined}
    >
      <span className="m-tree-chev" />
      {glyph !== undefined && <span className="m-glyph">{glyph}</span>}
      <span className="m-tree-label">{children}</span>
      {count !== undefined && <span className="m-tree-n">{count}</span>}
    </div>
  )
}

export function TreeChildren({ children }: { children?: ReactNode }) {
  return (
    <div className="m-tree-children" role="group">
      {children}
    </div>
  )
}

/* ---- command palette ----------------------------------------------------- */

export interface PaletteRowSpec {
  id: string
  icon?: IconId
  label: ReactNode
  meta?: ReactNode
}

export interface PaletteGroup {
  label: string
  rows: PaletteRowSpec[]
}

export interface PaletteProps extends BoxProps {
  query?: string
  onQueryChange?: (query: string) => void
  placeholder?: string
  groups: PaletteGroup[]
  activeId?: string
  onSelect?: (id: string) => void
  /** Replaces the search input (static drawings). */
  input?: ReactNode
}

/** ⌘K: one input, grouped rows, the active row in --m-hover (§7.11). */
export function Palette({ query, onQueryChange, placeholder = 'Search…', groups, activeId, onSelect, input, className, style }: PaletteProps) {
  return (
    <div className={cx('m-palette', className)} style={style} role="dialog" aria-label="Command palette">
      {input ?? (
        <input
          className="m-input is-lg"
          style={{ marginBottom: 4 }}
          value={query ?? ''}
          onChange={(e) => onQueryChange?.(e.target.value)}
          placeholder={placeholder}
          aria-label="Search"
          autoFocus
        />
      )}
      {groups.map((g) => (
        <Fragment key={g.label}>
          <div className="m-group-label">{g.label}</div>
          {g.rows.map((r) => (
            <PaletteRow key={r.id} active={r.id === activeId} icon={r.icon} meta={r.meta} onClick={() => onSelect?.(r.id)}>
              {r.label}
            </PaletteRow>
          ))}
        </Fragment>
      ))}
    </div>
  )
}

export interface PaletteRowProps {
  active?: boolean
  icon?: IconId
  meta?: ReactNode
  onClick?: () => void
  children: ReactNode
}

export function PaletteRow({ active, icon, meta, onClick, children }: PaletteRowProps) {
  return (
    <div className={cx('m-palette-row', active && 'is-active')} role="option" aria-selected={!!active} onClick={onClick}>
      {icon && <Icon id={icon} />}
      <span className="m-ellipsis" style={{ flex: 1 }}>
        {children}
      </span>
      {meta !== undefined && <span className="m-t-meta">{meta}</span>}
    </div>
  )
}
