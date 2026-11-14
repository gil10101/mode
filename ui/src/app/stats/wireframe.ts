// The Stats wireframe, generated from the same sample and strings the app
// draws (sample.ts, text.ts, tracker.ts), so docs/wireframes/app-window.html
// and components.html stay the reference the parity suite compares the view
// against. `node scripts/stats-wireframe.mjs` (from ui/) bundles this file and
// splices its output into the two pages; `docs/wireframes/tools/embed.py` then
// inlines the stylesheet and the artwork as it does for every page.
import { WIREFRAME_TODAY as TODAY, wireframeSample as sample, type sampleSummary } from './sample'
import { ACTIVITY_META, agentTable, dayBars, dayTable, HOURS_HEADING, hourSection, metricTiles, modelTable, moreDays, spendNote, spendTiles, stateCounts, streakLine, type Cell, type TableSpec, type Tile } from './text'
import { modelShares, trackerModel, WINDOWS, windowLabel, type TrackerModel } from './tracker'
import { cellWidth, DAY_BARS, METRICS, NOTE, SHARES, SPEND, STREAK, TABLE_ROWS, type Share } from './skeleton'

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
const attr = (name: string, v: string | undefined) => (v === undefined ? '' : ` ${name}="${esc(v)}"`)

function seg(label: string, items: readonly string[], active: number): string {
  return `<span class="m-seg" role="group" aria-label="${esc(label)}">${items.map((t, i) => `<span class="m-seg-item${i === active ? ' is-active' : ''}">${esc(t)}</span>`).join('')}</span>`
}

function tiles(list: Tile[], className: string, id?: string): string {
  const t = list.map((x) => `<div class="m-stat${x.hero ? ' is-hero' : ''}"${attr('title', x.title)}><div class="m-stat-n">${esc(x.value)}</div><div class="m-stat-l">${esc(x.label)}</div></div>`).join('')
  return `<div class="${className}"${attr('id', id)}>${t}</div>`
}

/** The kit's AgentAvatar at hue 210 (`avatar.8`), the light or dark mark. */
function avatar(variant: string, untracked: boolean, dark: boolean): string {
  const cls = untracked ? 'm-avatar-agent is-untracked' : 'm-avatar-agent'
  return `<span class="${cls}" style="--repo-hue:210"><img src="../../assets/logo/final/bot-${variant}-${dark ? 'dark' : 'light'}.png" alt=""></span>`
}

function cell(c: Cell, dark: boolean): string {
  if ('agent' in c) {
    const suffix = c.agent.suffix ? `<span class="is-dim">${esc(c.agent.suffix)}</span>` : ''
    return `<span class="aw-agent" role="link" tabindex="0">${avatar(c.agent.variant, c.agent.untracked, dark)}${esc(c.agent.name)}${suffix}</span>`
  }
  if (c.code) return `<code>${esc(c.text)}</code>`
  if (c.dim) return `<span class="is-dim">${esc(c.text)}</span>`
  return esc(c.text)
}

function table(spec: TableSpec, dark: boolean): string {
  const head = spec.columns.map((c) => `<th${c.num ? ' class="is-num"' : ''}>${esc(String(c.label))}</th>`).join('')
  const body = spec.rows
    .map((cells, r) => `<tr${attr('title', spec.rowTitles?.[r])}>${cells.map((c, i) => `<td${spec.columns[i]?.num ? ' class="is-num"' : ''}>${cell(c, dark)}</td>`).join('')}</tr>`)
    .join('')
  return `<table class="m-table" aria-label="${esc(spec.label)}"><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`
}

function hbars(rows: ReturnType<typeof modelShares>): string {
  const r = rows.map((x) => `<span class="m-hbar-l"${attr('title', x.title)}>${esc(String(x.label))}</span><span class="m-hbar"${attr('title', x.title)}><i style="--w:${Math.max(0, Math.min(100, x.share))}%"></i></span><span class="m-hbar-n">${esc(String(x.figure))}</span>`).join('')
  return `<div class="m-hbars">${r}</div>`
}

