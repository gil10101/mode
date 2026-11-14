// The browser mock of package 11 (v1.0): Chat, Automations, theme packs and
// the command palette, registered on `window.__mode_mock` outside Tauri (test
// and dev scaffolding only; the core implements the same commands). Seeded
// at the wireframes' clock: two conversations (the app-window.html thread
// with a reply still streaming, and one with inline citations and a pending
// action), three automations with runs, the built-in packs of 10 §5 with
// their real token maps, and a palette index over the other mocks' data.
// `window.__mode_v1` lets tests switch states.
import { mock } from '../tauri'
import { EVENTS } from '../state/contract'
import {
  CHAT_EVENTS,
  type ActivityGroup,
  type ChatAction,
  type ChatConnectors,
  type ChatSources,
  type ChatStatus,
  type ChatThread,
  type ChatThreadRow,
  type ChatTurn,
  type Citation,
  type Effort,
  type ItemRef,
  type TokenUsage,
  type ToolCall,
} from '../state/contract-chat'
import {
  AUTOMATION_EVENTS,
  type ActionJson,
  type ActionKind,
  type AutomationDraft,
  type AutomationRow,
  type CreateNoteAction,
  type FieldError,
  type RunDetail,
  type RunRow,
  type Schedule,
  type SchedulePreview,
  type StartSessionAction,
} from '../state/contract-automations'
import { THEME_EVENTS, type AppearancePref, type PackAppearance, type PackIssue, type ResolvedTheme, type Swatch, type ThemeImportResult, type ThemeRow } from '../state/contract-themes'
import type { PaletteKind, PaletteResult } from '../state/palette-store'
import { actionPhrase, deviceZone, exampleDraft, firstWords, triggerPhrase, validateCron, validateDraft, wallOf } from '../state/automations-store'
import { AGENT_EVENTS, type AgentDraft, type AgentSummary, type AgentView, type MemoryRow } from '../state/contract-agents'
import type { Catalogue, InstalledItem, ItemPreview, PackImportResult } from '../state/contract-packs'
import { emptyDraft as emptyAgentDraft, validateDraft as validateAgentDraft } from '../state/agents-store'
import type { NotesVault } from '../notes/mock'
import { sampleDetail, sampleFinishedDetail } from './mock-data'

type Args = Record<string, unknown> | undefined

const HOME = '/Users/ana'
const MODE_REPO = `${HOME}/code/mode`
const iso = (ms: number) => new Date(ms).toISOString()

function cmdError(code: string, message: string, fields?: FieldError[]) {
  return fields ? { code, message, fields } : { code, message }
}

/* ======================================================================================
   theme packs (10 §4.2, §5.9–§5.11)
   ====================================================================================== */

const ROLES = ['bg', 'surface', 'sunken', 'hover', 'selected', 'line', 'line-strong', 'fg', 'fg-2', 'fg-3', 'accent', 'accent-bg', 'ok', 'bad', 'info'] as const
const ANSI = [
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'bright-black',
  'bright-red',
  'bright-green',
  'bright-yellow',
  'bright-blue',
  'bright-magenta',
  'bright-cyan',
  'bright-white',
] as const

const FONT_UI = '-apple-system, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif'
const FONT_MONO = '"SF Mono", Menlo, "Cascadia Mono", Consolas, "DejaVu Sans Mono", ui-monospace, monospace'
const PAPER_MONO = '"SF Mono", Menlo, "Cascadia Mono", Consolas, ui-monospace, monospace'
const SHADOW = { light: '0 8px 24px rgba(0,0,0,.14)', dark: '0 8px 24px rgba(0,0,0,.5)' }

interface ColorSet {
  /** The fifteen roles in `ROLES` order. */
  m: string[]
  avatar: string[]
  /** 16 ANSI, then fg / bg / cursor / selection. */
  term: string[]
}

interface Pack {
  id: string
  slug: string
  name: string
  kind: ThemeRow['kind']
  appearance: PackAppearance
  author: string
  version: string
  mono: string
  light?: ColorSet
  dark?: ColorSet
  document?: unknown
  deleted?: boolean
}

const w = (s: string) => s.split(/\s+/).filter(Boolean)

/** §5.9 columns, §5.10 hues, §5.11 palettes; Paper per §4.2 (its dark set is the §3.2 dark column). */
const SETS: Record<string, ColorSet> = {
  'system-light': {
    m: w('#F5F5F7 #FFFFFF #ECECEE #EAEAEC #E4E4E8 #D9D9DE #B9B9BE #1D1D1F #515154 #6E6E73 #0066CC #E5F0FF #1F8A3B #C62828 #0066CC'),
    avatar: w('#E0553F #E07B2E #C99A1E #7FA832 #2FA36B #1F9E98 #2389C9 #4D6FE3 #7B5CD6 #B04FBF #D04C98 #D2546A'),
    term: w('#000000 #C91B00 #00C200 #C7C400 #0225C7 #CA30C7 #00C5C7 #C7C7C7 #686868 #FF6E67 #5FFA68 #FFFC67 #6871FF #FF77FF #60FDFF #FFFFFF #000000 #FFFFFF #4D4D4D #B4D5FF'),
  },
  'system-dark': {
    m: w('#1E1E20 #2A2A2D #161618 #333336 #3D3D41 #3A3A3E #525257 #F5F5F7 #C7C7CC #98989D #0A84FF #1C2E4A #30D158 #FF453A #0A84FF'),
    avatar: w('#FF6B5B #FF9452 #E6B84D #9ACB5B #5CC28E #4FC2BA #5BA9E6 #7A94F2 #A08AE8 #C97CD9 #E27AB5 #EE7B8D'),
    term: w('#0C0C0C #C50F1F #13A10E #C19C00 #0037DA #881798 #3A96DD #CCCCCC #767676 #E74856 #16C60C #F9F1A5 #3B78FF #B4009E #61D6D6 #F2F2F2 #CCCCCC #0C0C0C #FFFFFF #264F78'),
  },
  'paper-light': {
    m: w('#F3F1EC #FAF9F6 #ECE9E2 #EFECE6 #E9E5DC #E1DDD4 #C9C4B8 #1E1D1A #5B5952 #8A877E #D9480F #FBECE3 #2F7D4A #B3261E #35669A'),
    avatar: w('#C0392B #D35400 #B9770E #7D8C2A #3F7D4E #2E8B7A #2F6F9F #4F5FA8 #6E4FA3 #8E4A8E #B0497A #B85C5C'),
    term: w('#1E1D1A #B3261E #2F7D4A #B9770E #35669A #7B4EA3 #2E8B7A #C9C4B8 #8A877E #E74C3C #4E9A5F #D28A1A #3F86BD #9366C0 #3AA58F #FAF9F6 #1E1D1A #FAF9F6 #D9480F #E9E5DC'),
  },
  'paper-dark': {
    m: w('#1A1917 #22211E #141311 #2A2925 #33312C #3A3833 #58554D #ECEAE4 #B3B0A7 #85827A #F0652A #3A2418 #5DB37A #E5635A #6E9FD1'),
    avatar: w('#FF6B5B #FF9452 #E6B84D #9ACB5B #5CC28E #4FC2BA #5BA9E6 #7A94F2 #A08AE8 #C97CD9 #E27AB5 #EE7B8D'),
    term: w('#1A1917 #E5635A #5DB37A #D9A441 #6E9FD1 #B48EAD #5FB3A3 #B3B0A7 #85827A #F2857C #7FCB97 #E8BC63 #8FB6E0 #C9A7C4 #7FC7BA #ECEAE4 #ECEAE4 #1A1917 #F0652A #33312C'),
  },
  'gruvbox-dark': {
    m: w('#282828 #32302f #1d2021 #3c3836 #504945 #3c3836 #665c54 #ebdbb2 #d5c4a1 #a89984 #fe8019 #3d2a1c #b8bb26 #fb4934 #83a598'),
    avatar: w('#fb4934 #fe8019 #fabd2f #b8bb26 #8ec07c #83a598 #d3869b #cc241d #d65d0e #d79921 #98971a #689d6a'),
    term: w('#282828 #cc241d #98971a #d79921 #458588 #b16286 #689d6a #a89984 #928374 #fb4934 #b8bb26 #fabd2f #83a598 #d3869b #8ec07c #ebdbb2 #ebdbb2 #282828 #ebdbb2 #504945'),
  },
  'gruvbox-light': {
    m: w('#fbf1c7 #f9f5d7 #f2e5bc #ebdbb2 #d5c4a1 #ebdbb2 #bdae93 #3c3836 #504945 #665c54 #af3a03 #f2dcc9 #79740e #9d0006 #076678'),
    avatar: w('#9d0006 #af3a03 #b57614 #79740e #427b58 #076678 #8f3f71 #cc241d #d65d0e #d79921 #98971a #689d6a'),
    term: w('#fbf1c7 #cc241d #98971a #d79921 #458588 #b16286 #689d6a #7c6f64 #928374 #9d0006 #79740e #b57614 #076678 #8f3f71 #427b58 #3c3836 #3c3836 #fbf1c7 #3c3836 #d5c4a1'),
  },
  'cursor-dark': {
    m: w('#171717 #1F1F1F #111111 #262626 #2A2D35 #2E2E2E #3D3D3D #E6E6E6 #B3B3B3 #8C8C8C #F0883E #3A2A1C #3FB950 #F85149 #5B8DEF'),
    avatar: w('#F87171 #FB923C #FACC15 #A3E635 #34D399 #2DD4BF #38BDF8 #818CF8 #A78BFA #E879F9 #F472B6 #FB7185'),
    term: w('#171717 #F85149 #3FB950 #D29922 #5B8DEF #BC8CFF #39C5CF #B3B3B3 #4D4D4D #FF7B72 #56D364 #E3B341 #79C0FF #D2A8FF #56D4DD #E6E6E6 #E6E6E6 #171717 #E6E6E6 #2A2D35'),
  },
  nord: {
    m: w('#2E3440 #3B4252 #272C36 #434C5E #4C566A #434C5E #5E6779 #ECEFF4 #D8DEE9 #A8B2C5 #D08770 #4A3B38 #A3BE8C #BF616A #81A1C1'),
    avatar: w('#BF616A #D08770 #EBCB8B #A3BE8C #8FBCBB #88C0D0 #81A1C1 #5E81AC #B48EAD #C9826B #9DB9A0 #A0A8C8'),
    term: w('#3B4252 #BF616A #A3BE8C #EBCB8B #81A1C1 #B48EAD #88C0D0 #E5E9F0 #4C566A #BF616A #A3BE8C #EBCB8B #81A1C1 #B48EAD #8FBCBB #ECEFF4 #D8DEE9 #2E3440 #D8DEE9 #434C5E'),
  },
  'catppuccin-mocha': {
    m: w('#1E1E2E #313244 #181825 #45475A #585B70 #45475A #6C7086 #CDD6F4 #BAC2DE #A6ADC8 #FAB387 #45352F #A6E3A1 #F38BA8 #89B4FA'),
    avatar: w('#F38BA8 #FAB387 #F9E2AF #A6E3A1 #94E2D5 #89DCEB #89B4FA #B4BEFE #CBA6F7 #F5C2E7 #EBA0AC #74C7EC'),
    term: w('#45475A #F38BA8 #A6E3A1 #F9E2AF #89B4FA #F5C2E7 #94E2D5 #BAC2DE #585B70 #F38BA8 #A6E3A1 #F9E2AF #89B4FA #F5C2E7 #94E2D5 #A6ADC8 #CDD6F4 #1E1E2E #F5E0DC #585B70'),
  },
}

function builtinPacks(): Pack[] {
  const b = (id: string, slug: string, name: string, appearance: PackAppearance, light?: string, dark?: string, mono = FONT_MONO): Pack => ({
    id,
    slug,
    name,
    kind: 'builtin',
    appearance,
    author: 'Mode',
    version: slug === 'paper' ? '2.0.0' : '1.0.0',
    mono,
    light: light ? SETS[light] : undefined,
    dark: dark ? SETS[dark] : undefined,
  })
  return [
    b('th-system', 'system', 'System', 'auto', 'system-light', 'system-dark'),
    b('th-paper', 'paper', 'Paper', 'auto', 'paper-light', 'paper-dark', PAPER_MONO),
    b('th-gruvbox-dark', 'gruvbox-dark', 'Gruvbox Dark', 'dark', undefined, 'gruvbox-dark'),
    b('th-gruvbox-light', 'gruvbox-light', 'Gruvbox Light', 'light', 'gruvbox-light'),
    b('th-cursor-dark', 'cursor-dark', 'Cursor Dark', 'dark', undefined, 'cursor-dark'),
    b('th-nord', 'nord', 'Nord', 'dark', undefined, 'nord'),
    b('th-catppuccin-mocha', 'catppuccin-mocha', 'Catppuccin Mocha', 'dark', undefined, 'catppuccin-mocha'),
  ]
}

function luminance(hex: string): number {
  const h = hex.replace('#', '')
  const c = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
  return 0.2126 * c[0]! + 0.7152 * c[1]! + 0.0722 * c[2]!
}

export function contrast(a: string, b: string): number {
  const [x, y] = [luminance(a), luminance(b)].sort((p, q) => q - p) as [number, number]
  return (x + 0.05) / (y + 0.05)
}

function swatchOf(s: ColorSet | undefined): Swatch | undefined {
  if (!s) return undefined
  const r = (k: (typeof ROLES)[number]) => s.m[ROLES.indexOf(k)]!
  return { bg: r('bg'), surface: r('surface'), fg: r('fg'), accent: r('accent'), ok: r('ok'), info: r('info') }
}

