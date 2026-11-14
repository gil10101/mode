// The Gmail, Slack and Google Calendar cards (03 §7C.1–§7C.2, §7C.5) as pure
// functions of the accounts and sources the core reports: the status line
// (06 §9A: worst state wins, the newest sync time shown; "offline" replaces
// it), the count lines (the wireframe's: names · scope, messages this week ·
// unread; watched channels; calendars, read-only), and the one primary
// action. Paused accounts keep the card connected (06 §2.4 lists no paused
// card state); the Inbox banner and the accounts table say so.
import { invoke } from '../../tauri'
import type { AccountRow, SourceRow } from '../../state/contract-connectors'
import type { ConnectorCardKind } from '../../state/accounts-store'
import { ago, type CardLine, type CardStatus } from './model'

export type LiveActionKind = 'connect' | 'reconnect' | 'retry' | 'add' | 'channels' | 'calendars' | 'reauth-calendar'

export interface LiveCard {
  status: CardStatus
  lines: CardLine[]
  action: { kind: LiveActionKind; label: string; primary: boolean; disabled?: boolean; hint?: string; accountId?: string }
  accounts: AccountRow[]
}

const SERVICE: Record<ConnectorCardKind, 'gmail' | 'slack' | 'calendar'> = { gmail: 'gmail', slack: 'slack', 'google-calendar': 'calendar' }
const OFFLINE_HINT = 'needs a network connection'

