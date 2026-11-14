// The snooze popover (03 §4.7, presets of 07 §6.2): Later today, Tomorrow,
// Next week, Pick a date, Until reply (threads only), the scope (whole
// thread by default, or this message) and the preview of the return time.
// `1`–`5` move to a preset, Enter confirms (03 §10.3).
import { useEffect, useRef, useState } from 'react'
import { create } from 'zustand'
import { Button } from '../../kit/button'
import { Seg, SegItem } from '../../kit/controls'
import type { SnoozePreset } from '../../state/contract-connectors'
import { SNOOZE_PRESETS, previewLabel, resolvePreset, useInbox } from '../../state/inbox-store'
import { invoke } from '../../tauri'
import { cmdErrorText } from '../../state/contract'
import { useSessionsStore } from '../../state/sessions-store'
import { Popover } from './Popover'

interface SnoozeUi {
  open: { ids: string[]; anchor: DOMRect; messageId?: string } | null
  show(anchor: Element, ids: string[], messageId?: string): void
  close(): void
}

export const useSnoozeUi = create<SnoozeUi>((set) => ({
  open: null,
  show: (anchor, ids, messageId) => set({ open: { ids, anchor: anchor.getBoundingClientRect(), messageId } }),
  close: () => set({ open: null }),
}))

export function openSnooze(anchor: Element, ids: string[], messageId?: string) {
  useSnoozeUi.getState().show(anchor, ids, messageId)
}

function toLocalInput(ms: number): string {
  const d = new Date(ms)
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`
}

export function SnoozeHost() {
  const open = useSnoozeUi((s) => s.open)
  const close = useSnoozeUi((s) => s.close)
  if (!open) return null
  return (
    <Popover anchor={open.anchor} onClose={close} label="Snooze" testId="snooze-popover">
      <SnoozeMenu ids={open.ids} messageId={open.messageId} onDone={close} />
    </Popover>
  )
}

function SnoozeMenu({ ids, messageId, onDone }: { ids: string[]; messageId?: string; onDone: () => void }) {
  const [focus, setFocus] = useState(0)
  const [pick, setPick] = useState<number>(() => resolvePreset('tomorrow', Date.now()))
  const [scope, setScope] = useState<'thread' | 'message'>('thread')
  const ref = useRef<HTMLDivElement>(null)
  const now = Date.now()
  const rows = SNOOZE_PRESETS
  const current = rows[focus]!.preset

  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus()
  }, [])

  const confirm = async (preset: SnoozePreset) => {
    onDone()
    const inbox = useInbox.getState()
    if (scope === 'message' && messageId && ids.length === 1) {
      try {
        // "This message" (03 §4.7): `kind: 'message'` (`mode_core::connectors::api::SnoozeArgs`).
        await invoke('thread_snooze', { id: messageId, preset, kind: 'message', ...(preset === 'pick_date' ? { until: new Date(Math.max(pick, Date.now() + 5 * 60_000)).toISOString() } : {}) })
        useSessionsStore.getState().showNotice('Snoozed this message')
        void inbox.load()
      } catch (e) {
        useSessionsStore.getState().showNotice(`Couldn't snooze the message: ${cmdErrorText(e)}`)
      }
      return
    }
    await Promise.all(ids.map((id) => inbox.snooze(id, preset, preset === 'pick_date' ? pick : undefined)))
    if (ids.length > 1) inbox.setMulti(false)
  }

  const onKey = (e: React.KeyboardEvent) => {
    if (e.target instanceof HTMLInputElement) {
      if (e.key === 'Enter') {
        e.preventDefault()
        void confirm('pick_date')
      }
      return
    }
    const n = Number(e.key)
    if (n >= 1 && n <= rows.length) {
      e.preventDefault()
      setFocus(n - 1)
      ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]')[n - 1]?.focus()
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault()
      const next = (focus + (e.key === 'ArrowDown' ? 1 : rows.length - 1)) % rows.length
      setFocus(next)
      ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]')[next]?.focus()
    } else if (e.key === 'Enter') {
      e.preventDefault()
      void confirm(current)
    }
  }

  const hint = (p: SnoozePreset) => {
    if (p === 'pick_date') return ''
    if (p === 'until_reply') return 'or 30 days'
    return previewLabel(resolvePreset(p, now)).replace(/^back /, '')
  }

  return (
    <div className="m-menu" role="menu" aria-label="Snooze" ref={ref} onKeyDown={onKey}>
      {rows.map((r, i) => (
        <div
          key={r.preset}
          className={`m-menu-item${i === focus ? ' is-focus' : ''}`}
          role="menuitem"
          tabIndex={0}
          data-preset={r.preset}
          onMouseEnter={() => setFocus(i)}
          onFocus={() => setFocus(i)}
          onClick={() => {
            if (r.preset === 'pick_date') setFocus(i)
            else void confirm(r.preset)
          }}
        >
          {r.label}
          <span className="ib-snooze-hint">{hint(r.preset)}</span>
        </div>
      ))}
      {current === 'pick_date' && (
        <div className="ib-snooze-pick">
          <input
            className="m-input"
            type="datetime-local"
            aria-label="Snooze until"
            value={toLocalInput(pick)}
            min={toLocalInput(now + 5 * 60_000)}
            onChange={(e) => {
              const t = Date.parse(e.target.value)
              if (!Number.isNaN(t)) setPick(t)
            }}
          />
          <Button onClick={() => void confirm('pick_date')}>Snooze</Button>
        </div>
      )}
      <div className="ib-snooze-foot">
        {messageId && ids.length === 1 && (
          <Seg label="Snooze scope">
            <SegItem active={scope === 'thread'} onClick={() => setScope('thread')}>
              Whole thread
            </SegItem>
            <SegItem active={scope === 'message'} onClick={() => setScope('message')}>
              This message
            </SegItem>
          </Seg>
        )}
        <span className="ib-preview" data-testid="snooze-preview">
          {current === 'until_reply' ? 'back when someone replies (at most 30 days)' : previewLabel(resolvePreset(current, now, pick))}
        </span>
      </div>
    </div>
  )
}
