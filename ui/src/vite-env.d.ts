/// <reference types="vite/client" />

/** The landing page's mascot (site/bot.js), mounted by the onboarding Welcome step and the overlay's resting peek. */
interface ModeBotMount {
  play(): void
  pause(): void
  seek(t: number): void
  destroy(): void
  /** The eyes look at (x, y), canvas-relative px (the overlay feeds the core's pointer `move` events); a no-op under reduced motion. */
  look(x: number, y: number): void
  readonly t: number
  readonly playing: boolean
  /** Test hooks: frames drawn so far; a frame requested; (the resting gaze bot) a timer armed for the next blink. */
  readonly frames: number
  readonly looping: boolean
  readonly waiting: boolean
  /** Seconds until the next blink starts (0 during one). */
  readonly nextBlinkIn: number
}
interface ModeBotFrameOptions {
  w: number
  h: number
  dpr?: number
  ink?: string
  paper?: string
  ground?: boolean
  reduced?: boolean
}
interface Window {
  ModeBot?: {
    mount(canvas: HTMLCanvasElement, opts?: { scene?: 'hero' | 'hero-matrix' | 'matrix' | 'crowd'; reduced?: boolean; /** false: idle life only, the pointer is ignored; 'gaze': the eyes follow the pointer, no other pointer play. */ pointer?: boolean | 'gaze'; /** The pointer distance (px) at which the hero's gaze saturates; half the window by default. */ range?: number; /** false: no arrival turn (the hero scene). */ tour?: boolean }): ModeBotMount
    frame(ctx: CanvasRenderingContext2D, scene: string, t: number, opts: ModeBotFrameOptions): void
  }
}
