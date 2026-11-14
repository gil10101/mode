//! Now playing (OV-37, ADR-0065; 06 §12A, 02 §6.1, §7.1): the local music
//! player's track, read and controlled through the OS, never a streaming
//! account. One thread, `mode-media`, owns the player: it polls at the
//! cadence of 08 §10.2, pushes `media:changed` to both windows when the
//! track, the state or the app changes (and every 10 s while playing), and
//! runs the strip's commands in between, so the OS scripting layer is only
//! ever touched from that thread.
//!
//! The parsers ([`from_script`], [`from_mpris`]), the diffing ([`Tracker`])
//! and the cadence ([`cadence`]) are pure and tested on every OS; the
//! per-OS adapters (`macos`, `smtc` on Windows, `mpris` on Linux) only fetch and send.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mode_core::push::Event;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::overlay::Overlay;
use crate::state::CoreState;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod mpris;
#[cfg(windows)]
mod smtc;

/// The settings key (02 §17).
pub const SETTING: &str = "overlay.media";
/// While the overlay is on screen and a player runs (08 §10.2).
pub const FAST: Duration = Duration::from_secs(1);
/// No player, or the overlay off screen.
pub const SLOW: Duration = Duration::from_secs(5);
/// After macOS refused the Automation permission: try again this often.
pub const DENIED_RETRY: Duration = Duration::from_secs(60);
/// A push at least this often while playing, so the progress re-aims.
pub const HEARTBEAT_MS: u64 = 10_000;
/// A position this far from its interpolation is a seek.
pub const SEEK_MS: u64 = 2_000;
/// The strip keeps the last track this long after the player quits (02 §7.1);
/// a player that is running but stopped clears it at once.
pub const GONE_HOLD_MS: u64 = 60_000;
/// How long a command waits for the media thread.
const COMMAND_WAIT: Duration = Duration::from_secs(4);
/// A player updates its state a moment after a command; the snapshot that
/// follows the command waits this long.
const AFTER_COMMAND: Duration = Duration::from_millis(150);

/// The local player's current track (`ui/src/state/contract.ts` `NowPlaying`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NowPlaying {
    /// The player's name as the user knows it: `Spotify`, `Music`, a Windows
    /// source app, an MPRIS identity.
    pub app: String,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// The player's id for the track when it gives one (`spotify:track:…`,
    /// a Music persistent ID, an MPRIS `mpris:trackid`); what a Music-tab row
    /// plays (02 §7.10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<String>,
    /// An `https:` URL (Spotify) or a `data:` URL (a Windows thumbnail); none
    /// when the player gives no artwork the webview can load.
    pub artwork_url: Option<String>,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// The epoch ms at which `position_ms` was read.
    pub at_ms: u64,
}

/// The `media:changed` payload: the track's fields, or none (`{}`), plus
/// `permission: "denied"` when macOS refused the Automation permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct MediaChanged {
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub now: Option<NowPlaying>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    /// The OS refused (macOS Automation, −1743).
    Denied,
    /// Anything else; the last track is kept and the next poll tries again.
    Failed(String),
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::Denied => f.write_str("the system refused access to the player"),
            MediaError::Failed(m) => f.write_str(m),
        }
    }
}

/// What one look at the players found (02 §7.1 Lifetime).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapshot {
    /// A track, playing or paused.
    Track(NowPlaying),
    /// A player runs but holds no track (stopped, or nothing loaded): the
    /// strip leaves at once.
    Stopped,
    /// No player runs: the strip holds the last track for a minute.
    Gone,
}

/// One OS adapter. Only the `mode-media` thread calls it.
pub trait Player {
    /// The current track, a running player without one, or no player.
    fn snapshot(&mut self) -> Result<Snapshot, MediaError>;
    fn play_pause(&mut self) -> Result<(), MediaError>;
    fn next(&mut self) -> Result<(), MediaError>;
    fn previous(&mut self) -> Result<(), MediaError>;
    /// Bring the player to the front.
    fn open(&mut self) -> Result<(), MediaError>;
    /// Play a track the Music tab lists (02 §7.10) in the player it came
    /// from. Where the OS gives no way to start a track by its id (Windows,
    /// Linux), or the row has no id, the player is brought to the front instead.
    fn play_item(&mut self, app: &str, track_id: Option<&str>) -> Result<PlayOutcome, MediaError> {
        let _ = (app, track_id);
        self.open().map(|_| PlayOutcome::Opened)
    }
}

/// What activating a Music-tab row did (`media_play`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayOutcome {
    /// The player started the track.
    Played,
    /// The player came to the front; the track was not started.
    Opened,
}

#[cfg(target_os = "macos")]
use macos::AppleEvents as OsPlayer;
#[cfg(not(any(target_os = "macos", windows)))]
use mpris::Mpris as OsPlayer;
#[cfg(windows)]
use smtc::Smtc as OsPlayer;

/// This OS's adapter.
pub fn player() -> Box<dyn Player> {
    Box::new(OsPlayer::default())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ------------------------------------------------------------------ parsing

/// The field separator of the macOS script's one-line answer (ASCII unit
/// separator, which no title contains).
pub const SEP: char = '\u{1f}';

/// A millisecond count as AppleScript prints it: an integer, or a real in
/// either decimal notation (the user's locale decides `.` or `,`).
fn parse_ms(s: &str) -> u64 {
    let t = s.trim().replace(',', ".");
    t.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| v.round() as u64)
        .unwrap_or(0)
}

