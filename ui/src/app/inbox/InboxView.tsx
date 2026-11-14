// The Inbox view (03 §4; app-window.html data-view="inbox"): the toolbar
// (title, search, ⋮), the connector banners (§4.9), the account chips and
// filter chips, the thread list in sections (urgent, recent, then the quiet
// "Snoozed n ›  Low priority n ›" line that expands in place), the detail
// panel of the selected thread, and the full-page thread view
// (`inbox/thread/<id>`, `inbox/message/<id>`). Keys (03 §10.3; printed only
// in Shortcuts): ↑ ↓ / J K move, Enter opens (a second Enter the full page),
// Shift+Enter without marking read, R U E S O N V T X, ⌘/Ctrl+A, ⌘/Ctrl+Shift+A,
// `/` search, ⌘/Ctrl+Z undo, Esc back.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { Chip } from '../../kit/controls'
import { MainBody, MainHead, Split } from '../../kit/layout'
import { GroupLabel, List } from '../../kit/lists'
import type { ThreadRow } from '../../state/contract-connectors'
import { useAccounts } from '../../state/accounts-store'
import { groupSections, hasFilters, isSnoozed, knownSenders, useInbox, type SavedFilter } from '../../state/inbox-store'
import { useSetting } from '../../state/settings-store'
import { useShell } from '../shell-store'
import { navigate } from '../router'
import { openMenu, useNow, type MenuEntry } from '../ui'
import { levelEntries, noteFromMessage, openInSource, openThread } from './actions'
import { ConnectorBanners } from './banners'
import { FiltersDialog, SaveFilterDialog, filterChips, sourceLabel } from './FiltersDialog'
import { ThreadItem } from './rows'
import { SnoozeHost, openSnooze } from './SnoozeMenu'
import { LinkHost, openLinkPicker } from './LinkDialog'
import { ThreadPage } from './ThreadPage'
import { ThreadPanel } from './ThreadPanel'
import './inbox.css'

const INBOX_SERVICES = ['gmail', 'slack'] as const

function isTyping(el: EventTarget | null): boolean {
  const t = el as HTMLElement | null
  return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)
}

/** `inbox/message/<id>`: the message's thread (pushed rows, then `thread_get`, else the id itself), scrolled to the message. */
function MessageRoute({ id, markRead }: { id: string; markRead: boolean }) {
  const [thread, setThread] = useState<string | null>(() => useInbox.getState().messageThreads[id] ?? null)
  useEffect(() => {
    let live = true
    void useInbox
      .getState()
      .resolveMessage(id)
      .then((t) => {
        if (live) setThread(t)
      })
    return () => {
      live = false
    }
  }, [id])
  if (!thread) return <section className="ap-view" aria-label="Thread" aria-busy="true" data-testid="message-resolving" />
  return <ThreadPage id={thread} messageId={id} markRead={markRead} />
}

export function InboxView({ rest }: { rest?: string }) {
  const [kind, id] = (rest ?? '').split('/') as [string | undefined, string | undefined]
  const [noRead, setNoRead] = useState(false)

  useEffect(() => {
    const inbox = useInbox.getState()
    if (!inbox.loaded) void inbox.load()
    if (!useAccounts.getState().loaded) void useAccounts.getState().load()
  }, [])

  // Shift+Enter hands "without marking read" to the thread view it opens.
  useEffect(() => {
    const on = (e: Event) => setNoRead(!!(e as CustomEvent<boolean>).detail)
    window.addEventListener('mode:inbox-no-read', on)
    return () => window.removeEventListener('mode:inbox-no-read', on)
  }, [])

  return (
    <>
      {kind === 'thread' && id ? <ThreadPage id={id} markRead={!noRead} /> : kind === 'message' && id ? <MessageRoute id={id} markRead={!noRead} /> : <InboxList />}
      <SnoozeHost />
      <LinkHost />
    </>
  )
}

