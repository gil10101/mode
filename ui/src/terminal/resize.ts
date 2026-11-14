// PTY resize forwarding (05 §8.1, 10 §13): the emulator re-fits on every
// layout change; the PTY gets the size once it settles (100 ms debounce),
// and only when it actually changed.

export const RESIZE_DEBOUNCE_MS = 100

export interface ResizeSender {
  /** A new emulator size; sent after the debounce unless another one replaces it. */
  schedule(cols: number, rows: number): void
  /** Send a pending size now. */
  flush(): void
  /** The size the PTY is known to have (from `pty_open` / `pty.started`): not re-sent. */
  known(cols: number, rows: number): void
  dispose(): void
}

export function createResizeSender(send: (cols: number, rows: number) => void, delay = RESIZE_DEBOUNCE_MS): ResizeSender {
  let timer: ReturnType<typeof setTimeout> | null = null
  let next: { cols: number; rows: number } | null = null
  let last: { cols: number; rows: number } | null = null

  const fire = () => {
    timer = null
    const s = next
    next = null
    if (!s || s.cols < 1 || s.rows < 1) return
    if (last && last.cols === s.cols && last.rows === s.rows) return
    last = s
    send(s.cols, s.rows)
  }

  return {
    schedule(cols, rows) {
      next = { cols, rows }
      if (timer) clearTimeout(timer)
      timer = setTimeout(fire, delay)
    },
    flush() {
      if (timer) clearTimeout(timer)
      fire()
    },
    known(cols, rows) {
      last = { cols, rows }
    },
    dispose() {
      if (timer) clearTimeout(timer)
      timer = null
      next = null
    },
  }
}
