// When the first-run flow shows (03 §2.8): `health.first_run` from the core (true while
// `onboarding.completed_at` is unset), or "Run again" on Settings → General. Once
// shown it stays until finished or skipped in this window, whatever later
// `health.changed` pushes say.
import { useEffect } from 'react'
import { create } from 'zustand'
import { useSettingsStore } from '../../state/settings-store'

interface OnboardingState {
  /** Showing in this window. */
  active: boolean
  /** Opened from Settings (re-run): starts at the first step. */
  forced: boolean
  /** Finished or skipped in this window. */
  closed: boolean
  open(forced: boolean): void
  close(): void
}

export const useOnboarding = create<OnboardingState>((set) => ({
  active: false,
  forced: false,
  closed: false,
  open: (forced) => set({ active: true, forced, closed: false }),
  close: () => set({ active: false, forced: false, closed: true }),
}))

export function openOnboarding(forced = true) {
  useOnboarding.getState().open(forced)
}

/** The shell renders the stepper instead of the window while this is true. */
export function useOnboardingVisible(): boolean {
  const firstRun = useSettingsStore((s) => s.health?.first_run === true)
  const active = useOnboarding((s) => s.active)
  const closed = useOnboarding((s) => s.closed)
  useEffect(() => {
    if (firstRun && !active && !closed) useOnboarding.getState().open(false)
  }, [firstRun, active, closed])
  return active || (firstRun && !closed)
}
