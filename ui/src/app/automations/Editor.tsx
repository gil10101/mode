// The automation editor (03 §7B.2), a full page by the drill-in rule, drawn as
// groups of cards and rows in the centered page column (10 §7.4.19): Name;
// the schedule (Daily / Weekdays / Weekly / Custom, a time, the zone; Weekly
// adds the weekday, Custom a 5-field cron validated as typed with its plain
// echo) and the next five fire times from `schedule_preview`; the action (Start
// a Claude Code session / Create a note / Ask Chat) with its fields; the
// options; Save as the single primary, `⋮` Run now (saves first) and Delete.
// "Unsaved" shows in the header and Esc asks before leaving.
// Keys (03 §10.8; printed only in the Shortcuts view): ⌘/Ctrl+S save,
// ⌘/Ctrl+Enter run now, Esc leave.
import { useEffect, useRef, useState } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { Combo, Notice, Seg, SegItem, Select, Switch, TextArea } from '../../kit/controls'
import { MainBody } from '../../kit/layout'
import { SettingError, SettingGroup, SettingRow } from '../../kit/settings'
import type { ActionKind, AskChatAction, ChatEffort, CreateNoteAction, PermissionMode, ScheduleKind, StartSessionAction } from '../../state/contract-automations'
import type { FoldersList, GroupingsList } from '../../state/contract-notes'
import { deviceZone, formatTokens, parseTokens, previewLine, useAutomations } from '../../state/automations-store'
import { invoke } from '../../tauri'
import { navigate } from '../router'
import { Dialog, openMenu } from '../ui'
import { confirmDelete } from './AutomationsList'
import { CrumbHead } from './Runs'
import { useAgents } from '../../state/agents-store'

const KIND_LABEL: [ScheduleKind, string][] = [
  ['daily', 'Daily'],
  ['weekdays', 'Weekdays'],
  ['weekly', 'Weekly'],
  ['cron', 'Custom'],
]
const ACTION_LABEL: [ActionKind, string][] = [
  ['start_session', 'Start a Claude Code session'],
  ['create_note', 'Create a note'],
  ['ask_chat', 'Ask Chat'],
]
export const PERMISSION_LABEL: Record<PermissionMode, string> = {
  default: 'Default',
  plan: 'Plan',
  acceptEdits: 'Accept edits',
  auto: 'Auto',
  dontAsk: "Don't ask",
  bypassPermissions: 'Bypass permissions',
}
const EFFORT_LABEL: [ChatEffort, string][] = [
  ['auto', 'Auto'],
  ['quick', 'Quick'],
  ['thorough', 'Thorough'],
  ['local_only', 'Local-only'],
]
const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']
const ZONES = ['UTC', 'Europe/London', 'Europe/Berlin', 'Europe/Madrid', 'America/New_York', 'America/Chicago', 'America/Los_Angeles', 'Asia/Tokyo', 'Australia/Sydney']

