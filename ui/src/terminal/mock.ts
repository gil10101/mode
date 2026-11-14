// The PTY host in the browser mock (outside Tauri; test scaffolding only).
// Registers the terminal commands of contract-terminal.ts on
// `window.__mode_mock.respond` and runs a tiny fake PTY per embedded
// session: it echoes input, understands bracketed paste, answers a few
// commands (`ls`, `claude`, `colors`, `link`, `echo`, `clear`, `exit`) in
// ANSI color, and plays the prompt box's round trip (PTY stdin, or a
// headless turn when the session has no PTY) the way the core would:
// `UserPromptSubmit` → a `user` thread item and `working`, then a reply and
// `idle`.
//
// The app mock (src/app/mock.ts) owns the sample sessions. This mock layers
// its own facts on top (driver, alive, state, extra thread items) by
// wrapping the mock's `answer` (for `session_detail` / `sessions_list`) and
// `emit` (for `sessions.changed`), and hooks launch / resume to spawn a PTY.
//
// `window.__mode_pty` lets Playwright read the emulator's screen and steer
// the fake host (see `PtyTestHook`).
import { mock, type ModeMock } from '../tauri'
import { EVENTS } from '../state/contract'
import type { AppRosterRow, SessionDetail, TranscriptItem } from '../state/contract-sessions'
import { TERMINAL_EVENTS, type PtyOpenResult, type PromptDelivered } from '../state/contract-terminal'
import { base64ToBytes, bytesToBase64, bytesToText, concatBytes, textToBytes } from './base64'
import { terminals } from './registry'

type Args = Record<string, unknown> | undefined

/** Scrollback kept for `pty_open` replay. */
const BUFFER_BYTES = 256 * 1024
/** The real host waits 50 ms between the bracketed paste and `\r` (05 §14). */
const ENTER_DELAY_MS = 50
const SUBMIT_DELAY_MS = 400
const REPLY_DELAY_MS = 1200
const HEADLESS_DELAY_MS = 2500
const CLOSE_DELAY_MS = 800

interface FakePty {
  id: string
  cols: number
  rows: number
  chunks: Uint8Array[]
  size: number
  /** Stream offset of `chunks[0]`. */
  start: number
  /** Stream offset past the last byte. */
  end: number
  line: string
  esc: string
  pasting: boolean
  exited: number | null
  /** Everything `pty_write` delivered, decoded (for tests). */
  input: string
}

interface Overlay {
  driver?: string
  alive?: boolean
  state?: string
  ended_at?: string | null
  end_reason?: string | null
  touched?: string
  thread: TranscriptItem[]
}

interface MockState {
  ptys: Map<string, FakePty>
  overlays: Map<string, Overlay>
  timers: Map<string, ReturnType<typeof setTimeout>[]>
  turns: Map<string, string>
  closing: Set<string>
  seq: number
}

let state: MockState = fresh()
const wrapped = new WeakSet<ModeMock>()

function fresh(): MockState {
  return { ptys: new Map(), overlays: new Map(), timers: new Map(), turns: new Map(), closing: new Set(), seq: 0 }
}

const ESC = '\x1b'
const c = (code: string, text: string) => `${ESC}[${code}m${text}${ESC}[0m`
const visible = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, '').replace(/\x1b\]8;;[^\x07]*\x07/g, '')

/* ---- overlay on the app mock --------------------------------------------- */

function overlay(id: string): Overlay {
  let o = state.overlays.get(id)
  if (!o) state.overlays.set(id, (o = { thread: [] }))
  return o
}

function overlayRow(r: AppRosterRow): AppRosterRow {
  const o = state.overlays.get(r.id)
  if (!o) return r
  const next = { ...r }
  if (o.state) next.state = o.state
  if (o.state === 'working' || o.state === 'idle' || o.state === 'done') next.waiting_kind = null
  if ('ended_at' in o) next.ended_at = o.ended_at ?? null
  if (o.touched) {
    next.last_event_at = o.touched
    next.state_seconds = 0
  }
  return next
}

function overlayRows(rows: unknown): unknown {
  return Array.isArray(rows) ? (rows as AppRosterRow[]).map(overlayRow) : rows
}

