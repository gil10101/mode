// The rows of the Shortcuts view (03 §7D): the keyboard tables of 03 §10 and
// 02 §8.4, one action per row, in the group order of §7D. Defaults use
// `CmdOrCtrl` for ⌘/Ctrl. `contexts` are the "Where" cells a key acts in;
// two rows conflict when they share a group, a context and a key. `fixed`
// rows (digit ranges, the Esc chain) are listed but not re-bindable. The
// five global hotkeys live in their own device-scope settings keys; every
// other override is in `settings['shortcuts.bindings']`.
import type { Os } from '../../state/settings-store'
import { canonical } from './keys'

export type ShortcutGroup =
  | 'Global'
  | 'Overlay'
  | 'App window'
  | 'Sessions'
  | 'Inbox'
  | 'Notes'
  | 'Calendar'
  | 'Chat'
  | 'Agents'
  | 'Automations'
  | 'Integrations'
  | 'Shortcuts'
  | 'Settings'

/** §7D's order; Integrations, Shortcuts and Settings carry 03 §10.7, §10.9, §10.10, which §7D's list leaves out. */
export const GROUPS: readonly ShortcutGroup[] = ['Global', 'Overlay', 'App window', 'Sessions', 'Inbox', 'Notes', 'Calendar', 'Chat', 'Agents', 'Automations', 'Integrations', 'Shortcuts', 'Settings']

export interface ShortcutDef {
  id: string
  group: ShortcutGroup
  name: string
  /** Default chord; null = unbound by default. */
  keys: string | null
  /** The scope word (64-px column): global, app, list row, editor, prompt box, … */
  scope: string
  contexts: string[]
  /** Not re-bindable: the caps as drawn (`CmdOrCtrl`, `Alt`, `Shift` become platform caps). */
  fixed?: string[]
  /** Global hotkeys: the device-scope settings key. */
  settingKey?: string
}

type Row = [id: string, name: string, keys: string | null, scope: string, contexts?: string[] | string, fixed?: string[]]

function group(g: ShortcutGroup, rows: Row[]): ShortcutDef[] {
  return rows.map(([id, name, keys, scope, contexts, fixed]) => ({
    id,
    group: g,
    name,
    keys,
    scope,
    contexts: contexts === undefined ? [scope] : Array.isArray(contexts) ? contexts : [contexts],
    ...(fixed ? { fixed } : {}),
  }))
}

const GLOBAL: ShortcutDef[] = [
  { id: 'global.capture', group: 'Global', name: 'Capture a note', keys: null, scope: 'global', contexts: ['global'], settingKey: 'capture.hotkey' },
  { id: 'global.toggle_overlay', group: 'Global', name: 'Toggle overlay', keys: null, scope: 'global', contexts: ['global'], settingKey: 'overlay.toggle_hotkey' },
  { id: 'global.pin_overlay', group: 'Global', name: 'Pin overlay', keys: null, scope: 'global', contexts: ['global'], settingKey: 'overlay.pin_hotkey' },
  { id: 'global.open_app', group: 'Global', name: 'Open app window', keys: null, scope: 'global', contexts: ['global'], settingKey: 'window.hotkey' },
  { id: 'global.dictate', group: 'Global', name: 'Hold to dictate', keys: null, scope: 'global', contexts: ['global'], settingKey: 'dictation.hotkey' },
]

/** 02 §8.4, the overlay's keyboard session. */
const OVERLAY = group('Overlay', [
  ['overlay.to_rows', 'Move to the row list', 'Tab', 'capture'],
  ['overlay.to_capture', 'Back to the capture box', 'Shift+Tab', 'row list'],
  ['overlay.prev_row', 'Previous row', 'Up', 'row list', ['row list', 'capture']],
  ['overlay.next_row', 'Next row', 'Down', 'row list', ['row list', 'capture']],
  ['overlay.prev_tab', 'Previous tab', 'Left', 'overlay', ['row list', 'capture']],
  ['overlay.next_tab', 'Next tab', 'Right', 'overlay', ['row list', 'capture']],
  ['overlay.tab_n', 'Go to tab', null, 'overlay', ['row list', 'capture'], ['1', '…', '4']],
  ['overlay.primary', 'Save or primary action', 'Enter', 'overlay', ['row list', 'capture']],
  ['overlay.allow', 'Allow', 'A', 'row list'],
  ['overlay.deny', 'Deny', 'D', 'row list'],
  ['overlay.always', 'Always allow', 'Shift+A', 'row list'],
  ['overlay.jump', 'Jump to terminal', 'J', 'row list'],
  ['overlay.mark_read', 'Mark read', 'R', 'row list'],
  ['overlay.archive', 'Archive', 'E', 'row list'],
  ['overlay.snooze', 'Snooze', 'S', 'row list'],
  ['overlay.open_source', 'Open in source', 'O', 'row list'],
  ['overlay.note', 'Note from item', 'N', 'row list'],
  ['overlay.dismiss', 'Dismiss', 'X', 'row list'],
  ['overlay.done', 'Done toggle', 'Space', 'row list'],
  ['overlay.open_app', 'Open in app', 'CmdOrCtrl+Enter', 'overlay', ['row list', 'capture']],
  ['overlay.pin', 'Toggle pin', 'P', 'row list'],
  ['overlay.esc', 'Clear or collapse', null, 'overlay', ['row list', 'capture'], ['Esc']],
])

