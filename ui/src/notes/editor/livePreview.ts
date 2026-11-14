// CodeMirror 6 live preview for notes (04 §9.1, ADR-0034): frontmatter
// hidden (its fields live in the header strip and the Fields panel), sized
// headings, styled emphasis / code / links with their markers hidden off the
// caret line, task lines drawn as ○ ◑ ✱ ✓ glyphs that cycle on click (04
// §4.3), wikilinks resolved read-only, quotes, code blocks, rules and tables
// as line styles. Nothing here rewrites the text except the user's own edits
// (04 §9.4): the decorations only change how the buffer is drawn.
import { syntaxTree } from '@codemirror/language'
import { insertNewlineContinueMarkup } from '@codemirror/lang-markdown'
import { Annotation, EditorSelection, EditorState, Facet, Prec, StateField, Transaction, type Extension, type Range } from '@codemirror/state'
import { Decoration, EditorView, ViewPlugin, WidgetType, keymap, type DecorationSet, type ViewUpdate } from '@codemirror/view'
import { EMPTY_BRACKET_RE, cycleMarker, headRange, isDoneMarker, markerGlyph, markerLabel, nextBullet, parseTaskLine, toggleDoneMarker, type TaskMarker } from './markdown'

/** Marks transactions the store made (reload, field write): not user input, not undoable, never filtered. */
export const Remote = Annotation.define<boolean>()

/** Whether `[[Title]]` names an existing note (unresolved links draw dashed). */
export const wikiResolver = Facet.define<(title: string) => boolean, (title: string) => boolean>({
  combine: (v) => v[0] ?? (() => true),
})

const HEAD_SCAN = 20_000

/** The hidden head: the frontmatter and the title H1 (both edited elsewhere). */
function fmOf(state: EditorState) {
  return headRange(state.doc.sliceString(0, Math.min(state.doc.length, HEAD_SCAN)))
}

/* ---- the head (frontmatter + title): hidden, atomic, protected ------------- */

const hiddenFrontmatter = StateField.define<DecorationSet>({
  create: (s) => fmDecorations(s),
  update: (d, tr) => (tr.docChanged ? fmDecorations(tr.state) : d),
  provide: (f) => [EditorView.decorations.from(f), EditorView.atomicRanges.of((view) => view.state.field(f))],
})

function fmDecorations(state: EditorState): DecorationSet {
  const r = fmOf(state)
  if (!r || r.to <= r.from) return Decoration.none
  return Decoration.set([Decoration.replace({ block: true }).range(r.from, r.to)])
}

/** The caret never rests inside the hidden block. */
const clampCaret = EditorState.transactionFilter.of((tr) => {
  if (!tr.selection || tr.annotation(Remote)) return tr
  const r = headRange(tr.newDoc.sliceString(0, Math.min(tr.newDoc.length, HEAD_SCAN)))
  if (!r) return tr
  const min = Math.min(r.to + 1, tr.newDoc.length)
  const sel = tr.newSelection
  if (sel.ranges.every((x) => x.from >= min)) return tr
  return [tr, { selection: EditorSelection.create(sel.ranges.map((x) => EditorSelection.range(Math.max(x.anchor, min), Math.max(x.head, min))), sel.mainIndex), sequential: true }]
})

/** User edits cannot reach into the hidden block; the store's writes pass with `filter: false`. */
const protectFrontmatter = EditorState.changeFilter.of((tr) => {
  const r = fmOf(tr.startState)
  return r ? [r.from, r.to + 1] : true
})

/* ---- widgets ------------------------------------------------------------------ */

function cycleTaskAt(view: EditorView, pos: number) {
  if (view.state.readOnly) return
  const line = view.state.doc.lineAt(pos)
  const t = parseTaskLine(line.text)
  if (!t) return
  const at = line.from + t.markerAt
  view.dispatch({ changes: { from: at, to: at + 1, insert: cycleMarker(t.marker) }, userEvent: 'input.task' })
}

class GlyphWidget extends WidgetType {
  constructor(readonly marker: TaskMarker) {
    super()
  }

  eq(other: GlyphWidget) {
    return other.marker === this.marker
  }

