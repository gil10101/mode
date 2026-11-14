// Settings → Advanced (03 §8.16–§8.19): Diagnostics (`diagnostics_get`:
// core status, hooks, IPC, index, database, backups), Logs (`logs_tail`),
// Export & Import (`settings_export` / `settings_import` with the merge
// preview) and About. `settings/advanced/<section>` scrolls to a section.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { Button } from '../../kit/button'
import { Seg, SegItem, Switch } from '../../kit/controls'
import { invoke } from '../../tauri'
import type { DbBackup, Diagnostics, ExportSection, ImportReport, LogsTail, ReindexResult, SweepResult, WrittenFile } from '../../state/contract-settings'
import { useSettingsStore, errorText } from '../../state/settings-store'
import { useCoreStore } from '../../state/store'
import { useSessionsStore } from '../../state/sessions-store'
import { HookDialog } from '../integrations/HookPreview'
import { useHooks } from '../integrations/hooks-store'
import { ago, normaliseHookStatus, tilde } from '../integrations/model'
import { copyText, useNow } from '../ui'
import { Block, Field, Group, Note, NullableSelect } from './controls'
import { bytes, filterLogs, LOG_COMPONENTS, LOG_LEVELS, parseLogLine, type LogComponent, type LogLevel } from './logs'
import { BACKEND_NAME } from './AiPage'
import { reindexText } from './NotesPage'

const notice = (t: string) => useSessionsStore.getState().showNotice(t)

export function AdvancedPage() {
  const flow = useHooks((s) => s.flow)
  return (
    <>
      <DiagnosticsSection />
      <LogsSection />
      <ExportSection />
      <AboutSection />
      {flow && <HookDialog onClose={() => useHooks.getState().close()} />}
    </>
  )
}

/* ---- diagnostics (03 §8.16) -------------------------------------------------------- */

const HOOK_WORD: Record<string, string> = { installed: 'installed', stale: 'needs updating', missing: 'removed', conflict: 'another hook is registered', none: 'not installed' }

/** "Sweep finished · 412 rows removed" from `retention_sweep` (`deleted`: rows per table, non-zero only). */
export function sweepText(r: SweepResult | null | undefined): string {
  const n = Object.values(r?.deleted ?? {}).reduce((a, b) => a + (Number(b) || 0), 0)
  const head = r?.dry_run ? 'Sweep preview' : 'Sweep finished'
  return n ? `${head} · ${n} ${n === 1 ? 'row' : 'rows'} ${r?.dry_run ? 'to remove' : 'removed'}` : `${head} · nothing to remove`
}

/** "Sessions" line of Diagnostics: tracked, started today, running in Mode's terminals. */
export function sessionsLine(d: Pick<Diagnostics, 'sessions_tracked' | 'sessions_today' | 'sessions_embedded'>): string {
  const parts = [`${d.sessions_tracked} tracked`, `${d.sessions_today} started today`]
  if (d.sessions_embedded) parts.push(`${d.sessions_embedded} in Mode's terminals`)
  return parts.join(' · ')
}

