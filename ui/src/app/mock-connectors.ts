// The package 10 commands in the browser mock (outside Tauri): accounts with
// one paused (Personal) and one broken (Acme Slack, needs sign-in), their
// sources and the Slack channel list, the Inbox threads of
// docs/wireframes/app-window.html (Sam in #release and Priya by email in
// Urgent, Tom's DM, #docs, two Gmail threads, #alerts in the main list, one
// snoozed thread, four low-priority ones), important people, the calendar of the
// wireframe's week and month (the overlapping Standup / Sprint review and a
// declined call sit in the following week so the drawn week stays as drawn),
// and an OAuth loopback that finishes one second after `account_add_start`
// (or, set with `authNext`, refuses, never answers until canceled, or opens
// no browser).
// Data is relative to the install time (today at 09:41 …), so at the
// wireframes' clock (Wed 21 Oct 2026, 14:02) it is the wireframes' data.
// Mutations are applied to the mock's own state and pushed back as
// `inbox.changed` / `agenda.changed`, the way the core would. Test scaffolding only.
import { mock } from '../tauri'
import { EVENTS, type AgendaRow, type InboxRow } from '../state/contract'
import type {
  AccountRow,
  AlertRow,
  CalendarEvent,
  CoreInboxFilter,
  InboxFilter,
  MessageRow,
  SlackChannel,
  SnoozePreset,
  SourceRow,
  ThreadDetail,
  ThreadLink,
  ThreadRow,
  Tier,
  TriageResult,
  VipRow,
} from '../state/contract-connectors'
import type { AccountHealthReport, ConnectorCardStatus, ConnectorsStatus } from '../state/contract-connectors'
import { URGENT_TIERS, resolvePreset } from '../state/inbox-store'
import { slackManifest } from './integrations/accounts-model'
import { addDays, dateStart, localDate, weekStartOf } from '../state/calendar-store'
import { sampleHistory, sampleRoster } from './mock-data'

type Args = Record<string, unknown> | undefined

interface Thread {
  row: ThreadRow
  messages: MessageRow[]
  links: ThreadLink[]
  /** Removed links (`link_dismissals`): kept out of the merged `note_link_add` links. */
  dismissed?: Set<string>
  /** The tier the engine gave the thread before any important-person / user change. */
  base: Tier
  baseWhy: string | null
  baseSource: ThreadRow['urgency_source']
}

interface Contact {
  id: string
  display_name: string
  vip: boolean
  vip_note: string | null
  identities: string[]
  message_count: number
  last_seen_at: string | null
}

interface Pending {
  provider: 'google' | 'slack'
  accountId?: string
  services?: ('gmail' | 'calendar')[]
  readOnly?: boolean
  outcome?: AuthOutcome
  /** Rejects a waiting `account_add_finish` (`account_add_cancel`). */
  cancel?: () => void
}

interface State {
  now0: number
  accounts: AccountRow[]
  sources: SourceRow[]
  channels: Record<string, SlackChannel[]>
  threads: Thread[]
  contacts: Contact[]
  events: CalendarEvent[]
  alerts: AlertRow[]
  pending: Map<string, Pending>
  seq: number
  offline: boolean
  fail: Map<string, string>
  quiet: { quiet_hours: boolean; focus: boolean; presentation: boolean }
  oauthClients: Set<string>
  capture?: (text: string) => string
  /** How the next Connect run ends (`authNext`); once, then back to `connect`. */
  auth: AuthOutcome
}

/**
 * How the mock's next Connect run ends: `connect` (the loopback answers one second later), `fail`
 * (`account_add_finish` refuses with `reason`), `wait` (no answer until `account_add_cancel`) and
 * `open-fail` (the browser cannot be opened: `plugin:opener|open_url` refuses, then as `wait`).
 */
export type AuthOutcome = { kind: 'connect' } | { kind: 'fail'; reason: string } | { kind: 'wait' } | { kind: 'open-fail' }

let st: State | null = null

const RANK: Record<Tier, number> = { vip: 0, direct: 1, 'ai-urgent': 2, 'ai-normal': 3, unscored: 3, low: 4 }

const iso = (ms: number) => new Date(ms).toISOString()

/** Local time `hh:mm` on today + `day`. */
function atDay(now: number, day: number, hhmm: string): string {
  const [h, m] = hhmm.split(':').map(Number)
  const d = dateStart(addDays(localDate(now), day))
  d.setHours(h!, m!, 0, 0)
  return iso(d.getTime())
}

/* ---- accounts and sources ------------------------------------------------------------ */

const READ_ONLY_GOOGLE = ['openid', 'https://www.googleapis.com/auth/userinfo.email', 'https://www.googleapis.com/auth/gmail.readonly', 'https://www.googleapis.com/auth/calendar.readonly']
const SLACK_SCOPES = ['channels:history', 'groups:history', 'im:history', 'mpim:history', 'channels:read', 'groups:read', 'im:read', 'mpim:read', 'users:read', 'search:read', 'channels:write', 'groups:write', 'im:write', 'mpim:write']

function sampleAccounts(now: number): AccountRow[] {
  return [
    {
      id: 'acc-work',
      provider: 'google',
      display_name: 'Work',
      email: 'ana@acme.com',
      handle: null,
      team_id: null,
      team_name: null,
      avatar_url: null,
      services: ['gmail', 'calendar'],
      scopes: READ_ONLY_GOOGLE,
      status: 'active',
      status_detail: null,
      health: 'ok',
      color: null,
      position: 0,
      connected_at: iso(now - 4 * 86_400_000),
      last_sync_at: iso(now - 120_000),
      can_write: { mark_read: false, mark_unread: false, archive: false },
      messages_week: 300,
      unread: 9,
    },
    {
      id: 'acc-personal',
      provider: 'google',
      display_name: 'Personal',
      email: 'ana.ruiz@example.com',
      handle: null,
      team_id: null,
      team_name: null,
      avatar_url: null,
      services: ['gmail', 'calendar'],
      scopes: READ_ONLY_GOOGLE,
      status: 'paused',
      status_detail: null,
      health: 'paused',
      color: null,
      position: 1,
      connected_at: iso(now - 30 * 86_400_000),
      last_sync_at: iso(now - 3 * 3_600_000),
      can_write: { mark_read: false, mark_unread: false, archive: false },
      messages_week: 112,
      unread: 3,
    },
    {
      id: 'acc-acme',
      provider: 'slack',
      display_name: 'Acme Slack',
      email: null,
      handle: 'U02ANA',
      team_id: 'T0ACME',
      team_name: 'Acme',
      avatar_url: null,
      services: ['slack'],
      scopes: SLACK_SCOPES,
      status: 'needs_reauth',
      status_detail: 'token expired yesterday',
      health: 'needs_reauth',
      color: null,
      position: 2,
      connected_at: iso(now - 40 * 86_400_000),
      last_sync_at: iso(now - 26 * 3_600_000),
      can_write: { mark_read: true, mark_unread: false, archive: false },
      messages_week: 86,
      unread: 1,
    },
  ]
}

function src(id: string, account_id: string, kind: SourceRow['kind'], external_id: string, name: string, enabled: boolean, polled: string | null, extra: Partial<SourceRow> = {}): SourceRow {
  return { id, account_id, kind, external_id, name, enabled, vip: false, poll_seconds: null, last_polled_at: polled, ...extra }
}

function sampleSources(now: number): SourceRow[] {
  const ago = (ms: number) => iso(now - ms)
  return [
    src('s-work-inbox', 'acc-work', 'gmail_inbox', 'INBOX', 'Gmail inbox', true, ago(120_000)),
    src('s-work-clients', 'acc-work', 'gmail_label', 'Label_12', 'Clients', true, ago(120_000)),
    src('s-work-cal', 'acc-work', 'calendar', 'primary', 'Work', true, ago(300_000)),
    src('s-work-holidays', 'acc-work', 'calendar', 'en.usa#holiday', 'Holidays', false, null),
    src('s-pers-inbox', 'acc-personal', 'gmail_inbox', 'INBOX', 'Gmail inbox', true, ago(3 * 3_600_000)),
    src('s-pers-cal', 'acc-personal', 'calendar', 'primary', 'Personal', true, ago(3 * 3_600_000)),
    src('s-acme-dm-tom', 'acc-acme', 'slack_dm', 'D01TOM', 'Tom Keller', true, ago(26 * 3_600_000), { always_on: true }),
    src('s-acme-dm-sam', 'acc-acme', 'slack_dm', 'D02SAM', 'Sam Okafor', true, ago(26 * 3_600_000), { always_on: true }),
    src('s-acme-mpim', 'acc-acme', 'slack_mpim', 'G01LT', 'Lena, Tom', true, ago(26 * 3_600_000), { always_on: true }),
    src('s-acme-mentions', 'acc-acme', 'slack_mentions', '*', 'Mentions', true, ago(26 * 3_600_000), { always_on: true }),
    src('s-acme-release', 'acc-acme', 'slack_channel', 'C01REL', '#release', true, ago(26 * 3_600_000)),
    src('s-acme-docs', 'acc-acme', 'slack_channel', 'C02DOC', '#docs', true, ago(26 * 3_600_000)),
    src('s-acme-alerts', 'acc-acme', 'slack_channel', 'C03ALR', '#alerts', true, ago(26 * 3_600_000)),
  ]
}

