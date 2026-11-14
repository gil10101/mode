// Settings → Alerts (03 §8.11; 07 §8) on the connectors core's commands:
// quiet hours and "Pause alerts" through `quiet_set` (the core validates
// HH:MM, stores `notifications.quiet_hours` / `notifications.pause_until`
// and pushes `quiet.changed`), the sounds master switch (same key as
// Overlay), the OS notification options, the Focus / DND statement, the
// rules from `alert_rules_list` (the kit's switch row: on/off, what the rule
// listens to and where it goes, when it last fired, premade / edited /
// custom, ⋮ with Test, then Reset to premade on premade rows or Delete
// (confirmed, `alert_rule_delete`) on custom ones) with the selected rule's
// channels, sound, Focus / DND, quiet hours and cooldown below the list,
// every edit written through `alert_rule_set` (optimistic, put back on a
// refusal), Test through `alert_rule_test` (a sample event through the
// pipeline, suppression reported, nothing written), **New rule** (the full
// editor page, `settings/alerts/new`, AlertRuleEditor.tsx; `alert_rule_add`)
// and the recent alerts (`alerts_recent`). Premade rules are turned off or
// reset, never deleted.
import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Button } from '../../kit/button'
import { Select, Switch } from '../../kit/controls'
import { SettingError } from '../../kit/settings'
import { AutomationRow as SwitchRow } from '../../kit/views'
import { invoke } from '../../tauri'
import { cmdErrorText, toCmdError, type QuietChanged } from '../../state/contract'
import type { AlertChannel, AlertOutcome, AlertRow, AlertRuleDeleted, AlertRuleRow, AlertRuleSetArgs, QuietSetArgs } from '../../state/contract-connectors'
import { useSetting, useSettingsStore } from '../../state/settings-store'
import { ago } from '../integrations/model'
import { navigate } from '../router'
import { Dialog, openMenu, useNow, type MenuEntry } from '../ui'
import { ALERT_SOUNDS, channelList, KIND_WORD, mayIgnoreDnd, TIER_WORD } from './alert-rule-draft'
import { Block, CommitInput, Field, Group, Note, parseIntOr, SettingNumber, SettingPicker, SettingSwitch } from './controls'

export { ALERT_SOUNDS, channelList, mayIgnoreDnd }

interface QuietHours {
  enabled: boolean
  from: string
  to: string
}

/** The words of 07 §8.2's suppression rows. */
const SUPPRESSED: Record<string, string> = {
  dedup: 'already alerted',
  stale: 'too old',
  cooldown: 'in its cooldown',
  paused: 'alerts are paused',
  quiet_hours: 'quiet hours',
  focus: 'Focus / Do Not Disturb',
  presentation: 'presenting',
  screen_locked: 'screen locked',
  muted: 'sound muted',
  os_permission: 'OS notifications are off for Mode',
  sounds_off: 'sounds are off',
}

/** "delivered to overlay" / "would be held: quiet hours" for one rule of a test outcome. */
export function outcomeText(o: AlertOutcome | null | undefined): string {
  const r = o?.rules[0]
  if (!o || !r) return o?.overlay_only ? 'no rule matched; the overlay shows it' : 'no rule matched'
  if (r.suppressed_reason) return `would be held: ${SUPPRESSED[r.suppressed_reason] ?? r.suppressed_reason}`
  return `delivered to ${r.channels.join(', ') || 'overlay'}`
}

/** `quiet_set` and the settings it wrote, patched into the store at once. */
export async function setQuiet(args: QuietSetArgs): Promise<QuietChanged | null> {
  const r = await invoke<QuietChanged>('quiet_set', args as unknown as Record<string, unknown>)
  const patch: Record<string, unknown> = {}
  if (args.quiet_hours) patch['notifications.quiet_hours'] = args.quiet_hours
  if (typeof args.pause_minutes === 'number') patch['notifications.pause_until'] = args.pause_minutes > 0 ? new Date(Date.now() + args.pause_minutes * 60_000).toISOString() : null
  useSettingsStore.getState().patchValues(patch)
  return r ?? null
}

