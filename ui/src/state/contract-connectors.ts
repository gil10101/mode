// Package 10 (v0.2): accounts and the OAuth loopback (06 §3–§4), sources and
// the Slack channel picker (06 §3.4, §8.4), the Inbox (03 §4, 07 §2–§6),
// Important people (07 §5), the calendar (03 §6, 06 §7), alerts and quiet hours (07 §8)
// and the AI backends (06 §10).
//
// Reconciled with `mode_core::connectors::api` (the authoritative list of
// args and results; the app crate wraps each function as a command). Where
// the core lacks a field a view would like (sender address, contact id, important-person
// flag, urgency source, contact card, triage reason, message counts on
// accounts) the field is optional and the
// views render without it.
//
// Field names are snake_case and follow the columns of 09 §5.3–§5.5
// (`accounts`, `account_sources`, `threads`, `messages`, `contacts`,
// `calendar_events`, `snoozes`). Request arguments follow
// `mode_core::connectors::api` as it stood when this package landed
// (2026-10-22); responses are typed as the UI's rows *or* the core's
// (`Core*` below), and the stores normalize the core's shape into the UI's
// (`normalizeThread`, `normalizeDetail`, `normalizeTriage`,
// `normalizeAccount`, `normalizeChannels`, `normalizeVip`,
// `normalizeEvent`). Every `?` field is a UI-side proposal; the package
// report lists each one.
import type { AgendaRow, InboxRow, QuietChanged } from './contract'

/* ---- accounts (06 §3) ------------------------------------------------------ */

export type Provider = 'google' | 'slack'
export type Service = 'gmail' | 'calendar' | 'slack'
export type AccountStatus = 'active' | 'paused' | 'needs_reauth' | 'revoked' | 'error'
/** `health()` of 06 §2.4, derived by the core. */
export type AccountHealth = 'ok' | 'degraded' | 'rate_limited' | 'needs_reauth' | 'revoked' | 'offline' | 'error' | 'paused'

/** One `accounts` row as the Integrations cards, the accounts table and the Inbox banners read it. */
export interface AccountRow {
  id: string
  provider: Provider
  display_name: string
  email: string | null
  /** Slack user id (`U…`). */
  handle: string | null
  team_id: string | null
  team_name: string | null
  avatar_url: string | null
  /** `services_json`: what consent actually granted (06 §4.4). */
  services: Service[]
  /** `scopes_json`, verbatim. */
  scopes: string[]
  status: AccountStatus
  status_detail: string | null
  health?: AccountHealth
  color: string | null
  position: number
  connected_at: string
  last_sync_at: string | null
  /** Write-back capabilities of 06 §2.2 (read-only consent, Slack without `*:write`); the core sends `capabilities`. */
  can_write?: { mark_read: boolean; mark_unread: boolean; archive: boolean }
  /** "re-authorize within 1 day", "Calendar only — re-authorize to add Gmail", "write-back off — …" (06 §6.2, §6.10). */
  notices?: string[]
  /** Count lines of the Gmail card ("412 messages this week · 12 unread"). */
  messages_week?: number
  unread?: number
}

/** `account_add_start` / `account_reauthorise`: the browser URL and the flow's `state` (06 §4.1). */
export interface AuthStart {
  auth_url: string
  state: string
}

/** `account_add_start { provider, options }`: `ConnectOptions` of `mode_core::connectors::oauth`. */
export interface ConnectOptions {
  /** Google: the per-service toggles before consent (03 §7C.2). */
  gmail?: boolean
  calendar?: boolean
  /** Gmail with `gmail.readonly` (no write-back, 06 §6.3). */
  gmail_read_only?: boolean
  /** Slack with the `*:write` scopes for `conversations.mark` (06 §8.1). */
  slack_writeback?: boolean
  /** Re-authorize through a fresh consent (Google `login_hint`; "Re-authorize with Calendar"). */
  login_hint?: string | null
  /** Slack: which workspace app (`oauth_client:slack:<team_id>`). */
  team_id?: string | null
}

export interface AccountAddStartArgs {
  provider: Provider
  options?: ConnectOptions
}

/** `oauth_client_set`: the user's own OAuth client (06 §6.1 Google; §8.1 step 4 Slack). */
export interface OAuthClientArgs {
  provider: Provider
  client_id: string
  client_secret?: string | null
  project_id?: string | null
  team_id?: string | null
}

export type SourceKind = 'gmail_inbox' | 'gmail_label' | 'slack_dm' | 'slack_mpim' | 'slack_channel' | 'slack_mentions' | 'calendar'

/** One `account_sources` row (06 §3.4). */
export interface SourceRow {
  id: string
  account_id: string
  kind: SourceKind
  external_id: string
  /** `INBOX`, a label, `#release`, a DM partner, a calendar name. */
  name: string
  enabled: boolean
  /** Important conversation or label (07 §5). */
  vip: boolean
  /** `slack_dm`, `slack_mpim`, `slack_mentions`: listed, cannot be unwatched (ADR-0014). */
  always_on?: boolean
  poll_seconds: number | null
  last_polled_at: string | null
}

