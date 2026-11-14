// The Sources panel of 03 §7.1, §7.3: every tool call of the turn with its
// inputs and the items it returned (chips that open them), the turn's tokens
// and a plain count per kind in its head. Collapsed it is nothing at all: the
// column slides from 0 to 320 px (chat.css) and back. The head's chevron opens
// and closes it; open or closed is the device setting `chat.sources_open`.
import { useEffect, useRef } from 'react'
import { cx } from '../../kit/cx'
import { RAIL_KINDS, railCounts, useChat } from '../../state/chat-store'
import { ItemChip } from './Conversation'

export function formatTokens(n: number | null | undefined): string {
  if (n === null || n === undefined) return ''
  if (n < 1000) return `${n}`
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0).replace(/\.0$/, '')}k`
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
}

export function SourcesPanel({ open }: { open: boolean }) {
  const sources = useChat((s) => s.sources)
  const focus = useChat((s) => s.focusCall)
  const ref = useRef<HTMLElement>(null)
  useEffect(() => {
    if (!focus || !open) return
    const el = ref.current?.querySelector<HTMLElement>(`[data-call="${CSS.escape(focus)}"]`)
    el?.scrollIntoView?.({ block: 'nearest' })
  }, [focus, sources, open])
  const calls = sources?.calls ?? []
  const counts = railCounts(sources)
  return (
    <aside className="m-panel ch-sources" aria-label="Sources" aria-hidden={!open} inert={!open} ref={ref} id="chat-sources" data-testid="sources-panel">
      <div className="ch-sources-in">
        <div className="ch-sources-head">
          <span className="m-card-title">Sources</span>
          {sources?.tokens ? <span className="m-t-meta">{`${formatTokens(sources.tokens)} tokens`}</span> : null}
        </div>
        {counts.length > 0 && (
          <div className="ch-counts" data-testid="sources-counts">
            {counts.map((c) => {
              const k = RAIL_KINDS.find((x) => x.kind === c.kind)!
              return (
                <span key={c.kind} className="ch-count" title={k.label}>
                  <svg className="m-icon" role="img" aria-label={`${c.count} ${k.label}`}>
                    <use href={`#i-${k.icon}`} />
                  </svg>
                  <span className="m-t-meta" aria-hidden="true">
                    {c.count}
                  </span>
                </span>
              )
            })}
          </div>
        )}
        {!calls.length && <div className="m-t-meta">No tool calls in this turn.</div>}
        {calls.map((c) => (
          <div key={c.id} className={cx('ch-call', focus === c.id && 'is-focus')} data-call={c.id}>
            <div className="ch-call-head">
              <code>{c.tool}</code>
              <span>{c.input_line}</span>
              {c.state === 'running' && <span className="m-spinner" aria-label="Running" />}
            </div>
            {c.items.length ? (
              <div className="ch-items">
                {c.items.map((it) => (
                  <ItemChip key={`${it.kind}:${it.id}`} item={it} />
                ))}
              </div>
            ) : (
              <div className="m-t-meta">{c.state === 'running' ? 'searching…' : c.state === 'failed' ? 'failed' : 'nothing found'}</div>
            )}
          </div>
        ))}
      </div>
    </aside>
  )
}
