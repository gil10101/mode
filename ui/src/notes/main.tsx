// Dev-only entry (`notes.html`, left out of Tauri builds like the gallery):
// the Notes view inside the kit window, on the browser mock's sample vault at
// the wireframes' moment (Projects / Mode, "Draft v2 changelog" open).
// `?theme=dark` flips the tokens; `?empty=1` starts with an empty vault;
// `?board=1` opens the board; `?details=1` opens the detail panel.
import { anaPicture } from '../kit/mascot'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import '../kit/fonts.css'
import '../kit/mode.css'
import './dev.css'
import { IconDefs } from '../kit/icons'
import { BrandDefs } from '../kit/brands'
import { Main, ProfileRow, Sidebar, Window } from '../kit/layout'
import { ThemeProvider } from '../kit/theme'
import { isTauri } from '../tauri'
import { installNotesMock } from './mock'
import { markShell } from '../state/shell'
import { NotesView } from './NotesView'

markShell()
const params = new URLSearchParams(location.search)
const dark = params.get('theme') === 'dark'
if (dark) document.documentElement.dataset.theme = 'dark'
const empty = params.get('empty') === '1'
if (!isTauri()) installNotesMock({ sample: !empty })

const NAV = [
  { id: 'sessions', label: 'Sessions', icon: 'sessions' },
  { id: 'inbox', label: 'Inbox', icon: 'inbox' },
  { id: 'notes', label: 'Notes', icon: 'notes' },
  { id: 'calendar', label: 'Calendar', icon: 'calendar' },
  { id: 'chat', label: 'Chat', icon: 'chat' },
  { id: 'automations', label: 'Automations', icon: 'automations' },
  { id: 'integrations', label: 'Integrations', icon: 'integrations' },
  { id: 'shortcuts', label: 'Shortcuts', icon: 'shortcuts' },
] as const

const BOTTOM = [
  { id: 'stats', label: 'Stats', icon: 'stats' },
  { id: 'settings', label: 'Settings', icon: 'settings' },
] as const

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider theme={dark ? 'dark' : 'light'}>
      <IconDefs />
      <BrandDefs />
      <Window testId="window">
        <Sidebar items={[...NAV]} bottom={[...BOTTOM]} active="notes" profile={<ProfileRow name="Ana" detail="1.2M tokens this week" src={anaPicture} />} />
        <Main>
          <NotesView
            initial={
              empty
                ? { folderId: 'root' }
                : {
                    folderId: 'f-mode',
                    noteId: 'n-changelog',
                    expanded: ['f-projects'],
                    layout: params.get('board') === '1' ? 'board' : 'list',
                    detailOpen: params.get('details') === '1',
                  }
            }
          />
        </Main>
      </Window>
    </ThemeProvider>
  </StrictMode>,
)
