// The app window's answers in the browser mock (outside Tauri): the sample
// roster, `session_detail`, launch / resume and the proposed commands of
// contract-sessions.ts, registered through the extension point of
// src/tauri.ts (`mock().rows` for `sessions_list`, `mock().respond` for the
// rest). State changes (acknowledge, dismiss, decide, launch, resume) are
// applied to the sample and pushed back as `sessions.changed`, the way the
// core would.
import { AVATAR_VARIANTS } from '../kit/mascot'
import { mock } from '../tauri'
import { EVENTS, type Decision } from '../state/contract'
import type { AppRosterRow, DiffLine, DiffResult, FileRow, HistoryQuery, LaunchArgs, ResumeArgs, SessionDetail, SnoozePreset, TranscriptItem } from '../state/contract-sessions'
import { LAUNCH_PLACEHOLDER_PREFIX } from '../state/contract-sessions'
import { HOME, detailFromRow, sampleDetail, sampleDiff, sampleFinishedDetail, sampleHistory, sampleRoster, sampleTranscript } from './mock-data'

/** The core's rule (05 §11.2): the variant the fewest live top-level rows wear, the earlier one in list order on a tie. */
export function nextVariant(rows: readonly { avatar_variant: string; ended_at: string | null }[]): string {
  const counts = new Map<string, number>()
  for (const r of rows) if (!r.ended_at) counts.set(r.avatar_variant, (counts.get(r.avatar_variant) ?? 0) + 1)
  let best: string = AVATAR_VARIANTS[0]
  let fewest = Number.POSITIVE_INFINITY
  for (const v of AVATAR_VARIANTS) {
    const n = counts.get(v) ?? 0
    if (n < fewest) {
      fewest = n
      best = v
    }
  }
  return best
}

type Args = Record<string, unknown> | undefined

interface MockState {
  rows: AppRosterRow[]
  history: AppRosterRow[]
  details: Map<string, SessionDetail>
  seq: number
}

let state: MockState | null = null

function basename(path: string): string {
  const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/)
  return parts[parts.length - 1] || path
}

function detailOf(id: string, now = Date.now()): SessionDetail | null {
  const s = state!
  const cached = s.details.get(id)
  if (cached) return cached
  const row = s.rows.find((r) => r.id === id) ?? s.history.find((r) => r.id === id)
  if (!row) return null
  const d = id === 's-api' ? sampleDetail(now) : id === 's-rate' ? sampleFinishedDetail(now) : detailFromRow(now, row)
  // The finished and the failed samples ran as the Reviewer agent (03 §7E.3: the Resume dialog offers its backup model).
  if (id === 's-invoice' || id === 's-rate') d.session.profile_id = 'ag-reviewer'
  s.details.set(id, d)
  return d
}

/**
 * A file's diff as `mode_core::files::diff::parse_unified` gives it: the
 * `@@` header, then real unified-diff lines (`-…` / `+…` / ` …`) with
 * `old_no` / `new_no` only where the line has one.
 */
export function unifiedDiffOf(file: Pick<FileRow, 'lines_added' | 'lines_removed' | 'source' | 'change_kind'>): DiffResult {
  const del = Math.min(file.lines_removed, 6)
  const add = Math.min(file.lines_added, 6)
  const lines: DiffLine[] = [{ kind: 'hunk', text: `@@ -1,${del + 1} +1,${add + 1} @@` }, { kind: 'ctx', old_no: 1, new_no: 1, text: ' // unchanged' }]
  for (let i = 0; i < del; i++) lines.push({ kind: 'del', old_no: i + 2, text: `-line ${i + 1}` })
  for (let i = 0; i < add; i++) lines.push({ kind: 'add', new_no: i + 2, text: `+line ${i + 1}` })
  const basis = file.source === 'hook' ? 'last edit' : file.change_kind === 'added' ? 'new file' : 'working tree vs HEAD'
  return { basis, lines, truncated: false }
}

/** `session_snooze`'s end time for a preset (03 §4.7 local times: later today +3 h, tomorrow 09:00, next Monday 09:00). */
export function snoozeEnd(preset: SnoozePreset, now: number): string {
  const d = new Date(now)
  if (preset === 'later_today') return new Date(now + 3 * 3_600_000).toISOString()
  const at9 = (days: number) => {
    const x = new Date(d.getFullYear(), d.getMonth(), d.getDate() + days, 9, 0, 0, 0)
    return x.toISOString()
  }
  if (preset === 'tomorrow') return at9(1)
  return at9(((8 - d.getDay()) % 7) || 7)
}

