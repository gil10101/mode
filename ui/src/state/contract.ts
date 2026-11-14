// The overlay ↔ core contract (02 §16, 08 §5.3). Field names mirror the Rust
// structs verbatim (snake_case): `mode_core::queries::{PillCandidate,
// RosterRow}` and the event names of `mode_core::push::Event`. Inbox, notes
// and agenda rows are typed from 02 §7.6–§7.8 ahead of their packages.

/* ---- events: core → overlay ---------------------------------------------- */

export const EVENTS = {
  pill: 'pill.changed',
  sessions: 'sessions.changed',
  inbox: 'inbox.changed',
  notes: 'notes.changed',
  agenda: 'agenda.changed',
  quiet: 'quiet.changed',
  display: 'display.changed',
  health: 'health.changed',
  core: 'core.changed',
  hotkey: 'hotkey.fired',
  /** Proposed (not in push.rs yet): hot-zone hover, clicks outside the drawn frame and drag dwell, which only the core can see. */
  pointer: 'overlay.pointer',
  /** Proposed: `overlay.*` settings of 02 §17 as a partial object, on launch and on change. */
  settings: 'settings.changed',
  /** Dictation (XC-37, 02 §5.6): `DictationState`, to both windows, ≤ 10 Hz while listening (a stream, never coalesced). */
  dictation: 'dictation.state',
  /** A dictation model download (XC-37): `DictationModelProgress`, to both windows. */
  dictationModel: 'dictation.model',
  /** The words to insert at the caret (XC-37): `DictationText`, sent only to the focused Mode window; answer with `dictation_inserted`. */
  dictationText: 'dictation.text',
  /** A short confirmation in the notch for an action taken in the app window (02 §5.4): `OverlayFlash`, to the overlay (a stream). */
  flash: 'overlay.flash',
  /** Now playing (02 §16, OV-37): the local player's track (`MediaChanged`), to both windows (a stream). */
  media: 'media.changed',
  /** The Music tab (02 §7.10, ADR-0066): the history Mode saw playing (`MediaHistory`), after each recorded play and a clear. */
  mediaHistory: 'media.history',
} as const

/** `queries.rs` Q1 row. `kind` is one of the candidate kinds below; `ref` is the row id (null for `active_count`). */
export interface PillCandidate {
  priority: number
  kind: PillKind | string
  ref: string | null
  label: string
  at: string | null
}

export type PillKind = 'session_waiting' | 'urgent_message' | 'session_finished' | 'active_count' | 'next_event'

export interface PillChanged {
  candidate: PillCandidate | null
  /** Other candidates at the same priority (02 §5.2 "+n"). */
  count: number
}

export type SessionStateName = 'waiting' | 'working' | 'failed' | 'unread' | 'idle' | 'done' | 'untracked'
export type WaitingKind = 'permission' | 'question' | 'idle_prompt' | 'elicitation' | 'agent_needs_input'

/** The Claude Code `AskUserQuestion` shape (05 §7.3). */
export interface Question {
  question: string
  header?: string
  options?: { label: string; description?: string }[]
  multiSelect?: boolean
}

/** What a `waiting` session waits for (05 §4.3); `answerable` is false when only the notification was seen or the relay deadline passed. */
export interface Pending {
  kind: WaitingKind
  tool_name?: string
  tool_input?: unknown
  /** One line, e.g. "Bash: npm test" or the first question. */
  summary: string
  questions?: Question[]
  received_at: string
  answerable: boolean
}

/** `queries.rs` Q2 row plus `pending` for waiting rows. The three `?` fields after it are proposed additions to Q2 (02 §7.5 needs them). */
export interface RosterRow {
  id: string
  display_name: string
  avatar_color: string
  /** 05 §11.2: the accessory the bot mark wears for this session (`AVATAR_VARIANTS`), `''` for the plain mark. */
  avatar_variant: string
  state: SessionStateName | string
  waiting_kind: WaitingKind | string | null
  title: string | null
  cwd: string
  repo_root: string | null
  git_branch: string | null
  model: string | null
  current_tool: string | null
  lines_added: number
  lines_removed: number
  files_changed: number
  tracking: string
  launch_mode: string
  state_seconds: number
  folded: boolean
  live_agents: number
  pending?: Pending
  end_reason?: string | null
  state_reason?: string | null
  current_mcp_server?: string | null
}

export interface SessionsChanged {
  rows: RosterRow[]
}

