// What each Integrations card says (03 §7C.1–§7C.5), as pure functions of the
// core's answers: the status line (glyph word + detail), the count lines, the
// one primary action. Also the unified-diff reader of the hook install preview
// (03 §7C.3) and the relative times of the count lines ("12 s ago").
import type { BrandId } from '../../kit/brands'
import type { StateKind } from '../../kit/types'
import type { AiBackendsStatus, Diagnostics, HookConflict, HookInstallStatus, HooksStatus } from '../../state/contract-settings'
import type { AiBackendRow, AiHealth, ClaudeCodeCard, ConnectorCardStatus } from '../../state/contract-connectors'
import type { CatalogueResult } from '../../state/contract-packs'

/** "12 s ago", "2 m ago", "3 h ago", "4 d ago" (the wireframe's count-line form). */
export function ago(iso: string | null | undefined, now: number): string | null {
  if (!iso) return null
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return null
  const s = Math.max(0, Math.round((now - t) / 1000))
  if (s < 60) return `${s} s ago`
  if (s < 3600) return `${Math.floor(s / 60)} m ago`
  if (s < 86_400) return `${Math.floor(s / 3600)} h ago`
  return `${Math.floor(s / 86_400)} d ago`
}

/** `/Users/ana/.claude/settings.json` → `~/.claude/settings.json`. */
export function tilde(path: string | null | undefined): string {
  if (!path) return ''
  return path.replace(/^\/(?:Users|home)\/[^/]+(?=\/)/, '~').replace(/^[A-Za-z]:\\Users\\[^\\]+(?=\\)/, '~')
}

export type CardLine = string | { before: string; code: string; after?: string; /** A 12-px brand mark before the text (the AI backends). */ mark?: BrandId }

export interface CardStatus {
  state: StateKind
  /** The glyph word: connected, not connected, needs sign-in, error, offline, or the hook problem. */
  label: string
  /** After the word; the stylesheet draws the " · " between them (`.m-integration-detail::before`), never the string. */
  detail?: string
}

export type ClaudeAction = 'install' | 'update' | 'repair' | 'docs'

export interface ClaudeCard {
  status: CardStatus
  lines: CardLine[]
  action: { kind: ClaudeAction; label: string }
  /** `~/.claude` missing and `claude` not on PATH (03 §7C.5). */
  missing: boolean
}

export function normaliseHookStatus(s: HookInstallStatus | string | null | undefined): 'installed' | 'stale' | 'missing' | 'conflict' | 'none' {
  if (s === 'installed' || s === 'stale' || s === 'missing' || s === 'conflict') return s
  return 'none'
}

const HOOK_PROBLEM: Record<'stale' | 'missing' | 'conflict', string> = {
  stale: 'hooks need updating',
  missing: 'hooks were removed',
  conflict: 'another hook is registered',
}

/** The Claude Code card (03 §7C.2): `hook_installs.status` + the last round trip + `claude --version`. */
/** What `health.changed` says about Claude Code (used until `diagnostics_get` answers). */
export interface ClaudeHealth {
  claude_code?: 'found' | 'missing' | string
  claude_code_version?: string | null
}

