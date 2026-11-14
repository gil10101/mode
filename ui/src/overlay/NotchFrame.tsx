// The notch wrapper and frame (02 §6.1, §12): one pair of nodes across `peek`,
// `compact` and `expanded`, so the frame morphs between the states' sizes with
// a CSS transition instead of being replaced. The state views render only the
// frame's children, each laid out at its own final size; this measures them,
// sets the frame's target and tells the overlay where the frame is going and
// when it is at rest.
import { useRef, type HTMLAttributes, type ReactNode } from 'react'
import { ThemeProvider } from '../kit/theme'
import { notchVars } from './Compact'
import { notchRect, type MorphState, type Rect } from './morph'
import { useFrameMorph } from './useFrameMorph'

export type NotchFrameProps = {
  state: Exclude<MorphState, 'hidden'>
  notch: { width: number; height: number }
  /** The page zoom (`--ov-scale`). */
  scale: number
  reduced: boolean
  /** The frame's classes (`m-overlay is-notch …`). */
  frameClass: string
  /** The rect the wrapper is moving to (`settled: false`) or rests at (`settled: true`); null once it unmounts. */
  onMove: (rect: Rect | null, settled: boolean) => void
  children?: ReactNode
} & Omit<HTMLAttributes<HTMLDivElement>, 'className' | 'children'> & { 'data-measure'?: string; 'data-state'?: string }

export function NotchFrame({ state, notch, scale, reduced, frameClass, onMove, children, ...frameProps }: NotchFrameProps) {
  const wrapRef = useRef<HTMLDivElement | null>(null)
  const frameRef = useRef<HTMLDivElement | null>(null)
  useFrameMorph({
    wrapRef,
    frameRef,
    state,
    reduced,
    start: notch,
    rectOf: (w, h) => notchRect(w, h, window.innerWidth, scale || 1),
    onMove,
  })

  return (
    <ThemeProvider theme="dark">
      <div ref={wrapRef} className={`m-notch${state === 'peek' ? ' is-peek' : ''} ov-frame`} style={notchVars(notch)}>
        <div ref={frameRef} className={frameClass} data-testid="overlay-frame" {...frameProps}>
          {children}
        </div>
      </div>
    </ThemeProvider>
  )
}