function sampleChannels(): Record<string, SlackChannel[]> {
  return {
    'acc-acme': [
      { id: 'C03ALR', name: '#alerts', is_private: false, num_members: 41, watched: true },
      { id: 'C07DES', name: '#design', is_private: true, num_members: 9, watched: false },
      { id: 'C02DOC', name: '#docs', is_private: false, num_members: 23, watched: true },
      { id: 'C08ENG', name: '#eng-platform', is_private: false, num_members: 57, watched: false },
      { id: 'C04GEN', name: '#general', is_private: false, num_members: 212, watched: false },
      { id: 'C06INC', name: '#incidents', is_private: false, num_members: 64, watched: false },
      { id: 'C05RND', name: '#random', is_private: false, num_members: 198, watched: false },
      { id: 'C01REL', name: '#release', is_private: false, num_members: 18, watched: true },
    ],
  }
}

/* ---- the Inbox --------------------------------------------------------------------------- */

const CONTACTS: Contact[] = [
  { id: 'c-sam', display_name: 'Sam Okafor', vip: true, vip_note: 'your manager', identities: ['sam@acme.com', '@sam · Acme'], message_count: 214, last_seen_at: null },
  { id: 'c-priya', display_name: 'Priya Nair', vip: false, vip_note: null, identities: ['priya@acme.com'], message_count: 96, last_seen_at: null },
  { id: 'c-tom', display_name: 'Tom Keller', vip: false, vip_note: null, identities: ['@tom · Acme'], message_count: 141, last_seen_at: null },
  { id: 'c-lena', display_name: 'Lena Vogt', vip: false, vip_note: null, identities: ['lena@acme.com', '@lena · Acme'], message_count: 77, last_seen_at: null },
  { id: 'c-nina', display_name: 'Nina Brandt', vip: true, vip_note: 'client, Globex', identities: ['nina@globex.example'], message_count: 12, last_seen_at: null },
  { id: 'c-marta', display_name: 'Marta Silva', vip: false, vip_note: null, identities: ['marta@acme.com'], message_count: 30, last_seen_at: null },
]

interface ThreadSpec {
  id: string
  account: string
  provider: 'google' | 'slack'
  kind: ThreadRow['kind']
  title: string
  channel?: string
  source: string
  sender: string
  address: string
  contact?: string
  mentioned?: boolean
  tier: Tier
  why: string | null
  whySource?: ThreadRow['urgency_source']
  unread: boolean
  snippet: string
  messages: { at: string; text: string; sender?: string; address?: string; contact?: string; tier?: Tier; why?: string | null; attachments?: number }[]
  links?: ThreadLink[]
  snoozedUntil?: string
  archived?: boolean
  url: string
}

