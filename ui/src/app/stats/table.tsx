// The breakdown tables as the kit draws them from text.ts's cells: shared by
// StatsView and the dev gallery so both render the wireframe's markup.
import type { ReactNode } from 'react'
import { AgentAvatar } from '../../kit/avatars'
import { StatsTable } from '../../kit/stats'
import { avatarHue } from '../sessions/format'
import type { Cell, TableSpec } from './text'

export function cellNode(c: Cell, open?: (name: string) => void): ReactNode {
  if ('agent' in c) {
    return (
      <span className="ss-agent" role="link" tabIndex={0} onClick={open ? () => open(c.text) : undefined} onKeyDown={open ? (e) => (e.key === 'Enter' ? open(c.text) : undefined) : undefined}>
        <AgentAvatar hue={avatarHue('avatar.8')} variant={c.agent.variant} untracked={c.agent.untracked} />
        {c.agent.name}
        {c.agent.suffix && <span className="is-dim">{c.agent.suffix}</span>}
      </span>
    )
  }
  if (c.code) return <code>{c.text}</code>
  if (c.dim) return <span className="is-dim">{c.text}</span>
  return c.text
}

export function SpecTable({ spec, className, open }: { spec: TableSpec; className?: string; open?: (name: string) => void }) {
  return <StatsTable aria-label={spec.label} className={className} columns={spec.columns} rows={spec.rows.map((cells) => cells.map((c) => cellNode(c, open)))} rowTitles={spec.rowTitles} />
}
