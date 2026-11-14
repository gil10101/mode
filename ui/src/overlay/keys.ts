// DOM key events → machine key actions (02 §8.4). The root listens in the
// capture phase; whatever the reducer consumes is prevented.
import type { KeyAction } from './machine-keys'

export interface KeyLike {
  key: string
  shiftKey: boolean
  metaKey: boolean
  ctrlKey: boolean
  altKey: boolean
}

export function toKeyAction(e: KeyLike, ctx: { rows: string[]; captureEmpty: boolean }): Omit<KeyAction, 'now'> | null {
  if (e.altKey) return null
  return { type: 'key', key: e.key, shift: e.shiftKey, mod: e.metaKey || e.ctrlKey, rows: ctx.rows, captureEmpty: ctx.captureEmpty }
}
