// The launch and resume dialogs (03 §3.5, 05 §8.1–§8.2) and "Note from
// session" (AW-25). Both dialogs are a compact group of settings rows
// (10 §7.4.19): Folder (a combobox and Choose…, the core's folder picker),
// Agent, Permissions, Model, After launch. Launch and resume go to the PTY
// host's `session_launch` / `session_resume` (`{session_id, hooks_installed,
// error?, already_open?}`; Headless launches without a terminal, a
// Terminal-tab start passes its size); the note goes to `note_capture` with
// `created_from`.
import { useEffect, useState } from 'react'
import { Button } from '../../kit/button'
import { Capture, Combo, Notice, Seg, SegItem, Select } from '../../kit/controls'
import { Icon } from '../../kit/icons'
import { ModelPicker, SettingError, SettingGroup, SettingRow } from '../../kit/settings'
import { invoke } from '../../tauri'
import { type LaunchResult, type PermissionMode } from '../../state/contract-sessions'
import { useAgents } from '../../state/agents-store'
import { MODEL_ALIASES, type AgentView } from '../../state/contract-agents'
import { useSessionsStore } from '../../state/sessions-store'
import { cmdErrorText } from '../../state/contract'
import { navigate } from '../router'
import { useClaudeModels } from '../settings/ai'
import { Dialog } from '../ui'
import { estimateTerminalSize, useDialogs } from './actions'
import { basename, placeLabel, tildePath } from './format'

export function SessionDialogs() {
  const launch = useDialogs((s) => s.launch)
  const resume = useDialogs((s) => s.resume)
  const note = useDialogs((s) => s.note)
  return (
    <>
      {launch && <LaunchDialog />}
      {resume && <ResumeDialog id={resume} />}
      {note && <NoteDialog id={note} />}
    </>
  )
}

/** 03 §3.5: the permission modes in words; the CLI ids stay what is passed (`''` leaves the agent's or the CLI's own). */
export const PERMISSION_CHOICES: readonly (readonly [PermissionMode | '', string, string])[] = [
  ['', 'Default', 'asks before edits and commands'],
  ['plan', 'Plan', 'plan before editing'],
  ['acceptEdits', 'Accept edits', 'edits without asking, asks before commands'],
  ['auto', 'Auto', 'approves routine steps on its own'],
  ['dontAsk', "Don't ask", 'refuses what was not allowed in advance'],
  ['bypassPermissions', 'Bypass permissions', 'no prompts at all'],
]

function PermissionsRow({ id, value, onChange }: { id: string; value: PermissionMode | ''; onChange: (v: PermissionMode | '') => void }) {
  const desc = PERMISSION_CHOICES.find(([v]) => v === value)?.[2]
  return (
    <SettingRow title="Permissions" htmlFor={id}>
      <Select id={id} label="Permissions" value={value} options={PERMISSION_CHOICES.map(([v, l]) => [v, l] as const)} onChange={onChange} />
      {desc && <div className="m-set-desc ap-ctl-desc">{desc}</div>}
    </SettingRow>
  )
}

/** Where a start lands (03 §3.5 After launch): the Overview, the Terminal tab, or Headless (no terminal; launch only). */
export type AfterLaunch = 'overview' | 'terminal' | 'headless'

const AFTER_DESC: Record<AfterLaunch, string> = {
  overview: 'opens the session’s overview',
  terminal: 'opens the session in its Terminal tab',
  headless: 'no terminal; the first prompt runs on its own',
}

function AfterLaunchRow({ value, onChange, headless = true }: { value: AfterLaunch; onChange: (v: AfterLaunch) => void; headless?: boolean }) {
  return (
    <SettingRow title="After launch" desc={AFTER_DESC[value]}>
      <Seg label="After launch">
        <SegItem active={value === 'overview'} onClick={() => onChange('overview')}>
          Overview
        </SegItem>
        <SegItem active={value === 'terminal'} onClick={() => onChange('terminal')}>
          Terminal tab
        </SegItem>
        {headless && (
          <SegItem active={value === 'headless'} onClick={() => onChange('headless')}>
            Headless
          </SegItem>
        )}
      </Seg>
    </SettingRow>
  )
}

