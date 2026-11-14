// Key chords for the Shortcuts view (03 §7D): one string form for storage
// (`Ctrl+Alt+Shift+Space`, the form `hotkeys::parse_chord` reads; `CmdOrCtrl`
// for ⌘ on macOS and Ctrl elsewhere), key caps per platform (⌘ ⌥ ⇧ ⌃ on
// macOS, Ctrl Alt Shift Win on Windows and Linux), recording from a
// KeyboardEvent, a canonical form for conflict checks, and the search text.
// This is the only module that turns a key into text (03 §1 "Keys").
import type { Os } from '../../state/settings-store'

export interface Chord {
  ctrl: boolean
  alt: boolean
  shift: boolean
  /** ⌘ on macOS, Win elsewhere. */
  meta: boolean
  /** `K`, `1`, `Enter`, `Space`, `Up`, `[`, `F2`, `?`, `…`. */
  key: string
}

const KEY_ALIASES: Record<string, string> = {
  return: 'Enter',
  enter: 'Enter',
  escape: 'Esc',
  esc: 'Esc',
  space: 'Space',
  spacebar: 'Space',
  tab: 'Tab',
  backspace: 'Backspace',
  delete: 'Delete',
  del: 'Delete',
  up: 'Up',
  arrowup: 'Up',
  down: 'Down',
  arrowdown: 'Down',
  left: 'Left',
  arrowleft: 'Left',
  right: 'Right',
  arrowright: 'Right',
  comma: ',',
  period: '.',
  slash: '/',
  backslash: '\\',
  bracketleft: '[',
  bracketright: ']',
}

function keyName(t: string): string {
  const alias = KEY_ALIASES[t.toLowerCase()]
  if (alias) return alias
  if (/^f\d{1,2}$/i.test(t)) return t.toUpperCase()
  if (t.length === 1) return t.toUpperCase()
  return t
}

/** `Ctrl+Alt+Shift+Space`, `CmdOrCtrl+K`, `⌥⇧Space` → a chord for `os`; null for an empty or unbound string. */
export function parseChord(s: string | null | undefined, os: Os): Chord | null {
  if (!s || !s.trim()) return null
  const c: Chord = { ctrl: false, alt: false, shift: false, meta: false, key: '' }
  const glyphs: Record<string, keyof Omit<Chord, 'key'>> = { '⌘': 'meta', '⌥': 'alt', '⇧': 'shift', '⌃': 'ctrl' }
  // A literal "+" key is written as the last token ("CmdOrCtrl++" or "+").
  const raw = s.trim()
  const parts = raw === '+' ? ['+'] : raw.endsWith('++') ? [...raw.slice(0, -2).split('+'), '+'] : raw.split('+')
  for (let part of parts) {
    part = part.trim()
    while (part && glyphs[part[0]!] && part.length > 1) {
      c[glyphs[part[0]!]!] = true
      part = part.slice(1).trim()
    }
    if (part.length === 1 && glyphs[part]) {
      c[glyphs[part]!] = true
      continue
    }
    switch (part.toLowerCase()) {
      case 'ctrl':
      case 'control':
        c.ctrl = true
        continue
      case 'alt':
      case 'option':
      case 'opt':
        c.alt = true
        continue
      case 'shift':
        c.shift = true
        continue
      case 'cmd':
      case 'command':
      case 'super':
      case 'meta':
      case 'win':
        c.meta = true
        continue
      case 'cmdorctrl':
      case 'commandorcontrol':
      case 'mod':
        if (os === 'macos') c.meta = true
        else c.ctrl = true
        continue
      case '':
        continue
    }
    c.key = keyName(part)
  }
  return c.key ? c : null
}

/** The storage form: modifiers in a fixed order, then the key (`Ctrl+Alt+Shift+Space`, `Cmd+Shift+M`). */
export function formatChord(c: Chord, os: Os): string {
  const mods: string[] = []
  if (c.meta && os === 'macos') mods.push('Cmd')
  if (c.ctrl) mods.push('Ctrl')
  if (c.alt) mods.push('Alt')
  if (c.shift) mods.push('Shift')
  if (c.meta && os !== 'macos') mods.push('Win')
  return [...mods, c.key].join('+')
}

/** Canonical form for comparisons: modifiers sorted, key upper-cased. */
export function canonical(s: string | null | undefined, os: Os): string | null {
  const c = parseChord(s, os)
  if (!c) return null
  return [c.ctrl && 'ctrl', c.alt && 'alt', c.shift && 'shift', c.meta && 'meta', c.key.toUpperCase()].filter(Boolean).join('+')
}

