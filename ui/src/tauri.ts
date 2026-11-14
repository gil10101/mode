// The only place the UI touches `@tauri-apps/api`. Guarded so every entry also
// runs in a plain browser (vite dev / preview, vitest, Playwright) with a mock
// that answers the commands and lets tests push core events.

import type { CoreStatus } from './state/store'
import type { DecideResult, RosterRow } from './state/contract'

type Args = Record<string, unknown> | undefined

export function isTauri(): boolean {
  return typeof window !== 'undefined' && 'isTauri' in window && Boolean((window as { isTauri?: unknown }).isTauri)
}

const mockStatus: CoreStatus = {
  version: '0.1.0',
  dev_build: true,
  data_dir: '(browser mock)',
  db_path: '(browser mock)',
  endpoint: '(browser mock)',
  ipc_listening: false,
  hook_frames_received: 0,
}

export interface MockInvoke {
  cmd: string
  args: Args
}

type Handler = (payload: unknown) => void

/**
 * The browser stand-in for the core (`window.__mode_mock` outside Tauri):
 * `emit` pushes a core event to every `listen`er, `invokes` records every
 * command, `respond` overrides a command's answer, `reset` clears it all.
 */
export interface ModeMock {
  invokes: MockInvoke[]
  /** The roster `sessions_list` returns until a `sessions.changed` push replaces it. */
  rows: RosterRow[]
  emit(event: string, payload: unknown): number
  respond(cmd: string, handler: ((args: Args) => unknown) | null): void
  /** The next call of `cmd` rejects with `error` (once), whoever answers it; for the non-happy paths in tests. */
  failNext(cmd: string, error: unknown): void
  /** Every call of `cmd` waits, unanswered, until `release(cmd)`; for the loading states (a view's skeleton) in tests and shots. */
  hold(cmd: string): void
  /** Answers the held calls of `cmd` and stops holding it. */
  release(cmd: string): void
  reset(): void
  /** Event names something is listening to (for tests). */
  listening(): string[]
  /** @internal */
  subscribe(event: string, handler: Handler): () => void
  /** @internal */
  answer(cmd: string, args: Args): Promise<unknown>
}

function createMock(): ModeMock {
  const listeners = new Map<string, Set<Handler>>()
  const responders = new Map<string, (args: Args) => unknown>()
  const failures = new Map<string, unknown>()
  const held = new Map<string, (() => void)[]>()
  const releaseAll = (cmd: string) => {
    const waiting = held.get(cmd) ?? []
    held.delete(cmd)
    for (const go of waiting) go()
  }
  const mock: ModeMock = {
    invokes: [],
    rows: [],
    emit(event, payload) {
      const set = listeners.get(event)
      if (!set) return 0
      for (const h of Array.from(set)) h(payload)
      return set.size
    },
    respond(cmd, handler) {
      if (handler) responders.set(cmd, handler)
      else responders.delete(cmd)
    },
    failNext(cmd, error) {
      failures.set(cmd, error)
    },
    hold(cmd) {
      if (!held.has(cmd)) held.set(cmd, [])
    },
    release(cmd) {
      releaseAll(cmd)
    },
    reset() {
      mock.invokes = []
      mock.rows = []
      responders.clear()
      failures.clear()
      for (const cmd of Array.from(held.keys())) releaseAll(cmd)
    },
    listening() {
      return Array.from(listeners.entries())
        .filter(([, s]) => s.size > 0)
        .map(([e]) => e)
        .sort()
    },
    subscribe(event, handler) {
      let set = listeners.get(event)
      if (!set) listeners.set(event, (set = new Set()))
      set.add(handler)
      return () => {
        set!.delete(handler)
      }
    },
    async answer(cmd, args) {
      mock.invokes.push({ cmd, args })
      const waiting = held.get(cmd)
      if (waiting) await new Promise<void>((go) => waiting.push(go))
      if (failures.has(cmd)) {
        const error = failures.get(cmd)
        failures.delete(cmd)
        throw error
      }
      const custom = responders.get(cmd)
      if (custom) return custom(args)
      switch (cmd) {
        case 'ping':
          return 'pong'
        case 'core_status':
          return mockStatus
        case 'sessions_list':
          return mock.rows
        case 'session_decide':
          return { outcome: 'accepted' } satisfies DecideResult
        case 'note_capture':
          return { note_id: `mock-note-${mock.invokes.length}` }
        case 'session_acknowledge':
        case 'session_dismiss':
        case 'session_jump':
        case 'overlay_request':
        case 'overlay_measure':
        case 'app_open':
          return undefined
        default:
          throw new Error(`mock: unknown command ${cmd}`)
      }
    },
  }
  return mock
}

declare global {
  interface Window {
    __mode_mock?: ModeMock
  }
}

let fallback: ModeMock | undefined

/** The mock instance (browser only); shared through `window.__mode_mock` so Playwright can drive it. */
export function mock(): ModeMock {
  if (typeof window === 'undefined') return (fallback ??= createMock())
  return (window.__mode_mock ??= createMock())
}

export async function mockInvoke<T>(cmd: string, args?: Args): Promise<T> {
  return mock().answer(cmd, args) as Promise<T>
}

export async function invoke<T>(cmd: string, args?: Args): Promise<T> {
  if (isTauri()) {
    const { invoke } = await import('@tauri-apps/api/core')
    return invoke<T>(cmd, args)
  }
  return mockInvoke<T>(cmd, args)
}

export type Unlisten = () => void

/** Listen to a core push; outside Tauri the mock's `emit` feeds it. */
export async function listen<T>(event: string, handler: (payload: T) => void): Promise<Unlisten> {
  if (!isTauri()) return mock().subscribe(event, handler as Handler)
  const { listen } = await import('@tauri-apps/api/event')
  // Tauri 2 event names allow only [A-Za-z0-9-/:_]; the contract's `area.changed`
  // names travel as `area:changed` (mode_core::push::Event::wire_name).
  return listen<T>(event.replace(/\./g, ':'), (e) => handler(e.payload))
}