/** 02 §7.6. */
export interface InboxRow {
  id: string
  provider: 'gmail' | 'slack'
  sender_name: string
  sender_avatar_url: string | null
  channel_name: string | null
  mentioned: boolean
  subject: string | null
  snippet: string | null
  received_at: string
  urgency_tier: 'vip' | 'direct' | 'ai-urgent' | 'ai-normal' | 'low' | 'unscored'
  urgency_why: string | null
  is_read: boolean
  account_label: string | null
  /** reconciliation: the message's thread (`mode_core::urgency::route::InboxRow.thread_id`, additive): `inbox/message/<id>` resolves through it. */
  thread_id?: string | null
}

export interface InboxChanged {
  rows: InboxRow[]
  urgent_count: number
  unread_in_app: number
}

/** 02 §7.7. */
export interface NoteRow {
  id: string
  title: string
  status: 'todo' | 'doing' | 'done' | string
  glyph: string | null
  urgent: boolean
  when_bucket: 'overdue' | 'today' | 'tomorrow' | 'date' | 'none'
  when_date: string | null
  tags: string[]
  task_done: number
  task_total: number
  has_links: boolean
}

export interface NotesSection {
  grouping: string
  glyph: string | null
  rows: NoteRow[]
}

export interface NotesChanged {
  sections: NotesSection[]
  overdue_count: number
}

/** 02 §7.8. */
export interface AgendaRow {
  id: string
  title: string
  starts_at: string
  ends_at: string
  all_day: boolean
  status: 'confirmed' | 'tentative' | 'cancelled' | string
  my_response: 'accepted' | 'tentative' | 'needs_action' | 'declined' | string | null
  account_color: string | null
  calendar_name: string | null
  conference: 'meet' | 'zoom' | 'teams' | 'other' | null
  join_url: string | null
  location: string | null
  section: 'now' | 'next' | 'later' | 'tomorrow'
}

export interface AgendaChanged {
  rows: AgendaRow[]
  next_2h_count: number
}

export interface QuietChanged {
  quiet_hours: boolean
  focus: boolean
  presentation: boolean
  /** reconciliation: "Pause alerts" is on (`quiet_set {pause_minutes}`; `mode_core::urgency::alerts::QuietChanged.paused`, additive). */
  paused?: boolean
}

export type Placement = 'notch' | 'top-bar' | 'right-edge'

export interface DisplayChanged {
  placement: Placement
  notch?: { width: number; height: number }
  scale: number
  dark: boolean
  reduced_motion: boolean
  /** Proposed: the work area, for the height maxima of 02 §6. */
  work_area?: { width: number; height: number }
}

export type HookStatus = 'installed' | 'stale' | 'missing' | 'conflict' | 'none'

export interface HealthChanged {
  offline: boolean
  paused_accounts: string[]
  hooks: HookStatus
  vault_missing: boolean
  read_only: boolean
  /** `needs_reauth` / `error` accounts, by label. */
  broken_accounts: string[]
}

export interface HotkeyFired {
  hotkey: 'toggle' | 'capture' | 'pin'
}

export interface OverlayPointer {
  /** `move`: the pointer near the housing while the overlay is shown (at most 20 Hz, within 600 px), for the mascot's gaze. */
  kind: 'hot_enter' | 'hot_leave' | 'outside_click' | 'drag_dwell' | 'move'
  /** `move` only: the pointer relative to the overlay window's top-left, in CSS px (may be negative or past the window). */
  x?: number
  y?: number
}

/** `overlay.flash` payload (`mode_core::push::OverlayFlash`). */
export interface OverlayFlash {
  text: string
  kind: 'ok' | 'info' | 'bad'
  /** The pill stays after the hold (a Chat turn still thinking). */
  keep_pill: boolean
  /** Where a click on the flash goes (`app_open`'s view and item). */
  open?: { view: string; id?: string }
}

/* ---- dictation (XC-37, XC-38; 02 §5.6; `mode_core::speech`) --------------- */

export type DictationStateName = 'idle' | 'listening' | 'transcribing' | 'thinking' | 'answering' | 'done' | 'error'

/** Where the words went: a focused field, a captured note, a Chat request ("Hey Mode"), or nowhere. */
export type DictationTarget = 'field' | 'note' | 'chat' | 'none'

