// Chat over the user's own data (03 §7, ADR-0022): conversations, turns that
// stream, activity groups, citations, action proposals that run only after
// a confirmation, and the Sources panel.
//
// Aligned with `mode_core::chat::{api, edge}` (package 11; the core shapes
// its answers after this file). Field names are snake_case as the core sends
// them. Commands marked "beyond the list" were not in the package brief and
// are what the views need on top of it; fields marked "core extra" are sent
// by the core beyond what the views read (optional here, so a view or a
// test may build the shape without them).

/* ---- events: core → app window ------------------------------------------- */

export const CHAT_EVENTS = {
  /** Reply text and activity of a running turn, in order (a stream). */
  delta: 'chat.delta',
  /** A turn started / finished / failed / was stopped, with its tokens. */
  turn: 'chat.turn',
  /** A proposed action, and later its confirmed / rejected / done result. */
  action: 'chat.action',
} as const

/* ---- vocabulary ------------------------------------------------------------ */

/** The effort / backend picker of 03 §7.2, per conversation. */
export type Effort = 'auto' | 'quick' | 'thorough' | 'local_only'

export const EFFORTS: readonly Effort[] = ['auto', 'quick', 'thorough', 'local_only']

/** What a citation, a source item or an attachment points at. */
export type ItemKind = 'note' | 'session' | 'message' | 'event' | 'file'

/** The retrieval tools of 03 §7.3. */
export type ToolName = 'search_notes' | 'get_note' | 'search_messages' | 'get_thread' | 'list_sessions' | 'get_session' | 'calendar_lookup' | 'now'

/* ---- shapes ---------------------------------------------------------------- */

/** One item a tool returned, a citation target or an attachment. `route` is the app route that opens it (03 §2.5). */
export interface ItemRef {
  kind: ItemKind
  id: string
  title: string
  /** One line under the title in the Sources panel (folder, sender, repo, time). */
  detail?: string | null
  route?: string | null
}

/** `[n]` in the reply text refers to `n` (03 §7.4); stored in `chat_turns.citations_json`. */
export interface Citation extends ItemRef {
  n: number
  /** Core extra: the retrieved text the citation rests on (`detail` is its first 80 characters). */
  snippet?: string
}

/** One row of an expanded activity group (03 §7.1): a single line, a 24-px thumbnail for files. */
export interface ActivityRowSpec {
  text: string
  /** File kind for the `.m-thumb` (`md`, `py`); absent on non-file rows. */
  thumb?: string | null
  /** The tool call this row belongs to: clicking it opens that Sources entry. */
  call_id?: string | null
}

/** "Ran 3 commands", "Read 8 files", "Searched notes", "Updating the release note" (a sub-task). */
export interface ActivityGroup {
  id: string
  label: string
  rows: ActivityRowSpec[]
  /** "4 background tasks completed". */
  status?: string | null
  /** The group is running (its label shimmers). */
  active?: boolean
  /** Core extra: the tool call the group is (`search_notes`, …). */
  tool?: string
  /** Core extra: `running` | `done` | `failed`. */
  state?: ToolCall['state']
  /** Core extra: the items it returned. */
  count?: number
  /** Core extra: the failure line (also in `status`). */
  error?: string | null
}

/** One tool call of a turn, as the Sources panel lists it (03 §7.3). */
export interface ToolCall {
  id: string
  tool: ToolName | string
  /** The inputs as one line ("release · status todo"). */
  input_line: string
  items: ItemRef[]
  state: 'running' | 'done' | 'failed'
  /** Core extra: the activity label of the call. */
  label?: string
  /** Core extra: the failure. */
  error?: string | null
  /** Core extra: the inputs as sent to the tool. */
  input?: unknown
}

/** The 03 §7.5 kinds; the core also proposes `session_prompt` (send a prompt to a session) and `create_automation`. */
export type ActionKind = 'create_note' | 'edit_note' | 'set_fields' | 'snooze_message' | 'archive_message' | 'resume_session' | 'session_prompt' | 'create_automation'

export type ActionState = 'pending' | 'rejected' | 'running' | 'done' | 'failed'

