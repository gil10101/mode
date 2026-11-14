// The custom alert rule editor's model (03 §8.11; 07 §8.1–§8.2): the draft
// the form edits, the event picker's words, the per-event filter fields (only
// the keys `alerts::event_filters` names for the event), the client-side
// checks with the core's bounds (`ALERT_RULE_BOUNDS`), the `alert_rule_add`
// arguments, and the mapping of the core's `invalid` refusals onto the field
// they name (anything else is the form's one error line).
import { toCmdError } from '../../state/contract'
import {
  ALERT_EVENT_FILTERS,
  ALERT_MESSAGE_KINDS,
  ALERT_RULE_BOUNDS,
  ALERT_TIERS,
  ALERT_WAITING_KINDS,
  type AlertChannel,
  type AlertEventName,
  type AlertFilterKey,
  type AlertRuleAddArgs,
  type AlertRuleFilters,
} from '../../state/contract-connectors'

/** The cues of 02 §11.1 a rule may name (`captured` is quick capture's, not a rule's). */
export const ALERT_SOUNDS: readonly (readonly [string, string])[] = [
  ['', 'None'],
  ['waiting', 'Waiting'],
  ['urgent', 'Urgent'],
  ['finished', 'Finished'],
  ['failed', 'Failed'],
  ['starting', 'Starting'],
]

/** 07 §8.2: only `session.waiting` and `session.failed` rules may ignore Focus / DND. */
export function mayIgnoreDnd(event: string): boolean {
  return event === 'session.waiting' || event === 'session.failed'
}

/** `overlay` first and always there, then 07 §8.2's order; unknown names dropped. */
export function channelList(channels: readonly string[]): AlertChannel[] {
  return (['overlay', 'os_notification', 'sound'] as const).filter((c) => c === 'overlay' || channels.includes(c))
}

/**
 * The event picker's words: 07 §8.2's premade rule for each event, with the filter value its name carries taken out
 * ("Urgent message" is any message here, "Due today" any due note, "AI budget at 80 %" any threshold).
 */
export const EVENT_LABEL: Readonly<Record<AlertEventName, string>> = {
  'session.waiting': 'Session needs you',
  'session.failed': 'Session failed',
  'session.finished': 'Session finished',
  'message.received': 'Message received',
  'calendar.starting': 'Meeting starting',
  'note.due': 'Notes due',
  'usage.threshold': 'AI budget reached',
  'automation.failed': 'Automation failed',
}

/** The cue each event's premade rule names (07 §8.2); a new rule starts with it. */
export const EVENT_CUE: Readonly<Record<AlertEventName, string>> = {
  'session.waiting': 'waiting',
  'session.failed': 'failed',
  'session.finished': 'finished',
  'message.received': 'urgent',
  'calendar.starting': 'starting',
  'note.due': '',
  'usage.threshold': '',
  'automation.failed': '',
}

/** Chip words of the list filters (07 §8.1 payload values). */
export const WAITING_WORD: Readonly<Record<string, string>> = {
  permission: 'permission',
  question: 'question',
  idle_prompt: 'idle prompt',
  elicitation: 'elicitation',
  agent_needs_input: 'agent needs input',
}
export const TIER_WORD: Readonly<Record<string, string>> = { vip: 'important', direct: 'direct', 'ai-urgent': 'urgent', 'ai-normal': 'normal', low: 'low' }
export const KIND_WORD: Readonly<Record<string, string>> = { email: 'email', dm: 'DM', group_dm: 'group DM', channel: 'channel', channel_thread: 'thread' }

/** What the form edits; numbers stay text until Save so a half-typed value can be shown with its error. */
export interface RuleDraft {
  name: string
  event: AlertEventName
  /** List filters: empty = any. */
  waiting_kind: string[]
  tier: string[]
  kind: string[]
  /** A path prefix; empty = any repo. */
  repo_root: string
  account_id: string | null
  contact_id: string | null
  include_resurfaced: boolean
  /** `session.finished`: whole minutes, stored as `min_duration_seconds`; empty = any length. */
  min_minutes: string
  /** `calendar.starting`: 1–120. */
  minutes_before: string
  requires_join_url: boolean
  /** `note.due`: the start of the day or a fixed time. */
  at: 'start_of_day' | 'time'
  at_time: string
  urgent_only: boolean
  /** `usage.threshold`: 1–100; empty = any threshold. */
  percent: string
  automation_id: string | null
  channels: AlertChannel[]
  /** A cue id; empty = none. */
  sound: string
  respect_dnd: boolean
  quiet_hours: boolean
  cooldown: string
}

