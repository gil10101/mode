// The Sessions view (03 §3): the roster screen (toolbar, banner, list or
// cards or board, detail panel, empty / loading states) or, with a session id in the
// route, the session detail screen.
import { useEffect, useRef, useState } from 'react'
import { Mascot } from '../../kit/avatars'
import { invoke } from '../../tauri'
import { Button, MenuButton } from '../../kit/button'
import { Chip, Seg, SegItem } from '../../kit/controls'
import { MainBody, MainHead, Split } from '../../kit/layout'
import { ALL_KINDS, ALL_STATES, hasFilters, matchesFilters, matchesSearch, repoOptions, useSessionsStore, type RosterFilters } from '../../state/sessions-store'
import type { AppRosterRow } from '../../state/contract-sessions'
import { navigate, useRouter, type Route } from '../router'
import { useShell } from '../shell-store'
import { openMenu, useNow, type MenuEntry } from '../ui'
import { useDialogs } from './actions'
import { basename, stateWord } from './format'
import { Roster } from './Roster'
import { RosterPanel } from './RosterPanel'
import { SessionDetailView } from './SessionDetail'

export function SessionsView() {
  const route = useRouter((s) => s.route)
  useRouteFilter(route)
  if (route.id) return <SessionDetailView key={route.id} route={route} />
  return <RosterScreen />
}

/** `app.open({view: 'sessions', filter})` and `sessions?filter=` hand a filter to the roster once. */
function useRouteFilter(route: Route) {
  const setFilters = useSessionsStore((s) => s.setFilters)
  const setShowHistory = useSessionsStore((s) => s.setShowHistory)
  useEffect(() => {
    const f = route.filter
    if (!f) return
    if (f === 'history') void setShowHistory(true)
    else if ((ALL_STATES as readonly string[]).includes(f)) setFilters({ states: [f] })
    else if (f === 'finished') setFilters({ states: ['unread', 'done'] })
    else setFilters({ repos: [f] })
    navigate({ ...route, filter: undefined }, { replace: true })
  }, [route, setFilters, setShowHistory])
}

const STATE_LABEL: Record<string, string> = { unread: 'finished, unread', done: 'finished' }
const KIND_LABEL: Record<string, string> = { hooked: 'external', launched: 'launched from Mode', discovered: 'untracked' }

function filterChips(f: RosterFilters, set: (p: Partial<RosterFilters>) => void) {
  const chips: { key: string; label: string; remove: () => void }[] = []
  for (const s of f.states) chips.push({ key: `s:${s}`, label: `state: ${STATE_LABEL[s] ?? stateWord(s)}`, remove: () => set({ states: f.states.filter((x) => x !== s) }) })
  for (const r of f.repos) chips.push({ key: `r:${r}`, label: `repo: ${basename(r)}`, remove: () => set({ repos: f.repos.filter((x) => x !== r) }) })
  for (const k of f.kinds) chips.push({ key: `k:${k}`, label: `kind: ${KIND_LABEL[k] ?? k}`, remove: () => set({ kinds: f.kinds.filter((x) => x !== k) }) })
  if (f.date) chips.push({ key: 'd', label: f.date === 'today' ? 'started today' : 'started this week', remove: () => set({ date: null }) })
  return chips
}

function toggle(list: string[], v: string): string[] {
  return list.includes(v) ? list.filter((x) => x !== v) : [...list, v]
}

