// The browser mock of dictation (XC-36, XC-37), installed by both windows'
// entry points outside Tauri: every `dictation_*` command and
// `app_context_set`, and the pushes the core would make (`dictation.state`
// at 10 Hz while "listening", `dictation.model` while a model downloads,
// `dictation.text` to this window). Playwright drives it through
// `window.__mode_mock.dictation` (`say`, `press`, `release`, `hold`, …) or
// pushes states directly with `window.__mode_mock.emit('dictation.state', …)`.
import { mock, type ModeMock } from '../tauri'
import { EVENTS, type AppContextKind, type DictationCitation, type DictationModel, type DictationState, type DictationStatus, type DictationSurface } from '../state/contract'

export const MB = 1_000_000

/** `mode_core::speech::models`, as the core lists it. */
export const MOCK_MODELS: readonly Omit<DictationModel, 'installed'>[] = [
  { id: 'tiny.en', label: 'tiny.en', size_bytes: 32 * MB, multilingual: false },
  { id: 'base.en', label: 'base.en', size_bytes: 57 * MB, multilingual: false },
  { id: 'small.en', label: 'small.en', size_bytes: 182 * MB, multilingual: false },
  { id: 'base', label: 'base', size_bytes: 57 * MB, multilingual: true },
]

export const DEFAULT_UTTERANCE = 'the retry policy is final'

/** The wake phrase rule of `mode_core::speech::wake::classify`, with its measured misheard forms (ADR-0064): the remainder is a request, else the whole utterance is dictation. */
export function classify(text: string): { kind: 'dictation'; text: string } | { kind: 'command'; text: string } {
  const m = /^\s*(?:(?:hey|hi|ok|okay|k|a\s?k)[\s,.-]*(?:mode|mod|moe|mood|maud|mo)|high[\s,.-]*(?:mode|mo)|mode)\b[\s,.:!]*(.*)$/is.exec(text)
  return m ? { kind: 'command', text: m[1].trim() } : { kind: 'dictation', text: text.trim() }
}

/** The answer the mock gives a "what's on my list today" request, citing the overlay sample's notes (02 §5.7). */
export const TODAY_REPLY = 'Today you have three tasks:\n- Fix flaky auth test [1], urgent, 2 of 5 steps done\n- Book dentist [2]\n- Draft v2 changelog [3], in progress'
export const TODAY_CITATIONS: DictationCitation[] = [
  { n: 1, kind: 'note', id: 'n1', title: 'Fix flaky auth test' },
  { n: 2, kind: 'note', id: 'n2', title: 'Book dentist' },
  { n: 3, kind: 'note', id: 'n4', title: 'Draft v2 changelog' },
]

/** The stream chunks of a reply: a few words each, as the core forwards Chat's deltas. */
export function replyChunks(text: string, words = 3): string[] {
  const parts = text.split(/(?<=\s)/)
  const out: string[] = []
  for (let i = 0; i < parts.length; i += words) out.push(parts.slice(i, i + words).join(''))
  return out
}

export function wordCount(text: string): number {
  return text.trim().split(/\s+/).filter(Boolean).length
}

export interface DictationMock {
  /** The next utterance `release` transcribes (once; then back to the default). */
  say(text: string): void
  press(): void
  release(): void
  cancel(): void
  /** Press, hold `ms` (default 400), say `text`, release; resolves when the state is `done` or `error`. */
  hold(text?: string, ms?: number): Promise<DictationState>
  /** The last state pushed. */
  state: DictationState
  /** The last `app_context_set`. */
  context: { kind: AppContextKind | null; id: string | null }
  installed: Set<string>
  available: boolean
  /** `dictation_mic_check`'s refusal, or null for ok. */
  micError: string | null
  /** The core's copy of the dictation settings (mirrored from `settings.changed`). */
  settings: { enabled: boolean; model: string; commands: boolean; unfocused: 'capture' | 'nothing'; hotkey: string; thread_id: string | null }
  /** A model download's length in ms (default 1500). */
  downloadMs: number
  /** Where a request's answer shows (02 §5.7); `null` decides as the core would: the pill in the app window, the card elsewhere. */
  surface: DictationSurface | null
  /** The gap between `answering` pushes in ms (default 120). */
  chunkMs: number
}

