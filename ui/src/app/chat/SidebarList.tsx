// The conversations under the Chat entry of the sidebar (03 §2.3.2, ADR-0055):
// newest first, titled by their first question, five rows then "Show n more",
// `⋮` with Rename and Delete; the 90-day retention note.
import { useEffect, useState } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { SidebarMore, SidebarSubItem, SidebarSubNote } from '../../kit/layout'
import { useChat } from '../../state/chat-store'
import { useSessionsStore } from '../../state/sessions-store'
import { navigate } from '../router'
import { Dialog, openMenu } from '../ui'

/** Rows shown before "Show n more". */
export const SIDEBAR_LIMIT = 5

export function ChatSidebarList({ activeId }: { activeId: string | null }) {
  const threads = useChat((s) => s.threads)
  const loaded = useChat((s) => s.threadsLoaded)
  const [more, setMore] = useState(false)
  const [renaming, setRenaming] = useState<{ id: string; title: string } | null>(null)
  useEffect(() => {
    if (!loaded) void useChat.getState().loadThreads()
  }, [loaded])
  const shown = more ? threads : threads.slice(0, SIDEBAR_LIMIT)
  const hidden = threads.length - shown.length

  function remove(id: string, title: string) {
    // Undoable (03 §2.4): the delete waits for the notice to pass.
    const prev = useChat.getState().threads
    useChat.setState({ threads: prev.filter((t) => t.id !== id) })
    if (activeId === id) navigate({ view: 'chat' })
    let undone = false
    const timer = setTimeout(() => {
      if (!undone) void useChat.getState().remove(id)
    }, 8000)
    useSessionsStore.getState().showNotice(`Deleted “${title}”`, {
      label: 'Undo',
      run: () => {
        undone = true
        clearTimeout(timer)
        void useChat.getState().loadThreads()
      },
    })
  }

  return (
    <div className="ch-side" role="list" aria-label="Conversations" data-testid="chat-list">
      {loaded && !threads.length && <SidebarSubNote>No conversations yet</SidebarSubNote>}
      {shown.map((t) => (
        <SidebarSubItem
          key={t.id}
          active={t.id === activeId}
          title={t.title}
          testId={`chat-row-${t.id}`}
          onClick={() => navigate({ view: 'chat', rest: t.id })}
          trailing={
            <MenuButton
              label={`${t.title} options`}
              onClick={(e) => {
                e.stopPropagation()
                openMenu(
                  e.currentTarget,
                  [
                    { id: 'rename', label: 'Rename', run: () => setRenaming({ id: t.id, title: t.title }) },
                    { id: 'delete', label: 'Delete', danger: true, run: () => remove(t.id, t.title) },
                  ],
                  'Conversation',
                )
              }}
            />
          }
        >
          {t.title}
        </SidebarSubItem>
      ))}
      {hidden > 0 && <SidebarMore onClick={() => setMore(true)}>{`Show ${hidden} more`}</SidebarMore>}
      {more && threads.length > SIDEBAR_LIMIT && (
        <SidebarMore less onClick={() => setMore(false)}>
          Show less
        </SidebarMore>
      )}
      {renaming && <RenameDialog value={renaming.title} onClose={() => setRenaming(null)} onSave={(title) => void useChat.getState().rename(renaming.id, title)} />}
    </div>
  )
}

export function RenameDialog({ value, onClose, onSave }: { value: string; onClose: () => void; onSave: (title: string) => void }) {
  const [title, setTitle] = useState(value)
  const save = () => {
    if (title.trim()) onSave(title.trim())
    onClose()
  }
  return (
    <Dialog title="Rename conversation" onClose={onClose} testId="chat-rename">
      <input
        className="m-input"
        aria-label="Conversation name"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            save()
          }
        }}
      />
      <div className="m-card-actions">
        <Button variant="primary" onClick={save}>
          Rename
        </Button>
        <Button onClick={onClose}>Cancel</Button>
      </div>
    </Dialog>
  )
}
