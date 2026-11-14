// The overlay's data store: the pushed query results of 02 §16, the display
// the core assigned, the settings, and the overlay's own bookkeeping
// (decisions in flight, the 10-s `done` badge hold, the capture confirmation).
import { create } from 'zustand'
import {
  DEFAULT_SETTINGS,
  TIMING,
  type AgendaChanged,
  type DecideResult,
  type DisplayChanged,
  type HealthChanged,
  type InboxChanged,
  type MediaChanged,
  type MediaHistory,
  type NowPlaying,
  type NotesChanged,
  type OverlaySettings,
  type OverlayTab,
  type PillChanged,
  type QuietChanged,
  type RosterRow,
} from './contract'
import type { CoreChanged } from './store'

export type DecisionKind = 'allow' | 'deny' | 'always' | 'answer'

export interface Decided {
  kind: DecisionKind
  outcome: DecideResult['outcome'] | 'pending'
  /** The `pending.received_at` the decision answered; a newer request clears it. */
  received_at: string | null
  at: number
}

export interface OverlayData {
  rows: RosterRow[]
  rowsAt: number
  /** The last `pill.changed`; `null` until the core pushes one (the local ladder stands in). */
  pill: PillChanged | null
  inbox: InboxChanged | null
  notes: NotesChanged | null
  agenda: AgendaChanged | null
  quiet: QuietChanged | null
  health: HealthChanged | null
  display: DisplayChanged
  settings: OverlaySettings
  core: CoreChanged | null
  decided: Record<string, Decided>
  doneHeldUntil: number
  captureSaved: { note_id: string; until: number } | null
  foldOpen: boolean
  /** Now playing (02 §7.1): the local player's track, null when none is known. */
  media: NowPlaying | null
  /** The Music tab (02 §7.10): what Mode saw playing, null until the first answer. */
  history: MediaHistory | null

  applySessions(rows: RosterRow[], now?: number): void
  applyPill(p: PillChanged): void
  applyInbox(p: InboxChanged): void
  applyNotes(p: NotesChanged): void
  applyAgenda(p: AgendaChanged): void
  applyQuiet(p: QuietChanged): void
  applyHealth(p: HealthChanged): void
  applyDisplay(p: Partial<DisplayChanged>): void
  applySettings(p: Partial<OverlaySettings>): void
  applyCore(p: CoreChanged): void
  applyMedia(p: MediaChanged): void
  applyHistory(p: MediaHistory): void
  decide(session_id: string, kind: DecisionKind, received_at: string | null, now?: number): void
  settle(session_id: string, outcome: DecideResult['outcome']): void
  setCaptureSaved(note_id: string | null, now?: number): void
  setFoldOpen(open: boolean): void
  reset(): void
}

export const DEFAULT_DISPLAY: DisplayChanged = {
  placement: 'top-bar',
  scale: 1,
  dark: false,
  reduced_motion: false,
}

const EMPTY = {
  rows: [] as RosterRow[],
  rowsAt: 0,
  pill: null,
  inbox: null,
  notes: null,
  agenda: null,
  quiet: null,
  health: null,
  display: DEFAULT_DISPLAY,
  settings: DEFAULT_SETTINGS,
  core: null,
  decided: {},
  doneHeldUntil: 0,
  captureSaved: null,
  foldOpen: false,
  media: null,
  history: null as MediaHistory | null,
}

export const useOverlayStore = create<OverlayData>((set) => ({
  ...EMPTY,
  applySessions: (rows, now = Date.now()) =>
    set((s) => {
      const before = new Map(s.rows.map((r) => [r.id, r]))
      const finishedNow = rows.some((r) => r.state === 'unread' && before.get(r.id)?.state !== 'unread' && before.has(r.id))
      const decided: Record<string, Decided> = {}
      for (const [id, d] of Object.entries(s.decided)) {
        const row = rows.find((r) => r.id === id)
        if (row && row.state === 'waiting' && (row.pending?.received_at ?? null) === d.received_at) decided[id] = d
      }
      return { rows, rowsAt: now, decided, doneHeldUntil: finishedNow ? now + TIMING.done_hold_ms : s.doneHeldUntil }
    }),
  applyPill: (pill) => set({ pill }),
  applyInbox: (inbox) => set({ inbox }),
  applyNotes: (notes) => set({ notes }),
  applyAgenda: (agenda) => set({ agenda }),
  applyQuiet: (quiet) => set({ quiet }),
  applyHealth: (health) => set({ health }),
  applyDisplay: (p) => set((s) => ({ display: { ...s.display, ...p } })),
  applySettings: (p) => set((s) => ({ settings: { ...s.settings, ...p } })),
  applyCore: (core) => set({ core }),
  applyMedia: (p) => set({ media: mediaOf(p) }),
  applyHistory: (p) => set({ history: { tracks: Array.isArray(p?.tracks) ? p.tracks : [], albums: Array.isArray(p?.albums) ? p.albums : [] } }),
  decide: (id, kind, received_at, now = Date.now()) => set((s) => ({ decided: { ...s.decided, [id]: { kind, outcome: 'pending', received_at, at: now } } })),
  settle: (id, outcome) =>
    set((s) => {
      const d = s.decided[id]
      return d ? { decided: { ...s.decided, [id]: { ...d, outcome } } } : {}
    }),
  setCaptureSaved: (note_id, now = Date.now()) => set({ captureSaved: note_id ? { note_id, until: now + TIMING.capture_saved_ms } : null }),
  setFoldOpen: (foldOpen) => set({ foldOpen }),
  reset: () => set({ ...EMPTY }),
}))

/** A `media.changed` payload as a track (`{}` and a permission refusal are none). */
export function mediaOf(p: MediaChanged | null | undefined): NowPlaying | null {
  if (!p || typeof p !== 'object' || typeof p.app !== 'string' || typeof p.title !== 'string') return null
  return {
    app: p.app,
    playing: !!p.playing,
    title: p.title,
    artist: p.artist ?? '',
    album: p.album ?? '',
    ...(p.track_id ? { track_id: p.track_id } : {}),
    artwork_url: p.artwork_url ?? null,
    position_ms: Number(p.position_ms) || 0,
    duration_ms: Number(p.duration_ms) || 0,
    at_ms: Number(p.at_ms) || 0,
  }
}

/** Tab badges of 02 §7.3. */
export function tabCounts(s: Pick<OverlayData, 'rows' | 'inbox' | 'notes' | 'agenda'>): Record<OverlayTab, number> {
  return {
    // The Music tab carries no count (02 §7.3).
    music: 0,
    sessions: s.rows.filter((r) => r.state === 'waiting').length,
    inbox: s.inbox?.urgent_count ?? 0,
    notes: s.notes?.overdue_count ?? 0,
    calendar: s.agenda?.next_2h_count ?? 0,
  }
}