/** `slack_channels_list` row (06 §8.4). */
export interface SlackChannel {
  id: string
  /** With its `#`. */
  name: string
  is_private: boolean
  num_members: number
  watched: boolean
}

/* ---- inbox (03 §4, 07) ----------------------------------------------------- */

export type Tier = InboxRow['urgency_tier']
export type UrgencySource = 'rule' | 'ai' | 'user'
export type ThreadKind = 'email' | 'dm' | 'group_dm' | 'channel' | 'channel_thread'
/** The four sections of 03 §4.3, assigned by the core in rank order. */
export type InboxSection = 'urgent' | 'snoozed' | 'main' | 'low'
export type SnoozePreset = 'later_today' | 'tomorrow' | 'next_week' | 'pick_date' | 'until_reply'

/**
 * The Inbox filter as the UI keeps it: the chips of 03 §4.4 (all optional;
 * empty = everything not archived). `filterArgs` compiles it to the core's
 * `InboxFilter` (`CoreInboxFilter`: `topic`, `show_archived`, `from` / `to`).
 */
export interface InboxFilter {
  account_id?: string | null
  tiers?: Tier[]
  contact_id?: string | null
  /** Free text over `messages_fts` (topic, 03 §4.4) — also the toolbar search (§4.8). */
  query?: string | null
  source_id?: string | null
  unread_only?: boolean
  /** `today` · `week` · `{from, to}` on `received_at`. */
  date?: 'today' | 'week' | { from: string; to: string } | null
  archived?: boolean
}

/** One thread of the list: the `threads` row plus its latest message (03 §4.2, 01 §6.2). */
export interface ThreadRow {
  id: string
  account_id: string
  /** `accounts.display_name` (tooltip and detail panel only). */
  account_label: string | null
  /** `google` draws the Gmail mark, `slack` the Slack mark. */
  provider: Provider | 'gmail'
  kind: ThreadKind
  /** Email subject; Slack channel or DM name. */
  title: string
  channel_name: string | null
  source_id: string | null
  /** Latest message. */
  sender_name: string
  /** Not on the core's list rows (only the thread detail's messages carry it). */
  sender_address?: string | null
  sender_avatar_url: string | null
  /** Not on the core's list rows (only the thread detail's messages carry it). */
  sender_contact_id?: string | null
  /** The sender's contact is an important person (the detail panel's toggle). */
  sender_vip?: boolean
  /** The `@` glyph: DM, group DM, @mention, email addressed to the user. */
  mentioned: boolean
  last_message_at: string
  last_snippet: string | null
  message_count: number
  unread_count: number
  has_attachments: boolean
  /** Highest tier among unread messages; null when none is unread (07 §3.5). */
  urgency_tier: Tier | null
  urgency_why: string | null
  /** Not on the core's list rows (the thread detail's messages carry it). */
  urgency_source?: UrgencySource | null
  is_archived: boolean
  /** Active snooze (07 §6.1). */
  snoozed_until: string | null
  snooze_preset?: SnoozePreset | null
  /** Back from a snooze today (the "snoozed" marker, 07 §6.1). */
  resurfaced?: boolean
  /** A queued write-back the source has not confirmed (03 §4.6). */
  pending_writeback?: boolean
  source_url: string | null
  section?: InboxSection
  /** Search only: the FTS `hit` snippet (03 §4.8). */
  hit?: string | null
}

/** `mode_core::urgency::route::InboxFilter`. */
export interface CoreInboxFilter {
  section?: InboxSection | 'all' | null
  account_id?: string | null
  tiers?: Tier[] | null
  contact_id?: string | null
  topic?: string | null
  source_id?: string | null
  unread_only?: boolean
  from?: string | null
  to?: string | null
  show_archived?: boolean
}

export interface InboxListArgs {
  filter: CoreInboxFilter
  cursor?: string | null
  limit?: number
}

export interface InboxListResult {
  rows: (ThreadRow | CoreThreadRow)[]
  next_cursor?: string | null
  /** Threads per section (`SectionCounts`); seeds the sidebar's Inbox count before the first `inbox.changed`. */
  counts?: { urgent: number; snoozed: number; main: number; low: number; unscored: number }
}

export interface MessageRow {
  id: string
  thread_id: string
  sender_name: string
  sender_address: string
  sender_avatar_url: string | null
  received_at: string
  /** NULL past `retention.message_body_days` (03 §4.5). */
  body_text: string | null
  body_truncated: boolean
  snippet: string
  attachments: number
  is_read: boolean
  urgency_tier: Tier
  urgency_why: string | null
  urgency_source: UrgencySource | null
  mentioned: boolean
  source_url: string
}

/**
 * A row of the thread panel's "links" and the event panel's "linked"
 * (`mode_core::links::ItemLink`, 03 §4.1, §6.6): explicit links, then
 * mentions and (events) attendee messages.
 */
export interface ThreadLink {
  kind: 'note' | 'session' | 'message' | 'thread'
  id: string
  title: string
  /** Why it is here: `linked by you`, `mentions rate-limiter`, `from an attendee`, `prepare note`. */
  why?: string
  /** The user made it by hand. */
  by_user?: boolean
  /** A session's state, for the row glyph. */
  state?: string | null
}

