// Skeleton (10 §3.7 Loading, §7.4.21): one block of a view's loading state. A view composes its
// own skeleton from these in the layout of its content, so nothing moves when the data lands;
// there is no generic loading component and no "loading…" text.
import type { CSSProperties } from 'react'
import { cx } from './cx'

export type SkeletonVariant = 'text' | 'n' | 'square' | 'bar'

export interface SkeletonProps {
  /** A 12-px text line (default), a 20-px figure, an 11-px tracker cell, or a vertical bar. */
  variant?: SkeletonVariant
  /** Pixels (a column sized by its content), or a share of a container that has a width of its own. */
  width?: number | '40%' | '60%' | '80%'
  /** A tracker cell's week column: the sweep starts 20 ms later per column. */
  column?: number
  /** A bar's height, 0–100. */
  height?: number
  className?: string
}

const SHARE = { '40%': 'w-40', '60%': 'w-60', '80%': 'w-80' } as const

export function Skeleton({ variant = 'text', width, column, height, className }: SkeletonProps) {
  const style: Record<string, string | number> = {}
  if (typeof width === 'number') style.width = `${width}px`
  if (column !== undefined) style['--c'] = column
  if (height !== undefined) style['--h'] = `${height}%`
  return <span className={cx('m-skel', `is-${variant}`, typeof width === 'string' && SHARE[width], className)} style={Object.keys(style).length ? (style as CSSProperties) : undefined} aria-hidden="true" />
}
