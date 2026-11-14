// The Automations list (03 §7B.1; app-window.html data-view="automations"):
// the toolbar (search, New automation, ⋮ with Sort and "Show disabled only"),
// the kit's AutomationRow per automation with "disabled" ones last under a
// label, and the detail panel of the selected one (name, trigger phrase,
// action summary, Run now, the last three runs). With no automations, the
// empty state of §7B.5 with its two example cards.
// Keys (03 §10.8; printed only in the Shortcuts view): ↑ ↓ / J K move,
// Enter opens the editor, Space toggles, ⌘/Ctrl+Enter runs now,
// ⌘/Ctrl+D duplicates, H opens the history, Delete deletes (confirmation),
// ⌘/Ctrl+N new automation, `/` search.
import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { DetailPanel, MainBody, MainHead } from '../../kit/layout'
import { GroupLabel, List, Row } from '../../kit/lists'
import { AutomationRow as KitRow } from '../../kit/views'
import type { AskChatAction, AutomationRow, CreateNoteAction, StartSessionAction } from '../../state/contract-automations'
import { agoLabel, exampleDraft, lastRunCell, nextRunLabel, runDetail, runStartedLabel, deviceZone, useAutomations, type SortKey } from '../../state/automations-store'
import { invoke } from '../../tauri'
import { useAgents } from '../../state/agents-store'
import { usePacks } from '../../state/packs-store'
import { useSessionsStore } from '../../state/sessions-store'
import { useShell } from '../shell-store'
import { navigate } from '../router'
import { Dialog, openMenu, useNow, type MenuEntry } from '../ui'
import { runMenu } from './Runs'

function isTyping(el: EventTarget | null): boolean {
  const t = el as HTMLElement | null
  return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable)
}

const PERMISSION_LABEL: Record<string, string> = {
  default: 'default permissions',
  plan: 'plan mode',
  acceptEdits: 'accept edits',
  auto: 'auto',
  dontAsk: "don't ask",
  bypassPermissions: 'bypass permissions',
}

/** The panel's action summary: the repo or folder and the prompt's first line (03 §7B.1); the agent it runs as (03 §7E). */
export function actionSummary(r: AutomationRow, folder: (id: string | null) => string = (id) => id ?? 'Unsorted', agent: (id: string) => string | null = () => null): string[] {
  if (r.action_kind === 'start_session') {
    const a = r.action as StartSessionAction
    const as = a.profile_id ? agent(a.profile_id) : null
    return [
      `Start a Claude Code session in ${a.cwd.replace(/^\/Users\/[^/]+/, '~')}`,
      a.prompt.split('\n')[0] ?? '',
      [a.mode, PERMISSION_LABEL[a.permission_mode] ?? a.permission_mode, a.token_cap ? `token cap ${a.token_cap >= 1000 ? `${Math.round(a.token_cap / 1000)}k` : a.token_cap}` : null, as ? `as ${as}` : null]
        .filter(Boolean)
        .join(' · '),
    ]
  }
  if (r.action_kind === 'create_note') {
    const a = r.action as CreateNoteAction
    return [`Create a note in ${folder(a.folder_id)}/`, a.title_pattern, a.tags.length ? a.tags.map((t) => `#${t}`).join(' ') : ''].filter(Boolean)
  }
  const a = r.action as AskChatAction
  return [`Ask Chat and save the answer as a note in ${folder(a.folder_id)}/`, a.prompt.split('\n')[0] ?? '']
}

export function confirmDelete(r: AutomationRow, onDone?: () => void) {
  window.dispatchEvent(new CustomEvent('mode:automation-delete', { detail: { id: r.id, name: r.name, onDone } }))
}

/** The delete confirmation (03 §7B.1: "Delete (confirmation; runs go with it)"). */
export function DeleteHost() {
  const [target, setTarget] = useState<{ id: string; name: string; onDone?: () => void } | null>(null)
  useEffect(() => {
    const on = (e: Event) => setTarget((e as CustomEvent<{ id: string; name: string; onDone?: () => void }>).detail)
    window.addEventListener('mode:automation-delete', on)
    return () => window.removeEventListener('mode:automation-delete', on)
  }, [])
  if (!target) return null
  return (
    <Dialog title={`Delete ${target.name}?`} onClose={() => setTarget(null)} testId="automation-delete">
      <div className="m-card-body">Its run history goes with it.</div>
      <div className="m-card-actions">
        <Button
          variant="primary"
          onClick={() => {
            const t = target
            setTarget(null)
            void useAutomations
              .getState()
              .remove(t.id)
              .then((ok) => ok && t.onDone?.())
          }}
        >
          Delete
        </Button>
        <Button onClick={() => setTarget(null)}>Cancel</Button>
      </div>
    </Dialog>
  )
}

