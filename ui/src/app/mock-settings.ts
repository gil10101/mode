// The package 9 commands in the browser mock (outside Tauri): the settings
// registry with the wireframes' sample values (Ana on a Mac, today Wed 21 Oct
// 2026), the hook install states (installed / stale / missing / conflict /
// none) with a diff preview, diagnostics, logs, AI backends (the core's
// `ai_*` commands), secrets, quiet hours and alerts (`quiet_set`,
// `alerts_recent`, `alert_rules_list`, `alert_rule_set`, `alert_rule_add`,
// `alert_rule_delete`, `alert_rule_test`, validated as the core validates),
// keyword rules, the notes folders, the
// quit confirmation, hotkeys and 52 weeks of stats days whose last 16 weeks
// draw the wireframe's tracker. Shapes are the app crate's and
// `mode_core::connectors::api`'s. Registered on `window.__mode_mock.respond`;
// `window.__mode_settings` switches the hook state and the live sessions for
// tests.
import { anaPicture } from '../kit/mascot'
import { mock } from '../tauri'
import { EVENTS } from '../state/contract'
import type { FoldersList, GroupingRow, GroupingsList, StatusDef, TagRow } from '../state/contract-notes'
import { ALERT_EVENT_FILTERS, ALERT_EVENTS, ALERT_MESSAGE_KINDS, ALERT_TIERS, ALERT_WAITING_KINDS } from '../state/contract-connectors'
import type { AiBackend, AiBackendRow, AiHealth, AiProvider, AiProviderDraft, AiProviderKindRow, AiProviderRow, AiRoute, AlertChannel, AlertEventName, AlertOutcome, AlertRow, AlertRuleAddArgs, AlertRuleDeleted, AlertRuleFilters, AlertRuleRow, AlertRuleSetArgs, ClaudeCodeCard, ConnectorsStatus, KeywordRule, QuietSetArgs } from '../state/contract-connectors'
import type { ProviderAuth, ProviderKind } from '../state/contract-connectors'
import type { FieldError } from '../state/contract-automations'
import type { LiveSession } from '../state/contract-terminal'
import type {
  AiBackendsStatus,
  CmdError,
  Diagnostics,
  GroupingDraftWithPosition,
  HookAction,
  HookInstallStatus,
  HooksApplied,
  HooksPlan,
  HooksStatus,
  ImportReport,
  PlatformInfo,
  QuitAnswer,
  RoundTripReport,
  SettingsHealth,
  SettingsMap,
  StatsDay,
  StatsSummary,
} from '../state/contract-settings'
import { registryDefaults, validateSetting, defaultFor } from '../state/settings-store'
import { GLOBAL_BY_KEY, CATALOG } from './shortcuts/catalog'
import { localDate } from './stats/tracker'
import { sampleDays, sampleSummary } from './stats/sample'

type Args = Record<string, unknown> | undefined

export type HookScenario = 'installed' | 'stale' | 'missing' | 'conflict' | 'none'

export interface SettingsMockControl {
  /** Switch the hook install state the mock reports. */
  hooks(s: HookScenario): void
  /** `~/.claude` present or not. */
  claude(found: boolean): void
  /** The next `hooks_install_apply` fails with this message (once). */
  failNextApply(message: string): void
  /** A fresh install: no onboarding step recorded, no hooks; pushes settings.changed and health.changed (first_run). */
  freshInstall(): void
  /** The embedded sessions a quit would interrupt (`pty_list`, `app_quit`). */
  liveSessions(list: LiveSession[]): void
  /** The AI backend's health the mock reports (06 §10.7 states). */
  aiState(state: AiHealth['state']): void
  /** The next `alert_rule_set` is refused with this message (`invalid`, once). */
  failNextRuleSet(message: string): void
  /** The next `alert_rule_add` is refused with this message (`code`, default `invalid`; once). */
  failNextRuleAdd(message: string, code?: string): void
  /** The next `alert_rule_delete` is refused with this code and message (once). */
  failNextRuleDelete(code: string, message: string): void
  /** The stored alert rules (`alert_rules_list` order). */
  alertRules(): AlertRuleRow[]
  state(): { values: SettingsMap; hooks: HookScenario; quit: string[] }
}

declare global {
  interface Window {
    __mode_settings?: SettingsMockControl
  }
}

/** Chords the OS keeps for itself (Spotlight, input switching): `hotkey_set` refuses them. */
const TAKEN = new Set(['Cmd+Space', 'Ctrl+Space', 'Cmd+Tab', 'Alt+Tab'])

const EVENT_NAMES = [
  'PermissionRequest',
  'PreToolUse',
  'SessionStart',
  'SessionEnd',
  'UserPromptSubmit',
  'PostToolUse',
  'PostToolUseFailure',
  'Notification',
  'Stop',
  'StopFailure',
  'SubagentStart',
  'SubagentStop',
  'PreCompact',
  'PostCompact',
  'CwdChanged',
  'PreModelSwitch',
  'PostModelSwitch',
  'Elicitation',
]
const DECISION = new Set(['PermissionRequest', 'PreToolUse'])
const NO_MATCHER = new Set(['SessionEnd', 'UserPromptSubmit', 'Stop', 'StopFailure', 'CwdChanged', 'PreModelSwitch', 'PostModelSwitch', 'Elicitation'])

const HOME = '/Users/ana'
const SETTINGS_PATH = `${HOME}/.claude/settings.json`
const RELAY = '/Applications/Mode.app/Contents/MacOS/mode-hook'
const DATA_DIR = `${HOME}/Library/Application Support/Mode-dev`
const FOREIGN = '/usr/local/bin/lint-check'


function hookBlock(timeouts: { decision: number; async: number }): string[] {
  const lines: string[] = ['  "hooks": {']
  EVENT_NAMES.forEach((name, i) => {
    const decision = DECISION.has(name)
    const t = decision ? timeouts.decision : timeouts.async
    lines.push(`    "${name}": [`)
    lines.push('      {')
    if (!NO_MATCHER.has(name)) lines.push('        "matcher": "",')
    lines.push('        "hooks": [')
    lines.push(`          { "type": "command", "command": "${RELAY}", "args": ["${name}", "${t}"], "timeout": ${t}${decision ? '' : ', "async": true'} }`)
    lines.push('        ]')
    lines.push(`      }${name === 'PreToolUse' ? ',' : ''}`)
    if (name === 'PreToolUse') {
      lines.push('      {')
      lines.push('        "matcher": "Bash",')
      lines.push(`        "hooks": [{ "type": "command", "command": "${FOREIGN}", "timeout": 10 }]`)
      lines.push('      }')
    }
    lines.push(`    ]${i === EVENT_NAMES.length - 1 ? '' : ','}`)
  })
  lines.push('  },')
  return lines
}

const HEAD = ['{', '  "$schema": "https://json.schemastore.org/claude-code-settings.json",', '  "permissions": { "allow": ["Bash(npm test)", "Read"], "deny": [] },']
const FOREIGN_ONLY = ['  "hooks": {', '    "PreToolUse": [', '      {', '        "matcher": "Bash",', `        "hooks": [{ "type": "command", "command": "${FOREIGN}", "timeout": 10 }]`, '      }', '    ]', '  },']
const TAIL = ['  "model": "opus"', '}']

/** A unified diff over two line arrays (LCS, one hunk with all lines; files here are small). */
export function unifiedDiff(a: string[], b: string[], path: string): string {
  const n = a.length
  const m = b.length
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0))
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) dp[i]![j] = a[i] === b[j] ? dp[i + 1]![j + 1]! + 1 : Math.max(dp[i + 1]![j]!, dp[i]![j + 1]!)
  const out: string[] = [`--- ${path}`, `+++ ${path}`, `@@ -1,${n} +1,${m} @@`]
  let i = 0
  let j = 0
  while (i < n || j < m) {
    if (i < n && j < m && a[i] === b[j]) {
      out.push(` ${a[i]}`)
      i++
      j++
    } else if (j < m && (i >= n || dp[i]![j + 1]! >= dp[i + 1]![j]!)) out.push(`+${b[j++]}`)
    else out.push(`-${a[i++]}`)
  }
  return out.join('\n')
}

interface MockState {
  values: SettingsMap
  hooks: HookScenario
  claudeFound: boolean
  failApply: string | null
  secret: boolean
  backups: { path: string; size_bytes: number; created_at: string }[]
  /** Positions set through `grouping_save`. */
  positions: Map<string, number>
  deletedGroupings: Set<string>
  /** By `name_norm` of the sample vault's tags. */
  tagNames: Map<string, string>
  deletedTags: Set<string>
  days: Map<string, StatsDay>
  live: LiveSession[]
  /** `app_quit` calls answered: `ask` / `confirm` / `cancel`. */
  quit: string[]
  aiState: AiHealth['state'] | null
  providers: ProviderRecord[]
  routes: AiRoute[]
  alerts: AlertRow[]
  /** The live `alert_rules` with their `rev` (`edited` = differs from the premade seed, as the core reads it; deleted rows leave the list). */
  alertRules: (AlertRuleRow & { rev: number })[]
  failRuleSet: string | null
  failRuleAdd: { code: string; message: string } | null
  failRuleDelete: { code: string; message: string } | null
  /** Ids handed out by `alert_rule_add`. */
  ruleSeq: number
  /** `notes_folders_set`: folder path per kind, applied over the notes mock's folders. */
  folderKinds: { capture?: string; templates?: string | null }
  rulesSeeded?: boolean
}

let st: MockState | null = null

