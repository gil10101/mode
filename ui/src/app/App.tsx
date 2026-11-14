// The app window (03 §2): the kit's .m-window with the sidebar (eight main
// entries, the bottom group, the profile row; 56 px collapsed), hash routing
// with the deep links of §2.5, the shell-level states of §2.7, the Sessions
// view (03 §3) and placeholders for the views later packages fill.
import { useEffect, useState, type ReactNode } from 'react'
import { invoke, listen } from '../tauri'
import { Mascot } from '../kit/avatars'
import { IconDefs, type IconId } from '../kit/icons'
import { BrandDefs } from '../kit/brands'
import { Main, MainBody, MainHead, Sidebar, Window, type SidebarItemSpec } from '../kit/layout'
import { ChatSidebarList } from './chat/SidebarList'
import { ThemeProvider } from '../kit/theme'
import { EVENTS, type InboxChanged, type NotesChanged } from '../state/contract'
import { APP_OPEN_EVENT, type AppOpen } from '../state/contract-sessions'
import { connectSessions, useSessionsStore, waitingCount } from '../state/sessions-store'
import { useCoreStore, type CoreChanged, type CoreStatus, type View } from '../state/store'
import { navigate, routeFromAppOpen, upOneLevel, useRouter } from './router'
import { useDialogs } from './sessions/actions'
import { SessionDialogs } from './sessions/Dialogs'
import { SessionsView } from './sessions/SessionsView'
import { CardLayer, FloatingQuery, NotesView } from '../notes'
import type { ItemType } from '../state/contract-notes'
import { sidebarCollapsed, useShell } from './shell-store'
import { MenuHost, NoticeHost } from './ui'
// package 9: Settings, Integrations, Shortcuts, Stats, onboarding, the profile row
import { connectSettings } from '../state/settings-store'
import { SettingsView } from './settings/SettingsView'
import { AppProfile } from './settings/ProfileRow'
import { useAppearance } from './settings/theme'
import { IntegrationsView } from './integrations/IntegrationsView'
import { ShortcutsView } from './shortcuts/ShortcutsView'
import { StatsView } from './stats/StatsView'
import { Onboarding } from './onboarding/Onboarding'
import { useOnboarding, useOnboardingVisible } from './onboarding/state'
// package 10: Inbox, Calendar, the account flows
import { connectInbox } from '../state/inbox-store'
import { connectCalendar, useCalendar } from '../state/calendar-store'
import { connectAccounts } from '../state/accounts-store'
import { InboxView } from './inbox/InboxView'
import { CalendarView } from './calendar/CalendarView'
import { AccountDialogs } from './integrations/AccountDialogs'
// reconciliation: the quit confirmation (`app:quit-confirm`), the Inbox count before the first push
import { QuitDialog } from './settings/QuitDialog'
import { useInbox } from '../state/inbox-store'
// package 11: Chat, Automations, the command palette, theme packs
import { ChatView } from './chat/ChatView'
import { AutomationsView } from './automations/AutomationsView'
import { AgentsView } from './agents/AgentsView'
import { connectAgents } from '../state/agents-store'
import { CommandPalette } from './palette/CommandPalette'
import { connectChat } from '../state/chat-store'
import { connectAutomations } from '../state/automations-store'
import { connectTheme } from '../state/theme-store'
import './sessions.css'

interface NavItem extends SidebarItemSpec {
  id: View
  icon: IconId
}

export const NAV_ITEMS: NavItem[] = [
  { id: 'sessions', label: 'Sessions', icon: 'sessions' },
  { id: 'inbox', label: 'Inbox', icon: 'inbox' },
  { id: 'notes', label: 'Notes', icon: 'notes' },
  { id: 'calendar', label: 'Calendar', icon: 'calendar' },
  { id: 'chat', label: 'Chat', icon: 'chat' },
  { id: 'agents', label: 'Agents', icon: 'agents' },
  { id: 'automations', label: 'Automations', icon: 'automations' },
  { id: 'integrations', label: 'Integrations', icon: 'integrations' },
  { id: 'shortcuts', label: 'Shortcuts', icon: 'shortcuts' },
]

export const BOTTOM_ITEMS: NavItem[] = [
  { id: 'stats', label: 'Stats', icon: 'stats' },
  { id: 'settings', label: 'Settings', icon: 'settings' },
]

