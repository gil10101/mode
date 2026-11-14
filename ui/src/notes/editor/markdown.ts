// Pure Markdown rules the editor applies (04 §3, §4.1, §4.3, §9.4).

/** 04 §4.1: indentation · bullet (numbered allowed) · `[ ]` `[/]` `[!]` `[x]` · text. */
export const TASK_RE = /^(\s*)([-*+]|\d+[.)])([ \t]+)\[( |\/|!|x|X)\]([ \t]+)(.*)$/

export type TaskMarker = ' ' | '/' | '!' | 'x' | 'X'

export interface TaskLine {
  indent: string
  bullet: string
  marker: TaskMarker
  /** Offset of the marker character inside the line. */
  markerAt: number
  /** Offset of the bullet. */
  bulletAt: number
  /** Offset of the task text. */
  textAt: number
  text: string
}

export function parseTaskLine(line: string): TaskLine | null {
  const m = TASK_RE.exec(line)
  if (!m) return null
  const [, indent, bullet, gap, marker, gap2, text] = m
  const bulletAt = indent.length
  const markerAt = bulletAt + bullet.length + gap.length + 1
  return { indent, bullet, marker: marker as TaskMarker, markerAt, bulletAt, textAt: markerAt + 2 + gap2.length, text }
}

/** Glyph click (04 §4.3): `[ ]` → `[/]` → `[x]` → `[ ]`; `[!]` → `[x]`. Mode writes a lowercase `x`. */
export function cycleMarker(m: TaskMarker): TaskMarker {
  switch (m) {
    case ' ':
      return '/'
    case '/':
      return 'x'
    case '!':
      return 'x'
    default:
      return ' '
  }
}

/** Space / ⌘Enter (04 §4.3): any open marker → `[x]`; `[x]` → `[ ]`. */
export function toggleDoneMarker(m: TaskMarker): TaskMarker {
  return m === 'x' || m === 'X' ? ' ' : 'x'
}

export function isDoneMarker(m: TaskMarker): boolean {
  return m === 'x' || m === 'X'
}

export function markerGlyph(m: TaskMarker): string {
  switch (m) {
    case ' ':
      return '○'
    case '/':
      return '◑'
    case '!':
      return '✱'
    default:
      return '✓'
  }
}

export function markerLabel(m: TaskMarker): string {
  switch (m) {
    case ' ':
      return 'todo'
    case '/':
      return 'doing'
    case '!':
      return 'urgent'
    default:
      return 'done'
  }
}

/** The bullet of the line Enter continues to: numbers count up, bullets repeat. */
export function nextBullet(bullet: string): string {
  const m = /^(\d+)([.)])$/.exec(bullet)
  return m ? `${Number(m[1]) + 1}${m[2]}` : bullet
}

/** `- [] ` is corrected to `- [ ] ` as soon as it is typed (04 §4.3). */
export const EMPTY_BRACKET_RE = /^(\s*(?:[-*+]|\d+[.)])[ \t]+)\[\]([ \t])/

/* ---- frontmatter ---------------------------------------------------------- */

const FM_RE = /^(﻿?)---(\r?\n)([\s\S]*?)\r?\n(?:---|\.\.\.)[ \t]*(?:\r?\n|$)/

export interface SplitFile {
  bom: string
  /** The lines between the fences, or null without frontmatter. */
  block: string | null
  /** Everything after the closing fence's line ending, verbatim. */
  body: string
  eol: '\n' | '\r\n'
}

export function detectEol(text: string): '\n' | '\r\n' {
  const i = text.indexOf('\n')
  return i > 0 && text[i - 1] === '\r' ? '\r\n' : '\n'
}

export function splitFile(text: string): SplitFile {
  const eol = detectEol(text)
  const m = FM_RE.exec(text)
  if (!m) return { bom: text.startsWith('﻿') ? '﻿' : '', block: null, body: text.replace(/^﻿/, ''), eol }
  return { bom: m[1], block: m[3], body: text.slice(m[0].length), eol }
}

/** The hidden range of the live preview: from 0 to the end of the closing fence line (its line break excluded). */
export function frontmatterRange(text: string): { from: number; to: number } | null {
  const m = FM_RE.exec(text)
  if (!m) return null
  const end = m[0].endsWith('\n') ? m[0].length - (m[0].endsWith('\r\n') ? 2 : 1) : m[0].length
  return { from: 0, to: end }
}

/**
 * Mode rewrites only the frontmatter of what it was sent (04 §3.4: the body
 * is copied verbatim), so the written file's header can be laid onto the live
 * buffer without losing what was typed while the write was in flight.
 */
export function rebaseOnto(current: string, sent: string, written: string): string {
  if (current === sent) return written
  const s = splitFile(sent)
  const c = splitFile(current)
  const w = splitFile(written)
  const head = written.endsWith(s.body) ? written.slice(0, written.length - s.body.length) : written.slice(0, written.length - w.body.length)
  return head + c.body
}

/* ---- the title: the first H1 (04 §2.4), edited in the editor head ---------- */

const H1_RE = /^#(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$/

/**
 * What live preview hides at the top: the frontmatter, then the title H1 when
 * it is the first non-blank body line (it is edited in the head, 03 §5.4).
 * Offsets are CodeMirror's (`\n` text).
 */
export function headRange(text: string): { from: number; to: number } | null {
  const fm = frontmatterRange(text)
  let pos = fm ? fm.to + 1 : 0
  if (pos > text.length) return fm
  for (;;) {
    const end = text.indexOf('\n', pos)
    const line = text.slice(pos, end < 0 ? text.length : end)
    if (line.trim() === '' && end >= 0) {
      pos = end + 1
      continue
    }
    if (H1_RE.test(line)) {
      // ...and the one blank line that usually follows it.
      const after = end < 0 ? -1 : text.indexOf('\n', end + 1)
      const next = end < 0 ? null : text.slice(end + 1, after < 0 ? text.length : after)
      return { from: 0, to: next !== null && next.trim() === '' && after >= 0 ? after : pos + line.length }
    }
    return fm
  }
}

/** The first H1 of the body, or null. */
export function bufferTitle(text: string): string | null {
  for (const line of splitFile(text).body.split(/\r?\n/)) {
    const m = H1_RE.exec(line)
    if (m) return (m[1] ?? '').trim()
  }
  return null
}

/** Sets the first H1 (or adds one at the top of the body); the rest is untouched (04 §9.4). */
export function withTitle(text: string, title: string): string {
  const { body } = splitFile(text)
  const head = text.slice(0, text.length - body.length)
  const eol = detectEol(text)
  const lines = body.split(eol)
  const i = lines.findIndex((l) => H1_RE.test(l))
  if (i >= 0) {
    lines[i] = `# ${title}`
    return head + lines.join(eol)
  }
  return `${head}# ${title}${eol}${body}`
}
