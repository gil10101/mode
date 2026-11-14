// Now playing in the overlay (02 §6.1–6.3, §7.1; OV-37): the strip under
// the expanded panel's header while a local player is known and
// `overlay.media` is on, and on the resting peek of every placement the
// track's artwork (dimmed while paused), or the equalizer while a track
// without artwork plays. The strip folds away (160 ms) when the
// player is gone or the setting goes off; its controls call the core and
// show the spinner while the command runs.
import { useEffect, useMemo, useRef, useState } from 'react'
import { Equalizer, NowPlayingStrip, PeekArt, type NowPlayingControl } from '../kit/nowplaying'
import type { NowPlaying as Track } from '../state/contract'
import { useOverlayStore } from '../state/overlay-store'
import { invoke } from '../tauri'
import { useOverlay } from './context'

/** The strip's leave (10 §7.4.22, `motion.medium`). */
export const LEAVE_MS = 160

const COMMAND: Record<NowPlayingControl, string> = { previous: 'media_previous', play_pause: 'media_play_pause', next: 'media_next' }

/** The track to draw while the setting is on. */
function useTrack(): Track | null {
  const media = useOverlayStore((s) => s.media)
  const on = useOverlayStore((s) => s.settings.media !== false)
  return on ? media : null
}

/** The peek's artwork and equalizer fades (10 §7.4.22, `motion.base`). */
export const EQ_LEAVE_MS = 120

/** The resting peek's equalizer: only while a track without artwork plays; it fades in, and out when the track pauses, stops or gains artwork. */
function PeekEqualizer({ playing, reduced }: { playing: boolean; reduced: boolean }) {
  const [shown, setShown] = useState<'on' | 'leaving' | null>(playing ? 'on' : null)
  useEffect(() => {
    if (playing) {
      setShown('on')
      return
    }
    if (reduced) {
      setShown(null)
      return
    }
    setShown((s) => (s ? 'leaving' : null))
    const t = setTimeout(() => setShown(null), EQ_LEAVE_MS)
    return () => clearTimeout(t)
  }, [playing, reduced])
  if (!shown) return null
  return <Equalizer playing className={shown === 'leaving' ? 'is-leaving' : undefined} />
}

interface ArtShown {
  src: string
  /** The last track's image, under the new one while it fades in. */
  previous: string | null
  leaving: boolean
}

/** The resting peek's artwork: fades in, cross-fades on a track change, fades out when the track goes or has none. */
function PeekArtwork({ src, paused, reduced }: { src: string | null; paused: boolean; reduced: boolean }) {
  const [shown, setShown] = useState<ArtShown | null>(src ? { src, previous: null, leaving: false } : null)
  useEffect(() => {
    if (src) {
      setShown((s) => ({ src, previous: !reduced && s && !s.leaving && s.src !== src ? s.src : null, leaving: false }))
      if (reduced) return
      const t = setTimeout(() => setShown((s) => (s && s.previous ? { ...s, previous: null } : s)), EQ_LEAVE_MS)
      return () => clearTimeout(t)
    }
    if (reduced) {
      setShown(null)
      return
    }
    setShown((s) => (s ? { ...s, previous: null, leaving: true } : null))
    const t = setTimeout(() => setShown(null), EQ_LEAVE_MS)
    return () => clearTimeout(t)
  }, [src, reduced])
  if (!shown) return null
  return <PeekArt src={shown.src} previous={shown.previous} paused={paused} leaving={shown.leaving} />
}

/** Now playing on the resting peek (02 §6.1–6.3): the track's artwork (dimmed while paused), or the equalizer while a track without artwork plays. */
export function PeekNowPlaying() {
  const { m } = useOverlay()
  const track = useTrack()
  const reduced = m.settings.reduced_motion
  const art = track?.artwork_url || null
  return (
    <>
      <PeekArtwork src={art} paused={!!track && !track.playing} reduced={reduced} />
      <PeekEqualizer playing={!!track?.playing && !art} reduced={reduced} />
    </>
  )
}

export function NowPlayingSlot() {
  const { runner, m } = useOverlay()
  const track = useTrack()
  const reduced = m.settings.reduced_motion
  // The last track stays drawn while the strip folds away.
  const [shown, setShown] = useState<{ track: Track; leaving: boolean } | null>(track ? { track, leaving: false } : null)
  const [pending, setPending] = useState<NowPlayingControl | null>(null)
  const leaveTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  useEffect(() => {
    clearTimeout(leaveTimer.current)
    if (track) {
      setShown({ track, leaving: false })
      return
    }
    setShown((s) => (s && !reduced ? { ...s, leaving: true } : null))
    if (!reduced) leaveTimer.current = setTimeout(() => setShown(null), LEAVE_MS)
    return () => clearTimeout(leaveTimer.current)
  }, [track, reduced])
  // The fill starts where the track is when this push (or this mount) is drawn.
  const t = shown?.track
  const drawnAt = useMemo(() => Date.now(), [t?.at_ms, t?.playing, t?.position_ms]) // eslint-disable-line react-hooks/exhaustive-deps
  if (!shown) return null
  const control = (c: NowPlayingControl) => {
    setPending(c)
    invoke(COMMAND[c])
      .catch(() => runner.dispatch({ type: 'resolved', text: "Couldn't reach the player", keepPill: true }))
      .finally(() => setPending(null))
  }
  const s = shown.track
  return (
    <NowPlayingStrip
      title={s.title}
      artist={s.artist}
      artworkUrl={s.artwork_url}
      playing={s.playing}
      positionMs={s.position_ms}
      durationMs={s.duration_ms}
      atMs={s.at_ms}
      now={drawnAt}
      reduced={reduced}
      pending={pending}
      leaving={shown.leaving}
      onOpen={() => void invoke('media_open').catch(() => {})}
      onControl={control}
    />
  )
}