const LABEL: Record<View, string> = Object.fromEntries([...NAV_ITEMS, ...BOTTOM_ITEMS].map((i) => [i.id, i.label])) as Record<View, string>

/** Views not shipped in the current release (03 §2.3.2 assumption, §2.7). */
export const NOT_SHIPPED: Partial<Record<View, string>> = {
  // package 11: Chat and Automations ship in v1.0
}

/** Views whose package has not landed in this build; one line until it does. */
const PENDING_VIEW: Partial<Record<View, ReactNode>> = {}

/** `⌘/Ctrl+1…9` and `⌘/Ctrl+,` (03 §10.1; printed only in the Shortcuts view). */
const VIEW_KEYS: Record<string, View> = {
  Digit1: 'sessions',
  Digit2: 'inbox',
  Digit3: 'notes',
  Digit4: 'calendar',
  Digit5: 'chat',
  Digit6: 'agents',
  Digit7: 'automations',
  Digit8: 'integrations',
  Digit9: 'stats',
  Digit0: 'shortcuts',
  Comma: 'settings',
}

/** The floating query bar pre-commits the view's item type (03 §5.9.1). */
const VIEW_ITEM_TYPE: Partial<Record<View, ItemType>> = {
  sessions: 'sessions',
  inbox: 'messages',
  calendar: 'calendar_events',
}

function isTyping(el: EventTarget | null): boolean {
  const t = el as HTMLElement | null
  if (!t) return false
  return t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable
}

/** Global keys of 03 §10.1 that the shell owns. */
function onGlobalKey(e: KeyboardEvent) {
  // The shell is inert under onboarding (03 §2.8): its keys wait too.
  if (useOnboarding.getState().active) return
  const mod = e.metaKey || e.ctrlKey
  const route = useRouter.getState().route
  if (mod && !e.shiftKey && !e.altKey && VIEW_KEYS[e.code]) {
    e.preventDefault()
    navigate({ view: VIEW_KEYS[e.code] })
    return
  }
  if (mod && !e.shiftKey && e.key.toLowerCase() === 'k') {
    e.preventDefault()
    window.dispatchEvent(new Event('mode:floating-query'))
    return
  }
  if (mod && e.key === '\\') {
    e.preventDefault()
    useShell.getState().toggleSidebar()
    return
  }
  if (mod && !e.shiftKey && e.key.toLowerCase() === 'i') {
    e.preventDefault()
    const s = useShell.getState()
    s.setDetailOpen(!s.detailOpen)
    return
  }
  if (mod && e.key === '[') {
    e.preventDefault()
    useRouter.getState().back()
    return
  }
  if (mod && e.key === ']') {
    e.preventDefault()
    useRouter.getState().forward()
    return
  }
  if (e.altKey && !mod && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
    e.preventDefault()
    if (e.key === 'ArrowLeft') useRouter.getState().back()
    else useRouter.getState().forward()
    return
  }
  if (mod && !e.shiftKey && e.key.toLowerCase() === 'w') {
    // 03 §2.1 / the catalog's `app.close`: a focused card may take the key
    // (it cancels `mode:close-card`); otherwise the window hides and Mode
    // keeps running behind the overlay and the status item.
    e.preventDefault()
    if (window.dispatchEvent(new CustomEvent('mode:close-card', { cancelable: true }))) void invoke('app_hide', {}).catch(() => {})
    return
  }
  if (mod && !e.shiftKey && e.key.toLowerCase() === 'n' && route.view === 'sessions') {
    e.preventDefault()
    useDialogs.getState().openLaunch()
    return
  }
  if (mod && !e.shiftKey && e.key.toLowerCase() === 'n' && route.view === 'agents' && !route.rest) {
    e.preventDefault()
    navigate({ view: 'agents', rest: 'new' })
    return
  }
  if (mod && e.shiftKey && e.key.toLowerCase() === 'f') {
    e.preventDefault()
    useSessionsStore.getState().clearFilters()
    return
  }
  if (e.defaultPrevented || isTyping(e.target)) return
  if (e.key === 'Escape') {
    // Up exactly one drill-in level (03 §2.4): sub-detail → tab → roster → clear the selection.
    const up = upOneLevel(route)
    if (up) {
      e.preventDefault()
      navigate(up)
    } else if (route.view === 'sessions' && useSessionsStore.getState().selectedId) {
      useSessionsStore.getState().select(null)
    }
    return
  }
  if (mod || e.altKey) return
  if (e.key === '/' && route.view === 'sessions' && !route.id) {
    e.preventDefault()
    window.dispatchEvent(new Event('mode:focus-search'))
    return
  }
  if (e.key === '?') {
    e.preventDefault()
    navigate({ view: 'shortcuts', query: route.view })
    return
  }
  if ((e.key === 'h' || e.key === 'H') && route.view === 'sessions' && !route.id) {
    const s = useSessionsStore.getState()
    void s.setShowHistory(!s.showHistory)
  }
}

