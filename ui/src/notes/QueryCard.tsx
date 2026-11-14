// Query cards (03 §5.9.5–§5.9.6, 04 §8.5, query-cards.html): title composed
// from the chips, the live count, ↗ expand, ⋮ (Save as grouping, Show in
// overlay, Sort ›, Group by ›, Copy as text, Duplicate card, Close other
// cards), ×, 32-px rows with the glyph cycle and the hover control, ⊕ Add
// task when the chips are assignable, and the card's own refine bar.
// Cards are dragged by the title, raised on click and cascade on spawn.
import { Fragment, useMemo, useRef, useState, type CSSProperties, type MouseEvent as ReactMouseEvent, type ReactElement } from 'react'
import { Button, MenuButton } from '../kit/button'
import { cx } from '../kit/cx'
import type { NoteListRow, SortBy } from '../state/contract-notes'
import { useNotesStore, type CardState, type GroupBy } from '../state/notes-store'
import { whenLabel } from './dates'
import { QueryBar } from './QueryBar'
import { addTaskPlan, normTag, statusLabel } from './rules'
import { cardTitle } from './title'
import { FloatMenu, Item, NoteTask, Sep, useMenu } from './widgets'

const SORT_LABEL: Record<SortBy, string> = { urgent: 'urgent', updated_at: 'updated', created_at: 'created', when_date: 'when', title: 'title', status: 'status' }
const SORTS: SortBy[] = ['urgent', 'updated_at', 'created_at', 'when_date', 'title']
const GROUPS: GroupBy[] = ['none', 'status', 'when', 'tag', 'folder']

type MenuPage = 'main' | 'sort' | 'group' | 'save'

export interface QueryCardProps {
  card: CardState
  /** ↗: the shell may also switch to the Notes view (03 §5.9.5). */
  onExpand?: (card: CardState) => void
  /** Opening a row; defaults to selecting the note in the Notes view. */
  onOpenNote?: (id: string) => void
}

function asText(rows: NoteListRow[]): string {
  const marker = (r: NoteListRow) => (r.status === 'done' ? 'x' : r.urgent ? '!' : r.status === 'doing' ? '/' : ' ')
  return rows.map((r) => `- [${marker(r)}] ${r.title}`).join('\n') + '\n'
}

