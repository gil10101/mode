// Terminal-layer state per session: the PTY's status as the Terminal tab
// shows it, the prompt box's delivery, and the "Stop turn" in flight. Fed by
// the PTY host's events (`pty.started`, `pty.exited`, `turn.*`), connected
// once by the first Terminal tab or prompt box that mounts.
import { create } from 'zustand'
import { invoke, listen } from '../tauri'
import { cmdErrorAction, toCmdError, type CmdErrorAction } from '../state/contract'
import { TERMINAL_EVENTS, type PromptDelivered, type PromptRefusal, type PtyExited, type PtyStarted, type TurnFinished, type TurnStarted } from '../state/contract-terminal'
import type { TranscriptItem } from '../state/contract-sessions'
import { useSessionsStore } from '../state/sessions-store'
import { jumpToTerminal, useDialogs } from '../app/sessions/actions'
import { CONFIRM_MS, onAccepted, onRefused, onThread, onTimeout, startDelivery, type Delivery } from './prompt'

/** `attaching` → `live` → `closing` (Close terminal, SIGINT sent) → `exited`; `error` when attaching failed. */
export type PtyStatus = 'attaching' | 'live' | 'closing' | 'exited' | 'error'

export interface PtyState {
  status: PtyStatus
  cols?: number
  rows?: number
  code?: number | null
  error?: string
  /** Bumped when a new PTY starts after the last one ended: the emulator re-attaches. */
  gen: number
}

export interface TerminalState {
  ptys: Record<string, PtyState>
  deliveries: Record<string, Delivery | null>
  /** "Stop turn" sent, waiting for `turn.finished`. */
  stopping: Record<string, boolean>

  setPty(id: string, patch: Partial<PtyState>): void
  started(p: PtyStarted): void
  exited(p: PtyExited): void
  /** Close terminal (03 §3.8): SIGINT, then wait for `pty.exited`. */
  close(id: string): Promise<void>
  /**
   * Sends a prompt (the optimistic bubble, the 5 s confirmation timer). Resolves once `session_prompt` answered.
   * A refusal's notice offers its `action` (05 §14): `onAction` when given, else the Sessions view's own
   * Jump to terminal / Resume (`runRefusalAction`), as the prompt box does.
   */
  send(id: string, text: string, thread: readonly TranscriptItem[], onAction?: (action: CmdErrorAction) => void): Promise<Delivery>
  observe(id: string, thread: readonly TranscriptItem[]): void
  timeout(id: string, deliveryId: number): void
  stopTurn(id: string): Promise<void>
  turnEnded(id: string): void
  reset(): void
}

let deliverySeq = 0

/** The label of a refusal's button (05 §14). */
export const ACTION_LABEL: Record<CmdErrorAction, string> = { jump: 'Jump to terminal', resume: 'Resume' }

/** A refusal's button run the way the Sessions view runs it: Jump to terminal (embedded → the Terminal tab), or the Resume dialog. */
export function runRefusalAction(sessionId: string, action: CmdErrorAction) {
  if (action === 'resume') {
    useDialogs.getState().openResume(sessionId)
    return
  }
  const row = useSessionsStore.getState().rows.find((r) => r.id === sessionId)
  jumpToTerminal(row ?? { id: sessionId, state: 'idle', launch_mode: 'external', cwd: '' })
}

/**
 * One prompt's delivery, shared by the store's `send` and the prompt box (`sendPrompt`): the optimistic
 * bubble, "not confirmed" after 5 s without the hook (the timer lives here so leaving the tab does not stop
 * it), and a refusal as the core sent it (`{code, message, action?}`) — the notice says `message` and, when
 * `action` names one and there is a way to run it, offers it as the notice's button.
 */
export async function deliverPrompt(
  seq: number,
  id: string,
  text: string,
  thread: readonly TranscriptItem[],
  onAction?: (action: CmdErrorAction) => void,
): Promise<{ delivery: Delivery; refusal: PromptRefusal | null }> {
  const store = useTerminalStore
  const d = startDelivery(seq, text, thread, Date.now())
  store.setState((s) => ({ deliveries: { ...s.deliveries, [id]: d } }))
  setTimeout(() => store.getState().timeout(id, d.id), CONFIRM_MS)
  const update = (fn: (cur: Delivery) => Delivery) =>
    store.setState((s) => {
      const cur = s.deliveries[id]
      return cur && cur.id === d.id ? { deliveries: { ...s.deliveries, [id]: fn(cur) } } : {}
    })
  let refusal: PromptRefusal | null = null
  try {
    const r = await invoke<PromptDelivered>('session_prompt', { session_id: id, text })
    update((cur) => onAccepted(cur, r))
  } catch (e) {
    const err = toCmdError(e)
    const action = cmdErrorAction(e)
    refusal = { code: err.code, message: err.message, ...(action ? { action } : {}) }
    update((cur) => onRefused(cur, err.message || null))
    useSessionsStore.getState().showNotice(err.message || 'Not sent', action && onAction ? { label: ACTION_LABEL[action], run: () => onAction(action) } : undefined)
  }
  return { delivery: store.getState().deliveries[id] ?? d, refusal }
}

