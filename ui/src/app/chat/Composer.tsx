// The input of 03 §7.2: attached items as chips above it, the slash-command
// popover, the radius-24 container with `+`, the field (growing to six lines)
// and ↵ (Stop while a reply streams), then the meta line — the effort picker
// left, the model right (the only place Chat names a model, ADR-0037) — and
// the privacy line (XC-24). Disabled states say why and link to Settings → AI.
// The empty field carries the kit's `.is-placeholder` (app-window.html `#h-input`).
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react'
import { BrandMark } from '../../kit/brands'
import { Button } from '../../kit/button'
import { ChatMeta, SlashPop } from '../../kit/chat'
import { Menu, MenuItem, MenuSep, Notice, Switch } from '../../kit/controls'
import { SLASH_COMMANDS, type ChatConnectors, type ItemKind, type ItemRef } from '../../state/contract-chat'
import { invoke } from '../../tauri'
import { disabledReason, EFFORT_LABEL, modelShort, privacyLine, streamingTurnOf, useChat } from '../../state/chat-store'
import { useSessionsStore } from '../../state/sessions-store'
import { navigate } from '../router'
import { openMenu, type MenuEntry } from '../ui'
import { ItemChip } from './Conversation'
import { AnchoredPop } from './Popover'

const PLACEHOLDER = 'Ask about your notes, sessions and messages…'
const MAX_LINES = 6

export type AttachKind = Extract<ItemKind, 'note' | 'message' | 'session'>

export interface ComposerProps {
  onAttach: (kind: AttachKind) => void
  /** Text handed in from outside (an example question, a slash command from the `+` menu). */
  seed?: { text: string; n: number } | null
}

/** Opens the `+` menu from the view's ⌘/Ctrl+Shift+/ (03 §10.6). */
export const OPEN_PLUS_EVENT = 'mode:chat-plus'