/// The macOS script's answer, `state␟title␟artist␟album␟artwork␟duration_ms␟position_ms␟track_id`:
/// an empty answer when the player is not running, `stopped` when it holds
/// no track (stopped, or `current track` could not be read).
pub fn from_script(app: &str, out: &str, at_ms: u64) -> Snapshot {
    let out = out.trim_end_matches(['\n', '\r']);
    if out.trim().is_empty() {
        return Snapshot::Gone;
    }
    let f: Vec<&str> = out.split(SEP).collect();
    let playing = match f.first().map(|s| s.trim()) {
        Some("playing" | "fast forwarding" | "rewinding") => true,
        Some("paused") => false,
        _ => return Snapshot::Stopped,
    };
    let get = |i: usize| f.get(i).map(|s| s.trim()).unwrap_or("");
    let title = get(1);
    if title.is_empty() {
        return Snapshot::Stopped;
    }
    let art = get(4);
    Snapshot::Track(NowPlaying {
        app: app.to_string(),
        playing,
        title: title.to_string(),
        artist: get(2).to_string(),
        album: get(3).to_string(),
        track_id: Some(get(7)).filter(|t| !t.is_empty()).map(str::to_string),
        artwork_url: loadable_art(art),
        position_ms: parse_ms(get(6)),
        duration_ms: parse_ms(get(5)),
        at_ms,
    })
}

/// Artwork the webview can load (the CSP allows `https:` artwork hosts and
/// `data:`); a `file://` path or anything else shows the ♪ tile instead.
fn loadable_art(url: &str) -> Option<String> {
    let u = url.trim();
    (u.starts_with("https://") || u.starts_with("data:image/")).then(|| u.to_string())
}

/// An MPRIS property value, as far as Now playing needs one.
#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    Str(String),
    Strs(Vec<String>),
    Int(i64),
}

pub type Props = BTreeMap<String, Prop>;

fn prop_str(p: Option<&Prop>) -> String {
    match p {
        Some(Prop::Str(s)) => s.clone(),
        Some(Prop::Strs(v)) => v.join(", "),
        _ => String::new(),
    }
}

/// `org.mpris.MediaPlayer2.Player`'s `PlaybackStatus`, `Metadata` and
/// `Position` (µs) as one track.
pub fn from_mpris(
    app: &str,
    status: &str,
    metadata: &Props,
    position_us: i64,
    at_ms: u64,
) -> Option<NowPlaying> {
    let playing = match status {
        "Playing" => true,
        "Paused" => false,
        _ => return None,
    };
    let title = prop_str(metadata.get("xesam:title"));
    if title.trim().is_empty() {
        return None;
    }
    let length_us = match metadata.get("mpris:length") {
        Some(Prop::Int(n)) => *n,
        _ => 0,
    };
    Some(NowPlaying {
        app: app.to_string(),
        playing,
        title: title.trim().to_string(),
        artist: prop_str(metadata.get("xesam:artist")).trim().to_string(),
        album: prop_str(metadata.get("xesam:album")).trim().to_string(),
        track_id: Some(prop_str(metadata.get("mpris:trackid")))
            .map(|t| t.trim().to_string())
            .filter(|t| {
                !t.is_empty() && !t.starts_with("/org/mpris/MediaPlayer2/TrackList/NoTrack")
            }),
        artwork_url: loadable_art(&prop_str(metadata.get("mpris:artUrl"))),
        position_ms: (position_us.max(0) / 1000) as u64,
        duration_ms: (length_us.max(0) / 1000) as u64,
        at_ms,
    })
}

/// A Windows source app id as a name: `Spotify.exe` → `Spotify`,
/// `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify` → `Spotify`,
/// `Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic` → `ZuneMusic`.
pub fn app_name(source: &str) -> String {
    let s = source.rsplit('!').next().unwrap_or(source);
    let s = s.rsplit(['\\', '/']).next().unwrap_or(s);
    let s = s
        .strip_suffix(".exe")
        .or_else(|| s.strip_suffix(".EXE"))
        .unwrap_or(s);
    let s = s.rsplit('.').next().unwrap_or(s);
    if s.is_empty() {
        "Media".into()
    } else {
        s.to_string()
    }
}

/// Standard base64 with padding (a Windows thumbnail as a `data:` URL).
pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ------------------------------------------------------- cadence and diffs

/// How long the poller waits before the next snapshot (08 §10.2), or `None`
/// when it is stopped (`overlay.media` off, quiet hours). With `notified`
/// the players announce their changes (macOS distributed notifications wake
/// the poller, 08 §6.2), so the slow poll is only the fallback that catches
/// a seek or a missed notification.
pub fn cadence(
    enabled: bool,
    quiet: bool,
    overlay_shown: bool,
    player_running: bool,
    denied: bool,
    notified: bool,
) -> Option<Duration> {
    if !enabled || quiet {
        return None;
    }
    if denied {
        return Some(DENIED_RETRY);
    }
    Some(if overlay_shown && player_running && !notified {
        FAST
    } else {
        SLOW
    })
}

/// Whether `next` is news against what the windows were last told: another
/// track, app, state or artwork, or a position off its interpolation (a seek).
/// A track of unknown length (`duration_ms` 0: a stream, an ad) draws no
/// progress, and its position may stand still while it plays, so its
/// position is never news.
pub fn differs(prev: &NowPlaying, next: &NowPlaying) -> bool {
    if prev.app != next.app
        || prev.playing != next.playing
        || prev.title != next.title
        || prev.artist != next.artist
        || prev.album != next.album
        || prev.track_id != next.track_id
        || prev.artwork_url != next.artwork_url
        || prev.duration_ms != next.duration_ms
    {
        return true;
    }
    if next.duration_ms == 0 {
        return false;
    }
    let expected = if prev.playing {
        prev.position_ms + next.at_ms.saturating_sub(prev.at_ms)
    } else {
        prev.position_ms
    };
    expected.abs_diff(next.position_ms) > SEEK_MS
}

