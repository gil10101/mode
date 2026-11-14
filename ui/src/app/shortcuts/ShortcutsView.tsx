// Shortcuts (03 §7D): the only place in Mode where a key is printed. A search
// field (`/` focuses, Esc clears then blurs), groups by surface in two
// columns, one `.m-shortcut` row per action with its key caps, scope word,
// "custom" or the inline conflict, and `⋮` (Reset this shortcut, Copy key).
// Enter on a row or a click on the caps records a new binding: press the
// combination, Enter confirms, Esc cancels, Backspace clears. The global
// group writes its device-scope keys; every other row `shortcuts.bindings`,
// both through `hotkey_set { name, keys }`.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Button, MenuButton } from '../../kit/button'
import { Key } from '../../kit/controls'
import { MainBody, MainHead } from '../../kit/layout'
import { invoke } from '../../tauri'
import type { HotkeySetResult } from '../../state/contract-settings'
import { currentOs, defaultFor, useSettingsStore, type Os, errorText } from '../../state/settings-store'
import { useSessionsStore } from '../../state/sessions-store'
import { copyText, Dialog, openMenu } from '../ui'
import { CATALOG, conflictText, effectiveBindings, findConflicts, fixedCaps, GROUPS, type Binding, type ShortcutGroup } from './catalog'
import { caps, capsText, formatChord, parseChord, recordKey, searchWords, type Chord } from './keys'
import './shortcuts.css'

const notice = (t: string) => useSessionsStore.getState().showNotice(t)

/** `?` hands the view key (`sessions`, `notes`, …) as the query; it filters to that group. */
const VIEW_GROUP: Record<string, ShortcutGroup> = {
  sessions: 'Sessions',
  inbox: 'Inbox',
  notes: 'Notes',
  calendar: 'Calendar',
  chat: 'Chat',
  agents: 'Agents',
  automations: 'Automations',
  integrations: 'Integrations',
  shortcuts: 'Shortcuts',
  settings: 'Settings',
  stats: 'App window',
}

export function matchesSearch(b: Binding, query: string, os: Os): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return true
  const chord = parseChord(b.keys, os)
  const capsJoined = (b.def.fixed ? fixedCaps(b.def.fixed, os) : caps(chord, os)).join('').toLowerCase()
  const hay = `${b.def.name} ${b.def.group} ${b.def.scope} ${searchWords(chord, os)} ${capsJoined}`.toLowerCase()
  return words.every((w) => hay.includes(w) || capsJoined.includes(w.replace(/\+/g, '')))
}

interface Recording {
  id: string
  /** The combination pressed so far; `null` = cleared (unbound). `undefined` = nothing yet. */
  pending: Chord | null | undefined
}

