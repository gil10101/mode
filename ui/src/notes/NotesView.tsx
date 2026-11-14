// The Notes view (03 §5; app-window.html Notes, query-cards.html): the head
// with the List / Board / <grouping ▾> switcher, New note and ⋮ (sort, show
// archived, details); the sidebar tree; the list and the editor, or the
// groupings board; the optional detail panel; the card layer over them; the
// docked query bar at the bottom; the inline notice.
import { forwardRef, useEffect, useRef, useState, type ReactNode } from 'react'
import { Button, MenuButton } from '../kit/button'
import { Seg, SegItem } from '../kit/controls'
import { runs } from '../kit/text'
import { cx } from '../kit/cx'
import { MainBody, MainHead } from '../kit/layout'
import type { SortBy } from '../state/contract-notes'
import { useNotesStore, type Layout } from '../state/notes-store'
import { call } from './api'
import { Board } from './Board'
import { EditorPanel } from './EditorPanel'
import { NoteDetails } from './NoteDetails'
import { NoteList, SORT_WORD } from './NoteList'
import { NotesSidebar } from './NotesSidebar'
import { QueryBar } from './QueryBar'
import { CardLayer } from './QueryCard'
import { FloatMenu, Item, NoticeView, Sep } from './widgets'

export interface NotesViewProps {
  /** Where the view opens (the dev page mirrors the wireframe's moment). */
  initial?: {
    folderId?: string
    groupingSlug?: string
    noteId?: string
    layout?: Layout
    expanded?: string[]
    detailOpen?: boolean
  }
  className?: string
}

/** `.m-seg-item` with a ref, so the grouping picker can hang from it (kit candidate: SegItem forwarding refs). */
const SegButton = forwardRef<HTMLSpanElement, { active?: boolean; onClick: () => void; children: ReactNode }>(function SegButton({ active, onClick, children }, ref) {
  return (
    <span
      ref={ref}
      className={cx('m-seg-item', active && 'is-active')}
      role="button"
      tabIndex={0}
      aria-haspopup="menu"
      aria-pressed={!!active}
      data-testid="grouping-picker"
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onClick()
        }
      }}
    >
      {runs([children, ' '])}
      <span className="m-caret">▾</span>
    </span>
  )
})

/** Starts the store once (idempotent): the Notes view, card layers and the floating bar share it. */
export function useNotesStart(): void {
  useEffect(() => {
    void useNotesStore.getState().start()
  }, [])
}

const SORTS: SortBy[] = ['updated_at', 'created_at', 'when_date', 'title']

