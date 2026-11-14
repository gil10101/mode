// Settings → Alerts → New rule (03 §8.11 "event picker over … with the
// filters of 09 §5.5, channels …, `cooldown_seconds`, `respect_dnd`,
// `quiet_hours`"; 07 §8.2). §8.11 does not say page or panel, so this is the
// automation editor's pattern (03 §7B.1–§7B.2, the drill-in rule of 03 §2.4):
// the Alerts page's list keeps the summary and the selected rule's quick
// edits, the full editor is a page under `‹` + the breadcrumb at
// `settings/alerts/new`. The form: Name; the event picker (the eight events
// of `alerts::EVENTS`); the filter rows of that event only (list filters as
// chip groups with "any"; numbers checked against the core's bounds as typed);
// channels (overlay always), sound, Focus / DND (only session.waiting and
// session.failed may ignore it), quiet hours, cooldown; Save as the single
// primary → `alert_rule_add`. A refusal goes under the field it names, or on
// the form's one error line. Saved, the page returns to the list with the new
// rule selected. "Unsaved" shows in the head; `Esc` / `‹` ask before leaving.
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { Notice, Seg, SegItem, Select, Switch } from '../../kit/controls'
import { MainBody } from '../../kit/layout'
import { SettingError, SettingGroup, SettingRow } from '../../kit/settings'
import { invoke } from '../../tauri'
import { ALERT_EVENTS, ALERT_MESSAGE_KINDS, ALERT_TIERS, ALERT_WAITING_KINDS, type AccountRow, type AlertEventName, type AlertRuleRow, type CoreVipRow, type VipRow } from '../../state/contract-connectors'
import type { AutomationRow } from '../../state/contract-automations'
import { navigate } from '../router'
import { Dialog, openMenu } from '../ui'
import {
  ALERT_SOUNDS,
  buildRule,
  channelList,
  EVENT_LABEL,
  KIND_WORD,
  mayIgnoreDnd,
  newDraft,
  refusalField,
  TIER_WORD,
  visibleFields,
  WAITING_WORD,
  withEvent,
  type RuleDraft,
  type RuleErrors,
  type RuleField,
} from './alert-rule-draft'
import { selectRuleOnReturn } from './AlertsPage'

/** One level up (03 §2.4): the Alerts page at its rules. */
const UP = 'settings/alerts/rules'

interface Choice {
  id: string
  label: string
}

/** A row's error (03 §7B.2 Validation, as the automation editor shows them): under the control, naming its field. */
function fieldError(field: RuleField, error?: string) {
  return error ? <SettingError field={field}>{error}</SettingError> : undefined
}

/** A list filter as chips (07 §8.2 "array" filters): "any" while none is chosen; each value toggles. */
function ChipGroup({ label, values, words, chosen, onChange }: { label: string; values: readonly string[]; words: Readonly<Record<string, string>>; chosen: string[]; onChange: (next: string[]) => void }) {
  const toggle = (v: string) => onChange(chosen.includes(v) ? chosen.filter((x) => x !== v) : values.filter((x) => x === v || chosen.includes(x)))
  return (
    <div className="m-inline" role="group" aria-label={label}>
      <ChipToggle on={!chosen.length} onToggle={() => onChange([])}>
        any
      </ChipToggle>
      {values.map((v) => (
        <ChipToggle key={v} on={chosen.includes(v)} onToggle={() => toggle(v)}>
          {words[v] ?? v}
        </ChipToggle>
      ))}
    </div>
  )
}

function ChipToggle({ on, onToggle, children }: { on: boolean; onToggle: () => void; children: ReactNode }) {
  return (
    <span
      className={`m-chip sg-chip${on ? ' is-on' : ''}`}
      role="checkbox"
      aria-checked={on}
      tabIndex={0}
      onClick={onToggle}
      onKeyDown={(e) => {
        if (e.key === ' ' || e.key === 'Enter') {
          e.preventDefault()
          onToggle()
        }
      }}
    >
      {children}
    </span>
  )
}

/** A short number field and its unit. */
function NumberField({ label, value, unit, placeholder, onChange }: { label: string; value: string; unit: string; placeholder?: string; onChange: (v: string) => void }) {
  return (
    <>
      <input className="m-input sg-short" aria-label={label} inputMode="numeric" value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
      <span className="m-set-unit">{unit}</span>
    </>
  )
}

/** A choice over loaded rows, "any" first. */
function ChoicePicker({ label, any, choices, value, onPick }: { label: string; any: string; choices: Choice[]; value: string | null; onPick: (id: string | null) => void }) {
  return <Select label={label} value={value ?? ''} placeholder={value ?? undefined} options={[['', any] as const, ...choices.map((c) => [c.id, c.label] as const)]} onChange={(v) => onPick(v || null)} />
}

