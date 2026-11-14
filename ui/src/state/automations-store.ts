// The Automations view's state (03 §7B): the list with its phrases and
// last / next run cells, selection, search and sort, the editor's draft with
// validation and the schedule preview, run history and the run page. The
// phrase rules mirror `mode_core::scheduler` (10 §12.1) so an unsaved draft
// reads the same as a saved row.
import { create } from 'zustand'
import { invoke, listen, type Unlisten } from '../tauri'
import type { StateKind } from '../kit/types'
import {
  AUTOMATION_EVENTS,
  type ActionJson,
  type ActionKind,
  type AskChatAction,
  type AutomationDraft,
  type AutomationRow,
  type CreateNoteAction,
  type FieldError,
  type RunDetail,
  type RunRow,
  type SaveError,
  type Schedule,
  type SchedulePreview,
  type StartSessionAction,
} from './contract-automations'
import { cmdErrorText } from './contract'
import { useSessionsStore } from './sessions-store'

/* ---- time ------------------------------------------------------------------------------------- */

export function deviceZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}

const WEEKDAY_SHORT = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']
const MONTH_SHORT = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

export interface Wall {
  y: number
  m: number
  d: number
  hh: number
  mm: number
  /** ISO weekday 1–7. */
  wd: number
}

/** Wall-clock parts of an instant in an IANA zone. */
export function wallOf(ms: number, tz: string): Wall {
  let parts: Intl.DateTimeFormatPart[]
  try {
    parts = new Intl.DateTimeFormat('en-US', { timeZone: tz, year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: 'numeric', weekday: 'short', hourCycle: 'h23' }).formatToParts(
      new Date(ms),
    )
  } catch {
    parts = new Intl.DateTimeFormat('en-US', {
      timeZone: 'UTC',
      year: 'numeric',
      month: 'numeric',
      day: 'numeric',
      hour: 'numeric',
      minute: 'numeric',
      weekday: 'short',
      hourCycle: 'h23',
    }).formatToParts(new Date(ms))
  }
  const get = (t: string) => parts.find((p) => p.type === t)?.value ?? ''
  const wd = WEEKDAY_SHORT.indexOf(get('weekday')) + 1
  return { y: Number(get('year')), m: Number(get('month')), d: Number(get('day')), hh: Number(get('hour')) % 24, mm: Number(get('minute')), wd: wd || 1 }
}

/** `YYYY-MM-DD HH:MM` → wall parts (the core's `next_run_local`, `fires_local`). */
export function parseLocal(s: string): Wall | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/.exec(s)
  if (!m) return null
  const [y, mo, d, hh, mm] = m.slice(1).map(Number) as [number, number, number, number, number]
  const wd = ((new Date(Date.UTC(y, mo - 1, d)).getUTCDay() + 6) % 7) + 1
  return { y, m: mo, d, hh, mm, wd }
}

const pad = (n: number) => String(n).padStart(2, '0')

function dayNumber(w: Pick<Wall, 'y' | 'm' | 'd'>): number {
  return Math.floor(Date.UTC(w.y, w.m - 1, w.d) / 86_400_000)
}

/** "today 23:00", "tomorrow 08:00", "Fri 17:00" within the week, else "22 Oct 08:00". */
export function wallLabel(w: Wall, now: number, tz: string): string {
  const today = wallOf(now, tz)
  const diff = dayNumber(w) - dayNumber(today)
  const time = `${pad(w.hh)}:${pad(w.mm)}`
  if (diff === 0) return `today ${time}`
  if (diff === 1) return `tomorrow ${time}`
  if (diff > 1 && diff < 7) return `${WEEKDAY_SHORT[w.wd - 1]} ${time}`
  return `${w.d} ${MONTH_SHORT[w.m - 1]} ${time}`
}

/** "Thu 22 Oct 08:00" (the preview line). */
export function wallFull(w: Wall): string {
  return `${WEEKDAY_SHORT[w.wd - 1]} ${w.d} ${MONTH_SHORT[w.m - 1]} ${pad(w.hh)}:${pad(w.mm)}`
}

/** "Tue 21 Oct · 08:00" (run rows). */
export function runStartedLabel(iso: string, tz: string): string {
  const w = wallOf(Date.parse(iso), tz)
  return `${WEEKDAY_SHORT[w.wd - 1]} ${w.d} ${MONTH_SHORT[w.m - 1]} · ${pad(w.hh)}:${pad(w.mm)}`
}

