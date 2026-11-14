// The Settings pattern (10 §7.4.19, D1 of the design-consistency pass): a page list on the left, a
// centered 680-px page column, groups of a heading and a card of rows. Every settings page, the agent
// editor's tab bodies and the automation and alert editors draw with these; dialogs use the compact
// group (no card surface, 40-px rows).
import { useEffect, useRef, useState, type KeyboardEvent, type ReactElement, type ReactNode, type Ref } from 'react'
import { Select } from './controls'
import { cx } from './cx'
import { Icon, type IconId } from './icons'

export interface SettingsNavItem {
  id: string
  label: string
  icon: IconId
}

export interface SettingsNavProps {
  /** The accessible name of the list ("Settings pages"). */
  label: string
  active: string
  onSelect: (id: string) => void
  /** The filter field above the items (its own `.m-input`). */
  search?: ReactNode
  /** Hairline-separated groups of items. */
  groups: readonly { items: readonly SettingsNavItem[] }[]
  /** 56 px, icons only (a main column under 720 px). */
  collapsed?: boolean
  className?: string
  testId?: string
}

const NAV_KEYS = new Set(['ArrowUp', 'ArrowDown', 'Home', 'End'])

/** The page list: a vertical tablist, the active page alone in the tab order; Up / Down / Home / End move between pages and open them. */
export function SettingsNav({ label, active, onSelect, search, groups, collapsed, className, testId }: SettingsNavProps) {
  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    if (!NAV_KEYS.has(e.key) || e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return
    const items = Array.from(e.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]'))
    const i = items.indexOf(e.target as HTMLElement)
    if (i < 0) return
    const n = items.length
    const next = items[e.key === 'Home' ? 0 : e.key === 'End' ? n - 1 : (i + (e.key === 'ArrowDown' ? 1 : n - 1)) % n]
    e.preventDefault()
    next.focus()
    if (next !== items[i]) next.click()
  }
  return (
    <nav className={cx('m-snav', collapsed && 'is-collapsed', className)} role="tablist" aria-label={label} aria-orientation="vertical" data-testid={testId} onKeyDown={onKeyDown}>
      {search !== undefined && <div className="m-snav-search">{search}</div>}
      {groups.map((g, gi) => [
        gi > 0 && <div key={`sep-${gi}`} className="m-snav-sep" role="separator" />,
        ...g.items.map((it) => (
          <button
            key={it.id}
            type="button"
            className={cx('m-snav-item', it.id === active && 'is-active')}
            role="tab"
            aria-selected={it.id === active}
            tabIndex={it.id === active ? 0 : -1}
            title={collapsed ? it.label : undefined}
            data-page={it.id}
            onClick={() => onSelect(it.id)}
          >
            <Icon id={it.icon} />
            <span className="m-label">{it.label}</span>
          </button>
        )),
      ])}
    </nav>
  )
}

export interface SettingsPageProps {
  title: ReactNode
  /** Lifts the 680-px cap (the Appearance gallery, the Advanced logs). */
  wide?: boolean
  children?: ReactNode
  className?: string
  testId?: string
}

/** The page column: the page title, then its groups. */
export function SettingsPage({ title, wide, children, className, testId }: SettingsPageProps) {
  return (
    <div className={cx('m-set-page', wide && 'is-wide', className)} data-testid={testId}>
      <div className="m-t-page">{title}</div>
      {children}
    </div>
  )
}

export interface SettingGroupProps {
  /** The heading over the card; sentence case, never a row's title. */
  title?: ReactNode
  /** The deep-link anchor: the group renders `id="settings-<id>"`. */
  id?: string
  /** Dialogs: no card surface, 40-px rows. */
  compact?: boolean
  children?: ReactNode
  className?: string
}

export function SettingGroup({ title, id, compact, children, className }: SettingGroupProps) {
  return (
    <section className={cx('m-set-group', compact && 'is-compact', className)} id={id ? `settings-${id}` : undefined} aria-label={typeof title === 'string' ? title : undefined}>
      {title !== undefined && <div className="m-set-head">{title}</div>}
      <div className="m-set-card">{children}</div>
    </section>
  )
}