const MAC_KEY: Record<string, string> = { Enter: '↵', Up: '↑', Down: '↓', Left: '←', Right: '→', Backspace: '⌫', Delete: '⌦', Tab: '⇥' }
const OTHER_KEY: Record<string, string> = { Up: '↑', Down: '↓', Left: '←', Right: '→' }

/** One string per key cap, in the platform's order: ⌘ ⌃ ⌥ ⇧ key on macOS, Ctrl Alt Shift Win key elsewhere. */
export function caps(c: Chord | null, os: Os): string[] {
  if (!c) return []
  if (os === 'macos') {
    const out: string[] = []
    if (c.meta) out.push('⌘')
    if (c.ctrl) out.push('⌃')
    if (c.alt) out.push('⌥')
    if (c.shift) out.push('⇧')
    out.push(MAC_KEY[c.key] ?? c.key)
    return out
  }
  const out: string[] = []
  if (c.ctrl) out.push('Ctrl')
  if (c.alt) out.push('Alt')
  if (c.shift) out.push('Shift')
  if (c.meta) out.push('Win')
  out.push(OTHER_KEY[c.key] ?? c.key)
  return out
}

/** Caps joined for a menu's "Copy key" and the conflict line ("⌥⇧Space", "Ctrl+Alt+Space"). */
export function capsText(c: Chord | null, os: Os): string {
  const cs = caps(c, os)
  return os === 'macos' ? cs.join('') : cs.join('+')
}

/** Words a search can match: modifier names in both spellings plus the key ("cmd ctrl shift k"). */
export function searchWords(c: Chord | null, os: Os): string {
  if (!c) return ''
  const w: string[] = []
  if (c.meta) w.push(os === 'macos' ? 'cmd command ⌘' : 'win super')
  if (c.ctrl) w.push('ctrl control ⌃')
  if (c.alt) w.push(os === 'macos' ? 'alt option opt ⌥' : 'alt')
  if (c.shift) w.push('shift ⇧')
  w.push(c.key.toLowerCase())
  return `${w.join(' ')} ${capsText(c, os).toLowerCase()}`
}

const CODE_KEYS: Record<string, string> = {
  Space: 'Space',
  Enter: 'Enter',
  NumpadEnter: 'Enter',
  Escape: 'Esc',
  Tab: 'Tab',
  Backspace: 'Backspace',
  Delete: 'Delete',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Comma: ',',
  Period: '.',
  Slash: '/',
  Semicolon: ';',
  Quote: "'",
  Minus: '-',
  Equal: '=',
  Backquote: '`',
}

const MODIFIER_KEYS = new Set(['Shift', 'Control', 'Alt', 'Meta', 'OS', 'AltGraph', 'CapsLock', 'Fn'])

export type Recorded = { kind: 'chord'; chord: Chord } | { kind: 'modifier' } | { kind: 'cancel' } | { kind: 'clear' } | { kind: 'confirm' }

/**
 * What a keydown means while a row is recording (03 §7D Editing): Esc
 * cancels, Backspace clears, a plain Enter confirms when something is
 * pending, a lone modifier waits, anything else is the new chord. A symbol
 * typed with Shift (`?`, `!`, `#`, `+`) is recorded as the symbol.
 */
export function recordKey(e: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>, pending: boolean): Recorded {
  const plain = !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey
  if (MODIFIER_KEYS.has(e.key)) return { kind: 'modifier' }
  if (plain && e.key === 'Escape') return { kind: 'cancel' }
  if (plain && e.key === 'Backspace') return { kind: 'clear' }
  if (plain && e.key === 'Enter' && pending) return { kind: 'confirm' }
  let key: string | undefined
  let shift = e.shiftKey
  if (/^Key[A-Z]$/.test(e.code)) key = e.code.slice(3)
  else if (/^Digit\d$/.test(e.code)) key = e.code.slice(5)
  else if (/^Numpad\d$/.test(e.code)) key = e.code.slice(6)
  else if (/^F\d{1,2}$/.test(e.code)) key = e.code
  else if (CODE_KEYS[e.code]) key = CODE_KEYS[e.code]
  // Shifted symbols are their own keys ("?" rather than Shift+/), letters and digits keep Shift.
  if (e.shiftKey && e.key.length === 1 && !/[a-z0-9]/i.test(e.key) && !e.ctrlKey && !e.metaKey && !e.altKey) {
    key = e.key
    shift = false
  }
  if (!key && e.key.length === 1) key = e.key.toUpperCase()
  if (!key) return { kind: 'modifier' }
  return { kind: 'chord', chord: { ctrl: e.ctrlKey, alt: e.altKey, shift, meta: e.metaKey, key } }
}
