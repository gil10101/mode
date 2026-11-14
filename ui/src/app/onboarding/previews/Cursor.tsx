// The drawn pointer of a preview (03 §2.8): the macOS arrow (not a brand mark), 28 px, its tip on
// the point. It rests at `rest` (fractions of the preview), glides to the center of the element
// `to` names inside the preview (measured, so it lands on the real control at any window width)
// and presses while `pressed`. Not drawn under reduced motion, where the preview shows its end state.
import { useLayoutEffect, useRef, useState } from 'react'
import { reducedMotion } from './loop'

export interface CursorProps {
  /** A selector inside the preview to point at; null rests. */
  to: string | null
  pressed?: boolean
  visible?: boolean
  /** Where it rests, as fractions of the preview's width and height. */
  rest?: [number, number]
}

export function Cursor({ to, pressed, visible = true, rest = [0.62, 0.86] }: CursorProps) {
  const ref = useRef<SVGSVGElement>(null)
  const [at, setAt] = useState<{ x: number; y: number } | null>(null)
  const [reduced] = useState(reducedMotion)
  useLayoutEffect(() => {
    const preview = ref.current?.closest('.m-ob-preview')
    const target = to ? preview?.querySelector(to) : null
    if (!preview || !target) {
      setAt(null)
      return
    }
    const p = preview.getBoundingClientRect()
    const r = target.getBoundingClientRect()
    const x = Math.round(r.left - p.left + r.width / 2)
    const y = Math.round(r.top - p.top + r.height / 2)
    setAt((a) => (a && a.x === x && a.y === y ? a : { x, y }))
  }, [to])
  if (reduced) return null
  const left = at ? `${at.x}px` : `${rest[0] * 100}%`
  const top = at ? `${at.y}px` : `${rest[1] * 100}%`
  return (
    <svg
      ref={ref}
      className={`m-ob-cursor${pressed ? ' is-pressed' : ''}${visible ? '' : ' is-hidden'}`}
      style={{ left, top }}
      width="28"
      height="28"
      viewBox="0 0 24 24"
      aria-hidden="true"
      data-testid="preview-cursor"
    >
      <path d="M5.5 3.2v16.6l4.6-4.1 2.9 6.3 2.6-1.2-2.9-6.2 6.1-.6z" />
    </svg>
  )
}