/// What the windows know, and what the next poll result means for them.
#[derive(Debug, Default)]
pub struct Tracker {
    /// The last payload pushed (`None` before the first).
    sent: Option<MediaChanged>,
    sent_at_ms: u64,
    /// The last track a player reported.
    last: Option<NowPlaying>,
    /// When the player was last seen gone, while the strip holds its track.
    gone_since_ms: Option<u64>,
    denied: bool,
}

impl Tracker {
    pub fn denied(&self) -> bool {
        self.denied
    }

    /// Whether a player is known (running, or quit less than a minute ago).
    pub fn player_known(&self) -> bool {
        self.last.is_some() && self.gone_since_ms.is_none()
    }

    /// The source of the last track, for the commands.
    pub fn last_app(&self) -> Option<&str> {
        self.last.as_ref().map(|n| n.app.as_str())
    }

    fn push(&mut self, p: MediaChanged, now_ms: u64) -> Option<MediaChanged> {
        self.sent = Some(p.clone());
        self.sent_at_ms = now_ms;
        Some(p)
    }

    /// One poll result at `now_ms`: the payload to push, if any.
    pub fn step(
        &mut self,
        now_ms: u64,
        result: Result<Snapshot, MediaError>,
    ) -> Option<MediaChanged> {
        match result {
            Err(MediaError::Denied) => {
                self.denied = true;
                self.last = None;
                self.gone_since_ms = None;
                let p = MediaChanged {
                    now: None,
                    permission: Some("denied"),
                };
                if self.sent.as_ref() == Some(&p) {
                    return None;
                }
                self.push(p, now_ms)
            }
            // A transient failure (the player busy, a timeout): keep what shows.
            Err(MediaError::Failed(_)) => None,
            Ok(Snapshot::Track(np)) => {
                self.denied = false;
                self.gone_since_ms = None;
                self.last = Some(np.clone());
                let news = match self.sent.as_ref().and_then(|s| s.now.as_ref()) {
                    Some(prev) => {
                        differs(prev, &np)
                            || (np.playing
                                && now_ms.saturating_sub(self.sent_at_ms) >= HEARTBEAT_MS)
                    }
                    None => true,
                };
                if !news {
                    return None;
                }
                self.push(
                    MediaChanged {
                        now: Some(np),
                        permission: None,
                    },
                    now_ms,
                )
            }
            // A running player with no track: nothing to play, so nothing to hold.
            Ok(Snapshot::Stopped) => {
                self.denied = false;
                self.last = None;
                self.gone_since_ms = None;
                self.clear(now_ms)
            }
            Ok(Snapshot::Gone) => {
                self.denied = false;
                let Some(last) = self.last.clone() else {
                    return self.clear(now_ms);
                };
                let since = *self.gone_since_ms.get_or_insert(now_ms);
                if now_ms.saturating_sub(since) >= GONE_HOLD_MS {
                    self.last = None;
                    self.gone_since_ms = None;
                    return self.clear(now_ms);
                }
                // Held paused where it stopped.
                let held = NowPlaying {
                    playing: false,
                    ..last
                };
                let same = self
                    .sent
                    .as_ref()
                    .and_then(|s| s.now.as_ref())
                    .is_some_and(|prev| !prev.playing && prev.title == held.title);
                if same {
                    return None;
                }
                self.push(
                    MediaChanged {
                        now: Some(held),
                        permission: None,
                    },
                    now_ms,
                )
            }
        }
    }

    /// The poller stopped (`overlay.media` off, quiet hours): `{}` once if
    /// anything showed.
    pub fn stop(&mut self, now_ms: u64) -> Option<MediaChanged> {
        self.last = None;
        self.gone_since_ms = None;
        self.denied = false;
        self.clear(now_ms)
    }

    fn clear(&mut self, now_ms: u64) -> Option<MediaChanged> {
        let empty = MediaChanged::default();
        if self.sent.as_ref().is_none_or(|s| *s == empty) {
            // Nothing showed; the first poll after launch says so once.
            if self.sent.is_none() {
                return self.push(empty, now_ms);
            }
            return None;
        }
        self.push(empty, now_ms)
    }
}

// ------------------------------------------------------------- failure log

/// What the poller writes to the log about failed snapshots (08 §10.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogLine {
    /// `warn`: a failure the log has not shown yet (the first, or a new message).
    Failed(String),
    /// `info`: snapshots work again after failing.
    Recovered,
}

/// Keeps a failing snapshot visible without repeating it every poll: the
/// first failure and every change of message are written, a repeat is not,
/// and the first success after a failure is written once.
#[derive(Debug, Default)]
pub struct FailureLog {
    last: Option<String>,
}

impl FailureLog {
    /// One snapshot's outcome: its failure message, or `None` when it worked.
    pub fn observe(&mut self, failure: Option<&str>) -> Option<LogLine> {
        match failure {
            Some(m) if self.last.as_deref() == Some(m) => None,
            Some(m) => {
                self.last = Some(m.to_string());
                Some(LogLine::Failed(m.to_string()))
            }
            None => self.last.take().map(|_| LogLine::Recovered),
        }
    }
}

// --------------------------------------------------------- listening history