function DiagnosticsSection() {
  const now = useNow(5000)
  const [d, setD] = useState<Diagnostics | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [selfTest, setSelfTest] = useState<string | null>(null)
  const load = useCallback(() => {
    invoke<Diagnostics>('diagnostics_get').then(
      (x) => {
        setD(x)
        setError(null)
      },
      (e) => setError(errorText(e)),
    )
  }, [])
  useEffect(load, [load])
  const act = (cmd: string, done: (r: unknown) => string) =>
    void invoke(cmd).then(
      (r) => {
        notice(done(r))
        load()
      },
      (e) => notice(errorText(e)),
    )
  return (
    <div className="sg-section" data-testid="diagnostics">
      <Group title="Diagnostics" id="diagnostics">
        {error && <Note bad>{`Diagnostics could not be read: ${error}`}</Note>}
        {!d && !error && <Note>Reading…</Note>}
        {d && (
          <>
            <Field title="Core" desc={`Mode ${d.core.version}${d.core.dev_build ? ' (dev)' : ''} · ${d.core.hook_frames_received} hook events received`} />
            <Field title="Data folder" desc={<code className="sg-path">{tilde(d.core.data_dir)}</code>} />
            <Field title="Sessions" desc={<span data-testid="diag-sessions">{sessionsLine(d)}</span>} />
            {d.secrets_store && <Field title="Secrets" desc={d.secrets_store === 'os' ? 'The OS keyring.' : 'A development file store (no OS keyring here).'} />}
            <Field title="Export diagnostics" desc="One JSON file: logs, settings without secrets, table counts and hook status; no bodies, prompts or tokens.">
              <Button quiet onClick={load}>
                Refresh
              </Button>
              <Button onClick={() => act('diagnostics_export', (r) => ((r as WrittenFile | null)?.path ? `Diagnostics written to ${tilde((r as WrittenFile).path)}` : 'Export canceled'))}>Export diagnostics</Button>
            </Field>
          </>
        )}
      </Group>
      {d && (
        <>
          <Group title="Hooks" id="diag-hooks">
            {!d.claude.found && <Note>Claude Code not installed.</Note>}
            {d.hooks.length === 0 && <Note>No hook install recorded yet.</Note>}
            {d.hooks.map((h) => {
              const s = normaliseHookStatus(h.status)
              return (
                <Field
                  key={h.settings_path}
                  title={h.scope}
                  testId="diag-hook"
                  desc={
                    <>
                      <code className="sg-path">{tilde(h.settings_path)}</code>
                      {[` · ${HOOK_WORD[s]}`, h.relay_version ? ` · relay ${h.relay_version}` : '', h.verified_at ? ` · verified ${ago(h.verified_at, now) ?? ''}` : ''].join('')}
                    </>
                  }
                >
                  {s !== 'installed' && <Button onClick={() => void useHooks.getState().preview('install')}>{s === 'stale' ? 'Update hooks' : s === 'none' ? 'Install hooks' : 'Repair'}</Button>}
                </Field>
              )
            })}
            <Field
              title="Relay"
              desc={
                <>
                  <code className="sg-path">{tilde(d.relay.path) || 'not staged'}</code>
                  {d.relay.version ? ` · ${d.relay.version}` : ''}
                </>
              }
            />
            <Field
              title="Claude Code"
              desc={
                d.claude.found ? (
                  <>
                    <code className="sg-path">{tilde(d.claude.path)}</code>
                    {d.claude.version ? ` · ${d.claude.version}` : ''}
                  </>
                ) : (
                  'Not found.'
                )
              }
            />
          </Group>

          <Group title="IPC" id="diag-ipc">
            <Field
              title="Endpoint"
              desc={
                selfTest ? (
                  <span role="status" data-testid="self-test">
                    {selfTest}
                  </span>
                ) : (
                  <>
                    <code className="sg-path">{d.ipc.endpoint}</code>
                    {d.ipc.listening ? ' · listening' : ' · not listening'}
                  </>
                )
              }
            >
              <Button
                onClick={() =>
                  void invoke<{ latency_ms: number; acked: boolean }>('hooks_round_trip').then(
                    (r) => setSelfTest(r.acked ? `round trip ${r.latency_ms} ms` : 'no answer from the core'),
                    (e) => setSelfTest(errorText(e)),
                  )
                }
              >
                Self-test
              </Button>
            </Field>
            <Field title="Last events" stack>
              <div className="sg-events">
                {d.ipc.recent.length === 0 && <span className="m-set-desc">None yet.</span>}
                {d.ipc.recent.slice(0, 20).map((e, i) => (
                  <div key={i} className="m-stat-row">
                    <code>{e.event}</code>
                    <span className="m-t-meta">{`${ago(e.at, now) ?? ''}${e.latency_ms !== null ? ` · ${e.latency_ms} ms` : ''}`}</span>
                  </div>
                ))}
              </div>
            </Field>
          </Group>

          <Group title="Index" id="diag-index">
            <Field title="Notes folder" desc={<code className="sg-path">{tilde(d.index.vault_path)}</code>} />
            <Field title="Indexed" desc={`${d.index.files_indexed} files · last full scan ${ago(d.index.last_full_scan_at, now) ?? 'never'} · ${d.index.pending_touchups} touch-ups pending`}>
              <Button onClick={() => act('notes_reindex', (r) => reindexText(r as ReindexResult))}>Reindex</Button>
            </Field>
          </Group>

          <Group title="Database" id="diag-database">
            <Field
              title="File"
              desc={
                <>
                  <code className="sg-path">{tilde(d.database.path)}</code>
                  {` · ${bytes(d.database.size_bytes)} · WAL ${bytes(d.database.wal_bytes)}`}
                </>
              }
            />
            <Field title="Rows" desc={d.database.tables.map((t) => `${t.name} ${t.rows}`).join(' · ') || '—'} />
            {d.database.read_only && <Note bad>This database belongs to a newer Mode: nothing is written until Mode is updated.</Note>}
            <Field title="Sweep" desc={`Last run ${ago(d.database.last_sweep_at, now) ?? 'never'}.`}>
              <Button disabled={!!d.database.read_only} onClick={() => act('retention_sweep', (r) => sweepText(r as SweepResult))}>
                Run sweep now
              </Button>
              <Button onClick={() => act('db_backup', (r) => `Backup written to ${tilde((r as DbBackup | null)?.path ?? '')}`)}>Backup now</Button>
            </Field>
            <Field title="Backups" stack>
              <div className="sg-events" data-testid="backups">
                {d.database.backups.length === 0 && <span className="m-set-desc">None yet.</span>}
                {d.database.backups.map((b) => (
                  <div key={b.path} className="m-stat-row">
                    <code className="sg-path">{tilde(b.path)}</code>
                    <span className="m-t-meta">{`${bytes(b.size_bytes)} · ${ago(b.created_at, now) ?? ''}`}</span>
                  </div>
                ))}
              </div>
            </Field>
          </Group>
        </>
      )}
    </div>
  )
}

