// The first-run flow (03 §2.8; XC-01 … XC-03): one panel over the dimmed,
// inert shell. Left: an eyebrow, a headline, one paragraph and the controls of
// what the step sets; right: a live preview of the real view; a rail of ticks
// at the left edge; progress bars and Back / Continue at the bottom.
// Steps: Welcome, Overlay (placement; only without a notch, the one step that
// cannot be skipped), Claude Code, Notes, Inbox and calendar, Automations,
// Dictation, Done. Each outcome goes to `onboarding.steps` (`done` or
// `skipped`); the flow reopens at the first step neither; Open Mode writes
// `onboarding.completed_at`. Keys: Enter continues, Esc skips where skipping is
// allowed (as does the foot's Skip for now), ⌘/Ctrl+← / → move between steps. Each step comes in
// with a 240-ms fade and slide whose direction follows Continue or Back (none under reduced motion).
import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { BrandMark } from '../../kit/brands'
import { Button } from '../../kit/button'
import { Key, Seg, SegItem, Switch } from '../../kit/controls'
import { Icon, type IconId } from '../../kit/icons'
import { Spinner } from '../../kit/primitives'
import { Task } from '../../kit/lists'
import { listen, invoke } from '../../tauri'
import { EVENTS, type DictationModelProgress } from '../../state/contract'
import { hasService, useAccounts, type ConnectorCardKind } from '../../state/accounts-store'
import { deviceZone } from '../../state/automations-store'
import type { AutomationDraft, AutomationRow } from '../../state/contract-automations'
import type { VaultInspect, VaultPick } from '../../state/contract-settings'
import { currentOs, useSetting, useSettingsStore } from '../../state/settings-store'
import { navigate, useRouter } from '../router'
import { oauthClientKnown } from '../integrations/AccountsPage'
import { copyText } from '../ui'
import { HookPreview } from '../integrations/HookPreview'
import { useClaudeHealth, useHooks } from '../integrations/hooks-store'
import { claudeCodeCard, tilde } from '../integrations/model'
import { AutomationsPreview } from './previews/Automations'
import { DictationPreview } from './previews/Dictation'
import { asPlacement } from './previews/Frame'
import { InboxPreview } from './previews/Inbox'
import { reducedMotion } from './previews/loop'
import { NotesPreview } from './previews/Notes'
import { OverlayPreview } from './previews/Overlay'
import { SessionsPreview } from './previews/Sessions'
import { useOnboarding } from './state'
import { WelcomeBot } from './WelcomeBot'
import { COPY, DONE_LEDE, DONE_TITLE, firstOpen, holdCaps, knownRecord, REPO_URL, setLine, SKIPPED_LINE, skippable, starterMeta, stepsFor, WELCOME_LINE, type Outcome, type SettingStep, type StepId } from './steps'
import './onboarding.css'

export { firstOpen, stepsFor, type StepId } from './steps'

/** What Continue does on a step: the outcome to record, or null to stay (an error is showing). */
type Proceed = () => Promise<Outcome | null>

