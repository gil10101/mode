// One emulator bound to one session's PTY (05 §8.3, 03 §3.8): opens xterm in
// the pane, joins the scrollback replay of `pty_open` with live `pty.output`,
// sends keystrokes through `pty_write` (in order), forwards resizes through
// `pty_resize` (debounced), routes the reserved keys, copies on select,
// pastes through bracketed paste, opens links through the opener, honors
// OSC 52 writes, and follows the theme tokens. React-free; Terminal.tsx owns
// its lifetime.
import type { IDisposable, ITerminalOptions, Terminal as XTerm } from '@xterm/xterm'
import type { FitAddon } from '@xterm/addon-fit'
import type { SearchAddon } from '@xterm/addon-search'
import { invoke, listen } from '../tauri'
import { SCROLLBACK_LINES, TERMINAL_EVENTS, type PtyOpenResult, type PtyOutput } from '../state/contract-terminal'
import { base64ToBytes, binaryToBytes, textToBytes } from './base64'
import { routeKey, type Platform } from './keys'
import { linkTarget, parseOsc52, sanitizePaste } from './osc'
import { appearanceOf, resolvePalette, searchDecorations } from './palette'
import { terminals, type TerminalProbe } from './registry'
import { createResizeSender, type ResizeSender } from './resize'
import { createWriteQueue, OutputJoin, type WriteQueue } from './stream'
import type { Xterm } from './xterm'

/** 10 §5.11: 14 px `font.mono`, block cursor, no bold-as-bright. */
export const FONT_SIZE = 14
/** Copy on select waits for the drag to settle. */
const COPY_DELAY_MS = 150

export type Renderer = 'webgl' | 'dom'

export interface ControllerHooks {
  onFind(): void
  onLeave(): void
  onRenderer(r: Renderer): void
  onAttached(r: PtyOpenResult): void
  onAttachFailed(message: string): void
  onSearchResults(index: number, count: number): void
  openUrl(url: string): void
  openPath(path: string): void
}

function errorText(e: unknown): string | null {
  if (typeof e === 'string') return e
  if (e && typeof e === 'object' && 'message' in e && typeof (e as { message: unknown }).message === 'string') return (e as { message: string }).message
  return null
}

export class TerminalController {
  private term: XTerm | null = null
  private search: SearchAddon | null = null
  private cleanups: (() => void)[] = []
  private disposed = false
  private readOnly = false
  private renderer: Renderer = 'dom'
  private readonly join = new OutputJoin()
  private readonly writes: WriteQueue
  private readonly resize: ResizeSender

  constructor(
    private readonly x: Xterm,
    private readonly sessionId: string,
    private readonly pane: HTMLElement,
    private readonly host: HTMLElement,
    private readonly platform: Platform,
    private readonly hooks: ControllerHooks,
  ) {
    this.writes = createWriteQueue((data) => invoke('pty_write', { session_id: sessionId, data }))
    this.resize = createResizeSender((cols, rows) => void invoke('pty_resize', { session_id: sessionId, cols, rows }).catch(() => {}))
  }

  private palette() {
    const style = getComputedStyle(this.pane)
    return resolvePalette((name) => style.getPropertyValue(name), appearanceOf(style))
  }

  private setRenderer(r: Renderer) {
    this.renderer = r
    this.hooks.onRenderer(r)
  }

  private openLink(uri: string) {
    const t = linkTarget(uri)
    if (t.kind === 'url') this.hooks.openUrl(t.url)
    else if (t.kind === 'path') this.hooks.openPath(t.path)
  }

  private on(d: IDisposable | (() => void)) {
    this.cleanups.push(typeof d === 'function' ? d : () => d.dispose())
  }