function overlayDetail(d: SessionDetail): SessionDetail {
  const o = state.overlays.get(d.session.id)
  if (!o) return d
  const session = { ...d.session }
  if (o.state) {
    session.state = o.state
    if (o.touched) session.state_since = o.touched
  }
  if ('ended_at' in o) {
    session.ended_at = o.ended_at ?? null
    session.end_reason = o.end_reason ?? null
  }
  return { ...d, session, driver: o.driver ?? d.driver, alive: o.alive ?? d.alive, thread: o.thread.length ? [...d.thread, ...o.thread] : d.thread }
}

/** Push the roster the way the core does after a state change (the emit wrapper applies the overlay). */
function push(id: string) {
  overlay(id).touched = new Date().toISOString()
  const m = mock()
  m.emit(EVENTS.sessions, { rows: m.rows })
}

function later(id: string, ms: number, run: () => void) {
  const t = setTimeout(run, ms)
  const list = state.timers.get(id) ?? []
  list.push(t)
  state.timers.set(id, list)
}

function cancel(id: string) {
  for (const t of state.timers.get(id) ?? []) clearTimeout(t)
  state.timers.delete(id)
}

function addItem(id: string, item: Omit<TranscriptItem, 'seq' | 'at'> & Record<string, unknown>) {
  overlay(id).thread.push({ ...item, seq: 100_000 + ++state.seq, at: new Date().toISOString() } as TranscriptItem)
}

/* ---- the fake PTY ---------------------------------------------------------- */

function rowOf(id: string): AppRosterRow | undefined {
  return mock().rows.find((r) => r.id === id) as AppRosterRow | undefined
}

function home(path: string): string {
  return path.replace(/^\/(Users|home)\/[^/]+/, '~')
}

function out(p: FakePty, text: string) {
  const bytes = textToBytes(text)
  if (!bytes.length) return
  const offset = p.end
  p.chunks.push(bytes)
  p.size += bytes.length
  p.end += bytes.length
  while (p.size > BUFFER_BYTES && p.chunks.length > 1) {
    const first = p.chunks.shift()!
    p.size -= first.length
    p.start += first.length
  }
  mock().emit(TERMINAL_EVENTS.output, { session_id: p.id, data: bytesToBase64(bytes), offset })
}

function box(lines: string[]): string {
  const width = Math.max(...lines.map((l) => [...visible(l)].length)) + 2
  const edge = (a: string, b: string) => c('90', a + '─'.repeat(width) + b)
  const body = lines.map((l) => `${c('90', '│')} ${l}${' '.repeat(width - 1 - [...visible(l)].length)}${c('90', '│')}`)
  return [edge('╭', '╮'), ...body, edge('╰', '╯')].join('\r\n') + '\r\n'
}

function banner(id: string): string {
  const r = rowOf(id)
  const place = [r ? home(r.cwd) : '~', r?.git_branch].filter(Boolean).join(' · ')
  return (
    box([`${c('33', '✻')} Welcome to ${c('1', 'Claude Code')}`, '', `  ${c('90', place)}`]) +
    `\r\n  ${c('90', 'Try')} ls${c('90', ',')} colors${c('90', ',')} link ${c('90', 'or')} exit${c('90', '. Anything else is a prompt.')}\r\n\r\n`
  )
}

function prompt(p: FakePty) {
  out(p, `${c('33', '>')} `)
}

function spawn(id: string): FakePty {
  const existing = state.ptys.get(id)
  if (existing && existing.exited === null) return existing
  const p: FakePty = { id, cols: 80, rows: 24, chunks: [], size: 0, start: 0, end: 0, line: '', esc: '', pasting: false, exited: null, input: '' }
  state.ptys.set(id, p)
  // Bracketed paste on, as the CLI does at start.
  out(p, `${ESC}[?2004h`)
  out(p, banner(id))
  prompt(p)
  mock().emit(TERMINAL_EVENTS.started, { session_id: id, cols: p.cols, rows: p.rows })
  return p
}

const LS = `${c('1;34', 'crates')}  ${c('1;34', 'docs')}  ${c('1;34', 'migrations')}  ${c('1;34', 'ui')}  Cargo.toml  ${c('32', 'Makefile')}  README.md\r\n`

