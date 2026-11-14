// Run history and the run page (03 §7B.3; app-window.html "run history"
// figure): rows of state · started · duration and the link to the created
// session or note, `⋮` Open result / Copy error; Enter opens the run page
// under `Morning brief › Runs › Wed 21 Oct · 08:00` with the run's fields and
// the result below. Esc / ‹ go up one level.
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { GroupLabel, Row } from '../../kit/lists'
import { MainBody } from '../../kit/layout'
import { joinDot } from '../../kit/text'
import type { RunRow } from '../../state/contract-automations'
import { agoLabel, deviceZone, runDetail, runResultRoute, runStartedLabel, useAutomations, wallOf } from '../../state/automations-store'
import { useSessionsStore } from '../../state/sessions-store'
import { invoke } from '../../tauri'
import { navigate } from '../router'
import { copyText, openMenu, useNow, type MenuEntry } from '../ui'
import { actionSummary } from './AutomationsList'

export function runMenu(run: RunRow): MenuEntry[] {
  const route = runResultRoute(run)
  return [
    { id: 'open', label: 'Open result', dimmed: !route, reason: route ? undefined : 'No result', run: () => route && navigate(route) },
    { id: 'copy', label: 'Copy error', dimmed: !run.error, run: () => run.error && copyText(run.error, 'Copied the error') },
  ]
}

const STATE: Record<string, 'finished' | 'failed' | 'idle' | 'working'> = { finished: 'finished', failed: 'failed', skipped: 'idle', running: 'working' }

/**
 * The sub-detail head (03 §2.4): `‹` up one level, the breadcrumb (the item's
 * name first; earlier segments are links), `⋮`. The segments are bare spans
 * so the crumb is the kit's `.m-crumb` structure (app-window.html run history).
 */
export function CrumbHead({ items, more, extra }: { items: { label: ReactNode; to?: string }[]; more?: () => MenuEntry[]; extra?: ReactNode }) {
  const up = items.length > 1 ? items[items.length - 2]!.to : 'automations'
  return (
    <div className="m-main-head">
      <Button variant="icon" quiet aria-label="Back" onClick={() => navigate(up ?? 'automations')}>
        ‹
      </Button>
      <nav className="m-crumb au-crumb" aria-label="Breadcrumb">
        {items.map((it, i) =>
          it.to && i < items.length - 1 ? (
            <span key={i} className="au-link" role="link" tabIndex={0} onClick={() => navigate(it.to!)} onKeyDown={(e) => e.key === 'Enter' && navigate(it.to!)}>
              {it.label}
            </span>
          ) : (
            <span key={i} aria-current={i === items.length - 1 ? 'page' : undefined}>
              {it.label}
            </span>
          ),
        )}
      </nav>
      {extra}
      {more && <MenuButton onClick={(e) => openMenu(e.currentTarget, more(), 'Run')} />}
    </div>
  )
}

function RunDetailLine({ run, now }: { run: RunRow; now: number }) {
  const d = runDetail(run, now)
  const route = runResultRoute(run)
  const parts: ReactNode[] = []
  if (d.duration) parts.push(d.duration)
  if (d.result)
    parts.push(
      route ? (
        <a
          href={`#/${route}`}
          onClick={(e) => {
            e.preventDefault()
            e.stopPropagation()
            navigate(route)
          }}
        >
          {d.result}
        </a>
      ) : (
        d.result
      ),
    )
  if (d.error) parts.push(d.error)
  // Text and the one link only, as the wireframe's detail line (no wrappers).
  return <>{joinDot(...parts)}</>
}

export function RunHistory({ id }: { id: string }) {
  const now = useNow(1000)
  const row = useAutomations((s) => s.rows.find((r) => r.id === id))
  const runs = useAutomations((s) => s.runs[id])
  const loaded = useAutomations((s) => s.loaded)
  const listRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    void useAutomations.getState().loadRuns(id, 50)
  }, [id, row?.last_run?.id, row?.last_run?.state])

  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented && !(e.target as HTMLElement)?.closest?.('input,textarea')) navigate({ view: 'automations' })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  if (loaded && !row)
    return (
      <section className="ap-view au-view" aria-label="Run history">
        <CrumbHead items={[{ label: 'Unavailable' }]} />
        <MainBody>
          <div className="ap-empty">This automation no longer exists.</div>
        </MainBody>
      </section>
    )
  const tz = row?.schedule.tz || deviceZone()
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const els = Array.from(listRef.current?.querySelectorAll<HTMLElement>('.m-row') ?? [])
    const i = els.indexOf(e.target as HTMLElement)
    if (i < 0) return
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'j' || e.key === 'k') {
      e.preventDefault()
      els[Math.max(0, Math.min(els.length - 1, i + (e.key === 'ArrowDown' || e.key === 'j' ? 1 : -1)))]?.focus()
    }
  }
  return (
    <section className="ap-view au-view" aria-label="Run history" data-testid="automation-runs">
      <CrumbHead
        items={[{ label: row?.name ?? '…', to: `automations/${id}` }, { label: 'Runs' }]}
        more={() => [
          { id: 'run', label: 'Run now', dimmed: !!row?.running, run: () => void useAutomations.getState().runNow(id) },
          { id: 'edit', label: 'Edit', run: () => navigate({ view: 'automations', rest: id }) },
        ]}
      />
      <MainBody>
        <div className="m-list" ref={listRef} onKeyDown={onKeyDown}>
          <GroupLabel>{`Run history · ${row?.name ?? ''}`}</GroupLabel>
          {runs?.map((run) => (
            <Row
              key={run.id}
              plain
              state={STATE[run.state] ?? 'finished'}
              stateLabel={run.state}
              name={runStartedLabel(run.started_at, tz)}
              detail={<RunDetailLine run={run} now={now} />}
              meta={agoLabel(run.started_at, now)}
              onClick={() => navigate({ view: 'automations', rest: `${id}/runs/${run.id}` })}
              onMenu={(e) => openMenu(e.currentTarget, runMenu(run), 'Run')}
              testId={`run-${run.id}`}
            />
          ))}
          {runs && !runs.length && (
            <div className="au-list-empty">
              {'No runs yet. '}
              <Button quiet onClick={() => void useAutomations.getState().runNow(id)}>
                Run now
              </Button>
            </div>
          )}
        </div>
      </MainBody>
    </section>
  )
}