function statsBars(values: number[], labels: readonly string[], todayIndex: number | undefined, variant: 'days' | 'hours' | 'weeks', titles: readonly string[]): string {
  const mod = variant === 'hours' ? ' is-hours' : variant === 'weeks' ? ' is-weeks' : ''
  const n = variant === 'weeks' ? ` style="--n:${values.length}"` : ''
  const bars = values.map((v, i) => `<div class="m-stats-bar${i === todayIndex ? ' is-today' : ''}" style="--h:${v ? `${v}%` : '0'}"${attr('title', titles[i])}></div>`).join('')
  const days = labels.map((d) => `<span>${esc(d)}</span>`).join('')
  return `<div class="m-stats-bars${mod}"${n}>${bars}</div><div class="m-stats-days${mod}"${n}>${days}</div>`
}

/** The kit's Tracker with one tooltip drawn open. */
function tracker(model: TrackerModel, tips: string[], openIndex: number): string {
  const weeks = Math.max(1, Math.ceil(model.days.length / 7))
  const months = model.months.map((m) => `<span style="grid-column:${m.column}/span ${m.span ?? 2}">${esc(m.label)}</span>`).join('')
  const weekdays = model.weekdays.map((w) => `<span>${esc(w)}</span>`).join('')
  const grid = model.days
    .map((d, i) => {
      const open = i === openIndex
      const tip = open ? `<span class="m-tracker-tip">${esc(tips[i] ?? '')}</span>` : ''
      return `<i class="lvl-${d.level}${open ? ' is-open' : ''}" tabindex="0" style="grid-area:${(i % 7) + 1}/${Math.floor(i / 7) + 1}" role="gridcell" aria-label="${esc(tips[i] ?? '')}" data-date="${d.date}">${tip}</i>`
    })
    .join('')
  const legend = `<span class="m-tracker-legend">less&nbsp;<i class="lvl-0"></i><i class="lvl-1"></i><i class="lvl-2"></i><i class="lvl-3"></i><i class="lvl-4"></i>&nbsp;more</span>`
  return `<div class="m-tracker" style="--weeks:${weeks}"><div class="m-tracker-months">${months}</div><div class="m-tracker-days">${weekdays}</div><div class="m-tracker-grid">${grid}</div><div class="m-tracker-foot"><span>${esc(streakLine(model.streak, model.empty))}</span>${legend}</div></div>`
}

function stateRows(s: ReturnType<typeof sampleSummary>): string {
  return stateCounts(s)
    .map(({ state, count }) => `<div class="m-stat-row aw-link" role="button" tabindex="0" aria-label="${state}, ${count} sessions, open in Sessions"><span class="m-state is-${state}">${state}</span><span class="m-count">${count}</span></div>`)
    .join('')
}

/** The whole `<section data-view="stats">` of app-window.html. */
export function appWindowSection(): string {
  const { summary, model, tips, openIndex } = sample()
  const label = windowLabel('this_week')
  const d = dayBars(summary, TODAY, 'this_week', 'monday')
  const h = hourSection(summary)
  const { table: dayTab, more } = dayTable(summary, TODAY)
  return [
    `<section class="aw-view" data-view="stats" aria-label="Stats">`,
    `  <div class="m-main-head" id="t-head"><span class="m-t-title">Stats</span>${seg('Window', WINDOWS.map(([, t]) => t), 0)}</div>`,
    `  <div class="m-main-body aw-stats">`,
    `    <div class="m-group-label">spend</div>`,
    `    ${tiles(spendTiles(summary, label), 'm-stats is-spend', 't-spend')}`,
    `    <div class="m-t-meta aw-note" id="t-spend-note">${esc(spendNote(summary))}</div>`,
    `    <div class="m-group-label">${esc(label)}</div>`,
    `    ${tiles(metricTiles(summary), 'm-stats is-metrics', 't-metrics')}`,
    `    <div class="m-stats-head" id="t-bd"><div class="m-group-label">breakdown</div>${seg('Breakdown', ['Models', 'Agents', 'Days'], 0)}</div>`,
    `    <div class="m-t-meta aw-sub" id="t-track-meta">${esc(ACTIVITY_META)}</div>`,
    `    <div class="aw-tracker" id="t-track" role="grid">${tracker(model, tips, openIndex)}</div>`,
    `    <div class="aw-bd" id="t-models"><div class="aw-w" id="t-shares">${hbars(modelShares(summary.by_model))}</div>${table(modelTable(summary), false)}</div>`,
    `    <div class="aw-bd" id="t-agents">${table(agentTable(summary), false)}<div class="m-group-label">sessions by state</div><div class="aw-states" id="t-states">${stateRows(summary)}</div></div>`,
    `    <div class="aw-bd" id="t-days"><div class="aw-w" id="t-bars"><div class="m-t-meta">${esc(d.heading)}</div>${statsBars(d.bars.values, d.bars.labels, d.bars.todayIndex, d.bars.variant, d.titles)}</div><div class="aw-w" id="t-hours"><div class="m-t-meta">${HOURS_HEADING}</div>${statsBars(h.values, h.labels, h.peak, 'hours', h.titles)}</div>${table(dayTab, false)}${more > 0 ? `<div class="m-t-meta aw-more">${esc(moreDays(more))}</div>` : ''}</div>`,
    `  </div>`,
    `</section>`,
  ].join('\n')
}