/** `dictation.state` payload (`mode_core::speech::DictationState`). */
export interface DictationState {
  state: DictationStateName
  /** Unix ms when this state began. */
  since_ms?: number
  /** Seconds held so far (listening). */
  seconds?: number
  /** Input level 0..1 over the last 100 ms (listening). */
  level?: number
  target?: DictationTarget
  words?: number
  /** The first words of the result: the dictated text, or Chat's reply for a request. */
  text?: string
  note_id?: string
  thread_id?: string
  /** A "Hey Mode" request's Chat turn (from `thinking` on, 02 §5.7). */
  turn_id?: string
  /** A request's reply: the text so far on `answering`, all of it on `done`. */
  reply?: string
  /** A request's citations on `done`: the reply card's `[n]` chips. */
  citations?: DictationCitation[]
  /** Where a request's answer shows, decided at the release (02 §5.7, ADR-0064). */
  surface?: DictationSurface
  /** The reason, for `error` (with the OS microphone path when access was refused). */
  error?: string
  /** For `error`: the dictation itself failed (start, microphone, model, transcription) or a "Hey Mode" request did. */
  source?: DictationErrorSource
}

/** `mode_core::speech::Surface`: the pill while the app window is in front, else the overlay's reply card. */
export type DictationSurface = 'pill' | 'card'

/** `mode_core::speech::Citation`. */
export interface DictationCitation {
  n: number
  kind: string
  id: string
  title: string
}

/** What an `error` state is about (`mode_core::speech::ErrorSource`). */
export type DictationErrorSource = 'dictation' | 'request'

/** `dictation.model` payload: a model download's progress (`total` is 0 until known). */
export interface DictationModelProgress {
  model: string
  downloaded: number
  total: number
  done: boolean
  error?: string
}

/** `dictation.text` payload: insert at the caret of the focused field. */
export interface DictationText {
  text: string
}

/** `dictation_status` result. */
export interface DictationStatus {
  enabled: boolean
  /** The speech engine is compiled into this build. */
  available: boolean
  state: DictationState
  model: string
  model_installed: boolean
  hotkey: string
  error?: string
}

/** One row of `dictation_models`. */
export interface DictationModel {
  id: string
  label: string
  size_bytes: number
  installed: boolean
  multilingual: boolean
}

export interface DictationModels {
  models: DictationModel[]
}

export interface MicCheck {
  ok: boolean
  error?: string
}

/** What the calling window shows (`app_context_set`, kept per window): attached to a "Hey Mode" request, the overlay's item first, else the app window's. */
export type AppContextKind = 'note' | 'message' | 'session' | 'event'

/** The local player's track (02 §7.1, `mode_app::media::NowPlaying`); `at_ms` is the epoch ms at which `position_ms` was read. */
export interface NowPlaying {
  app: string
  playing: boolean
  title: string
  artist: string
  album: string
  /** The player's id for the track (`spotify:track:…`, a Music persistent ID), when it gives one. */
  track_id?: string | null
  artwork_url: string | null
  position_ms: number
  duration_ms: number
  at_ms: number
}

/** `media.changed`: a track, or `{}` when no player is known; `permission: 'denied'` when macOS refused the Automation permission (06 §12A). */
export type MediaChanged = (NowPlaying | { app?: undefined }) & { permission?: 'denied' }

/** A distinct track Mode saw playing (02 §7.10, `mode_core::media_history::TrackRow`); `id` is its newest play. */
export interface MediaTrackRow {
  id: string
  app: string
  track_id: string | null
  title: string
  artist: string
  album: string
  artwork_url: string | null
  duration_ms: number
  played_at: string
  plays: number
}

/** A distinct album from the same history; `id` is its most recently heard track's play (an album row plays that track). */
export interface MediaAlbumRow {
  id: string
  app: string
  album: string
  artist: string
  artwork_url: string | null
  tracks: number
  played_at: string
}

/** `media_history` and `media.history`: newest first, at most ten of each. */
export interface MediaHistory {
  tracks: MediaTrackRow[]
  albums: MediaAlbumRow[]
}

/* ---- commands: overlay → core (Tauri `invoke`) --------------------------- */

export type Decision = { behavior: 'allow' | 'deny'; always?: boolean; message?: string } | { answers: { question: string; answer: string }[] }

/** `session_decide` result (05 §7.4): the first decision for a pending item wins. */
export interface DecideResult {
  outcome: 'accepted' | 'already_answered' | 'answered_in_terminal' | 'failed'
  message?: string
}

export type OverlayStateName = 'hidden' | 'peek' | 'compact' | 'expanded'

export type AppView = 'sessions' | 'inbox' | 'notes' | 'calendar' | 'integrations' | 'settings' | 'shortcuts'

export interface CaptureResult {
  note_id: string
}