function InboxList() {
  const now = useNow(30_000)
  const rows = useInbox((s) => s.rows)
  const loaded = useInbox((s) => s.loaded)
  const error = useInbox((s) => s.error)
  const filter = useInbox((s) => s.filter)
  const account = useInbox((s) => s.account)
  const search = useInbox((s) => s.search)
  const searchRows = useInbox((s) => s.searchRows)
  const selectedId = useInbox((s) => s.selectedId)
  const stick = useInbox((s) => s.stick)
  const expanded = useInbox((s) => s.expanded)
  const multi = useInbox((s) => s.multi)
  const picked = useInbox((s) => s.picked)
  const details = useInbox((s) => s.details)
  const nextCursor = useInbox((s) => s.nextCursor)
  const accounts = useAccounts((s) => s.accounts)
  const accountsLoaded = useAccounts((s) => s.loaded)
  const sources = useAccounts((s) => s.sources)
  const detailOpen = useShell((s) => s.detailOpen)
  const [saved] = useSetting<SavedFilter[]>('inbox.saved_filters')
  const [dialog, setDialog] = useState<'filters' | 'save' | null>(null)
  const [query, setQuery] = useState(search)
  const searchRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const sentinel = useRef<HTMLDivElement>(null)
  const lastPick = useRef<string | null>(null)

  // Typing filters after 150 ms (03 §2.4); results replace the list (§4.8).
  useEffect(() => {
    const t = setTimeout(() => {
      useInbox.getState().setSearch(query)
      if (query.trim()) void useInbox.getState().runSearch()
    }, 150)
    return () => clearTimeout(t)
  }, [query])

  // Infinite scroll within retention (03 §4.1).
  useEffect(() => {
    const el = sentinel.current
    if (!el || typeof IntersectionObserver === 'undefined' || !nextCursor) return
    const io = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) void useInbox.getState().loadMore()
    })
    io.observe(el)
    return () => io.disconnect()
  }, [nextCursor, rows.length])

  // View-level keys: `/`, ⌘/Ctrl+Z, ⌘/Ctrl+Shift+A, ⌘/Ctrl+Shift+F, Esc.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (mod && !e.shiftKey && e.key.toLowerCase() === 'z' && !isTyping(e.target)) {
        e.preventDefault()
        useInbox.getState().undo()
      } else if (mod && e.shiftKey && e.key.toLowerCase() === 'a') {
        e.preventDefault()
        void useInbox.getState().markAllRead()
      } else if (mod && e.shiftKey && e.key.toLowerCase() === 'f') {
        useInbox.getState().clearFilters()
      } else if (!mod && !e.altKey && e.key === '/' && !isTyping(e.target)) {
        e.preventDefault()
        searchRef.current?.focus()
      } else if (e.key === 'Escape' && !isTyping(e.target) && !e.defaultPrevented) {
        const s = useInbox.getState()
        if (s.multi) s.setMulti(false)
        else if (s.searchRows) {
          setQuery('')
          s.setSearch('')
        } else if (s.selectedId) s.select(null)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const searching = searchRows !== null && !!search.trim()
  const sections = useMemo(() => groupSections(rows, now, { archived: filter.archived, stick }), [rows, now, filter.archived, stick])
  const visible: ThreadRow[] = searching ? searchRows! : [...sections.urgent, ...sections.main, ...(expanded.snoozed ? sections.snoozed : []), ...(expanded.low ? sections.low : [])]
  const selected = selectedId ? (rows.find((r) => r.id === selectedId) ?? searchRows?.find((r) => r.id === selectedId) ?? details[selectedId]?.thread) : undefined
  const senders = knownSenders(rows)
  const chips = filterChips(filter, {
    contact: (cid) => senders.find((x) => (x.contact_id ?? x.address ?? x.name) === cid)?.name ?? cid,
    source: (sid) => {
      const s = sources.find((x) => x.id === sid)
      return s ? sourceLabel(s, undefined) : sid
    },
  })
  const inboxAccounts = accounts.filter((a) => a.services.includes('gmail') || a.services.includes('slack'))
  const budgetPaused = rows.some((r) => r.urgency_why === 'not scored (budget)')

  const select = (row: ThreadRow, opts: { markRead?: boolean } = {}) => useInbox.getState().select(row.id, opts)
  const open = (row: ThreadRow, markRead: boolean) => openThread(row.id, markRead)

  const onRowClick = (row: ThreadRow) => (e: MouseEvent<HTMLDivElement>) => {
    const s = useInbox.getState()
    if (e.shiftKey && (s.multi || lastPick.current)) {
      // Shift+click picks the range (03 §2.4 multi-select).
      const ids = visible.map((r) => r.id)
      const a = ids.indexOf(lastPick.current ?? row.id)
      const b = ids.indexOf(row.id)
      const range = ids.slice(Math.min(a, b), Math.max(a, b) + 1)
      useInbox.setState({ multi: true, picked: Array.from(new Set([...s.picked, ...range])) })
      return
    }
    if (e.metaKey || e.ctrlKey || s.multi) {
      s.togglePick(row.id)
      lastPick.current = row.id
      return
    }
    lastPick.current = row.id
    select(row)
  }

  const onRowKey = (row: ThreadRow) => (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.metaKey || e.ctrlKey) {
      if (e.key.toLowerCase() === 'a' && !e.shiftKey) {
        e.preventDefault()
        useInbox.getState().pickAll()
      }
      return
    }
    if (e.altKey) return
    const inbox = useInbox.getState()
    const rowsEls = () => Array.from(listRef.current?.querySelectorAll<HTMLElement>('[data-thread]') ?? [])
    const move = (d: 1 | -1) => {
      const els = rowsEls()
      const i = els.indexOf(e.currentTarget)
      els[Math.max(0, Math.min(els.length - 1, i + d))]?.focus()
    }
    const k = e.key
    if (k === 'ArrowDown' || k === 'j' || k === 'J') return (e.preventDefault(), move(1))
    if (k === 'ArrowUp' || k === 'k' || k === 'K') return (e.preventDefault(), move(-1))
    if (k === 'Enter') {
      e.preventDefault()
      if (inbox.selectedId === row.id) open(row, !e.shiftKey)
      else select(row, { markRead: !e.shiftKey })
      return
    }
    if (k === ' ') return (e.preventDefault(), select(row))
    const detail = inbox.details[row.id]
    switch (k.toLowerCase()) {
      case 'r':
        e.preventDefault()
        void inbox.markRead(row.id, true)
        break
      case 'u':
        e.preventDefault()
        void inbox.markRead(row.id, false)
        break
      case 'e':
        e.preventDefault()
        move(1)
        void inbox.archive(row.id)
        break
      case 's':
        e.preventDefault()
        if (isSnoozed(row, Date.now())) void inbox.unsnooze(row.id)
        else openSnooze(e.currentTarget, inbox.multi && inbox.picked.length ? inbox.picked : [row.id], detail?.messages[detail.messages.length - 1]?.id)
        break
      case 'o':
        e.preventDefault()
        void openInSource(row.id)
        break
      case 'n':
        e.preventDefault()
        void noteFromMessage(row, detail)
        break
      case 'v':
        e.preventDefault()
        void inbox.setVip(row.id, !row.sender_vip)
        break
      case 'l':
        e.preventDefault()
        openLinkPicker(row.id)
        break
      case 't':
        e.preventDefault()
        openMenu(e.currentTarget, levelEntries(row), 'Level')
        break
      case 'x':
        e.preventDefault()
        inbox.togglePick(row.id)
        lastPick.current = row.id
        break
    }
  }

  const toolbarMenu = (): MenuEntry[] => {
    const inbox = useInbox.getState()
    const list: MenuEntry[] = [
      { id: 'filters', label: 'Filter…', run: () => setDialog('filters') },
      { id: 'unread', label: 'Unread only', checked: !!filter.unread_only, run: () => inbox.setFilter({ ...filter, unread_only: !filter.unread_only }) },
      { id: 'archived', label: 'Show archived', checked: !!filter.archived, run: () => inbox.setFilter({ ...filter, archived: !filter.archived }) },
      { id: 'select', label: multi ? 'Stop selecting' : 'Select', run: () => inbox.setMulti(!multi) },
      { id: 'all-read', label: 'Mark all read', run: () => void inbox.markAllRead() },
      { kind: 'sep', id: 'sep' },
      { id: 'save', label: 'Save filter…', dimmed: !hasFilters(filter) && !account, reason: !hasFilters(filter) && !account ? 'no filter set' : undefined, run: () => setDialog('save') },
    ]
    const savedList = Array.isArray(saved) ? saved : []
    if (savedList.length) {
      list.push({ kind: 'label', id: 'saved-label', label: 'saved filters' })
      for (const f of savedList)
        list.push({
          id: `saved-${f.name}`,
          label: f.name,
          run: () => {
            useInbox.setState({ account: f.account_id ?? null })
            inbox.setFilter(f.filter)
          },
        })
    }
    if (hasFilters(filter)) list.push({ kind: 'sep', id: 'sep2' }, { id: 'clear', label: 'Clear filters', run: () => inbox.clearFilters() })
    return list
  }

  const item = (r: ThreadRow) => <ThreadItem key={r.id} row={r} now={now} selected={r.id === selectedId || picked.includes(r.id)} query={searching ? search : undefined} onClick={onRowClick(r)} onKeyDown={onRowKey(r)} />

  const noAccounts = accountsLoaded && inboxAccounts.length === 0
  const firstSync = !noAccounts && loaded && rows.length === 0 && !hasFilters(filter) && inboxAccounts.every((a) => !a.last_sync_at)

  const list = (
    <div className="m-stack ib-list" ref={listRef} data-testid="inbox-list">
      <ConnectorBanners services={INBOX_SERVICES} />
      {budgetPaused && (
        <div className="ib-line" role="status">
          {'Scoring paused for today: the AI token budget was reached · '}
          <a href="#/settings/ai" onClick={(e) => (e.preventDefault(), navigate({ view: 'settings', rest: 'ai' }))}>
            AI settings
          </a>
        </div>
      )}
      <div className="aw-chips ib-chips" data-testid="account-chips">
        <AccountChip label="All" on={!account} onClick={() => useInbox.getState().setAccount(null)} />
        {inboxAccounts.map((a) => (
          <AccountChip key={a.id} label={a.display_name} on={account === a.id} onClick={() => useInbox.getState().setAccount(a.id)} title={a.status === 'paused' ? `${a.display_name} · paused` : undefined} />
        ))}
        {chips.map((c) => (
          <Chip key={c.key} removable onRemove={() => useInbox.getState().setFilter(c.remove())}>
            {c.label}
          </Chip>
        ))}
      </div>
      {multi && <BulkBar count={picked.length} />}
      {error && !loaded ? (
        <div className="ib-error" role="alert">
          <span>{error}</span>
          <Button onClick={() => void useInbox.getState().load()}>Retry</Button>
        </div>
      ) : !loaded || firstSync ? (
        <Skeleton first={firstSync} />
      ) : searching ? (
        searchRows!.length ? (
          <div data-section="results" role="listbox" aria-label="Search results">
            <GroupLabel className="ib-label">results</GroupLabel>
            <List divided>{searchRows!.map(item)}</List>
          </div>
        ) : (
          <div className="ap-empty" data-testid="no-match">
            {`No messages match “${search.trim()}”.`}
            <Button onClick={() => useInbox.getState().setFilter({ ...filter, archived: true })}>Include archived</Button>
          </div>
        )
      ) : rows.length === 0 && hasFilters(filter) ? (
        <div className="ap-empty" data-testid="no-match">
          Nothing matches these filters.
          <Button onClick={() => useInbox.getState().clearFilters()}>Clear filters</Button>
        </div>
      ) : (
        <>
          {sections.urgent.length ? (
            <div data-section="urgent" role="listbox" aria-label="Urgent" aria-multiselectable={multi || undefined}>
              <GroupLabel className="ib-label">urgent</GroupLabel>
              <List divided>{sections.urgent.map(item)}</List>
            </div>
          ) : (
            <div className="aw-fold" data-section="urgent">
              Nothing urgent
            </div>
          )}
          {sections.main.length > 0 && (
            <div data-section="main" role="listbox" aria-label="Recent" aria-multiselectable={multi || undefined}>
              <GroupLabel className="ib-label">recent</GroupLabel>
              <List divided>{sections.main.map(item)}</List>
            </div>
          )}
          {(sections.snoozed.length > 0 || sections.low.length > 0) && (
            <div className="aw-fold ib-fold" data-testid="fold">
              {sections.snoozed.length > 0 && (
                <span role="button" tabIndex={0} aria-expanded={expanded.snoozed} onClick={() => useInbox.getState().toggleSection('snoozed')} onKeyDown={(e) => e.key === 'Enter' && useInbox.getState().toggleSection('snoozed')}>
                  {`Snoozed ${sections.snoozed.length} ${expanded.snoozed ? '▾' : '›'}`}
                </span>
              )}
              {sections.low.length > 0 && (
                <span role="button" tabIndex={0} aria-expanded={expanded.low} onClick={() => useInbox.getState().toggleSection('low')} onKeyDown={(e) => e.key === 'Enter' && useInbox.getState().toggleSection('low')}>
                  {`Low priority ${sections.low.length} ${expanded.low ? '▾' : '›'}`}
                </span>
              )}
            </div>
          )}
          {expanded.snoozed && sections.snoozed.length > 0 && (
            <div data-section="snoozed" role="listbox" aria-label="Snoozed" aria-multiselectable={multi || undefined}>
              <GroupLabel className="ib-label">snoozed</GroupLabel>
              <List divided>{sections.snoozed.map(item)}</List>
            </div>
          )}
          {expanded.low && sections.low.length > 0 && (
            <div data-section="low" role="listbox" aria-label="Low priority" aria-multiselectable={multi || undefined}>
              <GroupLabel className="ib-label">low priority</GroupLabel>
              <List divided>{sections.low.map(item)}</List>
            </div>
          )}
          {rows.length === 0 && <div className="ib-line">No messages yet.</div>}
          {nextCursor && <div className="ib-sentinel" ref={sentinel} />}
        </>
      )}
    </div>
  )

  return (
    <section className="ap-view ib-view" aria-label="Inbox" data-testid="inbox-view">
      <MainHead title="Inbox" titleTestId="view-title">
        <input
          ref={searchRef}
          className="m-input aw-search"
          placeholder="Search messages…"
          aria-label="Search messages"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.preventDefault()
              setQuery('')
              useInbox.getState().setSearch('')
              e.currentTarget.blur()
            }
          }}
        />
        <MenuButton label="More" onClick={(e) => openMenu(e.currentTarget, toolbarMenu(), 'Inbox options')} />
      </MainHead>
      <MainBody testId="view-body">
        {noAccounts ? (
          <div className="ap-empty" data-testid="inbox-empty">
            Connect Gmail or Slack to see messages here.
            <Button variant="primary" onClick={() => navigate({ view: 'integrations' })}>
              Connect account
            </Button>
          </div>
        ) : selected && detailOpen ? (
          <Split className="ib-split">
            {list}
            <ThreadPanel row={selected} now={now} />
          </Split>
        ) : (
          <div className="ib-only">{list}</div>
        )}
      </MainBody>
      {dialog === 'filters' && <FiltersDialog onClose={() => setDialog(null)} />}
      {dialog === 'save' && <SaveFilterDialog onClose={() => setDialog(null)} />}
    </section>
  )
}

