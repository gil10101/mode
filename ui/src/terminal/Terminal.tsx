// The Terminal tab's emulator (03 §3.8): `@xterm/xterm` bound to one
// session's PTY, with the find strip. The pane keeps the wireframe's
// `.sd-term` box; xterm draws inside it with the theme's terminal palette.
import { useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react'
import { Button } from '../kit/button'
import { invoke } from '../tauri'
import { openPath } from '../app/sessions/actions'
import { useSessionsStore } from '../state/sessions-store'
import { TerminalController, type Renderer } from './controller'
import { detectPlatform } from './keys'
import { connectTerminal, useTerminalStore } from './store'
import './terminal.css'

export interface TerminalHandle {
  openFind(): void
  focus(): void
}

export interface TerminalProps {
  sessionId: string
  /** The PTY has exited: the last screen stays, input is refused. */
  readOnly?: boolean
  /** `⌘/Ctrl+Shift+Esc`: focus leaves the terminal. */
  onLeave?: () => void
  /** Focus the emulator once it is attached. */
  autoFocus?: boolean
  handle?: Ref<TerminalHandle>
}

/** OSC 8 and plain URLs (http, https, mailto) through tauri-plugin-opener. */
export function openUrl(url: string) {
  void invoke('plugin:opener|open_url', { url }).catch(() => useSessionsStore.getState().showNotice("Couldn't open the link"))
}

interface FindState {
  open: boolean
  index: number
  count: number
  missed: boolean
}

const CLOSED: FindState = { open: false, index: -1, count: 0, missed: false }

export function Terminal({ sessionId, readOnly = false, onLeave, autoFocus = true, handle }: TerminalProps) {
  const pane = useRef<HTMLDivElement>(null)
  const host = useRef<HTMLDivElement>(null)
  const ctl = useRef<TerminalController | null>(null)
  const [renderer, setRenderer] = useState<Renderer | null>(null)
  const [find, setFind] = useState<FindState>(CLOSED)
  const leave = useRef(onLeave)
  const ro = useRef(readOnly)
  const focusOnAttach = useRef(autoFocus)

  useEffect(() => {
    leave.current = onLeave
  }, [onLeave])

  useEffect(() => {
    ro.current = readOnly
    ctl.current?.setReadOnly(readOnly)
  }, [readOnly])

  useImperativeHandle(
    handle,
    () => ({
      openFind: () => setFind((f) => ({ ...f, open: true })),
      focus: () => ctl.current?.focus(),
    }),
    [],
  )

  useEffect(() => {
    let live = true
    const store = useTerminalStore.getState()
    const status = store.ptys[sessionId]?.status
    if (status !== 'closing' && status !== 'exited') store.setPty(sessionId, { status: 'attaching', error: undefined })
    void connectTerminal()
    // A failure never hides a closing or exited PTY: that state says more.
    const failed = (error: string) => {
      const s = useTerminalStore.getState()
      const now = s.ptys[sessionId]?.status
      if (now === 'attaching' || now === 'live') s.setPty(sessionId, { status: 'error', error })
    }
    import('./xterm')
      .then(({ xterm }) => {
        if (!live || !pane.current || !host.current) return
        const c = new TerminalController(xterm, sessionId, pane.current, host.current, detectPlatform(), {
          onFind: () => setFind((f) => ({ ...f, open: true })),
          onLeave: () => leave.current?.(),
          onRenderer: (r) => live && setRenderer(r),
          onAttached: (r) => {
            const s = useTerminalStore.getState()
            if (r?.exited) s.setPty(sessionId, { status: 'exited', code: r.exit_code ?? null })
            else if (s.ptys[sessionId]?.status === 'attaching') s.setPty(sessionId, { status: 'live', cols: r?.cols, rows: r?.rows })
            if (focusOnAttach.current && !ro.current) c.focus()
          },
          onAttachFailed: (message) => failed(message),
          onSearchResults: (index, count) => live && setFind((f) => ({ ...f, index, count })),
          openUrl,
          openPath,
        })
        ctl.current = c
        c.setReadOnly(ro.current)
        return c.start()
      })
      .catch(() => {
        if (live) failed("Couldn't start the terminal")
      })
    return () => {
      live = false
      ctl.current?.dispose()
      ctl.current = null
    }
  }, [sessionId])

  const closeFind = () => {
    ctl.current?.clearFind()
    setFind(CLOSED)
    ctl.current?.focus()
  }

  return (
    <div ref={pane} className="sd-term tm-pane" role="region" aria-label="Terminal" data-testid="terminal-pane" data-renderer={renderer ?? undefined}>
      {find.open && (
        <FindStrip
          state={find}
          onQuery={(q, direction, incremental) => {
            const hit = ctl.current?.find(q, direction, incremental) ?? false
            setFind((f) => ({ ...f, missed: !!q && !hit, ...(q ? {} : { index: -1, count: 0 }) }))
          }}
          onClose={closeFind}
        />
      )}
      <div ref={host} className="tm-host" />
    </div>
  )
}

function FindStrip({ state, onQuery, onClose }: { state: FindState; onQuery: (q: string, direction: 'next' | 'previous', incremental: boolean) => void; onClose: () => void }) {
  const [q, setQ] = useState('')
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => {
    input.current?.focus()
    input.current?.select()
  }, [])
  const result = !q ? '' : state.count > 0 && state.index >= 0 ? `${state.index + 1} of ${state.count}` : state.missed ? 'No results' : ''
  return (
    <div className="tm-find" role="search" aria-label="Find in terminal" data-testid="terminal-find">
      <input
        ref={input}
        className="m-input"
        type="text"
        placeholder="Find in terminal"
        aria-label="Find in terminal"
        value={q}
        onChange={(e) => {
          setQ(e.target.value)
          onQuery(e.target.value, 'next', true)
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            onQuery(q, e.shiftKey ? 'previous' : 'next', false)
          } else if (e.key === 'Escape') {
            e.preventDefault()
            e.stopPropagation()
            onClose()
          }
        }}
      />
      <span className="tm-find-count" aria-live="polite">
        {result}
      </span>
      <Button quiet disabled={!q} onClick={() => onQuery(q, 'previous', false)}>
        Previous
      </Button>
      <Button quiet disabled={!q} onClick={() => onQuery(q, 'next', false)}>
        Next
      </Button>
      <Button quiet onClick={onClose}>
        Close
      </Button>
    </div>
  )
}