/** "Pause alerts" is on: `notifications.pause_until` lies ahead. */
export function isPaused(pauseUntil: unknown, now: number): boolean {
  return typeof pauseUntil === 'string' && Date.parse(pauseUntil) > now
}

export function AlertsPage() {
  const now = useNow(30_000)
  const [quiet] = useSetting<QuietHours>('notifications.quiet_hours')
  const [pauseUntil] = useSetting<string | null>('notifications.pause_until')
  const q = quiet ?? { enabled: false, from: '22:00', to: '08:00' }
  const [focus] = useSetting<{ macos_assertions?: boolean; windows_wnf?: boolean }>('notifications.focus_detection')
  const [error, setError] = useState<string | null>(null)
  const paused = isPaused(pauseUntil, now)
  const writeQuiet = (next: QuietHours) =>
    void setQuiet({ quiet_hours: next }).then(
      () => setError(null),
      (e) => setError(cmdErrorText(e)),
    )
  const pause = (minutes: number) =>
    void setQuiet({ pause_minutes: minutes }).then(
      () => setError(null),
      (e) => setError(cmdErrorText(e)),
    )
  return (
    <>
      <Group title="Pausing" id="quiet">
        <Field
          title="Quiet hours"
          desc="Sounds and OS notifications wait; the overlay still shows everything."
          k="notifications.quiet_hours"
          error={error ? <SettingError testId="error-notifications.quiet_hours">{error}</SettingError> : undefined}
        >
          <CommitInput className="sg-time" label="Quiet hours from" value={q.from} onCommit={(v) => writeQuiet({ ...q, from: String(v).trim() })} placeholder="22:00" />
          <span className="m-set-unit">to</span>
          <CommitInput className="sg-time" label="Quiet hours to" value={q.to} onCommit={(v) => writeQuiet({ ...q, to: String(v).trim() })} placeholder="08:00" />
          <Switch on={q.enabled} label="Quiet hours" onChange={(on) => writeQuiet({ ...q, enabled: on })} />
        </Field>
        <Field
          title="Pause alerts"
          desc={paused ? <span data-testid="paused-until">{`Paused until ${new Date(String(pauseUntil)).toTimeString().slice(0, 5)}.`}</span> : 'For an hour; the overlay still shows everything.'}
          k="notifications.pause_until"
        >
          {paused ? <Button onClick={() => pause(0)}>Resume alerts</Button> : <Button onClick={() => pause(60)}>Pause for 1 hour</Button>}
        </Field>
        <Note>
          {`Mode silences sounds and OS notifications while the system reports Focus or Do Not Disturb (best effort);${focus && (focus.macos_assertions || focus.windows_wnf) ? ' detection is on.' : ' detection is off.'}`}
        </Note>
      </Group>

      <Group title="Notifications" id="os">
        <SettingPicker
          k="notifications.os_preview"
          title="Preview"
          desc="What an OS notification shows."
          options={[
            ['full', 'Sender and text'],
            ['sender', 'Sender only'],
            ['none', 'No preview'],
          ]}
        />
        <SettingSwitch k="notifications.os_sound" title="OS sound" desc="Play the system sound with an OS notification." />
        <SettingSwitch k="sounds.enabled" title="Sounds" desc="The master switch for Mode's own sounds (also on Overlay)." />
        <SettingNumber k="notifications.max_alert_age_minutes" title="Skip older than" desc="Items older than this never alert." unit="minutes" />
      </Group>

      <Group title="Rules" id="rules">
        <AlertRules now={now} />
        <Field title="New rule" desc="Listen to any event and choose where it goes.">
          <Button onClick={() => navigate({ view: 'settings', rest: 'alerts/new' })}>New rule</Button>
        </Field>
      </Group>

      <Group title="Recent alerts" id="recent">
        <RecentAlerts now={now} />
      </Group>
    </>
  )
}

/* ---- rules (07 §8.2) -------------------------------------------------------------------------- */

/** The rule the editor page just added: selected (its settings open below the list) when the list comes back. */
let selectOnReturn: string | null = null

export function selectRuleOnReturn(id: string | null) {
  selectOnReturn = id
}

