// Settings → Notes (03 §8.7) with the Groupings editor (§8.8), custom
// statuses and Tags (§8.9) as sections. The vault folder is chosen through
// `vault_pick` (the dialog plugin) or typed; groupings, folders and tags come
// from the notes commands (contract-notes.ts) plus the reorder / delete /
// rename / merge commands proposed in contract-settings.ts.
import { useCallback, useEffect, useRef, useState } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { Seg, SegItem, Select, Switch } from '../../kit/controls'
import { SettingError } from '../../kit/settings'
import { invoke } from '../../tauri'
import type { FoldersList, GroupingDraft, GroupingRow, GroupingsList, StatusDef, TagRow } from '../../state/contract-notes'
import { DEFAULT_STATUSES } from '../../state/contract-notes'
import type { BatchReport, GroupingDraftWithPosition, ReindexResult, VaultInspect, VaultPick } from '../../state/contract-settings'
import { errorText, useSetting, useSettingsStore, validateSetting } from '../../state/settings-store'
import { useSessionsStore } from '../../state/sessions-store'
import { Dialog, openMenu } from '../ui'
import { tilde } from '../integrations/model'
import { Block, CommitInput, Field, FieldError, Group, Note, SettingNumber, SettingSeg, SettingSwitch } from './controls'
import { addStatus, FILTER_KEYS, filterToRows, FIXED_STATUSES, moveId, moveStatus, OPS, patchStatus, removeStatus, rowsProblem, rowsToFilter, type FilterRow } from './groupings'

const notice = (text: string) => useSessionsStore.getState().showNotice(text)

/** "Reindexed · 27 files · 2 changed" (+ problems, when the scan reported any). */
export function reindexText(r: ReindexResult | null | undefined): string {
  if (!r) return 'Reindexed'
  const parts = [`Reindexed · ${r.files} files`]
  if (r.changed) parts.push(`${r.changed} changed`)
  if (r.tombstoned) parts.push(`${r.tombstoned} removed`)
  if (r.diagnostics?.length) parts.push(`${r.diagnostics.length} ${r.diagnostics.length === 1 ? 'problem' : 'problems'}`)
  return parts.join(' · ')
}

export function NotesPage() {
  const [folders, setFolders] = useState<FoldersList | null>(null)
  const reloadFolders = useCallback(() => {
    invoke<FoldersList>('folders_list').then(setFolders, () => setFolders(null))
  }, [])
  useEffect(reloadFolders, [reloadFolders])
  return (
    <>
      <Group title="Vault" id="vault">
        <VaultField />
        <FolderKindField folders={folders} kind="capture" label="Capture folder" onChange={setFolders} />
        <FolderKindField folders={folders} kind="templates" label="Templates folder" onChange={setFolders} />
        <SettingSwitch k="notes.touchup_enabled" title="Touch-up writes" desc="Fill missing frontmatter fields." />
        <SettingNumber k="notes.touchup_delay_seconds" title="Touch-up after" desc="Waits this long after the last edit." unit="seconds" />
        <SettingSwitch k="notes.rename_with_title" title="Rename files" desc="Rename the file when the title changes." />
        <SettingSeg
          k="notes.week_start"
          title="Week starts on"
          options={[
            ['monday', 'Monday'],
            ['sunday', 'Sunday'],
            ['saturday', 'Saturday'],
          ]}
        />
        <Field title="Index" desc="A full scan and a search rebuild.">
          <Button
            onClick={() =>
              void invoke<ReindexResult>('notes_reindex').then(
                (r) => notice(reindexText(r)),
                (e) => notice(`Reindex failed: ${errorText(e)}`),
              )
            }
          >
            Reindex vault
          </Button>
        </Field>
      </Group>
      <GroupingsSection />
      <StatusesSection />
      <TagsSection />
    </>
  )
}

/* ---- vault folder ------------------------------------------------------------ */

/** The hint under a typed notes folder, from `vault_inspect` (its `path` is the expanded one, `~` resolved). */
export function inspectHint(r: VaultInspect | null | undefined): string | null {
  if (!r) return null
  if (!r.exists) return `${tilde(r.path)} does not exist yet`
  return r.md_files ? `${r.md_files} Markdown files in ${tilde(r.path)}` : `no Markdown files in ${tilde(r.path)} yet`
}