/** The page CSS of the Stats view (mirrors ui/src/app/stats/stats.css, `ss-` → `aw-`). */
export const APP_WINDOW_CSS = `/* stats */
.aw-view[data-view="stats"] > .m-main-body{overflow:auto}   /* the page scrolls, as the app's does */
.aw-stats{display:flex;flex-direction:column;align-items:stretch;gap:0;padding:0 16px 24px}
.aw-stats .m-group-label{padding-left:0;margin-top:8px}
.aw-stats .m-stats-head{margin-top:8px}
.aw-stats .m-stat-row{padding:0 2px}
.aw-stats .m-tracker-tip{right:auto;left:-4px}   /* anchored to the square's left edge so it stays inside the frame */
.aw-stats .aw-w{max-width:520px}
.aw-stats .aw-sub{margin:4px 0 8px}
.aw-stats .aw-note{margin-top:8px}
.aw-stats .aw-more{margin-top:6px}
.aw-stats .aw-tracker{display:inline-block;max-width:100%;margin-bottom:12px}
.aw-stats .aw-bd{display:flex;flex-direction:column;gap:12px}
.aw-stats .aw-bd + .aw-bd{margin-top:24px}   /* the three panels stacked here; the app shows one at a time */
.aw-stats .aw-states{max-width:320px}
.aw-stats .aw-link{cursor:default;border-radius:var(--m-r-ctl)}
.aw-stats .aw-agent{display:inline-flex;align-items:center;cursor:default;border-radius:var(--m-r-ctl)}
`

/** The callout strip and the key card of the Stats view. */
export const APP_WINDOW_CALLOUTS = `  <div class="aw-co" data-view="stats"><span class="m-callout" data-for="t-head">1</span><span class="m-callout" data-for="t-spend">2</span><span class="m-callout" data-for="t-metrics">3</span><span class="m-callout" data-for="t-bd">4</span><span class="m-callout" data-for="t-track">5</span><span class="m-callout" data-for="t-shares">6</span><span class="m-callout" data-for="t-agents">7</span><span class="m-callout" data-for="t-bars">8</span></div>`

