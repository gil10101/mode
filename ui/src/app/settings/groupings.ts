// The groupings editor's filter builder (03 §8.8): rows of key → operator →
// value that round-trip with `filter_json` (04 §6.1), and the custom-status
// list edits of `notes.statuses`. Pure functions, unit-tested.
import type { FilterJson, SourceKind, StatusDef, WhenBucket } from '../../state/contract-notes'

export type FilterKey = 'status' | 'urgent' | 'when' | 'tags' | 'folder' | 'text' | 'person' | 'repo' | 'pinned' | 'archived' | 'source_kind'

export interface FilterRow {
  key: FilterKey
  op: string
  /** Comma-separated for lists; `true` / `false` for flags; a date range as `from..to`. */
  value: string
}

export const FILTER_KEYS: readonly FilterKey[] = ['status', 'urgent', 'when', 'tags', 'folder', 'text', 'person', 'repo', 'pinned', 'archived', 'source_kind']

/** Operators per key, in the builder's picker order. */
export const OPS: Record<FilterKey, readonly string[]> = {
  status: ['is any of', 'is none of'],
  urgent: ['is'],
  when: ['is in', 'between'],
  tags: ['has all of', 'has any of', 'has none of'],
  folder: ['is in', 'is directly in'],
  text: ['contains'],
  person: ['is'],
  repo: ['is'],
  pinned: ['is'],
  archived: ['is'],
  source_kind: ['is any of'],
}

export const WHEN_BUCKETS: readonly WhenBucket[] = ['overdue', 'today', 'tomorrow', 'scheduled', 'none']

const list = (v: string) =>
  v
    .split(',')
    .map((x) => x.trim())
    .filter(Boolean)

const flag = (v: string) => v.trim().toLowerCase() !== 'false'

/** filter_json → builder rows (unknown keys are dropped). */
export function filterToRows(f: FilterJson | null | undefined): FilterRow[] {
  const rows: FilterRow[] = []
  if (!f) return rows
  if (f.status?.in?.length) rows.push({ key: 'status', op: 'is any of', value: f.status.in.join(', ') })
  if (f.status?.not_in?.length) rows.push({ key: 'status', op: 'is none of', value: f.status.not_in.join(', ') })
  if (f.urgent !== undefined) rows.push({ key: 'urgent', op: 'is', value: String(f.urgent) })
  if (f.when) {
    if ('in' in f.when) rows.push({ key: 'when', op: 'is in', value: f.when.in.join(', ') })
    else rows.push({ key: 'when', op: 'between', value: `${f.when.from ?? ''}..${f.when.to ?? ''}` })
  }
  if (f.tags?.all?.length) rows.push({ key: 'tags', op: 'has all of', value: f.tags.all.join(', ') })
  if (f.tags?.any?.length) rows.push({ key: 'tags', op: 'has any of', value: f.tags.any.join(', ') })
  if (f.tags?.none?.length) rows.push({ key: 'tags', op: 'has none of', value: f.tags.none.join(', ') })
  if (f.folder) rows.push({ key: 'folder', op: f.folder.recursive === false ? 'is directly in' : 'is in', value: f.folder.id })
  if (f.text) rows.push({ key: 'text', op: 'contains', value: f.text })
  if (f.person) rows.push({ key: 'person', op: 'is', value: f.person.contact_id })
  if (f.repo) rows.push({ key: 'repo', op: 'is', value: f.repo.repo_root })
  if (f.pinned) rows.push({ key: 'pinned', op: 'is', value: 'true' })
  if (f.archived !== undefined) rows.push({ key: 'archived', op: 'is', value: String(f.archived) })
  if (f.source_kind?.in?.length) rows.push({ key: 'source_kind', op: 'is any of', value: f.source_kind.in.join(', ') })
  return rows
}

