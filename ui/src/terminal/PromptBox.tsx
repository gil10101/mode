// The Overview's prompt box and its delivery marks (03 §3.7, 05 §14). The
// rules for when the box shows and is disabled stay in tabs.tsx
// (`footFor`); this is the box itself, "Stop turn" while a headless turn
// runs, and the optimistic bubble at the end of the thread: drawn on Send,
// marked "sent" when the thread gets the matching `user` item, "not
// confirmed" after 5 s, "not sent" when `session_prompt` refuses. A refusal
// is `{ code: 'refused', message, action? }`: the notice says `message` and,
// when `action` names one, offers the existing Jump to terminal / Resume.
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { Button } from '../kit/button'
import type { TranscriptItem } from '../state/contract-sessions'
import type { PromptRefusal } from '../state/contract-terminal'
import { deliveryView, type Delivery } from './prompt'
import { connectTerminal, deliverPrompt, useTerminalStore } from './store'
import './terminal.css'

export { ACTION_LABEL } from './store'

export interface PromptBoxProps {
  sessionId: string
  disabled: boolean
  placeholder: string
  /** The session's thread when the prompt is sent (the confirmation looks for a new `user` item). */
  thread: readonly TranscriptItem[]
  /** A headless turn is running: Send becomes Stop turn (03 §3.7, 05 §8.5). */
  stop?: boolean
  /** A refusal named a button (05 §14): the view runs its own Jump to terminal / Resume. */
  onAction?: (action: 'jump' | 'resume') => void
}

// Prompt-box deliveries are numbered apart from the store's own (`send`), so the two never collide.
let promptSeq = 1_000_000

/**
 * The prompt box's Send: the store's delivery (optimistic bubble, 5 s
 * confirmation timer), plus the refusal as the core sent it so its
 * `action` can be offered through the view's `onAction`. Resolves once
 * `session_prompt` answered.
 */
export async function sendPrompt(sessionId: string, text: string, thread: readonly TranscriptItem[], onAction?: (action: 'jump' | 'resume') => void): Promise<{ delivery: Delivery; refusal: PromptRefusal | null }> {
  return deliverPrompt(++promptSeq, sessionId, text, thread, onAction)
}

/** `.m-input` + Send primary, no helper text (03 §3.7); disabled per 05 §14. A local Composer: the kit's has no disabled state. */
export function PromptBox({ sessionId, disabled, placeholder, thread, stop = false, onAction }: PromptBoxProps) {
  const [text, setText] = useState('')
  const stopping = useTerminalStore((s) => !!s.stopping[sessionId])

  useEffect(() => {
    void connectTerminal()
  }, [])

  const send = async () => {
    const t = text.trim()
    if (!t || disabled || stop) return
    setText('')
    const { delivery } = await sendPrompt(sessionId, t, thread, onAction)
    // Refused: the text comes back so it can be sent again.
    if (delivery.status === 'not_sent') setText((cur) => cur || t)
  }

  return (
    <div className="m-composer">
      <input
        className="m-input is-lg"
        placeholder={placeholder}
        aria-label="Reply"
        value={text}
        disabled={disabled}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault()
            void send()
          }
        }}
      />
      {stop ? (
        <Button variant="primary" disabled={stopping} onClick={() => void useTerminalStore.getState().stopTurn(sessionId)}>
          Stop turn
        </Button>
      ) : (
        <Button variant="primary" disabled={disabled} onClick={() => void send()}>
          Send
        </Button>
      )}
    </div>
  )
}

/** The end of the Overview thread: the optimistic bubble and its mark. Renders nothing when no prompt is in flight. */
export function PromptDelivery({ sessionId, thread }: { sessionId: string; thread: readonly TranscriptItem[] }) {
  const delivery = useTerminalStore((s) => s.deliveries[sessionId] ?? null)
  const view = deliveryView(delivery, thread)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    void connectTerminal()
  }, [])

  // Confirmation follows the thread while the Overview is open (the store also watches the sessions store).
  useEffect(() => {
    useTerminalStore.getState().observe(sessionId, thread)
  }, [sessionId, thread])

  // Keep the newest line in view, as the thread does for its own items.
  useLayoutEffect(() => {
    const scroller = ref.current?.closest('.sd-thread')
    if (scroller) scroller.scrollTop = scroller.scrollHeight
  }, [view?.bubble, view?.mark, delivery?.id])

  if (!view || !delivery) return null
  return (
    <div className="m-thread tm-delivery" ref={ref} data-testid="prompt-delivery" data-status={delivery.status}>
      {view.bubble && <div className="m-thread-user">{delivery.text}</div>}
      {view.mark && <div className="tm-mark">{view.mark}</div>}
    </div>
  )
}