/** Every command the overlay invokes, with its args and result. The Rust side implements them; the browser mock answers them. */
export interface Commands {
  ping: { args: undefined; result: string }
  core_status: { args: undefined; result: unknown }
  sessions_list: { args: undefined; result: RosterRow[] }
  session_decide: { args: { session_id: string; decision: Decision }; result: DecideResult }
  session_acknowledge: { args: { session_id: string }; result: void }
  session_dismiss: { args: { session_id: string }; result: void }
  session_jump: { args: { session_id: string }; result: void }
  /** What the webview wants to be; the core shows / hides / resizes the window and persists `overlay.pinned`. */
  overlay_request: { args: { state: OverlayStateName; pinned: boolean; reason: string }; result: void }
  /** The drawn frame's rect in CSS px from the stage's top-left, open menus included (02 §12): the target with `settled: false` at the start of a move, the rest rect with `settled: true` at its end. */
  overlay_measure: { args: { x: number; y: number; width: number; height: number; settled: boolean }; result: void }
  note_capture: { args: { text: string; created_from?: { kind: 'session' | 'message'; id: string } }; result: CaptureResult }
  app_open: { args: { view: AppView; id?: string; filter?: string }; result: void }
  /** The Now playing strip (02 §7.1, OV-37): each resolves once the player took the command. */
  media_play_pause: { args: undefined; result: void }
  media_next: { args: undefined; result: void }
  media_previous: { args: undefined; result: void }
  media_open: { args: undefined; result: void }
  /** The Music tab (02 §7.10, ADR-0066): what Mode saw playing; a row plays in its player (`played`) or opens it (`opened`). */
  media_history: { args: undefined; result: MediaHistory }
  media_play: { args: { id: string }; result: 'played' | 'opened' }
  /** Settings → Overlay "Clear history" (03 §8.3). */
  media_history_clear: { args: undefined; result: void }
  /** Open an http(s) link in the browser (the onboarding's Star on GitHub; 03 §2.8 row 7). The core's opener guard applies. */
  url_open: { args: { url: string }; result: void }
  /** `⌘/Ctrl+W` and the close button hide the window (03 §2.1); Mode keeps running. */
  app_hide: { args: Record<string, never>; result: void }
  /** App window (03 §3.6–§3.12); the shape is proposed in contract-sessions.ts. */
  session_detail: { args: { session_id: string }; result: import('./contract-sessions').SessionDetail }
  /** Launch dialog (03 §3.5, 05 §8.1). */
  session_launch: { args: import('./contract-sessions').LaunchArgs; result: import('./contract-sessions').LaunchResult }
  /** Resume dialog (03 §3.5, 05 §8.2). */
  session_resume: { args: import('./contract-sessions').ResumeArgs; result: import('./contract-sessions').LaunchResult }
  // notes (package 7): Notes view, editor, board, query bar and cards (03 §5, 04); shapes proposed in contract-notes.ts
  notes_list: import('./contract-notes').NotesCommands['notes_list']
  note_get: import('./contract-notes').NotesCommands['note_get']
  note_create: import('./contract-notes').NotesCommands['note_create']
  note_save: import('./contract-notes').NotesCommands['note_save']
  note_set_field: import('./contract-notes').NotesCommands['note_set_field']
  note_cycle_status: import('./contract-notes').NotesCommands['note_cycle_status']
  note_archive: import('./contract-notes').NotesCommands['note_archive']
  note_trash: import('./contract-notes').NotesCommands['note_trash']
  note_untrash: import('./contract-notes').NotesCommands['note_untrash']
  folders_list: import('./contract-notes').NotesCommands['folders_list']
  groupings_list: import('./contract-notes').NotesCommands['groupings_list']
  grouping_save: import('./contract-notes').NotesCommands['grouping_save']
  tags_list: import('./contract-notes').NotesCommands['tags_list']
  query_resolve: import('./contract-notes').NotesCommands['query_resolve']
  query_run: import('./contract-notes').NotesCommands['query_run']
  note_history: import('./contract-notes').NotesCommands['note_history']
  note_revision_get: import('./contract-notes').NotesCommands['note_revision_get']
  note_restore: import('./contract-notes').NotesCommands['note_restore']
  note_link_add: import('./contract-notes').NotesCommands['note_link_add']
  // terminal (package 8): the Terminal tab's PTY channel and the prompt box (05 §8, §14); shapes proposed in contract-terminal.ts
  pty_open: import('./contract-terminal').TerminalCommands['pty_open']
  pty_write: import('./contract-terminal').TerminalCommands['pty_write']
  pty_resize: import('./contract-terminal').TerminalCommands['pty_resize']
  pty_close: import('./contract-terminal').TerminalCommands['pty_close']
  session_prompt: import('./contract-terminal').TerminalCommands['session_prompt']
  turn_stop: import('./contract-terminal').TerminalCommands['turn_stop']
  // settings (package 9): Settings, Integrations, Shortcuts, Stats, Diagnostics, onboarding (03 §2.8, §7A, §7C, §7D, §8); shapes proposed in contract-settings.ts
  settings_get_all: import('./contract-settings').SettingsCommands['settings_get_all']
  settings_set: import('./contract-settings').SettingsCommands['settings_set']
  settings_reset: import('./contract-settings').SettingsCommands['settings_reset']
  settings_export: import('./contract-settings').SettingsCommands['settings_export']
  settings_import: import('./contract-settings').SettingsCommands['settings_import']
  health_get: import('./contract-settings').SettingsCommands['health_get']
  platform_info: import('./contract-settings').SettingsCommands['platform_info']
  diagnostics_get: import('./contract-settings').SettingsCommands['diagnostics_get']
  logs_tail: import('./contract-settings').SettingsCommands['logs_tail']
  diagnostics_export: import('./contract-settings').SettingsCommands['diagnostics_export']
  db_backup: import('./contract-settings').SettingsCommands['db_backup']
  retention_sweep: import('./contract-settings').SettingsCommands['retention_sweep']
  vault_pick: import('./contract-settings').SettingsCommands['vault_pick']
  vault_inspect: import('./contract-settings').SettingsCommands['vault_inspect']
  profile_picture_pick: import('./contract-settings').SettingsCommands['profile_picture_pick']
  hooks_status: import('./contract-settings').SettingsCommands['hooks_status']
  hooks_install_preview: import('./contract-settings').SettingsCommands['hooks_install_preview']
  hooks_install_apply: import('./contract-settings').SettingsCommands['hooks_install_apply']
  hooks_uninstall: import('./contract-settings').SettingsCommands['hooks_uninstall']
  hooks_round_trip: import('./contract-settings').SettingsCommands['hooks_round_trip']
  secret_status: import('./contract-settings').SettingsCommands['secret_status']
  secret_set: import('./contract-settings').SettingsCommands['secret_set']
  ai_backends_status: import('./contract-settings').SettingsCommands['ai_backends_status']
  ai_test_call: import('./contract-settings').SettingsCommands['ai_test_call']
  connectors_status: import('./contract-settings').SettingsCommands['connectors_status']
  hotkey_set: import('./contract-settings').SettingsCommands['hotkey_set']
  stats_days: import('./contract-settings').SettingsCommands['stats_days']
  stats_summary: import('./contract-settings').SettingsCommands['stats_summary']
  grouping_delete: import('./contract-settings').SettingsCommands['grouping_delete']
  notes_reindex: import('./contract-settings').SettingsCommands['notes_reindex']
  statuses_save: import('./contract-settings').SettingsCommands['statuses_save']
  status_move: import('./contract-settings').SettingsCommands['status_move']
  tag_rename: import('./contract-settings').SettingsCommands['tag_rename']
  tag_delete: import('./contract-settings').SettingsCommands['tag_delete']
  data_delete_all: import('./contract-settings').SettingsCommands['data_delete_all']
  app_quit: import('./contract-settings').SettingsCommands['app_quit']
  // connectors (package 10): accounts, sources, Inbox, important people, calendar, alerts, quiet hours, AI (06, 07; 03 §4, §6, §7C); shapes proposed in contract-connectors.ts
  accounts_list: import('./contract-connectors').ConnectorsCommands['accounts_list']
  account_add_start: import('./contract-connectors').ConnectorsCommands['account_add_start']
  account_add_finish: import('./contract-connectors').ConnectorsCommands['account_add_finish']
  account_add_cancel: import('./contract-connectors').ConnectorsCommands['account_add_cancel']
  account_pause: import('./contract-connectors').ConnectorsCommands['account_pause']
  account_resume: import('./contract-connectors').ConnectorsCommands['account_resume']
  account_retry: import('./contract-connectors').ConnectorsCommands['account_retry']
  account_reauthorise: import('./contract-connectors').ConnectorsCommands['account_reauthorise']
  account_remove: import('./contract-connectors').ConnectorsCommands['account_remove']
  oauth_client_set: import('./contract-connectors').ConnectorsCommands['oauth_client_set']
  sources_list: import('./contract-connectors').ConnectorsCommands['sources_list']
  source_set: import('./contract-connectors').ConnectorsCommands['source_set']
  slack_channels_list: import('./contract-connectors').ConnectorsCommands['slack_channels_list']
  slack_channels_set: import('./contract-connectors').ConnectorsCommands['slack_channels_set']
  inbox_list: import('./contract-connectors').ConnectorsCommands['inbox_list']
  thread_get: import('./contract-connectors').ConnectorsCommands['thread_get']
  message_mark_read: import('./contract-connectors').ConnectorsCommands['message_mark_read']
  thread_archive: import('./contract-connectors').ConnectorsCommands['thread_archive']
  thread_snooze: import('./contract-connectors').ConnectorsCommands['thread_snooze']
  thread_unsnooze: import('./contract-connectors').ConnectorsCommands['thread_unsnooze']
  message_set_tier: import('./contract-connectors').ConnectorsCommands['message_set_tier']
  open_in_source: import('./contract-connectors').ConnectorsCommands['open_in_source']
  vips_list: import('./contract-connectors').ConnectorsCommands['vips_list']
  vip_add: import('./contract-connectors').ConnectorsCommands['vip_add']
  vip_remove: import('./contract-connectors').ConnectorsCommands['vip_remove']
  agenda_list: import('./contract-connectors').ConnectorsCommands['agenda_list']
  event_join: import('./contract-connectors').ConnectorsCommands['event_join']
  event_prepare_notes: import('./contract-connectors').ConnectorsCommands['event_prepare_notes']
  event_links: import('./contract-connectors').ConnectorsCommands['event_links']
  item_link_add: import('./contract-connectors').ConnectorsCommands['item_link_add']
  item_link_remove: import('./contract-connectors').ConnectorsCommands['item_link_remove']
  alerts_recent: import('./contract-connectors').ConnectorsCommands['alerts_recent']
  alert_dismiss: import('./contract-connectors').ConnectorsCommands['alert_dismiss']
  quiet_set: import('./contract-connectors').ConnectorsCommands['quiet_set']
  ai_backends_list: import('./contract-connectors').ConnectorsCommands['ai_backends_list']
  ai_backend_set: import('./contract-connectors').ConnectorsCommands['ai_backend_set']
  ai_key_set: import('./contract-connectors').ConnectorsCommands['ai_key_set']
  ai_health: import('./contract-connectors').ConnectorsCommands['ai_health']
  // reconciliation: commands the app crate and the connectors core register beyond the lists above
  notes_folders_set: import('./contract-settings').SettingsCommands['notes_folders_set']
  claude_code_card: import('./contract-connectors').ConnectorsCommands['claude_code_card']
  slack_manifest: import('./contract-connectors').ConnectorsCommands['slack_manifest']
  keyword_rules_list: import('./contract-connectors').ConnectorsCommands['keyword_rules_list']
  keyword_rules_set: import('./contract-connectors').ConnectorsCommands['keyword_rules_set']
  urgency_rescore: import('./contract-connectors').ConnectorsCommands['urgency_rescore']
  urgency_diagnostics: import('./contract-connectors').ConnectorsCommands['urgency_diagnostics']
  alert_rule_test: import('./contract-connectors').ConnectorsCommands['alert_rule_test']
  alert_rules_list: import('./contract-connectors').ConnectorsCommands['alert_rules_list']
  alert_rule_set: import('./contract-connectors').ConnectorsCommands['alert_rule_set']
  alert_rule_add: import('./contract-connectors').ConnectorsCommands['alert_rule_add']
  alert_rule_delete: import('./contract-connectors').ConnectorsCommands['alert_rule_delete']
  pty_detach: import('./contract-terminal').TerminalCommands['pty_detach']
  pty_list: import('./contract-terminal').TerminalCommands['pty_list']
  // package 11: Chat (03 §7), Automations (03 §7B), theme packs (10 §4–§5), the command palette (10 §7.4.15); shapes in contract-chat.ts, contract-automations.ts, contract-themes.ts, palette-store.ts, aligned with mode_core::{chat, scheduler, themes, palette}::api
  chat_threads_list: import('./contract-chat').ChatCommands['chat_threads_list']
  chat_thread_get: import('./contract-chat').ChatCommands['chat_thread_get']
  chat_send: import('./contract-chat').ChatCommands['chat_send']
  chat_stop: import('./contract-chat').ChatCommands['chat_stop']
  chat_action_confirm: import('./contract-chat').ChatCommands['chat_action_confirm']
  chat_thread_delete: import('./contract-chat').ChatCommands['chat_thread_delete']
  chat_sources: import('./contract-chat').ChatCommands['chat_sources']
  chat_status: import('./contract-chat').ChatCommands['chat_status']
  chat_thread_rename: import('./contract-chat').ChatCommands['chat_thread_rename']
  chat_pin: import('./contract-chat').ChatCommands['chat_pin']
  chat_thread_set: import('./contract-chat').ChatCommands['chat_thread_set']
  chat_pick_file: import('./contract-chat').ChatCommands['chat_pick_file']
  automations_list: import('./contract-automations').AutomationsCommands['automations_list']
  automation_get: import('./contract-automations').AutomationsCommands['automation_get']
  automation_save: import('./contract-automations').AutomationsCommands['automation_save']
  automation_delete: import('./contract-automations').AutomationsCommands['automation_delete']
  automation_set_enabled: import('./contract-automations').AutomationsCommands['automation_set_enabled']
  automation_run_now: import('./contract-automations').AutomationsCommands['automation_run_now']
  automation_runs: import('./contract-automations').AutomationsCommands['automation_runs']
  automation_run_get: import('./contract-automations').AutomationsCommands['automation_run_get']
  schedule_preview: import('./contract-automations').AutomationsCommands['schedule_preview']
  automation_duplicate: import('./contract-automations').AutomationsCommands['automation_duplicate']
  automation_examples: import('./contract-automations').AutomationsCommands['automation_examples']
  themes_list: import('./contract-themes').ThemesCommands['themes_list']
  theme_get: import('./contract-themes').ThemesCommands['theme_get']
  theme_set_active: import('./contract-themes').ThemesCommands['theme_set_active']
  theme_import: import('./contract-themes').ThemesCommands['theme_import']
  theme_export: import('./contract-themes').ThemesCommands['theme_export']
  theme_delete: import('./contract-themes').ThemesCommands['theme_delete']
  theme_resolve: import('./contract-themes').ThemesCommands['theme_resolve']
  theme_duplicate: import('./contract-themes').ThemesCommands['theme_duplicate']
  theme_update: import('./contract-themes').ThemesCommands['theme_update']
  palette_query: import('./palette-store').PaletteCommands['palette_query']
  // Agents (03 §7E; ADR-0058); shapes in contract-agents.ts, aligned with mode_core::agents
  agents_list: import('./contract-agents').AgentsCommands['agents_list']
  agent_get: import('./contract-agents').AgentsCommands['agent_get']
  agent_save: import('./contract-agents').AgentsCommands['agent_save']
  agent_delete: import('./contract-agents').AgentsCommands['agent_delete']
  agent_memory_save: import('./contract-agents').AgentsCommands['agent_memory_save']
  agent_memory_delete: import('./contract-agents').AgentsCommands['agent_memory_delete']
  skills_list: import('./contract-agents').AgentsCommands['skills_list']
  mcp_servers_list: import('./contract-agents').AgentsCommands['mcp_servers_list']
  // The Marketplace (03 §7C.6; ADR-0060); shapes in contract-packs.ts, aligned with mode_core::packs::api
  pack_export: import('./contract-packs').PacksCommands['pack_export']
  pack_import: import('./contract-packs').PacksCommands['pack_import']
  catalogue_get: import('./contract-packs').PacksCommands['catalogue_get']
  // Dictation (XC-37, XC-38); aligned with mode_app::commands_dictation. The model commands and the mic check are app window only; app_context_set is per calling window (null clears that window's item).
  dictation_status: { args: undefined; result: DictationStatus }
  dictation_press: { args: undefined; result: void }
  dictation_release: { args: undefined; result: void }
  dictation_cancel: { args: undefined; result: void }
  /** After a `dictation.text`: `ok = false` (no editable field had focus) applies `dictation.unfocused`. */
  dictation_inserted: { args: { ok: boolean }; result: void }
  /** Dev builds only (08 §12): `text` through the core's route as if a hold had produced it; a release build refuses with `unavailable`. */
  dictation_say: { args: { text: string }; result: void }
  dictation_models: { args: undefined; result: DictationModels }
  /** Starts the download and returns at once; progress arrives on `dictation.model`. */
  dictation_model_ensure: { args: { model: string }; result: void }
  dictation_model_remove: { args: { model: string }; result: void }
  /** Opens the input device for 200 ms so the OS asks for the microphone. */
  dictation_mic_check: { args: undefined; result: MicCheck }
  app_context_set: { args: { kind: AppContextKind | null; id: string | null }; result: void }
}