function sampleThreads(now: number): Thread[] {
  const t = (day: number, hhmm: string) => atDay(now, day, hhmm)
  const gmail = (id: string) => `https://mail.google.com/mail/?authuser=ana@acme.com#all/${id}`
  const slack = (ch: string, ts: string) => `slack://channel?team=T0ACME&id=${ch}&message=${ts}`
  const specs: ThreadSpec[] = [
    {
      id: 't-sam',
      account: 'acc-acme',
      provider: 'slack',
      kind: 'channel_thread',
      title: '#release',
      channel: '#release',
      source: 's-acme-release',
      sender: 'Sam',
      address: 'U03SAM',
      contact: 'c-sam',
      mentioned: true,
      tier: 'vip',
      why: 'Important person: Sam Okafor (your manager)',
      unread: true,
      // A Slack thread: the row shows its root message; the reply is in the thread view.
      snippet: 'can we ship v2 behind the flag today? need your sign-off before the 3 pm train',
      messages: [
        { at: t(0, '09:41'), text: "can we ship v2 behind the flag today? need your sign-off before the 3 pm train. Lena's migration guide is in PR 412 and the rate limiter is green on main." },
        { at: t(0, '09:48'), text: '@you the flag is `v2_api` in config/flags.yml; default off, so nothing changes for customers until we flip it.' },
      ],
      // What `mode_core::links` finds: the message names the session ("the rate limiter"); the note was linked by hand.
      links: [
        { kind: 'session', id: 's-rate', title: 'rate-limiter', state: 'done', why: 'mentions rate-limiter', by_user: false },
        { kind: 'note', id: 'n-changelog', title: 'Draft v2 changelog', why: 'linked by you', by_user: true },
      ],
      url: slack('C01REL', '1729503660.000100'),
    },
    {
      id: 't-priya',
      account: 'acc-work',
      provider: 'google',
      kind: 'email',
      title: 'Re: API v2 rollout',
      source: 's-work-inbox',
      sender: 'Priya',
      address: 'priya@acme.com',
      contact: 'c-priya',
      tier: 'ai-urgent',
      why: 'asks for your rollout sign-off before Friday',
      whySource: 'ai',
      unread: true,
      snippet: 'Re: API v2 rollout',
      messages: [
        { at: t(-1, '17:30'), sender: 'Ana Ruiz', address: 'ana@acme.com', text: 'Sharing the rollout plan for v2: flag first, then 10 %, then everyone by Friday.', tier: 'ai-normal', why: 'not scored (from you)' },
        { at: t(-1, '18:05'), sender: 'Tom Keller', address: 'tom@acme.com', contact: 'c-tom', text: 'Looks good. The retry policy is still a draft; can we keep it behind the same flag?', tier: 'ai-normal', why: 'answers a question, no request' },
        { at: t(0, '08:12'), text: 'Thanks both. I need your sign-off on the rollout before Friday so legal can review the changelog. Can you confirm today?', attachments: 1 },
      ],
      links: [
        { kind: 'session', id: 's-api', title: 'api-refactor', state: 'waiting', why: 'linked by you', by_user: true },
        { kind: 'note', id: 'n-changelog', title: 'Draft v2 changelog', why: 'made from this thread', by_user: false },
      ],
      url: gmail('18f2a1'),
    },
    {
      id: 't-tom',
      account: 'acc-acme',
      provider: 'slack',
      kind: 'dm',
      title: 'Tom Keller',
      source: 's-acme-dm-tom',
      sender: 'Tom',
      address: 'U04TOM',
      contact: 'c-tom',
      mentioned: true,
      // Lowered by the user (urgency_source = 'user'): a DM in the main list.
      tier: 'ai-normal',
      why: 'set by you',
      whySource: 'user',
      unread: true,
      snippet: 'quick one: is the retry policy final or still a draft?',
      messages: [{ at: t(0, '11:20'), text: 'quick one: is the retry policy final or still a draft?' }],
      links: [{ kind: 'note', id: 'n-retry', title: 'Rate limiter retry policy', why: 'linked by you', by_user: true }],
      url: slack('D01TOM', '1729509600.000200'),
    },
    {
      id: 't-lena',
      account: 'acc-acme',
      provider: 'slack',
      kind: 'channel',
      title: '#docs',
      channel: '#docs',
      source: 's-acme-docs',
      sender: 'Lena',
      address: 'U05LEN',
      contact: 'c-lena',
      tier: 'ai-normal',
      why: 'shares a pull request, no request',
      whySource: 'ai',
      unread: false,
      snippet: 'pushed the migration guide draft, PR 412 is up',
      messages: [{ at: t(-1, '16:05'), text: 'pushed the migration guide draft, PR 412 is up' }],
      url: slack('C02DOC', '1729440300.000300'),
    },
    {
      id: 't-invoice',
      account: 'acc-personal',
      provider: 'google',
      kind: 'email',
      title: 'Invoice 2041 — second reminder',
      source: 's-pers-inbox',
      sender: 'Ana Ruiz',
      address: 'billing@studio.example',
      tier: 'ai-normal',
      why: 'a payment reminder, no deadline today',
      whySource: 'ai',
      unread: false,
      snippet: 'Invoice 2041 — second reminder',
      messages: [{ at: t(-2, '10:15'), text: 'Hi Ana, a friendly reminder that invoice 2041 is still open. The PDF is attached.', attachments: 1 }],
      url: gmail('18f19c'),
    },
    {
      id: 't-ops',
      account: 'acc-acme',
      provider: 'slack',
      kind: 'channel',
      title: '#alerts',
      channel: '#alerts',
      source: 's-acme-alerts',
      sender: 'Ops bot',
      address: 'B01OPS',
      tier: 'ai-normal',
      why: 'a status update',
      whySource: 'ai',
      unread: false,
      snippet: 'p95 latency back under 300 ms after the rollback',
      messages: [{ at: t(-2, '07:30'), text: 'p95 latency back under 300 ms after the rollback' }],
      url: slack('C03ALR', '1729323000.000400'),
    },
    {
      id: 't-github',
      account: 'acc-work',
      provider: 'google',
      kind: 'email',
      title: '[mode] PR 418: rate limiter backoff — review requested',
      source: 's-work-inbox',
      sender: 'GitHub',
      address: 'notifications@github.com',
      tier: 'ai-normal',
      why: 'a review request from a bot',
      whySource: 'ai',
      unread: false,
      snippet: '[mode] PR 418: rate limiter backoff — review requested',
      messages: [{ at: t(-3, '18:40'), text: '@lena requested your review on PR 418: rate limiter backoff.' }],
      url: gmail('18f0aa'),
    },
    {
      id: 't-marta',
      account: 'acc-work',
      provider: 'google',
      kind: 'email',
      title: 'Quarterly numbers draft',
      source: 's-work-inbox',
      sender: 'Marta Silva',
      address: 'marta@acme.com',
      contact: 'c-marta',
      tier: 'ai-normal',
      why: 'a draft to read, no deadline',
      whySource: 'ai',
      unread: true,
      snippet: 'Quarterly numbers draft',
      messages: [{ at: t(-1, '09:10'), text: 'First pass at the quarterly numbers; no rush, comments welcome before the board prep.', attachments: 2 }],
      snoozedUntil: t(1, '09:00'),
      url: gmail('18f1d0'),
    },
    {
      id: 't-digest',
      account: 'acc-personal',
      provider: 'google',
      kind: 'email',
      title: 'Your weekly digest',
      source: 's-pers-inbox',
      sender: 'Product Hunt',
      address: 'hello@digest.example',
      tier: 'low',
      why: 'a newsletter',
      whySource: 'ai',
      unread: true,
      snippet: 'Your weekly digest',
      messages: [{ at: t(0, '06:00'), text: 'The five launches of the week.' }],
      url: gmail('18f2b0'),
    },
    {
      id: 't-signin',
      account: 'acc-work',
      provider: 'google',
      kind: 'email',
      title: 'Security alert: new sign-in on Mac',
      source: 's-work-inbox',
      sender: 'Google',
      address: 'no-reply@accounts.google.com',
      tier: 'low',
      why: 'an automated notice about your own sign-in',
      whySource: 'ai',
      unread: true,
      snippet: 'Security alert: new sign-in on Mac',
      messages: [{ at: t(-1, '08:02'), text: 'A new sign-in on Mac. If this was you, you do not need to do anything.' }],
      url: gmail('18f1a2'),
    },
    {
      id: 't-random',
      account: 'acc-acme',
      provider: 'slack',
      kind: 'channel',
      title: '#random',
      channel: '#random',
      source: 's-acme-release',
      sender: 'Lena',
      address: 'U05LEN',
      contact: 'c-lena',
      tier: 'low',
      why: 'chatter',
      whySource: 'ai',
      unread: true,
      snippet: 'lunch photos from the offsite are in the shared folder',
      messages: [{ at: t(-1, '13:22'), text: 'lunch photos from the offsite are in the shared folder' }],
      url: slack('C05RND', '1729430520.000500'),
    },
    {
      id: 't-shipped',
      account: 'acc-personal',
      provider: 'google',
      kind: 'email',
      title: 'Your order has shipped',
      source: 's-pers-inbox',
      sender: 'Shop',
      address: 'orders@shop.example',
      tier: 'low',
      why: 'a shipping notice',
      whySource: 'ai',
      unread: true,
      snippet: 'Your order has shipped',
      messages: [{ at: t(-2, '12:00'), text: 'Your parcel is on its way.' }],
      url: gmail('18f150'),
    },
    {
      id: 't-offsite',
      account: 'acc-work',
      provider: 'google',
      kind: 'email',
      title: 'Offsite agenda',
      source: 's-work-inbox',
      sender: 'Priya',
      address: 'priya@acme.com',
      contact: 'c-priya',
      tier: 'ai-normal',
      why: 'information only',
      whySource: 'ai',
      unread: false,
      snippet: 'Offsite agenda',
      messages: [{ at: t(-9, '15:00'), text: 'The agenda for the offsite is attached.', attachments: 1 }],
      archived: true,
      url: gmail('18e9f0'),
    },
  ]
  return specs.map((s): Thread => {
    const msgs: MessageRow[] = s.messages.map((m, i) => ({
      id: `${s.id}-m${i + 1}`,
      thread_id: s.id,
      sender_name: m.sender ?? s.sender,
      sender_address: m.address ?? s.address,
      sender_avatar_url: null,
      received_at: m.at,
      body_text: m.text,
      body_truncated: false,
      snippet: i === s.messages.length - 1 && s.kind !== 'channel_thread' ? s.snippet : m.text.slice(0, 500),
      attachments: m.attachments ?? 0,
      is_read: !s.unread || i < s.messages.length - (s.kind === 'channel_thread' ? 2 : 1),
      urgency_tier: m.tier ?? s.tier,
      urgency_why: m.why === undefined ? s.why : m.why,
      urgency_source: m.tier ? 'ai' : (s.whySource ?? 'rule'),
      mentioned: !!s.mentioned && i === s.messages.length - 1,
      source_url: s.url,
    }))
    if (s.kind === 'channel_thread') msgs[0]!.snippet = s.snippet
    const lastAt = s.kind === 'channel_thread' ? s.messages[0]!.at : s.messages[s.messages.length - 1]!.at
    const latest = s.kind === 'channel_thread' ? msgs[0]! : msgs[msgs.length - 1]!
    const unread = msgs.filter((m) => !m.is_read).length
    const contact = CONTACTS.find((c) => c.id === s.contact)
    const row: ThreadRow = {
      id: s.id,
      account_id: s.account,
      account_label: null,
      provider: s.provider,
      kind: s.kind,
      title: s.title,
      channel_name: s.channel ?? null,
      source_id: s.source,
      sender_name: latest.sender_name === s.sender ? s.sender : latest.sender_name,
      sender_address: s.address,
      sender_avatar_url: null,
      sender_contact_id: s.contact ?? null,
      sender_vip: contact?.vip ?? false,
      mentioned: !!s.mentioned,
      last_message_at: lastAt,
      last_snippet: s.snippet,
      message_count: msgs.length,
      unread_count: unread,
      has_attachments: msgs.some((m) => m.attachments > 0),
      urgency_tier: unread ? s.tier : null,
      urgency_why: s.why,
      urgency_source: s.whySource ?? 'rule',
      is_archived: !!s.archived,
      snoozed_until: s.snoozedUntil ?? null,
      snooze_preset: s.snoozedUntil ? 'tomorrow' : null,
      resurfaced: false,
      pending_writeback: false,
      source_url: s.url,
    }
    return { row, messages: msgs, links: s.links ?? [], base: s.tier, baseWhy: s.why, baseSource: s.whySource ?? 'rule' }
  })
}

/* ---- the calendar ----------------------------------------------------------------------------- */

interface EvSpec {
  id: string
  day: number
  start?: string
  end?: string
  allDay?: boolean
  title: string
  cal?: 'work' | 'personal'
  join?: string
  status?: CalendarEvent['status']
  response?: CalendarEvent['my_response']
  attendees?: CalendarEvent['attendees']
  linked?: ThreadLink[]
  location?: string
  description?: string
}

const MEET_STANDUP = 'https://meet.google.com/abc-defg-hij'
const MEET_REVIEW = 'https://meet.google.com/kqa-vrbn-xyz'
const MEET_DESIGN = 'https://meet.google.com/dsg-nrev-qwe'

