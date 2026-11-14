// The sample a Stats view draws without a core (the browser mock) and the
// wireframe is generated from (docs/wireframes/tools/stats-markup): a year of
// days, then the summary of any window derived from them the way the core
// derives it, so the mock and the wireframe always agree. Deterministic.
import type { SessionStateCount, StatsAgentRow, StatsDay, StatsModelRow, StatsSummary, StatsSummaryDay, TokenSplit } from '../../state/contract-settings'
import { addDays, dayValue, daysBetween, MAX_WEEKS, statsRange, streaks, tipText, trackerModel, weekStartOf } from './tracker'

/** The wireframe's clock: Wed 21 Oct 2026, 14:02. */
export const WIREFRAME_TODAY = '2026-10-21'

/** The figures the wireframe and the dev gallery draw: this week from a Monday, the tooltip of Fri 16 Oct open. */
export function wireframeSample() {
  const days = sampleDays(WIREFRAME_TODAY)
  const range = statsRange(WIREFRAME_TODAY, 'this_week', 'monday')
  const summary = sampleSummary(days, range.from, range.to, WIREFRAME_TODAY)
  const model = trackerModel(Array.from(days.values()), WIREFRAME_TODAY, 'monday')
  const byDate = new Map(Array.from(days.values()).map((d) => [d.date, d]))
  const tips = model.dates.map((d) => tipText(d, byDate.get(d)))
  const openIndex = model.dates.indexOf('2026-10-16')
  return { days, summary, model, tips, openIndex, range }
}

/** Activity on the last 16 weeks as the first wireframe drew them (column-major from the Monday 15 weeks before today's week). */
const RECENT_LEVELS = '001000000000000110000010000001000002011201021201002231002302010232200033222001323312231132022233002330311423'

/** One (sessions, files) pair per level, so the quartiles reproduce the levels. */
const LEVEL_SPLITS: Record<string, readonly [number, number][]> = {
  '1': [
    [1, 1],
    [0, 2],
  ],
  '2': [
    [2, 4],
    [3, 3],
    [1, 5],
  ],
  '3': [
    [3, 9],
    [4, 8],
    [2, 10],
  ],
  '4': [[6, 14]],
}

/** The year of days: the wireframe's 16 weeks, an earlier 23-day streak, sparse days before it. */
export function sampleDays(today: string): Map<string, StatsDay> {
  const days = new Map<string, StatsDay>()
  const lastStart = weekStartOf(today, 'monday')
  const visibleFrom = addDays(lastStart, -7 * 15)
  const from = addDays(lastStart, -7 * (MAX_WEEKS - 1))
  let seed = 7
  const rnd = () => {
    seed = (seed * 1103515245 + 12345) % 2147483648
    return seed / 2147483648
  }
  const streakStart = addDays(visibleFrom, -120)
  for (const date of daysBetween(from, today)) {
    let sessions = 0
    let files = 0
    if (date >= visibleFrom) {
      const i = daysBetween(visibleFrom, date).length - 1
      const level = RECENT_LEVELS[i] ?? '0'
      const splits = LEVEL_SPLITS[level]
      if (splits) [sessions, files] = splits[i % splits.length]!
    } else {
      const inStreak = date >= streakStart && date < addDays(streakStart, 23)
      // A gap on either side of the 23-day run keeps it the longest.
      const nearStreak = date === addDays(streakStart, -1) || date === addDays(streakStart, 23)
      if (inStreak || (!nearStreak && rnd() < 0.45)) {
        sessions = 1 + Math.floor(rnd() * 4)
        files = Math.floor(rnd() * 12)
      }
    }
    days.set(date, { date, sessions, files, tokens: (sessions + files) * 17_500 })
  }
  // This week as the wireframe's header draws it: 1.2M tokens · 14 sessions.
  const week: [number, number, number][] = [
    [6, 14, 418_300],
    [3, 3, 470_000],
    [5, 7, 310_200],
  ]
  week.forEach(([s, f, t], k) => {
    const date = addDays(lastStart, k)
    if (date <= today) days.set(date, { date, sessions: s, files: f, tokens: t })
  })
  // The day the wireframe's tooltip names.
  const fri = addDays(lastStart, -3)
  if (days.has(fri)) days.set(fri, { date: fri, sessions: 3, files: 9, tokens: 210_000 })
  return days
}

/* ---- the summary of a window -------------------------------------------------------- */

/** List rates, USD per million tokens: input, 5-minute cache write, 1-hour cache write, cache read, output. */
const RATES: Record<string, readonly [number, number, number, number, number]> = {
  'claude-opus-5-5': [4, 5, 8, 0.2, 20],
  'claude-haiku-5-5': [0.1, 0.125, 0.2, 0.01, 0.5],
  'claude-sonnet-5-5': [2, 2.5, 4, 0.1, 10],
}