function swatch(): string {
  const names = ['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white']
  const normal = names.map((n, i) => c(`3${i}`, n)).join(' ')
  const bright = names.map((n, i) => c(`9${i}`, n)).join(' ')
  const ramp = Array.from({ length: 24 }, (_, i) => `${ESC}[48;5;${232 + i}m ${ESC}[0m`).join('')
  return `${normal}\r\n${bright}\r\n${ramp}\r\n${c('1', 'bold')} ${c('3', 'italic')} ${c('4', 'underline')} ${c('7', 'inverse')}\r\n`
}

function exit(p: FakePty, code: number, delay = 0) {
  if (p.exited !== null) return
  later(p.id, delay, () => {
    p.exited = code
    state.closing.delete(p.id)
    out(p, `${ESC}[?2004l`)
    const o = overlay(p.id)
    o.driver = 'none'
    o.alive = false
    o.state = code === 0 ? 'done' : 'failed'
    o.ended_at = new Date().toISOString()
    o.end_reason = 'prompt_input_exit'
    addItem(p.id, { kind: 'end', reason: 'prompt_input_exit' })
    mock().emit(TERMINAL_EVENTS.exited, { session_id: p.id, code })
    push(p.id)
  })
}

function run(p: FakePty) {
  const line = p.line
  p.line = ''
  out(p, '\r\n')
  const [name, ...rest] = line.trim().split(/\s+/)
  switch (name) {
    case '':
      break
    case 'ls':
      out(p, LS)
      break
    case 'claude':
      out(p, banner(p.id))
      break
    case 'colors':
      out(p, swatch())
      break
    case 'link':
      out(p, `${ESC}]8;;https://example.com/mode${ESC}\\the Mode docs${ESC}]8;;${ESC}\\ ${c('90', '(a hyperlink)')}\r\n`)
      break
    case 'echo':
      out(p, `${rest.join(' ')}\r\n`)
      break
    case 'clear':
      out(p, `${ESC}[2J${ESC}[3J${ESC}[H`)
      break
    case 'exit':
      exit(p, Number(rest[0]) || 0)
      return
    default:
      // Anything else is a prompt to Claude: a reply line once the turn ends.
      out(p, `${c('32', '⏺')} ${c('90', 'Working on it…')}\r\n`)
  }
  prompt(p)
}

/** One escape sequence from the input is complete. */
function escDone(seq: string): boolean {
  if (seq.length < 2) return false
  if (seq[1] === '[') return seq.length > 2 && /[\x40-\x7e]/.test(seq[seq.length - 1])
  if (seq[1] === 'O') return seq.length >= 3
  return true
}

function feed(p: FakePty, text: string) {
  if (p.exited !== null) return
  for (const ch of text) {
    if (p.esc) {
      p.esc += ch
      if (escDone(p.esc)) {
        if (p.esc === `${ESC}[200~`) p.pasting = true
        else if (p.esc === `${ESC}[201~`) p.pasting = false
        p.esc = ''
      }
      continue
    }
    if (ch === ESC) {
      p.esc = ch
      continue
    }
    if (p.pasting) {
      p.line += ch === '\r' ? '\n' : ch
      out(p, ch === '\r' ? '\r\n' : ch)
      continue
    }
    if (ch === '\r') run(p)
    else if (ch === '\x7f' || ch === '\b') {
      if (p.line) {
        p.line = p.line.slice(0, -1)
        out(p, '\b \b')
      }
    } else if (ch === '\x03') {
      p.line = ''
      out(p, '^C\r\n')
      prompt(p)
    } else if (ch === '\x04') {
      if (!p.line) exit(p, 0)
    } else if (ch === '\x0c') {
      out(p, `${ESC}[2J${ESC}[H`)
      prompt(p)
    } else if (ch >= ' ') {
      p.line += ch
      out(p, ch)
    }
  }
}

/* ---- the prompt round trip --------------------------------------------- */

function viaPty(id: string, text: string): PromptDelivered {
  const p = spawn(id)
  // 05 §14: bracketed paste, then `\r` after 50 ms; the terminal shows it as typed.
  feed(p, `${ESC}[200~${text}${ESC}[201~`)
  later(id, ENTER_DELAY_MS, () => feed(p, '\r'))
  later(id, SUBMIT_DELAY_MS, () => {
    addItem(id, { kind: 'user', text })
    overlay(id).state = 'working'
    push(id)
  })
  later(id, SUBMIT_DELAY_MS + REPLY_DELAY_MS, () => {
    addItem(id, { kind: 'assistant', text: `Done: ${text}` })
    overlay(id).state = 'idle'
    out(p, `${c('32', '⏺')} Done.\r\n`)
    prompt(p)
    push(id)
  })
  return { delivered_via: 'pty' }
}

