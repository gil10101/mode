// The Agents list (03 §7E.1; app-window.html data-view="agents"): the
// toolbar (search, New agent, ⋮ with Sort), one `.m-row` per agent (the live
// count in the state column, the bot mark with the agent's accessory, the
// name over "title · model · effort", the usage cell) and the detail panel of
// the selected one (the brain line, the usage, the rules, Launch session as
// the one primary). With no agents, the empty state of §7E.5.
// Keys (03 §10.9; printed only in the Shortcuts view): ↑ ↓ / J K move, Enter
// opens the editor, ⌘/Ctrl+Enter launches, Delete deletes (confirmation),
// ⌘/Ctrl+N new agent, `/` search.
import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { AgentAvatar } from '../../kit/avatars'
import { Button, MenuButton } from '../../kit/button'
import { DetailPanel, MainBody, MainHead } from '../../kit/layout'
import { Row } from '../../kit/lists'
import { joinDot } from '../../kit/text'
import type { AgentSummary } from '../../state/contract-agents'
import { agentHue, brainLine, useAgents, usageLine, visibleRows, type SortKey } from '../../state/agents-store'
import { usePacks } from '../../state/packs-store'
import { useSessionsStore } from '../../state/sessions-store'
import { useDialogs } from '../sessions/actions'
import { navigate } from '../router'
import { Dialog, openMenu, useNow, type MenuEntry } from '../ui'

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

export function confirmDelete(r: Pick<AgentSummary, 'id' | 'name' | 'automated'>, onDone?: () => void) {
  window.dispatchEvent(new CustomEvent('mode:agent-delete', { detail: { id: r.id, name: r.name, automated: r.automated, onDone } }))
}

