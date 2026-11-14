// Browser stand-in for the Rust notes core (package 7, Rust half pending):
// an in-memory sample vault, the notes commands of contract-notes.ts, and a
// small TypeScript chip resolver covering the worked examples of 04 §8.6.
//
// TEST SCAFFOLDING, not the product: the real vault walker, frontmatter
// parser, groupings compiler and chip resolver live in `mode_core::notes`.
// Registered through `window.__mode_mock.respond` (ui/src/tauri.ts) by
// `installNotesMock()`; every write emits `notes.changed` like the core.
import { mock as modeMock } from '../tauri'
import { EVENTS, type NoteRow, type NotesChanged } from '../state/contract'
import type {
  ChipKind,
  FilterJson,
  FolderRow,
  FoldersList,
  GroupingDraft,
  GroupingRow,
  GroupingsList,
  ItemType,
  NoteDetail,
  NoteFieldEdit,
  NoteLink,
  NoteListRow,
  NoteWriteResult,
  NotesListArgs,
  QueryChip,
  QueryContextArgs,
  QueryResolveResult,
  QueryRunArgs,
  QueryRunResult,
  QuerySuggestion,
  RevisionRow,
  SortKey,
  SourceKind,
  StatusDef,
  TagRow,
  WeekStart,
  WhenBucket,
  WhenFilter,
} from '../state/contract-notes'
import { DEFAULT_STATUSES } from '../state/contract-notes'
import { addDays, isValidDate, monthEnd, monthStart, rangeLabel, resolveDateToken, shortDate, weekStartOf, weekday } from './dates'
import { splitFile } from './editor/markdown'
import { closedSlugs, nextStatus, normTag } from './rules'

/** The wireframes' sample moment: Wednesday 2026-10-21 (query-cards.html §8). */
export const MOCK_TODAY = '2026-10-21'

/* ==== frontmatter (04 §3) ================================================== */

export interface Frontmatter {
  id: string | null
  status: string
  urgent: boolean
  when: string | null
  tags: string[]
  created: string | null
  updated: string | null
  rev: number
  /** Unknown keys, verbatim `key: value` text, in order. */
  extra: [string, string][]
}

function unquote(v: string): string {
  const t = v.trim()
  if ((t.startsWith('"') && t.endsWith('"')) || (t.startsWith("'") && t.endsWith("'"))) return t.slice(1, -1)
  return t
}

function parseList(v: string): string[] {
  const t = v.trim()
  const inner = t.startsWith('[') && t.endsWith(']') ? t.slice(1, -1) : t
  return inner
    .split(t.startsWith('[') ? ',' : /[,\s]+/)
    .map((s) => unquote(s).replace(/^#/, '').trim())
    .filter(Boolean)
}

/** A YAML subset parse; null when the block is not readable (04 §3.3). */
export function parseFrontmatter(block: string): Frontmatter | null {
  const fm: Frontmatter = { id: null, status: 'todo', urgent: false, when: null, tags: [], created: null, updated: null, rev: 1, extra: [] }
  const lines = block.split(/\r?\n/)
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    if (!line.trim() || line.trim().startsWith('#')) continue
    const m = /^([A-Za-z_][\w-]*):(?:[ \t]+(.*))?$/.exec(line)
    if (!m) return null
    const key = m[1]
    let value = m[2] ?? ''
    if (value === '' && key === 'tags') {
      const items: string[] = []
      while (i + 1 < lines.length && /^\s+-\s+/.test(lines[i + 1])) items.push(lines[++i].replace(/^\s+-\s+/, ''))
      value = `[${items.join(', ')}]`
    }
    switch (key) {
      case 'id':
        fm.id = unquote(value)
        break
      case 'status':
        fm.status = unquote(value).toLowerCase() || 'todo'
        break
      case 'urgent':
        fm.urgent = /^(true|yes|1)$/i.test(unquote(value))
        break
      case 'when': {
        const w = unquote(value).toLowerCase()
        fm.when = w === 'today' || w === 'tomorrow' || isValidDate(w.slice(0, 10)) ? (w.length > 10 ? w.slice(0, 10) : w) : null
        break
      }
      case 'tags':
        fm.tags = parseList(value)
        break
      case 'created':
        fm.created = unquote(value)
        break
      case 'updated':
        fm.updated = unquote(value)
        break
      case 'rev':
        fm.rev = /^\d+$/.test(value.trim()) ? Number(value.trim()) : 1
        break
      default:
        fm.extra.push([key, value])
    }
  }
  return fm
}

const PLAIN_TAG = /^[A-Za-z0-9][\w/ .-]*$/

/** The one shape Mode writes (04 §3.4). */
export function writeFrontmatter(fm: Frontmatter, eol = '\n'): string {
  const lines = ['---', `id: ${fm.id}`, `status: ${fm.status}`]
  if (fm.urgent) lines.push('urgent: true')
  if (fm.when) lines.push(`when: ${fm.when}`)
  if (fm.tags.length) lines.push(`tags: [${fm.tags.map((t) => (PLAIN_TAG.test(t) && !t.includes(': ') ? t : `"${t}"`)).join(', ')}]`)
  if (fm.created) lines.push(`created: ${fm.created}`)
  if (fm.updated) lines.push(`updated: ${fm.updated}`)
  lines.push(`rev: ${fm.rev}`)
  for (const [k, v] of fm.extra) lines.push(v ? `${k}: ${v}` : `${k}:`)
  lines.push('---')
  return lines.join(eol) + eol
}

export function titleOf(body: string, fileName: string): string {
  const m = /^# +(.+?)\s*#*\s*$/m.exec(body)
  return m ? m[1].trim() : fileName.replace(/\.md$/, '')
}

const TASK_LINE = /^(\s*)([-*+]|\d+[.)])[ \t]+\[( |\/|!|x|X)\][ \t]+(.*)$/

export function taskCounts(body: string): { total: number; done: number } {
  let total = 0
  let done = 0
  for (const line of body.split(/\r?\n/)) {
    const m = TASK_LINE.exec(line)
    if (!m) continue
    total++
    if (m[3] === 'x' || m[3] === 'X') done++
  }
  return { total, done }
}

/** FNV-1a, hex: a stand-in for the core's sha256 `content_hash`. */
export function hashText(s: string): string {
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return (h >>> 0).toString(16).padStart(8, '0')
}