function VaultField() {
  const [path, setPath] = useSetting<string>('notes.vault_path')
  const inputRef = useRef<HTMLInputElement>(null)
  const [hint, setHint] = useState<string | null>(null)
  const choose = async () => {
    try {
      const r = await invoke<VaultPick>('vault_pick', { current: path })
      if (r?.path) {
        await setPath(r.path)
        setHint(r.md_files ? `${r.md_files} Markdown files will be indexed` : null)
      }
    } catch {
      // No folder dialog: the typed path is the fallback.
      setHint('Type the folder path')
      inputRef.current?.focus()
    }
  }
  // A typed folder: stored, then inspected for the hint (how many notes, or that it is not there yet).
  const commit = async (typed: string) => {
    if (!(await setPath(typed))) return
    try {
      setHint(inspectHint(await invoke<VaultInspect>('vault_inspect', { path: typed })))
    } catch {
      setHint(null)
    }
  }
  return (
    <Field title="Notes folder" desc={hint ?? 'Where your Markdown notes live; changing it re-indexes.'} k="notes.vault_path" stack>
      <div className="m-inline sg-line">
        <CommitInput label="Notes folder" value={path} inputRef={inputRef} onCommit={(v) => void commit(String(v).trim())} testId="vault-path" />
        <Button onClick={() => void choose()}>Choose…</Button>
      </div>
    </Field>
  )
}

/**
 * The capture and templates folders (03 §8.7) through `notes_folders_set`: a
 * folder inside the notes folder, typed vault-relative (`Inbox`, `Work/Notes`)
 * or picked from the indexed folders; an empty templates folder clears it.
 * The core answers the folders list, which replaces the page's.
 */
function FolderKindField({ folders, kind, label, onChange }: { folders: FoldersList | null; kind: 'capture' | 'templates'; label: string; onChange: (f: FoldersList) => void }) {
  const current = folders?.folders.find((f) => f.kind === kind)
  const [error, setError] = useState<string | null>(null)
  const write = async (raw: string) => {
    const value = raw.trim().replace(/\/+$/, '')
    if (kind === 'capture' && !value) return
    try {
      const list = await invoke<FoldersList>('notes_folders_set', {
        [kind]: value,
      })
      if (list) onChange(list)
      setError(null)
      notice(value ? `${label}: ${value}/` : `${label} cleared`)
    } catch (e) {
      setError(errorText(e))
    }
  }
  return (
    <Field
      title={label}
      desc={folders ? (kind === 'capture' ? 'Quick captures land here, inside the notes folder.' : 'New notes can start from these; inside the notes folder.') : 'The notes folder is not indexed yet.'}
      k={`folders.${kind}`}
      error={error ? <SettingError testId={`error-folders.${kind}`}>{error}</SettingError> : undefined}
    >
      <CommitInput
        label={label}
        value={current ? current.path : ''}
        placeholder={kind === 'templates' ? 'none' : 'Unsorted'}
        disabled={!folders}
        onCommit={(v) => void write(String(v))}
        suggestions={(folders?.folders ?? []).map((f) => f.path)}
        testId={`folder-${kind}`}
      />
    </Field>
  )
}

/* ---- groupings (03 §8.8) ---------------------------------------------------- */

const GLYPHS = ['○', '◑', '✱', '✓', '▦', '#'] as const

