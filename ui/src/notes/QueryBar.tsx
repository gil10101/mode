// The query bar (03 §5.9.1–§5.9.3, 04 §8, query-cards.html): typed text
// becomes chips through the core's resolver (`query_resolve`) on every
// keystroke; Tab (or Enter on an arrowed suggestion) commits the highlighted
// suggestion; Enter resolves the rest and spawns a card; Backspace on an
// empty field turns the last chip back into text; Esc closes the list, then
// blurs. Three surfaces share it: the bar docked under the Notes view, a
// card's refine bar (bare, chips AND-ed into that card), and the floating
// variant the shell opens with ⌘/Ctrl+K.
import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Button } from '../kit/button'
import { Chip } from '../kit/controls'
import { cx } from '../kit/cx'
import type { ItemType, QueryChip, QuerySuggestion } from '../state/contract-notes'
import { useNotesStore } from '../state/notes-store'
import { call } from './api'
import { chipToText } from './rules'
import { chipText, typeChip } from './title'
import { Item } from './widgets'

/** Examples the placeholder cycles through (03 §5.9.1). */
export const PLACEHOLDERS = ['Show me tasks for next week…', "Bjorn's overdue tasks", '#gym this week', 'messages from Ana']

/** The `+` chip picker (query-cards.html state 7): a kind, then its values through the same autocomplete. */
const PICKER: { glyph: string; label: string; hint: string; prefix: string }[] = [
  { glyph: '◉', label: 'Person', hint: 'contacts', prefix: '@' },
  { glyph: '☐', label: 'Type', hint: 'tasks · notes', prefix: 'type:' },
  { glyph: '◑', label: 'Status', hint: 'todo · doing · done · urgent', prefix: 'is:' },
  { glyph: '▦', label: 'When', hint: 'today · tomorrow · a date', prefix: 'when:' },
  { glyph: '#', label: 'Tag', hint: 'tags', prefix: '#' },
  { glyph: '⎇', label: 'Repo', hint: 'repos', prefix: 'repo:' },
]

export interface QueryBarProps {
  chips: QueryChip[]
  onChipsChange: (chips: QueryChip[]) => void
  /** Enter with at least one chip; `force` = ⌘/Ctrl+Enter (a new card even if an identical one is open). */
  onSubmit?: (chips: QueryChip[], opts: { force: boolean }) => void
  surface?: 'bar' | 'refine' | 'floating'
  /** The floating bar's view type (03 §5.9.1). */
  viewType?: ItemType
  /** Esc on an empty, closed bar (the floating bar closes; a refine bar's second Esc closes its card). */
  onEscape?: () => void
  autoFocus?: boolean
  label?: string
  testId?: string
}

function sameList(a: QueryChip[], b: QueryChip[]): boolean {
  return a.length === b.length && a.every((c, i) => c.kind === b[i].kind && JSON.stringify(c.value) === JSON.stringify(b[i].value) && c.label === b[i].label)
}

const reducedMotion = () => typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches

