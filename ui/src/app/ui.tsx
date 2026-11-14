// Small pieces the kit does not have yet (kit candidates, listed in the
// package report): a popover that anchors a `.m-menu` to its ⋮, a modal
// dialog, the inline notice of 10 §12.3 and a ticking clock. Meta lines join
// their parts with kit/text.ts (joinDot).
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Button } from '../kit/button'
import { Menu, MenuItem, MenuSep, setMenuOpener } from '../kit/controls'
import { useSessionsStore } from '../state/sessions-store'

/** `Date.now()` that re-renders every `ms` (elapsed labels tick; 05 §11.3), paused while the window is hidden (02 §15). */
export function useNow(ms = 1000): number {
  const [now, setNow] = useState(() => Date.now())
  const [visible, setVisible] = useState(() => typeof document === 'undefined' || document.visibilityState !== 'hidden')
  useEffect(() => {
    const on = () => setVisible(document.visibilityState !== 'hidden')
    document.addEventListener('visibilitychange', on)
    return () => document.removeEventListener('visibilitychange', on)
  }, [])
  useEffect(() => {
    if (!visible) return
    setNow(Date.now())
    const t = setInterval(() => setNow(Date.now()), ms)
    return () => clearInterval(t)
  }, [ms, visible])
  return now
}

/* ---- menus ---------------------------------------------------------------- */

export type MenuEntry =
  | { kind?: 'item'; id: string; label: ReactNode; danger?: boolean; dimmed?: boolean; reason?: ReactNode; checked?: boolean; run: () => void; keepOpen?: boolean }
  | { kind: 'sep'; id: string }
  | { kind: 'label'; id: string; label: ReactNode }

export interface OpenMenuOptions {
  /** `end` (the default): the menu's right edge on the anchor's (a ⋮). `start`: its left edge on the anchor's, as a select's menu sits under it. */
  align?: 'start' | 'end'
  /** At least this wide (a select's menu is at least as wide as the control). */
  minWidth?: number
}

export interface OpenMenu extends OpenMenuOptions {
  anchor: DOMRect
  entries: MenuEntry[]
  label?: string
  /** A popover's own content instead of entries (the swatch grid, 10 §7.4.19); it gets the host's close. */
  content?: (close: () => void) => ReactNode
}

let setMenuGlobal: ((m: OpenMenu | null) => void) | null = null
let menuShown = false

/** A menu or popover of the host is open: its Escape closes it, not the dialog under it. */
export function isMenuOpen(): boolean {
  return menuShown
}

/** Opens a `.m-menu` under (or above) the element that was clicked. */
export function openMenu(anchor: Element, entries: MenuEntry[], label?: string, options: OpenMenuOptions = {}) {
  setMenuGlobal?.({ anchor: anchor.getBoundingClientRect(), entries, label, ...options })
}

/** Opens a popover of its own content (a `SwatchGrid`) on the same `.m-menu` surface and host: it enters the same way, Esc and a click outside close it. */
export function openPopover(anchor: Element, content: (close: () => void) => ReactNode, label?: string, options: OpenMenuOptions = { align: 'start' }) {
  setMenuGlobal?.({ anchor: anchor.getBoundingClientRect(), entries: [], label, content, ...options })
}

// The kit's Select and Combo open their option menus through this host.
setMenuOpener((anchor, entries, label, options) => openMenu(anchor, entries, label, options))

export function closeMenu() {
  setMenuGlobal?.(null)
}