/** A proposal (03 §7.5). Nothing runs until `chat_action_confirm {confirm: true}`. */
export interface ChatAction {
  id: string
  kind: ActionKind
  /** The card title: "Create note", "Set fields", "Snooze message". */
  title: string
  /** Label · value lines: folder, title, the thread, the session and `cwd`; `before` draws "before → after". */
  fields: { label: string; value: string; before?: string | null }[]
  /** Body preview or diff text (Create note, Edit note body). */
  preview?: string | null
  state: ActionState
  /** The result line that replaces the card ("Created note Prepare for 1:1 in Unsorted/"). */
  result?: string | null
  /** Where the result opens. */
  result_route?: string | null
  /** A failure line. */
  error?: string | null
  /** Core extras: where the card lives, a one-line summary, when it was proposed and decided. */
  turn_id?: string
  thread_id?: string
  summary?: string
  created_at?: string
  decided_at?: string | null
}

/** A turn's tokens (`usage_samples`, `purpose = 'chat'`), as the core counts them. */
export interface TokenUsage {
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_write_tokens: number
  total: number
}

export type TurnStatus = 'streaming' | 'finished' | 'failed' | 'stopped'

export interface ChatTurn {
  id: string
  role: 'user' | 'assistant'
  /** Markdown for assistant turns; the user's text as typed. */
  text: string
  created_at: string
  /** Assistant turns. */
  status?: TurnStatus
  /** "offline", "Can't reach Claude API (timeout)". */
  error?: string | null
  activity?: ActivityGroup[]
  citations?: Citation[]
  actions?: ChatAction[]
  /** User turns: items passed as retrieved context. */
  attachments?: ItemRef[]
  /** Assistant turns: the turn's tokens (`usage_samples`, `purpose = 'chat'`). */
  tokens?: number | null
  pinned?: boolean
  /** No retrieved item: "no sources — answered from the question alone". */
  no_sources?: boolean
  /** Core extras (replies): the model and effort that answered, the token split, an answer that cited nothing it retrieved, the no-sources label. */
  model?: string
  effort?: Effort
  usage?: TokenUsage
  uncited?: boolean
  no_sources_label?: string
}

export interface ChatConnectors {
  gmail: boolean
  slack: boolean
  calendar: boolean
}

export interface ChatThread {
  id: string
  title: string
  created_at: string
  updated_at: string
  effort: Effort
  connectors: ChatConnectors
  turns: ChatTurn[]
}

/** A pinned reply, listed in the conversations list's Pinned section. */
export interface PinnedReply {
  turn_id: string
  excerpt: string
}

export interface ChatThreadRow {
  id: string
  title: string
  updated_at: string
  /** The user messages in the conversation. */
  turn_count: number
  pinned: PinnedReply[]
  /** Core extra. */
  created_at?: string
}

/**
 * Backend state for the meta line and the disabled input (03 §7.2 "Backend state"). Beyond the list. The core reports
 * a rejected key as `ready: 'no_key'` with `state: 'invalid_key'`; the store reads that pair as `invalid_key`
 * (`readinessOf`), and takes `invalid_key` as `ready` too.
 */
export type ChatReadiness = 'ok' | 'no_key' | 'invalid_key' | 'local_only_no_backend' | 'budget' | 'unreachable' | 'offline'

export interface ChatStatus {
  /** `claude_api` | `ollama` | `lm_studio`. */
  backend: string
  /** "Claude API", "Ollama". */
  backend_label: string
  /** The model the next turn uses (`ai.model_chat` or the local model); the meta line shows its short form. */
  model: string | null
  local: boolean
  /** A local backend is configured (Local-only is offered). */
  local_available: boolean
  /** `ai.local_only` is on. */
  local_only: boolean
  ready: ChatReadiness
  /** "Can't reach Claude API (timeout)". */
  error?: string | null
  /** Which connectors have an account (the `+` menu reasons). */
  accounts: { gmail: boolean; slack: boolean; calendar: boolean }
  /**
   * Core extras: the AI health word, the disabled input's line ("Ready" otherwise; "API key rejected"), whether the
   * input is disabled, the effort asked about, the model's short form (`opus-5-5`), the privacy line for that effort
   * ("…sent to Claude API", "LAN backend — data leaves this computer but not your network", "Nothing leaves this
   * machine"), the local-backend hint. The views use `message` and `privacy_line` when present and their own words
   * otherwise.
   */
  state?: 'ready' | 'no_key' | 'local_only_blocked' | 'budget_reached' | 'unreachable' | 'invalid_key' | 'offline'
  message?: string
  input_disabled?: boolean
  effort?: Effort
  model_short?: string | null
  privacy_line?: string | null
  hint?: string | null
}

