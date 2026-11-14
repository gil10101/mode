import { IconDefs } from '../kit/icons'
import { BrandDefs } from '../kit/brands'
import { SECTIONS_1 } from './sections1'
import { SECTIONS_2 } from './sections2'
import { SECTIONS_3 } from './sections3'
import { SECTIONS_4 } from './sections4'
import { SECTIONS_5 } from './sections5'
import { Section } from './wf'

export const SECTIONS = [...SECTIONS_1, ...SECTIONS_2, ...SECTIONS_3, ...SECTIONS_4, ...SECTIONS_5]

export function Gallery() {
  return (
    <>
      <IconDefs />
      <BrandDefs />
      <main className="wf-page">
        <header>
          <h1 className="wf-h1">Mode components (kit)</h1>
          <p className="wf-status">ui/src/kit rendered by React · reference: docs/wireframes/components.html · stylesheet: ui/src/kit/mode.css (byte copy)</p>
          <p className="wf-summary">
            Every section below is the React kit drawing the gallery's sample data with the gallery's class structure, light on the left and dark on the right. e2e/kit.spec.ts screenshots each demo here and in the
            reference page and asserts they match.
          </p>
          <ul className="wf-toc">
            {SECTIONS.map((s) => (
              <li key={s.id}>
                <a href={`#${s.id}`}>{s.title}</a>
              </li>
            ))}
          </ul>
        </header>
        {SECTIONS.map((s) => (
          <Section key={s.id} {...s} />
        ))}
      </main>
    </>
  )
}