export const APP_WINDOW_KEY = `<div class="lg-card aw-key" data-view="stats"><h4>Stats</h4><ol class="m-callout-list" style="margin-top:0"><li>Window picker This week / Last week / 4 weeks / 12 months / All time; nothing else in the toolbar. It scopes the spend, the metrics and the breakdown; the tracker and the streak always show the year.</li><li>Spend first: the window as the hero figure, then today, this month and all time, all estimates from token counts at list price (ADR-0051); the note under it says how, and names any model the price list lacks.</li><li>Twelve metric tiles in the window: tokens (output in the label), messages, sessions started, agents spawned, favorite model with its share, peak hour, streak, active days, longest session, files with <code>+N −M</code>, the cache-hit share, tokens per session. Plain numbers, no badges.</li><li>Breakdown: the label with the Models · Agents · Days toggle at its right; the panel below changes with it (all three are drawn here, stacked).</li><li>The day tracker: 53 weeks × 7 days ending today (a year, as GitHub draws it), 11-px squares, 2-px gaps, five steps of <code>--m-ok</code> over <code>--m-sunken</code>; month labels above, M W F left; the tooltip "Fri 16 Oct · 3 sessions · 9 files · 210k tokens" open on one square; the streak line left, the legend right.</li><li>Models: share bars of every token by model (the source named when it is not Claude Code), then the table: messages, the four token kinds and the spend per model and source.</li><li>Agents: the sessions with usage in the window, the mark and name (subagents say whose), model, messages, tokens, duration, spend; a row opens the session. Sessions by state below it, a row opening the roster filtered.</li><li>Days: tokens per day (per week on the long windows, today in fg-2) with the split by model and source on hover, messages per local hour with the peak in fg-2, then the table of days newest first.</li></ol></div>`

/* ---- the loading state (03 §7A Loading; 10 §7.4.21) ------------------------------- */

/** The kit's Skeleton: a share of the container as a modifier, pixels inline. */
function skel(variant: 'text' | 'n' | 'square' | 'bar', width?: number | Share, custom?: string): string {
  const share = typeof width === 'string' ? ` w-${width.slice(0, 2)}` : ''
  const style = [typeof width === 'number' ? `width:${width}px` : '', custom ?? ''].filter(Boolean).join(';')
  return `<span class="m-skel is-${variant}${share}"${style ? ` style="${style}"` : ''} aria-hidden="true"></span>`
}

function skelTile(n: string, l: string, hero = false): string {
  return `<div class="m-stat${hero ? ' is-hero' : ''}"><div class="m-stat-n">${n}</div><div class="m-stat-l">${l}</div></div>`
}

function skelTable(spec: TableSpec): string {
  const head = spec.columns.map((c) => `<th${c.num ? ' class="is-num"' : ''}>${esc(String(c.label))}</th>`).join('')
  const body = Array.from({ length: TABLE_ROWS }, (_, r) => `<tr>${spec.columns.map((c, i) => `<td${c.num ? ' class="is-num"' : ''}>${skel('text', cellWidth(i, Boolean(c.num), r))}</td>`).join('')}</tr>`).join('')
  return `<table class="m-table" aria-label="${esc(spec.label)}"><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`
}

/** The tracker as the skeleton draws it: the real labels, one block per square, the streak line as a block, the legend. */
function skelTracker(model: TrackerModel): string {
  const weeks = Math.max(1, Math.ceil(model.days.length / 7))
  const months = model.months.map((m) => `<span style="grid-column:${m.column}/span ${m.span ?? 2}">${esc(m.label)}</span>`).join('')
  const weekdays = model.weekdays.map((w) => `<span>${esc(w)}</span>`).join('')
  const grid = model.days.map((_, i) => skel('square', undefined, `--c:${Math.floor(i / 7)}`)).join('')
  const legend = `<span class="m-tracker-legend">less&nbsp;<i class="lvl-0"></i><i class="lvl-1"></i><i class="lvl-2"></i><i class="lvl-3"></i><i class="lvl-4"></i>&nbsp;more</span>`
  return `<div class="m-tracker" style="--weeks:${weeks}"><div class="m-tracker-months">${months}</div><div class="m-tracker-days">${weekdays}</div><div class="m-tracker-grid">${grid}</div><div class="m-tracker-foot"><span>${skel('text', STREAK)}</span>${legend}</div></div>`
}

function skelSpend(id?: string): string {
  return `<div class="m-stats is-spend"${attr('id', id)}>${SPEND.map((t, i) => skelTile(skel('n', t.n), skel('text', t.l), i === 0)).join('')}</div>`
}

function skelMetrics(id?: string): string {
  return `<div class="m-stats is-metrics"${attr('id', id)}>${METRICS.map((t) => skelTile(skel('n', t.n), skel('text', t.l))).join('')}</div>`
}

