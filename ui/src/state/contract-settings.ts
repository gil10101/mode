// Package 9 commands: Settings, Integrations, Shortcuts, Stats, Diagnostics,
// onboarding and the profile row (03 §2.3.1, §2.8, §7A, §7C, §7D, §8).
//
// Reconciled with the app crate (`crates/mode-app/src/{commands.rs,
// commands_settings.rs, commands_diag.rs, commands_pty.rs, quit.rs,
// health.rs, secrets.rs}`) and, for the AI backends and the connectors, with
// `mode_core::connectors::api` (contract-connectors.ts). Field names are the
// snake_case wire names; the shapes mirror the Rust structs verbatim.
// Optional fields are ones the core omits when empty or that older builds
// lack. Every command rejects with `CmdError` (contract.ts).
import type { AppHealth } from './contract-sessions'
import type { LiveSession } from './contract-terminal'

/* ---- settings registry (09 §5.5) ------------------------------------------ */

export type SettingScope = 'user' | 'device'

/** `settings_get_all`: every registered key with its effective value (stored, else the default). */
export type SettingsMap = Record<string, unknown>

/** `settings.changed` as the app window receives it: a partial map of registry keys. */
export type SettingsChanged = Partial<SettingsMap>

/* ---- hooks (05 §3.6, 03 §7C.2–§7C.3) ------------------------------------- */

/** `hook_installs.status` plus "not installed" (`InstallStatus`); `none` is the health spelling of the same. */
export type HookInstallStatus = 'installed' | 'stale' | 'missing' | 'conflict' | 'removed' | 'not_installed' | 'none'

export interface HookConflict {
  event: string
  command: string
  matcher: string | null
}

export interface RoundTripReport {
  session_id: string
  latency_ms: number
  acked: boolean
  relay_version: string | null
  /** RFC 3339. */
  at: string
}

/** `hooks_status` (`mode_core::hooks_install::Status`). */
export interface HooksStatus {
  installed: boolean
  status: HookInstallStatus
  /** The relay version the file's command reports. */
  version: string | null
  /** The settings file. */
  path: string
  relay_path: string | null
  conflicts: HookConflict[]
  events: string[]
  block_hash: string | null
  detail: string | null
  last_round_trip: RoundTripReport | null
  /** `hook_installs.installed_at` / `verified_at` / `backup_path` for the facts table (03 §7C.3; `commands::StatusView`). */
  installed_at?: string | null
  verified_at?: string | null
  backup_path?: string | null
}

export type HookTargetKind = { kind: 'native' } | { kind: 'wsl'; distro: string }

export interface HookTarget {
  settings_path: string
  relay_path: string
  command: string
  kind: HookTargetKind
}

export interface HookEventLine {
  event: string
  timeout: number
  async: boolean
  matcher: string | null
}

export type HookAction = 'install' | 'uninstall'

/** `hooks_install_preview` (`Plan`). */
export interface HooksPlan {
  target: HookTarget
  action: HookAction
  file_existed: boolean
  before: string
  after: string
  /** Unified diff of before → after. */
  diff: string
  changed: boolean
  conflicts: HookConflict[]
  events: HookEventLine[]
  block_hash: string
  /** Where the backup will go (`<app_data_dir>/backups`) and the relay version, for the dialog header (`commands::PlanView`). */
  backup_dir?: string
  relay_version?: string | null
}

/** `hooks_install_apply` / `hooks_uninstall` (`Applied`). */
export interface HooksApplied {
  settings_path: string
  backup_path: string | null
  block_hash: string
  bytes_written: number
  written: boolean
}

/* ---- diagnostics and logs (03 §8.16, §8.17) ------------------------------- */

export interface HookInstallRow {
  scope: string
  settings_path: string
  status: HookInstallStatus
  relay_version: string | null
  installed_at: string | null
  verified_at: string | null
  backup_path: string | null
}

/** `mode_core::db::BackupFile` (`db_backup`, Diagnostics → Database). */
export interface DbBackup {
  path: string
  size_bytes: number
  /** The file's modification time, RFC 3339 UTC. */
  created_at: string
}

