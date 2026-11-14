// The six tabs of session detail (03 §3.7–§3.10, §3.9A, §3.9B): Overview
// (thread + prompt box), Terminal (the xterm emulator of `ui/src/terminal`
// on the PTY channel), Files (rows + diff pane), Agents (rows + agent thread), Connectors
// (read-only rows) and Transcript (one block per JSONL line).
import { Fragment, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { AgentAvatar } from '../../kit/avatars'
import { Button, MenuButton } from '../../kit/button'
import { Crumb } from '../../kit/controls'
import { Diff, GroupLabel, Row, StateCol } from '../../kit/lists'
import { BrandMark, brandForConnector } from '../../kit/brands'
import { ConnectorRow as ConnectorRowView } from '../../kit/views'
import { invoke } from '../../tauri'
import type { AgentRow, ConnectorRow, DiffResult, FileRow, SessionDetail, TranscriptLine, TranscriptPage } from '../../state/contract-sessions'
import { useSessionsStore } from '../../state/sessions-store'
import { cmdErrorText } from '../../state/contract'
import { navigate, type Route } from '../router'
import { copyText, openMenu } from '../ui'
import { jumpToTerminal, openPath, revealPath, useDialogs } from './actions'
import {
  between,
  changeLetter,
  clockSeconds,
  connectorState,
  elapsed,
  parseConnectorDetail,
  permissionModeMeaning,
  plural,
  rowState,
  secondsSince,
  stateWord,
  tokens,
} from './format'
import { isExternal, isResumable } from './header'
import { agentItems, AgentThread, OverviewThread, type ThreadContext } from './thread'
import { PromptBox, PromptDelivery } from '../../terminal/PromptBox'
import { TerminalPanel, useTerminalKind } from '../../terminal/TerminalPanel'

export interface TabProps {
  detail: SessionDetail
  route: Route
  now: number
  ctx: ThreadContext
}

function crumbLink(label: ReactNode, to: Route): ReactNode {
  return (
    <a
      href={`#/sessions/${to.id}`}
      onClick={(e) => {
        e.preventDefault()
        navigate(to)
      }}
    >
      {label}
    </a>
  )
}

/* ---- Overview --------------------------------------------------------------- */

/** The prompt box rules of 05 §14 / 03 §3.7: what the bottom of the Overview shows. */
export type FootKind =
  | { kind: 'composer'; disabled: boolean; placeholder: string }
  | { kind: 'hint'; text: string; action: 'jump' | 'resume' | null }

export function footFor(d: SessionDetail): FootKind {
  const h = d.session
  if (h.state === 'untracked') return { kind: 'hint', text: 'Resume to track this session', action: 'resume' }
  if (isResumable(h) || !d.alive) return { kind: 'hint', text: 'Resume to continue', action: 'resume' }
  if (isExternal(h) || h.launch_mode === 'external') return { kind: 'hint', text: 'Send prompts from your terminal', action: 'jump' }
  if (h.state === 'failed') return { kind: 'hint', text: 'Resume to see the failure in the terminal', action: 'resume' }
  const reply = `Reply to ${h.display_name}…`
  if (d.driver === 'headless') return { kind: 'composer', disabled: true, placeholder: 'A turn is running' }
  if (h.state === 'working') return { kind: 'composer', disabled: true, placeholder: 'Claude is working…' }
  if (h.state === 'waiting') {
    const k = d.pending?.kind ?? h.waiting_kind
    if (k === 'permission' || k === 'question') return { kind: 'composer', disabled: true, placeholder: 'Answer the pending card first' }
    if (k === 'elicitation' || k === 'agent_needs_input') return { kind: 'hint', text: 'Needs input in the terminal', action: 'jump' }
  }
  return { kind: 'composer', disabled: false, placeholder: reply }
}

function Hint({ text, action, row, tab }: { text: string; action: 'jump' | 'resume' | null; row: ThreadContext['row']; tab?: boolean }) {
  const link = (label: string) => (
    <a
      href="#"
      onClick={(e) => {
        e.preventDefault()
        if (action === 'jump') jumpToTerminal(row)
        else useDialogs.getState().openResume(row.id)
      }}
    >
      {label}
    </a>
  )
  // "Resume to continue": the verb itself is the link; otherwise the action follows after " · ".
  const lead = action === 'resume' && text.startsWith('Resume ')
  return (
    <div className={`sd-hint${tab ? ' is-tab' : ''}`}>
      {lead ? (
        <>
          {link('Resume')}
          {text.slice(6)}
        </>
      ) : (
        <>
          {action ? `${text} · ` : text}
          {action && link(action === 'jump' ? 'Jump to terminal' : 'Resume')}
        </>
      )}
    </div>
  )
}

export function OverviewTab({ detail, route, ctx }: TabProps) {
  const scroller = useRef<HTMLDivElement>(null)
  const foot = footFor(detail)
  const count = detail.thread.length
  // Open at the newest item, or at the card an approval route names (03 §2.5).
  useLayoutEffect(() => {
    const el = scroller.current
    if (!el) return
    const card = route.approval ? el.querySelector<HTMLElement>('.m-thread-permission:not(.is-done), .m-thread-question') : null
    if (card) card.scrollIntoView({ block: 'center' })
    else el.scrollTop = el.scrollHeight
  }, [route.approval, count])
  return (
    <section className="sd-panel is-active" role="tabpanel" aria-label="Overview" data-testid="tab-overview">
      <div className="sd-thread" ref={scroller}>
        {detail.thread.length === 0 && detail.session.state === 'untracked' ? (
          <div className="ap-empty">Resume to track this session.</div>
        ) : (
          <OverviewThread items={detail.thread} ctx={ctx} />
        )}
        <PromptDelivery sessionId={detail.session.id} thread={detail.thread} />
      </div>
      <div className="sd-foot">
        {foot.kind === 'composer' ? (
          <PromptBox
            sessionId={detail.session.id}
            disabled={foot.disabled}
            placeholder={foot.placeholder}
            thread={detail.thread}
            stop={detail.driver === 'headless'}
            onAction={(action) => (action === 'jump' ? jumpToTerminal(ctx.row) : useDialogs.getState().openResume(ctx.row.id))}
          />
        ) : (
          <Hint text={foot.text} action={foot.action} row={ctx.row} />
        )}
      </div>
    </section>
  )
}

/* ---- Terminal --------------------------------------------------------------- */

export function TerminalTab({ detail, ctx }: TabProps) {
  const h = detail.session
  // 03 §3.6 / §3.8, 05 §14: live PTY · headless turn · "Open a terminal" · resume / jump hints.
  const kind = useTerminalKind(detail)
  let body: ReactNode
  if (kind === 'resume') body = <Hint tab text="Resume to open a terminal" action="resume" row={ctx.row} />
  else if (kind === 'jump') body = <Hint tab text="This session runs in your terminal" action="jump" row={ctx.row} />
  else body = <TerminalPanel sessionId={h.id} kind={kind} onResume={() => useDialogs.getState().openResume(h.id)} />
  return (
    <section className="sd-panel is-active" role="tabpanel" aria-label="Terminal" data-testid="tab-terminal">
      {body}
    </section>
  )
}

/* ---- Files -------------------------------------------------------------------- */

function fileMenu(f: FileRow, diff: DiffResult | null): { id: string; label: string; run: () => void }[] {
  const items = [
    { id: 'editor', label: 'Open in editor', run: () => openPath(f.path) },
    { id: 'reveal', label: 'Reveal', run: () => revealPath(f.path) },
    { id: 'copy-path', label: 'Copy path', run: () => copyText(f.path, 'Path copied') },
  ]
  if (diff) items.push({ id: 'copy-diff', label: 'Copy diff', run: () => copyText(diff.lines.map((l) => l.text).join('\n'), 'Diff copied') })
  return items
}

export function FilesTab({ detail, route, now }: TabProps) {
  const h = detail.session
  const files = detail.files
  const selected = files.find((f) => f.id === route.sub) ?? files[0]
  const [diff, setDiff] = useState<DiffResult | null>(null)
  const [busy, setBusy] = useState(false)
  const load = useSessionsStore((s) => s.loadDetail)

  useEffect(() => {
    let live = true
    setDiff(null)
    if (!selected) return
    invoke<DiffResult>('session_diff', { session_id: h.id, file_change_id: selected.id })
      .then((d) => live && setDiff(d))
      // The core's refusal (`not_found` for a file change that is gone) in place of the basis.
      .catch((e) => live && setDiff({ basis: cmdErrorText(e), lines: [] }))
    return () => {
      live = false
    }
  }, [h.id, selected?.id, selected])

  const refresh = async () => {
    setBusy(true)
    await invoke('session_files_refresh', { session_id: h.id }).catch((e) => useSessionsStore.getState().showNotice(`Couldn't refresh git status: ${cmdErrorText(e)}`))
    await load(h.id)
    setBusy(false)
  }

  if (h.state === 'untracked' && !files.length)
    return (
      <section className="sd-panel is-active" role="tabpanel" aria-label="Files" data-testid="tab-files">
        <div className="ap-empty">
          No hook events.
          <Button onClick={() => void refresh()}>Run git status</Button>
        </div>
      </section>
    )

  const total = files.reduce((a, f) => ({ add: a.add + f.lines_added, del: a.del + f.lines_removed }), { add: 0, del: 0 })
  return (
    <section className="sd-panel is-active" role="tabpanel" aria-label="Files" data-testid="tab-files">
      <div className="sd-files">
        <div className="ap-files-col">
          <div className="ap-files-head">
            {files.length ? (
              <>
                <Diff add={total.add} del={total.del} />
                <span className="m-t-meta">{`· ${plural(files.length, 'file')}`}</span>
              </>
            ) : (
              <span className="m-t-meta">No changes yet</span>
            )}
            {h.repo_root ? (
              <Button quiet disabled={busy} onClick={() => void refresh()}>
                Refresh git status
              </Button>
            ) : null}
          </div>
          {!h.repo_root && <div className="ap-info">Not a git repository — hook events only</div>}
          {files.length > 0 && (
            <div className="m-list" role="listbox" aria-label="Files changed">
              {files.map((f) => {
                const kind = changeLetter(f.change_kind)
                const path = f.rel_path ?? f.path
                return (
                  <div
                    key={f.id}
                    className={`sd-file${f.id === selected?.id ? ' is-selected' : ''}`}
                    role="option"
                    aria-selected={f.id === selected?.id}
                    tabIndex={0}
                    title={`${f.source} · ${plural(f.edit_count, 'edit')}${f.committed ? ' · committed' : ''}`}
                    onClick={() => navigate({ view: 'sessions', id: h.id, tab: 'files', sub: f.id })}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') navigate({ view: 'sessions', id: h.id, tab: 'files', sub: f.id })
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault()
                      openMenu(e.currentTarget, fileMenu(f, f.id === selected?.id ? diff : null), `Actions for ${path}`)
                    }}
                  >
                    <span className={`k${kind === 'A' ? ' is-added' : kind === 'D' ? ' is-deleted' : ''}`}>{kind}</span>
                    <span className="p">{f.change_kind === 'renamed' && f.old_path ? `${f.old_path} → ${path}` : path}</span>
                    <Diff add={f.lines_added} del={f.lines_removed} />
                    <span className="t">{elapsed(secondsSince(f.last_seen_at, now))}</span>
                  </div>
                )
              })}
            </div>
          )}
        </div>
        {selected && (
          <div className="sd-diff" role="region" aria-label={`Diff of ${selected.rel_path ?? selected.path}`} data-testid="diff-pane">
            <div className="sd-diff-head">
              <Crumb items={[crumbLink(h.display_name, { view: 'sessions', id: h.id }), crumbLink('Files', { view: 'sessions', id: h.id, tab: 'files' }), <code key="p">{selected.rel_path ?? selected.path}</code>]} />
              <Diff add={selected.lines_added} del={selected.lines_removed} />
              {diff?.basis ? <span className="m-t-meta">{`· ${diff.basis}`}</span> : null}
              <MenuButton label="Diff actions" onClick={(e) => openMenu(e.currentTarget, fileMenu(selected, diff), 'Diff actions')} />
            </div>
            <pre aria-label="Diff preview">
              {(diff?.lines ?? []).map((l, i) => (
                <span key={i} className={`ln${l.kind === 'ctx' ? '' : ` ${l.kind}`}`}>
                  <span className="no">{l.kind === 'hunk' ? '' : `${l.old_no ?? ''}`.padEnd(5) + `${l.new_no ?? ''}`.padStart(2)}</span>
                  <span className="tx">{l.text}</span>
                </span>
              ))}
            </pre>
            {diff?.truncated && <div className="sd-note">Diff truncated; open the file</div>}
          </div>
        )}
      </div>
    </section>
  )
}

