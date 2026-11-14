// The gallery page chrome of docs/wireframes/components.html (.wf-* classes
// from mode.css §14): a section renders the same markup twice, light then
// dark, each example under its class-name caption.
import type { CSSProperties, ReactNode } from 'react'
import { ThemeProvider } from '../kit/theme'

export interface SectionSpec {
  id: string
  title: string
  note?: ReactNode
  /** Light over dark at full width (the wide sections). */
  stacked?: boolean
  /** Chat shows the dark pack first (the owner's reference is dark). */
  darkFirst?: boolean
  render: () => ReactNode
}

export function Section({ id, title, note, stacked, darkFirst, render }: SectionSpec) {
  const light = (
    <div className="wf-demo">
      <p className="wf-theme">light · default</p>
      {render()}
    </div>
  )
  const dark = (
    <div className="wf-demo" data-theme="dark">
      <p className="wf-theme">dark · [data-theme=dark]</p>
      <ThemeProvider theme="dark">{render()}</ThemeProvider>
    </div>
  )
  return (
    <section className="wf-section" id={id}>
      <h2 className="wf-h2">{title}</h2>
      {note && <p className="wf-note">{note}</p>}
      <div className="wf-pair" style={stacked ? { gridTemplateColumns: '1fr' } : undefined}>
        {darkFirst ? dark : light}
        {darkFirst ? light : dark}
      </div>
    </section>
  )
}

export function Ex({ caption, children }: { caption: string; children?: ReactNode }) {
  return (
    <div className="wf-ex">
      <p className="wf-caption">
        <b>{caption}</b>
      </p>
      {children}
    </div>
  )
}

export function Flow({ children, style }: { children?: ReactNode; style?: CSSProperties }) {
  return (
    <div className="wf-flow" style={style}>
      {children}
    </div>
  )
}

export function Item({ children }: { children?: ReactNode }) {
  return <span className="wf-item">{children}</span>
}
