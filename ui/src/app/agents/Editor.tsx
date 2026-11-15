// The agent edit page (03 §7E.2), a full page by the drill-in rule and the
// Settings pattern: the head (`‹`, the breadcrumb, "Unsaved", Save as the one
// primary and `⋮`), the page head (the mark with its pencil badge, name and
// title, the status line), then the tabs Identity · Brain · Knows · Does ·
// Rules · Advanced, each groups of cards and rows (10 §7.4.19) in the
// centered 680-px column. Lists of short
// values (backup models, tables, tool rules, extra arguments) are chips
// fields; memories and rules are rows; skills and connectors watched-rows
// grids; usage stat tiles. Does: usage, automations, repos, connectors.
// Rules: permissions, the free-text rules, Use memories, notifications.
// Keys (03 §10.8A; printed only in the Shortcuts view): ⌘/Ctrl+S save, Esc leave.
import { useEffect, useMemo, useRef, useState, type ReactElement } from 'react'
import { AgentAvatar, EditableAvatar } from '../../kit/avatars'
import { Button, MenuButton } from '../../kit/button'
import { Chip, ChipsField, Combo, Notice, Select, SwatchGrid, Switch, Tab, Tabs, TextArea } from '../../kit/controls'
import { MainBody } from '../../kit/layout'
import { Row } from '../../kit/lists'
import { AVATAR_VARIANTS } from '../../kit/mascot'
import { Stat } from '../../kit/stats'
import { joinDot } from '../../kit/text'
import type { StateKind } from '../../kit/types'
import { ModelPicker, SettingError, SettingGroup, SettingRow, useModelAdder } from '../../kit/settings'
import { SourceHead, SourceRow } from '../../kit/views'
import { EFFORTS, MAX_FALLBACKS, MAX_RULE, MODEL_ALIASES, type AutomationUse, type Effort, type MemoryRow } from '../../state/contract-agents'
import type { PermissionMode } from '../../state/contract-automations'
import { agentHue, compactTokens, useAgents } from '../../state/agents-store'
import { agoLabel } from '../../state/automations-store'
import { useSessionsStore } from '../../state/sessions-store'
import { PERMISSION_LABEL } from '../automations/Editor'
import { useDialogs } from '../sessions/actions'
import { providerHost, sessionProviders, useAi } from '../settings/ai'
import { navigate } from '../router'
import { Block, Note } from '../settings/controls'
import { Dialog, openMenu, openPopover, useNow, type MenuEntry } from '../ui'
import { confirmDelete } from './AgentsList'

export type AgentTab = 'identity' | 'brain' | 'knows' | 'does' | 'rules' | 'advanced'
export const AGENT_TABS: readonly [AgentTab, string][] = [
  ['identity', 'Identity'],
  ['brain', 'Brain'],
  ['knows', 'Knows'],
  ['does', 'Does'],
  ['rules', 'Rules'],
  ['advanced', 'Advanced'],
]
const EFFORT_LABEL: Record<Effort, string> = { low: 'Low', medium: 'Medium', high: 'High', xhigh: 'Extra high', max: 'Max' }
const VARIANT_LABEL: Record<string, string> = { '': 'Plain', beret: 'Beret', specs: 'Specs', tophat: 'Top hat', mustache: 'Mustache', beanie: 'Beanie', cap: 'Cap', squares: 'Squares', monocle: 'Monocle', crown: 'Crown', headphones: 'Headphones', ears: 'Cat ears', wizard: 'Wizard hat', patch: 'Eye patch', chef: 'Chef hat' }

function tilde(p: string): string {
  return p.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~')
}
function basename(p: string): string {
  return (
    p
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() || p
  )
}

