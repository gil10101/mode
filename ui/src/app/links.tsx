// The link rows of the Inbox thread panel ("links", 03 §4.1) and the
// Calendar event panel ("linked", 03 §6.6): sessions with their state
// glyph, notes `≡`, messages `✉`; the title; the why in the meta column
// ("linked by you", "mentions rate-limiter", "from an attendee"). A row
// opens its item; the × that appears on hover removes the link, or
// dismisses the suggestion for good (`mode_core::links`).
import { Button } from '../kit/button'
import { Task } from '../kit/lists'
import type { ThreadLink } from '../state/contract-connectors'
import { navigate } from './router'

/** The session state column's glyphs (10 §6.1), one per state. */
const STATE_GLYPH: Record<string, string> = { waiting: '◆', working: '⠋', failed: '✕', unread: '◉', done: '◉', idle: '○', untracked: '◌' }

export function linkGlyph(l: Pick<ThreadLink, 'kind' | 'state'>): string {
  if (l.kind === 'session') return (l.state && STATE_GLYPH[l.state]) || '▶'
  return l.kind === 'note' ? '≡' : '✉'
}

/** The meta column: the why, else the kind (a core or mock without it). */
export function linkMeta(l: Pick<ThreadLink, 'kind' | 'why'>): string {
  return l.why || (l.kind === 'thread' ? 'message' : l.kind)
}

export function openLink(l: Pick<ThreadLink, 'kind' | 'id'>) {
  if (l.kind === 'session') navigate({ view: 'sessions', id: l.id, tab: 'overview' })
  else if (l.kind === 'note') navigate({ view: 'notes', rest: l.id })
  else navigate({ view: 'inbox', rest: `${l.kind === 'message' ? 'message' : 'thread'}/${l.id}` })
}

export function LinkRows({ links, onRemove }: { links: readonly ThreadLink[]; onRemove?: (l: ThreadLink) => void }) {
  return (
    <>
      {links.map((l) => (
        <Task
          key={`${l.kind}:${l.id}`}
          className="ap-link-row"
          testId={`link-${l.kind}-${l.id}`}
          glyph={linkGlyph(l)}
          title={l.title}
          meta={linkMeta(l)}
          onClick={() => openLink(l)}
          trailing={
            onRemove && (
              <Button
                variant="icon"
                quiet
                className="ap-link-remove"
                aria-label={`Remove ${l.title}`}
                title={l.by_user ? 'Remove the link' : 'Not related: stop suggesting it'}
                onClick={(e) => {
                  e.stopPropagation()
                  onRemove(l)
                }}
                onKeyDown={(e) => e.stopPropagation()}
              >
                ×
              </Button>
            )
          }
        />
      ))}
    </>
  )
}
