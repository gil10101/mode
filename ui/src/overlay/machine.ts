// The overlay state machine of 02 §2–§3 as a pure reducer. Timers are data
// (absolute due times in `timers`); the runner fires `timer` actions when they
// come due, so every transition is testable as a table. Effects name what the
// host must do (tell the core, focus the capture box, act on a row).
import { DEFAULT_SETTINGS, TIMING, visibleTabs, type DictationCitation, type DictationState, type DictationTarget, type OverlayFlash, type OverlaySettings, type OverlayStateName, type OverlayTab, type PillCandidate, type PillChanged } from '../state/contract'
import { reduceKey, type KeyAction } from './machine-keys'

export type TimerKind = 'peek_dwell' | 'arrival_settle' | 'expand_hover' | 'collapse_delay' | 'auto_collapse' | 'pill_min' | 'flash' | 'new_strip' | 'keyboard_idle' | 'dictation' | 'reply_hold'

export type OpenedBy = 'hover' | 'click' | 'hotkey' | 'arrival' | 'pin' | 'drag' | 'reply'

export type Zone = 'capture' | 'rows'

export interface Flash {
  text: string
  /** Failure flash: the pill stays (02 §5.5). From the app window: the flash stays until Q1 decides (02 §5.4). */
  keepPill: boolean
  /** Set for a flash from the app window (`overlay.flash`, 02 §5.4); absent for the resolved flash of an inline action. */
  app?: { kind: OverlayFlash['kind']; open?: OverlayFlash['open'] }
}

/** The longest an app-window flash with `keep_pill` holds when no pill and no later flash takes over (a turn that never reports back). */
export const FLASH_KEEP_MAX_MS = 120_000

export interface Strip {
  ref: string | null
  kind: string
  tab: OverlayTab
  label: string
}

/** How long the done / error dictation pill stays (02 §5.6), like a resolved flash. */
export const DICTATION_HOLD_MS = 1600

/** The dictation pill's state (02 §5.6): local and transient, above every Q1 candidate. */
export interface Dictation {
  state: 'listening' | 'transcribing' | 'thinking' | 'answering' | 'done' | 'error'
  since: number
  /** Input level 0..1 (listening). */
  level: number
  seconds: number
  text?: string
  target?: DictationTarget
  words?: number
  error?: string
  threadId?: string
  noteId?: string
}

/** A voice request answered on the reply card (02 §5.7): the turn, the reply so far, and whether it ended. */
export interface Reply {
  threadId: string
  turnId: string
  text: string
  citations: DictationCitation[]
  done: boolean
  /** A failed turn: its reason shows in place of the reply. */
  error?: string
}

export interface MachineSettings extends OverlaySettings {
  reduced_motion: boolean
  /** Quiet hours are on (`quiet.changed`): the overlay rests `hidden` (02 §2.1). */
  quiet_hours?: boolean
}

export interface MachineState {
  state: OverlayStateName
  pinned: boolean
  /** Keyboard session (02 §9): the capture box holds key input. */
  keyboard: boolean
  zone: Zone
  hovering: boolean
  pill: PillCandidate | null
  count: number
  pillShownAt: number
  /** An equal-priority newer pill held by `pill_min`, or any push deferred by a flash. */
  pendingPill: PillChanged | null
  flash: Flash | null
  /** `peek` entered by an arrival (settles into `compact` after 200 ms). */
  arrival: boolean
  openedBy: OpenedBy | null
  tab: OverlayTab
  lastTab: OverlayTab | null
  focusRow: string | null
  strip: Strip | null
  suspended: boolean
  resumeTo: OverlayStateName | null
  /** Dictation in progress or just finished (02 §5.6); `compact` stays drawn while it is set. */
  dictation?: Dictation | null
  /** The voice reply card (02 §5.7); while set the panel shows it in place of the tabs, rows and capture box. */
  reply?: Reply | null
  /** The last turn whose card was dismissed: its later states are ignored. */
  dismissedTurn?: string | null
  timers: Partial<Record<TimerKind, number>>
  settings: MachineSettings
}

