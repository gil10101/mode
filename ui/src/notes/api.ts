// Typed `invoke` for the notes commands (contract.ts `Commands`, shapes in
// contract-notes.ts). Everything goes through `ui/src/tauri.ts`, so the
// browser mock (`installNotesMock`) answers outside Tauri.
import type { Commands } from '../state/contract'
import { invoke } from '../tauri'

export type Cmd = keyof Commands

export function call<K extends Cmd>(cmd: K, ...args: Commands[K]['args'] extends undefined ? [] : [Commands[K]['args']]): Promise<Commands[K]['result']> {
  return invoke<Commands[K]['result']>(cmd, args[0] as Record<string, unknown> | undefined)
}
