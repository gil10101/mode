// The desktop shell marks its root (03 §1 window chrome, 08 §6.2): `<html
// data-shell="tauri" data-os="macos|windows|linux">` before React mounts, so
// the kit can give the native title-bar controls the traffic dots' slot.
// Outside Tauri (vite, Playwright) nothing is set and the browser mock keeps
// matching the wireframes.
import { isTauri } from '../tauri'
import { currentOs } from './settings-store'

export function markShell(root: HTMLElement = document.documentElement, inTauri: boolean = isTauri()): void {
  if (!inTauri) return
  root.dataset.shell = 'tauri'
  root.dataset.os = currentOs()
}
