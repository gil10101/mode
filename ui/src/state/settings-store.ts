// The settings registry of 09 §5.5 as the webview sees it, and the store the
// Settings, Integrations, Shortcuts and Stats views read: `settings_get_all`
// once, `settings_set { key, value }` per change (optimistic, rolled back on
// a refusal), `settings.changed` partials from the core. The key list,
// scopes, defaults and validation mirror `mode_core::settings::{KEYS,
// validate}`; the extra UI ranges below are this package's (03 §8) and are
// stricter, never looser, than the core's.
import { create } from 'zustand'
import { invoke, listen } from '../tauri'
import { cmdErrorText, EVENTS } from './contract'
import type { PlatformInfo, SettingScope, SettingsChanged, SettingsHealth, SettingsMap } from './contract-settings'

export type Os = PlatformInfo['os']

export interface KeyDef {
  key: string
  scope: SettingScope
  /** A JSON value, or a function of the OS for the computed defaults (hotkeys, names). */
  default: unknown | ((os: Os) => unknown)
}

const k = (key: string, scope: SettingScope, def: unknown | ((os: Os) => unknown)): KeyDef => ({ key, scope, default: def })

const hotkey = (mac: string, other: string) => (os: Os) => (os === 'macos' ? mac : other)

/** `mode_core::settings::KEYS`, in the same order. */
export const REGISTRY: readonly KeyDef[] = [
  k('device.id', 'device', null),
  k('device.name', 'device', () => 'This device'),
  k('notes.vault_path', 'device', '~/Mode/Notes'),
  k('notes.statuses', 'user', [
    { slug: 'todo', label: 'Todo', glyph: '○', color: null, closed: false },
    { slug: 'doing', label: 'Doing', glyph: '◑', color: null, closed: false },
    { slug: 'done', label: 'Done', glyph: '✓', color: null, closed: true },
  ]),
  k('notes.touchup_delay_seconds', 'user', 30),
  k('overlay.placement', 'device', null),
  k('overlay.pinned', 'device', false),
  k('overlay.peek_delay_ms', 'user', 150),
  k('capture.hotkey', 'device', hotkey('Alt+Space', 'Ctrl+Alt+Space')),
  k('sessions.fold_after_minutes', 'user', 10),
  k('sessions.store_prompt_previews', 'user', false),
  k('sessions.terminal_app', 'device', null),
  k('polling.gmail_seconds', 'user', 60),
  k('polling.slack_seconds', 'user', 30),
  k('polling.slack_budget_per_minute', 'user', 40),
  k('ai.backend', 'user', 'claude_api'),
  k('ai.local_only', 'user', false),
  k('ai.model_chat', 'user', 'claude-opus-5-5'),
  k('ai.model_scoring', 'user', 'claude-haiku-5-5'),
  k('ai.scoring_chars', 'user', 500),
  k('ai.budget_tokens_month', 'user', 2_000_000),
  k('urgency.keyword_rules', 'user', []),
  k('notifications.quiet_hours', 'user', { enabled: false, from: '22:00', to: '08:00' }),
  k('retention.session_events_days', 'user', 7),
  k('retention.sessions_days', 'user', 90),
  k('retention.messages_days', 'user', 90),
  k('retention.message_body_days', 'user', 7),
  k('retention.usage_days', 'user', 180),
  k('retention.chat_days', 'user', 90),
  k('usage.budget_tokens_week', 'user', null),
  k('usage.reset_weekday', 'user', null),
  k('theme.active', 'device', 'system'),
  k('theme.appearance', 'device', 'auto'),
  k('theme.typography_style', 'user', 'pack'),
  k('theme.font_scale', 'user', 1.0),
  k('theme.terminal_pack', 'device', 'follow'),
  k('sounds.enabled', 'user', false),
  k('sounds.volume', 'user', 60),
  k('overlay.expand_on_hover', 'user', true),
  k('overlay.expand_hover_ms', 'user', 600),
  k('overlay.collapse_delay_ms', 'user', 2500),
  k('overlay.auto_collapse_ms', 'user', 8000),
  k('overlay.pill_min_ms', 'user', 2000),
  k('overlay.max_rows', 'user', 6),
  k('overlay.default_tab', 'user', 'auto'),
  k('overlay.display', 'device', 'main'),
  k('overlay.fullscreen', 'user', 'presentation-hides'),
  k('overlay.right_edge_dock', 'device', 'float'),
  k('overlay.motion', 'user', 'system'),
  k('overlay.show_mascot', 'user', true),
  k('overlay.scale', 'device', 100),
  k('overlay.toggle_hotkey', 'device', hotkey('Alt+Shift+Space', 'Ctrl+Alt+Shift+Space')),
  k('overlay.pin_hotkey', 'device', hotkey('Alt+Shift+P', 'Ctrl+Alt+Shift+P')),
  k('overlay.capture_sound', 'user', null),
  k('overlay.idle', 'device', 'mascot'),
  k('overlay.media', 'device', true),
  k('onboarding.steps', 'device', {}),
  k('onboarding.completed_at', 'device', null),
  k('window.bounds', 'device', null),
  k('window.sidebar_collapsed', 'device', false),
  k('window.detail_open', 'device', true),
  k('window.last_route', 'device', 'sessions'),
  k('window.hotkey', 'device', null),
  k('general.launch_at_login', 'device', false),
  k('general.status_item', 'device', { badge: true, pause_entry: true }),
  k('general.compact_rows', 'device', false),
  k('general.auto_update', 'device', true),
  k('general.language', 'user', 'en'),
  k('sessions.filter', 'device', {}),
  k('sessions.group_by_repo', 'device', false),
  k('sessions.claude_path', 'device', null),
  k('sessions.custom_names', 'device', {}),
  k('hooks.decision_timeout_seconds', 'user', 120),
  k('hooks.async_timeout_seconds', 'user', 5),
  k('inbox.saved_filters', 'user', []),
  k('notes.sort', 'device', {}),
  k('notes.board_groupings', 'device', ['todo', 'doing', 'urgent', 'done']),
  k('notes.rename_with_title', 'user', false),
  k('notes.touchup_enabled', 'user', true),
  k('notes.week_start', 'user', 'monday'),
  k('notes.autosave_delay_ms', 'user', 500),
  k('notes.show_archived_in_search', 'user', false),
  k('cards.open', 'device', []),
  k('polling.calendar_seconds', 'user', 300),
  k('gmail.initial_sync_days', 'user', 14),
  k('gmail.initial_sync_max', 'user', 500),
  k('gmail.body_fetch', 'user', 'auto'),
  k('slack.initial_sync_days', 'user', 7),
  k('slack.search_mentions_seconds', 'user', 60),
  k('connectors.oauth_ports', 'device', [48311, 48312, 48313, 48314, 48315]),
  k('ai.local_url', 'user', 'http://127.0.0.1:11434'),
  k('ai.model_summary', 'user', 'claude-haiku-5-5'),
  k('ai.local_model_chat', 'user', null),
  k('ai.local_model_scoring', 'user', null),
  k('ai.local_model_summary', 'user', null),
  k('ai.timeout_seconds', 'user', { scoring: 30, summary: 120, chat: 300 }),
  k('marketplace.index_url', 'user', 'https://raw.githubusercontent.com/gil10101/mode_cat/main/index.json'),
  k('urgency.direct_email_requires_known_contact', 'user', true),
  k('urgency.pill_message_max_age_hours', 'user', 24),
  k('notifications.max_alert_age_minutes', 'user', 30),
  k('notifications.snooze_wake_on_reply', 'user', true),
  k('notifications.pause_until', 'device', null),
  k('notifications.os_preview', 'user', 'full'),
  k('notifications.os_sound', 'user', false),
  k('notifications.focus_detection', 'user', { macos_assertions: true, windows_wnf: true }),
  k('profile.name', 'user', () => 'You'),
  k('profile.picture_path', 'device', null),
  k('shortcuts.bindings', 'user', {}),
  k('automations.catch_up_minutes', 'user', 30),
  k('dictation.enabled', 'device', false),
  k('dictation.hotkey', 'device', 'Ctrl+Alt+D'),
  k('dictation.model', 'device', 'base.en'),
  k('dictation.language', 'device', 'auto'),
  k('dictation.commands', 'device', true),
  k('dictation.reply_card', 'device', true),
  k('dictation.unfocused', 'device', 'capture'),
  k('dictation.thread_id', 'device', null),
  k('chat.sources_open', 'device', false),
  k('maintenance.last_sweep_at', 'device', null),
]