export function Onboarding() {
  const forced = useOnboarding((s) => s.forced)
  const close = useOnboarding((s) => s.close)
  const platform = useSettingsStore((s) => s.platform)
  const loaded = useSettingsStore((s) => s.loaded)
  const [stored, setRecord] = useSetting<Partial<Record<string, Outcome>> | null>('onboarding.steps')
  const [, setCompleted] = useSetting<string | null>('onboarding.completed_at')
  const [placementSetting] = useSetting<string | null>('overlay.placement')
  const notch = !!platform?.notch
  const steps = useMemo(() => stepsFor(notch), [notch])
  const record = useMemo(() => knownRecord(stored), [stored])
  const [step, setStep] = useState<StepId | null>(null)
  const [busy, setBusy] = useState(false)
  /** Which way the last step change went: the next step slides in from that side. */
  const [dir, setDir] = useState<'fwd' | 'back'>('fwd')
  const [placement, setPlacement] = useState<'top-bar' | 'right-edge'>(placementSetting === 'right-edge' ? 'right-edge' : 'top-bar')
  /** The starter automation's switch on the Automations step (the preview's row follows it). */
  const [starterOn, setStarterOn] = useState(false)
  /** Its switch as saved, for the Done line; null until the Automations step has read or saved it. */
  const [starterSaved, setStarterSaved] = useState<boolean | null>(null)
  const proceed = useRef<Proceed | null>(null)
  const panelRef = useRef<HTMLDivElement>(null)
  const layerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!useSettingsStore.getState().loaded) void useSettingsStore.getState().load()
    const before = document.title
    document.title = 'Welcome to Mode'
    return () => {
      document.title = before
    }
  }, [])
  useEffect(() => {
    if (step === null && loaded) setStep(forced ? steps[0]! : firstOpen(steps, record))
  }, [loaded, forced, steps, record, step])

  const current: StepId = step ?? steps[0]!
  const index = Math.max(0, steps.indexOf(current))
  const go = (id: StepId | undefined) => {
    if (!id) return
    proceed.current = null
    setDir(steps.indexOf(id) < index ? 'back' : 'fwd')
    setStep(id)
  }
  const mark = (id: StepId, outcome: Outcome) => {
    void setRecord({ ...(stored ?? {}), [id]: outcome })
    go(steps[steps.indexOf(id) + 1] ?? 'done')
  }
  const finish = () => {
    void setCompleted(new Date().toISOString())
    close()
    if (useRouter.getState().route.view !== 'shortcuts') navigate({ view: 'sessions' })
  }
  const next = async () => {
    if (busy) return
    if (current === 'done') return finish()
    setBusy(true)
    try {
      const outcome = proceed.current ? await proceed.current() : 'done'
      if (outcome) mark(current, outcome)
    } finally {
      setBusy(false)
    }
  }
  const skip = () => mark(current, 'skipped')

  useEffect(() => {
    panelRef.current?.querySelector<HTMLElement>('[data-autofocus], [data-ob-continue]')?.focus()
  }, [current])

  // On the window, so the keys work with nothing focused too; not from a dialog opened above the layer (Connect).
  const onKey = (e: KeyboardEvent) => {
    const t = e.target as HTMLElement
    if (t !== document.body && !layerRef.current?.contains(t)) return
    if ((e.metaKey || e.ctrlKey) && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
      e.preventDefault()
      go(steps[index + (e.key === 'ArrowRight' ? 1 : -1)])
      return
    }
    const phase = useHooks.getState().flow?.phase
    const hookBusy = phase === 'applying' || phase === 'verifying'
    if (e.key === 'Escape' && skippable(current) && !busy && !hookBusy) {
      e.preventDefault()
      skip()
    } else if (e.key === 'Enter' && !/^(BUTTON|INPUT|TEXTAREA|A)$/.test(t.tagName) && !t.closest('[role="button"], [role="switch"]')) {
      e.preventDefault()
      void next()
    }
  }

  const keys = useRef(onKey)
  keys.current = onKey
  useEffect(() => {
    const h = (e: KeyboardEvent) => keys.current(e)
    window.addEventListener('keydown', h)
    return () => window.removeEventListener('keydown', h)
  }, [])

  const health = useClaudeHealth()
  const hookStatus = useHooks((s) => s.status)
  const hookDiag = useHooks((s) => s.diag)
  const hookFlow = useHooks((s) => s.flow)
  const hookBusy = hookFlow?.phase === 'applying' || hookFlow?.phase === 'verifying'
  const claudeMissing = claudeCodeCard(hookStatus, hookDiag, Date.now(), health).missing
  // One primary per screen: on Claude Code the hook preview carries its own (Install hooks, then Done);
  // with the hooks already installed there is no preview and Continue is the primary again.
  const continuePrimary = current !== 'claude' || claudeMissing || (!hookFlow && hookStatus?.status === 'installed')
  const label = current === 'welcome' ? 'Get started' : current === 'done' ? 'Open Mode' : 'Continue'
  const register = (fn: Proceed) => {
    proceed.current = fn
  }
  const overlayPlacement = asPlacement(placementSetting, notch)

  let side: ReactNode = null
  let stage: ReactNode = null
  switch (current) {
    case 'placement':
      side = <PlacementSets value={placement} onChange={setPlacement} register={register} />
      stage = <OverlayPreview placement={placement} />
      break
    case 'claude':
      side = <HooksSets onDone={() => mark('claude', 'done')} register={register} />
      stage = <SessionsPreview />
      break
    case 'notes':
      side = <NotesSets register={register} />
      stage = <NotesPreview />
      break
    case 'accounts':
      side = <AccountsSets register={register} />
      stage = <InboxPreview />
      break
    case 'automations':
      side = <AutomationsSets on={starterOn} onChange={setStarterOn} onSaved={setStarterSaved} register={register} />
      stage = <AutomationsPreview starterOn={starterOn} />
      break
    case 'dictation':
      side = <DictationSets register={register} />
      stage = <DictationPreview placement={overlayPlacement} />
      break
    case 'done':
      side = <StarRepo />
      stage = <DoneLines steps={steps} record={record} starterSaved={starterSaved} />
      break
  }

  return (
    <div className="m-ob-layer" data-testid="onboarding" ref={layerRef}>
      <div className="m-ob-panel" ref={panelRef} role="dialog" aria-modal="true" aria-label="Welcome to Mode" data-step={current}>
        {current === 'welcome' ? (
          <div className="m-ob-welcome" data-testid="onboarding-welcome" data-enter={dir}>
            <WelcomeBot />
            <h2 className="m-ob-title">Mode</h2>
            <p className="m-ob-lede">{WELCOME_LINE}</p>
          </div>
        ) : (
          <>
            <div className="m-ob-rail" aria-hidden="true">
              {steps.map((s, i) => (
                <i key={s} className={i === index ? 'is-on' : undefined} />
              ))}
            </div>
            <div className="m-ob-side" key={`side-${current}`} data-testid={`onboarding-${current}`} data-enter={dir}>
              {current === 'done' ? <Copy title={DONE_TITLE} lede={DONE_LEDE} /> : <Copy {...COPY[current]} />}
              {side && <div className="m-ob-sets">{side}</div>}
            </div>
            <div className="m-ob-stage" key={`stage-${current}`} data-enter={dir}>
              {stage}
            </div>
          </>
        )}
        <div className="m-ob-foot">
          <div className="m-ob-bars" role="img" aria-label={`Step ${index + 1} of ${steps.length}`}>
            {steps.map((s, i) => (
              <i key={s} className={i <= index ? 'is-on' : undefined} />
            ))}
          </div>
          {skippable(current) && (
            <Button quiet disabled={busy || hookBusy} onClick={skip}>
              Skip for now
            </Button>
          )}
          {index > 0 && (
            <Button quiet onClick={() => go(steps[index - 1])}>
              Back
            </Button>
          )}
          <Button variant={continuePrimary ? 'primary' : 'default'} disabled={busy} onClick={() => void next()} data-ob-continue="">
            {label}
          </Button>
        </div>
      </div>
    </div>
  )
}

