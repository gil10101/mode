// The reserved-key split of the Terminal tab (03 §3.8): which keystrokes go
// to the PTY and which stay with the app, per OS.
//
//   macOS: every ⌘ shortcut stays with the app; everything else (Ctrl+C,
//          Esc, Option keys) goes to the PTY.
//   Windows (and Linux, which the spec does not list; it follows Windows):
//          Ctrl+Shift+<key> and Ctrl+1…9 stay with the app; Ctrl+C goes to
//          the PTY.
//   Both:  ⌘/Ctrl+F opens find, ⌘/Ctrl+V pastes (bracketed), and
//          ⌘/Ctrl+Shift+Esc moves focus out of the terminal.

export type Platform = 'mac' | 'windows' | 'linux'

/**
 * Where a keystroke goes: `pty` (the emulator encodes it), `app` (left to
 * the app's own shortcuts; the emulator ignores it), or one of the
 * terminal's own commands.
 */
export type KeyRoute = 'pty' | 'app' | 'find' | 'paste' | 'leave'

export interface KeyLike {
  key: string
  code?: string
  ctrlKey: boolean
  metaKey: boolean
  shiftKey: boolean
  altKey: boolean
  isComposing?: boolean
}

interface NavigatorLike {
  userAgentData?: { platform?: string } | null
  platform?: string
  userAgent?: string
}

/** The OS from `navigator` (Client Hints first, then `platform`, then the user agent). */
export function detectPlatform(nav?: NavigatorLike | null): Platform {
  const n = nav ?? (typeof navigator !== 'undefined' ? (navigator as NavigatorLike) : null)
  const hint = n?.userAgentData?.platform || n?.platform || n?.userAgent || ''
  const p = hint.toLowerCase()
  if (/mac|iphone|ipad|ipod/.test(p)) return 'mac'
  if (/win/.test(p)) return 'windows'
  return 'linux'
}

/** The letter a key types, layout first, physical key as a fallback for non-Latin layouts. */
function letter(e: KeyLike): string | null {
  if (e.key.length === 1 && /[a-z]/i.test(e.key)) return e.key.toLowerCase()
  const m = e.code ? /^Key([A-Z])$/.exec(e.code) : null
  return m ? m[1].toLowerCase() : null
}

function digit1to9(e: KeyLike): boolean {
  return /^[1-9]$/.test(e.key) || (!!e.code && /^(Digit|Numpad)[1-9]$/.test(e.code))
}

export function routeKey(e: KeyLike, platform: Platform): KeyRoute {
  // IME composition and dead keys always belong to the emulator.
  if (e.isComposing || e.key === 'Process' || e.key === 'Dead') return 'pty'
  const esc = e.key === 'Escape'
  if (platform === 'mac') {
    if (!e.metaKey) return 'pty'
    if (e.shiftKey && esc) return 'leave'
    const plain = !e.shiftKey && !e.altKey && !e.ctrlKey
    if (plain && letter(e) === 'f') return 'find'
    if (plain && letter(e) === 'v') return 'paste'
    return 'app'
  }
  if (!e.ctrlKey || e.metaKey || e.altKey) return 'pty' // Ctrl+Alt is AltGr on Windows: characters for the PTY
  if (e.shiftKey) return esc ? 'leave' : 'app'
  if (letter(e) === 'f') return 'find'
  if (letter(e) === 'v') return 'paste'
  if (digit1to9(e)) return 'app'
  return 'pty'
}