/** The toolbar's ⋮: filters (03 §3.3), Group by repo, History. */
function toolbarMenu(rows: AppRosterRow[]): MenuEntry[] {
  const s = useSessionsStore.getState()
  const f = s.filters
  const entries: MenuEntry[] = [
    { id: 'group', label: 'Group by repo', checked: s.groupByRepo, run: () => s.setGroupByRepo(!s.groupByRepo) },
    { id: 'history', label: 'History', checked: s.showHistory, run: () => void s.setShowHistory(!s.showHistory) },
    { kind: 'sep', id: 'sep1' },
    { kind: 'label', id: 'l-state', label: 'State' },
  ]
  for (const st of ALL_STATES)
    entries.push({ id: `state-${st}`, label: STATE_LABEL[st] ?? stateWord(st), checked: f.states.includes(st), run: () => s.setFilters({ states: toggle(f.states, st) }) })
  const repos = repoOptions(rows)
  if (repos.length) {
    entries.push({ kind: 'label', id: 'l-repo', label: 'Repo' })
    for (const r of repos) entries.push({ id: `repo-${r}`, label: basename(r), checked: f.repos.includes(r), run: () => s.setFilters({ repos: toggle(f.repos, r) }) })
  }
  entries.push({ kind: 'label', id: 'l-kind', label: 'Kind' })
  for (const k of ALL_KINDS) entries.push({ id: `kind-${k}`, label: KIND_LABEL[k], checked: f.kinds.includes(k), run: () => s.setFilters({ kinds: toggle(f.kinds, k) }) })
  entries.push({ kind: 'label', id: 'l-date', label: 'Started' })
  entries.push({ id: 'date-today', label: 'Today', checked: f.date === 'today', run: () => s.setFilters({ date: f.date === 'today' ? null : 'today' }) })
  entries.push({ id: 'date-week', label: 'This week', checked: f.date === 'week', run: () => s.setFilters({ date: f.date === 'week' ? null : 'week' }) })
  if (hasFilters(f) || s.search) {
    entries.push({ kind: 'sep', id: 'sep2' })
    entries.push({ id: 'clear', label: 'Clear filters', run: () => s.clearFilters() })
  }
  return entries
}

function RosterScreen() {
  const now = useNow(1000)
  const rows = useSessionsStore((s) => s.rows)
  const loaded = useSessionsStore((s) => s.loaded)
  const history = useSessionsStore((s) => s.history)
  const showHistory = useSessionsStore((s) => s.showHistory)
  const filters = useSessionsStore((s) => s.filters)
  const search = useSessionsStore((s) => s.search)
  const setSearch = useSessionsStore((s) => s.setSearch)
  const setFilters = useSessionsStore((s) => s.setFilters)
  const layout = useSessionsStore((s) => s.layout)
  const setLayout = useSessionsStore((s) => s.setLayout)
  const selectedId = useSessionsStore((s) => s.selectedId)
  const health = useSessionsStore((s) => s.health)
  const detailOpen = useShell((s) => s.detailOpen)
  const openLaunch = useDialogs((s) => s.openLaunch)
  const [query, setQuery] = useState(search)
  const searchRef = useRef<HTMLInputElement>(null)

  // Typing filters after 150 ms (03 §2.4).
  useEffect(() => {
    const t = setTimeout(() => setSearch(query), 150)
    return () => clearTimeout(t)
  }, [query, setSearch])
  useEffect(() => setQuery(search), [search])

  useEffect(() => {
    const focus = () => searchRef.current?.focus()
    window.addEventListener('mode:focus-search', focus)
    return () => window.removeEventListener('mode:focus-search', focus)
  }, [])

  const visible = rows.filter((r) => matchesFilters(r, filters, now) && matchesSearch(r, search))
  // `sessions_history` answers dismissed rows only (never a roster row; the store drops one that comes back).
  const visibleHistory = showHistory ? history.filter((r) => matchesFilters(r, filters, now) && matchesSearch(r, search)) : []
  const selected = selectedId ? (rows.find((r) => r.id === selectedId) ?? history.find((r) => r.id === selectedId)) : undefined
  const selectedIsHistory = !!selected && !rows.some((r) => r.id === selected.id)
  const chips = filterChips(filters, setFilters)
  const claudeMissing = health?.claude_code === 'missing'
  const hooksInstalled = health ? health.hooks === 'installed' : null
  const onlyUntracked = rows.length > 0 && rows.every((r) => r.state === 'untracked')

  return (
    <section className="ap-view" aria-label="Sessions" data-testid="sessions-view">
      <MainHead title="Sessions" titleTestId="view-title">
        <div className="ap-chips" aria-label="Filters">
          {chips.map((c) => (
            <Chip key={c.key} removable onRemove={c.remove}>
              {c.label}
            </Chip>
          ))}
          {showHistory && (
            <Chip removable onRemove={() => void useSessionsStore.getState().setShowHistory(false)}>
              history
            </Chip>
          )}
        </div>
        <input
          ref={searchRef}
          className="m-input aw-search"
          type="search"
          placeholder="Search sessions"
          aria-label="Search sessions"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              setQuery('')
              ;(e.target as HTMLInputElement).blur()
            }
          }}
        />
        <Seg label="Roster layout">
          <SegItem active={layout === 'list'} onClick={() => setLayout('list')}>
            List
          </SegItem>
          <SegItem active={layout === 'cards'} onClick={() => setLayout('cards')}>
            Cards
          </SegItem>
          <SegItem active={layout === 'board'} onClick={() => setLayout('board')}>
            Board
          </SegItem>
        </Seg>
        <Button onClick={() => openLaunch()} disabled={claudeMissing} title={claudeMissing ? 'Claude Code was not found' : undefined}>
          Launch session
        </Button>
        <MenuButton label="Sessions options" onClick={(e) => openMenu(e.currentTarget, toolbarMenu(rows), 'Sessions options')} />
      </MainHead>
      <MainBody testId="view-body">
        {claudeMissing && (
          <div className="ap-banner" role="status">
            <span>Claude Code was not found. Install it to track sessions.</span>
            <Button onClick={openClaudeDocs}>Open docs</Button>
          </div>
        )}
        {!claudeMissing && hooksInstalled === false && onlyUntracked && (
          <div className="ap-banner" role="status" data-testid="hooks-banner">
            <span>Install hooks to see state, approvals and files</span>
            <Button onClick={() => navigate('integrations/claude-code/install')}>Install hooks</Button>
          </div>
        )}
        {!loaded ? (
          <Skeleton />
        ) : rows.length === 0 && visibleHistory.length === 0 && !hasFilters(filters) && !search ? (
          <EmptySessions hooksInstalled={hooksInstalled} claudeMissing={claudeMissing} showHistory={showHistory} />
        ) : visible.length === 0 && visibleHistory.length === 0 ? (
          <div className="ap-empty" data-testid="no-match">
            {showHistory && !rows.length ? 'No finished sessions in the last 90 days.' : search ? `No sessions match “${search}”.` : 'No sessions match these filters.'}
            <Button onClick={() => useSessionsStore.getState().clearFilters()}>Clear filters</Button>
          </div>
        ) : (
          <Split open={!!selected && detailOpen}>
            <Roster rows={visible} history={visibleHistory} now={now} />
            {selected && detailOpen ? <RosterPanel row={selected} history={selectedIsHistory} now={now} /> : null}
          </Split>
        )}
      </MainBody>
    </section>
  )
}

