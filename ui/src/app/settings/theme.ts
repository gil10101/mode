// `theme.appearance` (device scope, 03 §8.5) applied to the app window: `auto`
// follows the OS; `dark` sets `data-theme="dark"` on <html> (mode.css §1).
// Theme packs (`theme.active`) arrive in v1.0.
import { useEffect, useState } from 'react'
import { useSetting } from '../../state/settings-store'
import type { Theme } from '../../kit/types'

function systemDark(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' && window.matchMedia('(prefers-color-scheme: dark)').matches
}

export function resolveAppearance(appearance: unknown, dark: boolean): Theme {
  if (appearance === 'dark') return 'dark'
  if (appearance === 'light') return 'light'
  return dark ? 'dark' : 'light'
}

/** The theme in force; keeps <html data-theme> in step. */
export function useAppearance(): Theme {
  const [appearance] = useSetting<string>('theme.appearance')
  const [dark, setDark] = useState(systemDark)
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return
    const mq = window.matchMedia('(prefers-color-scheme: dark)')
    const on = () => setDark(mq.matches)
    mq.addEventListener?.('change', on)
    return () => mq.removeEventListener?.('change', on)
  }, [])
  const theme = resolveAppearance(appearance, dark)
  useEffect(() => {
    const html = document.documentElement
    if (theme === 'dark') html.dataset.theme = 'dark'
    else delete html.dataset.theme
  }, [theme])
  return theme
}
