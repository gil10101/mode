// The eleven Settings pages of 03 §8.1 in the page list's order (three
// hairline-separated groups, 10 §7.4.19) and the index the filter field
// searches (03 §10.7 "Filter pages and controls"): one entry per control,
// with the page, the section anchor and the settings key it writes.
import type { IconId } from '../../kit/icons'

export type PageId = 'general' | 'appearance' | 'overlay' | 'dictation' | 'chat' | 'alerts' | 'notes' | 'urgency' | 'ai' | 'privacy' | 'advanced'

export interface PageSpec {
  id: PageId
  label: string
  /** The page list's kit icon (10 §7.4.19). */
  icon: IconId
  /** The page list group (0, 1, 2; a hairline between groups). */
  group: 0 | 1 | 2
  /** 03 §8.1 Release column (the parts of a later release say so on the page). */
  release: string
}

export const PAGES: readonly PageSpec[] = [
  { id: 'general', label: 'General', icon: 'settings', group: 0, release: 'v0.1' },
  { id: 'appearance', label: 'Appearance', icon: 'palette', group: 0, release: 'v0.1 / v1.0' },
  { id: 'overlay', label: 'Overlay', icon: 'overlay', group: 0, release: 'v0.1' },
  { id: 'dictation', label: 'Dictation', icon: 'mic', group: 0, release: 'v1.0' },
  { id: 'chat', label: 'Chat', icon: 'chat', group: 1, release: 'v1.0' },
  { id: 'alerts', label: 'Alerts', icon: 'bell', group: 1, release: 'v0.2' },
  { id: 'notes', label: 'Notes', icon: 'notes', group: 1, release: 'v0.1 / v0.2' },
  { id: 'urgency', label: 'Urgency', icon: 'flag', group: 1, release: 'v0.2' },
  { id: 'ai', label: 'AI', icon: 'sparkle', group: 1, release: 'v0.2 / v1.0' },
  { id: 'privacy', label: 'Privacy', icon: 'shield', group: 2, release: 'v0.2 / v1.0' },
  { id: 'advanced', label: 'Advanced', icon: 'wrench', group: 2, release: 'v0.1 / v1.0' },
]

/** `PAGES` as the page list's groups. */
export const NAV_GROUPS = [0, 1, 2].map((g) => ({ items: PAGES.filter((p) => p.group === g).map((p) => ({ id: p.id, label: p.label, icon: p.icon })) }))

export const PAGE_IDS: readonly string[] = PAGES.map((p) => p.id)

/** Sections of Advanced reachable as `settings/advanced/<section>`. */
export const ADVANCED_SECTIONS = ['diagnostics', 'logs', 'export', 'about'] as const

export interface IndexEntry {
  page: PageId
  label: string
  /** The `settings-<section>` anchor to scroll to. */
  section?: string
  /** The registry key (the control carries `data-setting`). */
  key?: string
  /** Extra words the filter matches. */
  words?: string
}

const e = (page: PageId, label: string, key?: string, section?: string, words?: string): IndexEntry => ({ page, label, key, section, words })

