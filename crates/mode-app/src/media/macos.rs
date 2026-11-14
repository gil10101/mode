//! Now playing on macOS (08 §6.2, 06 §12A): Apple Events through
//! `NSAppleScript` to Spotify, then Music. A player is scripted only while
//! `NSRunningApplication` lists it, and every script re-checks
//! `application id … is running` first, so Mode never launches a player.
//!
//! The AppleScript component serializes its work on the main queue (OSA
//! compiles and runs there, called from any thread), so each snapshot holds
//! the main thread for the few ms of one Apple Event round trip. The
//! Automation prompt must never be raised from there: before a player is
//! scripted, `AEDeterminePermissionToAutomateTarget` (thread-safe, no OSA)
//! asks on this `mode-media` thread, which alone waits for the user's answer.
//!
//! Both players post a distributed notification on every change of state or
//! track (`com.spotify.client.PlaybackStateChanged`, `com.apple.Music.playerInfo`);
//! [`observe_changes`] turns each into a wake of the poller, so a change
//! shows at once and the steady poll can stay slow (08 §6.2, §10.2).

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use objc2::AnyThread;
use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_app_kit::NSRunningApplication;
use objc2_foundation::{
    NSAppleEventDescriptor, NSAppleScript, NSAppleScriptErrorMessage, NSAppleScriptErrorNumber,
    NSDictionary, NSDistributedNotificationCenter, NSNotification, NSNumber, NSString,
};
use tauri::AppHandle;

use super::{MediaError, NowPlaying, PlayOutcome, Player, Snapshot, from_script, now_ms};

/// The distributed notifications the players post on a change of state or
/// track.
const CHANGE_NOTIFICATIONS: [&str; 2] = [
    "com.spotify.client.PlaybackStateChanged",
    "com.apple.Music.playerInfo",
];

/// Calls `wake` whenever a player posts one of [`CHANGE_NOTIFICATIONS`].
/// The observers are added on the main thread, whose run loop receives
/// distributed notifications; `notified` turns true once they are in place.
/// They live as long as the process.
pub fn observe_changes(
    app: &AppHandle,
    wake: Box<dyn Fn() + Send + 'static>,
    notified: Arc<AtomicBool>,
) {
    let added = app.run_on_main_thread(move || {
        let wake: Arc<dyn Fn() + Send + 'static> = Arc::from(wake);
        let center = NSDistributedNotificationCenter::defaultCenter();
        for name in CHANGE_NOTIFICATIONS {
            let wake = wake.clone();
            let block = RcBlock::new(move |_: NonNull<NSNotification>| {
                tracing::debug!(name, "media: a player announced a change");
                wake()
            });
            // SAFETY: the name is a valid NSString, no sender filter and no
            // queue (the block runs on the posting run loop, the main one);
            // the block only sends on a channel.
            let observer = unsafe {
                center.addObserverForName_object_queue_usingBlock(
                    Some(&NSString::from_str(name)),
                    None,
                    None,
                    &block,
                )
            };
            // Observed for the life of the process.
            std::mem::forget(observer);
        }
        notified.store(true, Ordering::Relaxed);
        tracing::info!("media: observing the players' change notifications");
    });
    if let Err(e) = added {
        tracing::warn!(error = %e, "media: change notifications unavailable; polling only");
    }
}

/// `errAEEventNotPermitted`: the user refused the Automation permission.
const NOT_PERMITTED: i32 = -1743;
/// `errAEEventWouldRequireUserConsent`: not asked yet.
const NEEDS_CONSENT: i32 = -1744;
/// `procNotFound`: the target is not running.
const PROC_NOT_FOUND: i32 = -600;
/// `typeApplicationBundleID` ('bund').
const TYPE_BUNDLE_ID: u32 = u32::from_be_bytes(*b"bund");
/// `typeWildCard` ('****'): any event class and id.
const WILDCARD: u32 = u32::from_be_bytes(*b"****");