declare module '../tauri' {
  interface ModeMock {
    dictation?: DictationMock
  }
}

type Args = Record<string, unknown> | undefined

/** Registers the dictation commands on the browser mock (idempotent). */
export function installDictationMock(m: ModeMock = mock()): DictationMock {
  let timer: ReturnType<typeof setInterval> | null = null
  let pendingText: { text: string; words: number } | null = null
  let next: string | null = null
  let since = 0
  let seq = 0
  const waiters = new Set<(s: DictationState) => void>()

  const d: DictationMock = {
    state: { state: 'idle' },
    context: { kind: null, id: null },
    installed: new Set(),
    available: true,
    micError: null,
    settings: { enabled: false, model: 'base.en', commands: true, unfocused: 'capture', hotkey: 'Ctrl+Alt+D', thread_id: null },
    downloadMs: 1500,
    surface: null,
    chunkMs: 120,
    say(text) {
      next = text
    },
    press() {
      if (d.state.state === 'listening') return
      stopTicks()
      since = Date.now()
      push({ state: 'listening', since_ms: since, seconds: 0, level: 0 })
      let t = 0
      timer = setInterval(() => {
        t += 0.1
        // A voice-like rise and fall, 0..1.
        const level = Math.max(0, Math.min(1, 0.5 + 0.42 * Math.sin(t * 5.3) * Math.abs(Math.sin(t * 1.7))))
        push({ state: 'listening', since_ms: since, seconds: Math.floor(t + 1e-9), level: Math.round(level * 100) / 100 })
      }, 100)
    },
    release() {
      if (d.state.state !== 'listening') return
      stopTicks()
      const text = next ?? DEFAULT_UTTERANCE
      next = null
      const run = ++seq
      push({ state: 'transcribing', since_ms: Date.now() })
      setTimeout(() => {
        if (run !== seq) return
        const u = d.settings.commands ? classify(text) : ({ kind: 'dictation', text } as const)
        if (u.kind === 'command') {
          if (!u.text) {
            finish({ state: 'done', since_ms: Date.now(), target: 'chat', text: 'What would you like?', words: 0 })
            return
          }
          const thread = d.settings.thread_id ?? 'th-voice'
          d.settings.thread_id = thread
          const turn = `turn-voice-${run}`
          const surface: DictationSurface = d.surface ?? (/overlay/.test(location.pathname) ? 'card' : 'pill')
          const base = { target: 'chat' as const, thread_id: thread, turn_id: turn, surface }
          const today = /todo|to do|list|today/i.test(u.text)
          const reply = today ? TODAY_REPLY : `Here is what I found: ${u.text}`
          const citations = today ? TODAY_CITATIONS : []
          push({ state: 'thinking', since_ms: Date.now(), ...base })
          // Chat streams the answer after its steps (08 §4.4); the core forwards it as `answering`, the whole reply so far.
          const chunks = replyChunks(reply)
          let sofar = ''
          let i = 0
          const step = () => {
            if (run !== seq) return
            if (i < chunks.length) {
              sofar += chunks[i++]
              push({ state: 'answering', since_ms: Date.now(), ...base, reply: sofar })
              setTimeout(step, d.chunkMs)
              return
            }
            finish({ state: 'done', since_ms: Date.now(), ...base, reply, citations, text: firstWords(reply, 8), words: wordCount(u.text) })
          }
          setTimeout(step, 900)
          return
        }
        pendingText = { text: u.text, words: wordCount(u.text) }
        // Only the focused window gets the words; with nobody listening the field could not take them.
        if (m.emit(EVENTS.dictationText, { text: u.text }) === 0) inserted(false)
      }, 600)
    },
    cancel() {
      seq++
      stopTicks()
      pendingText = null
      push({ state: 'idle' })
    },
    hold(text, ms = 400) {
      if (text !== undefined) d.say(text)
      return new Promise((resolve) => {
        waiters.add(resolve)
        d.press()
        setTimeout(() => d.release(), ms)
      })
    },
  }

  function stopTicks() {
    if (timer) clearInterval(timer)
    timer = null
  }

  function push(s: DictationState) {
    d.state = s
    m.emit(EVENTS.dictation, s)
  }

  function finish(s: DictationState) {
    push(s)
    for (const w of Array.from(waiters)) w(s)
    waiters.clear()
    const run = seq
    // The core goes back to idle; the overlay has cleared its pill on its own by then.
    setTimeout(() => {
      if (run === seq && (d.state.state === 'done' || d.state.state === 'error')) push({ state: 'idle' })
    }, 2400)
  }

  function inserted(ok: boolean) {
    const p = pendingText
    if (!p) return
    pendingText = null
    if (ok) finish({ state: 'done', since_ms: Date.now(), target: 'field', words: p.words, text: firstWords(p.text, 5) })
    else if (d.settings.unfocused === 'capture') finish({ state: 'done', since_ms: Date.now(), target: 'note', note_id: `mock-note-dictation-${seq}`, words: p.words, text: firstWords(p.text, 5) })
    else finish({ state: 'done', since_ms: Date.now(), target: 'none', words: p.words })
  }

  const status = (): DictationStatus => ({
    enabled: d.settings.enabled,
    available: d.available,
    state: d.state,
    model: d.settings.model,
    model_installed: d.installed.has(d.settings.model),
    hotkey: d.settings.hotkey,
  })

  const on = (cmd: string, fn: (a: Record<string, unknown>) => unknown) => m.respond(cmd, (args: Args) => fn(args ?? {}))
  on('dictation_status', status)
  on('dictation_press', () => d.press())
  on('dictation_release', () => d.release())
  on('dictation_cancel', () => d.cancel())
  on('dictation_inserted', (a) => inserted(a.ok === true))
  on('dictation_models', () => ({ models: MOCK_MODELS.map((x) => ({ ...x, installed: d.installed.has(x.id) })) }))
  on('dictation_model_ensure', (a) => {
    const model = String(a.model)
    const spec = MOCK_MODELS.find((x) => x.id === model)
    if (!spec) throw { code: 'not_found', message: `no model ${model}` }
    const total = spec.size_bytes
    if (d.installed.has(model)) {
      m.emit(EVENTS.dictationModel, { model, downloaded: total, total, done: true })
      return undefined
    }
    const steps = Math.max(1, Math.round(d.downloadMs / 100))
    let i = 0
    m.emit(EVENTS.dictationModel, { model, downloaded: 0, total, done: false })
    const t = setInterval(() => {
      i++
      const downloaded = Math.round((total * i) / steps)
      const done = i >= steps
      if (done) {
        clearInterval(t)
        d.installed.add(model)
      }
      m.emit(EVENTS.dictationModel, { model, downloaded, total, done })
    }, 100)
    return undefined
  })
  on('dictation_model_remove', (a) => {
    d.installed.delete(String(a.model))
    return undefined
  })
  on('dictation_mic_check', () => (d.micError ? { ok: false, error: d.micError } : { ok: true }))
  on('app_context_set', (a) => {
    d.context = { kind: (a.kind as AppContextKind | null) ?? null, id: (a.id as string | null) ?? null }
    return undefined
  })

  // The core reads the dictation settings itself; mirror what the settings mock changes.
  m.subscribe(EVENTS.settings, (p) => {
    const s = (p ?? {}) as Record<string, unknown>
    if (typeof s['dictation.enabled'] === 'boolean') d.settings.enabled = s['dictation.enabled']
    if (typeof s['dictation.model'] === 'string') d.settings.model = s['dictation.model']
    if (typeof s['dictation.commands'] === 'boolean') d.settings.commands = s['dictation.commands']
    if (s['dictation.unfocused'] === 'capture' || s['dictation.unfocused'] === 'nothing') d.settings.unfocused = s['dictation.unfocused']
    if (typeof s['dictation.hotkey'] === 'string') d.settings.hotkey = s['dictation.hotkey']
  })

  m.dictation = d
  return d
}

function firstWords(text: string, n: number): string {
  const words = text.trim().split(/\s+/).filter(Boolean)
  return words.length > n ? `${words.slice(0, n).join(' ')}…` : words.join(' ')
}