/** Builder rows → filter_json; rows with an empty value are skipped. */
export function rowsToFilter(rows: readonly FilterRow[]): FilterJson {
  const f: FilterJson = {}
  for (const r of rows) {
    const v = r.value.trim()
    if (!v) continue
    switch (r.key) {
      case 'status':
        f.status = { ...(f.status ?? {}), [r.op === 'is none of' ? 'not_in' : 'in']: list(v) }
        break
      case 'urgent':
        f.urgent = flag(v)
        break
      case 'when':
        if (r.op === 'between') {
          const [from, to] = v.split('..').map((x) => x.trim())
          f.when = { ...(from ? { from } : {}), ...(to ? { to } : {}) }
        } else f.when = { in: list(v).filter((b): b is WhenBucket => (WHEN_BUCKETS as readonly string[]).includes(b)) }
        break
      case 'tags':
        f.tags = { ...(f.tags ?? {}), [r.op === 'has all of' ? 'all' : r.op === 'has none of' ? 'none' : 'any']: list(v) }
        break
      case 'folder':
        f.folder = r.op === 'is directly in' ? { id: v, recursive: false } : { id: v }
        break
      case 'text':
        f.text = v
        break
      case 'person':
        f.person = { contact_id: v }
        break
      case 'repo':
        f.repo = { repo_root: v }
        break
      case 'pinned':
        if (flag(v)) f.pinned = true
        break
      case 'archived':
        f.archived = flag(v)
        break
      case 'source_kind':
        f.source_kind = { in: list(v) as SourceKind[] }
        break
    }
  }
  return f
}

/** Why the rows cannot be saved, or null. */
export function rowsProblem(rows: readonly FilterRow[], statuses: readonly StatusDef[]): string | null {
  for (const r of rows) {
    if (!r.value.trim()) continue
    if (r.key === 'status') {
      const known = new Set(['closed', ...statuses.map((s) => s.slug)])
      const bad = list(r.value).find((s) => !known.has(s))
      if (bad) return `'${bad}' is not a status yet.`
    }
    if (r.key === 'when' && r.op === 'is in') {
      const bad = list(r.value).find((b) => !(WHEN_BUCKETS as readonly string[]).includes(b))
      if (bad) return `'${bad}' is not one of ${WHEN_BUCKETS.join(', ')}.`
    }
  }
  return null
}

/* ---- custom statuses (`notes.statuses`) ------------------------------------- */

export const FIXED_STATUSES: readonly string[] = ['todo', 'doing', 'done']

export function slugify(label: string): string {
  return label
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 32)
}

export function addStatus(list: readonly StatusDef[], label: string, glyph = '○'): StatusDef[] {
  let slug = slugify(label) || 'status'
  let n = 2
  while (list.some((s) => s.slug === slug)) slug = `${slugify(label) || 'status'}-${n++}`
  // New statuses go before the closed ones (`done`).
  const at = list.findIndex((s) => s.closed)
  const next: StatusDef = { slug, label: label.trim(), glyph, color: null, closed: false }
  return at < 0 ? [...list, next] : [...list.slice(0, at), next, ...list.slice(at)]
}

export function moveStatus(list: readonly StatusDef[], slug: string, by: -1 | 1): StatusDef[] {
  const i = list.findIndex((s) => s.slug === slug)
  const j = i + by
  if (i < 0 || j < 0 || j >= list.length) return [...list]
  const out = [...list]
  ;[out[i], out[j]] = [out[j]!, out[i]!]
  return out
}

export function removeStatus(list: readonly StatusDef[], slug: string): StatusDef[] {
  if (FIXED_STATUSES.includes(slug)) return [...list]
  return list.filter((s) => s.slug !== slug)
}

export function patchStatus(list: readonly StatusDef[], slug: string, patch: Partial<Pick<StatusDef, 'label' | 'glyph'>>): StatusDef[] {
  return list.map((s) => (s.slug === slug ? { ...s, ...patch } : s))
}

/** Move `id` one place up or down in an id order. */
export function moveId(ids: readonly string[], id: string, by: -1 | 1): string[] {
  const i = ids.indexOf(id)
  const j = i + by
  if (i < 0 || j < 0 || j >= ids.length) return [...ids]
  const out = [...ids]
  ;[out[i], out[j]] = [out[j]!, out[i]!]
  return out
}
