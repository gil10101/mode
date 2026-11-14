// The frame's morph (02 §12), shared by every placement: one persistent node
// whose width and height are set to the arriving state's content size and
// carried there by a CSS transition (`--ov-w`, `--ov-h`, `--ov-ms`,
// `--ov-ease` on the wrapper), while the overlay is told where the frame is
// going (`settled: false`) and when it is at rest (`settled: true`).
import { useLayoutEffect, useRef, type RefObject } from 'react'
import { pickRow, type MorphState, type Rect, type Row } from './morph'

interface Motion {
  state: MorphState
  w: number
  h: number
  row: Row
  move: number
  settled: boolean
  timer?: ReturnType<typeof setTimeout>
}

export interface FrameMorphOptions {
  /** The element the variables are set on (the notch's `.m-notch`, the capsules' `.ov-morph`). */
  wrapRef: RefObject<HTMLElement | null>
  /** The element whose size animates; its children are the state's content, each at its own final size. */
  frameRef: RefObject<HTMLElement | null>
  state: Exclude<MorphState, 'hidden'>
  reduced: boolean
  /** The frame's rect in the stage for a content size of `w` × `h` local CSS px. */
  rectOf: (w: number, h: number) => Rect
  /** Where a move out of `hidden` starts (the notch: the housing); none: the frame appears at its size. */
  start?: { width: number; height: number }
  onMove: (rect: Rect | null, settled: boolean) => void
}

/** The children's own size in local CSS px: the widest child, and the column's bottom edge (never the frame's, which animates). */
export function contentSize(frame: HTMLElement): { w: number; h: number } {
  let w = 0
  let h = 0
  for (const c of Array.from(frame.children) as HTMLElement[]) {
    const margin = parseFloat(getComputedStyle(c).marginBottom) || 0
    w = Math.max(w, c.offsetWidth)
    h = Math.max(h, c.offsetTop + c.offsetHeight + margin)
  }
  return { w: Math.ceil(w), h: Math.ceil(h) }
}

export function useFrameMorph(opts: FrameMorphOptions): void {
  const motion = useRef<Motion>({ state: 'hidden', w: 0, h: 0, row: pickRow('hidden', 'peek'), move: 0, settled: true })
  const latest = useRef(opts)
  latest.current = opts
  const retargetRef = useRef<() => void>(() => {})

  const rectNow = () => {
    const m = motion.current
    return latest.current.rectOf(m.w, m.h)
  }
  const finish = (move: number) => {
    const m = motion.current
    if (m.move !== move || m.settled) return
    m.settled = true
    clearTimeout(m.timer)
    latest.current.onMove(rectNow(), true)
  }

  retargetRef.current = () => {
    const { wrapRef, frameRef, state: to, reduced: still, start } = latest.current
    const wrap = wrapRef.current
    const frame = frameRef.current
    if (!wrap || !frame) return
    const { w, h } = contentSize(frame)
    const m = motion.current
    if (m.state === to && m.w === w && m.h === h) return
    const from = m.state
    // A new size inside a move still under way keeps that move's row; a state change or a move from rest takes its own.
    if (from !== to || m.settled) m.row = pickRow(from, to)
    wrap.style.setProperty('--ov-ms', still ? '0ms' : `${m.row.ms}ms`)
    wrap.style.setProperty('--ov-ease', m.row.ease)
    if (from === 'hidden' && !still && start) {
      // Out of the housing: start at its size and flush, so the transition has a start (02 §12).
      wrap.style.setProperty('--ov-w', `${start.width}px`)
      wrap.style.setProperty('--ov-h', `${start.height}px`)
      void frame.offsetWidth
    }
    m.state = to
    m.w = w
    m.h = h
    m.move += 1
    wrap.style.setProperty('--ov-w', `${w}px`)
    wrap.style.setProperty('--ov-h', `${h}px`)
    clearTimeout(m.timer)
    if (still || (from === 'hidden' && !start)) {
      m.settled = true
      latest.current.onMove(rectNow(), true)
      return
    }
    m.settled = false
    latest.current.onMove(rectNow(), false)
    const move = m.move
    m.timer = setTimeout(() => finish(move), m.row.ms + 40)
  }

  // Every render: the state may have changed (its new children are mounted by now) or the content resized.
  useLayoutEffect(() => retargetRef.current())

  useLayoutEffect(() => {
    const frame = latest.current.frameRef.current
    if (!frame) return
    const onEnd = (e: TransitionEvent) => {
      if (e.target === frame && (e.propertyName === 'width' || e.propertyName === 'height')) finish(motion.current.move)
    }
    frame.addEventListener('transitionend', onEnd)
    const ro = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => retargetRef.current())
    const observe = () => {
      if (!ro) return
      ro.disconnect()
      for (const c of Array.from(frame.children)) ro.observe(c)
    }
    observe()
    // A child replaced without a render of the frame's component (a store update inside a state view).
    const mo =
      typeof MutationObserver === 'undefined'
        ? null
        : new MutationObserver(() => {
            observe()
            retargetRef.current()
          })
    mo?.observe(frame, { childList: true })
    const onResize = () => {
      const m = motion.current
      if (m.state !== 'hidden') latest.current.onMove(rectNow(), m.settled)
    }
    window.addEventListener('resize', onResize)
    // A remount of the effect (StrictMode) re-reports where the frame is and re-arms a move's fallback.
    onResize()
    const m = motion.current
    if (!m.settled) {
      const move = m.move
      clearTimeout(m.timer)
      m.timer = setTimeout(() => finish(move), m.row.ms + 40)
    }
    return () => {
      frame.removeEventListener('transitionend', onEnd)
      ro?.disconnect()
      mo?.disconnect()
      window.removeEventListener('resize', onResize)
      clearTimeout(motion.current.timer)
      latest.current.onMove(null, true)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
}