function skelShares(): string {
  return `<div class="m-hbars">${SHARES.map((r) => `<span class="m-hbar-l">${skel('text', r.l)}</span><span class="m-hbar"></span><span class="m-hbar-n">${skel('text', r.n)}</span>`).join('')}</div>`
}

/** The "Stats · loading" figure of app-window.html: the main region before stats_days and stats_summary answer. */
export function appWindowLoadingSection(): string {
  const model = trackerModel([], TODAY, 'monday')
  return [
    `<section class="wf-section" id="stats-loading">`,
    `<h2 class="wf-h2">Stats · loading</h2>`,
    `<p class="wf-note">03 §7A Loading and 10 §3.7: until <code>stats_days</code> and <code>stats_summary</code> answer, every band is drawn as its skeleton at its final height (<code>.m-skel</code>, 10 §7.4.21), sweeping with the Chat shimmer, static under reduced motion. What the view knows already is drawn as itself: the window picker, the group labels, the breakdown head, the activity line, the tracker's month and weekday labels and its legend, the table's headings. The figures replace the blocks with <code>m-in-rise</code>; a window change keeps the old figures at 60 % (<code>.is-stale</code>) instead. Drawn as the main region alone, Models selected.</p>`,
    `<div class="m-stage"><div class="aw-frame is-wide"><div class="m-main">`,
    `<div class="m-main-head" id="l-head"><span class="m-t-title">Stats</span>${seg('Window', WINDOWS.map(([, t]) => t), 0)}</div>`,
    `<div class="m-main-body aw-stats" aria-busy="true">`,
    `  <div class="m-group-label">spend</div>`,
    `  ${skelSpend('l-spend')}`,
    `  <div class="m-t-meta aw-note" id="l-spend-note">${skel('text', NOTE)}</div>`,
    `  <div class="m-group-label">${esc(windowLabel('this_week'))}</div>`,
    `  ${skelMetrics('l-metrics')}`,
    `  <div class="m-stats-head" id="l-bd"><div class="m-group-label">breakdown</div>${seg('Breakdown', ['Models', 'Agents', 'Days'], 0)}</div>`,
    `  <div class="m-t-meta aw-sub" id="l-track-meta">${esc(ACTIVITY_META)}</div>`,
    `  <div class="aw-tracker" id="l-track">${skelTracker(model)}</div>`,
    `  <div class="aw-bd" id="l-models"><div class="aw-w" id="l-shares">${skelShares()}</div>${skelTable(modelTable(null))}</div>`,
    `</div>`,
    `</div></div></div>`,
    `</section>`,
  ].join('\n')
}

/* ---- components.html ------------------------------------------------------------- */

function ex(caption: string, body: string): string {
  return `<div class="wf-ex"><p class="wf-caption"><b>${caption}</b></p>${body}</div>`
}

/** The demos of the `stats` section, for one theme. */
export function componentsStatsDemo(dark: boolean): string {
  const { summary } = sample()
  const spend = spendTiles(summary, 'this week')
  const metrics = metricTiles(summary)
  const d = dayBars(summary, TODAY, 'this_week', 'monday')
  const h = hourSection(summary)
  return [
    ex('.m-stats.is-spend · .m-stat.is-hero', tiles(spend, 'm-stats is-spend')),
    ex('.m-stats.is-metrics · .m-stat', tiles(metrics, 'm-stats is-metrics')),
    ex('.m-stats-head (a group label with a control)', `<div class="m-stats-head"><div class="m-group-label">breakdown</div>${seg('Breakdown', ['Models', 'Agents', 'Days'], 0)}</div>`),
    ex('.m-hbars · .m-hbar-l · .m-hbar · .m-hbar-n', `<div style="max-width:520px">${hbars(modelShares(summary.by_model))}</div>`),
    ex('.m-table · th/td.is-num · .is-dim', table(modelTable(summary), dark)),
    ex('.m-stats-bars · .m-stats-bar.is-today · .m-stats-days', `<div style="max-width:420px"><div class="m-t-meta">${esc(d.heading)}</div>${statsBars(d.bars.values, d.bars.labels, d.bars.todayIndex, 'days', d.titles)}</div>`),
    ex('.m-stats-bars.is-hours · .m-stats-days.is-hours', `<div style="max-width:420px"><div class="m-t-meta">${HOURS_HEADING}</div>${statsBars(h.values, h.labels, h.peak, 'hours', h.titles)}</div>`),
    ex('.m-stat-row (sessions by state)', `<div style="max-width:260px"><div class="m-t-meta" style="margin-bottom:4px">Sessions by state</div>${stateCounts(summary).map(({ state, count }) => `<div class="m-stat-row"><span class="m-state is-${state}">${state}</span><span class="m-count">${count}</span></div>`).join('')}</div>`),
  ].join('\n')
}

