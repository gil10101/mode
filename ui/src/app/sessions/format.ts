// Text rules shared by the roster and session detail. The elapsed format, the
// avatar hue, the state mapping and the detail-line wording are the overlay's
// (src/overlay/phrases.ts, 05 §11.3, 02 §7.5) so a session reads the same on
// both surfaces ("one system, two surfaces", 10 §2). Duplicated rather than
// imported while the overlay package is in flight; a kit candidate.
import type { SessionState, StateKind } from '../../kit/types'
import type { Pending, RosterRow } from '../../state/contract'
import type { ConnectorDetail, ConnectorRow } from '../../state/contract-sessions'

/** "12s", "15m 12s", "1h 04m", "1d 02h" (05 §11.3): the overlay's formatter, one definition for both surfaces. */
export { elapsed } from '../../overlay/phrases'
import { elapsed } from '../../overlay/phrases'

/** Elapsed spoken in words for screen-reader labels (10 §11.4). */
export function elapsedWords(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds))
  const unit = (n: number, w: string) => `${n} ${w}${n === 1 ? '' : 's'}`
  if (s < 60) return unit(s, 'second')
  const m = Math.floor(s / 60)
  if (m < 60) return unit(m, 'minute')
  const h = Math.floor(m / 60)
  if (h < 24) return unit(h, 'hour')
  return unit(Math.floor(h / 24), 'day')
}

export function secondsSince(iso: string | null | undefined, now: number): number {
  if (!iso) return 0
  const t = Date.parse(iso)
  return Number.isNaN(t) ? 0 : (now - t) / 1000
}

export function sinceIso(iso: string | null | undefined, now: number): string {
  return iso ? elapsed(secondsSince(iso, now)) : ''
}

/** Seconds between two instants (a finished agent's frozen clock). */
export function between(from: string, to: string | null | undefined, now: number): number {
  const a = Date.parse(from)
  const b = to ? Date.parse(to) : now
  return Number.isNaN(a) || Number.isNaN(b) ? 0 : (b - a) / 1000
}

/** `137k`, `1.2M`, `950` (10 §7.4.4). */
export function tokens(n: number): string {
  if (n < 1000) return String(Math.max(0, Math.round(n)))
  if (n < 1_000_000) {
    const k = n / 1000
    return `${k < 10 ? trim1(k) : Math.round(k)}k`
  }
  return `${trim1(n / 1_000_000)}M`
}

function trim1(x: number): string {
  const r = Math.round(x * 10) / 10
  return Number.isInteger(r) ? String(r) : r.toFixed(1)
}

/** "137k tokens · 6 turns" — tokens and turns only, never currency (03 §3.12). */
export function usageText(input: number, output: number, turns: number): string {
  return `${tokens(input + output)} tokens · ${turns} ${turns === 1 ? 'turn' : 'turns'}`
}

/** `avatar.<k>` → hue (twelve hues at 30° steps, 10 §5.10 fallback); unknown tokens keep the stylesheet default. */
export function avatarHue(token: string): number | undefined {
  const m = /^avatar\.(\d{1,2})$/.exec(token)
  if (!m) return undefined
  const k = Number(m[1])
  return k >= 1 && k <= 12 ? (k - 1) * 30 : undefined
}

/** The kit's state vocabulary for a session state (`done` renders as `finished`). */
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

/** The lowercase word of a state (10 §6.1). */
export function stateWord(state: string): string {
  return state === 'unread' || state === 'done' ? 'finished' : state
}

/** The screen-reader phrase: "waiting for you" survives only here (05 §11.3). */
export function statePhrase(state: string): string {
  return state === 'waiting' ? 'waiting for you' : stateWord(state)
}

export function basename(path: string): string {
  const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/)
  return parts[parts.length - 1] || path
}

export function dirname(path: string): string {
  const i = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return i > 0 ? path.slice(0, i) : path
}

/** `cwd` abbreviated with `~` (the home is read off the path itself). */
export function tildePath(path: string): string {
  const m = /^(\/(?:Users|home)\/[^/]+|[A-Za-z]:\\Users\\[^\\]+)(?=[\\/]|$)/.exec(path)
  return m ? `~${path.slice(m[1].length)}` : path
}

