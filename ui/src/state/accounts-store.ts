// Accounts and sources (06 §3, §4, §9A; 03 §7C): the rows every connector
// surface reads (Integrations cards and pages, the Inbox and Calendar
// banners), the Connect / Re-authorize flow through the OAuth loopback
// (`account_add_start` → the browser → `account_add_finish`), Pause / Resume
// (optimistic, rolled back on failure), Remove (confirmation, 03 §2.4) and
// the Slack channel picker (06 §8.4). The core owns every rule; the store
// keeps what it last said plus the flow in progress.
import { create } from 'zustand'
import { invoke, listen } from '../tauri'
import { cmdErrorText, EVENTS } from './contract'
import type { AccountRow, AuthStart, ConnectOptions, CoreAccountExtras, CoreSlackChannels, Provider, SlackChannel, SourceRow } from './contract-connectors'
import { useSessionsStore } from './sessions-store'

/** The three connector cards of 03 §7C.2. */
export type ConnectorCardKind = 'gmail' | 'slack' | 'google-calendar'

export type FlowPhase = 'setup' | 'starting' | 'waiting' | 'done' | 'failed'

/** One Connect / Re-authorize run (06 §4.1 as the UI sees it). */
export interface ConnectFlow {
  provider: Provider
  card: ConnectorCardKind
  mode: 'add' | 'reauth'
  accountId?: string
  /** For the dialog title ("Re-authorize Acme Slack"). */
  accountName?: string
  phase: FlowPhase
  /** Google: the per-service toggles before consent. */
  services: ('gmail' | 'calendar')[]
  /** Google: read-only consent; Slack: the manifest without `*:write`. */
  readOnly: boolean
  /** Slack: the workspace app's client id (06 §8.1 step 4). */
  clientId: string
  authUrl?: string
  /** The flow's `state` (for `account_add_cancel`). */
  state?: string
  /** The browser could not be opened: the dialog shows the link with Copy link (06 §4.4). */
  openFailed?: boolean
  error?: string
  account?: AccountRow
  /**
   * Started from a row that shows the wait and the answer itself (the onboarding Inbox and calendar step,
   * 03 §2.8): the dialog shows only the setup (Google's toggles, Slack's manifest steps) and closes once
   * the browser is asked.
   */
  inline?: boolean
}

export interface ChannelPicker {
  accountId: string
  channels: SlackChannel[] | null
  /** The core's "n channels · ≈ m requests/min at your interval" for the watched set when listed. */
  budgetLine?: string | null
  /** Ids ticked in the dialog (starts from `watched`). */
  picked: string[]
  query: string
  error: string | null
  saving: boolean
}

export interface AccountsState {
  accounts: AccountRow[]
  sources: SourceRow[]
  loaded: boolean
  error: string | null
  flow: ConnectFlow | null
  removing: AccountRow | null
  picker: ChannelPicker | null
  load(): Promise<void>
  /** Opens the Connect dialog for a card (setup phase: toggles, manifest steps); `inline`: see ConnectFlow. */
  startConnect(card: ConnectorCardKind, opts?: { services?: ('gmail' | 'calendar')[]; inline?: boolean }): void
  /** Re-authorize (and Reconnect): straight to the browser with the account's own toggles (or `services`, "Re-authorize with Calendar"). */
  startReauth(id: string, opts?: { services?: ('gmail' | 'calendar')[] }): Promise<void>
  patchFlow(patch: Partial<ConnectFlow>): void
  /** Runs `account_add_start`, opens the URL, waits on `account_add_finish`. */
  begin(): Promise<void>
  cancelFlow(): void
  pause(id: string): Promise<void>
  resume(id: string): Promise<void>
  /** `error` → Retry (03 §4.9, §7C.3). */
  retry(id: string): Promise<void>
  askRemove(id: string): void
  cancelRemove(): void
  confirmRemove(): Promise<void>
  setSource(id: string, patch: { enabled?: boolean; vip?: boolean }): Promise<void>
  openPicker(accountId: string): Promise<void>
  patchPicker(patch: Partial<ChannelPicker>): void
  savePicker(): Promise<boolean>
  closePicker(): void
}

/** A rejection as text (the core's `{ code, message }`; contract.ts's `toCmdError`). */
export function errorText(e: unknown): string {
  return cmdErrorText(e)
}

