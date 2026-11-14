// Every action the overlay can take on an item (02 §5.2, §7.5, §10), routed
// to the core's commands. A decision is sent once per pending request; the
// store keeps it so the buttons stay disabled until the next `sessions.changed`.
import type { Decision, DecideResult, OverlayTab, RosterRow } from '../state/contract'
import type { OverlayData } from '../state/overlay-store'
import type { RowAction } from './machine'
import { flashText, type PillActionId } from './pill'
import type { Runner } from './runner'

export type ActionId =
  | PillActionId
  | RowAction
  | 'acknowledge'
  | 'answer_in_app'
  | 'install_hooks'
  | 'fix_hooks'
  | 'launch'
  | 'connect'
  | 'open_tab'
  | 'expand_app'
  | 'settings'

export interface ActExtra {
  answers?: { question: string; answer: string }[]
  tab?: OverlayTab
  id?: string
  /** `open_dictation`, `open_flash`: the app view the pill's Open lands on (02 §5.4, §5.6). */
  view?: string
}

export interface ActorDeps {
  runner: Runner
  store: { getState(): OverlayData }
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>
  /** Puts key input in the capture box (keyboard session). */
  focusCapture: () => void
}

export type DecisionKind = 'allow' | 'deny' | 'always' | 'answer'

/** The primary action of a session row (02 §7.5). */
export function primaryFor(row: RosterRow): ActionId {
  switch (row.state) {
    case 'waiting': {
      const kind = row.pending?.kind ?? row.waiting_kind
      if (row.pending?.answerable === false) return 'jump'
      if (kind === 'permission') return 'allow'
      if (kind === 'question') return 'answer'
      return 'jump'
    }
    case 'failed':
    case 'done':
      return 'open_in_app'
    case 'unread':
      return 'acknowledge'
    default:
      return 'jump'
  }
}

export interface MenuEntry {
  id: ActionId
  label: string
}

const JUMP: MenuEntry = { id: 'jump', label: 'Jump to terminal' }
const OPEN: MenuEntry = { id: 'open_in_app', label: 'Open in app' }
const NOTE: MenuEntry = { id: 'note', label: 'Note from session' }
const DISMISS: MenuEntry = { id: 'dismiss', label: 'Dismiss' }

/** The ⋮ entries of a session row (02 §7.5). */
export function menuFor(row: RosterRow): MenuEntry[] {
  switch (row.state) {
    case 'waiting': {
      const kind = row.pending?.kind ?? row.waiting_kind
      if (kind === 'permission' && row.pending?.answerable !== false) return [{ id: 'deny', label: 'Deny' }, { id: 'always', label: 'Always' }, JUMP, OPEN]
      return [JUMP, OPEN]
    }
    case 'working':
    case 'idle':
      return [OPEN, NOTE]
    case 'failed':
      return [JUMP, DISMISS, NOTE]
    case 'unread':
      return [DISMISS, JUMP, NOTE]
    case 'done':
      return [DISMISS]
    default:
      return [DISMISS, OPEN]
  }
}

export function decisionFor(kind: DecisionKind, answers?: ActExtra['answers']): Decision {
  switch (kind) {
    case 'allow':
      return { behavior: 'allow' }
    case 'deny':
      return { behavior: 'deny', message: 'Denied in Mode' }
    case 'always':
      return { behavior: 'allow', always: true }
    case 'answer':
      return { answers: answers ?? [] }
  }
}

export function outcomeFlash(kind: DecisionKind, outcome: DecideResult['outcome']): { text: string; keepPill: boolean } {
  switch (outcome) {
    case 'accepted':
      return { text: flashText(kind) ?? 'Done ✓', keepPill: false }
    case 'already_answered':
      return { text: 'Already answered', keepPill: false }
    case 'answered_in_terminal':
      return { text: 'Answered in terminal', keepPill: false }
    default:
      return { text: "Couldn't do that", keepPill: true }
  }
}

