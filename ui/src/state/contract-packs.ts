// The Marketplace contract (03 §7C.6; ADR-0060): packs of agents, automations
// and theme packs as `mode_core::packs` carries them. A pack is one JSON file
// (`kind: "mode-pack"`); Export writes one from rows, Import previews one
// before anything is written, Install writes through the owning stores
// (`agent_save`, `automation_save` disabled, `theme_import`); the catalog
// is a read-only index fetched from `marketplace.index_url`.
import type { FieldError } from './contract-automations'
import type { Swatch } from './contract-themes'

export type ItemKind = 'agent' | 'automation' | 'theme'

/** One item of the preview: what it is, what Replace would overwrite, what stops it. */
export interface ItemPreview {
  kind: ItemKind
  /** The pack key (themes: the slug); `skip` entries are `<kind>:<key>`. */
  key: string
  name: string
  /** "Reviews pull requests · claude-opus-5-5 · high · 1 skill" / "Weekdays 23:00 · Start session in mode · …" / "dark · by Ana". */
  detail: string
  /** The row with the same name (agents, automations) or slug (themes). */
  existing?: string | null
  /** Field errors that stop the item. */
  issues: FieldError[]
  /** `start_session`: the folder the pack names is not here; Install needs one (`folders`). */
  needs_folder?: string | null
  /** Theme contrast warnings, shown, not blocking. */
  warnings?: string[]
  /** An agent's accessory (the bot mark the row draws). */
  avatar_variant?: string | null
  /** A theme's gallery swatches (the color cube the row draws). */
  swatches?: { light?: Swatch; dark?: Swatch } | null
}

export type PackOutcome = 'preview' | 'installed' | 'partial' | 'invalid'
export type ItemOutcome = 'created' | 'replaced' | 'skipped' | 'invalid' | 'failed'
export type PackConflict = 'replace' | 'keep_both'

export interface InstalledItem {
  kind: ItemKind
  key: string
  name: string
  outcome: ItemOutcome
  id?: string | null
  message?: string | null
}

export interface PackImportResult {
  /** The document is a pack (its items may still carry issues). */
  ok: boolean
  outcome: PackOutcome
  slug: string | null
  name: string | null
  version: string | null
  author: string | null
  description: string | null
  /** "reviewer.mode-pack.json" / the catalog URL / "pasted text". */
  source: string
  /** "2 agents · 1 automation · 1 theme". */
  counts: string
  items: ItemPreview[]
  /** `invalid`: why; after an install: "Installed 3 items." */
  message?: string | null
  installed: InstalledItem[]
}

export interface PackExportResult {
  path: string
  slug: string
  name: string
  counts: string
}

export interface FolderOverride {
  key: string
  cwd: string
}

export interface CatalogueEntry {
  slug: string
  name: string
  description: string | null
  author: string | null
  version: string | null
  /** Absolute, on the index's host. */
  url: string
  agents: number
  automations: number
  themes: number
  /** "2 agents · 1 automation". */
  counts: string
  tags: string[]
  /** A theme pack's colors (hex: bg, surface, fg, accent, ok, info), when the index gives them. */
  colors: string[]
  /** Its agents' accessories (`avatar_variant`), when the index gives them: the bot marks on the card. */
  avatars: string[]
}

export interface Catalogue {
  url: string
  host: string
  name: string | null
  fetched_at: string
  packs: CatalogueEntry[]
}

export interface CatalogueResult {
  /** `marketplace.index_url` as used. */
  url: string
  host: string
  catalogue?: Catalogue | null
  /** Why there is none, or why the refresh failed beside the held one. */
  error?: string | null
}

export interface PackExportArgs {
  agents?: string[]
  automations?: string[]
  themes?: string[]
  name?: string
  slug?: string
  author?: string
  description?: string
  path?: string
}

export interface PackImportArgs {
  text?: string
  path?: string
  url?: string
  apply?: boolean
  on_conflict?: PackConflict
  skip?: string[]
  folders?: FolderOverride[]
}

export interface PacksCommands {
  /** Export pack on a row; Create pack… on the Marketplace page. Default file `<Downloads>/<slug>.mode-pack.json`. */
  pack_export: { args: PackExportArgs; result: PackExportResult }
  /** `apply` absent: a preview; `apply: true`: the install, every item not in `skip`. */
  pack_import: { args: PackImportArgs; result: PackImportResult }
  /** Fetched once per run; `refresh` fetches again. */
  catalogue_get: { args: { refresh?: boolean } | undefined; result: CatalogueResult }
}