/** Export pack (03 §7C.6): the automation as a one-item pack in Downloads. */
async function exportAutomationPack(r: AutomationRow) {
  const out = await usePacks.getState().exportPack({ automations: [r.id] })
  useSessionsStore.getState().showNotice(out ? `Exported ${r.name} to ${out.path}` : `Couldn't export ${r.name}: ${usePacks.getState().error ?? 'unknown error'}`)
}

export function rowMenu(r: AutomationRow): MenuEntry[] {
  const s = useAutomations.getState()
  return [
    { id: 'run', label: 'Run now', dimmed: r.running, reason: r.running ? 'A run is still running' : undefined, run: () => void s.runNow(r.id) },
    { id: 'edit', label: 'Edit', run: () => navigate({ view: 'automations', rest: r.id }) },
    { id: 'dup', label: 'Duplicate', run: () => void s.duplicate(r.id) },
    { id: 'history', label: 'History', run: () => navigate({ view: 'automations', rest: `${r.id}/runs` }) },
    { id: 'export', label: 'Export pack', run: () => void exportAutomationPack(r) },
    { kind: 'sep', id: 'sep' },
    { id: 'delete', label: 'Delete', danger: true, run: () => confirmDelete(r) },
  ]
}

function Panel({ row, now }: { row: AutomationRow; now: number }) {
  const runs = useAutomations((s) => s.runs[row.id])
  const agents = useAgents((s) => s.rows)
  const tz = row.schedule.tz || deviceZone()
  const next = nextRunLabel(row, now)
  return (
    <DetailPanel className="au-panel" label={row.name}>
      <div className="au-panel-head">
        <span className="m-t-title">{row.name}</span>
        <MenuButton
          label={`${row.name} options`}
          onClick={(e) =>
            openMenu(
              e.currentTarget,
              rowMenu(row).filter((x) => x.id !== 'run'),
              row.name,
            )
          }
        />
      </div>
      <div className="m-meta">{`${row.trigger_phrase}${next ? ` · next ${next}` : ''}`}</div>
      <div className="m-card-body au-summary">
        {actionSummary(row, undefined, (id) => agents.find((g) => g.id === id)?.name ?? null).map((line, i) => (
          <div key={i} title={line}>
            {line}
          </div>
        ))}
      </div>
      {row.problem === 'claude_not_found' && (
        <div className="au-err">
          {'claude not found · '}
          <a onClick={() => navigate({ view: 'integrations', rest: 'claude-code' })} role="link">
            Claude Code card
          </a>
        </div>
      )}
      {row.problem === 'no_ai_backend' && (
        <div className="au-err">
          {'no AI backend · '}
          <a onClick={() => navigate({ view: 'settings', rest: 'ai' })} role="link">
            Settings → AI
          </a>
        </div>
      )}
      <div className="m-card-actions">
        <Button variant="primary" disabled={row.running} onClick={() => void useAutomations.getState().runNow(row.id)}>
          Run now
        </Button>
        <Button onClick={() => navigate({ view: 'automations', rest: row.id })}>Edit</Button>
      </div>
      <GroupLabel>Last runs</GroupLabel>
      <List>
        {runs && !runs.length && <div className="au-list-empty">No runs yet.</div>}
        {(runs ?? []).slice(0, 3).map((run) => {
          const d = runDetail(run, now)
          return (
            <Row
              key={run.id}
              plain
              state={run.state === 'skipped' ? 'idle' : run.state === 'running' ? 'working' : run.state === 'failed' ? 'failed' : 'finished'}
              stateLabel={run.state}
              name={runStartedLabel(run.started_at, tz)}
              detail={[d.duration, d.result ?? d.error].filter(Boolean).join(' · ')}
              meta={agoLabel(run.started_at, now)}
              onClick={() => navigate({ view: 'automations', rest: `${row.id}/runs/${run.id}` })}
              onMenu={(e) => openMenu(e.currentTarget, runMenu(run), 'Run')}
            />
          )
        })}
      </List>
    </DetailPanel>
  )
}

