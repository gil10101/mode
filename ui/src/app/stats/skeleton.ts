// The Stats skeleton's geometry (03 §7A Loading; 10 §7.4.21): the widths of the blocks that
// stand in for each band's figures until stats_days and stats_summary answer. Shared by the view
// (StatsSkeleton.tsx) and the wireframe (wireframe.ts), so the "Stats · loading" figure of
// docs/wireframes/app-window.html and the app draw the same blocks.

export type Share = '40%' | '60%' | '80%'

/** The spend row (content-sized columns, so pixels): the hero figure and its label, then today, this month, all time. */
export const SPEND: readonly { n: number; l: number }[] = [
  { n: 112, l: 180 },
  { n: 52, l: 32 },
  { n: 60, l: 56 },
  { n: 68, l: 44 },
]

/** The note under the spend row. */
export const NOTE: Share = '80%'

const N_SHARES: readonly Share[] = ['40%', '60%', '40%']
const L_SHARES: readonly Share[] = ['80%', '60%', '80%', '40%']

/** The twelve metric tiles (each 120 px or wider, so shares of the tile). */
export const METRICS: readonly { n: Share; l: Share }[] = Array.from({ length: 12 }, (_, i) => ({
  n: N_SHARES[i % N_SHARES.length]!,
  l: L_SHARES[i % L_SHARES.length]!,
}))

/** The streak line under the tracker. */
export const STREAK = 168

/** The Models panel's five share bars: the label and the figure (the track is drawn empty). */
export const SHARES: readonly { l: number; n: number }[] = [
  { l: 72, n: 64 },
  { l: 88, n: 56 },
  { l: 64, n: 60 },
  { l: 80, n: 48 },
  { l: 56, n: 52 },
]

/** Rows under a table's real headings. */
export const TABLE_ROWS = 5

const FIRST = [96, 112, 80, 104, 88]
const FIGURES = [36, 44, 32, 40, 28]

/** A table cell's block: the first column wider, figure columns by position. */
export function cellWidth(column: number, num: boolean, row: number): number {
  if (column === 0) return FIRST[row % FIRST.length]!
  if (!num) return 64
  return FIGURES[(column + row) % FIGURES.length]!
}

/** The Days panel's bars: seven days, then 24 hours (heights, 0–100). */
export const DAY_BARS: readonly number[] = [42, 64, 30, 78, 56, 70, 48]
export const HOUR_BARS: readonly number[] = [6, 4, 4, 4, 6, 8, 14, 24, 38, 52, 60, 56, 44, 50, 62, 72, 64, 52, 40, 30, 22, 16, 10, 8]
