// The first-run steps (03 §2.8; XC-01): ids, order, copy, and what the Done step
// says about each. Plain names: the owner did not know "vault" or "hooks".
import type { IconId } from '../../kit/icons'
import type { Os } from '../../state/settings-store'
import { caps, parseChord } from '../shortcuts/keys'

export type StepId = 'welcome' | 'placement' | 'claude' | 'notes' | 'accounts' | 'automations' | 'dictation' | 'done'
export type Outcome = 'done' | 'skipped'
/** The steps the Done step reports on. */
export type SettingStep = Exclude<StepId, 'welcome' | 'done'>

/** The steps in order; Overlay (placement) only without a notch (XC-02). */
export function stepsFor(notch: boolean): StepId[] {
  const all: StepId[] = ['welcome', 'placement', 'claude', 'notes', 'accounts', 'automations', 'dictation', 'done']
  return notch ? all.filter((s) => s !== 'placement') : all
}

/** Where the flow reopens: the first step neither done nor skipped (Done when all are). Ids this build does not know are ignored. */
export function firstOpen(steps: readonly StepId[], record: Partial<Record<string, Outcome>>): StepId {
  return steps.find((s) => s !== 'done' && record[s] !== 'done' && record[s] !== 'skipped') ?? 'done'
}

/** Only Overlay (on a machine without a notch), Welcome and Done cannot be skipped. */
export function skippable(step: StepId): boolean {
  return step !== 'welcome' && step !== 'placement' && step !== 'done'
}

/** The record with the old ids (`vault`, `hooks`) and anything unknown dropped. */
export function knownRecord(record: Partial<Record<string, Outcome>> | null | undefined): Partial<Record<StepId, Outcome>> {
  const out: Partial<Record<StepId, Outcome>> = {}
  const ids = stepsFor(false)
  for (const [k, v] of Object.entries(record ?? {})) if (ids.includes(k as StepId) && (v === 'done' || v === 'skipped')) out[k as StepId] = v
  return out
}

export interface StepCopy {
  eyebrow: string
  /** A kit icon, or the Claude Code brand mark. */
  icon: IconId | 'claudecode'
  title: string
  lede: string
}

export const COPY: Record<SettingStep, StepCopy> = {
  placement: {
    eyebrow: 'Overlay',
    icon: 'overlay',
    title: 'Lives in the notch. Or at the top, or the right edge.',
    lede: 'This screen has no notch, so pick where the pill sits. Sessions that need you show up there first.',
  },
  claude: {
    eyebrow: 'Claude Code',
    icon: 'claudecode',
    title: 'Approve from the pill. The terminal keeps going.',
    lede: 'Mode adds a few lines to your Claude Code settings, so permission asks, questions and finished runs reach the overlay.',
  },
  notes: {
    eyebrow: 'Notes',
    icon: 'notes',
    title: 'Markdown in a folder you choose. Mode only indexes it.',
    lede: 'Your files stay plain Markdown. Capture a line from anywhere and it lands in Unsorted.',
  },
  accounts: {
    eyebrow: 'Inbox and calendar',
    icon: 'inbox',
    title: 'Gmail, Slack and your calendar, ranked by what needs you.',
    lede: 'Urgent threads come first, each with a line saying why. Mode reads on this device and never sends.',
  },
  automations: {
    eyebrow: 'Automations',
    icon: 'automations',
    title: 'Agents on a schedule. Each run is kept with what it did.',
    lede: 'Start a session, write a note or ask Chat at a set time. Here is one to begin with; switch it on if you want it.',
  },
  dictation: {
    eyebrow: 'Dictation',
    icon: 'mic',
    title: "Hold a key, say it, it's typed. Start with 'Hey Mode' to ask.",
    lede: 'Speech is turned into text on this device and lands in the field you are in. With nothing focused it becomes a note.',
  },
}

export const WELCOME_LINE = 'Your agents, your notes, your inbox. One pill in the notch.'
export const DONE_TITLE = "That's Mode."
export const DONE_LEDE = 'Welcome to Mode. The pill is live in the notch; sessions, notes and messages show up there as they happen.'
/** The repository Star on GitHub opens (03 §2.8 step 7). */
export const REPO_URL = 'https://github.com/gil10101/mode'

/** What the Done step lists for a step that was set; the starter automation's line follows its saved switch. */
export const SET_LINE: Record<Exclude<SettingStep, 'automations'>, string> & { automations: { on: string; off: string; unknown: string } } = {
  placement: 'Overlay placement',
  claude: 'Claude Code linked to the overlay',
  notes: 'Notes folder',
  accounts: 'Inbox and calendar connected',
  automations: { on: 'Nightly tests, on', off: 'Nightly tests, switched off', unknown: 'Nightly tests' },
  dictation: 'Dictation on',
}

/** The Done line of a set step; `starterOn` is the saved switch of the starter automation (null when it could not be read). */
export function setLine(step: SettingStep, starterOn: boolean | null): string {
  return step === 'automations' ? SET_LINE.automations[starterOn === null ? 'unknown' : starterOn ? 'on' : 'off'] : SET_LINE[step]
}

/** The line under the starter's switch on the Automations step: `saved` is the stored automation's switch (null before it is added), `on` the switch shown. */
export function starterMeta(saved: boolean | null, on: boolean): string {
  const nightly = 'it starts a session every night at 23:00.'
  const later = 'Turn it on any time in Automations.'
  if (saved === null) return on ? `Continue adds it, on: ${nightly}` : `Continue adds it, switched off. ${later}`
  if (saved === on) return on ? `Added and on: ${nightly}` : `Added, switched off. ${later}`
  return on ? `Continue turns it on: ${nightly}` : `Continue switches it off. ${later}`
}

/** What the Done step lists for a skipped step: its name and where to set it later. */
export const SKIPPED_LINE: Record<SettingStep, string> = {
  placement: 'Overlay placement · Settings → Overlay',
  claude: 'Claude Code · Integrations → Claude Code',
  notes: 'Notes folder · Settings → Notes',
  accounts: 'Inbox and calendar · Integrations',
  automations: 'Starter automation · Automations',
  dictation: 'Dictation · Settings → Dictation',
}

/** The dictation hold key's caps as the Shortcuts view prints them for `os` (⌃ ⌥ D on macOS, Ctrl Alt D elsewhere; ADR-0062). */
export function holdCaps(hotkey: string | null | undefined, os: Os): string[] {
  return caps(parseChord(hotkey || 'Ctrl+Alt+D', os), os)
}
