// A small Markdown renderer for assistant messages (03 §3.7: "Markdown
// rendered, code blocks copyable"): paragraphs, fenced code, bullet and
// numbered lists, inline `code`, **bold** and *italic*. No HTML passes
// through; everything is React text.
import type { ReactNode } from 'react'
import { Button } from '../../kit/button'
import { Icon } from '../../kit/icons'
import { runs, type Runs } from '../../kit/text'
import { copyText } from '../ui'

type Block = { kind: 'p'; text: string } | { kind: 'code'; text: string; lang: string } | { kind: 'ul' | 'ol'; items: string[] }

export function parseBlocks(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, '\n').split('\n')
  const blocks: Block[] = []
  let para: string[] = []
  const flush = () => {
    if (para.length) blocks.push({ kind: 'p', text: para.join(' ') })
    para = []
  }
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const fence = /^```(\S*)\s*$/.exec(line)
    if (fence) {
      flush()
      const body: string[] = []
      i++
      while (i < lines.length && !/^```\s*$/.test(lines[i])) body.push(lines[i++])
      blocks.push({ kind: 'code', text: body.join('\n'), lang: fence[1] })
      continue
    }
    const ul = /^\s*[-*]\s+(.*)$/.exec(line)
    const ol = /^\s*\d+[.)]\s+(.*)$/.exec(line)
    if (ul || ol) {
      flush()
      const kind = ul ? 'ul' : 'ol'
      const last = blocks[blocks.length - 1]
      const text = (ul ?? ol)![1]
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

/** Inline spans: `code`, **bold**, *italic*. */
export function inline(text: string): Runs {
  const out: ReactNode[] = []
  const re = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*\s][^*]*\*)/g
  let last = 0
  let m: RegExpExecArray | null
  let k = 0
  while ((m = re.exec(text))) {
    if (m.index > last) out.push(text.slice(last, m.index))
    const tok = m[0]
    if (tok.startsWith('`')) out.push(<code key={k++}>{tok.slice(1, -1)}</code>)
    else if (tok.startsWith('**')) out.push(<b key={k++}>{tok.slice(2, -2)}</b>)
    else out.push(<i key={k++}>{tok.slice(1, -1)}</i>)
    last = m.index + tok.length
  }
  if (last < text.length) out.push(text.slice(last))
  return runs(out)
}

export function Markdown({ text }: { text: string }) {
  return (
    <>
      {parseBlocks(text).map((b, i) => {
        switch (b.kind) {
          case 'p':
            return <p key={i}>{inline(b.text)}</p>
          case 'code':
            return (
              <pre key={i} className="ap-code">
                {b.text}
                <Button variant="icon" quiet aria-label="Copy code" onClick={() => copyText(b.text, 'Code copied')}>
                  <Icon id="copy" />
                </Button>
              </pre>
            )
          case 'ul':
            return (
              <ul key={i}>
                {b.items.map((t, j) => (
                  <li key={j}>{inline(t)}</li>
                ))}
              </ul>
            )
          case 'ol':
            return (
              <ol key={i}>
                {b.items.map((t, j) => (
                  <li key={j}>{inline(t)}</li>
                ))}
              </ol>
            )
        }
      })}
    </>
  )
}