/// A track counts as played once it has been heard playing this long (02 §7.10).
pub const PLAYED_MS: u64 = 10_000;
/// The longest gap between two looks that counts as listening: the slow poll
/// and a little more; a machine that slept between two looks heard nothing.
const MAX_HEARD_STEP_MS: u64 = SLOW.as_millis() as u64 + 2_000;
/// A position back under this after the track was recorded is a replay.
const REPLAY_UNDER_MS: u64 = 5_000;

/// What makes two snapshots the same track: the player and its id, else the
/// title, artist and album.
fn play_key(np: &NowPlaying) -> String {
    match &np.track_id {
        Some(id) => format!("{}{SEP}{id}", np.app),
        None => format!(
            "{}{SEP}{}{SEP}{}{SEP}{}",
            np.app, np.title, np.artist, np.album
        ),
    }
}

#[derive(Debug)]
struct Listening {
    key: String,
    heard_ms: u64,
    last_ms: u64,
    last_position_ms: u64,
    playing: bool,
    recorded: bool,
}

/// Decides when a play goes into the Music tab's history (`media_plays`,
/// ADR-0066): once per play, when a track has been heard playing for
/// [`PLAYED_MS`] (paused time does not count); the same track playing on
/// is not recorded again, but the same track after another one, after the
/// player stopped, or started over from the top after it was recorded is a
/// new play.
#[derive(Debug, Default)]
pub struct PlayRecorder {
    current: Option<Listening>,
}

impl PlayRecorder {
    /// One look at the player at `now_ms`: the track, or none (stopped, gone,
    /// refused, the poller stopped). The track to record, once.
    pub fn observe(&mut self, now_ms: u64, seen: Option<&NowPlaying>) -> Option<NowPlaying> {
        let Some(np) = seen else {
            self.current = None;
            return None;
        };
        let key = play_key(np);
        let fresh = match &self.current {
            None => true,
            Some(l) if l.key != key => true,
            // Started over from the top after it was recorded (a replay, a repeat).
            Some(l) => {
                l.recorded
                    && np.position_ms < REPLAY_UNDER_MS
                    && np.position_ms + SEEK_MS < l.last_position_ms
            }
        };
        if fresh {
            self.current = Some(Listening {
                key,
                heard_ms: 0,
                last_ms: now_ms,
                last_position_ms: np.position_ms,
                playing: np.playing,
                recorded: false,
            });
            return None;
        }
        let l = self.current.as_mut()?;
        if l.playing && np.playing {
            l.heard_ms += now_ms.saturating_sub(l.last_ms).min(MAX_HEARD_STEP_MS);
        }
        l.last_ms = now_ms;
        l.last_position_ms = np.position_ms;
        l.playing = np.playing;
        if !l.recorded && l.heard_ms >= PLAYED_MS {
            l.recorded = true;
            return Some(np.clone());
        }
        None
    }
}

/// Write a play to `media_plays` and push the new history to both windows.
fn record_play(app: &AppHandle, core: &CoreState, np: &NowPlaying) {
    use mode_core::media_history::{self, Play};
    let play = Play {
        app: np.app.clone(),
        track_id: np.track_id.clone(),
        title: np.title.clone(),
        artist: np.artist.clone(),
        album: np.album.clone(),
        artwork_url: np.artwork_url.clone(),
        duration_ms: np.duration_ms,
    };
    let at = mode_core::db::now_rfc3339();
    match core
        .db
        .write(move |c| media_history::record_in(c, &play, &at))
    {
        Ok(_) => {
            tracing::info!(app = np.app.as_str(), "media.played");
            emit_history(app, core);
        }
        Err(e) => tracing::warn!(error = %e, "media play not recorded"),
    }
}

/// The Music tab's history (`media_history`).
pub fn history(core: &CoreState) -> Result<mode_core::media_history::History, String> {
    core.db
        .read(|c| mode_core::media_history::history_in(c, mode_core::media_history::SECTION_LIMIT))
        .map_err(|e| e.to_string())
}