/** `item_link_add` / `item_link_remove`: a thread (or one of its messages) or an event, and the target. */
export interface ItemLinkArgs {
  kind: 'thread' | 'calendar_event'
  id: string
  to_kind: 'session' | 'note' | 'message'
  to_id: string
}

/** `thread_get`: the thread view and the detail panel (03 §4.1, §4.5). */
export interface ThreadDetail {
  thread: ThreadRow
  participants: { name: string; address: string | null; contact_id: string | null }[]
  messages: MessageRow[]
  /** The sender's contact card (important-person toggle, identities); the core has no card yet: derived from the last message when absent. */
  contact?: { id: string; display_name: string; vip: boolean; vip_note: string | null; identities: string[] } | null
  /** Linked sessions, notes and messages (`mode_core::links`). */
  links?: ThreadLink[]
  /** The source this thread was seen through (`Gmail inbox`, `#release`). */
  source_name: string | null
  /** Body could not be fetched (offline, provider error): the thread shows Retry. */
  body_error?: string | null
  /** Triage that stays in Mode for this account (`mark_read`, `archive`, …; 06 §2.2, §2.5). */
  app_local_actions?: string[]
}

/** What a triage command did beyond the local change (06 §2.5): the write-back notice. */
export interface TriageResult {
  /** `queued` = will be written to Gmail / Slack; `local_only` = app-local (no capability). */
  write_back: 'queued' | 'local_only' | 'none'
  /** Why it is local only: "Gmail access is read-only", "Slack has no archive". */
  reason?: string | null
}

export interface MarkReadArgs {
  /** A message id, or a thread id with `kind: 'thread'`. */
  id: string
  kind?: 'thread' | 'message'
  /** `false` marks unread. */
  read?: boolean
}

export interface SnoozeArgs {
  id: string
  preset?: SnoozePreset
  /** UTC; `pick_date` or an explicit time. */
  until?: string
  /** "whole thread" (default) or "this message" (03 §4.7). */
  kind?: 'thread' | 'message'
  wake_on_reply?: boolean
}

export interface SnoozeResult {
  until: string
  preset?: string | null
  /** Rows created (the target plus its twins). */
  targets?: number
}

/** One important person (07 §5): a `contacts` row with `vip = 1` and its identities. */
export interface VipRow {
  id: string
  display_name: string
  /** Emails and Slack handles (`ana@acme.com`, `@sam · Acme`). */
  identities: string[]
  vip_note: string | null
  message_count: number
  last_seen_at: string | null
}

/* ---- the core's response shapes (mode_core, 2026-10-22), normalized by the stores ---- */

/** `mode_core::urgency::route::InboxThreadRow`. */
export interface CoreThreadRow {
  thread_id: string
  kind: ThreadKind
  provider: Provider
  title: string
  channel_name: string | null
  sender_name: string | null
  sender_avatar_url: string | null
  mentioned: boolean
  /** Email: the subject; Slack: the first line of the last snippet. */
  line2: string
  last_message_at: string | null
  unread_count: number
  message_count: number
  urgency_tier: Tier | null
  urgency_why: string | null
  section: InboxSection
  snoozed_until: string | null
  resurfaced: boolean
  accounts: { account_id: string; label: string; color: string | null }[]
  has_attachments: boolean
  pending_writeback: boolean
  is_archived: boolean
  source_url: string | null
}

/** `mode_core::urgency::route::ThreadMessageRow`. */
export interface CoreMessageRow {
  id: string
  sender_name: string
  sender_address: string
  sender_avatar_url: string | null
  sender_contact_id: string | null
  vip: boolean
  received_at: string
  subject: string | null
  snippet: string
  body_text: string | null
  body_truncated: boolean
  has_attachments: boolean
  is_read: boolean
  is_archived: boolean
  urgency_tier: Tier
  urgency_level: 'urgent' | 'normal' | 'low'
  urgency_why: string | null
  urgency_source: UrgencySource | null
  source_url: string
  not_synced: boolean
  snoozed_until: string | null
}

/** `mode_core::urgency::route::ThreadDetail`. */
export interface CoreThreadDetail {
  id: string
  kind: ThreadKind
  provider: Provider
  title: string
  channel_name: string | null
  participants: unknown
  account: { account_id: string; label: string; color: string | null }
  source_url: string | null
  message_count: number
  urgency_tier: Tier | null
  messages: CoreMessageRow[]
  /** `mode_core::links::ItemLink` rows; absent from a core older than the links. */
  links?: ThreadLink[]
  /** Actions that stay in Mode for this account (`mark_read`, `archive`, …). */
  app_local_actions: string[]
}

/** `mode_core::connectors::store::TriageResult`. */
export interface CoreTriageResult {
  messages: string[]
  queued: number
  app_local: boolean
  threads: string[]
  snoozes_cancelled: number
}

/** `mode_core::connectors::api::SlackChannels`. */
export interface CoreSlackChannels {
  channels: { channel_id: string; name: string; private: boolean; member_count: number | null; watched: boolean; source_id: string | null }[]
  budget_line: string
  over_budget: boolean
}