function Copy({ eyebrow, icon, title, lede }: { eyebrow?: string; icon?: (typeof COPY)[SettingStep]['icon']; title: string; lede: string }) {
  return (
    <div className="m-ob-copy">
      {eyebrow !== undefined && (
        <div className="m-ob-eyebrow">
          {icon === 'claudecode' ? <BrandMark id="claudecode" size={12} label="" /> : icon ? <Icon id={icon} /> : null}
          <span>{eyebrow}</span>
        </div>
      )}
      <h2 className="m-ob-title">{title}</h2>
      <p className="m-ob-lede">{lede}</p>
    </div>
  )
}

/* ---- Overlay: placement (XC-02) ------------------------------------------------------------ */

function PlacementSets({ value, onChange, register }: { value: 'top-bar' | 'right-edge'; onChange: (p: 'top-bar' | 'right-edge') => void; register: (fn: Proceed) => void }) {
  const [, setPlacement] = useSetting<string | null>('overlay.placement')
  const [error, setError] = useState<string | null>(null)
  register(async () => {
    if (await setPlacement(value)) return 'done'
    setError(useSettingsStore.getState().errors['overlay.placement'] ?? 'That placement could not be saved.')
    return null
  })
  return (
    <>
      <Seg label="Placement">
        <SegItem active={value === 'top-bar'} onClick={() => onChange('top-bar')}>
          Top bar
        </SegItem>
        <SegItem active={value === 'right-edge'} onClick={() => onChange('right-edge')}>
          Right edge
        </SegItem>
      </Seg>
      <div className="m-meta">You can change it later in Settings → Overlay.</div>
      {error && (
        <div className="m-ob-error" role="alert">
          {error}
        </div>
      )}
    </>
  )
}

/* ---- Claude Code: hook install with the diff preview (XC-03) ------------------------------- */

function HooksSets({ onDone, register }: { onDone: () => void; register: (fn: Proceed) => void }) {
  const flow = useHooks((s) => s.flow)
  const status = useHooks((s) => s.status)
  const diag = useHooks((s) => s.diag)
  const [showDiff, setShowDiff] = useState(false)
  // The status is read first: hooks already installed (a revisit, Back, a re-run) open no preview.
  const [checked, setChecked] = useState(false)
  useEffect(() => {
    let live = true
    void useHooks.getState().loadDiagnostics()
    void (async () => {
      await useHooks.getState().refresh()
      if (!live) return
      setChecked(true)
      if (useHooks.getState().status?.status !== 'installed') await useHooks.getState().preview('install')
    })()
    return () => {
      live = false
      useHooks.getState().close()
    }
  }, [])
  const health = useClaudeHealth()
  const card = claudeCodeCard(status, diag, Date.now(), health)
  const installed = flow?.phase === 'done' || status?.status === 'installed'
  register(async () => (installed ? 'done' : 'skipped'))
  if (card.missing) return <div className="m-meta">Claude Code was not found. Install it to track sessions.</div>
  if (!checked) return null
  if (!flow && status?.status === 'installed')
    return (
      <>
        <div className="m-ob-line" data-testid="hooks-installed">
          Hooks installed
        </div>
        <div className="m-meta">{`In ${tilde(status.path)}. Nothing else to do here.`}</div>
      </>
    )
  // The step's one skip is the foot's Skip for now, so the preview draws none.
  return (
    <div className={`ob-hooks${showDiff ? ' is-open' : ''}`}>
      {flow && <HookPreview flow={flow} onDone={onDone} applyLabel="Install hooks" />}
      {flow?.plan?.changed && flow.phase !== 'done' && (
        <Button quiet onClick={() => setShowDiff(!showDiff)}>
          {showDiff ? 'Hide the change' : 'Show the change'}
        </Button>
      )}
    </div>
  )
}