/* ---- logs (03 §8.17) ---------------------------------------------------------------- */

const TAIL = 2000

function LogsSection() {
  const [tail, setTail] = useState<LogsTail | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [level, setLevel] = useState<LogLevel>('info')
  const [component, setComponent] = useState<LogComponent | 'all'>('all')
  const [text, setText] = useState('')
  const [follow, setFollow] = useState(true)
  const load = useCallback(() => {
    invoke<LogsTail>('logs_tail', { lines: TAIL }).then(
      (t) => {
        setTail(t)
        setError(null)
      },
      (e) => setError(errorText(e)),
    )
  }, [])
  useEffect(load, [load])
  useEffect(() => {
    if (!follow) return
    const t = window.setInterval(load, 2000)
    return () => window.clearInterval(t)
  }, [follow, load])
  const parsed = useMemo(() => (tail?.lines ?? []).map(parseLogLine), [tail])
  const shown = useMemo(() => filterLogs(parsed, { level, component, text }), [parsed, level, component, text])
  return (
    <Group title="Logs" id="logs">
      <Block testId="logs">
      <div className="m-inline sg-log-tools">
        <Seg label="Level">
          {LOG_LEVELS.map((l) => (
            <SegItem key={l} active={level === l} onClick={() => setLevel(l)}>
              {l}
            </SegItem>
          ))}
        </Seg>
        <NullableSelect label="Component" value={component} options={[['all', 'all components'] as const, ...LOG_COMPONENTS.map((c) => [c, c] as const)]} onPick={setComponent} />
        <input className="m-input" placeholder="Filter lines…" aria-label="Filter log lines" value={text} onChange={(e) => setText(e.target.value)} />
        <span className="m-set-unit">Follow</span>
        <Switch on={follow} label="Follow" onChange={setFollow} />
        <Button onClick={() => copyText(shown.map((l) => l.raw).join('\n'), 'Lines copied')}>Copy</Button>
        <Button
          onClick={() => {
            const dir = tail?.path.replace(/[\\/][^\\/]*$/, '') ?? ''
            void invoke('plugin:opener|open_path', { path: dir }).catch((e) => notice(`Couldn't open the logs folder: ${errorText(e)}`))
          }}
          disabled={!tail}
        >
          Open logs folder
        </Button>
      </div>
      {error && <Note bad>{`Logs could not be read: ${error}`}</Note>}
      <pre className="sg-log" aria-label="Log lines" data-testid="log-lines">
        {shown.length === 0 ? (
          <span className="m-t-meta">{tail ? 'No lines match.' : 'Reading…'}</span>
        ) : (
          shown.map((l, i) => (
            <div key={i} className={`sg-log-line is-${l.level}`}>
              {l.raw}
            </div>
          ))
        )}
      </pre>
      <div className="m-set-desc">{tail ? `${shown.length} of ${parsed.length} lines · ${tilde(tail.path)} · never message bodies or secrets` : ''}</div>
      </Block>
    </Group>
  )
}

/* ---- export & import (03 §8.18) ------------------------------------------------------- */

const SECTIONS: readonly [ExportSection, string, string][] = [
  ['settings', 'Settings', 'your user settings (never keys or tokens)'],
  ['groupings', 'Groupings', 'custom groupings and statuses'],
  ['alert_rules', 'Alert rules', 'from v0.2'],
  ['contacts', 'Contacts & important people', 'from v0.2'],
  ['themes', 'Themes', 'custom and imported packs, from v1.0'],
]