/* ---- Agents ------------------------------------------------------------------- */

const AGENT_ORDER = ['working', 'waiting', 'finished', 'idle', 'failed']

export function agentGroups(agents: readonly AgentRow[]): { label: string; rows: AgentRow[] }[] {
  const map = new Map<string, AgentRow[]>()
  for (const a of agents) {
    const w = stateWord(a.state)
    if (!map.has(w)) map.set(w, [])
    map.get(w)!.push(a)
  }
  return Array.from(map.entries())
    .sort((a, b) => (AGENT_ORDER.indexOf(a[0]) + 99) % 99 - (AGENT_ORDER.indexOf(b[0]) + 99) % 99)
    .map(([label, rows]) => ({ label, rows }))
}

function agentSeconds(a: AgentRow, now: number): number {
  return a.ended_at ? between(a.started_at, a.ended_at, now) : secondsSince(a.started_at, now)
}

function agentMeta(a: AgentRow, now: number): string {
  return `${elapsed(agentSeconds(a, now))} · ${plural(a.tools_used, 'tool')}`
}

function agentMenu(sessionId: string, a: AgentRow) {
  return [
    { id: 'thread', label: 'Open agent thread', run: () => navigate({ view: 'sessions', id: sessionId, tab: 'agents', sub: a.agent_id ?? a.id }) },
    { id: 'transcript', label: 'Open transcript', run: () => navigate({ view: 'sessions', id: sessionId, tab: 'transcript' }) },
    { id: 'copy', label: 'Copy agent id', run: () => copyText(a.agent_id ?? a.id, 'Agent id copied') },
  ]
}

