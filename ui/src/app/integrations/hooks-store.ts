// The Claude Code hook install flow (05 §3.6, 03 §7C.3) shared by the
// Integrations card, its detail page, Diagnostics and onboarding step 3:
// `hooks_status` for the card, `hooks_install_preview` → the diff preview,
// `hooks_install_apply` (or `hooks_uninstall`) → `hooks_round_trip` to verify,
// then a fresh `hooks_status`. A failed write or round trip keeps the preview
// open with the error (the core restores a partial write).
import { errorText, useSettingsStore } from '../../state/settings-store'
import { useSessionsStore } from '../../state/sessions-store'
import type { ClaudeHealth } from './model'
import { create } from 'zustand'
import { invoke } from '../../tauri'
import type { Diagnostics, HookAction, HooksApplied, HooksPlan, HooksStatus, RoundTripReport } from '../../state/contract-settings'

export type FlowPhase = 'loading' | 'preview' | 'applying' | 'verifying' | 'done' | 'failed'

export interface HookFlow {
  action: HookAction
  phase: FlowPhase
  plan: HooksPlan | null
  applied: HooksApplied | null
  roundTrip: RoundTripReport | null
  error: string | null
}

interface HooksState {
  status: HooksStatus | null
  statusError: string | null
  diag: Diagnostics | null
  /** The open preview (dialog or onboarding step). */
  flow: HookFlow | null
  refresh(): Promise<void>
  loadDiagnostics(): Promise<void>
  preview(action: HookAction): Promise<void>
  apply(): Promise<boolean>
  roundTrip(): Promise<RoundTripReport | null>
  close(): void
}


export const useHooks = create<HooksState>((set, get) => ({
  status: null,
  statusError: null,
  diag: null,
  flow: null,

  async refresh() {
    try {
      const status = await invoke<HooksStatus>('hooks_status')
      set({ status, statusError: null })
    } catch (e) {
      set({ statusError: errorText(e) })
    }
  },

  async loadDiagnostics() {
    try {
      set({ diag: await invoke<Diagnostics>('diagnostics_get') })
    } catch {
      /* the card works without it */
    }
  },

  async preview(action) {
    set({ flow: { action, phase: 'loading', plan: null, applied: null, roundTrip: null, error: null } })
    try {
      const plan = await invoke<HooksPlan>('hooks_install_preview', action === 'uninstall' ? { action } : undefined)
      if (get().flow?.action !== action) return
      // A core without the uninstall preview answers the install plan: confirm without a diff then.
      const usable = action !== 'uninstall' || plan?.action === 'uninstall'
      set({ flow: { action, phase: 'preview', plan: usable ? plan : null, applied: null, roundTrip: null, error: null } })
    } catch (e) {
      set((s) => ({ flow: s.flow ? { ...s.flow, phase: 'failed', error: errorText(e) } : null }))
    }
  },

  async apply() {
    const flow = get().flow
    if (!flow || (flow.phase !== 'preview' && flow.phase !== 'failed') || (!flow.plan && flow.action !== 'uninstall')) return false
    set({ flow: { ...flow, phase: 'applying', error: null } })
    let applied: HooksApplied
    try {
      applied = await invoke<HooksApplied>(flow.action === 'uninstall' ? 'hooks_uninstall' : 'hooks_install_apply')
    } catch (e) {
      set((s) => ({ flow: s.flow ? { ...s.flow, phase: 'failed', error: errorText(e) } : null }))
      await get().refresh()
      return false
    }
    if (flow.action === 'uninstall') {
      set((s) => ({ flow: s.flow ? { ...s.flow, phase: 'done', applied } : null }))
      await get().refresh()
      return true
    }
    set((s) => ({ flow: s.flow ? { ...s.flow, phase: 'verifying', applied } : null }))
    try {
      const rt = await invoke<RoundTripReport>('hooks_round_trip')
      if (!rt.acked) throw new Error('the relay sent the frame but the core did not answer')
      set((s) => ({ flow: s.flow ? { ...s.flow, phase: 'done', roundTrip: rt } : null }))
      await get().refresh()
      return true
    } catch (e) {
      set((s) => ({ flow: s.flow ? { ...s.flow, phase: 'failed', error: `Written, but the round trip failed: ${errorText(e)}` } : null }))
      await get().refresh()
      return false
    }
  },

  async roundTrip() {
    try {
      const rt = await invoke<RoundTripReport>('hooks_round_trip')
      await get().refresh()
      return rt
    } catch (e) {
      set({ statusError: errorText(e) })
      return null
    }
  },

  close() {
    set({ flow: null })
  },
}))

/** "Installed · round trip 9 ms" and the other result lines of the preview. */
export function flowResult(f: HookFlow): string | null {
  if (f.phase === 'applying') return f.action === 'uninstall' ? 'Removing…' : 'Writing…'
  if (f.phase === 'verifying') return 'Verifying the round trip…'
  if (f.phase === 'done') {
    if (f.action === 'uninstall') return `Hooks removed${f.applied?.backup_path ? ` · backup at ${f.applied.backup_path}` : ''}`
    return `Installed · round trip ${f.roundTrip?.latency_ms ?? '?'} ms`
  }
  return null
}

/** Claude Code on this machine as `health.changed` reports it (`claude_code`, `claude_code_version`). */
export function useClaudeHealth(): ClaudeHealth | null {
  const a = useSettingsStore((s) => s.health)
  const b = useSessionsStore((s) => s.health) as ClaudeHealth | null
  const code = a?.claude_code ?? b?.claude_code
  const version = a?.claude_code_version ?? b?.claude_code_version ?? null
  if (!code && !version) return null
  return { claude_code: code, claude_code_version: version }
}
