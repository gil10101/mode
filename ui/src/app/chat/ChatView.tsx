// The Chat view (03 §7; app-window.html data-view="chat"): the toolbar (New
// conversation as a quiet button, ⋮ and the Sources chevron), one column with
// the thread and the input, and the Sources panel, which slides in from 0 to
// 320 px (the conversations live under Chat in the sidebar).
// `chat` opens a new conversation, `chat/<id>` that one (03 §2.5).
// Keys (03 §10.6; printed only in the Shortcuts view): ⌘/Ctrl+N new
// conversation, ⌘/Ctrl+. stop, ⌘/Ctrl+Shift+S the Sources panel,
// ⌘/Ctrl+Shift+/ the `+` menu, ⌥/Alt+1…9 open citation n of the last answer.
import { useEffect, useState } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { Chat } from '../../kit/chat'
import { Icon } from '../../kit/icons'
import { MainBody, MainHead } from '../../kit/layout'
import type { ActivityRowSpec } from '../../state/contract-chat'
import { lastAssistant, SOURCES_SETTING, useChat } from '../../state/chat-store'
import { useSessionsStore } from '../../state/sessions-store'
import { useSetting, useSettingsStore } from '../../state/settings-store'
import { navigate } from '../router'
import { openMenu } from '../ui'
import { AttachDialog } from './AttachDialog'
import { Composer, OPEN_PLUS_EVENT, type AttachKind } from './Composer'
import { Conversation, openItem } from './Conversation'
import { RenameDialog } from './SidebarList'
import { SourcesPanel } from './Sources'
import './chat.css'

const EXAMPLES = ['What did I promise Lena this week?', 'Which sessions failed since Monday?', 'What is on my calendar tomorrow, and what should I prepare?']

export function ChatView({ rest }: { rest?: string }) {
  const id = (rest ?? '').split('/')[0] || null
  const thread = useChat((s) => s.thread)
  const loading = useChat((s) => s.loading)
  const error = useChat((s) => s.error)
  const [sourcesOpen] = useSetting<boolean>(SOURCES_SETTING)
  const status = useChat((s) => s.status)
  const health = useSessionsStore((s) => s.health)
  const [attach, setAttach] = useState<AttachKind | null>(null)
  const [seed, setSeed] = useState<{ text: string; n: number } | null>(null)
  const [renaming, setRenaming] = useState(false)

  useEffect(() => {
    const s = useChat.getState()
    void s.loadStatus()
    void s.loadThreads()
  }, [])

  useEffect(() => {
    void useChat.getState().open(id)
  }, [id])

  // A new conversation gets its id from the first send: keep the route in step.
  // (Read the store, not this render's `thread`: leaving a conversation for `chat` must not bounce back.)
  useEffect(() => {
    const current = useChat.getState().thread
    if (!id && current?.id) navigate({ view: 'chat', rest: current.id }, { replace: true })
  }, [id, thread?.id])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'n') {
        e.preventDefault()
        navigate({ view: 'chat' })
      } else if (mod && !e.shiftKey && e.key === '.') {
        e.preventDefault()
        void useChat.getState().stop()
      } else if (mod && e.shiftKey && e.key.toLowerCase() === 's') {
        e.preventDefault()
        useChat.getState().setSourcesOpen(!useSettingsStore.getState().get<boolean>(SOURCES_SETTING))
      } else if (mod && e.shiftKey && (e.key === '/' || e.key === '?' || e.code === 'Slash')) {
        e.preventDefault()
        window.dispatchEvent(new Event(OPEN_PLUS_EVENT))
      } else if (e.altKey && !mod && /^Digit[1-9]$/.test(e.code)) {
        const n = Number(e.code.slice(5))
        const c = lastAssistant(useChat.getState().thread)?.citations?.find((x) => x.n === n)
        if (c) {
          e.preventDefault()
          openItem(c)
        }
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const onRow = (r: ActivityRowSpec) => useChat.getState().setSourcesOpen(true, r.call_id ?? null)
  const offline = status?.ready === 'offline' || !!health?.offline
  const empty = !loading && (!thread || thread.turns.length === 0)

  return (
    <section className="ap-view ch-view" aria-label="Chat" data-testid="chat-view">
      <MainHead title="Chat" titleTestId="view-title">
        <Button onClick={() => navigate({ view: 'chat' })}>New conversation</Button>
        <MenuButton
          onClick={(e) =>
            openMenu(
              e.currentTarget,
              [
                { id: 'rename', label: 'Rename conversation', dimmed: !thread?.id, run: () => setRenaming(true) },
                { id: 'ai', label: 'AI settings', run: () => navigate({ view: 'settings', rest: 'ai' }) },
              ],
              'Chat',
            )
          }
        />
        <Button
          variant="icon"
          quiet
          className="m-btn-chev"
          aria-label={sourcesOpen ? 'Hide sources' : 'Show sources'}
          aria-expanded={sourcesOpen}
          aria-controls="chat-sources"
          onClick={() => useChat.getState().setSourcesOpen(!sourcesOpen)}
        >
          <Icon id="chevron" />
        </Button>
      </MainHead>
      <MainBody className={`ch-body${sourcesOpen ? ' is-sources' : ''}`} testId="view-body">
        <div className="ch-col">
          {offline && (
            <div className="ap-banner" role="status">
              <span>Offline — showing what Mode has; pollers paused</span>
            </div>
          )}
          <Chat>
            {error && !thread ? (
              <div className="ch-empty" role="alert">
                <p>{error}</p>
                <Button onClick={() => navigate({ view: 'chat' })}>New conversation</Button>
              </div>
            ) : empty ? (
              <div className="ch-empty" data-testid="chat-empty">
                <p>Ask about your notes, sessions, messages and calendar.</p>
                <div className="ch-examples">
                  {EXAMPLES.map((q, i) => (
                    <Button key={q} onClick={() => setSeed({ text: q, n: i + Date.now() })}>
                      {q}
                    </Button>
                  ))}
                </div>
              </div>
            ) : thread ? (
              <Conversation thread={thread} onRow={onRow} />
            ) : (
              <div className="m-chat-thread" aria-busy="true" />
            )}
            <Composer onAttach={setAttach} seed={seed} />
          </Chat>
        </div>
        <SourcesPanel open={sourcesOpen} />
      </MainBody>
      {attach && <AttachDialog kind={attach} onClose={() => setAttach(null)} />}
      {renaming && thread?.id && <RenameDialog value={thread.title} onClose={() => setRenaming(false)} onSave={(t) => void useChat.getState().rename(thread.id, t)} />}
    </section>
  )
}