/** `mode_core::urgency::vips::VipRow`. */
export interface CoreVipRow {
  contact_id: string
  display_name: string
  vip_note: string | null
  avatar_url: string | null
  message_count: number
  identities: { kind: 'email' | 'slack_user'; value: string; scope: string }[]
}

/** `mode_core::connectors::accounts::AccountRow` adds these over the UI's row. */
export interface CoreAccountExtras {
  capabilities?: { mark_read_writeback: boolean; mark_unread_writeback: boolean; archive_writeback: boolean }
  sources?: number
}

/** `mode_core::calendar::EventRow` (attendees as Google sends them). */
export interface CoreEventRow extends Omit<CalendarEvent, 'section' | 'attendees' | 'account_name'> {
  account_label?: string
  attendees?: unknown
  synced_at?: string
}

/* ---- calendar (03 §6, 06 §7) ---------------------------------------------- */

export interface Attendee {
  name: string
  email: string | null
  response: 'accepted' | 'tentative' | 'declined' | 'needs_action'
  organizer?: boolean
  self?: boolean
}

/** `agenda_list` row: the overlay's `AgendaRow` plus what the detail panel shows (03 §6.6). */
export interface CalendarEvent extends Omit<AgendaRow, 'section'> {
  section?: AgendaRow['section']
  account_id?: string
  /** `accounts.display_name` / email: "Work · ana@acme.com". */
  account_name?: string | null
  timezone?: string | null
  organizer?: string | null
  attendees?: Attendee[]
  description?: string | null
  html_link?: string | null
  /** The `prepare_for` note when it exists (03 §6.7). */
  prepare_note_id?: string | null
  /** Linked notes and messages (§13 S1). */
  linked?: ThreadLink[]
}

/* ---- alerts, quiet hours, AI (07 §8, 06 §10) ------------------------------- */

/** `mode_core::connectors::api::AlertLogRow`. */
export interface AlertRow {
  id: string
  rule_id: string
  rule_name: string | null
  event: string
  item_key: string
  channels: string[]
  suppressed_reason: string | null
  fired_at: string | null
  created_at: string
}

/** `mode_core::connectors::api::QuietSetArgs`. */
export interface QuietSetArgs {
  /** "Pause alerts": minutes from now (0 = resume). */
  pause_minutes?: number | null
  /** `notifications.quiet_hours` (validated: HH:MM). */
  quiet_hours?: { enabled: boolean; from: string; to: string } | null
  /** The app's overlay controller (02 §6.6). */
  presentation?: boolean | null
  /** Detected Focus / DND (Windows); `null` = unknown. */
  focus?: boolean | null
  screen_locked?: boolean | null
  output_muted?: boolean | null
  os_permission?: 'granted' | 'denied' | 'unknown' | null
}

/** One rule's verdict in an `AlertOutcome` (`RuleOutcome`). */
export interface RuleOutcome {
  rule_id: string
  rule_name: string
  /** Delivered channels (`overlay` always when not dropped). */
  channels: string[]
  /** The first suppression that applied: `dedup`, `stale`, `cooldown`, `paused`, `quiet_hours`, `focus`, `presentation`, `screen_locked`, `muted`, `os_permission`, `sounds_off`. */
  suppressed_reason: string | null
}

/** `alert_rule_test { rule_id }` (`AlertOutcome`): a sample event through the pipeline; suppression reported, not applied. */
export interface AlertOutcome {
  event: string
  item_key: string
  rules: RuleOutcome[]
  sound: string | null
  notification: { identifier: string; title: string; subtitle: string; body: string[]; silent: boolean; [field: string]: unknown } | null
  /** Nothing matched: overlay only. */
  overlay_only: boolean
}

/** `trigger_json.filters` (`mode_core::urgency::alerts::Filters`, 07 §8.2): all optional, omitted = any; unknown keys are refused. */
export interface AlertRuleFilters {
  tier?: string[]
  account_id?: string
  contact_id?: string
  kind?: string[]
  waiting_kind?: string[]
  /** A path prefix. */
  repo_root?: string
  min_duration_seconds?: number
  /** 1–120. */
  minutes_before?: number
  requires_join_url?: boolean
  /** `start_of_day` or `HH:MM`. */
  at?: string
  urgent_only?: boolean
  percent?: number
  include_resurfaced?: boolean
  automation_id?: string
}

/** The alert events of 07 §8.1 (`mode_core::urgency::alerts::EVENTS`). */
export type AlertEventName = 'session.waiting' | 'session.failed' | 'session.finished' | 'message.received' | 'calendar.starting' | 'note.due' | 'usage.threshold' | 'automation.failed'

/** `alerts::EVENTS`, in the core's order (the event picker's order, 03 §8.11). */
export const ALERT_EVENTS: readonly AlertEventName[] = ['session.waiting', 'session.failed', 'session.finished', 'message.received', 'calendar.starting', 'note.due', 'usage.threshold', 'automation.failed']

export type AlertFilterKey = keyof AlertRuleFilters

