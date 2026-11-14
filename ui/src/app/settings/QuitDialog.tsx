// Quit with live embedded sessions (03 §2.1, 08 §2.6; `mode_app::quit`):
// ⌘Q / Ctrl+Q, the OS or the profile menu's Quit (`app_quit` without
// arguments) make the core hold the exit and send the app window
// `app:quit-confirm {sessions}`; this dialog lists those sessions and
// answers `app_quit {confirm: true}` (interrupt them, wait up to 10 s, exit)
// or `app_quit {confirm: false}` (stay). Mounted once by the shell.
import { useEffect } from 'react'
import { create } from 'zustand'
import { Button } from '../../kit/button'
import { invoke, listen } from '../../tauri'
import { cmdErrorText } from '../../state/contract'
import { QUIT_CONFIRM_EVENT, type QuitAnswer, type QuitConfirm } from '../../state/contract-settings'
import type { LiveSession } from '../../state/contract-terminal'
import { useSessionsStore } from '../../state/sessions-store'
import { Dialog } from '../ui'

export interface QuitState {
  /** The sessions the core asked about; null while no question is open. */
  sessions: LiveSession[] | null
  /** `app_quit {confirm: true}` sent: the sessions are being stopped. */
  quitting: boolean
  error: string | null
  ask(sessions: LiveSession[]): void
  /** Quit from the app (the profile menu): the core exits at once or asks. */
  request(): Promise<QuitAnswer | null>
  answer(confirm: boolean): Promise<void>
}

export const useQuit = create<QuitState>((set, get) => ({
  sessions: null,
  quitting: false,
  error: null,

  ask(sessions) {
    set({ sessions: sessions ?? [], quitting: false, error: null })
  },

  async request() {
    try {
      const r = await invoke<QuitAnswer>('app_quit')
      // The core also sends `app:quit-confirm`; whichever arrives first opens the dialog.
      if (r?.status === 'confirm') get().ask(r.sessions ?? [])
      return r ?? null
    } catch (e) {
      useSessionsStore.getState().showNotice(`Couldn't quit: ${cmdErrorText(e)}`)
      return null
    }
  },

  async answer(confirm) {
    if (confirm) set({ quitting: true, error: null })
    try {
      await invoke<QuitAnswer>('app_quit', { confirm })
      if (!confirm) set({ sessions: null, quitting: false })
    } catch (e) {
      set({ quitting: false, error: cmdErrorText(e) })
    }
  },
}))

/** Listens for the core's question. Returns the unsubscribe function. */
export function connectQuit(): Promise<() => void> {
  return listen<QuitConfirm>(QUIT_CONFIRM_EVENT, (p) => useQuit.getState().ask(p?.sessions ?? []))
}

/** "api-refactor · terminal · working". */
export function liveLine(s: LiveSession): string {
  return [s.display_name, s.driver === 'headless' ? 'turn running' : 'terminal', s.state].filter(Boolean).join(' · ')
}

export function QuitDialog() {
  const sessions = useQuit((s) => s.sessions)
  const quitting = useQuit((s) => s.quitting)
  const error = useQuit((s) => s.error)
  useEffect(() => {
    const off = connectQuit()
    return () => void off.then((u) => u())
  }, [])
  if (!sessions) return null
  const n = sessions.length
  const stay = () => void useQuit.getState().answer(false)
  return (
    <Dialog title="Quit Mode?" onClose={stay} testId="quit-dialog">
      <div className="ap-dialog-note">
        {`${n === 1 ? 'This session runs in Mode' : `These ${n} sessions run in Mode`}; quitting interrupts ${n === 1 ? 'it' : 'them'} and waits up to 10 seconds for the turn to end.`}
      </div>
      <ul className="m-list" aria-label="Running sessions" data-testid="quit-sessions">
        {sessions.map((s) => (
          <li key={s.session_id} className="m-t-dense" data-session={s.session_id}>
            {liveLine(s)}
          </li>
        ))}
      </ul>
      {quitting && (
        <div className="ap-dialog-note" role="status">
          Stopping the sessions…
        </div>
      )}
      {error && <div className="ap-dialog-note is-bad">{error}</div>}
      <div className="m-card-actions">
        <Button disabled={quitting} onClick={stay}>
          Cancel
        </Button>
        <Button variant="primary" disabled={quitting} onClick={() => void useQuit.getState().answer(true)}>
          Quit
        </Button>
      </div>
    </Dialog>
  )
}