/** The flat `--m-*` map of one color set (10 §3.1 names; `mode_core::themes::resolve` keys). */
export function tokensOf(p: Pack, set: ColorSet, dark: boolean): Record<string, string> {
  const t: Record<string, string> = {}
  ROLES.forEach((r, i) => (t[`--m-${r}`] = set.m[i]!))
  const accent = t['--m-accent']!
  t['--m-accent-fg'] = contrast('#FFFFFF', accent) >= 4.5 ? '#FFFFFF' : t['--m-fg']!
  t['--m-selection'] = t['--m-selected']!
  t['--m-font-ui'] = FONT_UI
  t['--m-font-mono'] = p.mono
  t['--m-shadow-pop'] = dark ? SHADOW.dark : SHADOW.light
  // The stylesheet's radius names, as `mode_core::themes::resolve` sends them (`RADIUS_CONTROL`, `RADIUS_CARD`).
  t['--m-r-ctl'] = '6px'
  t['--m-r-card'] = '10px'
  set.avatar.forEach((h, i) => (t[`--m-avatar-${i + 1}`] = h))
  ANSI.forEach((n, i) => (t[`--m-terminal-ansi-${n}`] = set.term[i]!))
  ;(['fg', 'bg', 'cursor', 'selection'] as const).forEach((k, i) => (t[`--m-terminal-${k}`] = set.term[16 + i]!))
  // The 21st terminal token: the glyph under the block cursor, the background unless the pack says otherwise.
  t['--m-terminal-cursor-accent'] = t['--m-terminal-bg']!
  return t
}

function rowOf(p: Pack, active: string): ThemeRow {
  return {
    id: p.id,
    slug: p.slug,
    name: p.name,
    kind: p.kind,
    appearance: p.appearance,
    author: p.author,
    version: p.version,
    active: p.slug === active || p.id === active,
    swatches: { light: swatchOf(p.light), dark: swatchOf(p.dark) },
    builtin: p.kind === 'builtin',
    source_path: p.kind === 'imported' ? `${HOME}/Downloads/${p.slug}.json` : null,
    updated_at: '2026-10-21T09:00:00Z',
    card_line: p.appearance === 'light' ? `${p.name} (light) · System Dark at night` : p.appearance === 'dark' ? `${p.name} (dark) · System Light by day` : null,
  }
}

/* ======================================================================================
   schedules (03 §7B.4; mirrors mode_core::scheduler::schedule)
   ====================================================================================== */

const pad = (n: number) => String(n).padStart(2, '0')

function cronExpr(s: Schedule): string {
  const [hh, mm] = (s.time ?? '00:00').split(':').map(Number) as [number, number]
  switch (s.kind) {
    case 'daily':
      return `${mm} ${hh} * * *`
    case 'weekdays':
      return `${mm} ${hh} * * 1-5`
    case 'weekly':
      return `${mm} ${hh} * * ${(s.weekday ?? 1) % 7}`
    default:
      return (s.cron ?? '').trim()
  }
}

function field(src: string, lo: number, hi: number): { values: number[]; star: boolean } {
  const out = new Set<number>()
  for (const item of src.split(',')) {
    const m = /^(\*|(\d+)(?:-(\d+))?)(?:\/(\d+))?$/.exec(item)
    if (!m) continue
    const step = m[4] ? Number(m[4]) : 1
    const a = m[1] === '*' ? lo : Number(m[2])
    const b = m[1] === '*' ? hi : m[3] !== undefined ? Number(m[3]) : m[4] ? hi : a
    for (let v = a; v <= b; v += step) out.add(v === 7 && hi === 7 ? 0 : v)
  }
  return { values: Array.from(out).sort((x, y) => x - y), star: src === '*' }
}

/** Offset of `tz` at instant `ms`, in ms (wall − UTC). */
function offsetAt(ms: number, tz: string): number {
  const w0 = wallOf(ms, tz)
  return Date.UTC(w0.y, w0.m - 1, w0.d, w0.hh, w0.mm) - Math.floor(ms / 60_000) * 60_000
}

function zonedToUtc(y: number, m: number, d: number, hh: number, mm: number, tz: string): number {
  const guess = Date.UTC(y, m - 1, d, hh, mm)
  const first = guess - offsetAt(guess, tz)
  return guess - offsetAt(first, tz)
}

/** The next `n` fire instants after `after`, with their wall time in the zone. */
export function nextFires(s: Schedule, after: number, n: number, deviceTz = deviceZone()): { utc: number; local: string }[] {
  const tz = s.tz || deviceTz
  const parts = cronExpr(s).split(/\s+/)
  if (parts.length !== 5 || validateCron(parts.join(' '))) return []
  const [mi, hr, dom, mon, dow] = [field(parts[0]!, 0, 59), field(parts[1]!, 0, 23), field(parts[2]!, 1, 31), field(parts[3]!, 1, 12), field(parts[4]!, 0, 7)]
  const out: { utc: number; local: string }[] = []
  const start = wallOf(after, tz)
  let day = Date.UTC(start.y, start.m - 1, start.d)
  for (let i = 0; i < 800 && out.length < n; i++, day += 86_400_000) {
    const dt = new Date(day)
    const y = dt.getUTCFullYear()
    const m = dt.getUTCMonth() + 1
    const d = dt.getUTCDate()
    const wd = dt.getUTCDay()
    if (!mon.values.includes(m)) continue
    const domOk = dom.values.includes(d)
    const dowOk = dow.values.includes(wd)
    const ok = dom.star && dow.star ? true : dom.star ? dowOk : dow.star ? domOk : domOk || dowOk
    if (!ok) continue
    for (const h of hr.values)
      for (const mm of mi.values) {
        const utc = zonedToUtc(y, m, d, h, mm, tz)
        if (utc > after && out.length < n) out.push({ utc, local: `${y}-${pad(m)}-${pad(d)} ${pad(h)}:${pad(mm)}` })
      }
  }
  return out
}

const WEEKDAY_LONG = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday']

function joinWords(items: string[]): string {
  return items.length <= 1 ? (items[0] ?? '') : `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`
}

/** The plain-language echo under the cron field ("every weekday at 08:00"). */
export function describeSchedule(s: Schedule): string {
  const expr = cronExpr(s)
  if (validateCron(expr)) return ''
  const p = expr.split(/\s+/)
  const mi = field(p[0]!, 0, 59)
  const hr = field(p[1]!, 0, 23)
  const dom = field(p[2]!, 1, 31)
  const mon = field(p[3]!, 1, 12)
  const dow = field(p[4]!, 0, 7)
  const step = /^\*\/(\d+)$/.exec(p[0]!)
  if (step && p.slice(1).every((x) => x === '*')) return `every ${step[1]} minutes`
  if (p[1] === '*' && p.slice(2).every((x) => x === '*') && mi.values.length === 1) return `every hour at :${pad(mi.values[0]!)}`
  if (mi.values.length !== 1 || hr.values.length !== 1) return 'on a custom schedule'
  const at = `at ${pad(hr.values[0]!)}:${pad(mi.values[0]!)}`
  if (mon.values.length !== 12) return `on a custom schedule ${at}`
  if (dom.star && dow.star) return `every day ${at}`
  if (dom.star) {
    const days = dow.values
    if (days.join() === '1,2,3,4,5') return `every weekday ${at}`
    if (days.join() === '0,6') return `every weekend day ${at}`
    return `every ${joinWords([...days].sort((a, b) => ((a + 6) % 7) - ((b + 6) % 7)).map((d) => WEEKDAY_LONG[d]!))} ${at}`
  }
  if (dow.star) return `on day ${joinWords(dom.values.map(String))} of every month ${at}`
  return `on a custom schedule ${at}`
}

/* ======================================================================================
   the install
   ====================================================================================== */

export interface V1MockControl {
  chat: {
    status(patch: Partial<ChatStatus>): void
    /** Delay between streamed chunks (ms). */
    speed(ms: number): void
    threads(): ChatThread[]
  }
  automations: {
    rows(): AutomationRow[]
    problem(id: string, p: AutomationRow['problem']): void
    /** No automations: the empty state. */
    clear(): void
    /** How long a Run now takes (ms). */
    runMs(ms: number): void
  }
  themes: { packs(): ThemeRow[]; resolved(): ResolvedTheme }
  packs: {
    /** The catalog fetch fails with this sentence (null: it answers). */
    fail(message: string | null): void
    /** Every pack `pack_export` wrote, with its path. */
    exported(): { path: string; pack: unknown }[]
    fetched(): number
  }
}

declare global {
  interface Window {
    __mode_v1?: V1MockControl
  }
}

export interface V1MockOptions {
  now?: number
  /** The notes mock: the palette index, citations and confirmed Create-note actions use it. */
  vault?: NotesVault
}