const BY_KEY = new Map(REGISTRY.map((d) => [d.key, d]))

/** Keys only the core writes (`INTERNAL_KEYS`). */
export const INTERNAL_KEYS: readonly string[] = ['device.id', 'maintenance.last_sweep_at']

/** The global hotkeys (02 §8.2, 03 §7D Global group), the dictation hold key last (XC-37). */
export const HOTKEY_KEYS: readonly string[] = ['capture.hotkey', 'overlay.toggle_hotkey', 'overlay.pin_hotkey', 'window.hotkey', 'dictation.hotkey']

export function keyDef(key: string): KeyDef | undefined {
  return BY_KEY.get(key)
}

export function defaultFor(key: string, os: Os = 'macos'): unknown {
  const d = BY_KEY.get(key)
  if (!d) return null
  return typeof d.default === 'function' ? (d.default as (os: Os) => unknown)(os) : clone(d.default)
}

const STABLE = new Map<string, unknown>()

/**
 * The default of a key as one shared value per key and OS. Selectors must
 * return the same reference for the same state: a fresh copy on every call
 * (an array or object default before `settings_get_all` answered) makes
 * `useSyncExternalStore` see a change on every render and loops the view.
 * Treat it as read-only; `defaultFor` hands out copies to mutate.
 */
export function stableDefault(key: string, os: Os = 'macos'): unknown {
  const id = `${os}\u0000${key}`
  if (!STABLE.has(id)) STABLE.set(id, defaultFor(key, os))
  return STABLE.get(id)
}