  toDOM(view: EditorView) {
    const el = document.createElement('span')
    el.className = 'm-task-glyph nt-glyph'
    el.dataset.marker = isDoneMarker(this.marker) ? 'x' : this.marker === ' ' ? 'o' : this.marker === '/' ? 'doing' : 'urgent'
    el.textContent = markerGlyph(this.marker)
    el.setAttribute('role', 'button')
    el.setAttribute('aria-label', `Task ${markerLabel(this.marker)}, change status`)
    el.addEventListener('mousedown', (e) => {
      e.preventDefault()
      e.stopPropagation()
      cycleTaskAt(view, view.posAtDOM(el))
    })
    return el
  }

  ignoreEvent() {
    return true
  }
}

class BulletWidget extends WidgetType {
  eq() {
    return true
  }

  toDOM() {
    const el = document.createElement('span')
    el.className = 'nt-bullet'
    el.textContent = '•'
    return el
  }
}

const bullet = Decoration.replace({ widget: new BulletWidget() })
const hide = Decoration.replace({})

/* ---- the preview pass ----------------------------------------------------- */

const WIKI_RE = /(!?)\[\[([^\]|\n]+)(?:\|([^\]\n]+))?\]\]/g

function build(view: EditorView): DecorationSet {
  const { state } = view
  const fm = fmOf(state)
  const bodyFrom = fm ? Math.min(fm.to + 1, state.doc.length) : 0
  const active = new Set<number>()
  if (view.hasFocus) {
    for (const r of state.selection.ranges) {
      const a = state.doc.lineAt(r.from).number
      const b = state.doc.lineAt(r.to).number
      for (let n = a; n <= b; n++) active.add(n)
    }
  }
  const resolve = state.facet(wikiResolver)
  const out: Range<Decoration>[] = []
  const lineClass = (pos: number, cls: string) => out.push(Decoration.line({ class: cls }).range(state.doc.lineAt(pos).from))
  const eachLine = (from: number, to: number, fn: (lineFrom: number) => void) => {
    for (let pos = from; pos <= to; ) {
      const line = state.doc.lineAt(pos)
      fn(line.from)
      pos = line.to + 1
    }
  }
  const isActive = (pos: number) => active.has(state.doc.lineAt(pos).number)
  const taskLines = new Set<number>()

  for (const { from, to } of view.visibleRanges) {
    const start = Math.max(from, bodyFrom)
    if (start > to) continue
    // Lines: task glyphs and wikilinks.
    for (let pos = start; pos <= to; ) {
      const line = state.doc.lineAt(pos)
      const lineActive = active.has(line.number)
      const t = parseTaskLine(line.text)
      if (t) {
        taskLines.add(line.number)
        out.push(Decoration.line({ class: 'nt-task-line' }).range(line.from))
        const b = line.from + t.bulletAt
        const textFrom = line.from + t.textAt
        if (lineActive) {
          out.push(Decoration.widget({ widget: new GlyphWidget(t.marker), side: -1 }).range(b))
          out.push(Decoration.mark({ class: 'nt-ghost' }).range(b, textFrom))
        } else {
          out.push(Decoration.replace({ widget: new GlyphWidget(t.marker) }).range(b, textFrom))
        }
        if (isDoneMarker(t.marker) && textFrom < line.to) out.push(Decoration.mark({ class: 'nt-done' }).range(textFrom, line.to))
      }
      WIKI_RE.lastIndex = 0
      let m: RegExpExecArray | null
      while ((m = WIKI_RE.exec(line.text))) {
        if (m[1]) continue
        const a = line.from + m.index
        const z = a + m[0].length
        const target = m[2].trim()
        const cls = resolve(target) ? 'nt-link' : 'nt-link is-unresolved'
        const attrs = { 'data-wiki': target }
        if (lineActive) {
          out.push(Decoration.mark({ class: cls, attributes: attrs }).range(a, z))
        } else if (m[3] !== undefined) {
          const aliasFrom = a + 2 + m[2].length + 1
          out.push(hide.range(a, aliasFrom), Decoration.mark({ class: cls, attributes: attrs }).range(aliasFrom, z - 2), hide.range(z - 2, z))
        } else {
          out.push(hide.range(a, a + 2), Decoration.mark({ class: cls, attributes: attrs }).range(a + 2, z - 2), hide.range(z - 2, z))
        }
      }
      pos = line.to + 1
    }

    // Syntax: headings, emphasis, code, links, quotes, rules, lists, tables.
    syntaxTree(state).iterate({
      from: start,
      to,
      enter: (node) => {
        if (node.from < bodyFrom) return node.to > bodyFrom
        const name = node.name
        const heading = /^ATXHeading(\d)$/.exec(name)
        if (heading) {
          lineClass(node.from, `nt-h nt-h${heading[1]}`)
          return
        }
        switch (name) {
          case 'HeaderMark': {
            if (isActive(node.from)) return
            const line = state.doc.lineAt(node.from)
            if (node.from === line.from + (line.text.length - line.text.trimStart().length)) {
              const after = state.doc.sliceString(node.to, node.to + 1) === ' ' ? 1 : 0
              out.push(hide.range(node.from, node.to + after))
            } else {
              const before = state.doc.sliceString(node.from - 1, node.from) === ' ' ? 1 : 0
              out.push(hide.range(node.from - before, node.to))
            }
            return
          }
          case 'Emphasis':
            out.push(Decoration.mark({ class: 'nt-em' }).range(node.from, node.to))
            return
          case 'StrongEmphasis':
            out.push(Decoration.mark({ class: 'nt-strong' }).range(node.from, node.to))
            return
          case 'Strikethrough':
            out.push(Decoration.mark({ class: 'nt-strike' }).range(node.from, node.to))
            return
          case 'InlineCode':
            out.push(Decoration.mark({ class: 'nt-code' }).range(node.from, node.to))
            return
          case 'EmphasisMark':
          case 'StrikethroughMark':
            if (!isActive(node.from)) out.push(hide.range(node.from, node.to))
            return
          case 'CodeMark':
            if (node.node.parent?.name === 'InlineCode' && !isActive(node.from)) out.push(hide.range(node.from, node.to))
            return
          case 'Image':
            return false
          case 'Link': {
            const marks = node.node.getChildren('LinkMark')
            const url = node.node.getChild('URL')
            if (marks.length < 2) return false
            const href = url ? state.doc.sliceString(url.from, url.to) : ''
            const textFrom = marks[0].to
            const textTo = marks[1].from
            if (textTo > textFrom) out.push(Decoration.mark({ class: 'nt-link', attributes: { 'data-href': href } }).range(textFrom, textTo))
            if (!isActive(node.from)) {
              out.push(hide.range(node.from, textFrom))
              if (node.to > textTo) out.push(hide.range(textTo, node.to))
            }
            return false
          }
          case 'Autolink': {
            const url = node.node.getChild('URL')
            if (!url) return false
            out.push(Decoration.mark({ class: 'nt-link', attributes: { 'data-href': state.doc.sliceString(url.from, url.to) } }).range(url.from, url.to))
            if (!isActive(node.from)) out.push(hide.range(node.from, url.from), hide.range(url.to, node.to))
            return false
          }
          case 'URL':
            out.push(Decoration.mark({ class: 'nt-link', attributes: { 'data-href': state.doc.sliceString(node.from, node.to) } }).range(node.from, node.to))
            return
          case 'Blockquote':
            eachLine(node.from, node.to, (l) => out.push(Decoration.line({ class: 'nt-quote' }).range(l)))
            return
          case 'QuoteMark':
            if (!isActive(node.from)) {
              const after = state.doc.sliceString(node.to, node.to + 1) === ' ' ? 1 : 0
              out.push(hide.range(node.from, node.to + after))
            }
            return
          case 'FencedCode':
          case 'CodeBlock':
            eachLine(node.from, node.to, (l) => out.push(Decoration.line({ class: 'nt-codeblock' }).range(l)))
            return false
          case 'HorizontalRule':
            lineClass(node.from, 'nt-hr')
            if (!isActive(node.from)) out.push(Decoration.mark({ class: 'nt-hr-mark' }).range(node.from, node.to))
            return
          case 'Table':
            eachLine(node.from, node.to, (l) => out.push(Decoration.line({ class: 'nt-table' }).range(l)))
            return false
          case 'ListMark': {
            const line = state.doc.lineAt(node.from)
            if (taskLines.has(line.number) || parseTaskLine(line.text)) return
            if (!isActive(node.from) && /^[-*+]$/.test(state.doc.sliceString(node.from, node.to))) out.push(bullet.range(node.from, node.to))
            return
          }
          default:
            return
        }
      },
    })
  }
  return Decoration.set(out, true)
}

