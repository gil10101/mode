// Pure rules of the Notes view: glyphs, the status cycle, the board's
// one-edit drop rule (04 §6.5) and its reuse for ⊕ Add task, the capture
// tokens that carry an Add task's fields (04 §7.2), and chip helpers.
import type {
  FilterJson,
  FolderRow,
  NoteFieldEdit,
  NoteListRow,
  QueryChip,
  StatusDef,
  WeekStart,
  WhenBucket,
} from '../state/contract-notes'
import { DEFAULT_STATUSES } from '../state/contract-notes'
import { resolveDateToken } from './dates'

/* ---- statuses and glyphs ------------------------------------------------- */

export const URGENT_GLYPH = '✱'
export const UNKNOWN_GLYPH = '?'

/** Identity of a tag (04 §5.4): lowercase, trimmed, inner whitespace collapsed, leading `#` stripped. */
export function normTag(name: string): string {
  return name.trim().replace(/^#/, '').replace(/\s+/g, ' ').toLowerCase()
}

export function statusDef(slug: string, statuses: readonly StatusDef[] = DEFAULT_STATUSES): StatusDef | undefined {
  return statuses.find((s) => s.slug === slug)
}

export function closedSlugs(statuses: readonly StatusDef[] = DEFAULT_STATUSES): string[] {
  return statuses.filter((s) => s.closed).map((s) => s.slug)
}

export function isClosed(slug: string, statuses: readonly StatusDef[] = DEFAULT_STATUSES): boolean {
  return statusDef(slug, statuses)?.closed ?? false
}

export function statusGlyph(slug: string, statuses: readonly StatusDef[] = DEFAULT_STATUSES): string {
  return statusDef(slug, statuses)?.glyph ?? UNKNOWN_GLYPH
}

export function statusLabel(slug: string, statuses: readonly StatusDef[] = DEFAULT_STATUSES): string {
  return statusDef(slug, statuses)?.label ?? slug
}

/**
 * The glyph a row shows (the wireframes and `mode_core::notes::task_glyph`):
 * `✱` in place of the status glyph while an urgent note is open; a closed
 * note keeps its own glyph (`✓`).
 */
export function rowGlyph(row: Pick<NoteListRow, 'status' | 'urgent'>, statuses: readonly StatusDef[] = DEFAULT_STATUSES): string {
  if (row.urgent && !isClosed(row.status, statuses)) return URGENT_GLYPH
  return statusGlyph(row.status, statuses)
}

/** Done renders struck through (10 §7.2). */
export function isDone(row: Pick<NoteListRow, 'status'>): boolean {
  return row.status === 'done'
}

/** Note level: todo → doing → done → todo (03 §5.3); anything else goes to todo. */
export function nextStatus(slug: string): string {
  const cycle = ['todo', 'doing', 'done']
  const i = cycle.indexOf(slug)
  return i < 0 ? 'todo' : cycle[(i + 1) % cycle.length]
}

/* ---- the one-edit drop rule (04 §6.5) ------------------------------------ */

export interface DropNote {
  status: string
  urgent: boolean
  when_bucket: WhenBucket
  when_date: string | null
  tags: string[]
  folder_id: string | null
  pinned: boolean
  archived_at: string | null
  source_kind: string
}

/** What ⊕ Add task evaluates: a blank note (todo, not urgent, no `when`, no tags). */
export const BLANK_NOTE: DropNote = {
  status: 'todo',
  urgent: false,
  when_bucket: 'none',
  when_date: null,
  tags: [],
  folder_id: null,
  pinned: false,
  archived_at: null,
  source_kind: 'manual',
}

export interface DropContext {
  statuses?: readonly StatusDef[]
  today: string
  weekStart?: WeekStart
  folders?: readonly FolderRow[]
  /** The note is already among the target's rows (covers `text`, `person`, `repo`). */
  alreadyIn?: boolean
  /**
   * ⊕ Add task (03 §5.5, §5.9.5): the filter's folder is where the note is
   * created and a person chip becomes a `references` link, so neither refuses.
   */
  forAdd?: boolean
}

/** The field each refused key or edit names in a hint. */
export type DropKey = 'status' | 'urgent' | 'when' | 'tags' | 'folder' | 'pinned' | 'archived' | 'source' | 'text' | 'person' | 'repo'

export type DropEdit =
  | { key: 'status'; value: string }
  | { key: 'urgent'; value: boolean }
  | { key: 'when'; value: 'today' | 'tomorrow' | null }
  | { key: 'tag_add'; value: string }
  | { key: 'tag_remove'; value: string }

export interface DropEvaluation {
  edits: DropEdit[]
  refused: DropKey[]
  /** ⊕ Add task: where to create the note and whom to link. */
  folderId: string | null
  personId: string | null
}

export type DropPlan =
  | { kind: 'noop'; hint: string }
  | { kind: 'edit'; edit: NoteFieldEdit; hint: string }
  | { kind: 'refuse'; hint: string; needs: DropKey[] }

function inFolder(folderId: string | null, target: { id: string; recursive?: boolean }, folders: readonly FolderRow[] | undefined): boolean {
  const recursive = target.recursive !== false
  if (target.id === 'root') return recursive || folderId === null
  if (folderId === target.id) return true
  if (!recursive || !folders) return false
  const parent = new Map(folders.map((f) => [f.id, f.parent_id]))
  let cur = folderId ? parent.get(folderId) ?? null : null
  while (cur) {
    if (cur === target.id) return true
    cur = parent.get(cur) ?? null
  }
  return false
}

/** The per-key evaluation of 04 §6.5 for one (note, target filter) pair. */
export function evaluateDrop(note: DropNote, filter: FilterJson, ctx: DropContext): DropEvaluation {
  const statuses = ctx.statuses ?? DEFAULT_STATUSES
  const out: DropEvaluation = { edits: [], refused: [], folderId: null, personId: null }
  const refuse = (k: DropKey) => {
    if (!out.refused.includes(k)) out.refused.push(k)
  }
  const tags = note.tags.map(normTag)

  if (filter.status?.in) {
    const want = filter.status.in
    if (!want.includes(note.status)) {
      if (want.length === 1) out.edits.push({ key: 'status', value: want[0] })
      else refuse('status')
    }
  }
  if (filter.status?.not_in) {
    const closed = closedSlugs(statuses)
    const banned = new Set(filter.status.not_in.flatMap((s) => (s === 'closed' ? closed : [s])))
    const pending = out.edits.find((e) => e.key === 'status')
    const status = pending ? (pending.value as string) : note.status
    if (banned.has(status)) {
      if (pending) refuse('status')
      else out.edits.push({ key: 'status', value: 'todo' })
    }
  }
  if (filter.urgent !== undefined && note.urgent !== filter.urgent) out.edits.push({ key: 'urgent', value: filter.urgent })

  if (filter.when) {
    if ('in' in filter.when) {
      const buckets = filter.when.in
      if (!buckets.includes(note.when_bucket)) {
        // Only today / tomorrow / none can be assigned; Today's [overdue, today] therefore assigns `today`.
        const assignable = buckets.filter((b): b is 'today' | 'tomorrow' | 'none' => b === 'today' || b === 'tomorrow' || b === 'none')
        if (assignable.length === 1) out.edits.push({ key: 'when', value: assignable[0] === 'none' ? null : assignable[0] })
        else refuse('when')
      }
    } else {
      const from = filter.when.from ? resolveDateToken(filter.when.from, ctx.today, ctx.weekStart) : null
      const to = filter.when.to ? resolveDateToken(filter.when.to, ctx.today, ctx.weekStart) : null
      const d = note.when_date
      const inside = d !== null && (!from || d >= from) && (!to || d <= to)
      if (!inside) refuse('when')
    }
  }

  if (filter.tags) {
    for (const t of (filter.tags.all ?? []).map(normTag)) if (!tags.includes(t)) out.edits.push({ key: 'tag_add', value: t })
    const any = (filter.tags.any ?? []).map(normTag)
    if (any.length && !any.some((t) => tags.includes(t))) {
      if (any.length === 1) out.edits.push({ key: 'tag_add', value: any[0] })
      else refuse('tags')
    }
    for (const t of (filter.tags.none ?? []).map(normTag)) if (tags.includes(t)) out.edits.push({ key: 'tag_remove', value: t })
  }

  if (filter.folder) {
    if (ctx.forAdd) out.folderId = filter.folder.id === 'root' ? null : filter.folder.id
    else if (!ctx.alreadyIn && !inFolder(note.folder_id, filter.folder, ctx.folders)) refuse('folder')
  }
  if (filter.pinned && !note.pinned) refuse('pinned')
  const archived = filter.archived ?? false
  if (archived !== (note.archived_at !== null)) refuse('archived')
  if (filter.source_kind && !(filter.source_kind.in as string[]).includes(note.source_kind)) refuse('source')
  if (filter.person) {
    if (ctx.forAdd) out.personId = filter.person.contact_id
    else if (!ctx.alreadyIn) refuse('person')
  }
  if (filter.text !== undefined && !ctx.alreadyIn) refuse('text')
  if (filter.repo && !ctx.alreadyIn) refuse('repo')
  return out
}

function editKey(e: DropEdit): DropKey {
  return e.key === 'tag_add' || e.key === 'tag_remove' ? 'tags' : e.key
}

export function joinWords(words: string[]): string {
  if (words.length <= 1) return words.join('')
  return `${words.slice(0, -1).join(', ')} and ${words[words.length - 1]}`
}

function toFieldEdit(e: DropEdit, note: DropNote): NoteFieldEdit {
  switch (e.key) {
    case 'status':
      return { field: 'status', value: e.value }
    case 'urgent':
      return { field: 'urgent', value: e.value }
    case 'when':
      return { field: 'when', value: e.value }
    case 'tag_add':
      return { field: 'tags', value: [...note.tags, e.value] }
    case 'tag_remove':
      return { field: 'tags', value: note.tags.filter((t) => normTag(t) !== e.value) }
  }
}

/**
 * The drop decision for a note dragged onto a grouping (04 §6.5, 03 §5.5):
 * one deterministic edit or a refusal whose hint names what is missing.
 */
export function planDrop(note: DropNote, target: { name: string; filter: FilterJson }, ctx: DropContext): DropPlan {
  if (ctx.alreadyIn) return { kind: 'noop', hint: `Already in ${target.name}` }
  const ev = evaluateDrop(note, target.filter, ctx)
  const needs: DropKey[] = [...ev.refused]
  if (ev.refused.length || ev.edits.length > 1) {
    for (const e of ev.edits) if (!needs.includes(editKey(e))) needs.push(editKey(e))
    return { kind: 'refuse', needs, hint: `Edit the note's fields to match ${target.name} · needs ${joinWords(needs)}` }
  }
  if (ev.edits.length === 0) return { kind: 'noop', hint: `Already in ${target.name}` }
  const edit = toFieldEdit(ev.edits[0], note)
  return { kind: 'edit', edit, hint: describeEdit(edit) }
}

export function describeEdit(edit: NoteFieldEdit, statuses: readonly StatusDef[] = DEFAULT_STATUSES): string {
  switch (edit.field) {
    case 'status':
      return `Status set to ${statusLabel(edit.value, statuses)}`
    case 'urgent':
      return edit.value ? 'Marked urgent' : 'No longer urgent'
    case 'when':
      return edit.value ? `Date set to ${edit.value}` : 'Date cleared'
    case 'tags':
      return 'Tags updated'
    case 'pinned':
      return edit.value ? 'Pinned' : 'Unpinned'
    case 'folder_id':
      return 'Moved'
  }
}

/* ---- ⊕ Add task (03 §5.5, §5.9.5; 04 §6.5 last paragraph, §7.2) ---------- */

export interface AddTaskPlan {
  /** Hidden when any key cannot be assigned to a blank note. */
  show: boolean
  /** Capture tokens appended to the typed title. */
  tokens: string[]
  folderId: string | null
  personId: string | null
}

const TOKEN_TAG = /^[^\s#,[\]"]{1,64}$/

export function addTaskPlan(filter: FilterJson, ctx: DropContext): AddTaskPlan {
  const ev = evaluateDrop(BLANK_NOTE, filter, { ...ctx, forAdd: true })
  const tokens: string[] = []
  let show = ev.refused.length === 0
  for (const e of ev.edits) {
    if (e.key === 'status' && e.value !== 'todo') tokens.push(`/${e.value}`)
    else if (e.key === 'urgent' && e.value) tokens.push('!')
    else if (e.key === 'urgent' && !e.value) continue
    else if (e.key === 'when' && e.value) tokens.push(`@${e.value}`)
    else if (e.key === 'tag_add') {
      if (TOKEN_TAG.test(e.value)) tokens.push(`#${e.value}`)
      else show = false
    }
  }
  return { show, tokens, folderId: ev.folderId, personId: ev.personId }
}

/** The text sent to `note_capture` for a typed Add task title. */
export function captureText(title: string, plan: AddTaskPlan, folder?: FolderRow | null): string {
  const parts = [title.trim(), ...plan.tokens]
  if (folder && !/\s/.test(folder.path)) parts.push(`in:${folder.path}`)
  return parts.filter(Boolean).join(' ')
}

/* ---- chips ---------------------------------------------------------------- */

/** Backspace on an empty field turns the last chip back into text (04 §8.7). */
export function chipToText(chip: QueryChip): string {
  switch (chip.kind) {
    case 'person':
      return chip.label
    case 'type':
      return chip.value === 'calendar_events' ? 'events' : chip.value
    case 'status':
      return chip.value === 'not-urgent' ? 'not urgent' : chip.value
    case 'when':
      if ('in' in chip.value && chip.value.in.length === 1) {
        const b = chip.value.in[0]
        return b === 'none' ? 'unscheduled' : b
      }
      return chip.label.toLowerCase()
    case 'tag':
      return typeof chip.value === 'string' ? `#${chip.value}` : chip.value.any.map((t) => `#${t}`).join(' or ')
    case 'repo':
      return `repo:${chip.label}`
    case 'folder':
      return `in:${chip.label}`
    case 'text':
      return chip.value
    case 'flag':
      return chip.value.startsWith('source:') ? chip.label.toLowerCase() : chip.value
  }
}

function chipKey(c: QueryChip): string {
  return `${c.kind}:${JSON.stringify(c.value)}`
}

/** Same query, whatever the commit order or origin (an identical card is raised instead of spawned, 03 §10.4). */
export function sameChips(a: readonly QueryChip[], b: readonly QueryChip[]): boolean {
  if (a.length !== b.length) return false
  const ka = a.map(chipKey).sort()
  const kb = b.map(chipKey).sort()
  return ka.every((k, i) => k === kb[i])
}
