// The keyboard map of 02 §8.3–§8.4, applied only inside a keyboard session.
// Letters act on the focused row only, so typing in the capture box never
// triggers them; no key is ever printed in the overlay (ADR-0033).
import type { OverlayTab } from '../state/contract'
import { visibleTabs } from '../state/contract'
import type { Effect, MachineState, OpenedBy, RowAction, TimerKind } from './machine'

export interface KeyAction {
  type: 'key'
  /** `KeyboardEvent.key`. */
  key: string
  shift?: boolean
  /** ⌘ or Ctrl. */
  mod?: boolean
  /** The visible row ids, in order. */
  rows: string[]
  captureEmpty: boolean
  now: number
}

export interface Draft {
  s: MachineState
  effects: Effect[]
}

export interface KeyHelpers {
  collapse(d: Draft, reason: string): void
  expand(d: Draft, by: OpenedBy, now: number, o?: { tab?: OverlayTab; row?: string | null; keyboard?: boolean }): void
  startKeyboard(d: Draft, now: number, focus: boolean): void
  releaseKeyboard(d: Draft): void
  togglePin(d: Draft, now: number): void
  cancel(d: Draft, ...kinds: TimerKind[]): void
  schedule(d: Draft, kind: TimerKind, at: number): void
}

const LETTERS: Readonly<Record<string, RowAction>> = {
  a: 'allow',
  d: 'deny',
  A: 'always',
  j: 'jump',
  x: 'dismiss',
  n: 'note',
  r: 'read',
  e: 'archive',
  s: 'snooze',
  o: 'open_source',
  ' ': 'done',
}

function set(d: Draft, patch: Partial<MachineState>) {
  d.s = { ...d.s, ...patch }
}

/** Returns whether the key was consumed. */
export function reduceKey(d: Draft, a: KeyAction, h: KeyHelpers): boolean {
  const s = d.s
  if (!s.keyboard || s.state !== 'expanded') return false
  const idle = s.settings.auto_collapse_ms * 4
  if (idle > 0) h.schedule(d, 'keyboard_idle', a.now + idle)
  const inCapture = s.zone === 'capture'
  const typing = inCapture && !a.captureEmpty

  switch (a.key) {
    case 'Escape':
      if (typing) {
        d.effects.push({ type: 'clear_capture' })
        return true
      }
      if (s.pinned) set(d, { pinned: false })
      h.collapse(d, 'esc')
      return true
    case 'Tab':
      if (inCapture && !a.shift) {
        const row = s.focusRow ?? (s.pill?.ref && a.rows.includes(s.pill.ref) ? s.pill.ref : (a.rows[0] ?? null))
        set(d, { zone: 'rows', focusRow: row })
      } else set(d, { zone: 'capture' })
      return true
    case 'ArrowDown':
    case 'ArrowUp': {
      if (typing) return false
      if (!a.rows.length) return true
      const i = s.focusRow ? a.rows.indexOf(s.focusRow) : -1
      const next = a.key === 'ArrowDown' ? Math.min(i + 1, a.rows.length - 1) : Math.max(i < 0 ? 0 : i - 1, 0)
      set(d, { zone: 'rows', focusRow: a.rows[next] })
      return true
    }
    case 'ArrowLeft':
    case 'ArrowRight': {
      if (typing) return false
      const tabs = visibleTabs(s.settings)
      const i = tabs.indexOf(s.tab)
      const tab = tabs[(i + (a.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length]
      set(d, { tab, lastTab: tab, focusRow: null })
      return true
    }
    case '1':
    case '2':
    case '3':
    case '4':
    case '5': {
      if (typing) return false
      // 5 is Music, only while it shows (02 §8.4).
      const tab = visibleTabs(s.settings)[Number(a.key) - 1]
      if (!tab) return false
      set(d, { tab, lastTab: tab, focusRow: null })
      return true
    }
    case 'Enter':
      if (typing) {
        d.effects.push({ type: 'save_capture', open: !!a.mod })
        return true
      }
      if (s.focusRow) {
        d.effects.push({ type: 'act', action: a.mod ? 'open_in_app' : 'primary', row: s.focusRow })
        return true
      }
      // Enter on an empty box with no focused row releases key input (02 §9.3).
      h.releaseKeyboard(d)
      if (!s.pinned && !s.hovering) h.schedule(d, 'collapse_delay', a.now + s.settings.collapse_delay_ms)
      d.effects.push({ type: 'request', state: d.s.state, pinned: d.s.pinned, keyboard: false, reason: 'keyboard_released' })
      return true
  }
  if (inCapture) return false
  if (a.key === 'p' || a.key === 'P') {
    h.togglePin(d, a.now)
    return true
  }
  const action = LETTERS[a.shift && a.key.toLowerCase() === 'a' ? 'A' : a.key]
  if (action && s.focusRow) {
    d.effects.push({ type: 'act', action, row: s.focusRow })
    return true
  }
  return a.key.length === 1
}