export function Composer({ onAttach, seed }: ComposerProps) {
  const status = useChat((s) => s.status)
  const thread = useChat((s) => s.thread)
  const attachments = useChat((s) => s.attachments)
  const sending = useChat((s) => s.sending)
  const draftEffort = useChat((s) => s.draftEffort)
  const draftConnectors = useChat((s) => s.draftConnectors)
  const health = useSessionsStore((s) => s.health)
  const [text, setText] = useState('')
  const [slashIndex, setSlashIndex] = useState(0)
  const [plus, setPlus] = useState<{ anchor: DOMRect; page: 'main' | 'connectors' } | null>(null)
  const field = useRef<HTMLTextAreaElement>(null)
  const plusBtn = useRef<HTMLButtonElement>(null)
  const box = useRef<HTMLDivElement>(null)

  const effort = thread?.effort ?? draftEffort
  const connectors: ChatConnectors = thread?.connectors ?? draftConnectors
  const streaming = streamingTurnOf(thread)
  const blocked = disabledReason(status)
  const busy = !!streaming || sending
  const slash = text.startsWith('/') && !text.includes(' ') && !text.includes('\n') ? SLASH_COMMANDS.filter((c) => c.name.startsWith(text.toLowerCase())) : []
  const showSlash = slash.length > 0 && !blocked

  useEffect(() => {
    if (seed) {
      setText(seed.text)
      field.current?.focus()
    }
  }, [seed])

  useEffect(() => setSlashIndex(0), [text])

  // Grows with the text up to six lines; one line keeps the stylesheet's height.
  useLayoutEffect(() => {
    const el = field.current
    if (!el) return
    el.style.height = ''
    const cs = getComputedStyle(el)
    const line = parseFloat(cs.lineHeight) || 21
    const max = line * MAX_LINES + (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0)
    if (el.scrollHeight > el.clientHeight + 1) el.style.height = `${Math.min(el.scrollHeight, max)}px`
    el.style.overflowY = el.scrollHeight > max ? 'auto' : ''
  }, [text])

  useEffect(() => {
    const open = () => {
      if (plusBtn.current) setPlus({ anchor: plusBtn.current.getBoundingClientRect(), page: 'main' })
    }
    window.addEventListener(OPEN_PLUS_EVENT, open)
    return () => window.removeEventListener(OPEN_PLUS_EVENT, open)
  }, [])

  async function send() {
    const body = text.trim()
    if (!body || busy || blocked) return
    // `/open` with an attached item opens it here (03 §7.2); the rest goes to the assistant.
    if (/^\/open\b/.test(body) && attachments.length) {
      const a = attachments[0]!
      setText('')
      useChat.getState().detach(a.kind, a.id)
      if (a.route) navigate(a.route)
      return
    }
    setText('')
    const id = await useChat.getState().send(body)
    if (!id && useChat.getState().thread?.turns.at(-1)?.role !== 'assistant') setText(body)
  }

  function pickSlash(name: string) {
    setText(`${name} `)
    field.current?.focus()
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (showSlash) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault()
        setSlashIndex((i) => (i + (e.key === 'ArrowDown' ? 1 : -1) + slash.length) % slash.length)
        return
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && text.length > 1 && slash[slashIndex]?.name !== text.trim())) {
        e.preventDefault()
        pickSlash(slash[slashIndex]!.name)
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        setText('')
        return
      }
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault()
      void send()
    }
  }

  function effortMenu() {
    const anchor = box.current?.querySelector('.m-chat-meta .m-effort')
    if (!anchor) return
    const local = status?.local_available ?? false
    const entries: MenuEntry[] = (['auto', 'quick', 'thorough', 'local_only'] as const).map((k) => ({
      id: k,
      label: EFFORT_LABEL[k],
      checked: effort === k,
      dimmed: k === 'local_only' && !local,
      reason: k === 'local_only' ? (local ? 'local models may handle actions less reliably' : 'Needs a local backend') : undefined,
      run: () => useChat.getState().setEffort(k),
    }))
    openMenu(anchor, entries, 'Effort')
  }

  const accounts = status?.accounts ?? { gmail: true, slack: true, calendar: true }
  const noMessages = !accounts.gmail && !accounts.slack
  const sessionsCount = useSessionsStore((s) => s.rows.length)
  const plusMain = (
    <Menu plus label="Add to the conversation" onClose={() => setPlus(null)}>
      <MenuItem icon="notes" onSelect={() => (setPlus(null), onAttach('note'))}>
        Attach note
      </MenuItem>
      <MenuItem icon="inbox" dimmed={noMessages} reason={noMessages ? 'Connect Gmail or Slack first' : undefined} onSelect={() => (setPlus(null), onAttach('message'))}>
        Attach message
      </MenuItem>
      <MenuItem icon="sessions" dimmed={sessionsCount === 0} reason={sessionsCount === 0 ? 'No sessions yet' : undefined} onSelect={() => (setPlus(null), onAttach('session'))}>
        Attach session
      </MenuItem>
      <MenuItem
        icon="file"
        onSelect={() => {
          setPlus(null)
          invoke<ItemRef | null>('chat_pick_file').then(
            (f) => f && useChat.getState().attach(f),
            () => {},
          )
        }}
      >
        Add file
      </MenuItem>
      <MenuItem
        icon="slash"
        onSelect={() => {
          setPlus(null)
          setText('/')
          field.current?.focus()
        }}
      >
        Slash commands
      </MenuItem>
      <MenuSep />
      <MenuItem icon="link" hint="›" onSelect={() => setPlus((p) => (p ? { ...p, page: 'connectors' } : p))}>
        Connectors
      </MenuItem>
    </Menu>
  )
  const plusConnectors = (
    <Menu plus label="Connectors for this conversation" onClose={() => setPlus(null)}>
      {(
        [
          ['gmail', 'Gmail', accounts.gmail, 'Connect Gmail first', 'gmail'],
          ['slack', 'Slack', accounts.slack, 'Connect Slack first', 'slack'],
          ['calendar', 'Calendar', accounts.calendar, 'Connect Google Calendar first', 'googlecalendar'],
        ] as const
      ).map(([k, label, ok, reason, brand]) => (
        <MenuItem
          key={k}
          mark={<BrandMark id={brand} size={16} label="" />}
          dimmed={!ok}
          reason={ok ? undefined : reason}
          trailing={<Switch on={ok && connectors[k]} label={`${label} in this conversation`} disabled={!ok} onChange={(v) => useChat.getState().setConnector(k, v)} />}
          onSelect={() => useChat.getState().setConnector(k, !connectors[k])}
        >
          {label}
        </MenuItem>
      ))}
    </Menu>
  )

  const model = modelShort(status?.model)
  const privacy = privacyLine(status, effort)
  const offline = status?.ready === 'offline' || !!health?.offline
  const unreachable = status?.ready === 'unreachable'

  return (
    <div className="ch-compose" ref={box}>
      {showSlash && (
        <div className="ch-slash">
          <SlashPop commands={slash} active={slash[slashIndex]?.name} onSelect={pickSlash} />
        </div>
      )}
      {attachments.length > 0 && (
        <div className="ch-attach" aria-label="Attached">
          {attachments.map((a) => (
            <ItemChip key={`${a.kind}:${a.id}`} item={a} onRemove={() => useChat.getState().detach(a.kind, a.id)} />
          ))}
        </div>
      )}
      {blocked && (
        <Notice
          className="ch-notice"
          testId="chat-blocked"
          action={
            <Button quiet onClick={() => navigate({ view: 'settings', rest: 'ai' })}>
              Open AI settings
            </Button>
          }
        >
          {blocked}
        </Notice>
      )}
      {unreachable && (
        <Notice
          state="bad"
          className="ch-notice"
          testId="chat-unreachable"
          action={
            <Button quiet onClick={() => void useChat.getState().loadStatus()}>
              Retry
            </Button>
          }
        >
          {status?.error ?? `Can't reach ${status?.backend_label ?? 'the backend'}`}
        </Notice>
      )}
      <div className={`m-chat-input${text.length ? ' has-text' : ''}${blocked ? ' is-disabled' : ''}`}>
        <button type="button" className="m-plus" aria-label="Add" ref={plusBtn} disabled={!!blocked} onClick={(e) => setPlus({ anchor: e.currentTarget.getBoundingClientRect(), page: 'main' })}>
          +
        </button>
        <textarea
          ref={field}
          className={`m-field ch-field${text.length ? '' : ' is-placeholder'}`}
          rows={1}
          placeholder={PLACEHOLDER}
          aria-label="Message"
          value={text}
          disabled={!!blocked}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
        />
        {busy && streaming ? (
          <button type="button" className="m-send" aria-label="Stop" onClick={() => void useChat.getState().stop()}>
            ■
          </button>
        ) : (
          <button type="button" className="m-send" aria-label="Send" disabled={!!blocked} onClick={() => void send()}>
            ↵
          </button>
        )}
      </div>
      <ChatMeta effort={EFFORT_LABEL[effort]} onEffort={effortMenu} model={model} busy={busy} />
      {privacy && !blocked && (
        <div className="m-t-meta ch-privacy" data-testid="chat-privacy">
          {offline && !(status?.local || effort === 'local_only') ? 'Offline · turns to the Claude API fail until the network is back' : privacy}
        </div>
      )}
      {plus && (
        <AnchoredPop anchor={plus.anchor} onClose={() => setPlus(null)} testId="chat-plus">
          {plus.page === 'main' ? plusMain : plusConnectors}
        </AnchoredPop>
      )}
    </div>
  )
}