function sampleEvents(now: number): CalendarEvent[] {
  // Days are counted from the Monday of the current week (Mon 19 Oct at the wireframes' clock).
  const monday = weekStartOf(localDate(now), 'monday')
  const specs: EvSpec[] = [
    // the month figure before the drawn week
    { id: 'e-standup-0928', day: -21, start: '09:30', end: '10:00', title: 'Standup', join: MEET_STANDUP },
    { id: 'e-board', day: -19, start: '16:00', end: '18:00', title: 'Board meeting' },
    { id: 'e-payroll-1001', day: -18, allDay: true, title: 'Payroll' },
    { id: 'e-sprint-1002', day: -17, start: '14:00', end: '15:00', title: 'Sprint review' },
    { id: 'e-standup-1005', day: -14, start: '09:30', end: '10:00', title: 'Standup', join: MEET_STANDUP },
    { id: 'e-standup-1007', day: -12, start: '09:30', end: '10:00', title: 'Standup', join: MEET_STANDUP },
    { id: 'e-1on1-1007', day: -12, start: '15:00', end: '15:30', title: '1:1 Priya' },
    { id: 'e-demo', day: -10, allDay: true, title: 'Demo day' },
    { id: 'e-standup-1012', day: -7, start: '09:30', end: '10:00', title: 'Standup', join: MEET_STANDUP },
    { id: 'e-relplan-1013', day: -6, start: '10:00', end: '12:00', title: 'Release planning' },
    { id: 'e-standup-1014', day: -5, start: '09:30', end: '10:00', title: 'Standup', join: MEET_STANDUP },
    { id: 'e-sprint-1014', day: -5, start: '14:00', end: '15:00', title: 'Sprint review' },
    { id: 'e-retro-1016', day: -3, start: '16:00', end: '16:45', title: 'Retro' },
    { id: 'e-gym-1017', day: -2, start: '10:00', end: '11:00', title: 'Gym', cal: 'personal' },
    // the drawn week (app-window.html, Calendar)
    { id: 'e-standup-mon', day: 0, start: '09:30', end: '10:00', title: 'Standup', join: MEET_STANDUP },
    {
      id: 'e-relplan',
      day: 1,
      start: '10:00',
      end: '12:00',
      title: 'Release planning',
      location: 'Room 4',
      attendees: [
        { name: 'Sam Okafor', email: 'sam@acme.com', response: 'accepted', organizer: true },
        { name: 'Ana', email: 'ana@acme.com', response: 'accepted', self: true },
        { name: 'Priya Nair', email: 'priya@acme.com', response: 'accepted' },
      ],
      // `event_links`: the checklist linked by hand; Sam's #release message shares "release" with the title.
      linked: [
        { kind: 'note', id: 'n-checklist', title: 'Release checklist', why: 'linked by you', by_user: true },
        { kind: 'message', id: 't-sam-m1', title: 'Sam · #release', why: 'from an attendee', by_user: false },
      ],
    },
    { id: 'e-standup-wed', day: 2, start: '09:30', end: '10:00', title: 'Standup', join: MEET_STANDUP },
    {
      id: 'e-review',
      day: 2,
      start: '11:00',
      end: '12:00',
      title: 'Review',
      join: MEET_REVIEW,
      attendees: [
        { name: 'Priya Nair', email: 'priya@acme.com', response: 'accepted', organizer: true },
        { name: 'Ana', email: 'ana@acme.com', response: 'accepted', self: true },
        { name: 'Lena Vogt', email: 'lena@acme.com', response: 'tentative' },
        { name: 'Tom Keller', email: 'tom@acme.com', response: 'needs_action' },
      ],
      linked: [
        { kind: 'note', id: 'n-retry', title: 'Rate limiter retry policy', why: 'linked by you', by_user: true },
        { kind: 'message', id: 't-priya-m3', title: 'Priya · Re: API v2 rollout', why: 'linked by you', by_user: true },
      ],
    },
    { id: 'e-pairing', day: 2, start: '12:00', end: '12:30', title: 'Pairing · Lena' },
    { id: 'e-1on1', day: 2, start: '15:00', end: '15:30', title: '1:1 Priya' },
    { id: 'e-docsfreeze', day: 3, allDay: true, title: 'Docs freeze' },
    { id: 'e-dentist', day: 3, start: '13:00', end: '14:00', title: 'Dentist', cal: 'personal', location: 'Calle Mayor 12' },
    { id: 'e-design', day: 3, start: '16:00', end: '17:00', title: 'Design review', join: MEET_DESIGN },
    { id: 'e-release', day: 4, start: '10:00', end: '12:00', title: 'Release v2' },
    { id: 'e-retro', day: 4, start: '16:00', end: '16:45', title: 'Retro' },
    { id: 'e-gym', day: 5, start: '10:00', end: '11:00', title: 'Gym', cal: 'personal' },
    { id: 'e-dinner', day: 6, start: '18:00', end: '19:00', title: "Dinner at Nina's", cal: 'personal' },
    // the following week: overlaps, a declined call
    { id: 'e-standup-1026', day: 7, start: '09:30', end: '10:00', title: 'Standup', join: MEET_STANDUP },
    { id: 'e-vendor', day: 8, start: '11:00', end: '11:30', title: 'Vendor demo', response: 'declined' },
    { id: 'e-standup-1028', day: 9, start: '09:30', end: '10:00', title: 'Standup', join: MEET_STANDUP },
    { id: 'e-sprint-1028', day: 9, start: '09:45', end: '10:45', title: 'Sprint review', join: MEET_DESIGN },
    { id: 'e-retro-1030', day: 11, start: '16:00', end: '16:45', title: 'Retro' },
    { id: 'e-payroll-1101', day: 13, allDay: true, title: 'Payroll' },
    // November: tentative and canceled
    { id: 'e-offsite', day: 15, start: '10:00', end: '16:00', title: 'Offsite planning', status: 'tentative', response: 'tentative' },
    { id: 'e-boardprep', day: 16, start: '09:00', end: '10:00', title: 'Board prep', status: 'cancelled' },
  ]
  return specs.map((s): CalendarEvent => {
    const date = addDays(monday, s.day)
    const personal = s.cal === 'personal'
    let starts: string
    let ends: string
    if (s.allDay) {
      starts = iso(dateStart(date).getTime())
      ends = iso(dateStart(addDays(date, 1)).getTime())
    } else {
      const at = (hhmm: string) => {
        const [h, m] = hhmm.split(':').map(Number)
        const d = dateStart(date)
        d.setHours(h!, m!, 0, 0)
        return iso(d.getTime())
      }
      starts = at(s.start!)
      ends = at(s.end!)
    }
    return {
      id: s.id,
      title: s.title,
      starts_at: starts,
      ends_at: ends,
      all_day: !!s.allDay,
      status: s.status ?? 'confirmed',
      my_response: s.response ?? 'accepted',
      account_color: null,
      calendar_name: personal ? 'Personal' : 'Work',
      conference: s.join ? 'meet' : null,
      join_url: s.join ?? null,
      location: s.location ?? null,
      section: 'later',
      account_id: personal ? 'acc-personal' : 'acc-work',
      account_name: personal ? 'ana.ruiz@example.com' : 'ana@acme.com',
      timezone: null,
      organizer: s.attendees?.find((a) => a.organizer)?.name ?? null,
      attendees: s.attendees ?? [],
      description: s.description ?? null,
      html_link: `https://calendar.google.com/calendar/event?eid=${s.id}`,
      prepare_note_id: null,
      linked: s.linked ?? [],
    }
  })
}

/* ---- derived payloads ---------------------------------------------------------------------- */

function accountLabel(id: string): string | null {
  return st!.accounts.find((a) => a.id === id)?.display_name ?? null
}

function rowOut(t: Thread): ThreadRow {
  return { ...t.row, account_label: accountLabel(t.row.account_id) }
}

function snoozed(r: ThreadRow, now: number): boolean {
  return !!r.snoozed_until && Date.parse(r.snoozed_until) > now
}

function sectionOf(r: ThreadRow, now: number): ThreadRow['section'] {
  if (snoozed(r, now)) return 'snoozed'
  if (r.urgency_tier === 'low') return 'low'
  if (r.urgency_tier && URGENT_TIERS.includes(r.urgency_tier) && r.unread_count > 0) return 'urgent'
  return 'main'
}

const SECTION_ORDER = { urgent: 0, snoozed: 1, main: 2, low: 3 } as const

/** Q3 then the snoozed rows, Q8 and the low rows, each in the core's order (07 §3.5). */
function ranked(rows: ThreadRow[], now: number): ThreadRow[] {
  return rows
    .map((r) => ({ ...r, section: sectionOf(r, now) }))
    .sort((a, b) => {
      const s = SECTION_ORDER[a.section!] - SECTION_ORDER[b.section!]
      if (s) return s
      if (a.section === 'urgent') {
        const t = RANK[a.urgency_tier!] - RANK[b.urgency_tier!]
        if (t) return t
      }
      if (a.section === 'snoozed') return Date.parse(a.snoozed_until!) - Date.parse(b.snoozed_until!)
      return Date.parse(b.last_message_at) - Date.parse(a.last_message_at)
    })
}