function AccountChip({ label, on, onClick, title }: { label: string; on: boolean; onClick: () => void; title?: string }) {
  return (
    <span
      className={`m-chip ib-chip${on ? ' is-on' : ''}`}
      role="radio"
      aria-checked={on}
      tabIndex={0}
      title={title}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onClick()
        }
      }}
    >
      {label}
    </span>
  )
}

function BulkBar({ count }: { count: number }) {
  const inbox = useInbox.getState()
  return (
    <div className="ib-bulk" data-testid="bulk-bar">
      <span>{count ? `${count} selected` : 'Select threads with X or a click'}</span>
      <Button quiet disabled={!count} onClick={() => void inbox.bulk('read')}>
        Mark read
      </Button>
      <Button quiet disabled={!count} onClick={() => void inbox.bulk('archive')}>
        Archive
      </Button>
      <Button quiet disabled={!count} onClick={(e) => openSnooze(e.currentTarget, useInbox.getState().picked)}>
        Snooze
      </Button>
      <Button quiet onClick={() => inbox.setMulti(false)}>
        Done
      </Button>
    </div>
  )
}

/** Skeleton rows after 150 ms (03 §9); the first sync line when accounts have not synced yet (§4.10). */
function Skeleton({ first }: { first: boolean }) {
  const [show, setShow] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setShow(true), 150)
    return () => clearTimeout(t)
  }, [])
  if (!show && !first) return null
  return (
    <div className="ib-skeleton" aria-busy="true" aria-label="Loading messages" data-testid="inbox-skeleton">
      {first && <div className="ib-line">first sync running</div>}
      <i />
      <i />
      <i />
    </div>
  )
}