function notice(text: string, action?: { label: string; run: () => void }) {
  useSessionsStore.getState().showNotice(text, action)
}

export const CARD_PROVIDER: Record<ConnectorCardKind, Provider> = { gmail: 'google', 'google-calendar': 'google', slack: 'slack' }

/** Flow runs are numbered so a canceled run's late answer is ignored (the core times out after 5 min, 06 §4.3). */
let flowSeq = 0

export const useAccounts = create<AccountsState>((set, get) => ({
  accounts: [],
  sources: [],
  loaded: false,
  error: null,
  flow: null,
  removing: null,
  picker: null,

  async load() {
    try {
      const [accounts, sources] = await Promise.all([invoke<(AccountRow & CoreAccountExtras)[]>('accounts_list'), invoke<SourceRow[]>('sources_list', {}).catch(() => [] as SourceRow[])])
      set({ accounts: sortAccounts((accounts ?? []).map(normalizeAccount)), sources: sources ?? [], loaded: true, error: null })
    } catch (e) {
      set({ loaded: true, error: errorText(e) })
    }
  },

  startConnect(card, opts) {
    flowSeq++
    const provider = CARD_PROVIDER[card]
    set({
      flow: {
        provider,
        card,
        mode: 'add',
        phase: 'setup',
        services: opts?.services ?? (card === 'slack' ? [] : ['gmail', 'calendar']),
        readOnly: false,
        clientId: '',
        inline: opts?.inline,
      },
    })
  },

  async startReauth(id, opts) {
    const a = get().accounts.find((x) => x.id === id)
    if (!a) return
    const run = ++flowSeq
    const card: ConnectorCardKind = a.provider === 'slack' ? 'slack' : a.services.includes('gmail') ? 'gmail' : 'google-calendar'
    set({
      flow: {
        provider: a.provider,
        card,
        mode: 'reauth',
        accountId: id,
        accountName: a.display_name,
        phase: 'starting',
        services: opts?.services ?? a.services.filter((s): s is 'gmail' | 'calendar' => s === 'gmail' || s === 'calendar'),
        readOnly: false,
        clientId: '',
      },
    })
    try {
      // Different services (Re-authorize with Calendar) need a fresh consent for the same identity (06 §3.1, §4.2 `login_hint`).
      const start = opts?.services
        ? await invoke<AuthStart>('account_add_start', { provider: a.provider, options: { gmail: opts.services.includes('gmail'), calendar: opts.services.includes('calendar'), login_hint: a.email } satisfies ConnectOptions })
        : await invoke<AuthStart>('account_reauthorise', { id })
      await waitForBrowser(run, start)
    } catch (e) {
      if (run === flowSeq) set((s) => (s.flow ? { flow: { ...s.flow, phase: 'failed', error: errorText(e) } } : {}))
    }
  },

  patchFlow(patch) {
    set((s) => (s.flow ? { flow: { ...s.flow, ...patch } } : {}))
  },

  async begin() {
    const f = get().flow
    if (!f || f.phase === 'starting' || f.phase === 'waiting') return
    if (f.mode === 'reauth' && f.accountId) {
      void get().startReauth(f.accountId, { services: f.services })
      return
    }
    const run = ++flowSeq
    set({ flow: { ...f, phase: 'starting', error: undefined, openFailed: false } })
    try {
      const options: ConnectOptions =
        f.provider === 'google' ? { gmail: f.services.includes('gmail'), calendar: f.services.includes('calendar'), gmail_read_only: f.readOnly } : { slack_writeback: !f.readOnly }
      // Slack: the workspace app's client id goes to the keyring first (06 §8.1 step 4).
      if (f.provider === 'slack' && f.clientId.trim()) await invoke('oauth_client_set', { provider: 'slack', client_id: f.clientId.trim() })
      const start = await invoke<AuthStart>('account_add_start', { provider: f.provider, options })
      await waitForBrowser(run, start)
    } catch (e) {
      if (run === flowSeq) set((s) => (s.flow ? { flow: { ...s.flow, phase: 'failed', error: errorText(e) } } : {}))
    }
  },

  cancelFlow() {
    flowSeq++
    const st = get().flow
    set({ flow: null })
    // Close the loopback listener at once (06 §4.3) when a flow is waiting.
    if (st?.state && (st.phase === 'waiting' || st.phase === 'starting')) void invoke('account_add_cancel', { state: st.state }).catch(() => {})
  },

  async pause(id) {
    await setStatus(id, 'paused', 'account_pause', 'Paused', "Couldn't pause the account")
  },

  async resume(id) {
    await setStatus(id, 'active', 'account_resume', 'Resumed', "Couldn't resume the account")
  },

  async retry(id) {
    await setStatus(id, 'active', 'account_retry', 'Retrying', "Couldn't retry the account")
  },

  askRemove(id) {
    const a = get().accounts.find((x) => x.id === id) ?? null
    set({ removing: a })
  },

  cancelRemove() {
    set({ removing: null })
  },

  async confirmRemove() {
    const a = get().removing
    if (!a) return
    const before = get().accounts
    const sources = get().sources
    set({ removing: null, accounts: before.filter((x) => x.id !== a.id), sources: sources.filter((s) => s.account_id !== a.id) })
    try {
      await invoke('account_remove', { id: a.id })
      notice(`Removed ${a.display_name}`)
    } catch (e) {
      set({ accounts: before, sources })
      notice(`Couldn't remove ${a.display_name}: ${errorText(e)}`)
    }
  },

  async setSource(id, patch) {
    const before = get().sources
    set({ sources: before.map((s) => (s.id === id ? { ...s, ...patch } : s)) })
    try {
      const row = await invoke<SourceRow>('source_set', { id, ...patch })
      if (row) set((s) => ({ sources: s.sources.map((x) => (x.id === id ? row : x)) }))
    } catch (e) {
      set({ sources: before })
      notice(`Couldn't change the source: ${errorText(e)}`)
    }
  },

  async openPicker(accountId) {
    const watched = get()
      .sources.filter((s) => s.account_id === accountId && s.kind === 'slack_channel' && s.enabled)
      .map((s) => s.external_id)
    set({ picker: { accountId, channels: null, picked: watched, query: '', error: null, saving: false } })
    try {
      const { channels, budgetLine } = normalizeChannels(await invoke<SlackChannel[] | CoreSlackChannels>('slack_channels_list', { account_id: accountId }))
      set((s) => (s.picker?.accountId === accountId ? { picker: { ...s.picker, channels, budgetLine, picked: channels.filter((c) => c.watched).map((c) => c.id) } } : {}))
    } catch (e) {
      set((s) => (s.picker?.accountId === accountId ? { picker: { ...s.picker, channels: [], error: errorText(e) } } : {}))
    }
  },

  patchPicker(patch) {
    set((s) => (s.picker ? { picker: { ...s.picker, ...patch } } : {}))
  },

  async savePicker() {
    const p = get().picker
    if (!p) return false
    set({ picker: { ...p, saving: true, error: null } })
    try {
      const rows = await invoke<SourceRow[]>('slack_channels_set', { account_id: p.accountId, channel_ids: p.picked })
      set((s) => ({
        picker: null,
        sources: rows ? [...s.sources.filter((x) => !(x.account_id === p.accountId && x.kind === 'slack_channel')), ...rows.filter((r) => r.kind === 'slack_channel')] : s.sources,
      }))
      notice(`Watching ${p.picked.length} ${p.picked.length === 1 ? 'channel' : 'channels'}`)
      return true
    } catch (e) {
      set((s) => (s.picker ? { picker: { ...s.picker, saving: false, error: errorText(e) } } : {}))
      return false
    }
  },

  closePicker() {
    set({ picker: null })
  },
}))