  /** Opens the emulator and attaches to the PTY. Throws only when xterm itself cannot start. */
  async start(): Promise<void> {
    const { Terminal, FitAddon, SearchAddon, Unicode11Addon, WebLinksAddon, WebglAddon } = this.x
    const style = getComputedStyle(this.pane)
    const options: ITerminalOptions = {
      allowProposedApi: true,
      scrollback: SCROLLBACK_LINES,
      fontFamily: style.getPropertyValue('--m-font-mono').trim() || 'monospace',
      fontSize: FONT_SIZE,
      cursorStyle: 'block',
      cursorBlink: false,
      drawBoldTextInBrightColors: false,
      theme: this.palette().theme,
      disableStdin: this.readOnly,
      // ConPTY on Windows 11 reflows; without this hint xterm turns reflow off.
      windowsPty: this.platform === 'windows' ? { backend: 'conpty', buildNumber: 22000 } : undefined,
      linkHandler: {
        activate: (_e, uri) => this.openLink(uri),
        hover: (_e, uri) => this.host.setAttribute('title', uri),
        leave: () => this.host.removeAttribute('title'),
        allowNonHttpProtocols: true,
      },
    }
    const term = new Terminal(options)
    this.term = term
    this.on(() => term.dispose())
    term.open(this.host)

    const fit = new FitAddon()
    term.loadAddon(fit)
    const search = new SearchAddon()
    this.search = search
    term.loadAddon(search)
    this.on(search.onDidChangeResults((r) => this.hooks.onSearchResults(r.resultIndex, r.resultCount)))
    term.loadAddon(new WebLinksAddon((_e, uri) => this.openLink(uri)))
    try {
      term.loadAddon(new Unicode11Addon())
      term.unicode.activeVersion = '11'
    } catch {
      /* keep the default widths */
    }
    // WebGL first, the DOM renderer when WebGL is missing or its context is lost.
    try {
      const gl = new WebglAddon()
      gl.onContextLoss(() => {
        gl.dispose()
        this.setRenderer('dom')
      })
      term.loadAddon(gl)
      this.setRenderer('webgl')
    } catch {
      this.setRenderer('dom')
    }

    this.bindInput(term)
    this.bindLayout(term, fit)
    this.bindTheme(term)
    this.register(term)
    await this.attach(term)
  }

  private bindInput(term: XTerm) {
    this.on(term.onData((d) => !this.readOnly && this.writes.push(textToBytes(d))))
    this.on(term.onBinary((d) => !this.readOnly && this.writes.push(binaryToBytes(d))))

    // Reserved keys (03 §3.8): app keys are left to bubble; the terminal's own commands act on keydown.
    term.attachCustomKeyEventHandler((ev) => {
      const route = routeKey(ev, this.platform)
      if (route === 'pty') return !this.readOnly
      if (ev.type === 'keydown') {
        if (route === 'find') {
          ev.preventDefault()
          this.hooks.onFind()
        } else if (route === 'leave') {
          ev.preventDefault()
          this.hooks.onLeave()
        }
        // `paste`: the browser's paste event follows and xterm brackets it.
      }
      return false
    })
    // Keys the terminal consumed never reach the app's shortcuts (Ctrl+K on Windows is the PTY's).
    const stop = (ev: KeyboardEvent) => {
      const t = ev.target as Element | null
      if (!t?.closest?.('.xterm')) return
      const route = routeKey(ev, this.platform)
      if (route === 'app' || (route === 'pty' && this.readOnly)) return
      ev.stopPropagation()
    }
    this.pane.addEventListener('keydown', stop)
    this.on(() => this.pane.removeEventListener('keydown', stop))

    // Pastes: xterm brackets text itself; text carrying ESC is cleaned first, and a paste with no
    // text (an image) becomes Ctrl+V for the CLI, which reads the clipboard itself.
    const paste = (ev: ClipboardEvent) => {
      if (this.readOnly || !ev.clipboardData) return
      const text = ev.clipboardData.getData('text/plain')
      const clean = text ? sanitizePaste(text) : null
      if (text && clean === null) return
      ev.preventDefault()
      ev.stopPropagation()
      if (clean !== null) term.paste(clean)
      else if (this.platform !== 'mac' && ev.clipboardData.types.length) this.writes.push(new Uint8Array([0x16]))
    }
    this.host.addEventListener('paste', paste, true)
    this.on(() => this.host.removeEventListener('paste', paste, true))

    // Copy on select.
    let copyTimer: ReturnType<typeof setTimeout> | undefined
    this.on(
      term.onSelectionChange(() => {
        clearTimeout(copyTimer)
        copyTimer = setTimeout(() => {
          const text = term.getSelection()
          if (text) void navigator.clipboard?.writeText(text).catch(() => {})
        }, COPY_DELAY_MS)
      }),
    )
    this.on(() => clearTimeout(copyTimer))

    // OSC 52: writes allowed, reads refused (05 §8.3).
    this.on(
      term.parser.registerOscHandler(52, (data) => {
        const r = parseOsc52(data)
        if (r.kind === 'write') void navigator.clipboard?.writeText(r.text).catch(() => {})
        return true
      }),
    )
  }

  private bindLayout(term: XTerm, fit: FitAddon) {
    this.on(term.onResize(({ cols, rows }) => this.resize.schedule(cols, rows)))
    const refit = () => {
      if (this.disposed) return
      try {
        fit.fit()
      } catch {
        /* hidden or not measurable yet */
      }
    }
    refit()
    if (typeof ResizeObserver !== 'undefined') {
      const ro = new ResizeObserver(() => refit())
      ro.observe(this.host)
      this.on(() => ro.disconnect())
    }
    this.on(() => this.resize.dispose())
  }