function iso(t: number): string {
  return new Date(t).toISOString()
}

function osFromLocation(): PlatformInfo['os'] {
  try {
    const q = new URLSearchParams(window.location.search).get('os')
    if (q === 'windows' || q === 'linux' || q === 'macos') return q
  } catch {
    /* no location */
  }
  return 'macos'
}

function hookStatus(now: number): HooksStatus {
  const s = st!
  const base = { path: SETTINGS_PATH, block_hash: null as string | null, detail: null as string | null, last_round_trip: null as RoundTripReport | null }
  const rt: RoundTripReport = { session_id: 'mode-round-trip-1', latency_ms: 9, acked: true, relay_version: '0.1.0', at: iso(now - 3_600_000) }
  switch (s.hooks) {
    case 'installed':
      return {
        ...base,
        installed: true,
        status: 'installed',
        version: '0.1.0',
        relay_path: RELAY,
        conflicts: [],
        events: [...EVENT_NAMES],
        block_hash: 'b3f1c2a9',
        last_round_trip: rt,
        installed_at: iso(now - 2 * 86_400_000),
        verified_at: iso(now - 3_600_000),
        backup_path: `${DATA_DIR}/backups/settings.json.mode-backup-20261019T091200Z`,
      }
    case 'stale':
      return { ...base, installed: true, status: 'stale', version: '0.0.9', relay_path: '/Applications/Mode.app/Contents/MacOS/mode-hook-0.0.9', conflicts: [], events: [...EVENT_NAMES], detail: 'relay path changed after an update' }
    case 'missing':
      return { ...base, installed: false, status: 'missing', version: null, relay_path: null, conflicts: [], events: [], detail: 'Mode entries are gone from the file' }
    case 'conflict':
      return { ...base, installed: true, status: 'conflict', version: '0.1.0', relay_path: RELAY, conflicts: [{ event: 'PreToolUse', command: FOREIGN, matcher: 'Bash' }], events: EVENT_NAMES.filter((e) => e !== 'PreToolUse'), detail: null }
    default:
      return { ...base, installed: false, status: 'not_installed' as HookInstallStatus, version: null, relay_path: null, conflicts: [], events: [] }
  }
}

function plan(action: HookAction): HooksPlan {
  const s = st!
  const timeouts = { decision: Number(s.values['hooks.decision_timeout_seconds'] ?? 120), async: Number(s.values['hooks.async_timeout_seconds'] ?? 5) }
  const installedLines = [...HEAD, ...hookBlock(timeouts), ...TAIL]
  const foreignLines = [...HEAD, ...FOREIGN_ONLY, ...TAIL]
  const current = s.hooks === 'installed' || s.hooks === 'conflict' ? installedLines : s.hooks === 'stale' ? [...HEAD, ...hookBlock(timeouts).map((l) => l.replace(RELAY, `${RELAY}-0.0.9`)), ...TAIL] : foreignLines
  const after = action === 'uninstall' ? foreignLines : installedLines
  const diff = unifiedDiff(current, after, '~/.claude/settings.json')
  return {
    target: { settings_path: SETTINGS_PATH, relay_path: RELAY, command: RELAY, kind: { kind: 'native' } },
    action,
    file_existed: true,
    before: current.join('\n') + '\n',
    after: after.join('\n') + '\n',
    diff,
    changed: current.join('\n') !== after.join('\n'),
    conflicts: [{ event: 'PreToolUse', command: FOREIGN, matcher: 'Bash' }],
    events: action === 'uninstall' ? [] : EVENT_NAMES.map((e) => ({ event: e, timeout: DECISION.has(e) ? timeouts.decision : timeouts.async, async: !DECISION.has(e), matcher: NO_MATCHER.has(e) ? null : '' })),
    block_hash: action === 'uninstall' ? '' : 'b3f1c2a9',
    backup_dir: `${DATA_DIR}/backups`,
    relay_version: '0.1.0',
  }
}

function health(now: number): SettingsHealth {
  const s = st!
  void now
  return {
    offline: false,
    paused_accounts: [],
    hooks: s.hooks === 'installed' ? 'installed' : s.hooks,
    vault_missing: false,
    read_only: false,
    broken_accounts: [],
    claude_code: s.claudeFound ? 'found' : 'missing',
    claude_code_version: s.claudeFound ? '2.1.4' : null,
    db_newer: false,
    backups_dir: `${DATA_DIR}/backups`,
    tokens_week: 1_200_000,
    // As the core: true while `onboarding.completed_at` is unset.
    first_run: !s.values['onboarding.completed_at'],
  }
}

function diagnostics(now: number): Diagnostics {
  const s = st!
  const hs = hookStatus(now)
  return {
    core: { version: '0.1.0', dev_build: true, data_dir: DATA_DIR, db_path: `${DATA_DIR}/mode.db`, endpoint: '/var/folders/q1/T/mode-dev.sock', ipc_listening: true, hook_frames_received: 1284 },
    claude: s.claudeFound ? { found: true, path: `${HOME}/.local/bin/claude`, version: '2.1.4' } : { found: false, path: null, version: null },
    relay: { path: RELAY, version: '0.1.0' },
    hooks:
      s.hooks === 'none'
        ? []
        : [{ scope: 'user', settings_path: SETTINGS_PATH, status: hs.status, relay_version: hs.version, installed_at: hs.installed_at ?? null, verified_at: hs.verified_at ?? null, backup_path: hs.backup_path ?? null }],
    ipc: {
      endpoint: '/var/folders/q1/T/mode-dev.sock',
      listening: true,
      last_event_at: iso(now - 12_000),
      recent: [
        { event: 'PostToolUse', at: iso(now - 12_000), latency_ms: 3 },
        { event: 'PreToolUse', at: iso(now - 14_000), latency_ms: 4 },
        { event: 'PermissionRequest', at: iso(now - 61_000), latency_ms: 6 },
        { event: 'UserPromptSubmit', at: iso(now - 95_000), latency_ms: 2 },
        { event: 'SessionStart', at: iso(now - 300_000), latency_ms: 5 },
      ],
      metrics: { frames: 1284, acks: 1284, decisions: 12, timeouts: 0 },
    },
    index: { vault_path: String(s.values['notes.vault_path']), files_indexed: 27, last_full_scan_at: iso(now - 7_200_000), pending_touchups: 1 },
    database: {
      path: `${DATA_DIR}/mode.db`,
      size_bytes: 4_718_592,
      wal_bytes: 262_144,
      tables: [
        { name: 'sessions', rows: 41 },
        { name: 'session_events', rows: 3812 },
        { name: 'notes', rows: 27 },
        { name: 'usage_samples', rows: 960 },
      ],
      last_sweep_at: iso(now - 20 * 3_600_000),
      backups: s.backups,
      read_only: false,
    },
    sessions_tracked: 7,
    sessions_today: 3,
    sessions_embedded: s.live.length,
    secrets_store: 'os',
    log_path: `${DATA_DIR}/logs/mode.log`,
  }
}

function logLines(now: number, n: number): string[] {
  const base = [
    ['INFO', 'mode_app', 'core starting dev=true'],
    ['INFO', 'mode_core::db', 'database open path=mode.db user_version=1'],
    ['INFO', 'mode_core::ipc', 'listening endpoint=/var/folders/q1/T/mode-dev.sock'],
    ['INFO', 'mode_core::hooks_install', 'hook install verified status=installed'],
    ['DEBUG', 'mode_core::sessions::engine', 'frame applied event=PreToolUse'],
    ['WARN', 'mode_core::transcript::watch', 'native watcher unavailable, polling every 2 s'],
    ['INFO', 'mode_core::notes::index', 'full scan finished files=27'],
    ['INFO', 'mode_app::overlay', 'overlay placement=top-bar'],
    ['ERROR', 'mode_core::transcript::tailer', 'transcript line 4 is not JSON; skipped'],
    ['INFO', 'mode_core::hooks_install', 'round trip latency_ms=9'],
  ]
  const out: string[] = []
  for (let i = 0; i < Math.min(n, 60); i++) {
    const [level, target, msg] = base[i % base.length]!
    out.push(`${iso(now - (60 - i) * 7_000)}  ${level!.padStart(5)} ${target}: ${msg}`)
  }
  return out
}