/** 03 §10.1. */
const APP = group('App window', [
  ['app.go_sessions', 'Go to Sessions', 'CmdOrCtrl+1', 'app'],
  ['app.go_inbox', 'Go to Inbox', 'CmdOrCtrl+2', 'app'],
  ['app.go_notes', 'Go to Notes', 'CmdOrCtrl+3', 'app'],
  ['app.go_calendar', 'Go to Calendar', 'CmdOrCtrl+4', 'app'],
  ['app.go_chat', 'Go to Chat', 'CmdOrCtrl+5', 'app'],
  ['app.go_agents', 'Go to Agents', 'CmdOrCtrl+6', 'app'],
  ['app.go_automations', 'Go to Automations', 'CmdOrCtrl+7', 'app'],
  ['app.go_integrations', 'Go to Integrations', 'CmdOrCtrl+8', 'app'],
  ['app.go_shortcuts', 'Go to Shortcuts', 'CmdOrCtrl+0', 'app'],
  ['app.go_stats', 'Go to Stats', 'CmdOrCtrl+9', 'app'],
  ['app.settings', 'Open Settings', 'CmdOrCtrl+,', 'app'],
  ['app.query', 'Query bar', 'CmdOrCtrl+K', 'app'],
  ['app.new', 'New in this view', 'CmdOrCtrl+N', 'app'],
  ['app.new_note', 'New note from anywhere', 'CmdOrCtrl+Shift+N', 'app'],
  ['app.find', 'Search in this view', 'CmdOrCtrl+F', 'app'],
  ['app.clear_filters', 'Clear all filters', 'CmdOrCtrl+Shift+F', 'app'],
  ['app.back', 'Back', 'CmdOrCtrl+[', 'app'],
  ['app.forward', 'Forward', 'CmdOrCtrl+]', 'app'],
  ['app.sidebar', 'Collapse sidebar', 'CmdOrCtrl+\\', 'app'],
  ['app.detail', 'Toggle the detail panel', 'CmdOrCtrl+I', 'app'],
  ['app.undo', 'Undo', 'CmdOrCtrl+Z', 'app'],
  ['app.redo', 'Redo', 'CmdOrCtrl+Shift+Z', 'app'],
  ['app.close', 'Close card or window', 'CmdOrCtrl+W', 'app'],
  ['app.quit', 'Quit', 'CmdOrCtrl+Q', 'app'],
  ['app.up_level', 'Up one level', null, 'app', 'app', ['Esc']],
  ['app.move_up', 'Move up', 'Up', 'list', 'list'],
  ['app.move_down', 'Move down', 'Down', 'list', 'list'],
  ['app.open', 'Open', 'Enter', 'list', 'list'],
  ['app.preview', 'Preview', 'Space', 'list', 'list'],
  ['app.select', 'Toggle select', 'X', 'list', 'list'],
  ['app.select_all', 'Select all', 'CmdOrCtrl+A', 'list', 'list'],
  ['app.search', "Focus the view's search", '/', 'app'],
  ['app.shortcuts', 'Shortcuts for this view', '?', 'app'],
])