export type CommandName = keyof Commands

/* ---- settings (02 §17) --------------------------------------------------- */

export interface OverlaySettings {
  peek_delay_ms: number
  expand_on_hover: boolean
  expand_hover_ms: number
  collapse_delay_ms: number
  auto_collapse_ms: number
  pill_min_ms: number
  max_rows: number
  default_tab: 'auto' | OverlayTab
  show_mascot: boolean
  fold_after_minutes: number
  /** What the overlay rests as with nothing to show (02 §2.1): the gaze mascot in `peek`, or `hidden`. */
  idle: 'mascot' | 'hidden'
  /** Now playing (02 §7.1, OV-37): show the local player's track. */
  media: boolean
}

export type OverlayTab = 'sessions' | 'inbox' | 'notes' | 'calendar' | 'music'

/** Every tab in its fixed order; Music shows only while `overlay.media` is on (02 §7.1, `visibleTabs`). */
export const OVERLAY_TABS: readonly OverlayTab[] = ['sessions', 'inbox', 'notes', 'calendar', 'music']

/** The tabs the overlay shows for these settings (02 §7.1): Music only while Now playing is on. */
export function visibleTabs(settings: { media?: boolean }): readonly OverlayTab[] {
  return settings.media === false ? OVERLAY_TABS.filter((t) => t !== 'music') : OVERLAY_TABS
}