/** `diagnostics_get` (`mode_app::commands_diag::Diagnostics`, 03 §8.16). */
export interface Diagnostics {
  /** `core_status` (`mode_core::diag::Snapshot`). */
  core: {
    version: string
    dev_build: boolean
    data_dir: string
    db_path: string
    endpoint: string
    ipc_listening: boolean
    hook_frames_received: number
  }
  /** `claude --version` and where it was found ("not found" when absent). */
  claude: { found: boolean; path: string | null; version: string | null }
  /** The relay as staged by the app. */
  relay: { path: string | null; version: string | null }
  hooks: HookInstallRow[]
  ipc: {
    endpoint: string
    listening: boolean
    last_event_at: string | null
    /** Last 20 hook events, newest first: name, time, latency only. */
    recent: { event: string; at: string; latency_ms: number | null }[]
    /** The IPC server's live counters (frames, acks, decisions, timeouts …); null before it listens. */
    metrics?: Record<string, unknown> | null
  }
  index: { vault_path: string; files_indexed: number; last_full_scan_at: string | null; pending_touchups: number }
  database: {
    path: string
    size_bytes: number
    wal_bytes: number
    tables: { name: string; rows: number }[]
    last_sweep_at: string | null
    backups: DbBackup[]
    /** The database belongs to a newer Mode: nothing is written. */
    read_only?: boolean
  }
  /** Sessions with hook activity, not ended (the Claude Code card's count line). */
  sessions_tracked: number
  /** Top-level sessions started today. */
  sessions_today: number
  /** Embedded sessions with a live process. */
  sessions_embedded?: number
  /** `os` (the keyring) or `file` (the development store). */
  secrets_store?: 'os' | 'file' | string
  /** `logs/mode.log`, when the app logs to a file. */
  log_path?: string | null
}

/** `logs_tail { lines? }` (2 000 by default, at most 10 000): the last lines of `logs/mode.log`, oldest first, each `<RFC 3339>  LEVEL target: message`. */
export interface LogsTail {
  path: string
  lines: string[]
}

/* ---- export / import (03 §8.18; `mode_core::settings::ImportReport`) -------- */

export type ExportSection = 'groupings' | 'alert_rules' | 'contacts' | 'themes' | 'settings'

export type ImportOutcome = { outcome: 'new' | 'updated' | 'unchanged' } | { outcome: 'skipped'; reason: string }

/** `settings_import` (`commands_settings::ImportView`): `path` is null when the open dialog was canceled. */
export interface ImportReport {
  path: string | null
  applied: boolean
  entries: Record<string, ImportOutcome>
}

/** `settings_export` / `diagnostics_export` (`WrittenFile`): where the file went; null when the save dialog was canceled. */
export interface WrittenFile {
  path: string | null
}

/** `retention_sweep { dry_run? }` (`SweepResult`): `mode_core::retention::Report` flattened, plus `deleted` (rows per table, non-zero only). */
export interface SweepResult {
  dry_run: boolean
  session_events_deleted?: number
  sessions_deleted?: number
  messages_deleted?: number
  usage_samples_deleted?: number
  backups_removed?: string[]
  deleted: Record<string, number>
  [field: string]: unknown
}

/* ---- secrets and AI backends (03 §8.14, §7C.2; ADR-0020) ------------------- */

/** Secrets never reach the webview: only "stored" comes back (XC-22); the app's store adds `updated_at`, the core's keyring the last four characters of an API key. */
export interface SecretStatus {
  name: string
  stored: boolean
  updated_at?: string | null
  last4?: string | null
}

export type AiBackend = 'claude_api' | 'ollama' | 'lm_studio'

export interface LocalBackendStatus {
  url: string
  reachable: boolean
  models: string[]
  error?: string | null
}

/** `ai_backends_status` (`mode_core::connectors::api::AiBackendsStatus`): both local servers probed, for the AI backends card's lines. */
export interface AiBackendsStatus {
  backend: AiBackend
  claude_api: { key_stored: boolean }
  ollama: LocalBackendStatus
  lm_studio: LocalBackendStatus
  /** Mode's own AI calls this calendar month (tokens). */
  tokens_used_month: number
}

/** `ai_test_call { backend? }` (`AiTestResult`). */
export interface AiTestResult {
  ok: boolean
  latency_ms: number | null
  error?: string | null
}