function push() {
  const m = mock()
  m.rows = state!.rows
  m.emit(EVENTS.sessions, { rows: state!.rows })
}

function patchRow(id: string, patch: Partial<AppRosterRow>) {
  const s = state!
  s.rows = s.rows.map((r) => (r.id === id ? { ...r, ...patch } : r))
  const d = s.details.get(id)
  if (d) {
    const sessionPatch: Record<string, unknown> = {}
    for (const k of ['state', 'waiting_kind', 'acknowledged_at', 'ended_at', 'dismissed_at', 'tracking', 'launch_mode'] as const) if (k in patch) sessionPatch[k] = patch[k]
    if ('state' in patch) sessionPatch.state_since = new Date().toISOString()
    s.details.set(id, { ...d, session: { ...d.session, ...sessionPatch }, pending: 'pending' in patch ? (patch.pending ?? null) : d.pending })
  }
}

function settleCard(id: string, decision: Decision) {
  const d = state!.details.get(id)
  if (!d) return
  const now = new Date().toISOString()
  const thread = d.thread.map((it): TranscriptItem => {
    if (it.kind === 'permission' && it.status === 'open') {
      const status = 'answers' in decision ? 'answered' : decision.behavior === 'deny' ? 'denied' : decision.always ? 'always' : 'allowed'
      return { ...it, status, answered_where: 'app', answered_at: now }
    }
    if (it.kind === 'question' && it.status === 'open' && 'answers' in decision) {
      return { ...it, status: 'answered', answered_where: 'app', answered_at: now, answers: Object.fromEntries(decision.answers.map((a) => [a.question, a.answer])) }
    }
    return it
  })
  state!.details.set(id, { ...d, thread, pending: null })
}

/**
 * Seeds the mock with the sample sessions and registers the app window's
 * commands. Idempotent: calling it again re-seeds from scratch.
 */
/** What the mock's folder picker answers (`sessions_pick_folder`). */
export const PICKED_FOLDER = `${HOME}/code/picked`