export const DEFAULT_SETTINGS: Readonly<OverlaySettings> = {
  peek_delay_ms: 150,
  expand_on_hover: true,
  expand_hover_ms: 600,
  collapse_delay_ms: 2500,
  auto_collapse_ms: 8000,
  pill_min_ms: 2000,
  max_rows: 6,
  default_tab: 'auto',
  show_mascot: true,
  fold_after_minutes: 10,
  idle: 'mascot',
  media: true,
}

/** Fixed timings of 02 §3. */
export const TIMING = {
  arrival_settle_ms: 200,
  resolved_flash_ms: 1200,
  failure_flash_ms: 2000,
  new_strip_ms: 4000,
  capture_saved_ms: 3000,
  done_hold_ms: 10_000,
  keyboard_idle_factor: 4,
  /** A voice reply card stays this long after its answer without the pointer (02 §3, §5.7). */
  reply_hold_ms: 20_000,
} as const

/* ---- command errors (reconciliation) ---------------------------------------- */

/** The buttons a refusal names (05 §14: only `session_prompt` refusals carry one). */
export type CmdErrorAction = 'jump' | 'resume'

/**
 * Every command of the app crate rejects with `{ code, message, action? }`
 * (`mode_app::cmd_error::CmdError`): codes `not_found`, `invalid`,
 * `read_only`, `io`, `internal`, `unavailable`, `pending`, `busy`, the PTY
 * host's `refused`, `claude_missing`, `cwd_missing`, `transcript_missing`,
 * and the connectors' `offline`, `keyring`, `needs_reauth`, `canceled`.
 */
