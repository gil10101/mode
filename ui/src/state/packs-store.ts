// The Marketplace store (03 §7C.6): the catalog (`catalogue_get`), the import
// preview and its install (`pack_import`), Export pack (`pack_export`). The
// preview keeps the source it was read from (text, a file name, a catalog
// URL) so Install sends the same pack back with the user's choices: the
// items switched off, the conflict rule, the folders for session
// automations whose folder is not on this computer.
import { create } from 'zustand'
import { invoke } from '../tauri'
import { cmdErrorText } from './contract'
import type { CatalogueEntry, CatalogueResult, FolderOverride, ItemKind, ItemPreview, PackConflict, PackExportArgs, PackExportResult, PackImportResult } from './contract-packs'

export interface PackPreview extends PackImportResult {
  /** What was sent: the pack text (a file or a paste) or the catalog URL. */
  text?: string
  url?: string
  /** Which items are switched on (every installable one at first). */
  on: Record<string, boolean>
  conflict: PackConflict
  /** `key` → folder typed for a session automation. */
  folders: Record<string, string>
}

export interface PacksState {
  catalogue: CatalogueResult | null
  loading: boolean
  loaded: boolean
  preview: PackPreview | null
  installing: boolean
  /** The last install's outcome (the dialog's closing line). */
  result: PackImportResult | null
  error: string | null
  load(refresh?: boolean): Promise<CatalogueResult | null>
  /** A file's or a paste's text (`apply: false`). */
  previewText(text: string, file?: string): Promise<PackImportResult | null>
  /** A catalog entry (`apply: false`; the core fetches and keeps it). */
  previewUrl(url: string): Promise<PackImportResult | null>
  toggle(item: ItemPreview, on: boolean): void
  setConflict(c: PackConflict): void
  setFolder(key: string, cwd: string): void
  /** Writes the previewed pack with the choices. */
  install(): Promise<PackImportResult | null>
  cancel(): void
  exportPack(args: PackExportArgs): Promise<PackExportResult | null>
}

/** `<kind>:<key>`, the `skip` spelling. */
export function itemTag(i: Pick<ItemPreview, 'kind' | 'key'>): string {
  return `${i.kind}:${i.key}`
}

export function installable(i: ItemPreview): boolean {
  return i.issues.length === 0
}

/** The dialog can install: something is on, and every on item that needs a folder has one. */
export function canInstall(p: PackPreview): boolean {
  const on = p.items.filter((i) => p.on[itemTag(i)] && installable(i))
  if (!on.length) return false
  return on.every((i) => !i.needs_folder || (p.folders[i.key] ?? '').trim() !== '')
}

function withChoices(r: PackImportResult, source: { text?: string; url?: string }): PackPreview {
  const on: Record<string, boolean> = {}
  for (const i of r.items) on[itemTag(i)] = installable(i)
  return { ...r, ...source, on, conflict: 'keep_both', folders: {} }
}

/** "by Ana · v2 · 2 agents · 1 automation" (the preview's head line). */
export function packLine(p: Pick<PackImportResult, 'author' | 'version' | 'counts'>): string {
  const parts: string[] = []
  if (p.author) parts.push(`by ${p.author}`)
  if (p.version) parts.push(`v${p.version}`)
  if (p.counts) parts.push(p.counts)
  return parts.join(' · ')
}

/** "by Mode · v1 · 1 agent · 1 automation" (a catalog row's detail). */
export function entryLine(e: CatalogueEntry): string {
  return packLine({ author: e.author, version: e.version, counts: e.counts })
}

/** "by Mode · v1" (a catalog card's meta; the counts are its chips). */
export function entryMeta(e: CatalogueEntry): string {
  return packLine({ author: e.author, version: e.version, counts: '' })
}

/** The words of the item's second line: what Replace / Keep both means for it. */
export function itemNote(i: ItemPreview, conflict: PackConflict): string {
  if (i.issues.length) return i.issues[0]!.message
  if (i.needs_folder) return `its folder ${i.needs_folder} is not on this computer`
  if (i.existing) return conflict === 'replace' ? `replaces ${i.existing}` : `kept beside ${i.existing}`
  if (i.kind === 'automation') return 'installed switched off'
  return 'new'
}

export const KIND_LABEL: Record<ItemKind, string> = { agent: 'Agents', automation: 'Automations', theme: 'Themes' }

export const usePacks = create<PacksState>((set, get) => ({
  catalogue: null,
  loading: false,
  loaded: false,
  preview: null,
  installing: false,
  result: null,
  error: null,
  async load(refresh) {
    set({ loading: true })
    try {
      const r = await invoke<CatalogueResult>('catalogue_get', refresh ? { refresh: true } : undefined)
      // A failed refresh keeps the held catalog browsable beside the error (03 §7C.6 States).
      const held = get().catalogue?.catalogue ?? null
      set({ catalogue: r.catalogue || !held ? r : { ...r, catalogue: held }, loading: false, loaded: true, error: null })
      return r
    } catch (e) {
      set({ loading: false, loaded: true, error: cmdErrorText(e) })
      return null
    }
  },
  async previewText(text, file) {
    try {
      const r = await invoke<PackImportResult>('pack_import', file ? { text, path: file } : { text })
      set({ preview: withChoices(r, { text }), result: null, error: null })
      return r
    } catch (e) {
      set({ error: cmdErrorText(e), preview: null })
      return null
    }
  },
  async previewUrl(url) {
    try {
      const r = await invoke<PackImportResult>('pack_import', { url })
      set({ preview: withChoices(r, { url }), result: null, error: null })
      return r
    } catch (e) {
      set({ error: cmdErrorText(e), preview: null })
      return null
    }
  },
  toggle(item, on) {
    const p = get().preview
    if (!p) return
    set({ preview: { ...p, on: { ...p.on, [itemTag(item)]: on } } })
  },
  setConflict(conflict) {
    const p = get().preview
    if (p) set({ preview: { ...p, conflict } })
  },
  setFolder(key, cwd) {
    const p = get().preview
    if (p) set({ preview: { ...p, folders: { ...p.folders, [key]: cwd } } })
  },
  async install() {
    const p = get().preview
    if (!p || !canInstall(p)) return null
    const skip = p.items.filter((i) => !p.on[itemTag(i)]).map(itemTag)
    const folders: FolderOverride[] = Object.entries(p.folders)
      .filter(([, cwd]) => cwd.trim())
      .map(([key, cwd]) => ({ key, cwd: cwd.trim() }))
    set({ installing: true })
    try {
      const r = await invoke<PackImportResult>('pack_import', {
        ...(p.url ? { url: p.url } : { text: p.text }),
        apply: true,
        on_conflict: p.conflict,
        skip,
        folders,
      })
      set({ installing: false, result: r, preview: null, error: null })
      return r
    } catch (e) {
      set({ installing: false, error: cmdErrorText(e) })
      return null
    }
  },
  cancel() {
    set({ preview: null, result: null, error: null })
  },
  async exportPack(args) {
    try {
      const r = await invoke<PackExportResult>('pack_export', args as unknown as Record<string, unknown>)
      set({ error: null })
      return r
    } catch (e) {
      set({ error: cmdErrorText(e) })
      return null
    }
  },
}))