export function App() {
  const route = useRouter((s) => s.route)
  const setStatus = useCoreStore((s) => s.setStatus)
  const applyChange = useCoreStore((s) => s.applyChange)
  const rows = useSessionsStore((s) => s.rows)
  const health = useSessionsStore((s) => s.health)
  const next2h = useCalendar((s) => s.next2h)
  // reconciliation: the Inbox count follows the store's last push (or its first `inbox_list` before any push)
  useEffect(() => useInbox.subscribe((s, prev) => void (s.urgentCount !== prev.urgentCount && useShell.getState().setCounts({ inboxUrgent: s.urgentCount }))), [])
  const shell = useShell()
  const [floating, setFloating] = useState(false)
  const theme = useAppearance()
  const onboarding = useOnboardingVisible()
  useEffect(() => {
    const open = () => setFloating(true)
    window.addEventListener('mode:floating-query', open)
    return () => window.removeEventListener('mode:floating-query', open)
  }, [])
  const collapsed = sidebarCollapsed(shell)

  // Core status, core pushes, the roster, counts and `app.open`.
  useEffect(() => {
    let cancelled = false
    invoke<CoreStatus>('core_status')
      .then((s) => {
        if (!cancelled) setStatus(s)
      })
      .catch(() => {})
    const offs = [
      listen<CoreChanged>(EVENTS.core, applyChange),
      listen<InboxChanged>(EVENTS.inbox, (p) => useShell.getState().setCounts({ inboxUrgent: p?.urgent_count ?? 0 })),
      listen<NotesChanged>(EVENTS.notes, (p) => useShell.getState().setCounts({ notesOverdue: p?.overdue_count ?? 0 })),
      listen<AppOpen>(APP_OPEN_EVENT, (p) => {
        if (p) navigate(routeFromAppOpen(p))
      }),
      connectSessions(),
      connectSettings(),
      connectInbox(),
      connectCalendar(),
      connectAccounts(),
      // package 11
      connectChat(),
      connectAutomations(),
      connectTheme(),
      connectAgents(),
    ]
    return () => {
      cancelled = true
      for (const off of offs) void off.then((u) => u())
    }
  }, [setStatus, applyChange])

  // Routing: the hash is the route; back / forward fire hashchange.
  useEffect(() => {
    const sync = () => useRouter.getState().sync()
    window.addEventListener('hashchange', sync)
    window.addEventListener('popstate', sync)
    return () => {
      window.removeEventListener('hashchange', sync)
      window.removeEventListener('popstate', sync)
    }
  }, [])

  // Width-driven collapse and the global keys.
  useEffect(() => {
    const onResize = () => useShell.getState().setWidth(window.innerWidth)
    window.addEventListener('resize', onResize)
    window.addEventListener('keydown', onGlobalKey)
    return () => {
      window.removeEventListener('resize', onResize)
      window.removeEventListener('keydown', onGlobalKey)
    }
  }, [])

  const waiting = waitingCount(rows)
  const count = (n: number) => (n > 0 ? n : undefined)
  const items: NavItem[] = NAV_ITEMS.map((i) =>
    i.id === 'sessions'
      ? { ...i, count: count(waiting), waiting: waiting > 0 }
      : i.id === 'inbox'
        ? { ...i, count: count(shell.inboxUrgent) }
        : i.id === 'notes'
          ? { ...i, count: count(shell.notesOverdue) }
          : i.id === 'calendar'
            ? { ...i, count: count(next2h ?? 0) }
            : i.id === 'chat'
              ? { ...i, sub: <ChatSidebarList activeId={route.view === 'chat' ? route.rest || null : null} /> }
              : i,
  )

  // Database newer than the app: one full-window message, no view reachable (XC-33).
  if (health?.db_newer) {
    return (
      <>
        <IconDefs />
      <BrandDefs />
        <div className="ap-full" role="alert" data-testid="db-newer">
          <Mascot size={48} />
          <div>This database was written by a newer version of Mode.</div>
          {health.backups_dir && <code>{health.backups_dir}</code>}
          <button type="button" className="m-btn m-btn-primary">
            Check for updates
          </button>
        </div>
      </>
    )
  }

  // First run (03 §2.8): the shell stays mounted, dimmed and inert, under the onboarding layer until it is finished or skipped.
  return (
    <ThemeProvider theme={theme}>
      <IconDefs />
      <BrandDefs />
      <Window testId="window" className={collapsed ? 'is-sidebar-collapsed' : undefined} inert={onboarding}>
        <Sidebar
          items={items}
          bottom={BOTTOM_ITEMS}
          active={route.view}
          subActive={route.view === 'chat' && !!route.rest}
          collapsed={collapsed}
          onSelect={(id) => navigate({ view: id as View })}
          profile={<AppProfile collapsed={collapsed} />}
        />
        <Main>
          {route.view === 'sessions' ? (
            <SessionsView />
          ) : route.view === 'notes' ? (
            <NotesView initial={route.rest ? { noteId: route.rest } : undefined} />
          ) : route.view === 'settings' ? (
            <SettingsView rest={route.rest} />
          ) : route.view === 'integrations' ? (
            <IntegrationsView rest={route.rest} />
          ) : route.view === 'shortcuts' ? (
            <ShortcutsView query={route.query} />
          ) : route.view === 'stats' ? (
            <StatsView />
          ) : route.view === 'inbox' ? (
            <InboxView rest={route.rest} />
          ) : route.view === 'calendar' ? (
            <CalendarView rest={route.rest} />
          ) : /* package 11 */ route.view === 'chat' ? (
            <ChatView rest={route.rest} />
          ) : route.view === 'automations' ? (
            <AutomationsView rest={route.rest} />
          ) : route.view === 'agents' ? (
            <AgentsView rest={route.rest} />
          ) : (
            <OtherView view={route.view} rest={route.rest} />
          )}
        </Main>
      </Window>
      {!onboarding && route.view !== 'notes' && <CardLayer view={route.view} onExpand={() => navigate({ view: 'notes' })} />}
      {!onboarding && floating && <FloatingQuery view={route.view} viewType={VIEW_ITEM_TYPE[route.view] ?? 'notes'} onClose={() => setFloating(false)} />}
      {onboarding && <Onboarding />}
      {/* package 11 */}
      {!onboarding && <CommandPalette />}
      {!onboarding && <SessionDialogs />}
      <AccountDialogs />
      <MenuHost />
      <NoticeHost />
      {/* reconciliation */}
      <QuitDialog />
    </ThemeProvider>
  )
}