export function AgentsTab({ detail, now }: TabProps) {
  const h = detail.session
  if (!detail.agents.length)
    return (
      <section className="sd-panel is-active" role="tabpanel" aria-label="Agents" data-testid="tab-agents">
        <div className="ap-empty">{h.state === 'untracked' ? 'Install hooks to see agents.' : 'No agents seen.'}</div>
      </section>
    )
  return (
    <section className="sd-panel is-active" role="tabpanel" aria-label="Agents" data-testid="tab-agents">
      <div className="m-list sd-list" role="list" aria-label="Agents spawned by this session">
        {agentGroups(detail.agents).map((g) => (
          <Fragment key={g.label}>
            <GroupLabel>{g.label}</GroupLabel>
            {g.rows.map((a) => {
              const type = a.agent_type ?? 'agent'
              const open = () => navigate({ view: 'sessions', id: h.id, tab: 'agents', sub: a.agent_id ?? a.id })
              return (
                <Row
                  key={a.id}
                  agent
                  state={rowState(a.state)}
                  avatar={<AgentAvatar />}
                  name={type}
                  detail={
                    a.current_tool && !a.ended_at ? (
                      <>
                        <code>{a.current_tool}</code>
                        {a.current_tool_arg ? ` ${a.current_tool_arg}` : ''}
                      </>
                    ) : (
                      (a.last_assistant_preview ?? '')
                    )
                  }
                  meta={agentMeta(a, now)}
                  menuLabel={`More for ${type}`}
                  onMenu={(e) => openMenu(e.currentTarget, agentMenu(h.id, a), `Actions for ${type}`)}
                  onClick={open}
                  testId={`agent-row-${a.id}`}
                />
              )
            })}
          </Fragment>
        ))}
      </div>
    </section>
  )
}

