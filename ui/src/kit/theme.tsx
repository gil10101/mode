// Which artwork pack a subtree uses. The app sets `data-theme` on <html> for
// the tokens; this context only picks the light or dark PNGs, and the notch
// overlay forces `dark` because the bezel is always dark (mode.css §1).
import { createContext, useContext, type ReactNode } from 'react'
import { artwork, type Artwork } from './mascot'
import type { Theme } from './types'

const ThemeContext = createContext<Theme>('light')

export function ThemeProvider({ theme, children }: { theme: Theme; children?: ReactNode }) {
  return <ThemeContext.Provider value={theme}>{children}</ThemeContext.Provider>
}

export function useTheme(): Theme {
  return useContext(ThemeContext)
}

export function useArtwork(): Artwork {
  return artwork[useTheme()]
}