const CLAUDE_CODE_DOCS = 'https://docs.anthropic.com/en/docs/claude-code/overview'

function openClaudeDocs() {
  void invoke('plugin:opener|open_url', { url: CLAUDE_CODE_DOCS }).catch(() => {})
}

/** Skeleton rows after 150 ms, never a blank window (03 §2.7, §3.13). */
function Skeleton() {
  const [show, setShow] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setShow(true), 150)
    return () => clearTimeout(t)
  }, [])
  if (!show) return null
  return (
    <div className="m-list ap-skeleton" aria-label="Loading sessions" data-testid="skeleton">
      <div className="m-shimmer">
        <i />
        <i />
        <i />
      </div>
    </div>
  )
}

function EmptySessions({ hooksInstalled, claudeMissing, showHistory }: { hooksInstalled: boolean | null; claudeMissing: boolean; showHistory: boolean }) {
  const openLaunch = useDialogs((s) => s.openLaunch)
  if (showHistory) return <div className="ap-empty">No finished sessions in the last 90 days.</div>
  if (claudeMissing)
    return (
      <div className="ap-empty" data-testid="empty-sessions">
        <Mascot size={48} />
        Claude Code was not found. Install it to track sessions.
        <Button onClick={openClaudeDocs}>Open docs</Button>
      </div>
    )
  return (
    <div className="ap-empty" data-testid="empty-sessions">
      <Mascot size={48} />
      No sessions yet.
      <div className="m-btn-group">
        {hooksInstalled === false && <Button onClick={() => navigate('integrations/claude-code/install')}>Install hooks</Button>}
        <Button variant="primary" onClick={() => openLaunch()}>
          Launch session
        </Button>
      </div>
    </div>
  )
}