export type PointerKind = 'hot_enter' | 'hot_leave' | 'frame_enter' | 'frame_leave' | 'outside_click' | 'drag_dwell'

export type Action =
  | { type: 'pill'; candidate: PillCandidate | null; count: number; now: number }
  | { type: 'pointer'; kind: PointerKind; now: number }
  | { type: 'click'; target: 'hot' | 'peek' | 'pill'; row?: string | null; now: number }
  | { type: 'hotkey'; hotkey: 'toggle' | 'capture' | 'pin'; captureEmpty?: boolean; now: number }
  | { type: 'pin'; now: number }
  | { type: 'tab'; tab: OverlayTab; now: number }
  | { type: 'focus_row'; row: string | null; now: number }
  | { type: 'capture_focus'; now: number }
  | { type: 'capture_blur'; now: number }
  | KeyAction
  | { type: 'resolved'; text: string; keepPill?: boolean; ms?: number; now: number }
  /** The user is leaving (open in app, jump to terminal): collapse unless pinned. */
  | { type: 'leave'; now: number }
  | { type: 'timer'; kind: TimerKind; now: number }
  | { type: 'suspend'; now: number }
  | { type: 'resume'; now: number }
  | { type: 'settings'; settings: Partial<MachineSettings>; now: number }
  /** A `dictation.state` push. */
  | { type: 'dictation_state'; payload: DictationState; now: number }
  /** Drops the dictation pill at once. */
  | { type: 'dictation_clear'; now: number }
  /** An `overlay.flash` push: an action taken in the app window (02 §5.4). */
  | { type: 'flash'; flash: OverlayFlash; now: number }
  /** Drops an app-window flash at once (its Open was clicked). */
  | { type: 'flash_clear'; now: number }
  /** The reply card's Dismiss (02 §5.7). */
  | { type: 'reply_dismiss'; now: number }

export type RowAction = 'primary' | 'open_in_app' | 'allow' | 'deny' | 'always' | 'jump' | 'dismiss' | 'note' | 'done' | 'read' | 'archive' | 'snooze' | 'open_source'

export type Effect =
  | { type: 'request'; state: OverlayStateName; pinned: boolean; keyboard: boolean; reason: string }
  | { type: 'focus_capture' }
  | { type: 'blur' }
  | { type: 'clear_capture' }
  | { type: 'save_capture'; open: boolean }
  | { type: 'act'; action: RowAction; row: string }
  | { type: 'pulse' }

export interface Result {
  next: MachineState
  effects: Effect[]
  /** The action was consumed (a key action: the host prevents the default). */
  handled: boolean
}

export function tabOfCandidate(c: PillCandidate | null): OverlayTab {
  if (!c) return 'sessions'
  if (c.kind === 'urgent_message') return 'inbox'
  if (c.kind === 'next_event') return 'calendar'
  return 'sessions'
}

/** What the overlay rests as with nothing to show (02 §2.1): `peek` with the gaze mascot when `overlay.idle = mascot`, else `hidden`; quiet hours and presentation mode always rest `hidden`. */
export function restState(s: { settings: MachineSettings; suspended?: boolean }): 'peek' | 'hidden' {
  return s.settings.idle === 'hidden' || s.settings.quiet_hours || s.suspended ? 'hidden' : 'peek'
}

export function initialState(opts: { settings?: Partial<MachineSettings>; pinned?: boolean; pill?: PillChanged | null; now?: number; lastTab?: OverlayTab | null } = {}): MachineState {
  const settings = { ...DEFAULT_SETTINGS, reduced_motion: false, ...opts.settings }
  const pill = opts.pill?.candidate ?? null
  const pinned = !!opts.pinned
  return {
    state: pinned ? 'expanded' : pill ? 'compact' : restState({ settings }),
    pinned,
    keyboard: false,
    zone: 'capture',
    hovering: false,
    pill,
    count: opts.pill?.count ?? 0,
    pillShownAt: opts.now ?? 0,
    pendingPill: null,
    flash: null,
    arrival: false,
    openedBy: pinned ? 'pin' : null,
    tab: pinned ? (opts.lastTab ?? 'sessions') : tabOfCandidate(pill),
    lastTab: opts.lastTab ?? null,
    focusRow: null,
    strip: null,
    suspended: false,
    resumeTo: null,
    timers: {},
    settings,
  }
}

