// The command palette (10 §7.4.15; app-window.html `#palette`; proposed, not
// decided): the kit's Palette top-centered at 20 % of the window, results
// from `palette_query` grouped by kind, recent items on an empty input.
// ⌘/Ctrl+P opens it (⌘/Ctrl+K is the floating query bar, 03 §10.1); ↑ ↓
// move, Enter opens the row's route, Esc or a click outside closes. Nothing
// else on the window changes while it is open; no key is printed (ADR-0038).
import { useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { Palette, type PaletteGroup } from '../../kit/controls'
import { flatRows, rowIcon, rowKey, usePalette, type PaletteResult } from '../../state/palette-store'
import { navigate } from '../router'
import './palette.css'

/** ⌘/Ctrl+P, except where the key belongs to the terminal (the PTY gets Ctrl+P on Windows and Linux, 03 §3.8). */
export function isPaletteKey(e: Pick<KeyboardEvent, 'key' | 'code' | 'metaKey' | 'ctrlKey' | 'shiftKey' | 'altKey'>): boolean {
  return (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && (e.key.toLowerCase() === 'p' || e.code === 'KeyP')
}

function open(r: PaletteResult) {
  usePalette.getState().choose(r)
  if (r.route) navigate(r.route)
}

export function CommandPalette() {
  const isOpen = usePalette((s) => s.open)
  const text = usePalette((s) => s.text)
  const active = usePalette((s) => s.active)
  usePalette((s) => s.results)
  usePalette((s) => s.recent)
  const loading = usePalette((s) => s.loading)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isPaletteKey(e) || e.defaultPrevented) return
      if ((e.target as HTMLElement | null)?.closest?.('.xterm')) return
      e.preventDefault()
      const s = usePalette.getState()
      if (s.open) s.hide()
      else s.show()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // Results follow the text after 80 ms.
  useEffect(() => {
    if (!isOpen || !text.trim()) return
    const t = setTimeout(() => void usePalette.getState().query(), 80)
    return () => clearTimeout(t)
  }, [isOpen, text])

  useEffect(() => {
    if (!isOpen) return
    const onDown = (e: MouseEvent) => {
      const box = ref.current?.querySelector('.m-palette')
      if (box && !box.contains(e.target as Node)) usePalette.getState().hide()
    }
    window.addEventListener('mousedown', onDown, true)
    return () => window.removeEventListener('mousedown', onDown, true)
  }, [isOpen])

  useEffect(() => {
    if (!isOpen || !active) return
    ref.current?.querySelector<HTMLElement>('.m-palette-row.is-active')?.scrollIntoView?.({ block: 'nearest' })
  }, [isOpen, active])

  if (!isOpen) return null
  const groups = usePalette.getState().groups()
  const rows = flatRows(groups)
  const kitGroups: PaletteGroup[] = groups.map((g) => ({
    label: g.label,
    rows: g.rows.map((r) => ({ id: rowKey(r), icon: rowIcon(r), label: r.title, meta: r.detail })),
  }))
  if (text.trim() && !loading && !rows.length && usePalette.getState().results.length === 0) kitGroups.push({ label: `Nothing matches “${text.trim()}”.`, rows: [] })

  return createPortal(
    <div
      className="pl-host"
      ref={ref}
      data-testid="palette"
      onKeyDown={(e) => {
        const s = usePalette.getState()
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault()
          s.move(e.key === 'ArrowDown' ? 1 : -1)
        } else if (e.key === 'Enter') {
          e.preventDefault()
          const r = rows.find((x) => rowKey(x) === s.active) ?? rows[0]
          if (r) open(r)
        } else if (e.key === 'Escape') {
          e.preventDefault()
          e.stopPropagation()
          s.hide()
        }
      }}
    >
      <Palette
        query={text}
        onQueryChange={(t) => usePalette.getState().setText(t)}
        placeholder="Search sessions, notes, messages, automations and settings…"
        groups={kitGroups}
        activeId={active ?? undefined}
        onSelect={(id) => {
          const r = rows.find((x) => rowKey(x) === id)
          if (r) open(r)
        }}
      />
    </div>,
    document.body,
  )
}
