// The notes sidebar (03 §5.2, §5.5; app-window.html #n-tree): the vault root,
// the folder tree with chevrons, counts and drop targets for notes, then the
// Views list (groupings with `show_in_sidebar`, premade first, custom last).
// A view's secondary actions (edit, hide, show in overlay) sit on its
// context menu so the rows stay one line.
import { useState, type DragEvent, type ReactElement, type ReactNode } from 'react'
import { cx } from '../kit/cx'
import { GroupLabel } from '../kit/lists'
import type { FolderRow, GroupingRow } from '../state/contract-notes'
import { useNotesStore } from '../state/notes-store'
import { call } from './api'
import { NOTE_DRAG, dragState, FloatMenu, Item } from './widgets'

interface RowProps {
  label: ReactNode
  count?: number
  glyph?: ReactNode
  open?: boolean
  leaf?: boolean
  active?: boolean
  title?: string
  onClick: () => void
  onDropNote?: (id: string) => void
  onContextMenu?: (anchor: HTMLElement) => void
  testId?: string
}

/** `.m-tree-item` as the kit draws it, plus drop and context-menu hooks (kit candidate). */
function TreeRow({ label, count, glyph, open, leaf, active, title, onClick, onDropNote, onContextMenu, testId }: RowProps) {
  const [over, setOver] = useState(false)
  return (
    <div
      className={cx('m-tree-item', open && 'is-open', leaf && 'is-leaf', active && 'is-active', over && 'nt-drop-over')}
      role="treeitem"
      aria-expanded={leaf ? undefined : !!open}
      aria-selected={!!active}
      tabIndex={0}
      title={title}
      data-testid={testId}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onClick()
        }
      }}
      onContextMenu={
        onContextMenu
          ? (e) => {
              e.preventDefault()
              onContextMenu(e.currentTarget)
            }
          : undefined
      }
      onDragOver={
        onDropNote
          ? (e: DragEvent) => {
              if (!dragState.row && !e.dataTransfer.types.includes(NOTE_DRAG)) return
              e.preventDefault()
              setOver(true)
            }
          : undefined
      }
      onDragLeave={() => setOver(false)}
      onDrop={
        onDropNote
          ? (e: DragEvent) => {
              e.preventDefault()
              setOver(false)
              const id = e.dataTransfer.getData(NOTE_DRAG) || dragState.row?.id
              if (id) onDropNote(id)
            }
          : undefined
      }
    >
      <span className="m-tree-chev" />
      {glyph !== undefined && <span className="m-glyph">{glyph}</span>}
      <span className="m-tree-label">{label}</span>
      {count !== undefined && <span className="m-tree-n">{count}</span>}
    </div>
  )
}

function byPosition(a: FolderRow, b: FolderRow) {
  return a.position - b.position || a.name.localeCompare(b.name)
}

export function NotesSidebar() {
  const folders = useNotesStore((s) => s.folders)
  const groupings = useNotesStore((s) => s.groupings)
  const expanded = useNotesStore((s) => s.expanded)
  const selection = useNotesStore((s) => s.selection)
  const vaultName = useNotesStore((s) => s.vaultName)
  const layout = useNotesStore((s) => s.layout)
  const total = folders.filter((f) => f.parent_id === null).reduce((n, f) => n + f.note_count, useNotesStore.getState().rootCount)
  const [menu, setMenu] = useState<{ anchor: HTMLElement; g: GroupingRow } | null>(null)
  const s = useNotesStore.getState

  const moveTo = (folderId: string | null) => (id: string) => void s().setField(id, { field: 'folder_id', value: folderId })

  function clickFolder(f: FolderRow, hasChildren: boolean) {
    const active = selection.kind === 'folder' && selection.id === f.id
    if (active && hasChildren) s().toggleFolder(f.id)
    else {
      if (hasChildren && !expanded.includes(f.id)) s().toggleFolder(f.id)
      void s().selectFolder(f.id)
    }
  }

  function branch(parent: string | null): ReactElement[] {
    return folders
      .filter((f) => f.parent_id === parent)
      .sort(byPosition)
      .flatMap((f) => {
        const kids = folders.some((c) => c.parent_id === f.id)
        const open = kids && expanded.includes(f.id)
        const row = (
          <TreeRow
            key={f.id}
            label={f.name}
            count={f.note_count}
            leaf={!kids}
            open={open}
            active={layout === 'list' && selection.kind === 'folder' && selection.id === f.id}
            title={f.kind === 'capture' ? `${f.name} · capture folder` : f.kind === 'templates' ? `${f.name} · templates` : f.path}
            onClick={() => clickFolder(f, kids)}
            onDropNote={moveTo(f.id)}
            testId={`folder-${f.path}`}
          />
        )
        return open
          ? [
              row,
              <div key={`${f.id}-children`} className="m-tree-children" role="group">
                {branch(f.id)}
              </div>,
            ]
          : [row]
      })
  }

  const views = groupings.filter((g) => g.show_in_sidebar).sort((a, b) => (a.kind === b.kind ? a.position - b.position : a.kind === 'premade' ? -1 : 1))

  return (
    <nav className="m-tree nt-tree" role="tree" aria-label="Folders and views" data-testid="notes-tree">
      <TreeRow
        label={vaultName}
        count={total}
        leaf
        active={layout === 'list' && selection.kind === 'folder' && selection.id === 'root'}
        title="The vault root"
        onClick={() => void s().selectFolder('root')}
        onDropNote={moveTo(null)}
        testId="folder-root"
      />
      {branch(null)}
      <GroupLabel>Views</GroupLabel>
      {views.map((g) => (
        <TreeRow
          key={g.id}
          label={g.name}
          count={g.count}
          glyph={g.glyph ?? '▣'}
          leaf
          active={layout === 'list' && selection.kind === 'grouping' && selection.slug === g.slug}
          onClick={() => {
            if (layout === 'board') void s().setLayout('list')
            void s().selectGrouping(g.slug)
          }}
          onContextMenu={(anchor) => setMenu({ anchor, g })}
          onDropNote={(id) => void s().dropOnGrouping(id, g.slug)}
          testId={`view-${g.slug}`}
        />
      ))}
      {menu && (
        <FloatMenu anchor={menu.anchor} onClose={() => setMenu(null)} label={menu.g.name}>
          <Item
            label="Edit view"
            onSelect={() => {
              setMenu(null)
              void call('app_open', { view: 'settings', filter: `notes.groupings:${menu.g.slug}` }).catch(() => {})
            }}
          />
          <Item
            label="Hide from sidebar"
            onSelect={() => {
              setMenu(null)
              void call('grouping_save', { grouping: { id: menu.g.id, name: menu.g.name, filter_json: menu.g.filter_json, show_in_sidebar: false } }).then(() => s().refresh())
            }}
          />
          <Item
            label="Show in overlay"
            current={menu.g.show_in_overlay}
            onSelect={() => {
              setMenu(null)
              void call('grouping_save', { grouping: { id: menu.g.id, name: menu.g.name, filter_json: menu.g.filter_json, show_in_overlay: !menu.g.show_in_overlay } }).then(() => s().refresh())
            }}
          />
        </FloatMenu>
      )}
    </nav>
  )
}