function GroupingsSection() {
  const [data, setData] = useState<GroupingsList | null>(null)
  const [failed, setFailed] = useState<string | null>(null)
  const [editing, setEditing] = useState<GroupingDraft | null>(null)
  const reload = useCallback(() => {
    invoke<GroupingsList>('groupings_list').then(
      (d) => {
        setData(d)
        setFailed(null)
      },
      (e) => setFailed(errorText(e)),
    )
  }, [])
  useEffect(reload, [reload])

  const save = async (draft: GroupingDraft) => {
    try {
      await invoke<GroupingRow>('grouping_save', { grouping: draft })
      setEditing(null)
      reload()
    } catch (e) {
      notice(`Couldn't save the grouping: ${errorText(e)}`)
    }
  }
  const rows = (data?.groupings ?? []).slice().sort((a, b) => a.position - b.position)
  // Reorder through `grouping_save` with `position` (03 §8.8): the two rows swap places.
  const move = async (id: string, by: -1 | 1) => {
    const order = moveId(
      rows.map((r) => r.id),
      id,
      by,
    )
    try {
      for (const [position, gid] of order.entries()) {
        const g = rows.find((r) => r.id === gid)!
        if (g.position === position) continue
        const draft: GroupingDraftWithPosition = {
          id: g.id,
          name: g.name,
          filter_json: g.filter_json,
          position,
        }
        await invoke<GroupingRow>('grouping_save', { grouping: draft })
      }
    } catch (e) {
      notice(errorText(e))
    }
    reload()
  }

  return (
    <Group title="Groupings" id="groupings">
      {failed && (
        <div className="ap-banner" role="status">
          <span>Groupings could not be read.</span>
          <Button quiet onClick={reload}>
            Retry
          </Button>
        </div>
      )}
      <Block>
        <div className="m-list sg-list" role="list" aria-label="Groupings">
          {rows.map((g, i) => (
            <div key={g.id} className="sg-grow" role="listitem" data-testid={`grouping-${g.slug}`}>
              <span className="sg-glyph">{g.glyph ?? '·'}</span>
              <span className="sg-grow-name">{g.name}</span>
              <span className="m-t-meta">{g.kind === 'premade' ? 'premade' : ''}</span>
              <span className="m-t-meta sg-num-col">{g.count}</span>
              <span className="sg-sw-cell">
                <Switch
                  on={g.show_in_sidebar}
                  label={`${g.name} in the sidebar`}
                  onChange={(on) =>
                    void save({
                      id: g.id,
                      name: g.name,
                      filter_json: g.filter_json,
                      show_in_sidebar: on,
                    })
                  }
                />
                <span className="m-t-meta">Sidebar</span>
              </span>
              <span className="sg-sw-cell">
                <Switch
                  on={g.show_in_overlay}
                  label={`${g.name} in the overlay`}
                  onChange={(on) =>
                    void save({
                      id: g.id,
                      name: g.name,
                      filter_json: g.filter_json,
                      show_in_overlay: on,
                    })
                  }
                />
                <span className="m-t-meta">Overlay</span>
              </span>
              <MenuButton
                label={`${g.name} options`}
                onClick={(e) =>
                  openMenu(
                    e.currentTarget,
                    [
                      {
                        id: 'edit',
                        label: 'Edit',
                        run: () =>
                          setEditing({
                            id: g.id,
                            name: g.name,
                            glyph: g.glyph,
                            color: g.color,
                            filter_json: g.filter_json,
                            sort_json: g.sort_json,
                          }),
                      },
                      {
                        id: 'up',
                        label: 'Move up',
                        dimmed: i === 0,
                        run: () => void move(g.id, -1),
                      },
                      {
                        id: 'down',
                        label: 'Move down',
                        dimmed: i === rows.length - 1,
                        run: () => void move(g.id, 1),
                      },
                      { kind: 'sep', id: 'sep' },
                      g.kind === 'premade'
                        ? {
                            id: 'hide',
                            label: g.show_in_sidebar ? 'Hide' : 'Show',
                            run: () =>
                              void save({
                                id: g.id,
                                name: g.name,
                                filter_json: g.filter_json,
                                show_in_sidebar: !g.show_in_sidebar,
                              }),
                          }
                        : {
                            id: 'delete',
                            label: 'Delete',
                            danger: true,
                            run: () => void invoke('grouping_delete', { id: g.id }).then(reload, (e) => notice(errorText(e))),
                          },
                    ],
                    `${g.name} options`,
                  )
                }
              />
            </div>
          ))}
        </div>
      </Block>
      {editing ? (
        <Block>
          <GroupingEditor draft={editing} premade={rows.find((r) => r.id === editing.id)?.kind === 'premade'} statuses={data?.statuses ?? [...DEFAULT_STATUSES]} onCancel={() => setEditing(null)} onSave={save} />
        </Block>
      ) : (
        <Field title="New grouping" desc="A saved filter for the sidebar, the overlay or the board.">
          <Button
            onClick={() =>
              setEditing({
                name: '',
                glyph: '○',
                filter_json: {},
                show_in_sidebar: true,
                show_in_overlay: false,
              })
            }
          >
            New grouping
          </Button>
        </Field>
      )}
    </Group>
  )
}

