// The editor column (03 §5.4, 04 §9; app-window.html #n-editor): the title
// with ⋮, the header strip (status, urgent, when, tags as chips; the folder
// path and the save state), the banners (conflict 04 §11.4, failed write,
// deleted outside Mode, read-only, unreadable frontmatter, unknown status,
// vault missing), then the CodeMirror body.
import { lazy, Suspense, useMemo, useState, type ReactNode } from 'react'
import { Button, MenuButton } from '../kit/button'
import type { NoteFieldEdit } from '../state/contract-notes'
import { useNotesStore } from '../state/notes-store'
import { call } from './api'
import { addDays, shortDate, weekdayName, whenLabel } from './dates'
import { bufferTitle } from './editor/markdown'
import { rowGlyph, statusGlyph, statusLabel } from './rules'
import { FloatMenu, Item, Sep, useMenu } from './widgets'

/** CodeMirror and the Markdown grammar load with the first open note, not with the view. */
const NoteEditor = lazy(() => import('./editor/NoteEditor').then((m) => ({ default: m.NoteEditor })))

function StripChip({ children, label, onClick }: { children: ReactNode; label: string; onClick: (el: HTMLElement) => void }) {
  return (
    <span
      className="m-chip nt-chip-btn"
      role="button"
      tabIndex={0}
      aria-label={label}
      onClick={(e) => onClick(e.currentTarget)}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault()
          onClick(e.currentTarget)
        }
      }}
    >
      {children}
    </span>
  )
}

function Banner({ children, testId }: { children: ReactNode; testId?: string }) {
  return (
    <div className="nt-banner" role="alert" data-testid={testId}>
      {children}
    </div>
  )
}

type Page = 'status' | 'when' | 'main' | 'move'

const SAVE_WORD = {
  clean: 'saved',
  saved: 'saved',
  dirty: 'saving…',
  saving: 'saving…',
  failed: 'not saved',
} as const