async function waitForBrowser(run: number, start: AuthStart) {
  const set = useAccounts.setState
  if (run !== flowSeq) return
  set((s) => (s.flow ? { flow: { ...s.flow, phase: 'waiting', authUrl: start.auth_url, state: start.state } } : {}))
  try {
    await invoke('plugin:opener|open_url', { url: start.auth_url })
  } catch {
    if (run === flowSeq) set((s) => (s.flow ? { flow: { ...s.flow, openFailed: true } } : {}))
  }
  const account = normalizeAccount(await invoke<AccountRow & CoreAccountExtras>('account_add_finish', { state: start.state }))
  if (run !== flowSeq) return
  set((s) => ({
    accounts: sortAccounts([...s.accounts.filter((a) => a.id !== account.id), account]),
    flow: s.flow ? { ...s.flow, phase: 'done', account } : null,
  }))
  notice(`Connected ${account.display_name}`)
  void useAccounts.getState().load()
}

async function setStatus(id: string, status: AccountRow['status'], cmd: 'account_pause' | 'account_resume' | 'account_retry', done: string, failed: string) {
  const { accounts } = useAccounts.getState()
  const a = accounts.find((x) => x.id === id)
  if (!a) return
  useAccounts.setState({ accounts: accounts.map((x) => (x.id === id ? { ...x, status, status_detail: status === 'active' ? null : x.status_detail } : x)) })
  try {
    const row = await invoke<AccountRow & CoreAccountExtras>(cmd, { id })
    useAccounts.setState((s) => ({ accounts: s.accounts.map((x) => (x.id === id ? (row ? normalizeAccount(row) : x) : x)) }))
    notice(`${done} ${a.display_name}`)
  } catch (e) {
    useAccounts.setState((s) => ({ accounts: s.accounts.map((x) => (x.id === id ? a : x)) }))
    notice(`${failed}: ${errorText(e)}`)
  }
}