function matches(t: Thread, core: CoreInboxFilter & InboxFilter, now: number): { ok: boolean; hit?: string } {
  // The core's names (`topic`, `show_archived`, `from` / `to`) and the UI's (`query`, `archived`, `date`).
  const f: InboxFilter = {
    ...core,
    query: core.topic ?? core.query,
    archived: core.show_archived ?? core.archived,
    date: core.date ?? (core.from || core.to ? { from: core.from ?? '1970-01-01T00:00:00Z', to: core.to ?? '9999-12-31T00:00:00Z' } : null),
  }
  const r = t.row
  if (f.account_id && r.account_id !== f.account_id) return { ok: false }
  if (r.is_archived && !f.archived) return { ok: false }
  if (f.tiers?.length) {
    const tier = r.urgency_tier ?? t.base
    if (!f.tiers.includes(tier)) return { ok: false }
  }
  if (f.contact_id && r.sender_contact_id !== f.contact_id && !t.messages.some((m) => m.sender_address === f.contact_id)) return { ok: false }
  if (f.source_id && r.source_id !== f.source_id) return { ok: false }
  if (f.unread_only && r.unread_count === 0) return { ok: false }
  if (f.date) {
    const at = Date.parse(r.last_message_at)
    const today = dateStart(localDate(now)).getTime()
    if (f.date === 'today' && at < today) return { ok: false }
    if (f.date === 'week' && at < today - 6 * 86_400_000) return { ok: false }
    if (typeof f.date === 'object' && (at < Date.parse(f.date.from) || at > Date.parse(f.date.to))) return { ok: false }
  }
  if (f.query) {
    const words = f.query.toLowerCase().split(/\s+/).filter(Boolean)
    const hay = [r.title, r.sender_name, r.last_snippet ?? '', ...t.messages.map((m) => `${m.sender_name} ${m.body_text ?? m.snippet}`)].join('\n').toLowerCase()
    if (!words.every((w) => hay.includes(w))) return { ok: false }
    const msg = t.messages.find((m) => words.some((w) => (m.body_text ?? m.snippet).toLowerCase().includes(w)))
    return { ok: true, hit: (msg?.body_text ?? msg?.snippet ?? r.last_snippet ?? '').slice(0, 160) }
  }
  return { ok: true }
}

/** `inbox.changed` (02 §7.6): Q3 rows, the urgent count and the unread count in the app. */
function inboxPayload(now: number) {
  const s = st!
  const live = s.threads.filter((t) => !t.row.is_archived && !snoozed(t.row, now))
  const urgent = ranked(
    live.map((t) => t.row),
    now,
  ).filter((r) => r.section === 'urgent')
  const rows: InboxRow[] = urgent.map((r) => {
    const t = s.threads.find((x) => x.row.id === r.id)!
    const last = t.messages[t.messages.length - 1]!
    return {
      id: last.id,
      provider: r.provider === 'slack' ? 'slack' : 'gmail',
      sender_name: r.sender_name,
      sender_avatar_url: null,
      channel_name: r.channel_name,
      mentioned: r.mentioned,
      subject: r.kind === 'email' ? r.title : null,
      snippet: r.last_snippet,
      received_at: r.last_message_at,
      urgency_tier: r.urgency_tier ?? 'unscored',
      urgency_why: r.urgency_why,
      is_read: false,
      account_label: accountLabel(r.account_id),
    }
  })
  return { rows, urgent_count: urgent.length, unread_in_app: live.filter((t) => t.row.unread_count > 0).length }
}

function agendaPayload(now: number) {
  const s = st!
  const today = localDate(now)
  const tomorrow = addDays(today, 1)
  const end = dateStart(addDays(today, 2)).getTime()
  const start = dateStart(today).getTime()
  const rows: AgendaRow[] = s.events
    .filter((e) => e.my_response !== 'declined' && Date.parse(e.starts_at) < end && Date.parse(e.ends_at) > start)
    .sort((a, b) => Date.parse(a.starts_at) - Date.parse(b.starts_at))
    .map((e) => {
      const st0 = Date.parse(e.starts_at)
      const en = Date.parse(e.ends_at)
      const section: AgendaRow['section'] =
        localDate(st0) === tomorrow ? 'tomorrow' : st0 <= now && en > now ? 'now' : st0 > now && st0 <= now + 2 * 3_600_000 ? 'next' : 'later'
      const { account_id: _a, account_name: _n, timezone: _t, organizer: _o, attendees: _at, description: _d, html_link: _h, prepare_note_id: _p, linked: _l, ...row } = e
      return { ...row, section }
    })
  const next_2h_count = rows.filter((r) => !r.all_day && Date.parse(r.starts_at) > now && Date.parse(r.starts_at) <= now + 2 * 3_600_000).length
  return { rows, next_2h_count }
}

function pushInbox() {
  mock().emit(EVENTS.inbox, inboxPayload(Date.now()))
}

function pushAgenda() {
  mock().emit(EVENTS.agenda, agendaPayload(Date.now()))
}

/** Pushes once a window listens (the core publishes on subscribe), retrying for up to 3 s. */
function pushWhenListened(event: string, payload: () => unknown, tries = 60) {
  const n = mock().emit(event, payload())
  if (n === 0 && tries > 0) setTimeout(() => pushWhenListened(event, payload, tries - 1), 50)
}

function cmdError(code: string, message: string) {
  return { code, message }
}

function maybeFail(cmd: string) {
  const why = st!.fail.get(cmd)
  if (why !== undefined) {
    st!.fail.delete(cmd)
    throw cmdError('failed', why)
  }
}

function threadOf(id: string): Thread {
  const t = st!.threads.find((x) => x.row.id === id || x.messages.some((m) => m.id === id))
  if (!t) throw cmdError('not_found', `No thread ${id}`)
  return t
}

function accountOf(id: string): AccountRow {
  const a = st!.accounts.find((x) => x.id === id)
  if (!a) throw cmdError('not_found', `No account ${id}`)
  return a
}

function recompute(t: Thread) {
  const unread = t.messages.filter((m) => !m.is_read)
  t.row.unread_count = unread.length
  if (!unread.length) t.row.urgency_tier = null
  else if (t.row.urgency_source === 'user') t.row.urgency_tier = t.row.urgency_tier ?? t.base
  else {
    const contact = st!.contacts.find((c) => c.id === t.row.sender_contact_id)
    if (contact?.vip) {
      t.row.urgency_tier = 'vip'
      t.row.urgency_why = `Important person: ${contact.display_name}${contact.vip_note ? ` (${contact.vip_note})` : ''}`
      t.row.urgency_source = 'rule'
    } else {
      t.row.urgency_tier = t.base === 'vip' ? 'unscored' : t.base
      t.row.urgency_why = t.base === 'vip' ? 'No longer important' : t.baseWhy
      t.row.urgency_source = t.baseSource
    }
  }
}

function vipRow(c: Contact): VipRow {
  return { id: c.id, display_name: c.display_name, identities: c.identities, vip_note: c.vip_note, message_count: c.message_count, last_seen_at: c.last_seen_at }
}

/* ---- install ---------------------------------------------------------------------------------- */

export interface ConnectorsMockHandle {
  state(): State
  /** The next call of `cmd` rejects with `message`. */
  failNext(cmd: string, message: string): void
  /** No accounts and no data: the empty states (03 §4.10, §6.8, §7C.5). */
  noAccounts(): void
  /** Re-emits the pushes. */
  push(): void
  /** How the next Connect run ends (once). */
  authNext(outcome: AuthOutcome): void
}

declare global {
  interface Window {
    __mode_connectors?: ConnectorsMockHandle
  }
}

/**
 * Seeds the mock and registers the package 10 commands. `capture` (the notes
 * mock's `capture`) makes Prepare notes create a real note in the sample vault.
 */
