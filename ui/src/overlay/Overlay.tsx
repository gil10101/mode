// The overlay renderer (08 §3.3 `ui/overlay`): a thin view over the pushed
// state. The core shows and hides the window, a fixed stage (02 §12); this
// reports what it wants (`overlay_request`) and where the drawn frame is
// (`overlay_measure`), morphs the notch frame inside the stage, draws the
// state the machine is in for the placement it was told, and never takes
// focus except in the capture box (02 §9).
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { IconDefs } from '../kit/icons'
import { BrandDefs } from '../kit/brands'
import { Menu, MenuItem } from '../kit/controls'
import { ThemeProvider } from '../kit/theme'
import {
  EVENTS,
  type AgendaChanged,
  type AppContextKind,
  type DictationState,
  type DisplayChanged,
  type HealthChanged,
  type HotkeyFired,
  type InboxChanged,
  type MediaChanged,
  type MediaHistory,
  type NotesChanged,
  type OverlayFlash,
  type OverlayPointer,
  type OverlaySettings,
  type PillChanged,
  type QuietChanged,
  type RosterRow,
  type SessionsChanged,
} from '../state/contract'
import { useOverlayStore } from '../state/overlay-store'
import type { CoreChanged } from '../state/store'
import { invoke, isTauri, listen } from '../tauri'
import { createActor } from './actions'
import { PeekView, PillView } from './Compact'
import { unionAll, type Rect } from './morph'
import { CapsuleFrame } from './CapsuleFrame'
import { NotchFrame } from './NotchFrame'
import { OverlayContext, type MenuSpec, type OverlayCtx } from './context'
import { Expanded } from './Expanded'
import { useFocusGuard } from './focus'
import { toKeyAction } from './keys'
import { initialState, type Effect, type MachineState } from './machine'
import { lookAt, type BotCanvas } from './NotchBot'
import { buildPill, flashPill, localLadder } from './pill'
import { hasSeconds, mascotBadge } from './phrases'
import { createRunner, useMachine, type Runner } from './runner'

export interface OverlayProps {
  /** A prepared runner (tests); otherwise one is created from the query string and the settings. */
  runner?: Runner
}

const CONTEXT_KIND: Record<string, AppContextKind> = { sessions: 'session', inbox: 'message', notes: 'note', calendar: 'event' }

function prefersReducedMotion(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches
}

function prefersDark(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-color-scheme: dark)').matches
}

/** Browser-only: `?placement=notch&dark=1&reduced_motion=1&notch=184x36&scale=1&pinned=1` seeds the display (dev and Playwright). */
export function displayFromQuery(search: string): Partial<DisplayChanged> & { pinned?: boolean } {
  const q = new URLSearchParams(search)
  const out: Partial<DisplayChanged> & { pinned?: boolean } = {}
  const placement = q.get('placement')
  if (placement === 'notch' || placement === 'top-bar' || placement === 'right-edge') out.placement = placement
  if (q.has('dark')) out.dark = q.get('dark') !== '0'
  if (q.has('reduced_motion')) out.reduced_motion = q.get('reduced_motion') !== '0'
  const notch = /^(\d+)x(\d+)$/.exec(q.get('notch') ?? '')
  if (notch) out.notch = { width: Number(notch[1]), height: Number(notch[2]) }
  if (q.get('scale')) out.scale = Number(q.get('scale'))
  if (q.has('pinned')) out.pinned = q.get('pinned') !== '0'
  return out
}

/** The clock elapsed labels tick on; stopped while the overlay or its document is hidden (02 §15). */
function useTick(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now())
  const [visible, setVisible] = useState(() => typeof document === 'undefined' || document.visibilityState !== 'hidden')
  useEffect(() => {
    const on = () => setVisible(document.visibilityState !== 'hidden')
    document.addEventListener('visibilitychange', on)
    return () => document.removeEventListener('visibilitychange', on)
  }, [])
  useEffect(() => {
    setNow(Date.now())
    if (!intervalMs || !visible) return
    const t = setInterval(() => setNow(Date.now()), intervalMs)
    return () => clearInterval(t)
  }, [intervalMs, visible])
  return now
}