export function NotesView({ initial, className }: NotesViewProps) {
  const layout = useNotesStore((s) => s.layout)
  const selection = useNotesStore((s) => s.selection)
  const groupings = useNotesStore((s) => s.groupings)
  const detailOpen = useNotesStore((s) => s.detailOpen)
  const barChips = useNotesStore((s) => s.barChips)
  const notice = useNotesStore((s) => s.notice)
  const showArchived = useNotesStore((s) => s.showArchived)
  const sorts = useNotesStore((s) => s.sorts)
  const [picker, setPicker] = useState(false)
  const [more, setMore] = useState<HTMLElement | null>(null)
  const pickerRef = useRef<HTMLSpanElement>(null)
  const s = useNotesStore.getState

  useEffect(() => {
    const st = s()
    if (initial && !st.ready) {
      useNotesStore.setState({
        ...(initial.folderId ? { selection: { kind: 'folder', id: initial.folderId } } : initial.groupingSlug ? { selection: { kind: 'grouping', slug: initial.groupingSlug } } : {}),
        ...(initial.layout ? { layout: initial.layout } : {}),
        ...(initial.expanded ? { expanded: initial.expanded } : {}),
        ...(initial.detailOpen ? { detailOpen: true } : {}),
      })
    }
    void st.start().then(() => {
      if (initial?.noteId && !s().editor.noteId) void s().selectNote(initial.noteId)
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const current = selection.kind === 'grouping' ? groupings.find((g) => g.slug === selection.slug) : undefined
  const premade = groupings.filter((g) => g.kind === 'premade').sort((a, b) => a.position - b.position)
  const custom = groupings.filter((g) => g.kind === 'custom').sort((a, b) => a.position - b.position)
  const sortKey = selection.kind === 'folder' ? `folder:${selection.id}` : selection.kind === 'grouping' ? `grouping:${selection.slug}` : 'query'
  const sortBy = sorts[sortKey] ?? 'updated_at'

  return (
    <section
      className={cx('nt-view', className)}
      aria-label="Notes"
      data-testid="notes-view"
      onKeyDown={(e) => {
        if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === 'b') {
          e.preventDefault()
          void s().setLayout(layout === 'board' ? 'list' : 'board')
        }
      }}
    >
      <MainHead title="Notes">
        <Seg label="Layout">
          <SegItem active={layout === 'list'} onClick={() => void s().setLayout('list')}>
            List
          </SegItem>
          <SegItem active={layout === 'board'} onClick={() => void s().setLayout('board')}>
            Board
          </SegItem>
          <SegButton ref={pickerRef} active={picker} onClick={() => setPicker((p) => !p)}>
            {current?.name ?? 'Views'}
          </SegButton>
        </Seg>
        <Button variant="primary" onClick={() => void s().newNote()}>
          New note
        </Button>
        <MenuButton onClick={(e) => setMore(more ? null : e.currentTarget)} />
      </MainHead>
      <MainBody className={cx('nt-notes', detailOpen && 'has-detail', layout === 'board' && 'is-board')} testId="notes-body">
        <NotesSidebar />
        {layout === 'list' ? (
          <>
            <NoteList />
            <EditorPanel />
          </>
        ) : (
          <Board />
        )}
        {detailOpen && <NoteDetails />}
        <CardLayer view="notes" />
        <div className="nt-dock">
          <QueryBar
            chips={barChips}
            onChipsChange={(c) => s().setBarChips(c)}
            onSubmit={(c, o) => {
              s().setBarChips([])
              void s().spawnCard(c, { force: o.force })
            }}
            testId="query-bar"
          />
        </div>
      </MainBody>
      <NoticeView notice={notice} onDismiss={() => s().dismissNotice()} />
      {picker && pickerRef.current && (
        <FloatMenu anchor={pickerRef.current} onClose={() => setPicker(false)} label="Groupings" align="right" testId="grouping-menu">
          {premade.map((g) => (
            <Item
              key={g.id}
              glyph={g.glyph ?? '▣'}
              label={g.name}
              hint={g.count}
              current={current?.id === g.id}
              onSelect={() => {
                setPicker(false)
                void s().selectGrouping(g.slug)
              }}
            />
          ))}
          {custom.length > 0 && <Sep />}
          {custom.map((g) => (
            <Item
              key={g.id}
              glyph={g.glyph ?? '▣'}
              label={g.name}
              hint={g.count}
              current={current?.id === g.id}
              onSelect={() => {
                setPicker(false)
                void s().selectGrouping(g.slug)
              }}
            />
          ))}
          <Sep />
          <Item
            label="Manage views…"
            onSelect={() => {
              setPicker(false)
              void call('app_open', { view: 'settings', filter: 'notes.groupings' }).catch(() => {})
            }}
          />
        </FloatMenu>
      )}
      {more && (
        <FloatMenu anchor={more} onClose={() => setMore(null)} label="Notes" align="right" testId="notes-menu">
          {SORTS.map((by) => (
            <Item
              key={by}
              label={`Sort by ${SORT_WORD[by]}`}
              current={sortBy === by}
              onSelect={() => {
                setMore(null)
                void s().setSort(by)
              }}
            />
          ))}
          <Sep />
          <Item
            label="Show archived"
            current={showArchived}
            onSelect={() => {
              setMore(null)
              void s().setShowArchived(!showArchived)
            }}
          />
          <Item
            label={detailOpen ? 'Hide details' : 'Show details'}
            onSelect={() => {
              setMore(null)
              s().setDetailOpen(!detailOpen)
            }}
          />
        </FloatMenu>
      )}
    </section>
  )
}