/** "just now", "4 m ago", "2 h ago", "4 d ago" (the wireframe's last-run and run-history cells). */
export function agoLabel(iso: string | null | undefined, now: number): string {
  if (!iso) return ''
  const t = Date.parse(iso)
  if (!Number.isFinite(t)) return ''
  const s = Math.max(0, Math.round((now - t) / 1000))
  if (s < 60) return 'just now'
  const m = Math.floor(s / 60)
  if (m < 60) return `${m} m ago`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h} h ago`
  return `${Math.floor(h / 24)} d ago`
}

/** "2 m 14 s"; "40 s" under a minute. */
export function durationLabel(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return ''
  const s = Math.max(0, Math.round(ms / 1000))
  const m = Math.floor(s / 60)
  return `${m} m ${s % 60} s`
}

export function secondsLabel(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000))
  if (s < 60) return `${s} s`
  const m = Math.floor(s / 60)
  return m < 60 ? `${m} m` : `${Math.floor(m / 60)} h`
}

/* ---- phrases (10 §12.1; mirrors mode_core::scheduler) -------------------------------------------- */

/** The prompt's first words: one line, lower-cased first letter, an ellipsis past 40 characters. */
export function firstWords(text: string): string {
  const line = (text.split('\n').find((l) => l.trim()) ?? '').trim()
  const chars = Array.from(line)
  if (!chars.length) return ''
  let s = chars[0]!.toLowerCase() + chars.slice(1).join('')
  if (Array.from(s).length > 40) s = `${Array.from(s).slice(0, 39).join('').trimEnd()}…`
  return s
}

/** "Daily plan {date}" → "Daily plan". */
export function patternStem(p: string): string {
  return p
    .replace(/\{[^}]*\}/g, '')
    .split(/\s+/)
    .filter(Boolean)
    .join(' ')
}

/** "New York" for `America/New_York`. */
export function city(tz: string): string {
  return (tz.split('/').pop() ?? tz).replace(/_/g, ' ')
}

/** "Daily 23:00", "Weekdays 08:00", "Weekly · Mon 09:00", "Cron 0 8 * * 1-5"; " · New York" only when the zone differs from the device's. */
export function triggerPhrase(s: Schedule, deviceTz = deviceZone()): string {
  const t = parseTime(s.time ?? '')
  const time = t ? `${pad(t[0])}:${pad(t[1])}` : ''
  let p: string
  switch (s.kind) {
    case 'daily':
      p = `Daily ${time}`
      break
    case 'weekdays':
      p = `Weekdays ${time}`
      break
    case 'weekly':
      p = `Weekly · ${s.weekday && s.weekday >= 1 && s.weekday <= 7 ? WEEKDAY_SHORT[s.weekday - 1] : '?'} ${time}`
      break
    default:
      p = `Cron ${(s.cron ?? '').trim()}`
  }
  if (s.tz && s.tz !== deviceTz) p += ` · ${city(s.tz)}`
  return p
}

function basename(path: string): string {
  const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/)
  return parts[parts.length - 1] || path
}

/** "Start session in mode · summarize inbox…", "Create note in Unsorted/ · Daily plan", "Ask Chat · what is due today". */
export function actionPhrase(kind: ActionKind, action: ActionJson, folderLabel: (id: string | null) => string = (id) => (id ? id : 'Unsorted')): string {
  if (kind === 'start_session') {
    const a = action as StartSessionAction
    return `Start session in ${basename(a.cwd || '…')} · ${firstWords(a.prompt)}`
  }
  if (kind === 'create_note') {
    const a = action as CreateNoteAction
    const f = folderLabel(a.folder_id)
    return `Create note in ${f ? `${f}/` : 'the vault'} · ${patternStem(a.title_pattern)}`
  }
  return `Ask Chat · ${firstWords((action as AskChatAction).prompt)}`
}

/* ---- validation ---------------------------------------------------------------------------- */

export function parseTime(s: string): [number, number] | null {
  const m = /^\s*(\d{1,2}):(\d{2})\s*$/.exec(s)
  if (!m) return null
  const h = Number(m[1])
  const mi = Number(m[2])
  return h <= 23 && mi <= 59 ? [h, mi] : null
}

const CRON_FIELDS: readonly [string, number, number][] = [
  ['Minute', 0, 59],
  ['Hour', 0, 23],
  ['Day of month', 1, 31],
  ['Month', 1, 12],
  ['Weekday', 0, 7],
]

/** A 5-field cron (minute hour day-of-month month day-of-week; ranges, lists, steps). Null when valid. */
export function validateCron(expr: string): string | null {
  const parts = expr.trim().split(/\s+/).filter(Boolean)
  if (parts.length !== 5) return 'Cron needs five fields: minute hour day month weekday.'
  for (let i = 0; i < 5; i++) {
    const [name, lo, hi] = CRON_FIELDS[i]!
    for (const item of parts[i]!.split(',')) {
      const m = /^(\*|(\d+)(?:-(\d+))?)(?:\/(\d+))?$/.exec(item)
      if (!m) return `${name} “${item}” is not a cron field.`
      if (m[4] !== undefined && Number(m[4]) < 1) return `${name} step must be at least 1.`
      for (const v of [m[2], m[3]]) {
        if (v === undefined) continue
        const n = Number(v)
        if (n < lo || n > hi) return `${name} ${n} is out of range (${lo}–${hi}).`
      }
      if (m[2] !== undefined && m[3] !== undefined && Number(m[3]) < Number(m[2])) return `${name} range ${m[2]}–${m[3]} runs backwards.`
    }
  }
  return null
}

/** Client-side checks before `automation_save` (03 §7B.2 Validation); the core adds folder and `claude` checks. */
export function validateDraft(d: AutomationDraft): FieldError[] {
  const out: FieldError[] = []
  if (!d.name.trim()) out.push({ field: 'name', message: 'Name the automation.' })
  if (d.schedule.kind === 'cron') {
    const e = validateCron(d.schedule.cron ?? '')
    if (e) out.push({ field: 'schedule.cron', message: e })
  } else if (!parseTime(d.schedule.time ?? '')) out.push({ field: 'schedule.time', message: 'Use HH:MM, 24 h.' })
  if (d.schedule.kind === 'weekly' && !(d.schedule.weekday && d.schedule.weekday >= 1 && d.schedule.weekday <= 7)) out.push({ field: 'schedule.weekday', message: 'Pick a weekday.' })
  if (d.action_kind === 'start_session') {
    const a = d.action as StartSessionAction
    if (!a.cwd?.trim()) out.push({ field: 'action.cwd', message: 'Choose a repository.' })
    if (!a.prompt?.trim()) out.push({ field: 'action.prompt', message: 'Write the prompt.' })
    if (a.token_cap !== null && a.token_cap !== undefined && !(a.token_cap > 0)) out.push({ field: 'action.token_cap', message: 'The token cap must be above 0.' })
  } else if (d.action_kind === 'create_note') {
    if (!(d.action as CreateNoteAction).title_pattern?.trim()) out.push({ field: 'action.title_pattern', message: 'Write a title pattern.' })
  } else if (!(d.action as AskChatAction).prompt?.trim()) out.push({ field: 'action.prompt', message: 'Write the question.' })
  if (!(d.keep_runs >= 1)) out.push({ field: 'keep_runs', message: 'Keep at least one run.' })
  return out
}

/** "200k" / "1.5M" / "200000" → tokens; null when empty; NaN when unreadable. */
export function parseTokens(s: string): number | null {
  const t = s
    .trim()
    .toLowerCase()
    .replace(/[,_\s]/g, '')
  if (!t) return null
  const m = /^(\d+(?:\.\d+)?)([km]?)$/.exec(t)
  if (!m) return Number.NaN
  const n = Number(m[1]) * (m[2] === 'k' ? 1_000 : m[2] === 'm' ? 1_000_000 : 1)
  return Math.round(n)
}

export function formatTokens(n: number | null | undefined): string {
  if (n === null || n === undefined) return ''
  if (n >= 1_000_000 && n % 100_000 === 0) return `${n / 1_000_000}M`
  if (n >= 1_000 && n % 1_000 === 0) return `${n / 1_000}k`
  return String(n)
}

/* ---- row cells ------------------------------------------------------------------------------- */

const RUN_STATE: Record<string, StateKind> = { finished: 'finished', failed: 'failed', skipped: 'idle', running: 'working' }

/** The last-run cell: glyph + relative time; spinner "running · 40 s"; ✕ "claude not found" (03 §7B.7). */
export function lastRunCell(r: AutomationRow, now: number): { state: StateKind; label: string; title: string } | null {
  const run = r.last_run
  if (r.running || run?.state === 'running') {
    const started = run ? Date.parse(run.started_at) : now
    return { state: 'working', label: `running · ${secondsLabel(now - started)}`, title: 'running' }
  }
  if (r.problem === 'claude_not_found' && (!run || run.state !== 'finished')) return { state: 'failed', label: 'claude not found', title: 'claude not found' }
  if (r.problem === 'no_ai_backend' && (!run || run.state !== 'finished')) return { state: 'failed', label: 'no AI backend', title: 'no AI backend' }
  if (!run) return null
  const word = run.state === 'skipped' ? 'skipped' : run.state === 'failed' ? 'failed' : 'finished'
  const ago = agoLabel(run.started_at, now)
  return { state: RUN_STATE[run.state] ?? 'finished', label: ago, title: `${word} · ${ago}${run.error ? ` · ${run.error}` : ''}` }
}

/** The next-run cell: "tomorrow 08:00" in the schedule's zone; "off" while disabled. */
export function nextRunLabel(r: AutomationRow, now: number): string {
  if (!r.enabled) return 'off'
  const tz = r.schedule.tz || deviceZone()
  const w = r.next_run_local ? parseLocal(r.next_run_local) : r.next_run_at ? wallOf(Date.parse(r.next_run_at), tz) : null
  return w ? wallLabel(w, now, tz) : ''
}

/** The run row's detail: "2 m 14 s · session · morning-brief", "0 m 12 s · backend unreachable", the skip reason. */
export function runDetail(run: RunRow, now: number): { duration: string; result: string | null; error: string | null } {
  const duration = run.state === 'running' ? `running · ${secondsLabel(now - Date.parse(run.started_at))}` : run.state === 'skipped' ? '' : durationLabel(run.duration_ms)
  const result = run.result_kind && run.state !== 'failed' && run.state !== 'skipped' ? `${run.result_kind} · ${run.result_title ?? run.result_id ?? 'unavailable'}` : null
  return { duration, result, error: run.state === 'failed' || run.state === 'skipped' ? run.error : null }
}

export function runResultRoute(run: RunRow): string | null {
  if (!run.result_id) return null
  return run.result_kind === 'session' ? `sessions/${run.result_id}` : run.result_kind === 'note' ? `notes/${run.result_id}` : null
}

export type SortKey = 'position' | 'name' | 'next' | 'last'

/** Rows by `position`, then name; disabled rows last (they get the "disabled" label). */
export function sortRows(rows: AutomationRow[], sort: SortKey = 'position'): AutomationRow[] {
  const cmp = (a: AutomationRow, b: AutomationRow): number => {
    if (sort === 'name') return a.name.localeCompare(b.name)
    if (sort === 'next') return (a.next_run_at ?? '9').localeCompare(b.next_run_at ?? '9') || a.name.localeCompare(b.name)
    if (sort === 'last') return (b.last_run_at ?? '').localeCompare(a.last_run_at ?? '') || a.name.localeCompare(b.name)
    return a.position - b.position || a.name.localeCompare(b.name)
  }
  const on = rows.filter((r) => r.enabled).sort(cmp)
  const off = rows.filter((r) => !r.enabled).sort(cmp)
  return [...on, ...off]
}

export function matchesSearch(r: AutomationRow, q: string): boolean {
  const t = q.trim().toLowerCase()
  if (!t) return true
  return `${r.name} ${r.trigger_phrase} ${r.action_phrase}`.toLowerCase().includes(t)
}

/* ---- drafts ---------------------------------------------------------------------------------- */

export function defaultAction(kind: ActionKind): ActionJson {
  if (kind === 'start_session') return { cwd: '', prompt: '', permission_mode: 'default', mode: 'headless', token_cap: null }
  if (kind === 'create_note') return { template_note_id: null, folder_id: null, grouping_slug: null, tags: [], title_pattern: 'Daily plan {date}' }
  return { prompt: '', folder_id: null, effort: 'auto' }
}

export function emptyDraft(tz = deviceZone()): AutomationDraft {
  return { name: '', enabled: true, schedule: { kind: 'weekdays', time: '08:00', tz }, action_kind: 'start_session', action: defaultAction('start_session'), notify_on_failure: true, keep_runs: 20 }
}

export function draftFromRow(r: AutomationRow): AutomationDraft {
  return {
    id: r.id,
    name: r.name,
    enabled: r.enabled,
    schedule: { ...r.schedule },
    action_kind: r.action_kind,
    action: JSON.parse(JSON.stringify(r.action)) as ActionJson,
    notify_on_failure: r.notify_on_failure,
    keep_runs: r.keep_runs,
  }
}

/** The two example automations of 03 §7B.5; created disabled. `cwd` is the last used repository. */
export function exampleDraft(which: 'morning' | 'nightly', cwd: string, tz = deviceZone()): AutomationDraft {
  if (which === 'morning')
    return {
      name: 'Morning brief',
      enabled: false,
      schedule: { kind: 'weekdays', time: '08:00', tz },
      action_kind: 'ask_chat',
      action: { prompt: "Summarize today's inbox and calendar", folder_id: null, effort: 'auto' },
      notify_on_failure: true,
      keep_runs: 20,
    }
  return {
    name: 'Nightly test run',
    enabled: false,
    schedule: { kind: 'daily', time: '23:00', tz },
    action_kind: 'start_session',
    action: { cwd, prompt: 'Run the test suite and summarize the failures', permission_mode: 'default', mode: 'headless', token_cap: null },
    notify_on_failure: true,
    keep_runs: 20,
  }
}

function sameDraft(a: AutomationDraft | null, b: AutomationDraft | null): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}

/** The preview line: "Thu 22 Oct 08:00 · Fri 23 Oct 08:00 · …" in the schedule's zone. */
export function previewLine(p: SchedulePreview | null, tz: string): string {
  if (!p || p.error) return ''
  const walls = p.fires_local?.length ? p.fires_local.map(parseLocal) : p.fires.map((f) => wallOf(Date.parse(f), tz))
  return walls
    .filter((w): w is Wall => !!w)
    .map(wallFull)
    .join(' · ')
}

/* ---- the store --------------------------------------------------------------------------------- */

function message(e: unknown): string {
  return cmdErrorText(e)
}

/** The inline field errors of a save refusal: `invalid`'s `fields`; none for any other refusal (`fields` absent). */
export function fieldsOf(e: unknown): FieldError[] {
  const f = e && typeof e === 'object' ? (e as SaveError).fields : undefined
  return Array.isArray(f) ? f.filter((x): x is FieldError => !!x && typeof x.field === 'string' && typeof x.message === 'string') : []
}

/** The editor's errors for a save refusal: the fields inline, else one line for the whole form. */
export function saveErrors(e: unknown): FieldError[] {
  const fields = fieldsOf(e)
  return fields.length ? fields : [{ field: 'form', message: message(e) }]
}

export interface EditorState {
  draft: AutomationDraft
  /** The saved version (`null` for a new automation). */
  base: AutomationDraft | null
  errors: FieldError[]
  saving: boolean
  preview: SchedulePreview | null
}

export interface AutomationsState {
  rows: AutomationRow[]
  loaded: boolean
  error: string | null
  selectedId: string | null
  search: string
  sort: SortKey
  disabledOnly: boolean
  runs: Record<string, RunRow[]>
  run: RunDetail | null
  editor: EditorState | null
  reset(): void
  load(): Promise<void>
  setRows(rows: AutomationRow[]): void
  select(id: string | null): void
  setSearch(q: string): void
  setSort(s: SortKey): void
  setDisabledOnly(v: boolean): void
  visible(): AutomationRow[]
  setEnabled(id: string, enabled: boolean, opts?: { undo?: boolean }): Promise<void>
  runNow(id: string): Promise<RunRow | null>
  /** `automation_duplicate`: "<name> copy", disabled, saved by the core; selected. */
  duplicate(id: string): Promise<AutomationRow | null>
  /** `automation_examples`: the empty state's two drafts from the core (Morning brief, Nightly test run); null when it does not answer. */
  examples(): Promise<AutomationDraft[] | null>
  remove(id: string): Promise<boolean>
  loadRuns(id: string, limit?: number): Promise<void>
  loadRun(runId: string): Promise<void>
  /** Opens the editor on a row, on a new draft, or on a given draft. */
  edit(target: AutomationRow | AutomationDraft | null): void
  closeEditor(): void
  patchDraft(patch: Partial<AutomationDraft>): void
  patchAction(patch: Partial<StartSessionAction & CreateNoteAction & AskChatAction>): void
  setActionKind(kind: ActionKind): void
  dirty(): boolean
  save(): Promise<AutomationRow | null>
  previewSchedule(): Promise<void>
}

const initial = {
  rows: [] as AutomationRow[],
  loaded: false,
  error: null as string | null,
  selectedId: null as string | null,
  search: '',
  sort: 'position' as SortKey,
  disabledOnly: false,
  runs: {} as Record<string, RunRow[]>,
  run: null as RunDetail | null,
  editor: null as EditorState | null,
}

let previewSeq = 0

export const useAutomations = create<AutomationsState>((set, get) => ({
  ...initial,
  reset() {
    set({ ...initial })
  },
  async load() {
    try {
      const rows = await invoke<AutomationRow[]>('automations_list')
      set({ rows: Array.isArray(rows) ? rows : [], loaded: true, error: null })
    } catch (e) {
      set({ loaded: true, error: message(e) })
    }
  },
  setRows(rows) {
    set({ rows, loaded: true })
    const id = get().selectedId
    if (id && !rows.some((r) => r.id === id)) set({ selectedId: null })
  },
  select(id) {
    set({ selectedId: id })
    if (id) void get().loadRuns(id, 3)
  },
  setSearch: (search) => set({ search }),
  setSort: (sort) => set({ sort }),
  setDisabledOnly: (disabledOnly) => set({ disabledOnly }),
  visible() {
    const s = get()
    return sortRows(
      s.rows.filter((r) => matchesSearch(r, s.search) && (!s.disabledOnly || !r.enabled)),
      s.sort,
    )
  },
  async setEnabled(id, enabled, opts) {
    const prev = get().rows
    const row = prev.find((r) => r.id === id)
    if (!row) return
    set({ rows: prev.map((r) => (r.id === id ? { ...r, enabled } : r)) })
    try {
      const saved = await invoke<AutomationRow>('automation_set_enabled', { id, enabled })
      if (saved?.id) set({ rows: get().rows.map((r) => (r.id === id ? saved : r)) })
      if (opts?.undo !== false) useSessionsStore.getState().showNotice(`${row.name} ${enabled ? 'on' : 'off'}`, { label: 'Undo', run: () => void get().setEnabled(id, !enabled, { undo: false }) })
    } catch (e) {
      set({ rows: prev, error: message(e) })
      useSessionsStore.getState().showNotice(`Couldn't switch ${row.name} ${enabled ? 'on' : 'off'}`)
    }
  },
  async runNow(id) {
    const row = get().rows.find((r) => r.id === id)
    if (row?.running) return null
    set({ rows: get().rows.map((r) => (r.id === id ? { ...r, running: true } : r)) })
    try {
      const run = await invoke<RunRow>('automation_run_now', { id })
      set({ rows: get().rows.map((r) => (r.id === id ? { ...r, running: run.state === 'running', last_run: run, last_run_at: run.started_at } : r)) })
      void get().loadRuns(id, get().runs[id]?.length && get().runs[id]!.length > 3 ? 50 : 3)
      return run
    } catch (e) {
      set({ rows: get().rows.map((r) => (r.id === id ? { ...r, running: false } : r)), error: message(e) })
      useSessionsStore.getState().showNotice(`Couldn't run ${row?.name ?? 'the automation'}: ${message(e)}`)
      return null
    }
  },
  async duplicate(id) {
    if (!get().rows.some((r) => r.id === id)) return null
    try {
      const saved = await invoke<AutomationRow>('automation_duplicate', { id })
      set({ rows: [...get().rows.filter((r) => r.id !== saved.id), saved], selectedId: saved.id, error: null })
      return saved
    } catch (e) {
      set({ error: message(e) })
      useSessionsStore.getState().showNotice(`Couldn't duplicate: ${message(e)}`)
      return null
    }
  },
  async examples() {
    try {
      const drafts = await invoke<AutomationDraft[]>('automation_examples')
      return Array.isArray(drafts) && drafts.length ? drafts : null
    } catch {
      return null
    }
  },
  async remove(id) {
    try {
      await invoke('automation_delete', { id })
      set({ rows: get().rows.filter((r) => r.id !== id), selectedId: get().selectedId === id ? null : get().selectedId })
      return true
    } catch (e) {
      set({ error: message(e) })
      return false
    }
  },
  async loadRuns(id, limit) {
    try {
      const runs = await invoke<RunRow[]>('automation_runs', limit ? { id, limit } : { id })
      set({ runs: { ...get().runs, [id]: Array.isArray(runs) ? runs : [] } })
    } catch {
      /* the panel keeps what it had */
    }
  },
  async loadRun(runId) {
    if (get().run?.run.id === runId) return
    set({ run: null })
    try {
      set({ run: await invoke<RunDetail>('automation_run_get', { id: runId }) })
    } catch (e) {
      set({ error: message(e) })
    }
  },
  edit(target) {
    const draft = !target ? emptyDraft() : 'action_phrase' in target ? draftFromRow(target) : target
    const base = target && 'action_phrase' in target ? draftFromRow(target) : null
    set({ editor: { draft, base, errors: [], saving: false, preview: null } })
  },
  closeEditor() {
    set({ editor: null })
  },
  patchDraft(patch) {
    const ed = get().editor
    if (!ed) return
    const draft = { ...ed.draft, ...patch, schedule: patch.schedule ? { ...ed.draft.schedule, ...patch.schedule } : ed.draft.schedule }
    const touched = Object.keys(patch)
    set({ editor: { ...ed, draft, errors: ed.errors.filter((e) => !touched.some((k) => e.field === k || e.field.startsWith(`${k}.`))) } })
  },
  patchAction(patch) {
    const ed = get().editor
    if (!ed) return
    const action = { ...ed.draft.action, ...patch } as ActionJson
    const touched = Object.keys(patch).map((k) => `action.${k}`)
    set({ editor: { ...ed, draft: { ...ed.draft, action }, errors: ed.errors.filter((e) => !touched.includes(e.field)) } })
  },
  setActionKind(kind) {
    const ed = get().editor
    if (!ed || ed.draft.action_kind === kind) return
    // Switching keeps what the base had for that kind, else the defaults.
    const action = ed.base?.action_kind === kind ? ed.base.action : defaultAction(kind)
    set({ editor: { ...ed, draft: { ...ed.draft, action_kind: kind, action: JSON.parse(JSON.stringify(action)) as ActionJson }, errors: ed.errors.filter((e) => !e.field.startsWith('action.')) } })
  },
  dirty() {
    const ed = get().editor
    if (!ed) return false
    return !sameDraft(ed.draft, ed.base)
  },
  async save() {
    const ed = get().editor
    if (!ed || ed.saving) return null
    const errors = validateDraft(ed.draft)
    if (errors.length) {
      set({ editor: { ...ed, errors } })
      return null
    }
    set({ editor: { ...ed, saving: true, errors: [] } })
    try {
      const saved = await invoke<AutomationRow>('automation_save', { draft: ed.draft })
      const rows = get().rows.some((r) => r.id === saved.id) ? get().rows.map((r) => (r.id === saved.id ? saved : r)) : [...get().rows, saved]
      const base = draftFromRow(saved)
      set({ rows, editor: { ...get().editor!, draft: base, base, saving: false } })
      return saved
    } catch (e) {
      set({ editor: { ...get().editor!, saving: false, errors: saveErrors(e) } })
      return null
    }
  },
  async previewSchedule() {
    const ed = get().editor
    if (!ed) return
    const schedule = ed.draft.schedule
    const seq = ++previewSeq
    if (schedule.kind === 'cron') {
      const err = validateCron(schedule.cron ?? '')
      if (err) {
        set({ editor: { ...ed, preview: { fires: [], echo: null, error: { field: 'schedule.cron', message: err } } } })
        return
      }
    }
    try {
      const preview = await invoke<SchedulePreview>('schedule_preview', { schedule_json: schedule })
      const cur = get().editor
      if (seq !== previewSeq || !cur) return
      set({ editor: { ...cur, preview } })
    } catch (e) {
      const cur = get().editor
      if (seq !== previewSeq || !cur) return
      set({ editor: { ...cur, preview: { fires: [], echo: null, error: { field: 'schedule', message: message(e) } } } })
    }
  },
}))

/** `automations.changed` carries the list (or `{rows}`). */
export function connectAutomations(): Promise<Unlisten> {
  return listen<AutomationRow[] | { rows: AutomationRow[] }>(AUTOMATION_EVENTS.changed, (p) => {
    const rows = Array.isArray(p) ? p : p && Array.isArray((p as { rows?: unknown }).rows) ? (p as { rows: AutomationRow[] }).rows : null
    if (rows) {
      useAutomations.getState().setRows(rows)
      const sel = useAutomations.getState().selectedId
      if (sel) void useAutomations.getState().loadRuns(sel, 3)
    } else void useAutomations.getState().load()
  })
}