export function Overlay({ runner: given }: OverlayProps) {
  const store = useOverlayStore
  const runnerRef = useRef<Runner | null>(given ?? null)
  if (!runnerRef.current) {
    const seed = isTauri() ? {} : displayFromQuery(location.search)
    if (Object.keys(seed).length) store.getState().applyDisplay(seed)
    const s = store.getState()
    runnerRef.current = createRunner(initialState({ settings: { ...s.settings, reduced_motion: s.display.reduced_motion || prefersReducedMotion() }, pinned: seed.pinned }))
    // Browser only: lets Playwright and the dev console read the machine (`window.__mode_overlay.runner.get()`).
    if (!isTauri()) (window as unknown as { __mode_overlay?: unknown }).__mode_overlay = { runner: runnerRef.current }
  }
  const runner = runnerRef.current
  const m = useMachine(runner)

  const rows = store((s) => s.rows)
  const rowsAt = store((s) => s.rowsAt)
  const pushedPill = store((s) => s.pill)
  const inbox = store((s) => s.inbox)
  const agenda = store((s) => s.agenda)
  const quiet = store((s) => s.quiet)
  const display = store((s) => s.display)
  const settings = store((s) => s.settings)
  const doneHeldUntil = store((s) => s.doneHeldUntil)
  const decided = store((s) => s.decided)

  // 02 §15: once a second while expanded, or while the pill's elapsed label
  // carries seconds (under an hour); once a minute otherwise; never hidden.
  const [pillSeconds, setPillSeconds] = useState(false)
  const now = useTick(m.state === 'expanded' ? 1000 : m.state === 'compact' || m.state === 'peek' ? (pillSeconds ? 1000 : 60_000) : 0)
  const rootRef = useRef<HTMLDivElement | null>(null)
  const botRef = useRef<BotCanvas | null>(null)
  const captureRef = useRef<HTMLInputElement | null>(null)
  const [captureText, setCaptureText] = useState('')
  const [link, setLink] = useState<string | null>(null)
  const [menu, setMenu] = useState<{ items: MenuSpec[]; onSelect: (id: string) => void; right: number; top: number } | null>(null)
  const [pulse, setPulse] = useState(false)
  const dark = display.dark

  /* ---- core events ---------------------------------------------------------- */
  useEffect(() => {
    const st = store.getState()
    const subs = [
      listen<SessionsChanged>(EVENTS.sessions, (p) => store.getState().applySessions(p.rows)),
      listen<PillChanged>(EVENTS.pill, (p) => store.getState().applyPill(p)),
      listen<InboxChanged>(EVENTS.inbox, (p) => store.getState().applyInbox(p)),
      listen<NotesChanged>(EVENTS.notes, (p) => store.getState().applyNotes(p)),
      listen<AgendaChanged>(EVENTS.agenda, (p) => store.getState().applyAgenda(p)),
      listen<QuietChanged>(EVENTS.quiet, (p) => store.getState().applyQuiet(p)),
      listen<DisplayChanged>(EVENTS.display, (p) => store.getState().applyDisplay(p)),
      listen<HealthChanged>(EVENTS.health, (p) => store.getState().applyHealth(p)),
      listen<CoreChanged>(EVENTS.core, (p) => store.getState().applyCore(p)),
      listen<Partial<OverlaySettings>>(EVENTS.settings, (p) => store.getState().applySettings(p)),
      listen<HotkeyFired>(EVENTS.hotkey, (p) => runner.dispatch({ type: 'hotkey', hotkey: p.hotkey, captureEmpty: !(captureRef.current?.value ?? '') })),
      // `move` only turns the resting mascot's eyes (02 §6.1); the other kinds drive the machine.
      listen<OverlayPointer>(EVENTS.pointer, (p) => (p.kind === 'move' ? lookAt(botRef.current, p.x ?? 0, p.y ?? 0) : runner.dispatch({ type: 'pointer', kind: p.kind }))),
      listen<OverlayFlash>(EVENTS.flash, (p) => runner.dispatch({ type: 'flash', flash: p })),
      listen<DictationState>(EVENTS.dictation, (p) => runner.dispatch({ type: 'dictation_state', payload: p })),
      listen<MediaChanged>(EVENTS.media, (p) => store.getState().applyMedia(p)),
      listen<MediaHistory>(EVENTS.mediaHistory, (p) => store.getState().applyHistory(p)),
    ]
    invoke<RosterRow[]>('sessions_list')
      .then((r) => {
        if (Array.isArray(r) && !store.getState().rowsAt) store.getState().applySessions(r)
      })
      .catch(() => {})
    void invoke('overlay_request', { state: runner.get().state, pinned: runner.get().pinned, keyboard: false, reason: 'launch' }).catch(() => {})
    void st
    return () => {
      for (const s of subs) void s.then((u) => u())
    }
  }, [runner, store])

  /* ---- derived: settings, pill, suspend, theme ------------------------------ */
  // Quiet hours rest the overlay hidden, whatever `overlay.idle` says (02 §2.1).
  const quietHours = !!quiet?.quiet_hours
  useEffect(() => {
    runner.dispatch({ type: 'settings', settings: { ...settings, reduced_motion: display.reduced_motion || prefersReducedMotion(), quiet_hours: quietHours } })
  }, [runner, settings, display.reduced_motion, quietHours])

  const pillChanged = useMemo(() => pushedPill ?? localLadder(rows, Date.now(), inbox?.rows, agenda?.rows), [pushedPill, rows, inbox, agenda])
  const lastPill = useRef<string>('')
  useEffect(() => {
    const key = JSON.stringify(pillChanged)
    if (key === lastPill.current) return
    lastPill.current = key
    runner.dispatch({ type: 'pill', candidate: pillChanged.candidate, count: pillChanged.count })
  }, [runner, pillChanged])

  useEffect(() => {
    runner.dispatch({ type: quiet?.presentation ? 'suspend' : 'resume' })
  }, [runner, quiet?.presentation])

  useEffect(() => {
    const html = document.documentElement
    const isDark = isTauri() || display.dark !== undefined ? display.dark : prefersDark()
    if (isDark) html.dataset.theme = 'dark'
    else delete html.dataset.theme
  }, [display.dark])

  useEffect(() => {
    if (m.state !== 'expanded') {
      setLink(null)
      setMenu(null)
    } else if (m.openedBy === 'click' && m.focusRow) setLink(m.focusRow)
  }, [m.state, m.openedBy, m.focusRow])

  // XC-37: while the panel is open, its selected item is what a "Hey Mode" request is about; null once it closes.
  const sentContext = useRef('')
  useEffect(() => {
    const kind: AppContextKind | null = m.state === 'expanded' && m.focusRow ? CONTEXT_KIND[m.tab] : null
    const id = kind ? m.focusRow : null
    const key = kind ? `${kind}:${id}` : ''
    if (key === sentContext.current) return
    sentContext.current = key
    void invoke('app_context_set', { kind, id }).catch(() => {})
  }, [m.state, m.focusRow, m.tab])

  /* ---- actions and effects -------------------------------------------------- */
  const focusCapture = useCallback(() => captureRef.current?.focus(), [])
  const actor = useMemo(() => createActor({ runner, store, invoke, focusCapture }), [runner, store, focusCapture])
  const act: OverlayCtx['act'] = useCallback(
    (id, row, extra) => {
      if (id === 'note' && row) setLink(row.id)
      actor(id, row, extra)
    },
    [actor],
  )

  const saveCapture = useCallback(
    async (open: boolean) => {
      const text = (captureRef.current?.value ?? captureText).trim()
      if (!text) return
      const linked = store.getState().rows.find((r) => r.id === link)
      try {
        const r = await invoke<{ note_id: string } | undefined>('note_capture', linked ? { text, created_from: { kind: 'session', id: linked.id } } : { text })
        setCaptureText('')
        store.getState().setCaptureSaved(r?.note_id ?? 'note')
        if (open) act('open_tab', undefined, { tab: 'notes', id: r?.note_id })
      } catch {
        runner.dispatch({ type: 'resolved', text: "Couldn't save", keepPill: true })
      }
    },
    [captureText, link, store, act, runner],
  )

  const latest = useRef({ act, saveCapture })
  latest.current = { act, saveCapture }
  useEffect(() => {
    runner.setEffectSink((e: Effect, s: MachineState) => {
      switch (e.type) {
        case 'request':
          void invoke('overlay_request', { state: e.state, pinned: e.pinned, keyboard: e.keyboard, reason: e.reason }).catch(() => {})
          return
        case 'focus_capture':
          // The box mounts with the expanded frame in the same tick; focus it once it exists.
          setTimeout(() => captureRef.current?.focus(), 0)
          return
        case 'blur':
          if (document.activeElement === captureRef.current) captureRef.current?.blur()
          return
        case 'clear_capture':
          setCaptureText('')
          return
        case 'save_capture':
          void latest.current.saveCapture(e.open)
          return
        case 'act': {
          const row = store.getState().rows.find((r) => r.id === e.row)
          if (row) latest.current.act(e.action, row)
          else latest.current.act(e.action, undefined, { id: e.row, tab: s.tab })
          return
        }
        case 'pulse':
          setPulse(true)
          setTimeout(() => setPulse(false), 220)
          return
      }
    })
    return () => runner.setEffectSink(undefined)
  }, [runner, store])

  /* ---- geometry: report the drawn frame (02 §12) ------------------------- */
  // The window is a fixed stage; the core is told the frame's rect in it (CSS px from the viewport's top-left, with any
  // open menu): the target at the start of a move (`settled: false`) and the rest rect once it ends, from the frame
  // (NotchFrame, CapsuleFrame).
  const frameMove = useRef<{ rect: Rect | null; settled: boolean }>({ rect: null, settled: true })
  const lastRect = useRef('')
  const report = useCallback(() => {
    const root = rootRef.current
    if (!root) return
    const rects: Rect[] = []
    const settled = frameMove.current.settled
    if (frameMove.current.rect) {
      rects.push(frameMove.current.rect)
      root.querySelectorAll('.ov-menu').forEach((el) => {
        const r = el.getBoundingClientRect()
        rects.push({ x: r.left, y: r.top, width: r.width, height: r.height })
      })
    }
    const rect = unionAll(rects)
    const key = `${rect.x},${rect.y},${rect.width},${rect.height},${settled}`
    if (key === lastRect.current) return
    lastRect.current = key
    void invoke('overlay_measure', { ...rect, settled }).catch(() => {})
  }, [])
  const onFrameMove = useCallback(
    (rect: Rect | null, settled: boolean) => {
      frameMove.current = { rect, settled }
      report()
    },
    [report],
  )
  // Every render: a menu opening or closing.
  useLayoutEffect(() => report())

  /* ---- input ------------------------------------------------------------------- */
  const guard = useFocusGuard(rootRef, captureRef)
  const rowIds = useMemo(() => rows.map((r) => r.id), [rows])
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (menu) {
      if (e.key === 'Escape') {
        setMenu(null)
        e.preventDefault()
      }
      return
    }
    const action = toKeyAction(e, { rows: rowIds, captureEmpty: !(captureRef.current?.value ?? captureText) })
    if (action && runner.dispatch(action)) e.preventDefault()
  }

  const openMenu: OverlayCtx['openMenu'] = useCallback((anchor, items, onSelect) => {
    const root = rootRef.current
    if (!root) return
    const a = anchor.getBoundingClientRect()
    const b = root.getBoundingClientRect()
    // Anchored by its right edge to the trigger's (the root is the whole stage, so a left edge plus a translate would
    // let the class's `right: 0` squeeze the box and push the menu past the frame, 02 §12).
    setMenu({ items, onSelect, right: b.right - a.right, top: a.bottom - b.top })
  }, [])
  const closeMenu = useCallback(() => setMenu(null), [])

  /* ---- context ---------------------------------------------------------------- */
  const badge = mascotBadge({ rows, urgentUnread: inbox?.urgent_count ?? 0, quiet, doneHeldUntil, now })
  const pill = useMemo(() => {
    const built = buildPill(m.pill ? { candidate: m.pill, count: m.count } : null, { rows, inbox: inbox?.rows, agenda: agenda?.rows, now, rowsAt, dictation: m.dictation })
    // An app-window flash draws over whatever shows for its hold (02 §5.4); the machine never holds one with dictation.
    return (m.flash?.app && built?.kind !== 'dictation' && flashPill(m.flash)) || built
  }, [m.pill, m.count, rows, inbox, agenda, now, rowsAt, m.dictation, m.flash])
  const ticksSeconds = pill?.seconds !== undefined && hasSeconds(pill.seconds)
  useEffect(() => setPillSeconds(ticksSeconds), [ticksSeconds])
  const notch = display.notch ?? { width: 184, height: 36 }
  const linkedRow = useMemo(() => rows.find((r) => r.id === link) ?? null, [rows, link])
  const ctx: OverlayCtx = {
    runner,
    m,
    now,
    rowsAt,
    placement: display.placement,
    notch,
    dark,
    showMascot: settings.show_mascot,
    pill,
    badge,
    act,
    captureRef,
    botRef,
    linkedRow,
    setTab: (tab) => runner.dispatch({ type: 'tab', tab }),
    openMenu,
    closeMenu,
    decidedFor: (id) => decided[id],
    capture: { value: captureText, set: setCaptureText, save: (open) => void saveCapture(open), unlink: () => setLink(null) },
  }

  const reduced = display.reduced_motion || prefersReducedMotion()
  const scale = display.scale || 1
  const content = m.state === 'hidden' ? null : m.state === 'peek' ? <PeekView /> : m.state === 'compact' ? <PillView /> : <Expanded />
  // The frame is one node across the states in every placement, so it morphs (02 §12); the state views draw its content.
  let view = content
  const framed = m.state !== 'compact' || !!pill
  if (display.placement === 'notch' && m.state !== 'hidden' && framed) {
    const frame =
      m.state === 'peek'
        ? { frameClass: `m-overlay is-notch is-peek ${m.arrival ? 'ov-pulse' : ''}`, 'data-measure': '', onClick: () => runner.dispatch({ type: 'click', target: 'peek' }) }
        : m.state === 'compact'
          ? { frameClass: 'm-overlay is-notch', 'data-measure': '', onClick: () => runner.dispatch({ type: 'click', target: 'pill', row: pill?.ref }), 'aria-label': pill?.label }
          : { frameClass: 'm-overlay is-notch', 'data-state': 'expanded' }
    view = (
      <NotchFrame state={m.state} notch={notch} scale={scale} reduced={reduced} onMove={onFrameMove} {...frame}>
        {content}
      </NotchFrame>
    )
  } else if (display.placement !== 'notch' && m.state !== 'hidden' && framed) {
    view = (
      <CapsuleFrame state={m.state} placement={display.placement} scale={scale} reduced={reduced} onMove={onFrameMove}>
        {content}
      </CapsuleFrame>
    )
  }

  return (
    <OverlayContext.Provider value={ctx}>
      <ThemeProvider theme={dark ? 'dark' : 'light'}>
        <div
          ref={rootRef}
          className={`ov-root${pulse ? ' ov-pulse' : ''}`}
          data-placement={display.placement}
          data-state={m.state}
          data-pinned={m.pinned || undefined}
          data-keyboard={m.keyboard || undefined}
          data-motion={reduced ? 'reduced' : 'full'}
          style={display.scale && display.scale !== 1 ? ({ '--ov-scale': String(display.scale) } as React.CSSProperties) : undefined}
          data-testid="overlay"
          onMouseEnter={() => runner.dispatch({ type: 'pointer', kind: 'frame_enter' })}
          onMouseLeave={() => runner.dispatch({ type: 'pointer', kind: 'frame_leave' })}
          onKeyDownCapture={onKeyDown}
          onMouseDownCapture={(e) => {
            guard.onMouseDownCapture(e)
            if (menu && !(e.target as HTMLElement).closest?.('.ov-menu')) setMenu(null)
          }}
          onFocusCapture={guard.onFocusCapture}
        >
          <IconDefs />
          <BrandDefs />
          {view}
          {menu && (
            <div className="ov-menu" style={{ position: 'absolute', right: menu.right, top: menu.top }} data-testid="menu">
              <Menu label="Actions" onClose={closeMenu}>
                {menu.items.map((item) => (
                  <MenuItem
                    key={item.id}
                    dimmed={item.disabled}
                    onSelect={() => {
                      setMenu(null)
                      menu.onSelect(item.id)
                    }}
                  >
                    {item.label}
                  </MenuItem>
                ))}
              </Menu>
            </div>
          )}
        </div>
      </ThemeProvider>
    </OverlayContext.Provider>
  )
}