/// `AEDesc` (CoreServices AE.framework).
#[repr(C)]
struct AeDesc {
    descriptor_type: u32,
    data_handle: *mut c_void,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AECreateDesc(type_code: u32, data: *const c_void, size: isize, result: *mut AeDesc) -> i16;
    fn AEDisposeDesc(desc: *mut AeDesc) -> i16;
    fn AEDeterminePermissionToAutomateTarget(
        target: *const AeDesc,
        event_class: u32,
        event_id: u32,
        ask_user_if_needed: u8,
    ) -> i32;
}

/// The Automation permission for `bundle` (0 allowed, −1743 refused, −1744
/// not asked, −600 not running); with `ask`, raises the prompt while it is
/// undecided and waits for the answer (on the calling thread).
fn permission(bundle: &str, ask: bool) -> i32 {
    let mut desc = AeDesc {
        descriptor_type: 0,
        data_handle: std::ptr::null_mut(),
    };
    // SAFETY: AECreateDesc copies `bundle`'s bytes into a new descriptor,
    // which is read by the permission call and disposed of here.
    unsafe {
        let made = AECreateDesc(
            TYPE_BUNDLE_ID,
            bundle.as_ptr().cast(),
            bundle.len() as isize,
            &mut desc,
        );
        if made != 0 {
            return i32::from(made);
        }
        let status =
            AEDeterminePermissionToAutomateTarget(&desc, WILDCARD, WILDCARD, u8::from(ask));
        AEDisposeDesc(&mut desc);
        status
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Spotify,
    Music,
}

impl Source {
    const ALL: [Source; 2] = [Source::Spotify, Source::Music];

    fn index(self) -> usize {
        self as usize
    }

    fn bundle(self) -> &'static str {
        match self {
            Source::Spotify => "com.spotify.client",
            Source::Music => "com.apple.Music",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Source::Spotify => "Spotify",
            Source::Music => "Music",
        }
    }

    /// One line: state, title, artist, album, artwork URL, duration ms,
    /// position ms, track id (Spotify's `spotify url`, Music's `persistent
    /// ID`), separated by `SEP` (character id 31). Spotify's
    /// duration is already ms; Music's is seconds. Music has no artwork URL.
    /// `stopped` when the player holds no track: its state says so, or
    /// `current track` cannot be read (−1728; a running Spotify with nothing
    /// loaded); a timeout (−1712) stays an error.
    fn snapshot_script(self) -> String {
        let (duration, artwork, track_id) = match self {
            Source::Spotify => (
                "(duration of t)",
                "my s(artwork url of t)",
                "spotify url of t",
            ),
            Source::Music => (
                "(round ((duration of t) * 1000))",
                "\"\"",
                "persistent ID of t",
            ),
        };
        format!(
            r#"on s(v)
	try
		if v is missing value then return ""
		return v as text
	on error
		return ""
	end try
end s
if application id "{id}" is running then
	with timeout of 2 seconds
		tell application id "{id}"
			set pstate to player state as text
			if pstate is "stopped" then return "stopped"
			try
				set t to current track
				set n to name of t
			on error errText number errNum
				if errNum is -1712 then error errText number errNum
				return "stopped"
			end try
			set pos to ""
			try
				set pos to round ((player position) * 1000)
			end try
			set u to character id 31
			return pstate & u & my s(n) & u & my s(artist of t) & u & my s(album of t) & u & {artwork} & u & my s({duration}) & u & my s(pos) & u & my s({track_id})
		end tell
	end timeout
end if
return """#,
            id = self.bundle()
        )
    }

    fn from_name(app: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|s| s.name() == app)
    }

    /// Start one track by its id (02 §7.10): Spotify plays its URI; Music
    /// plays the library track with that persistent ID. `None` for an id
    /// that is not one the player issues (only letters, digits, `:`, `_`,
    /// `-` and `.` are ever put in a script).
    fn play_script(self, track_id: &str) -> Option<String> {
        let safe = !track_id.is_empty()
            && track_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '-' | '.'));
        if !safe {
            return None;
        }
        let verb = match self {
            Source::Spotify => format!("play track \"{track_id}\""),
            Source::Music => {
                format!(
                    "play (some track of library playlist 1 whose persistent ID is \"{track_id}\")"
                )
            }
        };
        Some(self.command_script(&verb))
    }

    fn command_script(self, verb: &str) -> String {
        format!(
            "if application id \"{id}\" is running then\n\twith timeout of 2 seconds\n\t\ttell application id \"{id}\" to {verb}\n\tend timeout\nend if",
            id = self.bundle()
        )
    }

    fn running(self) -> bool {
        autoreleasepool(|_| {
            NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(
                self.bundle(),
            ))
            .count()
                > 0
        })
    }
}

fn compile(source: &str) -> Option<Retained<NSAppleScript>> {
    NSAppleScript::initWithSource(NSAppleScript::alloc(), &NSString::from_str(source))
}

/// Run a script; its result as text, or the error number and message. The
/// script object keeps its compiled form, so a cached one compiles once.
fn execute(script: &NSAppleScript) -> Result<String, (i32, String)> {
    autoreleasepool(|_| {
        let mut error: Option<Retained<NSDictionary<NSString, AnyObject>>> = None;
        // SAFETY: `executeAndReturnError:` takes an out-pointer to an error
        // dictionary and returns an autoreleased descriptor, nil on failure
        // (the generated binding types it non-null, so it is sent directly).
        let out: Option<Retained<NSAppleEventDescriptor>> =
            unsafe { msg_send![script, executeAndReturnError: &mut error] };
        if let Some(e) = error {
            // SAFETY: Foundation's constant keys of the error dictionary.
            let (num_key, msg_key) =
                unsafe { (NSAppleScriptErrorNumber, NSAppleScriptErrorMessage) };
            let number = e
                .objectForKey(num_key)
                .and_then(|o| o.downcast::<NSNumber>().ok())
                .map(|n| n.integerValue() as i32)
                .unwrap_or(0);
            let message = e
                .objectForKey(msg_key)
                .and_then(|o| o.downcast::<NSString>().ok())
                .map(|m| m.to_string())
                .unwrap_or_default();
            return Err((number, message));
        }
        Ok(out
            .and_then(|d| d.stringValue())
            .map(|s| s.to_string())
            .unwrap_or_default())
    })
}

fn failure((number, message): (i32, String)) -> MediaError {
    if number == NOT_PERMITTED || number == NEEDS_CONSENT {
        MediaError::Denied
    } else {
        MediaError::Failed(format!("AppleScript error {number}: {message}"))
    }
}

/// Spotify, then Music (02 §7.1): the first one playing, else the first one
/// running and paused.
#[derive(Debug, Default)]
pub struct AppleEvents {
    /// The source of the last snapshot; the commands go to it.
    current: Option<Source>,
    /// Each source's compiled snapshot script.
    scripts: [Option<Retained<NSAppleScript>>; 2],
    /// The Automation permission was granted this run (checked again on a
    /// refusal from the script).
    allowed: [bool; 2],
}

impl AppleEvents {
    /// Whether `source` may be scripted, asking the user (on this thread)
    /// the first time.
    fn allowed(&mut self, source: Source) -> Result<bool, MediaError> {
        if self.allowed[source.index()] {
            return Ok(true);
        }
        let mut status = permission(source.bundle(), false);
        if status == NEEDS_CONSENT {
            tracing::info!(
                player = source.name(),
                "asking for the Automation permission"
            );
            status = permission(source.bundle(), true);
        }
        match status {
            0 => {
                self.allowed[source.index()] = true;
                Ok(true)
            }
            PROC_NOT_FOUND => Ok(false),
            NOT_PERMITTED | NEEDS_CONSENT => Err(MediaError::Denied),
            other => Err(MediaError::Failed(format!(
                "Automation permission check failed ({other})"
            ))),
        }
    }

    fn read(&mut self, source: Source) -> Result<Snapshot, MediaError> {
        if !source.running() || !self.allowed(source)? {
            return Ok(Snapshot::Gone);
        }
        let slot = &mut self.scripts[source.index()];
        if slot.is_none() {
            *slot = compile(&source.snapshot_script());
        }
        let script = slot
            .as_ref()
            .ok_or_else(|| MediaError::Failed("the script could not be created".into()))?;
        match execute(script).map_err(failure) {
            Ok(out) => Ok(from_script(source.name(), &out, now_ms())),
            Err(MediaError::Denied) => {
                // Revoked in System Settings since it was granted.
                self.allowed[source.index()] = false;
                Err(MediaError::Denied)
            }
            Err(e) => Err(e),
        }
    }

    fn target(&self) -> Option<Source> {
        self.current
            .or_else(|| Source::ALL.into_iter().find(|s| s.running()))
    }

    fn send(&mut self, verb: &str) -> Result<(), MediaError> {
        let source = self
            .target()
            .ok_or_else(|| MediaError::Failed("no player is running".into()))?;
        if !self.allowed(source)? {
            return Err(MediaError::Failed("no player is running".into()));
        }
        let script = compile(&source.command_script(verb))
            .ok_or_else(|| MediaError::Failed("the script could not be created".into()))?;
        execute(&script).map(|_| ()).map_err(failure)
    }
}

impl Player for AppleEvents {
    fn snapshot(&mut self) -> Result<Snapshot, MediaError> {
        let mut paused: Option<(Source, NowPlaying)> = None;
        let mut denied = false;
        let mut stopped = false;
        for source in Source::ALL {
            match self.read(source) {
                Ok(Snapshot::Track(np)) if np.playing => {
                    self.current = Some(source);
                    return Ok(Snapshot::Track(np));
                }
                Ok(Snapshot::Track(np)) => {
                    if paused.is_none() {
                        paused = Some((source, np));
                    }
                }
                Ok(Snapshot::Stopped) => stopped = true,
                Ok(Snapshot::Gone) => {}
                Err(MediaError::Denied) => denied = true,
                Err(e) => return Err(e),
            }
        }
        if let Some((source, np)) = paused {
            self.current = Some(source);
            return Ok(Snapshot::Track(np));
        }
        self.current = None;
        if denied {
            Err(MediaError::Denied)
        } else if stopped {
            Ok(Snapshot::Stopped)
        } else {
            Ok(Snapshot::Gone)
        }
    }

    fn play_pause(&mut self) -> Result<(), MediaError> {
        self.send("playpause")
    }

    fn next(&mut self) -> Result<(), MediaError> {
        self.send("next track")
    }

    fn previous(&mut self) -> Result<(), MediaError> {
        self.send("previous track")
    }

    fn open(&mut self) -> Result<(), MediaError> {
        open_bundle(self.target().unwrap_or(Source::Spotify))
    }

    /// A running player starts the track; a player that is not running is
    /// opened (Mode never scripts a player it had to launch), as is a row
    /// without an id.
    fn play_item(&mut self, app: &str, track_id: Option<&str>) -> Result<PlayOutcome, MediaError> {
        let source = Source::from_name(app).unwrap_or(Source::Spotify);
        let script = track_id.and_then(|id| source.play_script(id));
        let Some(script) = script.filter(|_| source.running()) else {
            return open_bundle(source).map(|_| PlayOutcome::Opened);
        };
        if !self.allowed(source)? {
            return open_bundle(source).map(|_| PlayOutcome::Opened);
        }
        let compiled = compile(&script)
            .ok_or_else(|| MediaError::Failed("the script could not be created".into()))?;
        execute(&compiled).map_err(failure)?;
        self.current = Some(source);
        Ok(PlayOutcome::Played)
    }
}

/// Bring a player to the front (launching it if needed): `open -b`.
fn open_bundle(source: Source) -> Result<(), MediaError> {
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-b", source.bundle()])
        .status()
        .map_err(|e| MediaError::Failed(e.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(MediaError::Failed(format!(
            "open -b {} failed",
            source.bundle()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_never_launch_a_player_and_name_the_bundle() {
        for s in Source::ALL {
            let snap = s.snapshot_script();
            assert!(snap.contains(&format!(
                "if application id \"{}\" is running then",
                s.bundle()
            )));
            assert!(snap.contains("with timeout of 2 seconds"));
            // No current track (−1728 on a running Spotify with nothing
            // loaded) reads as stopped; a timeout stays an error.
            let guarded = snap.find("set t to current track").unwrap();
            let try_at = snap[..guarded].rfind("try").unwrap();
            assert!(snap[try_at..guarded].trim() == "try");
            assert!(snap.contains("if errNum is -1712 then error errText number errNum"));
            assert!(snap.contains("return \"stopped\""));
            let cmd = s.command_script("playpause");
            assert!(cmd.starts_with(&format!(
                "if application id \"{}\" is running then",
                s.bundle()
            )));
        }
        assert!(
            Source::Music
                .snapshot_script()
                .contains("(duration of t) * 1000")
        );
        assert!(
            Source::Spotify
                .snapshot_script()
                .contains("artwork url of t")
        );
        assert_eq!(TYPE_BUNDLE_ID, 0x6275_6E64);
    }

    /// `st`, `nd`, `rd` and `th` are AppleScript's ordinal suffixes (`1st`):
    /// a variable of that name fails to compile with -2741, which the first
    /// run against a real Spotify showed.
    #[test]
    fn scripts_use_no_reserved_ordinal_suffix_as_a_variable() {
        for s in Source::ALL {
            let snap = s.snapshot_script();
            for word in ["st", "nd", "rd", "th"] {
                assert!(
                    !snap.contains(&format!("set {word} to ")),
                    "{word} is reserved in AppleScript"
                );
            }
        }
    }

    #[test]
    fn a_row_plays_by_the_players_own_id_and_nothing_else_reaches_a_script() {
        let sp = Source::Spotify
            .play_script("spotify:track:4uLU6hMCjMI75M1A2tKUQC")
            .unwrap();
        assert!(sp.contains("play track \"spotify:track:4uLU6hMCjMI75M1A2tKUQC\""));
        assert!(sp.starts_with("if application id \"com.spotify.client\" is running then"));
        let mu = Source::Music.play_script("A1B2C3D4E5F60718").unwrap();
        assert!(mu.contains("whose persistent ID is \"A1B2C3D4E5F60718\""));
        for bad in ["", "x\" & do shell script \"rm", "a b", "spotify:track:1\n"] {
            assert_eq!(Source::Spotify.play_script(bad), None, "{bad:?}");
        }
        assert!(
            Source::Spotify
                .snapshot_script()
                .contains("my s(spotify url of t)")
        );
        assert!(
            Source::Music
                .snapshot_script()
                .contains("my s(persistent ID of t)")
        );
        assert_eq!(Source::from_name("Music"), Some(Source::Music));
        assert_eq!(Source::from_name("VLC"), None);
    }

    #[test]
    fn a_refusal_is_denied_and_other_errors_fail() {
        assert_eq!(
            failure((-1743, "Not authorized".into())),
            MediaError::Denied
        );
        assert_eq!(failure((-1744, String::new())), MediaError::Denied);
        assert!(matches!(
            failure((-1712, "timeout".into())),
            MediaError::Failed(_)
        ));
    }

    /// The permission check is pure Apple Event Manager (no OSA, no main
    /// thread): for a bundle id no process has, it answers "not running"
    /// without asking anything.
    #[test]
    fn the_permission_check_runs_off_the_main_thread_without_a_prompt() {
        assert_eq!(
            permission("com.example.mode.no-such-player", false),
            PROC_NOT_FOUND
        );
    }
}
