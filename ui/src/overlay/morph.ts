// The frame's morph (02 §12): the native window is a fixed stage and the
// drawn frame moves inside it. These are the pure parts: which motion row a
// move takes, and the rect the core is told.

export type MorphState = 'hidden' | 'peek' | 'compact' | 'expanded'

export interface Row {
  ms: number
  ease: string
}

export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

/** The fillets either side of the frame (mode.css `.m-notch { padding: 0 14px }`). */
export const FILLET = 14

const RANK: Record<MorphState, number> = { hidden: 0, peek: 1, compact: 2, expanded: 3 }

/** The 02 §12 row for a move from one state to another (the same state: a new size). */
export function pickRow(from: MorphState, to: MorphState): Row {
  if (from === to) return { ms: 120, ease: 'var(--m-ease-in-out)' }
  if (RANK[to] < RANK[from]) return { ms: 160, ease: 'var(--m-ease-in)' }
  if (to === 'expanded') return { ms: 220, ease: 'var(--m-ease-in-out)' }
  if (from === 'hidden' && to === 'peek') return { ms: 120, ease: 'var(--m-ease-out)' }
  return { ms: 180, ease: 'var(--m-ease-out)' }
}

/** The notch wrapper's rect in the stage for a frame of `w` × `h` (local CSS px), centered, hanging from the top; `scale` is the page zoom. */
export function notchRect(w: number, h: number, viewportWidth: number, scale = 1): Rect {
  const width = (w + 2 * FILLET) * scale
  return { x: (viewportWidth - width) / 2, y: 0, width, height: h * scale }
}

/** A capsule placement's frame rect in the stage for a frame of `w` × `h` (local CSS px): centered (`top-bar`) or flush with the
 * stage's right edge (`right-edge`), its top at `top` (viewport px); `scale` is the page zoom. */
export function capsuleRect(placement: 'top-bar' | 'right-edge', w: number, h: number, viewportWidth: number, top = 0, scale = 1): Rect {
  const width = w * scale
  return { x: placement === 'right-edge' ? viewportWidth - width : (viewportWidth - width) / 2, y: top, width, height: h * scale }
}

export function unionRect(a: Rect, b: Rect): Rect {
  const x = Math.min(a.x, b.x)
  const y = Math.min(a.y, b.y)
  return { x, y, width: Math.max(a.x + a.width, b.x + b.width) - x, height: Math.max(a.y + a.height, b.y + b.height) - y }
}

/** The union of a list of rects; a zero rect when there are none. */
export function unionAll(rects: Rect[]): Rect {
  return rects.length ? rects.reduce(unionRect) : { x: 0, y: 0, width: 0, height: 0 }
}
