// The Chat view's state (03 §7): conversations, the open thread with turns
// assembled from the `chat.delta` / `chat.turn` / `chat.action` streams, the
// action gate (nothing runs until Confirm), attachments, the effort picker,
// the backend state and the Sources panel. The pure helpers at the top are
// the unit-tested core of it; the store is thin over them.
import { create } from 'zustand'
import { invoke, listen, type Unlisten } from '../tauri'
import { EVENTS } from './contract'
import { useSettingsStore } from './settings-store'
import {
  CHAT_EVENTS,
  type ActivityGroup,
  type ChatAction,
  type ChatActionEvent,
  type ChatConnectors,
  type ChatDelta,
  type ChatReadiness,
  type ChatSendResult,
  type ChatSources,
  type ChatStatus,
  type ChatThread,
  type ChatThreadRow,
  type ChatTurn,
  type ChatTurnEvent,
  type Effort,
  type ItemKind,
  type ItemRef,
} from './contract-chat'

/* ---- pure helpers ---------------------------------------------------------------------------- */

export const EFFORT_LABEL: Readonly<Record<Effort, string>> = {
  auto: 'Auto',
  quick: 'Quick',
  thorough: 'Thorough',
  local_only: 'Local-only',
}

export const DEFAULT_CONNECTORS: ChatConnectors = { gmail: true, slack: true, calendar: true }

/** The meta line's short model name: `claude-opus-5-5` → `opus-5-5`; local models as they are. */
export function modelShort(model: string | null | undefined): string {
  if (!model) return ''
  return model.replace(/^claude-/, '').replace(/-\d{8}$/, '')
}

/** Upserts a group by id, keeping order of first appearance. */
export function upsertActivity(list: ActivityGroup[] | undefined, g: ActivityGroup): ActivityGroup[] {
  const out = [...(list ?? [])]
  const i = out.findIndex((x) => x.id === g.id)
  if (i >= 0) out[i] = { ...out[i], ...g, rows: g.rows ?? out[i]!.rows }
  else out.push(g)
  return out
}

function upsertAction(list: ChatAction[] | undefined, a: ChatAction): ChatAction[] {
  const out = [...(list ?? [])]
  const i = out.findIndex((x) => x.id === a.id)
  if (i >= 0) out[i] = { ...out[i], ...a }
  else out.push(a)
  return out
}

function patchTurn(t: ChatThread, id: string, fn: (turn: ChatTurn) => ChatTurn, create?: () => ChatTurn): ChatThread {
  const i = t.turns.findIndex((x) => x.id === id)
  if (i < 0) return create ? { ...t, turns: [...t.turns, fn(create())] } : t
  const turns = [...t.turns]
  turns[i] = fn(turns[i]!)
  return { ...t, turns }
}

function streamingTurn(id: string, now = new Date().toISOString()): ChatTurn {
  return { id, role: 'assistant', text: '', created_at: now, status: 'streaming', activity: [], actions: [], citations: [] }
}

/** `chat.delta`: text appends; an activity group upserts (the stream's running assembly). */
export function applyDelta(t: ChatThread, d: ChatDelta): ChatThread {
  if (!d?.turn_id) return t
  return patchTurn(
    t,
    d.turn_id,
    (turn) => ({
      ...turn,
      text: d.text ? turn.text + d.text : turn.text,
      activity: d.activity ? upsertActivity(turn.activity, d.activity) : turn.activity,
      status: turn.status === 'finished' || turn.status === 'failed' || turn.status === 'stopped' ? turn.status : 'streaming',
    }),
    () => streamingTurn(d.turn_id),
  )
}