/* ---- Notes: the folder, a first capture ----------------------------------------------------- */

function NotesSets({ register }: { register: (fn: Proceed) => void }) {
  const [vault, setVault] = useSetting<string>('notes.vault_path')
  const [path, setPath] = useState(vault || '~/Mode/Notes')
  const [files, setFiles] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [typedPath, setTypedPath] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)
  /** Why the folder shown could not be read (`vault_inspect` refused). */
  const [inspectError, setInspectError] = useState<string | null>(null)
  // The count under the field: `vault_inspect {path}` (the core expands `~`) for the folder shown, 300 ms after typing stops.
  useEffect(() => {
    const p = path.trim()
    if (!p) {
      setFiles(null)
      setInspectError(null)
      return
    }
    let live = true
    const t = window.setTimeout(() => {
      invoke<VaultInspect>('vault_inspect', { path: p }).then(
        (r) => {
          if (!live) return
          setFiles(r?.exists ? r.md_files : null)
          setInspectError(null)
        },
        (e) => {
          if (!live) return
          setFiles(null)
          setInspectError(`That folder could not be read: ${errorText(e)}`)
        },
      )
    }, 300)
    return () => {
      live = false
      window.clearTimeout(t)
    }
  }, [path])
  const choose = async () => {
    try {
      // Closed without a pick (null): the field stays as it was.
      const r = await invoke<VaultPick>('vault_pick', { current: path })
      if (r?.path) {
        setPath(r.path)
        if (typeof r.md_files === 'number') setFiles(r.md_files)
      }
    } catch {
      // The folder dialog is not available: the typed path is the fallback.
      setTypedPath(true)
      inputRef.current?.focus()
    }
  }
  const save = async (): Promise<boolean> => {
    const p = path.trim()
    if (!p) {
      setError('Type a folder path.')
      return false
    }
    if (p !== vault && !(await setVault(p))) {
      setError(useSettingsStore.getState().errors['notes.vault_path'] ?? 'That folder cannot be used.')
      return false
    }
    setError(null)
    return true
  }
  register(async () => ((await save()) ? 'done' : null))
  const [note, setNote] = useState('')
  // What was captured here, newest first; ids keep each row's enter animation to its first draw.
  const [captured, setCaptured] = useState<{ id: number; text: string }[]>([])
  /** Why the last capture was not saved; the text stays in the field. */
  const [captureError, setCaptureError] = useState<string | null>(null)
  const noteRef = useRef<HTMLInputElement>(null)
  const capture = async (text: string) => {
    if (!(await save())) return
    try {
      await invoke('note_capture', { text })
      setNote('')
      setCaptureError(null)
      setCaptured((c) => [{ id: (c[0]?.id ?? 0) + 1, text }, ...c])
      noteRef.current?.focus()
    } catch (e) {
      setCaptureError(`Not saved: ${errorText(e)}`)
    }
  }
  return (
    <>
      <div className="m-inline">
        <input ref={inputRef} className="m-input ob-path" aria-label="Notes folder" value={path} onChange={(e) => setPath(e.target.value)} data-autofocus={typedPath ? '' : undefined} />
        <Button onClick={() => void choose()}>Choose…</Button>
      </div>
      {typedPath && <div className="m-meta">Type the folder path; Mode creates it if needed.</div>}
      {inspectError ? (
        <div className="m-ob-error" role="alert" data-testid="vault-inspect-error">
          {inspectError}
        </div>
      ) : files ? (
        <div className="m-meta" data-testid="vault-files">{`Index this vault · ${files} Markdown files`}</div>
      ) : (
        <div className="m-meta">Mode adds only a hidden .mode/ folder.</div>
      )}
      <input
        ref={noteRef}
        className="m-input"
        aria-label="Capture a first note"
        placeholder={captured.length ? 'Capture another…' : 'Capture a first note…'}
        value={note}
        onChange={(e) => setNote(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && note.trim()) {
            e.preventDefault()
            void capture(note.trim())
          }
        }}
      />
      {captureError && (
        <div className="m-ob-error" role="alert" data-testid="capture-error">
          {captureError}
        </div>
      )}
      {/* Three rows from the first paint, so the field and the copy above it never move when a note lands. */}
      <div className={`m-ob-captured${captured.length > 3 ? ' is-more' : ''}`} data-testid="captured-notes" aria-label="Captured in Unsorted">
        {captured.length ? captured.map((n) => <Task key={n.id} className="m-ob-enter" glyph="○" title={n.text} meta="just now" />) : <div className="m-ob-captured-empty">Captured notes show here.</div>}
      </div>
      {error && (
        <div className="m-ob-error" role="alert">
          {error}
        </div>
      )}
    </>
  )
}

/* ---- Inbox and calendar: the Integrations connect flow --------------------------------------- */