/** The demo of the `tracker` section, for one theme. */
export function componentsTrackerDemo(): string {
  const { model, tips, openIndex } = sample()
  return ex('.m-tracker · .m-tracker-months / -days / -grid · .lvl-0 … .lvl-4 · .m-tracker-tip (open) · .m-tracker-foot · .m-tracker-legend', `<div class="m-t-meta" style="margin-bottom:8px">${esc(ACTIVITY_META)}</div>${tracker(model, tips, openIndex)}`)
}

/** The demos of the `skeleton` section (10 §7.4.21), for one theme. */
export function componentsSkeletonDemo(): string {
  const { summary } = sample()
  const grid = Array.from({ length: 7 * 12 }, (_, i) => skel('square', undefined, `--c:${Math.floor(i / 7)}`)).join('')
  const bars = DAY_BARS.map((h) => skel('bar', undefined, `--h:${h}%`)).join('')
  return [
    ex('.m-skel.is-text · .w-80 / .w-60 / .w-40', `<div style="max-width:320px">${skel('text', '80%')}${skel('text', '60%')}${skel('text', '40%')}</div>`),
    ex('.m-stats.is-spend · .m-stat-n &gt; .m-skel.is-n (32 px in the hero) · .m-stat-l &gt; .m-skel.is-text', skelSpend()),
    ex('.m-tracker-grid · .m-skel.is-square (the sweep starts 20 ms later per week column)', `<div class="m-tracker"><div class="m-tracker-grid">${grid}</div></div>`),
    ex('.m-stats-bars · .m-skel.is-bar', `<div style="max-width:420px"><div class="m-stats-bars">${bars}</div></div>`),
    ex('.m-stats.is-metrics.is-stale (a refetch in flight)', tiles(metricTiles(summary).slice(0, 4), 'm-stats is-metrics is-stale')),
  ].join('\n')
}

/** Replace a page's stats pieces. Returns the new text or throws when a marker is missing. */
export function spliceAppWindow(html: string): string {
  const sectionStart = html.indexOf('<section class="aw-view" data-view="stats"')
  const sectionEnd = html.indexOf('</section>', sectionStart) + '</section>'.length
  const cssStart = html.indexOf('/* stats */\n')
  const cssEnd = html.indexOf('/* settings:', cssStart)
  const coStart = html.indexOf('  <div class="aw-co" data-view="stats">')
  const coEnd = html.indexOf('</div>\n', coStart) + '</div>'.length
  const keyStart = html.indexOf('<div class="lg-card aw-key" data-view="stats">')
  const keyEnd = html.indexOf('</ol></div>', keyStart) + '</ol></div>'.length
  if ([sectionStart, cssStart, cssEnd, coStart, keyStart].some((i) => i < 0)) throw new Error('app-window.html: a stats marker is missing')
  const parts: [number, number, string][] = [
    [sectionStart, sectionEnd, appWindowSection()],
    [cssStart, cssEnd, APP_WINDOW_CSS],
    [coStart, coEnd, APP_WINDOW_CALLOUTS],
    [keyStart, keyEnd, APP_WINDOW_KEY],
  ]
  parts.sort((a, b) => b[0] - a[0])
  let out = html
  for (const [s, e, text] of parts) out = out.slice(0, s) + text + out.slice(e)
  return spliceLoading(out)
}