/** The one popover host of the window; rendered once by the shell. */
export function MenuHost() {
  const [menu, setMenu] = useState<OpenMenu | null>(null)
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)

  useEffect(() => {
    setMenuGlobal = setMenu
    return () => {
      if (setMenuGlobal === setMenu) setMenuGlobal = null
    }
  }, [])

  useEffect(() => {
    menuShown = menu !== null
    return () => {
      menuShown = false
    }
  }, [menu])

  useLayoutEffect(() => {
    if (!menu || !ref.current) {
      setPos(null)
      return
    }
    // The layout size, not the painted one: the menu enters scaled to .98 (10 §3.7).
    const box = { width: ref.current.offsetWidth, height: ref.current.offsetHeight }
    const vw = window.innerWidth
    const vh = window.innerHeight
    let left = menu.align === 'start' ? Math.min(menu.anchor.left, vw - box.width - 8) : menu.anchor.right - box.width
    if (left < 8) left = Math.min(menu.anchor.left, vw - box.width - 8)
    let top = menu.anchor.bottom + 4
    if (top + box.height > vh - 8) top = Math.max(8, menu.anchor.top - box.height - 4)
    // Opening above: the menu grows from its bottom edge.
    ref.current.style.setProperty('--m-menu-origin', top < menu.anchor.top ? 'bottom' : 'top')
    setPos({ left: Math.max(8, left), top })
    const first = ref.current.querySelector<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"]), [role="menuitemradio"][tabindex="0"]')
    first?.focus()
  }, [menu])

  useEffect(() => {
    if (!menu) return
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setMenu(null)
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        setMenu(null)
      }
    }
    const onScroll = () => setMenu(null)
    window.addEventListener('mousedown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('resize', onScroll)
    return () => {
      window.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('resize', onScroll)
    }
  }, [menu])

  if (!menu) return null
  const at = pos ? { left: pos.left, top: pos.top } : { left: -9999, top: -9999 }
  if (menu.content)
    return createPortal(
      <div className="ap-popover" ref={ref} style={at} data-testid="popover">
        <div className="m-menu ap-popover-body" aria-label={menu.label}>
          {menu.content(() => setMenu(null))}
        </div>
      </div>,
      document.body,
    )
  return createPortal(
    <div className="ap-popover" ref={ref} style={at} data-testid="popover">
      <Menu label={menu.label} onClose={() => setMenu(null)} style={menu.minWidth ? { minWidth: menu.minWidth } : undefined}>
        {menu.entries.map((e) => {
          if (e.kind === 'sep') return <MenuSep key={e.id} />
          if (e.kind === 'label')
            return (
              <div key={e.id} className="ap-menu-label">
                {e.label}
              </div>
            )
          return (
            <MenuItem
              key={e.id}
              danger={e.danger}
              dimmed={e.dimmed}
              reason={e.reason}
              trailing={e.checked !== undefined ? <span className="ap-check">{e.checked ? '✓' : ''}</span> : undefined}
              onSelect={() => {
                if (!e.keepOpen) setMenu(null)
                e.run()
              }}
            >
              {e.label}
            </MenuItem>
          )
        })}
      </Menu>
    </div>,
    document.body,
  )
}

/* ---- dialog ------------------------------------------------------------- */

export function Dialog({ title, onClose, children, testId }: { title: ReactNode; onClose: () => void; children: ReactNode; testId?: string }) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const first = ref.current?.querySelector<HTMLElement>('input, select, textarea, button')
    first?.focus()
    const onKey = (e: KeyboardEvent) => {
      // A menu opened from the dialog (a Select, a Combo) takes its own Escape.
      if (e.key === 'Escape' && !isMenuOpen()) {
        e.stopPropagation()
        onClose()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [onClose])
  return createPortal(
    <div
      className="ap-scrim"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className="ap-dialog" role="dialog" aria-modal="true" aria-label={typeof title === 'string' ? title : undefined} ref={ref} data-testid={testId}>
        <div className="ap-dialog-title">{title}</div>
        {children}
      </div>
    </div>,
    document.body,
  )
}

/* ---- inline notice -------------------------------------------------------- */

/** "Dismissed · Undo" for 8 s, bottom-left, `role="status"` (10 §12.3). */
export function NoticeHost() {
  const notice = useSessionsStore((s) => s.notice)
  const clear = useSessionsStore((s) => s.clearNotice)
  if (!notice) return null
  return (
    <div className="ap-notice" role="status" data-testid="notice">
      <span>{notice.text}</span>
      {notice.action && (
        <Button
          quiet
          onClick={() => {
            const run = notice.action!.run
            clear(notice.id)
            run()
          }}
        >
          {notice.action.label}
        </Button>
      )}
    </div>
  )
}

/** Copies text; the notice confirms it (no OS notification for UI feedback, 03 §2.4). */
export function copyText(text: string, what = 'Copied') {
  const done = () => useSessionsStore.getState().showNotice(what)
  try {
    const p = navigator.clipboard?.writeText(text)
    if (p) p.then(done, done)
    else done()
  } catch {
    done()
  }
}