/** 03 §10.2. */
const SESSIONS = group('Sessions', [
  ['sessions.open', 'Open detail', 'Enter', 'list row', 'row'],
  ['sessions.acknowledge', 'Acknowledge', 'E', 'list row', 'row'],
  ['sessions.dismiss', 'Dismiss', 'X', 'list row', 'row'],
  ['sessions.jump', 'Jump to terminal', 'J', 'list row', ['row', 'detail']],
  ['sessions.resume', 'Resume', 'R', 'list row', ['row', 'detail']],
  ['sessions.note', 'Note from session', 'N', 'list row', ['row', 'detail']],
  ['sessions.copy_path', 'Copy repo path', 'C', 'list row', ['row', 'detail']],
  ['sessions.allow', 'Allow', 'A', 'card'],
  ['sessions.deny', 'Deny', 'D', 'card'],
  ['sessions.always', 'Always allow', 'Shift+A', 'card'],
  ['sessions.option_n', 'Pick an option', null, 'card', 'question', ['1', '…', '9']],
  ['sessions.history', 'Toggle history', 'H', 'list', 'roster'],
  ['sessions.expand', 'Expand agents', 'Right', 'list row', 'row'],
  ['sessions.collapse', 'Collapse agents', 'Left', 'list row', 'row'],
  ['sessions.next_tab', 'Next tab', 'Ctrl+Tab', 'detail'],
  ['sessions.prev_tab', 'Previous tab', 'Ctrl+Shift+Tab', 'detail'],
  ['sessions.tab_n', 'Go to tab', null, 'detail', 'detail', ['CmdOrCtrl', 'Shift', '1', '…', '6']],
  ['sessions.send', 'Send', 'Enter', 'prompt box'],
  ['sessions.newline', 'New line', 'Shift+Enter', 'prompt box'],
  ['sessions.stop', 'Stop turn', 'CmdOrCtrl+.', 'detail'],
  ['sessions.leave_terminal', 'Leave the terminal', 'CmdOrCtrl+Shift+Esc', 'terminal'],
  ['sessions.file_open', 'Open the diff', 'Enter', 'files row'],
  ['sessions.file_editor', 'Open in editor', 'CmdOrCtrl+Enter', 'files row'],
  ['sessions.file_reveal', 'Reveal', 'Shift+Enter', 'files row'],
  ['sessions.file_copy', 'Copy path', 'C', 'files row'],
  ['sessions.agent_open', 'Open the agent thread', 'Enter', 'agents row'],
  ['sessions.connector_open', 'Open in Integrations', 'Enter', 'tab row', 'connectors row'],
  ['sessions.activity', 'Expand or collapse activity', 'Space', 'detail', 'overview'],
])

/** 03 §10.3. */
const INBOX = group('Inbox', [
  ['inbox.open', 'Open', 'Enter', 'list row', 'row'],
  ['inbox.open_unread', 'Open without marking read', 'Shift+Enter', 'list row', 'row'],
  ['inbox.mark_read', 'Mark read', 'R', 'list row', ['row', 'thread']],
  ['inbox.mark_unread', 'Mark unread', 'U', 'list row', ['row', 'thread']],
  ['inbox.archive', 'Archive', 'E', 'list row', ['row', 'thread']],
  ['inbox.snooze', 'Snooze', 'S', 'list row', ['row', 'thread']],
  ['inbox.open_source', 'Open in source', 'O', 'list row', ['row', 'thread']],
  ['inbox.note', 'Note from message', 'N', 'list row', ['row', 'thread']],
  ['inbox.link', 'Link to note or session', 'L', 'list row', ['row', 'thread']],
  ['inbox.vip', 'Toggle important person', 'V', 'list row', ['row', 'thread']],
  ['inbox.tier', 'Change tier', 'T', 'list row', ['row', 'thread']],
  ['inbox.snooze_preset', 'Snooze preset', null, 'popover', 'popover', ['1', '…', '5']],
  ['inbox.mark_all_read', 'Mark all read', 'CmdOrCtrl+Shift+A', 'view'],
  ['inbox.back', 'Back to the list', null, 'search', 'search', ['Esc']],
])

/** 03 §10.4. */
const NOTES = group('Notes', [
  ['notes.open', 'Open in the editor', 'Enter', 'list row', 'row'],
  ['notes.cycle', 'Cycle status', 'Space', 'list row', 'row'],
  ['notes.urgent', 'Toggle urgent', '!', 'list row', 'row'],
  ['notes.when', 'When picker', 'T', 'list row', 'row'],
  ['notes.tags', 'Tag editor', '#', 'list row', 'row'],
  ['notes.pin', 'Pin', 'P', 'list row', 'row'],
  ['notes.archive', 'Archive', 'E', 'list row', 'row'],
  ['notes.history', 'History', 'H', 'list row', 'row'],
  ['notes.raw', 'Raw source or live preview', 'CmdOrCtrl+E', 'editor'],
  ['notes.toggle_task', 'Toggle task', 'CmdOrCtrl+Enter', 'editor'],
  ['notes.save', 'Save now', 'CmdOrCtrl+S', 'editor'],
  ['notes.move', 'Move to folder', 'CmdOrCtrl+Shift+M', 'editor'],
  ['notes.board', 'Groupings board', 'CmdOrCtrl+Shift+B', 'view'],
  ['notes.new_folder', 'New folder', 'CmdOrCtrl+Shift+F', 'tree'],
  ['notes.rename_folder', 'Rename folder', 'F2', 'tree'],
  ['notes.spawn_card', 'Spawn card or commit chip', 'Enter', 'query bar'],
  ['notes.accept', 'Accept suggestion', 'Tab', 'query bar'],
  ['notes.remove_chip', 'Remove last chip', 'Backspace', 'query bar'],
  ['notes.blur_query', 'Clear suggestions, then blur', null, 'query bar', 'query bar', ['Esc']],
  ['notes.spawn_new', 'Spawn a new card', 'CmdOrCtrl+Enter', 'query bar'],
  ['notes.card_expand', 'Expand card', 'CmdOrCtrl+E', 'card'],
  ['notes.card_save', 'Save as grouping', 'CmdOrCtrl+S', 'card'],
  ['notes.card_close', 'Close card', 'CmdOrCtrl+W', 'card'],
  ['notes.card_open', 'Open', 'Enter', 'card row'],
  ['notes.card_cycle', 'Cycle status', 'Space', 'card row'],
  ['notes.card_urgent', 'Toggle urgent', '!', 'card row'],
])