function finishTurn(id: string, outcome: 'finished' | 'stopped') {
  const turn = state.turns.get(id)
  if (!turn) return
  cancel(id)
  state.turns.delete(id)
  const o = overlay(id)
  o.driver = 'none'
  o.state = 'idle'
  if (outcome === 'stopped') addItem(id, { kind: 'notice', text: 'turn stopped' })
  mock().emit(TERMINAL_EVENTS.turnFinished, { session_id: id, turn_id: turn, outcome })
  push(id)
}

function viaHeadless(id: string, text: string): PromptDelivered {
  const turn = `turn-${++state.seq}`
  state.turns.set(id, turn)
  const o = overlay(id)
  o.driver = 'headless'
  o.state = 'working'
  mock().emit(TERMINAL_EVENTS.turnStarted, { session_id: id, turn_id: turn })
  push(id)
  later(id, SUBMIT_DELAY_MS, () => {
    addItem(id, { kind: 'user', text })
    push(id)
  })
  later(id, HEADLESS_DELAY_MS, () => {
    addItem(id, { kind: 'assistant', text: `Done: ${text}` })
    finishTurn(id, 'finished')
  })
  return { delivered_via: 'headless', turn_id: turn }
}

/* ---- install --------------------------------------------------------------- */

/** What Playwright can do through `window.__mode_pty`. */
export interface PtyTestHook {
  /** The emulator's buffer text for a session ('' when no Terminal tab is attached). */
  screen(id: string): string
  /** Color of the first cell of `needle` in the emulator: `palette:<n>`, `#rrggbb` or `default`. */
  colorOf(id: string, needle: string): string | null
  renderer(id: string): string | null
  size(id: string): { cols: number; rows: number } | null
  theme(id: string): { background?: string; foreground?: string; cursor?: string; red?: string } | null
  /** Everything the fake PTY received through `pty_write`, decoded. */
  input(id: string): string
  /** Bytes the fake PTY prints. */
  print(id: string, text: string): void
  /** The PTY goes away while the session stays alive (driver `none`): the next prompt runs headless. */
  detach(id: string): void
  /** The process exits with `code`. */
  exit(id: string, code: number): void
}

declare global {
  interface Window {
    __mode_pty?: PtyTestHook
  }
}

async function baseDetail(m: ModeMock, answer: ModeMock['answer'], id: string): Promise<SessionDetail | null> {
  const n = m.invokes.length
  const p = answer('session_detail', { session_id: id })
  // Not a call the UI made: keep `invokes` to what tests expect.
  m.invokes.splice(n, 1)
  try {
    return ((await p) as SessionDetail | null) ?? null
  } catch {
    return null
  }
}

/**
 * Registers the PTY host's commands on the browser mock. Install after the
 * app mock (so `session_prompt` is this one). Idempotent: installing again
 * drops every fake PTY.
 */