function EmptyState() {
  const create = async (which: 'morning' | 'nightly') => {
    const s = useAutomations.getState()
    // The core's drafts (`automation_examples`: the session example in the last used repository); the view's own otherwise.
    const name = which === 'morning' ? 'Morning brief' : 'Nightly test run'
    let draft = (await s.examples())?.find((d) => d.name === name) ?? null
    if (!draft) {
      let cwd = ''
      try {
        cwd = (await invoke<string[]>('sessions_recent_folders'))?.[0] ?? ''
      } catch {
        /* the editor asks for a repository */
      }
      draft = exampleDraft(which, cwd)
    }
    s.edit({ ...draft, enabled: false })
    const saved = await s.save()
    navigate({ view: 'automations', rest: saved ? saved.id : 'new' })
  }
  return (
    <>
      <p className="m-card-body" data-testid="automations-empty">
        Automations run on a schedule and start a session, create a note or ask Chat.
      </p>
      <div className="au-examples">
        <div className="au-example">
          <div className="m-row-name">Morning brief</div>
          <div className="m-meta">Weekdays 08:00 · Ask Chat, save as a note</div>
          <div className="m-meta">"Summarize today's inbox and calendar" → Unsorted</div>
          <Button onClick={() => void create('morning')}>Use this</Button>
        </div>
        <div className="au-example">
          <div className="m-row-name">Nightly test run</div>
          <div className="m-meta">Daily 23:00 · Start a Claude Code session</div>
          <div className="m-meta">last used repository · "Run the test suite and summarize the failures"</div>
          <Button onClick={() => void create('nightly')}>Use this</Button>
        </div>
      </div>
    </>
  )
}