function MemoryDialog({ memory, onClose }: { memory: MemoryRow | null; onClose: () => void }) {
  const [title, setTitle] = useState(memory?.title ?? '')
  const [body, setBody] = useState(memory?.body ?? '')
  const [error, setError] = useState<string | null>(null)
  const save = async () => {
    if (!title.trim()) {
      setError('Give the memory a title.')
      return
    }
    const row = await useAgents.getState().memorySave(title.trim(), body, memory?.id)
    if (row) onClose()
    else setError(useAgents.getState().error ?? 'The memory was not saved.')
  }
  return (
    <Dialog title={memory ? 'Edit memory' : 'New memory'} onClose={onClose} testId="agent-memory">
      <form
        onSubmit={(e) => {
          e.preventDefault()
          void save()
        }}
      >
        <SettingGroup compact>
          <SettingRow title="Title" htmlFor="memory-title" error={error ? <SettingError>{error}</SettingError> : undefined}>
            <input id="memory-title" className="m-input" value={title} onChange={(e) => (setTitle(e.target.value), setError(null))} autoFocus />
          </SettingRow>
          <SettingRow title="Remember" desc="Read by the agent at every start, after its instructions." stack htmlFor="memory-body">
            <TextArea id="memory-body" aria-label="Memory" value={body} onChange={(e) => setBody(e.target.value)} />
          </SettingRow>
        </SettingGroup>
      </form>
      <div className="m-card-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => void save()}>
          Save
        </Button>
      </div>
    </Dialog>
  )
}

/** The free-text rules: one row each, an input that adds on Enter (a comma does not split a rule). */
function RulesField({ values, onChange }: { values: string[]; onChange: (v: string[]) => void }) {
  const [text, setText] = useState('')
  const add = () => {
    const v = text.trim()
    if (!v || v.length > MAX_RULE) return
    setText('')
    if (!values.includes(v)) onChange([...values, v])
  }
  return (
    <div className="ag-rules">
      {values.length > 0 && (
        <div className="m-list" role="list" aria-label="Rules">
          {values.map((r, i) => (
            <div key={`${r}-${i}`} className="m-row is-plain ag-rule" role="listitem">
              <div className="m-row-text">
                <div className="m-row-name">{r}</div>
              </div>
              <Button variant="icon" quiet aria-label={`Remove rule ${r}`} onClick={() => onChange(values.filter((_, j) => j !== i))}>
                ×
              </Button>
            </div>
          ))}
        </div>
      )}
      <input
        className="m-input"
        aria-label="Add a rule"
        placeholder="Never edit files; report only · Enter adds"
        value={text}
        maxLength={MAX_RULE}
        onChange={(e) => setText(e.target.value)}
        onBlur={add}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            add()
          }
        }}
      />
    </div>
  )
}

/**
 * 03 §7E.2 Backup models: the chips in order (each with its ×), then an "Add backup" picker of the
 * known models not yet used (the primary and the listed ones left out) and "Other…" (the Model id
 * row, Enter adds); disabled at three. A chip added here rises in.
 */
function BackupModels({ values, primary, models, error, onChange }: { values: string[]; primary: string | null; models: readonly string[]; error?: ReactElement; onChange: (v: string[]) => void }) {
  const [added, setAdded] = useState<string[]>([])
  const full = values.length >= MAX_FALLBACKS
  const add = useModelAdder({
    id: 'ag-backup',
    label: 'Add backup',
    models: models.filter((m) => m !== primary && !values.includes(m)),
    disabled: full,
    onAdd: (m) => {
      if (full || values.includes(m)) return
      setAdded((a) => [...a, m])
      onChange([...values, m])
    },
  })
  return (
    <>
      <SettingRow title="Backup models" desc="Up to three, tried in order." stack error={error}>
        {values.length > 0 && (
          <div className="m-inline">
            {values.map((m, i) => (
              <Chip key={m} removable className={added.includes(m) ? 'is-entering' : undefined} onRemove={() => onChange(values.filter((_, j) => j !== i))}>
                {m}
              </Chip>
            ))}
          </div>
        )}
        {add.select}
      </SettingRow>
      {add.idRow}
    </>
  )
}

function runState(a: AutomationUse): { state: StateKind; label: string } {
  const s = a.last_state
  if (s === 'running') return { state: 'working', label: 'running' }
  if (s === 'finished') return { state: 'finished', label: 'finished' }
  if (s === 'failed') return { state: 'failed', label: 'failed' }
  if (s === 'skipped') return { state: 'idle', label: 'skipped' }
  return { state: 'idle', label: 'never ran' }
}