const ACCOUNT_ROWS: { kind: ConnectorCardKind; service: 'gmail' | 'slack' | 'calendar'; brand: 'gmail' | 'slack' | 'googlecalendar'; name: string }[] = [
  { kind: 'gmail', service: 'gmail', brand: 'gmail', name: 'Gmail' },
  { kind: 'slack', service: 'slack', brand: 'slack', name: 'Slack' },
  { kind: 'google-calendar', service: 'calendar', brand: 'googlecalendar', name: 'Google Calendar' },
]

function AccountsSets({ register }: { register: (fn: Proceed) => void }) {
  const accounts = useAccounts((s) => s.accounts)
  const [clients, setClients] = useState<{ google: boolean | null; slack: boolean | null }>({ google: null, slack: null })
  useEffect(() => {
    void useAccounts.getState().load()
    // A probe that fails says nothing either way: unknown, not parked.
    const probe = (name: string) =>
      invoke<{ stored?: boolean }>('secret_status', { name }).then(
        (r) => (typeof r?.stored === 'boolean' ? r.stored : null),
        () => null,
      )
    void Promise.all([probe('oauth_client:google'), probe('oauth_client:slack')]).then(([google, slack]) => setClients({ google, slack }))
    // A wait this step started ends with it (the dialog shows none of it); the core closes its listener.
    return () => {
      const f = useAccounts.getState().flow
      if (!f?.inline) return
      if (f.phase === 'starting' || f.phase === 'waiting') useAccounts.getState().cancelFlow()
      else if (f.phase !== 'setup') useAccounts.setState({ flow: null })
    }
  }, [])
  const connectedAs = (service: 'gmail' | 'slack' | 'calendar') => accounts.find((a) => hasService(a, service))?.display_name ?? null
  const any = ACCOUNT_ROWS.some((r) => connectedAs(r.service) !== null)
  register(async () => (any ? 'done' : 'skipped'))
  const google = oauthClientKnown(clients.google, accounts)
  const slack = clients.slack === null && accounts.some((a) => a.provider === 'slack') ? true : clients.slack
  // The OAuth clients are the owner's to register (06 §6.1, §8.1); until one is stored its Connect cannot finish.
  const parked = google === false || slack === false
  return (
    <>
      {ACCOUNT_ROWS.map((r) => (
        <ConnectRow key={r.kind} row={r} connectedAs={connectedAs(r.service)} noClient={(r.kind === 'slack' ? slack : google) === false} />
      ))}
      {parked && !any && <div className="m-meta" data-testid="accounts-parked">Needs the OAuth clients; skip for now.</div>}
    </>
  )
}

/** How a row's last Connect run ended when it did not connect: shown until the next run. */
type ConnectEnd = { kind: 'cancelled' } | { kind: 'failed'; reason: string }

/**
 * One account row (03 §2.8 step 4): **Connect** opens the Integrations connect flow's setup; from
 * `account_add_start` on the row shows the wait (the kit spinner, Cancel; Copy link when the browser
 * could not be opened), then the answer: connected (a check and the account's name, faded in),
 * canceled ("Canceled" for two seconds), or refused / timed out / failed (the reason, Try again).
 */