/** The share of a day's tokens per model and source. */
const MODELS: readonly { model: string; source: string; share: number }[] = [
  { model: 'claude-opus-5-5', source: 'claude_code', share: 0.78 },
  { model: 'claude-haiku-5-5', source: 'claude_code', share: 0.2 },
  { model: 'claude-sonnet-5-5', source: 'claude_api', share: 0.02 },
]

/** input · output · cache read · cache write, of a model's tokens. */
const KINDS = [0.03, 0.08, 0.82, 0.07] as const

/** Claude Code messages per token of the day. */
const TOKENS_PER_MESSAGE = 2_900

/** How the day's messages fall over the local hours (peak at 15:00). */
const HOUR_SHAPE: readonly number[] = [0, 0, 0, 0, 0, 0, 0, 1, 3, 6, 8, 9, 7, 8, 10, 12, 11, 9, 6, 4, 3, 2, 1, 0]

function kinds(tokens: number): { input: number; output: number; cache_read: number; cache_write: number } {
  const input = Math.round(tokens * KINDS[0])
  const output = Math.round(tokens * KINDS[1])
  const cache_read = Math.round(tokens * KINDS[2])
  return { input, output, cache_read, cache_write: tokens - input - output - cache_read }
}

function spendOf(model: string, k: { input: number; output: number; cache_read: number; cache_write: number }): number {
  const r = RATES[model]!
  return (k.input * r[0] + k.cache_write * r[1] + k.cache_read * r[3] + k.output * r[4]) / 1_000_000
}

/** The sessions the agents table lists for a week with usage, their share of the Claude Code tokens. */
const AGENTS: readonly { id: string; name: string; variant: string; parent: string | null; model: string; state: string; share: number; duration_s: number }[] = [
  { id: 's-api', name: 'api-refactor', variant: 'beret', parent: null, model: 'claude-opus-5-5', state: 'waiting', share: 0.27, duration_s: 18_720 },
  { id: 's-docs', name: 'docs-site', variant: 'specs', parent: null, model: 'claude-opus-5-5', state: 'working', share: 0.16, duration_s: 7_260 },
  { id: 's-rate', name: 'rate-limiter', variant: 'beanie', parent: null, model: 'claude-opus-5-5', state: 'done', share: 0.14, duration_s: 4_980 },
  { id: 's-invoice', name: 'invoice-parser', variant: 'tophat', parent: null, model: 'claude-opus-5-5', state: 'failed', share: 0.11, duration_s: 3_600 },
  { id: 'a-reviewer', name: 'code-reviewer', variant: 'beret', parent: 's-api', model: 'claude-haiku-5-5', state: 'working', share: 0.09, duration_s: 640 },
  { id: 's-readme', name: 'readme-tidy', variant: 'mustache', parent: null, model: 'claude-haiku-5-5', state: 'unread', share: 0.08, duration_s: 1_500 },
  { id: 'a-runner', name: 'test-runner', variant: 'beret', parent: 's-api', model: 'claude-haiku-5-5', state: 'done', share: 0.06, duration_s: 410 },
  { id: 's-webhook', name: 'webhook-spike', variant: 'cap', parent: null, model: 'claude-opus-5-5', state: 'idle', share: 0.05, duration_s: 2_100 },
  { id: 's-scratch', name: 'scratch', variant: 'squares', parent: null, model: 'claude-opus-5-5', state: 'untracked', share: 0.04, duration_s: 900 },
]

export function sampleByState(from: string, to: string, today: string): Record<SessionStateCount, number> {
  const thisWeek = from <= today && today <= to
  if (thisWeek && daysBetween(from, to).length <= 7) return { waiting: 1, working: 3, failed: 1, finished: 6, idle: 2, untracked: 1 }
  if (thisWeek) return { waiting: 1, working: 3, failed: 2, finished: 31, idle: 4, untracked: 3 }
  return { waiting: 0, working: 0, failed: 1, finished: 17, idle: 1, untracked: 2 }
}

interface Acc {
  tokens: number
  messages: number
  input: number
  output: number
  cache_read: number
  cache_write: number
  spend: number
}

function acc(): Acc {
  return { tokens: 0, messages: 0, input: 0, output: 0, cache_read: 0, cache_write: 0, spend: 0 }
}