export function installAppMock(now = Date.now()): void {
  const m = mock()
  state = { rows: sampleRoster(now), history: sampleHistory(now), details: new Map(), seq: 0 }
  m.rows = state.rows

  m.respond('session_detail', (args: Args) => detailOf(String(args?.session_id ?? '')))

  // Dismissed rows only, most recently dismissed first; `query` / `before` / `limit` as the core reads them.
  m.respond('sessions_history', (args: Args) => {
    const q = (args ?? {}) as HistoryQuery
    const text = q.query?.trim().toLowerCase()
    return state!.history
      .filter((r) => !!r.dismissed_at)
      .filter((r) => !q.before || (r.dismissed_at ?? '') < q.before)
      .filter((r) => !text || [r.display_name, r.title, r.cwd, r.git_branch].some((v) => typeof v === 'string' && v.toLowerCase().includes(text)))
      .slice(0, q.limit ?? 200)
  })

  m.respond('session_acknowledge', (args: Args) => {
    const id = String(args?.session_id)
    const row = state!.rows.find((r) => r.id === id)
    if (row?.state === 'unread') {
      patchRow(id, { state: 'done', acknowledged_at: new Date().toISOString(), state_seconds: 0 })
      push()
    }
    return undefined
  })

  m.respond('session_dismiss', (args: Args) => {
    const id = String(args?.session_id)
    const row = state!.rows.find((r) => r.id === id)
    if (row) {
      state!.rows = state!.rows.filter((r) => r.id !== id)
      state!.history = [{ ...row, dismissed_at: new Date().toISOString() }, ...state!.history]
      push()
    }
    return undefined
  })

  m.respond('session_undismiss', (args: Args) => {
    const id = String(args?.session_id)
    const row = state!.history.find((r) => r.id === id)
    if (row) {
      state!.history = state!.history.filter((r) => r.id !== id)
      state!.rows = [...state!.rows, { ...row, dismissed_at: null }]
      push()
    }
    return undefined
  })

  // Returns when the snooze ends; no preset / until (the Undo) cancels it and answers null.
  m.respond('session_snooze', (args: Args) => {
    const preset = args?.preset as SnoozePreset | null | undefined
    if (typeof args?.until === 'string') return args.until
    return preset ? snoozeEnd(preset, Date.now()) : null
  })

  m.respond('session_decide', (args: Args) => {
    const id = String(args?.session_id)
    const decision = args?.decision as Decision
    const row = state!.rows.find((r) => r.id === id)
    if (!row?.pending) return { outcome: 'already_answered' }
    settleCard(id, decision)
    patchRow(id, { state: 'working', waiting_kind: null, pending: undefined, current_tool: row.pending.tool_name ?? null, state_seconds: 0 })
    push()
    return { outcome: 'accepted' }
  })

  m.respond('session_launch', (args: Args) => {
    const a = args as unknown as LaunchArgs
    const id = `s-launch-${++state!.seq}`
    const at = new Date().toISOString()
    const row: AppRosterRow = {
      id,
      display_name: basename(a.cwd),
      avatar_color: 'avatar.3',
      avatar_variant: nextVariant(state!.rows),
      state: 'idle',
      waiting_kind: null,
      title: null,
      cwd: a.cwd,
      repo_root: a.cwd,
      git_branch: 'main',
      model: a.model || 'claude-opus-5-5',
      current_tool: null,
      lines_added: 0,
      lines_removed: 0,
      files_changed: 0,
      tracking: 'launched',
      launch_mode: 'embedded',
      state_seconds: 0,
      folded: false,
      live_agents: 0,
      started_at: at,
      last_event_at: at,
      // The core's placeholder until the first hook names the real session id.
      claude_session_id: `${LAUNCH_PLACEHOLDER_PREFIX}00000000-0000-4000-8000-${String(state!.seq).padStart(12, '0')}`,
      ended_at: null,
      acknowledged_at: null,
      last_assistant_preview: null,
    }
    state!.rows = [row, ...state!.rows]
    push()
    return { session_id: id, hooks_installed: true }
  })

  m.respond('session_resume', (args: Args) => {
    const a = args as unknown as ResumeArgs
    const id = a.session_id
    const fromHistory = state!.history.find((r) => r.id === id)
    if (fromHistory) {
      state!.history = state!.history.filter((r) => r.id !== id)
      state!.rows = [{ ...fromHistory, dismissed_at: null }, ...state!.rows]
    }
    patchRow(id, { state: 'idle', tracking: 'launched', launch_mode: 'embedded', ended_at: null, dismissed_at: null, state_seconds: 0 })
    const d = state!.details.get(id)
    if (d) state!.details.set(id, { ...d, alive: true, driver: 'pty' })
    push()
    return { session_id: id, hooks_installed: true }
  })

  m.respond('session_files_refresh', (args: Args) => detailOf(String(args?.session_id))?.files ?? [])

  m.respond('session_diff', (args: Args) => {
    const d = detailOf(String(args?.session_id))
    const file = d?.files.find((f) => f.id === args?.file_change_id)
    if (!file) throw { code: 'not_found', message: `no file change ${String(args?.file_change_id)}` }
    return file.id === 'f-fixtures' ? { ...sampleDiff(), truncated: false } : unifiedDiffOf(file)
  })

  m.respond('session_transcript', (args: Args) => {
    const d = detailOf(String(args?.session_id))
    return sampleTranscript(now, d?.transcript.path ?? null)
  })

  m.respond('session_prompt', () => ({ delivered_via: 'pty' }))

  m.respond('sessions_recent_folders', () => Array.from(new Set([...state!.rows, ...state!.history].map((r) => r.cwd))))

  // The OS folder picker: a fixture folder (tests answer `null` for a canceled pick).
  m.respond('sessions_pick_folder', () => PICKED_FOLDER)

  // tauri-plugin-opener commands used by the ⋮ menus (open folder, reveal, open in editor).
  for (const cmd of ['plugin:opener|open_path', 'plugin:opener|reveal_item_in_dir', 'plugin:opener|open_url']) m.respond(cmd, () => undefined)
  // `url_open {url}` (Star on GitHub): the core's opener with its http(s) guard.
  m.respond('url_open', (args: Args) => {
    if (!/^https?:\/\//.test(String(args?.url ?? ''))) throw { code: 'invalid', message: 'only http and https links open in the browser' }
    return undefined
  })
}
