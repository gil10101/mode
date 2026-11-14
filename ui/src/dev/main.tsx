// Dev-only entry: the kit rendered with the gallery's sample data, light and
// dark side by side, in the same page chrome as docs/wireframes/components.html
// so e2e/kit.spec.ts can compare the two. `?theme=dark` flips the page tokens.
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import '../kit/fonts.css'
import '../kit/mode.css'
import { Gallery } from './gallery'

const theme = new URLSearchParams(location.search).get('theme')
if (theme === 'dark') document.documentElement.dataset.theme = 'dark'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Gallery />
  </StrictMode>,
)