/* ---- helpers --------------------------------------------------------------- */

type Draft = { s: MachineState; effects: Effect[] }

function schedule(d: Draft, kind: TimerKind, at: number) {
  d.s = { ...d.s, timers: { ...d.s.timers, [kind]: at } }
}

export function cancel(d: Draft, ...kinds: TimerKind[]) {
  const timers = { ...d.s.timers }
  for (const k of kinds) delete timers[k]
  d.s = { ...d.s, timers }
}

function set(d: Draft, patch: Partial<MachineState>) {
  d.s = { ...d.s, ...patch }
}

function request(d: Draft, reason: string) {
  d.effects.push({ type: 'request', state: d.s.state, pinned: d.s.pinned, keyboard: d.s.keyboard, reason })
}

function collapseTarget(s: MachineState): OverlayStateName {
  return s.pill || s.dictation ? 'compact' : restState(s)
}

export function defaultTab(s: MachineState): OverlayTab {
  const shown = (t: OverlayTab | null) => (t && visibleTabs(s.settings).includes(t) ? t : null)
  if (s.settings.default_tab !== 'auto') return shown(s.settings.default_tab) ?? 'sessions'
  return s.pill ? tabOfCandidate(s.pill) : (shown(s.lastTab) ?? 'sessions')
}

function releaseKeyboard(d: Draft) {
  if (!d.s.keyboard) return
  set(d, { keyboard: false, zone: 'capture', focusRow: null })
  cancel(d, 'keyboard_idle')
  d.effects.push({ type: 'blur' })
}

export function startKeyboard(d: Draft, now: number, focus: boolean) {
  const idle = d.s.settings.auto_collapse_ms * TIMING.keyboard_idle_factor
  set(d, { keyboard: true, zone: 'capture' })
  cancel(d, 'collapse_delay', 'auto_collapse')
  if (idle > 0) schedule(d, 'keyboard_idle', now + idle)
  if (focus) d.effects.push({ type: 'focus_capture' })
}

interface OpenOptions {
  tab?: OverlayTab
  row?: string | null
  keyboard?: boolean
}

export function expand(d: Draft, by: OpenedBy, now: number, o: OpenOptions = {}) {
  const tab = o.tab ?? defaultTab(d.s)
  set(d, { state: 'expanded', openedBy: by, arrival: false, tab, lastTab: tab, focusRow: o.row ?? null })
  cancel(d, 'peek_dwell', 'arrival_settle', 'expand_hover', 'collapse_delay', 'auto_collapse')
  const auto = d.s.settings.auto_collapse_ms
  if ((by === 'hotkey' || by === 'arrival' || by === 'drag') && auto > 0 && !d.s.pinned && !d.s.hovering) schedule(d, 'auto_collapse', now + auto)
  if (o.keyboard) startKeyboard(d, now, true)
  request(d, `expand:${by}`)
}

export function collapse(d: Draft, reason: string) {
  const target = collapseTarget(d.s)
  releaseKeyboard(d)
  set(d, { state: target, openedBy: null, focusRow: null, strip: null, arrival: false, reply: null })
  cancel(d, 'collapse_delay', 'auto_collapse', 'new_strip', 'expand_hover', 'peek_dwell', 'arrival_settle', 'reply_hold')
  request(d, `collapse:${reason}`)
}