export function createActor(deps: ActorDeps) {
  const { runner, store, invoke } = deps

  async function decide(row: RosterRow, kind: DecisionKind, answers?: ActExtra['answers']) {
    const st = store.getState()
    if (st.decided[row.id]) return
    st.decide(row.id, kind, row.pending?.received_at ?? null)
    let outcome: DecideResult['outcome']
    try {
      const r = await invoke<DecideResult | undefined>('session_decide', { session_id: row.id, decision: decisionFor(kind, answers) })
      outcome = r?.outcome ?? 'accepted'
    } catch {
      outcome = 'failed'
    }
    store.getState().settle(row.id, outcome)
    runner.dispatch({ type: 'resolved', ...outcomeFlash(kind, outcome) })
  }

  async function open(view: OverlayTab | 'integrations' | 'settings' | 'chat' | (string & {}), id?: string) {
    try {
      await invoke('app_open', id ? { view, id } : { view })
    } catch {
      /* the core reports its own failures */
    }
    runner.dispatch({ type: 'leave' })
  }

  async function simple(cmd: string, row: RosterRow, text: string | null) {
    try {
      await invoke(cmd, { session_id: row.id })
      if (text) runner.dispatch({ type: 'resolved', text })
    } catch {
      runner.dispatch({ type: 'resolved', text: "Couldn't do that", keepPill: true })
    }
  }

  return function act(id: ActionId, row?: RosterRow, extra: ActExtra = {}) {
    switch (id) {
      case 'primary':
        if (row) act(primaryFor(row), row, extra)
        return
      case 'allow':
      case 'deny':
      case 'always':
        if (row?.pending?.kind === 'permission' && row.pending.answerable) void decide(row, id)
        return
      case 'answer':
        if (!row) return
        if (extra.answers) void decide(row, 'answer', extra.answers)
        else runner.dispatch({ type: 'click', target: 'pill', row: row.id })
        return
      case 'view':
        if (!row) return
        if (row.state === 'unread') void simple('session_acknowledge', row, flashText('acknowledge'))
        else runner.dispatch({ type: 'click', target: 'pill', row: row.id })
        return
      case 'acknowledge':
        if (row) {
          void simple('session_acknowledge', row, flashText('acknowledge'))
          if (row.state === 'unread') void open('sessions', row.id)
        }
        return
      case 'dismiss':
        if (row) void simple('session_dismiss', row, flashText('dismiss'))
        return
      case 'jump':
        if (row) {
          void simple('session_jump', row, null)
          runner.dispatch({ type: 'leave' })
        }
        return
      case 'open_in_app':
      case 'answer_in_app':
        void open('sessions', row?.id ?? extra.id)
        return
      case 'note':
        if (row) runner.dispatch({ type: 'focus_row', row: row.id })
        deps.focusCapture()
        return
      case 'install_hooks':
      case 'fix_hooks':
      case 'connect':
        void open('integrations')
        return
      case 'settings':
        void open('settings')
        return
      case 'launch':
        void open('sessions')
        return
      case 'open_dictation':
        // The captured note, or the Chat conversation that answered a "Hey Mode" request; the pill goes with it.
        if (extra.view && extra.id) {
          runner.dispatch({ type: 'dictation_clear' })
          void open(extra.view, extra.id)
        }
        return
      case 'open_flash':
        // The item an app-window flash names (02 §5.4); the flash goes with it.
        if (extra.view) {
          runner.dispatch({ type: 'flash_clear' })
          void open(extra.view, extra.id)
        }
        return
      case 'open_tab':
      case 'expand_app': {
        // The Music tab has no app view: its rows live under Settings › Overlay (Now playing).
        const tab = extra.tab ?? runner.get().tab
        if (tab === 'music') void open('settings', 'overlay')
        else void open(tab, extra.id)
        return
      }
      // Message, note and calendar actions arrive with their packages; until then they open the item in the app.
      case 'open_source':
      case 'read':
      case 'archive':
      case 'snooze':
      case 'done':
      case 'join':
      case 'prepare':
        void open(extra.tab ?? (id === 'join' || id === 'prepare' ? 'calendar' : id === 'done' ? 'notes' : 'inbox'), extra.id)
        return
    }
  }
}