export function scopeOf(key: string): SettingScope | undefined {
  return BY_KEY.get(key)?.scope
}

/** Every key with its default (what `settings_get_all` returns on a fresh database). */
export function registryDefaults(os: Os = 'macos'): SettingsMap {
  return Object.fromEntries(REGISTRY.map((d) => [d.key, defaultFor(d.key, os)]))
}

function clone<T>(v: T): T {
  return v === null || typeof v !== 'object' ? v : (JSON.parse(JSON.stringify(v)) as T)
}

/* ---- validation (mirrors `settings::validate`, plus the UI ranges) ---------- */

function kindOf(v: unknown): string {
  if (v === null) return 'null'
  if (Array.isArray(v)) return 'array'
  return typeof v
}

function intIn(v: unknown, lo: number, hi: number): string | null {
  if (typeof v !== 'number' || !Number.isInteger(v)) return 'expects a whole number'
  if (v < lo || v > hi) return `${v} is outside ${lo}–${hi}`
  return null
}

function oneOf(v: unknown, allowed: readonly string[]): string | null {
  return typeof v === 'string' && allowed.includes(v) ? null : `expects one of ${allowed.join(', ')}`
}

function shortString(v: unknown, max: number): string | null {
  if (typeof v !== 'string') return 'expects a string'
  if (!v.trim() || Array.from(v).length > max) return `expects 1–${max} characters`
  return null
}

const bool = (v: unknown) => (typeof v === 'boolean' ? null : 'expects true or false')