export function claudeCodeCard(status: HooksStatus | null, diag: Diagnostics | null, now: number, health?: ClaudeHealth | null, core?: ClaudeCodeCard | null): ClaudeCard {
  // `claude_code_card` (the core's card) fills in until `hooks_status` / `diagnostics_get` answer.
  const hs = normaliseHookStatus(status?.status ?? core?.hooks)
  const claudeFound = diag ? diag.claude.found : health?.claude_code !== 'missing'
  const settingsPath = tilde(status?.path ?? diag?.hooks[0]?.settings_path ?? '~/.claude/settings.json') || '~/.claude/settings.json'
  const cliVersion = diag?.claude.version ?? health?.claude_code_version ?? null
  const cli = cliVersion ? `CLI ${cliVersion}` : !claudeFound ? 'CLI not found' : diag || health ? 'CLI version unknown' : 'CLI …'
  const last = ago(diag?.ipc.last_event_at, now)
  const tracked = diag?.sessions_tracked ?? core?.sessions_tracked
  const line2 = `${last ? `last hook event ${last}` : 'no hook events yet'}${tracked !== undefined ? ` · ${tracked} ${tracked === 1 ? 'session' : 'sessions'} tracked` : ''}`

  if (!claudeFound && hs === 'none') {
    return {
      status: { state: 'idle', label: 'not connected' },
      lines: ['Claude Code was not found. Install it to track sessions.'],
      action: { kind: 'docs', label: 'Open docs' },
      missing: true,
    }
  }
  if (hs === 'none') {
    return {
      status: { state: 'idle', label: 'not connected', detail: 'hooks not installed' },
      lines: [{ before: `${cli} · Mode edits only `, code: settingsPath }, line2],
      action: { kind: 'install', label: 'Install hooks' },
      missing: false,
    }
  }
  const version = status?.version ?? diag?.relay.version
  const line1: CardLine = { before: `${cli} · hooks ${version ? `v${version}` : '(version unknown)'} in `, code: settingsPath }
  if (hs !== 'installed') {
    return {
      status: { state: 'failed', label: HOOK_PROBLEM[hs], detail: status?.detail ? `${status.detail}` : undefined },
      lines: [line1, line2],
      action: hs === 'stale' ? { kind: 'update', label: 'Update hooks' } : { kind: 'repair', label: 'Repair' },
      missing: false,
    }
  }
  const rt = status?.last_round_trip
  if (rt && !rt.acked) {
    return {
      status: { state: 'failed', label: 'error', detail: 'the last round trip got no answer' },
      lines: [line1, line2],
      action: { kind: 'repair', label: 'Repair' },
      missing: false,
    }
  }
  const relay = diag && !diag.core.ipc_listening ? 'relay not listening' : 'relay connected'
  return {
    status: { state: 'connected', label: 'connected', detail: `hooks installed · ${relay}` },
    lines: [line1, line2],
    action: { kind: 'install', label: 'Install hooks' },
    missing: false,
  }
}

export interface AiCard {
  status: CardStatus
  lines: CardLine[]
  action: { kind: 'key' | 'test'; label: string }
}

