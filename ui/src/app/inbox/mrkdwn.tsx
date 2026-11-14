// Markdown-light for Slack mrkdwn (03 §4.5): `code`, *bold*, _italic_,
// ~strike~ and <url|label> links; everything else is plain text. Email
// bodies are plain text (no rendering).
import type { ReactNode } from 'react'
import { runs, type Runs } from '../../kit/text'

const TOKEN = /`([^`\n]+)`|\*([^*\n]+)\*|_([^_\n]+)_|~([^~\n]+)~|<(https?:\/\/[^|>\s]+)(?:\|([^>]+))?>/g

export function renderMrkdwn(text: string): Runs {
  const out: ReactNode[] = []
  let last = 0
  let i = 0
  for (const m of text.matchAll(TOKEN)) {
    const at = m.index ?? 0
    if (at > last) out.push(text.slice(last, at))
    const key = `t${i++}`
    if (m[1] !== undefined) out.push(<code key={key}>{m[1]}</code>)
    else if (m[2] !== undefined) out.push(<b key={key}>{m[2]}</b>)
    else if (m[3] !== undefined) out.push(<i key={key}>{m[3]}</i>)
    else if (m[4] !== undefined) out.push(<s key={key}>{m[4]}</s>)
    else if (m[5] !== undefined)
      out.push(
        <a key={key} href={m[5]} target="_blank" rel="noreferrer">
          {m[6] ?? m[5]}
        </a>,
      )
    last = at + m[0].length
  }
  if (last < text.length) out.push(text.slice(last))
  return runs(out)
}

/** Highlights the words of a search in a hit snippet (03 §4.8). */
export function highlight(text: string, query: string): Runs {
  const words = query
    .toLowerCase()
    .split(/\s+/)
    .filter((w) => w.length > 1)
    .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
  if (!words.length) return runs([text])
  const re = new RegExp(`(${words.join('|')})`, 'gi')
  return runs(text.split(re).map((part, i) => (i % 2 ? <b key={i}>{part}</b> : part)))
}
