// The app window tells the core what is on screen (XC-37: a "Hey Mode"
// request attaches it): every route change goes through the router store,
// and the Notes, Inbox and Calendar selections refine it. One
// `app_context_set` per change.
import { useCalendar } from '../state/calendar-store'
import { useInbox } from '../state/inbox-store'
import { useNotesStore } from '../state/notes-store'
import { contextOf, contextSender } from './dictation'
import { useRouter } from './router'

export function connectAppContext(): () => void {
  const send = contextSender()
  const update = () =>
    send(
      contextOf(useRouter.getState().route, {
        note: useNotesStore.getState().selectedId,
        message: useInbox.getState().selectedId,
        event: useCalendar.getState().selectedId,
      }),
    )
  update()
  const subs = [useRouter.subscribe(update), useNotesStore.subscribe(update), useInbox.subscribe(update), useCalendar.subscribe(update)]
  return () => subs.forEach((u) => u())
}