function hostOf(url: string): string {
  return url.replace(/^https?:\/\//, '').replace(/\/+$/, '').replace(/^127\.0\.0\.1(?=:)/, 'localhost')
}

const BACKEND_LABEL: Record<string, string> = { claude_api: 'Claude API', ollama: 'Ollama', lm_studio: 'LM Studio', openai_compat: 'OpenAI-compatible' }

/** The card's status from `ai_health` (06 §10.7 states); an added provider is named (06 §10.1A). */
export function aiHealthStatus(h: AiHealth): CardStatus {
  const name = h.label || BACKEND_LABEL[h.backend] || h.backend
  const local = h.backend === 'ollama' || h.backend === 'lm_studio'
  switch (h.state) {
    case 'ready':
      return { state: 'connected', label: 'connected', detail: local ? `${name} reachable` : h.backend === 'claude_api' && h.provider === 'claude_api' ? 'Claude API key present' : `${name} key present` }
    case 'no_key':
      return { state: 'idle', label: 'not connected', detail: 'no API key' }
    case 'invalid_key':
      return { state: 'failed', label: 'error', detail: 'the API key was refused' }
    case 'unreachable':
      return { state: 'failed', label: 'error', detail: `${name} unreachable` }
    case 'budget_reached':
      return { state: 'failed', label: 'error', detail: 'monthly token budget reached' }
    case 'local_only_blocked':
      return { state: 'idle', label: 'not connected', detail: 'local-only is on' }
    default:
      return { state: 'idle', label: 'not connected', detail: h.message ? `${h.message}` : undefined }
  }
}

/** What the card reads besides `ai_backends_status`: `ai_health` (the active backend's state) and `ai_backends_list` (the stored key). */
export interface AiCardInput {
  health?: AiHealth | null
  rows?: readonly AiBackendRow[] | null
}

/**
 * The AI backends card (03 §7C.2): the active backend's state (`ai_health`
 * when it answered), one line per local server (`ai_backends_status`), the
 * model named at most once on the card (the first model of the first
 * reachable local server), no pricing, no slogan.
 */
export function aiCard(s: AiBackendsStatus | null, input: AiCardInput = {}): AiCard {
  const health = input.health ?? null
  const keyRow = input.rows?.find((r) => r.backend === 'claude_api')
  if (!s && health) {
    const needsKey = health.state === 'no_key' || health.state === 'invalid_key'
    return { status: aiHealthStatus(health), lines: [], action: needsKey ? { kind: 'key', label: 'Add API key' } : { kind: 'test', label: 'Test call' } }
  }
  if (!s) return { status: { state: 'idle', label: 'not connected', detail: 'no backend' }, lines: [], action: { kind: 'key', label: 'Add API key' } }
  let named = false
  const local = (name: string, b: AiBackendsStatus['ollama']): CardLine => {
    if (!b.reachable) return `${name} · not running`
    const host = hostOf(b.url)
    let tail: string
    if (!b.models.length) tail = 'no models'
    else if (!named) {
      named = true
      tail = `${b.models[0]} ready`
    } else tail = `${b.models.length} ${b.models.length === 1 ? 'model' : 'models'}`
    return { mark: name === 'Ollama' ? 'ollama' : 'lmstudio', before: `${name} · `, code: host, after: ` · ${tail}` }
  }
  const lines = [local('Ollama', s.ollama), local('LM Studio', s.lm_studio)]
  const keyStored = keyRow?.key_stored ?? s.claude_api.key_stored
  let status: CardStatus
  if (health && health.backend === s.backend) {
    status = aiHealthStatus(health)
  } else if (s.backend === 'claude_api') {
    status = keyStored ? { state: 'connected', label: 'connected', detail: 'Claude API key present' } : { state: 'idle', label: 'not connected', detail: 'no API key' }
  } else {
    const b = s.backend === 'ollama' ? s.ollama : s.lm_studio
    const name = s.backend === 'ollama' ? 'Ollama' : 'LM Studio'
    status = b.reachable ? { state: 'connected', label: 'connected', detail: `${name} reachable` } : { state: 'failed', label: 'error', detail: `${name} unreachable` }
  }
  const needsKey = s.backend === 'claude_api' && (!keyStored || health?.state === 'invalid_key')
  return { status, lines, action: needsKey ? { kind: 'key', label: 'Add API key' } : { kind: 'test', label: 'Test call' } }
}

export type ConnectorKind = 'gmail' | 'slack' | 'google-calendar'

/** The `connectors_status` card line of a connector card (`gmail`, `calendar`, `slack`). */
export const CORE_CARD: Record<ConnectorKind, 'gmail' | 'calendar' | 'slack'> = { gmail: 'gmail', slack: 'slack', 'google-calendar': 'calendar' }

const CORE_STATE: Record<ConnectorCardStatus['state'], { state: StateKind; label: string }> = {
  connected: { state: 'connected', label: 'connected' },
  not_connected: { state: 'idle', label: 'not connected' },
  needs_sign_in: { state: 'failed', label: 'needs sign-in' },
  error: { state: 'failed', label: 'error' },
  offline: { state: 'idle', label: 'offline' },
  rate_limited: { state: 'pending', label: 'rate limited' },
}

/**
 * A connector card's status line with the core's word (`connectors_status`,
 * 06 §9A: worst health wins) over the one the accounts give: when both agree
 * the accounts' detail stays (it names the watched sources' sync), otherwise
 * the core's word and line are shown.
 */
export function withCoreStatus(live: CardStatus, core: ConnectorCardStatus | null | undefined): CardStatus {
  if (!core) return live
  const mapped = CORE_STATE[core.state]
  if (!mapped || mapped.label === live.label) return live
  const line = core.line && core.line !== mapped.label ? core.line : null
  return { state: mapped.state, label: mapped.label, detail: line ? `${line}` : live.detail }
}

/* ---- the diff preview ----------------------------------------------------------- */

export interface DiffLine {
  kind: 'add' | 'del' | 'ctx' | 'hunk' | 'file'
  text: string
  /** Old / new line numbers (context and changes). */
  old?: number
  new?: number
  /** A foreign hook on one of Mode's events, kept as is (03 §7C.3 gutter note). */
  keep?: boolean
}

/** A unified diff (`similar`'s output) as lines, with the conflict gutter marked. */
export function parseUnifiedDiff(diff: string, conflicts: readonly HookConflict[] = []): DiffLine[] {
  const out: DiffLine[] = []
  let o = 0
  let n = 0
  const foreign = conflicts.map((c) => c.command).filter(Boolean)
  for (const raw of diff.replace(/\r\n/g, '\n').split('\n')) {
    if (raw === '' && out.length && out[out.length - 1]!.kind !== 'ctx') continue
    if (raw.startsWith('---') || raw.startsWith('+++')) {
      out.push({ kind: 'file', text: raw })
      continue
    }
    const h = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(raw)
    if (h) {
      o = Number(h[1])
      n = Number(h[2])
      out.push({ kind: 'hunk', text: raw })
      continue
    }
    if (raw.startsWith('+')) out.push({ kind: 'add', text: raw.slice(1), new: n++ })
    else if (raw.startsWith('-')) out.push({ kind: 'del', text: raw.slice(1), old: o++ })
    else if (raw.startsWith('\\')) continue
    else {
      const text = raw.startsWith(' ') ? raw.slice(1) : raw
      out.push({ kind: 'ctx', text, old: o++, new: n++, keep: foreign.some((c) => text.includes(c)) })
    }
  }
  while (out.length && out[out.length - 1]!.kind === 'ctx' && out[out.length - 1]!.text === '') out.pop()
  return out
}

/** Diff stats for the dialog header ("+18 −0"). */
export function diffStats(lines: readonly DiffLine[]): { added: number; removed: number } {
  return { added: lines.filter((l) => l.kind === 'add').length, removed: lines.filter((l) => l.kind === 'del').length }
}

/* ---- the Marketplace card (03 §7C.6) --------------------------------------------------- */

export interface MarketplaceCard {
  status: CardStatus
  lines: CardLine[]
  action: { kind: 'browse' | 'retry'; label: string }
}

/**
 * The Marketplace card: `◉` connected "· 3 packs · fetched 2 m ago" once the
 * catalog answered; `○` not fetched before the first answer; `✕` error with
 * the core's sentence when the fetch failed (the held catalog, if any, is
 * still browsable). The lines name what packs are and where the index lives.
 */
export function marketplaceCard(r: CatalogueResult | null, now: number, loaded: boolean): MarketplaceCard {
  const host = r?.host || 'raw.githubusercontent.com'
  const lines: CardLine[] = ['Agents, automations and themes as packs', { before: 'index ', code: host, after: ' · nothing is sent' }]
  const c = r?.catalogue ?? null
  if (r?.error && !c) return { status: { state: 'failed', label: 'error', detail: `${r.error}` }, lines, action: { kind: 'retry', label: 'Retry' } }
  if (!c) return { status: { state: 'idle', label: loaded ? 'not fetched' : 'not fetched', detail: loaded ? 'the catalog did not answer' : 'fetching the catalog' }, lines, action: { kind: 'browse', label: 'Browse' } }
  const n = c.packs.length
  const when = ago(c.fetched_at, now)
  const detail = `${n} ${n === 1 ? 'pack' : 'packs'}${when ? ` · fetched ${when}` : ''}`
  if (r?.error) return { status: { state: 'failed', label: 'error', detail: `${r.error}` }, lines, action: { kind: 'retry', label: 'Retry' } }
  return { status: { state: 'connected', label: 'connected', detail }, lines, action: { kind: 'browse', label: 'Browse' } }
}