/* ---- connectors (v0.2; 03 §7C.2, §7C.5) ------------------------------------ */

/** `connectors_status`: the core's shape lives in contract-connectors.ts. */
export type { ConnectorsStatus } from './contract-connectors'

/* ---- shortcuts (03 §7D) ----------------------------------------------------- */

export interface HotkeySetResult {
  name: string
  /** The chord stored (`Alt+Shift+Space`), or null when cleared. */
  keys: string | null
  /** Global hotkeys only: whether the OS accepted the registration. */
  registered: boolean
  /** "taken by the system" and similar when `registered` is false. */
  reason?: string | null
}

/* ---- stats (03 §7A) -------------------------------------------------------- */

/** One day of Q11: top-level sessions started + file changes first seen, on the device's local date. */
export interface StatsDay {
  /** `YYYY-MM-DD`. */
  date: string
  sessions: number
  files: number
  /** `usage_samples` with `source = 'claude_code'`; tooltip only. */
  tokens: number
}

export type SessionStateCount = 'waiting' | 'working' | 'failed' | 'finished' | 'idle' | 'untracked'

export interface TokenSplit {
  model: string
  source: string
  tokens: number
}

/** Estimates at list price (ADR-0051); `null` when the window has tokens but none of its models is priced. */
export interface StatsSpend {
  window: number | null
  today: number | null
  month: number | null
  /** The last 400 days. */
  all: number | null
  /** Models whose tokens are counted but not priced. */
  unpriced_models: string[]
  /** The date of the price list (`YYYY-MM-DD`). */
  prices_as_of: string
}

export interface StatsModelRow {
  model: string
  source: string
  messages: number
  tokens: number
  input: number
  output: number
  cache_read: number
  cache_write: number
  spend_usd: number | null
}

export interface StatsAgentRow {
  id: string
  name: string
  avatar_variant: string
  parent_id: string | null
  model: string | null
  state: string
  messages: number
  tokens: number
  spend_usd: number | null
  duration_s: number
  started_at: string
}

export interface StatsSummaryDay {
  date: string
  sessions: number
  /** Claude Code assistant messages. */
  messages: number
  /** Every source. */
  tokens: number
  spend_usd: number | null
  split: TokenSplit[]
}

/** `StatsSummary` (03 §7A): the spend tiles, the metrics and the breakdown of one window. */
export interface StatsSummary {
  from: string
  to: string
  spend: StatsSpend
  /** Every source. */
  tokens: number
  tokens_split: { input: number; output: number; cache_read: number; cache_write: number }
  /** Claude Code assistant messages. */
  messages: number
  /** Top-level sessions started in the window. */
  sessions: number
  /** Subagent sessions started in the window. */
  agents: number
  files: number
  lines_added: number
  lines_removed: number
  /** Days with any activity, of the window's elapsed days. */
  active_days: number
  window_days: number
  /** Over the last 400 days, not the window. */
  streak: { current: number; longest: number }
  longest_session_s: number
  /** By Claude Code tokens; `share` is 0 ..= 1. */
  favorite_model: { model: string; share: number } | null
  /** The local hour with the most Claude Code messages. */
  peak_hour: { hour: number; messages: number } | null
  /** cache reads / (input + cache reads + cache writes), 0 ..= 1; `null` without input. */
  cache_hit: number | null
  by_model: StatsModelRow[]
  /** The sessions with usage in the window, by tokens (live rows only, at most 100). */
  by_agent: StatsAgentRow[]
  /** One row per day of the window. */
  days: StatsSummaryDay[]
  /** Claude Code messages per local hour, 24 entries. */
  hours: number[]
  /** Sessions with activity in the window, by state-column word. */
  by_state: Record<SessionStateCount, number>
}

/* ---- platform, vault, onboarding --------------------------------------------- */

/** One display of `platform_info`: `id` is the monitor's name as the OS reports it (`Display <n>` when it has none), the same string as `name`. */
export interface DisplayInfo {
  id: string
  name: string
  main: boolean
  notch: boolean
}