function GroupingEditor({ draft, premade, statuses, onCancel, onSave }: { draft: GroupingDraft; premade: boolean; statuses: StatusDef[]; onCancel: () => void; onSave: (d: GroupingDraft) => void }) {
  const [name, setName] = useState(draft.name)
  const [glyph, setGlyph] = useState(draft.glyph ?? '○')
  const [rows, setRows] = useState<FilterRow[]>(() => filterToRows(draft.filter_json))
  const problem = rowsProblem(rows, statuses) ?? (name.trim() ? null : 'Name the grouping.')
  const patch = (i: number, p: Partial<FilterRow>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...p } : r)))
  return (
    <div className="sg-editor" role="group" aria-label={draft.id ? `Edit ${draft.name}` : 'New grouping'} data-testid="grouping-editor">
      <Group compact>
        <Field title="Name">
          <input className="m-input" aria-label="Grouping name" value={name} disabled={premade} onChange={(e) => setName(e.target.value)} />
        </Field>
        <Field title="Glyph">
          <Seg label="Glyph">
            {GLYPHS.map((g) => (
              <SegItem key={g} active={glyph === g} onClick={() => setGlyph(g)}>
                {g}
              </SegItem>
            ))}
          </Seg>
        </Field>
        <Field title="Filter" desc="Notes that match every condition." stack error={problem && rows.length > 0 ? <SettingError>{problem}</SettingError> : undefined}>
          {rows.map((r, i) => (
            <div key={i} className="m-inline sg-line">
              <Select label="Field" value={r.key} options={FILTER_KEYS.map((k) => [k, k.replace('_', ' ')] as const)} onChange={(key) => patch(i, { key, op: OPS[key][0]!, value: '' })} />
              <Select label="Operator" value={r.op} options={OPS[r.key].map((o) => [o, o] as const)} onChange={(op) => patch(i, { op })} />
              <input className="m-input" aria-label="Value" value={r.value} placeholder={placeholderFor(r)} onChange={(e) => patch(i, { value: e.target.value })} />
              <Button quiet aria-label="Remove condition" onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}>
                ×
              </Button>
            </div>
          ))}
          <div className="m-inline">
            <Button onClick={() => setRows((rs) => [...rs, { key: 'status', op: 'is any of', value: '' }])}>Add condition</Button>
          </div>
        </Field>
      </Group>
      <div className="m-card-actions">
        <Button onClick={onCancel}>Cancel</Button>
        <Button
          variant="primary"
          disabled={!!problem}
          onClick={() =>
            onSave({
              ...draft,
              name: name.trim(),
              glyph,
              filter_json: rowsToFilter(rows),
            })
          }
        >
          Save grouping
        </Button>
      </div>
    </div>
  )
}

function placeholderFor(r: FilterRow): string {
  switch (r.key) {
    case 'status':
      return 'todo, doing'
    case 'when':
      return r.op === 'between' ? '2026-10-01..2026-10-31' : 'today, tomorrow'
    case 'tags':
      return 'groceries, gym'
    case 'urgent':
    case 'pinned':
    case 'archived':
      return 'true'
    case 'folder':
      return 'folder id or root'
    default:
      return ''
  }
}

/* ---- custom statuses (`notes.statuses`) ---------------------------------------- */