function excerptOf(body: string): string {
  return body
    .split(/\r?\n/)
    .filter((l) => !/^#\s/.test(l))
    .map((l) => l.replace(TASK_LINE, '$4').replace(/[*_`~>#[\]]/g, '').trim())
    .filter(Boolean)
    .join(' ')
    .slice(0, 200)
}

function fileNameFor(title: string): string {
  const clean = title
    .replace(/[/\\:*?"<>|\u0000-\u001f]/g, '-')
    .replace(/\s+/g, ' ')
    .replace(/^[ .]+|[ .]+$/g, '')
    .slice(0, 120)
  return `${clean || 'Untitled'}.md`
}

/* ==== the sample vault ===================================================== */

interface Contact {
  id: string
  display_name: string
  initials: string
}

interface MockNote {
  id: string
  folder_id: string | null
  file: string
  text: string
  pinned: boolean
  archived_at: string | null
  source_kind: SourceKind
  people: string[]
  repos: string[]
  links: NoteLink[]
  revisions: { rev: number; origin: RevisionRow['origin']; created_at: string; content: string }[]
  deleted: boolean
  read_only?: boolean
}

interface MockGrouping extends Omit<GroupingRow, 'count'> {}

interface SeedNote {
  id: string
  folder: string | null
  title: string
  status?: string
  urgent?: boolean
  when?: string | null
  tags?: string[]
  updated: string
  body?: string
  people?: string[]
  repos?: string[]
  archived?: boolean
  pinned?: boolean
  source?: SourceKind
}

const FOLDERS: Omit<FolderRow, 'note_count'>[] = [
  { id: 'f-unsorted', parent_id: null, path: 'Unsorted', name: 'Unsorted', kind: 'capture', position: 0 },
  { id: 'f-projects', parent_id: null, path: 'Projects', name: 'Projects', kind: 'normal', position: 1 },
  { id: 'f-mode', parent_id: 'f-projects', path: 'Projects/Mode', name: 'Mode', kind: 'normal', position: 0 },
  { id: 'f-docs', parent_id: 'f-projects', path: 'Projects/Docs site', name: 'Docs site', kind: 'normal', position: 1 },
  { id: 'f-clients', parent_id: null, path: 'Clients', name: 'Clients', kind: 'normal', position: 2 },
  { id: 'f-acme', parent_id: 'f-clients', path: 'Clients/Acme', name: 'Acme', kind: 'normal', position: 0 },
  { id: 'f-globex', parent_id: 'f-clients', path: 'Clients/Globex', name: 'Globex', kind: 'normal', position: 1 },
  { id: 'f-personal', parent_id: null, path: 'Personal', name: 'Personal', kind: 'normal', position: 3 },
  { id: 'f-health', parent_id: 'f-personal', path: 'Personal/Health', name: 'Health', kind: 'normal', position: 0 },
  { id: 'f-home', parent_id: 'f-personal', path: 'Personal/Home', name: 'Home', kind: 'normal', position: 1 },
]

const CONTACTS: Contact[] = [
  { id: 'c-bjorn', display_name: 'Bjorn', initials: 'B' },
  { id: 'c-tim', display_name: 'Tim', initials: 'T' },
  { id: 'c-lena', display_name: 'Lena', initials: 'L' },
  { id: 'c-priya', display_name: 'Priya', initials: 'P' },
  { id: 'c-ana', display_name: 'Ana', initials: 'A' },
]

const REPOS = ['/Users/me/code/mode', '/Users/me/code/docs-site']

const CHANGELOG_BODY = [
  '# Draft v2 changelog',
  '',
  'Lead with what a user notices, then the migration steps from `guides/migration.md`. Under 120 words, link the **PR numbers**.',
  '',
  '- [x] Rate limiter retries with exponential backoff (PR 418)',
  '- [x] Auth fixture pins the clock and isolates the database (PR 417)',
  "- [/] Migration steps from Lena's guide (PR 412)",
  '- [ ] Mention that the webhook spike is parked',
  '- [!] Ask Priya whether the rollout dates moved',
  '',
  'Tone: the same as the last two notes in `CHANGELOG.md`; no exclamation marks. See [[Release checklist]] and [the guide](https://example.com/guide).',
  '',
].join('\n')

function seeds(today: string): SeedNote[] {
  const d = (n: number) => addDays(today, n)
  const at = (n: number, hm: string) => `${d(n)}T${hm}:00.000Z`
  return [
    // Projects/Mode: the eight rows of app-window.html #n-list, newest first
    { id: 'n-auth', folder: 'f-mode', title: 'Fix flaky auth test', urgent: true, when: d(0), updated: at(0, '09:30'), repos: ['/Users/me/code/mode'], body: 'Pin the clock in the auth fixture; isolate the database per test.\n' },
    { id: 'n-changelog', folder: 'f-mode', title: 'Draft v2 changelog', status: 'doing', when: d(1), tags: ['mode', 'release'], updated: at(0, '09:20'), people: ['c-lena'], body: CHANGELOG_BODY },
    { id: 'n-checklist', folder: 'f-mode', title: 'Release checklist', when: d(-2), updated: at(0, '08:00') },
    { id: 'n-retry', folder: 'f-mode', title: 'Rate limiter retry policy', when: d(2), updated: at(-1, '18:00') },
    { id: 'n-webhook', folder: 'f-mode', title: 'Webhook spike notes', updated: at(-1, '15:00') },
    {
      id: 'n-onboarding',
      folder: 'f-mode',
      title: 'Onboarding copy',
      status: 'doing',
      updated: at(-1, '12:00'),
      body: '# Onboarding copy\n\n- [x] Welcome screen\n- [x] Vault picker\n- [ ] Hooks step\n- [ ] Overlay placement\n- [ ] Done screen\n',
    },
    { id: 'n-lena', folder: 'f-mode', title: 'Reply to Lena about PR 412', status: 'done', updated: at(-2, '16:00'), people: ['c-lena'] },
    { id: 'n-keys', folder: 'f-mode', title: 'Rotate the API keys', status: 'done', updated: at(-3, '10:00') },
    // Unsorted (the capture folder)
    { id: 'n-plumber', folder: 'f-unsorted', title: 'Call the plumber', when: d(1), tags: ['home'], updated: at(-1, '20:17'), people: ['c-bjorn'], source: 'capture' },
    { id: 'n-overlay', folder: 'f-unsorted', title: 'Overlay spike', updated: at(-4, '11:00'), repos: ['/Users/me/code/mode'], source: 'session', body: '**Repo:** /Users/me/code/mode\n\nThe overlay spike: NSPanel above the menu bar.\n' },
    { id: 'n-dentist', folder: 'f-unsorted', title: 'Book dentist', updated: at(-5, '09:00'), source: 'capture' },
    // Projects/Docs site
    { id: 'n-relnotes', folder: 'f-docs', title: 'Draft the release notes', status: 'doing', when: d(1), updated: at(0, '07:45'), people: ['c-bjorn'], repos: ['/Users/me/code/docs-site'], body: '- [x] Outline\n- [x] Screenshots\n- [ ] Migration\n- [ ] Links\n- [ ] Review\n' },
    { id: 'n-migration', folder: 'f-docs', title: "Review Lena's migration guide", status: 'doing', when: d(1), updated: at(-1, '17:00'), people: ['c-bjorn', 'c-lena'], repos: ['/Users/me/code/docs-site'] },
    { id: 'n-showreel', folder: 'f-docs', title: 'Upload the latest showreel', urgent: true, when: d(-1), tags: ['launch'], updated: at(-1, '10:00'), people: ['c-tim'] },
    { id: 'n-studio', folder: 'f-docs', title: 'Send the weekly studio update', when: d(-5), tags: ['launch'], updated: at(-5, '09:00'), people: ['c-tim'] },
    // Clients
    { id: 'n-domain', folder: 'f-acme', title: 'Renew the domain', urgent: true, when: d(1), tags: ['admin'], updated: at(-2, '13:00'), people: ['c-bjorn'] },
    { id: 'n-contractor', folder: 'f-acme', title: 'Welcome the new contractor', status: 'done', when: d(1), updated: at(-1, '11:00'), people: ['c-bjorn'] },
    { id: 'n-screening', folder: 'f-globex', title: 'Book the next team screening', when: d(-2), updated: at(-3, '15:00'), people: ['c-tim'] },
    { id: 'n-budget', folder: 'f-globex', title: 'Q4 budget', tags: ['admin'], updated: at(-20, '10:00'), archived: true, body: 'Budget review for Q4: travel, tooling, contractors.\n' },
    // Personal
    { id: 'n-milk', folder: 'f-home', title: 'Buy milk', status: 'done', when: d(1), tags: ['groceries'], updated: at(-1, '08:00'), people: ['c-bjorn'] },
    { id: 'n-oatmilk', folder: 'f-home', title: 'Buy oat milk', when: d(0), tags: ['groceries'], updated: at(-1, '07:00') },
    { id: 'n-coffee', folder: 'f-home', title: 'Order coffee beans', when: d(3), tags: ['groceries'], updated: at(-2, '07:00') },
    { id: 'n-bottles', folder: 'f-home', title: 'Return the bottles', when: d(2), tags: ['groceries'], updated: at(-2, '06:00') },
    { id: 'n-parcel', folder: 'f-home', title: 'Post the parcel', status: 'done', tags: ['errands'], updated: at(-3, '12:00') },
    { id: 'n-gym-legs', folder: 'f-health', title: 'Gym: legs and core', when: d(-2), tags: ['gym'], updated: at(-3, '06:30') },
    { id: 'n-gym-upper', folder: 'f-health', title: 'Gym: upper body', when: d(1), tags: ['gym'], updated: at(-3, '06:31') },
    // The vault root
    { id: 'n-groceries', folder: null, title: 'Groceries', tags: ['groceries'], updated: at(-6, '18:00'), pinned: true, body: '# Groceries\n\n- [ ] Oat milk\n- [ ] Coffee\n- [x] Bread\n' },
  ]
}

const PREMADE: MockGrouping[] = [
  { id: 'g-todo', slug: 'todo', name: 'Todo', kind: 'premade', glyph: '○', color: null, position: 0, show_in_overlay: false, show_in_sidebar: true, filter_json: { status: { in: ['todo'] } }, sort_json: todaySort() },
  { id: 'g-doing', slug: 'doing', name: 'Doing', kind: 'premade', glyph: '◑', color: null, position: 1, show_in_overlay: false, show_in_sidebar: true, filter_json: { status: { in: ['doing'] } }, sort_json: todaySort() },
  { id: 'g-urgent', slug: 'urgent', name: 'Urgent', kind: 'premade', glyph: '✱', color: null, position: 2, show_in_overlay: true, show_in_sidebar: true, filter_json: { urgent: true, status: { not_in: ['closed'] } }, sort_json: [{ by: 'when_date', dir: 'asc' }, { by: 'updated_at', dir: 'desc' }] },
  { id: 'g-done', slug: 'done', name: 'Done', kind: 'premade', glyph: '✓', color: null, position: 3, show_in_overlay: false, show_in_sidebar: true, filter_json: { status: { in: ['done'] } }, sort_json: [{ by: 'updated_at', dir: 'desc' }] },
  { id: 'g-today', slug: 'today', name: 'Today', kind: 'premade', glyph: '▦', color: null, position: 4, show_in_overlay: true, show_in_sidebar: true, filter_json: { when: { in: ['overdue', 'today'] }, status: { not_in: ['closed'] } }, sort_json: todaySort() },
  { id: 'g-tomorrow', slug: 'tomorrow', name: 'Tomorrow', kind: 'premade', glyph: '▦', color: null, position: 5, show_in_overlay: false, show_in_sidebar: true, filter_json: { when: { in: ['tomorrow'] }, status: { not_in: ['closed'] } }, sort_json: todaySort() },
]

const CUSTOM: MockGrouping[] = [
  { id: 'g-groceries', slug: 'groceries', name: 'Groceries', kind: 'custom', glyph: '#', color: null, position: 6, show_in_overlay: false, show_in_sidebar: true, filter_json: { tags: { any: ['groceries'] }, status: { not_in: ['closed'] } }, sort_json: todaySort() },
  { id: 'g-gym', slug: 'gym', name: 'Gym', kind: 'custom', glyph: '#', color: null, position: 7, show_in_overlay: false, show_in_sidebar: true, filter_json: { tags: { all: ['gym'] }, when: { from: 'week_start', to: 'week_end' } }, sort_json: todaySort() },
]

function todaySort(): SortKey[] {
  return [
    { by: 'urgent', dir: 'desc' },
    { by: 'when_date', dir: 'asc' },
    { by: 'updated_at', dir: 'desc' },
  ]
}

export interface NotesMockOptions {
  today?: string
  /** `false`: an empty vault (the empty state). */
  sample?: boolean
  /** Emit `notes.changed` after writes (default true). */
  emit?: boolean
}

/* ==== the vault ============================================================ */

export class NotesVault {
  today: string
  weekStart: WeekStart = 'monday'
  statuses: StatusDef[] = DEFAULT_STATUSES.map((s) => ({ ...s }))
  folders: Omit<FolderRow, 'note_count'>[]
  notes = new Map<string, MockNote>()
  groupings: MockGrouping[]
  board = ['todo', 'doing', 'urgent', 'done']
  contacts = CONTACTS
  repos = REPOS
  /** Display names by `name_norm` (first spelling seen). */
  tagNames = new Map<string, string>()
  emit: boolean
  private seq = 0
  private clock = 0

  constructor(opts: NotesMockOptions = {}) {
    this.today = opts.today ?? MOCK_TODAY
    this.emit = opts.emit ?? true
    const sample = opts.sample ?? true
    this.folders = sample ? FOLDERS.map((f) => ({ ...f })) : [{ ...FOLDERS[0] }]
    this.groupings = [...PREMADE, ...(sample ? CUSTOM : [])].map((g) => ({ ...g }))
    if (sample) for (const s of seeds(this.today)) this.seed(s)
  }

  private seed(s: SeedNote) {
    const created = s.updated
    const fm: Frontmatter = {
      id: s.id,
      status: s.status ?? 'todo',
      urgent: !!s.urgent,
      when: s.when ?? null,
      tags: s.tags ?? [],
      created,
      updated: s.updated,
      rev: 3,
      extra: [],
    }
    const body = s.body && s.body.startsWith('# ') ? s.body : `# ${s.title}\n\n${s.body ?? ''}`
    const text = writeFrontmatter(fm) + body
    for (const t of fm.tags) if (!this.tagNames.has(normTag(t))) this.tagNames.set(normTag(t), t)
    this.notes.set(s.id, {
      id: s.id,
      folder_id: s.folder,
      file: fileNameFor(s.title),
      text,
      pinned: !!s.pinned,
      archived_at: s.archived ? `${addDays(this.today, -2)}T10:00:00.000Z` : null,
      source_kind: s.source ?? 'manual',
      people: s.people ?? [],
      repos: s.repos ?? [],
      links: (s.people ?? []).map((c, i) => ({
        id: `l-${s.id}-${i}`,
        link_type: 'references' as const,
        direction: 'from' as const,
        kind: 'contact' as const,
        target_id: c,
        label: null,
        title: this.contacts.find((x) => x.id === c)?.display_name ?? null,
        available: true,
      })),
      revisions: [
        { rev: 1, origin: 'mode', created_at: created, content: text },
        { rev: 3, origin: 'external', created_at: s.updated, content: text },
      ],
      deleted: false,
    })
  }

  /* ---- derived rows ------------------------------------------------------ */

  now(): string {
    // Writes stamp the sample day at a moving minute so sorting by `updated_at` is stable and newest-first.
    this.clock++
    const ms = Date.parse(`${this.today}T12:00:00.000Z`) + this.clock * 60_000
    return new Date(ms).toISOString()
  }

  fm(n: MockNote): Frontmatter {
    const { block } = splitFile(n.text)
    return (block !== null && parseFrontmatter(block)) || { id: n.id, status: 'todo', urgent: false, when: null, tags: [], created: null, updated: null, rev: 1, extra: [] }
  }

  whenDate(fm: Frontmatter): string | null {
    if (!fm.when) return null
    const anchor = fm.updated?.slice(0, 10) ?? this.today
    if (fm.when === 'today') return anchor
    if (fm.when === 'tomorrow') return addDays(anchor, 1)
    return fm.when
  }

  bucket(status: string, date: string | null): WhenBucket {
    if (!date) return 'none'
    const closed = closedSlugs(this.statuses).includes(status)
    if (date < this.today && !closed) return 'overdue'
    if (date === this.today) return 'today'
    if (date === addDays(this.today, 1)) return 'tomorrow'
    return 'scheduled'
  }

  folderPath(id: string | null): string {
    return id ? this.folders.find((f) => f.id === id)?.path ?? '' : ''
  }

  row(n: MockNote): NoteListRow {
    const fm = this.fm(n)
    const { body } = splitFile(n.text)
    const tasks = taskCounts(body)
    const date = this.whenDate(fm)
    const folder = this.folderPath(n.folder_id)
    return {
      id: n.id,
      title: titleOf(body, n.file),
      status: fm.status,
      urgent: fm.urgent,
      when_date: date,
      when_bucket: this.bucket(fm.status, date),
      path: folder ? `${folder}/${n.file}` : n.file,
      folder_id: n.folder_id,
      excerpt: excerptOf(body),
      task_total: tasks.total,
      task_done: tasks.done,
      pinned: n.pinned,
      archived_at: n.archived_at,
      created_at: fm.created ?? fm.updated ?? `${this.today}T00:00:00.000Z`,
      updated_at: fm.updated ?? `${this.today}T00:00:00.000Z`,
      source_kind: n.source_kind,
      tags: fm.tags.map((t) => this.tagNames.get(normTag(t)) ?? t),
      has_links: n.links.length > 0,
    }
  }

  detail(n: MockNote): NoteDetail {
    const row = this.row(n)
    const { block } = splitFile(n.text)
    return {
      row,
      text: n.text,
      content_hash: hashText(n.text),
      rev: this.fm(n).rev,
      folder_path: this.folderPath(n.folder_id),
      read_only: !!n.read_only,
      read_only_reason: n.read_only ? 'Read-only' : null,
      frontmatter_error: block !== null && parseFrontmatter(block) === null,
      unknown_status: !this.statuses.some((s) => s.slug === row.status),
      links: n.links,
    }
  }

  live(): MockNote[] {
    return Array.from(this.notes.values()).filter((n) => !n.deleted)
  }

  get(id: string): MockNote {
    const n = this.notes.get(id)
    if (!n || n.deleted) throw new Error(`No note ${id}`)
    return n
  }

  /* ---- filter_json (04 §6.1) --------------------------------------------- */

  private inFolder(folderId: string | null, target: { id: string; recursive?: boolean }): boolean {
    const recursive = target.recursive !== false
    if (target.id === 'root') return recursive || folderId === null
    if (folderId === target.id) return true
    if (!recursive) return false
    let cur = folderId ? this.folders.find((f) => f.id === folderId)?.parent_id ?? null : null
    while (cur) {
      if (cur === target.id) return true
      cur = this.folders.find((f) => f.id === cur)?.parent_id ?? null
    }
    return false
  }

  matches(n: MockNote, f: FilterJson): boolean {
    const r = this.row(n)
    const archived = f.archived ?? false
    if (archived !== (n.archived_at !== null)) return false
    if (f.status?.in && !f.status.in.includes(r.status)) return false
    if (f.status?.not_in) {
      const closed = closedSlugs(this.statuses)
      const banned = f.status.not_in.flatMap((s) => (s === 'closed' ? closed : [s]))
      if (banned.includes(r.status)) return false
    }
    if (f.urgent !== undefined && r.urgent !== f.urgent) return false
    if (f.when) {
      if ('in' in f.when) {
        if (!f.when.in.includes(r.when_bucket)) return false
      } else {
        const from = f.when.from ? resolveDateToken(f.when.from, this.today, this.weekStart) : null
        const to = f.when.to ? resolveDateToken(f.when.to, this.today, this.weekStart) : null
        if (!r.when_date || (from && r.when_date < from) || (to && r.when_date > to)) return false
      }
    }
    const tags = r.tags.map(normTag)
    if (f.tags?.all && !f.tags.all.every((t) => tags.includes(normTag(t)))) return false
    if (f.tags?.any && !f.tags.any.some((t) => tags.includes(normTag(t)))) return false
    if (f.tags?.none && f.tags.none.some((t) => tags.includes(normTag(t)))) return false
    if (f.folder && !this.inFolder(n.folder_id, f.folder)) return false
    if (f.text) {
      const hay = `${r.title}\n${splitFile(n.text).body}\n${r.tags.join(' ')}`.toLowerCase()
      if (!f.text.toLowerCase().split(/\s+/).filter(Boolean).every((w) => hay.includes(w))) return false
    }
    if (f.person && !n.people.includes(f.person.contact_id)) return false
    if (f.repo && !n.repos.includes(f.repo.repo_root)) return false
    if (f.pinned && !n.pinned) return false
    if (f.source_kind && !f.source_kind.in.includes(n.source_kind)) return false
    return true
  }

  sortRows(rows: NoteListRow[], sort: SortKey[]): NoteListRow[] {
    const order = (s: string) => {
      const i = this.statuses.findIndex((x) => x.slug === s)
      return i < 0 ? 99 : i
    }
    return [...rows].sort((a, b) => {
      if (a.pinned !== b.pinned) return a.pinned ? -1 : 1
      for (const k of sort) {
        const dir = k.dir === 'desc' ? -1 : 1
        let c = 0
        if (k.by === 'when_date') {
          if (a.when_date === b.when_date) c = 0
          else if (!a.when_date) return 1
          else if (!b.when_date) return -1
          else c = a.when_date < b.when_date ? -1 : 1
        } else if (k.by === 'urgent') c = Number(a.urgent) - Number(b.urgent)
        else if (k.by === 'status') c = order(a.status) - order(b.status)
        else if (k.by === 'title') c = a.title.localeCompare(b.title)
        else c = a[k.by] < b[k.by] ? -1 : a[k.by] > b[k.by] ? 1 : 0
        if (c) return c * dir
      }
      return 0
    })
  }

  query(f: FilterJson, sort: SortKey[]): NoteListRow[] {
    return this.sortRows(
      this.live()
        .filter((n) => this.matches(n, f))
        .map((n) => this.row(n)),
      sort,
    )
  }

  /* ---- writes ------------------------------------------------------------ */

  private changed() {
    if (!this.emit) return
    const payload = this.overlayPayload()
    setTimeout(() => modeMock().emit(EVENTS.notes, payload), 0)
  }

  overlayPayload(): NotesChanged {
    const toOverlay = (r: NoteListRow): NoteRow => ({
      id: r.id,
      title: r.title,
      status: r.status,
      glyph: null,
      urgent: r.urgent,
      when_bucket: r.when_bucket === 'scheduled' ? 'date' : r.when_bucket,
      when_date: r.when_date,
      tags: r.tags,
      task_done: r.task_done,
      task_total: r.task_total,
      has_links: r.has_links,
    })
    const sections = this.groupings
      .filter((g) => g.show_in_overlay)
      .sort((a, b) => a.position - b.position)
      .map((g) => ({ grouping: g.name, glyph: g.glyph, rows: this.query(g.filter_json, g.sort_json).map(toOverlay) }))
    const overdue = this.query({ when: { in: ['overdue'] }, status: { not_in: ['closed'] } }, []).length
    return { sections, overdue_count: overdue }
  }

  /** Writes `text` as Mode would: frontmatter rewritten with `updated = now`, `rev + 1`; the body verbatim. */
  private write(n: MockNote, text: string, origin: RevisionRow['origin'] = 'mode', mutate?: (fm: Frontmatter) => void): void {
    const split = splitFile(text)
    const fm = (split.block !== null && parseFrontmatter(split.block)) || { id: n.id, status: 'todo', urgent: false, when: null, tags: [], created: null, updated: null, rev: this.fm(n).rev, extra: [] }
    const prevRev = n.text ? this.fm(n).rev : 0
    fm.id = n.id
    mutate?.(fm)
    fm.rev = Math.max(prevRev, fm.rev) + 1
    fm.updated = this.now()
    fm.created ??= fm.updated
    for (const t of fm.tags) if (!this.tagNames.has(normTag(t))) this.tagNames.set(normTag(t), t)
    n.text = split.bom + writeFrontmatter(fm, split.eol) + split.body
    n.revisions.push({ rev: fm.rev, origin, created_at: fm.updated, content: n.text })
    this.changed()
  }

  private saved(n: MockNote, copy_id?: string): NoteWriteResult {
    const d = this.detail(n)
    return { outcome: 'saved', row: d.row, text: d.text, content_hash: d.content_hash, rev: d.rev, updated_at: d.row.updated_at, ...(copy_id ? { copy_id } : {}) }
  }

  create(folderId: string | null, text: string, source: SourceKind = 'manual'): MockNote {
    const id = `n-new-${++this.seq}`
    const split = splitFile(text)
    const title = titleOf(split.body, 'Untitled.md')
    let file = fileNameFor(title)
    const taken = new Set(this.live().filter((x) => x.folder_id === folderId).map((x) => x.file.toLowerCase()))
    for (let i = 2; taken.has(file.toLowerCase()); i++) file = fileNameFor(`${title} ${i}`)
    const n: MockNote = { id, folder_id: folderId, file, text: '', pinned: false, archived_at: null, source_kind: source, people: [], repos: [], links: [], revisions: [], deleted: false }
    this.notes.set(id, n)
    const fm: Frontmatter = (split.block !== null && parseFrontmatter(split.block)) || { id, status: 'todo', urgent: false, when: null, tags: [], created: null, updated: null, rev: 0, extra: [] }
    fm.rev = 0
    // 04 §3.4: one blank line between the closing fence and the body of a file Mode creates.
    const body = split.block === null && !split.body.startsWith('\n') ? `\n${split.body}` : split.body
    this.write(n, writeFrontmatter(fm) + body)
    return n
  }

  save(args: { note_id: string; text: string; base_hash: string; resolve?: 'mine' | 'theirs' | 'both' }): NoteWriteResult {
    const n = this.get(args.note_id)
    if (n.read_only) return { outcome: 'failed', message: 'The file is read-only.' }
    const disk = hashText(n.text)
    if (args.resolve === 'theirs') {
      n.revisions.push({ rev: this.fm(n).rev, origin: 'mode', created_at: this.now(), content: args.text })
      return this.saved(n)
    }
    if (args.resolve === 'both') {
      const title = titleOf(splitFile(args.text).body, n.file)
      const stamp = `${this.today} ${this.now().slice(11, 16).replace(':', '')}`
      const copy = this.create(n.folder_id, args.text.replace(/^(﻿?---[\s\S]*?\nid: )\S+/, '$1pending'))
      copy.file = fileNameFor(`${title} (conflict ${stamp})`)
      copy.links.push({ id: `l-${copy.id}-rel`, link_type: 'related', direction: 'from', kind: 'note', target_id: n.id, label: null, title, available: true })
      return this.saved(n, copy.id)
    }
    if (disk !== args.base_hash && args.resolve !== 'mine') {
      return { outcome: 'conflict', disk_text: n.text, disk_hash: disk, disk_rev: this.fm(n).rev }
    }
    if (args.resolve === 'mine') n.revisions.push({ rev: this.fm(n).rev, origin: 'external', created_at: this.now(), content: n.text })
    this.write(n, args.text)
    return this.saved(n)
  }

  setField(id: string, edit: NoteFieldEdit, buffer?: { text: string; base_hash: string }): NoteWriteResult {
    const n = this.get(id)
    if (buffer) {
      if (hashText(n.text) !== buffer.base_hash) return { outcome: 'conflict', disk_text: n.text, disk_hash: hashText(n.text), disk_rev: this.fm(n).rev }
      n.text = buffer.text
    }
    if (edit.field === 'pinned') {
      n.pinned = edit.value
      this.write(n, n.text)
    } else if (edit.field === 'folder_id') {
      n.folder_id = edit.value
      this.write(n, n.text)
    } else {
      this.write(n, n.text, 'mode', (fm) => {
        if (edit.field === 'status') fm.status = edit.value
        else if (edit.field === 'urgent') fm.urgent = edit.value
        else if (edit.field === 'when') fm.when = edit.value === 'today' ? this.today : edit.value === 'tomorrow' ? addDays(this.today, 1) : edit.value
        else if (edit.field === 'tags') {
          const seen = new Set<string>()
          fm.tags = edit.value.filter((t) => {
            const k = normTag(t)
            if (!k || seen.has(k)) return false
            seen.add(k)
            return true
          })
        }
      })
    }
    return this.saved(n)
  }

  /** A change made outside Mode (another editor, a sync client): the watcher → reconcile → `notes.changed`. */
  externalEdit(id: string, text: string): void {
    const n = this.get(id)
    n.text = text
    n.revisions.push({ rev: this.fm(n).rev, origin: 'external', created_at: this.now(), content: text })
    this.changed()
  }

  /* ---- lists ------------------------------------------------------------- */

  foldersList(): FoldersList {
    const active = this.live().filter((n) => !n.archived_at)
    return {
      vault_name: 'Notes',
      vault_path: '~/Mode/Notes',
      root_count: active.filter((n) => n.folder_id === null).length,
      folders: this.folders.map((f) => ({ ...f, note_count: active.filter((n) => this.inFolder(n.folder_id, { id: f.id, recursive: true })).length })),
    }
  }

  groupingsList(): GroupingsList {
    return {
      groupings: this.groupings
        .slice()
        .sort((a, b) => a.position - b.position)
        .map((g) => ({ ...g, count: this.query(g.filter_json, []).length })),
      board: [...this.board],
      statuses: this.statuses.map((s) => ({ ...s })),
      today: this.today,
      week_start: this.weekStart,
    }
  }

  tagsList(): TagRow[] {
    const counts = new Map<string, number>()
    for (const n of this.live()) if (!n.archived_at) for (const t of this.fm(n).tags) counts.set(normTag(t), (counts.get(normTag(t)) ?? 0) + 1)
    return Array.from(this.tagNames.entries())
      .map(([norm, name]) => ({ id: `t-${norm}`, name, name_norm: norm, color: null, count: counts.get(norm) ?? 0 }))
      .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name))
  }

  saveGrouping(d: GroupingDraft): GroupingRow {
    const name = d.name.trim()
    if (!name) throw new Error('A grouping needs a name.')
    const clash = this.groupings.find((g) => g.name.toLowerCase() === name.toLowerCase() && g.id !== d.id)
    if (clash) throw new Error(`A grouping named ${clash.name} already exists.`)
    let g = d.id ? this.groupings.find((x) => x.id === d.id) : undefined
    if (!g) {
      const base = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'grouping'
      let slug = base
      for (let i = 2; this.groupings.some((x) => x.slug === slug); i++) slug = `${base}-${i}`
      g = {
        id: `g-${slug}`,
        slug,
        name,
        kind: 'custom',
        glyph: d.glyph ?? '▣',
        color: d.color ?? null,
        position: Math.max(...this.groupings.map((x) => x.position)) + 1,
        show_in_overlay: d.show_in_overlay ?? false,
        show_in_sidebar: d.show_in_sidebar ?? true,
        filter_json: d.filter_json,
        sort_json: d.sort_json ?? todaySort(),
      }
      this.groupings.push(g)
    } else {
      g.name = name
      if (d.glyph !== undefined) g.glyph = d.glyph
      if (d.color !== undefined) g.color = d.color
      if (g.kind === 'custom') g.filter_json = d.filter_json
      if (d.sort_json) g.sort_json = d.sort_json
      if (d.show_in_overlay !== undefined) g.show_in_overlay = d.show_in_overlay
      if (d.show_in_sidebar !== undefined) g.show_in_sidebar = d.show_in_sidebar
    }
    this.changed()
    return { ...g, count: this.query(g.filter_json, []).length }
  }

  list(args: NotesListArgs): { rows: NoteListRow[]; count: number } {
    let filter: FilterJson
    let sort: SortKey[] = [{ by: 'updated_at', dir: 'desc' }]
    if (args.scope.kind === 'folder') filter = { folder: { id: args.scope.folder_id, recursive: args.scope.recursive ?? true } }
    else if (args.scope.kind === 'grouping') {
      const slug = args.scope.slug
      const g = this.groupings.find((x) => x.slug === slug)
      if (!g) throw new Error(`No grouping ${slug}`)
      filter = g.filter_json
      sort = g.sort_json
    } else filter = args.scope.filter
    if (args.sort?.length) sort = args.sort
    let rows = this.query(filter, sort)
    if (args.show_archived && filter.archived === undefined) rows = this.sortRows([...rows, ...this.query({ ...filter, archived: true }, sort)], sort)
    const offset = args.offset ?? 0
    return { rows: rows.slice(offset, offset + (args.limit ?? 500)), count: rows.length }
  }

  /** `note_capture` (04 §7.2) for ⊕ Add task and the overlay's capture box. */
  capture(text: string): string {
    const [first, ...more] = text.split(/\r?\n/)
    const fm: Frontmatter = { id: null, status: 'todo', urgent: false, when: null, tags: [], created: null, updated: null, rev: 0, extra: [] }
    let folder = this.folders.find((f) => f.kind === 'capture')?.id ?? null
    const words: string[] = []
    const parts = first.split(/\s+/).filter(Boolean)
    parts.forEach((w, i) => {
      const esc = /^\\([#@/!])/.exec(w)
      if (esc) return void words.push(w.slice(1))
      let m: RegExpExecArray | null
      if ((m = /^#([^\s#,[\]"]{1,64})$/.exec(w))) return void fm.tags.push(m[1])
      if (w === '!' || (i === parts.length - 1 && w.endsWith('!') && w.length > 1 && /!$/.test(first.trimEnd()))) {
        fm.urgent = true
        if (w !== '!') words.push(w.slice(0, -1))
        return
      }
      if ((m = /^@(\S+)$/.exec(w))) {
        const v = m[1].toLowerCase()
        if (v === 'today') return void (fm.when = this.today)
        if (v === 'tomorrow') return void (fm.when = addDays(this.today, 1))
        if (isValidDate(v)) return void (fm.when = v)
        const wd = WEEKDAY_NAMES.findIndex((names) => names.includes(v))
        if (wd >= 0) return void (fm.when = addDays(this.today, (wd - weekday(this.today) + 7) % 7))
      }
      if ((m = /^\/([a-z][a-z0-9-]{0,31})$/.exec(w)) && this.statuses.some((s) => s.slug === m![1])) return void (fm.status = m[1])
      if ((m = /^in:(\S+)$/.exec(w))) {
        const f = this.folders.find((x) => x.path.toLowerCase() === m![1].toLowerCase())
        if (f) return void (folder = f.id)
      }
      words.push(w)
    })
    const title = words.join(' ').slice(0, 120) || 'Untitled'
    const n = this.create(folder, writeFrontmatter({ ...fm, id: 'pending' }) + `\n# ${title}\n${more.length ? `\n${more.join('\n')}\n` : ''}`, 'capture')
    return n.id
  }
}

const WEEKDAY_NAMES = [
  ['sun', 'sunday'],
  ['mon', 'monday'],
  ['tue', 'tues', 'tuesday'],
  ['wed', 'wednesday'],
  ['thu', 'thur', 'thurs', 'thursday'],
  ['fri', 'friday'],
  ['sat', 'saturday'],
]

/* ==== the chip resolver (04 §8; test scaffolding) ========================== */

const STOP_WORDS = new Set(['show', 'list', 'find', 'what', 'which', 'are', 'is', 'my', 'the', 'all', 'any', 'please', 'for', 'with', 'of', 'and', 'me', 'about'])
// `about` is not in 04 §8.2's list, but the worked example "archived notes about budget" needs it.
const PREPOSITIONS = new Set(['for', 'from', 'with', 'on', 'by', 'before', 'until', 'after', 'in', 'tag', 'tagged', 'repo', 'folder', 'next', 'this', 'not', 'due', 'past', 'no', 'to', 'claude', 'slack'])
const MONTHS = ['jan', 'feb', 'mar', 'apr', 'may', 'jun', 'jul', 'aug', 'sep', 'oct', 'nov', 'dec']

interface Tok {
  raw: string
  w: string
  quoted: boolean
  open: boolean
  start: number
}

function tokenize(text: string): Tok[] {
  const out: Tok[] = []
  const re = /"([^"]*)("?)|“([^”]*)(”?)|(\S+)/g
  let m: RegExpExecArray | null
  while ((m = re.exec(text))) {
    if (m[5] !== undefined) out.push({ raw: m[5], w: m[5] === '!' ? '!' : m[5].toLowerCase().replace(/[.,;?]+$/, ''), quoted: false, open: false, start: m.index })
    else {
      const inner = m[1] ?? m[3] ?? ''
      out.push({ raw: inner, w: inner.toLowerCase(), quoted: true, open: !(m[2] || m[4]), start: m.index })
    }
  }
  return out
}

type Chip = QueryChip
/** A chip before its origin is known (distributes over the kinds). */
type Made = Chip extends infer C ? (C extends unknown ? Omit<C, 'origin'> : never) : never

const STATUS_GLYPH: Record<string, string> = { todo: '○', doing: '◑', done: '✓' }
const TYPE_CHIP: Record<ItemType, { label: string; glyph: string }> = {
  tasks: { label: 'Tasks', glyph: '☐' },
  notes: { label: 'Notes', glyph: '≡' },
  messages: { label: 'Messages', glyph: '✉' },
  sessions: { label: 'Sessions', glyph: '▶' },
  calendar_events: { label: 'Events', glyph: '▦' },
}
const BUCKET_LABEL: Record<WhenBucket, string> = { overdue: 'Overdue', today: 'Today', tomorrow: 'Tomorrow', scheduled: 'Scheduled', none: 'Unscheduled' }

interface Phrase {
  words: string[]
  make: (v: NotesVault) => Made | 'stop'
  kind: ChipKind | 'stop'
}

function bucketChip(...b: WhenBucket[]): () => Made {
  return () => ({ kind: 'when', value: { in: b }, label: b.length === 1 ? BUCKET_LABEL[b[0]] : 'Due', glyph: '▦' })
}

function statusChip(slug: string, label: string): () => Made {
  return () => ({ kind: 'status', value: slug, label, glyph: slug === 'urgent' ? '✱' : slug === 'not-urgent' ? null : STATUS_GLYPH[slug] ?? null })
}

function typeChip(t: ItemType): () => Made {
  return () => ({ kind: 'type', value: t, ...TYPE_CHIP[t] })
}

function rangeChip(v: NotesVault, value: { from?: string; to?: string }): Made {
  const from = value.from ? resolveDateToken(value.from, v.today, v.weekStart) : null
  const to = value.to ? resolveDateToken(value.to, v.today, v.weekStart) : null
  return { kind: 'when', value, label: rangeLabel(from, to), glyph: '▦' }
}

function phrases(v: NotesVault): Phrase[] {
  const p = (s: string, kind: Phrase['kind'], make: Phrase['make']): Phrase => ({ words: s.split(' '), kind, make })
  const list: Phrase[] = [
    ...['today', 'tonight', 'due today'].map((s) => p(s, 'when', bucketChip('today'))),
    ...['tomorrow', 'tmrw', 'tmr'].map((s) => p(s, 'when', bucketChip('tomorrow'))),
    ...['overdue', 'late', 'past due'].map((s) => p(s, 'when', bucketChip('overdue'))),
    ...['scheduled', 'dated', 'with a date'].map((s) => p(s, 'when', bucketChip('scheduled'))),
    ...['unscheduled', 'no date', 'undated', 'someday'].map((s) => p(s, 'when', bucketChip('none'))),
    ...['due', 'upcoming'].map((s) => p(s, 'when', bucketChip('overdue', 'today', 'tomorrow', 'scheduled'))),
    p('yesterday', 'when', (x) => rangeChip(x, { from: 'today-1', to: 'today-1' })),
    p('this week', 'when', (x) => rangeChip(x, { from: 'week_start', to: 'week_end' })),
    p('next week', 'when', (x) => {
      const s = addDays(weekStartOf(x.today, x.weekStart), 7)
      return rangeChip(x, { from: s, to: addDays(s, 6) })
    }),
    p('this month', 'when', (x) => rangeChip(x, { from: 'month_start', to: 'month_end' })),
    p('next month', 'when', (x) => {
      const s = addDays(monthEnd(x.today), 1)
      return rangeChip(x, { from: monthStart(s), to: monthEnd(s) })
    }),
    ...['todo', 'to do', 'to-do', 'open', 'not started'].map((s) => p(s, 'status', statusChip('todo', 'Todo'))),
    ...['doing', 'in progress', 'wip', 'started'].map((s) => p(s, 'status', statusChip('doing', 'Doing'))),
    ...['done', 'completed', 'finished', 'closed'].map((s) => p(s, 'status', statusChip('done', 'Done'))),
    ...['urgent', 'flagged', 'important', '!'].map((s) => p(s, 'status', statusChip('urgent', 'Urgent'))),
    ...['not urgent', 'unflagged'].map((s) => p(s, 'status', statusChip('not-urgent', 'Not urgent'))),
    ...['tasks', 'task', 'todos', 'to-dos', 'items', 'things'].map((s) => p(s, 'type', typeChip('tasks'))),
    ...['notes', 'note', 'everything'].map((s) => p(s, 'type', typeChip('notes'))),
    ...['messages', 'emails', 'mail', 'slack messages', 'dms'].map((s) => p(s, 'type', typeChip('messages'))),
    ...['sessions', 'claude sessions'].map((s) => p(s, 'type', typeChip('sessions'))),
    ...['events', 'meetings', 'calendar'].map((s) => p(s, 'type', typeChip('calendar_events'))),
    p('pinned', 'flag', () => ({ kind: 'flag', value: 'pinned', label: 'Pinned', glyph: '⌖' })),
    ...['archived', 'in archive'].map((s) => p(s, 'flag', () => ({ kind: 'flag', value: 'archived', label: 'Archived', glyph: '▫' }))),
    p('captured', 'flag', () => ({ kind: 'flag', value: 'source:capture', label: 'Captured', glyph: null })),
    p('from messages', 'flag', () => ({ kind: 'flag', value: 'source:message', label: 'From messages', glyph: null })),
    p('from sessions', 'flag', () => ({ kind: 'flag', value: 'source:session', label: 'From sessions', glyph: null })),
    p('imported', 'flag', () => ({ kind: 'flag', value: 'source:import', label: 'Imported', glyph: null })),
    ...['show me', 'show', 'list', 'find', 'what', 'which', 'are', 'is', 'my', 'the', 'all', 'any', 'please', 'for', 'with', 'of', 'and', 'about'].map((s) => p(s, 'stop', () => 'stop')),
  ]
  for (const s of v.statuses) {
    if (s.slug in STATUS_GLYPH) continue
    list.push(p(s.slug, 'status', () => ({ kind: 'status', value: s.slug, label: s.label, glyph: s.glyph })))
    if (s.label.toLowerCase() !== s.slug) list.push(p(s.label.toLowerCase(), 'status', () => ({ kind: 'status', value: s.slug, label: s.label, glyph: s.glyph })))
  }
  return list.sort((a, b) => b.words.length - a.words.length)
}

function personChip(c: Contact): Made {
  return { kind: 'person', value: { contact_id: c.id }, label: c.display_name, initials: c.initials, glyph: null }
}

function contactsFor(v: NotesVault, name: string): Contact[] {
  const n = name.toLowerCase()
  const exact = v.contacts.filter((c) => c.display_name.toLowerCase() === n || c.display_name.toLowerCase().split(' ')[0] === n)
  if (exact.length) return exact
  return n.length >= 2 ? v.contacts.filter((c) => c.display_name.toLowerCase().startsWith(n)) : []
}

function tagChip(v: NotesVault, norm: string): Made {
  return { kind: 'tag', value: norm, label: v.tagNames.get(norm) ?? norm, glyph: '#' }
}

function repoChip(root: string): Made {
  return { kind: 'repo', value: { repo_root: root }, label: root.split('/').pop() ?? root, glyph: '⎇' }
}

function folderChip(f: { id: string; name: string }): Made {
  return { kind: 'folder', value: { id: f.id }, label: f.name, glyph: '▸' }
}

function parseDateAt(v: NotesVault, toks: Tok[], i: number): { date: string; len: number } | null {
  const a = toks[i]?.w
  if (!a) return null
  if (isValidDate(a)) return { date: a, len: 1 }
  const year = v.today.slice(0, 4)
  const md = /^(\d{1,2})\/(\d{1,2})$/.exec(a)
  if (md) {
    const d = `${year}-${md[1].padStart(2, '0')}-${md[2].padStart(2, '0')}`
    return isValidDate(d) ? { date: d, len: 1 } : null
  }
  const month = (w: string | undefined) => (w ? MONTHS.findIndex((m) => w.startsWith(m) && w.length <= 9) : -1)
  const b = toks[i + 1]?.w
  let mi = month(a)
  if (mi >= 0 && b && /^\d{1,2}$/.test(b)) {
    const d = `${year}-${String(mi + 1).padStart(2, '0')}-${b.padStart(2, '0')}`
    return isValidDate(d) ? { date: d, len: 2 } : null
  }
  mi = month(b)
  if (/^\d{1,2}$/.test(a) && mi >= 0) {
    const d = `${year}-${String(mi + 1).padStart(2, '0')}-${a.padStart(2, '0')}`
    return isValidDate(d) ? { date: d, len: 2 } : null
  }
  const wd = WEEKDAY_NAMES.findIndex((names) => names.includes(a))
  if (wd >= 0) return { date: addDays(v.today, (wd - weekday(v.today) + 7) % 7), len: 1 }
  return null
}

interface Match {
  len: number
  chip?: Made
  stop?: boolean
  or?: boolean
  /** A longer phrase could still start here (the field is not final). */
  extendable?: boolean
}

function matchAt(v: NotesVault, toks: Tok[], i: number, table: Phrase[]): Match | null {
  const t = toks[i]
  if (t.quoted) return { len: 1, chip: { kind: 'text', value: t.raw, label: t.raw, glyph: null } }
  const w = t.w

  // 1 · explicit prefixes (04 §8.3: an explicit prefix wins)
  let m: RegExpExecArray | null
  if ((m = /^#([^\s#,[\]"]{1,64})$/.exec(t.raw))) return { len: 1, chip: tagChip(v, normTag(m[1])) }
  if ((m = /^@(.+)$/.exec(t.raw))) {
    const cs = contactsFor(v, m[1])
    return cs.length === 1 ? { len: 1, chip: personChip(cs[0]) } : null
  }
  if ((m = /^in:(.+)$/i.exec(t.raw))) {
    const f = v.folders.find((x) => x.path.toLowerCase() === m![1].toLowerCase() || x.name.toLowerCase() === m![1].toLowerCase())
    return f ? { len: 1, chip: folderChip(f) } : null
  }
  if ((m = /^repo:(.+)$/i.exec(t.raw))) {
    const r = v.repos.find((x) => x.split('/').pop()?.toLowerCase() === m![1].toLowerCase())
    return r ? { len: 1, chip: repoChip(r) } : null
  }
  if ((m = /^(is|status|when|type):(.+)$/i.exec(t.raw))) {
    const inner = matchAt(v, [{ ...t, raw: m[2], w: m[2].toLowerCase() }], 0, table)
    return inner?.chip ? { len: 1, chip: inner.chip } : null
  }

  // 2 · possessive: Bjorn's
  if ((m = /^(.+?)['’]s$/i.exec(t.raw))) {
    const cs = contactsFor(v, m[1])
    if (cs.length === 1) return { len: 1, chip: personChip(cs[0]) }
  }

  // 3 · lexicon phrases, longest first
  for (const ph of table) {
    const n = ph.words.length
    if (i + n > toks.length) continue
    if (ph.words.every((pw, k) => toks[i + k].w === pw && !toks[i + k].quoted)) {
      const made = ph.make(v)
      const longer = table.some((o) => o.words.length > n && o.words.slice(0, n).every((pw, k) => pw === ph.words[k]))
      if (made === 'stop') {
        // "for Bjorn" / "with Ana": a stop word that introduces a person
        if ((w === 'for' || w === 'with') && toks[i + 1]) {
          const cs = contactsFor(v, toks[i + 1].w.replace(/['’]s$/, ''))
          if (cs.length === 1) return { len: 2, chip: personChip(cs[0]) }
        }
        return { len: n, stop: true, extendable: longer || w === 'for' || w === 'with' }
      }
      return { len: n, chip: made, extendable: longer }
    }
  }
  if (w === 'or') return { len: 1, or: true }

  // 4 · dates
  const num = toks[i + 2] && /^(next|in)$/.test(w) && /^\d{1,3}$/.test(toks[i + 1].w) && /^days?$/.test(toks[i + 2].w)
  if (num) {
    const n = Number(toks[i + 1].w)
    return { len: 3, chip: rangeChip(v, w === 'next' ? { from: 'today', to: `today+${n}` } : { from: `today+${n}`, to: `today+${n}` }) }
  }
  if (/^(on|by|before|until|after|from)$/.test(w)) {
    const d = parseDateAt(v, toks, i + 1)
    if (d) {
      const value = w === 'on' ? { from: d.date, to: d.date } : /^(by|before|until)$/.test(w) ? { to: d.date } : { from: d.date }
      return { len: 1 + d.len, chip: rangeChip(v, value) }
    }
    if (w === 'from' && toks[i + 1]) {
      const cs = contactsFor(v, toks[i + 1].w)
      if (cs.length === 1) return { len: 2, chip: personChip(cs[0]) }
    }
  }
  const date = parseDateAt(v, toks, i)
  if (date) return { len: date.len, chip: { ...rangeChip(v, { from: date.date, to: date.date }), label: shortDate(date.date) }, extendable: date.len === 1 && /^\d{1,2}$/.test(w) }

  // 5 · prepositions + entity: in mode, tag gym, repo docs-site, folder Mode
  const next = toks[i + 1]
  if (next && /^(in|tag|tagged|repo|folder)$/.test(w)) {
    const target = next.w
    if (w === 'tag' || w === 'tagged') return v.tagNames.has(normTag(target)) ? { len: 2, chip: tagChip(v, normTag(target)) } : null
    const repo = v.repos.find((r) => r.split('/').pop()?.toLowerCase() === target)
    if (repo && w !== 'folder') return { len: 2, chip: repoChip(repo) }
    const folder = folderMatch(v, toks, i + 1)
    if (folder && w !== 'repo') return { len: 1 + folder.len, chip: folderChip(folder.f) }
  }

  // 6 · bare entities: tag > folder > contact > repo
  if (v.tagNames.has(normTag(t.raw))) return { len: 1, chip: tagChip(v, normTag(t.raw)) }
  const folder = folderMatch(v, toks, i)
  if (folder) return { len: folder.len, chip: folderChip(folder.f) }
  const cs = contactsFor(v, w)
  if (cs.length === 1 && cs[0].display_name.toLowerCase() === w) return { len: 1, chip: personChip(cs[0]) }
  const repo = v.repos.find((r) => r.split('/').pop()?.toLowerCase() === w)
  if (repo) return { len: 1, chip: repoChip(repo) }
  return null
}

function folderMatch(v: NotesVault, toks: Tok[], i: number): { f: Omit<FolderRow, 'note_count'>; len: number } | null {
  let best: { f: Omit<FolderRow, 'note_count'>; len: number } | null = null
  for (const f of v.folders) {
    const words = f.name.toLowerCase().split(/\s+/)
    if (i + words.length > toks.length) continue
    if (words.every((fw, k) => toks[i + k].w === fw) && (!best || words.length > best.len)) best = { f, len: words.length }
  }
  return best
}

/** Adds a chip with the merge rules of 04 §8.3; the earlier position is kept. */
export function addChip(list: Chip[], c: Chip, or = false): Chip[] {
  const out = [...list]
  const same = (k: ChipKind) => out.findIndex((x) => x.kind === k)
  switch (c.kind) {
    case 'text': {
      const i = same('text')
      if (i >= 0) {
        const merged = `${(out[i] as Extract<Chip, { kind: 'text' }>).value} ${c.value}`.trim()
        out[i] = { ...c, value: merged, label: merged }
      } else out.push(c)
      return out
    }
    case 'when': {
      const i = same('when')
      if (i < 0) return [...out, c]
      const prev = out[i] as Extract<Chip, { kind: 'when' }>
      if ('in' in prev.value && 'in' in c.value) {
        const union = Array.from(new Set([...prev.value.in, ...c.value.in]))
        out[i] = { ...prev, value: { in: union }, label: union.map((b) => BUCKET_LABEL[b]).join(' or ') }
      } else out[i] = c
      return out
    }
    case 'person':
    case 'repo':
    case 'folder':
    case 'type': {
      const i = same(c.kind)
      if (i >= 0) out[i] = c
      else out.push(c)
      return out
    }
    case 'tag': {
      const last = out[out.length - 1]
      if (or && last?.kind === 'tag') {
        const prev = typeof last.value === 'string' ? [last.value] : last.value.any
        const add = typeof c.value === 'string' ? [c.value] : c.value.any
        const any = Array.from(new Set([...prev, ...add]))
        out[out.length - 1] = { ...last, value: { any }, label: any.join(' or #'), glyph: '#' }
        return out
      }
      if (out.some((x) => x.kind === 'tag' && JSON.stringify(x.value) === JSON.stringify(c.value))) return out
      return [...out, c]
    }
    default:
      if (out.some((x) => x.kind === c.kind && JSON.stringify(x.value) === JSON.stringify(c.value))) return out
      return [...out, c]
  }
}

function originFor(ctx: QueryContextArgs): QueryChip['origin'] {
  return ctx.surface === 'refine' ? 'refine' : 'typed'
}

export function resolveQuery(v: NotesVault, text: string, ctx: QueryContextArgs): QueryResolveResult {
  const table = phrases(v)
  const toks = tokenize(text)
  const origin = originFor(ctx)
  let chips: Chip[] = [...ctx.chips]
  let partial: Tok | null = null
  let completed = toks
  if (!ctx.final && toks.length && (!/\s$/.test(text) || toks[toks.length - 1].open)) {
    partial = toks[toks.length - 1]
    completed = toks.slice(0, -1)
  }
  const residual: Tok[] = []
  let i = 0
  let stopStart = -1
  let orPending = false
  /** First token that stays typed (preceding stop words included). */
  let restAt = -1
  /** First token of a phrase still being typed ("next w"), for the suggestions. */
  let phraseAt = -1
  while (i < completed.length) {
    const m = matchAt(v, completed, i, table)
    if (!ctx.final && m && i + m.len === completed.length && (m.extendable || (m.len === 1 && PREPOSITIONS.has(completed[i].w) && !m.chip))) {
      restAt = stopStart >= 0 ? stopStart : i
      phraseAt = i
      break
    }
    if (m?.chip) {
      chips = addChip(chips, { ...m.chip, origin } as Chip, orPending)
      orPending = false
      stopStart = -1
      i += m.len
      continue
    }
    if (m?.or) {
      orPending = true
      i += 1
      continue
    }
    if (m?.stop) {
      if (stopStart < 0) stopStart = i
      i += m.len
      continue
    }
    if (!ctx.final) {
      restAt = stopStart >= 0 ? stopStart : i
      if (i === completed.length - 1 && PREPOSITIONS.has(completed[i].w)) phraseAt = i
      break
    }
    residual.push(completed[i])
    orPending = false
    i++
  }
  if (ctx.final && residual.length) chips = addChip(chips, { kind: 'text', value: residual.map((t) => t.raw).join(' '), label: residual.map((t) => t.raw).join(' '), glyph: null, origin })
  if (ctx.final) return { chips, rest: '', suggestions: [] }

  if (restAt < 0 && stopStart >= 0) restAt = stopStart
  const restOffset = restAt >= 0 ? completed[restAt].start : partial ? partial.start : text.length
  const rest = text.slice(restOffset)
  // The phrase being completed: a deferred phrase ("next w"), else the partial word.
  const phraseOffset = phraseAt >= 0 ? completed[phraseAt].start : partial ? partial.start : text.length
  const phrase = text.slice(phraseOffset)
  let keep = text.slice(restOffset, phraseOffset)
  if (tokenize(keep).every((t) => STOP_WORDS.has(t.w))) keep = ''
  return { chips, rest, suggestions: suggest(v, phrase, chips, keep, table, origin) }
}

function suggest(v: NotesVault, phrase: string, chips: Chip[], keep: string, table: Phrase[], origin: QueryChip['origin']): QuerySuggestion[] {
  const p = phrase.trim().toLowerCase()
  if (!p) return []
  const out: QuerySuggestion[] = []
  const seen = new Set<string>()
  const add = (made: Made, hint: string, typed: string) => {
    const key = `${made.kind}:${JSON.stringify(made.value)}`
    if (seen.has(key) || out.length >= 6) return
    seen.add(key)
    const label = made.label
    const completion = typed && label.toLowerCase().startsWith(typed) && label.length > typed.length ? label.slice(typed.length) : null
    out.push({ id: key, label, glyph: made.glyph, hint, completion, apply: { chips: addChip(chips, { ...made, origin: 'autocomplete' } as Chip), rest: keep } })
  }
  const prefixed = /^(#|@|in:|repo:|is:|status:|when:|type:)(.*)$/.exec(p)
  if (prefixed) {
    const [, pre, q] = prefixed
    if (pre === '#') for (const t of v.tagsList()) if (t.name_norm.startsWith(q)) add(tagChip(v, t.name_norm), `${t.count} ${t.count === 1 ? 'note' : 'notes'}`, q)
    if (pre === '@') for (const c of v.contacts) if (c.display_name.toLowerCase().startsWith(q)) add(personChip(c), 'person', q)
    if (pre === 'in:') for (const f of v.folders) if (f.name.toLowerCase().startsWith(q) || f.path.toLowerCase().startsWith(q)) add(folderChip(f), 'folder', q)
    if (pre === 'repo:') for (const r of v.repos) if ((r.split('/').pop() ?? '').startsWith(q)) add(repoChip(r), 'repo', q)
    if (pre === 'is:' || pre === 'status:') {
      for (const s of v.statuses) if (s.slug.startsWith(q) || s.label.toLowerCase().startsWith(q)) add({ kind: 'status', value: s.slug, label: s.label, glyph: s.glyph }, 'status', q)
      if ('urgent'.startsWith(q)) add(statusChip('urgent', 'Urgent')(), 'status', q)
    }
    if (pre === 'when:') for (const s of ['today', 'tomorrow', 'overdue', 'scheduled', 'unscheduled', 'this week', 'next week']) if (s.startsWith(q)) add(table.find((ph) => ph.words.join(' ') === s)!.make(v) as Made, 'when', q)
    if (pre === 'type:') for (const t of ['tasks', 'notes'] as ItemType[]) if (t.startsWith(q)) add(typeChip(t)(), 'type', q)
    return out
  }
  for (const ph of table) {
    if (ph.kind === 'stop') continue
    const s = ph.words.join(' ')
    if (p.length >= 2 && s.startsWith(p)) add(ph.make(v) as Made, ph.kind, p)
  }
  for (const t of v.tagsList()) if (p.length >= 2 && t.name_norm.startsWith(p)) add(tagChip(v, t.name_norm), 'tag', p)
  for (const f of v.folders) if (p.length >= 2 && f.name.toLowerCase().startsWith(p)) add(folderChip(f), 'folder', p)
  for (const c of v.contacts) if (p.length >= 2 && c.display_name.toLowerCase().startsWith(p)) add(personChip(c), 'person', p)
  for (const r of v.repos) if (p.length >= 2 && (r.split('/').pop() ?? '').startsWith(p)) add(repoChip(r), 'repo', p)
  const term = phrase.trim()
  if (out.length >= 6) out.length = 5
  out.push({
    id: `text:${term}`,
    label: `Search for “${term}”`,
    glyph: null,
    hint: 'text',
    completion: null,
    apply: { chips: addChip(chips, { kind: 'text', value: term, label: term, glyph: null, origin }), rest: keep },
  })
  return out
}

/** Chips → `filter_json` (04 §8.4). Bucket and range `when` chips keep their relative tokens. */
export function compileChips(chips: readonly QueryChip[]): FilterJson {
  const f: FilterJson = {}
  const statusIn: string[] = []
  const all: string[] = []
  const any: string[] = []
  for (const c of chips) {
    switch (c.kind) {
      case 'status':
        if (c.value === 'urgent') f.urgent = true
        else if (c.value === 'not-urgent') f.urgent = false
        else if (!statusIn.includes(c.value)) statusIn.push(c.value)
        break
      case 'type':
        if (c.value === 'tasks') f.status = { ...f.status, not_in: ['closed'] }
        break
      case 'when':
        f.when = c.value as WhenFilter
        break
      case 'tag':
        if (typeof c.value === 'string') all.push(c.value)
        else any.push(...c.value.any)
        break
      case 'person':
        f.person = c.value
        break
      case 'repo':
        f.repo = c.value
        break
      case 'folder':
        f.folder = { id: c.value.id, recursive: true }
        break
      case 'text':
        f.text = f.text ? `${f.text} ${c.value}` : c.value
        break
      case 'flag':
        if (c.value === 'pinned') f.pinned = true
        else if (c.value === 'archived') f.archived = true
        else {
          const kind = c.value.slice('source:'.length) as SourceKind
          f.source_kind = { in: [...(f.source_kind?.in ?? []), kind] }
        }
        break
    }
  }
  if (statusIn.length) f.status = { ...f.status, in: statusIn }
  if (all.length || any.length) f.tags = { ...(all.length ? { all } : {}), ...(any.length ? { any } : {}) }
  return f
}

function itemType(chips: readonly QueryChip[], fallback: ItemType = 'notes'): ItemType {
  const t = chips.find((c) => c.kind === 'type')
  return t && t.kind === 'type' ? t.value : fallback
}

export function runQuery(v: NotesVault, args: QueryRunArgs): QueryRunResult {
  const filter = compileChips(args.chips)
  const type = itemType(args.chips)
  if (type !== 'notes' && type !== 'tasks') return { rows: [], count: 0, filter, type }
  const rows = v.query(filter, args.sort?.length ? args.sort : todaySort())
  return { rows: rows.slice(0, args.limit ?? 50), count: rows.length, filter, type }
}

/* ==== registration ========================================================= */

declare global {
  interface Window {
    /** The sample vault behind the notes mock (tests drive external edits through it). */
    __mode_notes_mock?: NotesVault
  }
}

type Args = Record<string, unknown> | undefined

/**
 * Registers every notes command on the browser mock (`window.__mode_mock`,
 * `respond`), overriding its plain `note_capture`. Outside Tauri only; call
 * again after `__mode_mock.reset()`. Returns the vault for tests.
 */
export function installNotesMock(opts: NotesMockOptions = {}): NotesVault {
  const v = new NotesVault(opts)
  const m = modeMock()
  const on = <A,>(cmd: string, fn: (a: A) => unknown) => m.respond(cmd, (args: Args) => fn((args ?? {}) as A))
  on('notes_list', (a) => v.list(a as unknown as NotesListArgs))
  on('note_get', (a: { note_id: string }) => v.detail(v.get(a.note_id)))
  on('note_create', (a: { folder_id: string | null; text: string }) => v.detail(v.create(a.folder_id, a.text)))
  on('note_save', (a: { note_id: string; text: string; base_hash: string; resolve?: 'mine' | 'theirs' | 'both' }) => v.save(a))
  on('note_set_field', (a: { note_id: string; edit: NoteFieldEdit; buffer?: { text: string; base_hash: string } }) => v.setField(a.note_id, a.edit, a.buffer))
  on('note_cycle_status', (a: { note_id: string }) => v.setField(a.note_id, { field: 'status', value: nextStatus(v.row(v.get(a.note_id)).status) }))
  on('note_archive', (a: { note_id: string; archived: boolean }) => {
    const n = v.get(a.note_id)
    n.archived_at = a.archived ? v.now() : null
    return v.setField(a.note_id, { field: 'pinned', value: n.pinned })
  })
  on('note_trash', (a: { note_id: string }) => {
    v.get(a.note_id).deleted = true
    if (v.emit) setTimeout(() => m.emit(EVENTS.notes, v.overlayPayload()), 0)
  })
  on('note_untrash', (a: { note_id: string }) => {
    const n = v.notes.get(a.note_id)
    if (!n) throw new Error(`No note ${a.note_id}`)
    n.deleted = false
    return v.setField(a.note_id, { field: 'pinned', value: n.pinned })
  })
  on('folders_list', () => v.foldersList())
  on('groupings_list', () => v.groupingsList())
  on('grouping_save', (a: { grouping: GroupingDraft }) => v.saveGrouping(a.grouping))
  on('tags_list', () => v.tagsList())
  on('query_resolve', (a: { text: string; context: QueryContextArgs }) => resolveQuery(v, a.text, a.context))
  on('query_run', (a) => runQuery(v, a as unknown as QueryRunArgs))
  on('note_history', (a: { note_id: string }) =>
    v
      .get(a.note_id)
      .revisions.slice()
      .reverse()
      .map((r): RevisionRow => ({ rev: r.rev, origin: r.origin, created_at: r.created_at, device_id: 'dev-this', device_name: 'This Mac', bytes: r.content.length })),
  )
  on('note_revision_get', (a: { note_id: string; rev: number }) => {
    const r = [...v.get(a.note_id).revisions].reverse().find((x) => x.rev === a.rev)
    if (!r) throw new Error(`No revision ${a.rev}`)
    return { rev: r.rev, content: r.content }
  })
  on('note_restore', (a: { note_id: string; rev: number }) => {
    const n = v.get(a.note_id)
    const r = [...n.revisions].reverse().find((x) => x.rev === a.rev)
    if (!r) throw new Error(`No revision ${a.rev}`)
    const res = v.save({ note_id: n.id, text: r.content, base_hash: hashText(n.text) })
    n.revisions[n.revisions.length - 1].origin = 'restore'
    return res
  })
  on('note_link_add', (a: { note_id: string; kind: NoteLink['kind']; target_id: string; link_type: NoteLink['link_type']; label?: string }) => {
    const n = v.get(a.note_id)
    const link: NoteLink = {
      id: `l-${n.id}-${n.links.length + 1}`,
      link_type: a.link_type,
      direction: 'from',
      kind: a.kind,
      target_id: a.target_id,
      label: a.label ?? null,
      title: a.kind === 'contact' ? v.contacts.find((c) => c.id === a.target_id)?.display_name ?? null : a.kind === 'url' ? a.label ?? a.target_id : null,
      available: true,
    }
    n.links.push(link)
    if (a.kind === 'contact') n.people.push(a.target_id)
    return link
  })
  on('note_capture', (a: { text: string }) => ({ note_id: v.capture(a.text) }))
  if (typeof window !== 'undefined') window.__mode_notes_mock = v
  return v
}