/** The local servers the mock pretends to reach. */
const OLLAMA_MODELS = ['llama-3.3', 'qwen-2.5-coder', 'mistral-small', 'phi-4']
/** What the Anthropic API's model list answers in the mock (`GET /v1/models`). */
const CLAUDE_MODELS = ['claude-opus-5-5', 'claude-sonnet-5-5', 'claude-haiku-5-5']
/** The provider registry of the mock (06 §10.1A): the built-ins and two added providers. */
const PROVIDER_KINDS: AiProviderKindRow[] = [
  { kind: 'claude_api', label: 'Anthropic API', callable: true, session_capable: true, auths: ['api_key', 'bearer'], default_url: 'https://api.anthropic.com' },
  { kind: 'openai_compat', label: 'OpenAI-compatible', callable: true, session_capable: false, auths: ['bearer', 'none'], default_url: null },
  { kind: 'ollama', label: 'Ollama', callable: true, session_capable: true, auths: ['bearer', 'none'], default_url: 'http://127.0.0.1:11434' },
  { kind: 'lm_studio', label: 'LM Studio', callable: true, session_capable: true, auths: ['bearer', 'none'], default_url: 'http://127.0.0.1:1234' },
  { kind: 'bedrock', label: 'Amazon Bedrock', callable: false, session_capable: true, auths: ['bearer', 'none'], default_url: null },
  { kind: 'vertex', label: 'Google Vertex AI', callable: false, session_capable: true, auths: ['none', 'bearer'], default_url: null },
  { kind: 'foundry', label: 'Microsoft Foundry', callable: false, session_capable: true, auths: ['api_key', 'bearer'], default_url: null },
]
type ProviderRecord = AiProvider & { key: boolean; last4: string | null; agents: number }
function seedProviders(): ProviderRecord[] {
  const base = (id: string, kind: ProviderKind, name: string, auth: ProviderAuth, position: number, builtin: boolean): ProviderRecord => ({ id, kind, name, base_url: null, auth, models: [], extra: {}, enabled: true, builtin, position, key: false, last4: null, agents: 0 })
  return [
    base('claude_api', 'claude_api', 'Claude API', 'api_key', 0, true),
    base('ollama', 'ollama', 'Ollama', 'bearer', 1, true),
    base('lm_studio', 'lm_studio', 'LM Studio', 'bearer', 2, true),
    { ...base('pv-openrouter', 'claude_api', 'OpenRouter', 'bearer', 3, false), base_url: 'https://openrouter.ai/api', models: ['anthropic/claude-sonnet-4.5', 'anthropic/claude-haiku-4.5'], key: true, last4: '9c1d', agents: 1 },
    { ...base('pv-groq', 'openai_compat', 'Groq', 'bearer', 4, false), base_url: 'https://api.groq.com/openai', models: ['llama-3.3-70b'], key: true, last4: 'f00d' },
  ]
}
function seedRoutes(): AiRoute[] {
  return [
    { purpose: 'chat', provider_id: 'pv-openrouter', model: null, fallbacks: [{ provider_id: 'claude_api', model: 'claude-haiku-5-5' }] },
    { purpose: 'scoring', provider_id: null, model: null, fallbacks: [] },
    { purpose: 'summary', provider_id: null, model: null, fallbacks: [] },
  ]
}
const BACKEND_LABEL: Record<AiBackend, string> = { claude_api: 'Claude API', ollama: 'Ollama', lm_studio: 'LM Studio' }
const DEFAULT_URL: Record<AiBackend, string> = { claude_api: '', ollama: 'http://127.0.0.1:11434', lm_studio: 'http://127.0.0.1:1234' }

