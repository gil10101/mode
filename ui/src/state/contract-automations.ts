// Automations (03 §7B; 09 §5.4A): schedule-triggered automations that start
// a Claude Code session, create a note or ask Chat, and their runs.
//
// Aligned with `mode_core::scheduler::api` (package 11): the row and run
// shapes are `mode_core::scheduler::store::{AutomationRow, RunRow}`, the
// schedule `schedule::Schedule`. Fields marked "core extra" are sent by the
// core beyond what the views read (optional here).

export const AUTOMATION_EVENTS = {
  /** The list after a save, a toggle, a run starting or finishing. */
  changed: 'automations.changed',
} as const

/* ---- schedule_json -------------------------------------------------------------- */

export type ScheduleKind = 'daily' | 'weekdays' | 'weekly' | 'cron'

export const SCHEDULE_KINDS: readonly ScheduleKind[] = ['daily', 'weekdays', 'weekly', 'cron']

export interface Schedule {
  kind: ScheduleKind
  /** `HH:MM`, local to `tz` (daily / weekdays / weekly). */
  time?: string
  /** An IANA zone; the device zone when absent. */
  tz?: string
  /** ISO weekday 1–7, Monday = 1 (weekly). */
  weekday?: number
  /** A 5-field expression (cron). */
  cron?: string
}

/* ---- action_json ------------------------------------------------------------------ */

export type ActionKind = 'start_session' | 'create_note' | 'ask_chat'

export const ACTION_KINDS: readonly ActionKind[] = ['start_session', 'create_note', 'ask_chat']

export type PermissionMode = 'default' | 'plan' | 'acceptEdits' | 'auto' | 'dontAsk' | 'bypassPermissions'

export const PERMISSION_MODES: readonly PermissionMode[] = ['default', 'plan', 'acceptEdits', 'auto', 'dontAsk', 'bypassPermissions']

export type ChatEffort = 'auto' | 'quick' | 'thorough' | 'local_only'

export interface StartSessionAction {
  cwd: string
  prompt: string
  permission_mode: PermissionMode
  mode: 'headless' | 'embedded'
  /** Tokens; the run is stopped with SIGINT once the session passes it. */
  token_cap: number | null
  /** The agent profile the session runs as (03 §7E); absent = the plain launch. */
  profile_id?: string | null
}

export interface CreateNoteAction {
  template_note_id: string | null
  /** `null` = the capture folder. */
  folder_id: string | null
  grouping_slug: string | null
  tags: string[]
  title_pattern: string
}

export interface AskChatAction {
  prompt: string
  folder_id: string | null
  effort: ChatEffort
}

export type ActionJson = StartSessionAction | CreateNoteAction | AskChatAction

/* ---- rows ------------------------------------------------------------------------- */

export type RunState = 'running' | 'finished' | 'failed' | 'skipped'

export interface RunRow {
  id: string
  automation_id: string
  /** `schedule` | `manual` (Run now). */
  trigger_kind: 'schedule' | 'manual' | string
  started_at: string
  finished_at: string | null
  state: RunState | string
  result_kind: 'session' | 'note' | null
  result_id: string | null
  /** The failure text, or the skip reason. */
  error: string | null
  duration_ms: number | null
  /** The result's display name ("morning-brief", "Daily plan 14 Oct"); null before there is one or once it is gone. */
  result_title?: string | null
  /** A session result's diff. */
  result_diff?: { add: number; del: number } | null
}

/** Why a row shows `✕` before it has run (03 §7B.7): the store sets `folder_missing`, the dispatcher the other two. */
export type AutomationProblem = 'claude_not_found' | 'no_ai_backend' | 'folder_missing'

export interface AutomationRow {
  id: string
  name: string
  enabled: boolean
  trigger_kind: 'schedule' | string
  schedule: Schedule
  /** "Weekdays 08:00". */
  trigger_phrase: string
  action_kind: ActionKind
  action: ActionJson
  /** "Start session in mode · summarize inbox and calendar". */
  action_phrase: string
  notify_on_failure: boolean
  keep_runs: number
  position: number
  last_run_at: string | null
  /** UTC; null while disabled. */
  next_run_at: string | null
  /** `next_run_at` as wall time in the schedule's zone, `YYYY-MM-DD HH:MM`. */
  next_run_local: string | null
  last_run: RunRow | null
  /** A run is in `running` (Run now is disabled). */
  running: boolean
  created_at: string
  updated_at: string
  problem?: AutomationProblem | null
}