function Result({ run }: { run: RunRow }) {
  const session = useSessionsStore((s) => (run.result_kind === 'session' ? s.rows.find((r) => r.id === run.result_id) : undefined))
  const route = runResultRoute(run)
  if (!run.result_kind) return null
  if (run.result_kind === 'session')
    return (
      <div className="au-result">
        <GroupLabel>Session</GroupLabel>
        <div className="m-list">
          <Row
            state={session ? ((session.state === 'unread' ? 'unread' : session.state === 'done' ? 'finished' : session.state) as 'finished') : 'idle'}
            plain
            name={session?.display_name ?? run.result_title ?? 'unavailable'}
            detail={session ? [session.repo_root?.split('/').pop(), session.git_branch].filter(Boolean).join(' · ') : 'The session was pruned.'}
            menu={false}
            onClick={route ? () => navigate(route) : undefined}
          />
        </div>
      </div>
    )
  return <NoteResult id={run.result_id} title={run.result_title ?? null} />
}

function NoteResult({ id, title }: { id: string | null; title: string | null }) {
  const text = useNoteText(id)
  return (
    <div className="au-result">
      <GroupLabel>{`Note · ${title ?? id ?? 'unavailable'}`}</GroupLabel>
      {text === null ? <div className="m-t-meta">Loading…</div> : text === '' ? <div className="m-t-meta">The note was removed.</div> : <pre className="au-note">{text}</pre>}
    </div>
  )
}

/** The created note's body, read-only (frontmatter left out); '' when it is gone. */
function useNoteText(id: string | null): string | null {
  const [text, setText] = useState<string | null>(null)
  useEffect(() => {
    if (!id) {
      setText('')
      return
    }
    let live = true
    invoke<{ text: string }>('note_get', { note_id: id }).then(
      (n) => live && setText(n?.text?.replace(/^---[\s\S]*?---\n+/, '') ?? ''),
      () => live && setText(''),
    )
    return () => {
      live = false
    }
  }, [id])
  return text
}

export function RunPage({ id, runId }: { id: string; runId: string }) {
  const now = useNow(1000)
  const detail = useAutomations((s) => (s.run?.run.id === runId ? s.run : null))
  const row = useAutomations((s) => s.rows.find((r) => r.id === id))
  useEffect(() => {
    void useAutomations.getState().loadRun(runId)
  }, [runId])
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented) navigate({ view: 'automations', rest: `${id}/runs` })
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [id])
  const run = detail?.run
  const auto = detail?.automation ?? row
  const tz = auto?.schedule.tz || deviceZone()
  const started = run ? runStartedLabel(run.started_at, tz) : '…'
  const fin = run?.finished_at ? wallOf(Date.parse(run.finished_at), tz) : null
  return (
    <section className="ap-view au-view" aria-label="Run" data-testid="automation-run">
      <CrumbHead items={[{ label: auto?.name ?? '…', to: `automations/${id}` }, { label: 'Runs', to: `automations/${id}/runs` }, { label: started }]} more={run ? () => runMenu(run) : undefined} />
      <MainBody>
        {run && auto ? (
          <div className="au-run">
            <div className="au-facts">
              <span className="m-fg-2">Trigger</span>
              <span>{run.trigger_kind === 'manual' ? 'Run now' : auto.trigger_phrase}</span>
              <span className="m-fg-2">Action</span>
              <span>{auto.action_phrase}</span>
              <span className="m-fg-2">Started</span>
              <span>{`${started} · ${agoLabel(run.started_at, now)}`}</span>
              <span className="m-fg-2">Finished</span>
              <span>
                {fin ? `${String(fin.hh).padStart(2, '0')}:${String(fin.mm).padStart(2, '0')} · ${runDetail(run, now).duration}` : run.state === 'running' ? runDetail(run, now).duration : '—'}
              </span>
              <span className="m-fg-2">State</span>
              <span>{run.state}</span>
              {run.error && (
                <>
                  <span className="m-fg-2">{run.state === 'skipped' ? 'Skip reason' : 'Error'}</span>
                  <span>{run.error}</span>
                </>
              )}
              <span className="m-fg-2">Automation</span>
              <span>{actionSummary(auto)[0]}</span>
            </div>
            <Result run={run} />
          </div>
        ) : (
          <div className="ap-skeleton" aria-busy="true" />
        )}
      </MainBody>
    </section>
  )
}