export function Editor({ id }: { id: string }) {
  const editor = useAutomations((s) => s.editor)
  const rows = useAutomations((s) => s.rows)
  const loaded = useAutomations((s) => s.loaded)
  const dirty = useAutomations((s) => s.dirty())
  const [leave, setLeave] = useState<string | null>(null)
  const [recent, setRecent] = useState<string[]>([])
  const [folders, setFolders] = useState<{ id: string | null; label: string }[]>([{ id: null, label: 'Unsorted' }])
  const [groupings, setGroupings] = useState<{ slug: string; name: string }[]>([])
  const nameRef = useRef<HTMLInputElement>(null)
  const isNew = id === 'new'
  const row = isNew ? undefined : rows.find((r) => r.id === id)
  const agents = useAgents((s) => s.rows)
  const agentsLoaded = useAgents((s) => s.loaded)
  useEffect(() => {
    if (!agentsLoaded) void useAgents.getState().load()
  }, [agentsLoaded])

  // Open the draft for this route (a row, a new one, or the example the empty state saved).
  useEffect(() => {
    const s = useAutomations.getState()
    if (isNew) {
      if (!s.editor || s.editor.draft.id) s.edit(null)
      setTimeout(() => nameRef.current?.focus(), 0)
    } else if (row && s.editor?.draft.id !== row.id) s.edit(row)
    else if (!loaded) void s.load()
  }, [id, isNew, row, loaded])

  useEffect(() => () => useAutomations.getState().closeEditor(), [])

  useEffect(() => {
    invoke<FoldersList>('folders_list').then(
      (f) => setFolders([{ id: null, label: 'Unsorted' }, ...(f?.folders ?? []).filter((x) => x.kind !== 'capture').map((x) => ({ id: x.id, label: x.path }))]),
      () => {},
    )
    invoke<string[]>('sessions_recent_folders').then(
      (r) => setRecent(Array.isArray(r) ? r : []),
      () => {},
    )
    invoke<GroupingsList>('groupings_list').then(
      (g) => setGroupings((g?.groupings ?? []).map((x) => ({ slug: x.slug, name: x.name }))),
      () => {},
    )
  }, [])

  // The preview follows the schedule (150 ms after the last change).
  const scheduleKey = JSON.stringify(editor?.draft.schedule ?? null)
  useEffect(() => {
    if (!editor) return
    const t = setTimeout(() => void useAutomations.getState().previewSchedule(), 150)
    return () => clearTimeout(t)
  }, [scheduleKey, !!editor])

  const go = (to: string) => {
    if (useAutomations.getState().dirty()) setLeave(to)
    else navigate(to)
  }

  const save = async (): Promise<string | null> => {
    const saved = await useAutomations.getState().save()
    if (saved && isNew) navigate({ view: 'automations', rest: saved.id }, { replace: true })
    return saved?.id ?? null
  }
  const runNow = async () => {
    const savedId = useAutomations.getState().dirty() || isNew ? await save() : id
    if (savedId) void useAutomations.getState().runNow(savedId)
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (mod && !e.shiftKey && e.key.toLowerCase() === 's') {
        e.preventDefault()
        void save()
      } else if (mod && e.key === 'Enter') {
        e.preventDefault()
        void runNow()
      } else if (e.key === 'Escape' && !e.defaultPrevented && !document.querySelector('.ap-scrim')) {
        const t = e.target as HTMLElement | null
        if (t?.tagName === 'INPUT' || t?.tagName === 'TEXTAREA') {
          t.blur()
          return
        }
        e.preventDefault()
        go('automations')
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  if (!editor) {
    return (
      <section className="ap-view au-view" aria-label="Automation">
        <CrumbHead items={[{ label: loaded ? 'Unavailable' : '…' }]} />
        <MainBody>{loaded && !isNew && !row && <div className="ap-empty">This automation no longer exists.</div>}</MainBody>
      </section>
    )
  }

  const d = editor.draft
  const err = (field: string) => editor.errors.find((e) => e.field === field)?.message
  const s = useAutomations.getState()
  const sched = d.schedule
  const tz = sched.tz || deviceZone()
  const preview = editor.preview
  const a = d.action

  const fail = (...fields: string[]) => {
    const e = fields.map(err).find(Boolean)
    return e ? <SettingError>{e}</SettingError> : undefined
  }
  const sa = a as StartSessionAction
  const na = a as CreateNoteAction
  const ca = a as AskChatAction

  const actionFields =
    d.action_kind === 'start_session' ? (
      <>
        <SettingRow title="Repo" desc="Where the session starts." error={fail('action.cwd')}>
          <Combo label="Repository" placeholder="~/code/mode" value={sa.cwd ?? ''} suggestions={recent} render={(p) => p.replace(/^\/Users\/[^/]+/, '~')} onChange={(v) => s.patchAction({ cwd: v })} />
        </SettingRow>
        <SettingRow title="Prompt" stack error={fail('action.prompt')}>
          <TextArea aria-label="Prompt" value={sa.prompt} onChange={(e) => s.patchAction({ prompt: e.target.value })} />
        </SettingRow>
        {agents.length > 0 && (
          <SettingRow title="Agent" desc="Runs with the agent's model, tools and rules.">
            <Select label="Agent" value={sa.profile_id ?? ''} options={[['', 'No agent'] as const, ...agents.map((g) => [g.id, g.name] as const)]} onChange={(v) => s.patchAction({ profile_id: v || null })} />
          </SettingRow>
        )}
        <SettingRow title="Permission mode">
          <Select
            label="Permission mode"
            value={sa.permission_mode}
            options={(Object.keys(PERMISSION_LABEL) as PermissionMode[]).map((k) => [k, PERMISSION_LABEL[k]] as const)}
            onChange={(v) => s.patchAction({ permission_mode: v })}
          />
        </SettingRow>
        <SettingRow title="Run as" desc="Headless runs in the background; embedded opens a terminal in Mode.">
          <Seg label="Run as">
            <SegItem active={sa.mode === 'headless'} onClick={() => s.patchAction({ mode: 'headless' })}>
              Headless
            </SegItem>
            <SegItem active={sa.mode === 'embedded'} onClick={() => s.patchAction({ mode: 'embedded' })}>
              Embedded
            </SegItem>
          </Seg>
        </SettingRow>
        <SettingRow title="Token cap" desc="The run stops at this many tokens; empty for none." error={fail('action.token_cap')}>
          <TokenCap value={sa.token_cap} onChange={(v) => s.patchAction({ token_cap: v })} />
        </SettingRow>
      </>
    ) : d.action_kind === 'create_note' ? (
      <>
        <SettingRow title="Template">
          <TemplatePicker value={na.template_note_id} onChange={(v) => s.patchAction({ template_note_id: v })} />
        </SettingRow>
        <SettingRow title="Folder">
          <FolderPicker folders={folders} value={na.folder_id} onChange={(v) => s.patchAction({ folder_id: v })} />
        </SettingRow>
        <SettingRow title="Grouping">
          <Select label="Grouping" value={na.grouping_slug ?? ''} options={[['', 'None'] as const, ...groupings.map((g) => [g.slug, g.name] as const)]} onChange={(v) => s.patchAction({ grouping_slug: v || null })} />
        </SettingRow>
        <SettingRow title="Tags" desc="Separated by commas.">
          <input
            className="m-input"
            aria-label="Tags"
            placeholder="work, daily"
            value={na.tags.join(', ')}
            onChange={(e) =>
              s.patchAction({
                tags: e.target.value
                  .split(',')
                  .map((t) => t.trim().replace(/^#/, ''))
                  .filter(Boolean),
              })
            }
          />
        </SettingRow>
        <SettingRow title="Title pattern" error={fail('action.title_pattern')}>
          <input className="m-input" aria-label="Title pattern" value={na.title_pattern} onChange={(e) => s.patchAction({ title_pattern: e.target.value })} />
        </SettingRow>
      </>
    ) : (
      <>
        <SettingRow title="Prompt" stack error={fail('action.prompt')}>
          <TextArea aria-label="Prompt" value={ca.prompt} onChange={(e) => s.patchAction({ prompt: e.target.value })} />
        </SettingRow>
        <SettingRow title="Folder" desc="Where the answer is saved as a note.">
          <FolderPicker folders={folders} value={ca.folder_id} onChange={(v) => s.patchAction({ folder_id: v })} />
        </SettingRow>
        <SettingRow title="Effort">
          <Seg label="Effort">
            {EFFORT_LABEL.map(([k, label]) => (
              <SegItem key={k} active={ca.effort === k} onClick={() => s.patchAction({ effort: k })}>
                {label}
              </SegItem>
            ))}
          </Seg>
        </SettingRow>
      </>
    )

  const problem =
    row?.problem === 'claude_not_found' && d.action_kind === 'start_session' ? (
      <>
        {'claude not found · '}
        <a role="link" className="au-link" onClick={() => navigate({ view: 'integrations', rest: 'claude-code' })}>
          Claude Code card
        </a>
      </>
    ) : row?.problem === 'no_ai_backend' && d.action_kind === 'ask_chat' ? (
      <>
        {'No AI backend · '}
        <a role="link" className="au-link" onClick={() => navigate({ view: 'settings', rest: 'ai' })}>
          Settings → AI
        </a>
      </>
    ) : null

  const formError = err('form')

  return (
    <section className="ap-view au-view" aria-label={d.name || 'New automation'} data-testid="automation-editor">
      <div className="m-main-head">
        <Button variant="icon" quiet aria-label="Back" onClick={() => go('automations')}>
          ‹
        </Button>
        <nav className="m-crumb au-crumb" aria-label="Breadcrumb">
          <span aria-current="page">{d.name.trim() || (isNew ? 'New automation' : (row?.name ?? '…'))}</span>
        </nav>
        {dirty && <span className="m-t-meta au-dirty">Unsaved</span>}
      </div>
      <MainBody className="au-editor">
        <div className="m-set-page au-page">
          <SettingGroup title="Name and schedule">
            <SettingRow title="Name" htmlFor="au-name" error={fail('name')}>
              <input id="au-name" ref={nameRef} className="m-input" aria-label="Name" value={d.name} onChange={(e) => s.patchDraft({ name: e.target.value })} />
            </SettingRow>
            <SettingRow title="Repeats" error={fail('schedule.weekday')}>
              <Seg label="Schedule">
                {KIND_LABEL.map(([k, label]) => (
                  <SegItem
                    key={k}
                    active={sched.kind === k}
                    onClick={() =>
                      s.patchDraft({
                        schedule: {
                          kind: k,
                          tz: sched.tz,
                          time: k === 'cron' ? undefined : (sched.time ?? '08:00'),
                          weekday: k === 'weekly' ? (sched.weekday ?? 1) : undefined,
                          cron: k === 'cron' ? (sched.cron ?? `${Number((sched.time ?? '08:00').split(':')[1])} ${Number((sched.time ?? '08:00').split(':')[0])} * * 1-5`) : undefined,
                        },
                      })
                    }
                  >
                    {label}
                  </SegItem>
                ))}
              </Seg>
              {sched.kind === 'weekly' && (
                <Select label="Weekday" value={sched.weekday ?? 1} options={WEEKDAYS.map((w, i) => [i + 1, w] as const)} onChange={(v) => s.patchDraft({ schedule: { ...sched, weekday: v } })} />
              )}
            </SettingRow>
            {sched.kind === 'cron' ? (
              <SettingRow title="Cron" desc="Five fields: minute, hour, day, month, weekday." error={fail('schedule.cron')}>
                <input className="m-input au-cron" aria-label="Cron" placeholder="0 8 * * 1-5" value={sched.cron ?? ''} onChange={(e) => s.patchDraft({ schedule: { ...sched, cron: e.target.value } })} />
              </SettingRow>
            ) : (
              <SettingRow title="Time" error={fail('schedule.time')}>
                <input className="m-input au-time" aria-label="Time" value={sched.time ?? ''} onChange={(e) => s.patchDraft({ schedule: { ...sched, time: e.target.value } })} />
              </SettingRow>
            )}
            <SettingRow title="Timezone">
              <Select
                label="Timezone"
                value={tz}
                options={Array.from(new Set([deviceZone(), ...ZONES, tz])).map((z) => [z, z === deviceZone() ? `${z} (this device)` : z] as const)}
                onChange={(z) => s.patchDraft({ schedule: { ...sched, tz: z } })}
              />
            </SettingRow>
            <SettingRow
              title="Next runs"
              desc={preview?.error && sched.kind === 'cron' ? undefined : <span data-testid="schedule-preview">{`${sched.kind === 'cron' && preview?.echo ? `${preview.echo} · ` : ''}${previewLine(preview, tz)}`}</span>}
              error={preview?.error && sched.kind === 'cron' ? <SettingError testId="schedule-error">{preview.error.message}</SettingError> : undefined}
            />
          </SettingGroup>
          <SettingGroup title="Action">
            <SettingRow title="What it does" stack error={problem ? <SettingError>{problem}</SettingError> : undefined}>
              <div className="m-inline">
                <Seg label="Action">
                  {ACTION_LABEL.map(([k, label]) => (
                    <SegItem key={k} active={d.action_kind === k} onClick={() => s.setActionKind(k)}>
                      {label}
                    </SegItem>
                  ))}
                </Seg>
              </div>
            </SettingRow>
            {actionFields}
          </SettingGroup>
          <SettingGroup title="Options">
            <SettingRow title="Notify on failure" desc="An alert when a run fails.">
              <Switch on={d.notify_on_failure} label="Notify on failure" onChange={(v) => s.patchDraft({ notify_on_failure: v })} />
            </SettingRow>
            <SettingRow title="Keep" desc="Older runs are pruned." error={fail('keep_runs')}>
              <input className="m-input au-time" aria-label="Keep last runs" value={String(d.keep_runs)} onChange={(e) => s.patchDraft({ keep_runs: Number(e.target.value.replace(/\D/g, '')) || 0 })} />
              <span className="m-set-unit">last runs</span>
            </SettingRow>
          </SettingGroup>
          {formError && <Notice state="bad">{formError}</Notice>}
          <div className="m-btn-group au-save">
            <Button variant="primary" disabled={editor.saving} onClick={() => void save()}>
              Save
            </Button>
            <MenuButton
              label="More"
              onClick={(e) =>
                openMenu(
                  e.currentTarget,
                  [
                    { id: 'run', label: 'Run now', dimmed: !!row?.running, reason: row?.running ? 'A run is still running' : undefined, run: () => void runNow() },
                    { id: 'history', label: 'History', dimmed: isNew, run: () => navigate({ view: 'automations', rest: `${id}/runs` }) },
                    { kind: 'sep', id: 'sep' },
                    { id: 'delete', label: 'Delete', danger: true, dimmed: isNew, run: () => row && confirmDelete(row, () => navigate('automations')) },
                  ],
                  'Automation',
                )
              }
            />
          </div>
        </div>
      </MainBody>
      {leave && (
        <Dialog title="Leave without saving?" onClose={() => setLeave(null)} testId="automation-leave">
          <div className="m-card-body">Your changes to this automation are not saved.</div>
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
                useAutomations.getState().closeEditor()
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

function TokenCap({ value, onChange }: { value: number | null; onChange: (v: number | null) => void }) {
  const [text, setText] = useState(formatTokens(value))
  useEffect(() => {
    const parsed = parseTokens(text)
    if (parsed !== value && !(Number.isNaN(parsed) && value === null)) setText(formatTokens(value))
    // only when the draft changes from outside
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value])
  return (
    <input
      className="m-input au-cap"
      aria-label="Token cap"
      placeholder="none"
      value={text}
      onChange={(e) => {
        setText(e.target.value)
        const v = parseTokens(e.target.value)
        onChange(v === null ? null : Number.isNaN(v) ? -1 : v)
      }}
    />
  )
}

function FolderPicker({ folders, value, onChange }: { folders: { id: string | null; label: string }[]; value: string | null; onChange: (v: string | null) => void }) {
  return <Select label="Folder" value={value ?? ''} placeholder={value ?? undefined} options={folders.map((f) => [f.id ?? '', f.label] as const)} onChange={(v) => onChange(v || null)} />
}

function TemplatePicker({ value, onChange }: { value: string | null; onChange: (v: string | null) => void }) {
  const [templates, setTemplates] = useState<{ id: string; title: string }[]>([])
  useEffect(() => {
    invoke<{ id: string; title: string; detail: string }[]>('palette_query', { text: '', limit: 200, kinds: ['note'] }).then(
      (r) => setTemplates((r ?? []).filter((x) => /templates/i.test(x.detail)).map((x) => ({ id: x.id, title: x.title }))),
      () => {},
    )
  }, [])
  return <Select label="Template" value={value ?? ''} placeholder={value ?? undefined} options={[['', 'None'] as const, ...templates.map((t) => [t.id, t.title] as const)]} onChange={(v) => onChange(v || null)} />
}