const preview = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet
    constructor(view: EditorView) {
      this.decorations = build(view)
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged || u.selectionSet || u.focusChanged || syntaxTree(u.startState) !== syntaxTree(u.state)) this.decorations = build(u.view)
    }
  },
  { decorations: (v) => v.decorations },
)

/* ---- commands --------------------------------------------------------------- */

/** ⌘Enter / Ctrl+Enter: toggle the task line at the caret done / open (04 §4.3). */
export function toggleTaskAtCaret(view: EditorView): boolean {
  if (view.state.readOnly) return false
  const changes = []
  for (const r of view.state.selection.ranges) {
    const line = view.state.doc.lineAt(r.head)
    const t = parseTaskLine(line.text)
    if (t) changes.push({ from: line.from + t.markerAt, to: line.from + t.markerAt + 1, insert: toggleDoneMarker(t.marker) })
  }
  if (!changes.length) return false
  view.dispatch({ changes, userEvent: 'input.task' })
  return true
}

/** Enter at the end of a task line continues the list with `- [ ] `; on an empty task line it removes the marker (04 §4.3). */
export function continueTask(view: EditorView): boolean {
  const { state } = view
  const sel = state.selection.main
  if (!sel.empty || state.readOnly) return false
  const line = state.doc.lineAt(sel.head)
  const t = parseTaskLine(line.text)
  if (!t || sel.head !== line.to) return false
  if (!t.text.trim()) {
    view.dispatch({ changes: { from: line.from, to: line.to, insert: t.indent }, selection: { anchor: line.from + t.indent.length }, userEvent: 'input' })
    return true
  }
  const insert = `${state.lineBreak}${t.indent}${nextBullet(t.bullet)} [ ] `
  view.dispatch({ changes: { from: sel.head, insert }, selection: { anchor: sel.head + insert.length }, userEvent: 'input', scrollIntoView: true })
  return true
}

