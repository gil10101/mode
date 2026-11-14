// Registry-bound controls for the Settings pages (03 §8) on the kit's rows
// (10 §7.4.19): each names its `settings` key, writes on change (no Save
// button), marks device-scope keys with the laptop glyph after the row title and
// shows the core's refusal inline under the control (`.m-set-err`). A group is
// a heading and one card; a row is a title and a one-line description on the
// left, the control on the right (`stack` puts a wide control under the text).
import { useEffect, useRef, useState, type ReactElement, type ReactNode, type Ref } from 'react'
import { Combo, Notice, Seg, SegItem, Select, Switch } from '../../kit/controls'
import { SettingError, SettingGroup, SettingRow } from '../../kit/settings'
import { reasonOf, scopeOf, useSetting, useSettingsStore } from '../../state/settings-store'

/** Whether `k` is a device-scope key, marked by the laptop glyph (03 §8). */
export function isDeviceKey(k: string): boolean {
  return scopeOf(k) === 'device'
}

/** The last refusal for `k`, under the control. */
export function FieldError({ k }: { k: string }) {
  const why = useSettingsStore((s) => s.errors[k])
  if (!why) return null
  return <SettingError testId={`error-${k}`}>{reasonOf(why, k)}</SettingError>
}

/** A group: the heading and its card; `id` anchors `settings/<page>/<section>`. */
export function Group({ title, id, compact, children }: { title?: ReactNode; id?: string; compact?: boolean; children: ReactNode }) {
  return (
    <SettingGroup title={title} id={id} compact={compact}>
      {children}
    </SettingGroup>
  )
}

export interface FieldProps {
  title: ReactNode
  desc?: ReactNode
  /** A registry key: `data-setting` for search hits, its device glyph and refusal. */
  k?: string
  tag?: ReactNode
  stack?: boolean
  htmlFor?: string
  error?: ReactElement | null | false
  testId?: string
  children?: ReactNode
}

/** A row; with `k` it carries the key's `data-setting`, device glyph and refusal. */
export function Field({ title, desc, k, tag, stack, htmlFor, error, testId, children }: FieldProps) {
  return (
    <SettingRow title={title} desc={desc} tag={tag} device={k ? isDeviceKey(k) : false} stack={stack} htmlFor={htmlFor} setting={k} error={error ?? (k ? <FieldError k={k} /> : undefined)} testId={testId}>
      {children}
    </SettingRow>
  )
}

/** A row with nothing but its control at full width (a list, a gallery). */
export function Block({ children, setting, testId, className }: { children: ReactNode; setting?: string; testId?: string; className?: string }) {
  return (
    <div className={className ? `m-set-row is-stack ${className}` : 'm-set-row is-stack'} data-setting={setting} data-testid={testId}>
      <div className="m-set-ctl">{children}</div>
    </div>
  )
}

/** A row with only a description (a statement on the page); `bad` makes it a notice. */
export function Note({ children, bad, testId }: { children: ReactNode; bad?: boolean; testId?: string }) {
  if (bad)
    return (
      <Block testId={testId}>
        <Notice state="bad">{children}</Notice>
      </Block>
    )
  return (
    <div className="m-set-row" data-testid={testId}>
      <div className="m-set-text">
        <div className="m-set-desc">{children}</div>
      </div>
    </div>
  )
}

/** A switch row; `read` / `write` map an object key's field (`general.status_item.badge`). */
export function SettingSwitch({ k, title, desc, label, read, write }: { k: string; title: string; desc?: ReactNode; label?: string; read?: (v: unknown) => boolean; write?: (on: boolean, v: unknown) => unknown }) {
  const [value, set] = useSetting<unknown>(k)
  const on = read ? read(value) : value === true
  return (
    <Field title={title} desc={desc} k={k}>
      <Switch on={on} label={label ?? title} onChange={(next) => void set(write ? write(next, value) : next)} />
    </Field>
  )
}

/** A segmented choice over a fixed vocabulary. */
export function SettingSeg<V extends string | number>({ k, title, desc, options }: { k: string; title: string; desc?: ReactNode; options: readonly (readonly [V, string])[] }) {
  const [value, set] = useSetting<V>(k)
  return (
    <Field title={title} desc={desc} k={k}>
      <Seg label={title}>
        {options.map(([v, text]) => (
          <SegItem key={String(v)} active={value === v} onClick={() => void set(v)}>
            {text}
          </SegItem>
        ))}
      </Seg>
    </Field>
  )
}

/** The kit `Select` over values that may be `null` ("not set"), which rides as ''. */
export function NullableSelect<V extends string | number | null>({
  label,
  value,
  options,
  onPick,
  testId,
  disabled,
}: {
  label: string
  value: V
  options: readonly (readonly [V, ReactNode])[]
  onPick: (v: V) => void
  testId?: string
  disabled?: boolean
}) {
  const enc = (v: V) => (v === null || v === undefined ? '' : v) as string | number
  return (
    <Select
      label={label}
      value={enc(value)}
      options={options.map(([v, t]) => [enc(v), t] as const)}
      onChange={(x) => onPick((options.find(([v]) => enc(v) === x)?.[0] ?? null) as V)}
      testId={testId}
      disabled={disabled}
    />
  )
}