/** The agent thread sub-detail: the crumb bar replaces the tab strip (session-detail.html callout 16). */
export function AgentSubPage({ detail, route, now, ctx }: TabProps) {
  const h = detail.session
  const agent = detail.agents.find((a) => (a.agent_id ?? a.id) === route.sub || a.id === route.sub)
  const up = () => navigate({ view: 'sessions', id: h.id, tab: 'agents' })
  const items = agent ? agentItems(detail.thread, agent.agent_id ?? agent.id) : []
  return (
    <div className="sd-sub is-active" role="region" aria-label={`Agent thread: ${agent?.agent_type ?? route.sub}`} data-testid="agent-thread">
      <div className="sd-crumbbar">
        <Button variant="icon" quiet className="sd-back" aria-label="Back" onClick={up}>
          ‹
        </Button>
        <Crumb items={[crumbLink(h.display_name, { view: 'sessions', id: h.id }), crumbLink('Agents', { view: 'sessions', id: h.id, tab: 'agents' }), agent?.agent_type ?? 'agent']} />
        {agent && (
          <>
            <StateCol state={rowState(agent.state)} />
            <span className="m-t-meta">{agentMeta(agent, now)}</span>
          </>
        )}
      </div>
      <div className="sd-thread">
        {agent ? items.length ? <AgentThread items={items} ctx={ctx} running={!agent.ended_at} /> : <div className="ap-empty">Nothing recorded for this agent yet.</div> : <div className="ap-empty">This agent is no longer in the session.</div>}
      </div>
    </div>
  )
}

