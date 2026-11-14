// Max rows, fold and overflow of the Sessions tab (02 §7.3–§7.4): the list is
// a window onto the first N rows, never scrolls, and drops rows into
// "+N more · open in app" when the panel's height budget runs out.
import type { RosterRow } from '../state/contract'

/** Display groups: `unread` and `done` share the "finished" group (ADR-0032 in docs/13-decisions.md, wireframe N-4), the fold row ends it. */
export const GROUP_ORDER = ['waiting', 'working', 'failed', 'finished', 'idle', 'untracked'] as const

export function groupOf(state: string): string {
  return state === 'unread' || state === 'done' ? 'finished' : state
}

/** 02 §6.4 heights in logical px. */
export const H = {
  header: 36,
  tabs: 32,
  row: 32,
  label: 20,
  fold: 28,
  overflow: 28,
  capture: 45,
  permissionCard: 76,
  questionBase: 56,
  questionOption: 26,
} as const

export type SessionItem =
  | { kind: 'label'; state: string }
  | { kind: 'row'; row: RosterRow; card: boolean }
  | { kind: 'fold'; count: number; open: boolean }
  | { kind: 'overflow'; count: number }

export interface SessionsLayout {
  items: SessionItem[]
  /** Ids of the visible rows, in order (keyboard navigation). */
  rowIds: string[]
  hidden: number
}

export interface LayoutOptions {
  maxRows: number
  /** Height available for the list (the frame maximum minus header, tabs and capture). */
  maxHeight?: number
  foldOpen?: boolean
}

export function groupLabel(state: string): string {
  return state === 'unread' || state === 'done' ? 'finished' : state
}

export function cardHeight(row: RosterRow): number {
  if (row.state !== 'waiting' || !row.pending?.answerable) return 0
  if (row.pending.kind === 'permission') return H.permissionCard
  if (row.pending.kind === 'question') return H.questionBase + H.questionOption * (row.pending.questions?.[0]?.options?.length ?? 0)
  return 0
}

export function hasCard(row: RosterRow): boolean {
  return cardHeight(row) > 0
}

export function layoutSessions(rows: RosterRow[], { maxRows, maxHeight = Infinity, foldOpen = false }: LayoutOptions): SessionsLayout {
  const groups = new Map<string, RosterRow[]>()
  for (const state of GROUP_ORDER) groups.set(state, [])
  for (const r of rows) {
    const g = groupOf(r.state)
    if (!groups.has(g)) groups.set(g, [])
    groups.get(g)!.push(r)
  }
  // Inside "finished": unread first, then acknowledged, then the fold (02 §7.3).
  const finished = groups.get('finished') ?? []
  const folded = finished.filter((r) => r.state === 'done' && r.folded)
  groups.set('finished', [...finished.filter((r) => r.state === 'unread'), ...finished.filter((r) => r.state === 'done' && !r.folded)])

  const items: SessionItem[] = []
  const rowIds: string[] = []
  let height = 0
  let visible = 0
  let hidden = 0
  let full = false

  const fits = (h: number) => visible < maxRows && height + h <= maxHeight

  for (const [state, list] of groups) {
    const isDone = state === 'finished'
    if (!list.length && !(isDone && folded.length)) continue
    let labelled = false
    const lead = () => (labelled ? 0 : H.label)
    const label = () => {
      if (labelled) return
      items.push({ kind: 'label', state })
      height += H.label
      labelled = true
    }
    const place = (row: RosterRow) => {
      if (full) {
        hidden++
        return
      }
      const card = cardHeight(row)
      if (!fits(lead() + H.row + card)) {
        full = true
        hidden++
        return
      }
      label()
      items.push({ kind: 'row', row, card: card > 0 })
      rowIds.push(row.id)
      height += H.row + card
      visible++
    }
    for (const row of list) place(row)
    if (isDone && folded.length) {
      if (!full && fits(lead() + H.fold)) {
        label()
        items.push({ kind: 'fold', count: folded.length, open: foldOpen })
        height += H.fold
        visible++
        if (foldOpen) for (const row of folded) place(row)
      } else {
        full = true
        hidden += foldOpen ? folded.length : 1
      }
    }
  }
  if (hidden > 0) items.push({ kind: 'overflow', count: hidden })
  return { items, rowIds, hidden }
}

/** Height budget for the list from the frame maximum of 02 §6 (`notch` / `top-bar`: min(480, 60 %); `right-edge`: 70 %). */
export function listBudget(placement: string, workAreaHeight?: number): number {
  const screen = workAreaHeight ?? 800
  const max = placement === 'right-edge' ? Math.floor(screen * 0.7) : Math.min(480, Math.floor(screen * 0.6))
  return max - H.header - H.tabs - H.capture - H.overflow
}