/** A `Select` row over a fixed vocabulary. */
export function SettingPicker<V extends string | number | null>({ k, title, desc, options, testId }: { k: string; title: string; desc?: ReactNode; options: readonly (readonly [V, string])[]; testId?: string }) {
  const [value, set] = useSetting<V>(k)
  return (
    <Field title={title} desc={desc} k={k}>
      <NullableSelect label={title} value={value} options={options} onPick={(v) => void set(v)} testId={testId} />
    </Field>
  )
}

/**
 * A text or number field that commits on Enter and on blur (03 §8:
 * changes apply immediately); Esc restores the stored value. `parse`
 * turns the text into the stored value (null = refuse with `invalid`).
 * With `suggestions` it is the kit `Combo` (free text, a chevron menu).
 */
export function CommitInput({
  value,
  onCommit,
  parse,
  format,
  label,
  className,
  placeholder,
  type,
  inputRef,
  testId,
  disabled,
  id,
  suggestions,
}: {
  value: unknown
  onCommit: (v: unknown) => void
  parse?: (text: string) => unknown
  format?: (v: unknown) => string
  label: string
  className?: string
  placeholder?: string
  type?: string
  inputRef?: Ref<HTMLInputElement>
  testId?: string
  disabled?: boolean
  id?: string
  suggestions?: readonly string[]
}) {
  const shown = format ? format(value) : value === null || value === undefined ? '' : String(value)
  const [text, setText] = useState(shown)
  const editing = useRef(false)
  useEffect(() => {
    if (!editing.current) setText(shown)
  }, [shown])
  const commit = (next: string) => {
    editing.current = false
    if (next === shown) return
    onCommit(parse ? parse(next) : next)
  }
  const restore = (el: HTMLInputElement) => {
    editing.current = false
    setText(shown)
    el.blur()
  }
  if (suggestions)
    return (
      <Combo
        className={className}
        id={id}
        label={label}
        value={text}
        placeholder={placeholder}
        suggestions={suggestions}
        disabled={disabled}
        inputRef={inputRef}
        inputTestId={testId}
        onFocus={() => {
          editing.current = true
        }}
        onChange={(v) => {
          editing.current = true
          setText(v)
        }}
        onCommit={commit}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault()
            e.stopPropagation()
            restore(e.currentTarget)
          }
        }}
      />
    )
  return (
    <input
      ref={inputRef}
      id={id}
      className={className ? `m-input ${className}` : 'm-input'}
      aria-label={label}
      value={text}
      type={type}
      placeholder={placeholder}
      disabled={disabled}
      data-testid={testId}
      onFocus={() => {
        editing.current = true
      }}
      onChange={(e) => setText(e.target.value)}
      onBlur={() => commit(text)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') {
          e.preventDefault()
          commit(text)
          ;(e.target as HTMLInputElement).blur()
        } else if (e.key === 'Escape') {
          e.stopPropagation()
          restore(e.target as HTMLInputElement)
        }
      }}
    />
  )
}

/** Whole numbers; empty text = `null` when `nullable`, else the text (validation reports it). */
export function parseIntOr(nullable: boolean) {
  return (text: string): unknown => {
    const t = text.trim().replace(/[\s,_]/g, '')
    if (!t) return nullable ? null : text
    return /^-?\d+$/.test(t) ? Number(t) : text
  }
}

/**
 * A number row with its unit beside the field (`.m-set-unit`). `read` / `write`
 * map a field of an object key (`ai.timeout_seconds.chat`); the core refuses an
 * out-of-range value and its reason shows under the row.
 */
export function SettingNumber({
  k,
  title,
  desc,
  unit,
  nullable,
  scale = 1,
  label,
  read,
  write,
}: {
  k: string
  title: string
  desc?: ReactNode
  unit?: string
  nullable?: boolean
  scale?: number
  label?: string
  read?: (v: unknown) => number | null
  write?: (n: number | null, v: unknown) => unknown
}) {
  const [stored, set] = useSetting<unknown>(k)
  const value = read ? read(stored) : (stored as number | null)
  return (
    <Field title={title} desc={desc} k={k}>
      <CommitInput
        className="sg-num"
        label={label ?? title}
        value={value === null || value === undefined ? null : Math.round(value / scale)}
        parse={(t) => {
          const v = parseIntOr(!!nullable)(t)
          return typeof v === 'number' ? v * scale : v
        }}
        onCommit={(v) => void set(write ? write(v as number | null, stored) : v)}
        placeholder={nullable ? 'not set' : undefined}
      />
      {unit && <span className="m-set-unit">{unit}</span>}
    </Field>
  )
}

/** A short text row (200 px); `stack` for a long value under the title. */
export function SettingText({ k, title, desc, placeholder, stack }: { k: string; title: string; desc?: ReactNode; placeholder?: string; stack?: boolean }) {
  const [value, set] = useSetting<string | null>(k)
  return (
    <Field title={title} desc={desc} k={k} stack={stack}>
      <CommitInput label={title} value={value} placeholder={placeholder} onCommit={(v) => void set(v as string)} />
    </Field>
  )
}