/** `alerts::event_filters`: the filter keys each event reads (09 §5.5); any other key is refused (`<event> has no filter <key>`). */
export const ALERT_EVENT_FILTERS: Readonly<Record<AlertEventName, readonly AlertFilterKey[]>> = {
  'session.waiting': ['waiting_kind', 'repo_root'],
  'session.failed': ['repo_root'],
  'session.finished': ['repo_root', 'min_duration_seconds'],
  'message.received': ['tier', 'account_id', 'contact_id', 'kind', 'include_resurfaced'],
  'calendar.starting': ['minutes_before', 'requires_join_url', 'account_id'],
  'note.due': ['at', 'urgent_only'],
  'usage.threshold': ['percent'],
  'automation.failed': ['automation_id'],
}

/** The values a list filter may hold (`alerts::TIERS`, `MESSAGE_KINDS`, `WAITING_KINDS`; 07 §8.1 payloads). */
export const ALERT_TIERS = ['vip', 'direct', 'ai-urgent', 'ai-normal', 'low'] as const
export const ALERT_MESSAGE_KINDS = ['email', 'dm', 'group_dm', 'channel', 'channel_thread'] as const
export const ALERT_WAITING_KINDS = ['permission', 'question', 'idle_prompt', 'elicitation', 'agent_needs_input'] as const

/**
 * The core's bounds (`connectors::api::rule_name` / `check_cooldown`, `alerts::parse_trigger` / `check_filters`): a name of
 * 1–60 characters, a cooldown of 0–86400 s, `minutes_before` 1–120, `percent` 1–100, `min_duration_seconds` 0 or more,
 * `at` = `start_of_day` or `HH:MM`.
 */
export const ALERT_RULE_BOUNDS = {
  nameChars: 60,
  cooldownSeconds: [0, 86_400],
  minutesBefore: [1, 120],
  percent: [1, 100],
  minDurationSeconds: 0,
} as const

/** `channels_json` ⊆ these; `overlay` is always present. */
export type AlertChannel = 'overlay' | 'os_notification' | 'sound'

/**
 * `mode_core::connectors::api::AlertRuleRow`: one rule as Settings → Alerts lists it (07 §8.2), by position. The trigger
 * comes split into `event` and `filters`; `edited` is a premade rule whose definition differs from its seed (name,
 * enabled, event and filters with list filters as sets, channels as a set, sound, cooldown, `respect_dnd`,
 * `quiet_hours`; `position` ignored), so a Reset reads `false` again; custom rules are never edited.
 */
export interface AlertRuleRow {
  id: string
  name: string
  kind: 'premade' | 'custom'
  enabled: boolean
  /** One of `AlertEventName`; `unknown` when the stored trigger does not parse. */
  event: AlertEventName | 'unknown' | (string & {})
  filters: AlertRuleFilters
  channels: AlertChannel[]
  /** A cue id (02 §11.1). */
  sound: string | null
  /** Hold sound and OS notification under Focus / DND; `false` only for `session.waiting` / `session.failed`. */
  respect_dnd: boolean
  /** Obey `notifications.quiet_hours`. */
  quiet_hours: boolean
  /** 0–86400; scoped to the rule and the item (07 §8.2 Cooldown). */
  cooldown_seconds: number
  position: number
  last_fired_at: string | null
  edited: boolean
}

/**
 * `alert_rule_set {id, …}` (`AlertRuleSetArgs`): every field optional. `channels` gets `overlay` put back; `sound: ""`
 * clears the cue; `reset: true` restores a premade rule's seed first (`invalid` for a custom rule). Refusals are
 * `{code: invalid, message}` ("the cooldown is 0–86400 seconds", "only session rules may ignore Focus / Do Not Disturb")
 * or `not_found`.
 */
export interface AlertRuleSetArgs {
  id: string
  enabled?: boolean
  name?: string
  channels?: AlertChannel[]
  sound?: string
  cooldown_seconds?: number
  respect_dnd?: boolean
  quiet_hours?: boolean
  filters?: AlertRuleFilters
  position?: number
  reset?: boolean
}

/**
 * `alert_rule_add {name, event, filters, channels, sound?, respect_dnd?, quiet_hours?, cooldown_seconds?}`
 * (`AlertRuleAddArgs`): a custom rule from the event picker (03 §8.11), enabled, `position` = the last rule's + 1. The core
 * puts `overlay` first in `channels`; `sound` absent or `""` = none; `respect_dnd` and `quiet_hours` default `true`, the
 * cooldown 0. Every refusal is `{code: invalid, message}`, checked in this order: the name ("a rule name is 1–60
 * characters", `a rule named "…" exists`, compared case-insensitively), the cooldown ("the cooldown is 0–86400 seconds"),
 * the event ("unknown event …"), `minutes_before` ("minutes_before is 1–120"), `at` ("at is start_of_day or HH:MM"), a
 * key the event does not read ("<event> has no filter <keys>"), list values (`tier "…" is not one of …`),
 * "min_duration_seconds is 0 or more", "percent is 1–100", the channels ("channels are overlay, os_notification,
 * sound"), Focus ("only session rules may ignore Focus / Do Not Disturb").
 */
export interface AlertRuleAddArgs {
  name: string
  event: AlertEventName | (string & {})
  /** Only the keys `ALERT_EVENT_FILTERS[event]` names; omitted = any. */
  filters: AlertRuleFilters
  channels: AlertChannel[]
  sound?: string
  respect_dnd?: boolean
  quiet_hours?: boolean
  cooldown_seconds?: number
}