/* ---- events ---------------------------------------------------------------- */

/** `chat.delta`: append `text`; upsert `activity` by id (the group's label, rows and `active`). The core always sends `thread_id`. */
export interface ChatDelta {
  turn_id: string
  thread_id?: string
  text?: string
  activity?: ActivityGroup
  /** Core extras: the delta's order in the turn, and which of `text` / `activity` it carries. */
  seq?: number
  kind?: 'text' | 'activity'
}

/** `chat.turn`. `turn` (finished) carries the stored turn with citations and activity. */
export interface ChatTurnEvent {
  turn_id: string
  thread_id?: string
  status: 'started' | 'finished' | 'failed' | 'stopped'
  usage?: TokenUsage | null
  error?: string | null
  turn?: ChatTurn
  /** Beyond the list: the reply was stopped (03 §7.6, the partial text kept). */
  stopped?: boolean
  /** Core extras: the reply's citations, its no-sources label, an answer that cited nothing it retrieved. */
  citations?: Citation[]
  no_sources_label?: string | null
  uncited?: boolean
}

export interface ChatActionEvent {
  turn_id: string
  thread_id?: string
  action: ChatAction
}

/* ---- command args / results -------------------------------------------------- */

export interface ChatSendArgs {
  /** Omitted: a new conversation. */
  thread_id?: string
  text: string
  effort?: Effort
  attachments?: { kind: ItemKind; id: string }[]
  /** Per-conversation retrieval toggles (`+` → Connectors ›). Beyond the list. */
  connectors?: ChatConnectors
}

/** `turn_id` is the assistant turn that streams; `thread_id` and `user_turn_id` are beyond the list. */
export interface ChatSendResult {
  turn_id: string
  thread_id: string
  user_turn_id?: string
}

export interface ChatSources {
  turn_id: string
  calls: ToolCall[]
  tokens: number | null
  /** Core extras: the conversation, the turn's attachments, its token split, every item the calls returned. */
  thread_id?: string
  attachments?: ItemRef[]
  usage?: TokenUsage
  items?: ItemRef[]
}

export interface ChatCommands {
  chat_threads_list: { args: undefined; result: ChatThreadRow[] }
  chat_thread_get: { args: { id: string }; result: ChatThread }
  chat_send: { args: ChatSendArgs; result: ChatSendResult }
  /** `stopped: false` when the turn had already ended. */
  chat_stop: { args: { turn_id: string }; result: { stopped: boolean } }
  chat_action_confirm: { args: { turn_id: string; action_id: string; confirm: boolean }; result: ChatAction }
  /** The rows removed (a running turn is stopped first). */
  chat_thread_delete: { args: { id: string }; result: { deleted: number } }
  chat_sources: { args: { turn_id: string }; result: ChatSources }
  /** Beyond the list: the meta line, the disabled states, the `+` menu reasons. */
  chat_status: { args: { effort?: Effort } | undefined; result: ChatStatus }
  /** Beyond the list: the conversations list's ⋮ → Rename. */
  chat_thread_rename: { args: { id: string; title: string }; result: void }
  /** Beyond the list: the reply footer's pin. */
  chat_pin: { args: { turn_id: string; pinned: boolean }; result: void }
  /** Beyond the list: the effort picker and the connector toggles stored per conversation. */
  chat_thread_set: { args: { id: string; effort?: Effort; connectors?: ChatConnectors }; result: void }
  /** Beyond the list: `+` → Add file; the app shows the OS file picker and answers the file's `ItemRef` (`id` its absolute path, `title` the file name, `detail` the folder with home as `~`, `route: null`), `null` when canceled; a file is cited, never uploaded. */
  chat_pick_file: { args: undefined; result: ItemRef | null }
}

/* ---- the slash commands of 03 §7.2 ------------------------------------------- */

export const SLASH_COMMANDS: readonly { name: string; description: string }[] = [
  { name: '/summarize', description: 'Summarize the attached note, session or thread' },
  { name: '/todo', description: 'Turn the reply into a todo note' },
  { name: '/snooze', description: 'Snooze a message until a time' },
  { name: '/open', description: 'Open a note, session or message' },
  { name: '/link', description: 'Link the current note to a session' },
]
