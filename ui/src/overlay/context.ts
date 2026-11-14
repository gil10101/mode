// What every overlay view needs from the root: the machine, the clock, the
// placement, the pill model, the mascot badge and the action dispatcher.
import { createContext, useContext, type MutableRefObject, type RefObject } from 'react'
import type { SessionState } from '../kit/types'
import type { OverlayTab, Placement, RosterRow } from '../state/contract'
import type { Decided } from '../state/overlay-store'
import type { ActionId, ActExtra } from './actions'
import type { MachineState } from './machine'
import type { BotCanvas } from './NotchBot'
import type { PillModel } from './pill'
import type { Runner } from './runner'

export interface OverlayCtx {
  runner: Runner
  m: MachineState
  /** The clock the elapsed labels tick on (60 s in `compact`, 1 s in `expanded`). */
  now: number
  rowsAt: number
  placement: Placement
  notch: { width: number; height: number }
  dark: boolean
  showMascot: boolean
  pill: PillModel | null
  badge: SessionState | undefined
  act: (id: ActionId, row?: RosterRow, extra?: ActExtra) => void
  captureRef: RefObject<HTMLInputElement | null>
  /** The resting mascot's canvas while the peek draws it (the root feeds it the core's pointer moves). */
  botRef: MutableRefObject<BotCanvas | null>
  /** `null` while no card or row is linked to the capture box. */
  linkedRow: RosterRow | null
  setTab: (tab: OverlayTab) => void
  /** Opens the one ⋮ menu of the overlay under `anchor` (drawn at the root, outside the clipped frame). */
  openMenu: (anchor: HTMLElement, items: MenuSpec[], onSelect: (id: string) => void) => void
  closeMenu: () => void
  /** The decision already sent for a waiting session, if any (buttons stay disabled until the next push). */
  decidedFor: (sessionId: string) => Decided | undefined
  /** The capture box text lives at the root: the key handler needs to know whether it is empty. */
  capture: { value: string; set: (value: string) => void; save: (open: boolean) => void; unlink: () => void }
}

export interface MenuSpec {
  id: string
  label: string
  disabled?: boolean
}

export const OverlayContext = createContext<OverlayCtx | null>(null)

export function useOverlay(): OverlayCtx {
  const ctx = useContext(OverlayContext)
  if (!ctx) throw new Error('useOverlay outside <OverlayContext>')
  return ctx
}

/** `state_seconds` kept counting since the push that carried it. */
export function liveSeconds(row: RosterRow, ctx: Pick<OverlayCtx, 'now' | 'rowsAt'>): number {
  return row.state_seconds + (ctx.rowsAt ? Math.max(0, (ctx.now - ctx.rowsAt) / 1000) : 0)
}