function ConnectRow({ row, connectedAs, noClient }: { row: (typeof ACCOUNT_ROWS)[number]; connectedAs: string | null; noClient: boolean }) {
  const flow = useAccounts((s) => (s.flow?.inline && s.flow.card === row.kind ? s.flow : null))
  const [end, setEnd] = useState<ConnectEnd | null>(null)
  const last = useRef<typeof flow>(null)
  useEffect(() => {
    const before = last.current
    last.current = flow
    if (flow) {
      if (flow.phase === 'failed') setEnd({ kind: 'failed', reason: flow.error ?? 'Could not finish connecting.' })
      else if (flow.phase !== 'done') setEnd((e) => (e?.kind === 'failed' && flow.phase === 'setup' ? e : null))
      return
    }
    // The run went away without an answer: the dialog closed in setup, or Cancel during the wait.
    if (before && (before.phase === 'setup' || before.phase === 'starting' || before.phase === 'waiting')) setEnd({ kind: 'cancelled' })
  }, [flow])
  useEffect(() => {
    if (end?.kind !== 'cancelled') return
    const t = window.setTimeout(() => setEnd(null), 2000)
    return () => window.clearTimeout(t)
  }, [end])
  const connect = () => {
    // One run at a time: another row's wait is canceled (its listener closed) before this one starts.
    const acc = useAccounts.getState()
    if (acc.flow && (acc.flow.phase === 'starting' || acc.flow.phase === 'waiting')) acc.cancelFlow()
    acc.startConnect(row.kind, { inline: true, ...(row.kind === 'google-calendar' ? { services: ['gmail', 'calendar'] } : {}) })
  }
  const waiting = !!flow && (flow.phase === 'starting' || flow.phase === 'waiting')
  let right: ReactNode
  let note: ReactNode = null
  if (connectedAs !== null) {
    right = (
      // Connected by this row's run: the check fades in; one connected before (a reopen) shows from the first paint.
      <span className={`m-ob-connected${flow?.phase === 'done' ? ' m-ob-enter' : ''}`} data-testid="connected">
        <span className="m-ob-check" aria-hidden="true">
          ✓
        </span>
        <span className="m-ellipsis">{`Connected · ${connectedAs}`}</span>
      </span>
    )
  } else if (waiting) {
    right = <Button onClick={() => useAccounts.getState().cancelFlow()}>Cancel</Button>
    note = flow.openFailed ? (
      <div className="m-ob-row-note" role="status">
        {"Couldn't open the browser"}
        <Button quiet onClick={() => flow.authUrl && copyText(flow.authUrl, 'Link copied')}>
          Copy link
        </Button>
      </div>
    ) : (
      <div className="m-ob-row-note" role="status">
        <Spinner />
        <span>Waiting for your browser…</span>
      </div>
    )
  } else {
    right = <Button onClick={connect}>{end?.kind === 'failed' ? 'Try again' : 'Connect'}</Button>
    if (end?.kind === 'failed')
      note = (
        <div className="m-ob-row-note m-ob-error" role="alert">
          {end.reason}
        </div>
      )
    else if (end?.kind === 'cancelled') note = <div className="m-ob-row-note is-quiet">Canceled</div>
    else if (noClient) note = <div className="m-ob-row-note is-quiet">Needs its OAuth client</div>
  }
  return (
    <div className="m-ob-account" data-testid={`connect-${row.kind}`}>
      <div className="m-ob-row">
        <BrandMark id={row.brand} size={16} label="" />
        <span className="m-ob-row-name">{row.name}</span>
        {right}
      </div>
      {note}
    </div>
  )
}

/* ---- Automations: one starter, off until switched on ------------------------------------------ */

const STARTER = { name: 'Nightly tests', prompt: 'Run the suite', time: '23:00' }

/**
 * The starter (03 §2.8 step 5): its switch starts off and is live. Continue saves it with `enabled` equal to the
 * switch (`automation_save`); one already there is left alone when it matches and flipped through
 * `automation_set_enabled` (as the Automations view's switch does) when it does not.
 */
function AutomationsSets({ on, onChange, onSaved, register }: { on: boolean; onChange: (on: boolean) => void; onSaved: (on: boolean) => void; register: (fn: Proceed) => void }) {
  const [vault] = useSetting<string>('notes.vault_path')
  const [existing, setExisting] = useState<AutomationRow | null>(null)
  const [error, setError] = useState<string | null>(null)
  const saved = (row: AutomationRow) => {
    setExisting(row)
    onSaved(row.enabled)
  }
  useEffect(() => {
    invoke<AutomationRow[]>('automations_list').then(
      (rows) => {
        const row = (rows ?? []).find((r) => r.name === STARTER.name)
        if (!row) return
        saved(row)
        onChange(row.enabled)
      },
      (e) => setError(`Your automations could not be read: ${refusal(e)}`),
    )
  }, [])
  const refusal = (e: unknown) => (e as { fields?: { message: string }[]; message?: string })?.fields?.[0]?.message ?? (e as { message?: string })?.message ?? String(e)
  register(async () => {
    setError(null)
    if (existing) {
      if (existing.enabled === on) return 'done'
      try {
        saved(await invoke<AutomationRow>('automation_set_enabled', { id: existing.id, enabled: on }))
        return 'done'
      } catch (e) {
        // The switch goes back to what is stored.
        onChange(existing.enabled)
        setError(`Not changed: ${refusal(e)} Skip for now and change it in Automations.`)
        return null
      }
    }
    let cwd = ''
    try {
      cwd = (await invoke<string[]>('sessions_recent_folders'))?.[0] ?? ''
    } catch {
      /* no sessions yet: the notes folder below */
    }
    if (!cwd && vault?.startsWith('/')) cwd = vault
    const draft: AutomationDraft = {
      name: STARTER.name,
      enabled: on,
      schedule: { kind: 'daily', time: STARTER.time, tz: deviceZone() },
      action_kind: 'start_session',
      action: { cwd, prompt: STARTER.prompt, permission_mode: 'default', mode: 'headless', token_cap: null },
      notify_on_failure: true,
      keep_runs: 20,
    }
    try {
      saved(await invoke<AutomationRow>('automation_save', { draft }))
      return 'done'
    } catch (e) {
      setError(`Not added: ${refusal(e)} Skip for now and add it in Automations.`)
      return null
    }
  })
  return (
    <>
      <div className="m-ob-row" data-testid="starter-automation">
        <Switch on={on} onChange={onChange} label={STARTER.name} />
        <div className="m-row-text">
          <div className="m-row-name">{STARTER.name}</div>
          <div className="m-row-detail">{`Daily ${STARTER.time} · Start session · ${STARTER.prompt.toLowerCase()}`}</div>
        </div>
      </div>
      <div className="m-meta" data-testid="starter-meta">
        {starterMeta(existing ? existing.enabled : null, on)}
      </div>
      {error && (
        <div className="m-ob-error" role="alert">
          {error}
        </div>
      )}
    </>
  )
}

