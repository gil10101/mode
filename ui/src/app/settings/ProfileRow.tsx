// The profile row (03 §2.3.1): the picture (the mascot until one is chosen),
// `profile.name`, and "1.2M tokens this week" from `health.tokens_week`
// (hidden below 700 px of window height). Click opens the menu anchored above
// it: Edit profile (→ Settings → General, name focused), Quiet hours as a
// switch (pauses alerts for an hour through `quiet_set {pause_minutes}`),
// Settings, Quit (`app_quit`; the core asks first when sessions run in Mode,
// QuitDialog.tsx). The kit's ProfileRow markup with a menu button's semantics
// (kit candidate: `aria-label` / `aria-haspopup` on ProfileRow).
import { useEffect, useLayoutEffect, useRef, useState, type ReactElement, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Mascot } from '../../kit/avatars'
import { Menu, MenuItem, MenuSep, Switch } from '../../kit/controls'
import { cmdErrorText } from '../../state/contract'
import { useSetting, useSettingsStore } from '../../state/settings-store'
import { useSessionsStore } from '../../state/sessions-store'
import { useCoreStore } from '../../state/store'
import { navigate } from '../router'
import { tokens } from '../sessions/format'
import { useNow } from '../ui'
import { isPaused, setQuiet } from './AlertsPage'
import { fileSrc } from './GeneralPage'
import { useQuit } from './QuitDialog'

/** How long the profile menu's Quiet hours pauses alerts (07 §8.2 "Pause alerts for 1 h"). */
export const QUIET_PAUSE_MINUTES = 60

export function profileDetail(tokensWeek: number | undefined, hooksListening: boolean | null): string {
  if (tokensWeek !== undefined) return `${tokens(tokensWeek)} tokens this week`
  if (hooksListening === null) return '…'
  return hooksListening ? 'hooks connected' : 'hooks not connected'
}

export function AppProfile({ collapsed }: { collapsed?: boolean }) {
  const [name] = useSetting<string>('profile.name')
  const [picture] = useSetting<string | null>('profile.picture_path')
  const sessionsWeek = useSessionsStore((s) => s.health?.tokens_week)
  const settingsWeek = useSettingsStore((s) => s.health?.tokens_week)
  const status = useCoreStore((s) => s.status)
  const detail = profileDetail(sessionsWeek ?? settingsWeek, status ? status.ipc_listening : null)
  const shown = name || 'You'
  const ref = useRef<HTMLDivElement>(null)
  const [open, setOpen] = useState(false)
  const label = `Profile, ${shown}, ${detail}, menu`
  return (
    <>
      <div
        ref={ref}
        className="m-profile"
        role="button"
        tabIndex={0}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        title={collapsed ? `${shown} · ${detail}` : undefined}
        data-testid="profile"
        onClick={() => setOpen((o) => !o)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' || e.key === ' ' || e.key === 'ArrowUp') {
            e.preventDefault()
            setOpen(true)
          }
        }}
      >
        <Mascot size={24} src={picture ? fileSrc(picture) : undefined} picture={Boolean(picture)} alt="Profile picture" />
        <div className="m-row-text">
          <div className="m-row-name" data-testid="profile-name-text">
            {shown}
          </div>
          <div className="m-row-detail" data-testid="profile-detail">
            {detail}
          </div>
        </div>
      </div>
      {open && ref.current && <ProfileMenu anchor={ref.current} onClose={() => setOpen(false)} />}
    </>
  )
}

function ProfileMenu({ anchor, onClose }: { anchor: HTMLElement; onClose: () => void }) {
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  const now = useNow(30_000)
  const [pauseUntil] = useSetting<string | null>('notifications.pause_until')
  const paused = isPaused(pauseUntil, now)
  const togglePause = (on: boolean) =>
    void setQuiet({ pause_minutes: on ? QUIET_PAUSE_MINUTES : 0 }).catch((e) => useSessionsStore.getState().showNotice(`Couldn't change quiet hours: ${cmdErrorText(e)}`))

  useLayoutEffect(() => {
    const a = anchor.getBoundingClientRect()
    const box = ref.current?.getBoundingClientRect()
    const h = box?.height ?? 0
    // Anchored above the row (03 §2.3.1); below it only when there is no room.
    const top = a.top - h - 4 >= 8 ? a.top - h - 4 : Math.min(window.innerHeight - h - 8, a.bottom + 4)
    setPos({ left: Math.max(8, a.left), top })
    ref.current?.querySelector<HTMLElement>('[role="menuitem"]')?.focus()
  }, [anchor])

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current?.contains(e.target as Node) || anchor.contains(e.target as Node)) return
      onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation()
        onClose()
        anchor.focus()
      }
    }
    window.addEventListener('mousedown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('mousedown', onDown, true)
      window.removeEventListener('keydown', onKey, true)
    }
  }, [anchor, onClose])

  const item = (label: ReactNode, run: () => void, trailing?: ReactElement) => (
    <MenuItem
      trailing={trailing}
      onSelect={() => {
        run()
      }}
    >
      {label}
    </MenuItem>
  )

  return createPortal(
    <div
      className="ap-popover"
      ref={ref}
      style={pos ?? { left: -9999, top: -9999 }}
      data-testid="profile-menu"
      onKeyDown={(e) => {
        if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return
        const items = Array.from(ref.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])
        const i = items.indexOf(document.activeElement as HTMLElement)
        e.preventDefault()
        items[(i + (e.key === 'ArrowDown' ? 1 : items.length - 1) + items.length) % items.length]?.focus()
      }}
    >
      <Menu label="Profile" onClose={onClose}>
        {item('Edit profile', () => {
          onClose()
          navigate({ view: 'settings', rest: 'general/name' })
        })}
        {item('Quiet hours', () => togglePause(!paused), <Switch on={paused} label="Quiet hours" className="sg-push" onChange={togglePause} />)}
        <MenuSep />
        {item('Settings', () => {
          onClose()
          navigate({ view: 'settings' })
        })}
        {item('Quit', () => {
          onClose()
          void useQuit.getState().request()
        })}
      </Menu>
    </div>,
    document.body,
  )
}