export const useTerminalStore = create<TerminalState>((set, get) => ({
  ptys: {},
  deliveries: {},
  stopping: {},

  setPty(id, patch) {
    set((s) => {
      const base: PtyState = s.ptys[id] ?? { gen: 0, status: 'attaching' }
      return { ptys: { ...s.ptys, [id]: { ...base, ...patch } } }
    })
  },

  started(p) {
    const cur = get().ptys[p.session_id]
    const over = !cur || cur.status === 'exited' || cur.status === 'error'
    get().setPty(p.session_id, { status: 'live', cols: p.cols, rows: p.rows, code: undefined, error: undefined, gen: (cur?.gen ?? 0) + (over && cur ? 1 : 0) })
  },

  exited(p) {
    get().setPty(p.session_id, { status: 'exited', code: p.code })
  },

  async close(id) {
    const before = get().ptys[id]?.status
    get().setPty(id, { status: 'closing' })
    try {
      await invoke('pty_close', { session_id: id })
    } catch {
      if (get().ptys[id]?.status === 'closing') get().setPty(id, { status: before && before !== 'closing' ? before : 'live' })
      useSessionsStore.getState().showNotice("Couldn't close the terminal")
    }
  },

  async send(id, text, thread, onAction) {
    const { delivery } = await deliverPrompt(++deliverySeq, id, text, thread, onAction ?? ((action) => runRefusalAction(id, action)))
    return delivery
  },

  observe(id, thread) {
    const cur = get().deliveries[id]
    if (!cur) return
    const next = onThread(cur, thread)
    if (next !== cur) set((s) => ({ deliveries: { ...s.deliveries, [id]: next } }))
  },

  timeout(id, deliveryId) {
    const cur = get().deliveries[id]
    if (!cur) return
    const next = onTimeout(cur, deliveryId)
    if (next !== cur) set((s) => ({ deliveries: { ...s.deliveries, [id]: next } }))
  },

  async stopTurn(id) {
    set((s) => ({ stopping: { ...s.stopping, [id]: true } }))
    try {
      await invoke('turn_stop', { session_id: id })
    } catch {
      set((s) => ({ stopping: { ...s.stopping, [id]: false } }))
      useSessionsStore.getState().showNotice("Couldn't stop the turn")
    }
  },

  turnEnded(id) {
    if (get().stopping[id]) set((s) => ({ stopping: { ...s.stopping, [id]: false } }))
  },

  reset() {
    set({ ptys: {}, deliveries: {}, stopping: {} })
  },
}))

/** Reload a detail the app already shows, so `driver` / `alive` follow the PTY host. */
function refresh(id: string) {
  const s = useSessionsStore.getState()
  if (s.details[id]) void s.loadDetail(id)
}

let connected: Promise<void> | null = null

/**
 * Subscribes the store to the PTY host's lifecycle events and to the
 * session details (a prompt is confirmed by its detail's thread) once per
 * app run.
 */
export function connectTerminal(): Promise<void> {
  if (connected) return connected
  useSessionsStore.subscribe((s, prev) => {
    const t = useTerminalStore.getState()
    for (const id of Object.keys(t.deliveries)) {
      const d = s.details[id]
      if (d && d !== prev.details[id]) t.observe(id, d.thread)
    }
  })
  connected = Promise.all([
    listen<PtyStarted>(TERMINAL_EVENTS.started, (p) => {
      if (!p?.session_id) return
      useTerminalStore.getState().started(p)
      refresh(p.session_id)
    }),
    listen<PtyExited>(TERMINAL_EVENTS.exited, (p) => {
      if (!p?.session_id) return
      useTerminalStore.getState().exited(p)
      refresh(p.session_id)
    }),
    listen<TurnStarted>(TERMINAL_EVENTS.turnStarted, (p) => {
      if (p?.session_id) refresh(p.session_id)
    }),
    listen<TurnFinished>(TERMINAL_EVENTS.turnFinished, (p) => {
      if (!p?.session_id) return
      useTerminalStore.getState().turnEnded(p.session_id)
      refresh(p.session_id)
    }),
  ]).then(() => undefined)
  return connected
}
