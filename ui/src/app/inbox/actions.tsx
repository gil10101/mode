// Thread actions shared by the row keys, the detail panel's ⋮, the thread
// view's action bar and the bulk bar (03 §4.6): mark read / unread, archive,
// snooze, open in source, note from message, mark sender as an important person, change the
// level (03 §4.5: a menu of the three levels; `urgency_source = 'user'`).
import { invoke } from '../../tauri'
import { cmdErrorText } from '../../state/contract'
import type { ThreadDetail, ThreadRow, Tier } from '../../state/contract-connectors'
import { isSnoozed, levelOf, useInbox, type Level } from '../../state/inbox-store'
import { useSessionsStore } from '../../state/sessions-store'
import { navigate } from '../router'
import { openMenu, type MenuEntry } from '../ui'
import { openSnooze } from './SnoozeMenu'
import { openLinkPicker } from './LinkDialog'

export const LEVEL_TIER: Record<Level, Tier> = { urgent: 'ai-urgent', normal: 'ai-normal', low: 'low' }

function notice(text: string, action?: { label: string; run: () => void }) {
  useSessionsStore.getState().showNotice(text, action)
}

/** The thread view (the drill-in rule's full page); `markRead: false` is Shift+Enter (03 §10.3). */
export function openThread(id: string, markRead = true) {
  window.dispatchEvent(new CustomEvent('mode:inbox-no-read', { detail: !markRead }))
  navigate({ view: 'inbox', rest: `thread/${id}` })
}

export async function openInSource(id: string) {
  try {
    // A string, or the core's `{ url, fallback_url }` (Slack's web link when no app handles `slack://`).
    const r = await invoke<string | { url: string; fallback_url: string | null }>('open_in_source', { id })
    const url = typeof r === 'string' ? r : r?.url
    if (url) {
      try {
        await invoke('plugin:opener|open_url', { url })
      } catch (e) {
        if (typeof r === 'object' && r?.fallback_url) await invoke('plugin:opener|open_url', { url: r.fallback_url })
        else throw e
      }
    }
  } catch (e) {
    notice(`Couldn't open the message in its app: ${cmdErrorText(e)}`)
  }
}

/** Escapes the capture grammar's tokens (04 §7.2) so a `#channel` stays text. */
function captureSafe(line: string): string {
  return line.replace(/(^|\s)([#@/!])/g, '$1\\$2')
}

/** Note from message (03 §4.6): a note in the capture folder, linked `created_from`. */
export async function noteFromMessage(row: ThreadRow, detail?: ThreadDetail | null) {
  const last = detail?.messages[detail.messages.length - 1]
  const title = row.kind === 'email' ? row.title : `Message from ${row.sender_name}${row.channel_name ? ` in ${row.channel_name}` : ''}`
  const body = [`From: ${row.sender_name}${row.sender_address ? ` <${row.sender_address}>` : ''}`, row.kind === 'email' ? `Subject: ${row.title}` : null, '', last?.snippet ?? row.last_snippet ?? ''].filter((x) => x !== null)
  try {
    const r = await invoke<{ note_id: string }>('note_capture', { text: [captureSafe(title), ...body].join('\n'), created_from: { kind: 'message', id: last?.id ?? row.id } })
    notice('Note created', { label: 'Open', run: () => navigate({ view: 'notes', rest: r.note_id }) })
  } catch (e) {
    notice(`Couldn't create the note: ${cmdErrorText(e)}`)
  }
}

export function levelEntries(row: ThreadRow): MenuEntry[] {
  const current = levelOf(row.urgency_tier ?? (useInbox.getState().tierMemory[row.id] ?? null))
  return (['urgent', 'normal', 'low'] as Level[]).map((lv) => ({
    id: `level-${lv}`,
    label: `Level: ${lv}`,
    checked: current === lv,
    run: () => void useInbox.getState().setTier(row.id, LEVEL_TIER[lv]),
  }))
}

/** The thread's ⋮ (03 §4.5 action bar, wireframe callout 3). */
export function threadMenu(row: ThreadRow, detail: ThreadDetail | null | undefined, anchor: Element, opts: { page?: boolean } = {}): MenuEntry[] {
  const inbox = useInbox.getState()
  const snoozed = isSnoozed(row, Date.now())
  const entries: MenuEntry[] = []
  if (!opts.page) entries.push({ id: 'source', label: 'Open in source', run: () => void openInSource(row.id) })
  entries.push(
    row.unread_count > 0
      ? { id: 'read', label: 'Mark read', run: () => void inbox.markRead(row.id, true) }
      : { id: 'unread', label: 'Mark unread', run: () => void inbox.markRead(row.id, false) },
    row.is_archived ? { id: 'unarchive', label: 'Move to inbox', run: () => void inbox.archive(row.id, false) } : { id: 'archive', label: 'Archive', run: () => void inbox.archive(row.id) },
    snoozed ? { id: 'unsnooze', label: 'Cancel snooze', run: () => void inbox.unsnooze(row.id) } : { id: 'snooze', label: 'Snooze…', run: () => openSnooze(anchor, [row.id], detail?.messages[detail.messages.length - 1]?.id) },
    { id: 'note', label: 'Note from message', run: () => void noteFromMessage(row, detail) },
    { id: 'link', label: 'Link…', run: () => openLinkPicker(row.id) },
    row.sender_vip
      ? { id: 'vip', label: 'Remove sender from important people', run: () => void inbox.setVip(row.id, false) }
      : { id: 'vip', label: 'Mark sender as important person', run: () => void inbox.setVip(row.id, true) },
    { kind: 'sep', id: 'sep-level' },
    ...levelEntries(row),
  )
  if (!opts.page) entries.push({ kind: 'sep', id: 'sep-select' }, { id: 'select', label: 'Select', run: () => inbox.togglePick(row.id) })
  return entries
}

export function showThreadMenu(anchor: Element, row: ThreadRow, detail: ThreadDetail | null | undefined, opts: { page?: boolean } = {}) {
  openMenu(anchor, threadMenu(row, detail, anchor, opts), `${row.sender_name} options`)
}
