//! Now playing on Windows (08 §6.3, 06 §12A): the System Media Transport
//! Controls session manager's current session. Blocking `join()` on the
//! `mode-media` thread; the thumbnail is read once per track into a `data:`
//! URL. No permission prompt.

use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;

use super::{MediaError, NowPlaying, Player, Snapshot, app_name, base64, now_ms};

/// 100-ns ticks between 1601-01-01 and 1970-01-01, in ms.
const EPOCH_DELTA_MS: i64 = 11_644_473_600_000;
/// Thumbnails above this are not inlined.
const MAX_THUMBNAIL: u64 = 2 * 1024 * 1024;

fn failed(e: windows::core::Error) -> MediaError {
    MediaError::Failed(e.message())
}

#[derive(Default)]
pub struct Smtc {
    manager: Option<Manager>,
    /// The last track's key and its thumbnail as a `data:` URL.
    art: Option<(String, Option<String>)>,
}

impl Smtc {
    fn manager(&mut self) -> Result<&Manager, MediaError> {
        if self.manager.is_none() {
            let m = Manager::RequestAsync()
                .and_then(|op| op.join())
                .map_err(failed)?;
            self.manager = Some(m);
        }
        Ok(self.manager.as_ref().expect("set above"))
    }

    fn session(&mut self) -> Result<Option<Session>, MediaError> {
        // No current session is an error from WinRT, not a null.
        Ok(self.manager()?.GetCurrentSession().ok())
    }

    fn thumbnail(
        props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
    ) -> Option<String> {
        let reference = props.Thumbnail().ok()?;
        let stream = reference.OpenReadAsync().ok()?.join().ok()?;
        let size = stream.Size().ok()?;
        if size == 0 || size > MAX_THUMBNAIL {
            return None;
        }
        let kind = stream
            .ContentType()
            .map(|h| h.to_string())
            .unwrap_or_default();
        let kind = if kind.starts_with("image/") {
            kind
        } else {
            "image/png".to_string()
        };
        let reader = DataReader::CreateDataReader(&stream).ok()?;
        let loaded = reader.LoadAsync(size as u32).ok()?.join().ok()?;
        let mut bytes = vec![0u8; loaded as usize];
        reader.ReadBytes(&mut bytes).ok()?;
        Some(format!("data:{kind};base64,{}", base64(&bytes)))
    }
}

impl Player for Smtc {
    fn snapshot(&mut self) -> Result<Snapshot, MediaError> {
        let Some(session) = self.session()? else {
            return Ok(Snapshot::Gone);
        };
        let playing = match session
            .GetPlaybackInfo()
            .and_then(|i| i.PlaybackStatus())
            .map_err(failed)?
        {
            Status::Playing => true,
            Status::Paused => false,
            // A session that is stopped, closed or changing holds no track.
            _ => return Ok(Snapshot::Stopped),
        };
        let props = session
            .TryGetMediaPropertiesAsync()
            .and_then(|op| op.join())
            .map_err(failed)?;
        let title = props.Title().map(|h| h.to_string()).unwrap_or_default();
        if title.trim().is_empty() {
            return Ok(Snapshot::Stopped);
        }
        let artist = props.Artist().map(|h| h.to_string()).unwrap_or_default();
        let album = props
            .AlbumTitle()
            .map(|h| h.to_string())
            .unwrap_or_default();
        let key = format!("{title}\u{1f}{artist}\u{1f}{album}");
        let artwork_url = match &self.art {
            Some((k, url)) if *k == key => url.clone(),
            _ => {
                let url = Self::thumbnail(&props);
                self.art = Some((key, url.clone()));
                url
            }
        };
        let timeline = session.GetTimelineProperties().map_err(failed)?;
        let position_ms = timeline
            .Position()
            .map(|t| (t.Duration / 10_000).max(0) as u64)
            .unwrap_or(0);
        let end_ms = timeline
            .EndTime()
            .map(|t| (t.Duration / 10_000).max(0) as u64)
            .unwrap_or(0);
        // The position is as of the timeline's last update.
        let at_ms = timeline
            .LastUpdatedTime()
            .ok()
            .map(|d| d.UniversalTime / 10_000 - EPOCH_DELTA_MS)
            .filter(|ms| *ms > 0)
            .map(|ms| ms as u64)
            .unwrap_or_else(now_ms);
        let app = session
            .SourceAppUserModelId()
            .map(|h| app_name(&h.to_string()))
            .unwrap_or_else(|_| "Media".into());
        Ok(Snapshot::Track(NowPlaying {
            app,
            playing,
            title: title.trim().to_string(),
            artist: artist.trim().to_string(),
            album: album.trim().to_string(),
            // SMTC gives no track id (and no way to start a track by one).
            track_id: None,
            artwork_url,
            position_ms,
            duration_ms: end_ms,
            at_ms,
        }))
    }

    fn play_pause(&mut self) -> Result<(), MediaError> {
        let s = self
            .session()?
            .ok_or_else(|| MediaError::Failed("no player is running".into()))?;
        s.TryTogglePlayPauseAsync()
            .and_then(|op| op.join())
            .map(|_| ())
            .map_err(failed)
    }

    fn next(&mut self) -> Result<(), MediaError> {
        let s = self
            .session()?
            .ok_or_else(|| MediaError::Failed("no player is running".into()))?;
        s.TrySkipNextAsync()
            .and_then(|op| op.join())
            .map(|_| ())
            .map_err(failed)
    }

    fn previous(&mut self) -> Result<(), MediaError> {
        let s = self
            .session()?
            .ok_or_else(|| MediaError::Failed("no player is running".into()))?;
        s.TrySkipPreviousAsync()
            .and_then(|op| op.join())
            .map(|_| ())
            .map_err(failed)
    }

    fn open(&mut self) -> Result<(), MediaError> {
        // SMTC has no "raise"; the session's app is brought up through its
        // app id with the shell (a packaged app's AUMID, else the exe name).
        let s = self
            .session()?
            .ok_or_else(|| MediaError::Failed("no player is running".into()))?;
        let id = s.SourceAppUserModelId().map_err(failed)?.to_string();
        let target = if id.contains('!') {
            format!("shell:AppsFolder\\{id}")
        } else {
            id
        };
        std::process::Command::new("explorer.exe")
            .arg(target)
            .spawn()
            .map(|_| ())
            .map_err(|e| MediaError::Failed(e.to_string()))
    }
}