/** What the editor saves; no id = a new automation (`automation_examples` sends `id: null`). */
export interface AutomationDraft {
  id?: string | null
  name: string
  enabled: boolean
  schedule: Schedule
  action_kind: ActionKind
  action: ActionJson
  notify_on_failure: boolean
  keep_runs: number
  /** The row's place in the manual order; the core appends when absent. */
  position?: number | null
}

/** A field-level problem (`name`, `schedule.cron`, `action.cwd`, `action.prompt`, …). */
export interface FieldError {
  field: string
  message: string
}

/** `automation_save`'s `invalid` refusal: the inline field errors (03 §7B.2 Validation). */
export interface InvalidDraft {
  code: 'invalid'
  message: string
  fields: FieldError[]
}

/** Any other refusal (`busy`, `not_found`, a database error): the app crate's `CmdError`, which sends no `fields`. */
export interface SaveRefusal {
  code: string
  message: string
  /** Absent outside `invalid` (an empty list from an older core reads the same). */
  fields?: FieldError[]
  action?: string
}

/** What `automation_save` rejects with; the editor shows `fields` inline, anything else as the form's line. */
export type SaveError = InvalidDraft | SaveRefusal

/** `schedule_preview`: the next five fire times (UTC) and the plain-language echo, or the error. */
export interface SchedulePreview {
  fires: string[]
  /** As wall time in the schedule's zone, `YYYY-MM-DD HH:MM`, aligned with `fires` (the core always sends it; a preview the view builds may not). */
  fires_local?: string[]
  /** "every weekday at 08:00". */
  echo: string | null
  error: FieldError | null
  /** Core extras: the schedule compiles, its trigger phrase ("Weekdays 08:00 · New York"), the zone the times are in. */
  valid?: boolean
  trigger_phrase?: string
  tz?: string
}

/** The run page (03 §7B.3). */
export interface RunDetail {
  run: RunRow
  /** The automation (a deleted one keeps its last row). */
  automation: AutomationRow
  /** Core extras: the automation's id and name, its phrases, the result's title and route (`sessions/<id>`, `notes/<id>`). */
  automation_id?: string
  automation_name?: string
  trigger_phrase?: string
  action_phrase?: string
  result_title?: string | null
  result_route?: string | null
}

/** `automations_list`'s filters (the view filters and sorts on its own; the core offers the same). */
export interface AutomationsListArgs {
  /** Name, trigger and action phrases. */
  query?: string
  disabled_only?: boolean
  sort?: 'position' | 'name' | 'next_run' | 'last_run'
}

export interface AutomationsCommands {
  /** Disabled rows last (the "disabled" group). */
  automations_list: { args: AutomationsListArgs | undefined; result: AutomationRow[] }
  automation_get: { args: { id: string }; result: AutomationRow }
  /** Rejects with `SaveError`. */
  automation_save: { args: { draft: AutomationDraft }; result: AutomationRow }
  automation_delete: { args: { id: string }; result: { deleted: boolean } }
  automation_set_enabled: { args: { id: string; enabled: boolean }; result: AutomationRow }
  /** Refused with `{code: 'busy'}` while a run of the automation is running. */
  automation_run_now: { args: { id: string }; result: RunRow }
  automation_runs: { args: { id: string; limit?: number }; result: RunRow[] }
  automation_run_get: { args: { id: string }; result: RunDetail }
  /** `schedule_json` is the Schedule object (the core also accepts its JSON text). */
  schedule_preview: { args: { schedule_json: Schedule }; result: SchedulePreview }
  /** Core extra: "<name> copy", disabled, saved (03 §7B.1 Duplicate; the row ⋮ and ⌘/Ctrl+D). */
  automation_duplicate: { args: { id: string }; result: AutomationRow }
  /** Core extra: the two example drafts of 03 §7B.5, disabled, the session example in the last used repository (the empty state's cards; `exampleDraft` when the core does not answer). */
  automation_examples: { args: undefined; result: AutomationDraft[] }
}
