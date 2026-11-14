// Window-level state of the app shell (03 §2.1–§2.3): the sidebar collapse
// (a remembered user choice, else automatic below 1280 px, 10 §13), the
// detail-panel toggle and the sidebar counts that come from other pushes.
import { create } from 'zustand'

/** `window.sidebar_collapsed` stands in here until the device settings command exists (package 9). */
export const SIDEBAR_KEY = 'mode.window.sidebar_collapsed'
export const DETAIL_KEY = 'mode.window.detail_open'

/** 10 §13: the sidebar auto-collapses to 56 px below 1280 px. */
export const AUTO_COLLAPSE_BELOW = 1280

function readBool(key: string): boolean | null {
  try {
    const v = window.localStorage.getItem(key)
    return v === null ? null : v === 'true'
  } catch {
    return null
  }
}

function writeBool(key: string, v: boolean | null) {
  try {
    if (v === null) window.localStorage.removeItem(key)
    else window.localStorage.setItem(key, String(v))
  } catch {
    /* storage unavailable: the choice lasts for this window only */
  }
}

export interface ShellState {
  /** The user's explicit choice; `null` = follow the window width. */
  collapsedPref: boolean | null
  width: number
  detailOpen: boolean
  inboxUrgent: number
  notesOverdue: number
  setWidth(w: number): void
  toggleSidebar(): void
  setDetailOpen(open: boolean): void
  setCounts(c: Partial<Pick<ShellState, 'inboxUrgent' | 'notesOverdue'>>): void
}

export const useShell = create<ShellState>((set, get) => ({
  collapsedPref: typeof window === 'undefined' ? null : readBool(SIDEBAR_KEY),
  width: typeof window === 'undefined' ? 1280 : window.innerWidth,
  detailOpen: typeof window === 'undefined' ? true : (readBool(DETAIL_KEY) ?? true),
  inboxUrgent: 0,
  notesOverdue: 0,
  setWidth: (width) => set({ width }),
  toggleSidebar() {
    const next = !sidebarCollapsed(get())
    writeBool(SIDEBAR_KEY, next)
    set({ collapsedPref: next })
  },
  setDetailOpen(open) {
    writeBool(DETAIL_KEY, open)
    set({ detailOpen: open })
  },
  setCounts: (c) => set(c),
}))

export function sidebarCollapsed(s: Pick<ShellState, 'collapsedPref' | 'width'>): boolean {
  return s.collapsedPref ?? s.width < AUTO_COLLAPSE_BELOW
}