/** The Folder row: free text with the recent folders, and Choose… (the core's OS picker; a cancel changes nothing). */
function FolderRow({
  id,
  value,
  onChange,
  folders,
  placeholder,
  error,
  onError,
}: {
  id: string
  value: string
  onChange: (v: string) => void
  folders: readonly string[]
  placeholder?: string
  error: string | null
  onError: (e: string | null) => void
}) {
  const choose = async () => {
    try {
      const picked = await invoke<string | null>('sessions_pick_folder', value.trim() ? { start: value.trim() } : {})
      if (typeof picked === 'string' && picked) {
        onChange(picked)
        onError(null)
      }
    } catch (e) {
      onError(cmdErrorText(e))
    }
  }
  return (
    <SettingRow title="Folder" htmlFor={id} className="ap-folder-row" error={error ? <SettingError>{error}</SettingError> : null}>
      <Combo
        id={id}
        label="Folder"
        value={value}
        placeholder={placeholder}
        suggestions={folders}
        render={tildePath}
        onChange={(v) => {
          onChange(v)
          if (v.trim()) onError(null)
        }}
      />
      <Button quiet onClick={() => void choose()}>
        <Icon id="folder" />
        Choose…
      </Button>
    </SettingRow>
  )
}

/** The size argument of a start that opens the Terminal tab (the PTY gets the tab's real size on `pty_open`). */
export function sizeFor(open: 'overview' | 'terminal'): { cols?: number; rows?: number } {
  return open === 'terminal' ? estimateTerminalSize() : {}
}

/** After `session_launch` / `session_resume`: the row it made, the hooks notice, the terminal's failure line (05 §16). */
export function afterStart(r: LaunchResult | null, open: 'overview' | 'terminal') {
  if (!r) return
  useDialogs.getState().close()
  const s = useSessionsStore.getState()
  if (r.error) s.showNotice(`The terminal did not start: ${r.error}`)
  else if (r.already_open) s.showNotice('The terminal is already open')
  else if (!r.hooks_installed) s.showNotice('Hooks are not installed: approvals will appear in the Terminal tab only')
  navigate({ view: 'sessions', id: r.session_id, tab: r.error ? 'overview' : open })
}

