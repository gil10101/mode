// What the Terminal tab shows for an embedded session (03 §3.6, §3.8, 05
// §14): the live emulator with Find and Close terminal; while a headless
// turn runs, "A turn is running · Stop turn"; once it has ended (or the
// session never had a PTY), "Open a terminal". External, untracked and
// ended sessions keep the hints drawn by tabs.tsx.
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Button } from '../kit/button'
import type { SessionDetail } from '../state/contract-sessions'
import { useSessionsStore } from '../state/sessions-store'
import { isResumable } from '../app/sessions/header'
import { navigate } from '../app/router'
import { Terminal, type TerminalHandle } from './Terminal'
import { connectTerminal, useTerminalStore } from './store'

/** `pty` live emulator · `headless` a driven turn · `open` no PTY yet · `resume` ended · `jump` not Mode's terminal. */
export type TerminalKind = 'pty' | 'headless' | 'open' | 'resume' | 'jump'

export function terminalKind(d: Pick<SessionDetail, 'session' | 'driver' | 'alive'>): TerminalKind {
  const h = d.session
  if (isResumable(h) || !d.alive) return 'resume'
  const embedded = h.launch_mode === 'embedded' || h.launch_mode === 'headless'
  if (!embedded || h.state === 'untracked') return 'jump'
  if (d.driver === 'headless' || h.launch_mode === 'headless') return 'headless'
  if (d.driver === 'pty') return 'pty'
  return 'open'
}

/**
 * The kind, holding `pty` while the emulator this tab showed is closing or
 * has just exited, so the last screen stays with "Terminal closed" until the
 * tab is left (the next visit shows "Resume to open a terminal").
 */
export function useTerminalKind(d: Pick<SessionDetail, 'session' | 'driver' | 'alive'>): TerminalKind {
  const id = d.session.id
  const kind = terminalKind(d)
  const status = useTerminalStore((s) => s.ptys[id]?.status)
  const [shown, setShown] = useState<string | null>(null)
  useEffect(() => {
    if (kind === 'pty') setShown(id)
  }, [kind, id])
  if (kind !== 'pty' && shown === id && (status === 'closing' || status === 'exited')) return 'pty'
  return kind
}

/** `⌘/Ctrl+Shift+Esc`: focus goes to the active session tab. */
function leaveTerminal() {
  const tab = document.querySelector<HTMLElement>('.sd-tabs [role="tab"][aria-selected="true"]')
  if (tab) tab.focus()
  else (document.activeElement as HTMLElement | null)?.blur?.()
}

function TabHint({ children, testId }: { children: ReactNode; testId?: string }) {
  return (
    <div className="sd-hint is-tab" data-testid={testId}>
      {children}
    </div>
  )
}

function link(label: string, run: () => void, disabled = false) {
  return (
    <a
      href="#"
      aria-disabled={disabled || undefined}
      onClick={(e) => {
        e.preventDefault()
        if (!disabled) run()
      }}
    >
      {label}
    </a>
  )
}

export function TerminalPanel({ sessionId, kind, onResume }: { sessionId: string; kind: Exclude<TerminalKind, 'resume' | 'jump'>; onResume: () => void }) {
  const pty = useTerminalStore((s) => s.ptys[sessionId])
  const stopping = useTerminalStore((s) => !!s.stopping[sessionId])
  const handle = useRef<TerminalHandle>(null)

  useEffect(() => {
    void connectTerminal()
  }, [])

  if (kind === 'headless')
    return (
      <TabHint testId="terminal-headless">
        {stopping ? 'A turn is running' : 'A turn is running · '}
        {!stopping && link('Stop turn', () => void useTerminalStore.getState().stopTurn(sessionId))}
      </TabHint>
    )

  if (kind === 'open')
    return <TabHint testId="terminal-open">{link('Open a terminal', () => void useSessionsStore.getState().resume({ session_id: sessionId, open: 'terminal' }))}</TabHint>

  const status = pty?.status ?? 'attaching'
  let line: ReactNode = null
  if (status === 'closing') line = 'Waiting for the turn to end…'
  else if (status === 'exited')
    line = (
      <>
        {`${pty?.code ? `Terminal closed · exit code ${pty.code}` : 'Terminal closed'} · `}
        {link('Resume', onResume)}
      </>
    )
  else if (status === 'error')
    line = (
      <>
        {`${pty?.error ?? "Couldn't attach to the terminal"} · `}
        {link('Open Diagnostics', () => navigate('settings/diagnostics'))}
      </>
    )

  return (
    <>
      <div className="tm-bar">
        <span className="tm-status" role="status" data-testid="terminal-status" data-status={status}>
          {line}
        </span>
        <Button quiet onClick={() => handle.current?.openFind()}>
          Find
        </Button>
        {status !== 'exited' && status !== 'error' && (
          <Button disabled={status === 'closing'} onClick={() => void useTerminalStore.getState().close(sessionId)}>
            Close terminal
          </Button>
        )}
      </div>
      <Terminal key={`${sessionId}:${pty?.gen ?? 0}`} sessionId={sessionId} readOnly={status === 'exited'} onLeave={leaveTerminal} handle={handle} />
    </>
  )
}