function StatusesSection() {
  const [list] = useSetting<StatusDef[]>('notes.statuses')
  const statuses = Array.isArray(list) && list.length ? list : [...DEFAULT_STATUSES]
  /** `statuses_save` validates and stores `notes.statuses`; the store keeps the list the core returns. */
  const setList = async (next: StatusDef[]): Promise<boolean> => {
    const why = validateSetting('notes.statuses', next)
    if (why) {
      useSettingsStore.setState((st) => ({
        errors: { ...st.errors, 'notes.statuses': why },
      }))
      return false
    }
    try {
      const saved = await invoke<StatusDef[]>('statuses_save', {
        statuses: next,
      })
      useSettingsStore.setState((st) => {
        const errors = { ...st.errors }
        delete errors['notes.statuses']
        return {
          values: {
            ...st.values,
            'notes.statuses': Array.isArray(saved) ? saved : next,
          },
          errors,
        }
      })
      return true
    } catch (e) {
      useSettingsStore.setState((st) => ({
        errors: { ...st.errors, 'notes.statuses': errorText(e) },
      }))
      return false
    }
  }
  const [newLabel, setNewLabel] = useState('')
  const [deleting, setDeleting] = useState<StatusDef | null>(null)
  return (
    <Group title="Custom statuses" id="statuses">
      <Block>
        <div className="m-list sg-list" role="list" aria-label="Statuses" data-setting="notes.statuses">
          {statuses.map((s, i) => (
            <div key={s.slug} className="sg-grow" role="listitem">
              <CommitInput className="sg-glyph-in" label={`${s.label} glyph`} value={s.glyph} onCommit={(v) => void setList(patchStatus(statuses, s.slug, { glyph: String(v).trim() }))} />
              <CommitInput className="sg-text" label={`${s.label} label`} value={s.label} onCommit={(v) => void setList(patchStatus(statuses, s.slug, { label: String(v).trim() }))} />
              <code className="m-t-meta">{s.slug}</code>
              <span className="m-t-meta">{s.closed ? 'closed' : ''}</span>
              <MenuButton
                label={`${s.label} options`}
                onClick={(e) =>
                  openMenu(
                    e.currentTarget,
                    [
                      {
                        id: 'up',
                        label: 'Move up',
                        dimmed: i === 0,
                        run: () => void setList(moveStatus(statuses, s.slug, -1)),
                      },
                      {
                        id: 'down',
                        label: 'Move down',
                        dimmed: i === statuses.length - 1,
                        run: () => void setList(moveStatus(statuses, s.slug, 1)),
                      },
                      { kind: 'sep', id: 'sep' },
                      {
                        id: 'delete',
                        label: 'Delete',
                        danger: true,
                        dimmed: FIXED_STATUSES.includes(s.slug),
                        reason: FIXED_STATUSES.includes(s.slug) ? 'built in' : undefined,
                        run: () => setDeleting(s),
                      },
                    ],
                    `${s.label} options`,
                  )
                }
              />
            </div>
          ))}
        </div>
      </Block>
      <Field title="Add status" desc="Todo, Doing and Done are built in." error={<FieldError k="notes.statuses" />}>
        <input className="m-input" aria-label="New status" placeholder="Waiting on others" value={newLabel} onChange={(e) => setNewLabel(e.target.value)} />
        <Button
          disabled={!newLabel.trim()}
          onClick={() => {
            void setList(addStatus(statuses, newLabel))
            setNewLabel('')
          }}
        >
          Add status
        </Button>
      </Field>
      {deleting && (
        <DeleteStatus
          status={deleting}
          others={statuses.filter((s) => s.slug !== deleting.slug)}
          onClose={() => setDeleting(null)}
          onDone={async (to) => {
            try {
              const r = await invoke<BatchReport>('status_move', {
                from: deleting.slug,
                to,
              })
              if (r?.stopped) throw new Error(`stopped at ${r.stopped[0]}: ${r.stopped[1]}`)
              if (await setList(removeStatus(statuses, deleting.slug))) notice(`Deleted ${deleting.label} · ${r?.written ?? 0} notes moved`)
            } catch (e) {
              notice(`Couldn't delete ${deleting.label}: ${errorText(e)}`)
            }
            setDeleting(null)
          }}
        />
      )}
    </Group>
  )
}