export function installV1Mock(opts: V1MockOptions = {}): V1MockControl {
  const now0 = opts.now ?? Date.now()
  const vault = opts.vault
  const m = mock()
  const on = (cmd: string, fn: (a: Record<string, unknown>) => unknown) => m.respond(cmd, (args: Args) => fn(args ?? {}))
  const later = (ms: number, fn: () => void) => setTimeout(fn, ms)
  let seq = 0
  const nid = (p: string) => `${p}-${++seq}`

  /* ---- the other mocks' data (read lazily: they may install after this one) ---------------- */

  const settingsValues = (): Record<string, unknown> => window.__mode_settings?.state().values ?? themeState.settings
  const sessionRows = () => [...(m.rows ?? [])] as { id: string; display_name: string; state: string; repo_root: string | null; cwd: string }[]
  const noteRows = () => (vault ? vault.live().map((n) => vault.row(n)) : [])
  const threads = () => window.__mode_connectors?.state().threads ?? []
  const folderLabel = (id: string | null) => (id ? (vault?.folderPath(id) ?? id) : 'Unsorted')

  /* ---- themes ------------------------------------------------------------------------------------- */

  const themeState = { packs: builtinPacks(), settings: { 'theme.active': 'system', 'theme.appearance': 'auto' } as Record<string, unknown> }

  const osDark = () => typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia('(prefers-color-scheme: dark)').matches
  const livePacks = () => themeState.packs.filter((p) => !p.deleted)
  const packBy = (id: string) => livePacks().find((p) => p.id === id || p.slug === id)

  function resolve(id?: string): ResolvedTheme {
    const values = settingsValues()
    const active = String(id ?? values['theme.active'] ?? 'system')
    const pref = String(values['theme.appearance'] ?? 'auto') as AppearancePref
    const dark = pref === 'dark' || (pref === 'auto' && osDark())
    const which = dark ? 'dark' : 'light'
    const pack = packBy(active) ?? packBy('system')!
    const sysName = `System ${dark ? 'Dark' : 'Light'}`
    // Browser mock: System resolves to the stylesheet's own tokens (there is no OS accent to read, and
    // mode.css carries the defaults every parity test compares against). The core sends System's full map.
    if (pack.slug === 'system') return { tokens: {}, dark, slug: 'system', name: sysName, active: 'system', appearance: which, notices: [], accent_adjusted: false }
    const set = pack[which]
    if (!set) {
      const notice = pref === 'auto' ? `${pack.name} (${pack.appearance}) · ${sysName} ${dark ? 'at night' : 'by day'}` : `${pack.name} has no ${which} set — showing ${sysName}`
      return { tokens: {}, dark, slug: 'system', name: sysName, active: pack.slug, appearance: which, notices: [notice], accent_adjusted: false }
    }
    return { tokens: tokensOf(pack, set, dark), dark, slug: pack.slug, name: pack.name, active: pack.slug, appearance: which, notices: [], accent_adjusted: false }
  }

  function pushTheme() {
    m.emit(THEME_EVENTS.changed, resolve())
  }

  function setSettings(patch: Record<string, unknown>) {
    const values = window.__mode_settings?.state().values
    for (const [k, v] of Object.entries(patch)) {
      if (values) values[k] = v
      themeState.settings[k] = v
    }
    m.emit(EVENTS.settings, patch)
  }

  on('themes_list', () => {
    const active = String(settingsValues()['theme.active'] ?? 'system')
    return livePacks().map((p) => rowOf(p, active))
  })
  on('theme_get', (a) => {
    const p = packBy(String(a.id))
    if (!p) throw cmdError('not_found', 'No such pack.')
    const issues = p.document ? validatePack(JSON.stringify(p.document)).issues : []
    const report = {
      valid: !issues.some((i) => i.severity === 'error'),
      errors: issues.filter((i) => i.severity === 'error'),
      warnings: issues.filter((i) => i.severity === 'warning'),
      fills: issues.filter((i) => i.severity === 'fill'),
    }
    return { row: rowOf(p, String(settingsValues()['theme.active'] ?? 'system')), document: p.document ?? { format: 1, slug: p.slug, name: p.name, appearance: p.appearance }, report }
  })
  on('theme_resolve', (a) => resolve(a.id ? String(a.id) : undefined))
  on('theme_set_active', (a) => {
    const p = packBy(String(a.id))
    if (!p) throw cmdError('not_found', 'No such pack.')
    const patch: Record<string, unknown> = { 'theme.active': p.slug }
    if (a.mode === 'light' || a.mode === 'dark' || a.mode === 'auto') patch['theme.appearance'] = a.mode
    setSettings(patch)
    const r = resolve()
    m.emit(THEME_EVENTS.changed, r)
    return r
  })
  on('theme_export', (a) => {
    const p = packBy(String(a.id))
    if (!p) throw cmdError('not_found', 'No such pack.')
    return { path: `${HOME}/Downloads/${p.slug}.mode-theme.json` }
  })
  on('theme_delete', (a) => {
    const p = packBy(String(a.id))
    if (!p) throw cmdError('not_found', 'No such pack.')
    if (p.kind === 'builtin') throw cmdError('builtin', 'Built-in packs cannot be removed.')
    p.deleted = true
    const activeReset = String(settingsValues()['theme.active']) === p.slug
    if (activeReset) {
      setSettings({ 'theme.active': 'system' })
      pushTheme()
    }
    return { deleted: true, active_reset: activeReset }
  })
  on('theme_import', (a) => {
    const text = typeof a.text === 'string' ? a.text : ''
    const { issues, pack } = validatePack(text)
    const errors = issues.filter((i) => i.severity === 'error')
    const warnings = issues.filter((i) => i.severity === 'warning').length
    const label = typeof a.path === 'string' ? (a.path.split(/[\\/]/).pop() ?? a.path) : (pack?.name ?? 'the pack')
    const existing = pack ? packBy(pack.slug) : undefined
    const active = String(settingsValues()['theme.active'] ?? 'system')
    // `mode_core::themes::api::ThemeImportResult`: the outcome and the line the card shows.
    const base: ThemeImportResult = {
      ok: !errors.length,
      saved: false,
      slug: pack?.slug ?? null,
      name: pack?.name ?? null,
      appearance: pack?.appearance ?? null,
      issues,
      conflict: !!existing,
      swatches: pack ? { light: swatchOf(pack.light), dark: swatchOf(pack.dark) } : null,
      row: null,
      outcome: errors.length ? 'invalid' : 'preview',
      message: errors.length ? `Could not import ${label}: ${errors.length} errors` : warnings ? `${pack?.name ?? label} has ${warnings} contrast warnings; text may be hard to read.` : null,
      existing: existing ? rowOf(existing, active) : null,
    }
    const apply = a.apply === true || (a.apply === undefined && a.dry_run === false)
    if (errors.length || !pack || !apply) return base
    if (existing && !a.on_conflict) return { ...base, outcome: 'conflict' }
    if (existing && a.on_conflict === 'replace') {
      if (existing.kind === 'builtin') throw cmdError('builtin', 'Built-in packs cannot be replaced; keep both instead.')
      Object.assign(existing, { ...pack, id: existing.id })
      return { ...base, saved: true, outcome: 'replaced', row: rowOf(existing, active) }
    }
    let slug = pack.slug
    for (let n = 2; packBy(slug); n++) slug = `${pack.slug}-${n}`
    const saved: Pack = { ...pack, slug, id: nid('th') }
    themeState.packs.push(saved)
    return { ...base, slug, saved: true, outcome: 'imported', row: rowOf(saved, active) }
  })

  // `themes::api::theme_duplicate {id, name?}`: a `custom` copy of any pack (built-ins too), slug `<slug>-copy-<n>`.
  on('theme_duplicate', (a) => {
    const src = packBy(String(a.id))
    if (!src) throw cmdError('not_found', 'No such pack.')
    const name = typeof a.name === 'string' && a.name.trim() ? a.name.trim() : `${src.name} copy`
    let n = 2
    while (packBy(`${src.slug}-copy-${n}`)) n++
    const copy: Pack = { ...src, id: nid('th'), slug: `${src.slug}-copy-${n}`, name, kind: 'custom', deleted: false, document: src.document ? { ...(src.document as object), slug: `${src.slug}-copy-${n}`, name } : undefined }
    themeState.packs.push(copy)
    return rowOf(copy, String(settingsValues()['theme.active'] ?? 'system'))
  })
  // `themes::api::theme_update {id, document}`: the token editor's Save on an imported or custom pack (the slug stays the row's).
  on('theme_update', (a) => {
    const current = packBy(String(a.id))
    if (!current) throw cmdError('not_found', 'No such pack.')
    if (current.kind === 'builtin') throw cmdError('invalid', 'Built-in packs cannot be edited; duplicate it first.')
    const doc: Record<string, unknown> = { ...((a.document ?? {}) as Record<string, unknown>), slug: current.slug }
    const { issues, pack } = validatePack(JSON.stringify(doc))
    const errors = issues.filter((i) => i.severity === 'error')
    const warnings = issues.filter((i) => i.severity === 'warning').length
    const active = String(settingsValues()['theme.active'] ?? 'system')
    const base: ThemeImportResult = {
      ok: !errors.length,
      saved: false,
      slug: current.slug,
      name: typeof doc.name === 'string' ? doc.name : current.name,
      appearance: pack?.appearance ?? null,
      issues,
      conflict: false,
      swatches: pack && !errors.length ? { light: swatchOf(pack.light), dark: swatchOf(pack.dark) } : null,
      row: null,
      outcome: 'invalid',
      message: `Could not save ${current.name}: ${errors.length} errors`,
    }
    if (errors.length || !pack) return base
    Object.assign(current, { ...pack, id: current.id, slug: current.slug, kind: current.kind })
    if (current.slug === active) pushTheme()
    return { ...base, saved: true, outcome: 'replaced', message: warnings ? `${current.name} has ${warnings} contrast warnings; text may be hard to read.` : null, row: rowOf(current, active) }
  })

  /* ---- palette ------------------------------------------------------------------------------------- */

  const SETTINGS_PAGES: [string, string][] = [
    ['general', 'General'],
    ['appearance', 'Appearance'],
    ['overlay', 'Overlay'],
    ['alerts', 'Alerts'],
    ['notes', 'Notes'],
    ['urgency', 'Urgency'],
    ['ai', 'AI'],
    ['privacy', 'Privacy'],
    ['advanced', 'Advanced'],
  ]

  const STATE_WORD: Record<string, string> = { unread: 'finished', done: 'finished' }

  /** `palette::index::VIEWS` and `COMMANDS` (the core's static entries). */
  const VIEWS: [string, string][] = [
    ['Sessions', 'sessions'],
    ['Inbox', 'inbox'],
    ['Notes', 'notes'],
    ['Calendar', 'calendar'],
    ['Chat', 'chat'],
    ['Automations', 'automations'],
    ['Integrations', 'integrations'],
    ['Shortcuts', 'shortcuts'],
    ['Stats', 'stats'],
    ['Settings', 'settings/general'],
  ]
  const COMMANDS: [string, string, string][] = [
    ['note.new', 'New note', 'notes'],
    ['session.launch', 'Launch session', 'sessions'],
    ['chat.new', 'New conversation', 'chat'],
    ['automation.new', 'New automation', 'automations'],
    ['theme.import', 'Import theme pack', 'settings/appearance'],
    ['hooks.install', 'Install hooks', 'integrations/claude-code/install'],
    ['settings.export', 'Export settings', 'settings/advanced/export-import'],
    ['diagnostics.open', 'Open diagnostics', 'settings/advanced/diagnostics'],
    ['account.connect', 'Connect an account', 'integrations'],
  ]

  function paletteIndex(): Omit<PaletteResult, 'score'>[] {
    const out: Omit<PaletteResult, 'score'>[] = []
    for (const r of sessionRows()) {
      const repo = (r.repo_root ?? r.cwd ?? '').split('/').filter(Boolean).pop() ?? ''
      out.push({ kind: 'session', id: r.id, title: r.display_name, detail: [STATE_WORD[r.state] ?? r.state, repo].filter(Boolean).join(' · '), route: `sessions/${r.id}` })
    }
    for (const n of noteRows()) {
      const folder = n.path.includes('/') ? n.path.slice(0, n.path.lastIndexOf('/')) : 'Unsorted'
      out.push({ kind: 'note', id: n.id, title: n.title, detail: `${n.status} · ${folder}`, route: `notes/${n.id}` })
    }
    // One row per conversation, its latest message (a channel thread: the parent): the core's rows carry message ids and
    // open `inbox/message/<id>`, which resolves the thread.
    for (const { row: t, messages } of threads()) {
      const latest = t.kind === 'channel_thread' ? messages[0] : messages[messages.length - 1]
      if (!latest || t.is_archived) continue
      const time = wallOf(Date.parse(t.last_message_at), deviceZone())
      const title = [t.sender_name, t.channel_name ? `#${t.channel_name.replace(/^#/, '')}` : null, t.last_snippet ?? t.title].filter(Boolean).join(' · ')
      out.push({ kind: 'message', id: latest.id, title, detail: `${pad(time.hh)}:${pad(time.mm)}`, route: `inbox/message/${latest.id}` })
    }
    for (const r of autoState.rows) {
      const p = r.trigger_phrase
      out.push({ kind: 'automation', id: r.id, title: r.name, detail: p.charAt(0).toLowerCase() + p.slice(1), route: `automations/${r.id}` })
    }
    for (const t of chatState.threads) out.push({ kind: 'chat', id: t.id, title: t.title, detail: 'Chat', route: `chat/${t.id}` })
    for (const [slug, label] of SETTINGS_PAGES) out.push({ kind: 'setting', id: `settings-${slug}`, title: label, detail: 'Settings', route: `settings/${slug}` })
    // `palette::index::statics`: the views ("Go to …", `view.<name>`) and the actions, each opening the view it acts in.
    for (const [name, route] of VIEWS) out.push({ kind: 'command', id: `view.${route.split('/')[0]}`, title: `Go to ${name}`, detail: 'View', route })
    for (const [id, title, route] of COMMANDS) out.push({ kind: 'command', id, title, detail: 'Command', route })
    // Files the sessions touched (latest session per path): the absolute path as the id, the file name as the title.
    const seen = new Set<string>()
    const live = new Set(sessionRows().map((r) => r.id))
    for (const [sid, d] of [
      ['s-api', sampleDetail(now0)],
      ['s-rate', sampleFinishedDetail(now0)],
    ] as const) {
      if (!live.has(sid)) continue
      for (const f of d.files) {
        if (f.change_kind === 'deleted' || seen.has(f.path)) continue
        seen.add(f.path)
        const dir = f.rel_path?.includes('/') ? f.rel_path.slice(0, f.rel_path.lastIndexOf('/')) : ''
        out.push({ kind: 'file', id: f.path, title: f.path.split('/').pop() ?? f.path, detail: dir ? `mode · ${dir}/` : 'mode', route: `sessions/${sid}/files` })
      }
    }
    return out
  }

  /** Fuzzy score: whole-title prefix > word prefix > substring > in-order letters; 0 = no match. */
  function score(title: string, q: string): number {
    const t = title.toLowerCase()
    const s = q.toLowerCase().trim()
    if (!s) return 0
    if (t.startsWith(s)) return 100 - Math.min(40, t.length / 4)
    if (new RegExp(`(^|[\\s\\-_/·#])${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`).test(t)) return 80 - Math.min(30, t.length / 5)
    if (t.includes(s)) return 50 - Math.min(20, t.length / 6)
    let i = 0
    for (const ch of t) if (ch === s[i]) i++
    return i === s.length ? 20 - Math.min(15, t.length / 8) : 0
  }

  on('palette_query', (a) => {
    const text = String(a.text ?? '')
    const limit = typeof a.limit === 'number' ? a.limit : 30
    const kinds = Array.isArray(a.kinds) ? (a.kinds as PaletteKind[]) : null
    const rows = paletteIndex()
      .filter((r) => !kinds || kinds.includes(r.kind))
      .map((r) => ({ ...r, score: text.trim() ? score(r.title, text) : 1 }))
      .filter((r) => r.score > 0)
      .sort((x, y) => y.score - x.score || x.title.localeCompare(y.title))
    return rows.slice(0, limit).map((r) => ({ ...r, score: Math.round(r.score * 10) / 10 }))
  })

  /* ---- chat ---------------------------------------------------------------------------------------- */

  const status: ChatStatus = {
    backend: 'claude_api',
    backend_label: 'Claude API',
    model: 'claude-opus-5-5',
    local: false,
    local_available: true,
    local_only: false,
    ready: 'ok',
    error: null,
    accounts: { gmail: true, slack: true, calendar: true },
  }

  const LONG_ASK =
    "Summarize what changed in api-refactor this week and draft a release note for #release. Include the rate limiter, the auth fixture fix and anything Lena flagged in the docs PR. Keep it under 120 words, lead with what a user would notice, and end with the migration steps from guides/migration.md. If the invoice-parser failure is related, say so in one line; otherwise leave it out. Mention that the webhook spike is parked. Use the same tone as the last two release notes in CHANGELOG.md, no exclamation marks, and link the PR numbers. When you are done, pin the note so I can find it from the overlay, and tell me which files you read so I can double-check them before I post. Also check whether Priya's thread about the v2 rollout changes any of the dates."

  const cite = (n: number, kind: Citation['kind'], id: string, title: string, detail: string, route: string): Citation => ({ n, kind, id, title, detail, route })

  const releaseCitations: Citation[] = [
    cite(1, 'session', 's-api', 'api-refactor', 'mode · feat/api', 'sessions/s-api'),
    cite(2, 'note', 'n-lena', 'Reply to Lena about PR 412', 'Projects/Mode', 'notes/n-lena'),
    cite(3, 'message', 'th-release', 'Sam · #release', 'can we ship v2 behind the flag today?', 'inbox/thread/th-release'),
  ]

  const threadA: ChatThread = {
    id: 'c-release',
    title: 'Release note for api-refactor',
    created_at: iso(now0 - 6 * 60_000),
    updated_at: iso(now0 - 10_000),
    effort: 'auto',
    connectors: { gmail: true, slack: true, calendar: true },
    turns: [
      { id: 't-a1', role: 'user', text: LONG_ASK, created_at: iso(now0 - 6 * 60_000) },
      {
        id: 't-a2',
        role: 'assistant',
        status: 'finished',
        created_at: iso(now0 - 20_000),
        text: "Three things changed in `api-refactor` this week: the rate limiter retries with exponential backoff (PR 418), the auth test is deterministic (PR 417), and Lena's PR 412 adds the migration guide. The invoice-parser failure is unrelated; the webhook spike is parked.",
        activity: [
          {
            id: 'g-a1',
            label: 'Ran 3 commands',
            rows: [
              { text: 'list_sessions · repo mode · since Monday', call_id: 'call-a1' },
              { text: 'get_session · api-refactor', call_id: 'call-a2' },
              { text: 'search_messages · #release · v2', call_id: 'call-a3' },
            ],
          },
          {
            id: 'g-a2',
            label: 'Read 8 files',
            rows: [
              { text: 'guides/migration.md', thumb: 'md', call_id: 'call-a2' },
              { text: 'tests/test_auth.py', thumb: 'py', call_id: 'call-a2' },
              { text: 'src/auth/limiter.ts', thumb: 'ts', call_id: 'call-a2' },
              { text: 'Checked Playwright availability and loaded the migration guide draft from the docs-site session', call_id: 'call-a4' },
            ],
            status: '4 background tasks completed',
          },
        ],
        citations: releaseCitations,
        tokens: 18_400,
      },
      { id: 't-a3', role: 'user', text: 'Draft it for #release.', created_at: iso(now0 - 12_000) },
      {
        id: 't-a4',
        role: 'assistant',
        status: 'streaming',
        created_at: iso(now0 - 10_000),
        text: '',
        activity: [{ id: 'g-a3', label: 'Updating the release note', rows: [], active: true }],
        citations: [],
      },
    ],
  }

  const threadB: ChatThread = {
    id: 'c-priya',
    title: "Tomorrow's 1:1 with Priya",
    created_at: iso(now0 - 2 * 3_600_000),
    updated_at: iso(now0 - 2 * 3_600_000 + 40_000),
    effort: 'auto',
    connectors: { gmail: true, slack: true, calendar: true },
    turns: [
      { id: 't-b1', role: 'user', text: "What do I need for tomorrow's 1:1 with Priya?", created_at: iso(now0 - 2 * 3_600_000) },
      {
        id: 't-b2',
        role: 'assistant',
        status: 'finished',
        created_at: iso(now0 - 2 * 3_600_000 + 30_000),
        text: 'Your 1:1 with Priya is tomorrow at 10:00 [1]. She asked in #release whether the v2 rollout dates moved [2], and the changelog draft still has that question open [3].\n\n- Confirm the rollout dates\n- Bring the migration steps from PR 412\n\nI can create a prep note for it.',
        activity: [
          { id: 'g-b1', label: 'Checked the calendar', rows: [{ text: 'calendar_lookup · tomorrow', call_id: 'call-b1' }] },
          {
            id: 'g-b2',
            label: 'Searched notes and messages',
            rows: [
              { text: 'search_messages · Priya · since last week', call_id: 'call-b2' },
              { text: 'search_notes · rollout', call_id: 'call-b3' },
            ],
          },
        ],
        citations: [
          cite(1, 'event', 'e-priya', '1:1 Priya', 'tomorrow 10:00', 'calendar/e-priya'),
          cite(2, 'message', 'th-release', 'Sam · #release', 'can we ship v2 behind the flag today?', 'inbox/thread/th-release'),
          cite(3, 'note', 'n-changelog', 'Draft v2 changelog', 'Projects/Mode', 'notes/n-changelog'),
        ],
        actions: [
          {
            id: 'act-b1',
            kind: 'create_note',
            title: 'Create note',
            fields: [
              { label: 'Folder', value: 'Unsorted/' },
              { label: 'Title', value: 'Prepare for 1:1 with Priya' },
              { label: 'Status', value: 'todo' },
              { label: 'When', value: 'tomorrow' },
            ],
            preview: '- [ ] Confirm the v2 rollout dates\n- [ ] Migration steps from PR 412',
            state: 'pending',
          },
        ],
        tokens: 9_200,
      },
    ],
  }

  const sourcesByTurn: Record<string, ToolCall[]> = {
    't-a2': [
      {
        id: 'call-a1',
        tool: 'list_sessions',
        input_line: 'repo mode · since Monday',
        state: 'done',
        items: [{ kind: 'session', id: 's-api', title: 'api-refactor', detail: 'waiting · mode', route: 'sessions/s-api' }],
      },
      {
        id: 'call-a2',
        tool: 'get_session',
        input_line: 'api-refactor',
        state: 'done',
        items: [{ kind: 'session', id: 's-api', title: 'api-refactor', detail: '+43 −18 · 4 files', route: 'sessions/s-api' }],
      },
      {
        id: 'call-a3',
        tool: 'search_messages',
        input_line: '#release · v2',
        state: 'done',
        items: [
          { kind: 'message', id: 'th-release', title: 'Sam · #release', detail: 'can we ship v2 behind the flag today?', route: 'inbox/thread/th-release' },
          { kind: 'message', id: 'th-lena', title: 'Lena · PR 412', detail: 'migration guide', route: 'inbox/thread/th-lena' },
        ],
      },
      {
        id: 'call-a4',
        tool: 'search_notes',
        input_line: 'release · PR 412',
        state: 'done',
        items: [{ kind: 'note', id: 'n-lena', title: 'Reply to Lena about PR 412', detail: 'Projects/Mode', route: 'notes/n-lena' }],
      },
    ],
    't-a4': [
      {
        id: 'call-a5',
        tool: 'search_notes',
        input_line: 'release · status any',
        state: 'done',
        items: [
          { kind: 'note', id: 'n-changelog', title: 'Draft v2 changelog', detail: 'Projects/Mode', route: 'notes/n-changelog' },
          { kind: 'note', id: 'n-checklist', title: 'Release checklist', detail: 'Projects/Mode', route: 'notes/n-checklist' },
          { kind: 'note', id: 'n-relnotes', title: 'Draft the release notes', detail: 'Projects/Docs site', route: 'notes/n-relnotes' },
        ],
      },
      {
        id: 'call-a6',
        tool: 'get_session',
        input_line: 'api-refactor',
        state: 'done',
        items: [{ kind: 'session', id: 's-api', title: 'api-refactor', detail: '+43 −18 · 4 files', route: 'sessions/s-api' }],
      },
      {
        id: 'call-a7',
        tool: 'search_messages',
        input_line: '#release · since Monday',
        state: 'done',
        items: [
          { kind: 'message', id: 'th-release', title: 'Sam · #release', detail: 'can we ship v2 behind the flag today?', route: 'inbox/thread/th-release' },
          { kind: 'message', id: 'th-lena', title: 'Lena · PR 412', detail: 'migration guide', route: 'inbox/thread/th-lena' },
        ],
      },
      {
        id: 'call-a8',
        tool: 'calendar_lookup',
        input_line: 'this week',
        state: 'done',
        items: [{ kind: 'event', id: 'e-release', title: 'Release v2', detail: 'Fri 23 Oct', route: 'calendar/e-release' }],
      },
      { id: 'call-a9', tool: 'search_notes', input_line: 'webhook spike', state: 'running', items: [] },
    ],
    't-b2': [
      {
        id: 'call-b1',
        tool: 'calendar_lookup',
        input_line: 'tomorrow',
        state: 'done',
        items: [{ kind: 'event', id: 'e-priya', title: '1:1 Priya', detail: 'tomorrow 10:00', route: 'calendar/e-priya' }],
      },
      {
        id: 'call-b2',
        tool: 'search_messages',
        input_line: 'Priya · since last week',
        state: 'done',
        items: [{ kind: 'message', id: 'th-release', title: 'Sam · #release', detail: 'can we ship v2 behind the flag today?', route: 'inbox/thread/th-release' }],
      },
      {
        id: 'call-b3',
        tool: 'search_notes',
        input_line: 'rollout',
        state: 'done',
        items: [{ kind: 'note', id: 'n-changelog', title: 'Draft v2 changelog', detail: 'Projects/Mode', route: 'notes/n-changelog' }],
      },
      { id: 'call-b4', tool: 'search_notes', input_line: 'Priya · status todo', state: 'done', items: [] },
    ],
  }
  const tokensByTurn: Record<string, number> = { 't-a2': 18_400, 't-a4': 6_100, 't-b2': 9_200 }

  // Older conversations (the sidebar shows five, then "Show n more", 03 §2.3.2).
  const older: ChatThread[] = (
    [
      ['c-retry', 'Is the retry policy final?', 26],
      ['c-standup', 'Standup notes for Monday', 50],
      ['c-invoice', 'Why did invoice-parser fail?', 74],
      ['c-gym', 'Gym plan for the week', 98],
      ['c-docs', 'Docs freeze checklist', 122],
      ['c-lena', 'Reply to Lena about PR 412', 146],
    ] as const
  ).map(([id, title, hoursAgo]) => ({
    id,
    title,
    created_at: iso(now0 - hoursAgo * 3_600_000),
    updated_at: iso(now0 - hoursAgo * 3_600_000 + 60_000),
    effort: 'auto' as const,
    connectors: { gmail: true, slack: true, calendar: true },
    turns: [
      { id: `${id}-u`, role: 'user' as const, text: title, created_at: iso(now0 - hoursAgo * 3_600_000) },
      { id: `${id}-a`, role: 'assistant' as const, status: 'finished' as const, created_at: iso(now0 - hoursAgo * 3_600_000 + 60_000), text: 'Answered earlier.', activity: [], citations: [], actions: [], tokens: 800 },
    ],
  }))
  const chatState = { threads: [threadA, threadB, ...older] as ChatThread[], speed: 60, timers: new Map<string, ReturnType<typeof setTimeout>[]>() }
  const threadBy = (id: string) => chatState.threads.find((t) => t.id === id)
  const turnBy = (turnId: string) => {
    for (const t of chatState.threads) {
      const turn = t.turns.find((x) => x.id === turnId)
      if (turn) return { thread: t, turn }
    }
    return null
  }

  function threadRow(t: ChatThread): ChatThreadRow {
    return {
      id: t.id,
      title: t.title,
      updated_at: t.updated_at,
      turn_count: t.turns.length,
      pinned: t.turns.filter((x) => x.pinned).map((x) => ({ turn_id: x.id, excerpt: x.text.replace(/\s+/g, ' ').slice(0, 80) })),
    }
  }

  /** The core's `chat_status` extras (`mode_core::chat::api::ChatStatus`), derived from the switches above. */
  const STATE_OF: Record<ChatStatus['ready'], NonNullable<ChatStatus['state']>> = {
    ok: 'ready',
    no_key: 'no_key',
    invalid_key: 'invalid_key',
    local_only_no_backend: 'local_only_blocked',
    budget: 'budget_reached',
    unreachable: 'unreachable',
    offline: 'offline',
  }
  const MESSAGE_OF: Partial<Record<ChatStatus['ready'], string>> = {
    no_key: 'Add your Anthropic API key to use Chat',
    invalid_key: 'API key rejected',
    local_only_no_backend: 'Local-only is on and no local backend is configured.',
    budget: 'Monthly AI budget reached',
    offline: "You're offline",
  }
  const blocked = () => status.ready === 'no_key' || status.ready === 'invalid_key' || status.ready === 'local_only_no_backend' || status.ready === 'budget'
  /** 06 §10.3 as the core classifies `ai.local_url`: a private address is a LAN backend. */
  const lanBackend = () => /^https?:\/\/(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(String(settingsValues()['ai.local_url'] ?? ''))
  on('chat_status', (a): ChatStatus => {
    const effort = (a.effort as Effort | undefined) ?? 'auto'
    const local = status.local || status.local_only || effort === 'local_only'
    const disabled = blocked()
    return {
      ...status,
      // The core reports a rejected key as `no_key` with `state: invalid_key` (`mode_core::chat::api::chat_status`).
      ready: status.ready === 'invalid_key' ? 'no_key' : status.ready,
      error: status.ready === 'invalid_key' ? 'API key rejected' : status.error,
      accounts: { ...status.accounts },
      state: STATE_OF[status.ready],
      message: MESSAGE_OF[status.ready] ?? (status.ready === 'unreachable' ? (status.error ?? `Can't reach ${status.backend_label}`) : 'Ready'),
      input_disabled: disabled,
      effort,
      model_short: status.model ? status.model.replace(/^claude-/, '') : null,
      privacy_line: disabled
        ? null
        : !local
          ? `Your question and the retrieved snippets are sent to ${status.backend_label}`
          : lanBackend()
            ? 'LAN backend — data leaves this computer but not your network'
            : 'Nothing leaves this machine',
      hint: local ? 'local models may handle actions less reliably' : null,
    }
  })
  on('chat_threads_list', () => [...chatState.threads].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).map(threadRow))
  on('chat_thread_get', (a) => {
    const t = threadBy(String(a.id))
    if (!t) throw cmdError('not_found', 'This conversation is no longer available.')
    return JSON.parse(JSON.stringify(t)) as ChatThread
  })
  on('chat_thread_delete', (a) => {
    const t = threadBy(String(a.id))
    if (!t) throw cmdError('not_found', `No conversation ${String(a.id)}.`)
    chatState.threads = chatState.threads.filter((x) => x.id !== a.id)
    return { deleted: t.turns.length }
  })
  on('chat_thread_rename', (a) => {
    const t = threadBy(String(a.id))
    if (t) t.title = String(a.title)
    return undefined
  })
  on('chat_thread_set', (a) => {
    const t = threadBy(String(a.id))
    if (t && a.effort) t.effort = a.effort as Effort
    if (t && a.connectors) t.connectors = a.connectors as ChatConnectors
    return undefined
  })
  on('chat_pin', (a) => {
    const r = turnBy(String(a.turn_id))
    if (r) r.turn.pinned = !!a.pinned
    return undefined
  })
  // `commands_chat::file_ref`: the absolute path as the id, the file name as the title, the folder (home as `~`), no route.
  on('chat_pick_file', () => ({ kind: 'file', id: `${MODE_REPO}/README.md`, title: 'README.md', detail: '~/code/mode', route: null }))
  on('chat_sources', (a) => {
    const id = String(a.turn_id)
    return { turn_id: id, calls: JSON.parse(JSON.stringify(sourcesByTurn[id] ?? [])) as ToolCall[], tokens: tokensByTurn[id] ?? null } satisfies ChatSources
  })

  function emitTurn(
    thread: ChatThread,
    turn: ChatTurn,
    statusName: 'started' | 'finished' | 'failed' | 'stopped',
    extra: { error?: string; usage?: TokenUsage } = {},
  ) {
    m.emit(CHAT_EVENTS.turn, {
      turn_id: turn.id,
      thread_id: thread.id,
      status: statusName,
      stopped: statusName === 'stopped',
      ...extra,
      ...(statusName === 'started' ? {} : { turn: JSON.parse(JSON.stringify(turn)) as ChatTurn, citations: turn.citations ?? [] }),
    })
  }

  /** A canned answer grounded on the palette index: the first matches become citations. */
  function compose(text: string): { reply: string; citations: Citation[]; calls: ToolCall[]; activity: ActivityGroup[]; action: ChatAction | null } {
    const words = text
      .replace(/^\/\w+\s*/, '')
      .toLowerCase()
      .split(/[^a-z0-9]+/)
      .filter((x) => x.length > 3)
    const index = paletteIndex().filter((r) => r.kind === 'note' || r.kind === 'session' || r.kind === 'message')
    const hits: ItemRef[] = []
    for (const r of index) {
      if (hits.length >= 3) break
      if (words.some((wd) => r.title.toLowerCase().includes(wd)))
        hits.push({ kind: r.kind === 'message' ? 'message' : r.kind === 'session' ? 'session' : 'note', id: r.id, title: r.title, detail: r.detail, route: r.route })
    }
    const callId = nid('call')
    const calls: ToolCall[] = [{ id: callId, tool: 'search_notes', input_line: words.slice(0, 3).join(' ') || text.slice(0, 24), state: 'done', items: hits }]
    const activity: ActivityGroup[] = [{ id: nid('g'), label: hits.length ? `Searched notes and found ${hits.length}` : 'Searched notes', rows: hits.map((h) => ({ text: h.title, call_id: callId })) }]
    const citations = hits.map((h, i) => ({ ...h, n: i + 1 }))
    const isTodo = /^\/todo\b/.test(text.trim())
    const action: ChatAction | null = isTodo
      ? {
          id: nid('act'),
          kind: 'create_note',
          title: 'Create note',
          fields: [
            { label: 'Folder', value: 'Unsorted/' },
            { label: 'Title', value: firstWords(text.replace(/^\/todo\s*/, '')) || 'New todo' },
            { label: 'Status', value: 'todo' },
          ],
          preview: null,
          state: 'pending',
        }
      : null
    const reply = hits.length
      ? `I found ${hits.length === 1 ? 'one item' : `${hits.length} items`} about that: ${citations.map((c) => `${c.title} [${c.n}]`).join(', ')}.${isTodo ? ' I can turn it into a todo note.' : ''}`
      : "I couldn't find anything about that in your notes, sessions, messages or calendar."
    return { reply, citations, calls, activity, action }
  }

  on('chat_send', (a) => {
    const text = String(a.text ?? '').trim()
    if (!text) throw cmdError('invalid', 'Write a question first.')
    if (blocked()) throw cmdError(status.ready, 'Chat is not available.')
    const effort = (a.effort as Effort) ?? 'auto'
    let thread = a.thread_id ? threadBy(String(a.thread_id)) : undefined
    const at = iso(Date.now())
    if (!thread) {
      thread = {
        id: nid('c'),
        title: text.length > 60 ? `${text.slice(0, 59).trimEnd()}…` : text,
        created_at: at,
        updated_at: at,
        effort,
        connectors: (a.connectors as ChatConnectors) ?? { gmail: true, slack: true, calendar: true },
        turns: [],
      }
      chatState.threads.push(thread)
    }
    const attachments = Array.isArray(a.attachments)
      ? (a.attachments as { kind: ItemRef['kind']; id: string }[]).map((x) => ({ kind: x.kind, id: x.id, title: paletteIndex().find((r) => r.id === x.id)?.title ?? x.id }))
      : undefined
    const user: ChatTurn = { id: nid('t'), role: 'user', text, created_at: at, attachments }
    const turn: ChatTurn = { id: nid('t'), role: 'assistant', text: '', created_at: at, status: 'streaming', activity: [], citations: [], actions: [] }
    thread.turns.push(user, turn)
    thread.updated_at = at
    thread.effort = effort
    const th = thread
    const { reply, citations, calls, activity, action } = compose(text)
    const chunks = reply.match(/.{1,24}(\s|$)/g) ?? [reply]
    const timers: ReturnType<typeof setTimeout>[] = []
    const step = chatState.speed
    const failWith = status.ready === 'unreachable' ? (status.error ?? "Can't reach Claude API") : status.ready === 'offline' && effort !== 'local_only' && !status.local ? 'offline' : null
    timers.push(later(step, () => emitTurn(th, turn, 'started')))
    if (failWith) {
      timers.push(
        later(step * 3, () => {
          turn.status = 'failed'
          turn.error = failWith
          emitTurn(th, turn, 'failed', { error: failWith })
        }),
      )
    } else {
      const group = { ...activity[0]!, active: true }
      timers.push(
        later(step * 2, () => {
          turn.activity = [group]
          m.emit(CHAT_EVENTS.delta, { turn_id: turn.id, thread_id: th.id, activity: group })
        }),
      )
      chunks.forEach((c, i) =>
        timers.push(
          later(step * (4 + i), () => {
            turn.text += c
            m.emit(CHAT_EVENTS.delta, { turn_id: turn.id, thread_id: th.id, text: c })
          }),
        ),
      )
      timers.push(
        later(step * (5 + chunks.length), () => {
          turn.status = 'finished'
          turn.activity = activity
          turn.citations = citations
          turn.no_sources = !citations.length
          turn.tokens = 2_400 + text.length * 3
          sourcesByTurn[turn.id] = calls
          tokensByTurn[turn.id] = turn.tokens
          if (action) {
            turn.actions = [action]
            m.emit(CHAT_EVENTS.action, { turn_id: turn.id, thread_id: th.id, action })
          }
          emitTurn(th, turn, 'finished', { usage: { input_tokens: 2_000, output_tokens: turn.tokens - 2_000, cache_read_tokens: 0, cache_write_tokens: 0, total: turn.tokens } })
          chatState.timers.delete(turn.id)
        }),
      )
    }
    chatState.timers.set(turn.id, timers)
    return { turn_id: turn.id, thread_id: th.id, user_turn_id: user.id }
  })

  on('chat_stop', (a) => {
    const r = turnBy(String(a.turn_id))
    if (!r || r.turn.status !== 'streaming') return { stopped: false }
    for (const t of chatState.timers.get(r.turn.id) ?? []) clearTimeout(t)
    chatState.timers.delete(r.turn.id)
    r.turn.status = 'stopped'
    r.turn.activity = (r.turn.activity ?? []).map((g) => ({ ...g, active: false }))
    emitTurn(r.thread, r.turn, 'stopped')
    return { stopped: true }
  })

  on('chat_action_confirm', (a) => {
    const r = turnBy(String(a.turn_id))
    const act = r?.turn.actions?.find((x) => x.id === a.action_id)
    if (!r || !act) throw cmdError('not_found', 'That proposal is gone.')
    if (act.state !== 'pending') return act
    if (!a.confirm) {
      act.state = 'rejected'
      act.result = null
      return { ...act }
    }
    // Executes the same operation the UI would (03 §7.5): Create note writes through the notes mock.
    if (act.kind === 'create_note') {
      const title = act.fields.find((f) => f.label === 'Title')?.value ?? 'New note'
      const body = `---\nstatus: todo\n---\n# ${title}\n\n${act.preview ?? ''}\n`
      const created = vault?.create('f-unsorted', body)
      act.state = 'done'
      act.result = `Created note ${title} in Unsorted/`
      act.result_route = created ? `notes/${created.id}` : null
    } else {
      act.state = 'done'
      act.result = `${act.title} done`
    }
    later(10, () => m.emit(CHAT_EVENTS.action, { turn_id: r.turn.id, thread_id: r.thread.id, action: { ...act } }))
    return { ...act }
  })

  /* ---- automations ------------------------------------------------------------------------------------ */

  const tzDev = deviceZone()

  function makeRow(d: AutomationDraft & { id: string }, base?: Partial<AutomationRow>): AutomationRow {
    const schedule: Schedule = { ...d.schedule, tz: d.schedule.tz || tzDev }
    const next = d.enabled ? nextFires(schedule, Date.now(), 1, tzDev)[0] : undefined
    return {
      id: d.id,
      name: d.name.trim(),
      enabled: d.enabled,
      trigger_kind: 'schedule',
      schedule,
      trigger_phrase: triggerPhrase(schedule, tzDev),
      action_kind: d.action_kind,
      action: d.action,
      action_phrase: actionPhrase(d.action_kind, d.action, folderLabel),
      notify_on_failure: d.notify_on_failure,
      keep_runs: d.keep_runs,
      position: base?.position ?? autoState.rows.length,
      last_run_at: base?.last_run_at ?? null,
      next_run_at: next ? iso(next.utc) : null,
      next_run_local: next ? next.local : null,
      last_run: base?.last_run ?? null,
      running: base?.running ?? false,
      created_at: base?.created_at ?? iso(Date.now()),
      updated_at: iso(Date.now()),
      problem: base?.problem ?? null,
    }
  }

  /** A wall time `hh:mm` on the day `dayOffset` from today, in the device zone. */
  const wallAt = (dayOffset: number, hh: number, mm: number) => {
    const t = wallOf(now0, tzDev)
    const d = new Date(Date.UTC(t.y, t.m - 1, t.d + dayOffset))
    return zonedToUtc(d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate(), hh, mm, tzDev)
  }
  const run = (id: string, automation: string, started: number, ms: number | null, state: RunRow['state'], result: Partial<RunRow> = {}): RunRow => ({
    id,
    automation_id: automation,
    trigger_kind: 'schedule',
    started_at: iso(started),
    finished_at: ms === null ? null : iso(started + ms),
    state,
    result_kind: null,
    result_id: null,
    error: null,
    duration_ms: ms,
    ...result,
  })

  const autoState = { rows: [] as AutomationRow[], runs: new Map<string, RunRow[]>(), runMs: 1500 }

  function seedAutomations() {
    const morning: AutomationDraft & { id: string } = {
      id: 'au-morning',
      name: 'Morning brief',
      enabled: true,
      schedule: { kind: 'weekdays', time: '08:00', tz: tzDev },
      action_kind: 'start_session',
      action: {
        cwd: MODE_REPO,
        prompt: 'Summarize today’s inbox and calendar into a note in Work/Mode; flag anything from Priya or #release.',
        permission_mode: 'acceptEdits',
        mode: 'headless',
        token_cap: 200_000,
      },
      notify_on_failure: true,
      keep_runs: 20,
    }
    const nightly: AutomationDraft & { id: string } = {
      id: 'au-nightly',
      name: 'Nightly test run',
      enabled: true,
      schedule: { kind: 'daily', time: '23:00', tz: tzDev },
      action_kind: 'start_session',
      action: { cwd: MODE_REPO, prompt: 'Run pytest and note failures', permission_mode: 'default', mode: 'headless', token_cap: null, profile_id: 'ag-runner' },
      notify_on_failure: true,
      keep_runs: 20,
    }
    const weekly: AutomationDraft & { id: string } = {
      id: 'au-changelog',
      name: 'Weekly changelog',
      enabled: false,
      schedule: { kind: 'weekly', time: '17:00', tz: tzDev, weekday: 5 },
      action_kind: 'ask_chat',
      action: { prompt: "Summarize this week's changes in mode", folder_id: 'f-mode', effort: 'thorough' },
      notify_on_failure: false,
      keep_runs: 10,
    }
    const mr = [
      run('r-m1', 'au-morning', wallAt(0, 8, 0), 134_000, 'finished', { result_kind: 'session', result_id: 's-rate', result_title: 'morning-brief' }),
      run('r-m2', 'au-morning', wallAt(-1, 8, 0), 118_000, 'finished', { result_kind: 'session', result_id: 's-rate', result_title: 'morning-brief' }),
      run('r-m3', 'au-morning', wallAt(-5, 8, 0), 12_000, 'failed', { error: 'backend unreachable' }),
      run('r-m4', 'au-morning', wallAt(-6, 8, 0), null, 'skipped', { error: 'Mode was closed past the catch-up window' }),
    ]
    const nr = [
      run('r-n1', 'au-nightly', wallAt(-1, 23, 0), 64_000, 'failed', { error: 'pytest exited with status 1 · 3 failures' }),
      run('r-n2', 'au-nightly', wallAt(-2, 23, 0), 71_000, 'finished', { result_kind: 'session', result_id: 's-invoice', result_title: 'nightly-tests' }),
    ]
    // Last Friday 17:00 (four days before the wireframes' Wednesday).
    const today = wallOf(now0, tzDev)
    const back = (today.wd - 5 + 7) % 7 || 7
    const wr = [run('r-w1', 'au-changelog', wallAt(-back, 17, 0), 41_000, 'finished', { result_kind: 'note', result_id: 'n-changelog', result_title: 'Draft v2 changelog' })]
    autoState.runs = new Map([
      ['au-morning', mr],
      ['au-nightly', nr],
      ['au-changelog', wr],
    ])
    autoState.rows = []
    for (const [d, runs] of [
      [morning, mr],
      [nightly, nr],
      [weekly, wr],
    ] as const) {
      autoState.rows.push(makeRow(d, { position: autoState.rows.length, last_run: runs[0]!, last_run_at: runs[0]!.started_at, created_at: iso(now0 - 30 * 86_400_000) }))
    }
  }
  seedAutomations()

  const pushAutomations = () =>
    m.emit(
      AUTOMATION_EVENTS.changed,
      autoState.rows.map((r) => ({ ...r })),
    )
  const autoBy = (id: string) => {
    const r = autoState.rows.find((x) => x.id === id)
    if (!r) throw cmdError('not_found', 'This automation no longer exists.')
    return r
  }
  const replaceRow = (row: AutomationRow) => {
    autoState.rows = autoState.rows.map((r) => (r.id === row.id ? row : r))
    if (!autoState.rows.some((r) => r.id === row.id)) autoState.rows.push(row)
  }
  const draftOf = (r: AutomationRow): AutomationDraft & { id: string } => ({
    id: r.id,
    name: r.name,
    enabled: r.enabled,
    schedule: r.schedule,
    action_kind: r.action_kind,
    action: r.action,
    notify_on_failure: r.notify_on_failure,
    keep_runs: r.keep_runs,
  })

  on('automations_list', () => autoState.rows.map((r) => ({ ...r })))
  on('automation_get', (a) => ({ ...autoBy(String(a.id)) }))
  on('automation_save', (a) => {
    const d = a.draft as AutomationDraft
    const fields = validateDraft(d)
    if (d.action_kind === 'start_session' && (d.action as StartSessionAction).cwd && !(d.action as StartSessionAction).cwd.startsWith('/'))
      fields.push({ field: 'action.cwd', message: 'That folder no longer exists.' })
    if (fields.length) throw cmdError('invalid', fields[0]!.message, fields)
    const base = d.id ? autoState.rows.find((r) => r.id === d.id) : undefined
    const row = makeRow({ ...d, id: d.id ?? nid('au'), action: JSON.parse(JSON.stringify(d.action)) as ActionJson }, base ?? { position: autoState.rows.length })
    replaceRow(row)
    if (!autoState.runs.has(row.id)) autoState.runs.set(row.id, [])
    pushAutomations()
    return { ...row }
  })
  // `scheduler::api::automation_duplicate`: "<name> copy", disabled, appended, saved.
  on('automation_duplicate', (a) => {
    const src = autoBy(String(a.id))
    const row = makeRow({
      id: nid('au'),
      name: `${src.name} copy`,
      enabled: false,
      schedule: { ...src.schedule },
      action_kind: src.action_kind,
      action: JSON.parse(JSON.stringify(src.action)) as ActionJson,
      notify_on_failure: src.notify_on_failure,
      keep_runs: src.keep_runs,
    })
    replaceRow(row)
    autoState.runs.set(row.id, [])
    pushAutomations()
    return { ...row }
  })
  // `scheduler::api::automation_examples`: the two cards of 03 §7B.5 as disabled drafts; the session one in the last used repository.
  on('automation_examples', (): AutomationDraft[] => {
    const last = [...sessionRows()].sort((x, y) => String((y as { started_at?: string }).started_at ?? '').localeCompare(String((x as { started_at?: string }).started_at ?? '')))[0]
    const cwd = last ? (last.repo_root ?? last.cwd ?? '') : ''
    return [
      { id: null, ...exampleDraft('morning', '', tzDev), position: null },
      { id: null, ...exampleDraft('nightly', cwd, tzDev), position: null },
    ]
  })
  on('automation_delete', (a) => {
    autoBy(String(a.id))
    autoState.rows = autoState.rows.filter((r) => r.id !== a.id)
    autoState.runs.delete(String(a.id))
    pushAutomations()
    return { deleted: true }
  })
  on('automation_set_enabled', (a) => {
    const r = autoBy(String(a.id))
    const row = makeRow({ ...draftOf(r), enabled: !!a.enabled }, r)
    replaceRow(row)
    pushAutomations()
    return { ...row }
  })
  on('automation_runs', (a) => {
    const runs = autoState.runs.get(String(a.id)) ?? []
    return runs.slice(0, typeof a.limit === 'number' ? a.limit : 50).map((r) => ({ ...r }))
  })
  on('automation_run_get', (a): RunDetail => {
    for (const [aid, runs] of autoState.runs) {
      const r = runs.find((x) => x.id === a.id)
      if (!r) continue
      const auto = autoBy(aid)
      const result_route = r.result_id ? (r.result_kind === 'session' ? `sessions/${r.result_id}` : r.result_kind === 'note' ? `notes/${r.result_id}` : null) : null
      return {
        run: { ...r },
        automation: { ...auto },
        automation_id: auto.id,
        automation_name: auto.name,
        trigger_phrase: auto.trigger_phrase,
        action_phrase: auto.action_phrase,
        result_title: r.result_title ?? null,
        result_route,
      }
    }
    throw cmdError('not_found', 'This run was pruned.')
  })
  on('automation_run_now', (a) => {
    const r = autoBy(String(a.id))
    if (r.running) throw cmdError('busy', 'A run of this automation is still running.')
    const started = Date.now()
    const runRow = run(nid('r'), r.id, started, null, 'running', { trigger_kind: 'manual' })
    autoState.runs.set(r.id, [runRow, ...(autoState.runs.get(r.id) ?? [])].slice(0, r.keep_runs))
    replaceRow({ ...r, running: true, last_run: runRow, last_run_at: runRow.started_at })
    pushAutomations()
    later(autoState.runMs, () => {
      const cur = autoState.rows.find((x) => x.id === r.id)
      if (!cur) return
      const finished: RunRow = { ...runRow, state: 'finished', finished_at: iso(Date.now()), duration_ms: Math.max(1000, Date.now() - started) }
      if (cur.action_kind === 'start_session') Object.assign(finished, { result_kind: 'session', result_id: 's-rate', result_title: (cur.action as StartSessionAction).cwd.split('/').pop() })
      else {
        const title = cur.action_kind === 'create_note' ? (cur.action as CreateNoteAction).title_pattern.replace('{date}', new Date().toISOString().slice(0, 10)) : cur.name
        const note = vault?.create((cur.action as { folder_id: string | null }).folder_id ?? 'f-unsorted', `# ${title}\n`)
        Object.assign(finished, { result_kind: 'note', result_id: note?.id ?? 'n-changelog', result_title: title })
      }
      autoState.runs.set(
        r.id,
        (autoState.runs.get(r.id) ?? []).map((x) => (x.id === runRow.id ? finished : x)),
      )
      replaceRow({ ...cur, running: false, last_run: finished, last_run_at: finished.started_at, problem: null })
      pushAutomations()
    })
    return { ...runRow }
  })
  on('schedule_preview', (a) => {
    const raw = typeof a.schedule_json === 'string' ? (JSON.parse(a.schedule_json) as unknown) : a.schedule_json
    const s = (raw ?? {}) as Schedule
    const tz = s.tz || tzDev
    const preview: SchedulePreview = { fires: [], fires_local: [], echo: null, error: null, valid: false, trigger_phrase: '', tz }
    if (s.kind === 'cron') {
      const e = validateCron(s.cron ?? '')
      if (e) return { ...preview, error: { field: 'schedule.cron', message: e } }
    } else if (!/^\d{1,2}:\d{2}$/.test(s.time ?? '')) return { ...preview, error: { field: 'schedule.time', message: 'Use HH:MM, 24 h.' } }
    const fires = nextFires(s, Date.now(), 5, tzDev)
    return { ...preview, fires: fires.map((f) => iso(f.utc)), fires_local: fires.map((f) => f.local), echo: describeSchedule(s), valid: true, trigger_phrase: triggerPhrase({ ...s, tz }, tzDev) }
  })

  /* ---- control ---------------------------------------------------------------------------------- */

  /* ---- Agents (03 §7E; mode_core::agents) ---------------------------------------------------- */

  type AgentRecord = AgentDraft & { id: string; memories: MemoryRow[]; usage: AgentView['usage']; live: number; last_used_at: string | null; created_at: string; updated_at: string }
  const agentState = { rows: [] as AgentRecord[] }
  const agentDraft = (id: string, name: string, patch: Partial<Omit<AgentDraft, 'id'>>): AgentDraft & { id: string } => ({
    id,
    name,
    title: null,
    instructions: null,
    avatar_variant: '',
    model: null,
    effort: null,
    fallback_models: [],
    ask_before_switching: true,
    skills: [],
    tables: [],
    permission_mode: null,
    allowed_tools: [],
    denied_tools: [],
    notify: true,
    workspace: null,
    claude_path: null,
    extra_args: [],
    isolate_config: false,
    provider_id: null,
    repos: [],
    connectors: [],
    rules: [],
    use_memories: true,
    ...patch,
  })
  const seedAgents = () => {
    const mem = (id: string, title: string, body: string, days: number): MemoryRow => ({ id, title, body, updated_at: iso(now0 - days * 86_400_000) })
    agentState.rows = [
      {
        ...agentDraft('ag-reviewer', 'Reviewer', {
          title: 'Reviews pull requests',
          instructions: 'Review the diff for correctness first, then style. Be terse; one finding per paragraph.',
          avatar_variant: 'specs',
          model: 'opus',
          effort: 'high',
          fallback_models: ['sonnet'],
          permission_mode: 'plan',
          allowed_tools: ['Read', 'Grep', 'Bash(git diff *)', 'Bash(git log *)'],
          denied_tools: ['WebSearch'],
          tables: ['Projects/mode.md'],
          workspace: MODE_REPO,
          repos: [MODE_REPO, '~/code/docs-site'],
          connectors: ['github'],
          rules: ['Never edit files; report only', 'Cite the file and line for every finding'],
        }),
        memories: [mem('mem-r1', 'Repo layout', 'Mode is a Tauri 2 app: crates/mode-core holds the engine, ui/ the React window.', 3), mem('mem-r2', 'Owner rules', 'No Claude attribution in commits. Keybinds appear only in the Shortcuts view.', 12)],
        usage: { sessions: 14, sessions_7d: 4, tokens: 2_140_000, tokens_7d: 480_000, turns: 96 },
        live: 1,
        last_used_at: iso(now0 - 2 * 3_600_000),
        created_at: iso(now0 - 40 * 86_400_000),
        updated_at: iso(now0 - 3 * 86_400_000),
      },
      {
        ...agentDraft('ag-writer', 'Writer', {
          title: 'Drafts notes and release text',
          instructions: 'Write in plain sentences. No marketing voice.',
          avatar_variant: 'beret',
          model: 'sonnet',
          effort: 'medium',
          skills: ['release-notes'],
          notify: false,
        }),
        memories: [mem('mem-w1', 'Voice', 'Short sentences, no em-dashes, no exclamation marks.', 7)],
        usage: { sessions: 6, sessions_7d: 1, tokens: 310_000, tokens_7d: 40_000, turns: 22 },
        live: 0,
        last_used_at: iso(now0 - 5 * 86_400_000),
        created_at: iso(now0 - 21 * 86_400_000),
        updated_at: iso(now0 - 7 * 86_400_000),
      },
      {
        ...agentDraft('ag-runner', 'Nightly runner', {
          title: 'Runs the test suite',
          model: 'haiku',
          effort: 'low',
          fallback_models: ['sonnet', 'opus'],
          ask_before_switching: false,
          permission_mode: 'acceptEdits',
          allowed_tools: ['Bash(pytest *)', 'Read'],
          workspace: MODE_REPO,
          isolate_config: true,
          provider_id: 'pv-openrouter',
        }),
        memories: [],
        usage: { sessions: 31, sessions_7d: 7, tokens: 1_020_000, tokens_7d: 230_000, turns: 31 },
        live: 0,
        last_used_at: iso(now0 - 15 * 3_600_000),
        created_at: iso(now0 - 60 * 86_400_000),
        updated_at: iso(now0 - 1 * 86_400_000),
      },
    ]
  }
  seedAgents()
  const agentBy = (id: string) => {
    const r = agentState.rows.find((x) => x.id === id)
    if (!r) throw cmdError('not_found', 'This agent no longer exists.')
    return r
  }
  const agentAutomations = (id: string): AgentView['automations'] =>
    autoState.rows
      .filter((r) => r.action_kind === 'start_session' && (r.action as StartSessionAction).profile_id === id)
      .map((r) => {
        const last = (autoState.runs.get(r.id) ?? [])[0]
        return { id: r.id, name: r.name, enabled: r.enabled, last_run_at: last?.started_at ?? null, last_state: last?.state ?? null }
      })
  const agentSummary = (r: AgentRecord): AgentSummary => ({
    id: r.id,
    name: r.name,
    title: r.title,
    avatar_variant: r.avatar_variant,
    model: r.model,
    effort: r.effort,
    notify: r.notify,
    sessions: r.usage.sessions,
    live: r.live,
    last_used_at: r.last_used_at,
    automated: agentAutomations(r.id).length > 0,
  })
  const agentView = (r: AgentRecord): AgentView => {
    const { live: _live, ...rest } = r
    return { ...rest, automations: agentAutomations(r.id) }
  }
  const pushAgents = () => m.emit(AGENT_EVENTS.changed, agentState.rows.map(agentSummary))

  on('agents_list', () => agentState.rows.map(agentSummary))
  on('agent_get', (a) => agentView(agentBy(String(a.id))))
  on('agent_save', (a) => {
    const d = a.draft as AgentDraft
    const fields = validateAgentDraft(d)
    if (fields.length) throw cmdError('invalid', 'The agent cannot be saved as it is.', fields)
    const clean = (s: string | null) => (s && s.trim() ? s.trim() : null)
    const base = d.id ? agentState.rows.find((r) => r.id === d.id) : undefined
    const row: AgentRecord = {
      ...(base ?? { memories: [], usage: { sessions: 0, sessions_7d: 0, tokens: 0, tokens_7d: 0, turns: 0 }, live: 0, last_used_at: null, created_at: iso(now0) }),
      ...d,
      id: base?.id ?? nid('ag'),
      name: d.name.trim(),
      title: clean(d.title),
      instructions: clean(d.instructions),
      model: clean(d.model),
      workspace: clean(d.workspace),
      claude_path: clean(d.claude_path),
      provider_id: clean(d.provider_id),
      fallback_models: d.fallback_models.map((s) => s.trim()).filter(Boolean),
      skills: d.skills.map((s) => s.trim()).filter(Boolean),
      tables: d.tables.map((s) => s.trim()).filter(Boolean),
      allowed_tools: d.allowed_tools.map((s) => s.trim()).filter(Boolean),
      denied_tools: d.denied_tools.map((s) => s.trim()).filter(Boolean),
      extra_args: d.extra_args.map((s) => s.trim()).filter(Boolean),
      repos: (d.repos ?? []).map((s) => s.trim()).filter(Boolean),
      connectors: (d.connectors ?? []).map((s) => s.trim()).filter(Boolean),
      rules: (d.rules ?? []).map((s) => s.trim()).filter(Boolean),
      use_memories: d.use_memories ?? true,
      updated_at: iso(now0),
    }
    agentState.rows = base ? agentState.rows.map((r) => (r.id === row.id ? row : r)) : [...agentState.rows, row]
    pushAgents()
    return agentView(row)
  })
  on('agent_delete', (a) => {
    agentBy(String(a.id))
    agentState.rows = agentState.rows.filter((r) => r.id !== a.id)
    for (const r of autoState.rows) {
      if (r.action_kind === 'start_session' && (r.action as StartSessionAction).profile_id === a.id) delete (r.action as StartSessionAction).profile_id
    }
    pushAgents()
    pushAutomations()
    return true
  })
  on('agent_memory_save', (a) => {
    const r = agentBy(String(a.profile_id))
    const title = String(a.title ?? '').trim()
    if (!title) throw cmdError('invalid', 'Give the memory a title.', [{ field: 'title', message: 'Give the memory a title.' }])
    const existing = a.id ? r.memories.find((x) => x.id === a.id) : undefined
    const row: MemoryRow = { id: existing?.id ?? nid('mem'), title, body: String(a.body ?? ''), updated_at: iso(now0) }
    r.memories = existing ? r.memories.map((x) => (x.id === row.id ? row : x)) : [...r.memories, row]
    r.updated_at = row.updated_at
    pushAgents()
    return row
  })
  on('agent_memory_delete', (a) => {
    for (const r of agentState.rows) r.memories = r.memories.filter((x) => x.id !== a.id)
    pushAgents()
    return true
  })
  on('skills_list', (a) => (a.workspace ? ['code-review', 'deploy', 'docs', 'release-notes', 'triage'] : ['code-review', 'docs', 'release-notes']))
  on('mcp_servers_list', (a) => [
    { name: 'github', scope: 'user', transport: 'stdio' },
    { name: 'linear', scope: 'user', transport: 'http' },
    ...(Array.isArray(a.folders) && a.folders.length ? [{ name: 'sentry', scope: 'local', transport: 'http' }] : []),
  ])

  /* ---- the Marketplace (03 §7C.6): the catalog, packs, import with a preview, install, export ---- */

  const INDEX_URL = 'https://raw.githubusercontent.com/gil10101/mode_cat/main/index.json'
  const PACK_BASE = 'https://raw.githubusercontent.com/gil10101/mode_cat/main/packs/'
  type MockAgentItem = Partial<Omit<AgentDraft, 'id'>> & { key: string; name: string }
  type MockAutomationItem = { key: string; name: string; schedule: Schedule; action_kind: ActionKind; action: Record<string, unknown>; notify_on_failure?: boolean; keep_runs?: number }
  type MockPack = { format: number; kind: string; slug: string; name: string; version?: string; author?: string; description?: string; agents: MockAgentItem[]; automations: MockAutomationItem[]; themes: Record<string, unknown>[] }
  const slugify = (name: string) => name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48) || 'pack'
  const countsLine = (agents: number, automations: number, themes: number) => {
    const w = (n: number, one: string, many: string) => (n === 1 ? `1 ${one}` : `${n} ${many}`)
    const parts = [agents && w(agents, 'agent', 'agents'), automations && w(automations, 'automation', 'automations'), themes && w(themes, 'theme', 'themes')].filter(Boolean)
    return parts.length ? parts.join(' · ') : 'empty'
  }
  const oceanDoc = (): Record<string, unknown> => {
    const set = SETS['nord']!
    return { format: 1, slug: 'ocean', name: 'Ocean', author: 'Mode', version: '1', appearance: 'dark', colors: { dark: { m: Object.fromEntries(ROLES.map((r, i) => [r, set.m[i]])) } } }
  }
  const swatchOf = (set: ColorSet | undefined): Swatch | undefined => (set ? { bg: set.m[0]!, surface: set.m[1]!, fg: set.m[7]!, accent: set.m[10]!, ok: set.m[12]!, info: set.m[14]! } : undefined)
  const catalogPacks: MockPack[] = [
    {
      format: 1, kind: 'mode-pack', slug: 'reviewer-starter', name: 'Reviewer starter', version: '1', author: 'Mode', description: 'A pull-request reviewer agent with a nightly review session.',
      agents: [{ key: 'reviewer', name: 'Reviewer', title: 'Reviews pull requests', instructions: 'Review the open pull requests of this repository.', avatar_variant: 'specs', model: 'claude-opus-5-5', effort: 'high', fallback_models: ['claude-sonnet-5-5'], skills: [], permission_mode: 'plan', allowed_tools: ['Read', 'Grep', 'Bash(git diff *)'], denied_tools: ['WebSearch'] }],
      automations: [{ key: 'nightly-review', name: 'Nightly review', schedule: { kind: 'weekdays', time: '23:00' }, action_kind: 'start_session', action: { cwd: '~/code/project', prompt: 'Review every open pull request and leave your findings as comments.', permission_mode: 'plan', mode: 'headless', token_cap: 400000, profile_key: 'reviewer' }, notify_on_failure: true, keep_runs: 20 }],
      themes: [],
    },
    {
      format: 1, kind: 'mode-pack', slug: 'daily-plan', name: 'Daily plan', version: '1', author: 'Mode', description: "A weekday note with the day's plan, created before you sit down.",
      agents: [],
      automations: [{ key: 'daily-plan', name: 'Daily plan', schedule: { kind: 'weekdays', time: '08:00' }, action_kind: 'create_note', action: { title_pattern: 'Daily plan {date}', tags: ['plan'] }, notify_on_failure: false, keep_runs: 20 }],
      themes: [],
    },
    { format: 1, kind: 'mode-pack', slug: 'ocean', name: 'Ocean', version: '1', author: 'Mode', description: 'A cool dark set with a calm accent.', agents: [], automations: [], themes: [oceanDoc()] },
  ]
  const packState = {
    fail: null as string | null,
    fetched: 0,
    exported: [] as { path: string; pack: MockPack }[],
    packs: new Map<string, MockPack>(catalogPacks.map((p) => [`${PACK_BASE}${p.slug}.mode-pack.json`, p])),
    /** The last catalog fetched: a failed refresh answers with it beside the error, as the core does. */
    held: null as Catalogue | null,
  }
  const indexUrl = () => String(settingsValues()['marketplace.index_url'] ?? INDEX_URL)
  const indexHost = () => {
    try {
      return new URL(indexUrl()).host
    } catch {
      return ''
    }
  }
  const catalogueNow = () => ({
    url: indexUrl(),
    host: indexHost(),
    name: 'Mode packs',
    fetched_at: iso(now0 - 2 * 60_000),
    packs: catalogPacks.map((p) => {
      const first = p.themes[0] ? validatePack(JSON.stringify(p.themes[0])).pack : null
      const sw = first ? swatchOf(first.dark ?? first.light) : undefined
      return { slug: p.slug, name: p.name, description: p.description ?? null, author: p.author ?? null, version: p.version ?? null, url: `${PACK_BASE}${p.slug}.mode-pack.json`, agents: p.agents.length, automations: p.automations.length, themes: p.themes.length, counts: countsLine(p.agents.length, p.automations.length, p.themes.length), tags: [], colors: sw ? [sw.bg, sw.surface, sw.fg, sw.accent, sw.ok, sw.info] : [], avatars: p.agents.map((a) => a.avatar_variant ?? '').filter(Boolean) }
    }),
  })
  on('catalogue_get', () => {
    packState.fetched++
    if (!/^https:\/\//.test(indexUrl())) return { url: indexUrl(), host: indexHost(), catalogue: null, error: `The catalog must be fetched over https, not ${indexUrl().split(':')[0]}.` }
    if (packState.fail) return { url: indexUrl(), host: indexHost(), catalogue: packState.held, error: packState.fail }
    packState.held = catalogueNow()
    return { url: indexUrl(), host: indexHost(), catalogue: packState.held, error: null }
  })

  const readPack = (a: Record<string, unknown>): { pack: MockPack; source: string } | { invalid: string; source: string } => {
    if (typeof a.url === 'string') {
      const p = packState.packs.get(a.url)
      if (!p) throw cmdError('invalid', `${a.url} is not on the catalog's host ${indexHost()}.`)
      return { pack: p, source: a.url }
    }
    if (typeof a.text !== 'string') throw cmdError('invalid', 'Give a file, the pack text or a catalog address.')
    const source = typeof a.path === 'string' ? (a.path.split(/[\\/]/).pop() ?? a.path) : 'pasted text'
    let doc: Record<string, unknown>
    try {
      doc = JSON.parse(a.text) as Record<string, unknown>
    } catch (e) {
      return { invalid: `Not a JSON document: ${e instanceof Error ? e.message : String(e)}.`, source }
    }
    if (!doc || typeof doc !== 'object' || Array.isArray(doc)) return { invalid: 'A pack is one JSON object.', source }
    if (doc.kind !== 'mode-pack') {
      if (doc.colors && typeof doc.format === 'number') return { pack: { format: 1, kind: 'mode-pack', slug: String(doc.slug ?? 'theme'), name: String(doc.name ?? 'Theme'), author: typeof doc.author === 'string' ? doc.author : undefined, version: typeof doc.version === 'string' ? doc.version : undefined, agents: [], automations: [], themes: [doc] }, source }
      return { invalid: typeof doc.kind === 'string' ? `This is a ${doc.kind} file, not a Mode pack.` : 'This file is not a Mode pack (no `kind`).', source }
    }
    if (typeof doc.format !== 'number' || doc.format < 1) return { invalid: '`format` is missing.', source }
    if (doc.format > 1) return { invalid: `This pack needs a newer Mode (format ${doc.format}; this Mode reads format 1).`, source }
    if (!/^[a-z0-9-]{2,48}$/.test(String(doc.slug ?? ''))) return { invalid: '`slug` must be 2–48 characters of a–z, 0–9 and -.', source }
    if (!String(doc.name ?? '').trim()) return { invalid: '`name` is missing.', source }
    return { pack: { ...(doc as unknown as MockPack), agents: (doc.agents as MockAgentItem[] | undefined) ?? [], automations: (doc.automations as MockAutomationItem[] | undefined) ?? [], themes: (doc.themes as Record<string, unknown>[] | undefined) ?? [] }, source }
  }
  const sameName = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase()
  const nextName = (name: string, taken: (n: string) => boolean) => {
    for (let n = 2; n < 100; n++) if (!taken(`${name} ${n}`)) return `${name} ${n}`
    return `${name} copy`
  }
  const agentDraftOf = (item: MockAgentItem): AgentDraft => {
    const { key: _key, ...rest } = item
    return { ...emptyAgentDraft(), ...rest, id: null, workspace: null, claude_path: null, provider_id: null, name: item.name.trim() }
  }
  const automationDraftOf = (item: MockAutomationItem, agentId: (key: string) => string | null, folder: string | null): AutomationDraft => {
    const action: Record<string, unknown> = { ...item.action }
    const key = typeof action.profile_key === 'string' ? action.profile_key : null
    delete action.profile_key
    delete action.profile_id
    const id = key ? agentId(key) : null
    if (id) action.profile_id = id
    if (folder && item.action_kind === 'start_session') action.cwd = folder
    return { id: null, name: item.name.trim(), enabled: false, schedule: { ...item.schedule }, action_kind: item.action_kind, action: action as unknown as ActionJson, notify_on_failure: item.notify_on_failure ?? true, keep_runs: item.keep_runs ?? 20 }
  }
  on('pack_import', async (a) => {
    const read = readPack(a)
    if ('invalid' in read) return { ok: false, outcome: 'invalid', slug: null, name: null, version: null, author: null, description: null, source: read.source, counts: '', items: [], message: read.invalid, installed: [] }
    const { pack, source } = read
    const folders = (a.folders as { key: string; cwd: string }[] | undefined) ?? []
    const folderFor = (key: string) => folders.find((f) => f.key === key)?.cwd?.trim() || null
    const items: ItemPreview[] = []
    for (const it of pack.agents) {
      const d = agentDraftOf(it)
      const issues = validateAgentDraft(d).filter((f) => !['workspace', 'claude_path', 'provider_id'].includes(f.field))
      const detail = [d.title, d.model, d.effort, d.skills.length ? `${d.skills.length} ${d.skills.length === 1 ? 'skill' : 'skills'}` : null].filter(Boolean).join(' · ') || 'agent'
      items.push({ kind: 'agent', key: it.key, name: d.name, detail, existing: agentState.rows.find((r) => sameName(r.name, d.name))?.name ?? null, issues, avatar_variant: d.avatar_variant })
    }
    for (const it of pack.automations) {
      const d = automationDraftOf(it, () => null, null)
      let issues = validateDraft(d)
      let needs_folder: string | null = null
      if (it.action_kind === 'start_session') {
        const cwd = String((it.action as { cwd?: unknown }).cwd ?? '')
        if (!cwd.startsWith('/')) {
          needs_folder = cwd
          issues = issues.filter((f) => f.field !== 'action.cwd')
        }
      }
      const detail = `${triggerPhrase({ ...d.schedule, tz: d.schedule.tz || tzDev }, tzDev)} · ${actionPhrase(d.action_kind, d.action, folderLabel)}`
      items.push({ kind: 'automation', key: it.key, name: d.name, detail, existing: autoState.rows.find((r) => sameName(r.name, d.name))?.name ?? null, issues, needs_folder })
    }
    pack.themes.forEach((doc, i) => {
      const v = validatePack(JSON.stringify(doc))
      const slug = String(doc.slug ?? '')
      const existing = slug ? livePacks().find((p) => p.slug === slug && p.kind !== 'builtin') : undefined
      const appearance = String(doc.appearance ?? 'auto')
      items.push({
        kind: 'theme',
        key: slug || `theme-${i + 1}`,
        name: String(doc.name ?? `Theme ${i + 1}`),
        detail: `${appearance}${typeof doc.author === 'string' && doc.author ? ` · by ${doc.author}` : ''}`,
        existing: existing?.name ?? null,
        issues: v.issues.filter((x) => x.severity === 'error').map((x) => ({ field: `themes[${i}].${x.path}`, message: x.message })),
        warnings: v.issues.filter((x) => x.severity === 'warning').map((x) => `${x.path} ${x.message}`),
        swatches: v.pack ? { light: swatchOf(v.pack.light), dark: swatchOf(v.pack.dark) } : null,
      })
    })
    const base: PackImportResult = { ok: true, outcome: 'preview', slug: pack.slug, name: pack.name, version: pack.version ?? null, author: pack.author ?? null, description: pack.description ?? null, source, counts: countsLine(pack.agents.length, pack.automations.length, pack.themes.length), items, installed: [] }
    if (!a.apply) return base
    const skip = new Set((a.skip as string[] | undefined) ?? [])
    const conflict = (a.on_conflict as 'replace' | 'keep_both' | undefined) ?? 'keep_both'
    const installed: InstalledItem[] = []
    const agentIds = new Map<string, string>()
    const failure = (e: unknown): { outcome: 'invalid' | 'failed'; message: string } => {
      const err = e as { code?: string; message?: string; fields?: { message: string }[] }
      return { outcome: err?.code === 'invalid' ? 'invalid' : 'failed', message: err?.fields?.[0]?.message ?? err?.message ?? String(e) }
    }
    for (const [i, it] of pack.agents.entries()) {
      const pv = items.filter((x) => x.kind === 'agent')[i]!
      const row: InstalledItem = { kind: 'agent', key: it.key, name: pv.name, outcome: 'skipped', id: null, message: null }
      if (skip.has(`agent:${it.key}`)) {
        const ex = agentState.rows.find((r) => sameName(r.name, it.name))
        if (ex) agentIds.set(it.key, ex.id)
        installed.push(row)
        continue
      }
      if (pv.issues.length) {
        installed.push({ ...row, outcome: 'invalid', message: pv.issues[0]!.message })
        continue
      }
      const d = agentDraftOf(it)
      const ex = agentState.rows.find((r) => sameName(r.name, d.name))
      let replaced = false
      if (ex && conflict === 'replace') {
        d.id = ex.id
        replaced = true
      } else if (ex) d.name = nextName(d.name, (n) => agentState.rows.some((r) => sameName(r.name, n)))
      try {
        const view = (await m.answer('agent_save', { draft: d })) as AgentView
        agentIds.set(it.key, view.id)
        installed.push({ ...row, outcome: replaced ? 'replaced' : 'created', id: view.id, name: d.name })
      } catch (e) {
        installed.push({ ...row, ...failure(e) })
      }
    }
    for (const [i, it] of pack.automations.entries()) {
      const pv = items.filter((x) => x.kind === 'automation')[i]!
      const row: InstalledItem = { kind: 'automation', key: it.key, name: pv.name, outcome: 'skipped', id: null, message: null }
      if (skip.has(`automation:${it.key}`)) {
        installed.push(row)
        continue
      }
      if (pv.issues.length) {
        installed.push({ ...row, outcome: 'invalid', message: pv.issues[0]!.message })
        continue
      }
      const folder = folderFor(it.key)
      if (pv.needs_folder && !folder) {
        installed.push({ ...row, outcome: 'invalid', message: 'Choose a folder for its sessions.' })
        continue
      }
      const d = automationDraftOf(it, (k) => agentIds.get(k) ?? null, folder)
      const ex = autoState.rows.find((r) => sameName(r.name, d.name))
      let replaced = false
      if (ex && conflict === 'replace') {
        d.id = ex.id
        replaced = true
      } else if (ex) d.name = nextName(d.name, (n) => autoState.rows.some((r) => sameName(r.name, n)))
      try {
        const saved = (await m.answer('automation_save', { draft: d })) as AutomationRow
        installed.push({ ...row, outcome: replaced ? 'replaced' : 'created', id: saved.id, name: d.name })
      } catch (e) {
        installed.push({ ...row, ...failure(e) })
      }
    }
    for (const [i, doc] of pack.themes.entries()) {
      const pv = items.filter((x) => x.kind === 'theme')[i]!
      const row: InstalledItem = { kind: 'theme', key: pv.key, name: pv.name, outcome: 'skipped', id: null, message: null }
      if (skip.has(`theme:${pv.key}`)) {
        installed.push(row)
        continue
      }
      if (pv.issues.length) {
        installed.push({ ...row, outcome: 'invalid', message: pv.issues[0]!.message })
        continue
      }
      try {
        const r = (await m.answer('theme_import', { text: JSON.stringify(doc), apply: true, on_conflict: conflict })) as ThemeImportResult
        if (r.saved) installed.push({ ...row, outcome: r.outcome === 'replaced' ? 'replaced' : 'created', id: r.row?.id ?? null, name: r.name ?? pv.name })
        else installed.push({ ...row, outcome: 'invalid', message: r.message ?? r.issues[0]?.message ?? 'refused' })
      } catch (e) {
        installed.push({ ...row, ...failure(e) })
      }
    }
    const written = installed.filter((x) => x.outcome === 'created' || x.outcome === 'replaced').length
    const failed = installed.filter((x) => x.outcome === 'invalid' || x.outcome === 'failed').length
    const w = `${written} ${written === 1 ? 'item' : 'items'}`
    return { ...base, outcome: failed ? 'partial' : 'installed', installed, message: written === 0 && failed === 0 ? 'Nothing was chosen.' : failed ? `Installed ${w}; ${failed} could not be.` : `Installed ${w}.` }
  })

  on('pack_export', (a) => {
    const ids = (k: string) => ((a[k] as string[] | undefined) ?? []).map(String)
    const agentIds = ids('agents')
    const autoIds = ids('automations')
    const themeIds = ids('themes')
    if (!agentIds.length && !autoIds.length && !themeIds.length) throw cmdError('invalid', 'Choose at least one item for the pack.')
    const single = agentIds.length + autoIds.length + themeIds.length === 1
    let name = typeof a.name === 'string' && a.name.trim() ? a.name.trim() : 'Mode pack'
    if (!(typeof a.name === 'string' && a.name.trim()) && single) name = agentIds[0] ? agentBy(agentIds[0]).name : autoIds[0] ? autoBy(autoIds[0]).name : (packBy(themeIds[0]!)?.name ?? name)
    const slug = typeof a.slug === 'string' && a.slug ? a.slug : slugify(name)
    if (!/^[a-z0-9-]{2,48}$/.test(slug)) throw cmdError('invalid', `"${slug}" is not a slug (a–z, 0–9 and -).`)
    const keys = new Map<string, string>()
    const taken = new Set<string>()
    const uniqueKey = (n: string) => {
      const b = slugify(n)
      let k = b
      for (let i = 2; taken.has(k); i++) k = `${b}-${i}`
      taken.add(k)
      return k
    }
    const agents: MockAgentItem[] = agentIds.map((id) => {
      const r = agentBy(id)
      const key = uniqueKey(r.name)
      keys.set(r.id, key)
      const { id: _id, memories: _m, usage: _u, live: _l, last_used_at: _la, created_at: _c, updated_at: _up, workspace: _w, claude_path: _cp, provider_id: _p, ...rest } = r
      return { key, ...rest }
    })
    taken.clear()
    const automations: MockAutomationItem[] = autoIds.map((id) => {
      const r = autoBy(id)
      const action: Record<string, unknown> = { ...(r.action as unknown as Record<string, unknown>) }
      const pid = typeof action.profile_id === 'string' ? action.profile_id : null
      delete action.profile_id
      delete action.folder_id
      delete action.template_note_id
      if (pid && keys.get(pid)) action.profile_key = keys.get(pid)
      return { key: uniqueKey(r.name), name: r.name, schedule: { ...r.schedule }, action_kind: r.action_kind, action, notify_on_failure: r.notify_on_failure, keep_runs: r.keep_runs }
    })
    const themes = themeIds.map((id) => {
      const p = packBy(id)
      if (!p) throw cmdError('not_found', `No theme pack ${id}.`)
      return (p.document as Record<string, unknown> | undefined) ?? { format: 1, slug: p.slug, name: p.name, appearance: p.appearance, author: p.author, version: p.version, colors: Object.fromEntries((['light', 'dark'] as const).filter((w) => p[w]).map((w) => [w, { m: Object.fromEntries(ROLES.map((r, i) => [r, p[w]!.m[i]])) }])) }
    })
    const pack: MockPack = { format: 1, kind: 'mode-pack', slug, name, version: '1', author: typeof a.author === 'string' && a.author.trim() ? a.author.trim() : undefined, description: typeof a.description === 'string' && a.description.trim() ? a.description.trim() : undefined, agents, automations, themes }
    const path = typeof a.path === 'string' && a.path ? a.path : `${HOME}/Downloads/${slug}.mode-pack.json`
    packState.exported.push({ path, pack })
    return { path, slug, name, counts: countsLine(agents.length, automations.length, themes.length) }
  })

  const control: V1MockControl = {
    chat: {
      status(patch) {
        Object.assign(status, patch)
        m.emit(EVENTS.settings, { 'ai.backend': status.backend })
      },
      speed(ms) {
        chatState.speed = ms
      },
      threads: () => chatState.threads,
    },
    automations: {
      rows: () => autoState.rows,
      problem(id, p) {
        replaceRow({ ...autoBy(id), problem: p })
        pushAutomations()
      },
      clear() {
        autoState.rows = []
        autoState.runs.clear()
        pushAutomations()
      },
      runMs(ms) {
        autoState.runMs = ms
      },
    },
    themes: {
      packs: () => livePacks().map((p) => rowOf(p, String(settingsValues()['theme.active'] ?? 'system'))),
      resolved: () => resolve(),
    },
    packs: {
      fail(message) {
        packState.fail = message
      },
      exported: () => packState.exported,
      fetched: () => packState.fetched,
    },
  }
  if (typeof window !== 'undefined') window.__mode_v1 = control
  return control
}