/** `alert_rule_delete {id}` → `AlertRuleDeleted`. Refusals: `premade` (a premade rule is turned off or reset, never deleted), `not_found`. */
export interface AlertRuleDeleted {
  deleted: boolean
}

/** One entry of `urgency.keyword_rules` (`mode_core::urgency::rules::KeywordRule`, 07 §3.3). */
export interface KeywordRule {
  /** Empty for a new rule: the core assigns one. */
  id: string
  /** 1–40 characters; the why line says `rule: <name>`. */
  name: string
  enabled: boolean
  field: 'subject' | 'snippet' | 'sender' | 'channel' | 'any'
  match: 'contains' | 'word' | 'regex' | 'deadline'
  /** Empty only for `deadline` (or with `labels_any`); a regex is at most 512 characters. */
  pattern: string
  effect: 'raise' | 'lower'
  /** 1–30 (deadline rules). */
  horizon_days: number
  position: number
  labels_any?: string[]
  labels_mode?: 'or' | 'and'
}

/** `urgency_diagnostics` (07 §11.6): counts only, never text. */
export interface UrgencyDiagnostics {
  counters: { batches: number; scored: number; cache_hits: number; twin_copies: number; injection_flags: number; fallbacks: Record<string, number>; tokens: number }
  queue_depth: number
  writebacks_queued: number
}

export type AiBackend = 'claude_api' | 'ollama' | 'lm_studio'
/** `usage_samples.source` of Mode's own calls: the built-ins and a remote OpenAI-compatible provider (06 §10.1A). */
export type AiSource = AiBackend | 'openai_compat'

/* ---- providers and routes (06 §10.1A, ADR-0059) ---------------------------- */

/** `ai_providers.kind`: what a provider speaks; the last three run sessions only. */
export type ProviderKind = 'claude_api' | 'openai_compat' | 'ollama' | 'lm_studio' | 'bedrock' | 'vertex' | 'foundry'
/** How the keyring secret reaches the endpoint. */
export type ProviderAuth = 'none' | 'api_key' | 'bearer'

export interface ProviderExtra {
  /** Bedrock `AWS_REGION`, Vertex `CLOUD_ML_REGION`. */
  region?: string | null
  /** Vertex project id. */
  project_id?: string | null
  /** Foundry resource name. */
  resource?: string | null
  /** `ANTHROPIC_CUSTOM_HEADERS` for a session, one `Name: value` per line. */
  custom_headers?: string | null
}

/** A provider as the core stores it (`mode_core::connectors::ai::providers::Provider`). */
export interface AiProvider {
  id: string
  kind: ProviderKind
  name: string
  /** The row's own address; null = the kind's default (a built-in local kind reads `ai.local_url`). */
  base_url: string | null
  auth: ProviderAuth
  /** The models the user named; the first is the default. */
  models: string[]
  extra: ProviderExtra
  enabled: boolean
  /** The three built-ins cannot be deleted. */
  builtin: boolean
  position: number
}

/** `ai_provider_save { draft }`; `id` null on a new provider. */
export interface AiProviderDraft {
  id: string | null
  kind: ProviderKind
  name: string
  base_url: string | null
  auth: ProviderAuth
  models: string[]
  extra: ProviderExtra
  enabled: boolean
}

/** `ai_providers_list` row (`ProviderRow`): the provider's fields flat, plus what the page shows. */
export interface AiProviderRow extends AiProvider {
  /** The address its calls go to. */
  url: string | null
  address: AddressClass
  /** A secret is stored (never the secret itself) and its last four characters. */
  key_stored: boolean
  key_last4: string | null
  /** Mode's own features can call it (a route may name it). */
  callable: boolean
  /** An agent's sessions can run on it. */
  session_capable: boolean
  /** The routes whose primary or backup it is. */
  routed: AiPurpose[]
  /** Agent profiles whose sessions run on it. */
  agents: number
}

export type AiPurpose = 'chat' | 'scoring' | 'summary'

export interface AiRouteTarget {
  provider_id: string
  /** null = the provider's default model. */
  model: string | null
}

/** `ai_routes` row: what one feature calls; `provider_id` null = the selected backend of `ai.backend`. */
export interface AiRoute {
  purpose: AiPurpose
  provider_id: string | null
  model: string | null
  /** Tried in order when the primary cannot be called; at most three. */
  fallbacks: AiRouteTarget[]
}

/** `ai_provider_kinds` row: what the add dialog offers per kind. */
export interface AiProviderKindRow {
  kind: ProviderKind
  label: string
  callable: boolean
  session_capable: boolean
  auths: ProviderAuth[]
  default_url: string | null
}

/** 06 §10.3 address classes of a local backend URL. */
export type AddressClass = 'loopback' | 'private' | 'public' | 'invalid'

/** `ai_backends_list` row (`mode_core::connectors::api::BackendRow`). */
export interface AiBackendRow {
  backend: AiBackend
  label: string
  selected: boolean
  local: boolean
  /** Claude API: the key is stored (never the key itself); null for the local backends. */
  key_stored: boolean | null
  key_last4: string | null
  url: string | null
  address: AddressClass | null
}