function openLink(href: string) {
  const s = useNotesStore.getState()
  const m = /^mode:\/\/note\/(.+)$/.exec(href)
  if (m) void s.selectNote(m[1])
  else if (/^https?:\/\//.test(href)) window.open(href, '_blank', 'noopener')
}

export function EditorPanel() {
  const editor = useNotesStore((s) => s.editor)
  const statuses = useNotesStore((s) => s.statuses)
  const today = useNotesStore((s) => s.today)
  const folders = useNotesStore((s) => s.folders)
  const vaultName = useNotesStore((s) => s.vaultName)
  const health = useNotesStore((s) => s.health)
  const detailOpen = useNotesStore((s) => s.detailOpen)
  const rows = useNotesStore((s) => s.rows)
  const menu = useMenu<Page>()
  const [date, setDate] = useState('')
  const s = useNotesStore.getState

  const titles = useMemo(() => new Set(rows.map((r) => r.title.toLowerCase())), [rows])

  if (!editor.noteId && !editor.draft) {
    return (
      <div className="nt-panel nt-editor-panel is-empty" data-testid="editor-panel">
        <p className="nt-empty">No note open.</p>
      </div>
    )
  }

  const d = editor.detail
  const row = d?.row
  const id = editor.noteId
  const set = (edit: NoteFieldEdit) => {
    menu.close()
    if (id) void s().setField(id, edit)
  }
  const folderPath = d ? d.folder_path : (folders.find((f) => f.id === editor.draft?.folder_id)?.path ?? '')
  const where = folderPath ? folderPath.split('/').join(' / ') : vaultName
  const readOnly = !!d?.read_only || !!health?.vault_missing || !!health?.read_only
  const title = bufferTitle(editor.text) ?? row?.title ?? ''
  const when = row ? whenLabel(row.when_bucket, row.when_date, today) : null

  return (
    <div className="nt-panel nt-editor-panel" data-testid="editor-panel">
      <div className="nt-editor-head">
        <input
          className="m-t-title nt-title-input"
          aria-label="Title"
          placeholder="Untitled"
          value={title}
          readOnly={readOnly}
          autoFocus={!!editor.draft}
          onChange={(e) => s().setTitle(e.target.value)}
          onBlur={() => void s().saveNow()}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === 'ArrowDown') {
              e.preventDefault()
              document.querySelector<HTMLElement>('[data-testid="note-editor"] .cm-content')?.focus()
            }
          }}
          data-testid="editor-title"
        />
        <MenuButton onClick={(e) => (menu.open ? menu.close() : menu.show(e.currentTarget, 'main'))} />
      </div>
      <div className="m-inline nt-strip" data-testid="editor-strip">
        {row ? (
          <>
            <StripChip label="Status" onClick={(el) => menu.show(el, 'status')}>
              {`${rowGlyph({ status: row.status, urgent: false }, statuses)} ${statusLabel(row.status, statuses)}`}
            </StripChip>
            {row.urgent && (
              <StripChip label="Urgent, clear" onClick={() => set({ field: 'urgent', value: false })}>
                ✱ Urgent
              </StripChip>
            )}
            {when && (
              <StripChip label="When" onClick={(el) => menu.show(el, 'when')}>
                {`▦ ${when.charAt(0).toUpperCase() + when.slice(1)}`}
              </StripChip>
            )}
            {row.tags.map((t) => (
              <StripChip key={t} label={`Tag ${t}`} onClick={() => void s().tagToBar(t)}>
                {`#${t}`}
              </StripChip>
            ))}
          </>
        ) : null}
        <span className="m-t-meta" data-testid="save-state">
          {`${where} · ${editor.draft && !id ? 'new' : SAVE_WORD[editor.save]}`}
        </span>
      </div>
      {health?.vault_missing && (
        <Banner>
          <span>The notes folder is not available. Notes will reappear when it is.</span>
          <Button onClick={() => void call('app_open', { view: 'settings' }).catch(() => {})}>Choose folder</Button>
        </Banner>
      )}
      {editor.conflict && (
        <Banner testId="conflict-banner">
          <span>This note changed on disk while you were editing. Keep yours, keep theirs, or keep both.</span>
          <Button onClick={() => void s().resolveConflict('mine')}>Mine</Button>
          <Button onClick={() => void s().resolveConflict('theirs')}>Theirs</Button>
          <Button onClick={() => void s().resolveConflict('both')}>Both</Button>
        </Banner>
      )}
      {editor.save === 'failed' && (
        <Banner testId="save-error">
          <span>{`Couldn't save · ${editor.error ?? ''}`}</span>
          <Button onClick={() => void s().retrySave()}>Retry</Button>
        </Banner>
      )}
      {editor.gone && (
        <Banner>
          <span>This file was deleted outside Mode.</span>
          <Button onClick={() => void s().retrySave()}>Save again</Button>
          <Button onClick={() => void s().selectNote(null)}>Close</Button>
        </Banner>
      )}
      {d?.read_only && <Banner>{d.read_only_reason ?? 'Read-only'}</Banner>}
      {d?.frontmatter_error && (
        <Banner>
          <span>Frontmatter could not be read.</span>
          <Button onClick={() => s().setRaw(true)}>Fix in raw source</Button>
        </Banner>
      )}
      {d?.unknown_status && row && (
        <Banner>
          <span>{`'${row.status}' is not a status yet.`}</span>
          <Button onClick={() => void call('app_open', { view: 'settings' }).catch(() => {})}>Add as status</Button>
        </Banner>
      )}
      <div className="nt-editor-body">
        <Suspense fallback={<div className="nt-cm" data-testid="note-editor-loading" />}>
          <NoteEditor
            docKey={id ?? 'draft'}
            text={editor.text}
            version={editor.version}
            readOnly={readOnly}
            raw={editor.raw}
            label={`${title || 'Untitled'}, body`}
            resolveWiki={(t) => titles.has(t.toLowerCase())}
            onInput={(t) => s().editorInput(t)}
            onBlur={() => void s().saveNow()}
            onSave={() => void s().saveNow()}
            onToggleRaw={() => s().setRaw(!s().editor.raw)}
            onCommand={(c) => {
              const r = s().editor.detail?.row
              if (!id || !r) return
              if (c === 'urgent') void s().setField(id, { field: 'urgent', value: !r.urgent })
              else if (c === 'today') void s().setField(id, { field: 'when', value: 'today' })
              else void s().setField(id, { field: 'status', value: 'done' })
            }}
            onEscape={() => document.querySelector<HTMLElement>('[data-testid="note-list"]')?.focus()}
            onOpenLink={openLink}
            onOpenWiki={(t) => {
              const hit = s().rows.find((r) => r.title.toLowerCase() === t.toLowerCase())
              if (hit) void s().selectNote(hit.id)
              else
                s().showNotice(`No note named ${t}`, {
                  label: 'Create note',
                  run: () =>
                    void s()
                      .newNote()
                      .then(() => s().setTitle(t)),
                })
            }}
          />
        </Suspense>
      </div>
      {menu.open && menu.anchor && (
        <FloatMenu
          anchor={menu.anchor}
          onClose={menu.close}
          label={menu.page === 'status' ? 'Status' : menu.page === 'when' ? 'When' : 'Note'}
          align={menu.page === 'main' ? 'right' : 'left'}
          testId={`editor-menu-${menu.page}`}
        >
          {menu.page === 'status' &&
            statuses.map((st) => (
              <Item key={st.slug} glyph={statusGlyph(st.slug, statuses)} label={st.label} current={row?.status === st.slug} onSelect={() => set({ field: 'status', value: st.slug })} />
            ))}
          {menu.page === 'when' && (
            <>
              <Item glyph="▦" label="Today" hint={shortDate(today)} current={row?.when_bucket === 'today'} onSelect={() => set({ field: 'when', value: 'today' })} />
              <Item glyph="▦" label="Tomorrow" hint={shortDate(addDays(today, 1))} current={row?.when_bucket === 'tomorrow'} onSelect={() => set({ field: 'when', value: 'tomorrow' })} />
              {[2, 3, 4, 5, 6].map((n) => {
                const day = addDays(today, n)
                return <Item key={n} glyph="" label={weekdayName(day)} hint={shortDate(day)} current={row?.when_date === day} onSelect={() => set({ field: 'when', value: day })} />
              })}
              <div className="m-menu-item nt-menu-input">
                <input
                  type="date"
                  className="m-input"
                  aria-label="Pick a date"
                  value={date}
                  onChange={(e) => setDate(e.target.value)}
                  onKeyDown={(e) => e.key === 'Enter' && date && set({ field: 'when', value: date })}
                />
                <Button disabled={!date} onClick={() => set({ field: 'when', value: date })}>
                  Set
                </Button>
              </div>
              <Sep />
              <Item label="None" current={!row?.when_date} onSelect={() => set({ field: 'when', value: null })} />
            </>
          )}
          {menu.page === 'move' && (
            <>
              <Item label={vaultName} current={row?.folder_id === null} onSelect={() => set({ field: 'folder_id', value: null })} />
              {folders
                .slice()
                .sort((a, b) => a.path.localeCompare(b.path))
                .map((f) => (
                  <Item key={f.id} label={f.path.split('/').join(' / ')} current={row?.folder_id === f.id} onSelect={() => set({ field: 'folder_id', value: f.id })} />
                ))}
            </>
          )}
          {menu.page === 'main' && row && id && (
            <>
              <Item label={row.urgent ? 'Clear urgent' : 'Mark urgent'} onSelect={() => set({ field: 'urgent', value: !row.urgent })} />
              <Item label="Set date" hint="›" onSelect={() => menu.go('when')} />
              <Item label="Set status" hint="›" onSelect={() => menu.go('status')} />
              <Item label="Move to folder" hint="›" onSelect={() => menu.go('move')} />
              <Item label={row.pinned ? 'Unpin' : 'Pin'} onSelect={() => set({ field: 'pinned', value: !row.pinned })} />
              <Item
                label={row.archived_at ? 'Unarchive' : 'Archive'}
                onSelect={() => {
                  menu.close()
                  void s().archive(id, !row.archived_at)
                }}
              />
              <Sep />
              <Item
                label={detailOpen ? 'Hide details' : 'Show details'}
                onSelect={() => {
                  menu.close()
                  s().setDetailOpen(!detailOpen)
                }}
              />
              <Item
                label={editor.raw ? 'Live preview' : 'Raw source'}
                onSelect={() => {
                  menu.close()
                  s().setRaw(!editor.raw)
                }}
              />
              <Item
                label="Copy as Markdown"
                onSelect={() => {
                  menu.close()
                  void navigator.clipboard?.writeText(editor.text).catch(() => {})
                  s().showNotice('Copied')
                }}
              />
              <Sep />
              <Item
                label="Delete"
                danger
                onSelect={() => {
                  menu.close()
                  void s().trash(id)
                }}
              />
            </>
          )}
        </FloatMenu>
      )}
    </div>
  )
}
