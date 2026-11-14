// Now playing (02 §6.1, §7.1; 10 §7.4.22; OV-37): the peek's artwork (the
// equalizer for a track without one) and the expanded panel's strip. The strip's progress hairline runs on CSS alone:
// each push sets where the fill starts and how long it has to the end, and a
// linear animation carries it there (remounted per push, so it re-aims); a
// paused track or reduced motion draws the fill still. A track of unknown
// length (duration 0: a stream, an ad) draws no hairline, and one without an
// artist draws the title alone, centered.
import type { CSSProperties } from 'react'
import { Button } from './button'
import { cx } from './cx'
import { Icon } from './icons'
import { Spinner } from './primitives'

/** Three bars in the peek's left wing; they move only while `playing`. */
export function Equalizer({ playing, className }: { playing: boolean; className?: string }) {
  return (
    <span className={cx('m-eq', playing && 'is-playing', className)} aria-hidden="true" data-testid="np-eq">
      <i />
      <i />
      <i />
    </span>
  )
}

/** The resting peek's artwork (02 §6.1–6.3): dimmed while paused; `previous` is the last track's image, kept under the new one while it fades in. */
export function PeekArt({ src, previous, paused, leaving }: { src: string; previous?: string | null; paused?: boolean; leaving?: boolean }) {
  return (
    <span className={cx('m-peek-art', paused && 'is-paused', leaving && 'is-leaving')} aria-hidden="true" data-testid="np-peek-art">
      {previous && previous !== src && <img key={`out:${previous}`} className="is-out" src={previous} alt="" />}
      <img key={src} src={src} alt="" />
    </span>
  )
}

export interface Progress {
  /** Where the fill starts, in % of the track (two decimals). */
  fromPct: number
  /** How long the fill takes to reach the end (0 when paused). */
  leftMs: number
}

/** The hairline's start and remaining time at `now`, from the position read at `atMs`. */
export function progress(positionMs: number, durationMs: number, atMs: number, now: number, playing: boolean): Progress {
  if (!(durationMs > 0)) return { fromPct: 0, leftMs: 0 }
  const elapsed = playing ? Math.max(0, now - atMs) : 0
  const pos = Math.min(durationMs, Math.max(0, positionMs) + elapsed)
  return { fromPct: Math.round((pos / durationMs) * 10_000) / 100, leftMs: playing ? Math.round(durationMs - pos) : 0 }
}

export type NowPlayingControl = 'previous' | 'play_pause' | 'next'

export interface NowPlayingStripProps {
  title: string
  artist: string
  /** The player's artwork; a ♪ tile without one. */
  artworkUrl?: string | null
  playing: boolean
  positionMs: number
  durationMs: number
  atMs: number
  /** The clock the start of the fill is computed at (the push's own time by default). */
  now?: number
  /** Draws the fill still (reduced motion). */
  reduced?: boolean
  /** The control whose command is in flight: it shows the spinner. */
  pending?: NowPlayingControl | null
  /** The strip is leaving (the player quit a minute ago, the setting went off): it folds away. */
  leaving?: boolean
  onOpen?: () => void
  onControl?: (c: NowPlayingControl) => void
  className?: string
}

export function NowPlayingStrip({
  title,
  artist,
  artworkUrl,
  playing,
  positionMs,
  durationMs,
  atMs,
  now,
  reduced,
  pending,
  leaving,
  onOpen,
  onControl,
  className,
}: NowPlayingStripProps) {
  const p = progress(positionMs, durationMs, atMs, now ?? atMs, playing)
  const still = reduced || !playing || p.leftMs === 0
  const fill = { '--np-from': `${p.fromPct}%`, '--np-left': `${p.leftMs}ms` } as CSSProperties
  const control = (id: NowPlayingControl, label: string, icon: 'skip-back' | 'play' | 'pause' | 'skip-forward') => (
    <Button
      variant="icon"
      quiet
      aria-label={label}
      title={label}
      onClick={(e) => {
        e.stopPropagation()
        if (!pending) onControl?.(id)
      }}
      data-testid={`np-${id}`}
    >
      {pending === id ? <Spinner /> : <Icon id={icon} />}
    </Button>
  )
  const open = (e: { stopPropagation(): void }) => {
    e.stopPropagation()
    onOpen?.()
  }
  return (
    <div className={cx('m-nowplaying', leaving && 'is-leaving', className)} role="group" aria-label="Now playing" data-testid="nowplaying">
      {artworkUrl ? (
        <img className="m-nowplaying-art" src={artworkUrl} alt="" onClick={open} />
      ) : (
        <span className="m-nowplaying-art" aria-hidden="true" onClick={open}>
          ♪
        </span>
      )}
      <div className="m-nowplaying-text" onClick={open}>
        <span className="m-nowplaying-title" data-testid="np-title">
          {title}
        </span>
        {artist.trim() !== '' && <span className="m-nowplaying-artist">{artist}</span>}
      </div>
      <div className="m-nowplaying-controls">
        {control('previous', 'Previous', 'skip-back')}
        {control('play_pause', playing ? 'Pause' : 'Play', playing ? 'pause' : 'play')}
        {control('next', 'Next', 'skip-forward')}
      </div>
      {durationMs > 0 && (
        <span className="m-nowplaying-bar" aria-hidden="true">
          <i key={`${atMs}:${playing}`} className={cx('m-nowplaying-fill', still && 'is-static')} style={fill} />
        </span>
      )}
    </div>
  )
}
