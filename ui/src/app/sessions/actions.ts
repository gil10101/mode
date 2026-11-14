// The roster and header actions of 03 §3.4 / §3.6 in one place, so a row's
// ⋮, the detail panel and the session header offer the same verbs.
import { create } from 'zustand'
import { invoke } from '../../tauri'
import { cmdErrorText, type Decision, type Pending } from '../../state/contract'
import { isLaunchPlaceholder, type AppRosterRow, type SnoozePreset } from '../../state/contract-sessions'
import { canDismiss, canResume, canSnooze, useSessionsStore, type DecisionKind } from '../../state/sessions-store'
import { navigate } from '../router'
import { copyText, type MenuEntry } from '../ui'

/* ---- dialogs ----------------------------------------------------------------- */

export interface DialogState {
  launch: boolean
  /** The agent profile the Launch dialog pre-selects (03 §7E.1 "Launch session"). */
  launchProfile: string | null
  resume: string | null
  note: string | null
  openLaunch(profileId?: string): void
  openResume(id: string): void
  openNote(id: string): void
  close(): void
}

export const useDialogs = create<DialogState>((set) => ({
  launch: false,
  launchProfile: null,
  resume: null,
  note: null,
  openLaunch: (profileId) => set({ launch: true, launchProfile: profileId ?? null, resume: null, note: null }),
  openResume: (id) => set({ launch: false, resume: id, note: null }),
  openNote: (id) => set({ launch: false, resume: null, note: id }),
  close: () => set({ launch: false, launchProfile: null, resume: null, note: null }),
}))

/* ---- verbs --------------------------------------------------------------------- */

interface RowLike {
  id: string
  state: string
  launch_mode: string
  cwd: string
  ended_at?: string | null
  dismissed_at?: string | null
}

/** The Terminal tab's size before it exists (14 px mono ≈ 8.4 × 17 px cells in the main region), for `session_launch` / `session_resume`; `pty_open` resizes to the real tab. */
export function estimateTerminalSize(width = typeof window === 'undefined' ? 1280 : window.innerWidth, height = typeof window === 'undefined' ? 800 : window.innerHeight): { cols: number; rows: number } {
  const cols = Math.floor((width - 240 - 48) / 8.4)
  const rows = Math.floor((height - 180) / 17)
  return { cols: Math.max(40, Math.min(400, cols)), rows: Math.max(10, Math.min(200, rows)) }
}

export function openSession(id: string) {
  navigate({ view: 'sessions', id, tab: 'overview' })
}

/** Embedded → the Terminal tab; external / untracked → the core finds the terminal (05 §9.3). */
export function jumpToTerminal(row: RowLike) {
  if (row.launch_mode === 'embedded' && !row.ended_at) {
    navigate({ view: 'sessions', id: row.id, tab: 'terminal' })
    return
  }
  void invoke('session_jump', { session_id: row.id }).catch((e) => useSessionsStore.getState().showNotice(`Couldn't find the terminal: ${cmdErrorText(e)}`))
}

export function decisionFor(kind: Exclude<DecisionKind, 'answer'>): Decision {
  switch (kind) {
    case 'allow':
      return { behavior: 'allow' }
    case 'always':
      return { behavior: 'allow', always: true }
    case 'deny':
      return { behavior: 'deny' }
  }
}

export function decide(sessionId: string, kind: Exclude<DecisionKind, 'answer'>, pending: Pending | null | undefined) {
  return useSessionsStore.getState().decide(sessionId, kind, decisionFor(kind), pending?.received_at ?? null)
}

export function answer(sessionId: string, answers: { question: string; answer: string }[], pending: Pending | null | undefined) {
  return useSessionsStore.getState().decide(sessionId, 'answer', { answers }, pending?.received_at ?? null)
}

export function revealPath(path: string) {
  void invoke('plugin:opener|reveal_item_in_dir', { path }).catch((e) => useSessionsStore.getState().showNotice(`Couldn't reveal it: ${cmdErrorText(e)}`))
}

export function openPath(path: string) {
  void invoke('plugin:opener|open_path', { path }).catch((e) => useSessionsStore.getState().showNotice(`Couldn't open it: ${cmdErrorText(e)}`))
}

const SNOOZE: { preset: SnoozePreset; label: string }[] = [
  { preset: 'later_today', label: 'Snooze until later today' },
  { preset: 'tomorrow', label: 'Snooze until tomorrow' },
  { preset: 'next_week', label: 'Snooze until next week' },
]

export interface MenuContext {
  history?: boolean
  /** `false` on the detail screen itself. */
  open?: boolean
  /** Omit the verb shown as the primary button. */
  primary?: string
  /** The header's ⋮ (copy ids, transcript) vs a row's ⋮. */
  header?: { claude_session_id: string; transcript_path: string | null }
}

/** Every secondary action of a session (03 §3.4, §3.6); menu rows print no keys (10 §12.4). */
export function sessionMenu(row: AppRosterRow, ctx: MenuContext = {}): MenuEntry[] {
  const s = useSessionsStore.getState()
  const d = useDialogs.getState()
  const out: MenuEntry[] = []
  const add = (id: string, label: string, run: () => void, extra: Partial<Extract<MenuEntry, { run: () => void }>> = {}) => {
    if (ctx.primary === id) return
    out.push({ id, label, run, ...extra })
  }
  if (ctx.open !== false) add('open', 'Open', () => openSession(row.id))
  if (row.state === 'waiting' && row.pending?.answerable && row.pending.kind === 'permission') {
    add('allow', 'Allow', () => void decide(row.id, 'allow', row.pending))
    add('deny', 'Deny', () => void decide(row.id, 'deny', row.pending))
    add('always', 'Always', () => void decide(row.id, 'always', row.pending))
  }
  if (row.state === 'unread') add('acknowledge', 'Acknowledge', () => void s.acknowledge(row.id))
  if (!ctx.history && !row.ended_at) add('jump', 'Jump to terminal', () => jumpToTerminal(row))
  if (canResume(row, ctx.history)) add('resume', 'Resume', () => d.openResume(row.id))
  if (!ctx.history && canSnooze(row)) {
    out.push({ kind: 'sep', id: 'sep-snooze' })
    for (const p of SNOOZE) add(`snooze-${p.preset}`, p.label, () => void s.snooze(row.id, p.preset))
  }
  out.push({ kind: 'sep', id: 'sep-note' })
  add('note', 'Note from session', () => d.openNote(row.id))
  add('copy-path', 'Copy repo path', () => copyText(row.cwd, 'Path copied'))
  if (ctx.header) {
    const h = ctx.header
    // A launched row carries `mode-launch:<uuid>` until its first hook: not a Claude Code session id.
    if (!isLaunchPlaceholder(h.claude_session_id)) add('copy-id', 'Copy session id', () => copyText(h.claude_session_id, 'Session id copied'))
    if (h.transcript_path) {
      const tp = h.transcript_path
      add('copy-transcript', 'Copy transcript path', () => copyText(tp, 'Path copied'))
      add('reveal-transcript', 'Reveal transcript', () => revealPath(tp))
    }
    add('open-folder', 'Open folder', () => openPath(row.cwd))
  }
  if (!ctx.history && canDismiss(row)) {
    out.push({ kind: 'sep', id: 'sep-dismiss' })
    add('dismiss', 'Dismiss', () => void s.dismiss(row.id))
  }
  // Drop a trailing or doubled separator.
  return out.filter((e, i, all) => e.kind !== 'sep' || (i > 0 && i < all.length - 1 && all[i - 1].kind !== 'sep'))
}
