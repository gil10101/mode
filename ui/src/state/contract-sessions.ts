// The app window's Sessions contract (03 §3, 05 §4.5, §6, §6A, §11.5, §13).
//
// The shape was proposed here from the specs and has been reconciled with
// what the core serializes: `mode_core::sessions::views::{SessionItem,
// SessionHeader, SessionDetail, AgentRow, LinkRow, TranscriptInfo}`,
// `mode_core::sessions::thread::ThreadItem` (the `kind`-tagged Overview item)
// and `mode_core::queries::{SessionFile, ConnectorRow}`. Field names are the
// Rust names verbatim (snake_case), as in `contract.ts`. Fields marked
// "UI-side, optional" are not sent by the core (yet); the UI renders without
// them and the browser mock fills them in.
//
// Commands beyond the three added to `Commands` in contract.ts
// (`session_detail`, `session_launch`, `session_resume`) are listed in
// `SessionsCommands` below with the shapes `crates/mode-app/src/
// commands_sessions.rs` and `commands_pty.rs` answer; the browser mock
// answers all of them. Every command rejects with `CmdError` (contract.ts).
import type { HealthChanged, Pending, Question, RosterRow } from './contract'

export type Tracking = 'hooked' | 'launched' | 'discovered'
export type LaunchMode = 'external' | 'embedded' | 'headless'
export type PermissionMode = 'default' | 'plan' | 'acceptEdits' | 'auto' | 'dontAsk' | 'bypassPermissions'

export const PERMISSION_MODES: readonly PermissionMode[] = ['default', 'plan', 'acceptEdits', 'auto', 'dontAsk', 'bypassPermissions']

/** 05 §14: at most one driver per session. */
export type Driver = 'pty' | 'headless' | 'none'

/**
 * A `sessions.changed` row as the app window reads it (`SessionItem`): Q2
 * (`RosterRow`) plus the columns the date filter, history and the detail
 * panel need.
 */
export interface AppRosterRow extends RosterRow {
  /** A launched session shows `mode-launch:<uuid>` until its first hook names the real id (`isLaunchPlaceholder`). */
  claude_session_id: string
  started_at: string
  last_event_at: string
  ended_at: string | null
  acknowledged_at: string | null
  last_assistant_preview: string | null
  /** Set on history rows (`sessions_history`, `HistoryItem.dismissed_at`); roster rows are never dismissed. */
  dismissed_at?: string | null
  /** UI-side, optional: child rows for the roster nest; the core does not send them, so they load from `session_detail` on expand. */
  agents?: AgentRow[]
}

/** The header columns of 05 §11.5 plus what the tabs need; a subset of `repo::Session` (no `cost_usd`: never shown). */
export interface SessionHeader {
  id: string
  claude_session_id: string
  display_name: string
  title: string | null
  avatar_color: string
  avatar_variant: string
  state: string
  state_since: string
  state_reason: string | null
  waiting_kind: string | null
  cwd: string
  repo_root: string | null
  git_branch: string | null
  model: string | null
  permission_mode: PermissionMode | string | null
  /** The agent profile the session was launched as (03 §7E); absent on older rows. */
  profile_id?: string | null
  tracking: Tracking | string
  launch_mode: LaunchMode | string
  current_tool: string | null
  current_mcp_server: string | null
  transcript_path: string | null
  terminal_app: string | null
  last_assistant_preview: string | null
  lines_added: number
  lines_removed: number
  files_changed: number
  num_turns: number
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_write_tokens: number
  context_tokens: number | null
  started_at: string
  last_event_at: string
  ended_at: string | null
  end_reason: string | null
  acknowledged_at: string | null
  dismissed_at: string | null
}

/** How an approval or question card ended (03 §3.7, 05 §7.4). */
export type CardStatus = 'open' | 'allowed' | 'denied' | 'always' | 'answered' | 'answered_in_terminal' | 'timed_out' | 'session_ended'

/** Where a card was answered: `app` = by Mode (overlay or app window), `terminal` = in the CLI. */
export type AnsweredWhere = 'app' | 'terminal'

interface ItemBase {
  /** `session_events.seq` (or a transcript line number) — the React key and the tie-break after `at`. */
  seq: number
  /** `occurred_at`; items are sorted by it, then `seq` (05 §4.4). */
  at: string
  /** Set when the item belongs to a subagent (shown inside its activity group / thread). */
  agent_id?: string | null
}