function LaunchDialog() {
  const close = useDialogs((s) => s.close)
  const preset = useDialogs((s) => s.launchProfile)
  const rows = useSessionsStore((s) => s.rows)
  const health = useSessionsStore((s) => s.health)
  const launch = useSessionsStore((s) => s.launch)
  const agents = useAgents((s) => s.rows)
  const agentsLoaded = useAgents((s) => s.loaded)
  const providerModels = useClaudeModels()
  const [folders, setFolders] = useState<string[]>([])
  const [cwd, setCwd] = useState('')
  const [folderError, setFolderError] = useState<string | null>(null)
  const [mode, setMode] = useState<PermissionMode | ''>('')
  const [model, setModel] = useState('')
  const [profile, setProfile] = useState(preset ?? '')
  const [after, setAfter] = useState<AfterLaunch>('overview')
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    invoke<string[]>('sessions_recent_folders')
      .then((f) => setFolders(Array.isArray(f) ? f : []))
      .catch(() => setFolders(Array.from(new Set(rows.map((r) => r.cwd)))))
  }, [rows])
  useEffect(() => {
    if (!agentsLoaded) void useAgents.getState().load()
  }, [agentsLoaded])

  const agent = agents.find((a) => a.id === profile) ?? null
  const [firstRepo, setFirstRepo] = useState<string | null>(null)
  // An agent with repos starts in its first when the folder is left empty
  // (03 §7E.3); else its workspace fills the folder once. The user's own entry wins.
  useEffect(() => {
    setFirstRepo(null)
    if (!agent) return
    void invoke<AgentView>('agent_get', { id: agent.id }).then(
      (v) => {
        const repo = v?.repos?.[0] ?? null
        setFirstRepo(repo)
        if (!repo && v?.workspace) setCwd((c) => c || v.workspace!)
      },
      () => {},
    )
    // only when the chosen agent changes
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agent?.id])

  // "CLI default" (the picker's first option) then the agent's model, the aliases, the Claude provider's models, then Other….
  const models = [...(agent?.model ? [agent.model] : []), ...MODEL_ALIASES, ...providerModels]
  const claudeMissing = health?.claude_code === 'missing'
  const hooksMissing = health ? health.hooks !== 'installed' : false
  const submit = async () => {
    if (claudeMissing || busy) return
    if (!cwd.trim() && !firstRepo) {
      setFolderError('Choose the folder the session starts in.')
      return
    }
    setBusy(true)
    const where = after === 'terminal' ? 'terminal' : 'overview'
    const r = await launch({
      cwd: cwd.trim(),
      open: where,
      ...(mode ? { permission_mode: mode } : {}),
      ...(model.trim() ? { model: model.trim() } : {}),
      ...(profile ? { profile_id: profile } : {}),
      ...(after === 'headless' ? { overview_only: true } : sizeFor(where)),
    })
    setBusy(false)
    afterStart(r, where)
  }

  return (
    <Dialog title="Launch session" onClose={close} testId="launch-dialog">
      <form
        onSubmit={(e) => {
          e.preventDefault()
          void submit()
        }}
      >
        <SettingGroup compact>
          <FolderRow
            id="launch-folder"
            value={cwd}
            onChange={setCwd}
            folders={folders}
            placeholder={firstRepo ? `${tildePath(firstRepo)} · the agent's first repo` : '~/code/mode'}
            error={folderError}
            onError={setFolderError}
          />
          {agents.length > 0 && (
            <SettingRow title="Agent" htmlFor="launch-agent">
              <Select id="launch-agent" label="Agent" value={profile} options={[['', 'No agent'] as const, ...agents.map((a) => [a.id, a.name] as const)]} onChange={setProfile} />
            </SettingRow>
          )}
          <PermissionsRow id="permission-mode" value={mode} onChange={setMode} />
          <ModelPicker id="launch-model" defaultLabel="CLI default" models={models} value={model || null} onChange={(v) => setModel(v ?? '')} />
          <AfterLaunchRow value={after} onChange={setAfter} />
        </SettingGroup>
      </form>
      {agent && <div className="ap-dialog-note">{`Launches as ${agent.name}: its instructions, memories, tools and rules apply; the model and permissions above win over its defaults.`}</div>}
      {claudeMissing && (
        <Notice state="bad" className="ap-dialog-notice">
          Claude Code was not found. Install it to track sessions.
        </Notice>
      )}
      {!claudeMissing && hooksMissing && <Notice className="ap-dialog-notice">Hooks are not installed: approvals will appear in the Terminal tab only.</Notice>}
      <div className="m-card-actions">
        <Button onClick={close}>Cancel</Button>
        <Button variant="primary" disabled={claudeMissing || busy} onClick={() => void submit()}>
          Launch
        </Button>
      </div>
    </Dialog>
  )
}