/** Ranges the core enforces (02 §17). */
const CORE_RULES: Record<string, (v: unknown) => string | null> = {
  'overlay.placement': (v) => (v === null ? null : oneOf(v, ['notch', 'top-bar', 'right-edge'])),
  'overlay.pinned': bool,
  'overlay.expand_on_hover': bool,
  'overlay.show_mascot': bool,
  'overlay.peek_delay_ms': (v) => intIn(v, 0, 1000),
  'overlay.expand_hover_ms': (v) => intIn(v, 300, 2000),
  'overlay.collapse_delay_ms': (v) => intIn(v, 500, 10_000),
  'overlay.auto_collapse_ms': (v) => (v === 0 ? null : intIn(v, 3000, 60_000) && `${intIn(v, 3000, 60_000)} (or 0 for never)`),
  'overlay.pill_min_ms': (v) => intIn(v, 500, 10_000),
  'overlay.max_rows': (v) => intIn(v, 4, 10),
  'overlay.default_tab': (v) => oneOf(v, ['auto', 'sessions', 'inbox', 'notes', 'calendar', 'music']),
  'overlay.display': (v) => shortString(v, 200),
  'overlay.fullscreen': (v) => oneOf(v, ['presentation-hides', 'show', 'hide']),
  'overlay.right_edge_dock': (v) => oneOf(v, ['float', 'appbar']),
  'overlay.motion': (v) => oneOf(v, ['system', 'reduced', 'full']),
  'overlay.scale': (v) => intIn(v, 100, 150),
  'overlay.capture_sound': (v) => (v === null ? null : shortString(v, 64)),
  'overlay.idle': (v) => oneOf(v, ['mascot', 'hidden']),
  'overlay.media': bool,
  'retention.chat_days': (v) => intIn(v, 7, 3650),
  'window.hotkey': (v) => (v === null ? null : shortString(v, 64)),
  'capture.hotkey': (v) => shortString(v, 64),
  'overlay.toggle_hotkey': (v) => shortString(v, 64),
  'overlay.pin_hotkey': (v) => shortString(v, 64),
  'dictation.hotkey': (v) => shortString(v, 64),
  'dictation.enabled': bool,
  'dictation.commands': bool,
  'dictation.reply_card': bool,
  'chat.sources_open': bool,
  'dictation.model': (v) => oneOf(v, ['tiny.en', 'base.en', 'small.en', 'base']),
  'dictation.language': (v) => shortString(v, 16),
  'dictation.unfocused': (v) => oneOf(v, ['capture', 'nothing']),
  'dictation.thread_id': (v) => (v === null ? null : shortString(v, 64)),
  'sessions.fold_after_minutes': (v) => intIn(v, 1, 1440),
}

const nonNegInt = (hi: number) => (v: unknown) => intIn(v, 0, hi)