export function AutomationsList() {
  const now = useNow(1000)
  const rows = useAutomations((s) => s.rows)
  const loaded = useAutomations((s) => s.loaded)
  const error = useAutomations((s) => s.error)
  const search = useAutomations((s) => s.search)
  const sort = useAutomations((s) => s.sort)
  const disabledOnly = useAutomations((s) => s.disabledOnly)
  const selectedId = useAutomations((s) => s.selectedId)
  const detailOpen = useShell((s) => s.detailOpen)
  const searchRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const menuAnchor = useRef<Element | null>(null)
  // search, sort and disabledOnly above subscribe the view; `visible()` reads them.
  void [search, sort, disabledOnly]
  const visible = useAutomations.getState().visible()
  const on = visible.filter((r) => r.enabled)
  const off = visible.filter((r) => !r.enabled)
  const selected = rows.find((r) => r.id === selectedId) ?? null
  const empty = loaded && rows.length === 0

  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'n') {
        e.preventDefault()
        navigate({ view: 'automations', rest: 'new' })
      } else if (!mod && !e.altKey && e.key === '/' && !isTyping(e.target)) {
        e.preventDefault()
        searchRef.current?.focus()
      } else if (e.key === 'Escape' && !isTyping(e.target) && !e.defaultPrevented && useAutomations.getState().selectedId) {
        useAutomations.getState().select(null)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const order = [...on, ...off]
  const indexOf = (el: EventTarget | null) => {
    const rowsEls = Array.from(listRef.current?.querySelectorAll('.m-automation') ?? [])
    return rowsEls.indexOf(el as Element)
  }
  const focusRow = (i: number) => (listRef.current?.querySelectorAll<HTMLElement>('.m-automation')[i] ?? null)?.focus()

  function onListKey(e: KeyboardEvent<HTMLDivElement>) {
    const i = indexOf(e.target)
    if (i < 0) return
    const r = order[i]!
    const mod = e.metaKey || e.ctrlKey
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'j' || e.key === 'k') {
      e.preventDefault()
      const j = Math.max(0, Math.min(order.length - 1, i + (e.key === 'ArrowDown' || e.key === 'j' ? 1 : -1)))
      useAutomations.getState().select(order[j]!.id)
      focusRow(j)
    } else if (e.key === 'Enter' && mod) {
      e.preventDefault()
      void useAutomations.getState().runNow(r.id)
    } else if (e.key === 'Enter') {
      e.preventDefault()
      navigate({ view: 'automations', rest: r.id })
    } else if (e.key === ' ') {
      e.preventDefault()
      void useAutomations.getState().setEnabled(r.id, !r.enabled)
    } else if (mod && e.key.toLowerCase() === 'd') {
      e.preventDefault()
      void useAutomations.getState().duplicate(r.id)
    } else if (!mod && (e.key === 'h' || e.key === 'H')) {
      e.preventDefault()
      navigate({ view: 'automations', rest: `${r.id}/runs` })
    } else if (e.key === 'Delete' || (mod && e.key === 'Backspace')) {
      e.preventDefault()
      confirmDelete(r)
    }
  }

  const sortEntries = (): MenuEntry[] => {
    const s = useAutomations.getState()
    const opt = (k: SortKey, label: string): MenuEntry => ({ id: `sort-${k}`, label, checked: s.sort === k, run: () => s.setSort(k) })
    return [
      { kind: 'label', id: 'sort', label: 'Sort' },
      opt('position', 'Manual order'),
      opt('name', 'Name'),
      opt('next', 'Next run'),
      opt('last', 'Last run'),
      { kind: 'sep', id: 'sep' },
      { id: 'disabled', label: 'Show disabled only', checked: s.disabledOnly, run: () => s.setDisabledOnly(!s.disabledOnly) },
    ]
  }

  const row = (r: AutomationRow) => {
    const cell = lastRunCell(r, now)
    return (
      <KitRow
        key={r.id}
        name={r.name}
        on={r.enabled}
        onToggle={(v) => void useAutomations.getState().setEnabled(r.id, v)}
        trigger={r.trigger_phrase}
        action={r.action_phrase}
        lastRun={cell ? { state: cell.state, label: cell.label } : undefined}
        nextRun={nextRunLabel(r, now)}
        onClick={() => useAutomations.getState().select(r.id === selectedId ? null : r.id)}
        onMore={() => {
          const anchor = menuAnchor.current?.closest('button') ?? menuAnchor.current
          if (anchor) openMenu(anchor, rowMenu(r), r.name)
        }}
        className={r.id === selectedId ? 'is-selected' : undefined}
      />
    )
  }

  // The ⋮ of a kit row reports no anchor: the pointer-down target is the button it opens from.
  const list = (
    <div ref={listRef} className="m-list" onPointerDownCapture={(e) => (menuAnchor.current = e.target as Element)} onKeyDown={onListKey} role="list" aria-label="Automations">
      {on.map(row)}
      {off.length > 0 && <GroupLabel>disabled</GroupLabel>}
      {off.map(row)}
      {!visible.length && <div className="au-list-empty">{search ? `No automation matches “${search}”.` : 'Nothing here.'}</div>}
    </div>
  )

  return (
    <section className="ap-view au-view" aria-label="Automations" data-testid="automations-view">
      <MainHead title="Automations" titleTestId="view-title">
        {!empty && (
          <input
            ref={searchRef}
            className="m-input aw-search"
            placeholder="Search automations…"
            aria-label="Search automations"
            value={search}
            onChange={(e) => useAutomations.getState().setSearch(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Escape') {
                useAutomations.getState().setSearch('')
                e.currentTarget.blur()
              }
            }}
          />
        )}
        <Button variant={empty ? 'primary' : 'default'} onClick={() => navigate({ view: 'automations', rest: 'new' })}>
          New automation
        </Button>
        <MenuButton label="More" onClick={(e) => openMenu(e.currentTarget, sortEntries(), 'Automations options')} />
      </MainHead>
      <MainBody className={empty ? 'au-empty' : selected && detailOpen ? 'au-split' : 'au-only'} testId="view-body">
        {error && !rows.length ? (
          <div className="ap-banner" role="alert">
            <span>{error}</span>
            <Button quiet onClick={() => void useAutomations.getState().load()}>
              Retry
            </Button>
          </div>
        ) : empty ? (
          <EmptyState />
        ) : (
          <>
            {list}
            {selected && detailOpen && <Panel row={selected} now={now} />}
          </>
        )}
      </MainBody>
    </section>
  )
}