export function installConnectorsMock(opts: { now?: number; capture?: (text: string) => string } = {}): ConnectorsMockHandle {
  const now0 = opts.now ?? Date.now()
  st = {
    now0,
    accounts: sampleAccounts(now0),
    sources: sampleSources(now0),
    channels: sampleChannels(),
    threads: sampleThreads(now0),
    contacts: CONTACTS.map((c) => ({ ...c, identities: [...c.identities] })),
    events: sampleEvents(now0),
    alerts: [
      { id: 'al-1', rule_id: 'r-urgent', rule_name: 'Urgent message', event: 'message.received', item_key: 'slack:T0ACME:C01REL:1729503660.000100', channels: ['overlay'], suppressed_reason: null, fired_at: atDay(now0, 0, '09:41'), created_at: atDay(now0, 0, '09:41') },
      { id: 'al-2', rule_id: 'r-meeting', rule_name: 'Meeting starting', event: 'calendar.starting', item_key: 'event:e-review', channels: ['overlay'], suppressed_reason: null, fired_at: atDay(now0, 0, '10:55'), created_at: atDay(now0, 0, '10:55') },
    ],
    pending: new Map(),
    seq: 0,
    offline: false,
    fail: new Map(),
    quiet: { quiet_hours: false, focus: false, presentation: false },
    oauthClients: new Set(['google']),
    capture: opts.capture,
    auth: { kind: 'connect' },
  }
  const s = st
  const m = mock()
  const on = (cmd: string, fn: (a: Record<string, unknown>) => unknown) =>
    m.respond(cmd, (args: Args) => {
      maybeFail(cmd)
      return fn(args ?? {})
    })
  const now = () => Date.now()
  const after = (ms: number, fn: () => void) => setTimeout(fn, ms)

  /* accounts and sources */
  on('accounts_list', () => s.accounts.map((a) => ({ ...a })))
  on('sources_list', (a) => s.sources.filter((x) => !a.account_id || x.account_id === a.account_id).map((x) => ({ ...x })))
  on('source_set', (a) => {
    const row = s.sources.find((x) => x.id === a.id)
    if (!row) throw cmdError('not_found', `No source ${String(a.id)}`)
    if (row.always_on && a.enabled === false) throw cmdError('invalid', 'DMs, group DMs and mentions are always watched')
    if (typeof a.enabled === 'boolean') row.enabled = a.enabled
    if (typeof a.vip === 'boolean') row.vip = a.vip
    return { ...row }
  })
  on('slack_channels_list', (a) => {
    const id = String(a.account_id)
    accountOf(id)
    return (s.channels[id] ?? []).map((c) => ({ ...c }))
  })
  on('slack_channels_set', (a) => {
    const id = String(a.account_id)
    const ids = (a.channel_ids as string[]) ?? []
    const list = s.channels[id] ?? []
    for (const c of list) c.watched = ids.includes(c.id)
    for (const c of list) {
      const row = s.sources.find((x) => x.account_id === id && x.kind === 'slack_channel' && x.external_id === c.id)
      if (row) row.enabled = c.watched
      else if (c.watched) s.sources.push(src(`s-${id}-${c.id}`, id, 'slack_channel', c.id, c.name, true, null))
    }
    return s.sources.filter((x) => x.account_id === id).map((x) => ({ ...x }))
  })
  on('account_add_start', (a) => {
    if (s.offline) throw cmdError('offline', 'needs a network connection')
    const provider = a.provider === 'slack' ? 'slack' : 'google'
    const o = (a.options ?? {}) as { gmail?: boolean; calendar?: boolean; gmail_read_only?: boolean; slack_writeback?: boolean; login_hint?: string | null }
    const state = `st-${++s.seq}-${Math.random().toString(36).slice(2, 10)}`
    const services = (['gmail', 'calendar'] as const).filter((x) => o[x] !== false)
    // A login hint naming a connected Google identity re-authorizes it in place (06 §3.1).
    const same = o.login_hint ? s.accounts.find((x) => x.provider === 'google' && x.email === o.login_hint) : undefined
    const outcome = s.auth
    s.auth = { kind: 'connect' }
    s.pending.set(state, { ...(same ? { provider, accountId: same.id, services } : { provider, services, readOnly: provider === 'google' ? !!o.gmail_read_only : o.slack_writeback === false }), outcome })
    if (outcome.kind === 'open-fail')
      m.respond('plugin:opener|open_url', () => {
        m.respond('plugin:opener|open_url', () => undefined)
        throw cmdError('failed', 'no browser is set up')
      })
    return { auth_url: authUrl(provider, state), state, redirect_uri: 'http://127.0.0.1:48311/callback' }
  })
  on('account_add_cancel', (a) => {
    const p = s.pending.get(String(a.state))
    s.pending.delete(String(a.state))
    p?.cancel?.()
    return undefined
  })
  on('account_reauthorise', (a) => {
    const acc = accountOf(String(a.id))
    const state = `st-${++s.seq}-${Math.random().toString(36).slice(2, 10)}`
    const services = (a.services as ('gmail' | 'calendar')[] | undefined) ?? undefined
    s.pending.set(state, { provider: acc.provider, accountId: acc.id, services })
    return { auth_url: authUrl(acc.provider, state), state }
  })
  on(
    'account_add_finish',
    (a) =>
      new Promise((resolve, reject) => {
        const p = s.pending.get(String(a.state))
        if (!p) return reject(cmdError('not_found', 'the browser did not come back'))
        const outcome = p.outcome ?? { kind: 'connect' }
        if (outcome.kind === 'wait' || outcome.kind === 'open-fail') {
          p.cancel = () => reject(cmdError('cancelled', 'the flow was canceled'))
          return
        }
        if (outcome.kind === 'fail')
          return after(1000, () => {
            s.pending.delete(String(a.state))
            reject(cmdError('failed', outcome.reason))
          })
        // The loopback answers one second later (the user clicked Allow).
        after(1000, () => {
          if (!s.pending.has(String(a.state))) return reject(cmdError('cancelled', 'the flow was canceled'))
          s.pending.delete(String(a.state))
          if (p.accountId) {
            const acc = accountOf(p.accountId)
            Object.assign(acc, { status: 'active', status_detail: null, health: 'ok', last_sync_at: iso(now()) })
            if (p.services?.length) acc.services = Array.from(new Set([...acc.services, ...p.services]))
            if (acc.services.includes('calendar') && !s.sources.some((y) => y.account_id === acc.id && y.kind === 'calendar'))
              s.sources.push(src(`s-${acc.id}-cal`, acc.id, 'calendar', 'primary', acc.email ?? acc.display_name, true, null))
            for (const x of s.sources.filter((y) => y.account_id === acc.id)) x.last_polled_at = iso(now())
            pushInbox()
            emitHealth()
            return resolve({ ...acc })
          }
          const n = ++s.seq
          const id = `acc-new-${n}`
          const acc: AccountRow =
            p.provider === 'google'
              ? {
                  id,
                  provider: 'google',
                  display_name: `ana.side${n}@example.com`,
                  email: `ana.side${n}@example.com`,
                  handle: null,
                  team_id: null,
                  team_name: null,
                  avatar_url: null,
                  services: p.services?.length ? p.services : ['gmail', 'calendar'],
                  scopes: p.readOnly ? READ_ONLY_GOOGLE : ['openid', 'https://www.googleapis.com/auth/userinfo.email', 'https://www.googleapis.com/auth/gmail.modify', 'https://www.googleapis.com/auth/calendar.readonly'],
                  status: 'active',
                  status_detail: null,
                  health: 'ok',
                  color: null,
                  position: s.accounts.length,
                  connected_at: iso(now()),
                  last_sync_at: null,
                  can_write: { mark_read: !p.readOnly, mark_unread: !p.readOnly, archive: !p.readOnly },
                  messages_week: 0,
                  unread: 0,
                }
              : {
                  id,
                  provider: 'slack',
                  display_name: 'Globex · @ana',
                  email: null,
                  handle: 'U09ANA',
                  team_id: 'T0GLBX',
                  team_name: 'Globex',
                  avatar_url: null,
                  services: ['slack'],
                  scopes: p.readOnly ? SLACK_SCOPES.filter((x) => !x.endsWith(':write')) : SLACK_SCOPES,
                  status: 'active',
                  status_detail: null,
                  health: 'ok',
                  color: null,
                  position: s.accounts.length,
                  connected_at: iso(now()),
                  last_sync_at: null,
                  can_write: { mark_read: !p.readOnly, mark_unread: false, archive: false },
                  messages_week: 0,
                  unread: 0,
                }
          s.accounts.push(acc)
          if (acc.provider === 'google') {
            if (acc.services.includes('gmail')) s.sources.push(src(`s-${id}-inbox`, id, 'gmail_inbox', 'INBOX', 'Gmail inbox', true, null))
            if (acc.services.includes('calendar')) s.sources.push(src(`s-${id}-cal`, id, 'calendar', 'primary', acc.email ?? 'Calendar', true, null))
          } else {
            s.sources.push(src(`s-${id}-mentions`, id, 'slack_mentions', '*', 'Mentions', true, null, { always_on: true }))
            s.channels[id] = [
              { id: 'G1GEN', name: '#general', is_private: false, num_members: 40, watched: false },
              { id: 'G2PRJ', name: '#project-x', is_private: true, num_members: 6, watched: false },
            ]
          }
          resolve({ ...acc })
        })
      }),
  )
  on('account_pause', (a) => {
    const acc = accountOf(String(a.id))
    Object.assign(acc, { status: 'paused', health: 'paused' })
    emitHealth()
    return { ...acc }
  })
  on('account_resume', (a) => {
    const acc = accountOf(String(a.id))
    if (acc.status === 'needs_reauth' || acc.status === 'revoked') throw cmdError('needs_reauth', `${acc.display_name} needs to be re-authorized first`)
    Object.assign(acc, { status: 'active', status_detail: null, health: 'ok', last_sync_at: iso(now()) })
    for (const x of s.sources.filter((y) => y.account_id === acc.id && y.enabled)) x.last_polled_at = iso(now())
    emitHealth()
    return { ...acc }
  })
  on('account_retry', (a) => {
    const acc = accountOf(String(a.id))
    if (acc.status === 'needs_reauth' || acc.status === 'revoked') throw cmdError('needs_reauth', `${acc.display_name} needs to be re-authorized first`)
    Object.assign(acc, { status: 'active', status_detail: null, health: 'ok', last_sync_at: iso(now()) })
    emitHealth()
    return { ...acc }
  })
  on('oauth_client_set', (a) => {
    if (!String(a.client_id ?? '').trim()) throw cmdError('invalid', 'a client id is needed')
    if (a.provider === 'google' && !String(a.client_secret ?? '').trim()) throw cmdError('invalid', 'Google desktop clients need the client secret too')
    s.oauthClients.add(String(a.provider))
    return { name: `oauth_client:${String(a.provider)}${a.team_id ? `:${String(a.team_id)}` : ''}`, stored: true }
  })
  on('account_remove', (a) => {
    const id = String(a.id)
    accountOf(id)
    s.accounts = s.accounts.filter((x) => x.id !== id)
    s.sources = s.sources.filter((x) => x.account_id !== id)
    s.threads = s.threads.filter((t) => t.row.account_id !== id)
    s.events = s.events.filter((e) => e.account_id !== id)
    pushInbox()
    pushAgenda()
    emitHealth()
    return undefined
  })
  // `connectors_status` (06 §9A): one card line per service, the worst health of its accounts wins.
  on('connectors_status', (): ConnectorsStatus => {
    const t = now()
    const reports: AccountHealthReport[] = s.accounts.map((a) => ({
      account_id: a.id,
      display_name: a.display_name,
      provider: a.provider,
      status: a.status,
      health: s.offline ? 'offline' : a.status === 'active' ? 'ok' : a.status,
      line: `${a.display_name} · ${a.status === 'active' ? 'ok' : a.status.replace('_', ' ')}`,
      last_sync_at: a.last_sync_at,
      budget_used: 0,
      budget_cap: a.provider === 'slack' ? 40 : 250,
    }))
    const card = (svc: 'gmail' | 'calendar' | 'slack'): ConnectorCardStatus => {
      const mine = s.accounts.filter((a) => a.services.includes(svc))
      if (!mine.length) return { state: 'not_connected', glyph: '○', line: 'not connected', accounts: 0 }
      const n = mine.length
      const unit = n === 1 ? '1 account' : `${n} accounts`
      if (mine.some((a) => a.status === 'needs_reauth' || a.status === 'revoked')) return { state: 'needs_sign_in', glyph: '✕', line: 'needs sign-in', accounts: n }
      const failed = mine.find((a) => a.status === 'error')
      if (failed) return { state: 'error', glyph: '✕', line: failed.status_detail ?? 'error', accounts: n }
      if (s.offline) return { state: 'offline', glyph: '◉', line: 'offline', accounts: n }
      const newest = mine.map((a) => a.last_sync_at).filter((x): x is string => !!x).sort().pop()
      const secs = newest ? Math.max(0, Math.round((t - Date.parse(newest)) / 1000)) : null
      const ago = secs === null ? null : secs < 60 ? `${secs} s ago` : secs < 3600 ? `${Math.floor(secs / 60)} m ago` : secs < 86_400 ? `${Math.floor(secs / 3600)} h ago` : `${Math.floor(secs / 86_400)} d ago`
      return { state: 'connected', glyph: '◉', line: ago ? `${unit} · synced ${ago}` : unit, accounts: n }
    }
    return { shipped: true, offline: s.offline, gmail: card('gmail'), calendar: card('calendar'), slack: card('slack'), accounts: reports }
  })
  on('slack_manifest', (a) => slackManifest(a.writeback !== false))

  /* the Inbox */
  on('inbox_list', (a) => {
    const f = (a.filter as CoreInboxFilter & InboxFilter) ?? {}
    const t = now()
    const hits = new Map<string, string | undefined>()
    const rows = s.threads.filter((x) => {
      const r = matches(x, f, t)
      if (r.ok && r.hit !== undefined) hits.set(x.row.id, r.hit)
      return r.ok
    })
    const all = ranked(rows.map(rowOut), t).map((r) => (hits.has(r.id) ? { ...r, hit: hits.get(r.id) ?? null } : r))
    const offset = Number(a.cursor ?? 0) || 0
    const limit = Number(a.limit ?? 100) || 100
    const page = all.slice(offset, offset + limit)
    return { rows: page, next_cursor: offset + limit < all.length ? String(offset + limit) : null }
  })
  on('thread_get', (a) => {
    const t = threadOf(String(a.thread_id ?? a.id))
    const contact = s.contacts.find((c) => c.id === t.row.sender_contact_id) ?? null
    const source = s.sources.find((x) => x.id === t.row.source_id)
    const participants = Array.from(new Map(t.messages.map((mm) => [mm.sender_address, { name: mm.sender_name, address: mm.sender_address, contact_id: null as string | null }])).values())
    const detail: ThreadDetail = {
      thread: { ...rowOut(t), section: sectionOf(t.row, now()) },
      participants,
      messages: t.messages.map((mm) => ({ ...mm })),
      contact: contact ? { id: contact.id, display_name: contact.display_name, vip: contact.vip, vip_note: contact.vip_note, identities: contact.identities } : null,
      // Links written by `note_link_add` (the notes mock owns it) show up on the thread, as the core's `links` would.
      links: [
        ...t.links,
        ...mock()
          .invokes.filter((i) => i.cmd === 'note_link_add' && (i.args as { kind?: string; target_id?: string } | undefined)?.target_id === t.row.id)
          .map((i) => {
            const a = i.args as { note_id: string }
            return { kind: 'note' as const, id: a.note_id, title: noteTitle(a.note_id), why: 'linked by you', by_user: true }
          })
          .filter((l, n, all) => !t.links.some((x) => x.id === l.id) && !t.dismissed?.has(`note:${l.id}`) && all.findIndex((y) => y.id === l.id) === n),
      ],
      source_name: source?.name ?? null,
      body_error: null,
      app_local_actions: (() => {
        const can = accountOf(t.row.account_id).can_write
        return [!can?.mark_read && 'mark_read', !can?.mark_unread && 'mark_unread', (t.row.provider === 'slack' || !can?.archive) && 'archive'].filter((x): x is string => !!x)
      })(),
    }
    return detail
  })
  on('message_mark_read', (a) => {
    const t = threadOf(String(a.id))
    const read = a.read !== false
    const acc = accountOf(t.row.account_id)
    const single = a.kind === 'message' || a.scope === 'message' ? t.messages.find((mm) => mm.id === a.id) : null
    for (const mm of single ? [single] : t.messages) {
      mm.is_read = read
      if (!read && mm === t.messages[t.messages.length - 1]) break
    }
    if (!read && !single) {
      // Mark unread flags the latest message (Gmail's behavior for a thread).
      t.messages.forEach((mm, i) => (mm.is_read = i < t.messages.length - 1))
    }
    recompute(t)
    let result: TriageResult
    const can = read ? acc.can_write?.mark_read : acc.can_write?.mark_unread
    if (!can) result = { write_back: 'local_only', reason: acc.provider === 'google' ? 'Gmail access is read-only' : read ? 'mark read is off for this workspace' : 'Slack has no mark unread' }
    else {
      result = { write_back: 'queued' }
      t.row.pending_writeback = true
      if (acc.status === 'active') after(1500, () => {
        t.row.pending_writeback = false
        pushInbox()
      })
    }
    pushInbox()
    return result
  })
  on('thread_archive', (a) => {
    const t = threadOf(String(a.id))
    const archived = a.archived !== false
    t.row.is_archived = archived
    if (archived) {
      t.row.snoozed_until = null
      t.row.snooze_preset = null
    }
    const acc = accountOf(t.row.account_id)
    pushInbox()
    if (acc.provider === 'slack') return { write_back: 'local_only', reason: 'Slack has no archive' } satisfies TriageResult
    if (!acc.can_write?.archive) return { write_back: 'local_only', reason: 'Gmail access is read-only' } satisfies TriageResult
    return { write_back: 'queued' } satisfies TriageResult
  })
  on('thread_snooze', (a) => {
    const t = threadOf(String(a.id))
    const preset = (a.preset as SnoozePreset | undefined) ?? 'pick_date'
    if (preset === 'until_reply' && t.row.kind !== 'email' && t.row.kind !== 'channel_thread' && t.row.kind !== 'dm' && t.row.kind !== 'group_dm') throw cmdError('invalid', 'until reply needs a thread')
    const until = a.until ? Date.parse(String(a.until)) : resolvePreset(preset, now())
    t.row.snoozed_until = iso(until)
    t.row.snooze_preset = preset
    t.row.resurfaced = false
    pushInbox()
    return { until: iso(until) }
  })
  on('thread_unsnooze', (a) => {
    const t = threadOf(String(a.id))
    t.row.snoozed_until = null
    t.row.snooze_preset = null
    pushInbox()
    return undefined
  })
  on('message_set_tier', (a) => {
    const t = threadOf(String(a.id))
    const tier = a.tier as Tier
    t.base = tier
    t.row.urgency_source = 'user'
    t.row.urgency_why = 'set by you'
    if (t.row.unread_count > 0) t.row.urgency_tier = tier
    for (const mm of t.messages) if (!mm.is_read) Object.assign(mm, { urgency_tier: tier, urgency_why: 'set by you', urgency_source: 'user' })
    pushInbox()
    return undefined
  })
  on('open_in_source', (a) => threadOf(String(a.id)).row.source_url)

  /* important people (07 §5) */
  on('vips_list', () => s.contacts.filter((c) => c.vip).map(vipRow))
  on('vip_add', (a) => {
    const contactId = a.contact_id ?? a.contact
    let c = contactId ? s.contacts.find((x) => x.id === contactId) : undefined
    const address = String(a.email ?? (a.slack_user ? `@${String(a.slack_user)}` : undefined) ?? a.address ?? '').trim()
    if (!c && address) {
      c = s.contacts.find((x) => x.identities.some((i) => i.toLowerCase() === address.toLowerCase()))
      if (!c) {
        if (!/^@\S+|^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(address)) throw cmdError('invalid', 'type an email address or a Slack handle')
        c = { id: `c-new-${++s.seq}`, display_name: address.replace(/^@/, ''), vip: false, vip_note: null, identities: [address], message_count: 0, last_seen_at: null }
        s.contacts.push(c)
      }
    }
    if (!c) throw cmdError('not_found', 'No such contact')
    c.vip = true
    const note = a.vip_note ?? a.note
    if (typeof note === 'string') c.vip_note = note.trim() || null
    for (const t of s.threads) if (t.row.sender_contact_id === c.id) {
      t.row.sender_vip = true
      recompute(t)
    }
    pushInbox()
    return vipRow(c)
  })
  on('vip_remove', (a) => {
    const c = s.contacts.find((x) => x.id === (a.contact_id ?? a.id))
    if (!c) throw cmdError('not_found', 'No such contact')
    c.vip = false
    for (const t of s.threads) if (t.row.sender_contact_id === c.id) {
      t.row.sender_vip = false
      recompute(t)
    }
    pushInbox()
    return undefined
  })

  /* the calendar */
  on('agenda_list', (a) => {
    const from = Date.parse(String(a.from))
    const to = Date.parse(String(a.to))
    return s.events
      .filter((e) => Date.parse(e.starts_at) < to && Date.parse(e.ends_at) > from)
      .sort((x, y) => Date.parse(x.starts_at) - Date.parse(y.starts_at))
      .map((e) => ({ ...e }))
  })
  on('event_join', (a) => {
    const e = s.events.find((x) => x.id === a.id)
    if (!e?.join_url) throw cmdError('not_found', 'This calendar event has no join link')
    return e.join_url
  })
  on('event_prepare_notes', (a) => {
    const e = s.events.find((x) => x.id === a.id)
    if (!e) throw cmdError('not_found', `No calendar event ${String(a.id)}`)
    if (e.prepare_note_id) return { note_id: e.prepare_note_id }
    const when = new Date(e.starts_at)
    const day = `${when.getFullYear()}-${String(when.getMonth() + 1).padStart(2, '0')}-${String(when.getDate()).padStart(2, '0')}`
    const lines = [
      e.title,
      `${day} ${e.all_day ? 'all day' : `${String(when.getHours()).padStart(2, '0')}:${String(when.getMinutes()).padStart(2, '0')}`}`,
      ...(e.attendees?.length ? [`Attendees: ${e.attendees.map((x) => x.name).join(', ')}`] : []),
      ...(e.join_url ? [`Join: ${e.join_url}`] : []),
    ]
    const id = s.capture ? s.capture(lines.join('\n')) : `n-prepare-${e.id}`
    e.prepare_note_id = id
    e.linked = [{ kind: 'note', id, title: e.title, why: 'prepare note', by_user: false }, ...(e.linked ?? [])]
    return { note_id: id }
  })

  /* thread and event links (`mode_core::links`): the sample's links are what the matcher found; adds and removes persist here */
  const linkTarget = (kind: string, id: string): ThreadLink => {
    if (kind === 'session') {
      const r = [...sampleRoster(now()), ...sampleHistory(now())].find((x) => x.id === id)
      if (!r) throw cmdError('invalid', `the session is gone`)
      return { kind: 'session', id, title: r.display_name, state: r.state, why: 'linked by you', by_user: true }
    }
    if (kind === 'message') {
      const t = threadOf(id)
      const m = t.messages.find((x) => x.id === id)!
      return { kind: 'message', id, title: `${m.sender_name} · ${t.row.channel_name ?? t.row.title}`, why: 'linked by you', by_user: true }
    }
    if (kind === 'note') return { kind: 'note', id, title: noteTitle(id), why: 'linked by you', by_user: true }
    throw cmdError('invalid', `${JSON.stringify(kind)} is not a link target`)
  }
  const anchorLinks = (a: Args): { get(): ThreadLink[]; put(l: ThreadLink[]): void } => {
    if (a?.kind === 'calendar_event') {
      const e = s.events.find((x) => x.id === a.id)
      if (!e) throw cmdError('not_found', 'the event is no longer in the calendar')
      return { get: () => e.linked ?? [], put: (l) => (e.linked = l) }
    }
    const t = threadOf(String(a?.id))
    return { get: () => t.links, put: (l) => (t.links = l) }
  }
  on('event_links', (a) => {
    const e = s.events.find((x) => x.id === a.id)
    if (!e) throw cmdError('not_found', 'the event is no longer in the calendar')
    return [...(e.linked ?? [])]
  })
  on('item_link_add', (a) => {
    const at = anchorLinks(a)
    const l = linkTarget(String(a.to_kind), String(a.to_id))
    const rest = at.get().filter((x) => !(x.kind === l.kind && x.id === l.id))
    const kinds = ['session', 'note', 'message', 'thread']
    at.put([...rest, l].sort((x, y) => kinds.indexOf(x.kind) - kinds.indexOf(y.kind) || Number(!!y.by_user) - Number(!!x.by_user)))
    if (a.kind !== 'calendar_event') threadOf(String(a.id)).dismissed?.delete(`${l.kind}:${l.id}`)
    return [...at.get()]
  })
  on('item_link_remove', (a) => {
    const at = anchorLinks(a)
    at.put(at.get().filter((x) => !((x.kind === a.to_kind || (x.kind === 'thread' && a.to_kind === 'message')) && x.id === a.to_id)))
    if (a.kind !== 'calendar_event') {
      const t = threadOf(String(a.id))
      ;(t.dismissed ??= new Set()).add(`${String(a.to_kind)}:${String(a.to_id)}`)
    }
    return [...at.get()]
  })

  // Alerts, quiet hours, keyword rules and the AI backends are the settings mock's (mock-settings.ts):
  // they live in the settings registry and answer without a connected account.

  pushWhenListened(EVENTS.inbox, () => inboxPayload(Date.now()))
  pushWhenListened(EVENTS.agenda, () => agendaPayload(Date.now()))

  const handle: ConnectorsMockHandle = {
    state: () => st!,
    failNext(cmd, message) {
      st!.fail.set(cmd, message)
    },
    noAccounts() {
      st!.accounts = []
      st!.sources = []
      st!.threads = []
      st!.events = []
      pushInbox()
      pushAgenda()
    },
    push() {
      pushInbox()
      pushAgenda()
    },
    authNext(outcome) {
      st!.auth = outcome
    },
  }
  if (typeof window !== 'undefined') window.__mode_connectors = handle
  return handle
}

