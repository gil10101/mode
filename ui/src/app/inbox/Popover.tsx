// A popover anchored under (or above) the element that opened it, closed by
// Esc and outside clicks: the snooze popover of 03 §4.7 (kit candidate next
// to the package 6 menu host, which only takes menu rows).
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'

export function Popover({ anchor, onClose, children, label, testId }: { anchor: DOMRect; onClose: () => void; children: ReactNode; label?: string; testId?: string }) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  useLayoutEffect(() => {
    if (!ref.current) return
    const box = ref.current.getBoundingClientRect()
    const vw = window.innerWidth
    const vh = window.innerHeight
    let left = anchor.right - box.width
    if (left < 8) left = Math.min(anchor.left, vw - box.width - 8)
    let top = anchor.bottom + 4
    if (top + box.height > vh - 8) top = Math.max(8, anchor.top - box.height - 4)
    setPos({ left: Math.max(8, left), top })
  }, [anchor])
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        e.preventDefault()
        onClose()
      }
    }
    window.addEventListener('mousedown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('keydown', onKey, true)
    }
  }, [onClose])
  return createPortal(
    <div className="ib-popover" ref={ref} style={pos ? { left: pos.left, top: pos.top } : { left: -9999, top: -9999 }} role="dialog" aria-label={label} data-testid={testId}>
      {children}
    </div>,
    document.body,
  )
}