/** Views of later packages: the title, then "Coming in vX" or one neutral line (03 §2.7). */
function OtherView({ view, rest }: { view: View; rest?: string }) {
  const status = useCoreStore((s) => s.status)
  const health = useSessionsStore((s) => s.health)
  const version = NOT_SHIPPED[view]
  const offline = health?.offline && (view === 'inbox' || view === 'calendar' || view === 'chat')
  return (
    <section className="ap-view" aria-label={LABEL[view]}>
      <MainHead title={LABEL[view] ?? 'Sessions'} titleTestId="view-title" />
      <MainBody testId="view-body">
        {offline && (
          <div className="ap-banner" role="status">
            <span>Offline — showing what Mode has; pollers paused</span>
          </div>
        )}
        {view === 'notes' && health?.vault_missing && (
          <div className="ap-banner" role="status">
            <span>The notes folder is not available. Notes will reappear when it is.</span>
          </div>
        )}
        {version ? (
          <div className="ap-empty" data-testid="not-shipped">
            {`Coming in ${version}`}
          </div>
        ) : (
          (PENDING_VIEW[view] ?? (
            <div className="m-list">
              <div className="m-group-label" data-testid="view-status">
                {`${status ? `Mode ${status.version}${status.dev_build ? ' (dev)' : ''}` : 'starting'}${rest ? ` · ${rest}` : ''}`}
              </div>
            </div>
          ))
        )}
      </MainBody>
    </section>
  )
}

