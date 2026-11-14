// A popover anchored above (or below) an element: the `+` menu and its
// Connectors submenu. Esc and a click outside close it; focus moves to its
// first menu row. (Kit candidate: the shell's MenuHost only draws plain rows.)
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'

export function AnchoredPop({ anchor, onClose, children, testId }: { anchor: DOMRect; onClose: () => void; children: ReactNode; testId?: string }) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const box = el.getBoundingClientRect()
    const vw = window.innerWidth
    let top = anchor.top - box.height - 6
    if (top < 8) top = Math.min(anchor.bottom + 6, window.innerHeight - box.height - 8)
    const left = Math.max(8, Math.min(anchor.left, vw - box.width - 8))
    setPos({ left, top })
    el.querySelector<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])')?.focus()
  }, [anchor])

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onClose()
      } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        const items = Array.from(ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]:not([aria-disabled="true"])') ?? [])
        if (!items.length) return
        e.preventDefault()
        const i = items.indexOf(document.activeElement as HTMLElement)
        const next = items[(i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length]
        next?.focus()
      }
    }
    window.addEventListener('mousedown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('resize', onClose)
    return () => {
      window.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('resize', onClose)
    }
  }, [onClose])

  return createPortal(
    <div className="ch-pop" ref={ref} style={pos ? { left: pos.left, top: pos.top } : { left: -9999, top: -9999 }} data-testid={testId}>
      {children}
    </div>,
    document.body,
  )
}
