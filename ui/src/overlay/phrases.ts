// Text rules of the overlay: elapsed labels (05 §11.3), the session detail
// line (02 §7.5), waiting sub-reasons (05 §4.3), the mascot badge (02 §11.2).
import type { SessionState } from '../kit/types'
import { TIMING, type Pending, type QuietChanged, type RosterRow } from '../state/contract'

/** "12s", "15m 12s" (seconds zero-padded: "2m 04s"), "1h 04m", "1d 02h" (05 §11.3). */
export function elapsed(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  const pad = (n: number) => String(n).padStart(2, '0')
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m ${pad(s % 60)}s`
  if (s < 86_400) return `${Math.floor(s / 3600)}h ${pad(Math.floor((s % 3600) / 60))}m`
  return `${Math.floor(s / 86_400)}d ${pad(Math.floor((s % 86_400) / 3600))}h`
}

/** The label carries seconds (under an hour), so it ticks once a second (02 §15). */
export function hasSeconds(seconds: number): boolean {
  return Math.max(0, Math.floor(seconds)) < 3600
}

export function sinceIso(iso: string | null | undefined, now: number): string {
  if (!iso) return ''
  const t = Date.parse(iso)
  return Number.isNaN(t) ? '' : elapsed((now - t) / 1000)
}

/** How long ago, as the Inbox's time column reads it (02 §7.6, §7.10): "now", "3m", "2h", "Mon", "Oct 3". */
export function agoLabel(iso: string | null | undefined, now: number): string {
  if (!iso) return ''
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return ''
  const min = Math.floor(Math.max(0, now - t) / 60_000)
  if (min < 1) return 'now'
  if (min < 60) return `${min}m`
  if (min < 24 * 60) return `${Math.floor(min / 60)}h`
  const d = new Date(t)
  if (min < 7 * 24 * 60) return d.toLocaleDateString('en-US', { weekday: 'short' })
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
}

/** "in 12 min" within the hour, else "at 14:00" (02 §5.2). */
export function startsLabel(iso: string, now: number): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return ''
  const min = Math.round((t - now) / 60_000)
  if (min <= 60) return min <= 0 ? 'now' : `in ${min} min`
  const d = new Date(t)
  return `at ${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

export function basename(path: string): string {
  const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/)
  return parts[parts.length - 1] || path
}

/** `cwd` abbreviated with `~` for display outside a repository (the home is read off the path itself). */
export function tildePath(path: string): string {
  const m = /^(\/(?:Users|home)\/[^/]+|[A-Za-z]:\\Users\\[^\\]+)(?=[\\/]|$)/.exec(path)
  return m ? `~${path.slice(m[1].length)}` : path
}

/** Repository name, else the folder: the first half of the detail line. */
export function placeLabel(row: Pick<RosterRow, 'repo_root' | 'cwd'>): { text: string; mono: boolean } {
  if (row.repo_root) return { text: basename(row.repo_root), mono: false }
  return { text: tildePath(row.cwd), mono: true }
}

/** The short word of a pending tool: `pytest` from "Bash: pytest tests/…", the file name of an edit. */
export function toolShort(p: Pending): string {
  const input = (p.tool_input ?? {}) as Record<string, unknown>
  if (typeof input.command === 'string') return programOf(input.command)
  if (typeof input.file_path === 'string') return basename(input.file_path)
  if (p.tool_name) return p.tool_name
  return p.summary.split(':')[0]?.trim() || 'tool'
}

export function programOf(command: string): string {
  const words = command.trim().split(/\s+/).filter((w) => !/^[A-Za-z_][A-Za-z0-9_]*=/.test(w) && w !== 'sudo' && w !== 'env')
  return words[0] ?? command
}

/** The one or two line preview of a permission card (02 §7.5). */
export function pendingPreview(p: Pending): string {
  const input = (p.tool_input ?? {}) as Record<string, unknown>
  if (typeof input.command === 'string') return input.command
  if (typeof input.file_path === 'string') return input.file_path
  if (typeof input.url === 'string') return input.url
  return p.summary.replace(/^[^:]+:\s*/, '')
}

export function waitingDetail(row: RosterRow): string {
  const kind = row.pending?.kind ?? row.waiting_kind
  switch (kind) {
    case 'permission':
      return row.pending ? (row.pending.answerable ? `permission for ${toolShort(row.pending)}` : 'answer in the terminal') : 'waiting for you'
    case 'question':
      return 'asks a question'
    case 'idle_prompt':
      return 'turn ended'
    case 'elicitation':
      return 'needs input in the terminal'
    case 'agent_needs_input':
      return 'a subagent needs input in the terminal'
    default:
      return 'waiting for you'
  }
}

export interface Detail {
  text?: string
  diff?: { add: number; del: number; files: number }
}

/** The state's half of the detail line. */
export function stateDetail(row: RosterRow): Detail {
  switch (row.state) {
    case 'waiting':
      return { text: waitingDetail(row) }
    case 'working':
      return { text: row.current_tool ? (row.current_mcp_server ? `mcp: ${row.current_mcp_server} · ${row.current_tool}` : row.current_tool) : 'working' }
    case 'failed':
      return { text: row.end_reason || row.state_reason || 'failed' }
    case 'unread':
    case 'done':
      return { diff: { add: row.lines_added, del: row.lines_removed, files: row.files_changed } }
    case 'idle':
      return { text: 'no activity' }
    case 'untracked':
      return { text: 'no hooks' }
    default:
      return {}
  }
}

export function agentsSuffix(n: number): string | null {
  if (n <= 0) return null
  return `${n} ${n === 1 ? 'agent' : 'agents'}`
}

/** Pill wording for a waiting session: the detail part after the name (02 §5.2). */
export function waitingPillDetail(row: RosterRow): string | null {
  const p = row.pending
  switch (p?.kind ?? row.waiting_kind) {
    case 'permission':
      return p?.answerable === false ? 'waiting in the terminal' : (p?.summary ?? null)
    case 'question':
      // The word, not the text: the question itself is the card's and the screen-reader label's.
      return 'question'
    case 'idle_prompt':
      return 'turn ended'
    case 'elicitation':
    case 'agent_needs_input':
      return 'needs input in the terminal'
    default:
      return null
  }
}

export interface BadgeInput {
  rows: RosterRow[]
  urgentUnread: number
  quiet: QuietChanged | null
  doneHeldUntil: number
  now: number
}

/** 02 §11.2: waiting › failed › sleeping › done (10 s) › working › idle (no badge). */
export function mascotBadge({ rows, urgentUnread, quiet, doneHeldUntil, now }: BadgeInput): SessionState | undefined {
  if (rows.some((r) => r.state === 'waiting') || urgentUnread > 0) return 'waiting'
  if (rows.some((r) => r.state === 'failed')) return 'failed'
  if (quiet && (quiet.quiet_hours || quiet.focus || quiet.presentation)) return 'finished'
  if (doneHeldUntil > now) return 'unread'
  if (rows.some((r) => r.state === 'working')) return 'working'
  return undefined
}

export const DONE_HOLD_MS = TIMING.done_hold_ms

/** The kit's seven-state vocabulary for a roster state (`done` renders as `finished`). */
export function rowState(state: string): SessionState {
  switch (state) {
    case 'waiting':
    case 'working':
    case 'failed':
    case 'unread':
    case 'idle':
    case 'untracked':
      return state
    default:
      return 'finished'
  }
}

/** `avatar.<k>` → hue (twelve hues at 30° steps, 10 §5.10 fallback); unknown tokens keep the stylesheet default. */
export function avatarHue(token: string): number | undefined {
  const m = /^avatar\.(\d{1,2})$/.exec(token)
  if (!m) return undefined
  const k = Number(m[1])
  return k >= 1 && k <= 12 ? (k - 1) * 30 : undefined
}
