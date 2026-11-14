// Routing and deep links of the app window (03 §2.5). Routes are the spec's
// path strings (`sessions/<id>/files/<file_change_id>`); the window keeps the
// current one in `location.hash` so back / forward are the webview's history
// (per window, as §2.5 asks). `mode://` URLs from OS notifications and the
// overlay's `app.open` land here through `openFromCore`.
import { create } from 'zustand'
import type { View } from '../state/store'
import type { AppOpen } from '../state/contract-sessions'

export const VIEWS: readonly View[] = ['sessions', 'inbox', 'notes', 'calendar', 'chat', 'agents', 'automations', 'integrations', 'shortcuts', 'stats', 'settings']

export type SessionTab = 'overview' | 'terminal' | 'files' | 'agents' | 'connectors' | 'transcript'

export const SESSION_TABS: readonly SessionTab[] = ['overview', 'terminal', 'files', 'agents', 'connectors', 'transcript']

export interface Route {
  view: View
  /** Sessions: the session id of the detail screen. */
  id?: string
  /** Sessions: the detail tab (Overview when `id` is set and nothing else is). */
  tab?: SessionTab
  /** Sessions: a sub-detail (`file_change_id` on Files, `agent_id` on Agents). */
  sub?: string
  /** Sessions: `approval/<tool_use_id>` — Overview scrolled to that card. */
  approval?: string
  /** Sessions: a roster filter handed over by `app.open` (`waiting`, `history`, a repo path). */
  filter?: string
  /** Any other view: the rest of the path, kept verbatim for its package (`thread/<id>`, `general`). */
  rest?: string
  /** `shortcuts?q=<text>`. */
  query?: string
}

export const DEFAULT_ROUTE: Route = { view: 'sessions' }

/** Retired routes of ADR-0033 (03 §2.5): old overlay and notification links keep working. */
const RETIRED: Record<string, string> = {
  'settings/hotkeys': 'shortcuts',
  'settings/connectors': 'integrations',
  'settings/accounts': 'integrations',
  'settings/sessions': 'settings/general',
}

function clean(input: string): { path: string; params: URLSearchParams } {
  let s = input.trim()
  s = s.replace(/^mode:\/\//i, '')
  s = s.replace(/^#/, '')
  s = s.replace(/^\/+/, '')
  const q = s.indexOf('?')
  const params = new URLSearchParams(q >= 0 ? s.slice(q + 1) : '')
  let path = (q >= 0 ? s.slice(0, q) : s).replace(/\/+$/, '')
  if (RETIRED[path]) path = RETIRED[path]
  return { path, params }
}

/** Parses a route string (`sessions/abc/files`, `#/sessions`, `mode://inbox/thread/7`); unknown views fall back to Sessions. */
export function parseRoute(input: string): Route {
  const { path, params } = clean(input)
  const parts = path.split('/').filter(Boolean).map(safeDecode)
  const view = parts[0] as View | undefined
  if (!view || !(VIEWS as readonly string[]).includes(view)) return { ...DEFAULT_ROUTE }
  if (view !== 'sessions') {
    const route: Route = { view }
    if (parts.length > 1) route.rest = parts.slice(1).join('/')
    const q = params.get('q')
    if (q !== null) route.query = q
    return route
  }
  const route: Route = { view: 'sessions' }
  const filter = params.get('filter')
  if (filter) route.filter = filter
  const [, id, second, third] = parts
  if (!id) return route
  route.id = id
  if (!second) {
    route.tab = 'overview'
    return route
  }
  if (second === 'approval') {
    route.tab = 'overview'
    if (third) route.approval = third
    return route
  }
  if ((SESSION_TABS as readonly string[]).includes(second)) {
    route.tab = second as SessionTab
    if (third && (second === 'files' || second === 'agents')) route.sub = third
    return route
  }
  route.tab = 'overview'
  return route
}

function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s)
  } catch {
    return s
  }
}

/** The canonical path of a route (no leading `#` or `/`). */
export function formatRoute(r: Route): string {
  if (r.view !== 'sessions') {
    let s: string = r.view
    if (r.rest) s += `/${r.rest}`
    if (r.query !== undefined) s += `?q=${encodeURIComponent(r.query)}`
    return s
  }
  let s = 'sessions'
  if (r.id) {
    s += `/${encodeURIComponent(r.id)}`
    if (r.approval) s += `/approval/${encodeURIComponent(r.approval)}`
    else if (r.tab && r.tab !== 'overview') {
      s += `/${r.tab}`
      if (r.sub && (r.tab === 'files' || r.tab === 'agents')) s += `/${encodeURIComponent(r.sub)}`
    }
  }
  if (r.filter) s += `?filter=${encodeURIComponent(r.filter)}`
  return s
}

/** `app.open` → a route (02 §16): a full `route` wins; otherwise view + id (+ filter). */
export function routeFromAppOpen(p: AppOpen): Route {
  if (p.route) return parseRoute(p.route)
  const view = (VIEWS as readonly string[]).includes(p.view) ? (p.view as View) : 'sessions'
  if (view === 'sessions') {
    const r: Route = { view }
    if (p.id) {
      r.id = p.id
      r.tab = 'overview'
    }
    if (p.filter) r.filter = p.filter
    return r
  }
  const r: Route = { view }
  if (p.id) r.rest = p.id
  if (p.filter && view === 'shortcuts') r.query = p.filter
  return r
}

export function sameRoute(a: Route, b: Route): boolean {
  return formatRoute(a) === formatRoute(b)
}

/* ---- the router store ------------------------------------------------------ */

interface RouterState {
  route: Route
  navigate(to: Route | string, opts?: { replace?: boolean }): void
  /** Re-reads `location.hash` (hashchange). */
  sync(): void
  back(): void
  forward(): void
}

function readHash(): Route {
  if (typeof window === 'undefined') return { ...DEFAULT_ROUTE }
  return window.location.hash ? parseRoute(window.location.hash) : { ...DEFAULT_ROUTE }
}

export const useRouter = create<RouterState>((set, get) => ({
  route: readHash(),
  navigate(to, opts) {
    const route = typeof to === 'string' ? parseRoute(to) : to
    const path = `#/${formatRoute(route)}`
    if (typeof window !== 'undefined' && window.location.hash !== path) {
      if (opts?.replace) window.history.replaceState(window.history.state, '', path)
      else window.history.pushState(window.history.state, '', path)
    }
    if (!sameRoute(get().route, route) || route.filter) set({ route })
  },
  sync() {
    const route = readHash()
    if (!sameRoute(get().route, route)) set({ route })
  },
  back() {
    if (typeof window !== 'undefined') window.history.back()
  },
  forward() {
    if (typeof window !== 'undefined') window.history.forward()
  },
}))

export function navigate(to: Route | string, opts?: { replace?: boolean }) {
  useRouter.getState().navigate(to, opts)
}

/** Up exactly one drill-in level (03 §2.4: `Esc` / `‹`). */
export function upOneLevel(r: Route): Route | null {
  if (r.view !== 'sessions' || !r.id) return null
  if (r.sub) return { view: 'sessions', id: r.id, tab: r.tab }
  if (r.approval) return { view: 'sessions', id: r.id, tab: 'overview' }
  return { view: 'sessions' }
}