function ExportSection() {
  const [picked, setPicked] = useState<Record<ExportSection, boolean>>({ settings: true, groupings: true, alert_rules: false, contacts: false, themes: false })
  const [report, setReport] = useState<ImportReport | null>(null)
  const [busy, setBusy] = useState(false)
  const doExport = async () => {
    setBusy(true)
    try {
      const r = await invoke<WrittenFile>('settings_export', { sections: SECTIONS.map(([s]) => s).filter((s) => picked[s]), include_device: false })
      notice(r?.path ? `Exported to ${tilde(r.path)}` : 'Export canceled')
    } catch (e) {
      notice(`Export failed: ${errorText(e)}`)
    } finally {
      setBusy(false)
    }
  }
  const preview = async () => {
    try {
      const r = await invoke<ImportReport>('settings_import', { path: null, apply: false })
      // `path: null`: the open dialog was canceled.
      if (r?.path) setReport(r)
    } catch (e) {
      notice(`Could not read that file: ${errorText(e)}`)
    }
  }
  const apply = async () => {
    if (!report) return
    try {
      const r = await invoke<ImportReport>('settings_import', { path: report.path ?? null, apply: true })
      const changed = Object.values(r?.entries ?? {}).filter((o) => o.outcome === 'new' || o.outcome === 'updated').length
      setReport(null)
      notice(`Imported · ${changed} ${changed === 1 ? 'change' : 'changes'}`)
      await useSettingsStore.getState().load()
    } catch (e) {
      notice(`Import failed: ${errorText(e)}`)
    }
  }
  const entries = report ? Object.entries(report.entries) : []
  return (
    <div className="sg-section" data-testid="export-import">
      <Group title="Export & Import" id="export">
        {SECTIONS.map(([sec, label, hint]) => (
          <Field key={sec} title={label} desc={hint}>
            <Switch on={picked[sec]} label={`Export ${label}`} onChange={(on) => setPicked((p) => ({ ...p, [sec]: on }))} />
          </Field>
        ))}
        <Field title="File" desc="Writes mode-export-<date>.json; back up the notes folder separately.">
          <Button onClick={() => void preview()}>Import…</Button>
          <Button disabled={busy || !Object.values(picked).some(Boolean)} onClick={() => void doExport()}>
            Export…
          </Button>
        </Field>
      </Group>
      {report && (
        <div className="sg-import" data-testid="import-preview">
          <div className="m-set-head">{`Import preview${report.path ? ` · ${tilde(report.path)}` : ''}`}</div>
          {entries.length === 0 && <Note>Nothing to import.</Note>}
          {entries.map(([key, o]) => (
            <div key={key} className="m-stat-row">
              <code>{key}</code>
              <span className="m-t-meta">{o.outcome === 'skipped' ? `skipped · ${o.reason}` : o.outcome}</span>
            </div>
          ))}
          <div className="m-card-actions">
            <Button onClick={() => setReport(null)}>Cancel</Button>
            <Button variant="primary" disabled={!entries.some(([, o]) => o.outcome === 'new' || o.outcome === 'updated')} onClick={() => void apply()}>
              Apply
            </Button>
          </div>
        </div>
      )}
    </div>
  )
}

/* ---- about (03 §8.19) ------------------------------------------------------------------- */

function AboutSection() {
  const status = useCoreStore((s) => s.status)
  const backend = useSettingsStore((s) => s.values['ai.backend']) as keyof typeof BACKEND_NAME | undefined
  return (
    <div className="sg-section" data-testid="about">
      <Group title="About" id="about">
        <Field title="Version" desc={status ? `Mode ${status.version}${status.dev_build ? ' · development build' : ''}` : 'Mode'} />
        <Field title="Channel" desc={`${status?.dev_build ? 'development' : 'stable'} · updates arrive with v1.0`} />
        <Field title="License" desc="private now, MIT planned" />
        <Field title="AI" desc={backend === 'claude_api' ? 'Claude API by Anthropic, with your own key' : backend ? `${BACKEND_NAME[backend]} on this machine` : 'no backend'} />
        <Note>Claude, Claude Code, Gmail, Slack, Google Calendar, Google Meet, Zoom, Microsoft Teams, Outlook, GitHub, Linear, Notion, Ollama, LM Studio and the terminal apps are trademarks of their owners; their marks identify the apps and services Mode connects to.</Note>
      </Group>
    </div>
  )
}