/** A new rule as `alert_rule_add` defaults it: overlay only, Focus / DND and quiet hours respected, no cooldown. */
export function newDraft(event: AlertEventName = 'session.waiting'): RuleDraft {
  return {
    name: '',
    event,
    waiting_kind: [],
    tier: [],
    kind: [],
    repo_root: '',
    account_id: null,
    contact_id: null,
    include_resurfaced: false,
    min_minutes: '',
    minutes_before: '5',
    requires_join_url: false,
    at: 'start_of_day',
    at_time: '09:00',
    urgent_only: false,
    percent: '80',
    automation_id: null,
    channels: ['overlay'],
    sound: EVENT_CUE[event],
    respect_dnd: true,
    quiet_hours: true,
    cooldown: '0',
  }
}

/** Another event: its cue follows unless the user picked one; Focus / DND is respected again where it must be. */
export function withEvent(d: RuleDraft, event: AlertEventName): RuleDraft {
  return {
    ...d,
    event,
    sound: d.sound === EVENT_CUE[d.event] ? EVENT_CUE[event] : d.sound,
    respect_dnd: mayIgnoreDnd(event) ? d.respect_dnd : true,
  }
}

/** A field the form shows an error under. */
export type RuleField = 'name' | 'event' | AlertFilterKey | 'channels' | 'sound' | 'respect_dnd' | 'quiet_hours' | 'cooldown_seconds'
export type RuleErrors = Partial<Record<RuleField, string>>

/** The fields the form shows for an event (the filter rows between the event picker and the channels). */
export function visibleFields(event: AlertEventName): RuleField[] {
  return ['name', 'event', ...ALERT_EVENT_FILTERS[event], 'channels', 'sound', 'respect_dnd', 'quiet_hours', 'cooldown_seconds']
}

/** `connectors::api::name_taken` compares with SQLite's `lower()`, which folds ASCII only. */
export function ruleNameKey(name: string): string {
  return name.trim().replace(/[A-Z]/g, (c) => c.toLowerCase())
}

/** A whole number as typed ("1 200" and "1,200" too); null when it is not one. */
export function wholeNumber(text: string): number | null {
  const t = text.trim().replace(/[\s,_]/g, '')
  return /^-?\d+$/.test(t) ? Number(t) : null
}

/** `alerts::parse_hm` (`H:MM` or `HH:MM`, hours < 24, minutes < 60), written back as `HH:MM`; null otherwise. */
export function clockTime(text: string): string | null {
  const m = /^\s*(\d{1,2}):(\d{2})\s*$/.exec(text)
  if (!m) return null
  const h = Number(m[1])
  const min = Number(m[2])
  return h < 24 && min < 60 ? `${String(h).padStart(2, '0')}:${String(min).padStart(2, '0')}` : null
}

/** The values of `chosen` that `known` lists, in the core's order (the list filters are compared as sets). */
function inOrder(known: readonly string[], chosen: readonly string[]): string[] {
  return known.filter((v) => chosen.includes(v))
}

/**
 * The `alert_rule_add` arguments for a draft and what the core would refuse in it, checked here with the core's bounds
 * so nothing out of range is sent: a name of 1–60 characters not used by another rule (`taken`: the live rules' names),
 * a cooldown of 0–86400 s, and only the filters the event reads (`ALERT_EVENT_FILTERS`), within range.
 */