  private bindTheme(term: XTerm) {
    const retheme = () => {
      if (!this.disposed) term.options.theme = this.palette().theme
    }
    if (typeof MutationObserver !== 'undefined') {
      const mo = new MutationObserver(retheme)
      mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'class', 'style'] })
      this.on(() => mo.disconnect())
    }
    const mq = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null
    if (mq?.addEventListener) {
      mq.addEventListener('change', retheme)
      this.on(() => mq.removeEventListener('change', retheme))
    }
  }

  private register(term: XTerm) {
    const probe: TerminalProbe = {
      text: () => {
        const b = term.buffer.active
        const lines: string[] = []
        for (let y = 0; y < b.length; y++) lines.push(b.getLine(y)?.translateToString(true) ?? '')
        while (lines.length && !lines[lines.length - 1]) lines.pop()
        return lines.join('\n')
      },
      renderer: () => this.renderer,
      colorOf: (needle) => {
        const b = term.buffer.active
        for (let y = 0; y < b.length; y++) {
          const line = b.getLine(y)
          const x = line?.translateToString(true).indexOf(needle) ?? -1
          if (!line || x < 0) continue
          const cell = line.getCell(x)
          if (!cell) return null
          if (cell.isFgPalette()) return `palette:${cell.getFgColor()}`
          if (cell.isFgRGB()) return `#${cell.getFgColor().toString(16).padStart(6, '0')}`
          return 'default'
        }
        return null
      },
      size: () => ({ cols: term.cols, rows: term.rows }),
      theme: () => {
        const t = term.options.theme ?? {}
        return { background: t.background, foreground: t.foreground, cursor: t.cursor, red: t.red }
      },
    }
    terminals.set(this.sessionId, probe)
    this.on(() => {
      if (terminals.get(this.sessionId) === probe) terminals.delete(this.sessionId)
    })
  }

  /** Subscribe to output first, then ask for the replay, so nothing falls between the two. */
  private async attach(term: XTerm) {
    const off = await listen<PtyOutput>(TERMINAL_EVENTS.output, (p) => {
      if (!p || p.session_id !== this.sessionId || this.disposed) return
      let bytes: Uint8Array
      try {
        bytes = base64ToBytes(p.data)
      } catch {
        return
      }
      const out = this.join.push({ data: bytes, offset: p.offset })
      if (out) term.write(out)
    })
    if (this.disposed) {
      off()
      return
    }
    this.on(off)
    try {
      const r = await invoke<PtyOpenResult>('pty_open', { session_id: this.sessionId, cols: term.cols, rows: term.rows })
      if (this.disposed) return
      const data = r?.data ? base64ToBytes(r.data) : new Uint8Array()
      for (const run of this.join.replay(data, r?.offset ?? 0)) term.write(run)
      if (r?.cols && r?.rows) this.resize.known(r.cols, r.rows)
      if (!r?.cols || r.cols !== term.cols || r.rows !== term.rows) this.resize.schedule(term.cols, term.rows)
      this.hooks.onAttached(r)
    } catch (e) {
      if (!this.disposed) this.hooks.onAttachFailed(errorText(e) ?? "Couldn't attach to the terminal")
    }
  }

  setReadOnly(on: boolean) {
    this.readOnly = on
    if (this.term) this.term.options.disableStdin = on
  }

  focus() {
    this.term?.focus()
  }

  /** Find (03 §3.8). `incremental` keeps the current match while the query grows. */
  find(query: string, direction: 'next' | 'previous', incremental = false): boolean {
    if (!this.search || !this.term) return false
    if (!query) {
      this.search.clearDecorations()
      this.hooks.onSearchResults(-1, 0)
      return false
    }
    const decorations = searchDecorations(this.term.options.theme ?? {}) ?? undefined
    return direction === 'next' ? this.search.findNext(query, { decorations, incremental }) : this.search.findPrevious(query, { decorations })
  }

  clearFind() {
    this.search?.clearDecorations()
    this.term?.clearSelection()
  }

  dispose() {
    if (this.disposed) return
    this.disposed = true
    // The core keeps the PTY and its scrollback; it only stops streaming to this pane.
    void invoke<void>('pty_detach', { session_id: this.sessionId }).catch(() => {})
    this.writes.dispose()
    for (const c of this.cleanups.splice(0).reverse()) {
      try {
        c()
      } catch {
        /* already gone */
      }
    }
    this.term = null
    this.search = null
  }
}
