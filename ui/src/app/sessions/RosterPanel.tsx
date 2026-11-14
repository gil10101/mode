// The roster's detail panel (03 §3.1, the first drill-in level): the header
// fields with one primary action, the last reply, the changed files as
// drill-in rows into the Files tab, the live agents and the Links panel.
import { useEffect } from 'react'
import { DetailPanel } from '../../kit/layout'
import { Spinner } from '../../kit/primitives'
import { Diff, GroupLabel } from '../../kit/lists'
import type { AppRosterRow, LinkRow, SessionDetail } from '../../state/contract-sessions'
import { useSessionsStore } from '../../state/sessions-store'
import { navigate } from '../router'
import { openMenu } from '../ui'
import { decide, jumpToTerminal, sessionMenu, useDialogs } from './actions'
import { changeGlyph, elapsed, plural, secondsSince } from './format'
import { isResumable, SessionHeaderView } from './header'
import { liveSeconds } from './Roster'

export interface Primary {
  id: string
  label: string
  run: () => void
}

/**
 * The header's one primary (03 §3.6): Resume when the session can be resumed,
 * Allow while an approval is answerable (the panel, 03 §3.1), else Jump to
 * terminal.
 */
export function headerPrimary(row: AppRosterRow, opts: { history?: boolean; panel?: boolean } = {}): Primary {
  if (opts.history || isResumable(row as Parameters<typeof isResumable>[0]))
    return { id: 'resume', label: 'Resume', run: () => useDialogs.getState().openResume(row.id) }
  if (opts.panel && row.state === 'waiting' && row.pending?.answerable && row.pending.kind === 'permission') {
    const p = row.pending
    return { id: 'allow', label: 'Allow', run: () => void decide(row.id, 'allow', p) }
  }
  return { id: 'jump', label: 'Jump to terminal', run: () => jumpToTerminal(row) }
}

const LINK_GLYPH: Record<string, string> = { note: '≡', message: '✉', thread: '✉', event: '◷' }

export function LinkRows({ links }: { links: LinkRow[] }) {
  return (
    <>
      {links.map((l) => (
        <div
          key={`${l.kind}:${l.id}`}
          className="m-task"
          role="button"
          tabIndex={0}
          aria-disabled={!l.available || undefined}
          onClick={() => l.available && navigate(linkRoute(l))}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && l.available) navigate(linkRoute(l))
          }}
        >
          <span className="m-task-glyph">{LINK_GLYPH[l.kind] ?? '·'}</span>
          <span className="m-task-title">{l.available ? l.title : 'unavailable'}</span>
          <span className="m-t-meta">{l.kind}</span>
        </div>
      ))}
    </>
  )
}

export function linkRoute(l: Pick<LinkRow, 'kind' | 'id'>): string {
  switch (l.kind) {
    case 'note':
      return `notes/${l.id}`
    case 'message':
      return `inbox/message/${l.id}`
    case 'thread':
      return `inbox/thread/${l.id}`
    case 'event':
      return `calendar/${l.id}`
    default:
      return 'sessions'
  }
}

export function RosterPanel({ row, history, now }: { row: AppRosterRow; history: boolean; now: number }) {
  const detail: SessionDetail | undefined = useSessionsStore((s) => s.details[row.id])
  const status = useSessionsStore((s) => s.detailStatus[row.id])
  const rowsAt = useSessionsStore((s) => s.rowsAt)
  const load = useSessionsStore((s) => s.loadDetail)

  useEffect(() => {
    if (!detail && status !== 'loading') void load(row.id)
    // Load once per selection; pushes refresh it (sessions-store).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [row.id])

  // The roster row is the live source for state and timing; the detail adds the rest.
  const header = detail
    ? { ...detail.session, state: row.state, state_since: new Date(now - liveSeconds(row, rowsAt, now) * 1000).toISOString() }
    : null
  const primary = headerPrimary(row, { history, panel: true })
  const preview = row.last_assistant_preview ?? detail?.session.last_assistant_preview
  const files = detail?.files ?? []
  const agents = (detail?.agents ?? row.agents ?? []).filter((a) => !a.ended_at)
  const links = detail?.links ?? []

  return (
    <DetailPanel label={`${row.display_name} summary`} className="ap-panel" swapKey={row.id}>
      {header ? (
        <SessionHeaderView
          header={header}
          now={now}
          primaryLabel={primary.label}
          onPrimary={primary.run}
          onMore={(e) => openMenu(e.currentTarget, sessionMenu(row, { history, primary: primary.id, header: { claude_session_id: header.claude_session_id, transcript_path: header.transcript_path } }))}
        />
      ) : (
        <div className="ap-panel-empty">{status === 'error' ? "Couldn't load this session." : 'Loading…'}</div>
      )}
      {preview && (
        <>
          <GroupLabel>last reply</GroupLabel>
          <p className="m-card-body ap-panel-text">{preview}</p>
        </>
      )}
      {files.length > 0 && (
        <>
          <GroupLabel>{`files · ${files.length}`}</GroupLabel>
          {files.map((f) => (
            <div
              key={f.id}
              className="m-task"
              role="button"
              tabIndex={0}
              title={`${f.source} · ${plural(f.edit_count, 'edit')}`}
              onClick={() => navigate({ view: 'sessions', id: row.id, tab: 'files', sub: f.id })}
              onKeyDown={(e) => {
                if (e.key === 'Enter') navigate({ view: 'sessions', id: row.id, tab: 'files', sub: f.id })
              }}
            >
              <span className="m-task-glyph">{changeGlyph(f.change_kind)}</span>
              <code className="m-task-title">{f.rel_path ?? f.path}</code>
              <Diff add={f.lines_added} del={f.lines_removed} />
              <span className="aw-chev">›</span>
            </div>
          ))}
        </>
      )}
      {agents.length > 0 && (
        <>
          <GroupLabel>{`agents · ${agents.length}`}</GroupLabel>
          {agents.map((a) => (
            <div
              key={a.id}
              className="m-task"
              role="button"
              tabIndex={0}
              onClick={() => navigate({ view: 'sessions', id: row.id, tab: 'agents', sub: a.agent_id ?? a.id })}
              onKeyDown={(e) => {
                if (e.key === 'Enter') navigate({ view: 'sessions', id: row.id, tab: 'agents', sub: a.agent_id ?? a.id })
              }}
            >
              <span className="m-task-glyph">
                <Spinner />
              </span>
              <code className="m-task-title">{a.agent_type ?? 'agent'}</code>
              <span className="m-t-meta">{elapsed(secondsSince(a.started_at, now))}</span>
              <span className="aw-chev">›</span>
            </div>
          ))}
        </>
      )}
      {links.length > 0 && (
        <>
          <GroupLabel>links</GroupLabel>
          <LinkRows links={links} />
        </>
      )}
    </DetailPanel>
  )
}