export interface SettingRowProps {
  title: ReactNode
  /** One line under the title (it wraps when it must). */
  desc?: ReactNode
  /** A quiet text pill after the title. */
  tag?: ReactNode
  /** A device-scope key: the laptop glyph after the title ("This device only"). */
  device?: boolean
  /** The control under the text at full width (a text area, chips, a long field). */
  stack?: boolean
  /** The control's id, so the title labels it. */
  htmlFor?: string
  /** The setting key, for deep links (`data-setting`). */
  setting?: string
  /** A `SettingError` under the control. */
  error?: ReactElement | null | false
  children?: ReactNode
  className?: string
  testId?: string
}

/** The device-scope mark (10 §7.4.19): a 14-px laptop in fg-3, named and titled "This device only". */
export function DeviceGlyph() {
  return (
    <svg className="m-icon m-set-device" role="img" aria-label="This device only">
      <title>This device only</title>
      <use href="#i-laptop" />
    </svg>
  )
}

/** A row: title (+ device glyph, tag) and description left, the control right; an error slides in under the control. */
export function SettingRow({ title, desc, tag, device, stack, htmlFor, setting, error, children, className, testId }: SettingRowProps) {
  return (
    <div className={cx('m-set-row', stack && 'is-stack', className)} data-setting={setting} data-testid={testId}>
      <div className="m-set-text">
        <label className="m-set-title" htmlFor={htmlFor}>
          {title}
          {device && <DeviceGlyph />}
          {tag !== undefined && <span className="m-tag">{tag}</span>}
        </label>
        {desc !== undefined && <div className="m-set-desc">{desc}</div>}
      </div>
      {(children !== undefined || error) && (
        <div className="m-set-ctl">
          {children}
          {error}
        </div>
      )}
    </div>
  )
}

/** The inline error of a row (12/16 --m-bad), announced at once. */
export function SettingError({ children, testId, field }: { children: ReactNode; testId?: string; field?: string }) {
  return (
    <div className="m-set-err" role="alert" data-testid={testId} data-field={field}>
      {children}
    </div>
  )
}

/** The option value behind "Other…" (never a model id). */
const OTHER = '\u0000other'

/** The stacked "Model id" row an "Other…" reveals; it rises in under the row that opened it. */
function ModelIdRow({ id, value, onChange, onFocus, onBlur, onEnter, inputRef }: { id?: string; value: string; onChange: (v: string) => void; onFocus?: () => void; onBlur: (v: string) => void; onEnter?: (v: string) => void; inputRef: Ref<HTMLInputElement> }) {
  return (
    <SettingRow title="Model id" desc="Any id the provider accepts." stack htmlFor={id} className="is-entering">
      <input
        ref={inputRef}
        id={id}
        className="m-input"
        aria-label="Model id"
        value={value}
        spellCheck={false}
        autoComplete="off"
        onFocus={onFocus}
        onChange={(e) => onChange(e.target.value)}
        onBlur={(e) => onBlur(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && onEnter) {
            e.preventDefault()
            onEnter(e.currentTarget.value)
          }
        }}
      />
    </SettingRow>
  )
}

/** Focuses the Model id field once it has mounted after an "Other…" pick. */
function useFocusOnOpen(open: boolean) {
  const field = useRef<HTMLInputElement>(null)
  const pending = useRef(false)
  useEffect(() => {
    if (open && pending.current) {
      pending.current = false
      field.current?.focus()
    }
  }, [open])
  const request = () => {
    pending.current = true
    field.current?.focus()
  }
  return { field, request }
}

export interface ModelChoiceOptions {
  /** The stored model id; null is the default. */
  value: string | null
  onChange: (value: string | null) => void
  /** The first option, standing for null ("CLI default", "Provider default", "Default · claude-haiku-5-5"). */
  defaultLabel: string
  /** The select's label while the default is chosen, when the menu's names a concrete id ("Default"): a model name at most once per surface (10 §12). */
  restLabel?: string
  /** The select button's `data-testid`. */
  selectTestId?: string
  /** The known ids in menu order; duplicates and empties are dropped. */
  models: readonly string[]
  /** The select's accessible name. */
  label: string
  /** The select's id; the Model id field is `<id>-other`. */
  id?: string
  /** Where a write is costly (a route): called on a pick and when the Model id field is left or Enter is pressed; `onChange` then only tracks the typing. */
  onCommit?: (value: string | null) => void
}