/** The channel list with one optional channel switched on or off. */
export function withChannel(channels: readonly string[], channel: Exclude<AlertChannel, 'overlay'>, on: boolean): AlertChannel[] {
  return channelList(on ? [...channels, channel] : channels.filter((c) => c !== channel))
}

/** "overlay only" / "overlay, sound, OS notification". */
export function channelsPhrase(channels: readonly string[]): string {
  const extra = [channels.includes('sound') ? 'sound' : null, channels.includes('os_notification') ? 'OS notification' : null].filter(Boolean)
  return extra.length ? `overlay, ${extra.join(', ')}` : 'overlay only'
}

function either(words: string[]): string {
  return words.length < 2 ? (words[0] ?? '') : `${words.slice(0, -1).join(', ')} or ${words[words.length - 1]}`
}

function capital(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1)
}

/** " in mode" for a rule narrowed to a repo (`repo_root` is a path prefix). */
function inRepo(root: string | undefined): string {
  const name = root?.replace(/[\\/]+$/, '').split(/[\\/]/).pop()
  return name ? ` in ${name}` : ''
}

/** What a rule listens to, from its event and filters (07 §8.1–§8.2): "Important, direct or urgent message", "Event in 5 min". */
export function eventPhrase(r: Pick<AlertRuleRow, 'event' | 'filters'>): string {
  const f = r.filters ?? {}
  switch (r.event) {
    case 'session.waiting':
      return (f.waiting_kind?.length && f.waiting_kind.length < 4 ? `Session waiting for ${either(f.waiting_kind.map((k) => k.replace(/_/g, ' ')))}` : 'Session waiting for you') + inRepo(f.repo_root)
    case 'session.failed':
      return `Session failed${inRepo(f.repo_root)}`
    case 'session.finished':
      return (f.min_duration_seconds ? `Session finished after ${Math.round(f.min_duration_seconds / 60)} min or more` : 'Session finished') + inRepo(f.repo_root)
    case 'message.received': {
      const what = f.kind?.length && f.kind.length < 5 ? either(f.kind.map((k) => KIND_WORD[k] ?? k)) : 'message'
      const tiers = f.tier?.length ? `${either(f.tier.map((t) => TIER_WORD[t] ?? t))} ${what}` : `any ${what}`
      return capital(f.include_resurfaced ? `${tiers}, resurfaced too` : tiers)
    }
    case 'calendar.starting':
      return `Event in ${f.minutes_before ?? 5} min${f.requires_join_url ? ' with a join link' : ''}`
    case 'note.due':
      return `${f.urgent_only ? 'Urgent notes' : 'Notes'} due, ${!f.at || f.at === 'start_of_day' ? 'at the start of the day' : `at ${f.at}`}`
    case 'usage.threshold':
      return typeof f.percent === 'number' ? `AI tokens at ${f.percent} % of the budget` : 'AI tokens at a budget threshold'
    case 'automation.failed':
      return 'Automation run failed'
    default:
      return r.event
  }
}

/** The row's meta cell (03 §8.11 "premade rules marked"): premade, edited (a premade rule the user changed), custom. */
export function ruleMarker(r: Pick<AlertRuleRow, 'kind' | 'edited'>): string {
  return r.kind !== 'premade' ? 'custom' : r.edited ? 'edited' : 'premade'
}

/** The optimistic row for an edit (what `alert_rule_set` will answer when it accepts it). */
export function applyRuleEdit(r: AlertRuleRow, args: Omit<AlertRuleSetArgs, 'id'>): AlertRuleRow {
  const next: AlertRuleRow = { ...r }
  if (args.enabled !== undefined) next.enabled = args.enabled
  if (args.channels !== undefined) next.channels = channelList(args.channels)
  if (args.sound !== undefined) next.sound = args.sound.trim() || null
  if (args.cooldown_seconds !== undefined) next.cooldown_seconds = args.cooldown_seconds
  if (args.respect_dnd !== undefined) next.respect_dnd = args.respect_dnd
  if (args.quiet_hours !== undefined) next.quiet_hours = args.quiet_hours
  if (r.kind === 'premade' && !args.reset) next.edited = true
  return next
}

