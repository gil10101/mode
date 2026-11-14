// The Music tab (02 §7.10, ADR-0066): what Mode itself saw playing, from the
// core's `media_plays` (no account, no player history): Recently played
// (distinct tracks) and Albums (distinct albums), newest first, inside the
// overlay's row budget. A row plays its track in the player it came from, or
// brings that player to the front where the OS cannot start a track by id.
import { useEffect, useState } from 'react'
import { GroupLabel } from '../kit/lists'
import { Spinner } from '../kit/primitives'
import type { MediaAlbumRow, MediaTrackRow } from '../state/contract'
import { useOverlayStore } from '../state/overlay-store'
import { invoke } from '../tauri'
import { useOverlay } from './context'
import { agoLabel } from './phrases'

/** How the row budget splits (02 §7.10): albums take at most half, tracks the rest, and either section gives the other what it leaves. */
export function splitRows(max: number, tracks: number, albums: number): { tracks: number; albums: number } {
  const a0 = Math.min(albums, Math.floor(max / 2))
  const t = Math.min(tracks, max - a0)
  return { tracks: t, albums: Math.min(albums, max - t) }
}

function Art({ src }: { src: string | null }) {
  return src ? (
    <img className="m-row-art" src={src} alt="" />
  ) : (
    <span className="m-row-art" aria-hidden="true">
      ♪
    </span>
  )
}

interface RowProps {
  id: string
  testId: string
  art: string | null
  name: string
  detail: string
  label: string
  meta: string
  pending: boolean
  onPlay: () => void
}

function MusicRow({ testId, art, name, detail, label, meta, pending, onPlay }: RowProps) {
  return (
    <div className="m-row is-dense is-media" role="button" aria-label={label} title={label} onClick={onPlay} data-testid={testId}>
      <Art src={art} />
      <div className="m-row-text">
        <div className="m-row-name">{name}</div>
        {detail && <div className="m-row-detail">{detail}</div>}
      </div>
      <span className="m-row-meta">{pending ? <Spinner /> : meta}</span>
    </div>
  )
}

export function MusicTab() {
  const { runner, now } = useOverlay()
  const history = useOverlayStore((s) => s.history)
  const max = useOverlayStore((s) => s.settings.max_rows)
  const [pending, setPending] = useState<string | null>(null)
  useEffect(() => {
    invoke<{ tracks: MediaTrackRow[]; albums: MediaAlbumRow[] }>('media_history')
      .then((h) => h && useOverlayStore.getState().applyHistory(h))
      .catch(() => {})
  }, [])
  const tracks = history?.tracks ?? []
  const albums = history?.albums ?? []
  const n = splitRows(max, tracks.length, albums.length)
  const play = (key: string, id: string) => {
    if (pending) return
    setPending(key)
    invoke<'played' | 'opened'>('media_play', { id })
      .catch(() => runner.dispatch({ type: 'resolved', text: "Couldn't reach the player", keepPill: true }))
      .finally(() => setPending(null))
  }
  return (
    <div className="m-list" role="list" aria-label="Music" data-testid="music-list">
      {tracks.length === 0 ? (
        <div className="ov-empty" data-testid="empty-music">
          Nothing played yet. Tracks you play in Spotify or Music show up here.
        </div>
      ) : (
        <>
          <GroupLabel>recently played</GroupLabel>
          {tracks.slice(0, n.tracks).map((t) => (
            <MusicRow
              key={`t:${t.id}`}
              id={t.id}
              testId={`music-track-${t.id}`}
              art={t.artwork_url}
              name={t.title}
              detail={t.artist}
              label={t.artist ? `Play ${t.title} by ${t.artist}` : `Play ${t.title}`}
              meta={agoLabel(t.played_at, now)}
              pending={pending === `t:${t.id}`}
              onPlay={() => play(`t:${t.id}`, t.id)}
            />
          ))}
          {n.albums > 0 && <GroupLabel>albums</GroupLabel>}
          {albums.slice(0, n.albums).map((a) => (
            <MusicRow
              key={`a:${a.id}`}
              id={a.id}
              testId={`music-album-${a.id}`}
              art={a.artwork_url}
              name={a.album}
              detail={[a.artist, `${a.tracks} ${a.tracks === 1 ? 'track' : 'tracks'} heard`].filter(Boolean).join(' · ')}
              label={`Play ${a.album}${a.artist ? ` by ${a.artist}` : ''}`}
              meta={agoLabel(a.played_at, now)}
              pending={pending === `a:${a.id}`}
              onPlay={() => play(`a:${a.id}`, a.id)}
            />
          ))}
        </>
      )}
    </div>
  )
}
