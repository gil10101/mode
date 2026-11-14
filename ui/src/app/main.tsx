import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import '../kit/fonts.css'
import '../kit/mode.css'
import './app.css'
import { isTauri } from '../tauri'
import { App } from './App'
import { installAppMock } from './mock'
import { installNotesMock } from '../notes'
import { installSettingsMock } from './mock-settings'
import { installTerminalMock } from '../terminal/mock'
import { installConnectorsMock } from './mock-connectors'
// package 11
import { installV1Mock } from './mock-v1'
import { installDictationMock } from './mock-dictation'
import { installMediaMock } from './mock-media'
import { connectDictation } from './dictation'
import { connectAppContext } from './app-context'
import { markShell } from '../state/shell'

// The native title bar takes the traffic dots' slot (03 §1 window chrome).
markShell()

// Outside Tauri (vite dev / preview, Playwright) the browser mock of
// src/tauri.ts answers `invoke`; seed it with the sample sessions and the
// app window's commands before the first render.
if (!isTauri()) {
  installAppMock()
  const vault = installNotesMock()
  installSettingsMock({ vault })
  installTerminalMock()
  installConnectorsMock({ capture: (text) => vault.capture(text) })
  // package 11: Chat, Automations, theme packs, the palette index
  installV1Mock({ vault })
  installDictationMock()
  installMediaMock()
}

// Dictated words land at the caret of the focused field (XC-36); the core hears what is on screen (XC-37).
void connectDictation()
connectAppContext()

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
