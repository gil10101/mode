// Prompt delivery from the Overview's prompt box (05 §14, 03 §3.7): the
// bubble is drawn optimistically, marked "sent" when the next
// `UserPromptSubmit` shows up as a new `user` item in the thread, and "not
// confirmed" when 5 s pass without it (a late confirmation still turns it
// into "sent"). A refused or failed `session_prompt` marks it "not sent".
// Pure functions over one `Delivery`; the store and the timer live in
// PromptBox.tsx.
import type { TranscriptItem } from '../state/contract-sessions'
import type { PromptDelivered } from '../state/contract-terminal'

export const CONFIRM_MS = 5000

export type DeliveryStatus = 'pending' | 'sent' | 'not_confirmed' | 'not_sent'

export interface Delivery {
  id: number
  text: string
  startedAt: number
  status: DeliveryStatus
  via: PromptDelivered['delivered_via'] | null
  turnId: string | null
  /** Keys of the `user` items the thread had when the prompt was sent. */
  seen: string[]
  /** Key of the thread item that confirmed it. */
  confirmedBy: string | null
  /** Why it was not sent (the core's sentence). */
  error: string | null
}

type Item = Pick<TranscriptItem, 'kind' | 'seq' | 'at'> & { text?: string | null }

export function itemKey(i: Pick<Item, 'seq' | 'at'>): string {
  return `${i.seq}@${i.at}`
}

function userItems(thread: readonly Item[]): Item[] {
  return thread.filter((i) => i.kind === 'user')
}

/** Whitespace-insensitive comparison (bracketed paste keeps newlines; hooks may trim). */
export function sameText(a: string, b: string): boolean {
  const norm = (s: string) => s.replace(/\s+/g, ' ').trim()
  return norm(a) === norm(b)
}

export function startDelivery(id: number, text: string, thread: readonly Item[], now: number): Delivery {
  return { id, text, startedAt: now, status: 'pending', via: null, turnId: null, seen: userItems(thread).map(itemKey), confirmedBy: null, error: null }
}

/** `session_prompt` answered. */
export function onAccepted(d: Delivery, r: PromptDelivered | null | undefined): Delivery {
  if (d.status === 'not_sent') return d
  return { ...d, via: r?.delivered_via ?? null, turnId: r?.turn_id ?? null }
}

/** `session_prompt` refused or failed. A prompt the thread already shows stays "sent". */
export function onRefused(d: Delivery, message: string | null): Delivery {
  if (d.status === 'sent') return d
  return { ...d, status: 'not_sent', error: message }
}

/** The thread changed: a new `user` item with the same text (or a hidden preview) confirms delivery. */
export function onThread(d: Delivery, thread: readonly Item[]): Delivery {
  if (d.status !== 'pending' && d.status !== 'not_confirmed') return d
  const seen = new Set(d.seen)
  const hit = userItems(thread).find((i) => !seen.has(itemKey(i)) && (i.text === null || i.text === undefined || sameText(i.text, d.text)))
  return hit ? { ...d, status: 'sent', confirmedBy: itemKey(hit), error: null } : d
}

/** The 5 s timer of delivery `id` fired. */
export function onTimeout(d: Delivery, id: number): Delivery {
  return d.id === id && d.status === 'pending' ? { ...d, status: 'not_confirmed' } : d
}

/** The word under the bubble (10 §12: "sent", "not confirmed"; no narration). */
export function deliveryMark(d: Delivery): string {
  switch (d.status) {
    case 'sent':
      return 'sent'
    case 'not_confirmed':
      return 'not confirmed'
    case 'not_sent':
      return 'not sent'
    default:
      return ''
  }
}

/**
 * What the end of the thread shows for a delivery: the optimistic bubble
 * until the thread has the real item, then only the "sent" mark, and only
 * while that item is still the newest one.
 */
export function deliveryView(d: Delivery | null | undefined, thread: readonly Item[]): { bubble: boolean; mark: string } | null {
  if (!d) return null
  if (d.status !== 'sent') return { bubble: true, mark: deliveryMark(d) }
  const last = thread[thread.length - 1]
  return last && itemKey(last) === d.confirmedBy ? { bubble: false, mark: 'sent' } : null
}
