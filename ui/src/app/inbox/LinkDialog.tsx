// Link… (03 §4.6 "Link to note / session", `L`; 03 §6.6): a picker over
// the notes and the sessions; the pick writes a `references` link from the
// thread or the calendar event through `item_link_add` (`mode_core::links`)
// and the panel's links show it at once with "linked by you".
import { useEffect, useMemo, useState } from 'react'
import { create } from 'zustand'
import { Button } from '../../kit/button'
import { Task } from '../../kit/lists'
import type { ThreadLink } from '../../state/contract-connectors'
import type { NoteListRow, NotesListResult } from '../../state/contract-notes'
import { useCalendar } from '../../state/calendar-store'
import { useInbox } from '../../state/inbox-store'
import { useSessionsStore } from '../../state/sessions-store'
import { invoke } from '../../tauri'
import { linkGlyph } from '../links'
import { Dialog } from '../ui'

/** What the picked item is linked from. */
export interface LinkAnchor {
  kind: 'thread' | 'calendar_event'
  id: string
}

interface LinkUi {
  anchor: LinkAnchor | null
  open(a: LinkAnchor): void
  close(): void
}

export const useLinkUi = create<LinkUi>((set) => ({
  anchor: null,
  open: (anchor) => set({ anchor }),
  close: () => set({ anchor: null }),
}))

/** Open the picker for a thread (an id) or an anchor. */
export function openLinkPicker(target: string | LinkAnchor) {
  useLinkUi.getState().open(typeof target === 'string' ? { kind: 'thread', id: target } : target)
}

export function LinkHost() {
  const anchor = useLinkUi((s) => s.anchor)
  return anchor ? <LinkDialog anchor={anchor} onClose={() => useLinkUi.getState().close()} /> : null
}

type Choice = Pick<ThreadLink, 'kind' | 'id' | 'title' | 'state'>

function LinkDialog({ anchor, onClose }: { anchor: LinkAnchor; onClose: () => void }) {
  const [notes, setNotes] = useState<NoteListRow[] | null>(null)
  const [query, setQuery] = useState('')
  const roster = useSessionsStore((s) => s.rows)
  const history = useSessionsStore((s) => s.history)
  useEffect(() => {
    invoke<NotesListResult>('notes_list', { scope: { kind: 'filter', filter: {} }, limit: 500 }).then(
      (r) => setNotes(r?.rows ?? []),
      () => setNotes([]),
    )
  }, [])
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase()
    const seen = new Set<string>()
    const sessions: Choice[] = [...roster, ...history]
      .filter((r) => !seen.has(r.id) && (seen.add(r.id), true))
      .map((r) => ({ kind: 'session' as const, id: r.id, title: r.display_name, state: r.state }))
    const all: Choice[] = [...sessions, ...(notes ?? []).map((n) => ({ kind: 'note' as const, id: n.id, title: n.title }))]
    return all.filter((c) => !q || c.title.toLowerCase().includes(q)).slice(0, 50)
  }, [notes, roster, history, query])
  const pick = (c: Choice) => {
    onClose()
    const to = { kind: c.kind, id: c.id, title: c.title }
    if (anchor.kind === 'thread') void useInbox.getState().linkAdd(anchor.id, to)
    else void useCalendar.getState().linkAdd(anchor.id, to)
  }
  return (
    <Dialog title="Link to a note or session" onClose={onClose} testId="link-dialog">
      <input className="m-input" aria-label="Search notes and sessions" placeholder="Search notes and sessions…" value={query} onChange={(e) => setQuery(e.target.value)} />
      <div className="ib-link-list" role="list" aria-label="Notes and sessions">
        {notes === null && <div className="ib-more">Loading…</div>}
        {shown.map((c) => (
          <Task key={`${c.kind}:${c.id}`} glyph={linkGlyph(c)} title={c.title} meta={c.kind} onClick={() => pick(c)} />
        ))}
        {notes !== null && !shown.length && <div className="ib-more">Nothing matches.</div>}
      </div>
      <div className="m-card-actions">
        <Button onClick={onClose}>Cancel</Button>
      </div>
    </Dialog>
  )
}
