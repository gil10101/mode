import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import '../kit/fonts.css'
import '../kit/mode.css'
import './overlay.css'
import { Overlay } from './Overlay'
// package 11: theme packs — the resolved `--m-*` map of `theme.changed` on this webview's :root too (10 §3.1)
import { connectThemeTokens } from '../state/theme-store'
import { invoke, isTauri } from '../tauri'
import { connectDictation } from '../app/dictation'
import { installDictationMock } from '../app/mock-dictation'
import { installMediaMock } from '../app/mock-media'
import { markShell } from '../state/shell'

markShell()
// Outside Tauri the browser mock answers the dictation and Now playing commands (Playwright drives it).
if (!isTauri()) {
  installDictationMock()
  installMediaMock()
}
void connectThemeTokens()
// Dictated words land in the capture box when it has focus (XC-36).
void connectDictation()
// `tauri dev` only (08 §12): the Vite server's `/__mode/say?text=…` arrives here and goes through the core's real route.
if (import.meta.hot && isTauri()) import.meta.hot.on('mode:say', (d: { text?: string }) => void invoke('dictation_say', { text: d.text ?? '' }).catch(() => {}))

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Overlay />
  </StrictMode>,
)