export function QueryBar({ chips, onChipsChange, onSubmit, surface = 'bar', viewType, onEscape, autoFocus, label = 'Query bar', testId }: QueryBarProps) {
  const [text, setText] = useState('')
  const [suggestions, setSuggestions] = useState<QuerySuggestion[]>([])
  const [highlight, setHighlight] = useState(0)
  const [explicit, setExplicit] = useState(false)
  const [picker, setPicker] = useState(false)
  const [focused, setFocused] = useState(false)
  const [example, setExample] = useState(0)
  const [spark, setSpark] = useState<Set<string>>(new Set())
  const input = useRef<HTMLInputElement>(null)
  const seq = useRef(0)
  const chipsRef = useRef(chips)
  chipsRef.current = chips
  const showError = useNotesStore((s) => s.showNotice)

  // The placeholder cycles while the bar is empty and idle.
  useEffect(() => {
    if (focused || chips.length || text || surface === 'refine') return
    const t = setInterval(() => setExample((i) => (i + 1) % PLACEHOLDERS.length), 4000)
    return () => clearInterval(t)
  }, [focused, chips.length, text, surface])

  useEffect(() => {
    if (autoFocus) input.current?.focus()
  }, [autoFocus])

  function sparkle(prev: QueryChip[], next: QueryChip[]) {
    if (reducedMotion()) return
    const old = new Set(prev.map((c) => `${c.kind}:${JSON.stringify(c.value)}`))
    const fresh = next.map((c) => `${c.kind}:${JSON.stringify(c.value)}`).filter((k) => !old.has(k))
    if (!fresh.length) return
    setSpark(new Set(fresh))
    setTimeout(() => setSpark(new Set()), 320)
  }

  function commit(next: QueryChip[], rest: string) {
    sparkle(chipsRef.current, next)
    onChipsChange(next)
    setText(rest)
  }

  async function resolve(value: string) {
    const n = ++seq.current
    if (!value.trim()) {
      setSuggestions([])
      return
    }
    try {
      const r = await call('query_resolve', { text: value, context: { chips: chipsRef.current, final: false, surface, view_type: viewType } })
      if (n !== seq.current) return
      if (!sameList(r.chips, chipsRef.current)) commit(r.chips, r.rest)
      else if (r.rest !== value) setText(r.rest)
      setSuggestions(r.suggestions)
      setHighlight(0)
      setExplicit(false)
    } catch (e) {
      if (n === seq.current) setSuggestions([])
      showError(e instanceof Error ? e.message : String(e))
    }
  }

  function accept(s: QuerySuggestion) {
    seq.current++
    commit(s.apply.chips, s.apply.rest)
    setSuggestions([])
    setExplicit(false)
    input.current?.focus()
    if (s.apply.rest.trim()) void resolve(s.apply.rest)
  }

  async function submit(force: boolean) {
    const n = ++seq.current
    let next = chipsRef.current
    if (text.trim()) {
      try {
        const r = await call('query_resolve', { text, context: { chips: next, final: true, surface, view_type: viewType } })
        if (n !== seq.current) return
        next = r.chips
      } catch (e) {
        showError(e instanceof Error ? e.message : String(e))
        return
      }
    }
    setSuggestions([])
    setText('')
    if (!sameList(next, chipsRef.current)) {
      sparkle(chipsRef.current, next)
      onChipsChange(next)
    }
    if (next.length && onSubmit) onSubmit(next, { force })
  }

  function onKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    const open = suggestions.length > 0
    if (e.key === 'Enter') {
      e.preventDefault()
      if (open && explicit) accept(suggestions[highlight])
      else void submit(e.metaKey || e.ctrlKey)
    } else if (e.key === 'Tab' && open) {
      e.preventDefault()
      accept(suggestions[highlight])
    } else if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && open) {
      e.preventDefault()
      const d = e.key === 'ArrowDown' ? 1 : -1
      setHighlight((h) => (h + d + suggestions.length) % suggestions.length)
      setExplicit(true)
    } else if (e.key === 'Escape') {
      e.preventDefault()
      if (open || picker) {
        seq.current++
        setSuggestions([])
        setPicker(false)
      } else {
        input.current?.blur()
        onEscape?.()
      }
    } else if (e.key === 'Backspace' && text === '' && chips.length) {
      e.preventDefault()
      const last = chips[chips.length - 1]
      onChipsChange(chips.slice(0, -1))
      const back = chipToText(last)
      setText(back)
      void resolve(back)
    }
  }

  const ghost = suggestions[highlight]?.completion && text && !/\s$/.test(text) ? suggestions[highlight].completion : null
  const placeholder = chips.length ? (surface === 'floating' && chips.length === 1 && chips[0].kind === 'type' && chips[0].value !== 'notes' ? 'from Ana this week…' : 'Refine…') : surface === 'refine' ? 'Refine…' : PLACEHOLDERS[example]

  const query = (
    <div className="m-query" onMouseDown={(e) => e.target === e.currentTarget && (e.preventDefault(), input.current?.focus())}>
      {chips.map((c, i) => (
        <Chip
          key={`${c.kind}:${JSON.stringify(c.value)}:${i}`}
          className={cx(c.kind === 'text' && 'nt-term', spark.has(`${c.kind}:${JSON.stringify(c.value)}`) && 'nt-spark')}
          person={c.kind === 'person' ? { initials: c.initials } : undefined}
          removable
          onRemove={() => onChipsChange(chips.filter((_, k) => k !== i))}
        >
          {c.kind === 'person' ? c.label : chipText(c)}
        </Chip>
      ))}
      <span className="nt-entry">
        <span className="nt-sizer" aria-hidden="true">
          {text || placeholder}
        </span>
        <input
          ref={input}
          size={1}
          value={text}
          placeholder={placeholder}
          aria-label={label}
          aria-autocomplete="list"
          aria-expanded={suggestions.length > 0}
          spellCheck={false}
          autoComplete="off"
          onChange={(e) => {
            setText(e.target.value)
            setPicker(false)
            void resolve(e.target.value)
          }}
          onKeyDown={onKeyDown}
          onFocus={() => setFocused(true)}
          onBlur={() => {
            setFocused(false)
            setPicker(false)
          }}
          data-testid={testId ? `${testId}-input` : undefined}
        />
      </span>
      {ghost && <span className="nt-completion">{ghost}</span>}
    </div>
  )

  const pop = (suggestions.length > 0 || picker) && (
    <div className="m-menu nt-menu nt-pop" role="listbox" aria-label="Suggestions" data-testid={testId ? `${testId}-pop` : undefined}>
      {picker
        ? PICKER.map((p) => (
            <Item
              key={p.label}
              glyph={p.glyph}
              label={p.label}
              hint={p.hint}
              onSelect={() => {
                setPicker(false)
                setText(p.prefix)
                input.current?.focus()
                void resolve(p.prefix)
              }}
            />
          ))
        : suggestions.map((s, i) => <Item key={s.id} glyph={s.glyph ?? ''} label={s.label} hint={s.hint} active={i === highlight} onSelect={() => accept(s)} />)}
    </div>
  )

  if (surface === 'refine') {
    return (
      <>
        {query}
        {pop}
      </>
    )
  }

  return (
    <div className={cx('nt-bar', surface === 'floating' && 'is-floating')} role="search" aria-label={label} data-testid={testId}>
      <Button
        variant="icon"
        quiet
        aria-label="Add a chip"
        onMouseDown={(e) => e.preventDefault()}
        onClick={() => {
          setSuggestions([])
          setPicker((p) => !p)
          input.current?.focus()
        }}
      >
        +
      </Button>
      {query}
      <Button
        variant="icon"
        quiet
        aria-label="Clear"
        onClick={() => {
          seq.current++
          onChipsChange([])
          setText('')
          setSuggestions([])
          input.current?.focus()
        }}
      >
        ×
      </Button>
      {pop}
    </div>
  )
}

export interface FloatingQueryProps {
  /** The card layer the spawned cards join (03 §5.9.6). */
  view?: string
  /** The view's item type: its type chip is pre-committed (Inbox → messages, …). */
  viewType?: ItemType
  onClose?: () => void
}

/** The ⌘/Ctrl+K variant: centered near the top of any view; the shell wires the key. */
export function FloatingQuery({ view = 'notes', viewType = 'notes', onClose }: FloatingQueryProps) {
  const [chips, setChips] = useState<QueryChip[]>(() => (viewType === 'notes' ? [] : [typeChip(viewType)]))
  const spawn = useNotesStore((s) => s.spawnCard)
  return (
    <div className="nt-floating" role="dialog" aria-label="Query" data-testid="floating-query">
      <QueryBar
        surface="floating"
        chips={chips}
        onChipsChange={setChips}
        viewType={viewType}
        autoFocus
        onEscape={onClose}
        onSubmit={(c, o) => {
          void spawn(c, { view, force: o.force })
          onClose?.()
        }}
        testId="floating-bar"
      />
    </div>
  )
}