/**
 * A model choice as two pieces (03 §9, 10 §7.4.19): the one `Select` (the default first, the known
 * ids, then "Other…") and, after "Other…" or while the stored id is not a known one, the stacked
 * "Model id" row to place under the row that holds the select. A custom id is the select's label;
 * clearing the field returns to the default once it loses focus.
 */
export function useModelChoice({ value, onChange, defaultLabel, restLabel, selectTestId, models, label, id, onCommit }: ModelChoiceOptions): { select: ReactElement; idRow: ReactElement | null } {
  const known = Array.from(new Set(models.filter((m) => m !== '')))
  const custom = !!value && !known.includes(value)
  const [other, setOther] = useState(custom)
  const open = other || custom
  const { field, request } = useFocusOnOpen(open)
  const select = (
    <Select
      id={id}
      label={label}
      value={open ? OTHER : (value ?? '')}
      display={open ? (value ?? undefined) : value ? undefined : restLabel}
      testId={selectTestId}
      options={[['', defaultLabel] as const, ...known.map((m) => [m, m] as const), [OTHER, 'Other…'] as const]}
      onChange={(v) => {
        if (v === OTHER) {
          if (!open) onChange(null)
          setOther(true)
          request()
          return
        }
        setOther(false)
        onChange(v || null)
        onCommit?.(v || null)
      }}
    />
  )
  const idRow = open ? (
    <ModelIdRow
      id={id ? `${id}-other` : undefined}
      inputRef={field}
      value={value ?? ''}
      onFocus={() => setOther(true)}
      onChange={(v) => onChange(v.trim() ? v : null)}
      onEnter={onCommit && ((v) => onCommit(v.trim() || null))}
      onBlur={(v) => {
        if (!v.trim()) setOther(false)
        onCommit?.(v.trim() || null)
      }}
    />
  ) : null
  return { select, idRow }
}

export interface ModelPickerProps extends Omit<ModelChoiceOptions, 'label'> {
  /** The row title and the select's accessible name. */
  title?: string
  label?: string
  desc?: ReactNode
  error?: ReactElement | null | false
  /** The setting key, for deep links (`data-setting`). */
  setting?: string
  testId?: string
}

/** A model row: `useModelChoice`'s select in a settings row titled "Model", its Model id row under it. */
export function ModelPicker({ title = 'Model', label, desc, error, setting, testId, ...choice }: ModelPickerProps) {
  const { select, idRow } = useModelChoice({ ...choice, label: label ?? title })
  return (
    <>
      <SettingRow title={title} desc={desc} htmlFor={choice.id} error={error} setting={setting} testId={testId}>
        {select}
      </SettingRow>
      {idRow}
    </>
  )
}

export interface ModelAdderOptions {
  /** The ids offered, in menu order (the caller leaves out the ones already chosen). */
  models: readonly string[]
  onAdd: (id: string) => void
  /** The select's accessible name and its placeholder ("Add backup"). */
  label: string
  /** The list is full: the select stays, disabled. */
  disabled?: boolean
  id?: string
}

/**
 * Adding a model to a list (the agent's backups): a `Select` that acts, its label shown as the
 * placeholder, over the offered ids then "Other…"; "Other…" reveals the Model id row, where Enter
 * adds what was typed. Place `idRow` under the row that holds the select.
 */
export function useModelAdder({ models, onAdd, label, disabled, id }: ModelAdderOptions): { select: ReactElement; idRow: ReactElement | null } {
  const known = Array.from(new Set(models.filter((m) => m !== '')))
  const [open, setOpen] = useState(false)
  const [text, setText] = useState('')
  const { field, request } = useFocusOnOpen(open)
  const select = (
    <Select<string>
      id={id}
      label={label}
      value={undefined}
      placeholder={label}
      disabled={disabled}
      options={[...known.map((m) => [m, m] as const), [OTHER, 'Other…'] as const]}
      onChange={(v) => {
        if (v === OTHER) {
          setOpen(true)
          request()
          return
        }
        setOpen(false)
        onAdd(v)
      }}
    />
  )
  const idRow =
    open && !disabled ? (
      <ModelIdRow
        id={id ? `${id}-other` : undefined}
        inputRef={field}
        value={text}
        onChange={setText}
        onEnter={(v) => {
          if (!v.trim()) return
          onAdd(v.trim())
          setText('')
          setOpen(false)
        }}
        onBlur={(v) => {
          if (!v.trim()) setOpen(false)
        }}
      />
    ) : null
  return { select, idRow }
}
