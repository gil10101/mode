// The card title rule (04 §8.5, 03 §5.9.5, query-cards.html cards A–C):
// `[Person's] [tasks | notes] · [other chips…] +n`, composed in chip order.
// The type word follows a person's possessive ("Bjorn's tasks"); without a
// person it is shown only for tasks (and the v1.0 types), never for notes.
// At most three segments follow the first; the rest fold into `+n`. A card
// saved as a grouping is titled with the grouping's glyph and name.
import type { ItemType, QueryChip } from '../state/contract-notes'

const TYPE_WORD: Record<ItemType, string> = {
  notes: 'notes',
  tasks: 'tasks',
  messages: 'messages',
  sessions: 'sessions',
  calendar_events: 'events',
}

const MAX_AFTER_FIRST = 3

/** A chip as text: `▦ Tomorrow`, `# Gym`, `“plumber”`, `☐ Tasks`, `Bjorn`. */
export function chipText(chip: QueryChip): string {
  if (chip.kind === 'text') return `“${chip.value}”`
  return chip.glyph ? `${chip.glyph} ${chip.label}` : chip.label
}

function capitalise(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1)
}

export function cardTitle(chips: readonly QueryChip[], grouping?: { glyph: string | null; name: string } | null): string {
  if (grouping) return `${grouping.glyph ?? '▣'} ${grouping.name}`
  const person = chips.find((c) => c.kind === 'person')
  const type = chips.find((c) => c.kind === 'type')
  const typeValue: ItemType = type && type.kind === 'type' ? type.value : 'notes'

  const segments: string[] = []
  let first: string | null = null
  if (person) first = `${person.label}'s ${TYPE_WORD[typeValue]}`
  else if (typeValue !== 'notes') first = capitalise(TYPE_WORD[typeValue])

  for (const c of chips) {
    if (c === person || c === type) continue
    segments.push(chipText(c))
  }
  const all = first ? [first, ...segments] : segments
  if (all.length === 0) return 'Notes'
  const shown = all.slice(0, 1 + MAX_AFTER_FIRST)
  const folded = all.length - shown.length
  return shown.join(' · ') + (folded > 0 ? ` +${folded}` : '')
}

const TYPE_CHIP: Record<ItemType, { label: string; glyph: string }> = {
  tasks: { label: 'Tasks', glyph: '☐' },
  notes: { label: 'Notes', glyph: '≡' },
  messages: { label: 'Messages', glyph: '✉' },
  sessions: { label: 'Sessions', glyph: '▶' },
  calendar_events: { label: 'Events', glyph: '▦' },
}

/** The floating bar's default type chip (03 §5.9.1). */
export function typeChip(type: ItemType): QueryChip {
  return { kind: 'type', value: type, origin: 'typed', ...TYPE_CHIP[type] }
}
