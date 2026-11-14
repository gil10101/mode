// Theme packs in the webviews (10 §3.1, §4.4, §4.5; 03 §8.5). The core
// resolves the active pack, `theme.appearance` and the OS into one flat
// `--m-*` map and pushes it as `theme.changed`; this store applies the map
// as custom properties on `:root` (and `data-theme`), asks `theme_resolve`
// at start, and keeps the list the Appearance gallery draws. The webview
// never parses a pack. Both the app window (`connectTheme`) and the overlay
// (`connectThemeTokens`) apply the same map.
import { create } from 'zustand'
import { invoke, listen, type Unlisten } from '../tauri'
import { EVENTS } from './contract'
import { THEME_EVENTS, type AppearancePref, type ResolvedTheme, type ThemeImportResult, type ThemeRow } from './contract-themes'

/**
 * The long radius names (the pack format's `radius.control` / `radius.card`)
 * that `mode.css` reads as `--m-r-ctl` / `--m-r-card`. The core's resolver
 * sends the stylesheet's names (`RADIUS_CONTROL`, `RADIUS_CARD`); a map that
 * carries the long ones gets both set.
 */
export const TOKEN_ALIASES: Readonly<Record<string, string>> = {
  '--m-radius-control': '--m-r-ctl',
  '--m-radius-card': '--m-r-card',
}