/* ---- Connectors --------------------------------------------------------------- */

const CONNECTOR_GROUPS: { kind: string; label: (mode: string) => string }[] = [
  { kind: 'mcp_server', label: () => 'MCP servers' },
  { kind: 'tool_rule', label: (mode) => `Tools · ${mode}` },
  { kind: 'plugin', label: () => 'Plugins' },
  { kind: 'relay', label: () => 'Relay' },
]

export function connectorText(c: ConnectorRow, now: number): { name: ReactNode; detail?: ReactNode; meta?: ReactNode } {
  const d = parseConnectorDetail(c)
  switch (c.kind) {
    case 'mcp_server':
      return { name: c.name, detail: d.detail ?? c.status, meta: d.tools !== undefined ? plural(d.tools, 'tool') : '—' }
    case 'tool_rule':
      return { name: <code>{c.name}</code>, detail: d.detail ?? d.origin ?? 'cli' }
    case 'plugin':
      return { name: c.name, detail: d.detail, meta: d.version }
    case 'relay': {
      const last = d.last_event_at ?? c.seen_at
      return { name: 'Mode relay', detail: `last event ${elapsed(secondsSince(last, now))} ago`, meta: d.hooks !== undefined ? plural(d.hooks, 'hook') : d.relay_version }
    }
    default:
      return { name: c.name, detail: d.detail }
  }
}

/** The row's mark (10 §7.4.16): an MCP server's service, or the MCP mark; rules, plugins and the relay carry none. */
export function connectorMark(c: ConnectorRow): ReactNode {
  const brand = brandForConnector(c.kind, c.name)
  return brand ? <BrandMark id={brand} size={16} label="" /> : undefined
}

export function ConnectorsTab({ detail, now }: TabProps) {
  const h = detail.session
  const mode = h.permission_mode ?? 'default'
  const open = (c: ConnectorRow) => navigate(c.kind === 'plugin' && /backend|ai/i.test(c.name) ? 'integrations/ai-backends' : 'integrations/claude-code')
  if (!detail.connectors.length)
    return (
      <section className="sd-panel is-active" role="tabpanel" aria-label="Connectors" data-testid="tab-connectors">
        <div className="ap-empty">{h.state === 'untracked' ? 'Install hooks to see what this session can reach.' : 'Nothing seen yet.'}</div>
      </section>
    )
  return (
    <section className="sd-panel is-active" role="tabpanel" aria-label="Connectors" data-testid="tab-connectors">
      <div className="m-list sd-list" role="list" aria-label="What this session can reach">
        {CONNECTOR_GROUPS.map((g) => {
          const rows = detail.connectors.filter((c) => c.kind === g.kind)
          if (!rows.length) return null
          return (
            <Fragment key={g.kind}>
              <div className="m-group-label" title={g.kind === 'tool_rule' ? `${mode}: ${permissionModeMeaning(mode)}` : undefined}>
                {g.label(mode)}
              </div>
              {rows.map((c) => {
                const t = connectorText(c, now)
                return (
                  <ConnectorRowView
                    key={`${c.kind}:${c.name}`}
                    state={connectorState(c.status)}
                    stateLabel={c.status}
                    mark={connectorMark(c)}
                    name={t.name}
                    detail={t.detail}
                    meta={t.meta}
                    onClick={() => open(c)}
                    onMore={() => undefined}
                  />
                )
              })}
            </Fragment>
          )
        })}
      </div>
      <p className="sd-note">{`What this session can reach, read-only · ${detail.connectors_source === 'init' ? "from this session's init event" : 'from hook events so far'}`}</p>
    </section>
  )
}

/* ---- Transcript --------------------------------------------------------------- */

const PAGE = 500