function hide(d: Draft, reason: string) {
  releaseKeyboard(d)
  set(d, { state: 'hidden', openedBy: null, focusRow: null, strip: null, arrival: false, hovering: false })
  const kept: MachineState['timers'] = {}
  if (d.s.flash) kept.flash = d.s.timers.flash
  if (d.s.dictation && d.s.timers.dictation !== undefined) kept.dictation = d.s.timers.dictation
  d.s = { ...d.s, timers: kept }
  request(d, `hide:${reason}`)
}

/** Nothing left to show: back to the resting state (02 §2.1), `peek` with the mascot or `hidden`. */
function rest(d: Draft, reason: string) {
  if (restState(d.s) === 'hidden') return hide(d, reason)
  releaseKeyboard(d)
  const was = d.s.state
  // As `hide` does: the pointer still over the frame re-fires "enter" when the shape changes, which arms the dwell again.
  set(d, { state: 'peek', openedBy: null, focusRow: null, strip: null, arrival: false, hovering: was === 'peek' ? d.s.hovering : false })
  cancel(d, 'peek_dwell', 'arrival_settle', 'collapse_delay', 'auto_collapse', 'new_strip')
  if (was !== 'peek') {
    cancel(d, 'expand_hover')
    request(d, `rest:${reason}`)
  }
}

function togglePin(d: Draft, now: number) {
  if (!d.s.pinned) {
    set(d, { pinned: true })
    cancel(d, 'collapse_delay', 'auto_collapse')
    if (d.s.state !== 'expanded') expand(d, 'pin', now)
    else request(d, 'pin')
    return
  }
  set(d, { pinned: false })
  if (d.s.state === 'expanded' && !d.s.hovering && !d.s.keyboard) schedule(d, 'collapse_delay', now + d.s.settings.collapse_delay_ms)
  request(d, 'unpin')
}

/* ---- the pill ---------------------------------------------------------------- */

function stripFor(c: PillCandidate): Strip {
  return { ref: c.ref, kind: c.kind, tab: tabOfCandidate(c), label: c.label }
}

function showPill(d: Draft, p: PillChanged, now: number, fresh: boolean) {
  set(d, { pill: p.candidate, count: p.count, pillShownAt: now, pendingPill: null })
  cancel(d, 'pill_min')
  if (fresh && d.s.state === 'expanded' && p.candidate) {
    set(d, { strip: stripFor(p.candidate) })
    schedule(d, 'new_strip', now + TIMING.new_strip_ms)
  }
}

function applyPill(d: Draft, p: PillChanged, now: number) {
  if (d.s.flash?.app && d.s.flash.keepPill && p.candidate) {
    // "launching · …" holds until the ladder's own pill takes over (02 §5.4).
    set(d, { flash: null })
    cancel(d, 'flash')
  }
  const s = d.s
  if (s.flash) {
    set(d, { pendingPill: p })
    return
  }
  const c = p.candidate
  if (!c) {
    set(d, { pill: null, count: 0, pendingPill: null })
    cancel(d, 'pill_min')
    // A compact overlay with no candidate stays drawn while dictation is shown (02 §5.6).
    if (s.dictation && (s.state === 'compact' || s.state === 'peek')) {
      if (s.state === 'peek') {
        set(d, { state: 'compact', arrival: false })
        request(d, 'dictation')
      }
      return
    }
    if (s.state === 'compact' || (s.state === 'peek' && s.arrival)) rest(d, 'resolved')
    return
  }
  if (!s.pill) {
    showPill(d, p, now, true)
    if (s.suspended) return
    if (s.state === 'hidden' || (s.state === 'peek' && !s.hovering)) {
      if (s.settings.reduced_motion) {
        set(d, { state: 'compact', arrival: false })
        request(d, 'arrival')
      } else {
        set(d, { state: 'peek', arrival: true })
        cancel(d, 'peek_dwell')
        schedule(d, 'arrival_settle', now + TIMING.arrival_settle_ms)
        d.effects.push({ type: 'pulse' })
        request(d, 'arrival')
      }
    } else if (s.state === 'peek') {
      set(d, { arrival: true })
      schedule(d, 'arrival_settle', now + TIMING.arrival_settle_ms)
      d.effects.push({ type: 'pulse' })
    }
    return
  }
  if (c.ref === s.pill.ref && c.kind === s.pill.kind) {
    set(d, { pill: c, count: p.count })
    return
  }
  if (c.priority === s.pill.priority) {
    const readyAt = s.pillShownAt + s.settings.pill_min_ms
    if (now >= readyAt) showPill(d, p, now, true)
    else {
      set(d, { count: p.count, pendingPill: p })
      schedule(d, 'pill_min', readyAt)
      if (s.state === 'expanded') {
        set(d, { strip: stripFor(c) })
        schedule(d, 'new_strip', now + TIMING.new_strip_ms)
      }
    }
    return
  }
  // Higher priority replaces at once; a lower one means the shown item resolved.
  showPill(d, p, now, c.priority < s.pill.priority)
}

