// The previews' clock (03 §2.8): each preview plays one loop of `period` ms and
// starts again; `t` is the time into the current loop. With reduced motion the
// loop does not run and `t` stays at the end, so a preview shows its end state.
import { useEffect, useState } from 'react'

const TICK = 50

export function reducedMotion(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches
}

export function useLoop(period: number): number {
  const [reduced] = useState(reducedMotion)
  const [t, setT] = useState(reduced ? period : 0)
  useEffect(() => {
    if (reduced) return
    // Counted in ticks, not read from a clock: a test's frozen clock does not stop the preview.
    const id = window.setInterval(() => setT((x) => (x + TICK) % period), TICK)
    return () => window.clearInterval(id)
  }, [period, reduced])
  return t
}

/** The first `n` characters of `text` typed at `cps` characters a second from `from` ms. */
export function typed(text: string, t: number, from: number, cps = 28): string {
  if (t <= from) return ''
  return text.slice(0, Math.floor(((t - from) / 1000) * cps))
}