/** `- [] ` becomes `- [ ] ` as it is typed (04 §4.3). */
const fixEmptyBracket = EditorState.transactionFilter.of((tr) => {
  if (!tr.docChanged || !tr.isUserEvent('input.type')) return tr
  const line = tr.newDoc.lineAt(tr.newSelection.main.head)
  const m = EMPTY_BRACKET_RE.exec(line.text)
  if (!m) return tr
  const at = line.from + m[1].length
  return [tr, { changes: { from: at, to: at + 2, insert: '[ ]' }, sequential: true }]
})

/** The task keymap; Enter falls back to Markdown list continuation. */
export const taskKeymap: Extension = Prec.high(
  keymap.of([
    { key: 'Enter', run: (v) => continueTask(v) || insertNewlineContinueMarkup(v) },
    { key: 'Mod-Enter', run: toggleTaskAtCaret },
  ]),
)

/** Live preview: everything that changes how the buffer is drawn. Raw source = without it. */
export function livePreview(): Extension {
  return [hiddenFrontmatter, clampCaret, protectFrontmatter, preview]
}

/** Always on, raw or not. */
export function taskEditing(): Extension {
  return [taskKeymap, fixEmptyBracket]
}

/** Replaces the document with `text` as one minimal change the user's undo history skips. */
export function replaceDoc(view: EditorView, text: string): void {
  const sep = view.state.lineBreak
  const a = view.state.doc.toString()
  const b = sep === '\n' ? text : text.split(sep).join('\n')
  if (a === b) return
  let s = 0
  const max = Math.min(a.length, b.length)
  while (s < max && a.charCodeAt(s) === b.charCodeAt(s)) s++
  let e = 0
  while (e < max - s && a.charCodeAt(a.length - 1 - e) === b.charCodeAt(b.length - 1 - e)) e++
  const insert = b.slice(s, b.length - e)
  view.dispatch({
    changes: { from: s, to: a.length - e, insert: sep === '\n' ? insert : insert.split('\n').join(sep) },
    annotations: [Remote.of(true), Transaction.addToHistory.of(false)],
    filter: false,
  })
}
