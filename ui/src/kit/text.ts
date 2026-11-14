// Text that renders as one text node. React gives every string child of an element a text node
// of its own, so `+{n}` or `{a} · {b}` is several adjacent text nodes; the browser lays each one
// out to 1/64 px, and the text lands a fraction of a pixel away from the wireframes', which write
// it as one node. A run of text is therefore one string: a template literal for plain values,
// these helpers where parts can be elements. `npm run lint:text` (scripts/check-text-nodes.mjs)
// fails on any element that may render two text nodes in a row.
import { cloneElement, isValidElement, type ReactElement, type ReactNode } from 'react'

/**
 * Nodes for one place in the tree whose adjacent strings and numbers are merged into one string
 * (one text node), with the parts that render nothing (null, undefined, booleans, '') dropped and
 * nested arrays flattened, as React would render them. The brand tells the text-node check that
 * the items never put two text nodes in a row.
 */
export type Runs = readonly ReactNode[] & { readonly __textRuns: true }

/** `parts` as Runs: what React renders for them, with each run of text as one string. Elements without a key get their position as one. */
export function runs(parts: readonly ReactNode[]): Runs {
  const out: ReactNode[] = []
  const add = (p: ReactNode) => {
    if (p === null || p === undefined || typeof p === 'boolean' || p === '') return
    if (Array.isArray(p)) return p.forEach(add)
    if (typeof p === 'string' || typeof p === 'number' || typeof p === 'bigint') {
      const last = out.length - 1
      if (typeof out[last] === 'string') out[last] = `${out[last] as string}${p}`
      else out.push(`${p}`)
      return
    }
    out.push(isValidElement(p) && p.key === null ? cloneElement(p as ReactElement, { key: `r${out.length}` }) : p)
  }
  parts.forEach(add)
  return out as unknown as Runs
}

/** The separator of meta lines: `mode · feat/api · 14m`. */
export const DOT = ' · '

/**
 * Every part (empty ones too) joined with ` · ` as one run of text: a string when the parts are
 * strings and numbers, else Runs (the parts' elements between merged strings).
 */
export function joinDot(...parts: (string | number)[]): string
export function joinDot(...parts: ReactNode[]): Runs
export function joinDot(...parts: ReactNode[]): string | Runs {
  if (parts.every((p) => typeof p === 'string' || typeof p === 'number')) return parts.join(DOT)
  return runs(parts.flatMap((p, i) => (i ? [DOT, p] : [p])))
}