/* ---- theme pack validation (10 §4.3), the browser mock's reading -------------------------------- */

const HEX = /^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$/

function validatePack(text: string): { issues: PackIssue[]; pack: Pack | null } {
  const issues: PackIssue[] = []
  let doc: Record<string, unknown>
  try {
    doc = JSON.parse(text) as Record<string, unknown>
  } catch {
    return { issues: [{ severity: 'error', kind: 'structure', path: '$', message: 'This file is not JSON.' }], pack: null }
  }
  if (!doc || typeof doc !== 'object' || Array.isArray(doc)) return { issues: [{ severity: 'error', kind: 'structure', path: '$', message: 'A pack is one JSON object.' }], pack: null }
  if (/url\(|expression\(|<|javascript:/i.test(text)) issues.push({ severity: 'error', kind: 'structure', path: '$', message: 'Packs are tokens only.' })
  const format = doc.format
  if (typeof format !== 'number' || !Number.isInteger(format)) issues.push({ severity: 'error', kind: 'structure', path: 'format', message: '`format` is missing.' })
  else if (format > 1) issues.push({ severity: 'error', kind: 'structure', path: 'format', message: `This pack needs a newer Mode (format ${format}; this Mode reads format 1).` })
  const slug = typeof doc.slug === 'string' ? doc.slug : ''
  if (!/^[a-z0-9-]{2,32}$/.test(slug)) issues.push({ severity: 'error', kind: 'structure', path: 'slug', message: '`slug` must be 2–32 characters of a–z, 0–9 and -.' })
  const name = typeof doc.name === 'string' ? doc.name : ''
  if (!name) issues.push({ severity: 'error', kind: 'structure', path: 'name', message: '`name` is missing.' })
  const appearance = doc.appearance as PackAppearance
  if (!['light', 'dark', 'auto'].includes(String(appearance))) issues.push({ severity: 'error', kind: 'structure', path: 'appearance', message: '`appearance` must be light, dark or auto.' })
  const colors = (doc.colors ?? {}) as Record<string, Record<string, unknown> | undefined>
  const want = appearance === 'auto' ? ['light', 'dark'] : appearance ? [appearance] : []
  const sets: Partial<Record<'light' | 'dark', ColorSet>> = {}
  for (const which of want as ('light' | 'dark')[]) {
    const c = colors[which]
    if (!c) {
      issues.push({ severity: 'error', kind: 'structure', path: `colors.${which}`, message: `\`appearance\` is \`${appearance}\` but \`colors.${which}\` is missing.` })
      continue
    }
    const mm = (c.m ?? {}) as Record<string, unknown>
    const roles: string[] = []
    for (const r of ROLES) {
      const v = mm[r]
      if (typeof v !== 'string' || !HEX.test(v)) issues.push({ severity: 'error', kind: 'structure', path: `colors.${which}.m.${r}`, message: `\`colors.${which}.m.${r}\` is missing.` })
      roles.push(typeof v === 'string' ? v : '#000000')
    }
    const fallback = SETS[which === 'dark' ? 'system-dark' : 'system-light']!
    const avatar = Array.isArray(c.avatar) && c.avatar.length === 12 && c.avatar.every((h) => typeof h === 'string' && HEX.test(h)) ? (c.avatar as string[]) : null
    if (!avatar) issues.push({ severity: 'fill', kind: 'fill', path: `colors.${which}.avatar`, message: 'avatar palette generated' })
    if (!c.terminal) issues.push({ severity: 'fill', kind: 'fill', path: `colors.${which}.terminal`, message: `System ${which === 'dark' ? 'Dark' : 'Light'} terminal palette used` })
    sets[which] = { m: roles, avatar: avatar ?? fallback.avatar, term: fallback.term }
    if (roles.length === 15 && HEX.test(roles[8]!) && HEX.test(roles[1]!)) {
      const ratio = contrast(roles[8]!.slice(0, 7), roles[1]!.slice(0, 7))
      if (ratio < 4.5) issues.push({ severity: 'warning', kind: 'contrast', ratio: Math.round(ratio * 100) / 100, path: `colors.${which}.m.fg-2`, message: `\`--m-fg-2\` on \`--m-surface\` is ${ratio.toFixed(1)}:1 (target 4.5:1).` })
    }
  }
  if (issues.some((i) => i.severity === 'error')) return { issues, pack: null }
  return {
    issues,
    pack: {
      id: '',
      slug,
      name,
      kind: 'imported',
      appearance,
      author: typeof doc.author === 'string' ? doc.author : '',
      version: typeof doc.version === 'string' ? doc.version : '',
      mono: FONT_MONO,
      light: sets.light,
      dark: sets.dark,
      document: doc,
    },
  }
}
