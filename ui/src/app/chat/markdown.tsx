// The reply renderer of 03 §7.1 / §7.4: paragraphs, bullet and numbered
// lists, fenced code (copyable), inline `code`, **bold**, *italic*, and the
// numbered citation markers `[n]` as chips that open the cited item. No HTML
// passes through; everything is React text.
import type { ReactNode } from 'react'
import { runs, type Runs } from '../../kit/text'
import type { Citation } from '../../state/contract-chat'

export type Block = { kind: 'p'; text: string } | { kind: 'code'; text: string; lang: string } | { kind: 'ul' | 'ol'; items: string[] }

export function parseBlocks(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, '\n').split('\n')
  const blocks: Block[] = []
  let para: string[] = []
  const flush = () => {
    if (para.length) blocks.push({ kind: 'p', text: para.join(' ') })
    para = []
  }
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!
    const fence = /^```(\S*)\s*$/.exec(line)
    if (fence) {
      flush()
      const body: string[] = []
      i++
      while (i < lines.length && !/^```\s*$/.test(lines[i]!)) body.push(lines[i++]!)
      blocks.push({ kind: 'code', text: body.join('\n'), lang: fence[1]! })
      continue
    }
    const ul = /^\s*[-*]\s+(.*)$/.exec(line)
    const ol = /^\s*\d+[.)]\s+(.*)$/.exec(line)
    if (ul || ol) {
      flush()
      const kind = ul ? 'ul' : 'ol'
      const last = blocks[blocks.length - 1]
      const text = (ul ?? ol)![1]!
      if (last && last.kind === kind) last.items.push(text)
      else blocks.push({ kind, items: [text] })
      continue
    }
    if (!line.trim()) {
      flush()
      continue
    }
    para.push(line.trim())
  }
  flush()
  return blocks
}

export interface CiteProps {
  citation: Citation
  onOpen: (c: Citation) => void
}

/** Inline `[n]`: a small numbered chip; the title is its tooltip and label. */
function CiteMark({ citation, onOpen }: CiteProps) {
  return (
    <button type="button" className="m-chip ch-cite" title={citation.title} aria-label={`Source ${citation.n}: ${citation.title}`} onClick={() => onOpen(citation)}>
      {citation.n}
    </button>
  )
}

/** Inline spans: `code`, **bold**, *italic*, `[n]` citations (unknown numbers stay text). */
export function inline(text: string, citations: Citation[] = [], onOpen: (c: Citation) => void = () => {}): Runs {
  const out: ReactNode[] = []
  const re = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*\s][^*]*\*|\[\d{1,2}\])/g
  let last = 0
  let m: RegExpExecArray | null
  let k = 0
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index))
    const tok = m[0]
    if (tok.startsWith('`')) out.push(<code key={k++}>{tok.slice(1, -1)}</code>)
    else if (tok.startsWith('**')) out.push(<strong key={k++}>{tok.slice(2, -2)}</strong>)
    else if (tok.startsWith('[')) {
      const n = Number(tok.slice(1, -1))
      const c = citations.find((x) => x.n === n)
      out.push(c ? <CiteMark key={k++} citation={c} onOpen={onOpen} /> : tok)
    } else out.push(<em key={k++}>{tok.slice(1, -1)}</em>)
    last = m.index + tok.length
  }
  if (last < text.length) out.push(text.slice(last))
  // An unknown `[n]` stays text next to the text around it: one text node.
  return runs(out)
}

export function ReplyMarkdown({ text, citations, onOpen, onCopyCode }: { text: string; citations?: Citation[]; onOpen: (c: Citation) => void; onCopyCode?: (code: string) => void }) {
  return (
    <>
      {parseBlocks(text).map((b, i) => {
        if (b.kind === 'p') return <p key={i}>{inline(b.text, citations, onOpen)}</p>
        if (b.kind === 'code')
          return (
            <pre key={i} className="ch-code">
              <code>{b.text}</code>
              {onCopyCode && (
                <button type="button" className="m-btn m-btn-icon is-quiet ch-code-copy" aria-label="Copy code" onClick={() => onCopyCode(b.text)}>
                  <svg className="m-icon" aria-hidden="true">
                    <use href="#i-copy" />
                  </svg>
                </button>
              )}
            </pre>
          )
        const items = b.items.map((it, j) => <li key={j}>{inline(it, citations, onOpen)}</li>)
        return b.kind === 'ul' ? <ul key={i}>{items}</ul> : <ol key={i}>{items}</ol>
      })}
    </>
  )
}
