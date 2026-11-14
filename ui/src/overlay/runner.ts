// Runs the pure machine against real time: one host timer armed for the
// earliest due entry of `timers`, effects handed to the host. Injectable
// clock and timer functions keep it testable with fake timers.
import { useSyncExternalStore } from 'react'
import { initialState, reduce, type Action, type Effect, type MachineState, type TimerKind } from './machine'

export interface RunnerHost {
  now?: () => number
  setTimeout?: (fn: () => void, ms: number) => unknown
  clearTimeout?: (handle: unknown) => void
  onEffect?: (effect: Effect, state: MachineState) => void
}

export interface Runner {
  get(): MachineState
  /** Dispatches without `now`; the runner stamps it. Returns `handled`. */
  dispatch(action: DistributiveOmit<Action, 'now'>): boolean
  subscribe(listener: () => void): () => void
  /** Replaces the effect sink (the React layer binds it after mount). */
  setEffectSink(sink: RunnerHost['onEffect']): void
  dispose(): void
}

type DistributiveOmit<T, K extends keyof T> = T extends unknown ? Omit<T, K> : never

export function createRunner(initial: MachineState = initialState(), host: RunnerHost = {}): Runner {
  const now = host.now ?? (() => Date.now())
  const setT = host.setTimeout ?? ((fn, ms) => setTimeout(fn, ms))
  const clearT = host.clearTimeout ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>))
  let sink = host.onEffect
  let state = initial
  let handle: unknown = null
  const listeners = new Set<() => void>()

  function arm() {
    if (handle !== null) {
      clearT(handle)
      handle = null
    }
    let soonest: [TimerKind, number] | null = null
    for (const [kind, at] of Object.entries(state.timers) as [TimerKind, number][]) {
      if (at !== undefined && (!soonest || at < soonest[1])) soonest = [kind, at]
    }
    if (!soonest) return
    const delay = Math.max(0, soonest[1] - now())
    handle = setT(fire, delay)
  }

  function fire() {
    handle = null
    const t = now()
    const due = (Object.entries(state.timers) as [TimerKind, number][]).filter(([, at]) => at <= t + 1).sort((a, b) => a[1] - b[1])
    for (const [kind] of due) if (state.timers[kind] !== undefined) run({ type: 'timer', kind, now: t })
    arm()
  }

  function run(action: Action): boolean {
    const { next, effects, handled } = reduce(state, action)
    state = next
    for (const e of effects) sink?.(e, state)
    for (const l of listeners) l()
    return handled
  }

  return {
    get: () => state,
    dispatch(action) {
      const handled = run({ ...action, now: now() } as Action)
      arm()
      return handled
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
    setEffectSink(next) {
      sink = next
    },
    dispose() {
      if (handle !== null) clearT(handle)
      handle = null
      listeners.clear()
    },
  }
}

export function useMachine(runner: Runner): MachineState {
  return useSyncExternalStore(runner.subscribe, runner.get, runner.get)
}