/* ---- timers ------------------------------------------------------------------ */

function onTimer(d: Draft, kind: TimerKind, now: number) {
  if (d.s.timers[kind] === undefined) return
  cancel(d, kind)
  const s = d.s
  switch (kind) {
    case 'peek_dwell':
      if (s.state === 'hidden' && s.hovering && !s.suspended) {
        set(d, { state: 'peek', arrival: false })
        schedule(d, 'expand_hover', now + s.settings.expand_hover_ms)
        request(d, 'peek')
      }
      return
    case 'arrival_settle':
      if (s.state === 'peek' && s.arrival) {
        if (s.pill || s.dictation) {
          set(d, { state: 'compact', arrival: false })
          request(d, 'arrival')
        } else rest(d, 'resolved')
      }
      return
    case 'expand_hover':
      if (!s.hovering) return
      if (s.state === 'peek') {
        if (s.pill) {
          set(d, { state: 'compact', arrival: false })
          request(d, 'hover')
        } else expand(d, 'hover', now)
      } else if (s.state === 'compact' && s.settings.expand_on_hover) expand(d, 'hover', now, { tab: tabOfCandidate(s.pill), row: s.pill?.ref })
      return
    case 'collapse_delay':
    case 'auto_collapse':
      if (s.state === 'expanded' && !s.pinned && !s.keyboard && !s.hovering) collapse(d, kind)
      return
    case 'pill_min':
      if (s.pendingPill) applyPill(d, s.pendingPill, now)
      return
    case 'flash': {
      set(d, { flash: null })
      if (s.pendingPill) applyPill(d, s.pendingPill, now)
      // An app-window flash grown from the resting peek settles back (02 §5.4).
      if (d.s.state === 'compact' && !d.s.pill && !d.s.dictation) rest(d, 'flash')
      return
    }
    case 'new_strip':
      set(d, { strip: null })
      return
    case 'dictation':
      clearDictation(d)
      return
    case 'keyboard_idle':
      releaseKeyboard(d)
      if (s.state === 'expanded' && !s.pinned) collapse(d, 'keyboard_idle')
      return
    case 'reply_hold':
      if (!s.reply || s.hovering) return
      if (s.state === 'expanded' && !s.pinned && !s.keyboard) collapse(d, 'reply_hold')
      else set(d, { reply: null })
      return
  }
}

/* ---- the reducer ------------------------------------------------------------- */

