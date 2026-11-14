//! What Mode saw playing (the overlay's Music tab; 02 §7.3, §7.10; 06 §12A;
//! 09 §5.5; ADR-0066): `media_plays`, one row per play the Now playing
//! poller recorded. No player's local interface and no account gives a
//! listening history, so this table is the whole of it. Device-local
//! (class L), never synced; [`sweep_in`] keeps the newest
//! [`KEEP_PLAYS`] plays and nothing older than [`KEEP_DAYS`] days.

use rusqlite::{Connection, OptionalExtension, named_params};
use serde::Serialize;

use crate::db::Result;

/// The sweep keeps at most this many plays (09 §10).
pub const KEEP_PLAYS: i64 = 500;
/// And nothing older than this.
pub const KEEP_DAYS: i64 = 90;
/// Rows per section the overlay is sent (it draws at most `overlay.max_rows` in all).
pub const SECTION_LIMIT: usize = 10;

/// One play to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Play {
    pub app: String,
    pub track_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork_url: Option<String>,
    pub duration_ms: u64,
}

/// A distinct track, at its newest play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrackRow {
    /// The newest play's id (`media_play` takes it).
    pub id: String,
    pub app: String,
    pub track_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub artwork_url: Option<String>,
    pub duration_ms: u64,
    pub played_at: String,
    /// Plays of this track in the history.
    pub plays: i64,
}

/// A distinct album, at its newest play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlbumRow {
    /// The album's most recently heard track's play id (an album row plays that track).
    pub id: String,
    pub app: String,
    pub album: String,
    pub artist: String,
    pub artwork_url: Option<String>,
    /// Distinct tracks of the album heard.
    pub tracks: i64,
    pub played_at: String,
}

/// The `media_history` answer and the `media:history` payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct History {
    pub tracks: Vec<TrackRow>,
    pub albums: Vec<AlbumRow>,
}

/// What makes two plays the same track: the player's id when it gives one,
/// else the title, artist and album (lowercased), per player.
const TRACK_KEY: &str = "app || char(31) || COALESCE(track_id, lower(title) || char(31) || lower(artist) || char(31) || lower(album))";
/// What makes two plays the same album.
const ALBUM_KEY: &str = "lower(album) || char(31) || lower(artist)";

/// Insert one play at `played_at` (RFC 3339); its new id.
pub fn record_in(conn: &Connection, play: &Play, played_at: &str) -> Result<String> {
    let id = crate::repo::new_id();
    conn.execute(
        "INSERT INTO media_plays (id, app, track_id, title, artist, album, artwork_url, duration_ms, played_at)
         VALUES (:id, :app, :track_id, :title, :artist, :album, :art, :dur, :at)",
        named_params! {
            ":id": id,
            ":app": play.app,
            ":track_id": play.track_id.as_deref().filter(|s| !s.trim().is_empty()),
            ":title": play.title,
            ":artist": play.artist,
            ":album": play.album,
            ":art": play.artwork_url,
            ":dur": play.duration_ms as i64,
            ":at": played_at,
        },
    )?;
    Ok(id)
}