/** Packs are tokens only (10 §4.3): a value that could load or run something is dropped here too. */
export function safeTokenValue(v: unknown): v is string {
  return typeof v === 'string' && v.length < 512 && !/url\(|expression\(|[<>{};]|javascript:/i.test(v)
}

const appliedKeys = new WeakMap<HTMLElement, Set<string>>()

/**
 * Applies a resolved map on `root` (`:root` by default): every `--m-*`
 * token as a custom property, the aliases, and `data-theme` from `dark`.
 * Keys a previous map set and this one lacks are removed, so an empty map
 * returns to the stylesheet's own tokens. Returns the keys now set.
 */
export function applyTokens(
  tokens: Record<string, string>,
  dark: boolean,
  root: HTMLElement | null = typeof document === 'undefined' ? null : document.documentElement,
  opts: { dataTheme?: boolean } = {},
): string[] {
  if (!root) return []
  const next = new Map<string, string>()
  for (const [k, v] of Object.entries(tokens ?? {})) {
    if (!k.startsWith('--m-') || !safeTokenValue(v)) continue
    next.set(k, v)
    const alias = TOKEN_ALIASES[k]
    if (alias && !(alias in tokens)) next.set(alias, v)
  }
  const prev = appliedKeys.get(root) ?? new Set<string>()
  for (const k of prev) if (!next.has(k)) root.style.removeProperty(k)
  for (const [k, v] of next) root.style.setProperty(k, v)
  appliedKeys.set(root, new Set(next.keys()))
  if (opts.dataTheme !== false) {
    if (dark) root.dataset.theme = 'dark'
    else delete root.dataset.theme
  }
  return Array.from(next.keys())
}

export interface ThemeState {
  /** The last map applied (`theme_resolve` or `theme.changed`). */
  resolved: ResolvedTheme | null
  themes: ThemeRow[]
  loaded: boolean
  error: string | null
  /** The import preview (validated, not saved yet). */
  preview: (ThemeImportResult & { text: string; file: string }) | null
  apply(r: ResolvedTheme): void
  resolve(id?: string): Promise<ResolvedTheme | null>
  load(): Promise<void>
  setActive(id: string, mode?: AppearancePref): Promise<boolean>
  /** Validates and previews a pack file's text (`apply: false`). */
  previewImport(text: string, file: string): Promise<ThemeImportResult | null>
  /** Writes the previewed pack. */
  confirmImport(onConflict?: 'replace' | 'keep_both'): Promise<ThemeImportResult | null>
  cancelImport(): void
  exportPack(id: string): Promise<string | null>
  remove(id: string): Promise<boolean>
  /** `theme_duplicate`: a `custom` copy of any pack (10 §4.5 Duplicate), added to the gallery. */
  duplicate(id: string, name?: string): Promise<ThemeRow | null>
  /** `theme_update`: a changed document for an imported or custom pack; the outcome (`invalid` keeps the pack as it was). */
  update(id: string, document: unknown): Promise<ThemeImportResult | null>
}

function message(e: unknown): string {
  if (e && typeof e === 'object' && 'message' in e) return String((e as { message: unknown }).message)
  return String(e)
}

export const useThemes = create<ThemeState>((set, get) => ({
  resolved: null,
  themes: [],
  loaded: false,
  error: null,
  preview: null,
  apply(r) {
    if (!r || typeof r !== 'object') return
    applyTokens(r.tokens ?? {}, !!r.dark)
    set({ resolved: r })
    // The gallery's selection follows the pack the core says is active.
    if (r.active) set((s) => ({ themes: s.themes.map((t) => ({ ...t, active: t.slug === r.active || t.id === r.active })) }))
  },
  async resolve(id) {
    try {
      const r = await invoke<ResolvedTheme>('theme_resolve', id ? { id } : undefined)
      get().apply(r)
      return r
    } catch {
      return null
    }
  },
  async load() {
    try {
      const themes = await invoke<ThemeRow[]>('themes_list')
      set({ themes, loaded: true, error: null })
    } catch (e) {
      set({ loaded: true, error: message(e) })
    }
  },
  async setActive(id, mode) {
    const prev = get().themes
    set({ themes: prev.map((t) => ({ ...t, active: t.id === id })), error: null })
    try {
      const r = await invoke<ResolvedTheme>('theme_set_active', mode ? { id, mode } : { id })
      if (r) get().apply(r)
      return true
    } catch (e) {
      set({ themes: prev, error: message(e) })
      return false
    }
  },
  async previewImport(text, file) {
    try {
      const r = await invoke<ThemeImportResult>('theme_import', { text, apply: false })
      set({ preview: { ...r, text, file }, error: null })
      return r
    } catch (e) {
      set({ error: message(e), preview: null })
      return null
    }
  },
  async confirmImport(onConflict) {
    const p = get().preview
    if (!p) return null
    try {
      const r = await invoke<ThemeImportResult>('theme_import', onConflict ? { text: p.text, apply: true, on_conflict: onConflict } : { text: p.text, apply: true })
      if (r.saved) {
        set({ preview: null })
        await get().load()
      } else set({ preview: { ...p, ...r } })
      return r
    } catch (e) {
      set({ error: message(e) })
      return null
    }
  },
  cancelImport() {
    set({ preview: null })
  },
  async exportPack(id) {
    try {
      const r = await invoke<{ path: string }>('theme_export', { id })
      return r?.path ?? null
    } catch (e) {
      set({ error: message(e) })
      return null
    }
  },
  async remove(id) {
    try {
      await invoke('theme_delete', { id })
      await get().load()
      return true
    } catch (e) {
      set({ error: message(e) })
      return false
    }
  },
  async duplicate(id, name) {
    try {
      const row = await invoke<ThemeRow>('theme_duplicate', name ? { id, name } : { id })
      if (row && typeof row === 'object' && row.id) set((s) => ({ themes: [...s.themes.filter((t) => t.id !== row.id), row], error: null }))
      return row ?? null
    } catch (e) {
      set({ error: message(e) })
      return null
    }
  },
  async update(id, document) {
    try {
      const r = await invoke<ThemeImportResult>('theme_update', { id, document })
      const row = r?.saved ? r.row : null
      if (row) set((s) => ({ themes: s.themes.map((t) => (t.id === row.id ? row : t)), error: null }))
      else if (r?.message) set({ error: r.message })
      return r ?? null
    } catch (e) {
      set({ error: message(e) })
      return null
    }
  },
}))

function systemDarkQuery(): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' ? window.matchMedia('(prefers-color-scheme: dark)') : null
}

const THEME_KEYS = ['theme.active', 'theme.appearance', 'theme.font_scale', 'theme.terminal_pack']

/**
 * App window: applies `theme.changed`, resolves at start, and asks again when
 * a `theme.*` setting or the OS appearance changes (the core pushes too; a
 * second apply of the same map is a no-op).
 */
export function connectTheme(): Promise<Unlisten> {
  const offs: Promise<Unlisten>[] = [
    listen<ResolvedTheme>(THEME_EVENTS.changed, (r) => useThemes.getState().apply(r)),
    listen<Record<string, unknown>>(EVENTS.settings, (patch) => {
      if (patch && typeof patch === 'object' && THEME_KEYS.some((k) => k in patch)) void useThemes.getState().resolve()
    }),
  ]
  const mq = systemDarkQuery()
  const onOs = () => void useThemes.getState().resolve()
  mq?.addEventListener?.('change', onOs)
  void useThemes.getState().resolve()
  return Promise.resolve(() => {
    mq?.removeEventListener?.('change', onOs)
    for (const off of offs) void off.then((u) => u())
  })
}

/**
 * The overlay: the same map on its own `:root`. `data-theme` stays with the
 * overlay's `display.changed` handling unless `dataTheme` is set.
 */
export function connectThemeTokens(opts: { dataTheme?: boolean } = {}): Promise<Unlisten> {
  const on = (r: ResolvedTheme | null) => {
    if (r && typeof r === 'object') applyTokens(r.tokens ?? {}, !!r.dark, undefined, { dataTheme: opts.dataTheme ?? false })
  }
  const off = listen<ResolvedTheme>(THEME_EVENTS.changed, on)
  invoke<ResolvedTheme>('theme_resolve').then(on, () => {})
  return off
}

/* ---- gallery helpers ----------------------------------------------------------------------- */

/** The card's meta line ("light · dark", "dark only · System Light by day"). */
export function packMeta(t: Pick<ThemeRow, 'slug' | 'appearance' | 'kind'>): string {
  const parts: string[] = []
  if (t.appearance === 'auto') parts.push('light · dark')
  else if (t.appearance === 'dark') parts.push('dark only · System Light by day')
  else parts.push('light only · System Dark at night')
  if (t.slug === 'system') parts.push('OS accent')
  if (t.kind === 'imported') parts.push('imported')
  if (t.kind === 'custom') parts.push('custom')
  return parts.join(' · ')
}

/** Built-ins first in 03 §8.5 order, then imported and custom packs by name. */
export const BUILTIN_ORDER = ['system', 'paper', 'gruvbox-dark', 'gruvbox-light', 'cursor-dark', 'nord', 'catppuccin-mocha']

export function sortPacks(rows: ThemeRow[]): ThemeRow[] {
  const rank = (t: ThemeRow) => {
    const i = BUILTIN_ORDER.indexOf(t.slug)
    return t.kind === 'builtin' && i >= 0 ? i : BUILTIN_ORDER.length
  }
  return [...rows].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name))
}
