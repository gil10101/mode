// The window a preview draws in (03 §2.8): the real view's head and body at
// their own size, 115 % of the column, inert (a picture of the view, not a
// second copy of its controls).
import type { ReactNode } from 'react'
import { MainHead } from '../../../kit/layout'

export function Frame({ title, head, step, children }: { title?: string; head?: ReactNode; step: string; children: ReactNode }) {
  return (
    <div className="m-ob-preview" inert data-testid={`preview-${step}`}>
      {title !== undefined && <MainHead title={title}>{head}</MainHead>}
      {children}
    </div>
  )
}

export type Placement = 'notch' | 'top-bar' | 'right-edge'

export function asPlacement(v: unknown, notch: boolean): Placement {
  return v === 'top-bar' || v === 'right-edge' || v === 'notch' ? (v === 'notch' && !notch ? 'top-bar' : v) : notch ? 'notch' : 'top-bar'
}
