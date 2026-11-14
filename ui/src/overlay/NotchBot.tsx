// The resting mascot of the overlay (02 §6.1, `overlay.idle = mascot`): the landing page's vector
// bot (site/bot.js, scene `hero`, one bot) on the kit's 28-px-tall `.m-notch-bot` canvas at device
// scale. bot.js reads its ink from the canvas's color (`--m-fg`) and its paper from what is behind
// it (the bezel in the notch, the tab's surface elsewhere). Its eyes follow the pointer: over the
// window through bot.js's own listener, beyond it through `look(x, y)`, which the overlay root calls
// with the core's pointer `move` events (the window sees the pointer only over itself). No arrival
// turn (the peek remounts after every flash). Under reduced motion it draws one still frame.
import { useEffect, useRef, type MutableRefObject } from 'react'
import '../../../site/bot.js'
import { StateBadge } from '../kit/avatars'
import type { SessionState } from '../kit/types'

/** The canvas carries its mount, so the root can feed it the pointer and the e2e walk can read it. */
export type BotCanvas = HTMLCanvasElement & { modeBot?: ModeBotMount }

/** The pointer distance (px) at which the gaze saturates: the core stops sending moves past 600 px (02 §16). */
export const GAZE_RANGE_PX = 600

export interface NotchBotProps {
  badge?: SessionState
  reduced: boolean
  /** Set to the canvas while it is mounted (the root's pointer feed). */
  canvasRef?: MutableRefObject<BotCanvas | null>
}

export function NotchBot({ badge, reduced, canvasRef }: NotchBotProps) {
  const ref = useRef<BotCanvas>(null)
  useEffect(() => {
    const canvas = ref.current
    const bot = window.ModeBot
    if (!canvas || !bot || !canvas.getContext('2d')) return
    const mounted = bot.mount(canvas, { scene: 'hero', pointer: 'gaze', reduced, range: GAZE_RANGE_PX, tour: false })
    canvas.modeBot = mounted
    canvas.dataset.drawn = ''
    if (canvasRef) canvasRef.current = canvas
    // A pack change repaints the frame behind it; draw again with its colors.
    const scheme = window.matchMedia?.('(prefers-color-scheme: dark)')
    const redraw = () => mounted.seek(mounted.t)
    const attrs = new MutationObserver(redraw)
    attrs.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'class', 'style'] })
    scheme?.addEventListener?.('change', redraw)
    return () => {
      mounted.destroy()
      delete canvas.modeBot
      if (canvasRef?.current === canvas) canvasRef.current = null
      attrs.disconnect()
      scheme?.removeEventListener?.('change', redraw)
    }
  }, [reduced, canvasRef])
  return (
    <span className="m-notch-bot" data-testid="notch-bot">
      <canvas ref={ref} aria-hidden="true" />
      {badge && <StateBadge state={badge} />}
    </span>
  )
}

/** Feeds a pointer position relative to the overlay window (CSS px) to the bot, converted to its canvas. */
export function lookAt(canvas: BotCanvas | null, x: number, y: number) {
  const bot = canvas?.modeBot
  if (!canvas || !bot) return
  const r = canvas.getBoundingClientRect()
  bot.look(x - r.left, y - r.top)
}