/**
 * One item of the Overview thread (03 §3.7), oldest first, already paired and
 * de-duplicated by the core (05 §4.4). Assistant text is read from the
 * transcript on demand; nothing here is stored beyond `session_events`.
 */
export type TranscriptItem =
  | (ItemBase & { kind: 'start'; source: string; model: string | null })
  | (ItemBase & { kind: 'end'; reason: string })
  /** `text: null` = prompt previews are off ("prompt · hidden by setting"). */
  | (ItemBase & { kind: 'user'; text: string | null })
  | (ItemBase & { kind: 'assistant'; text: string })
  | (ItemBase & {
      kind: 'tool'
      tool_use_id: string
      tool_name: string
      /** The argument: path, command, pattern. */
      summary: string
      /** Right-aligned result ("84 lines", "1 failed · 23 passed"). */
      meta?: string | null
      lines_added?: number | null
      lines_removed?: number | null
      duration_ms?: number | null
      failed?: boolean
      /** Expanded body: trimmed `payload_json` (4 KB per field, redacted). */
      body?: string | null
    })
  | (ItemBase & {
      kind: 'permission'
      tool_use_id: string | null
      tool_name: string
      /** The command line or file path. */
      command: string
      status: CardStatus
      answered_where?: AnsweredWhere | null
      answered_at?: string | null
      /** The `always` rule key it created (`Bash:npm`). */
      rule?: string | null
    })
  | (ItemBase & {
      kind: 'question'
      tool_use_id: string | null
      questions: Question[]
      status: CardStatus
      /** UI-side, optional: question text → chosen label(s); the core does not send it, so answered cards show no choice filled. */
      answers?: Record<string, string | string[]> | null
      answered_where?: AnsweredWhere | null
      answered_at?: string | null
    })
  | (ItemBase & { kind: 'state'; from: string; to: string })
  | (ItemBase & { kind: 'compaction' })
  | (ItemBase & { kind: 'model_switch'; from: string | null; to: string })
  | (ItemBase & { kind: 'cwd'; cwd: string })
  /** `Notification` types other than permission_prompt / idle_prompt (auth, elicitation, quota). */
  | (ItemBase & { kind: 'notice'; text: string })
  | (ItemBase & {
      kind: 'agent'
      agent_id: string
      agent_type: string
      state: string
      started_at: string
      ended_at?: string | null
      /** One row per tool call of that agent ("Read src/auth/limiter.ts"). */
      tools: { tool_name: string; summary: string }[]
    })

export type TranscriptItemKind = TranscriptItem['kind']

/** One Files-tab row: `file_changes` (Rust `SessionFile`) plus its id for `sessions/<id>/files/<file_change_id>`. */
export interface FileRow {
  id: string
  path: string
  rel_path: string | null
  change_kind: 'added' | 'modified' | 'deleted' | 'renamed' | 'untracked' | string
  old_path: string | null
  lines_added: number
  lines_removed: number
  edit_count: number
  source: 'hook' | 'git' | 'both' | string
  last_seen_at: string
  /** UI-side, optional: the 05 §6.2 "committed" mark (not sent by the core yet). */
  committed?: boolean
}

/** One Agents-tab row (Rust `AgentRow`). */
export interface AgentRow {
  id: string
  agent_id: string | null
  agent_type: string | null
  state: string
  state_since: string
  started_at: string
  ended_at: string | null
  current_tool: string | null
  /** UI-side, optional: the argument of `current_tool` ("src/auth/limiter.ts"); not sent by the core, the row shows the tool name alone. */
  current_tool_arg?: string | null
  tools_used: number
  last_assistant_preview: string | null
  transcript_path: string | null
}

export type ConnectorKind = 'mcp_server' | 'tool_rule' | 'plugin' | 'relay'

/** One `session_connectors` row (Rust `ConnectorRow`, 05 §6A). */
export interface ConnectorRow {
  kind: ConnectorKind | string
  name: string
  status: 'connected' | 'pending' | 'failed' | 'allowed' | 'denied' | string
  /** `{"tools": 12}` · `{"origin": "always rule"}` · `{"version": "1.4"}` · `{"last_event_at": …, "relay_version": …}`; `detail` (proposed) is a ready one-line description. */
  detail_json: string | null
  seen_at: string
}