function DeleteStatus({ status, others, onClose, onDone }: { status: StatusDef; others: StatusDef[]; onClose: () => void; onDone: (to: string) => void }) {
  const [to, setTo] = useState(others[0]?.slug ?? 'todo')
  return (
    <Dialog title={`Delete ${status.label}`} onClose={onClose} testId="delete-status">
      <Group compact>
        <Field title="Move its notes to">
          <Select label="Move its notes to" value={to} options={others.map((s) => [s.slug, `${s.glyph} ${s.label}`] as const)} onChange={setTo} />
        </Field>
      </Group>
      <div className="m-card-actions">
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="primary" onClick={() => onDone(to)}>
          Delete
        </Button>
      </div>
    </Dialog>
  )
}

/* ---- tags (03 §8.9) ------------------------------------------------------------ */

function TagsSection() {
  const [tags, setTags] = useState<TagRow[] | null>(null)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [confirm, setConfirm] = useState<TagRow | null>(null)
  const reload = useCallback(() => {
    invoke<TagRow[]>('tags_list').then(
      (t) => setTags(Array.isArray(t) ? t : []),
      () => setTags([]),
    )
  }, [])
  useEffect(reload, [reload])
  // Renames and deletes rewrite every note with the tag (`BatchReport`); a stopped batch names the file.
  const run = (cmd: 'tag_rename' | 'tag_delete', args: Record<string, unknown>, done: string) =>
    void invoke<BatchReport>(cmd, args).then(
      (r) => {
        notice(r?.stopped ? `Stopped at ${r.stopped[0]}: ${r.stopped[1]}` : done)
        reload()
      },
      (e) => notice(errorText(e)),
    )
  return (
    <Group title="Tags" id="tags">
      {tags && !tags.length && <Note>No tags yet. Type # in a note to add one.</Note>}
      {tags && tags.length > 0 && (
        <Block>
          <div className="m-list sg-list" role="list" aria-label="Tags">
            {(tags ?? []).map((t) => (
              <div key={t.id} className="sg-grow" role="listitem" data-testid={`tag-${t.name_norm}`}>
                <span className="sg-glyph">#</span>
                {renaming === t.id ? (
                  <CommitInput
                    className="sg-text"
                    label={`Rename ${t.name}`}
                    value={t.name}
                    onCommit={(v) => {
                      setRenaming(null)
                      const name = String(v).trim().replace(/^#/, '')
                      if (name && name !== t.name) run('tag_rename', { from: t.name, to: name }, `Renamed to #${name}`)
                    }}
                  />
                ) : (
                  <span className="sg-grow-name">{t.name}</span>
                )}
                <span className="m-t-meta sg-num-col">{t.count === 1 ? '1 note' : `${t.count} notes`}</span>
                <MenuButton
                  label={`${t.name} options`}
                  onClick={(e) =>
                    openMenu(
                      e.currentTarget,
                      [
                        {
                          id: 'rename',
                          label: 'Rename',
                          run: () => setRenaming(t.id),
                        },
                        {
                          kind: 'label',
                          id: 'merge-label',
                          label: 'Merge into',
                        },
                        ...(tags ?? [])
                          .filter((o) => o.id !== t.id)
                          .map((o) => ({
                            id: `merge-${o.id}`,
                            label: `#${o.name}`,
                            run: () => run('tag_rename', { from: t.name, to: o.name }, `Merged #${t.name} into #${o.name}`),
                          })),
                        { kind: 'sep', id: 'sep' },
                        {
                          id: 'delete',
                          label: 'Delete',
                          danger: true,
                          run: () => setConfirm(t),
                        },
                      ],
                      `${t.name} options`,
                    )
                  }
                />
              </div>
            ))}
          </div>
        </Block>
      )}
      {confirm && (
        <Dialog title={`Delete #${confirm.name}`} onClose={() => setConfirm(null)} testId="delete-tag">
          <div className="ap-dialog-note">{`The tag is removed from ${confirm.count === 1 ? '1 note' : `${confirm.count} notes`}; the notes stay.`}</div>
          <div className="m-card-actions">
            <Button onClick={() => setConfirm(null)}>Cancel</Button>
            <Button
              variant="primary"
              onClick={() => {
                run('tag_delete', { name: confirm.name }, `Deleted #${confirm.name}`)
                setConfirm(null)
              }}
            >
              Delete
            </Button>
          </div>
        </Dialog>
      )}
    </Group>
  )
}