export function QueryCard({ card, onExpand, onOpenNote }: QueryCardProps) {
  const store = useNotesStore
  const statuses = useNotesStore((s) => s.statuses)
  const today = useNotesStore((s) => s.today)
  const weekStart = useNotesStore((s) => s.weekStart)
  const folders = useNotesStore((s) => s.folders)
  const topZ = useNotesStore((s) => s.cards.reduce((m, c) => (c.view === card.view ? Math.max(m, c.z) : m), 0))
  const menu = useMenu<MenuPage>()
  const [name, setName] = useState('')
  const [saveError, setSaveError] = useState<string | null>(null)
  const [adding, setAdding] = useState<string | null>(null)
  const ref = useRef<HTMLElement>(null)

  const chips = useMemo(() => [...card.chips, ...card.refine], [card.chips, card.refine])
  const title = cardTitle(chips, card.grouping)
  const ownWhen = chips.some((c) => c.kind === 'when' && 'in' in c.value && c.value.in.length === 1)
  const ownTags = chips.flatMap((c) => (c.kind === 'tag' && typeof c.value === 'string' ? [c.value] : []))
  const plan = card.filter && (card.type === 'notes' || card.type === 'tasks') ? addTaskPlan(card.filter, { statuses, today, weekStart, folders }) : null
  const count = card.status === 'loading' && !card.rows.length ? '…' : card.status === 'error' ? '—' : String(card.count)

  function startDrag(e: ReactMouseEvent<HTMLDivElement>) {
    if ((e.target as HTMLElement).closest('button, .m-actions')) return
    const el = ref.current
    const layer = el?.offsetParent as HTMLElement | null
    if (!el || !layer) return
    e.preventDefault()
    store.getState().raiseCard(card.id)
    const startX = e.clientX
    const startY = e.clientY
    const x0 = el.offsetLeft
    const y0 = el.offsetTop
    const move = (ev: MouseEvent) => {
      const x = Math.max(0, Math.min(layer.clientWidth - 80, x0 + ev.clientX - startX))
      const y = Math.max(0, Math.min(layer.clientHeight - 40, y0 + ev.clientY - startY))
      store.getState().moveCard(card.id, { x, y, anchor: 'top' })
    }
    const up = () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
  }

  function startResize(e: ReactMouseEvent<HTMLDivElement>) {
    e.preventDefault()
    e.stopPropagation()
    const startX = e.clientX
    const w0 = card.width
    const move = (ev: MouseEvent) => store.getState().resizeCard(card.id, w0 + ev.clientX - startX)
    const up = () => {
      window.removeEventListener('mousemove', move)
      window.removeEventListener('mouseup', up)
    }
    window.addEventListener('mousemove', move)
    window.addEventListener('mouseup', up)
  }

  function open(id: string) {
    if (onOpenNote) onOpenNote(id)
    else {
      const s = store.getState()
      if (s.layout === 'board') void s.setLayout('list')
      void s.selectNote(id)
    }
  }

  const rows: ReactElement[] = []
  const groupOf = (r: NoteListRow): string => {
    switch (card.groupBy) {
      case 'status':
        return statusLabel(r.status, statuses)
      case 'when':
        return whenLabel(r.when_bucket, r.when_date, today) ?? 'no date'
      case 'tag':
        return r.tags[0] ? `#${r.tags[0]}` : 'no tag'
      case 'folder':
        return r.path.includes('/') ? r.path.slice(0, r.path.lastIndexOf('/')) : 'vault root'
      default:
        return ''
    }
  }
  let lastGroup: string | null = null
  for (const r of card.rows) {
    if (card.groupBy !== 'none') {
      const g = groupOf(r)
      if (g !== lastGroup) rows.push(<div key={`g-${g}`} className="m-group-label">{g}</div>)
      lastGroup = g
    }
    rows.push(
      <NoteTask
        key={r.id}
        row={r}
        statuses={statuses}
        today={today}
        variant="card"
        hideWhen={ownWhen}
        hideTags={ownTags.map(normTag)}
        onOpen={() => open(r.id)}
        onCycle={() => void store.getState().cycleStatus(r.id)}
        onToggleUrgent={() => void store.getState().setField(r.id, { field: 'urgent', value: !r.urgent })}
        onTag={(t) => void store.getState().tagToBar(t)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') open(r.id)
          else if (e.key === ' ') {
            e.preventDefault()
            void store.getState().cycleStatus(r.id)
          } else if (e.key === '!') void store.getState().setField(r.id, { field: 'urgent', value: !r.urgent })
        }}
        draggable
        testId="card-row"
      />,
    )
  }

  const style: CSSProperties = {
    left: card.pos.x,
    width: card.width,
    zIndex: card.z,
    ...(card.pos.anchor === 'top' ? { top: card.pos.y } : { bottom: card.pos.y }),
  }

  return (
    <section
      ref={ref}
      className={cx('m-card nt-card', card.z === topZ && 'is-front')}
      style={style}
      aria-label={title}
      data-testid="query-card"
      data-card={card.id}
      tabIndex={-1}
      onMouseDown={() => store.getState().raiseCard(card.id)}
      onKeyDown={(e) => {
        // The focused card (03 §10.4): ⌘/Ctrl+W close · ⌘/Ctrl+E expand · ⌘/Ctrl+S save as grouping; Esc on the card closes it.
        const mod = e.metaKey || e.ctrlKey
        if (e.key === 'Escape' && e.target === e.currentTarget) store.getState().closeCard(card.id)
        else if (mod && e.key.toLowerCase() === 'w') {
          e.preventDefault()
          store.getState().closeCard(card.id)
        } else if (mod && e.key.toLowerCase() === 'e') {
          e.preventDefault()
          if (onExpand) onExpand(card)
          else void store.getState().showQuery(chips)
        } else if (mod && e.key.toLowerCase() === 's') {
          e.preventDefault()
          const more = ref.current?.querySelector<HTMLElement>('.m-actions [aria-label="More"]')
          if (more) {
            setName('')
            setSaveError(null)
            menu.show(more, 'save')
          }
        }
      }}
    >
      <div className="m-card-head" onMouseDown={startDrag}>
        <span className="m-card-title" data-testid="card-title">
          {title}
        </span>
        <span className="m-n" data-testid="card-count">
          {count}
        </span>
        <span className="m-actions">
          <Button
            variant="icon"
            quiet
            aria-label="Expand"
            onClick={() => {
              if (onExpand) onExpand(card)
              else void store.getState().showQuery(chips)
            }}
          >
            ↗
          </Button>
          <MenuButton onClick={(e) => (menu.open ? menu.close() : menu.show(e.currentTarget, 'main'))} />
          <Button variant="icon" quiet aria-label="Close" onClick={() => store.getState().closeCard(card.id)}>
            ×
          </Button>
        </span>
      </div>
      <div className="nt-rows">
        {card.status === 'loading' && !card.rows.length && (
          <div className="m-shimmer">
            <i />
            <i />
            <i />
          </div>
        )}
        {card.status === 'error' && (
          <>
            <p className="nt-empty">Couldn't run this query.</p>
            <div className="m-card-actions">
              <Button onClick={() => void store.getState().runCard(card.id)}>Retry</Button>
            </div>
          </>
        )}
        {card.status === 'ready' && !card.rows.length && <p className="nt-empty">{card.type === 'notes' || card.type === 'tasks' ? 'Nothing matches.' : 'Coming in v1.0'}</p>}
        {rows}
        {card.count > card.rows.length && card.status === 'ready' && (
          <div className="m-task nt-add-row" role="button" tabIndex={0} onClick={() => void store.getState().showQuery(chips)}>
            <span className="m-task-glyph" />
            <span className="m-task-title">{`${card.count - card.rows.length} more`}</span>
          </div>
        )}
        {plan?.show && card.status !== 'error' &&
          (adding === null ? (
            <div className="m-task nt-add-row" role="button" tabIndex={0} data-testid="card-add" onClick={() => setAdding('')}>
              <span className="m-task-glyph">⊕</span>
              <span className="m-task-title">Add task</span>
            </div>
          ) : (
            <div className="m-task nt-add-row">
              <span className="m-task-glyph">⊕</span>
              <input
                className="m-input"
                autoFocus
                value={adding}
                aria-label="New task title"
                placeholder="Title"
                onChange={(e) => setAdding(e.target.value)}
                onBlur={() => !adding && setAdding(null)}
                onKeyDown={(e) => {
                  if (e.key === 'Escape') setAdding(null)
                  if (e.key === 'Enter' && adding.trim() && card.filter) {
                    const t = adding
                    setAdding(null)
                    void store.getState().addTask(t, { filter: card.filter })
                  }
                }}
              />
            </div>
          ))}
      </div>
      <div className="nt-refine">
        <QueryBar
          surface="refine"
          chips={card.refine}
          onChipsChange={(c) => void store.getState().setCardRefine(card.id, c)}
          onEscape={() => ref.current?.focus()}
          label={`Refine ${title}`}
          testId="refine"
        />
      </div>
      <div className="nt-resize" aria-hidden="true" onMouseDown={startResize} />
      {menu.open && menu.anchor && (
        <FloatMenu anchor={menu.anchor} onClose={menu.close} label="Card" align="right" testId="card-menu">
          {menu.page === 'main' && (
            <>
              <Item
                label="Save as grouping"
                onSelect={() => {
                  setName('')
                  setSaveError(null)
                  menu.go('save')
                }}
              />
              <Item label="Show in overlay" dimmed={!card.grouping} reason={card.grouping ? undefined : 'Save as a grouping first'} current={!!card.grouping?.show_in_overlay} onSelect={() => void store.getState().toggleCardOverlay(card.id)} />
              <Item label="Sort" hint={`${SORT_LABEL[card.sort]} ›`} onSelect={() => menu.go('sort')} />
              <Item label="Group by" hint={`${card.groupBy} ›`} onSelect={() => menu.go('group')} />
              <Item
                label="Copy as text"
                onSelect={() => {
                  void navigator.clipboard?.writeText(asText(card.rows)).catch(() => {})
                  store.getState().showNotice('Copied')
                  menu.close()
                }}
              />
              <Item
                label="Duplicate card"
                onSelect={() => {
                  menu.close()
                  void store.getState().duplicateCard(card.id)
                }}
              />
              <Sep />
              <Item
                label="Close other cards"
                onSelect={() => {
                  menu.close()
                  store.getState().closeOtherCards(card.id)
                }}
              />
            </>
          )}
          {menu.page === 'sort' &&
            SORTS.map((s) => (
              <Item
                key={s}
                label={SORT_LABEL[s]}
                current={card.sort === s}
                onSelect={() => {
                  menu.close()
                  void store.getState().setCardSort(card.id, s)
                }}
              />
            ))}
          {menu.page === 'group' &&
            GROUPS.map((g) => (
              <Item
                key={g}
                label={g}
                current={card.groupBy === g}
                onSelect={() => {
                  menu.close()
                  store.getState().setCardGroupBy(card.id, g)
                }}
              />
            ))}
          {menu.page === 'save' && (
            <form
              className="nt-save"
              onSubmit={(e) => {
                e.preventDefault()
                void store
                  .getState()
                  .saveCardAsGrouping(card.id, name)
                  .then((err) => (err ? setSaveError(err) : menu.close()))
              }}
            >
              <input className="m-input" autoFocus aria-label="Grouping name" placeholder="Name" value={name} onChange={(e) => setName(e.target.value)} data-testid="grouping-name" />
              {saveError && <p className="nt-error">{saveError}</p>}
              <button type="submit" className="m-btn" disabled={!name.trim()}>
                Save
              </button>
            </form>
          )}
        </FloatMenu>
      )}
    </section>
  )
}

export interface CardLayerProps {
  /** Cards belong to the view they were spawned in (03 §5.9.6). */
  view?: string
  onExpand?: (card: CardState) => void
  onOpenNote?: (id: string) => void
  className?: string
}

/** The layer over a view's content where its cards float. */
export function CardLayer({ view = 'notes', onExpand, onOpenNote, className }: CardLayerProps) {
  const cards = useNotesStore((s) => s.cards)
  return (
    <div className={cx('nt-cards', className)} data-testid="card-layer">
      {cards
        .filter((c) => c.view === view)
        .map((c) => (
          <Fragment key={c.id}>
            <QueryCard card={c} onExpand={onExpand} onOpenNote={onOpenNote} />
          </Fragment>
        ))}
    </div>
  )
}
