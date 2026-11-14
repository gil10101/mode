// The inline SVG icon set from docs/wireframes (ADR-0034). `<IconDefs/>` is
// rendered once per document; `<Icon id="sessions"/>` references a symbol.

export type IconId =
  | 'sessions'
  | 'inbox'
  | 'notes'
  | 'calendar'
  | 'chat'
  | 'stats'
  | 'file'
  | 'slash'
  | 'link'
  | 'copy'
  | 'check'
  | 'pin'
  | 'edit'
  | 'agents'
  | 'automations'
  | 'integrations'
  | 'shortcuts'
  | 'settings'
  | 'refresh'
  | 'overlay'
  | 'mic'
  | 'chevron'
  | 'mail'
  | 'palette'
  | 'bell'
  | 'flag'
  | 'sparkle'
  | 'shield'
  | 'wrench'
  | 'folder'
  | 'laptop'
  | 'skip-back'
  | 'play'
  | 'pause'
  | 'skip-forward'

export function IconDefs() {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" style={{ display: 'none' }} aria-hidden="true">
      <symbol id="i-sessions" viewBox="0 0 24 24"><rect x="3" y="5" width="18" height="15" rx="5" /><path d="M9 11v3M15 11v3" /></symbol>
      <symbol id="i-inbox" viewBox="0 0 24 24"><path d="M3 13h5l2 3h4l2-3h5" /><path d="M5 5h14l2 8v6H3v-6z" /></symbol>
      <symbol id="i-notes" viewBox="0 0 24 24"><path d="M5 3h10l4 4v14H5z" /><path d="M9 12h6M9 16h4" /></symbol>
      <symbol id="i-calendar" viewBox="0 0 24 24"><rect x="3" y="5" width="18" height="16" rx="2" /><path d="M3 10h18M8 3v4M16 3v4" /></symbol>
      <symbol id="i-chat" viewBox="0 0 24 24"><path d="M4 5h16v11H9l-5 4z" /></symbol>
      <symbol id="i-stats" viewBox="0 0 24 24"><path d="M5 20v-9M12 20V5M19 20v-7" /></symbol>
      <symbol id="i-file" viewBox="0 0 24 24"><path d="M6 3h9l4 4v14H6z" /><path d="M15 3v4h4" /></symbol>
      <symbol id="i-slash" viewBox="0 0 24 24"><path d="m15 4-6 16" /></symbol>
      <symbol id="i-link" viewBox="0 0 24 24"><path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1.5 1.5" /><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1.5-1.5" /></symbol>
      <symbol id="i-copy" viewBox="0 0 24 24"><rect x="9" y="9" width="11" height="11" rx="2" /><path d="M5 15V6a2 2 0 0 1 2-2h9" /></symbol>
      <symbol id="i-check" viewBox="0 0 24 24"><path d="m5 12.5 4.5 4.5L19 7.5" /></symbol>
      <symbol id="i-pin" viewBox="0 0 24 24"><path d="M9 3h6l-1 6 3 3v2H7v-2l3-3z" /><path d="M12 14v7" /></symbol>
      <symbol id="i-edit" viewBox="0 0 24 24"><path d="m4 20 4-1L19 8l-3-3L5 16z" /><path d="m14 7 3 3" /></symbol>
      <symbol id="i-agents" viewBox="0 0 24 24"><circle cx="12" cy="8" r="4" /><path d="M5 20a7 7 0 0 1 14 0" /><path d="M9 8h.01M15 8h.01" /></symbol>
      <symbol id="i-automations" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8" /><path d="M12 8v4l3 2" /></symbol>
      <symbol id="i-integrations" viewBox="0 0 24 24"><path d="M9 3v4M15 3v4M6 7h12v4a6 6 0 0 1-12 0z" /><path d="M12 17v4" /></symbol>
      <symbol id="i-shortcuts" viewBox="0 0 24 24"><rect x="3" y="7" width="18" height="11" rx="2" /><path d="M7 11h.01M11 11h.01M15 11h.01M7 14h10" /></symbol>
      <symbol id="i-settings" viewBox="0 0 24 24"><path d="M4 7h3M11 7h9M4 12h9M17 12h3M4 17h5M13 17h7" /><circle cx="9" cy="7" r="2" /><circle cx="15" cy="12" r="2" /><circle cx="11" cy="17" r="2" /></symbol>
      <symbol id="i-refresh" viewBox="0 0 24 24"><path d="M20 12a8 8 0 1 1-2.4-5.7" /><path d="M20 4v5h-5" /></symbol>
      <symbol id="i-overlay" viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M8 4v1.5a1.5 1.5 0 0 0 1.5 1.5h5A1.5 1.5 0 0 0 16 5.5V4" /></symbol>
      <symbol id="i-mic" viewBox="0 0 24 24"><rect x="9" y="3" width="6" height="11" rx="3" /><path d="M5 11a7 7 0 0 0 14 0M12 18v3" /></symbol>
      {/* kit glyphs for the Settings page list, the select chevron, the IMAP tile and the launch dialog's Choose…; i-mail, i-wrench and i-folder follow Lucide's drawings (ISC) */}
      <symbol id="i-chevron" viewBox="0 0 24 24"><path d="m6 9 6 6 6-6" /></symbol>
      <symbol id="i-mail" viewBox="0 0 24 24"><rect x="2" y="4" width="20" height="16" rx="2" /><path d="m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7" /></symbol>
      <symbol id="i-palette" viewBox="0 0 24 24"><path d="M12 3a9 9 0 1 0 0 18 2 2 0 0 0 1.5-3.3 2 2 0 0 1 1.5-3.4h2.4A4.6 4.6 0 0 0 21 9.8C21 6 17 3 12 3z" /><path d="M7.5 11h.01M10 7h.01M15 7h.01" /></symbol>
      <symbol id="i-bell" viewBox="0 0 24 24"><path d="M6 16v-5a6 6 0 0 1 12 0v5l2 2H4z" /><path d="M10 21h4" /></symbol>
      <symbol id="i-flag" viewBox="0 0 24 24"><path d="M5 21V4" /><path d="M5 4h12l-2.5 4 2.5 4H5" /></symbol>
      <symbol id="i-sparkle" viewBox="0 0 24 24"><path d="m12 3 1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z" /><path d="M19 16v4M17 18h4" /></symbol>
      <symbol id="i-shield" viewBox="0 0 24 24"><path d="M12 3 19 6v5c0 4.5-3 8.5-7 10-4-1.5-7-5.5-7-10V6z" /></symbol>
      <symbol id="i-wrench" viewBox="0 0 24 24"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.8-3.8a6 6 0 0 1-7.9 7.9l-6.9 6.9a2.1 2.1 0 0 1-3-3l6.9-6.9a6 6 0 0 1 7.9-7.9z" /></symbol>
      <symbol id="i-folder" viewBox="0 0 24 24"><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z" /></symbol>
      {/* the device-scope mark of a settings row (10 §7.4.19), Lucide's laptop (ISC) */}
      <symbol id="i-laptop" viewBox="0 0 24 24"><path d="M20 16V7a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v9m16 0H4m16 0 1.28 2.55a1 1 0 0 1-.9 1.45H3.62a1 1 0 0 1-.9-1.45L4 16" /></symbol>
      {/* the Now playing strip's controls (02 §7.1), after Lucide's skip-back, play, pause and skip-forward (ISC) */}
      <symbol id="i-skip-back" viewBox="0 0 24 24"><path d="M19 20 9 12l10-8z" /><path d="M5 19V5" /></symbol>
      <symbol id="i-play" viewBox="0 0 24 24"><path d="m6 3 14 9-14 9z" /></symbol>
      <symbol id="i-pause" viewBox="0 0 24 24"><rect x="6" y="4" width="4" height="16" rx="1" /><rect x="14" y="4" width="4" height="16" rx="1" /></symbol>
      <symbol id="i-skip-forward" viewBox="0 0 24 24"><path d="m5 4 10 8-10 8z" /><path d="M19 5v14" /></symbol>
    </svg>
  )
}

export function Icon({ id, className }: { id: IconId; className?: string }) {
  return (
    <svg className={className ? `m-icon ${className}` : 'm-icon'} aria-hidden="true">
      <use href={`#i-${id}`} />
    </svg>
  )
}