export interface PlatformInfo {
  os: 'macos' | 'windows' | 'linux'
  /** A notched built-in display exists (02 §4.1; onboarding step 1 is skipped). */
  notch: boolean
  displays: DisplayInfo[]
  reduced_motion: boolean
  /** Terminal apps found on this machine, for `sessions.terminal_app`. */
  terminal_apps: string[]
}

/** `vault_pick { current? }`: rejects `unavailable` when no folder dialog exists (type the path instead). */
export interface VaultPick {
  /** null: the dialog was canceled. */
  path: string | null
  /** `.md` files already in the folder ("Index this vault"). */
  md_files: number
}

/** `vault_inspect { path }`: `path` is the expanded one (`~` resolved). */
export interface VaultInspect {
  path: string
  exists: boolean
  md_files: number
}

/** `profile_picture_pick`: the copy under `<data dir>/profile/` (what `profile.picture_path` stores) and its asset-protocol URL; both null when canceled. Not resized. */
export interface PicturePick {
  path: string | null
  url: string | null
}

/** `app_quit {confirm?}` (`mode_app::quit::QuitAnswer`). */
export interface QuitAnswer {
  /** `quitting`: the app exits (after stopping the sessions when confirmed); `confirm`: `app:quit-confirm` was sent; `canceled`: it stays. */
  status: 'quitting' | 'confirm' | 'cancelled'
  sessions: LiveSession[]
}

/** The event the app window gets when a quit needs confirmation (`quit::QUIT_CONFIRM_EVENT`). */
export const QUIT_CONFIRM_EVENT = 'app:quit-confirm'

export interface QuitConfirm {
  sessions: LiveSession[]
}

/** `health_get` / `health.changed` (`mode_app::health::HealthChanged`). */
export interface SettingsHealth extends AppHealth {
  /** `claude --version` (`2.1.305`) when the binary answered. */
  claude_code_version?: string | null
  /** Onboarding never finished: `onboarding.completed_at` is unset (03 §2.8). */
  first_run?: boolean
}

/** A batch of note rewrites (`mode_core::notes::write::BatchReport`): `stopped` = the file that stopped it and why. */
export interface BatchReport {
  written: number
  stopped: [string, string] | null
}

/** `notes_reindex` (`mode_core::notes::api::ReindexResult`): a full scan now. */
export interface ReindexResult {
  files: number
  changed: number
  tombstoned: number
  /** `[path, problem]` pairs. */
  diagnostics: [string, string][]
}

/** `grouping_save` takes `position` too (`GroupingDraft.position` in the core): the reorder of 03 §8.8. */
export type GroupingDraftWithPosition = import('./contract-notes').GroupingDraft & { position?: number }

/** Commands reject with `{ code, message, action? }` (`mode_app::cmd_error::CmdError`); the type and the reader live in contract.ts. */
export type { CmdError } from './contract'

/* ---- the command map ----------------------------------------------------------- */

