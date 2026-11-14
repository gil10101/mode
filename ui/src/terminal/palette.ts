// The Terminal layer palette (03 §3.8, 10 §2, §4.2, §5.11): the theme pack's
// terminal tokens, delivered as `--m-terminal-<name>` (`terminal.ansi.red` →
// `--m-terminal-ansi-red`). `mode.css` does not define them yet, so each
// entry falls back to the closest of the fifteen `--m-*` roles, per
// appearance where the System pack differs (ANSI black, white, bright white).
// The fallbacks are a stopgap: yellow, magenta and cyan have no role of their
// own and share one with another color until the terminal tokens ship.
import type { ITheme } from '@xterm/xterm'

export type Appearance = 'light' | 'dark'

type Fallback = string | { light: string; dark: string }

/** xterm theme key → [pack token, fallback role]. */
export const PALETTE_MAP: Record<string, [string, Fallback]> = {
  foreground: ['--m-terminal-fg', '--m-fg'],
  background: ['--m-terminal-bg', '--m-surface'],
  cursor: ['--m-terminal-cursor', '--m-accent'],
  cursorAccent: ['--m-terminal-bg', '--m-surface'],
  selectionBackground: ['--m-terminal-selection', '--m-selected'],
  black: ['--m-terminal-ansi-black', { light: '--m-fg', dark: '--m-bg' }],
  red: ['--m-terminal-ansi-red', '--m-bad'],
  green: ['--m-terminal-ansi-green', '--m-ok'],
  yellow: ['--m-terminal-ansi-yellow', '--m-accent'],
  blue: ['--m-terminal-ansi-blue', '--m-info'],
  magenta: ['--m-terminal-ansi-magenta', '--m-bad'],
  cyan: ['--m-terminal-ansi-cyan', '--m-info'],
  white: ['--m-terminal-ansi-white', { light: '--m-line-strong', dark: '--m-fg-2' }],
  brightBlack: ['--m-terminal-ansi-bright-black', '--m-fg-3'],
  brightRed: ['--m-terminal-ansi-bright-red', '--m-bad'],
  brightGreen: ['--m-terminal-ansi-bright-green', '--m-ok'],
  brightYellow: ['--m-terminal-ansi-bright-yellow', '--m-accent'],
  brightBlue: ['--m-terminal-ansi-bright-blue', '--m-info'],
  brightMagenta: ['--m-terminal-ansi-bright-magenta', '--m-bad'],
  brightCyan: ['--m-terminal-ansi-bright-cyan', '--m-info'],
  brightWhite: ['--m-terminal-ansi-bright-white', { light: '--m-surface', dark: '--m-fg' }],
}

export interface ResolvedPalette {
  theme: ITheme
  appearance: Appearance
  /** Pack tokens that were not set (their fallback role was used). */
  fallbacks: string[]
}

/** Reads a custom property (`getComputedStyle(el).getPropertyValue`). */
export type TokenReader = (name: string) => string

export function resolvePalette(read: TokenReader, appearance: Appearance): ResolvedPalette {
  const theme: Record<string, string> = {}
  const fallbacks: string[] = []
  for (const [key, [token, fallback]] of Object.entries(PALETTE_MAP)) {
    let value = read(token).trim()
    if (!value) {
      if (!fallbacks.includes(token)) fallbacks.push(token)
      const role = typeof fallback === 'string' ? fallback : fallback[appearance]
      value = read(role).trim()
    }
    if (value) theme[key] = value
  }
  // The scrollbar slider follows the foreground (xterm's default) and the inactive selection keeps the selection color.
  if (theme.selectionBackground) theme.selectionInactiveBackground = theme.selectionBackground
  return { theme: theme as ITheme, appearance, fallbacks }
}

/** The appearance a subtree renders in (`color-scheme` set by `mode.css` per `data-theme`). */
export function appearanceOf(style: Pick<CSSStyleDeclaration, 'colorScheme'> | null | undefined): Appearance {
  return style?.colorScheme?.includes('dark') ? 'dark' : 'light'
}

/** Find decorations need `#RRGGBB`; null when the palette is in another syntax. */
export function searchDecorations(theme: ITheme) {
  const hex = (v: string | undefined) => (v && /^#[0-9a-f]{6}$/i.test(v) ? v : null)
  const match = hex(theme.selectionBackground)
  const active = hex(theme.cursor)
  const ruler = hex(theme.brightBlack)
  if (!match || !active || !ruler) return null
  return { matchBackground: match, activeMatchBackground: match, activeMatchBorder: active, matchOverviewRuler: ruler, activeMatchColorOverviewRuler: active }
}
