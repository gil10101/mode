// The groupings board (03 §5.5, 04 §6.5; app-window.html #n-board): one
// column per grouping of `notes.board_groupings`, rows in compact form, and
// the one-edit drop rule while a note hovers a column (the hint says what a
// refused drop would need). ⊕ Add task shows when a blank note can be made
// to match the column; it creates the note with those fields.
import { useState, type DragEvent } from 'react'
import { MenuButton } from '../kit/button'
import { cx } from '../kit/cx'
import type { GroupingRow } from '../state/contract-notes'
import { dropNoteOf, useNotesStore, type BoardColumn } from '../state/notes-store'
import { addTaskPlan, planDrop, type DropPlan } from './rules'
import { FloatMenu, Item, NOTE_DRAG, NoteTask, dragState } from './widgets'

const SHOWN = 12

function Column({ col, g }: { col: BoardColumn; g: GroupingRow }) {
  const statuses = useNotesStore((s) => s.statuses)
  const today = useNotesStore((s) => s.today)
  const weekStart = useNotesStore((s) => s.weekStart)
  const folders = useNotesStore((s) => s.folders)
  const selectedId = useNotesStore((s) => s.selectedId)
  const [hover, setHover] = useState<DropPlan | null>(null)
  const [adding, setAdding] = useState<string | null>(null)
  const [menu, setMenu] = useState<HTMLElement | null>(null)
  const s = useNotesStore.getState
  const plan = addTaskPlan(g.filter_json, { statuses, today, weekStart, folders })

  function evaluate(): DropPlan | null {
    const row = dragState.row
    if (!row) return null
    return planDrop(dropNoteOf(row), { name: g.name, filter: g.filter_json }, { statuses, today, weekStart, folders, alreadyIn: col.rows.some((r) => r.id === row.id) })
  }

  return (
    <div
      className={cx('m-list nt-list-pad nt-col', hover && (hover.kind === 'edit' ? 'nt-drop-ok' : 'nt-drop-no'))}
      role="group"
      aria-label={g.name}
      data-testid={`column-${g.slug}`}
      onDragOver={(e: DragEvent) => {
        if (!dragState.row && !e.dataTransfer.types.includes(NOTE_DRAG)) return
        e.preventDefault()
        // The drop still fires on a refused column, so the explanation can stay on screen as a notice.
        const p = evaluate()
        if (p && (p.kind !== hover?.kind || p.hint !== hover?.hint)) setHover(p)
      }}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setHover(null)
      }}
      onDrop={(e: DragEvent) => {
        e.preventDefault()
        setHover(null)
        const id = e.dataTransfer.getData(NOTE_DRAG) || dragState.row?.id
        dragState.row = null
        if (id) void s().dropOnGrouping(id, g.slug)
      }}
    >
      <div className="nt-col-head">
        <span className="m-task-glyph">{g.glyph ?? '▣'}</span>
        <span className="m-ellipsis">{g.name}</span>
        <span className="m-n">{col.count}</span>
        <MenuButton onClick={(e) => setMenu(menu ? null : e.currentTarget)} />
      </div>
      {hover && hover.kind !== 'edit' && (
        <p className="nt-drop-hint" role="status">
          {hover.hint}
        </p>
      )}
      {col.rows.slice(0, SHOWN).map((r) => (
        <NoteTask
          key={r.id}
          row={r}
          statuses={statuses}
          today={today}
          variant="board"
          selected={r.id === selectedId}
          onOpen={() => void s().selectNote(r.id)}
          onCycle={() => void s().cycleStatus(r.id)}
          onToggleUrgent={() => void s().setField(r.id, { field: 'urgent', value: !r.urgent })}
          onTag={(t) => void s().tagToBar(t)}
          draggable
          testId="board-row"
        />
      ))}
      {col.rows.length === 0 && <p className="nt-empty">{`Nothing in ${g.name}.`}</p>}
      {col.count > SHOWN && (
        <div className="nt-fold" role="button" tabIndex={0} onClick={() => void s().setLayout('list').then(() => s().selectGrouping(g.slug))}>
          {`${col.count - SHOWN} more ›`}
        </div>
      )}
      {plan.show &&
        (adding === null ? (
          <div className="nt-add" role="button" tabIndex={0} data-testid="column-add" onClick={() => setAdding('')}>
            ⊕ Add task
          </div>
        ) : (
          <div className="nt-add">
            <input
              className="m-input"
              autoFocus
              aria-label={`New task in ${g.name}`}
              placeholder="Title"
              value={adding}
              onChange={(e) => setAdding(e.target.value)}
              onBlur={() => !adding && setAdding(null)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') setAdding(null)
                if (e.key === 'Enter' && adding.trim()) {
                  const t = adding
                  setAdding(null)
                  void s().addTask(t, { filter: g.filter_json })
                }
              }}
            />
          </div>
        ))}
      {menu && (
        <FloatMenu anchor={menu} onClose={() => setMenu(null)} label={g.name} align="right">
          <Item
            label="Open as list"
            onSelect={() => {
              setMenu(null)
              void s().setLayout('list').then(() => s().selectGrouping(g.slug))
            }}
          />
        </FloatMenu>
      )}
    </div>
  )
}

export function Board() {
  const columns = useNotesStore((s) => s.columns)
  const groupings = useNotesStore((s) => s.groupings)
  return (
    <div className="nt-board" data-testid="board">
      {columns.map((col) => {
        const g = groupings.find((x) => x.slug === col.slug)
        return g ? <Column key={col.slug} col={col} g={g} /> : null
      })}
    </div>
  )
}