function FolderDialog({ value, onClose, onSave }: { value: string; onClose: () => void; onSave: (v: string) => void }) {
  const [v, setV] = useState(value)
  const done = () => {
    onSave(v.trim())
    onClose()
  }
  return (
    <Dialog title="Repo" onClose={onClose} testId="rule-folder">
      <input className="m-input" aria-label="Folder path" placeholder="/path/to/repo" value={v} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && done()} />
      <div className="m-card-actions">
        <Button variant="primary" onClick={done}>
          Use folder
        </Button>
        <Button onClick={onClose}>Cancel</Button>
      </div>
    </Dialog>
  )
}

export function AlertRuleEditor() {
  const [draft, setDraft] = useState<RuleDraft>(() => newDraft())
  const initial = useRef(JSON.stringify(draft))
  const [taken, setTaken] = useState<string[]>([])
  const [tried, setTried] = useState(false)
  const [refused, setRefused] = useState<{ field: RuleField | null; message: string } | null>(null)
  const [saving, setSaving] = useState(false)
  const [leave, setLeave] = useState<string | null>(null)
  const [folderDialog, setFolderDialog] = useState(false)
  const [recent, setRecent] = useState<string[]>([])
  const [accounts, setAccounts] = useState<(Choice & { services: string[] })[]>([])
  const [vips, setVips] = useState<Choice[]>([])
  const [automations, setAutomations] = useState<Choice[]>([])
  const nameRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    setTimeout(() => nameRef.current?.focus(), 0)
    invoke<string[]>('sessions_recent_folders').then(
      (r) => setRecent(Array.isArray(r) ? r : []),
      () => {},
    )
    // The names in use (the core refuses a second one, case-insensitively), and the rows the id filters pick from.
    invoke<AlertRuleRow[]>('alert_rules_list').then(
      (r) => setTaken((Array.isArray(r) ? r : []).map((x) => x.name)),
      () => {},
    )
    invoke<AccountRow[]>('accounts_list').then(
      (r) => setAccounts((Array.isArray(r) ? r : []).map((a) => ({ id: a.id, label: a.display_name, services: a.services ?? [] }))),
      () => {},
    )
    invoke<(VipRow | CoreVipRow)[]>('vips_list').then(
      (r) => setVips((Array.isArray(r) ? r : []).map((v) => ({ id: 'contact_id' in v ? v.contact_id : v.id, label: v.display_name }))),
      () => {},
    )
    invoke<AutomationRow[]>('automations_list').then(
      (r) => setAutomations((Array.isArray(r) ? r : []).map((a) => ({ id: a.id, label: a.name }))),
      () => {},
    )
  }, [])

  const built = useMemo(() => buildRule(draft, taken), [draft, taken])
  const dirty = JSON.stringify(draft) !== initial.current
  const visible = visibleFields(draft.event)
  // Errors show as typed, except the empty name of a form not yet saved.
  const errors: RuleErrors = { ...built.errors }
  if (!tried && !draft.name.trim()) delete errors.name
  if (refused?.field && visible.includes(refused.field)) errors[refused.field] = refused.message
  const formError = refused && !(refused.field && visible.includes(refused.field)) ? refused.message : null

  const patch = (p: Partial<RuleDraft>) => {
    setDraft((d) => ({ ...d, ...p }))
    setRefused(null)
  }

  const go = (to: string) => {
    if (JSON.stringify(draft) !== initial.current) setLeave(to)
    else navigate(to)
  }

  const save = async (): Promise<boolean> => {
    setTried(true)
    const { args, errors: problems } = buildRule(draft, taken)
    if (Object.keys(problems).length) return false
    setSaving(true)
    setRefused(null)
    try {
      const row = await invoke<AlertRuleRow>('alert_rule_add', args as unknown as Record<string, unknown>)
      initial.current = JSON.stringify(draft)
      selectRuleOnReturn(row && typeof row === 'object' ? row.id : null)
      navigate(UP, { replace: true })
      return true
    } catch (e) {
      setRefused(refusalField(e))
      return false
    } finally {
      setSaving(false)
    }
  }

  // `Esc` leaves (03 §2.4: up exactly one level); in a field it only leaves the field.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || document.querySelector('.ap-scrim')) return
      const t = e.target as HTMLElement | null
      if (t?.tagName === 'INPUT' || t?.tagName === 'TEXTAREA') {
        t.blur()
        return
      }
      e.preventDefault()
      go(UP)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  const d = draft
  const session = mayIgnoreDnd(d.event)
  const accountChoices = accounts.filter((a) => !a.services.length || (d.event === 'calendar.starting' ? a.services.includes('calendar') : a.services.some((s) => s !== 'calendar')))
  const sound = d.channels.includes('sound')
  const os = d.channels.includes('os_notification')
  const withChannel = (channel: 'sound' | 'os_notification', on: boolean) => patch({ channels: channelList(on ? [...d.channels, channel] : d.channels.filter((c) => c !== channel)) })

  const tildePath = (p: string) => p.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~')
  const OTHER = '\u0000other'
  const repoRow = (
    <SettingRow title="Repo" desc="A path prefix; sessions under it.">
      <Select
        label="Repo"
        value={d.repo_root}
        options={[['', 'Any repo'] as const, ...Array.from(new Set([...recent, ...(d.repo_root ? [d.repo_root] : [])])).map((p) => [p, tildePath(p)] as const), [OTHER, 'Other folder…'] as const]}
        onChange={(v) => (v === OTHER ? setFolderDialog(true) : patch({ repo_root: v }))}
      />
    </SettingRow>
  )
  const accountRow = (
    <SettingRow title="Account">
      <ChoicePicker label="Account" any="Any account" choices={accountChoices} value={d.account_id} onPick={(v) => patch({ account_id: v })} />
    </SettingRow>
  )

  // The filter rows of the event: exactly the keys `alerts::event_filters` names for it.
  const filterRows: Record<AlertEventName, ReactNode> = {
    'session.waiting': (
      <>
        <SettingRow title="Waiting for" stack error={fieldError('waiting_kind', errors.waiting_kind)}>
          <ChipGroup label="Waiting for" values={ALERT_WAITING_KINDS} words={WAITING_WORD} chosen={d.waiting_kind} onChange={(v) => patch({ waiting_kind: v })} />
        </SettingRow>
        {repoRow}
      </>
    ),
    'session.failed': repoRow,
    'session.finished': (
      <>
        {repoRow}
        <SettingRow title="Longer than" desc="Empty for any length." error={fieldError('min_duration_seconds', errors.min_duration_seconds)}>
          <NumberField label="Longer than" value={d.min_minutes} placeholder="any" unit="minutes" onChange={(v) => patch({ min_minutes: v })} />
        </SettingRow>
      </>
    ),
    'message.received': (
      <>
        <SettingRow title="Level" stack error={fieldError('tier', errors.tier)}>
          <ChipGroup label="Level" values={ALERT_TIERS} words={TIER_WORD} chosen={d.tier} onChange={(v) => patch({ tier: v })} />
        </SettingRow>
        <SettingRow title="Kind" stack error={fieldError('kind', errors.kind)}>
          <ChipGroup label="Kind" values={ALERT_MESSAGE_KINDS} words={KIND_WORD} chosen={d.kind} onChange={(v) => patch({ kind: v })} />
        </SettingRow>
        {accountRow}
        <SettingRow title="From">
          <ChoicePicker label="From" any="Anyone" choices={vips} value={d.contact_id} onPick={(v) => patch({ contact_id: v })} />
        </SettingRow>
        <SettingRow title="Resurfaced" desc="Also when a snoozed message comes back.">
          <Switch on={d.include_resurfaced} label="Resurfaced" onChange={(on) => patch({ include_resurfaced: on })} />
        </SettingRow>
      </>
    ),
    'calendar.starting': (
      <>
        <SettingRow title="Before start" error={fieldError('minutes_before', errors.minutes_before)}>
          <NumberField label="Before start" value={d.minutes_before} unit="minutes" onChange={(v) => patch({ minutes_before: v })} />
        </SettingRow>
        <SettingRow title="Join link" desc="Only events with a join link.">
          <Switch on={d.requires_join_url} label="Join link" onChange={(on) => patch({ requires_join_url: on })} />
        </SettingRow>
        {accountRow}
      </>
    ),
    'note.due': (
      <>
        <SettingRow title="When" error={fieldError('at', errors.at)}>
          <Seg label="When">
            <SegItem active={d.at === 'start_of_day'} onClick={() => patch({ at: 'start_of_day' })}>
              Start of day
            </SegItem>
            <SegItem active={d.at === 'time'} onClick={() => patch({ at: 'time' })}>
              At a time
            </SegItem>
          </Seg>
          {d.at === 'time' && <input className="m-input sg-short" aria-label="Time" placeholder="09:00" value={d.at_time} onChange={(e) => patch({ at_time: e.target.value })} />}
        </SettingRow>
        <SettingRow title="Urgent only" desc="Only notes marked urgent.">
          <Switch on={d.urgent_only} label="Urgent only" onChange={(on) => patch({ urgent_only: on })} />
        </SettingRow>
      </>
    ),
    'usage.threshold': (
      <SettingRow title="Threshold" desc="Of the monthly AI token budget." error={fieldError('percent', errors.percent)}>
        <NumberField label="Threshold" value={d.percent} placeholder="any" unit="%" onChange={(v) => patch({ percent: v })} />
      </SettingRow>
    ),
    'automation.failed': (
      <SettingRow title="Automation">
        <ChoicePicker label="Automation" any="Any automation" choices={automations} value={d.automation_id} onPick={(v) => patch({ automation_id: v })} />
      </SettingRow>
    ),
  }

  const title = d.name.trim() || 'New rule'
  return (
    <section className="ap-view sg-rule-view" aria-label={title} data-testid="rule-new">
      <div className="m-main-head">
        <Button variant="icon" quiet aria-label="Back" onClick={() => go(UP)}>
          ‹
        </Button>
        <nav className="m-crumb sg-crumb" aria-label="Breadcrumb">
          <span aria-current="page">{title}</span>
        </nav>
        {dirty && <span className="m-t-meta sg-dirty">Unsaved</span>}
      </div>
      <MainBody className="sg-rule-page">
        <div className="m-set-page">
          <SettingGroup title="Rule">
            <SettingRow title="Name" htmlFor="sg-rule-name" error={fieldError('name', errors.name)}>
              <input id="sg-rule-name" ref={nameRef} className="m-input" aria-label="Name" value={d.name} onChange={(e) => patch({ name: e.target.value })} />
            </SettingRow>
            <SettingRow title="Event" desc="What the rule listens to; the rows below follow it." error={fieldError('event', errors.event)}>
              <Select
                label="Event"
                value={d.event}
                options={ALERT_EVENTS.map((ev) => [ev, EVENT_LABEL[ev]] as const)}
                onChange={(ev) => {
                  setDraft((x) => withEvent(x, ev))
                  setRefused(null)
                }}
              />
            </SettingRow>
            {filterRows[d.event]}
          </SettingGroup>
          <SettingGroup title="Delivery">
            <SettingRow title="Sound" desc="Plays with the alert; the overlay always shows it." error={fieldError('channels', errors.channels)}>
              <Switch on={sound} label="Sound" onChange={(on) => withChannel('sound', on)} />
            </SettingRow>
            <SettingRow title="Which sound" desc={!sound && d.sound ? 'Plays once the sound channel is on.' : undefined}>
              <Select label="Sound" value={d.sound} options={ALERT_SOUNDS} onChange={(v) => patch({ sound: v })} />
            </SettingRow>
            <SettingRow title="OS notification" desc="A system notification as well.">
              <Switch on={os} label="OS notification" onChange={(on) => withChannel('os_notification', on)} />
            </SettingRow>
            <SettingRow
              title="Focus / DND"
              desc={session ? (d.respect_dnd ? 'Hold sound and OS notification under Focus / DND.' : 'Ignores Focus / DND; the OS may still hold the notification.') : 'Always held under Focus / DND.'}
              error={fieldError('respect_dnd', errors.respect_dnd)}
            >
              <Switch on={session ? d.respect_dnd : true} label="Respect Focus / Do Not Disturb" disabled={!session} onChange={(on) => patch({ respect_dnd: on })} />
            </SettingRow>
            <SettingRow title="Quiet hours" desc="Hold sound and OS notification during quiet hours.">
              <Switch on={d.quiet_hours} label="Obey quiet hours" onChange={(on) => patch({ quiet_hours: on })} />
            </SettingRow>
            <SettingRow title="Cooldown" desc="Per item; 0 for none." error={fieldError('cooldown_seconds', errors.cooldown_seconds)}>
              <NumberField label="Cooldown" value={d.cooldown} unit="seconds" onChange={(v) => patch({ cooldown: v })} />
            </SettingRow>
          </SettingGroup>
          {formError && (
            <Notice state="bad" testId="rule-form-error">
              {formError}
            </Notice>
          )}
          <div className="m-btn-group sg-save">
            <Button variant="primary" disabled={saving} onClick={() => void save()}>
              Save
            </Button>
            <MenuButton label="More" onClick={(e) => openMenu(e.currentTarget, [{ id: 'discard', label: 'Discard', run: () => navigate(UP) }], 'Rule')} />
          </div>
        </div>
      </MainBody>
      {folderDialog && <FolderDialog value={d.repo_root} onClose={() => setFolderDialog(false)} onSave={(v) => patch({ repo_root: v })} />}
      {leave && (
        <Dialog title="Leave without saving?" onClose={() => setLeave(null)} testId="rule-leave">
          <div className="m-card-body">This rule is not saved.</div>
          <div className="m-card-actions">
            <Button
              variant="primary"
              onClick={() => {
                setLeave(null)
                void save()
              }}
            >
              Save
            </Button>
            <Button
              onClick={() => {
                const to = leave
                setLeave(null)
                navigate(to)
              }}
            >
              Discard
            </Button>
          </div>
        </Dialog>
      )}
    </section>
  )
}
