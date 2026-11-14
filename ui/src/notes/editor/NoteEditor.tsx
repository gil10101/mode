// The Markdown editor (04 §9, ADR-0034: CodeMirror 6). The store owns the
// buffer and the save pipeline; this component draws it, reports every user
// edit, and takes the store's own changes (reload, field writes) as one
// minimal remote change so the caret and the undo history survive.
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands'
import { markdown, markdownLanguage } from '@codemirror/lang-markdown'
import { Compartment, EditorState, type Extension } from '@codemirror/state'
import { EditorView, keymap } from '@codemirror/view'
import { useEffect, useRef } from 'react'
import { detectEol, headRange } from './markdown'
import { Remote, livePreview, replaceDoc, taskEditing, wikiResolver } from './livePreview'
import { spaced, type EditorHook } from '../../app/dictation'

export interface NoteEditorProps {
  /** The note id (or `draft`): a new key starts a fresh history. */
  docKey: string
  text: string
  /** Bumped by the store when `text` replaces the buffer. */
  version: number
  readOnly: boolean
  raw: boolean
  resolveWiki: (title: string) => boolean
  onInput: (text: string) => void
  onBlur: () => void
  onSave: () => void
  onToggleRaw: () => void
  /** ⌘⇧U urgent · ⌘⇧T when = today · ⌘⇧D status = done (04 §9.5). */
  onCommand?: (command: 'urgent' | 'today' | 'done') => void
  /** Esc leaves the editor to the list. */
  onEscape?: () => void
  /** ⌘/Ctrl+click on a link: a URL or `mode://note/<id>`. */
  onOpenLink: (href: string) => void
  /** ⌘/Ctrl+click on a wikilink; unresolved ones offer "Create note". */
  onOpenWiki: (title: string) => void
  label?: string
}

const theme = EditorView.theme({
  '&': { backgroundColor: 'transparent', color: 'var(--m-fg)' },
  '&.cm-focused': { outline: 'none' },
  '.cm-content': { caretColor: 'var(--m-fg)', padding: '0' },
  '.cm-line': { padding: '0' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--m-fg)' },
  '&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection': { backgroundColor: 'var(--m-selected)' },
})

export function NoteEditor(props: NoteEditorProps) {
  const host = useRef<HTMLDivElement>(null)
  const view = useRef<EditorView | null>(null)
  const cb = useRef(props)
  cb.current = props
  const preview = useRef(new Compartment())
  const editable = useRef(new Compartment())
  const applied = useRef({ key: '', version: -1 })

  const isRaw = () => cb.current.raw

  function stateFor(text: string): EditorState {
    const eol = detectEol(text)
    const extensions: Extension[] = [
      ...(eol === '\r\n' ? [EditorState.lineSeparator.of('\r\n')] : []),
      history(),
      taskEditing(),
      keymap.of([
        { key: 'Mod-s', preventDefault: true, run: () => (cb.current.onSave(), true) },
        { key: 'Mod-e', preventDefault: true, run: () => (cb.current.onToggleRaw(), true) },
        { key: 'Mod-Shift-u', preventDefault: true, run: () => (cb.current.onCommand?.('urgent'), true) },
        { key: 'Mod-Shift-t', preventDefault: true, run: () => (cb.current.onCommand?.('today'), true) },
        { key: 'Mod-Shift-d', preventDefault: true, run: () => (cb.current.onCommand?.('done'), true) },
        { key: 'Escape', run: () => (cb.current.onEscape ? (cb.current.onEscape(), true) : false) },
        ...defaultKeymap,
        ...historyKeymap,
      ]),
      markdown({ base: markdownLanguage }),
      EditorView.lineWrapping,
      theme,
      preview.current.of(cb.current.raw ? [] : livePreview()),
      editable.current.of([EditorState.readOnly.of(cb.current.readOnly), EditorView.editable.of(!cb.current.readOnly)]),
      wikiResolver.of((t) => cb.current.resolveWiki(t)),
      EditorView.contentAttributes.of({ 'aria-label': cb.current.label ?? 'Note body' }),
      EditorView.updateListener.of((u) => {
        if (u.docChanged && !u.transactions.some((t) => t.annotation(Remote))) cb.current.onInput(u.state.sliceDoc())
      }),
      EditorView.domEventHandlers({
        blur: () => {
          cb.current.onBlur()
        },
        mousedown: (e) => {
          const target = (e.target as HTMLElement | null)?.closest<HTMLElement>('.nt-link')
          if (!target || !(e.metaKey || e.ctrlKey)) return false
          e.preventDefault()
          if (target.dataset.wiki) cb.current.onOpenWiki(target.dataset.wiki)
          else if (target.dataset.href) cb.current.onOpenLink(target.dataset.href)
          return true
        },
      }),
    ]
    const s0 = EditorState.create({ doc: text, extensions })
    const fm = isRaw() ? null : headRange(s0.doc.sliceString(0, Math.min(s0.doc.length, 20_000)))
    if (!fm) return s0
    return s0.update({ selection: { anchor: Math.min(fm.to + 1, s0.doc.length) }, annotations: Remote.of(true), filter: false }).state
  }

  // Mount once; a new document key swaps the state (fresh history, caret at the body).
  useEffect(() => {
    const v = new EditorView({ parent: host.current!, state: stateFor(props.text) })
    view.current = v
    applied.current = { key: props.docKey, version: props.version }
    // Dictation (XC-36) inserts at this editor's caret through the hook, as one user edit.
    const hook: EditorHook = {
      dom: v.dom,
      insert(text) {
        if (cb.current.readOnly) return false
        const sel = v.state.selection.main
        const piece = spaced(v.state.sliceDoc(Math.max(0, sel.from - 1), sel.from), text)
        if (!piece) return false
        v.dispatch({ changes: { from: sel.from, to: sel.to, insert: piece }, selection: { anchor: sel.from + piece.length }, userEvent: 'input.type', scrollIntoView: true })
        return true
      },
    }
    window.__mode_editor = hook
    return () => {
      if (window.__mode_editor === hook) delete window.__mode_editor
      v.destroy()
      view.current = null
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    const v = view.current
    if (!v) return
    if (applied.current.key !== props.docKey) {
      v.setState(stateFor(props.text))
      applied.current = { key: props.docKey, version: props.version }
      return
    }
    if (applied.current.version !== props.version) {
      applied.current.version = props.version
      if (v.state.sliceDoc() !== props.text) replaceDoc(v, props.text)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.docKey, props.version, props.text])

  useEffect(() => {
    view.current?.dispatch({ effects: preview.current.reconfigure(props.raw ? [] : livePreview()) })
  }, [props.raw])

  useEffect(() => {
    view.current?.dispatch({ effects: editable.current.reconfigure([EditorState.readOnly.of(props.readOnly), EditorView.editable.of(!props.readOnly)]) })
  }, [props.readOnly])

  return <div className="nt-cm" ref={host} data-testid="note-editor" />
}
