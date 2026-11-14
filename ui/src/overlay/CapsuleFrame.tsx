// The capsule placements' frame (02 §6.2, §6.3, §12): one node across `peek`,
// `compact` and `expanded`, as in the notch, so the tab, the pill and the
// panel grow and shrink into each other instead of swapping in one step. The
// node draws the silhouette while it moves (the surface, the hairline edge,
// the state's corners) and clips the arriving state's view, which is laid out
// at its own final size and fades in over the move; at rest it coincides with
// that view's own box.
import { Fragment, useRef, type ReactNode } from 'react'
import { capsuleRect, type MorphState, type Rect } from './morph'
import { useFrameMorph } from './useFrameMorph'

export interface CapsuleFrameProps {
  state: Exclude<MorphState, 'hidden'>
  placement: 'top-bar' | 'right-edge'
  /** The page zoom (`--ov-scale`). */
  scale: number
  reduced: boolean
  onMove: (rect: Rect | null, settled: boolean) => void
  children?: ReactNode
}

export function CapsuleFrame({ state, placement, scale, reduced, onMove, children }: CapsuleFrameProps) {
  const ref = useRef<HTMLDivElement | null>(null)
  useFrameMorph({
    wrapRef: ref,
    frameRef: ref,
    state,
    reduced,
    rectOf: (w, h) => capsuleRect(placement, w, h, window.innerWidth, ref.current?.getBoundingClientRect().top ?? 0, scale || 1),
    onMove,
  })
  return (
    <div ref={ref} className="ov-morph" data-shape={state} data-testid="overlay-morph">
      {/* A state change remounts the view, so its content fades in over the move. */}
      <Fragment key={state}>{children}</Fragment>
    </div>
  )
}