function AlertRules({ now }: { now: number }) {
  const [rules, setRules] = useState<AlertRuleRow[] | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [error, setError] = useState<{ id: string; text: string } | null>(null)
  // A rule the editor page just added comes back selected, its settings open below the list.
  const [selected, setSelected] = useState<string | null>(() => selectOnReturn)
  const [confirm, setConfirm] = useState<AlertRuleRow | null>(null)
  const [results, setResults] = useState<Record<string, string>>({})
  const list = useRef<HTMLDivElement>(null)
  const anchor = useRef<Element | null>(null)
  // The rows as stored: what an edit reverts to when the core refuses it.
  const stored = useRef<Record<string, AlertRuleRow>>({})

  const load = useCallback(() => {
    invoke<AlertRuleRow[]>('alert_rules_list').then(
      (r) => {
        const rows = Array.isArray(r) ? r : []
        stored.current = Object.fromEntries(rows.map((x) => [x.id, x]))
        setRules(rows)
        setLoadError(null)
      },
      (e) => {
        setRules([])
        setLoadError(cmdErrorText(e))
      },
    )
  }, [])
  useEffect(load, [load])
  useEffect(() => {
    selectOnReturn = null
  }, [])
  // The returning rule is the last row: bring it into view once the list is drawn.
  const shown = rules !== null
  useEffect(() => {
    if (shown) list.current?.querySelector('.m-automation.is-selected')?.scrollIntoView?.({ block: 'nearest' })
  }, [shown])

  const put = (row: AlertRuleRow) => setRules((rs) => (rs ?? []).map((x) => (x.id === row.id ? row : x)))
  const edit = async (rule: AlertRuleRow, args: Omit<AlertRuleSetArgs, 'id'>) => {
    put(applyRuleEdit(rule, args))
    try {
      const saved = await invoke<AlertRuleRow>('alert_rule_set', { id: rule.id, ...args })
      const row = saved && typeof saved === 'object' && saved.id === rule.id ? saved : applyRuleEdit(rule, args)
      stored.current[rule.id] = row
      put(row)
      setError((e) => (e?.id === rule.id ? null : e))
    } catch (e) {
      const back = stored.current[rule.id]
      if (back) put(back)
      setError({ id: rule.id, text: `${rule.name}: ${cmdErrorText(e)}` })
    }
  }
  const test = async (rule: AlertRuleRow) => {
    setSelected(rule.id)
    setResults((r) => ({ ...r, [rule.id]: 'testing…' }))
    try {
      const o = await invoke<AlertOutcome>('alert_rule_test', { rule_id: rule.id })
      setResults((r) => ({ ...r, [rule.id]: outcomeText(o) }))
    } catch (e) {
      setResults((r) => ({ ...r, [rule.id]: cmdErrorText(e) }))
    }
  }
  /** `alert_rule_delete` after the confirmation: the row goes; a refusal (`premade`, `not_found`) says why and keeps it (`not_found` re-reads the list). */
  const remove = async (rule: AlertRuleRow) => {
    try {
      const r = await invoke<AlertRuleDeleted>('alert_rule_delete', { id: rule.id })
      delete stored.current[rule.id]
      setRules((rs) => (rs ?? []).filter((x) => x.id !== rule.id))
      setSelected((s) => (s === rule.id ? null : s))
      setError((e) => (e?.id === rule.id ? null : e))
      if (r && typeof r === 'object' && r.deleted === false) load()
    } catch (e) {
      const err = toCmdError(e)
      setError({ id: rule.id, text: `${rule.name}: ${err.message}` })
      if (err.code === 'not_found') load()
    }
  }
  // Premade rules: Reset (dimmed until changed), never Delete; custom rules: Delete, confirmed first.
  const menu = (rule: AlertRuleRow): MenuEntry[] => [
    { id: 'test', label: 'Test', run: () => void test(rule) },
    ...(rule.kind === 'premade'
      ? [{ id: 'reset', label: 'Reset to premade', dimmed: !rule.edited, reason: rule.edited ? undefined : 'Not changed', run: () => void edit(rule, { reset: true }) } as MenuEntry]
      : [{ kind: 'sep', id: 'sep' } as MenuEntry, { id: 'delete', label: 'Delete', danger: true, run: () => setConfirm(rule) } as MenuEntry]),
  ]

  const rows = rules ?? []
  const current = rows.find((r) => r.id === selected) ?? null
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const els = Array.from(list.current?.querySelectorAll('.m-automation') ?? [])
    const i = els.indexOf(e.target as Element)
    const rule = rows[i]
    if (i < 0 || !rule) return
    if (e.key === 'Enter') {
      e.preventDefault()
      setSelected(rule.id === selected ? null : rule.id)
    } else if (e.key === ' ') {
      e.preventDefault()
      void edit(rule, { enabled: !rule.enabled })
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      ;(els[Math.max(0, Math.min(els.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))] as HTMLElement | undefined)?.focus()
    }
  }

  return (
    <Block setting="alert_rules">
      {rules === null && <div className="m-set-desc">Loading…</div>}
      {loadError && (
        <div className="ap-banner" role="status">
          <span>{`The rules could not be read (${loadError}).`}</span>
          <Button quiet onClick={load}>
            Retry
          </Button>
        </div>
      )}
      {rows.length > 0 && (
        <div ref={list} className="m-list" role="list" aria-label="Alert rules" onPointerDownCapture={(e) => (anchor.current = e.target as Element)} onKeyDown={onKey}>
          {rows.map((rule, i) => {
            const fired = ago(rule.last_fired_at, now)
            return (
              <SwitchRow
                key={rule.id}
                name={rule.name}
                on={rule.enabled}
                onToggle={(on) => void edit(rule, { enabled: on })}
                trigger={eventPhrase(rule)}
                action={channelsPhrase(rule.channels)}
                lastRun={fired ? { state: 'finished', label: fired } : undefined}
                nextRun={ruleMarker(rule)}
                onClick={() => setSelected(rule.id === selected ? null : rule.id)}
                onMore={() => {
                  // The kit row reports no anchor: the pointer-down target, else this row's ⋮ (keyboard).
                  const at = anchor.current?.closest('button') ?? list.current?.querySelectorAll('.m-automation')[i]?.querySelector('button.m-btn-icon')
                  anchor.current = null
                  if (at) openMenu(at, menu(rule), `${rule.name} options`)
                }}
                className={rule.id === selected ? 'is-selected' : undefined}
              />
            )
          })}
        </div>
      )}
      {error && <SettingError testId="rules-error">{error.text}</SettingError>}
      {current && (
        <RuleEditor
          key={current.id}
          rule={current}
          result={results[current.id]}
          onEdit={(args) => void edit(current, args)}
          onRefuse={(text) => setError({ id: current.id, text: `${current.name}: ${text}` })}
          onTest={() => void test(current)}
        />
      )}
      {rows.length > 0 && <div className="m-set-desc">Every rule shows in the overlay; sound and OS notifications are opt-in per rule. Test sends a sample through the rule; nothing is stored.</div>}
      {confirm && (
        <Dialog title={`Delete ${confirm.name}?`} onClose={() => setConfirm(null)} testId="rule-delete">
          <div className="m-card-body">The rule stops alerting; its past alerts stay under Recent.</div>
          <div className="m-card-actions">
            <Button
              variant="primary"
              onClick={() => {
                const rule = confirm
                setConfirm(null)
                void remove(rule)
              }}
            >
              Delete
            </Button>
            <Button onClick={() => setConfirm(null)}>Cancel</Button>
          </div>
        </Dialog>
      )}
    </Block>
  )
}