/** Repository name, else the folder in mono with `~`. */
export function placeLabel(row: { repo_root: string | null; cwd: string }): { text: string; mono: boolean } {
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
  const words = command
    .trim()
    .split(/\s+/)
    .filter((w) => !/^[A-Za-z_][A-Za-z0-9_]*=/.test(w) && w !== 'sudo' && w !== 'env')
  return words[0] ?? command
}

/** The command line or path an approval card shows. */
export function pendingPreview(p: Pending): string {
  const input = (p.tool_input ?? {}) as Record<string, unknown>
  if (typeof input.command === 'string') return input.command
  if (typeof input.file_path === 'string') return input.file_path
  if (typeof input.url === 'string') return input.url
  return p.summary.replace(/^[^:]+:\s*/, '')
}

/** The waiting sub-reason on the detail line (05 §4.3). */
export function waitingDetail(row: Pick<RosterRow, 'pending' | 'waiting_kind'>): string {
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

export interface StateDetail {
  text?: string
  diff?: { add: number; del: number; files: number }
}

/** The state's half of the detail line (03 §3.2). */
export function stateDetail(row: RosterRow): StateDetail {
  switch (row.state) {
    case 'waiting':
      return { text: waitingDetail(row) }
    case 'working':
      return { text: row.current_tool ? (row.current_mcp_server ? `mcp: ${row.current_mcp_server} · ${row.current_tool}` : row.current_tool) : 'working' }
    case 'failed':
      return { text: row.state_reason || row.end_reason || 'failed' }
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

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`
}

/** "▾ 2 agents" / "▸ 2 agents · 1 running" (03 §3.2.1). */
export function agentsToggle(total: number, running: number, open: boolean): string {
  const base = `${open ? '▾' : '▸'} ${plural(total, 'agent')}`
  return !open && running > 0 && running < total ? `${base} · ${running} running` : base
}

/** `HH:MM` of an instant in local time. */
export function clock(iso: string | null | undefined): string {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`
}

/** `HH:MM:SS` of an instant in local time. */
export function clockSeconds(iso: string | null | undefined): string {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  return `${clock(iso)}:${String(d.getSeconds()).padStart(2, '0')}`
}

/** The file-kind letter of the Files tab (session-detail.html callout 12). */
export function changeLetter(kind: string): string {
  switch (kind) {
    case 'added':
    case 'untracked':
      return 'A'
    case 'deleted':
      return 'D'
    case 'renamed':
      return 'R'
    default:
      return 'M'
  }
}

/** `~` modified · `+` added · `−` deleted, as in the roster detail panel's file rows. */
export function changeGlyph(kind: string): string {
  switch (kind) {
    case 'added':
    case 'untracked':
      return '+'
    case 'deleted':
      return '−'
    default:
      return '~'
  }
}

export function parseConnectorDetail(row: Pick<ConnectorRow, 'detail_json'>): ConnectorDetail {
  if (!row.detail_json) return {}
  try {
    const v = JSON.parse(row.detail_json) as unknown
    return v && typeof v === 'object' ? (v as ConnectorDetail) : {}
  } catch {
    return {}
  }
}

/** The state column of a Connectors row (10 §7.4.9). */
export function connectorState(status: string): StateKind {
  switch (status) {
    case 'connected':
    case 'pending':
    case 'failed':
    case 'allowed':
    case 'denied':
      return status
    default:
      return 'pending'
  }
}

/** One-line meaning of a permission mode (03 §3.9B). */
export function permissionModeMeaning(mode: string | null | undefined): string {
  switch (mode) {
    case 'plan':
      return 'plans first, edits nothing until you approve'
    case 'acceptEdits':
      return 'file edits are accepted without asking'
    case 'auto':
      return 'Claude Code decides which tools need approval'
    case 'dontAsk':
      return 'tools not already allowed are refused without asking'
    case 'bypassPermissions':
      return 'every tool runs without asking'
    default:
      return 'asks before tools that are not allowed yet'
  }
}
