// Shared vocabulary of the kit. Session states follow the design system (seven
// states, lowercase words); connector states §7.6; urgency levels 07 §2.1;
// overlay states and placements 02 §2 and §4.

export type SessionState = 'waiting' | 'working' | 'failed' | 'unread' | 'finished' | 'idle' | 'untracked'

/** The vertical grouping order of session lists. */
export const SESSION_STATES: readonly SessionState[] = [
  'waiting',
  'working',
  'failed',
  'unread',
  'finished',
  'idle',
  'untracked',
]

/** The word the state column shows; `unread` is a finished session not yet looked at. */
export const SESSION_STATE_LABEL: Readonly<Record<SessionState, string>> = {
  waiting: 'waiting',
  working: 'working',
  failed: 'failed',
  unread: 'finished',
  finished: 'finished',
  idle: 'idle',
  untracked: 'untracked',
}

/** Connector and permission-rule statuses of the Connectors tab (§7.6). `pending` has no glyph rule and shows the base ○. */
export type ConnectorState = 'connected' | 'pending' | 'failed' | 'allowed' | 'denied'

/** Anything `.m-state.is-*` can show. */
export type StateKind = SessionState | ConnectorState

export function stateLabel(state: StateKind): string {
  return (SESSION_STATE_LABEL as Record<string, string>)[state] ?? state
}

/** The three UI levels a message row can carry (07 §2.1): only `urgent` draws anything (the 2-px bar). */
export type UrgencyLevel = 'urgent' | 'normal' | 'low'

/** The engine tiers behind the levels (07 §2.1). Never shown on a row. */
export type UrgencyTier = 'vip' | 'direct' | 'ai-urgent' | 'ai-normal' | 'low' | 'unscored'

export function urgencyLevel(tier: UrgencyTier): UrgencyLevel {
  switch (tier) {
    case 'vip':
    case 'direct':
    case 'ai-urgent':
      return 'urgent'
    case 'low':
      return 'low'
    default:
      return 'normal'
  }
}

/** Overlay placements (02 §4). */
export type Placement = 'notch' | 'top-bar' | 'right-edge'

export const PLACEMENT_CLASS: Readonly<Record<Placement, string>> = {
  notch: 'is-notch',
  'top-bar': 'is-topbar',
  'right-edge': 'is-rightedge',
}

/** The overlay state ladder (02 §2). */
export type OverlayState = 'hidden' | 'peek' | 'compact' | 'expanded'

export type Theme = 'light' | 'dark'

export type MessageSource = 'gmail' | 'slack'

/** Task glyphs of query cards and the Notes views: ○ todo · ◑ doing · ✱ urgent · ✓ done. */
export type TaskGlyph = '○' | '◑' | '✱' | '✓'

/** Day tracker intensity (§7.3): sunken, then --m-ok at 25 / 50 / 75 / 100 %. */
export type TrackerLevel = 0 | 1 | 2 | 3 | 4