/** The selected rule's channels, sound, Focus / DND, quiet hours and cooldown (03 §8.11), each written at once. */
function RuleEditor({
  rule,
  result,
  onEdit,
  onRefuse,
  onTest,
}: {
  rule: AlertRuleRow
  result?: string
  onEdit: (args: Omit<AlertRuleSetArgs, 'id'>) => void
  /** A value refused before it reaches the core (not a whole number). */
  onRefuse: (text: string) => void
  onTest: () => void
}) {
  // Remounts the cooldown field after a local refusal so it shows the stored value again.
  const [cooldownKey, setCooldownKey] = useState(0)
  const sound = rule.channels.includes('sound')
  const os = rule.channels.includes('os_notification')
  const session = mayIgnoreDnd(rule.event)
  return (
    <div className="sg-editor" role="group" aria-label={`${rule.name} settings`} data-testid="rule-editor" data-rule={rule.id}>
      <Group title={rule.name} compact>
        <Field title="Sound" desc="Plays with the alert; the overlay always shows it.">
          <Switch on={sound} label="Sound" onChange={(on) => onEdit({ channels: withChannel(rule.channels, 'sound', on) })} />
        </Field>
        <Field title="Which sound" desc={!sound && rule.sound ? 'Plays once the sound channel is on.' : undefined}>
          <Select label="Sound" value={rule.sound ?? ''} options={ALERT_SOUNDS} onChange={(v) => onEdit({ sound: v })} testId="rule-sound" />
        </Field>
        <Field title="OS notification" desc="A system notification as well.">
          <Switch on={os} label="OS notification" onChange={(on) => onEdit({ channels: withChannel(rule.channels, 'os_notification', on) })} />
        </Field>
        <Field
          title="Focus / DND"
          desc={session ? (rule.respect_dnd ? 'Hold sound and OS notification under Focus / DND.' : 'Ignores Focus / DND; the OS may still hold the notification.') : 'Always held under Focus / DND.'}
        >
          <Switch on={rule.respect_dnd} label="Respect Focus / Do Not Disturb" disabled={!session && rule.respect_dnd} onChange={(on) => onEdit({ respect_dnd: on })} />
        </Field>
        <Field title="Quiet hours" desc="Hold sound and OS notification during quiet hours.">
          <Switch on={rule.quiet_hours} label="Obey quiet hours" onChange={(on) => onEdit({ quiet_hours: on })} />
        </Field>
        <Field title="Cooldown" desc="Per item; 0 for none.">
          <CommitInput
            key={cooldownKey}
            className="sg-num"
            label="Cooldown"
            value={rule.cooldown_seconds}
            parse={parseIntOr(false)}
            onCommit={(v) => {
              if (typeof v === 'number') onEdit({ cooldown_seconds: v })
              else {
                onRefuse('the cooldown is a whole number of seconds')
                setCooldownKey((k) => k + 1)
              }
            }}
            testId="rule-cooldown"
          />
          <span className="m-set-unit">seconds</span>
        </Field>
        <Field
          title="Test"
          desc={
            result ? (
              <span role="status" data-testid="rule-test">
                {result}
              </span>
            ) : (
              'Sends a sample through the rule; nothing is stored.'
            )
          }
        >
          <Button onClick={onTest}>Test</Button>
        </Field>
      </Group>
    </div>
  )
}