export function Editor({ id, tab }: { id: string; tab: AgentTab }) {
  const now = useNow(30_000)
  const editor = useAgents((s) => s.editor)
  const loaded = useAgents((s) => s.loaded)
  const dirty = useAgents((s) => s.dirty())
  const skillsByWs = useAgents((s) => s.skills)
  const mcpByFolders = useAgents((s) => s.mcp)
  const providerRows = useAi((s) => s.providers)
  const providers = useMemo(() => sessionProviders(providerRows), [providerRows])
  const summary = useAgents((s) => s.rows.find((r) => r.id === id) ?? null)
  const claudeMissing = useSessionsStore((s) => s.health?.claude_code === 'missing')
  const [leave, setLeave] = useState<string | null>(null)
  const [missing, setMissing] = useState(false)
  const [memory, setMemory] = useState<MemoryRow | null | 'new'>(null)
  const nameRef = useRef<HTMLInputElement>(null)
  const isNew = id === 'new'

  useEffect(() => {
    const s = useAgents.getState()
    if (isNew) {
      if (!s.editor || s.editor.draft.id) s.edit(null)
      setTimeout(() => nameRef.current?.focus(), 0)
    } else if (s.editor?.view?.id !== id) void s.open(id).then((ok) => setMissing(!ok))
  }, [id, isNew])

  useEffect(() => () => useAgents.getState().closeEditor(), [])
  useEffect(() => {
    if (!useAi.getState().providers) void useAi.getState().loadProviders()
  }, [])

  const workspace = editor?.draft.workspace ?? null
  useEffect(() => {
    if (editor) void useAgents.getState().loadSkills(workspace)
  }, [workspace, !!editor])
  // The MCP servers Claude Code has for the agent's folders (its repos, its workspace) and the user.
  const mcpFolders = [...(editor?.draft.repos ?? []), ...(workspace ? [workspace] : [])]
  const mcpKey = mcpFolders.join('\n')
  useEffect(() => {
    if (editor) void useAgents.getState().loadMcp(mcpKey ? mcpKey.split('\n') : [])
  }, [mcpKey, !!editor])

  const go = (to: string) => {
    if (useAgents.getState().dirty()) setLeave(to)
    else navigate(to)
  }
  const save = async (): Promise<string | null> => {
    const saved = await useAgents.getState().save()
    if (saved && isNew) navigate({ view: 'agents', rest: `${saved.id}/${tab}` }, { replace: true })
    return saved?.id ?? null
  }
  const showTab = (t: AgentTab) => navigate({ view: 'agents', rest: `${id}/${t}` }, { replace: true })

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (mod && !e.shiftKey && e.key.toLowerCase() === 's') {
        e.preventDefault()
        void save()
      } else if (e.key === 'Escape' && !e.defaultPrevented && !document.querySelector('.ap-scrim')) {
        const t = e.target as HTMLElement | null
        if (t?.tagName === 'INPUT' || t?.tagName === 'TEXTAREA') {
          t.blur()
          return
        }
        e.preventDefault()
        go('agents')
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  if (!editor) {
    return (
      <section className="ap-view ag-view" aria-label="Agent">
        <div className="m-main-head">
          <Button variant="icon" quiet aria-label="Back" onClick={() => navigate('agents')}>
            ‹
          </Button>
          <nav className="m-crumb ag-crumb" aria-label="Breadcrumb">
            <span aria-current="page">{missing ? 'Unavailable' : '…'}</span>
          </nav>
        </div>
        <MainBody>{loaded && !isNew && missing && <div className="ap-empty">This agent no longer exists.</div>}</MainBody>
      </section>
    )
  }

  const d = editor.draft
  const view = editor.view
  const s = useAgents.getState()
  const err = (field: string) => editor.errors.find((e) => e.field === field)?.message
  const formError = err('form')
  const installed = skillsByWs[workspace ?? ''] ?? []
  const skillOn = (name: string) => d.skills.length === 0 || d.skills.includes(name)
  const toggleSkill = (name: string, on: boolean) => {
    const current = d.skills.length ? d.skills : installed
    s.patchDraft({ skills: on ? Array.from(new Set([...current, name])) : current.filter((x) => x !== name) })
  }
  const configured = mcpByFolders[mcpKey] ?? []
  const servers = [
    ...configured.map((m) => ({ name: m.name, note: m.scope === 'local' ? 'this repo' : m.scope === 'project' ? '.mcp.json' : 'user' })),
    ...d.connectors.filter((n) => !configured.some((m) => m.name === n)).map((n) => ({ name: n, note: 'no longer configured' })),
  ]
  const toggleConnector = (name: string, on: boolean) => s.patchDraft({ connectors: on ? Array.from(new Set([...d.connectors, name])) : d.connectors.filter((x) => x !== name) })
  const provider = d.provider_id ? (providers.find((p) => p.id === d.provider_id) ?? null) : null
  const providerNote = !d.provider_id
    ? 'The CLI signs in on its own.'
    : provider
      ? `Sessions run through ${provider.name}${providerHost(provider.url) ? ` at ${providerHost(provider.url)}` : ''}${provider.key_stored ? '.' : ' · no key stored.'}`
      : 'This provider is gone or turned off.'
  const modelOptions = provider?.models.length ? Array.from(new Set([...provider.models, ...MODEL_ALIASES])) : MODEL_ALIASES
  const name = d.name.trim() || (isNew ? 'New agent' : (view?.name ?? '…'))
  const status = view
    ? [
        summary && summary.live > 0 ? `${summary.live} live` : 'idle',
        view.last_used_at ? `last used ${agoLabel(view.last_used_at, now)}` : 'never used',
        view.automations.length ? 'automated' : null,
        d.notify ? null : 'muted',
      ]
        .filter(Boolean)
        .join(' · ')
    : 'not saved yet'
  const more = (): MenuEntry[] => [
    {
      id: 'launch',
      label: 'Launch session',
      dimmed: isNew || claudeMissing,
      reason: claudeMissing ? 'Claude Code was not found' : isNew ? 'Save the agent first' : undefined,
      run: () => view && useDialogs.getState().openLaunch(view.id),
    },
    { kind: 'sep', id: 'sep' },
    { id: 'delete', label: 'Delete', danger: true, dimmed: isNew, run: () => view && confirmDelete({ id: view.id, name: view.name, automated: view.automations.length > 0 }, () => navigate('agents')) },
  ]

  const fail = (field: string) => {
    const e = err(field)
    return e ? <SettingError>{e}</SettingError> : undefined
  }

  const body =
    tab === 'identity' ? (
      <SettingGroup title="About this agent">
        <SettingRow title="Name" htmlFor="ag-name" error={fail('name')}>
          <input id="ag-name" ref={nameRef} className="m-input" aria-label="Name" value={d.name} onChange={(e) => s.patchDraft({ name: e.target.value })} />
        </SettingRow>
        <SettingRow title="Title" desc="One line under the name." htmlFor="ag-title" error={fail('title')}>
          <input id="ag-title" className="m-input" aria-label="Title" placeholder="Reviews pull requests" value={d.title ?? ''} onChange={(e) => s.patchDraft({ title: e.target.value || null })} />
        </SettingRow>
        <SettingRow title="Instructions" desc="What this agent is for and how it works; added to every session's system prompt." stack htmlFor="ag-instructions" error={fail('instructions')}>
          <TextArea id="ag-instructions" aria-label="Instructions" value={d.instructions ?? ''} onChange={(e) => s.patchDraft({ instructions: e.target.value || null })} />
        </SettingRow>
      </SettingGroup>
    ) : tab === 'brain' ? (
      <>
        <SettingGroup title="Model and reasoning">
          <SettingRow title="Provider" desc={providerNote} error={fail('provider_id')}>
            <Select
              label="Provider"
              value={d.provider_id ?? ''}
              options={[['', "Claude Code's own sign-in"] as const, ...providers.map((p) => [p.id, p.name] as const)]}
              placeholder={d.provider_id ?? undefined}
              onChange={(v) => s.patchDraft({ provider_id: v || null })}
            />
          </SettingRow>
          <ModelPicker
            id="ag-model"
            desc="The CLI's default unless you pick one; Other… takes any id."
            defaultLabel="CLI default"
            models={modelOptions}
            value={d.model}
            onChange={(v) => s.patchDraft({ model: v })}
            error={fail('model')}
          />
          <SettingRow title="Reasoning" error={fail('effort')}>
            <Select
              label="Reasoning effort"
              value={d.effort ?? ''}
              options={[['', "The CLI's default"] as const, ...EFFORTS.map((e) => [e, EFFORT_LABEL[e]] as const)]}
              onChange={(v) => s.patchDraft({ effort: (v || null) as Effort | null })}
            />
          </SettingRow>
        </SettingGroup>
        <SettingGroup title="Backups">
          <BackupModels values={d.fallback_models} primary={d.model} models={modelOptions} error={fail('fallback_models')} onChange={(v) => s.patchDraft({ fallback_models: v })} />
          <SettingRow
            title="Ask before switching"
            desc={d.ask_before_switching ? 'Mode offers a backup when a session fails; the CLI never switches on its own.' : 'The CLI switches to the first available backup and says so.'}
          >
            <Switch on={d.ask_before_switching} label="Ask before switching" onChange={(v) => s.patchDraft({ ask_before_switching: v })} />
          </SettingRow>
        </SettingGroup>
      </>
    ) : tab === 'knows' ? (
      <>
        <SettingGroup title="Memories">
          {view && view.memories.length > 0 && (
            <Block>
              <div className="m-list ag-list" role="list" aria-label="Memories">
                {view.memories.map((m) => (
                  <div key={m.id} className="m-row is-plain ag-memory" role="listitem">
                    <span className="m-t-meta">{agoLabel(m.updated_at, now)}</span>
                    <div className="m-row-text">
                      <div className="m-row-name">{m.title}</div>
                      <div className="m-row-detail">{m.body.split('\n')[0] ?? ''}</div>
                    </div>
                    <span />
                    <MenuButton
                      label={`${m.title} options`}
                      onClick={(e) =>
                        openMenu(
                          e.currentTarget,
                          [
                            { id: 'edit', label: 'Edit', run: () => setMemory(m) },
                            { kind: 'sep', id: 'sep' },
                            { id: 'delete', label: 'Delete', danger: true, run: () => void s.memoryDelete(m.id) },
                          ],
                          m.title,
                        )
                      }
                    />
                  </div>
                ))}
              </div>
            </Block>
          )}
          <SettingRow title="Add memory" desc={view ? 'Read at every start, after the instructions.' : 'Save the agent first.'}>
            <Button disabled={!view} onClick={() => setMemory('new')}>
              Add memory
            </Button>
          </SettingRow>
        </SettingGroup>
        <SettingGroup title="Skills">
          {installed.length ? (
            <Block>
              <div className="ag-skills">
                <SourceHead labels={['On']} />
                {installed.map((name) => (
                  <SourceRow key={name} name={name} cells={[<Switch key="on" on={skillOn(name)} label={`Skill ${name}`} onChange={(v) => toggleSkill(name, v)} />]} />
                ))}
              </div>
            </Block>
          ) : (
            <Note>{workspace ? 'No skills installed for this workspace or in ~/.claude/skills.' : 'No skills in ~/.claude/skills; set a workspace under Advanced to see its own.'}</Note>
          )}
        </SettingGroup>
        <SettingGroup title="Tables">
          <SettingRow title="Folders and notes" desc="What the agent may read, besides its repos." stack error={fail('tables')}>
            <ChipsField label="Tables" values={d.tables} placeholder="~/notes/Projects" render={basename} title={(v) => tilde(v)} onChange={(v) => s.patchDraft({ tables: v })} />
          </SettingRow>
        </SettingGroup>
      </>
    ) : tab === 'does' ? (
      <>
        <SettingGroup title="Usage">
          {view ? (
            <Block>
              <div className="m-stats" data-testid="agent-usage">
                <Stat value={String(view.usage.sessions)} label="sessions" />
                <Stat value={String(view.usage.sessions_7d)} label="this week" />
                <Stat value={compactTokens(view.usage.tokens)} label="tokens" />
                <Stat value={compactTokens(view.usage.tokens_7d)} label="tokens this week" />
                <Stat value={String(view.usage.turns)} label="turns" />
              </div>
            </Block>
          ) : (
            <Note>Nothing yet: the agent has not been saved.</Note>
          )}
        </SettingGroup>
        <SettingGroup title="Automations">
          {view && view.automations.length > 0 ? (
            <Block>
              <div className="m-list ag-list" role="list" aria-label="Automations">
                {view.automations.map((a) => {
                  const st = runState(a)
                  return (
                    <Row
                      key={a.id}
                      plain
                      state={st.state}
                      stateLabel={st.label}
                      name={a.name}
                      detail={joinDot(a.enabled ? 'on' : 'off', a.last_run_at ? `last run ${agoLabel(a.last_run_at, now)}` : null)}
                      menu={false}
                      onClick={() => navigate({ view: 'automations', rest: a.id })}
                    />
                  )
                })}
              </div>
            </Block>
          ) : (
            <Note>No automation runs this agent.</Note>
          )}
        </SettingGroup>
        <SettingGroup title="Repos">
          <SettingRow title="Folders" desc="The first is where a launch with no folder starts." stack error={fail('repos')}>
            <ChipsField label="Repos" values={d.repos} placeholder="~/code/mode" render={basename} title={(v) => tilde(v)} onChange={(v) => s.patchDraft({ repos: v })} />
          </SettingRow>
        </SettingGroup>
        <SettingGroup title="Connectors">
          {servers.length ? (
            <Block>
              <div className="ag-skills" data-testid="agent-connectors">
                <SourceHead labels={['On']} />
                {servers.map((m) => (
                  <SourceRow key={m.name} name={m.name} note={m.note} cells={[<Switch key="on" on={d.connectors.includes(m.name)} label={`Connector ${m.name}`} onChange={(v) => toggleConnector(m.name, v)} />]} />
                ))}
              </div>
            </Block>
          ) : (
            <Note>No MCP servers configured in Claude Code (claude mcp add).</Note>
          )}
          {err('connectors') && <Note bad>{err('connectors')}</Note>}
          <Note>{d.connectors.length ? 'Its sessions load only these MCP servers.' : 'None on: its sessions load every MCP server Claude Code has.'}</Note>
        </SettingGroup>
      </>
    ) : tab === 'rules' ? (
      <>
        <SettingGroup title="Permissions">
          <SettingRow title="Mode" desc="How its sessions ask before acting." error={fail('permission_mode')}>
            <Select
              label="Permission mode"
              value={d.permission_mode ?? ''}
              options={[['', "The launch's choice"] as const, ...(Object.keys(PERMISSION_LABEL) as PermissionMode[]).map((k) => [k, PERMISSION_LABEL[k]] as const)]}
              onChange={(v) => s.patchDraft({ permission_mode: (v || null) as PermissionMode | null })}
            />
          </SettingRow>
          <SettingRow title="Allowed tools" desc="As --allowedTools takes them." stack error={fail('allowed_tools')}>
            <ChipsField label="Allowed tools" values={d.allowed_tools} placeholder="Read, Bash(git diff *)" onChange={(v) => s.patchDraft({ allowed_tools: v })} />
          </SettingRow>
          <SettingRow title="Denied tools" stack error={fail('denied_tools')}>
            <ChipsField label="Denied tools" values={d.denied_tools} placeholder="WebSearch" onChange={(v) => s.patchDraft({ denied_tools: v })} />
          </SettingRow>
        </SettingGroup>
        <SettingGroup title="Written rules">
          <SettingRow title="Rules" desc="One per line, added after the instructions; Enter adds." stack error={fail('rules')}>
            <RulesField values={d.rules} onChange={(v) => s.patchDraft({ rules: v })} />
          </SettingRow>
        </SettingGroup>
        <SettingGroup title="Memory and alerts">
          <SettingRow title="Use memories" desc={d.use_memories ? 'Its memories are read at every start, after the instructions and rules.' : 'Its memories stay out of the system prompt.'}>
            <Switch on={d.use_memories} label="Use memories" onChange={(v) => s.patchDraft({ use_memories: v })} />
          </SettingRow>
          <SettingRow title="Alerts" desc={d.notify ? 'Its sessions raise alerts like any other.' : 'Its sessions raise no alerts; they still show in the roster.'}>
            <Switch on={d.notify} label="Alerts for this agent" onChange={(v) => s.patchDraft({ notify: v })} />
          </SettingRow>
        </SettingGroup>
      </>
    ) : (
      <>
        <SettingGroup title="Launch">
          <SettingRow title="Workspace" desc="Empty and the launch dialog asks." htmlFor="ag-workspace" error={fail('workspace')}>
            <Combo
              id="ag-workspace"
              label="Workspace"
              placeholder="the launch dialog asks"
              value={d.workspace ?? ''}
              suggestions={d.repos}
              render={(v) => tilde(v)}
              onChange={(v) => s.patchDraft({ workspace: v.trim() || null })}
            />
          </SettingRow>
          <SettingRow title="Path to claude" desc="Empty for the Claude Code card's." htmlFor="ag-claude" error={fail('claude_path')}>
            <input id="ag-claude" className="m-input" aria-label="Path to claude" value={d.claude_path ?? ''} onChange={(e) => s.patchDraft({ claude_path: e.target.value || null })} />
          </SettingRow>
          <SettingRow title="Extra arguments" desc="Added to the command as given." stack error={fail('extra_args')}>
            <ChipsField label="Extra arguments" values={d.extra_args} placeholder="--verbose" onChange={(v) => s.patchDraft({ extra_args: v })} />
          </SettingRow>
        </SettingGroup>
        <SettingGroup title="Isolation">
          <SettingRow title="Own config folder" desc={d.isolate_config ? 'Its own memory, transcripts and settings (CLAUDE_CONFIG_DIR).' : 'Shares ~/.claude with every other session.'}>
            <Switch on={d.isolate_config} label="Own config folder" onChange={(v) => s.patchDraft({ isolate_config: v })} />
          </SettingRow>
        </SettingGroup>
      </>
    )

  return (
    <section className="ap-view ag-view" aria-label={name} data-testid="agent-editor">
      <div className="m-main-head">
        <Button variant="icon" quiet aria-label="Back" onClick={() => go('agents')}>
          ‹
        </Button>
        <nav className="m-crumb ag-crumb" aria-label="Breadcrumb">
          <span aria-current="page">{name}</span>
        </nav>
        {dirty && <span className="m-t-meta ag-dirty">Unsaved</span>}
        <Button variant="primary" disabled={editor.saving} onClick={() => void save()}>
          Save
        </Button>
        <MenuButton label="More" onClick={(e) => openMenu(e.currentTarget, more(), 'Agent')} />
      </div>
      <div className="ag-page-head" data-testid="agent-page-head">
        <EditableAvatar
          size={40}
          hue={agentHue(view?.id ?? d.name)}
          variant={d.avatar_variant}
          label="Change the mark"
          onEdit={(e) =>
            // 03 §7E.2: the accessory swatch grid, the 40-px mark wearing each; the names are labels and tooltips only.
            openPopover(
              e.currentTarget,
              (close) => (
                <SwatchGrid
                  label="Accessory"
                  testId="accessory-grid"
                  value={d.avatar_variant || 'plain'}
                  onClose={close}
                  onPick={(id) => {
                    s.patchDraft({ avatar_variant: id === 'plain' ? '' : id })
                    close()
                  }}
                  options={['', ...AVATAR_VARIANTS].map((v) => ({
                    id: v || 'plain',
                    label: VARIANT_LABEL[v] ?? v,
                    node: <AgentAvatar size={40} hue={agentHue(view?.id ?? d.name)} variant={v || undefined} />,
                  }))}
                />
              ),
              'Accessory',
            )
          }
        />
        <div className="m-integration-head">
          <span className="m-card-title">{name}</span>
          <div className="m-integration-status">
            <span className="m-integration-detail">{d.title?.trim() ? `${d.title.trim()} · ${status}` : status}</span>
          </div>
        </div>
      </div>
      <div className="ag-tabs-wrap">
        <Tabs label="Agent pages" className="ag-tabs">
          {AGENT_TABS.map(([t, label]) => (
            <Tab key={t} active={t === tab} onClick={() => showTab(t)}>
              {label}
            </Tab>
          ))}
        </Tabs>
      </div>
      <MainBody className="ag-body" testId="agent-body">
        <div className="m-set-page ag-page" data-tab={tab} role="tabpanel" aria-label={AGENT_TABS.find(([t]) => t === tab)?.[1]}>
          {formError && <Notice state="bad">{formError}</Notice>}
          {body}
        </div>
      </MainBody>
      {memory !== null && <MemoryDialog memory={memory === 'new' ? null : memory} onClose={() => setMemory(null)} />}
      {leave && (
        <Dialog title="Leave without saving?" onClose={() => setLeave(null)} testId="agent-leave">
          <div className="m-card-body">Your changes to this agent are not saved.</div>
          <div className="m-card-actions">
            <Button
              variant="primary"
              onClick={async () => {
                const to = leave
                setLeave(null)
                if (await save()) navigate(to)
              }}
            >
              Save
            </Button>
            <Button
              onClick={() => {
                const to = leave
                setLeave(null)
                useAgents.getState().closeEditor()
                navigate(to)
              }}
            >
              Discard
            </Button>
          </div>
        </Dialog>
      )}
    </section>
  )
}
