// The Notes detail panel (03 §5.1, §5.8): Fields (status, urgent, when, tags,
// folder, pinned, archive), Links (AW-63, Add URL), History (AW-60: the
// revisions, a side-by-side diff against the current text, Restore) and the
// file info (path, rev, updated). Every field writes the frontmatter at once
// through the store (04 §9.2).
import { useEffect, useMemo, useState } from 'react'
import { Button } from '../kit/button'
import { Chip, Seg, SegItem, Switch } from '../kit/controls'
import { DetailPanel } from '../kit/layout'
import { GroupLabel } from '../kit/lists'
import type { NoteFieldEdit } from '../state/contract-notes'
import { useNotesStore } from '../state/notes-store'
import { call } from './api'
import { relativeTime } from './dates'
import { normTag } from './rules'

interface DiffRow {
  kind: 'same' | 'del' | 'add' | 'change'
  left: string | null
  right: string | null
}

/** A line diff (LCS) for the History side-by-side view. */
export function lineDiff(a: string, b: string): DiffRow[] {
  const x = a.split(/\r?\n/)
  const y = b.split(/\r?\n/)
  const n = x.length
  const m = y.length
  const lcs = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0))
  for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) lcs[i][j] = x[i] === y[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1])
  const out: DiffRow[] = []
  let i = 0
  let j = 0
  while (i < n || j < m) {
    if (i < n && j < m && x[i] === y[j]) {
      out.push({ kind: 'same', left: x[i++], right: y[j++] })
    } else if (j < m && (i >= n || lcs[i][j + 1] >= lcs[i + 1][j])) {
      const prev = out[out.length - 1]
      if (prev?.kind === 'del' && prev.right === null) {
        prev.kind = 'change'
        prev.right = y[j++]
      } else out.push({ kind: 'add', left: null, right: y[j++] })
    } else {
      out.push({ kind: 'del', left: x[i++], right: null })
    }
  }
  return out
}

