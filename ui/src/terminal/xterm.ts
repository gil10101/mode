// `@xterm/xterm` and its addons, loaded on first use so the app's first
// paint does not carry the emulator (the Terminal tab is one of six).
import '@xterm/xterm/css/xterm.css'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon } from '@xterm/addon-search'
import { Unicode11Addon } from '@xterm/addon-unicode11'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'

export const xterm = { Terminal, FitAddon, SearchAddon, Unicode11Addon, WebLinksAddon, WebglAddon }
export type Xterm = typeof xterm