/** The newest `alert_log` rows (7 days). */
function RecentAlerts({ now }: { now: number }) {
  const [rows, setRows] = useState<AlertRow[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const load = useCallback(() => {
    invoke<AlertRow[]>('alerts_recent', { limit: 20 }).then(
      (r) => {
        setRows(Array.isArray(r) ? r : [])
        setError(null)
      },
      (e) => {
        setRows([])
        setError(cmdErrorText(e))
      },
    )
  }, [])
  useEffect(load, [load])
  return (
    <Block>
      <div className="sg-events" data-testid="recent-alerts">
        {rows === null && <span className="m-set-desc">Reading…</span>}
        {rows !== null && rows.length === 0 && !error && <span className="m-set-desc">No alerts in the last 7 days.</span>}
        {error && <span className="m-set-desc">{`Not available (${error}).`}</span>}
        {rows?.map((a) => (
          <div key={a.id} className="m-stat-row" data-alert={a.id}>
            <span>{a.rule_name ?? a.event}</span>
            <span className="m-t-meta">{`${ago(a.fired_at ?? a.created_at, now) ?? ''}${a.suppressed_reason ? ` · held: ${SUPPRESSED[a.suppressed_reason] ?? a.suppressed_reason}` : ` · ${a.channels.join(', ')}`}`}</span>
          </div>
        ))}
      </div>
    </Block>
  )
}