/** 06 §10.7 states. */
export type AiHealthState = 'ready' | 'no_key' | 'unreachable' | 'invalid_key' | 'budget_reached' | 'local_only_blocked'

/** `ai_health { probe? }` / `ai_backend_set` (`mode_core::connectors::ai::AiHealth`). */
export interface AiHealth {
  backend: AiSource
  /** The provider scoring goes to (`ai_providers.id`) and its name. */
  provider: string
  label: string
  local_only: boolean
  state: AiHealthState
  message: string
  /** `loopback` | `private` for a local backend (the LAN notice). */
  address: AddressClass | null
  models: string[]
  tokens_used_month: number
  budget_tokens_month: number
  /** Backups behind the primary on the scoring route. */
  backups: number
}

/** `ai_backend_set` (`AiBackendSetArgs`): every field optional; `local_url` must be loopback or private. */
export interface AiBackendSetArgs {
  backend?: AiBackend
  local_url?: string
  local_only?: boolean
  model_scoring?: string
  model_summary?: string
  model_chat?: string
  local_model_scoring?: string
  local_model_summary?: string
  local_model_chat?: string
}

/** `mode_core::keyring::SecretStatus`: only "stored" and, for an API key, its last four characters. */
export interface CoreSecretStatus {
  name: string
  stored: boolean
  last4?: string | null
}

/** `connectors_status`: one card line per service (06 §9A). */
export interface ConnectorCardStatus {
  state: 'connected' | 'not_connected' | 'needs_sign_in' | 'error' | 'offline' | 'rate_limited'
  glyph: string
  line: string
  accounts: number
}

/** One account's health with its Diagnostics line (`HealthReport`). */
export interface AccountHealthReport {
  account_id: string
  display_name: string
  provider: Provider
  status: AccountStatus
  health: AccountHealth
  line: string
  last_sync_at: string | null
  budget_used: number
  budget_cap: number
}

/** `connectors_status` (`mode_core::connectors::api::ConnectorsStatus`). */
export interface ConnectorsStatus {
  shipped: boolean
  offline: boolean
  gmail: ConnectorCardStatus
  calendar: ConnectorCardStatus
  slack: ConnectorCardStatus
  accounts: AccountHealthReport[]
}

/** `claude_code_card { binary_found }` (`mode_core::connectors::claude_code::ClaudeCodeCard`, 06 §9). */
export interface ClaudeCodeCard {
  state: 'connected' | 'not_connected' | 'needs_attention'
  glyph: string
  line: string
  /** The latest install's status, or `none`. */
  hooks: string
  sessions_tracked: number
  sessions_today: number
}

/* ---- the command table -------------------------------------------------------- */