function ResumeDialog({ id }: { id: string }) {
  const close = useDialogs((s) => s.close)
  const row = useSessionsStore((s) => s.rows.find((r) => r.id === id) ?? s.history.find((r) => r.id === id))
  const detail = useSessionsStore((s) => s.details[id])
  const load = useSessionsStore((s) => s.loadDetail)
  const resume = useSessionsStore((s) => s.resume)
  const [mode, setMode] = useState<PermissionMode | ''>('')
  const [after, setAfter] = useState<AfterLaunch>('overview')
  const [cwd, setCwd] = useState('')
  const [folderError, setFolderError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  // 03 §7E.3 "ask before switching": the agent's backup models are offered; a failed session pre-selects the first.
  const [agent, setAgent] = useState<AgentView | null>(null)
  const [model, setModel] = useState('')

  useEffect(() => {
    if (!detail) void load(id)
  }, [id, detail, load])
  const profileId = detail?.session.profile_id ?? null
  const failed = (row?.state ?? detail?.session.state) === 'failed'
  useEffect(() => {
    if (!profileId) return
    void invoke<AgentView>('agent_get', { id: profileId }).then(
      (v) => {
        setAgent(v)
        if (failed && v.ask_before_switching && v.fallback_models[0]) setModel(v.fallback_models[0])
      },
      () => setAgent(null),
    )
  }, [profileId, failed])
  const offers = agent && agent.ask_before_switching && agent.fallback_models.length ? agent : null

  const name = row?.display_name ?? detail?.session.display_name ?? 'session'
  const where = row ?? detail?.session
  const transcriptMissing = detail ? !detail.transcript.exists : false
  const submit = async () => {
    setBusy(true)
    const open = after === 'terminal' ? 'terminal' : 'overview'
    const r = await resume({ session_id: id, open, ...(mode ? { permission_mode: mode } : {}), ...(model ? { model } : {}), ...(cwd.trim() ? { cwd: cwd.trim() } : {}), ...sizeFor(open) })
    setBusy(false)
    afterStart(r, open)
  }

  return (
    <Dialog title={`Resume ${name}`} onClose={close} testId="resume-dialog">
      <form
        onSubmit={(e) => {
          e.preventDefault()
          void submit()
        }}
      >
        <SettingGroup compact>
          <FolderRow id="resume-folder" value={cwd} onChange={setCwd} folders={[]} placeholder={where ? tildePath(where.cwd) : ''} error={folderError} onError={setFolderError} />
          <PermissionsRow id="permission-mode" value={mode} onChange={setMode} />
          {offers && (
            <SettingRow title="Model" htmlFor="resume-model">
              <Select
                id="resume-model"
                label="Model"
                value={model}
                options={[['', offers.model ? `keep ${offers.model}` : 'keep the model'] as const, ...offers.fallback_models.map((m) => [m, `backup: ${m}`] as const)]}
                onChange={setModel}
              />
            </SettingRow>
          )}
          {/* A resume always reopens Claude Code's own terminal session: no Headless (05 §8.2). */}
          <AfterLaunchRow value={after} onChange={setAfter} headless={false} />
        </SettingGroup>
      </form>
      {offers && <div className="ap-dialog-note">{failed ? `${offers.name} asks before switching models; the session failed, so its first backup is selected.` : `${offers.name} asks before switching models; pick a backup to resume with it.`}</div>}
      <div className="ap-dialog-note" id="resume-folder-note">
        {where ? `Resumes in ${placeLabel(where).text}; choose another folder only if this one moved.` : ''}
      </div>
      {transcriptMissing && (
        <Notice state="bad" className="ap-dialog-notice">{`Transcript not found at ${detail?.transcript.path ?? 'its recorded path'}; Claude Code may not find this session.`}</Notice>
      )}
      <div className="m-card-actions">
        <Button onClick={close}>Cancel</Button>
        <Button variant="primary" disabled={busy} onClick={() => void submit()}>
          Resume
        </Button>
      </div>
    </Dialog>
  )
}

function NoteDialog({ id }: { id: string }) {
  const close = useDialogs((s) => s.close)
  const row = useSessionsStore((s) => s.rows.find((r) => r.id === id) ?? s.history.find((r) => r.id === id))
  const [text, setText] = useState(() => (row ? `${row.display_name} · ${row.repo_root ? basename(row.repo_root) : tildePath(row.cwd)} · ` : ''))
  const save = async (t: string) => {
    try {
      const r = await invoke<{ note_id: string }>('note_capture', { text: t, created_from: { kind: 'session', id } })
      close()
      useSessionsStore.getState().showNotice('Saved', { label: 'Open', run: () => navigate(`notes/${r.note_id}`) })
    } catch (e) {
      useSessionsStore.getState().showNotice(`Couldn't save the note: ${cmdErrorText(e)}`)
    }
  }
  return (
    <Dialog title="Note from session" onClose={close} testId="note-dialog">
      <Capture value={text} onChange={setText} onSave={(t) => void save(t)} />
    </Dialog>
  )
}