/* ---- Dictation: the switch, the model download, the microphone ------------------------------ */

const mb = (n: number) => Math.round(n / 1_000_000)

/**
 * Turning dictation on (03 §2.8, §8 Dictation): write `dictation.enabled`, `dictation_model_ensure {model}` (the
 * download reports through `dictation.model` until `done`), then `dictation_mic_check` once, which raises the OS
 * microphone prompt. A failure turns the switch back off and says why.
 */
function DictationSets({ register }: { register: (fn: Proceed) => void }) {
  const [enabled, setEnabled] = useSetting<boolean>('dictation.enabled')
  const [model] = useSetting<string>('dictation.model')
  const [progress, setProgress] = useState<DictationModelProgress | null>(null)
  const [phase, setPhase] = useState<'idle' | 'downloading' | 'checking' | 'ready'>('idle')
  const [error, setError] = useState<string | null>(null)
  /** The last failure came from turning it on (the download, an offline machine, the microphone): Try again runs it again. */
  const [retry, setRetry] = useState(false)
  const name = model || 'base.en'
  const waiting = useRef(false)
  const fail = async (why: string) => {
    waiting.current = false
    setPhase('idle')
    setError(why)
    setRetry(true)
    await setEnabled(false)
  }
  const micCheck = async () => {
    setPhase('checking')
    try {
      const r = await invoke<{ ok: boolean; error?: string | null }>('dictation_mic_check')
      if (r && r.ok === false) return void fail(r.error ?? 'The microphone is not available.')
      setPhase('ready')
    } catch (e) {
      await fail(errorText(e))
    }
  }
  useEffect(() => {
    let off: (() => void) | null = null
    let live = true
    void listen<DictationModelProgress>(EVENTS.dictationModel, (p) => {
      if (!p || (p.model && p.model !== name)) return
      setProgress(p)
      if (!waiting.current) return
      if (p.error) void fail(p.error)
      else if (p.done) {
        waiting.current = false
        void micCheck()
      }
    }).then((u) => (live ? (off = u) : u()))
    return () => {
      live = false
      off?.()
    }
  }, [name])
  register(async () => (enabled === true ? 'done' : 'skipped'))
  const turn = async (on: boolean) => {
    setError(null)
    setRetry(false)
    if (!(await setEnabled(on))) {
      setError(useSettingsStore.getState().errors['dictation.enabled'] ?? 'Dictation could not be turned on.')
      return
    }
    if (!on) {
      waiting.current = false
      setPhase('idle')
      setProgress(null)
      return
    }
    waiting.current = true
    setPhase('downloading')
    try {
      await invoke('dictation_model_ensure', { model: name })
    } catch (e) {
      await fail(errorText(e))
    }
  }
  const downloading = phase === 'downloading' && progress && !progress.done && progress.total > 0
  const getting = phase === 'downloading' || phase === 'checking'
  return (
    <>
      <div className="m-ob-row">
        <Switch on={enabled === true} onChange={(on) => void turn(on)} label="Dictation" />
        <span className="m-ob-row-name">Dictation</span>
      </div>
      <HoldKey />
      {getting ? (
        <div className="m-ob-getting" data-testid="dictation-progress">
          <div className="m-meta m-ob-busy">
            <Spinner />
            <span>{phase === 'checking' ? 'Asking for the microphone…' : downloading ? `Downloading ${name} · ${mb(progress.downloaded)} of ${mb(progress.total)} MB` : `Getting ${name} ready…`}</span>
          </div>
          {downloading && (
            <div className="m-ob-progress" role="progressbar" aria-label={`Downloading ${name}`} aria-valuemin={0} aria-valuemax={progress.total} aria-valuenow={progress.downloaded}>
              <i style={{ width: `${Math.min(100, (progress.downloaded / progress.total) * 100)}%` }} />
            </div>
          )}
        </div>
      ) : enabled === true ? (
        <div className="m-meta" data-testid="dictation-ready">
          Ready. The microphone is on only while the key is held.
        </div>
      ) : null}
      {error && (
        <div className="m-ob-error" role="alert" data-testid="dictation-error">
          {error}
        </div>
      )}
      {error && retry && (
        <div className="m-inline">
          <Button onClick={() => void turn(true)}>Try again</Button>
        </div>
      )}
      {enabled === true && phase !== 'downloading' && phase !== 'checking' && <input className="m-input" aria-label="Try it" placeholder="Hold the key and say something" />}
    </>
  )
}

