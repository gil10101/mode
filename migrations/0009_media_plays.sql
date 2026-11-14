-- 0009 — Tracks Mode saw playing (the overlay's Music tab; 02 §7.3, §7.10;
-- 06 §12A; 09 §5.5; ADR-0066): the Now playing poller records a row once
-- per play, when a track has been heard playing for 10 s. No streaming
-- account and no player interface gives a listening history, so this is
-- only what Mode itself observed. Local to the device (class L), never
-- synced; the sweep keeps the newest 500 plays and nothing older than
-- 90 days; "Clear history" and "Delete everything" empty it.

CREATE TABLE media_plays (
  id          TEXT PRIMARY KEY,                                 -- UUID v7
  app         TEXT NOT NULL,                                    -- the player as the user knows it: Spotify, Music, a Windows source app, an MPRIS identity
  track_id    TEXT,                                             -- spotify:track:…, a Music persistent ID, an MPRIS track id; NULL when the player gives none
  title       TEXT NOT NULL,
  artist      TEXT NOT NULL DEFAULT '',
  album       TEXT NOT NULL DEFAULT '',
  artwork_url TEXT,                                             -- https: or data: (as the player gave it); NULL without artwork
  duration_ms INTEGER NOT NULL DEFAULT 0 CHECK (duration_ms >= 0),
  played_at   TEXT NOT NULL                                     -- when the play was recorded (10 s in)
) STRICT;

CREATE INDEX media_plays_played_at ON media_plays (played_at);