export function buildRule(d: RuleDraft, taken: readonly string[]): { args: AlertRuleAddArgs; errors: RuleErrors } {
  const errors: RuleErrors = {}
  const B = ALERT_RULE_BOUNDS
  const name = d.name.trim()
  if (!name || Array.from(name).length > B.nameChars) errors.name = `a rule name is 1–${B.nameChars} characters`
  else if (taken.some((t) => ruleNameKey(t) === ruleNameKey(name))) errors.name = `a rule named ${JSON.stringify(name)} exists`

  const cooldown = wholeNumber(d.cooldown)
  if (cooldown === null) errors.cooldown_seconds = 'the cooldown is a whole number of seconds'
  else if (cooldown < B.cooldownSeconds[0] || cooldown > B.cooldownSeconds[1]) errors.cooldown_seconds = `the cooldown is ${B.cooldownSeconds[0]}–${B.cooldownSeconds[1]} seconds`

  const filters: AlertRuleFilters = {}
  for (const key of ALERT_EVENT_FILTERS[d.event]) {
    switch (key) {
      case 'waiting_kind':
        if (d.waiting_kind.length) filters.waiting_kind = inOrder(ALERT_WAITING_KINDS, d.waiting_kind)
        break
      case 'tier':
        if (d.tier.length) filters.tier = inOrder(ALERT_TIERS, d.tier)
        break
      case 'kind':
        if (d.kind.length) filters.kind = inOrder(ALERT_MESSAGE_KINDS, d.kind)
        break
      case 'repo_root':
        if (d.repo_root.trim()) filters.repo_root = d.repo_root.trim()
        break
      case 'account_id':
        if (d.account_id) filters.account_id = d.account_id
        break
      case 'contact_id':
        if (d.contact_id) filters.contact_id = d.contact_id
        break
      case 'include_resurfaced':
        if (d.include_resurfaced) filters.include_resurfaced = true
        break
      case 'min_duration_seconds': {
        if (!d.min_minutes.trim()) break
        const m = wholeNumber(d.min_minutes)
        if (m === null || m * 60 < B.minDurationSeconds) errors.min_duration_seconds = 'a whole number of minutes, 0 or more'
        else filters.min_duration_seconds = m * 60
        break
      }
      case 'minutes_before': {
        const m = wholeNumber(d.minutes_before)
        if (m === null || m < B.minutesBefore[0] || m > B.minutesBefore[1]) errors.minutes_before = `${B.minutesBefore[0]}–${B.minutesBefore[1]} minutes before the start`
        else filters.minutes_before = m
        break
      }
      case 'requires_join_url':
        if (d.requires_join_url) filters.requires_join_url = true
        break
      case 'at': {
        if (d.at === 'start_of_day') filters.at = 'start_of_day'
        else {
          const t = clockTime(d.at_time)
          if (t === null) errors.at = 'a time as HH:MM, 00:00–23:59'
          else filters.at = t
        }
        break
      }
      case 'urgent_only':
        if (d.urgent_only) filters.urgent_only = true
        break
      case 'percent': {
        if (!d.percent.trim()) break
        const p = wholeNumber(d.percent)
        if (p === null || p < B.percent[0] || p > B.percent[1]) errors.percent = `${B.percent[0]}–${B.percent[1]} % of the budget`
        else filters.percent = p
        break
      }
      case 'automation_id':
        if (d.automation_id) filters.automation_id = d.automation_id
        break
    }
  }

  const args: AlertRuleAddArgs = {
    name,
    event: d.event,
    filters,
    channels: channelList(d.channels),
    respect_dnd: mayIgnoreDnd(d.event) ? d.respect_dnd : true,
    quiet_hours: d.quiet_hours,
    cooldown_seconds: cooldown ?? 0,
  }
  if (d.sound.trim()) args.sound = d.sound.trim()
  return { args, errors }
}

/** The core's `invalid` messages (`connectors::api`, `alerts::parse_trigger` / `check_filters` / `validate_rule`) by the field they are about. */
const REFUSAL_FIELD: readonly (readonly [RegExp, RuleField])[] = [
  [/^a rule name(d)? /, 'name'],
  [/^the cooldown /, 'cooldown_seconds'],
  [/^unknown event /, 'event'],
  [/^minutes_before /, 'minutes_before'],
  [/^at is /, 'at'],
  [/^waiting_kind /, 'waiting_kind'],
  [/^tier /, 'tier'],
  [/^kind /, 'kind'],
  [/^min_duration_seconds /, 'min_duration_seconds'],
  [/^percent /, 'percent'],
  [/^channels are |^the overlay channel /, 'channels'],
  [/ Focus \/ Do Not Disturb$/, 'respect_dnd'],
]

/** A refusal of `alert_rule_add`: an `invalid` message that names a field goes under it; the rest (`<event> has no filter …`, other codes) is the form's line. */
export function refusalField(e: unknown): { field: RuleField | null; message: string } {
  const err = toCmdError(e)
  if (err.code === 'invalid') {
    for (const [re, field] of REFUSAL_FIELD) if (re.test(err.message)) return { field, message: err.message }
  }
  return { field: null, message: err.message }
}