function size(bytes: number): string {
  return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} kB`
}

export function NoteDetails() {
  const editor = useNotesStore((s) => s.editor)
  const statuses = useNotesStore((s) => s.statuses)
  const folders = useNotesStore((s) => s.folders)
  const tags = useNotesStore((s) => s.tags)
  const vaultName = useNotesStore((s) => s.vaultName)
  const history = useNotesStore((s) => s.history)
  const s = useNotesStore.getState
  const [tagText, setTagText] = useState('')
  const [url, setUrl] = useState('')
  const [rev, setRev] = useState<{ rev: number; content: string } | null>(null)
  const row = editor.detail?.row
  const id = editor.noteId

  useEffect(() => setRev(null), [id])

  const diff = useMemo(() => (rev ? lineDiff(rev.content, editor.text) : []), [rev, editor.text])

  if (!row || !id || !editor.detail) {
    return (
      <DetailPanel label="Note details" className="nt-details">
        <p className="nt-empty">No note open.</p>
      </DetailPanel>
    )
  }

  const set = (edit: NoteFieldEdit) => void s().setField(id, edit)
  const addTag = () => {
    const t = tagText.trim().replace(/^#/, '')
    if (!t) return
    setTagText('')
    if (!row.tags.some((x) => normTag(x) === normTag(t))) set({ field: 'tags', value: [...row.tags, t] })
  }

  return (
    <DetailPanel label="Note details" className="nt-details">
      <GroupLabel>Fields</GroupLabel>
      <div className="nt-fields">
        <span className="m-t-meta">Status</span>
        <Seg label="Status">
          {statuses.map((st) => (
            <SegItem key={st.slug} active={row.status === st.slug} onClick={() => set({ field: 'status', value: st.slug })}>
              {`${st.glyph} ${st.label}`}
            </SegItem>
          ))}
        </Seg>
        <span className="m-t-meta">Urgent</span>
        <Switch on={row.urgent} label="Urgent" onChange={(on) => set({ field: 'urgent', value: on })} />
        <span className="m-t-meta">When</span>
        <div className="m-inline">
          <Seg label="When">
            <SegItem active={row.when_bucket === 'today'} onClick={() => set({ field: 'when', value: 'today' })}>
              Today
            </SegItem>
            <SegItem active={row.when_bucket === 'tomorrow'} onClick={() => set({ field: 'when', value: 'tomorrow' })}>
              Tomorrow
            </SegItem>
            <SegItem active={!row.when_date} onClick={() => set({ field: 'when', value: null })}>
              None
            </SegItem>
          </Seg>
          <input type="date" className="m-input nt-date" aria-label="Date" value={row.when_date ?? ''} onChange={(e) => e.target.value && set({ field: 'when', value: e.target.value })} />
        </div>
        <span className="m-t-meta">Tags</span>
        <div className="m-inline">
          {row.tags.map((t) => (
            <Chip key={t} removable onRemove={() => set({ field: 'tags', value: row.tags.filter((x) => x !== t) })}>
              {`#${t}`}
            </Chip>
          ))}
          <input
            className="m-input nt-tag-input"
            list="nt-tag-names"
            aria-label="Add tag"
            placeholder="Add tag"
            value={tagText}
            onChange={(e) => setTagText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' || e.key === ',') {
                e.preventDefault()
                addTag()
              }
            }}
          />
          <datalist id="nt-tag-names">
            {tags.map((t) => (
              <option key={t.id} value={t.name} />
            ))}
          </datalist>
        </div>
        <span className="m-t-meta">Folder</span>
        <select className="m-input" aria-label="Folder" value={row.folder_id ?? ''} onChange={(e) => set({ field: 'folder_id', value: e.target.value || null })}>
          <option value="">{vaultName}</option>
          {folders
            .slice()
            .sort((a, b) => a.path.localeCompare(b.path))
            .map((f) => (
              <option key={f.id} value={f.id}>
                {f.path.split('/').join(' / ')}
              </option>
            ))}
        </select>
        <span className="m-t-meta">Pinned</span>
        <Switch on={row.pinned} label="Pinned" onChange={(on) => set({ field: 'pinned', value: on })} />
        <span className="m-t-meta">Archive</span>
        <div>
          <Button onClick={() => void s().archive(id, !row.archived_at)}>{row.archived_at ? 'Unarchive' : 'Archive'}</Button>
        </div>
      </div>

      <GroupLabel>Links</GroupLabel>
      <div className="nt-links">
        {editor.detail.links.length === 0 && <p className="nt-empty">No links.</p>}
        {editor.detail.links.map((l) => (
          <div key={l.id} className="nt-link-row">
            <span className="m-ellipsis">{l.kind === 'url' ? <a href={l.target_id} target="_blank" rel="noopener noreferrer">{l.title ?? l.label ?? l.target_id}</a> : l.available ? l.title ?? l.target_id : 'unavailable'}</span>
            <span className="m-t-meta">{l.kind === 'calendar_event' ? 'event' : l.kind}</span>
          </div>
        ))}
        <form
          className="m-inline"
          onSubmit={(e) => {
            e.preventDefault()
            void s().addUrl(url)
            setUrl('')
          }}
        >
          <input className="m-input" type="url" aria-label="Add URL" placeholder="Add URL" value={url} onChange={(e) => setUrl(e.target.value)} />
        </form>
      </div>

      <GroupLabel>History</GroupLabel>
      <div className="nt-history" data-testid="history">
        {history.length === 0 && <p className="nt-empty">No history yet.</p>}
        {history.map((h) => (
          <div
            key={`${h.rev}-${h.created_at}`}
            className="nt-link-row nt-rev"
            role="button"
            tabIndex={0}
            aria-pressed={rev?.rev === h.rev}
            onClick={() =>
              void call('note_revision_get', { note_id: id, rev: h.rev })
                .then(setRev)
                .catch(() => s().showNotice('This revision is no longer kept.'))
            }
          >
            <span className="m-ellipsis">{`rev ${h.rev} · ${h.origin}`}</span>
            <span className="m-t-meta">{`${relativeTime(h.created_at)} · ${size(h.bytes)}`}</span>
          </div>
        ))}
        {rev && (
          <>
            <div className="nt-diff" aria-label={`Revision ${rev.rev} against the current text`}>
              {diff.map((d, i) => (
                <div key={i} className={`nt-diff-row is-${d.kind}`}>
                  <code>{d.left ?? ''}</code>
                  <code>{d.right ?? ''}</code>
                </div>
              ))}
            </div>
            <div className="m-inline">
              <Button onClick={() => void s().restore(rev.rev).then(() => setRev(null))}>Restore</Button>
              <Button onClick={() => setRev(null)}>Close</Button>
            </div>
          </>
        )}
      </div>

      <GroupLabel>File</GroupLabel>
      <div className="nt-file">
        <code className="m-ellipsis" title={row.path}>
          {row.path}
        </code>
        <span className="m-t-meta">{`rev ${editor.detail.rev} · updated ${relativeTime(row.updated_at)}`}</span>
      </div>
    </DetailPanel>
  )
}