/** An account row from the core: `capabilities` become `can_write` (06 §2.2). */
export function normalizeAccount(raw: AccountRow & CoreAccountExtras): AccountRow {
  if (raw.can_write || !raw.capabilities) return raw
  const c = raw.capabilities
  return { ...raw, can_write: { mark_read: c.mark_read_writeback, mark_unread: c.mark_unread_writeback, archive: c.archive_writeback } }
}

/** `slack_channels_list`: the UI's rows, or the core's `{channels, budget_line, over_budget}`. */
export function normalizeChannels(raw: SlackChannel[] | CoreSlackChannels | null | undefined): { channels: SlackChannel[]; budgetLine: string | null } {
  if (!raw) return { channels: [], budgetLine: null }
  if (Array.isArray(raw)) return { channels: raw, budgetLine: null }
  return {
    channels: raw.channels.map((c) => ({ id: c.channel_id, name: c.name.startsWith('#') ? c.name : `#${c.name}`, is_private: c.private, num_members: c.member_count ?? 0, watched: c.watched })),
    budgetLine: raw.budget_line ?? null,
  }
}

export function sortAccounts(rows: AccountRow[]): AccountRow[] {
  return rows.slice().sort((a, b) => a.position - b.position || a.display_name.localeCompare(b.display_name))
}

/* ---- derived views ------------------------------------------------------------ */

export function hasService(a: AccountRow, service: 'gmail' | 'calendar' | 'slack'): boolean {
  return a.services.includes(service)
}

export interface ConnectorBanner {
  key: string
  accountId: string | null
  /** 03 §4.9 rows. */
  kind: 'needs_reauth' | 'error' | 'paused' | 'revoked' | 'offline'
  text: string
}

/**
 * The connector status banners of 03 §4.9, one per account in trouble and one
 * for offline, filtered to the accounts that feed a view (`services`).
 */
export function connectorBanners(accounts: readonly AccountRow[], offline: boolean, services: readonly ('gmail' | 'calendar' | 'slack')[]): ConnectorBanner[] {
  const out: ConnectorBanner[] = []
  for (const a of accounts) {
    if (!a.services.some((s) => services.includes(s))) continue
    const name = a.display_name
    if (a.status === 'needs_reauth') out.push({ key: a.id, accountId: a.id, kind: 'needs_reauth', text: `${name} needs to be re-authorized` })
    else if (a.status === 'error') out.push({ key: a.id, accountId: a.id, kind: 'error', text: `${name}: ${a.status_detail ?? 'error'}` })
    else if (a.status === 'paused') out.push({ key: a.id, accountId: a.id, kind: 'paused', text: `${name} is paused` })
    else if (a.status === 'revoked') out.push({ key: a.id, accountId: a.id, kind: 'revoked', text: `${name} access was revoked` })
  }
  if (offline) out.push({ key: 'offline', accountId: null, kind: 'offline', text: services.includes('calendar') && services.length === 1 ? 'Offline — showing cached events' : 'Offline — showing cached messages' })
  return out
}

/**
 * Subscribes the store to the core: `health.changed` (paused / broken
 * accounts and offline change account rows) reloads it. Returns the
 * unsubscribe function.
 */
export async function connectAccounts(): Promise<() => void> {
  const off = await listen(EVENTS.health, () => void useAccounts.getState().load())
  void useAccounts.getState().load()
  return off
}