/** `chat.turn`: started → a streaming turn; finished → the stored turn (or the usage); failed / stopped keep the partial text. */
export function applyTurnEvent(t: ChatThread, e: ChatTurnEvent): ChatThread {
  if (!e?.turn_id) return t
  if (e.status === 'started')
    return patchTurn(
      t,
      e.turn_id,
      (turn) => ({ ...turn, status: 'streaming' }),
      () => streamingTurn(e.turn_id),
    )
  const tokens = e.usage ? e.usage.input_tokens + e.usage.output_tokens : undefined
  return patchTurn(
    t,
    e.turn_id,
    (turn) => {
      const base: ChatTurn = e.turn ? { ...turn, ...e.turn, actions: e.turn.actions ?? turn.actions } : turn
      const activity = (base.activity ?? []).map((g) => (g.active ? { ...g, active: false } : g))
      return {
        ...base,
        activity,
        status: e.status === 'finished' ? 'finished' : e.status === 'stopped' ? 'stopped' : 'failed',
        error: e.status === 'failed' ? (e.error ?? 'failed') : (base.error ?? null),
        tokens: tokens ?? base.tokens ?? null,
      }
    },
    () => streamingTurn(e.turn_id),
  )
}

/** `chat.action`: a proposal (or its result) upserts into the turn's actions. */
export function applyActionEvent(t: ChatThread, e: ChatActionEvent): ChatThread {
  if (!e?.turn_id || !e.action) return t
  return patchTurn(t, e.turn_id, (turn) => ({ ...turn, actions: upsertAction(turn.actions, e.action) }))
}

/** Every action still waiting for the user, in order (batched proposals confirm one by one). */
export function pendingActions(t: ChatThread | null): { turnId: string; action: ChatAction }[] {
  if (!t) return []
  const out: { turnId: string; action: ChatAction }[] = []
  for (const turn of t.turns) for (const a of turn.actions ?? []) if (a.state === 'pending') out.push({ turnId: turn.id, action: a })
  return out
}

/** The action gate: only a pending action can be confirmed or rejected, and only the first of a batch. */
export function canDecide(t: ChatThread | null, turnId: string, actionId: string): boolean {
  const first = pendingActions(t)[0]
  return !!first && first.turnId === turnId && first.action.id === actionId
}

export function streamingTurnOf(t: ChatThread | null): ChatTurn | null {
  if (!t) return null
  for (let i = t.turns.length - 1; i >= 0; i--) if (t.turns[i]!.status === 'streaming') return t.turns[i]!
  return null
}

export function lastAssistant(t: ChatThread | null): ChatTurn | null {
  if (!t) return null
  for (let i = t.turns.length - 1; i >= 0; i--) if (t.turns[i]!.role === 'assistant') return t.turns[i]!
  return null
}

/** The Sources rail: one icon per kind with a plain count (app-window.html `#h-rail`). */
export const RAIL_KINDS: readonly { kind: ItemKind; icon: 'notes' | 'sessions' | 'inbox' | 'calendar' | 'file'; label: string }[] = [
  { kind: 'note', icon: 'notes', label: 'notes' },
  { kind: 'session', icon: 'sessions', label: 'sessions' },
  { kind: 'message', icon: 'inbox', label: 'messages' },
  { kind: 'event', icon: 'calendar', label: 'events' },
  { kind: 'file', icon: 'file', label: 'files' },
]

export function railCounts(s: ChatSources | null): { kind: ItemKind; count: number }[] {
  if (!s) return []
  const seen = new Map<ItemKind, Set<string>>()
  for (const c of s.calls)
    for (const it of c.items) {
      if (!seen.has(it.kind)) seen.set(it.kind, new Set())
      seen.get(it.kind)!.add(it.id)
    }
  return RAIL_KINDS.filter((k) => seen.has(k.kind)).map((k) => ({ kind: k.kind, count: seen.get(k.kind)!.size }))
}

/** "just now", "4m", "2h", "3d" (03 §7.1 reply footer; 10 §12.1 relative times in rows). */
export function relTime(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return ''
  const t = Date.parse(iso)
  if (!Number.isFinite(t)) return ''
  const s = Math.max(0, Math.round((now - t) / 1000))
  if (s < 60) return 'just now'
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `${h}h`
  return `${Math.floor(h / 24)}d`
}