export function ShortcutsView({ query }: { query?: string }) {
  const values = useSettingsStore((s) => s.values)
  const platformOs = useSettingsStore((s) => s.platform?.os)
  const os = platformOs ?? currentOs()
  const initial = query ? (VIEW_GROUP[query] ?? query) : ''
  const [search, setSearch] = useState(initial)
  const [recording, setRecording] = useState<Recording | null>(null)
  const [refused, setRefused] = useState<Record<string, string>>({})
  const [confirmReset, setConfirmReset] = useState(false)
  const searchRef = useRef<HTMLInputElement>(null)
  const listsRef = useRef<HTMLDivElement>(null)

  useEffect(() => setSearch(query ? (VIEW_GROUP[query] ?? query) : ''), [query])
  useEffect(() => {
    if (!useSettingsStore.getState().loaded) void useSettingsStore.getState().load()
  }, [])

  const bindings = useMemo(
    () =>
      effectiveBindings(
        {
          get: (k) => (k in values ? values[k] : defaultFor(k, os)),
          defaultOf: (k) => defaultFor(k, os),
        },
        os,
      ),
    [values, os],
  )
  const conflicts = useMemo(() => findConflicts(bindings, os), [bindings, os])
  const conflictCount = useMemo(() => {
    const pairs = new Set<string>()
    for (const [id, others] of conflicts) for (const o of others) pairs.add([id, o.id].sort().join('|'))
    return pairs.size + Object.keys(refused).length
  }, [conflicts, refused])

  const visible = bindings.filter((b) => matchesSearch(b, search, os))
  const columns = splitColumns(visible)

  const save = useCallback(
    async (b: Binding, chord: Chord | null) => {
      const keys = chord ? formatChord(chord, os) : null
      try {
        const r = await invoke<HotkeySetResult>('hotkey_set', { name: b.def.id, keys })
        if (b.def.settingKey && r && r.registered === false) {
          setRefused((x) => ({ ...x, [b.def.id]: r.reason ?? 'taken by the system' }))
          return
        }
        setRefused((x) => {
          const n = { ...x }
          delete n[b.def.id]
          return n
        })
        applyLocally(b, r?.keys ?? keys)
      } catch (e) {
        const why = errorText(e)
        if (b.def.settingKey) setRefused((x) => ({ ...x, [b.def.id]: /taken|registered/i.test(why) ? 'taken by the system' : why }))
        else notice(`Couldn't change ${b.def.name}: ${why}`)
      }
    },
    [os],
  )

  const reset = useCallback(
    (b: Binding) => {
      if (b.def.settingKey) {
        void useSettingsStore.getState().reset(b.def.settingKey)
        setRefused((x) => {
          const n = { ...x }
          delete n[b.def.id]
          return n
        })
        void invoke('hotkey_set', { name: b.def.id, keys: defaultFor(b.def.settingKey, os) as string | null }).catch(() => {})
      } else {
        const cur = { ...((values['shortcuts.bindings'] as Record<string, string | null>) ?? {}) }
        delete cur[b.def.id]
        void useSettingsStore.getState().set('shortcuts.bindings', cur)
      }
    },
    [values, os],
  )

  // While recording, every key belongs to the recorder (capture phase, before the shell's keys).
  useEffect(() => {
    if (!recording) return
    const onKey = (e: KeyboardEvent) => {
      const b = bindings.find((x) => x.def.id === recording.id)
      if (!b) return
      e.preventDefault()
      e.stopPropagation()
      const r = recordKey(e, recording.pending !== undefined)
      if (r.kind === 'modifier') return
      if (r.kind === 'cancel') return setRecording(null)
      if (r.kind === 'clear') return setRecording({ id: recording.id, pending: null })
      if (r.kind === 'confirm') {
        setRecording(null)
        void save(b, recording.pending ?? null)
        return
      }
      setRecording({ id: recording.id, pending: r.chord })
    }
    const onDown = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest?.(`[data-recording="true"]`)) setRecording(null)
    }
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('mousedown', onDown, true)
    return () => {
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('mousedown', onDown, true)
    }
  }, [recording, bindings, save])

  // `/` focuses the search field (03 §10.10).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
      e.preventDefault()
      searchRef.current?.focus()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const startRecording = (b: Binding) => {
    if (b.def.fixed) return
    setRecording({ id: b.def.id, pending: undefined })
  }

  const rowKeys = (e: React.KeyboardEvent, b: Binding) => {
    if (recording) return
    const rows = Array.from(listsRef.current?.querySelectorAll<HTMLElement>('.m-shortcut') ?? [])
    const i = rows.indexOf(e.currentTarget as HTMLElement)
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      rows[Math.max(0, Math.min(rows.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))]?.focus()
    } else if (e.key === 'Enter' && e.target === e.currentTarget) {
      e.preventDefault()
      startRecording(b)
    } else if ((e.key === 'r' || e.key === 'R') && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault()
      reset(b)
    }
  }

  return (
    <section className="ap-view sc-view" aria-label="Shortcuts" data-testid="shortcuts-view">
      <MainHead title="Shortcuts" titleTestId="view-title">
        <input
          ref={searchRef}
          className="m-input sc-search"
          placeholder="Search shortcuts…"
          aria-label="Search shortcuts"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              e.stopPropagation()
              if (search) setSearch('')
              else e.currentTarget.blur()
            } else if (e.key === 'ArrowDown') {
              e.preventDefault()
              listsRef.current?.querySelector<HTMLElement>('.m-shortcut')?.focus()
            }
          }}
        />
        {conflictCount > 0 && <span className="m-t-meta" data-testid="conflict-count">{conflictCount === 1 ? '1 conflict' : `${conflictCount} conflicts`}</span>}
        <MenuButton
          label="Shortcuts options"
          onClick={(e) =>
            openMenu(
              e.currentTarget,
              [
                { id: 'reset', label: 'Reset to defaults', run: () => setConfirmReset(true) },
                { id: 'import', label: 'Import…', dimmed: true, reason: 'v1.0', run: () => {} },
                { id: 'export', label: 'Export…', dimmed: true, reason: 'v1.0', run: () => {} },
              ],
              'Shortcuts options',
            )
          }
        />
      </MainHead>
      <MainBody className="sc-body" testId="shortcuts-body">
        {visible.length === 0 ? (
          <div className="ap-empty" data-testid="no-shortcut">
            {`No shortcut matches “${search}”.`}
          </div>
        ) : (
          <div className="sc-lists" ref={listsRef}>
            {columns.map((col, ci) => (
              <div key={ci} className="m-list">
                {col.map(({ group, rows }) => [
                  <div key={`g-${group}`} className="m-group-label">
                    {group}
                  </div>,
                  ...rows.map((b) => (
                    <ShortcutRow
                      key={b.def.id}
                      b={b}
                      os={os}
                      recording={recording?.id === b.def.id ? recording : null}
                      conflict={refused[b.def.id] ?? (conflicts.get(b.def.id) ? conflictText(conflicts.get(b.def.id)!) : undefined)}
                      onRecord={() => startRecording(b)}
                      onKeyDown={(e) => rowKeys(e, b)}
                      onMore={(el) =>
                        openMenu(
                          el,
                          [
                            { id: 'reset', label: 'Reset this shortcut', dimmed: !b.custom && !refused[b.def.id], run: () => reset(b) },
                            { id: 'copy', label: 'Copy key', dimmed: !b.keys && !b.def.fixed, run: () => copyText(b.def.fixed ? fixedCaps(b.def.fixed, os).join(os === 'macos' ? '' : '+') : capsText(parseChord(b.keys, os), os), 'Key copied') },
                          ],
                          `${b.def.name} options`,
                        )
                      }
                    />
                  )),
                ])}
              </div>
            ))}
          </div>
        )}
      </MainBody>
      {confirmReset && (
        <Dialog title="Reset every shortcut" onClose={() => setConfirmReset(false)} testId="reset-shortcuts">
          <div className="ap-dialog-note">Every key goes back to its default, the global hotkeys included.</div>
          <div className="m-card-actions">
            <Button onClick={() => setConfirmReset(false)}>Cancel</Button>
            <Button
              variant="primary"
              onClick={() => {
                setConfirmReset(false)
                setRefused({})
                void useSettingsStore.getState().set('shortcuts.bindings', {})
                for (const d of CATALOG) if (d.settingKey) void useSettingsStore.getState().reset(d.settingKey)
              }}
            >
              Reset
            </Button>
          </div>
        </Dialog>
      )}
    </section>
  )
}