export function TranscriptTab({ detail }: TabProps) {
  const h = detail.session
  const [page, setPage] = useState<TranscriptPage | null>(null)
  const [query, setQuery] = useState('')
  const [open, setOpen] = useState<Record<string, 'content' | 'raw'>>({})
  const [failed, setFailed] = useState(false)

  useEffect(() => {
    let live = true
    invoke<TranscriptPage>('session_transcript', { session_id: h.id, limit: PAGE })
      .then((p) => live && setPage(p))
      .catch(() => live && setFailed(true))
    return () => {
      live = false
    }
  }, [h.id])

  const earlier = async () => {
    if (!page?.lines.length) return
    const first = Math.min(...page.lines.filter((l) => !l.agent_id).map((l) => l.line))
    const more = await invoke<TranscriptPage>('session_transcript', { session_id: h.id, before_line: first, limit: PAGE }).catch(() => null)
    if (more) setPage({ ...page, has_more: more.has_more, lines: [...more.lines, ...page.lines] })
  }

  const path = page?.path ?? detail.transcript.path
  const q = query.trim().toLowerCase()
  const lines = useMemo(() => (page?.lines ?? []).filter((l) => !q || `${l.label} ${l.text} ${l.content ?? ''}`.toLowerCase().includes(q)), [page, q])
  const main = lines.filter((l) => !l.agent_id)
  const subs = new Map<string, TranscriptLine[]>()
  for (const l of lines) if (l.agent_id) subs.set(l.agent_id, [...(subs.get(l.agent_id) ?? []), l])

  const missing = failed || page?.exists === false || detail.transcript.exists === false
  const row = (l: TranscriptLine) => {
    const key = `${l.agent_id ?? ''}:${l.line}`
    const mode = open[key]
    const toggle = () => setOpen((o) => ({ ...o, [key]: o[key] ? (undefined as never) : 'content' }))
    return (
      <Fragment key={key}>
        <div
          className="sd-ev"
          role="button"
          tabIndex={0}
          aria-expanded={!!mode}
          onClick={toggle}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              toggle()
            }
          }}
        >
          <span className="t">{clockSeconds(l.at)}</span>
          <span className="n">{l.label}</span>
          <span className="s">{l.text}</span>
          <span className="a">{l.usage ? `${tokens(l.usage.input_tokens + l.usage.output_tokens)} tokens` : ''}</span>
        </div>
        {mode && (
          <div className="ap-ev-body">
            {mode === 'raw' ? (l.raw ?? '') : (l.content ?? l.text)}
            {l.raw && (
              <div>
                <Button quiet onClick={() => setOpen((o) => ({ ...o, [key]: mode === 'raw' ? 'content' : 'raw' }))}>
                  {mode === 'raw' ? 'Content' : 'Raw JSON'}
                </Button>
              </div>
            )}
          </div>
        )}
      </Fragment>
    )
  }

  return (
    <section className="sd-panel is-active" role="tabpanel" aria-label="Transcript" data-testid="tab-transcript">
      <div className="sd-events">
        <div className="sd-events-head">
          <span className="m-meta">
            <code>{path ?? 'no transcript'}</code>
          </span>
          {page?.size_bytes != null && <span className="m-t-meta">{`· ${Math.round(page.size_bytes / 1000)} kB`}</span>}
          <input className="m-input ap-transcript-search" type="search" placeholder="Search this transcript" aria-label="Search this transcript" value={query} onChange={(e) => setQuery(e.target.value)} />
          <MenuButton
            label="Transcript actions"
            onClick={(e) =>
              openMenu(
                e.currentTarget,
                path
                  ? [
                      { id: 'open', label: 'Open file', run: () => openPath(path) },
                      { id: 'copy', label: 'Copy path', run: () => copyText(path, 'Path copied') },
                    ]
                  : [{ id: 'none', label: 'No transcript', dimmed: true, run: () => undefined }],
                'Transcript actions',
              )
            }
          />
        </div>
        {missing ? (
          <div className="ap-empty">{`Transcript not found at ${path ?? 'its recorded path'}.`}</div>
        ) : !page ? (
          <div className="m-list ap-skeleton">
            <div className="m-shimmer">
              <i />
              <i />
              <i />
            </div>
          </div>
        ) : (
          <div className="m-list" aria-label="Transcript, oldest first">
            {page.has_more && (
              <div className="aw-fold" role="button" tabIndex={0} onClick={() => void earlier()}>
                Load earlier lines
              </div>
            )}
            {main.map(row)}
            {Array.from(subs.entries()).map(([id, ls]) => (
              <Fragment key={id}>
                <GroupLabel>{ls[0].agent_type ?? id}</GroupLabel>
                {ls.map(row)}
              </Fragment>
            ))}
            {page.subagent_files === 0 && <div className="sd-note">Subagent transcripts: none</div>}
          </div>
        )}
      </div>
    </section>
  )
}