export function reduce(state: MachineState, a: Action): Result {
  const d: Draft = { s: state, effects: [] }
  let handled = true
  const now = a.now
  switch (a.type) {
    case 'pill':
      applyPill(d, { candidate: a.candidate, count: a.count }, now)
      break
    case 'pointer':
      onPointer(d, a.kind, now)
      break
    case 'click':
      if (a.target === 'pill') expand(d, 'click', now, { tab: tabOfCandidate(d.s.pill), row: a.row ?? d.s.pill?.ref ?? null })
      else expand(d, 'click', now)
      break
    case 'hotkey':
      onHotkey(d, a.hotkey, !!a.captureEmpty, now)
      break
    case 'pin':
      togglePin(d, now)
      break
    case 'tab':
      set(d, { tab: a.tab, lastTab: a.tab, focusRow: null })
      break
    case 'focus_row':
      set(d, { focusRow: a.row, zone: a.row ? 'rows' : d.s.zone })
      break
    case 'capture_focus':
      if (!d.s.keyboard) startKeyboard(d, now, false)
      set(d, { zone: 'capture' })
      request(d, 'keyboard')
      break
    case 'capture_blur':
      if (d.s.keyboard) {
        releaseKeyboard(d)
        if (d.s.state === 'expanded' && !d.s.pinned && !d.s.hovering) schedule(d, 'collapse_delay', now + d.s.settings.collapse_delay_ms)
        request(d, 'keyboard_released')
      }
      break
    case 'key':
      handled = reduceKey(d, a, { collapse, expand, startKeyboard, releaseKeyboard, togglePin, cancel, schedule })
      break
    case 'resolved':
      set(d, { flash: { text: a.text, keepPill: !!a.keepPill } })
      schedule(d, 'flash', now + (a.ms ?? (a.keepPill ? TIMING.failure_flash_ms : TIMING.resolved_flash_ms)))
      // An inline action never expands the overlay (02 §5.2): the pointer resting on the pill after the click must not either.
      cancel(d, 'expand_hover')
      break
    case 'leave':
      if (d.s.state === 'expanded' && !d.s.pinned) collapse(d, 'leave')
      else {
        releaseKeyboard(d)
        clearReply(d)
      }
      break
    case 'timer':
      onTimer(d, a.kind, now)
      break
    case 'suspend':
      if (!d.s.suspended) {
        const resumeTo = d.s.state
        releaseKeyboard(d)
        set(d, { suspended: true, resumeTo, state: 'hidden', openedBy: null, strip: null, arrival: false, hovering: false, reply: null, timers: {} })
        request(d, 'suspend')
      }
      break
    case 'resume':
      if (d.s.suspended) {
        const back: OverlayStateName = d.s.resumeTo === 'expanded' && d.s.pinned ? 'expanded' : d.s.pill || d.s.dictation ? 'compact' : restState({ settings: d.s.settings })
        set(d, { suspended: false, resumeTo: null, state: back })
        request(d, 'resume')
      }
      break
    case 'settings':
      set(d, { settings: { ...d.s.settings, ...a.settings } })
      // Now playing turned off while the Music tab shows: back to Sessions (02 §7.1).
      if (!visibleTabs(d.s.settings).includes(d.s.tab)) set(d, { tab: 'sessions', lastTab: 'sessions', focusRow: null })
      settleRest(d)
      break
    case 'dictation_state':
      onDictation(d, a.payload, now)
      break
    case 'dictation_clear':
      clearDictation(d)
      break
    case 'flash':
      onAppFlash(d, a.flash, now)
      break
    case 'flash_clear':
      if (d.s.flash?.app) onTimer(d, 'flash', now)
      break
    case 'reply_dismiss':
      if (!d.s.reply) break
      set(d, { dismissedTurn: d.s.reply.turnId })
      if (d.s.state === 'expanded' && !d.s.pinned) collapse(d, 'dismiss')
      else clearReply(d)
      break
  }
  return { next: d.s, effects: d.effects, handled }
}

/** `overlay.idle` or quiet hours changed: an overlay at rest moves to the new resting state. */
function settleRest(d: Draft) {
  const s = d.s
  if (s.suspended || s.hovering || s.pill || s.dictation || s.flash || s.reply) return
  const target = restState(s)
  if (s.state === 'hidden' && target === 'peek') {
    set(d, { state: 'peek', arrival: false })
    cancel(d, 'peek_dwell')
    request(d, 'rest:settings')
  } else if (s.state === 'peek' && !s.arrival && target === 'hidden') hide(d, 'settings')
}