/** `health.changed` as the core sends it: the settings mock's full payload with this mock's account lists. */
function emitHealth() {
  const send = (base: unknown) => mock().emit(EVENTS.health, { ...(base && typeof base === 'object' ? base : {}), ...healthPatch() })
  mock()
    .answer('health_get', undefined)
    .then(send, () => send(null))
}

/** A note's title from the notes mock (`window.__mode_notes_mock`), for links written through it. */
function noteTitle(id: string): string {
  const v = (globalThis as { __mode_notes_mock?: { get(id: string): unknown; row(n: unknown): { title: string } } }).__mode_notes_mock
  try {
    return v ? v.row(v.get(id)).title : id
  } catch {
    return id
  }
}

function healthPatch() {
  const s = st!
  return {
    offline: s.offline,
    paused_accounts: s.accounts.filter((a) => a.status === 'paused').map((a) => a.display_name),
    broken_accounts: s.accounts.filter((a) => a.status === 'needs_reauth' || a.status === 'error' || a.status === 'revoked').map((a) => a.display_name),
    hooks: 'installed',
    vault_missing: false,
    read_only: false,
  }
}

function authUrl(provider: 'google' | 'slack', state: string): string {
  const redirect = encodeURIComponent('http://127.0.0.1:48311/callback')
  if (provider === 'slack')
    return `https://slack.com/oauth/v2/authorize?client_id=mock&user_scope=${encodeURIComponent(SLACK_SCOPES.join(','))}&redirect_uri=${redirect}&state=${state}&code_challenge=mock&code_challenge_method=S256`
  return `https://accounts.google.com/o/oauth2/v2/auth?client_id=mock&redirect_uri=${redirect}&response_type=code&scope=${encodeURIComponent(READ_ONLY_GOOGLE.join(' '))}&access_type=offline&prompt=consent&code_challenge=mock&code_challenge_method=S256&state=${state}`
}
