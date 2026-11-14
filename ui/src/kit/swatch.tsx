// A theme's colors as card art (10 §4.5; 03 §7C.6): the Appearance gallery's
// card draws both pieces and the Marketplace's theme pack card the dots: a
// window in the pack's bg and surface with its accent dot and fg bars, and a
// row of one dot per role. The colors travel as custom properties on the
// element, so the stylesheet names no color of its own.
import type { CSSProperties } from 'react'
import { cx } from './cx'

/** The six roles a gallery swatch shows, as the theme packs' `swatches` and the catalog's `colors` carry them. */
export interface SwatchColors {
  bg: string
  surface: string
  fg: string
  accent: string
  ok: string
  info: string
}

export const SWATCH_ROLES: readonly (keyof SwatchColors)[] = ['bg', 'surface', 'fg', 'accent', 'ok', 'info']

/** The six custom properties for a swatch; `undefined` leaves the tokens of the page. */
export function swatchStyle(s: SwatchColors | null | undefined): CSSProperties | undefined {
  if (!s) return undefined
  return { '--m-bg': s.bg, '--m-surface': s.surface, '--m-fg': s.fg, '--m-accent': s.accent, '--m-ok': s.ok, '--m-info': s.info } as CSSProperties
}

/** A catalog entry's `colors` (bg, surface, fg, accent, ok, info) as a swatch; fewer than six = none. */
export function swatchOfList(colors: readonly string[] | null | undefined): SwatchColors | null {
  if (!colors || colors.length < 6) return null
  const [bg, surface, fg, accent, ok, info] = colors as [string, string, string, string, string, string]
  return { bg, surface, fg, accent, ok, info }
}

export interface ThemeSwatchProps {
  swatch?: SwatchColors | null
  /** Names the picture for a reader; without it the swatch is decoration. */
  label?: string
  className?: string
}

/** `.m-swatch`: the window in the pack's colors. */
export function ThemeSwatch({ swatch, label, className }: ThemeSwatchProps) {
  return (
    <div className={cx('m-swatch', className)} style={swatchStyle(swatch)} role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <span>
        <i />
        <b />
        <b />
      </span>
    </div>
  )
}

/** `.m-swatch-dots`: one 14-px dot per role, in the roles' order. */
export function SwatchDots({ swatch, className }: { swatch?: SwatchColors | null; className?: string }) {
  return (
    <div className={cx('m-swatch-dots', className)} style={swatchStyle(swatch)} aria-hidden="true">
      {SWATCH_ROLES.map((r) => (
        <i key={r} className={`is-${r}`} />
      ))}
    </div>
  )
}
