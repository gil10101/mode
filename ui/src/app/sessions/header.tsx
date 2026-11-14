// The session header (03 §3.6, 05 §11.5) used by the roster's detail panel
// and the session detail screen: 32-px avatar, the title on one line, one
// meta line (repo · branch · model · elapsed; the model's only mention),
// the state, the diff, the usage, one primary button and ⋮.
import type { MouseEvent, ReactNode } from 'react'
import { AgentAvatar } from '../../kit/avatars'
import { Button, MenuButton } from '../../kit/button'
import { SessionHead } from '../../kit/cards'
import { joinDot, type Runs } from '../../kit/text'
import type { AppRosterRow, SessionDetail, SessionHeader } from '../../state/contract-sessions'
import { avatarHue, elapsed, placeLabel, rowState, secondsSince, usageText } from './format'

/** The roster-row view of a detail header (for the shared action menus). */
export function rowFromHeader(h: SessionHeader, d?: Pick<SessionDetail, 'pending'> | null): AppRosterRow {
  return {
    id: h.id,
    display_name: h.display_name,
    avatar_color: h.avatar_color,
    avatar_variant: h.avatar_variant,
    state: h.state,
    waiting_kind: h.waiting_kind,
    title: h.title,
    cwd: h.cwd,
    repo_root: h.repo_root,
    git_branch: h.git_branch,
    model: h.model,
    current_tool: h.current_tool,
    lines_added: h.lines_added,
    lines_removed: h.lines_removed,
    files_changed: h.files_changed,
    tracking: h.tracking,
    launch_mode: h.launch_mode,
    state_seconds: 0,
    folded: false,
    live_agents: 0,
    pending: d?.pending ?? undefined,
    end_reason: h.end_reason,
    state_reason: h.state_reason,
    current_mcp_server: h.current_mcp_server,
    claude_session_id: h.claude_session_id,
    started_at: h.started_at,
    last_event_at: h.last_event_at,
    ended_at: h.ended_at,
    dismissed_at: h.dismissed_at,
    acknowledged_at: h.acknowledged_at,
    last_assistant_preview: h.last_assistant_preview,
  }
}

/** External = hooked and running in the user's own terminal (05 §11.5). */
export function isExternal(h: Pick<SessionHeader, 'tracking' | 'launch_mode'>): boolean {
  return h.tracking === 'hooked' && h.launch_mode === 'external'
}

/** Resume is the primary when the session can be resumed: ended or a history row (03 §3.6). */
export function isResumable(h: Pick<SessionHeader, 'ended_at' | 'dismissed_at'>): boolean {
  return !!h.ended_at || !!h.dismissed_at
}

/** `mode · feat/api · claude-opus-5-5 · 14m`, plus `external` and a non-default permission mode. */
export function headerMeta(h: SessionHeader, now: number): Runs {
  const place = placeLabel(h)
  const parts: ReactNode[] = [place.mono ? <code key="p">{place.text}</code> : place.text]
  if (h.git_branch) parts.push(<code key="b">{h.git_branch}</code>)
  if (isExternal(h)) parts.push('external')
  if (h.model) parts.push(h.model)
  parts.push(elapsed(secondsSince(h.state_since, now)))
  if (h.permission_mode && h.permission_mode !== 'default') parts.push(h.permission_mode)
  return joinDot(...parts)
}

export interface HeaderProps {
  header: SessionHeader
  now: number
  primaryLabel: string
  onPrimary: () => void
  onMore: (e: MouseEvent<HTMLButtonElement>) => void
  /** Live state word override (the row's state ticks faster than the detail). */
  state?: string
}

export function SessionHeaderView({ header: h, now, primaryLabel, onPrimary, onMore, state }: HeaderProps) {
  const changed = h.lines_added > 0 || h.lines_removed > 0
  const usage = h.state === 'untracked' && !h.num_turns ? undefined : usageText(h.input_tokens, h.output_tokens, h.num_turns)
  return (
    <SessionHead
      avatar={<AgentAvatar size={32} hue={avatarHue(h.avatar_color)} variant={h.avatar_variant} untracked={h.state === 'untracked'} />}
      title={h.display_name}
      model={h.model ?? ''}
      elapsed=""
      meta={headerMeta(h, now)}
      actions={
        <>
          <Button variant="primary" onClick={onPrimary}>
            {primaryLabel}
          </Button>
          <MenuButton label={`More for ${h.display_name}`} onClick={onMore} />
        </>
      }
      state={rowState(state ?? h.state)}
      diff={changed ? { add: h.lines_added, del: h.lines_removed } : undefined}
      files={changed ? h.files_changed : undefined}
      usage={changed ? usage : undefined}
      sub={
        changed ? undefined : (
          <>
            <span className="m-t-meta">no changes</span>
            {usage !== undefined && <span className="m-usage">{usage}</span>}
          </>
        )
      }
    />
  )
}