export const SEARCH_INDEX: readonly IndexEntry[] = [
  e('general', 'Profile picture', 'profile.picture_path', 'profile', 'mascot avatar'),
  e('general', 'Profile name', 'profile.name', 'profile', 'user name'),
  e('general', 'Launch at login', 'general.launch_at_login', 'startup', 'startup'),
  e('general', 'Status item', 'general.status_item', 'startup', 'menu bar tray badge'),
  e('general', 'Updates', 'general.auto_update', 'startup', 'check for updates automatically'),
  e('general', 'Compact rows', 'general.compact_rows', 'rows', 'density'),
  e('general', 'Language', 'general.language', 'rows'),
  e('general', 'Run onboarding again', undefined, 'rows', 'welcome setup'),
  e('general', 'Fold recently completed after', 'sessions.fold_after_minutes', 'sessions', 'sessions'),
  e('general', 'Store prompt previews', 'sessions.store_prompt_previews', 'sessions', 'sessions'),
  e('general', 'Weekly token budget', 'usage.budget_tokens_week', 'sessions', 'usage soft budget tokens'),
  e('general', 'Budget week starts', 'usage.reset_weekday', 'sessions', 'reset weekday'),
  e('appearance', 'Appearance', 'theme.appearance', 'appearance', 'light dark system theme mode'),
  e('appearance', 'Theme', 'theme.active', 'themes', 'pack gallery'),
  e('overlay', 'Placement', 'overlay.placement', 'placement', 'notch top-bar right-edge'),
  e('overlay', 'When nothing is happening', 'overlay.idle', 'placement', 'idle mascot peek hide hidden notch'),
  e('overlay', 'Host display', 'overlay.display', 'placement', 'monitor screen'),
  e('overlay', 'Reserve screen space', 'overlay.right_edge_dock', 'placement', 'appbar windows dock'),
  e('overlay', 'Peek delay', 'overlay.peek_delay_ms', 'behaviour'),
  e('overlay', 'Expand on hover', 'overlay.expand_on_hover', 'behaviour'),
  e('overlay', 'Collapse after hover-out', 'overlay.collapse_delay_ms', 'behaviour'),
  e('overlay', 'Auto-collapse', 'overlay.auto_collapse_ms', 'behaviour'),
  e('overlay', 'Pinned by default', 'overlay.pinned', 'behaviour', 'pin'),
  e('overlay', 'Default tab', 'overlay.default_tab', 'behaviour'),
  e('overlay', 'Full-screen behavior', 'overlay.fullscreen', 'behaviour', 'presentation fullscreen'),
  e('overlay', 'Rows per overlay tab', 'overlay.max_rows', 'behaviour'),
  e('overlay', 'Sounds', 'sounds.enabled', 'presence', 'sound audio'),
  e('overlay', 'Mascot', 'overlay.show_mascot', 'presence', 'duck'),
  e('overlay', 'Motion', 'overlay.motion', 'presence', 'reduced motion animation'),
  e('overlay', 'Now playing', 'overlay.media', 'presence', 'spotify music player song track media controls'),
  e('overlay', 'Music history', undefined, 'presence', 'clear recently played albums tracks spotify music forget'),
  e('dictation', 'Turn on dictation', 'dictation.enabled', 'dictation', 'speech voice microphone talk whisper download'),
  e('dictation', 'Hold to dictate', 'dictation.hotkey', 'dictation', 'hotkey shortcut key push to talk'),
  e('dictation', 'Dictation model', 'dictation.model', 'dictation', 'speech whisper tiny base small download installed'),
  e('dictation', 'Dictation language', 'dictation.language', 'dictation', 'speech multilingual'),
  e('dictation', '“Hey Mode” commands', 'dictation.commands', 'requests', 'hey mode wake phrase voice request chat'),
  e('dictation', 'Answer in the overlay', 'dictation.reply_card', 'requests', 'hey mode reply card notch answer voice'),
  e('dictation', 'When nothing in Mode has focus', 'dictation.unfocused', 'requests', 'dictation capture note unfocused'),
  e('dictation', 'Try dictation', undefined, 'try', 'test microphone'),
  e('chat', 'Chat model', 'ai.model_chat', 'conversations', 'model opus sonnet haiku'),
  e('chat', 'Chat timeout', 'ai.timeout_seconds', 'conversations', 'seconds wait reply'),
  e('chat', 'Keep conversations', 'retention.chat_days', 'conversations', 'retention days delete history'),
  e('chat', 'Sources panel', 'chat.sources_open', 'conversations', 'sources open panel'),
  e('alerts', 'Quiet hours', 'notifications.quiet_hours', 'quiet', 'do not disturb'),
  e('alerts', 'Sounds', 'sounds.enabled', 'os', 'audio'),
  e('alerts', 'OS notification preview', 'notifications.os_preview', 'os', 'notification'),
  e('alerts', 'OS notification sound', 'notifications.os_sound', 'os', 'notification'),
  e('alerts', 'Focus and Do Not Disturb', undefined, 'quiet', 'dnd focus'),
  e('alerts', 'Pause alerts', 'notifications.pause_until', 'quiet', 'pause mute snooze'),
  e('alerts', 'Alert rules', undefined, 'rules', 'test premade custom new rule delete event recent alerts'),
  e('notes', 'Notes folder', 'notes.vault_path', 'vault', 'vault path choose'),
  e('notes', 'Touch-up frontmatter writes', 'notes.touchup_enabled', 'vault', 'frontmatter'),
  e('notes', 'Rename file with title', 'notes.rename_with_title', 'vault', 'file name'),
  e('notes', 'Week starts on', 'notes.week_start', 'vault', 'monday sunday'),
  e('notes', 'Reindex vault', undefined, 'vault', 'index rebuild'),
  e('notes', 'Capture folder', undefined, 'vault', 'quick capture inbox folder'),
  e('notes', 'Templates folder', undefined, 'vault', 'template folder'),
  e('notes', 'Groupings', undefined, 'groupings', 'views board filter'),
  e('notes', 'Custom statuses', 'notes.statuses', 'statuses', 'status todo doing done'),
  e('notes', 'Tags', undefined, 'tags', 'tag rename merge'),
  e('urgency', 'Keyword rules', 'urgency.keyword_rules', 'keywords', 'regex pattern'),
  e('urgency', 'Scoring model', 'ai.model_scoring', 'scoring', 'model'),
  e('urgency', 'Characters sent for scoring', 'ai.scoring_chars', 'scoring'),
  e('urgency', 'Monthly AI token budget', 'ai.budget_tokens_month', 'scoring', 'budget tokens'),
  e('urgency', 'Important people', undefined, 'vips', 'important people contact'),
  e('urgency', 'Re-score', undefined, 'scoring', 'rescore again last 24 hours'),
  e('ai', 'Backend', 'ai.backend', 'backend', 'claude api ollama lm studio'),
  e('ai', 'Anthropic API key', undefined, 'backend', 'key secret keyring'),
  e('ai', 'Local backend address', 'ai.local_url', 'local', 'ollama lm studio url'),
  e('ai', 'Models', 'ai.model_scoring', 'models', 'model scoring summary'),
  e('ai', 'Local-only', 'ai.local_only', 'backend', 'privacy remote'),
  e('ai', 'AI token budget', 'ai.budget_tokens_month', 'budget', 'tokens'),
  e('ai', 'Test call', undefined, 'backend', 'latency'),
  e('privacy', 'What leaves this device', undefined, 'leaves', 'privacy data'),
  e('privacy', 'Retention', 'retention.sessions_days', 'retention', 'days keep prune'),
  e('privacy', 'Prompt previews', 'sessions.store_prompt_previews', 'retention', 'prompts'),
  e('privacy', 'Delete all Mode data', undefined, 'delete', 'erase remove reset'),
  e('advanced', 'Diagnostics', undefined, 'diagnostics', 'hooks ipc database index'),
  e('advanced', 'Logs', undefined, 'logs', 'log tail'),
  e('advanced', 'Export & Import', undefined, 'export', 'backup restore file'),
  e('advanced', 'About', undefined, 'about', 'version license'),
]

/** Entries whose label, page, key or words contain every word of `q`. */
export function searchSettings(q: string): IndexEntry[] {
  const words = q.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return []
  return SEARCH_INDEX.filter((x) => {
    const page = PAGES.find((p) => p.id === x.page)!.label
    const hay = `${x.label} ${page} ${x.key ?? ''} ${x.words ?? ''}`.toLowerCase()
    return words.every((w) => hay.includes(w))
  })
}

/** `rest` of a `settings/…` route → page and section (`advanced/logs`, `general/name`). */
export function parseSettingsRest(rest: string | undefined): { page: PageId; section?: string } {
  const [page, section] = (rest ?? '').split('/')
  if (page && PAGE_IDS.includes(page)) return { page: page as PageId, section: section || undefined }
  return { page: 'general' }
}