export function installTerminalMock(): void {
  const m = mock()
  for (const id of state.timers.keys()) cancel(id)
  state = fresh()

  if (!wrapped.has(m)) {
    wrapped.add(m)
    const answer = m.answer.bind(m)
    const emit = m.emit.bind(m)
    m.answer = async (cmd, args) => {
      const id = typeof args?.session_id === 'string' ? args.session_id : ''
      // Resume and launch give the session a fresh PTY (05 §8.1–§8.2); never while a headless turn runs (05 §14).
      if (cmd === 'session_resume' && id) {
        if (state.turns.has(id)) throw new Error('Stop the running turn first')
        cancel(id)
        state.overlays.delete(id)
        state.ptys.delete(id)
        state.turns.delete(id)
      }
      const result = await answer(cmd, args)
      if (cmd === 'session_detail' && result) return overlayDetail(result as SessionDetail)
      if (cmd === 'sessions_list') return overlayRows(result)
      if ((cmd === 'session_resume' || cmd === 'session_launch') && result && typeof (result as { session_id?: unknown }).session_id === 'string') spawn((result as { session_id: string }).session_id)
      return result
    }
    m.emit = (event, payload) => {
      if (event === EVENTS.sessions && payload && typeof payload === 'object' && 'rows' in payload) return emit(event, { ...(payload as object), rows: overlayRows((payload as { rows: unknown }).rows) })
      return emit(event, payload)
    }
  }
  const answer = m.answer

  m.respond('pty_open', async (args: Args) => {
    const id = String(args?.session_id ?? '')
    const d = await baseDetail(m, answer, id)
    const existing = state.ptys.get(id)
    if (!d || (!existing && (d.driver !== 'pty' || !d.alive))) throw new Error('This session has no terminal')
    const p = existing ?? spawn(id)
    const cols = Number(args?.cols) || p.cols
    const rows = Number(args?.rows) || p.rows
    p.cols = cols
    p.rows = rows
    const result: PtyOpenResult = { cols, rows, data: bytesToBase64(concatBytes(p.chunks.length ? p.chunks : [new Uint8Array()])), offset: p.start }
    if (p.exited !== null) {
      result.exited = true
      result.exit_code = p.exited
    }
    return result
  })

  m.respond('pty_write', (args: Args) => {
    const p = state.ptys.get(String(args?.session_id ?? ''))
    if (!p || p.exited !== null) throw new Error('This session has no terminal')
    const text = bytesToText(base64ToBytes(String(args?.data ?? '')))
    p.input += text
    feed(p, text)
    return undefined
  })

  m.respond('pty_resize', (args: Args) => {
    const p = state.ptys.get(String(args?.session_id ?? ''))
    if (p) {
      p.cols = Number(args?.cols) || p.cols
      p.rows = Number(args?.rows) || p.rows
    }
    return undefined
  })

  m.respond('pty_detach', () => undefined)
  m.respond('pty_close', (args: Args) => {
    const id = String(args?.session_id ?? '')
    const p = state.ptys.get(id)
    if (!p || p.exited !== null) throw new Error('This session has no terminal')
    state.closing.add(id)
    // SIGINT; the CLI finishes the turn, then exits.
    out(p, '^C\r\n')
    const working = overlay(id).state === 'working'
    exit(p, 0, CLOSE_DELAY_MS + (working ? REPLY_DELAY_MS : 0))
    return undefined
  })

  m.respond('session_prompt', async (args: Args) => {
    const id = String(args?.session_id ?? '')
    const text = String(args?.text ?? '')
    const d = await baseDetail(m, answer, id)
    const row = rowOf(id)
    if (!d) throw new Error('This session is no longer available')
    const st = (row ? overlayRow(row).state : null) ?? d.session.state
    if (d.driver === 'headless' || state.turns.has(id)) throw new Error('A turn is running')
    if (st === 'working') throw new Error('Claude is working')
    if (d.pending || row?.pending) throw new Error('Answer the pending card first')
    if (!d.alive || d.session.ended_at) throw new Error('Resume to continue')
    const p = state.ptys.get(id)
    if ((p && p.exited === null) || (!p && d.driver === 'pty')) return viaPty(id, text)
    return viaHeadless(id, text)
  })

  m.respond('turn_stop', (args: Args) => {
    const id = String(args?.session_id ?? '')
    if (!state.turns.has(id)) throw new Error('No turn is running')
    later(id, 300, () => finishTurn(id, 'stopped'))
    return undefined
  })

  if (typeof window !== 'undefined')
    window.__mode_pty = {
      screen: (id) => terminals.get(id)?.text() ?? '',
      colorOf: (id, needle) => terminals.get(id)?.colorOf(needle) ?? null,
      renderer: (id) => terminals.get(id)?.renderer() ?? null,
      size: (id) => terminals.get(id)?.size() ?? null,
      theme: (id) => terminals.get(id)?.theme() ?? null,
      input: (id) => state.ptys.get(id)?.input ?? '',
      print: (id, text) => {
        const p = state.ptys.get(id)
        if (p) out(p, text)
      },
      detach: (id) => {
        const p = state.ptys.get(id)
        state.ptys.delete(id)
        const o = overlay(id)
        o.driver = 'none'
        o.alive = true
        if (p) mock().emit(TERMINAL_EVENTS.exited, { session_id: id, code: 0 })
        push(id)
      },
      exit: (id, code) => {
        const p = state.ptys.get(id)
        if (p) exit(p, code)
      },
    }
}