/** 06 §10.3 as the core classifies `ai.local_url` (literal IPs and `localhost` only). */
export function addressClass(raw: string): 'loopback' | 'private' | 'public' | 'invalid' {
  let u: URL
  try {
    u = new URL(raw)
  } catch {
    return 'invalid'
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return 'invalid'
  const h = u.hostname.replace(/^\[|\]$/g, '')
  if (h === 'localhost' || h === '::1' || /^127\./.test(h)) return 'loopback'
  if (/^10\./.test(h) || /^192\.168\./.test(h) || /^172\.(1[6-9]|2\d|3[01])\./.test(h) || /^f[cd]/i.test(h)) return 'private'
  if (/^[\d.]+$/.test(h) || h.includes(':')) return 'public'
  return 'public'
}

/** The three premade keyword rules (07 §3.3), seeded disabled on the first `keyword_rules_list`. */
export function premadeKeywordRules(): KeywordRule[] {
  return [
    { id: '00000000-0000-7000-8000-000000000301', name: 'Deadlines', enabled: false, field: 'any', match: 'deadline', pattern: '', effect: 'raise', horizon_days: 3, position: 0 },
    { id: '00000000-0000-7000-8000-000000000302', name: 'Newsletters', enabled: false, field: 'any', match: 'contains', pattern: 'unsubscribe', effect: 'lower', horizon_days: 3, position: 1, labels_any: ['broadcast'], labels_mode: 'or' },
    { id: '00000000-0000-7000-8000-000000000303', name: 'Bulk mail', enabled: false, field: 'any', match: 'contains', pattern: '', effect: 'lower', horizon_days: 3, position: 2, labels_any: ['list', 'bulk'], labels_mode: 'or' },
  ]
}

/** The premade alert rules as the migration seeds them (`0001_initial.sql` 5.5: 07 §8.2 plus "Automation failed", 09 §5.5): overlay only, Focus / DND and quiet hours respected. */
export function premadeAlertRules(): AlertRuleRow[] {
  const r = (n: number, name: string, enabled: boolean, event: string, filters: AlertRuleRow['filters'], sound: string | null, cooldown: number): AlertRuleRow => ({
    id: `00000000-0000-7000-8000-00000000020${n}`,
    name,
    kind: 'premade',
    enabled,
    event,
    filters,
    channels: ['overlay'],
    sound,
    respect_dnd: true,
    quiet_hours: true,
    cooldown_seconds: cooldown,
    position: n - 1,
    last_fired_at: null,
    edited: false,
  })
  return [
    r(1, 'Session needs you', true, 'session.waiting', { waiting_kind: ['permission', 'question', 'elicitation', 'agent_needs_input'] }, 'waiting', 0),
    r(2, 'Session failed', true, 'session.failed', {}, 'failed', 0),
    r(3, 'Session finished', true, 'session.finished', { min_duration_seconds: 0 }, 'finished', 0),
    r(4, 'Urgent message', true, 'message.received', { tier: ['vip', 'direct', 'ai-urgent'] }, 'urgent', 120),
    r(5, 'Meeting starting', true, 'calendar.starting', { minutes_before: 5, requires_join_url: false }, 'starting', 0),
    r(6, 'Due today', true, 'note.due', { at: 'start_of_day', urgent_only: false }, null, 0),
    r(7, 'AI budget at 80 %', false, 'usage.threshold', { percent: 80 }, null, 0),
    r(8, 'Automation failed', true, 'automation.failed', {}, null, 0),
  ]
}

/** `alerts::parse_hm`: `H:MM` / `HH:MM` (hours < 24, minutes < 60), surrounding blanks ignored. */
function parseHm(v: string): number | null {
  const m = /^\s*(\d+):(\d+)\s*$/.exec(v)
  if (!m) return null
  const h = Number(m[1])
  const min = Number(m[2])
  return h < 24 && min < 60 ? h * 60 + min : null
}

/** A filter value the core would store (`Option::None` for `undefined` and `null`). */
function present(v: unknown): boolean {
  return v !== undefined && v !== null
}

/**
 * `alerts::parse_trigger` then `alerts::check_filters` (07 §8.2), with the core's messages: a known event,
 * `minutes_before` 1–120, `at` = `start_of_day` or `HH:MM`, only the keys the event reads, list values the payload
 * can carry, `min_duration_seconds` ≥ 0, `percent` 1–100.
 */
export function alertFiltersProblem(event: string, f: AlertRuleFilters): string | null {
  if (!(ALERT_EVENTS as readonly string[]).includes(event)) return `unknown event ${event}`
  if (present(f.minutes_before) && !(Number(f.minutes_before) >= 1 && Number(f.minutes_before) <= 120)) return 'minutes_before is 1–120'
  if (present(f.at) && f.at !== 'start_of_day' && parseHm(String(f.at)) === null) return 'at is start_of_day or HH:MM'
  const allowed: readonly string[] = ALERT_EVENT_FILTERS[event as AlertEventName]
  // serde_json's map is ordered by key.
  const stray = Object.keys(f)
    .filter((k) => present((f as Record<string, unknown>)[k]) && !allowed.includes(k))
    .sort()
  if (stray.length) return `${event} has no filter ${stray.join(', ')}`
  const lists: [string, string[] | undefined, readonly string[]][] = [
    ['tier', f.tier, ALERT_TIERS],
    ['kind', f.kind, ALERT_MESSAGE_KINDS],
    ['waiting_kind', f.waiting_kind, ALERT_WAITING_KINDS],
  ]
  for (const [key, list, known] of lists) {
    const bad = (list ?? []).find((v) => !known.includes(v))
    if (bad !== undefined) return `${key} ${JSON.stringify(bad)} is not one of ${known.join(', ')}`
  }
  if (present(f.min_duration_seconds) && Number(f.min_duration_seconds) < 0) return 'min_duration_seconds is 0 or more'
  if (present(f.percent) && !(Number(f.percent) >= 1 && Number(f.percent) <= 100)) return 'percent is 1–100'
  return null
}

/** `connectors::api::name_taken`: SQLite's `lower()` folds ASCII only. */
export function ruleNameKey(name: string): string {
  return name.trim().replace(/[A-Z]/g, (c) => c.toLowerCase())
}

/** The filters as `alerts::normalized` compares them: list filters sorted and de-duplicated, absent keys dropped. */
function canonicalFilters(f: AlertRuleFilters): string {
  const out: Record<string, unknown> = {}
  for (const k of Object.keys(f).sort()) {
    const v = (f as Record<string, unknown>)[k]
    if (!present(v)) continue
    out[k] = Array.isArray(v) ? Array.from(new Set(v as string[])).sort() : v
  }
  return JSON.stringify(out)
}

/**
 * `alerts::edited`: a premade rule whose definition differs from its seed (found by id, else by name): name, enabled,
 * event and filters (list filters as sets), channels as a set, sound, cooldown, `respect_dnd`, `quiet_hours`; `position`
 * ignored. Custom rules are never edited.
 */
export function alertRuleEdited(r: AlertRuleRow): boolean {
  if (r.kind !== 'premade') return false
  const seeds = premadeAlertRules()
  const seed = seeds.find((x) => x.id === r.id) ?? seeds.find((x) => x.name === r.name)
  if (!seed) return false
  const same =
    r.name === seed.name &&
    r.enabled === seed.enabled &&
    r.event === seed.event &&
    canonicalFilters(r.filters) === canonicalFilters(seed.filters) &&
    Array.from(new Set(r.channels)).sort().join() === 'overlay' &&
    (r.sound ?? null) === seed.sound &&
    r.cooldown_seconds === seed.cooldown_seconds &&
    r.respect_dnd &&
    r.quiet_hours
  return !same
}

/** `alerts::validate_rule` (07 §8.2): overlay always, the three channels only, `respect_dnd = false` for session rules only. */
export function alertRuleProblem(r: Pick<AlertRuleRow, 'event' | 'channels' | 'respect_dnd'>): string | null {
  if (!r.channels.includes('overlay')) return 'the overlay channel is always on'
  if (r.channels.some((c) => c !== 'overlay' && c !== 'os_notification' && c !== 'sound')) return 'channels are overlay, os_notification, sound'
  if (!r.respect_dnd && r.event !== 'session.waiting' && r.event !== 'session.failed') return 'only session rules may ignore Focus / Do Not Disturb'
  return null
}

/** `urgency::rules::validate`: name 1–40, horizon 1–30, a regex ≤ 512 that compiles, a pattern unless deadline / labels. */
export function keywordRuleProblem(r: KeywordRule): string | null {
  const name = (r.name ?? '').trim()
  if (!name || Array.from(name).length > 40) return 'a rule name is 1–40 characters'
  if (!(r.horizon_days >= 1 && r.horizon_days <= 30)) return 'horizon_days is 1–30'
  if (r.match === 'regex') {
    if (r.pattern.length > 512) return 'a regex is at most 512 characters'
    try {
      new RegExp(r.pattern)
    } catch {
      return `${r.pattern} is not a valid regex`
    }
  }
  if ((r.match === 'contains' || r.match === 'word') && !r.pattern.trim() && !r.labels_any?.length) return 'the pattern is empty'
  return null
}

function hhmm(v: string): number | null {
  const m = /^([01]\d|2[0-3]):([0-5]\d)$/.exec(v)
  return m ? Number(m[1]) * 60 + Number(m[2]) : null
}

/** Quiet hours active at `now` (local time; a window may cross midnight). */
export function quietActive(q: { enabled: boolean; from: string; to: string } | null | undefined, now: number): boolean {
  if (!q?.enabled) return false
  const from = hhmm(q.from)
  const to = hhmm(q.to)
  if (from === null || to === null || from === to) return false
  const d = new Date(now)
  const m = d.getHours() * 60 + d.getMinutes()
  return from < to ? m >= from && m < to : m >= from || m < to
}

/** The fixed ids of the premade alert rules the log rows name. */
const RULE_ID = { waiting: '00000000-0000-7000-8000-000000000201', urgent: '00000000-0000-7000-8000-000000000204', meeting: '00000000-0000-7000-8000-000000000205' }

/** Commands reject with a plain `{ code, message }` object, as the core does (`CmdError`); `fields` names a draft's fields. */
function cmdError(code: string, message: string, fields?: FieldError[]): CmdError & { fields?: FieldError[] } {
  return fields ? { code, message, fields } : { code, message }
}

let seq = 0
/** `pv-1`, `pv-2`: a fresh id for a row the mock creates. */
function nid(prefix: string): string {
  seq += 1
  return `${prefix}-${seq}`
}

function emitSettings(patch: SettingsMap) {
  mock().emit(EVENTS.settings, patch)
}

function emitHealth(now: number) {
  mock().emit(EVENTS.health, health(now))
}

/**
 * Seeds and registers the package 9 commands. `vault` (the notes mock) lets
 * the groupings and tags edits show up in `groupings_list` / `tags_list`.
 */
export function installSettingsMock(opts: { now?: number; vault?: { groupingsList(): GroupingsList; tagsList(): TagRow[]; foldersList(): FoldersList; saveGrouping(d: GroupingDraftWithPosition): GroupingRow } } = {}): void {
  const now0 = opts.now ?? Date.now()
  const os = osFromLocation()
  const values: SettingsMap = {
    ...registryDefaults(os),
    'device.name': 'Ana’s MacBook Pro',
    'profile.name': 'Ana',
    // The sample user's picture, bundled; the app stores a file path here and `fileSrc` maps it to the asset protocol.
    'profile.picture_path': anaPicture,
    'overlay.placement': 'top-bar',
    'sessions.terminal_app': 'Ghostty',
    'usage.budget_tokens_week': 3_000_000,
    'onboarding.completed_at': iso(now0 - 30 * 86_400_000),
    'onboarding.steps': { placement: 'done', vault: 'done', hooks: 'done' },
  }
  st = {
    values,
    hooks: 'installed',
    claudeFound: true,
    failApply: null,
    secret: true,
    backups: [
      { path: `${DATA_DIR}/backups/mode-20261020T020000Z.db`, size_bytes: 4_521_984, created_at: iso(now0 - 36 * 3_600_000) },
      { path: `${DATA_DIR}/backups/mode-20261019T020000Z.db`, size_bytes: 4_456_448, created_at: iso(now0 - 60 * 3_600_000) },
    ],
    positions: new Map(),
    deletedGroupings: new Set(),
    tagNames: new Map(),
    deletedTags: new Set(),
    days: sampleDays(localDate(now0)),
    live: [],
    quit: [],
    aiState: null,
    providers: seedProviders(),
    routes: seedRoutes(),
    alerts: [
      { id: 'al-1', rule_id: RULE_ID.urgent, rule_name: 'Urgent message', event: 'message.received', item_key: 'slack:T0ACME:C01REL:1729503660.000100', channels: ['overlay'], suppressed_reason: null, fired_at: iso(now0 - 4 * 3_600_000), created_at: iso(now0 - 4 * 3_600_000) },
      { id: 'al-2', rule_id: RULE_ID.meeting, rule_name: 'Meeting starting', event: 'calendar.starting', item_key: 'event:e-review', channels: ['overlay'], suppressed_reason: 'quiet_hours', fired_at: null, created_at: iso(now0 - 3 * 3_600_000) },
      { id: 'al-3', rule_id: RULE_ID.waiting, rule_name: 'Session needs you', event: 'session.waiting', item_key: 'session:s-api:toolu_01', channels: ['overlay'], suppressed_reason: null, fired_at: iso(now0 - 60_000), created_at: iso(now0 - 60_000) },
    ],
    // `last_fired_at` follows the delivered rows of the log above.
    alertRules: premadeAlertRules().map((r) => ({ ...r, rev: 1, last_fired_at: r.id === RULE_ID.urgent ? iso(now0 - 4 * 3_600_000) : r.id === RULE_ID.waiting ? iso(now0 - 60_000) : null })),
    failRuleAdd: null,
    failRuleDelete: null,
    ruleSeq: 0,
    failRuleSet: null,
    folderKinds: {},
  }
  const m = mock()
  const s = st
  const now = () => Date.now()
  const on = (cmd: string, fn: (a: Record<string, unknown>) => unknown) => m.respond(cmd, (args: Args) => fn(args ?? {}))

  on('settings_get_all', () => JSON.parse(JSON.stringify(s.values)))
  on('settings_set', (a) => {
    const key = String(a.key)
    const why = validateSetting(key, a.value)
    if (why) throw cmdError('invalid', why)
    if (GLOBAL_BY_KEY.has(key) && typeof a.value === 'string' && TAKEN.has(a.value)) throw cmdError('invalid', `${a.value} could not be registered: taken by the system`)
    s.values[key] = a.value
    emitSettings({ [key]: a.value })
    return a.value
  })
  on('settings_reset', (a) => {
    const key = String(a.key)
    s.values[key] = defaultFor(key, os)
    emitSettings({ [key]: s.values[key] })
    return s.values[key]
  })
  on('settings_export', (a) => {
    const sections = (a.sections as string[] | undefined) ?? []
    if (!sections.length) throw cmdError('invalid', 'pick at least one section')
    return { path: `${HOME}/Downloads/mode-export-${localDate(now())}.json` }
  })
  on('settings_import', (a) => {
    const report: ImportReport = {
      path: `${HOME}/Downloads/mode-export-2026-10-01.json`,
      applied: a.apply === true,
      entries: {
        'overlay.max_rows': { outcome: 'updated' },
        'shortcuts.bindings': { outcome: 'unchanged' },
        'urgency.keyword_rules': { outcome: 'new' },
        'profile.name': { outcome: 'unchanged' },
        'notes.sort': { outcome: 'skipped', reason: 'device-scope key' },
      },
    }
    if (a.apply === true) {
      s.values['overlay.max_rows'] = 8
      s.values['urgency.keyword_rules'] = [{ id: 'rule-outage', name: 'Outage', enabled: true, field: 'subject', match: 'contains', pattern: 'outage', effect: 'raise', horizon_days: 3, position: 0 }]
      emitSettings({ 'overlay.max_rows': 8, 'urgency.keyword_rules': s.values['urgency.keyword_rules'] })
    }
    return report
  })
  on('health_get', () => health(now()))
  on('platform_info', (): PlatformInfo => ({
    os,
    notch: false,
    // As the core: a display's id is the OS's monitor name (what `overlay.display` stores).
    displays: [
      { id: 'Built-in Retina Display', name: 'Built-in Retina Display', main: true, notch: false },
      { id: 'DELL U2720Q', name: 'DELL U2720Q', main: false, notch: false },
    ],
    reduced_motion: false,
    terminal_apps: os === 'macos' ? ['Terminal.app', 'iTerm2', 'Ghostty'] : os === 'windows' ? ['Windows Terminal', 'PowerShell', 'WSL'] : ['GNOME Terminal', 'Ghostty'],
  }))
  on('diagnostics_get', () => diagnostics(now()))
  on('logs_tail', (a) => ({ path: `${DATA_DIR}/logs/mode.log`, lines: logLines(now(), Math.min(10_000, Number(a.lines ?? 2000))) }))
  const stamp = () => iso(now()).replace(/[^0-9A-Za-z]/g, '')
  on('diagnostics_export', () => ({ path: `${DATA_DIR}/diagnostics/mode-diagnostics-${stamp()}.json` }))
  on('db_backup', () => {
    const b = { path: `${DATA_DIR}/backups/mode-backup-${stamp()}.db`, size_bytes: 4_718_592, created_at: iso(now()) }
    s.backups = [b, ...s.backups].slice(0, 5)
    return b
  })
  on('retention_sweep', (a) => {
    const dry = a.dry_run === true
    return { dry_run: dry, session_events_deleted: 412, session_events_compacted: 0, sessions_deleted: 0, usage_samples_deleted: 0, messages_deleted: 0, tombstones_purged: {}, backups_removed: [], deleted: { session_events: 412 } }
  })
  on('vault_pick', () => ({ path: `${HOME}/Documents/Notes`, md_files: 42 }))
  on('vault_inspect', (a) => {
    const p = String(a.path ?? '').trim().replace(/^~(?=\/|$)/, HOME)
    if (p.endsWith('/Mode/Notes')) return { path: p, exists: true, md_files: 27 }
    if (p.endsWith('/Documents/Notes')) return { path: p, exists: true, md_files: 42 }
    return { path: p, exists: false, md_files: 0 }
  })
  on('profile_picture_pick', () => ({ path: null, url: null }))

  on('hooks_status', () => hookStatus(now()))
  on('hooks_install_preview', (a) => plan(a.action === 'uninstall' ? 'uninstall' : 'install'))
  on('hooks_install_apply', (): HooksApplied => {
    if (s.failApply) {
      const why = s.failApply
      s.failApply = null
      throw cmdError('io', why)
    }
    s.hooks = 'installed'
    emitHealth(now())
    return { settings_path: SETTINGS_PATH, backup_path: `${DATA_DIR}/backups/settings.json.mode-backup-${iso(now()).replace(/[-:]/g, '').slice(0, 15)}Z`, block_hash: 'b3f1c2a9', bytes_written: 4096, written: true }
  })
  on('hooks_uninstall', (): HooksApplied => {
    s.hooks = 'none'
    emitHealth(now())
    return { settings_path: SETTINGS_PATH, backup_path: `${DATA_DIR}/backups/settings.json.mode-backup-${iso(now()).replace(/[-:]/g, '').slice(0, 15)}Z`, block_hash: '', bytes_written: 512, written: true }
  })
  on('hooks_round_trip', (): RoundTripReport => {
    if (s.hooks === 'none' || s.hooks === 'missing') throw cmdError('unavailable', 'no frame reached the core within 1000 ms')
    return { session_id: `mode-round-trip-${now()}`, latency_ms: 9, acked: true, relay_version: '0.1.0', at: iso(now()) }
  })

  // Secrets: the two AI entries, and the OAuth clients for `secret_status` only (`mode_app::commands_ai::status_entry`:
  // `oauth_client:google`, `oauth_client:slack`, `oauth_client:slack:<team_id>`; `secret_set` refuses them, they are
  // written through `oauth_client_set`). Whether a client is stored is the connectors mock's word when it is installed.
  const OAUTH_CLIENT = /^oauth_client:(google|slack)(?::[A-Za-z0-9_-]+)?$/
  const oauthStatus = (name: string) => {
    const m = OAUTH_CLIENT.exec(name)
    if (!m || name.length > 64 || (m[1] === 'google' && name !== 'oauth_client:google')) throw cmdError('invalid', `"${name}" is not a secret Mode keeps`)
    const clients = window.__mode_connectors?.state().oauthClients
    return { name, stored: clients ? clients.has(m[1]!) : false, updated_at: null }
  }
  const SECRET_NAMES: Record<string, 'anthropic_api_key' | 'local_token'> = { anthropic_api_key: 'anthropic_api_key', 'ai:anthropic_api_key': 'anthropic_api_key', local_token: 'local_token', 'ai:local_token': 'local_token' }
  const secretEntry = (name: unknown) => {
    const n = SECRET_NAMES[String(name)]
    if (!n) throw cmdError('invalid', `"${String(name)}" is not a secret Mode keeps`)
    return n
  }
  const keyStatus = (name: string) => (name === 'anthropic_api_key' ? { name, stored: s.secret, updated_at: s.secret ? iso(now0 - 9 * 86_400_000) : null, ...(s.secret ? { last4: '4f2a' } : {}) } : { name, stored: false, updated_at: null })
  const setKey = (name: string, value: unknown) => {
    const v = typeof value === 'string' ? value.trim() : ''
    if (name === 'anthropic_api_key') s.secret = v.length > 0
    return name === 'anthropic_api_key' ? { name, stored: s.secret, updated_at: iso(now()), ...(s.secret ? { last4: v.slice(-4) } : {}) } : { name, stored: v.length > 0, updated_at: iso(now()) }
  }
  on('secret_status', (a) => (String(a.name).startsWith('oauth_client:') ? oauthStatus(String(a.name)) : keyStatus(secretEntry(a.name))))
  on('secret_set', (a) => setKey(secretEntry(a.name), a.value))

  // AI backends (`mode_core::connectors::api`): the selected backend, the key, the local servers.
  const backendNow = (): AiBackend => ((s.values['ai.backend'] as AiBackend) ?? 'claude_api')
  const localUrl = (b: AiBackend) => (backendNow() === b ? String(s.values['ai.local_url'] ?? DEFAULT_URL[b]) : DEFAULT_URL[b])
  const reachable = (b: AiBackend) => b === 'ollama'
  const aiHealth = (): AiHealth => {
    const b = backendNow()
    const local = b !== 'claude_api' || s.values['ai.local_only'] === true
    const kind: AiBackend = b === 'claude_api' && local ? 'ollama' : b
    const used = 1_300_000
    const budget = Number(s.values['ai.budget_tokens_month'] ?? 2_000_000)
    let state: AiHealth['state'] = s.aiState ?? (kind === 'claude_api' ? (s.secret ? 'ready' : 'no_key') : reachable(kind) ? 'ready' : 'unreachable')
    if (!s.aiState && budget > 0 && used >= budget) state = 'budget_reached'
    const message = { ready: 'ready', no_key: 'no API key', unreachable: `${BACKEND_LABEL[kind]} is not running`, invalid_key: 'the API key was refused', budget_reached: 'the monthly budget is reached', local_only_blocked: 'local-only is on' }[state]
    return { backend: kind, provider: kind, label: BACKEND_LABEL[kind], local_only: s.values['ai.local_only'] === true, state, message, address: kind === 'claude_api' ? null : addressClass(localUrl(kind)), models: kind === 'ollama' ? [...OLLAMA_MODELS] : [], tokens_used_month: used, budget_tokens_month: budget, backups: 0 }
  }
  on('ai_backends_list', (): AiBackendRow[] =>
    (['claude_api', 'ollama', 'lm_studio'] as const).map((b) => ({
      backend: b,
      label: BACKEND_LABEL[b],
      selected: backendNow() === b,
      local: b !== 'claude_api',
      key_stored: b === 'claude_api' ? s.secret : null,
      key_last4: b === 'claude_api' && s.secret ? '4f2a' : null,
      url: b === 'claude_api' ? null : localUrl(b),
      address: b === 'claude_api' ? null : addressClass(localUrl(b)),
    })),
  )
  on('ai_backend_set', (a) => {
    const patch: SettingsMap = {}
    if (typeof a.local_url === 'string') {
      const cls = addressClass(a.local_url)
      if (cls !== 'loopback' && cls !== 'private') throw cmdError('invalid', 'a local backend must be on this computer or your private network')
      patch['ai.local_url'] = a.local_url.replace(/\/+$/, '')
    }
    if (typeof a.backend === 'string') {
      if (!(a.backend in BACKEND_LABEL)) throw cmdError('invalid', `${a.backend} is not a backend`)
      if (a.backend !== 'claude_api' && typeof a.local_url !== 'string' && a.backend !== s.values['ai.backend']) patch['ai.local_url'] = DEFAULT_URL[a.backend as AiBackend]
      patch['ai.backend'] = a.backend
    }
    if (typeof a.local_only === 'boolean') patch['ai.local_only'] = a.local_only
    for (const k of ['model_scoring', 'model_summary', 'model_chat', 'local_model_scoring', 'local_model_summary', 'local_model_chat']) if (typeof a[k] === 'string') patch[`ai.${k}`] = a[k]
    Object.assign(s.values, patch)
    if (Object.keys(patch).length) emitSettings(patch)
    return aiHealth()
  })
  on('ai_key_set', (a) => {
    const backend = String(a.backend)
    if (!(backend in BACKEND_LABEL)) throw cmdError('invalid', `${backend} is not a backend`)
    const r = setKey(backend === 'claude_api' ? 'anthropic_api_key' : 'local_token', a.key)
    return { name: backend === 'claude_api' ? 'ai:anthropic_api_key' : 'ai:local_token', stored: r.stored, ...('last4' in r ? { last4: r.last4 } : {}) }
  })
  on('ai_health', () => aiHealth())
  on('ai_backends_status', (): AiBackendsStatus => ({
    backend: backendNow(),
    claude_api: { key_stored: s.secret },
    ollama: { url: localUrl('ollama'), reachable: true, models: [...OLLAMA_MODELS] },
    lm_studio: { url: localUrl('lm_studio'), reachable: false, models: [], error: 'connection refused' },
    tokens_used_month: 1_300_000,
  }))
  on('ai_test_call', (a) => {
    const backend = String(a.backend ?? backendNow())
    if (backend === 'lm_studio') return { ok: false, latency_ms: null, error: 'connection refused' }
    if (backend === 'claude_api' && !s.secret) return { ok: false, latency_ms: null, error: 'no API key' }
    return { ok: true, latency_ms: backend === 'ollama' ? 180 : 420, error: null }
  })

  // Providers and routes (06 §10.1A; `mode_core::connectors::ai::providers`).
  const providerUrl = (p: ProviderRecord): string | null => p.base_url ?? (p.kind === 'ollama' || p.kind === 'lm_studio' ? localUrl(p.kind) : (PROVIDER_KINDS.find((k) => k.kind === p.kind)?.default_url ?? null))
  const keyStored = (p: ProviderRecord) => (p.builtin && p.kind === 'claude_api' ? s.secret : p.key)
  const providerRow = (p: ProviderRecord): AiProviderRow => {
    const { key: _k, last4, agents, ...rest } = p
    const url = providerUrl(p)
    const kind = PROVIDER_KINDS.find((k) => k.kind === p.kind)!
    return {
      ...rest,
      url,
      address: url ? addressClass(url) : 'invalid',
      key_stored: keyStored(p),
      key_last4: p.builtin && p.kind === 'claude_api' ? (s.secret ? '4f2a' : null) : p.key ? last4 : null,
      callable: kind.callable,
      session_capable: kind.session_capable,
      routed: s.routes.filter((r) => r.provider_id === p.id || r.fallbacks.some((t) => t.provider_id === p.id)).map((r) => r.purpose),
      agents,
    }
  }
  const providerBy = (id: unknown) => {
    const p = s.providers.find((x) => x.id === String(id))
    if (!p) throw cmdError('not_found', 'no such provider')
    return p
  }
  on('ai_provider_kinds', () => PROVIDER_KINDS.map((k) => ({ ...k, auths: [...k.auths] })))
  on('ai_providers_list', (): AiProviderRow[] => s.providers.map(providerRow))
  on('ai_provider_save', (a) => {
    const d = a.draft as AiProviderDraft
    const existing = d.id ? s.providers.find((p) => p.id === d.id) : undefined
    if (d.id && !existing) throw cmdError('not_found', 'The provider is gone.')
    const kind = existing?.kind ?? d.kind
    const k = PROVIDER_KINDS.find((x) => x.kind === kind)
    if (!k) throw cmdError('invalid', `${String(kind)} is not a kind`)
    const fields: FieldError[] = []
    if (!d.name.trim()) fields.push({ field: 'name', message: 'Give the provider a name.' })
    if (!k.auths.includes(d.auth)) fields.push({ field: 'auth', message: `${k.label} takes ${k.auths.join(' or ')}.` })
    const url = d.base_url?.trim() || null
    if (url && !existing?.builtin) {
      const cls = addressClass(url)
      if (cls === 'invalid') fields.push({ field: 'base_url', message: 'Expects an http:// or https:// address.' })
      else if (cls === 'public' && (kind === 'ollama' || kind === 'lm_studio')) fields.push({ field: 'base_url', message: 'A local server must be on this computer or your private network.' })
      else if (cls === 'public' && url.startsWith('http://')) fields.push({ field: 'base_url', message: 'A remote endpoint needs https:// (http:// only on this computer or your private network).' })
    } else if (!url && kind === 'openai_compat' && !existing?.builtin) fields.push({ field: 'base_url', message: "Give the endpoint's address." })
    if ((kind === 'bedrock' || kind === 'vertex') && !d.extra.region?.trim()) fields.push({ field: 'extra.region', message: kind === 'bedrock' ? 'Give the AWS region.' : 'Give the Vertex region.' })
    if (kind === 'vertex' && !d.extra.project_id?.trim()) fields.push({ field: 'extra.project_id', message: 'Give the Google Cloud project id.' })
    if (kind === 'foundry' && !d.extra.resource?.trim() && !url) fields.push({ field: 'extra.resource', message: 'Give the Foundry resource name or an address.' })
    const models = d.models.map((m) => m.trim()).filter(Boolean)
    if (new Set(models).size !== models.length) fields.push({ field: 'models', message: 'A model is listed twice.' })
    if (fields.length) throw cmdError('invalid', 'The provider cannot be saved as it is.', fields)
    const row: ProviderRecord = {
      ...(existing ?? { id: nid('pv'), kind, position: s.providers.length, builtin: false, key: false, last4: null, agents: 0 }),
      name: d.name.trim(),
      base_url: existing?.builtin ? null : url?.replace(/\/+$/, '') ?? null,
      auth: d.auth,
      models,
      extra: Object.fromEntries(Object.entries(d.extra).filter(([, v]) => typeof v === 'string' && v.trim())),
      enabled: d.enabled,
    }
    s.providers = existing ? s.providers.map((p) => (p.id === row.id ? row : p)) : [...s.providers, row]
    return providerRow(row)
  })
  on('ai_provider_delete', (a) => {
    const p = providerBy(a.id)
    if (p.builtin) throw cmdError('invalid', `${p.name} is built in and stays.`)
    s.providers = s.providers.filter((x) => x.id !== p.id)
    s.routes = s.routes.map((r) => ({ ...r, provider_id: r.provider_id === p.id ? null : r.provider_id, model: r.provider_id === p.id ? null : r.model, fallbacks: r.fallbacks.filter((t) => t.provider_id !== p.id) }))
    return true
  })
  on('ai_provider_key_set', (a) => {
    const p = providerBy(a.id)
    const v = typeof a.key === 'string' ? a.key.trim() : ''
    if (p.builtin) {
      const r = setKey(p.kind === 'claude_api' ? 'anthropic_api_key' : 'local_token', v || null)
      return { name: p.kind === 'claude_api' ? 'ai:anthropic_api_key' : 'ai:local_token', stored: r.stored, ...('last4' in r ? { last4: r.last4 } : {}) }
    }
    p.key = v.length > 0
    p.last4 = v.length >= 8 ? v.slice(-4) : null
    return { name: `ai:provider:${p.id}`, stored: p.key, last4: p.last4 }
  })
  on('ai_provider_test', (a) => {
    const p = providerBy(a.id)
    const k = PROVIDER_KINDS.find((x) => x.kind === p.kind)!
    if (!k.callable) return { ok: false, latency_ms: null, error: `${k.label} runs sessions; launch an agent on it to try it` }
    if (p.kind === 'lm_studio') return { ok: false, latency_ms: null, error: 'connection refused' }
    if (p.auth !== 'none' && !keyStored(p)) return { ok: false, latency_ms: null, error: p.kind === 'claude_api' ? 'Add your Anthropic API key' : 'no key' }
    return { ok: true, latency_ms: p.kind === 'ollama' ? 180 : p.builtin ? 420 : 610, error: null }
  })
  on('ai_provider_models', (a) => {
    const p = providerBy(a.id)
    // The named models first, then what the endpoint lists (the core's `GET /v1/models`): the local
    // server's models, or the Claude catalog once the built-in Claude API has its key.
    const listed = p.kind === 'ollama' ? OLLAMA_MODELS : p.builtin && p.kind === 'claude_api' && keyStored(p) ? CLAUDE_MODELS : []
    return [...p.models, ...listed.filter((m) => !p.models.includes(m))]
  })
  on('ai_routes_list', () => s.routes.map((r) => ({ ...r, fallbacks: r.fallbacks.map((t) => ({ ...t })) })))
  on('ai_route_set', (a) => {
    const r = a.route as AiRoute
    const fields: FieldError[] = []
    const callable = (id: string): string | null => {
      const p = s.providers.find((x) => x.id === id)
      if (!p) return 'No such provider.'
      if (!PROVIDER_KINDS.find((k) => k.kind === p.kind)?.callable) return `${p.name} can run sessions, not Mode's own calls.`
      if (!p.enabled) return `${p.name} is turned off.`
      return null
    }
    if (r.provider_id) {
      const why = callable(r.provider_id)
      if (why) fields.push({ field: 'provider_id', message: why })
    }
    if (r.fallbacks.length > 3) fields.push({ field: 'fallbacks', message: 'At most 3 backups.' })
    for (const t of r.fallbacks) {
      const why = callable(t.provider_id)
      if (why) fields.push({ field: 'fallbacks', message: why })
    }
    if (fields.length) throw cmdError('invalid', 'The route cannot be saved as it is.', fields)
    const saved: AiRoute = { purpose: r.purpose, provider_id: r.provider_id ?? null, model: r.provider_id ? r.model?.trim() || null : null, fallbacks: r.fallbacks.map((t) => ({ provider_id: t.provider_id, model: t.model?.trim() || null })) }
    s.routes = s.routes.map((x) => (x.purpose === saved.purpose ? saved : x))
    return saved
  })

  // The connector cards with nothing connected (the connectors mock answers with its accounts).
  const notConnected = { state: 'not_connected' as const, glyph: '○', line: 'not connected', accounts: 0 }
  on('connectors_status', (): ConnectorsStatus => ({ shipped: true, offline: false, gmail: notConnected, calendar: notConnected, slack: notConnected, accounts: [] }))
  on('claude_code_card', (a): ClaudeCodeCard => {
    const h = s.hooks
    if (h === 'installed') return { state: 'connected', glyph: '◉', line: 'hooks installed · 7 sessions', hooks: 'installed', sessions_tracked: 7, sessions_today: 3 }
    if (h === 'none') return { state: 'not_connected', glyph: '○', line: a.binary_found === false || !s.claudeFound ? 'Claude Code not found' : 'hooks not installed', hooks: 'none', sessions_tracked: 7, sessions_today: 3 }
    return { state: 'needs_attention', glyph: '✕', line: `hooks ${h} — reinstall`, hooks: h, sessions_tracked: 7, sessions_today: 3 }
  })

  // Keyword rules (07 §3.3): stored in `urgency.keyword_rules`; the premade three are seeded on the first list.
  const rulesOf = () => (Array.isArray(s.values['urgency.keyword_rules']) ? (s.values['urgency.keyword_rules'] as KeywordRule[]) : [])
  on('keyword_rules_list', () => {
    if (!rulesOf().length && !s.rulesSeeded) {
      s.rulesSeeded = true
      s.values['urgency.keyword_rules'] = premadeKeywordRules()
      emitSettings({ 'urgency.keyword_rules': s.values['urgency.keyword_rules'] })
    }
    return rulesOf().map((r) => ({ ...r }))
  })
  on('keyword_rules_set', (a) => {
    const rules = ((a.rules as KeywordRule[] | undefined) ?? []).map((r, i) => ({ ...r, name: (r.name ?? '').trim(), id: r.id?.trim() ? r.id : `rule-${now()}-${i}`, position: i }))
    for (const r of rules) {
      const why = keywordRuleProblem(r)
      if (why) throw cmdError('invalid', `${r.name}: ${why}`)
    }
    s.rulesSeeded = true
    s.values['urgency.keyword_rules'] = rules
    emitSettings({ 'urgency.keyword_rules': rules })
    return rules.map((r) => ({ ...r }))
  })
  on('urgency_rescore', () => 14)
  on('urgency_diagnostics', () => ({ counters: { batches: 31, scored: 412, cache_hits: 88, twin_copies: 6, injection_flags: 1, fallbacks: { rule: 3 }, tokens: 1_300_000 }, queue_depth: 2, writebacks_queued: 0 }))

  // Alerts and quiet hours (07 §8).
  const quietPayload = () => ({ quiet_hours: quietActive(s.values['notifications.quiet_hours'] as { enabled: boolean; from: string; to: string } | null, now()), focus: false, presentation: false, paused: typeof s.values['notifications.pause_until'] === 'string' && Date.parse(String(s.values['notifications.pause_until'])) > now() })
  on('quiet_set', (a) => {
    const q = a as QuietSetArgs
    const patch: SettingsMap = {}
    if (typeof q.pause_minutes === 'number') patch['notifications.pause_until'] = q.pause_minutes > 0 ? iso(now() + q.pause_minutes * 60_000) : null
    if (q.quiet_hours) {
      if (hhmm(q.quiet_hours.from) === null || hhmm(q.quiet_hours.to) === null) throw cmdError('invalid', 'quiet hours are HH:MM')
      patch['notifications.quiet_hours'] = { enabled: !!q.quiet_hours.enabled, from: q.quiet_hours.from, to: q.quiet_hours.to }
    }
    Object.assign(s.values, patch)
    if (Object.keys(patch).length) emitSettings(patch)
    const payload = quietPayload()
    mock().emit(EVENTS.quiet, payload)
    return payload
  })
  on('alerts_recent', (a) => s.alerts.slice(0, Math.max(1, Math.min(500, Number(a.limit ?? 50)))).map((x) => ({ ...x, channels: [...x.channels] })))
  on('alert_dismiss', (a) => (a.item_key ? [String(a.item_key)] : []))
  const ruleOut = (r: AlertRuleRow & { rev: number }): AlertRuleRow => {
    const { rev: _rev, ...row } = r
    return { ...row, filters: JSON.parse(JSON.stringify(row.filters)) as AlertRuleRow['filters'], channels: [...row.channels], edited: alertRuleEdited(r) }
  }
  const liveRules = () => [...s.alertRules].sort((x, y) => x.position - y.position || x.name.localeCompare(y.name))
  const nameTaken = (name: string, selfId: string) => s.alertRules.some((x) => x.id !== selfId && ruleNameKey(x.name) === ruleNameKey(name))
  /** `connectors::api::rule_name`: trimmed, 1–60 characters. */
  const ruleName = (raw: unknown): string => {
    const n = String(raw ?? '').trim()
    if (!n || Array.from(n).length > 60) throw cmdError('invalid', 'a rule name is 1–60 characters')
    return n
  }
  const checkCooldown = (cd: unknown): number => {
    const n = Number(cd)
    if (!(Number.isInteger(n) && n >= 0 && n <= 86_400)) throw cmdError('invalid', 'the cooldown is 0–86400 seconds')
    return n
  }
  /** `connectors::api::rule_channels`: `overlay` first and always, no repeats (unknown names stay for the check to refuse). */
  const ruleChannels = (ch: unknown): AlertChannel[] => Array.from(new Set(['overlay', ...(Array.isArray(ch) ? ch.map(String) : [])])) as AlertChannel[]
  /** `alerts::validate_rule` over the whole row. */
  const validateRule = (r: AlertRuleRow) => {
    const why = alertFiltersProblem(r.event, r.filters) ?? alertRuleProblem(r)
    if (why) throw cmdError('invalid', why)
  }
  /** Filters as serde reads them: `null` is absent. */
  const filtersIn = (f: unknown): AlertRuleFilters => Object.fromEntries(Object.entries(f && typeof f === 'object' ? (f as Record<string, unknown>) : {}).filter(([, v]) => present(v))) as AlertRuleFilters
  on('alert_rules_list', () => liveRules().map(ruleOut))
  // `connectors::api::alert_rule_set`: reset first, then each field, validated as a whole; every save bumps `rev`.
  on('alert_rule_set', (a) => {
    const args = a as unknown as AlertRuleSetArgs
    const stored = s.alertRules.find((r) => r.id === args.id)
    if (!stored) throw cmdError('not_found', 'no such rule')
    if (s.failRuleSet) {
      const why = s.failRuleSet
      s.failRuleSet = null
      throw cmdError('invalid', why)
    }
    const r = { ...stored, filters: { ...stored.filters }, channels: [...stored.channels] }
    if (args.reset === true) {
      const seed = premadeAlertRules().find((x) => x.id === r.id || x.name === r.name)
      if (!seed) throw cmdError('invalid', 'only a premade rule can be reset')
      Object.assign(r, { name: seed.name, enabled: seed.enabled, event: seed.event, filters: { ...seed.filters }, channels: ['overlay'] as AlertChannel[], sound: seed.sound, respect_dnd: true, quiet_hours: true, cooldown_seconds: seed.cooldown_seconds, position: seed.position })
    }
    if (typeof args.enabled === 'boolean') r.enabled = args.enabled
    if (typeof args.name === 'string') {
      const n = ruleName(args.name)
      if (n !== r.name && nameTaken(n, r.id)) throw cmdError('invalid', `a rule named ${JSON.stringify(n)} exists`)
      r.name = n
    }
    if (Array.isArray(args.channels)) r.channels = ruleChannels(args.channels)
    if (typeof args.sound === 'string') r.sound = args.sound.trim() || null
    if (typeof args.cooldown_seconds === 'number') r.cooldown_seconds = checkCooldown(args.cooldown_seconds)
    if (typeof args.respect_dnd === 'boolean') r.respect_dnd = args.respect_dnd
    if (typeof args.quiet_hours === 'boolean') r.quiet_hours = args.quiet_hours
    if (args.filters && typeof args.filters === 'object') r.filters = filtersIn(args.filters)
    if (typeof args.position === 'number') r.position = args.position
    validateRule(r)
    r.rev = stored.rev + 1
    s.alertRules = s.alertRules.map((x) => (x.id === r.id ? r : x))
    return ruleOut(r)
  })
  // `connectors::api::alert_rule_add` (03 §8.11): name, then its uniqueness, then the cooldown, then the whole rule;
  // every refusal `invalid`; enabled, `kind: custom`, placed after the last live rule.
  on('alert_rule_add', (a) => {
    const args = a as unknown as AlertRuleAddArgs
    if (s.failRuleAdd) {
      const { code, message } = s.failRuleAdd
      s.failRuleAdd = null
      throw cmdError(code, message)
    }
    const name = ruleName(args.name)
    if (nameTaken(name, '')) throw cmdError('invalid', `a rule named ${JSON.stringify(name)} exists`)
    const cooldown = checkCooldown(args.cooldown_seconds ?? 0)
    const position = s.alertRules.reduce((max, x) => Math.max(max, x.position), -1) + 1
    s.ruleSeq += 1
    const r: AlertRuleRow & { rev: number } = {
      id: `00000000-0000-7000-8000-${String(900 + s.ruleSeq).padStart(12, '0')}`,
      name,
      kind: 'custom',
      enabled: true,
      event: String(args.event ?? '').trim(),
      filters: filtersIn(args.filters),
      channels: ruleChannels(args.channels),
      sound: typeof args.sound === 'string' && args.sound.trim() ? args.sound.trim() : null,
      respect_dnd: args.respect_dnd ?? true,
      quiet_hours: args.quiet_hours ?? true,
      cooldown_seconds: cooldown,
      position,
      last_fired_at: null,
      edited: false,
      rev: 1,
    }
    validateRule(r)
    s.alertRules = [...s.alertRules, r]
    return ruleOut(r)
  })
  // `connectors::api::alert_rule_delete`: a custom rule is tombstoned (its log rows stay); premade ones refuse.
  on('alert_rule_delete', (a): AlertRuleDeleted => {
    if (s.failRuleDelete) {
      const { code, message } = s.failRuleDelete
      s.failRuleDelete = null
      throw cmdError(code, message)
    }
    const r = s.alertRules.find((x) => x.id === String(a.id ?? ''))
    if (!r) throw cmdError('not_found', 'no such rule')
    if (r.kind === 'premade') throw cmdError('premade', 'premade rules can be turned off or reset, not deleted')
    s.alertRules = s.alertRules.filter((x) => x.id !== r.id)
    return { deleted: true }
  })
  // Settings → Alerts → Test: the rule's channels, or the first suppression that applies (reported, not applied).
  on('alert_rule_test', (a): AlertOutcome => {
    const rule = s.alertRules.find((r) => r.id === a.rule_id)
    if (!rule) throw cmdError('not_found', 'no such rule')
    const q = quietPayload()
    const suppressed = q.paused ? 'paused' : q.quiet_hours && rule.quiet_hours ? 'quiet_hours' : null
    return {
      event: rule.event,
      item_key: `test:${iso(now())}`,
      rules: [{ rule_id: rule.id, rule_name: rule.name, channels: suppressed ? ['overlay'] : [...rule.channels], suppressed_reason: suppressed }],
      sound: !suppressed && rule.channels.includes('sound') ? rule.sound : null,
      notification: null,
      overlay_only: false,
    }
  })

  on('hotkey_set', (a) => {
    const name = String(a.name)
    const keys = (a.keys ?? null) as string | null
    const def = CATALOG.find((d) => d.id === name)
    if (!def) throw cmdError('not_found', `${name} is not a shortcut`)
    if (def.settingKey) {
      if (keys && TAKEN.has(keys)) return { name, keys: s.values[def.settingKey] as string | null, registered: false, reason: 'taken by the system' }
      s.values[def.settingKey] = keys
      emitSettings({ [def.settingKey]: keys })
      return { name, keys, registered: true }
    }
    const bindings = { ...((s.values['shortcuts.bindings'] as Record<string, string | null>) ?? {}), [name]: keys }
    s.values['shortcuts.bindings'] = bindings
    emitSettings({ 'shortcuts.bindings': bindings })
    return { name, keys, registered: true }
  })

  on('stats_days', (a) => {
    const from = String(a.from)
    const to = String(a.to)
    return Array.from(s.days.values()).filter((d) => d.date >= from && d.date <= to)
  })
  on('stats_summary', (a): StatsSummary => sampleSummary(s.days, String(a.from), String(a.to), localDate(now())))

  // Notes settings: applied over the notes mock's own answers when it is there.
  const vault = opts.vault
  if (vault) {
    on('groupings_list', () => {
      const g = vault.groupingsList()
      let rows = g.groupings.filter((r) => !s.deletedGroupings.has(r.id)).map((r) => (s.positions.has(r.id) ? { ...r, position: s.positions.get(r.id)! } : r))
      rows = rows.slice().sort((x, y) => x.position - y.position)
      const statuses = s.values['notes.statuses'] as StatusDef[]
      return { ...g, groupings: rows, statuses: Array.isArray(statuses) ? statuses : g.statuses }
    })
    // `grouping_save` with `position` reorders (03 §8.8); the rest goes to the notes mock's vault.
    on('grouping_save', (a) => {
      const draft = a.grouping as GroupingDraftWithPosition
      if (draft.id && typeof draft.position === 'number') s.positions.set(draft.id, draft.position)
      const row = vault.saveGrouping(draft)
      return s.positions.has(row.id) ? { ...row, position: s.positions.get(row.id)! } : row
    })
    on('tags_list', () =>
      vault
        .tagsList()
        .filter((t) => !s.deletedTags.has(t.name_norm))
        .map((t) => (s.tagNames.has(t.name_norm) ? { ...t, name: s.tagNames.get(t.name_norm)!, name_norm: s.tagNames.get(t.name_norm)!.toLowerCase() } : t)),
    )
  }
  on('grouping_delete', (a) => {
    s.deletedGroupings.add(String(a.id))
    return undefined
  })
  on('notes_reindex', () => ({ files: 27, changed: 0, tombstoned: 0, diagnostics: [] }))
  on('statuses_save', (a) => {
    const why = validateSetting('notes.statuses', a.statuses)
    if (why) throw cmdError('invalid', why)
    s.values['notes.statuses'] = a.statuses
    emitSettings({ 'notes.statuses': a.statuses })
    return a.statuses
  })
  on('status_move', () => ({ written: 2, stopped: null }))
  // Renaming onto an existing tag is the merge (03 §8.9).
  on('tag_rename', (a) => {
    const from = String(a.from).toLowerCase()
    const to = String(a.to)
    const names = vault ? vault.tagsList().map((t) => (s.tagNames.get(t.name_norm) ?? t.name).toLowerCase()) : []
    if (names.includes(to.toLowerCase())) s.deletedTags.add(from)
    else s.tagNames.set(from, to)
    return { written: 3, stopped: null }
  })
  on('tag_delete', (a) => {
    s.deletedTags.add(String(a.name).toLowerCase())
    return { written: 3, stopped: null }
  })
  on('data_delete_all', (a) => {
    if (a.confirm !== 'delete') throw cmdError('invalid', 'type delete to confirm')
    if (s.live.length) throw cmdError('busy', `Close the ${s.live.length} running terminal${s.live.length === 1 ? '' : 's'} first.`)
    return undefined
  })

  // Quit (03 §2.1, `mode_app::quit`): ask when embedded sessions are live.
  on('pty_list', () => s.live.map((x) => ({ ...x })))
  on('app_hide', () => undefined)
  on('app_quit', (a): QuitAnswer => {
    if (a.confirm === false) {
      s.quit.push('cancel')
      return { status: 'cancelled', sessions: [] }
    }
    if (a.confirm === true) {
      s.quit.push('confirm')
      return { status: 'quitting', sessions: [] }
    }
    s.quit.push('ask')
    if (!s.live.length) return { status: 'quitting', sessions: [] }
    const sessions = s.live.map((x) => ({ ...x }))
    mock().emit('app:quit-confirm', { sessions })
    return { status: 'confirm', sessions }
  })

  // The capture / templates folders (03 §8.7) over the notes mock's folders.
  const vaultFolders = opts.vault
  if (vaultFolders) {
    const withKinds = (): FoldersList => {
      const list = vaultFolders.foldersList()
      const k = s.folderKinds
      if (k.capture === undefined && k.templates === undefined) return list
      return {
        ...list,
        folders: list.folders.map((f) => {
          let kind = f.kind
          if (k.capture !== undefined) kind = f.path === k.capture ? 'capture' : kind === 'capture' ? 'normal' : kind
          if (k.templates !== undefined) kind = k.templates !== null && f.path === k.templates ? 'templates' : kind === 'templates' ? 'normal' : kind
          return { ...f, kind }
        }),
      }
    }
    on('folders_list', () => withKinds())
    on('notes_folders_set', (a) => {
      const list = vaultFolders.foldersList()
      const inVault = (raw: string) => {
        const rel = raw.trim().replace(/^\/+|\/+$/g, '')
        if (!rel) throw cmdError('invalid', 'pick a folder inside the notes folder, not the folder itself')
        if (rel.split('/').some((p) => p.startsWith('.'))) throw cmdError('invalid', 'hidden folders cannot hold notes')
        if (!list.folders.some((f) => f.path === rel)) throw cmdError('not_found', `${raw.trim()} does not exist in the notes folder`)
        return rel
      }
      const capture = typeof a.capture === 'string' ? inVault(a.capture) : undefined
      const templates = typeof a.templates === 'string' ? (a.templates.trim() ? inVault(a.templates) : null) : undefined
      if (capture === undefined && templates === undefined) throw cmdError('invalid', 'name a capture or templates folder')
      if (capture && templates && capture === templates) throw cmdError('invalid', 'the capture and templates folders must differ')
      if (capture !== undefined) s.folderKinds.capture = capture
      if (templates !== undefined) s.folderKinds.templates = templates
      return withKinds()
    })
  }

  if (typeof window !== 'undefined') {
    window.__mode_settings = {
      hooks(h) {
        s.hooks = h
        emitHealth(now())
      },
      claude(found) {
        s.claudeFound = found
        emitHealth(now())
      },
      failNextApply(message) {
        s.failApply = message
      },
      freshInstall() {
        s.values['onboarding.steps'] = {}
        s.values['onboarding.completed_at'] = null
        s.hooks = 'none'
        emitSettings({ 'onboarding.steps': {}, 'onboarding.completed_at': null })
        emitHealth(now())
      },
      liveSessions(list) {
        s.live = list.map((x) => ({ ...x }))
      },
      aiState(state) {
        s.aiState = state
      },
      failNextRuleSet(message) {
        s.failRuleSet = message
      },
      failNextRuleAdd(message, code = 'invalid') {
        s.failRuleAdd = { code, message }
      },
      failNextRuleDelete(code, message) {
        s.failRuleDelete = { code, message }
      },
      alertRules: () => liveRules().map(ruleOut),
      state: () => ({ values: s.values, hooks: s.hooks, quit: s.quit }),
    }
  }
}