/** This package's ranges for the controls of 03 §8 (the core checks only the JSON kind of these). */
const UI_RULES: Record<string, (v: unknown) => string | null> = {
  'profile.name': (v) => shortString(v, 80),
  'notes.vault_path': (v) => shortString(v, 1024),
  'notes.touchup_delay_seconds': (v) => intIn(v, 0, 3600),
  'retention.sessions_days': (v) => intIn(v, 1, 3650),
  'retention.session_events_days': (v) => intIn(v, 1, 3650),
  'retention.usage_days': (v) => intIn(v, 1, 3650),
  'retention.messages_days': (v) => intIn(v, 1, 3650),
  'retention.message_body_days': (v) => intIn(v, 1, 3650),
  'hooks.decision_timeout_seconds': (v) => intIn(v, 10, 600),
  'hooks.async_timeout_seconds': (v) => intIn(v, 1, 60),
  'ai.backend': (v) => oneOf(v, ['claude_api', 'ollama', 'lm_studio']),
  'ai.scoring_chars': (v) => intIn(v, 100, 4000),
  'ai.budget_tokens_month': nonNegInt(1_000_000_000),
  'ai.local_url': (v) => (typeof v === 'string' && /^https?:\/\/[^\s]+$/.test(v) ? null : 'expects an http:// or https:// address'),
  'marketplace.index_url': (v) => (typeof v === 'string' && /^https:\/\/[^\s]+$/.test(v) ? null : 'expects an https:// address'),
  'usage.budget_tokens_week': (v) => (v === null ? null : intIn(v, 1, 1_000_000_000)),
  'usage.reset_weekday': (v) => (v === null ? null : oneOf(v, ['monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday', 'sunday'])),
  'theme.appearance': (v) => oneOf(v, ['light', 'dark', 'auto']),
  'general.language': (v) => oneOf(v, ['en']),
  'urgency.pill_message_max_age_hours': (v) => intIn(v, 1, 720),
  'notifications.os_preview': (v) => oneOf(v, ['full', 'sender', 'none']),
  'notifications.quiet_hours': (v) => {
    const q = v as { enabled?: unknown; from?: unknown; to?: unknown } | null
    if (!q || typeof q !== 'object') return 'expects { enabled, from, to }'
    if (typeof q.enabled !== 'boolean') return 'enabled expects true or false'
    const hhmm = /^([01]\d|2[0-3]):[0-5]\d$/
    if (typeof q.from !== 'string' || !hhmm.test(q.from) || typeof q.to !== 'string' || !hhmm.test(q.to)) return 'expects times as HH:MM'
    return null
  },
  'notes.statuses': (v) => {
    if (!Array.isArray(v) || !v.length) return 'expects at least one status'
    const slugs = new Set<string>()
    for (const s of v as { slug?: unknown; label?: unknown; glyph?: unknown }[]) {
      if (typeof s?.slug !== 'string' || !/^[a-z0-9][a-z0-9-]{0,31}$/.test(s.slug)) return 'a status slug is lowercase letters, digits and dashes'
      if (slugs.has(s.slug)) return `"${s.slug}" is listed twice`
      slugs.add(s.slug)
      if (typeof s.label !== 'string' || !s.label.trim()) return 'every status needs a label'
      if (typeof s.glyph !== 'string' || Array.from(s.glyph).length !== 1) return 'a glyph is one character'
    }
    for (const fixed of ['todo', 'doing', 'done']) if (!slugs.has(fixed)) return `${fixed} cannot be deleted`
    return null
  },
  // `mode_core::urgency::rules::KeywordRule` (`match`: contains | word | regex | deadline); `regex: true` is the older form.
  'urgency.keyword_rules': (v) => {
    if (!Array.isArray(v)) return 'expects a list of rules'
    for (const r of v as { pattern?: unknown; regex?: unknown; match?: unknown; labels_any?: unknown }[]) {
      const free = r?.match === 'deadline' || (Array.isArray(r?.labels_any) && r.labels_any.length > 0)
      if (typeof r?.pattern !== 'string' || (!r.pattern.trim() && !free)) return 'every rule needs a pattern'
      if (r.regex || r.match === 'regex') {
        try {
          new RegExp(r.pattern)
        } catch {
          return `"${r.pattern}" is not a valid pattern`
        }
      }
    }
    return null
  },
}

/** Why `value` cannot be stored under `key` (the core's message shape: `key: reason`), or null. */
export function validateSetting(key: string, value: unknown): string | null {
  const def = BY_KEY.get(key)
  if (!def) return `${key} is not a registered setting`
  if (INTERNAL_KEYS.includes(key)) return `${key} is managed by Mode`
  const core = CORE_RULES[key]
  let why: string | null
  if (core) why = core(value)
  else {
    const d = typeof def.default === 'function' ? (def.default as (os: Os) => unknown)('macos') : def.default
    why = d === null || value === null || kindOf(d) === kindOf(value) ? null : 'does not match the kind of its default'
  }
  if (!why && UI_RULES[key]) why = UI_RULES[key](value)
  return why ? `${key}: ${why}` : null
}

