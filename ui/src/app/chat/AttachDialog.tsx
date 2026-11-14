// `+` → Attach note / message / session (03 §7.2): a picker over the same
// index as the command palette (`palette_query` restricted to one kind). The
// chosen item becomes a chip above the input and travels with the next turn
// as retrieved context.
import { useEffect, useState } from 'react'
import { Icon } from '../../kit/icons'
import { cx } from '../../kit/cx'
import { invoke } from '../../tauri'
import type { ItemKind } from '../../state/contract-chat'
import { useChat } from '../../state/chat-store'
import type { PaletteKind, PaletteResult } from '../../state/palette-store'
import { Dialog } from '../ui'
import { KIND_ICON } from './Conversation'
import type { AttachKind } from './Composer'

const LABEL: Record<AttachKind, string> = { note: 'Attach note', message: 'Attach message', session: 'Attach session' }
const PALETTE_KIND: Record<AttachKind, PaletteKind> = { note: 'note', message: 'message', session: 'session' }

export function AttachDialog({ kind, onClose }: { kind: AttachKind; onClose: () => void }) {
  const [text, setText] = useState('')
  const [rows, setRows] = useState<PaletteResult[]>([])
  const [active, setActive] = useState(0)
  useEffect(() => {
    let live = true
    const t = setTimeout(() => {
      invoke<PaletteResult[]>('palette_query', { text, limit: 20, kinds: [PALETTE_KIND[kind]] }).then(
        (r) => {
          if (live) {
            setRows(Array.isArray(r) ? r.filter((x) => x.kind === PALETTE_KIND[kind]) : [])
            setActive(0)
          }
        },
        () => live && setRows([]),
      )
    }, 120)
    return () => {
      live = false
      clearTimeout(t)
    }
  }, [text, kind])

  const pick = (r: PaletteResult) => {
    useChat.getState().attach({ kind: kind as ItemKind, id: r.id, title: r.title, detail: r.detail, route: r.route })
    onClose()
  }

  return (
    <Dialog title={LABEL[kind]} onClose={onClose} testId="chat-attach">
      <input
        className="m-input"
        placeholder={kind === 'note' ? 'Search notes…' : kind === 'message' ? 'Search messages…' : 'Search sessions…'}
        aria-label={LABEL[kind]}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
            e.preventDefault()
            setActive((i) => Math.max(0, Math.min(rows.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1))))
          } else if (e.key === 'Enter' && rows[active]) {
            e.preventDefault()
            pick(rows[active]!)
          }
        }}
      />
      <div className="ch-picker-list" role="listbox" aria-label="Results">
        {rows.map((r, i) => (
          <div key={r.id} className={cx('m-palette-row', i === active && 'is-active')} role="option" aria-selected={i === active} onMouseEnter={() => setActive(i)} onClick={() => pick(r)}>
            <Icon id={KIND_ICON[kind]} />
            <span className="m-ellipsis">{r.title}</span>
            <span className="m-t-meta">{r.detail}</span>
          </div>
        ))}
        {!rows.length && <div className="m-t-meta">Nothing matches.</div>}
      </div>
    </Dialog>
  )
}