export interface ConnectorsCommands {
  accounts_list: { args: undefined; result: (AccountRow & CoreAccountExtras)[] }
  account_add_start: { args: AccountAddStartArgs; result: AuthStart & { redirect_uri?: string } }
  /** Resolves when the loopback completes (06 §4.1); rejects on `access_denied`, timeout or a failed exchange. */
  account_add_finish: { args: { state: string }; result: AccountRow & CoreAccountExtras }
  /** Cancel in the dialog closes the listener at once (06 §4.3). */
  account_add_cancel: { args: { state: string }; result: boolean }
  account_pause: { args: { id: string }; result: AccountRow & CoreAccountExtras }
  account_resume: { args: { id: string }; result: AccountRow & CoreAccountExtras }
  /** Retry after `error` (03 §4.9, §7C.3). */
  account_retry: { args: { id: string }; result: AccountRow & CoreAccountExtras }
  /** Runs §4 again with the account's own services; the UI opens `auth_url` and waits on `account_add_finish`. */
  account_reauthorise: { args: { id: string }; result: AuthStart }
  account_remove: { args: { id: string }; result: unknown }
  /** Stored in the keyring as `oauth_client:google` / `oauth_client:slack[:<team_id>]`; only "stored" comes back. */
  oauth_client_set: { args: OAuthClientArgs; result: CoreSecretStatus }
  /** The manifest (YAML) to paste at api.slack.com (06 §8.1); without the `*:write` scopes when `writeback` is false. */
  slack_manifest: { args: { writeback?: boolean } | undefined; result: string }
  connectors_status: { args: undefined; result: ConnectorsStatus }
  claude_code_card: { args: { binary_found: boolean }; result: ClaudeCodeCard }
  sources_list: { args: { account_id?: string } | undefined; result: SourceRow[] }
  source_set: { args: { id: string; enabled?: boolean; vip?: boolean }; result: SourceRow }
  slack_channels_list: { args: { account_id: string; refresh?: boolean }; result: SlackChannel[] | CoreSlackChannels }
  slack_channels_set: { args: { account_id: string; channel_ids: string[] }; result: SourceRow[] }
  inbox_list: { args: InboxListArgs; result: InboxListResult }
  /** A thread id; `inbox/message/<id>` sends the message id and uses the answer's thread once the core resolves message ids (else `not_found`). */
  thread_get: { args: { thread_id: string }; result: ThreadDetail | CoreThreadDetail }
  message_mark_read: { args: MarkReadArgs; result: TriageResult | CoreTriageResult }
  thread_archive: { args: { id: string; kind?: 'thread' | 'message'; archived?: boolean }; result: TriageResult | CoreTriageResult }
  thread_snooze: { args: SnoozeArgs; result: SnoozeResult }
  thread_unsnooze: { args: { id: string; kind?: 'thread' | 'message' }; result: unknown }
  /** The level override (03 §4.5, 07 §3.5): `urgency_source = 'user'`; `id` is the message the level is read from. */
  message_set_tier: { args: { id: string; tier: Tier }; result: unknown }
  open_in_source: { args: { id: string }; result: string | { url: string; fallback_url: string | null } }
  vips_list: { args: undefined; result: (VipRow | CoreVipRow)[] }
  vip_add: { args: { contact_id?: string; email?: string; slack_user?: string; display_name?: string; vip_note?: string | null }; result: VipRow | { contact_id: string; retiered: number } }
  vip_remove: { args: { contact_id: string }; result: unknown }
  agenda_list: { args: { from: string; to: string }; result: (CalendarEvent | CoreEventRow)[] }
  event_join: { args: { id: string }; result: string }
  event_prepare_notes: { args: { id: string }; result: { note_id: string } }
  /** The event panel's linked sessions, notes and messages (03 §6.6). */
  event_links: { args: { id: string }; result: ThreadLink[] }
  /** Link by hand; returns the anchor's links. */
  item_link_add: { args: ItemLinkArgs; result: ThreadLink[] }
  /** Remove a link, or dismiss a suggestion; returns the anchor's links. */
  item_link_remove: { args: ItemLinkArgs; result: ThreadLink[] }
  /** The newest `alert_log` rows (`limit` 50, 1–500). */
  alerts_recent: { args: { limit?: number } | undefined; result: AlertRow[] }
  /** Withdraws the OS notification(s) of an item; returns the identifiers withdrawn. */
  alert_dismiss: { args: { item_key?: string; target_kind?: string; target_id?: string }; result: string[] }
  alert_rule_test: { args: { rule_id: string }; result: AlertOutcome }
  /** The live rules by position (the premade ones upserted by name at every start). */
  alert_rules_list: { args: undefined; result: AlertRuleRow[] }
  /** One rule's edit; answers the stored row. */
  alert_rule_set: { args: AlertRuleSetArgs; result: AlertRuleRow }
  /** A custom rule (03 §8.11); answers the stored row (`kind: custom`, enabled, placed last). */
  alert_rule_add: { args: AlertRuleAddArgs; result: AlertRuleRow }
  /** Tombstones a custom rule (its `alert_log` rows stay). */
  alert_rule_delete: { args: { id: string }; result: AlertRuleDeleted }
  /** Returns the merged `quiet.changed` payload and pushes it. */
  quiet_set: { args: QuietSetArgs; result: QuietChanged }
  /** The rules (the three premade ones seeded, disabled, on the first call). */
  keyword_rules_list: { args: undefined; result: KeywordRule[] }
  /** Validated (name ≤ 40, regex ≤ 512 and compiling, horizon 1–30); new rules get an id; positions follow the list. */
  keyword_rules_set: { args: { rules: KeywordRule[] }; result: KeywordRule[] }
  /** Re-score the last 24 h (07 §3.4.8); the messages queued. */
  urgency_rescore: { args: undefined; result: number }
  urgency_diagnostics: { args: undefined; result: UrgencyDiagnostics }
  ai_backends_list: { args: undefined; result: AiBackendRow[] }
  ai_backend_set: { args: AiBackendSetArgs; result: AiHealth }
  /** `claude_api` (the Anthropic key) or a local backend (its optional bearer token); `key: null` removes it. */
  ai_key_set: { args: { backend: AiBackend; key: string | null }; result: CoreSecretStatus }
  /** `probe: true` also calls `GET /v1/models` on the Claude API (no tokens billed). */
  ai_health: { args: { probe?: boolean } | undefined; result: AiHealth }
  /** The provider registry (06 §10.1A): built-ins first, then the user's. */
  ai_providers_list: { args: undefined; result: AiProviderRow[] }
  /** Rejects with `{code: 'invalid', fields}` naming the draft's fields. */
  ai_provider_save: { args: { draft: AiProviderDraft }; result: AiProviderRow }
  /** A built-in is refused; the routes and agents that named it fall back. */
  ai_provider_delete: { args: { id: string }; result: boolean }
  ai_provider_key_set: { args: { id: string; key: string | null }; result: CoreSecretStatus }
  ai_provider_test: { args: { id: string; model?: string | null }; result: { ok: boolean; latency_ms: number | null; error?: string | null } }
  /** The models the endpoint lists, the named ones first. */
  ai_provider_models: { args: { id: string }; result: string[] }
  ai_provider_kinds: { args: undefined; result: AiProviderKindRow[] }
  ai_routes_list: { args: undefined; result: AiRoute[] }
  ai_route_set: { args: { route: AiRoute }; result: AiRoute }
}