/** The reason without the `key: ` prefix, for inline field errors. */
export function reasonOf(message: string, key: string): string {
  const s = message.startsWith(`${key}: `) ? message.slice(key.length + 2) : message
  return s.charAt(0).toUpperCase() + s.slice(1)
}

/* ---- settings.changed: registry keys, or the overlay's short fields --------- */

/** `prefs::OVERLAY_SETTINGS`: the overlay window's payload uses these field names. */
export const OVERLAY_FIELDS: Record<string, string> = {
  peek_delay_ms: 'overlay.peek_delay_ms',
  expand_on_hover: 'overlay.expand_on_hover',
  expand_hover_ms: 'overlay.expand_hover_ms',
  collapse_delay_ms: 'overlay.collapse_delay_ms',
  auto_collapse_ms: 'overlay.auto_collapse_ms',
  pill_min_ms: 'overlay.pill_min_ms',
  max_rows: 'overlay.max_rows',
  default_tab: 'overlay.default_tab',
  show_mascot: 'overlay.show_mascot',
  fold_after_minutes: 'sessions.fold_after_minutes',
  idle: 'overlay.idle',
  media: 'overlay.media',
}

/** A `settings.changed` payload as registry keys; unknown fields are dropped. */
export function normaliseChanged(p: SettingsChanged | null | undefined): SettingsMap {
  const out: SettingsMap = {}
  if (!p || typeof p !== 'object') return out
  for (const [key, value] of Object.entries(p)) {
    if (BY_KEY.has(key)) out[key] = value
    else if (OVERLAY_FIELDS[key]) out[OVERLAY_FIELDS[key]] = value
  }
  return out
}

/* ---- errors ---------------------------------------------------------------- */

/** A command's rejection as text: the core's `{ code, message, action? }` (`CmdError`, read by contract.ts's `toCmdError`), an Error, a string. */
export function errorText(e: unknown): string {
  return cmdErrorText(e)
}

/* ---- the store ------------------------------------------------------------ */

export interface SettingsState {
  values: SettingsMap
  loaded: boolean
  /** `settings_get_all` failed: the views show the defaults read-only with Retry. */
  loadError: string | null
  /** Keys with a write in flight. */
  pending: Record<string, true>
  /** The last refusal per key (inline under the control). */
  errors: Record<string, string>
  health: SettingsHealth | null
  platform: PlatformInfo | null
  load(): Promise<void>
  get<T = unknown>(key: string): T
  set(key: string, value: unknown): Promise<boolean>
  reset(key: string): Promise<boolean>
  applyChanged(p: SettingsChanged | null | undefined): void
  /** Values a core command wrote itself (`ai_backend_set`, `quiet_set`, `keyword_rules_set`), applied without a write. */
  patchValues(patch: SettingsMap): void
  applyHealth(h: SettingsHealth | null): void
  clearError(key: string): void
}

function osGuess(): Os {
  if (typeof navigator === 'undefined') return 'macos'
  const p = `${navigator.platform ?? ''} ${navigator.userAgent ?? ''}`.toLowerCase()
  if (p.includes('mac')) return 'macos'
  if (p.includes('win')) return 'windows'
  return 'linux'
}

/** The OS the views render for: `platform_info` once it answered, else the browser's guess. */
export function currentOs(): Os {
  return useSettingsStore.getState().platform?.os ?? osGuess()
}

let writeSeq = 0
const latestWrite: Record<string, number> = {}