/** 03 §10.5. */
const CALENDAR = group('Calendar', [
  ['calendar.today', 'Today', 'T', 'view'],
  ['calendar.prev', 'Previous period', 'Left', 'view'],
  ['calendar.next', 'Next period', 'Right', 'view'],
  ['calendar.day', 'Day', 'D', 'view'],
  ['calendar.week', 'Week', 'W', 'view'],
  ['calendar.month', 'Month', 'M', 'view'],
  ['calendar.select', 'Select', 'Enter', 'view'],
  ['calendar.prepare', 'Prepare notes', 'P', 'view'],
  ['calendar.join', 'Join', 'J', 'view'],
  ['calendar.open', 'Open in Google Calendar', 'O', 'view'],
  ['calendar.copy', 'Copy join link', 'C', 'view'],
  ['calendar.hours', 'Expand the hours', '+', 'view'],
])

/** 03 §10.6. */
const CHAT = group('Chat', [
  ['chat.send', 'Send', 'Enter', 'prompt box'],
  ['chat.newline', 'New line', 'Shift+Enter', 'prompt box'],
  ['chat.stop', 'Stop', 'CmdOrCtrl+.', 'streaming'],
  ['chat.slash', 'Slash commands', '/', 'prompt box'],
  ['chat.plus', 'The + menu', 'CmdOrCtrl+Shift+/', 'prompt box'],
  ['chat.confirm', 'Confirm action', 'Enter', 'card'],
  ['chat.reject', 'Reject action', null, 'card', 'card', ['Esc']],
  ['chat.citation', 'Open citation', null, 'answer', 'answer', ['Alt', '1', '…', '9']],
  ['chat.activity', 'Expand or collapse activity', 'Space', 'thread'],
  ['chat.sources', 'Sources panel', 'CmdOrCtrl+Shift+S', 'view'],
])

/** 03 §10.8. */
const AUTOMATIONS = group('Automations', [
  ['automations.open', 'Open the editor', 'Enter', 'list row', 'row'],
  ['automations.toggle', 'Turn on or off', 'Space', 'list row', 'row'],
  ['automations.run', 'Run now', 'CmdOrCtrl+Enter', 'list row', ['row', 'editor']],
  ['automations.duplicate', 'Duplicate', 'CmdOrCtrl+D', 'list row', 'row'],
  ['automations.history', 'History', 'H', 'list row', 'row'],
  ['automations.delete', 'Delete', 'Delete', 'list row', 'row'],
  ['automations.save', 'Save', 'CmdOrCtrl+S', 'editor'],
  ['automations.leave', 'Leave the editor', null, 'editor', 'editor', ['Esc']],
  ['automations.run_open', 'Open the run', 'Enter', 'run row'],
  ['automations.run_back', 'Back to the runs', null, 'run page', 'run page', ['Esc']],
])

/** 03 §10.9A. */
const AGENTS = group('Agents', [
  ['agents.open', 'Open the editor', 'Enter', 'list row', 'row'],
  ['agents.launch', 'Launch a session as this agent', 'CmdOrCtrl+Enter', 'list row', 'row'],
  ['agents.delete', 'Delete', 'Delete', 'list row', 'row'],
  ['agents.save', 'Save', 'CmdOrCtrl+S', 'editor'],
  ['agents.leave', 'Leave the editor', null, 'editor', 'editor', ['Esc']],
])

