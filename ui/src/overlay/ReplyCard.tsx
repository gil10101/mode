// The voice reply card (02 §5.7, ADR-0064): a "Hey Mode" request asked while
// the app window was not in front is answered here, in place of the tabs, the
// rows and the capture box. The reply streams in place (no per-token motion),
// is rendered as Chat renders a reply, clamps at twelve lines with a fade (the
// overlay never scrolls), and the footer opens the voice conversation, copies
// the answer or dismisses the card.
import { useLayoutEffect, useRef, useState } from 'react'
import { Button } from '../kit/button'
import { Shimmer } from '../kit/chat'
import { ReplyMarkdown } from '../app/chat/markdown'
import type { Citation, ItemKind } from '../state/contract-chat'
import { useOverlay } from './context'
import type { Reply } from './machine'

/** The app view a citation's item opens on (`app_open`). */
const VIEW_OF: Record<string, string> = { note: 'notes', message: 'inbox', session: 'sessions', event: 'calendar' }

/** How long "Copied" replaces "Copy". */
const COPIED_MS = 1200

export function ReplyCard({ reply }: { reply: Reply }) {
  const { act, runner } = useOverlay()
  const bodyRef = useRef<HTMLDivElement | null>(null)
  const [clamped, setClamped] = useState(false)
  const [copied, setCopied] = useState(false)
  const citations: Citation[] = reply.citations.map((c) => ({ ...c, kind: c.kind as ItemKind }))
  const pending = !reply.done && !reply.text

  // Twelve lines at most (the class's max-height); a longer reply fades out at the bottom.
  useLayoutEffect(() => {
    const el = bodyRef.current
    setClamped(!!el && el.scrollHeight > el.clientHeight + 1)
  }, [reply.text, reply.error, pending])

  const openChat = () => act('open_dictation', undefined, { view: 'chat', id: reply.threadId })
  const openCitation = (c: Citation) => {
    const view = VIEW_OF[c.kind]
    if (view) act('open_dictation', undefined, { view, id: c.id })
  }
  const copy = () => {
    void navigator.clipboard?.writeText(reply.text).then(
      () => {
        setCopied(true)
        setTimeout(() => setCopied(false), COPIED_MS)
      },
      () => {},
    )
  }

  return (
    <div className="m-reply-card" data-testid="reply-card" aria-live="polite" aria-busy={!reply.done}>
      {pending ? (
        <Shimmer lines={2} />
      ) : reply.error ? (
        <div className="m-reply-card-error" data-testid="reply-error">
          {reply.error}
        </div>
      ) : (
        <div ref={bodyRef} className={`m-reply${clamped ? ' is-clamped' : ''}`} data-testid="reply-text">
          <ReplyMarkdown text={reply.text} citations={citations} onOpen={openCitation} />
        </div>
      )}
      <div className="m-reply-card-foot">
        <Button onClick={openChat} data-testid="reply-open">
          Open in Chat
        </Button>
        <Button quiet disabled={!reply.done || !!reply.error} onClick={copy} data-testid="reply-copy">
          <span key={copied ? 'copied' : 'copy'} className="m-reply-card-label">
            {copied ? 'Copied' : 'Copy'}
          </span>
        </Button>
        <Button quiet onClick={() => runner.dispatch({ type: 'reply_dismiss' })} data-testid="reply-dismiss">
          Dismiss
        </Button>
      </div>
    </div>
  )
}