/// Push `media:history` with the current history to both windows.
pub fn emit_history(app: &AppHandle, core: &CoreState) {
    match history(core) {
        Ok(h) => {
            if let Err(e) = app.emit(Event::MediaHistory.wire_name(), &h) {
                tracing::warn!(error = %e, "emit media.history failed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "media history unavailable"),
    }
}

// ------------------------------------------------------------------ poller

/// The strip's commands (02 §16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    Open,
}

/// A Music-tab row to play (`media_play`, 02 §7.10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayItem {
    pub app: String,
    pub track_id: Option<String>,
}

enum Msg {
    /// A setting changed, or a player announced a change: re-read the
    /// settings and poll now.
    Wake,
    Run(Command, mpsc::Sender<Result<(), String>>),
    Play(PlayItem, mpsc::Sender<Result<PlayOutcome, String>>),
}

/// The handle the commands and the settings writes talk to (Tauri state).
pub struct Media {
    tx: Mutex<mpsc::Sender<Msg>>,
}

impl Media {
    /// Poll again now (a setting the poller reads changed).
    pub fn wake(&self) {
        let _ = CoreState::lock(&self.tx).send(Msg::Wake);
    }

    /// Run `cmd` on the media thread and wait for the player's answer.
    pub async fn run(&self, cmd: Command) -> Result<(), String> {
        let (reply, answer) = mpsc::channel();
        CoreState::lock(&self.tx)
            .send(Msg::Run(cmd, reply))
            .map_err(|_| "Now playing is not running".to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            answer
                .recv_timeout(COMMAND_WAIT)
                .unwrap_or_else(|_| Err("the player did not answer".to_string()))
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

impl Media {
    /// Play a Music-tab row on the media thread and wait for the player's answer.
    pub async fn play(&self, item: PlayItem) -> Result<PlayOutcome, String> {
        let (reply, answer) = mpsc::channel();
        CoreState::lock(&self.tx)
            .send(Msg::Play(item, reply))
            .map_err(|_| "Now playing is not running".to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            answer
                .recv_timeout(COMMAND_WAIT)
                .unwrap_or_else(|_| Err("the player did not answer".to_string()))
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

fn enabled(core: &CoreState) -> bool {
    mode_core::settings::get(&core.db, SETTING)
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn quiet_hours(core: &CoreState) -> bool {
    core.connectors
        .get()
        .and_then(|c| c.quiet_state().ok())
        .is_some_and(|q| q.quiet_hours)
}

fn emit(app: &AppHandle, p: &MediaChanged) {
    tracing::info!(
        app = p.now.as_ref().map(|n| n.app.as_str()).unwrap_or("-"),
        playing = p.now.as_ref().is_some_and(|n| n.playing),
        permission = p.permission.unwrap_or("-"),
        "media.changed"
    );
    if let Err(e) = app.emit(Event::MediaChanged.wire_name(), p) {
        tracing::warn!(error = %e, "emit media.changed failed");
    }
}

fn run_command(player: &mut dyn Player, cmd: Command) -> Result<(), MediaError> {
    match cmd {
        Command::PlayPause => player.play_pause(),
        Command::Next => player.next(),
        Command::Previous => player.previous(),
        Command::Open => player.open(),
    }
}

/// Start the `mode-media` thread (after the overlay, 08 §10.2).
pub fn start(app: AppHandle, core: Arc<CoreState>, overlay: Arc<Overlay>) -> Arc<Media> {
    let (tx, rx) = mpsc::channel::<Msg>();
    // The players' own change notifications wake the poller (macOS, 08 §6.2).
    let notified = Arc::new(AtomicBool::new(false));
    #[cfg(target_os = "macos")]
    {
        let wake = Mutex::new(tx.clone());
        macos::observe_changes(
            &app,
            Box::new(move || {
                let _ = CoreState::lock(&wake).send(Msg::Wake);
            }),
            notified.clone(),
        );
    }
    let media = Arc::new(Media { tx: Mutex::new(tx) });
    let spawned = std::thread::Builder::new()
        .name("mode-media".into())
        .spawn(move || {
            let mut player = player();
            let mut tracker = Tracker::default();
            let mut failures = FailureLog::default();
            let mut recorder = PlayRecorder::default();
            loop {
                let on = enabled(&core);
                let wait = match cadence(
                    on,
                    on && quiet_hours(&core),
                    overlay.is_visible(),
                    tracker.player_known(),
                    tracker.denied(),
                    notified.load(Ordering::Relaxed),
                ) {
                    None => {
                        recorder.observe(now_ms(), None);
                        if let Some(p) = tracker.stop(now_ms()) {
                            emit(&app, &p);
                        }
                        SLOW
                    }
                    Some(wait) => {
                        let result = player.snapshot();
                        let line = match &result {
                            Err(MediaError::Failed(e)) => failures.observe(Some(e)),
                            Ok(_) => failures.observe(None),
                            // A refusal is pushed and logged as `media.changed`.
                            Err(MediaError::Denied) => None,
                        };
                        match line {
                            Some(LogLine::Failed(e)) => {
                                tracing::warn!(error = %e, "media snapshot failed")
                            }
                            Some(LogLine::Recovered) => tracing::info!("media snapshot recovered"),
                            None => {}
                        }
                        let heard = match &result {
                            Ok(Snapshot::Track(np)) => recorder.observe(now_ms(), Some(np)),
                            Ok(_) | Err(MediaError::Denied) => recorder.observe(now_ms(), None),
                            Err(MediaError::Failed(_)) => None,
                        };
                        if let Some(np) = heard {
                            record_play(&app, &core, &np);
                        }
                        if let Some(p) = tracker.step(now_ms(), result) {
                            emit(&app, &p);
                        }
                        wait
                    }
                };
                match rx.recv_timeout(wait) {
                    Ok(Msg::Wake) | Err(RecvTimeoutError::Timeout) => {}
                    Ok(Msg::Run(cmd, reply)) => {
                        let r = if enabled(&core) {
                            run_command(player.as_mut(), cmd).map_err(|e| e.to_string())
                        } else {
                            Err("Now playing is off".to_string())
                        };
                        if let Err(e) = &r {
                            tracing::info!(?cmd, error = %e, "media command failed");
                        }
                        let _ = reply.send(r);
                        std::thread::sleep(AFTER_COMMAND);
                    }
                    Ok(Msg::Play(item, reply)) => {
                        let r = if enabled(&core) {
                            player
                                .play_item(&item.app, item.track_id.as_deref())
                                .map_err(|e| e.to_string())
                        } else {
                            Err("Now playing is off".to_string())
                        };
                        if let Err(e) = &r {
                            tracing::info!(app = item.app.as_str(), error = %e, "media play failed");
                        }
                        let _ = reply.send(r);
                        std::thread::sleep(AFTER_COMMAND);
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "Now playing unavailable: the media thread did not start");
    }
    media
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn track(title: &str, playing: bool, pos: u64, at: u64) -> NowPlaying {
        NowPlaying {
            app: "Spotify".into(),
            playing,
            title: title.into(),
            artist: "The Eragny Quartet".into(),
            album: "Harbour Lights".into(),
            track_id: None,
            artwork_url: Some("https://i.scdn.co/image/ab67".into()),
            position_ms: pos,
            duration_ms: 222_000,
            at_ms: at,
        }
    }

    #[test]
    fn the_script_answer_parses_in_both_decimal_notations() {
        let s = format!(
            "playing{SEP}Northeaster{SEP}The Eragny Quartet{SEP}Harbour Lights{SEP}https://i.scdn.co/image/ab67{SEP}222000{SEP}65000"
        );
        assert_eq!(
            from_script("Spotify", &s, 5),
            Snapshot::Track(track("Northeaster", true, 65_000, 5))
        );
        // Music: a real duration, a comma locale, no artwork URL, a trailing newline.
        let m = format!("paused{SEP}Song{SEP}A{SEP}B{SEP}{SEP}222000,4{SEP}1500.6\n");
        let Snapshot::Track(np) = from_script("Music", &m, 9) else {
            panic!("a paused track")
        };
        assert!(!np.playing);
        assert_eq!((np.duration_ms, np.position_ms), (222_000, 1_501));
        assert_eq!(np.artwork_url, None);
        assert_eq!(from_script("Spotify", "", 1), Snapshot::Gone, "not running");
        assert_eq!(from_script("Spotify", "\n", 1), Snapshot::Gone);
        assert_eq!(from_script("Spotify", "stopped", 1), Snapshot::Stopped);
        assert_eq!(
            from_script("Spotify", &format!("playing{SEP}{SEP}a"), 1),
            Snapshot::Stopped,
            "no title: a running player with nothing to show"
        );
        let ff = format!("fast forwarding{SEP}T{SEP}{SEP}{SEP}file:///x.jpg{SEP}-1{SEP}x");
        let Snapshot::Track(np) = from_script("Music", &ff, 1) else {
            panic!("a playing track")
        };
        assert!(np.playing);
        assert_eq!(
            (np.artwork_url, np.duration_ms, np.position_ms),
            (None, 0, 0)
        );
    }

    /// Live Spotify: a track with no artist and a duration of 0, then a
    /// position that never moves while it plays.
    #[test]
    fn a_track_without_artist_or_length_parses_and_is_not_news_every_poll() {
        let s = format!("playing{SEP}Advertisement{SEP}{SEP}{SEP}{SEP}0{SEP}0");
        let Snapshot::Track(np) = from_script("Spotify", &s, 1_000) else {
            panic!("a playing track")
        };
        assert_eq!(
            (np.artist.as_str(), np.duration_ms, np.position_ms),
            ("", 0, 0)
        );
        let later = NowPlaying {
            at_ms: 5_000,
            ..np.clone()
        };
        assert!(
            !differs(&np, &later),
            "a stalled position of an unknown length"
        );
        // A known length still reports a seek.
        let a = track("A", true, 0, 1_000);
        let b = track("A", true, 0, 5_000);
        assert!(differs(&a, &b));
    }

    #[test]
    fn mpris_metadata_parses() {
        let mut md = Props::new();
        md.insert("xesam:title".into(), Prop::Str(" Northeaster ".into()));
        md.insert(
            "xesam:artist".into(),
            Prop::Strs(vec!["The Eragny Quartet".into(), "Guest".into()]),
        );
        md.insert("xesam:album".into(), Prop::Str("Harbour Lights".into()));
        md.insert("mpris:length".into(), Prop::Int(222_000_000));
        md.insert(
            "mpris:artUrl".into(),
            Prop::Str("https://i.scdn.co/image/ab67".into()),
        );
        let np = from_mpris("spotify", "Playing", &md, 65_000_000, 7).unwrap();
        assert_eq!(np.title, "Northeaster");
        assert_eq!(np.artist, "The Eragny Quartet, Guest");
        assert_eq!(
            (np.position_ms, np.duration_ms, np.at_ms),
            (65_000, 222_000, 7)
        );
        assert!(np.playing);
        assert_eq!(
            np.artwork_url.as_deref(),
            Some("https://i.scdn.co/image/ab67")
        );
        md.insert("mpris:artUrl".into(), Prop::Str("file:///tmp/a.png".into()));
        let np = from_mpris("vlc", "Paused", &md, -5, 7).unwrap();
        assert!(!np.playing);
        assert_eq!((np.artwork_url, np.position_ms), (None, 0));
        assert_eq!(from_mpris("vlc", "Stopped", &md, 0, 7), None);
        assert_eq!(from_mpris("vlc", "Playing", &Props::new(), 0, 7), None);
    }

    #[test]
    fn windows_source_ids_read_as_names_and_base64_pads() {
        assert_eq!(app_name("Spotify.exe"), "Spotify");
        assert_eq!(
            app_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
            "Spotify"
        );
        assert_eq!(
            app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "ZuneMusic"
        );
        assert_eq!(app_name(r"C:\Program Files\Foo\foo.exe"), "foo");
        assert_eq!(app_name(""), "Media");
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn cadence_follows_the_table() {
        assert_eq!(
            cadence(false, false, true, true, false, false),
            None,
            "setting off"
        );
        assert_eq!(
            cadence(true, true, true, true, false, false),
            None,
            "quiet hours"
        );
        assert_eq!(cadence(true, false, true, true, false, false), Some(FAST));
        assert_eq!(
            cadence(true, false, false, true, false, false),
            Some(SLOW),
            "off screen"
        );
        assert_eq!(
            cadence(true, false, true, false, false, false),
            Some(SLOW),
            "no player"
        );
        assert_eq!(
            cadence(true, false, true, true, true, false),
            Some(DENIED_RETRY)
        );
        assert_eq!(
            cadence(true, false, true, true, false, true),
            Some(SLOW),
            "the players announce their changes: the poll is only the fallback"
        );
    }

    #[test]
    fn the_payload_is_flat_or_empty() {
        let p = MediaChanged {
            now: Some(track("Northeaster", true, 65_000, 5)),
            permission: None,
        };
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            json!({
                "app": "Spotify", "playing": true, "title": "Northeaster",
                "artist": "The Eragny Quartet", "album": "Harbour Lights",
                "artwork_url": "https://i.scdn.co/image/ab67",
                "position_ms": 65000, "duration_ms": 222000, "at_ms": 5
            })
        );
        assert_eq!(
            serde_json::to_value(MediaChanged::default()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(MediaChanged {
                now: None,
                permission: Some("denied")
            })
            .unwrap(),
            json!({"permission": "denied"})
        );
    }

    #[test]
    fn the_tracker_pushes_news_heartbeats_and_holds_a_quit_player() {
        let mut t = Tracker::default();
        // First poll, nothing running: one empty push so the windows know.
        assert_eq!(t.step(0, Ok(Snapshot::Gone)), Some(MediaChanged::default()));
        assert_eq!(t.step(1_000, Ok(Snapshot::Gone)), None);
        // A track starts.
        let a = track("A", true, 0, 2_000);
        assert_eq!(
            t.step(2_000, Ok(Snapshot::Track(a.clone()))).unwrap().now,
            Some(a)
        );
        assert!(t.player_known());
        // Position advancing in step: no push.
        assert_eq!(
            t.step(3_000, Ok(Snapshot::Track(track("A", true, 1_000, 3_000)))),
            None
        );
        // A transient failure keeps what shows.
        assert_eq!(t.step(3_500, Err(MediaError::Failed("busy".into()))), None);
        // A seek (5 s ahead of the interpolation): pushed.
        assert!(
            t.step(4_000, Ok(Snapshot::Track(track("A", true, 7_000, 4_000))))
                .is_some()
        );
        // Ten seconds of play since the last push: a heartbeat.
        assert_eq!(
            t.step(
                13_000,
                Ok(Snapshot::Track(track("A", true, 16_000, 13_000)))
            ),
            None
        );
        assert!(
            t.step(
                14_000,
                Ok(Snapshot::Track(track("A", true, 17_000, 14_000)))
            )
            .is_some()
        );
        // Paused: pushed once, then quiet (no heartbeat while paused).
        assert!(
            t.step(
                15_000,
                Ok(Snapshot::Track(track("A", false, 18_000, 15_000)))
            )
            .is_some()
        );
        assert_eq!(
            t.step(
                40_000,
                Ok(Snapshot::Track(track("A", false, 18_000, 40_000)))
            ),
            None
        );
        // Next track.
        assert!(
            t.step(41_000, Ok(Snapshot::Track(track("B", true, 0, 41_000))))
                .is_some()
        );
        // The player quits: the track holds, paused, for a minute, then `{}`.
        let held = t.step(42_000, Ok(Snapshot::Gone)).unwrap().now.unwrap();
        assert!(!held.playing && held.title == "B");
        assert!(!t.player_known());
        assert_eq!(t.step(80_000, Ok(Snapshot::Gone)), None);
        assert_eq!(
            t.step(102_000, Ok(Snapshot::Gone)),
            Some(MediaChanged::default())
        );
        assert_eq!(t.step(103_000, Ok(Snapshot::Gone)), None);
        assert_eq!(t.last_app(), None);
    }

    /// A player that is running but stopped (live Spotify: `player state`
    /// stopped, no current track) clears the strip at once; only a player
    /// that quit is held.
    #[test]
    fn a_stopped_player_clears_at_once() {
        let mut t = Tracker::default();
        assert!(
            t.step(0, Ok(Snapshot::Track(track("A", true, 0, 0))))
                .is_some()
        );
        assert!(
            t.step(1_000, Ok(Snapshot::Track(track("A", false, 1_000, 1_000))))
                .is_some()
        );
        assert_eq!(
            t.step(2_000, Ok(Snapshot::Stopped)),
            Some(MediaChanged::default())
        );
        assert!(!t.player_known());
        assert_eq!(t.last_app(), None);
        assert_eq!(t.step(3_000, Ok(Snapshot::Stopped)), None);
        // Gone after stopped: nothing was held, nothing to push.
        assert_eq!(t.step(4_000, Ok(Snapshot::Gone)), None);
        // A track again.
        assert!(
            t.step(5_000, Ok(Snapshot::Track(track("B", true, 0, 5_000))))
                .is_some()
        );
    }

    #[test]
    fn a_failing_snapshot_is_logged_once_per_message_and_its_recovery_once() {
        let mut log = FailureLog::default();
        assert_eq!(log.observe(None), None, "working from the start: silent");
        let e = "AppleScript error -2741: Expected expression";
        assert_eq!(log.observe(Some(e)), Some(LogLine::Failed(e.into())));
        assert_eq!(
            log.observe(Some(e)),
            None,
            "the same message repeats: quiet"
        );
        assert_eq!(log.observe(Some(e)), None);
        let e2 = "AppleScript error -1712: timeout";
        assert_eq!(log.observe(Some(e2)), Some(LogLine::Failed(e2.into())));
        assert_eq!(log.observe(None), Some(LogLine::Recovered));
        assert_eq!(log.observe(None), None);
        // The same message after a recovery is news again.
        assert_eq!(log.observe(Some(e)), Some(LogLine::Failed(e.into())));
    }

    #[test]
    fn the_tracker_reports_a_refusal_once_and_clears_when_stopped() {
        let mut t = Tracker::default();
        assert!(
            t.step(0, Ok(Snapshot::Track(track("A", true, 0, 0))))
                .is_some()
        );
        assert_eq!(t.last_app(), Some("Spotify"));
        let denied = t.step(1_000, Err(MediaError::Denied)).unwrap();
        assert_eq!(denied.permission, Some("denied"));
        assert_eq!(denied.now, None);
        assert!(t.denied());
        assert_eq!(t.step(61_000, Err(MediaError::Denied)), None);
        // Allowed later: the track arrives without the permission field.
        let p = t
            .step(62_000, Ok(Snapshot::Track(track("A", true, 0, 62_000))))
            .unwrap();
        assert_eq!(p.permission, None);
        assert!(!t.denied());
        // `overlay.media` turned off: `{}` once.
        assert_eq!(t.stop(63_000), Some(MediaChanged::default()));
        assert_eq!(t.stop(64_000), None);
    }

    fn at(title: &str, playing: bool, pos: u64) -> NowPlaying {
        NowPlaying {
            position_ms: pos,
            ..track(title, playing, pos, 0)
        }
    }

    /// 02 §7.10: a play is recorded once it has been heard for 10 s.
    #[test]
    fn a_track_is_recorded_once_after_ten_seconds_of_play() {
        let mut r = PlayRecorder::default();
        assert_eq!(
            r.observe(0, Some(&at("A", true, 0))),
            None,
            "first look: nothing heard yet"
        );
        assert_eq!(r.observe(5_000, Some(&at("A", true, 5_000))), None);
        assert_eq!(r.observe(9_000, Some(&at("A", true, 9_000))), None, "9 s");
        let rec = r
            .observe(10_000, Some(&at("A", true, 10_000)))
            .expect("10 s heard");
        assert_eq!(rec.title, "A");
        // The same track playing on is not recorded again.
        for t in [15_000, 20_000, 120_000, 125_000] {
            assert_eq!(r.observe(t, Some(&at("A", true, t))), None);
        }
    }

    #[test]
    fn paused_time_and_a_sleeping_machine_are_not_heard() {
        let mut r = PlayRecorder::default();
        r.observe(0, Some(&at("A", true, 0)));
        r.observe(4_000, Some(&at("A", false, 4_000)));
        // Paused for a minute: nothing heard.
        assert_eq!(r.observe(64_000, Some(&at("A", false, 4_000))), None);
        r.observe(65_000, Some(&at("A", true, 4_000)));
        assert_eq!(
            r.observe(69_000, Some(&at("A", true, 8_000))),
            None,
            "8 s heard"
        );
        // A gap of an hour between two looks counts as at most one slow poll and a little.
        assert_eq!(
            r.observe(3_669_000, Some(&at("A", true, 9_000))),
            Some(at("A", true, 9_000))
        );
        let mut r = PlayRecorder::default();
        r.observe(0, Some(&at("B", true, 0)));
        assert_eq!(
            r.observe(3_600_000, Some(&at("B", true, 1_000))),
            None,
            "slept: 7 s at most"
        );
    }

    #[test]
    fn a_replay_later_counts_again() {
        let mut r = PlayRecorder::default();
        let play = |r: &mut PlayRecorder, title: &str, t0: u64| {
            let mut got = None;
            for s in 0..=3 {
                got = got.or(r.observe(t0 + s * 5_000, Some(&at(title, true, s * 5_000))));
            }
            got
        };
        assert!(play(&mut r, "A", 0).is_some());
        assert!(play(&mut r, "B", 100_000).is_some(), "another track");
        assert!(play(&mut r, "A", 200_000).is_some(), "A again after B");
        // A again after the player stopped.
        r.observe(300_000, None);
        assert!(play(&mut r, "A", 400_000).is_some());
        // A started over from the top on repeat, after it was recorded.
        r.observe(600_000, Some(&at("A", true, 200_000)));
        assert!(play(&mut r, "A", 601_000).is_some());
        // A track that changes before 10 s is never recorded.
        let mut r = PlayRecorder::default();
        r.observe(0, Some(&at("C", true, 0)));
        assert_eq!(r.observe(6_000, Some(&at("C", true, 6_000))), None);
        assert_eq!(r.observe(7_000, Some(&at("D", true, 0))), None);
        assert_eq!(r.observe(12_000, Some(&at("D", true, 5_000))), None);
    }

    #[test]
    fn the_track_id_parses_and_keys_a_play() {
        let s = format!("playing{SEP}A{SEP}B{SEP}C{SEP}{SEP}1000{SEP}0{SEP}spotify:track:42");
        let Snapshot::Track(np) = from_script("Spotify", &s, 1) else {
            panic!("a track")
        };
        assert_eq!(np.track_id.as_deref(), Some("spotify:track:42"));
        let no_id = NowPlaying {
            track_id: None,
            ..np.clone()
        };
        assert_ne!(play_key(&np), play_key(&no_id));
        assert!(differs(&np, &no_id));
        let mut md = Props::new();
        md.insert("xesam:title".into(), Prop::Str("T".into()));
        md.insert(
            "mpris:trackid".into(),
            Prop::Str("/com/spotify/track/42".into()),
        );
        assert_eq!(
            from_mpris("Spotify", "Playing", &md, 0, 0)
                .unwrap()
                .track_id
                .as_deref(),
            Some("/com/spotify/track/42")
        );
        md.insert(
            "mpris:trackid".into(),
            Prop::Str("/org/mpris/MediaPlayer2/TrackList/NoTrack".into()),
        );
        assert_eq!(
            from_mpris("Spotify", "Playing", &md, 0, 0)
                .unwrap()
                .track_id,
            None
        );
    }
}