export const useSettingsStore = create<SettingsState>((set, get) => ({
  values: {},
  loaded: false,
  loadError: null,
  pending: {},
  errors: {},
  health: null,
  platform: null,

  async load() {
    try {
      const all = await invoke<SettingsMap>('settings_get_all')
      set({ values: { ...registryDefaults(currentOs()), ...(all && typeof all === 'object' ? all : {}) }, loaded: true, loadError: null })
    } catch (e) {
      set((s) => ({ values: Object.keys(s.values).length ? s.values : registryDefaults(currentOs()), loaded: true, loadError: errorText(e) }))
    }
  },

  get<T>(key: string): T {
    const v = get().values
    return (key in v ? v[key] : defaultFor(key, currentOs())) as T
  },

  async set(key, value) {
    const why = validateSetting(key, value)
    if (why) {
      set((s) => ({ errors: { ...s.errors, [key]: why } }))
      return false
    }
    const before = get().values[key]
    const seq = ++writeSeq
    latestWrite[key] = seq
    set((s) => {
      const errors = { ...s.errors }
      delete errors[key]
      return { values: { ...s.values, [key]: value }, pending: { ...s.pending, [key]: true }, errors }
    })
    try {
      const stored = await invoke<unknown>('settings_set', { key, value })
      if (latestWrite[key] === seq) {
        set((s) => {
          const pending = { ...s.pending }
          delete pending[key]
          return { values: { ...s.values, [key]: stored === undefined ? value : stored }, pending }
        })
      }
      return true
    } catch (e) {
      if (latestWrite[key] === seq) {
        set((s) => {
          const pending = { ...s.pending }
          delete pending[key]
          return { values: { ...s.values, [key]: before }, pending, errors: { ...s.errors, [key]: errorText(e) } }
        })
      }
      return false
    }
  },

  async reset(key) {
    try {
      const value = await invoke<unknown>('settings_reset', { key })
      set((s) => ({ values: { ...s.values, [key]: value === undefined ? defaultFor(key, currentOs()) : value } }))
      return true
    } catch (e) {
      set((s) => ({ errors: { ...s.errors, [key]: errorText(e) } }))
      return false
    }
  },

  applyChanged(p) {
    const patch = normaliseChanged(p)
    if (Object.keys(patch).length) set((s) => ({ values: { ...s.values, ...patch } }))
  },

  patchValues(patch) {
    const keys = Object.keys(patch).filter((k) => BY_KEY.has(k))
    if (!keys.length) return
    set((s) => ({ values: { ...s.values, ...Object.fromEntries(keys.map((k) => [k, patch[k]])) } }))
  },

  applyHealth(h) {
    set((s) => ({ health: h ? { ...(s.health ?? {}), ...h } : null }))
  },

  clearError(key) {
    set((s) => {
      if (!(key in s.errors)) return s
      const errors = { ...s.errors }
      delete errors[key]
      return { errors }
    })
  },
}))

/** One setting and its writer; re-renders only when that key changes. Before `settings_get_all` answers, the default is one stable value (`stableDefault`). */
export function useSetting<T>(key: string): [T, (value: T) => Promise<boolean>] {
  const value = useSettingsStore((s) => (key in s.values ? s.values[key] : stableDefault(key, s.platform?.os ?? osGuess()))) as T
  return [value, (v: T) => useSettingsStore.getState().set(key, v)]
}

/**
 * Subscribes the store to the core: `settings.changed`, `health.changed`,
 * then `settings_get_all`, `health_get` and `platform_info`. Returns the
 * unsubscribe function. Every call is guarded: a core without the command
 * leaves the defaults in place.
 */
export async function connectSettings(): Promise<() => void> {
  const store = useSettingsStore.getState()
  const offs = await Promise.all([
    listen<SettingsChanged>(EVENTS.settings, (p) => useSettingsStore.getState().applyChanged(p)),
    listen<SettingsHealth>(EVENTS.health, (p) => useSettingsStore.getState().applyHealth(p)),
  ])
  await Promise.all([
    store.load(),
    invoke<SettingsHealth>('health_get').then(
      (h) => {
        // A push that raced the request wins.
        if (!useSettingsStore.getState().health) useSettingsStore.getState().applyHealth(h)
      },
      () => {},
    ),
    invoke<PlatformInfo>('platform_info').then(
      (p) => useSettingsStore.setState({ platform: p }),
      () => {},
    ),
  ])
  return () => offs.forEach((off) => off())
}