/**
 * A flash from the app window (02 §5.4): from the resting peek (or `hidden`) it grows to `compact` for the hold and
 * settles back; over a pill it plays as a resolved flash and the pill returns. It never displaces a `waiting` pill,
 * and the dictation pill wins while it shows (the "captured · note" flash of a dictated note is dropped, 02 §5.6).
 */
function onAppFlash(d: Draft, f: OverlayFlash, now: number) {
  const s = d.s
  if (s.suspended || s.dictation || s.state === 'expanded' || s.pill?.kind === 'session_waiting') return
  set(d, { flash: { text: f.text, keepPill: !!f.keep_pill, app: { kind: f.kind, ...(f.open ? { open: f.open } : {}) } } })
  const hold = f.keep_pill ? FLASH_KEEP_MAX_MS : f.kind === 'bad' ? TIMING.failure_flash_ms : TIMING.resolved_flash_ms
  schedule(d, 'flash', now + hold)
  if (s.state === 'hidden' || s.state === 'peek') {
    set(d, { state: 'compact', arrival: false })
    cancel(d, 'peek_dwell', 'arrival_settle', 'expand_hover')
    request(d, 'flash')
  }
}

function onPointer(d: Draft, kind: PointerKind, now: number) {
  const s = d.s
  switch (kind) {
    case 'hot_enter':
    case 'frame_enter':
      // A DOM change under a resting pointer re-fires "enter" (webviews report it with no related target): only a real entry arms the dwell.
      if (s.hovering) return
      set(d, { hovering: true })
      // The user is reading the reply card: its hold stops (02 §5.7).
      cancel(d, 'collapse_delay', 'auto_collapse', 'reply_hold')
      if (s.state === 'hidden') {
        if (!s.suspended) schedule(d, 'peek_dwell', now + s.settings.peek_delay_ms)
      } else if (s.state === 'peek' && !s.arrival) schedule(d, 'expand_hover', now + s.settings.expand_hover_ms)
      else if (s.state === 'compact' && s.settings.expand_on_hover) schedule(d, 'expand_hover', now + s.settings.expand_hover_ms)
      return
    case 'hot_leave':
    case 'frame_leave':
      set(d, { hovering: false })
      cancel(d, 'peek_dwell', 'expand_hover')
      if (s.state === 'peek' && !s.arrival && !s.pill && !s.dictation) rest(d, 'hover_out')
      else if (s.state === 'peek' && !s.arrival && (s.pill || s.dictation)) {
        set(d, { state: 'compact' })
        request(d, 'hover_out')
      } else if (s.state === 'expanded' && !s.pinned && !s.keyboard) schedule(d, 'collapse_delay', now + s.settings.collapse_delay_ms)
      return
    case 'outside_click':
      if (s.state === 'expanded' && !s.pinned) collapse(d, 'outside_click')
      else releaseKeyboard(d)
      return
    case 'drag_dwell':
      if (s.state !== 'expanded') expand(d, 'drag', now, { tab: 'notes' })
      else set(d, { tab: 'notes', lastTab: 'notes' })
      return
  }
}

function onHotkey(d: Draft, hotkey: 'toggle' | 'capture' | 'pin', captureEmpty: boolean, now: number) {
  const s = d.s
  if (hotkey === 'pin') return togglePin(d, now)
  if (s.state === 'expanded') {
    if (hotkey === 'capture' && !(s.keyboard && s.zone === 'capture' && captureEmpty)) {
      set(d, { tab: 'notes', lastTab: 'notes', focusRow: null })
      startKeyboard(d, now, true)
      request(d, 'hotkey:capture')
      return
    }
    if (s.pinned) set(d, { pinned: false })
    collapse(d, `hotkey:${hotkey}`)
    return
  }
  expand(d, 'hotkey', now, { keyboard: true, tab: hotkey === 'capture' ? 'notes' : undefined })
}

/* ---- dictation (02 §5.6) ------------------------------------------------------ */

/** A request's state that belongs on the reply card rather than the pill (02 §5.7). */
function forCard(p: DictationState): boolean {
  if (p.surface !== 'card' || !p.turn_id) return false
  if (p.state === 'thinking' || p.state === 'answering') return true
  if (p.state === 'done') return p.target === 'chat'
  return p.state === 'error' && p.source === 'request'
}