/** Writes a confirmed binding into the store the way the core will push it back (`settings.changed`). */
function applyLocally(b: Binding, keys: string | null) {
  const st = useSettingsStore.getState()
  if (b.def.settingKey) {
    useSettingsStore.setState({ values: { ...st.values, [b.def.settingKey]: keys } })
    return
  }
  const cur = { ...((st.values['shortcuts.bindings'] as Record<string, string | null>) ?? {}) }
  cur[b.def.id] = keys
  useSettingsStore.setState({ values: { ...st.values, 'shortcuts.bindings': cur } })
}

/** Groups in §7D order split into two columns of about the same height. */
export function splitColumns(rows: readonly Binding[]): { group: ShortcutGroup; rows: Binding[] }[][] {
  const groups = GROUPS.map((g) => ({ group: g, rows: rows.filter((r) => r.def.group === g) })).filter((g) => g.rows.length)
  const total = groups.reduce((n, g) => n + g.rows.length + 1, 0)
  const left: typeof groups = []
  let used = 0
  let i = 0
  while (i < groups.length && (left.length === 0 || used + groups[i]!.rows.length / 2 < total / 2)) {
    used += groups[i]!.rows.length + 1
    left.push(groups[i]!)
    i++
  }
  const right = groups.slice(i)
  return right.length ? [left, right] : [left]
}

/**
 * One `.m-shortcut` (the kit's Shortcut markup plus a "custom" detail line,
 * the recording state and row keyboard handling: kit candidate).
 */
function ShortcutRow({
  b,
  os,
  recording,
  conflict,
  onRecord,
  onKeyDown,
  onMore,
}: {
  b: Binding
  os: Os
  recording: Recording | null
  conflict?: string
  onRecord: () => void
  onKeyDown: (e: React.KeyboardEvent) => void
  onMore: (el: HTMLElement) => void
}) {
  const shown = recording ? (recording.pending === undefined ? parseChord(b.keys, os) : recording.pending) : parseChord(b.keys, os)
  const keyCaps = b.def.fixed ? fixedCaps(b.def.fixed, os) : caps(shown, os)
  const detail = conflict ?? (b.custom ? 'custom' : undefined)
  const label = `${b.def.name}: ${keyCaps.length ? keyCaps.join(' ') : 'unbound'}`
  return (
    <div className="m-shortcut" tabIndex={0} aria-label={`${label}, ${b.def.scope}${detail ? `, ${detail}` : ''}`} data-testid={`shortcut-${b.def.id}`} data-recording={recording ? 'true' : undefined} onKeyDown={onKeyDown}>
      <div className="m-row-text">
        <div className="m-row-name">{b.def.name}</div>
        {detail !== undefined && <div className={conflict ? 'm-row-detail is-conflict' : 'm-row-detail'}>{detail}</div>}
      </div>
      <span
        className={`m-keys${recording ? ' sc-recording' : ''}${b.def.fixed ? ' sc-fixed' : ''}`}
        role="button"
        tabIndex={-1}
        aria-label={recording ? 'Press the new keys, then Enter' : b.def.fixed ? `${b.def.name} cannot be changed` : `Change ${b.def.name}`}
        onClick={(e) => {
          e.stopPropagation()
          onRecord()
        }}
      >
        {keyCaps.length ? keyCaps.map((k, i) => <Key key={i}>{k}</Key>) : <span className="m-t-meta sc-unbound">—</span>}
      </span>
      <span className="m-row-meta">{b.def.scope}</span>
      <MenuButton
        label={`${b.def.name} options`}
        onClick={(e) => {
          e.stopPropagation()
          onMore(e.currentTarget)
        }}
      />
    </div>
  )
}
