// Theme packs (10 §3.1, §4, §5; 03 §8.5): the list for the gallery, the
// resolver's token map that both webviews apply on `:root`, import / export /
// delete. The webview never parses a pack; it applies the map it is given.
//
// Aligned with `mode_core::themes::api` (package 11). The `theme.changed`
// payload is `mode_core::themes::resolve::ResolvedTheme`; its token names are
// the stylesheet's (`--m-r-ctl`, `--m-r-card`, the 21st terminal token
// `--m-terminal-cursor-accent`). Fields marked "core extra" are sent by the
// core beyond what the views read (optional here).

export const THEME_EVENTS = {
  /** After a pack activation, an import, an OS appearance or accent change. */
  changed: 'theme.changed',
} as const

export type PackKind = 'builtin' | 'imported' | 'custom'

export type PackAppearance = 'light' | 'dark' | 'auto'

/** `theme.appearance`. */
export type AppearancePref = 'auto' | 'light' | 'dark'

/** Six swatches for a gallery card (bg, surface, fg, accent, ok, info). Beyond the list: the core derives them so the webview never reads a pack. */
export interface Swatch {
  bg: string
  surface: string
  fg: string
  accent: string
  ok: string
  info: string
}

export interface ThemeRow {
  id: string
  slug: string
  name: string
  kind: PackKind
  appearance: PackAppearance
  author?: string | null
  version?: string | null
  /** The active pack (`theme.active`). */
  active: boolean
  swatches: { light?: Swatch; dark?: Swatch }
  /** Core extras: a built-in, the file it was imported from, the last write, and for one-appearance packs the card line ("Gruvbox Dark (dark) · System Light by day"). */
  builtin?: boolean
  source_path?: string | null
  updated_at?: string
  card_line?: string | null
}

/** The resolver's answer and the `theme.changed` payload (the core sends every field; a map the view builds may carry only `tokens` and `dark`). */
export interface ResolvedTheme {
  /** `--m-*` → value. An empty map means the stylesheet's own tokens. */
  tokens: Record<string, string>
  /** `[data-theme=dark]` on the webviews. */
  dark: boolean
  /** The pack the colors came from (`system` when it fell back). */
  slug?: string
  name?: string
  /** The pack the user chose. */
  active?: string
  /** `light` | `dark`: the appearance rendered. */
  appearance?: 'light' | 'dark'
  /** "Gruvbox Light has no dark set — showing System Dark". */
  notices?: string[]
  /** The resolver changed the accent for contrast. */
  accent_adjusted?: boolean
}

/** One validation finding (10 §4.3) with its JSON path. */
export interface PackIssue {
  severity: 'error' | 'warning' | 'fill'
  path: string
  message: string
  /** Structure (format, keys, color syntax, tokens-only), a §11.1 contrast pair, a key this format ignores, a default supplied. */
  kind: 'structure' | 'contrast' | 'unknown_key' | 'fill'
  /** A contrast finding's measured ratio. */
  ratio?: number
  /** A contrast finding below the §11.1 floor, not only the target. */
  required?: boolean
}

/** The validator's answer for a stored pack. */
export interface PackReport {
  valid: boolean
  errors: PackIssue[]
  warnings: PackIssue[]
  fills: PackIssue[]
}

/** What an import did: validated only, imported, replaced, refused (errors), or a slug conflict to ask about. */
export type ImportOutcome = 'preview' | 'imported' | 'replaced' | 'invalid' | 'conflict'

/** `theme_import`: `apply: false` validates and previews; `apply: true` writes the `themes` row. */
export interface ThemeImportResult {
  ok: boolean
  saved: boolean
  slug: string | null
  name: string | null
  appearance: PackAppearance | null
  issues: PackIssue[]
  /** The slug exists: offer Replace or Keep both. */
  conflict: boolean
  /** The preview card's swatches. */
  swatches?: { light?: Swatch; dark?: Swatch } | null
  /** The saved row (when `saved`). */
  row?: ThemeRow | null
  outcome: ImportOutcome
  /** "Could not import x.json: 3 errors" / "{pack} has {n} contrast warnings; text may be hard to read.". */
  message?: string | null
  /** Core extras: the colliding row (conflict), both appearances resolved for a full preview. */
  existing?: ThemeRow | null
  preview?: { light?: Record<string, string> | null; dark?: Record<string, string> | null } | null
}

export interface ThemeDetail {
  row: ThemeRow
  /** The stored document, for Export and the token editor (not parsed by the webview). */
  document: unknown
  report: PackReport
}

export interface ThemesCommands {
  themes_list: { args: undefined; result: ThemeRow[] }
  theme_get: { args: { id: string }; result: ThemeDetail }
  /** `mode` sets `theme.appearance` in the same write. */
  theme_set_active: { args: { id: string; mode?: AppearancePref }; result: ResolvedTheme }
  /** `apply` and `on_conflict` are beyond the list (the preview-then-import flow of 10 §4.5); `dry_run` is the older spelling of `apply: false`. */
  theme_import: { args: { path?: string; text?: string; apply?: boolean; dry_run?: boolean; on_conflict?: 'replace' | 'keep_both' }; result: ThemeImportResult }
  /** `path`: the file the save dialog chose; default `<export dir>/<slug>.json`. */
  theme_export: { args: { id: string; path?: string }; result: { path: string } }
  /** User packs only; `active_reset`: it was active and `theme.active` went back to `system`. */
  theme_delete: { args: { id: string }; result: { deleted: boolean; active_reset: boolean } }
  /** `id` previews a pack (default the active one); `mode` overrides `theme.appearance`. */
  theme_resolve: { args: { id?: string; mode?: AppearancePref } | undefined; result: ResolvedTheme }
  /** Core extra: a `custom` copy of any pack (10 §4.5 Duplicate). */
  theme_duplicate: { args: { id: string; name?: string }; result: ThemeRow }
  /** Core extra: the token editor's Save on an imported or custom pack. */
  theme_update: { args: { id: string; document: unknown }; result: ThemeImportResult }
}