function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`
}

/** "read-only scope" (wireframe) / "read, modify" (03 §7C.2) / "read, modify on Work". */
export function gmailScope(accounts: readonly AccountRow[]): string {
  const modify = accounts.filter((a) => a.scopes.some((s) => s.endsWith('/gmail.modify')))
  if (!modify.length) return 'read-only scope'
  if (modify.length === accounts.length) return 'read, modify'
  return `read, modify on ${modify.map((a) => a.display_name).join(', ')}`
}

function newest(values: (string | null | undefined)[]): string | null {
  return values.filter((v): v is string => !!v).sort().pop() ?? null
}

export function liveConnectorCard(kind: ConnectorCardKind, accounts: readonly AccountRow[], sources: readonly SourceRow[], opts: { offline: boolean; now: number }): LiveCard {
  const service = SERVICE[kind]
  const mine = accounts.filter((a) => a.services.includes(service))
  const ids = new Set(mine.map((a) => a.id))
  const own = sources.filter((s) => ids.has(s.account_id))
  const off = opts.offline ? { disabled: true, hint: OFFLINE_HINT } : {}

  if (!mine.length) {
    const google = accounts.filter((a) => a.provider === 'google')
    if (kind === 'google-calendar' && google.length) {
      return {
        status: { state: 'idle', label: 'not connected', detail: `Calendar not granted for ${google[0]!.display_name}` },
        lines: [],
        action: { kind: 'reauth-calendar', label: 'Re-authorize with Calendar', primary: false, accountId: google[0]!.id, ...off },
        accounts: [],
      }
    }
    return { status: { state: 'idle', label: 'not connected' }, lines: [], action: { kind: 'connect', label: 'Connect', primary: false, ...off }, accounts: [] }
  }

  const unit = kind === 'slack' ? plural(mine.length, 'workspace', 'workspaces') : plural(mine.length, 'account', 'accounts')
  const channels = own.filter((s) => s.kind === 'slack_channel' && s.enabled).map((s) => s.name)
  const watchedLine = channels.length ? `watched: ${channels.join(', ')}` : 'DMs and mentions watched'
  const broken = mine.find((a) => a.status === 'needs_reauth' || a.status === 'revoked')
  if (broken) {
    const line1 = `${broken.display_name} · ${broken.status_detail ?? (broken.status === 'revoked' ? 'access was revoked' : 'needs sign-in')}`
    const line2 = kind === 'slack' ? watchedLine : kind === 'gmail' ? `${mine.map((a) => a.display_name).join(', ')} · ${gmailScope(mine)}` : 'Nothing is written back'
    return {
      status: { state: 'failed', label: 'needs sign-in', detail: `${unit}` },
      lines: [line1, line2],
      action: { kind: 'reconnect', label: 'Reconnect', primary: true, accountId: broken.id, ...off },
      accounts: mine,
    }
  }
  const failed = mine.find((a) => a.status === 'error')
  if (failed) {
    return {
      status: { state: 'failed', label: 'error', detail: `${failed.status_detail ?? unit}` },
      lines: [failed.display_name],
      action: { kind: 'retry', label: 'Retry', primary: false, accountId: failed.id, ...off },
      accounts: mine,
    }
  }

  const syncSources = own.filter((s) => s.enabled && (kind === 'gmail' ? s.kind === 'gmail_inbox' || s.kind === 'gmail_label' : kind === 'google-calendar' ? s.kind === 'calendar' : s.kind.startsWith('slack')))
  const latest = newest(syncSources.length ? syncSources.map((s) => s.last_polled_at) : mine.map((a) => a.last_sync_at))
  const synced = opts.offline ? 'offline' : latest ? `synced ${ago(latest, opts.now)}` : 'first sync running'
  const names = mine.map((a) => a.display_name).join(', ')

  if (kind === 'gmail') {
    // The counts are optional on the core's rows; without them the card keeps the scope line only.
    const counted = mine.some((a) => a.messages_week !== undefined)
    const week = mine.reduce((n, a) => n + (a.messages_week ?? 0), 0)
    const unread = mine.reduce((n, a) => n + (a.unread ?? 0), 0)
    return {
      status: { state: 'connected', label: 'connected', detail: `${unit} · ${synced}` },
      lines: counted ? [`${names} · ${gmailScope(mine)}`, `${week} messages this week · ${unread} unread`] : [`${names} · ${gmailScope(mine)}`],
      action: { kind: 'add', label: 'Add account', primary: false, ...off },
      accounts: mine,
    }
  }
  if (kind === 'slack') {
    return {
      status: { state: 'connected', label: 'connected', detail: `${unit} · ${synced}` },
      lines: [`${names} · DMs and mentions watched`, watchedLine],
      action: { kind: 'channels', label: 'Choose channels', primary: false, accountId: mine[0]!.id, ...off },
      accounts: mine,
    }
  }
  const calendars = own.filter((s) => s.kind === 'calendar' && s.enabled)
  const calNames = Array.from(new Set(calendars.map((s) => mine.find((a) => a.id === s.account_id)?.display_name).filter(Boolean)))
  return {
    status: { state: 'connected', label: 'connected', detail: `${plural(calendars.length, 'calendar', 'calendars')} · ${synced}` },
    lines: [`${(calNames.length ? calNames : mine.map((a) => a.display_name)).join(', ')} · read-only`, 'Nothing is written back'],
    action: { kind: 'calendars', label: 'Choose calendars', primary: false },
    accounts: mine,
  }
}

/** The accounts table's status column (03 §7C.3). */
export function accountState(a: AccountRow): { state: 'connected' | 'pending' | 'failed'; label: string } {
  switch (a.status) {
    case 'active':
      return { state: 'connected', label: 'connected' }
    case 'paused':
      return { state: 'pending', label: 'paused' }
    case 'needs_reauth':
      return { state: 'failed', label: 'needs sign-in' }
    case 'revoked':
      return { state: 'failed', label: 'revoked' }
    default:
      return { state: 'failed', label: 'error' }
  }
}

/** Requests per minute the watched Slack conversations cost at an interval (06 §8.4 budget line). */
export function slackRequestsPerMinute(conversations: number, intervalSeconds: number): number {
  return Math.ceil((conversations * 60) / Math.max(1, intervalSeconds))
}

/** The manifest of 06 §8.1 with the port list; without the `*:write` scopes when write-back is off. */
export function slackManifest(writeBack: boolean): string {
  const scopes = ['channels:history', 'groups:history', 'im:history', 'mpim:history', 'channels:read', 'groups:read', 'im:read', 'mpim:read', 'users:read', 'search:read', ...(writeBack ? ['channels:write', 'groups:write', 'im:write', 'mpim:write'] : [])]
  return [
    'display_information:',
    '  name: Mode',
    '  description: Mode reads your DMs, mentions and chosen channels on this device. It never posts.',
    'oauth_config:',
    '  redirect_urls:',
    ...[48311, 48312, 48313, 48314, 48315].map((p) => `    - http://127.0.0.1:${p}/callback`),
    '  scopes:',
    '    user:',
    ...scopes.map((s) => `      - ${s}`),
    'settings:',
    '  org_deploy_enabled: false',
    '  socket_mode_enabled: false',
    '  token_rotation_enabled: true',
    '',
  ].join('\n')
}

/** The manifest from the core (`slack_manifest`, the OAuth ports as configured); the local copy when the core cannot answer. */
export async function loadManifest(writeback: boolean): Promise<string> {
  try {
    const text = await invoke<string>('slack_manifest', { writeback })
    if (typeof text === 'string' && text.trim()) return text
  } catch {
    /* an older core: the built-in copy below */
  }
  return slackManifest(writeback)
}

/** Save a text as a file through the webview's download (the manifest; no dialog command exists for it). */
export function saveText(name: string, text: string, type = 'text/yaml'): boolean {
  try {
    const url = URL.createObjectURL(new Blob([text], { type }))
    const a = document.createElement('a')
    a.href = url
    a.download = name
    document.body.appendChild(a)
    a.click()
    a.remove()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
    return true
  } catch {
    return false
  }
}