/** The summary of `from ..= to` from the days (every figure derived the way the core derives it). */
export function sampleSummary(days: ReadonlyMap<string, StatsDay>, from: string, to: string, today: string): StatsSummary {
  const dates = daysBetween(from, to)
  const perModel = new Map<string, Acc>()
  const total = acc()
  const out: StatsSummaryDay[] = []
  let sessions = 0
  let files = 0
  let hoursTotal = 0
  let longest = 0
  for (const date of dates) {
    const d = date <= today ? days.get(date) : undefined
    const t = d?.tokens ?? 0
    const split: TokenSplit[] = []
    let daySpend = 0
    let dayMessages = 0
    for (const m of MODELS) {
      const mt = Math.round(t * m.share)
      if (!mt) continue
      const k = kinds(mt)
      const s = spendOf(m.model, k)
      const msgs = m.source === 'claude_code' ? Math.round(mt / TOKENS_PER_MESSAGE) : 0
      const key = `${m.model}\u0000${m.source}`
      const a = perModel.get(key) ?? acc()
      a.tokens += mt
      a.messages += msgs
      a.input += k.input
      a.output += k.output
      a.cache_read += k.cache_read
      a.cache_write += k.cache_write
      a.spend += s
      perModel.set(key, a)
      split.push({ model: m.model, source: m.source, tokens: mt })
      daySpend += s
      dayMessages += msgs
      total.tokens += mt
      total.input += k.input
      total.output += k.output
      total.cache_read += k.cache_read
      total.cache_write += k.cache_write
      total.spend += s
    }
    total.messages += dayMessages
    hoursTotal += dayMessages
    sessions += d?.sessions ?? 0
    files += d?.files ?? 0
    if (d && d.sessions > 0) longest = Math.max(longest, 3_600 + d.sessions * 1_200 + d.files * 60)
    out.push({ date, sessions: d?.sessions ?? 0, messages: dayMessages, tokens: t, spend_usd: t ? daySpend : 0, split: split.sort((a, b) => b.tokens - a.tokens) })
  }
  const by_model: StatsModelRow[] = Array.from(perModel.entries())
    .map(([key, a]) => {
      const [model, source] = key.split('\u0000') as [string, string]
      return { model, source, messages: a.messages, tokens: a.tokens, input: a.input, output: a.output, cache_read: a.cache_read, cache_write: a.cache_write, spend_usd: a.spend }
    })
    .sort((a, b) => b.tokens - a.tokens)
  const cc = by_model.filter((r) => r.source === 'claude_code')
  const ccTokens = cc.reduce((n, r) => n + r.tokens, 0)
  const favorite = cc[0] ? { model: cc[0].model, share: ccTokens ? cc[0].tokens / ccTokens : 0 } : null
  const shapeSum = HOUR_SHAPE.reduce((n, w) => n + w, 0)
  const hours = HOUR_SHAPE.map((w) => Math.round((hoursTotal * w) / shapeSum))
  const peakIndex = hours.indexOf(Math.max(...hours))
  const by_agent: StatsAgentRow[] = ccTokens
    ? AGENTS.map((a) => {
        const tokens = Math.round(ccTokens * a.share)
        const k = kinds(tokens)
        return {
          id: a.id,
          name: a.name,
          avatar_variant: a.variant,
          parent_id: a.parent,
          model: a.model,
          state: a.state,
          messages: Math.round(tokens / TOKENS_PER_MESSAGE),
          tokens,
          spend_usd: spendOf(a.model, k),
          duration_s: a.duration_s,
          started_at: `${from}T09:00:00.000Z`,
        }
      })
    : []
  const values = new Map<string, number>(Array.from(days.values()).map((d): [string, number] => [d.date, dayValue(d)]))
  const elapsed = dates.filter((d) => d <= today)
  const active = elapsed.filter((d) => (days.get(d)?.sessions ?? 0) + (days.get(d)?.files ?? 0) > 0).length
  const monthStart = `${today.slice(0, 7)}-01`
  const sum = (a: string, b: string) => daysBetween(a, b).reduce((n, d) => n + daySpendOf(days.get(d)), 0)
  const prompt = total.input + total.cache_read + total.cache_write
  return {
    from,
    to,
    spend: {
      window: total.spend,
      today: sum(today, today),
      month: sum(monthStart, today),
      all: sum(addDays(today, -399), today),
      unpriced_models: [],
      prices_as_of: '2026-10-29',
    },
    tokens: total.tokens,
    tokens_split: { input: total.input, output: total.output, cache_read: total.cache_read, cache_write: total.cache_write },
    messages: total.messages,
    sessions,
    agents: Math.floor(sessions * 0.6),
    files,
    lines_added: files * 33,
    lines_removed: files * 11,
    active_days: active,
    window_days: elapsed.length,
    streak: streaks(values, today),
    longest_session_s: longest,
    favorite_model: favorite,
    peak_hour: hoursTotal ? { hour: peakIndex, messages: hours[peakIndex]! } : null,
    cache_hit: prompt ? total.cache_read / prompt : null,
    by_model,
    by_agent,
    days: out,
    hours,
    by_state: sampleByState(from, to, today),
  }
}

/** The spend of one day of the sample. */
function daySpendOf(d: StatsDay | undefined): number {
  if (!d?.tokens) return 0
  let s = 0
  for (const m of MODELS) {
    const mt = Math.round(d.tokens * m.share)
    if (mt) s += spendOf(m.model, kinds(mt))
  }
  return s
}