/** "Hold ⌃ ⌥ D, speak, let go.": the one key printed outside the Shortcuts view, because that view is under this panel (ADR-0062). */
function HoldKey() {
  const [hotkey] = useSetting<string>('dictation.hotkey')
  const os = useSettingsStore((s) => s.platform?.os) ?? currentOs()
  const keys = holdCaps(hotkey, os)
  return (
    <div className="m-ob-hold" data-testid="dictation-key">
      {'Hold'}
      <span className="m-keys" aria-label={keys.join(' ')}>
        {keys.map((k) => (
          <Key key={k}>{k}</Key>
        ))}
      </span>
      {', speak, let go.'}
    </div>
  )
}

function errorText(e: unknown): string {
  return String((e as { message?: string })?.message ?? e)
}

/* ---- Done --------------------------------------------------------------------------------- */

/** How long the wheel shows at the least, so it is seen, and the step between two lines turning. */
const DONE_WAIT_MS = 600
const DONE_STAGGER_MS = 80

/**
 * What was set (03 §2.8 step 7): every line starts with the kit spinner in its glyph's place while what was
 * set is read (the record; `automations_list` for the starter's switch when this pass did not save it), then,
 * 80 ms apart, the glyph turns: a check for a set step, the step's own icon for a skipped one. A read that
 * fails lists the lines from the record alone. Under reduced motion the end state at once.
 */
function DoneLines({ steps, record, starterSaved }: { steps: readonly StepId[]; record: Partial<Record<StepId, Outcome>>; starterSaved: boolean | null }) {
  const shown = steps.filter((s): s is SettingStep => s !== 'welcome' && s !== 'done')
  const set = shown.filter((s) => record[s] === 'done')
  const skipped = shown.filter((s) => record[s] === 'skipped')
  // The starter's switch as the Automations step saved it; on a reopen straight at Done, as the core lists it.
  const needList = starterSaved === null && record.automations === 'done'
  const [listed, setListed] = useState<boolean | null | 'failed'>(null)
  const [reduced] = useState(reducedMotion)
  /** How many lines have turned (all of them under reduced motion once the read is in). */
  const [turned, setTurned] = useState(0)
  const [minWait, setMinWait] = useState(reduced)
  useEffect(() => {
    if (reduced) return
    const t = window.setTimeout(() => setMinWait(true), DONE_WAIT_MS)
    return () => window.clearTimeout(t)
  }, [reduced])
  useEffect(() => {
    if (!needList) return
    let live = true
    invoke<AutomationRow[]>('automations_list').then(
      (rows) => live && setListed((rows ?? []).find((r) => r.name === STARTER.name)?.enabled ?? false),
      () => live && setListed('failed'),
    )
    return () => {
      live = false
    }
  }, [needList])
  const read = !needList || listed !== null
  const total = set.length + skipped.length
  const ready = read && minWait
  useEffect(() => {
    if (!ready) return
    if (reduced) return void setTurned(total)
    if (turned >= total) return
    const t = window.setTimeout(() => setTurned((n) => n + 1), turned === 0 ? 0 : DONE_STAGGER_MS)
    return () => window.clearTimeout(t)
  }, [ready, reduced, turned, total])
  const starterOn = starterSaved ?? (listed === 'failed' ? null : (listed ?? false))
  const glyph = (i: number, done: ReactNode) => (i < turned ? done : <Spinner />)
  return (
    <div className="m-ob-lines" data-testid="onboarding-lines" data-ready={turned >= total ? '' : undefined}>
      {set.map((s, i) => (
        <div key={s} className="m-ob-line">
          <span className="m-ob-glyph">
            {glyph(
              i,
              <span className="m-ob-check" aria-hidden="true">
                ✓
              </span>,
            )}
          </span>
          <span>{setLine(s, starterOn)}</span>
        </div>
      ))}
      {skipped.map((s, j) => (
        <div key={s} className="m-ob-line is-skipped">
          <span className="m-ob-glyph">{glyph(set.length + j, <span className="m-ob-skip">{COPY[s].icon === 'claudecode' ? <BrandMark id="claudecode" size={12} label="" /> : <Icon id={COPY[s].icon as IconId} />}</span>)}</span>
          <span>{`Skipped: ${SKIPPED_LINE[s]}`}</span>
        </div>
      ))}
      {!total && <div className="m-ob-line">Nothing changed. Mode works as it is.</div>}
    </div>
  )
}

/** Star on GitHub, under the Done lede: the repository in the browser (`url_open`, the opener's http(s) guard). */
function StarRepo() {
  const [error, setError] = useState<string | null>(null)
  const open = async () => {
    setError(null)
    try {
      await invoke('url_open', { url: REPO_URL })
    } catch {
      setError("Couldn't open the browser")
    }
  }
  return (
    <>
      <div className="m-inline">
        <Button onClick={() => void open()} data-testid="star-repo">
          <BrandMark id="github" size={16} label="" />
          Star on GitHub
        </Button>
      </div>
      {error && (
        <div className="m-ob-error" role="alert">
          {error}
        </div>
      )}
    </>
  )
}
