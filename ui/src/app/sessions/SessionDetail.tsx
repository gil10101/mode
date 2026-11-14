// Session detail (03 §3.6): the header, the six tabs and the sub-detail
// pages (Files diff pane, agent thread). Loads `session_detail` for the
// route's session and refreshes it on every roster push for that session.
import { useEffect, type KeyboardEvent } from 'react'
import { Tab, Tabs } from '../../kit/controls'
import { useSessionsStore } from '../../state/sessions-store'
import type { SessionDetail } from '../../state/contract-sessions'
import { navigate, SESSION_TABS, type Route, type SessionTab } from '../router'
import { openMenu, useNow, type MenuEntry } from '../ui'
import { decide, sessionMenu } from './actions'
import { headerPrimary, linkRoute } from './RosterPanel'
import { rowFromHeader, SessionHeaderView } from './header'
import { AgentSubPage, AgentsTab, ConnectorsTab, FilesTab, OverviewTab, TerminalTab, TranscriptTab, type TabProps } from './tabs'

const TAB_LABEL: Record<SessionTab, string> = {
  overview: 'Overview',
  terminal: 'Terminal',
  files: 'Files',
  agents: 'Agents',
  connectors: 'Connectors',
  transcript: 'Transcript',
}

export function SessionDetailView({ route }: { route: Route }) {
  const id = route.id!
  const now = useNow(1000)
  const detail: SessionDetail | undefined = useSessionsStore((s) => s.details[id])
  const status = useSessionsStore((s) => s.detailStatus[id])
  const row = useSessionsStore((s) => s.rows.find((r) => r.id === id))
  const load = useSessionsStore((s) => s.loadDetail)
  const acknowledge = useSessionsStore((s) => s.acknowledge)
  const tab: SessionTab = route.tab ?? 'overview'

  useEffect(() => {
    void load(id)
  }, [id, load])

  // Opening the Overview acknowledges an `unread` session (03 §2.5, §3.4).
  const state = row?.state ?? detail?.session.state
  useEffect(() => {
    if (tab === 'overview' && state === 'unread') void acknowledge(id)
  }, [tab, state, id, acknowledge])

  // The OS-level window title: "Mode — <display_name>" (03 §2.1).
  const name = detail?.session.display_name ?? row?.display_name
  useEffect(() => {
    if (!name) return
    const before = document.title
    document.title = `Mode — ${name}`
    return () => {
      document.title = before
    }
  }, [name])

  if (!detail) {
    return (
      <section className="ap-view sd-main" aria-label="Session" data-testid="session-detail">
        <div className="ap-full">
          {status === 'error' ? (
            <>
              This session is no longer available.
              <button type="button" className="m-btn" onClick={() => navigate({ view: 'sessions' })}>
                Back to sessions
              </button>
            </>
          ) : null}
        </div>
      </section>
    )
  }

  // The roster row is the live source for state and pending; the detail carries the rest.
  const header = row ? { ...detail.session, state: row.state, waiting_kind: row.waiting_kind } : detail.session
  const pending = row ? (row.pending ?? null) : (detail.pending ?? null)
  const view = rowFromHeader(header, { pending })
  const history = !row
  const primary = headerPrimary(view, { history: history && !!(detail.session.dismissed_at || detail.session.ended_at) })
  const ctx = { sessionId: id, row: view, pending, now }
  const props: TabProps = { detail: { ...detail, session: header, pending }, route, now, ctx }
  const sub = tab === 'agents' && route.sub

  const menu = (anchor: Element): MenuEntry[] => {
    const entries = sessionMenu(view, { open: false, history: false, primary: primary.id, header: { claude_session_id: header.claude_session_id, transcript_path: header.transcript_path } })
    if (detail.links.length)
      entries.splice(entries.findIndex((e) => e.id === 'note') + 1, 0, {
        id: 'links',
        label: 'Links',
        run: () =>
          openMenu(
            anchor,
            detail.links.map((l) => ({ id: `${l.kind}:${l.id}`, label: l.available ? l.title : 'unavailable', dimmed: !l.available, run: () => navigate(linkRoute(l)) })),
            'Links',
          ),
      })
    return entries
  }

  const onKey = (e: KeyboardEvent<HTMLElement>) => {
    // Ctrl+Tab / Ctrl+Shift+Tab cycle the tabs; ⌘/Ctrl+Shift+1…6 jump (03 §10.2; printed only in the Shortcuts view).
    if (e.ctrlKey && e.key === 'Tab') {
      e.preventDefault()
      const i = SESSION_TABS.indexOf(tab)
      const next = SESSION_TABS[(i + (e.shiftKey ? SESSION_TABS.length - 1 : 1)) % SESSION_TABS.length]
      navigate({ view: 'sessions', id, tab: next })
      return
    }
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && /^Digit[1-6]$/.test(e.code)) {
      e.preventDefault()
      navigate({ view: 'sessions', id, tab: SESSION_TABS[Number(e.code.slice(5)) - 1] })
      return
    }
    // A / D / Shift+A on a focused approval card.
    const target = e.target as HTMLElement
    if (pending?.kind === 'permission' && pending.answerable && target.closest('.m-thread-permission') && !e.metaKey && !e.ctrlKey && !e.altKey) {
      if (e.key === 'A') return void (e.preventDefault(), decide(id, 'always', pending))
      if (e.key === 'a') return void (e.preventDefault(), decide(id, 'allow', pending))
      if (e.key === 'd') return void (e.preventDefault(), decide(id, 'deny', pending))
    }
  }

  return (
    <section className="ap-view sd-main" aria-label={`Session ${header.display_name}`} data-testid="session-detail" onKeyDown={onKey}>
      <div className="sd-head">
        <SessionHeaderView header={header} now={now} primaryLabel={primary.label} onPrimary={primary.run} onMore={(e) => openMenu(e.currentTarget, menu(e.currentTarget), `Actions for ${header.display_name}`)} />
      </div>
      {sub ? (
        <AgentSubPage {...props} />
      ) : (
        <>
          <Tabs label="Session detail tabs" className="sd-tabs">
            {SESSION_TABS.map((t) => (
              <Tab
                key={t}
                active={t === tab}
                count={t === 'files' ? detail.files.length || undefined : t === 'agents' ? detail.agents.length || undefined : undefined}
                onClick={() => navigate({ view: 'sessions', id, tab: t })}
              >
                {TAB_LABEL[t]}
              </Tab>
            ))}
          </Tabs>
          {tab === 'overview' && <OverviewTab {...props} />}
          {tab === 'terminal' && <TerminalTab {...props} />}
          {tab === 'files' && <FilesTab {...props} />}
          {tab === 'agents' && <AgentsTab {...props} />}
          {tab === 'connectors' && <ConnectorsTab {...props} />}
          {tab === 'transcript' && <TranscriptTab {...props} />}
        </>
      )}
    </section>
  )
}