/** The reply card (02 §5.7): opens on the request's first state, takes the reply as it streams, holds after the end. */
function onReply(d: Draft, p: DictationState, now: number) {
  const turnId = p.turn_id!
  if (turnId === d.s.dismissedTurn) return
  // The card replaces the pill for this request.
  if (d.s.dictation) {
    set(d, { dictation: null })
    cancel(d, 'dictation')
  }
  const prev = d.s.reply?.turnId === turnId ? d.s.reply : null
  const ended = p.state === 'done' || p.state === 'error'
  const reply: Reply = {
    threadId: p.thread_id ?? prev?.threadId ?? '',
    turnId,
    text: p.state === 'thinking' ? (prev?.text ?? '') : (p.reply ?? prev?.text ?? ''),
    citations: p.citations ?? prev?.citations ?? [],
    done: ended,
    ...(p.state === 'error' ? { error: p.error?.trim() || 'Chat could not answer' } : {}),
  }
  set(d, { reply })
  if (ended && !d.s.hovering) schedule(d, 'reply_hold', now + TIMING.reply_hold_ms)
  else cancel(d, 'reply_hold')
  if (d.s.suspended || d.s.state === 'expanded') return
  expand(d, 'reply', now, { tab: d.s.tab })
}

function clearReply(d: Draft) {
  if (!d.s.reply) return
  set(d, { reply: null })
  cancel(d, 'reply_hold')
}

function onDictation(d: Draft, p: DictationState, now: number) {
  if (forCard(p)) return onReply(d, p, now)
  // The next hold: the card yields to the listening pill (02 §5.7).
  if (p.state === 'listening' && d.s.reply) {
    clearReply(d)
    set(d, { dictation: { state: 'listening', since: p.since_ms ?? now, level: Math.max(0, Math.min(1, p.level ?? 0)), seconds: p.seconds ?? 0 } })
    if (d.s.state === 'expanded' && !d.s.pinned) collapse(d, 'listening')
  }
  const prev = d.s.dictation
  if (p.state === 'idle') {
    // The core is idle again: a held done / error pill runs out on its own timer; anything in flight is canceled.
    if (prev && prev.state !== 'done' && prev.state !== 'error') clearDictation(d)
    return
  }
  const next: Dictation = {
    state: p.state,
    since: p.since_ms ?? (prev && prev.state === p.state ? prev.since : now),
    level: Math.max(0, Math.min(1, p.level ?? 0)),
    seconds: p.seconds ?? (prev && prev.state === p.state ? prev.seconds : 0),
    ...(p.text !== undefined ? { text: p.text } : {}),
    ...(p.target !== undefined ? { target: p.target } : {}),
    ...(p.words !== undefined ? { words: p.words } : {}),
    ...(p.error !== undefined ? { error: p.error } : {}),
    ...(p.thread_id !== undefined ? { threadId: p.thread_id } : {}),
    ...(p.note_id !== undefined ? { noteId: p.note_id } : {}),
  }
  set(d, { dictation: next })
  if (p.state === 'done' || p.state === 'error') schedule(d, 'dictation', now + DICTATION_HOLD_MS)
  else cancel(d, 'dictation')
  // Feedback is drawn even with nothing else to show: a hidden overlay comes up as the pill (never while suspended).
  if (d.s.suspended) return
  if (d.s.state === 'hidden' || d.s.state === 'peek') {
    set(d, { state: 'compact', arrival: false })
    cancel(d, 'peek_dwell', 'arrival_settle', 'expand_hover')
    request(d, 'dictation')
  }
}

function clearDictation(d: Draft) {
  if (!d.s.dictation) return
  set(d, { dictation: null })
  cancel(d, 'dictation')
  if (d.s.state === 'compact' && !d.s.pill && !d.s.flash) rest(d, 'dictation')
}
