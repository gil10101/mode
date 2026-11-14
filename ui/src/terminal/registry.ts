// Live emulators by session id, read-only views for the browser mock's test
// hook (`window.__mode_pty.screen`). Nothing in the app reads it.

export interface TerminalProbe {
  /** The buffer's text, scrollback included, trailing blank lines dropped. */
  text(): string
  /** The renderer in use. */
  renderer(): 'webgl' | 'dom'
  /** Color of the first cell at or after `x` on the first buffer line containing `needle`: palette index, `#rrggbb`, or `default`. */
  colorOf(needle: string): string | null
  size(): { cols: number; rows: number }
  /** The xterm theme in use (resolved from the tokens). */
  theme(): { background?: string; foreground?: string; cursor?: string; red?: string }
}

export const terminals = new Map<string, TerminalProbe>()
