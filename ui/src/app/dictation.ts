// Where dictated words land (XC-36; 02 §5.6): the core sends `dictation.text`
// to the focused Mode window only, and this puts the words at the caret of
// whatever editable element has focus there, then tells the core whether it
// could (`dictation_inserted { ok }`; on `false` the core captures a note or
// does nothing, per `dictation.unfocused`). Shared by both windows' entry
// points. The app window also reports the item on screen to the core on every
// navigation (`app_context_set`), which a "Hey Mode" request attaches.
import { EVENTS, type AppContextKind, type DictationState, type DictationText } from '../state/contract'
import { invoke, listen, type Unlisten } from '../tauri'

/** The CodeMirror editor's hook (NoteEditor installs it on mount): its DOM and an insert at its caret. */
export interface EditorHook {
  dom: HTMLElement
  /** Inserts `text` at the caret with the spacing rule applied; false when the editor is read-only. */
  insert(text: string): boolean
}

declare global {
  interface Window {
    __mode_editor?: EditorHook
  }
}

/** A leading space when the caret follows a non-space character; never a trailing one (the user keeps typing). */
export function spaced(before: string, text: string): string {
  const t = text.trim()
  if (!t) return ''
  return before.length > 0 && !/\s$/.test(before) ? ` ${t}` : t
}

const TEXT_INPUTS = new Set(['text', 'search', 'url', 'email', 'tel', ''])

function isTextField(el: Element): el is HTMLInputElement | HTMLTextAreaElement {
  if (el instanceof HTMLTextAreaElement) return true
  return el instanceof HTMLInputElement && TEXT_INPUTS.has(el.type)
}

/**
 * Sets an `<input>` / `<textarea>` value through the prototype's native setter
 * and fires `input`, so a React-controlled field sees the change as typing.
 */
function insertIntoField(el: HTMLInputElement | HTMLTextAreaElement, text: string): boolean {
  if (el.readOnly || el.disabled) return false
  const value = el.value
  const start = el.selectionStart ?? value.length
  const end = el.selectionEnd ?? start
  const piece = spaced(value.slice(0, start), text)
  if (!piece) return false
  const next = value.slice(0, start) + piece + value.slice(end)
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set
  if (setter) setter.call(el, next)
  else el.value = next
  const caret = start + piece.length
  el.setSelectionRange?.(caret, caret)
  el.dispatchEvent(new Event('input', { bubbles: true }))
  // React may re-render the controlled value; the caret stays after the words.
  el.setSelectionRange?.(caret, caret)
  return true
}

/** The text between the start of the caret's text node and the caret, for the spacing rule. */
function textBeforeCaret(): string {
  const sel = typeof window.getSelection === 'function' ? window.getSelection() : null
  if (!sel || sel.rangeCount === 0) return ''
  const r = sel.getRangeAt(0)
  const node = r.startContainer
  if (node.nodeType === Node.TEXT_NODE) return (node.textContent ?? '').slice(0, r.startOffset)
  const prev = node.childNodes[r.startOffset - 1]
  return prev?.textContent ?? ''
}

function insertIntoEditable(text: string): boolean {
  const piece = spaced(textBeforeCaret(), text)
  if (!piece) return false
  if (typeof document.execCommand !== 'function') return false
  return document.execCommand('insertText', false, piece)
}

/** Puts `text` at the caret of `el` (default: the focused element). False when nothing editable has focus. */
export function insertAtCaret(text: string, el: Element | null = typeof document === 'undefined' ? null : document.activeElement): boolean {
  if (!el || el === document.body) return false
  const editor = window.__mode_editor
  if (editor && editor.dom.contains(el)) return editor.insert(text)
  if (isTextField(el)) return insertIntoField(el, text)
  if (el instanceof HTMLElement && el.isContentEditable) return insertIntoEditable(text)
  return false
}

/**
 * Listens for `dictation.text` and answers every one with `dictation_inserted`;
 * while the core is listening, Esc in this window cancels (`dictation_cancel`)
 * and goes no further.
 */
export async function connectDictation(): Promise<Unlisten> {
  let listening = false
  const onKey = (e: KeyboardEvent) => {
    if (!listening || e.key !== 'Escape') return
    e.preventDefault()
    e.stopPropagation()
    listening = false
    void invoke('dictation_cancel').catch(() => {})
  }
  window.addEventListener('keydown', onKey, true)
  const subs = await Promise.all([
    listen<DictationText>(EVENTS.dictationText, (p) => {
      let ok = false
      try {
        ok = insertAtCaret(p?.text ?? '')
      } catch {
        ok = false
      }
      void invoke('dictation_inserted', { ok }).catch(() => {})
    }),
    listen<DictationState>(EVENTS.dictation, (p) => {
      listening = p?.state === 'listening'
    }),
  ])
  return () => {
    window.removeEventListener('keydown', onKey, true)
    subs.forEach((u) => u())
  }
}

/* ---- the app window's context (XC-37) ------------------------------------- */

export interface AppContext {
  kind: AppContextKind | null
  id: string | null
}

export const NO_CONTEXT: AppContext = { kind: null, id: null }

/**
 * The item on screen from the route and the views' selections: a session's
 * detail, the selected note, inbox thread or calendar event; null elsewhere.
 */
export function contextOf(route: { view: string; id?: string; rest?: string }, selected: { note?: string | null; message?: string | null; event?: string | null } = {}): AppContext {
  switch (route.view) {
    case 'sessions':
      return route.id ? { kind: 'session', id: route.id } : NO_CONTEXT
    case 'notes': {
      const id = selected.note ?? route.rest ?? null
      return id ? { kind: 'note', id } : NO_CONTEXT
    }
    case 'inbox': {
      const [kind, rid] = (route.rest ?? '').split('/')
      const id = selected.message ?? (kind === 'thread' && rid ? rid : null)
      return id ? { kind: 'message', id } : NO_CONTEXT
    }
    case 'calendar': {
      const id = selected.event ?? route.rest ?? null
      return id ? { kind: 'event', id } : NO_CONTEXT
    }
    default:
      return NO_CONTEXT
  }
}

/** Sends `app_context_set` when the context changes (dedup'd); returns the sender. */
export function contextSender(send: (c: AppContext) => Promise<unknown> = (c) => invoke('app_context_set', { kind: c.kind, id: c.id })) {
  let last = ''
  return (c: AppContext) => {
    const key = `${c.kind ?? ''}:${c.id ?? ''}`
    if (key === last) return
    last = key
    void send(c).catch(() => {})
  }
}
