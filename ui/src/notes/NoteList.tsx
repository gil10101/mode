// The note list (03 §5.3; app-window.html #n-list): a group label naming the
// folder or grouping, the count and the sort, then one 32-px row per note
// (glyph that cycles status, title, the when label or the task progress).
// Keyboard per 03 §10.4 / §2.4: ↑ ↓ / J K move, Enter opens, Space cycles,
// ! toggles urgent, P pins, E archives, Esc clears the selection.
import { useMemo, useRef, useState, type KeyboardEvent } from 'react'
import { Mascot } from '../kit/avatars'
import { Button } from '../kit/button'
import { Chip } from '../kit/controls'
import { useNotesStore, type Selection } from '../state/notes-store'
import type { FolderRow, GroupingRow, NoteListRow, SortBy } from '../state/contract-notes'
import { chipText } from './title'
import { FloatMenu, Item, NoteTask, Sep } from './widgets'

export const SORT_WORD: Record<SortBy, string> = { updated_at: 'updated', created_at: 'created', when_date: 'when', title: 'title', urgent: 'urgent', status: 'status' }

export function scopeName(sel: Selection, folders: FolderRow[], groupings: GroupingRow[], vaultName: string): string {
  if (sel.kind === 'folder') return sel.id === 'root' ? vaultName : folders.find((f) => f.id === sel.id)?.name ?? 'Folder'
  if (sel.kind === 'grouping') return groupings.find((g) => g.slug === sel.slug)?.name ?? sel.slug
  return 'Results'
}

function focusEditor() {
  document.querySelector<HTMLElement>('[data-testid="note-editor"] .cm-content')?.focus()
}

export function NoteList() {
  const rows = useNotesStore((s) => s.rows)
  const count = useNotesStore((s) => s.count)
  const selection = useNotesStore((s) => s.selection)
  const folders = useNotesStore((s) => s.folders)
  const groupings = useNotesStore((s) => s.groupings)
  const vaultName = useNotesStore((s) => s.vaultName)
  const statuses = useNotesStore((s) => s.statuses)
  const today = useNotesStore((s) => s.today)
  const selectedId = useNotesStore((s) => s.selectedId)
  const sorts = useNotesStore((s) => s.sorts)
  const ready = useNotesStore((s) => s.ready)
  const listLoading = useNotesStore((s) => s.listLoading)
  const showArchived = useNotesStore((s) => s.showArchived)
  const s = useNotesStore.getState
  const box = useRef<HTMLDivElement>(null)
  const [menu, setMenu] = useState<{ anchor: HTMLElement; row: NoteListRow } | null>(null)

  const name = scopeName(selection, folders, groupings, vaultName)
  const key = selection.kind === 'folder' ? `folder:${selection.id}` : selection.kind === 'grouping' ? `grouping:${selection.slug}` : 'query'
  const grouping = selection.kind === 'grouping' ? groupings.find((g) => g.slug === selection.slug) : undefined
  const sortWord = SORT_WORD[sorts[key] ?? (selection.kind === 'folder' ? 'updated_at' : grouping?.sort_json[0]?.by ?? 'updated_at')]
  const label = `${name} · ${count} ${count === 1 ? 'note' : 'notes'} · ${sortWord}`

  const index = useMemo(() => rows.findIndex((r) => r.id === selectedId), [rows, selectedId])

  function move(d: number) {
    if (!rows.length) return
    const next = rows[Math.max(0, Math.min(rows.length - 1, (index < 0 ? (d > 0 ? -1 : rows.length) : index) + d))]
    void s().selectNote(next.id)
    requestAnimationFrame(() => box.current?.querySelector<HTMLElement>(`[data-note="${next.id}"]`)?.scrollIntoView({ block: 'nearest' }))
  }

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if ((e.target as HTMLElement).closest('input, textarea, .cm-content')) return
    const row: NoteListRow | undefined = rows[index]
    switch (e.key) {
      case 'ArrowDown':
      case 'j':
        e.preventDefault()
        move(1)
        break
      case 'ArrowUp':
      case 'k':
        e.preventDefault()
        move(-1)
        break
      case 'Enter':
        if (row) {
          e.preventDefault()
          focusEditor()
        }
        break
      case ' ':
        if (row) {
          e.preventDefault()
          void s().cycleStatus(row.id)
        }
        break
      case '!':
        if (row) void s().setField(row.id, { field: 'urgent', value: !row.urgent })
        break
      case 'p':
        if (row) void s().setField(row.id, { field: 'pinned', value: !row.pinned })
        break
      case 'e':
        if (row) void s().archive(row.id, !row.archived_at)
        break
      case 'Escape':
        void s().selectNote(null)
        break
    }
  }

  let empty: string | null = null
  const vaultEmpty = ready && !listLoading && rows.length === 0 && selection.kind === 'folder' && selection.id === 'root'
  if (ready && !listLoading && rows.length === 0 && !vaultEmpty) {
    if (selection.kind === 'folder') empty = `No notes in ${name}.`
    else if (selection.kind === 'grouping') empty = `Nothing in ${name}.`
    else empty = 'Nothing matches.'
  }

  return (
    <div className="m-list nt-list-pad nt-list" role="listbox" aria-label={`${name} notes`} tabIndex={0} ref={box} onKeyDown={onKeyDown} data-testid="note-list">
      {selection.kind === 'query' && (
        <div className="m-inline nt-scope-chips">
          {selection.chips.map((c, i) => (
            <Chip key={i} removable onRemove={() => void s().showQuery(selection.chips.filter((_, k) => k !== i))} person={c.kind === 'person' ? { initials: c.initials } : undefined}>
              {c.kind === 'person' ? c.label : chipText(c)}
            </Chip>
          ))}
        </div>
      )}
      <div className="m-group-label" data-testid="list-label">
        {`${label}${showArchived ? ' · with archived' : ''}`}
      </div>
      {rows.map((r) => (
        <NoteTask
          key={r.id}
          row={r}
          statuses={statuses}
          today={today}
          selected={r.id === selectedId}
          onOpen={() => void s().selectNote(r.id)}
          onCycle={() => void s().cycleStatus(r.id)}
          onToggleUrgent={() => void s().setField(r.id, { field: 'urgent', value: !r.urgent })}
          onMenu={(anchor) => setMenu({ anchor, row: r })}
          onTag={(t) => void s().tagToBar(t)}
          draggable
          testId="note-row"
        />
      ))}
      {menu && (
        <FloatMenu anchor={menu.anchor} onClose={() => setMenu(null)} label={menu.row.title} align="right" testId="row-menu">
          {[
            { label: menu.row.pinned ? 'Unpin' : 'Pin', run: () => void s().setField(menu.row.id, { field: 'pinned', value: !menu.row.pinned }) },
            { label: menu.row.archived_at ? 'Unarchive' : 'Archive', run: () => void s().archive(menu.row.id, !menu.row.archived_at) },
          ].map((it) => (
            <Item
              key={it.label}
              label={it.label}
              onSelect={() => {
                setMenu(null)
                it.run()
              }}
            />
          ))}
          <Sep />
          <Item
            label="Delete"
            danger
            onSelect={() => {
              setMenu(null)
              void s().trash(menu.row.id)
            }}
          />
        </FloatMenu>
      )}
      {empty && <p className="nt-empty">{empty}</p>}
      {vaultEmpty && (
        <div className="nt-empty-vault">
          <Mascot size={48} />
          <p>Your vault is empty.</p>
          <Button onClick={() => void s().newNote()}>New note</Button>
        </div>
      )}
    </div>
  )
}