/** A chip's words: the item's title; a picked file (`chat_pick_file`: its absolute path as the id, no route) falls back to the file name. */
export function itemTitle(it: Pick<ItemRef, 'kind' | 'id' | 'title'>): string {
  const t = (it.title ?? '').trim()
  if (t) return t
  return it.kind === 'file' ? (it.id.replace(/^file:\/\//, '').split(/[\\/]/).filter(Boolean).pop() ?? it.id) : it.id
}

/** Kind → the route that opens it when a citation carries none (03 §2.5); a file has none. */
export function itemRoute(it: Pick<ItemRef, 'kind' | 'id' | 'route'>): string | null {
  if (it.route) return it.route
  switch (it.kind) {
    case 'note':
      return `notes/${it.id}`
    case 'session':
      return `sessions/${it.id}`
    case 'message':
      return `inbox/thread/${it.id}`
    case 'event':
      return `calendar/${it.id}`
    default:
      return null
  }
}

/** The conversation title from its first user turn (03 §7.1). */
export function titleFrom(text: string): string {
  const line = text.split('\n').find((l) => l.trim()) ?? ''
  const t = line.trim()
  return t.length > 60 ? `${t.slice(0, 59).trimEnd()}…` : t || 'New conversation'
}

/** `ready` with the core's `no_key` + `state: 'invalid_key'` pair read as `invalid_key` (a rejected key, 06 §10.7). */
export function readinessOf(s: Pick<ChatStatus, 'ready' | 'state'>): ChatReadiness {
  if (s.ready === 'no_key' && s.state === 'invalid_key') return 'invalid_key'
  return s.ready
}

/** The status as the views read it: `ready` normalized (`readinessOf`). */
export function normalizeStatus(s: ChatStatus): ChatStatus {
  const ready = readinessOf(s)
  return ready === s.ready ? s : { ...s, ready }
}

/** The view's own words for a state that blocks the input (used when the core sends no `message`). */
const BLOCKED_LINE: Partial<Record<ChatReadiness, string>> = {
  no_key: 'Add your Anthropic API key to use Chat',
  invalid_key: 'The Anthropic API key was rejected; replace it in Settings → AI',
  local_only_no_backend: 'Local-only is on and no local backend is configured.',
  budget: 'Monthly AI budget reached',
}

/** The disabled-input line of 03 §7.2 for a backend state, or null when Chat can send: the core's `message`, else the view's words. */
export function disabledReason(s: ChatStatus | null): string | null {
  if (!s) return null
  const ready = readinessOf(s)
  const own = BLOCKED_LINE[ready]
  if (!own && s.input_disabled !== true) return null
  const core = typeof s.message === 'string' ? s.message.trim() : ''
  return core && core !== 'Ready' ? core : (own ?? 'Chat is not available')
}

/**
 * XC-24: what leaves the device. The core's `privacy_line` when it was computed for this effort (it names a LAN
 * backend as such); the view's own words otherwise (an older core, or a status asked about another effort).
 */
export function privacyLine(s: ChatStatus | null, effort: Effort): string | null {
  if (!s) return null
  const core = typeof s.privacy_line === 'string' ? s.privacy_line.trim() : ''
  if (core && (s.effort === undefined || s.effort === effort)) return core
  if (s.local || effort === 'local_only' || s.local_only) return 'Nothing leaves this machine'
  return `Your question and the retrieved snippets are sent to ${s.backend_label}`
}

/* ---- the store ---------------------------------------------------------------------------------- */

/** Open or closed is a device setting (03 §7.1, §8.21); it was a `localStorage` flag. */
export const SOURCES_SETTING = 'chat.sources_open'

let localSeq = 0

function message(e: unknown): string {
  if (e && typeof e === 'object' && 'message' in e) return String((e as { message: unknown }).message)
  return String(e)
}

export interface ChatState {
  status: ChatStatus | null
  threads: ChatThreadRow[]
  threadsLoaded: boolean
  /** The open conversation; a new one has `id === ''` until the first send answers. */
  thread: ChatThread | null
  loading: boolean
  error: string | null
  /** Effort and connectors of a new conversation (before it has an id). */
  draftEffort: Effort
  draftConnectors: ChatConnectors
  attachments: ItemRef[]
  sources: ChatSources | null
  /** The Sources entry an activity row opened. */
  focusCall: string | null
  sending: boolean
  reset(): void
  loadStatus(): Promise<void>
  loadThreads(): Promise<void>
  open(id: string | null): Promise<void>
  send(text: string): Promise<string | null>
  stop(): Promise<void>
  /** Resends the user turn before a failed reply (03 §7.6 Retry). */
  retry(turnId: string): Promise<string | null>
  decide(turnId: string, actionId: string, confirm: boolean): Promise<ChatAction | null>
  setEffort(e: Effort): void
  setConnector(k: keyof ChatConnectors, on: boolean): void
  attach(item: ItemRef): void
  detach(kind: ItemKind, id: string): void
  rename(id: string, title: string): Promise<void>
  remove(id: string): Promise<void>
  pin(turnId: string, pinned: boolean): Promise<void>
  setSourcesOpen(open: boolean, focusCall?: string | null): void
  loadSources(turnId: string): Promise<void>
  onDelta(d: ChatDelta): void
  onTurn(e: ChatTurnEvent): void
  onAction(e: ChatActionEvent): void
}

function emptyThread(effort: Effort, connectors: ChatConnectors): ChatThread {
  const now = new Date().toISOString()
  return { id: '', title: 'New conversation', created_at: now, updated_at: now, effort, connectors, turns: [] }
}

const initial = {
  status: null,
  threads: [],
  threadsLoaded: false,
  thread: null,
  loading: false,
  error: null,
  draftEffort: 'auto' as Effort,
  draftConnectors: DEFAULT_CONNECTORS,
  attachments: [],
  sources: null,
  focusCall: null,
  sending: false,
}

export const useChat = create<ChatState>((set, get) => ({
  ...initial,
  reset() {
    set({ ...initial })
  },
  async loadStatus() {
    // The core words the privacy line and the blocked states for the effort the composer shows (03 §7.2).
    const effort = get().thread?.effort ?? get().draftEffort
    try {
      const status = await invoke<ChatStatus>('chat_status', effort === 'auto' ? undefined : { effort })
      if (status && typeof status === 'object') set({ status: normalizeStatus(status) })
    } catch {
      /* the meta line waits for the core */
    }
  },
  async loadThreads() {
    try {
      const threads = await invoke<ChatThreadRow[]>('chat_threads_list')
      set({ threads, threadsLoaded: true })
    } catch (e) {
      set({ threadsLoaded: true, error: message(e) })
    }
  },
  async open(id) {
    if (!id) {
      const s = get()
      if (s.thread && s.thread.id === '') return
      set({ thread: emptyThread(s.draftEffort, s.draftConnectors), sources: null, focusCall: null, attachments: [], error: null, loading: false })
      return
    }
    if (get().thread?.id === id) return
    set({ loading: true, error: null, sources: null, focusCall: null })
    try {
      const thread = await invoke<ChatThread>('chat_thread_get', { id })
      set({ thread, loading: false, attachments: [] })
      // A conversation kept at another effort: the status (privacy line, blocked states) for that one.
      const status = get().status
      if (status?.effort && thread?.effort && status.effort !== thread.effort) void get().loadStatus()
      const last = lastAssistant(thread)
      if (last) void get().loadSources(last.id)
    } catch (e) {
      set({ loading: false, error: message(e), thread: null })
    }
  },
  async send(text) {
    const s = get()
    const body = text.trim()
    if (!body || s.sending || streamingTurnOf(s.thread)) return null
    if (disabledReason(s.status)) return null
    const thread = s.thread ?? emptyThread(s.draftEffort, s.draftConnectors)
    const userTurn: ChatTurn = { id: `local-${++localSeq}`, role: 'user', text: body, created_at: new Date().toISOString(), attachments: s.attachments }
    const isNew = !thread.id
    set({ thread: { ...thread, title: isNew ? titleFrom(body) : thread.title, turns: [...thread.turns, userTurn] }, sending: true, attachments: [], error: null })
    try {
      const args: Record<string, unknown> = { text: body, effort: thread.effort }
      if (thread.id) args.thread_id = thread.id
      if (userTurn.attachments?.length) args.attachments = userTurn.attachments.map((a) => ({ kind: a.kind, id: a.id }))
      if (isNew) args.connectors = thread.connectors
      const r = await invoke<ChatSendResult>('chat_send', args)
      set((st) => {
        const t = st.thread
        if (!t) return { sending: false }
        let next: ChatThread = { ...t, id: t.id || r.thread_id }
        next = {
          ...next,
          turns: next.turns.map((x) => (x.id === userTurn.id && r.user_turn_id ? { ...x, id: r.user_turn_id } : x)),
        }
        if (!next.turns.some((x) => x.id === r.turn_id)) next = { ...next, turns: [...next.turns, streamingTurn(r.turn_id)] }
        return { thread: next, sending: false }
      })
      if (isNew) void get().loadThreads()
      return r.turn_id
    } catch (e) {
      // The inline error on the turn (03 §7.6) with Retry; the user's text stays in the thread.
      const failed: ChatTurn = { id: `local-${++localSeq}`, role: 'assistant', text: '', created_at: new Date().toISOString(), status: 'failed', error: message(e) }
      set((st) => ({ sending: false, thread: st.thread ? { ...st.thread, turns: [...st.thread.turns, failed] } : st.thread }))
      return null
    }
  },
  async retry(turnId) {
    const t = get().thread
    if (!t) return null
    const i = t.turns.findIndex((x) => x.id === turnId)
    if (i < 1 || t.turns[i]!.status !== 'failed') return null
    const user = t.turns[i - 1]!
    if (user.role !== 'user') return null
    set({ thread: { ...t, turns: t.turns.filter((_, k) => k !== i && k !== i - 1) }, attachments: user.attachments ?? [] })
    return get().send(user.text)
  },
  async stop() {
    const turn = streamingTurnOf(get().thread)
    if (!turn) return
    try {
      await invoke('chat_stop', { turn_id: turn.id })
    } catch {
      /* the turn event reports the outcome */
    }
  },
  async decide(turnId, actionId, confirm) {
    const t = get().thread
    // The gate (03 §7.5): only the first pending proposal, only by the user's click or key.
    if (!canDecide(t, turnId, actionId)) return null
    set((st) => ({
      thread: st.thread
        ? patchTurn(st.thread, turnId, (turn) => ({
            ...turn,
            actions: (turn.actions ?? []).map((a) => (a.id === actionId ? { ...a, state: confirm ? 'running' : 'rejected' } : a)),
          }))
        : st.thread,
    }))
    try {
      const a = await invoke<ChatAction>('chat_action_confirm', { turn_id: turnId, action_id: actionId, confirm })
      if (a) get().onAction({ turn_id: turnId, action: a })
      return a
    } catch (e) {
      set((st) => ({
        thread: st.thread
          ? patchTurn(st.thread, turnId, (turn) => ({
              ...turn,
              actions: (turn.actions ?? []).map((x) => (x.id === actionId ? { ...x, state: 'failed', error: message(e) } : x)),
            }))
          : st.thread,
      }))
      return null
    }
  },
  setEffort(e) {
    const t = get().thread
    const before = t?.effort ?? get().draftEffort
    set({ draftEffort: t?.id ? get().draftEffort : e, thread: t ? { ...t, effort: e } : t })
    if (t?.id) void invoke('chat_thread_set', { id: t.id, effort: e }).catch(() => {})
    if (e !== before) void get().loadStatus()
  },
  setConnector(k, on) {
    const t = get().thread
    const connectors = { ...(t?.connectors ?? get().draftConnectors), [k]: on }
    set({ thread: t ? { ...t, connectors } : t, draftConnectors: t?.id ? get().draftConnectors : connectors })
    if (t?.id) void invoke('chat_thread_set', { id: t.id, connectors }).catch(() => {})
  },
  attach(item) {
    const list = get().attachments
    if (list.some((a) => a.kind === item.kind && a.id === item.id)) return
    set({ attachments: [...list, item] })
  },
  detach(kind, id) {
    set({ attachments: get().attachments.filter((a) => !(a.kind === kind && a.id === id)) })
  },
  async rename(id, title) {
    const t = title.trim()
    if (!t) return
    set((st) => ({
      threads: st.threads.map((r) => (r.id === id ? { ...r, title: t } : r)),
      thread: st.thread?.id === id ? { ...st.thread, title: t } : st.thread,
    }))
    try {
      await invoke('chat_thread_rename', { id, title: t })
    } catch (e) {
      set({ error: message(e) })
      void get().loadThreads()
    }
  },
  async remove(id) {
    set((st) => ({ threads: st.threads.filter((r) => r.id !== id), thread: st.thread?.id === id ? null : st.thread }))
    try {
      await invoke('chat_thread_delete', { id })
    } catch (e) {
      set({ error: message(e) })
      void get().loadThreads()
    }
  },
  async pin(turnId, pinned) {
    set((st) => ({ thread: st.thread ? patchTurn(st.thread, turnId, (t) => ({ ...t, pinned })) : st.thread }))
    try {
      await invoke('chat_pin', { turn_id: turnId, pinned })
      void get().loadThreads()
    } catch (e) {
      set((st) => ({ error: message(e), thread: st.thread ? patchTurn(st.thread, turnId, (t) => ({ ...t, pinned: !pinned })) : st.thread }))
    }
  },
  setSourcesOpen(open, focusCall = null) {
    void useSettingsStore.getState().set(SOURCES_SETTING, open)
    set({ focusCall: open ? focusCall : null })
  },
  async loadSources(turnId) {
    try {
      const sources = await invoke<ChatSources>('chat_sources', { turn_id: turnId })
      if (lastAssistant(get().thread)?.id === turnId || get().sources?.turn_id === turnId || !get().sources) set({ sources })
    } catch {
      /* the panel keeps the previous turn's entries */
    }
  },
  onDelta(d) {
    const t = get().thread
    if (!t || !d) return
    if (d.thread_id && t.id && d.thread_id !== t.id) return
    set({ thread: applyDelta(t.id || !d.thread_id ? t : { ...t, id: d.thread_id }, d) })
  },
  onTurn(e) {
    const t = get().thread
    if (!t || !e) return
    if (e.thread_id && t.id && e.thread_id !== t.id) {
      if (e.status !== 'started') void get().loadThreads()
      return
    }
    set({ thread: applyTurnEvent(t.id || !e.thread_id ? t : { ...t, id: e.thread_id }, e) })
    if (e.status !== 'started') {
      void get().loadSources(e.turn_id)
      void get().loadThreads()
    }
  },
  onAction(e) {
    const t = get().thread
    if (!t || !e) return
    if (e.thread_id && t.id && e.thread_id !== t.id) return
    set({ thread: applyActionEvent(t, e) })
  },
}))

const AI_KEYS = /^ai\./

/** Subscribes the store to the chat streams and the backend state. */
export function connectChat(): Promise<Unlisten> {
  const offs = [
    listen<ChatDelta>(CHAT_EVENTS.delta, (d) => useChat.getState().onDelta(d)),
    listen<ChatTurnEvent>(CHAT_EVENTS.turn, (e) => useChat.getState().onTurn(e)),
    listen<ChatActionEvent>(CHAT_EVENTS.action, (e) => useChat.getState().onAction(e)),
    listen<Record<string, unknown>>(EVENTS.settings, (p) => {
      if (p && typeof p === 'object' && Object.keys(p).some((k) => AI_KEYS.test(k))) void useChat.getState().loadStatus()
    }),
    listen<{ offline?: boolean }>(EVENTS.health, () => void useChat.getState().loadStatus()),
  ]
  return Promise.resolve(() => {
    for (const off of offs) void off.then((u) => u())
  })
}
