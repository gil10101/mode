// `ui/state` (08 §3.3): the store fed by pushed events. Package 0 holds the
// core status and the last `core.changed` push; views are thin over it.
import { create } from 'zustand'

/** `mode_core::diag::Snapshot`, as `core_status` returns it. */
export interface CoreStatus {
  version: string
  dev_build: boolean
  data_dir: string
  db_path: string
  endpoint: string
  ipc_listening: boolean
  hook_frames_received: number
}

/** Payload of the `core.changed` event (`mode_app::events::CoreChanged`). */
export interface CoreChanged {
  reason: 'ipc_listening' | 'ipc_error' | 'hook' | string
  event?: string
  session_id?: string
  ipc_listening: boolean
  hook_frames_received: number
}

export type View =
  | 'sessions'
  | 'inbox'
  | 'notes'
  | 'calendar'
  | 'chat'
  | 'agents'
  | 'automations'
  | 'integrations'
  | 'shortcuts'
  | 'stats'
  | 'settings'

export interface CoreState {
  pong: string | null
  status: CoreStatus | null
  lastChange: CoreChanged | null
  view: View
  setPong: (pong: string) => void
  setStatus: (status: CoreStatus) => void
  applyChange: (change: CoreChanged) => void
  setView: (view: View) => void
}

export const useCoreStore = create<CoreState>((set) => ({
  pong: null,
  status: null,
  lastChange: null,
  view: 'sessions',
  setPong: (pong) => set({ pong }),
  setStatus: (status) => set({ status }),
  applyChange: (change) =>
    set((s) => ({
      lastChange: change,
      status: s.status
        ? { ...s.status, ipc_listening: change.ipc_listening, hook_frames_received: change.hook_frames_received }
        : s.status,
    })),
  setView: (view) => set({ view }),
}))
