// The `.m-form` label-column controls of the first build, kept only for the
// Integrations pages that still draw a `.m-form` (AccountsPage, ClaudeCodePage,
// MarketplacePage) until they move to the Settings rows (10 §7.4.19; the plan's
// step 3 retires `.m-form`). Settings pages use ./controls.tsx. Pickers here are
// the kit's one `Select` (10 §7.4.5).
import { type ReactNode } from 'react'
import { Select } from '../../kit/controls'
import { DeviceGlyph } from '../../kit/settings'
import { FormField } from '../../kit/views'
import { reasonOf, scopeOf, useSetting, useSettingsStore } from '../../state/settings-store'
import { CommitInput, parseIntOr } from './controls'

export { CommitInput }

/** The laptop glyph for device-scope keys (03 §8). */
export function DeviceOnly({ k }: { k: string }) {
  return scopeOf(k) === 'device' ? <DeviceGlyph /> : null
}

/** The last refusal for `k`, in --m-bad, under the control. */
export function FieldError({ k }: { k: string }) {
  const why = useSettingsStore((s) => s.errors[k])
  if (!why) return null
  return (
    <div className="sg-error" role="alert" data-testid={`error-${k}`}>
      {reasonOf(why, k)}
    </div>
  )
}

/** Label · control row inside `.m-form`; the control cell carries its own `<FieldError>`. */
export function Field({ label, htmlFor, children }: { label: ReactNode; htmlFor?: string; children: ReactNode }) {
  return (
    <FormField label={label} htmlFor={htmlFor}>
      {children}
    </FormField>
  )
}

export function SettingNumber({ k, label, unit, nullable, hint, scale = 1 }: { k: string; label: string; unit?: string; nullable?: boolean; hint?: ReactNode; scale?: number }) {
  const [value, set] = useSetting<number | null>(k)
  return (
    <Field label={label}>
      <div className="m-inline" data-setting={k}>
        <CommitInput
          className="sg-num"
          label={label}
          value={value === null ? null : Math.round((value as number) / scale)}
          parse={(t) => {
            const v = parseIntOr(!!nullable)(t)
            return typeof v === 'number' ? v * scale : v
          }}
          onCommit={(v) => void set(v as number | null)}
          placeholder={nullable ? 'not set' : undefined}
        />
        {unit && <span className="m-t-meta">{unit}</span>}
        {hint !== undefined && <span className="m-t-meta">{hint}</span>}
        <DeviceOnly k={k} />
        <FieldError k={k} />
      </div>
    </Field>
  )
}

export function SettingText({ k, label, placeholder, hint, wide }: { k: string; label: string; placeholder?: string; hint?: ReactNode; wide?: boolean }) {
  const [value, set] = useSetting<string | null>(k)
  return (
    <Field label={label}>
      <div className="m-inline" data-setting={k}>
        <CommitInput className={wide ? 'sg-wide' : 'sg-text'} label={label} value={value} placeholder={placeholder} onCommit={(v) => void set(v as string)} />
        {hint !== undefined && <span className="m-t-meta">{hint}</span>}
        <DeviceOnly k={k} />
        <FieldError k={k} />
      </div>
    </Field>
  )
}

/** The kit `Select` in a form cell. */
export function PickerField<V extends string | number | null>({ label, value, options, onPick, testId }: { label: string; value: V; options: readonly (readonly [V, string])[]; onPick: (v: V) => void; testId?: string }) {
  // `Select` keys options by string or number; `null` (an unset value) rides as ''.
  const enc = (v: V) => (v === null ? '' : v) as string | number
  return <Select label={label} value={enc(value)} options={options.map(([v, t]) => [enc(v), t] as const)} onChange={(x) => onPick((options.find(([v]) => enc(v) === x)?.[0] ?? null) as V)} testId={testId} />
}

/** A `.m-group-label` inside the form, spanning both columns; `id` anchors a section. */
export function Group({ children, id }: { children: ReactNode; id?: string }) {
  return (
    <div className="m-group-label" id={id ? `settings-${id}` : undefined}>
      {children}
    </div>
  )
}

/** A line of explanatory text across both columns. */
export function Note({ children, bad }: { children: ReactNode; bad?: boolean }) {
  return <div className={bad ? 'sg-note sg-error' : 'sg-note m-t-meta'}>{children}</div>
}