/** The "Stats · loading" figure after the Appearance figure, and its entry in the page's contents. */
function spliceLoading(html: string): string {
  let out = html
  const start = out.indexOf('<section class="wf-section" id="stats-loading">')
  if (start >= 0) {
    const end = out.indexOf('</section>', start) + '</section>'.length
    out = out.slice(0, start) + appWindowLoadingSection() + out.slice(end)
  } else {
    const at = out.indexOf('<section class="wf-section" id="palette">')
    if (at < 0) throw new Error('app-window.html: no #palette section to put the loading figure before')
    out = out.slice(0, at) + appWindowLoadingSection() + '\n\n' + out.slice(at)
  }
  const toc = '<li><a href="#stats-loading">Stats · loading</a></li>'
  if (!out.includes(toc)) {
    const li = '<li><a href="#appearance">Settings · Appearance</a></li>'
    if (!out.includes(li)) throw new Error('app-window.html: no Appearance entry in the contents')
    out = out.replace(li, li + toc)
  }
  return out
}

function section(html: string, id: string, title: string, note: string, light: string, dark: string): string {
  const start = html.indexOf(`<section class="wf-section" id="${id}">`)
  const end = html.indexOf('</section>', start) + '</section>'.length
  if (start < 0) throw new Error(`components.html: no section #${id}`)
  const body = `<section class="wf-section" id="${id}">
<h2 class="wf-h2">${title}</h2>
<p class="wf-note">${note}</p>
<div class="wf-pair">
<div class="wf-demo"><p class="wf-theme">light · default</p>
${light}
</div>
<div class="wf-demo" data-theme="dark"><p class="wf-theme">dark · [data-theme=dark]</p>
${dark}
</div>
</div>
</section>`
  return html.slice(0, start) + body + html.slice(end)
}

export function spliceComponents(html: string): string {
  let out = section(
    html,
    'stats',
    'Stats',
    'The Stats view’s pieces (03 §7A): the spend row with its hero figure (estimates at list price, ADR-0051: the one place currency appears), the metric tiles, a group label with a control at its right, horizontal share bars, a plain table of figures, tokens per day as neutral bars (today in fg-2), messages per hour, and sessions by state as a vertical list with counts.',
    componentsStatsDemo(false),
    componentsStatsDemo(true),
  )
  out = section(
    out,
    'tracker',
    'Day tracker',
    '§7.3: GitHub-style grid of Mode activity only, 53 weeks × 7 days (a year ending today), 11-px squares with 2-px gaps, radius 2. Five steps: <code>--m-sunken</code>, then <code>--m-ok</code> mixed at 25 / 50 / 75 / 100 %. Month labels above, weekday letters left, the legend bottom-right, one streak line. The tooltip opens on hover or focus; one is drawn open here. It opens the breakdown of Stats.',
    componentsTrackerDemo(),
    componentsTrackerDemo(),
  )
  if (!out.includes('<section class="wf-section" id="skeleton">')) {
    const notice = out.indexOf('<section class="wf-section" id="notice">')
    if (notice < 0) throw new Error('components.html: no #notice section to put the skeleton section after')
    const at = out.indexOf('</section>', notice) + '</section>'.length
    out = out.slice(0, at) + '\n<section class="wf-section" id="skeleton"></section>' + out.slice(at)
    out = out.replace('<li><a href="#notice">Notices</a></li>', '<li><a href="#notice">Notices</a></li><li><a href="#skeleton">Skeleton</a></li>')
  }
  out = section(out, 'skeleton', 'Skeleton', SKELETON_NOTE, componentsSkeletonDemo(), componentsSkeletonDemo())
  return out
}

/** The note of the `skeleton` section (the gallery prints the same words). */
export const SKELETON_NOTE =
  'A view’s loading state (10 §3.7, §7.4.21): blocks in the layout of the content at its final size, sweeping with the Chat shimmer (static under reduced motion), until the data replaces them with m-in-rise. A text line is 12 px, a figure 20 px (32 in a hero tile), a tracker cell 11 px, a bar takes its height from --h; widths are inline pixels or .w-40 / .w-60 / .w-80. A refetch keeps the shown figures at 60 % (.is-stale) instead.'