/** Parsed `detail_json`. */
export interface ConnectorDetail {
  tools?: number
  origin?: string
  version?: string
  last_event_at?: string
  relay_version?: string
  hooks?: number
  detail?: string
}

/** A row of the Links panel (03 §3.11, 09 §13 S1). */
export interface LinkRow {
  id: string
  kind: 'note' | 'message' | 'thread' | 'event' | string
  title: string
  link_type: 'created_from' | 'references' | 'prepare_for' | 'related' | 'attachment' | string
  /** The target is gone: rendered "unavailable". */
  available: boolean
}

export interface TranscriptInfo {
  path: string | null
  size_bytes: number | null
  exists: boolean
}

/** The session detail bundle (03 §3.6–§3.12). */
export interface SessionDetail {
  session: SessionHeader
  /** The open card (05 §4.3), as on the roster row. */
  pending?: Pending | null
  /** 05 §14: `pty` while Mode's terminal is attached, `headless` while a driven turn runs; the core sends `none` until the PTY host (package 8). */
  driver: Driver | string
  /** The process is alive (ended_at IS NULL and the liveness probe agrees). */
  alive: boolean
  /** Overview thread, oldest first. */
  thread: TranscriptItem[]
  /** More thread items exist before the first one (Q7 paging). */
  thread_has_more: boolean
  files: FileRow[]
  agents: AgentRow[]
  connectors: ConnectorRow[]
  /** "from this session's init event" (headless) or "from hook events so far". */
  connectors_source: 'init' | 'hooks' | string
  links: LinkRow[]
  transcript: TranscriptInfo
}

/** One block of the Transcript tab (03 §3.10): one JSONL line. */
export interface TranscriptLine {
  line: number
  at: string | null
  /** The record `type` (`user`, `assistant`, `summary`, `system`, …). */
  type: string
  /** What the block shows as its name: `user`, `assistant`, `tool_use · Bash`, `tool_result`, `summary`. */
  label: string
  /** One line of content; the expanded block shows `content`. */
  text: string
  content?: string | null
  usage?: { input_tokens: number; output_tokens: number; cache_read_tokens?: number; cache_write_tokens?: number } | null
  /** The raw JSON line ("Raw JSON" toggle). */
  raw?: string | null
  /** Set for lines from `<session-id>/subagents/<agent-id>.jsonl`. */
  agent_id?: string | null
  agent_type?: string | null
}

export interface TranscriptPage {
  path: string | null
  size_bytes: number | null
  exists: boolean
  lines: TranscriptLine[]
  /** Pages go backwards from the end (500 lines each). */
  has_more: boolean
  subagent_files: number
}

/** One line of the diff pane (`mode_core::files::diff::DiffLine`). */
export interface DiffLine {
  kind: 'hunk' | 'ctx' | 'add' | 'del'
  /** Omitted when the line has none (hunk headers; `old_no` on additions, `new_no` on deletions). */
  old_no?: number
  new_no?: number
  /** The unified-diff line as git prints it: `+…` / `-…` / ` …`, `@@ … @@`, or `commit <sha> <subject>` (a `hunk`). */
  text: string
}

export interface DiffResult {
  /** "working tree vs HEAD", "commits in this session", "last edit", "new file", "deleted", "binary file", "no diff available". */
  basis: string
  lines: DiffLine[]
  /** Cut at 3 000 lines: "Diff truncated; open the file" (always sent by the core). */
  truncated?: boolean
}

/** `session_launch` (`mode_core::pty::api::LaunchArgs`). */
export interface LaunchArgs {
  cwd: string
  permission_mode?: PermissionMode
  model?: string
  /** The agent profile to launch as (03 §7E): its flags are added; the dialog's model and permission mode win over its defaults. */
  profile_id?: string
  /** Which tab the UI opens; the core ignores it. */
  open: 'overview' | 'terminal'
  /** No terminal: the first prompt runs a headless turn (05 §8.5). */
  overview_only?: boolean
  /** Initial PTY size (the core's default is 100 × 30; `pty_open` resizes to the tab). */
  cols?: number
  rows?: number
}

/** `session_resume` (`mode_core::pty::api::ResumeArgs`). */
export interface ResumeArgs {
  session_id: string
  /** A replacement folder when the original `cwd` is gone. */
  cwd?: string
  permission_mode?: PermissionMode
  /** A model for the resumed session: the backup the agent offers when it asks before switching (03 §7E.3). */
  model?: string
  open: 'overview' | 'terminal'
  cols?: number
  rows?: number
}

