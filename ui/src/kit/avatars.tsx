import type { CSSProperties, MouseEvent, ReactNode } from 'react'
import { BrandMark, type BrandId } from './brands'
import { cx } from './cx'
import { Icon } from './icons'
import { botArt } from './mascot'
import { useArtwork } from './theme'
import type { MessageSource, SessionState } from './types'

export interface AgentAvatarProps {
  /** Repo tint, 0–360 (mode 200, docs-site 140). Omitted = the stylesheet default. */
  hue?: number
  /** 05 §11.2: the accessory the bot head wears (`sessions.avatar_variant`); empty or unknown = the plain mark. */
  variant?: string | null
  size?: 24 | 32 | 40
  /** Hairline circle instead of a tint (no hooks). */
  untracked?: boolean
  src?: string
  className?: string
}

/** Session avatar: the bot-head artwork, with the session's accessory, in a circle tinted per repo. */
export function AgentAvatar({ hue, variant, size = 24, untracked, src, className }: AgentAvatarProps) {
  const art = useArtwork()
  const style = hue === undefined ? undefined : ({ '--repo-hue': String(hue) } as CSSProperties)
  return (
    <span
      className={cx('m-avatar-agent', size === 32 && 'is-32', size === 40 && 'is-40', untracked && 'is-untracked', className)}
      style={style}
      data-variant={variant || undefined}
    >
      <img src={src ?? botArt(art, variant)} alt="" />
    </span>
  )
}

export interface EditableAvatarProps extends AgentAvatarProps {
  /** The badge's accessible name ("Change the mark"). */
  label: string
  onEdit?: (e: MouseEvent<HTMLButtonElement>) => void
}

/** 03 §7E.2: the agent's mark with a pencil badge at its bottom-right that opens the accessory menu. */
export function EditableAvatar({ label, onEdit, className, ...avatar }: EditableAvatarProps) {
  return (
    <span className={cx('m-avatar-editable', className)}>
      <AgentAvatar {...avatar} />
      <button type="button" className="m-avatar-edit" aria-label={label} onClick={onEdit}>
        <Icon id="edit" />
      </button>
    </span>
  )
}

export interface PersonAvatarProps {
  initials?: string
  src?: string
  alt?: string
  /** 12-px source mark at the bottom-right. */
  source?: MessageSource
  size?: 24 | 32
  className?: string
}

/** Sender avatar: a picture, or neutral initials, plus the Gmail / Slack mark. */
export function PersonAvatar({ initials, src, alt, source, size = 24, className }: PersonAvatarProps) {
  return (
    <span className={cx('m-avatar-person', source && `m-src-${source}`, size === 32 && 'is-32', className)}>
      {src ? <img src={src} alt={alt ?? ''} /> : initials}
    </span>
  )
}

export type MascotSize = 18 | 20 | 24 | 32 | 48

export interface MascotProps {
  size?: MascotSize
  /** The state badge at the bottom-right; poses are not drawn. */
  badge?: SessionState
  src?: string
  /** `src` is a picture, not a mark: shown as is, clipped round, never blended with the surface. */
  picture?: boolean
  alt?: string
  className?: string
}

export function Mascot({ size = 32, badge, src, picture, alt = 'Mode mascot', className }: MascotProps) {
  const art = useArtwork()
  return (
    <span className={cx('m-mascot', size !== 32 && `is-${size}`, picture && 'is-picture', className)}>
      <img src={src ?? art.duck} alt={alt} />
      {badge && <StateBadge state={badge} />}
    </span>
  )
}

export function StateBadge({ state, className }: { state: SessionState; className?: string }) {
  return <i className={cx('m-badge-state', `is-${state}`, className)} />
}

export interface BrandProps {
  /** Gmail and Slack come from the stylesheet (the official marks as data URIs); any other brand draws its mark (10 §7.4.16); no brand takes children (a generic icon). */
  brand?: BrandId
  label?: string
  children?: ReactNode
  className?: string
}

/** The 32-px icon tile of an integration card. */
export function Brand({ brand, label, children, className }: BrandProps) {
  return <span className={cx('m-brand', className)}>{brand ? <BrandMark id={brand} label={label} /> : children}</span>
}
