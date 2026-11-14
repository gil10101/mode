// The Welcome step's mascot (03 §2.8): the landing page's matrix bot (site/bot.js, scene
// `hero-matrix`) on a 200-px canvas. bot.js reads its ink from the canvas's color (`--m-fg`) and
// its paper from the panel behind it (`--m-surface`), so both packs draw right. It keeps its idle
// life (the multiply-in, blinks, gaze drift) and its eyes follow the pointer across the window,
// nothing more (`pointer: 'gaze'`: no shying away, no squint on click, no pointer cursor). Under
// reduced motion it draws one still frame.
import { useEffect, useRef } from 'react'
import '../../../../site/bot.js'
import { reducedMotion } from './previews/loop'

/** The canvas carries its mount so the e2e walk can hold the clock and check the gaze. */
export type BotCanvas = HTMLCanvasElement & { modeBot?: ModeBotMount }

export function WelcomeBot() {
  const ref = useRef<BotCanvas>(null)
  useEffect(() => {
    const canvas = ref.current
    const bot = window.ModeBot
    if (!canvas || !bot || !canvas.getContext('2d')) return
    const mounted = bot.mount(canvas, { scene: 'hero-matrix', reduced: reducedMotion(), pointer: 'gaze' })
    canvas.modeBot = mounted
    canvas.dataset.drawn = ''
    // A pack change repaints the panel; draw again with its colors.
    const scheme = window.matchMedia?.('(prefers-color-scheme: dark)')
    const redraw = () => mounted.seek(mounted.t)
    const attrs = new MutationObserver(redraw)
    attrs.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'class', 'style'] })
    scheme?.addEventListener?.('change', redraw)
    return () => {
      mounted.destroy()
      delete canvas.modeBot
      attrs.disconnect()
      scheme?.removeEventListener?.('change', redraw)
    }
  }, [])
  return <canvas ref={ref} className="m-ob-bot" width={400} height={400} aria-hidden="true" />
}