/** The package 9 commands as `Commands` entries (contract.ts), as the app crate registers them. */
export interface SettingsCommands {
  /** Validated against the registry (02 §17 ranges); returns the stored value. */
  settings_set: { args: { key: string; value: unknown }; result: unknown }
  health_get: { args: undefined; result: SettingsHealth }
  hooks_status: { args: undefined; result: HooksStatus }
  /** The install plan, or with `action: 'uninstall'` the Uninstall preview of 03 §7C.3. */
  hooks_install_preview: { args: { action?: HookAction } | undefined; result: HooksPlan }
  hooks_install_apply: { args: undefined; result: HooksApplied }
  hooks_uninstall: { args: undefined; result: HooksApplied }
  hooks_round_trip: { args: undefined; result: RoundTripReport }
  /** Full scan now (03 §8.7 Reindex vault, §8.16 Index). */
  notes_reindex: { args: undefined; result: ReindexResult }
  /** Writes `notes.statuses` (03 §8.8 custom statuses); returns the list stored. */
  statuses_save: { args: { statuses: import('./contract-notes').StatusDef[] }; result: import('./contract-notes').StatusDef[] }
  /** Moves every note from status `from` to `to` (before deleting a custom status). */
  status_move: { args: { from: string; to: string }; result: BatchReport }
  grouping_delete: { args: { id: string }; result: void }
  /** Renames a tag in every note; renaming onto an existing tag is the merge (03 §8.9). */
  tag_rename: { args: { from: string; to: string }; result: BatchReport }
  tag_delete: { args: { name: string }; result: BatchReport }
  settings_get_all: { args: undefined; result: SettingsMap }
  /** Deletes the row; returns the default (Reset this shortcut on the Global group). */
  settings_reset: { args: { key: string }; result: unknown }
  /** The core shows the save dialog (else writes under `<data dir>/exports/`) and writes `mode-export-<date>.json`; `path: null` = canceled. */
  settings_export: { args: { sections: ExportSection[]; include_device?: boolean }; result: WrittenFile }
  /** `apply: false` → the merge preview; `apply: true` with the same path writes it. `path: null` shows the open dialog (`unavailable` without one). */
  settings_import: { args: { path: string | null; apply: boolean; include_device?: boolean }; result: ImportReport }
  platform_info: { args: undefined; result: PlatformInfo }
  diagnostics_get: { args: undefined; result: Diagnostics }
  logs_tail: { args: { lines?: number } | undefined; result: LogsTail }
  /** One JSON file under `<data dir>/diagnostics/` (diagnostics, settings without secrets, hook status, the last 2 000 log lines). */
  diagnostics_export: { args: undefined; result: WrittenFile }
  /** `VACUUM INTO` `mode-backup-<ts>.db` (03 §8.16 Database). */
  db_backup: { args: undefined; result: DbBackup }
  /** The nightly sweep now, or with `dry_run` what it would do. */
  retention_sweep: { args: { dry_run?: boolean } | undefined; result: SweepResult }
  /** Folder picker through the dialog plugin; rejects `unavailable` without one (the UI falls back to a typed path). */
  vault_pick: { args: { current?: string }; result: VaultPick }
  vault_inspect: { args: { path: string }; result: VaultInspect }
  /** The capture / templates folder of 03 §8.7: vault-relative or absolute, inside the vault; `templates: ''` clears it. Returns the folders. */
  notes_folders_set: { args: { capture?: string; templates?: string }; result: import('./contract-notes').FoldersList }
  /** PNG / JPEG / GIF / WebP picker; the file is copied under `<data dir>/profile/` (not resized). */
  profile_picture_pick: { args: undefined; result: PicturePick }
  /** `anthropic_api_key` / `local_token` (or their `ai:` entry names); anything else is refused (`invalid`). */
  secret_status: { args: { name: string }; result: SecretStatus }
  /** `value: null` (or blank) removes the keyring entry. */
  secret_set: { args: { name: string; value: string | null }; result: SecretStatus }
  /** Both local servers probed (`mode_core::connectors::api::ai_backends_status`); the AI backends card's local lines. */
  ai_backends_status: { args: undefined; result: AiBackendsStatus }
  /** A minimal message to the selected backend, or `backend`; a failure answers `ok: false`, not a rejection. */
  ai_test_call: { args: { backend?: AiBackend; provider?: string; model?: string }; result: AiTestResult }
  connectors_status: { args: undefined; result: import('./contract-connectors').ConnectorsStatus }
  /** Global names write `capture.hotkey` / `overlay.toggle_hotkey` / `overlay.pin_hotkey` / `window.hotkey` and re-register; the rest write `shortcuts.bindings`. */
  hotkey_set: { args: { name: string; keys: string | null }; result: HotkeySetResult }
  /** At most 400 days. */
  stats_days: { args: { from: string; to: string }; result: StatsDay[] }
  /** At most 400 days; `by_state` folds `unread` and `done` into `finished`. */
  stats_summary: { args: { from: string; to: string }; result: StatsSummary }
  /** "Delete all Mode data" (03 §8.15): `confirm` must be `delete`; refused `busy` while terminals run and `read_only` on a newer database. */
  data_delete_all: { args: { confirm: 'delete' }; result: void }
  /** Quit (03 §2.1): without `confirm` it exits when nothing is live, else sends `app:quit-confirm` and answers `confirm`; `confirm: true` stops the sessions and exits, `false` stays. */
  app_quit: { args: { confirm?: boolean } | undefined; result: QuitAnswer }
}

/** The name of the Anthropic API key in the keyring (`secret_set` / `secret_status`). */
export const ANTHROPIC_KEY = 'anthropic_api_key'
