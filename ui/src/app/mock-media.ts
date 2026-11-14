// The browser mock of Now playing (02 §7.1, OV-37), installed by both windows'
// entry points outside Tauri: the `media_*` commands and the `media.changed`
// pushes the core's `mode-media` poller would make. Nothing plays until a test
// (or the dev console) calls `window.__mode_mock.media.play()`; the sample
// track is invented and its artwork is a crop of a public-domain painting.
import albumEragny from '../../../assets/sample/album-eragny.png'
import albumPoplars from '../../../assets/sample/album-poplars.png'
import albumSheep from '../../../assets/sample/album-sheep.png'
import albumThunderstorm from '../../../assets/sample/album-thunderstorm.png'
import albumWaterfall from '../../../assets/sample/album-waterfall.png'
import { nowPlayingArt } from '../kit/mascot'
import { EVENTS, type MediaAlbumRow, type MediaChanged, type MediaHistory, type MediaTrackRow, type NowPlaying } from '../state/contract'
import { mock, type ModeMock } from '../tauri'

/** The sample moment: 1:05 into a 3:42 track. */
export const SAMPLE_TRACKS: readonly Omit<NowPlaying, 'at_ms' | 'playing'>[] = [
  { app: 'Spotify', title: 'Northeaster', artist: 'The Eragny Quartet', album: 'Harbour Lights', artwork_url: nowPlayingArt, position_ms: 65_000, duration_ms: 222_000 },
  { app: 'Spotify', title: 'Harbour Lights', artist: 'The Eragny Quartet', album: 'Harbour Lights', artwork_url: null, position_ms: 0, duration_ms: 200_000 },
]

type SamplePlay = Omit<MediaTrackRow, 'id' | 'played_at' | 'plays' | 'duration_ms'> & { ago_min: number }

/** The Music tab's sample history (02 §7.10), newest first: invented titles and artists, artwork cropped from public-domain paintings
 * (scripts/make-sample-artwork.py); the second track has no artwork, like the mock's second track. */
export const SAMPLE_PLAYS: readonly SamplePlay[] = [
  { app: 'Spotify', track_id: 'spotify:track:mock-northeaster', title: 'Northeaster', artist: 'The Eragny Quartet', album: 'Harbour Lights', artwork_url: nowPlayingArt, ago_min: 2 },
  { app: 'Spotify', track_id: 'spotify:track:mock-harbour', title: 'Harbour Lights', artist: 'The Eragny Quartet', album: 'Harbour Lights', artwork_url: null, ago_min: 6 },
  { app: 'Spotify', track_id: 'spotify:track:mock-poplars', title: 'Autumn, Poplars', artist: 'Lucien and the Orchard', album: 'Poplars', artwork_url: albumPoplars, ago_min: 41 },
  { app: 'Music', track_id: 'A1B2C3D4E5F60718', title: 'Rocky Pasture', artist: 'Field Recording Society', album: 'Sheep in the Hills', artwork_url: albumSheep, ago_min: 190 },
  { app: 'Spotify', track_id: 'spotify:track:mock-morning', title: 'Morning at Éragny', artist: 'Lucien and the Orchard', album: 'Poplars', artwork_url: albumEragny, ago_min: 1500 },
  { app: 'Music', track_id: 'B2C3D4E5F6071829', title: 'Alpine Thunder', artist: 'Margit Ferrand', album: 'Weather Studies', artwork_url: albumThunderstorm, ago_min: 3000 },
  { app: 'Spotify', track_id: 'spotify:track:mock-waterfall', title: 'At the Waterfall', artist: 'The Claypoole Trio', album: 'Falling Water', artwork_url: albumWaterfall, ago_min: 4400 },
]

/** The history the core would answer for `SAMPLE_PLAYS` at `now`: distinct tracks and albums, newest first (each play here is distinct). */
export function sampleHistory(now: number): MediaHistory {
  const iso = (min: number) => new Date(now - min * 60_000).toISOString()
  const tracks: MediaTrackRow[] = SAMPLE_PLAYS.map((p, i) => {
    const { ago_min, ...rest } = p
    return { ...rest, id: `play-${i + 1}`, duration_ms: 200_000, played_at: iso(ago_min), plays: 1 }
  })
  const albums: MediaAlbumRow[] = []
  for (const t of tracks) {
    const key = `${t.album}\u001f${t.artist}`.toLowerCase()
    const seen = albums.find((a) => `${a.album}\u001f${a.artist}`.toLowerCase() === key)
    if (seen) seen.tracks += 1
    else albums.push({ id: t.id, app: t.app, album: t.album, artist: t.artist, artwork_url: t.artwork_url, tracks: 1, played_at: t.played_at })
  }
  return { tracks, albums }
}

export interface MediaMock {
  /** The track the windows were last told about (null: no player). */
  track: NowPlaying | null
  /** Start the player on a sample track (or the given fields) and push it. */
  play(over?: Partial<NowPlaying>): NowPlaying
  /** The player quits: `{}` is pushed. */
  quit(): void
  /** macOS refused the Automation permission. */
  deny(): void
  /** The Music tab's history (the sample until a test sets another). */
  history: MediaHistory
  /** Replace the history and push `media.history`, as the core does after a recorded play or a clear. */
  setHistory(h: MediaHistory): void
}

declare module '../tauri' {
  interface ModeMock {
    media?: MediaMock
  }
}

export function installMediaMock(m: ModeMock = mock(), now: () => number = Date.now): MediaMock {
  let index = 0
  const push = (p: MediaChanged) => void m.emit(EVENTS.media, p)
  // The position the player is at now, from the last push.
  const position = (t: NowPlaying) => Math.min(t.duration_ms, t.position_ms + (t.playing ? now() - t.at_ms : 0))
  const media: MediaMock = {
    track: null,
    play(over = {}) {
      const t: NowPlaying = { ...SAMPLE_TRACKS[index], playing: true, at_ms: now(), ...over }
      media.track = t
      push(t)
      return t
    },
    quit() {
      media.track = null
      push({})
    },
    deny() {
      media.track = null
      push({ permission: 'denied' })
    },
    history: sampleHistory(now()),
    setHistory(h) {
      media.history = h
      void m.emit(EVENTS.mediaHistory, h)
    },
  }
  const need = () => {
    if (!media.track) throw new Error('no player is running')
    return media.track
  }
  m.respond('media_play_pause', () => {
    const t = need()
    media.play({ ...t, position_ms: position(t), playing: !t.playing, at_ms: now() })
  })
  m.respond('media_next', () => {
    need()
    index = (index + 1) % SAMPLE_TRACKS.length
    media.play()
  })
  m.respond('media_previous', () => {
    const t = need()
    media.play({ ...t, position_ms: 0, at_ms: now() })
  })
  m.respond('media_open', () => {
    need()
  })
  m.respond('media_history', () => media.history)
  // A row plays its track in the player it came from (the mock's player starts it at once).
  m.respond('media_play', (args) => {
    const id = (args as { id?: string } | undefined)?.id
    const row = media.history.tracks.find((t) => t.id === id)
    if (!row) throw new Error('That track is no longer in the history')
    media.play({ app: row.app, title: row.title, artist: row.artist, album: row.album, artwork_url: row.artwork_url, track_id: row.track_id, position_ms: 0, duration_ms: row.duration_ms })
    return 'played'
  })
  m.respond('media_history_clear', () => {
    media.setHistory({ tracks: [], albums: [] })
  })
  m.media = media
  return media
}