fn track(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrackRow> {
    Ok(TrackRow {
        id: r.get("id")?,
        app: r.get("app")?,
        track_id: r.get("track_id")?,
        title: r.get("title")?,
        artist: r.get("artist")?,
        album: r.get("album")?,
        artwork_url: r.get("artwork_url")?,
        duration_ms: r.get::<_, i64>("duration_ms")?.max(0) as u64,
        played_at: r.get("played_at")?,
        plays: r.get("plays")?,
    })
}

/// Distinct tracks and distinct albums, newest first, `limit` of each.
pub fn history_in(conn: &Connection, limit: usize) -> Result<History> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, app, track_id, title, artist, album, artwork_url, duration_ms, played_at, plays FROM (
            SELECT *, ROW_NUMBER() OVER (PARTITION BY k ORDER BY played_at DESC, id DESC) AS rn,
                   COUNT(*) OVER (PARTITION BY k) AS plays
              FROM (SELECT *, {TRACK_KEY} AS k FROM media_plays))
          WHERE rn = 1 ORDER BY played_at DESC, id DESC LIMIT :limit"
    ))?;
    let tracks = stmt
        .query_map(named_params! { ":limit": limit as i64 }, track)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // SQLite has no DISTINCT in window aggregates: the distinct tracks per album are counted apart.
    let mut stmt = conn.prepare(&format!(
        "WITH p AS (SELECT *, {ALBUM_KEY} AS a, {TRACK_KEY} AS k FROM media_plays WHERE trim(album) <> ''),
              n AS (SELECT a, COUNT(DISTINCT k) AS tracks FROM p GROUP BY a),
              newest AS (SELECT *, ROW_NUMBER() OVER (PARTITION BY a ORDER BY played_at DESC, id DESC) AS rn FROM p)
         SELECT newest.id, newest.app, newest.album, newest.artist, newest.artwork_url, newest.played_at, n.tracks
           FROM newest JOIN n ON n.a = newest.a
          WHERE newest.rn = 1 ORDER BY newest.played_at DESC, newest.id DESC LIMIT :limit"
    ))?;
    let albums = stmt
        .query_map(named_params! { ":limit": limit as i64 }, |r| {
            Ok(AlbumRow {
                id: r.get(0)?,
                app: r.get(1)?,
                album: r.get(2)?,
                artist: r.get(3)?,
                artwork_url: r.get(4)?,
                played_at: r.get(5)?,
                tracks: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(History { tracks, albums })
}

/// One play by id (what a row of the tab plays).
pub fn get_in(conn: &Connection, id: &str) -> Result<Option<TrackRow>> {
    Ok(conn
        .query_row(
            "SELECT id, app, track_id, title, artist, album, artwork_url, duration_ms, played_at, 1 AS plays
               FROM media_plays WHERE id = ?1",
            [id],
            track,
        )
        .optional()?)
}

/// "Clear history" (03 §8.3): every play; how many there were.
pub fn clear_in(conn: &Connection) -> Result<usize> {
    Ok(conn.execute("DELETE FROM media_plays", [])?)
}

/// The retention rule (09 §10): plays older than `days` before `now`, then
/// all but the newest `keep`; how many were deleted.
pub fn sweep_in(conn: &Connection, now: &str, days: i64, keep: i64) -> Result<usize> {
    let mut n = conn.execute(
        "DELETE FROM media_plays WHERE played_at < strftime('%Y-%m-%dT%H:%M:%fZ', :now, :shift)",
        named_params! { ":now": now, ":shift": format!("-{days} days") },
    )?;
    n += conn.execute(
        "DELETE FROM media_plays WHERE id IN (
            SELECT id FROM media_plays ORDER BY played_at DESC, id DESC LIMIT -1 OFFSET :keep)",
        named_params! { ":keep": keep },
    )?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(include_str!("../../../migrations/0009_media_plays.sql"))
            .unwrap();
        c
    }

    fn play(title: &str, album: &str, id: Option<&str>) -> Play {
        Play {
            app: "Spotify".into(),
            track_id: id.map(str::to_string),
            title: title.into(),
            artist: "The Eragny Quartet".into(),
            album: album.into(),
            artwork_url: Some("https://i.scdn.co/image/ab67".into()),
            duration_ms: 222_000,
        }
    }

    fn at(min: u32) -> String {
        format!("2026-11-14T10:{min:02}:00.000Z")
    }

    #[test]
    fn the_history_lists_distinct_tracks_and_albums_newest_first() {
        let c = db();
        record_in(
            &c,
            &play("Northeaster", "Harbour Lights", Some("spotify:track:1")),
            &at(1),
        )
        .unwrap();
        record_in(
            &c,
            &play("Harbour Lights", "Harbour Lights", Some("spotify:track:2")),
            &at(5),
        )
        .unwrap();
        record_in(&c, &play("Poplars", "Autumn", None), &at(8)).unwrap();
        // The first track again: one row, at its newest play, two plays.
        let again = record_in(
            &c,
            &play("Northeaster", "Harbour Lights", Some("spotify:track:1")),
            &at(12),
        )
        .unwrap();
        // A play without an album lists as a track only.
        record_in(&c, &play("Advertisement", "", None), &at(13)).unwrap();

        let h = history_in(&c, 10).unwrap();
        let titles: Vec<_> = h.tracks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Advertisement", "Northeaster", "Poplars", "Harbour Lights"]
        );
        assert_eq!(h.tracks[1].id, again);
        assert_eq!(h.tracks[1].plays, 2);
        let albums: Vec<_> = h
            .albums
            .iter()
            .map(|a| (a.album.as_str(), a.tracks))
            .collect();
        assert_eq!(albums, [("Harbour Lights", 2), ("Autumn", 1)]);
        // An album row carries its most recently heard track's play.
        assert_eq!(h.albums[0].id, again);
        assert_eq!(history_in(&c, 2).unwrap().tracks.len(), 2);
        assert_eq!(
            get_in(&c, &again).unwrap().unwrap().track_id.as_deref(),
            Some("spotify:track:1")
        );
        assert_eq!(get_in(&c, "nope").unwrap(), None);
    }

    #[test]
    fn tracks_without_an_id_match_by_title_artist_and_album() {
        let c = db();
        record_in(&c, &play("Song", "Album", None), &at(1)).unwrap();
        record_in(&c, &play("song", "ALBUM", Some(" ")), &at(2)).unwrap();
        let h = history_in(&c, 10).unwrap();
        assert_eq!(h.tracks.len(), 1);
        assert_eq!(h.tracks[0].plays, 2);
        assert_eq!(h.tracks[0].track_id, None, "a blank id is stored as none");
    }

    #[test]
    fn the_sweep_keeps_ninety_days_and_the_newest_plays() {
        let c = db();
        record_in(&c, &play("Old", "A", None), "2026-08-01T00:00:00.000Z").unwrap();
        for m in 0..5 {
            record_in(&c, &play(&format!("T{m}"), "A", None), &at(m)).unwrap();
        }
        assert_eq!(sweep_in(&c, "2026-11-14T12:00:00.000Z", 90, 3).unwrap(), 3);
        let left: Vec<String> = c
            .prepare("SELECT title FROM media_plays ORDER BY played_at")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(left, ["T2", "T3", "T4"]);
        assert_eq!(clear_in(&c).unwrap(), 3);
        assert_eq!(history_in(&c, 10).unwrap(), History::default());
    }
}