/** `session_launch` / `session_resume` answer (`LaunchResult`): the roster row the launch created or reused. */
export interface LaunchResult {
  session_id: string
  /** Hooks not installed: "approvals will appear in the Terminal tab only" (03 §3.5). */
  hooks_installed: boolean
  /** The terminal did not start (05 §16): the row is `failed` with this line; the Terminal tab shows it. */
  error?: string
  /** Resume found the terminal already open (nothing was spawned). */
  already_open?: boolean
}

/** The placeholder `claude_session_id` of a launched row before its first hook (`mode-launch:<uuid>`). */
export const LAUNCH_PLACEHOLDER_PREFIX = 'mode-launch:'

/** A launched row's placeholder id is not a Claude Code session id: never offered for copying or shown. */
export function isLaunchPlaceholder(id: string | null | undefined): boolean {
  return !id || id.startsWith(LAUNCH_PLACEHOLDER_PREFIX)
}

/** The presets of 03 §4.7 that apply to sessions (`pick_date` and `until_reply` are message-only here). */
export type SnoozePreset = 'later_today' | 'tomorrow' | 'next_week'

/** `health.changed` / `health_get` as the app window reads it (`mode_app::health::HealthChanged`). */
export interface AppHealth extends HealthChanged {
  /** `~/.claude` missing and `claude` not on PATH (03 §2.7). */
  claude_code?: 'found' | 'missing'
  /** `user_version` ahead of the app (XC-33). */
  db_newer?: boolean
  backups_dir?: string
  /** Weekly tokens for the profile row's second line (03 §2.3.1). */
  tokens_week?: number
}

/** `sessions_history` arguments; every field is optional (`HistoryQuery`). */
export interface HistoryQuery {
  /** Rows per page (200 by default, at most 1 000). */
  limit?: number
  /** Paging cursor: rows dismissed before this time (the last row's `dismissed_at`). */
  before?: string
  /** Substring over name / title / folder / branch, case-insensitive. */
  query?: string
}

/** The Sessions view's commands beyond `Commands` in contract.ts, as `commands_sessions.rs` / `commands_pty.rs` answer them. */
export interface SessionsCommands {
  /** History rows (03 §3.3): **dismissed** sessions within `retention.sessions_days`, most recently dismissed first (never a roster row). */
  sessions_history: { args: HistoryQuery | undefined; result: AppRosterRow[] }
  session_undismiss: { args: { session_id: string }; result: void }
  /** A preset (03 §4.7 times, local) or `until` (UTC); neither (or `preset: null`) cancels the active snooze (Undo). Returns when the snooze ends, `null` when canceled. */
  session_snooze: { args: { session_id: string; preset?: SnoozePreset | null; until?: string | null }; result: string | null }
  session_files_refresh: { args: { session_id: string }; result: FileRow[] }
  session_diff: { args: { session_id: string; file_change_id: string }; result: DiffResult }
  /** `limit` lines (500) before `before_line` (the first page when absent), counted from the end of the transcript; `before_seq` is an alias. */
  session_transcript: { args: { session_id: string; before_line?: number; limit?: number }; result: TranscriptPage }
  /** The prompt box (05 §14): the shape lives in `contract-terminal.ts`; a refusal rejects with `{ code: 'refused', message, action? }`. */
  session_prompt: { args: { session_id: string; text: string }; result: import('./contract-terminal').PromptDelivered }
  /** The recent folders of the launch dialog: distinct `sessions.cwd`, newest first, at most 20, only folders that still exist. */
  sessions_recent_folders: { args: undefined; result: string[] }
  /** The launch dialog's "Choose…": the OS folder picker, starting in `start` when it exists; `null` when canceled; rejects `unavailable` when no picker can be shown. */
  sessions_pick_folder: { args: { start?: string }; result: string | null }
}

/** The event the core sends the app window to open a view or item (02 §16; `app:open` on the wire, `mode_app::app_window::AppOpen`). */
export const APP_OPEN_EVENT = 'app.open'

export interface AppOpen {
  view: string
  id?: string | null
  filter?: string | null
  /** Set only by jump to terminal for an embedded session. */
  tab?: string
  /** The full route the shell routes on (`sessions/<id>/terminal`, `notes/<id>`, `shortcuts?q=…`) or a `mode://` URL; the core always sends it. */
  route?: string
}