/** 03 §10.9. */
const INTEGRATIONS = group('Integrations', [
  ['integrations.open', 'Open the card', 'Enter', 'card'],
  ['integrations.primary', "The card's primary action", 'C', 'card'],
  ['integrations.diagnostics', 'Diagnostics for this card', 'D', 'card'],
])

/** 03 §10.10. */
const SHORTCUTS = group('Shortcuts', [
  ['shortcuts.search', 'Focus the search field', '/', 'view'],
  ['shortcuts.record', 'Record a new key', 'Enter', 'list row', 'row'],
  ['shortcuts.reset', 'Reset this shortcut', 'R', 'list row', 'row'],
])

/** 03 §10.7. */
const SETTINGS = group('Settings', [['settings.filter', 'Filter pages and controls', 'CmdOrCtrl+F', 'view']])

export const CATALOG: readonly ShortcutDef[] = [...GLOBAL, ...OVERLAY, ...APP, ...SESSIONS, ...INBOX, ...NOTES, ...CALENDAR, ...CHAT, ...AGENTS, ...AUTOMATIONS, ...INTEGRATIONS, ...SHORTCUTS, ...SETTINGS]

export const BY_ID: ReadonlyMap<string, ShortcutDef> = new Map(CATALOG.map((d) => [d.id, d]))

/** The global hotkey rows by settings key. */
export const GLOBAL_BY_KEY: ReadonlyMap<string, ShortcutDef> = new Map(GLOBAL.map((d) => [d.settingKey!, d]))

/* ---- effective bindings and conflicts ----------------------------------------- */

export interface Binding {
  def: ShortcutDef
  /** The chord in force (null = unbound). */
  keys: string | null
  /** Differs from the default. */
  custom: boolean
}

/**
 * The binding of every row: global rows from their settings keys (compared
 * with the registry default), the rest from `shortcuts.bindings` overrides.
 */
export function effectiveBindings(settings: { get(key: string): unknown; defaultOf(key: string): unknown }, os: Os): Binding[] {
  const overrides = (settings.get('shortcuts.bindings') ?? {}) as Record<string, string | null>
  return CATALOG.map((def) => {
    if (def.settingKey) {
      const v = settings.get(def.settingKey) as string | null
      const d = settings.defaultOf(def.settingKey) as string | null
      return { def, keys: v ?? null, custom: canonical(v, os) !== canonical(d, os) }
    }
    if (def.fixed) return { def, keys: null, custom: false }
    if (Object.prototype.hasOwnProperty.call(overrides, def.id)) {
      const v = overrides[def.id] ?? null
      return { def, keys: v, custom: canonical(v, os) !== canonical(def.keys, os) }
    }
    return { def, keys: def.keys, custom: false }
  })
}

/** Rows that share a group, a context and a chord with another row: id → the other rows. */
export function findConflicts(bindings: readonly Binding[], os: Os): Map<string, ShortcutDef[]> {
  const out = new Map<string, ShortcutDef[]>()
  const seen = new Map<string, Binding[]>()
  for (const b of bindings) {
    const c = canonical(b.keys, os)
    if (!c) continue
    for (const ctx of b.def.contexts) {
      const slot = `${b.def.group}\u0000${ctx}\u0000${c}`
      const list = seen.get(slot) ?? []
      for (const other of list) {
        if (other.def.id === b.def.id) continue
        add(out, b.def.id, other.def)
        add(out, other.def.id, b.def)
      }
      list.push(b)
      seen.set(slot, list)
    }
  }
  return out
}

function add(map: Map<string, ShortcutDef[]>, id: string, def: ShortcutDef) {
  const list = map.get(id) ?? []
  if (!list.some((d) => d.id === def.id)) list.push(def)
  map.set(id, list)
}

/** Which rows a new chord for `id` would collide with (shown before the write). */
export function conflictsFor(id: string, keys: string | null, bindings: readonly Binding[], os: Os): ShortcutDef[] {
  const next = bindings.map((b) => (b.def.id === id ? { ...b, keys } : b))
  return findConflicts(next, os).get(id) ?? []
}

/** "conflicts with Archive · Inbox" (03 §7D). */
export function conflictText(others: readonly ShortcutDef[]): string {
  return `conflicts with ${others.map((o) => `${o.name} · ${o.group}`).join(', ')}`
}

/** Caps of a fixed row for `os`. */
export function fixedCaps(tokens: readonly string[], os: Os): string[] {
  const mac = os === 'macos'
  const map: Record<string, string> = { CmdOrCtrl: mac ? '⌘' : 'Ctrl', Alt: mac ? '⌥' : 'Alt', Shift: mac ? '⇧' : 'Shift' }
  return tokens.map((t) => map[t] ?? t)
}