/** The delete confirmation (03 §7E.1): sessions keep their rows; automations run without the agent. */
export function DeleteHost() {
  const [target, setTarget] = useState<{ id: string; name: string; automated: boolean; onDone?: () => void } | null>(null)
  useEffect(() => {
    const on = (e: Event) => setTarget((e as CustomEvent<{ id: string; name: string; automated: boolean; onDone?: () => void }>).detail)
    window.addEventListener('mode:agent-delete', on)
    return () => window.removeEventListener('mode:agent-delete', on)
  }, [])
  if (!target) return null
  return (
    <Dialog title={`Delete ${target.name}?`} onClose={() => setTarget(null)} testId="agent-delete">
      <div className="m-card-body">{target.automated ? 'Its sessions stay in the roster; the automations that run it will run without it.' : 'Its sessions stay in the roster.'}</div>
      <div className="m-card-actions">
        <Button
          variant="primary"
          onClick={() => {
            const t = target
            setTarget(null)
            void useAgents
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

/** Export pack (03 §7C.6): the agent as a one-item pack in Downloads. */
async function exportAgentPack(r: AgentSummary) {
  const out = await usePacks.getState().exportPack({ agents: [r.id] })
  useSessionsStore.getState().showNotice(out ? `Exported ${r.name} to ${out.path}` : `Couldn't export ${r.name}: ${usePacks.getState().error ?? 'unknown error'}`)
}

export function rowMenu(r: AgentSummary): MenuEntry[] {
  return [
    { id: 'launch', label: 'Launch session', run: () => useDialogs.getState().openLaunch(r.id) },
    { id: 'edit', label: 'Edit', run: () => navigate({ view: 'agents', rest: r.id }) },
    { id: 'export', label: 'Export pack', run: () => void exportAgentPack(r) },
    { kind: 'sep', id: 'sep' },
    { id: 'delete', label: 'Delete', danger: true, run: () => confirmDelete(r) },
  ]
}

/** The state cell: "2 live" while sessions of the agent run, else "idle". */
function liveState(r: AgentSummary): { state: 'working' | 'idle'; label: string } {
  return r.live > 0 ? { state: 'working', label: `${r.live} live` } : { state: 'idle', label: 'idle' }
}

function Panel({ row, now }: { row: AgentSummary; now: number }) {
  const claudeMissing = useSessionsStore((s) => s.health?.claude_code === 'missing')
  return (
    <DetailPanel className="ag-panel" label={row.name}>
      <div className="ag-panel-head">
        <AgentAvatar size={32} hue={agentHue(row.id)} variant={row.avatar_variant} />
        <div className="m-row-text">
          <div className="m-t-title">{row.name}</div>
          <div className="m-meta">{row.title ?? 'no title'}</div>
        </div>
        <MenuButton label={`${row.name} options`} onClick={(e) => openMenu(e.currentTarget, rowMenu(row).filter((x) => x.id !== 'launch'), row.name)} />
      </div>
      <div className="m-card-body ag-summary">
        <div>{brainLine(row)}</div>
        <div>{usageLine(row, now)}</div>
        <div>{row.notify ? 'alerts on' : 'alerts off'}</div>
      </div>
      <div className="m-card-actions">
        <Button variant="primary" disabled={claudeMissing} title={claudeMissing ? 'Claude Code was not found' : undefined} onClick={() => useDialogs.getState().openLaunch(row.id)}>
          Launch session
        </Button>
        <Button onClick={() => navigate({ view: 'agents', rest: row.id })}>Edit</Button>
      </div>
    </DetailPanel>
  )
}

const SORT_LABEL: [SortKey, string][] = [
  ['position', 'Order added'],
  ['name', 'Name'],
  ['last_used', 'Last used'],
  ['sessions', 'Sessions'],
]

export function AgentsList() {
  const now = useNow(30_000)
  const loaded = useAgents((s) => s.loaded)
  const error = useAgents((s) => s.error)
  const search = useAgents((s) => s.search)
  const sort = useAgents((s) => s.sort)
  const selectedId = useAgents((s) => s.selectedId)
  const all = useAgents((s) => s.rows)
  const rows = useMemo(() => visibleRows(all, search, sort), [all, search, sort])
  const searchRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const selected = rows.find((r) => r.id === selectedId) ?? null

  const onKey = (e: KeyboardEvent<HTMLElement>) => {
    const mod = e.metaKey || e.ctrlKey
    if (mod && !e.shiftKey && e.key.toLowerCase() === 'n') {
      e.preventDefault()
      navigate({ view: 'agents', rest: 'new' })
      return
    }
    if (isTyping(e.target)) {
      if (e.key === 'Escape' && e.target === searchRef.current) {
        useAgents.getState().setSearch('')
        searchRef.current?.blur()
      }
      return
    }
    if (e.key === '/') {
      e.preventDefault()
      searchRef.current?.focus()
      return
    }
    const i = rows.findIndex((r) => r.id === selectedId)
    if (e.key === 'ArrowDown' || e.key === 'j' || e.key === 'ArrowUp' || e.key === 'k') {
      e.preventDefault()
      const down = e.key === 'ArrowDown' || e.key === 'j'
      const next = rows[Math.min(rows.length - 1, Math.max(0, i < 0 ? 0 : i + (down ? 1 : -1)))]
      if (next) useAgents.getState().select(next.id)
      return
    }
    if (!selected) return
    if (e.key === 'Enter' && mod) {
      e.preventDefault()
      useDialogs.getState().openLaunch(selected.id)
    } else if (e.key === 'Enter') {
      e.preventDefault()
      navigate({ view: 'agents', rest: selected.id })
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault()
      confirmDelete(selected)
    }
  }

  return (
    <section className="ap-view ag-view" aria-label="Agents" data-testid="agents-view" onKeyDown={onKey}>
      <MainHead title="Agents">
        <input ref={searchRef} className="m-input aw-search" placeholder="Search agents…" aria-label="Search agents" value={search} onChange={(e) => useAgents.getState().setSearch(e.target.value)} />
        <Button onClick={() => navigate({ view: 'agents', rest: 'new' })}>New agent</Button>
        <MenuButton
          onClick={(e) =>
            openMenu(
              e.currentTarget,
              SORT_LABEL.map(([k, label]) => ({ id: k, label: `Sort by ${label.toLowerCase()}`, checked: sort === k, run: () => useAgents.getState().setSort(k) })),
              'Agents',
            )
          }
        />
      </MainHead>
      {loaded && !all.length ? (
        <MainBody className="ag-empty">
          <p className="m-card-body">An agent is a way of working you launch sessions as: a model and effort, instructions and memories, the tools it may use.</p>
          <div className="m-btn-group">
            <Button variant="primary" onClick={() => navigate({ view: 'agents', rest: 'new' })}>
              New agent
            </Button>
          </div>
        </MainBody>
      ) : (
        <MainBody className={selected ? 'ag-split' : 'ag-only'}>
          <div ref={listRef} className="m-list" role="list" aria-label="Agents">
            {error && <div className="ag-err">{error}</div>}
            {rows.map((r) => {
              const live = liveState(r)
              return (
                <Row
                  key={r.id}
                  state={live.state}
                  stateLabel={live.label}
                  avatar={<AgentAvatar hue={agentHue(r.id)} variant={r.avatar_variant} />}
                  name={r.name}
                  detail={joinDot(r.title, r.model ?? "the CLI's default model", r.effort ? `${r.effort} effort` : null)}
                  meta={usageLine(r, now)}
                  selected={r.id === selectedId}
                  onClick={() => useAgents.getState().select(r.id === selectedId ? null : r.id)}
                  onMenu={(e) => {
                    e.stopPropagation()
                    openMenu(e.currentTarget, rowMenu(r), r.name)
                  }}
                  menuLabel={`${r.name} options`}
                  testId={`agent-${r.id}`}
                />
              )
            })}
            {loaded && !rows.length && search && <div className="ag-list-empty">{`No agent matches “${search}”.`}</div>}
          </div>
          {selected && <Panel row={selected} now={now} />}
        </MainBody>
      )}
    </section>
  )
}

export { PERMISSION_LABEL }