export interface CmdError {
  code: string
  message: string
  action?: CmdErrorAction | string
}

/** Any rejection as a `CmdError`: the core's object, an `Error` (runtime, browser mock), a JSON or plain string. */
export function toCmdError(e: unknown): CmdError {
  if (typeof e === 'string') {
    const t = e.trim()
    if (t.startsWith('{')) {
      try {
        const parsed: unknown = JSON.parse(t)
        if (parsed && typeof parsed === 'object' && typeof (parsed as { message?: unknown }).message === 'string') return toCmdError(parsed)
      } catch {
        /* a plain message that happens to start with a brace */
      }
    }
    return { code: 'internal', message: e }
  }
  if (e && typeof e === 'object') {
    const o = e as { code?: unknown; message?: unknown; action?: unknown }
    const code = typeof o.code === 'string' && o.code ? o.code : 'internal'
    const message = typeof o.message === 'string' && o.message ? o.message : code
    const out: CmdError = { code, message }
    if (typeof o.action === 'string' && o.action) out.action = o.action
    return out
  }
  return { code: 'internal', message: String(e) }
}

/** The sentence to show for a rejection (the core's `message`). */
export function cmdErrorText(e: unknown): string {
  return toCmdError(e).message
}

/** The `action` of a refusal when it names one of the buttons Mode offers. */
export function cmdErrorAction(e: unknown): CmdErrorAction | null {
  const a = toCmdError(e).action
  return a === 'jump' || a === 'resume' ? a : null
}
