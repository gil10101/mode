//! Managed state shared by commands, the IPC pump and the background tasks.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use mode_core::chat::Chat;
use mode_core::connectors::Connectors;
use mode_core::db::Db;
use mode_core::diag::Snapshot;
use mode_core::hooks_install;
use mode_core::ipc::Server;
use mode_core::keyring::Keyring;
use mode_core::notes::Notes;
use mode_core::packs::api::Packs;
use mode_core::palette::Palette;
use mode_core::pty::Pty;
use mode_core::push::{Event, FlashKind, FlashOpen, OverlayFlash, Publisher};
use mode_core::scheduler::{Scheduler, Wake};
use mode_core::sessions::Engine;
use mode_core::speech::Dictation;
use mode_core::themes::Themes;
use mode_core::transcript::Tailers;
use mode_wire::Endpoint;
use serde::Serialize;

/// Where `overlay.flash` goes (the overlay window in the app; a recorder in
/// the wiring tests).
pub type FlashSink = Arc<dyn Fn(&OverlayFlash) + Send + Sync>;

/// Hook frames kept for Diagnostics → IPC "last 20 hook events" (03 §8.16).
pub const RECENT_FRAMES: usize = 20;

/// One hook frame as Diagnostics shows it: the event name, when the core
/// applied it, and the relay → applied latency. Never the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecentFrame {
    pub event: String,
    pub at: String,
    pub latency_ms: Option<u64>,
}

/// Git passes in flight and when each session last ran one (05 §6.2).
#[derive(Debug, Default)]
pub struct GitPasses {
    pub scheduled: HashSet<String>,
    pub last: HashMap<String, Instant>,
}

pub struct CoreState {
    pub db: Arc<Db>,
    pub engine: Arc<Engine>,
    /// Set once the relay server is bound.
    pub server: OnceLock<Arc<Server>>,
    /// Set once the publisher task runs.
    pub publisher: OnceLock<Publisher>,
    pub tailers: Arc<Mutex<Tailers>>,
    pub git_passes: Mutex<GitPasses>,
    /// The overlay has been drawn (a non-hidden state was shown): a card can
    /// reach the user. Until then a decision frame waits at most
    /// [`crate::events::UI_GRACE`] for it, then gets `no_opinion` (08 §5.2).
    pub ui_ready: AtomicBool,
    pub ui_ready_notify: tokio::sync::Notify,
    /// `MODE_SMOKE=1` (the headless smoke run): decision frames are answered
    /// `no_opinion` at once whatever the UI does.
    pub smoke: bool,
    /// The last `health.changed` pushed.
    pub last_health: Mutex<Option<crate::health::HealthChanged>>,
    /// The last `hooks_status` answer.
    pub hooks: Mutex<Option<hooks_install::Status>>,
    /// The staged relay (`<app_data_dir>/bin/mode-hook`), when found.
    pub relay_path: Mutex<Option<PathBuf>>,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub endpoint: Endpoint,
    pub ipc_listening: AtomicBool,
    pub frames_received: AtomicU64,
    /// The notes service (04), set once at setup; empty while the database
    /// is newer than this build.
    pub notes: OnceLock<Arc<Notes>>,
    /// The real database is newer than this build (XC-33): the app runs on
    /// a scratch database, starts no IPC and no notes, and the app window
    /// shows the "update Mode" screen (`health.changed.db_newer`).
    pub db_newer: AtomicBool,
    /// `claude` on this machine, re-probed when the binary changes.
    pub claude: Mutex<Option<crate::claude::Probe>>,
    /// The PTY host (package 8), set once the publisher runs.
    pub pty: OnceLock<Pty>,
    /// Quit with live embedded sessions (03 §2.1).
    pub quit: crate::quit::QuitGate,
    /// The last [`RECENT_FRAMES`] hook frames, newest first.
    pub recent_frames: Mutex<VecDeque<RecentFrame>>,
    /// When the notes index last finished a full scan this run.
    pub last_full_scan_at: Mutex<Option<String>>,
    /// The secret store (`mode_core::keyring`), opened at setup or on first
    /// use: the OS keyring, or the explicit file store on Linux.
    pub keyring: OnceLock<Arc<Keyring>>,
    /// When each secret was last written this run (the keyring keeps no time).
    pub secret_updated: Mutex<HashMap<String, String>>,
    /// The connectors service (06, 07), set at setup; empty while the
    /// database is newer than this build or the HTTP client failed.
    pub connectors: OnceLock<Arc<Connectors>>,
    /// Session state transitions → `session.*` alerts and withdrawals.
    pub session_watch: crate::alerts::SessionWatch,
    /// Chat over my data (03 §7), set at setup when the connectors service
    /// (its AI backends) runs.
    pub chat: OnceLock<Arc<Chat>>,
    /// Automations (03 §7B); not on a scratch database.
    pub scheduler: OnceLock<Arc<Scheduler>>,
    /// Wakes the scheduler's run loop (OS resume, a saved automation).
    pub scheduler_wake: OnceLock<tokio::sync::mpsc::UnboundedSender<Wake>>,
    /// Theme packs and the token resolver (10 §3–§5).
    pub themes: OnceLock<Arc<Themes>>,
    /// The command palette index (10 §7.4.15).
    pub palette: OnceLock<Arc<Palette>>,
    /// Packs and the catalog (03 §7C.6); needs the themes service and
    /// the connectors' HTTP client.
    pub packs: OnceLock<Arc<Packs>>,
    /// Dictation (XC-37): the hold-to-talk state machine (`crate::speech`).
    pub dictation: Arc<Dictation>,
    /// The Mode window that has keyboard focus (`app` / `overlay`), from
    /// `WindowEvent::Focused`; `None` when another app is in front.
    pub focused_window: Mutex<Option<&'static str>>,
    /// What each window shows (`app_context_set`), by window label, as
    /// `(kind, id)`: attached to a "Hey Mode" request (XC-38), the
    /// overlay's item first (its panel is open), else the app window's.
    pub context: Mutex<HashMap<String, (String, String)>>,
    /// The words of the last dictation sent to a window as
    /// `dictation:text` (with a sequence number), kept until it answers
    /// `dictation_inserted`.
    pub dictation_pending: Mutex<Option<(u64, String)>>,
    /// The voice request's Chat turn in flight, so its streamed text and
    /// its end become the dictation state (the pill or the reply card).
    pub voice_turn: Mutex<Option<crate::speech::VoiceTurn>>,
    /// Model downloads in flight, so a second `dictation_model_ensure`
    /// for the same model waits on the first.
    pub model_downloads: Mutex<HashSet<String>>,
    /// `overlay.flash` (02 §5.4): set at setup; flashes before it are dropped.
    pub flash_sink: OnceLock<FlashSink>,
}

/// The secret store for this build (08 §8): the OS keyring on macOS and
/// Windows; on Linux (a development and CI target) the explicit file store
/// `<data dir>/secrets-dev.json` for dev builds, and for release builds when
/// no OS keyring exists. Which one is logged, never a value.
pub fn open_keyring(data_dir: &Path, dev: bool) -> Keyring {
    let linux = !cfg!(any(target_os = "macos", windows));
    if linux && dev {
        let path = Keyring::file_path(data_dir);
        tracing::info!(store = "file", path = %path.display(), "keyring: Linux dev build, explicit file store");
        return Keyring::file(path);
    }
    match Keyring::os() {
        Ok(k) => {
            tracing::info!(store = "os", "keyring: OS keyring");
            k
        }
        Err(e) => {
            let path = Keyring::file_path(data_dir);
            tracing::warn!(store = "file", path = %path.display(), error = %e, "keyring: no OS keyring, using the file store");
            Keyring::file(path)
        }
    }
}

impl CoreState {
    pub fn new(
        db: Arc<Db>,
        engine: Arc<Engine>,
        data_dir: PathBuf,
        db_path: PathBuf,
        endpoint: Endpoint,
    ) -> Self {
        Self {
            db,
            engine,
            server: OnceLock::new(),
            publisher: OnceLock::new(),
            tailers: Arc::new(Mutex::new(Tailers::new())),
            git_passes: Mutex::new(GitPasses::default()),
            ui_ready: AtomicBool::new(false),
            ui_ready_notify: tokio::sync::Notify::new(),
            smoke: std::env::var("MODE_SMOKE").is_ok_and(|v| v == "1"),
            last_health: Mutex::new(None),
            hooks: Mutex::new(None),
            relay_path: Mutex::new(None),
            data_dir,
            db_path,
            endpoint,
            ipc_listening: AtomicBool::new(false),
            frames_received: AtomicU64::new(0),
            notes: OnceLock::new(),
            db_newer: AtomicBool::new(false),
            claude: Mutex::new(None),
            pty: OnceLock::new(),
            quit: crate::quit::QuitGate::default(),
            recent_frames: Mutex::new(VecDeque::with_capacity(RECENT_FRAMES)),
            last_full_scan_at: Mutex::new(None),
            keyring: OnceLock::new(),
            secret_updated: Mutex::new(HashMap::new()),
            connectors: OnceLock::new(),
            session_watch: crate::alerts::SessionWatch::default(),
            chat: OnceLock::new(),
            scheduler: OnceLock::new(),
            scheduler_wake: OnceLock::new(),
            themes: OnceLock::new(),
            palette: OnceLock::new(),
            packs: OnceLock::new(),
            dictation: Dictation::new(),
            focused_window: Mutex::new(None),
            context: Mutex::new(HashMap::new()),
            dictation_pending: Mutex::new(None),
            voice_turn: Mutex::new(None),
            model_downloads: Mutex::new(HashSet::new()),
            flash_sink: OnceLock::new(),
        }
    }

    /// The Chat service, or `unavailable` (no connectors service: no AI
    /// backends to ask).
    pub fn chat(&self) -> crate::cmd_error::CmdResult<Arc<Chat>> {
        self.chat.get().cloned().ok_or_else(|| {
            crate::cmd_error::CmdError::new(
                "unavailable",
                "Chat is unavailable: the AI backends are not running.",
            )
        })
    }

    /// The scheduler, or `unavailable` (a newer database).
    pub fn scheduler(&self) -> crate::cmd_error::CmdResult<Arc<Scheduler>> {
        self.scheduler.get().cloned().ok_or_else(|| {
            crate::cmd_error::CmdError::new(
                "unavailable",
                "Automations are unavailable: the database was written by a newer Mode.",
            )
        })
    }

    /// The themes service, or `unavailable` before setup.
    pub fn themes(&self) -> crate::cmd_error::CmdResult<Arc<Themes>> {
        self.themes.get().cloned().ok_or_else(|| {
            crate::cmd_error::CmdError::new("unavailable", "Theme packs are still loading.")
        })
    }

    /// The packs service, or `unavailable` (no HTTP client: the connectors
    /// service failed, or a newer database).
    pub fn packs(&self) -> crate::cmd_error::CmdResult<Arc<Packs>> {
        self.packs.get().cloned().ok_or_else(|| {
            crate::cmd_error::CmdError::new(
                "unavailable",
                "The Marketplace is unavailable: the connectors service is not running.",
            )
        })
    }

    /// The palette, or `unavailable` before setup.
    pub fn palette(&self) -> crate::cmd_error::CmdResult<Arc<Palette>> {
        self.palette.get().cloned().ok_or_else(|| {
            crate::cmd_error::CmdError::new("unavailable", "The palette is still loading.")
        })
    }

    /// Wake the scheduler's loop (a no-op before it runs).
    pub fn wake_scheduler(&self, w: Wake) {
        if let Some(tx) = self.scheduler_wake.get() {
            let _ = tx.send(w);
        }
    }

    /// Mark the palette index stale (rebuilt in the background).
    pub fn invalidate_palette(&self) {
        if let Some(p) = self.palette.get() {
            p.invalidate();
        }
    }

    /// The PTY host, or `unavailable` before it started.
    pub fn pty(&self) -> crate::cmd_error::CmdResult<Pty> {
        self.pty.get().cloned().ok_or_else(|| {
            crate::cmd_error::CmdError::new(
                "unavailable",
                "The terminal host is still starting. Try again in a moment.",
            )
        })
    }

    /// The secret store (OS keyring, or the file store where none exists).
    pub fn keyring(&self) -> Arc<Keyring> {
        Arc::clone(
            self.keyring
                .get_or_init(|| Arc::new(open_keyring(&self.data_dir, mode_core::is_dev_build()))),
        )
    }

    /// The connectors service, or `unavailable` (a newer database, or the
    /// service did not start).
    pub fn connectors(&self) -> crate::cmd_error::CmdResult<Arc<Connectors>> {
        self.connectors.get().cloned().ok_or_else(|| {
            crate::cmd_error::CmdError::new(
                "unavailable",
                "Accounts and the Inbox are unavailable: the connectors service is not running.",
            )
        })
    }

    /// Remember a hook frame for Diagnostics.
    pub fn record_frame(&self, frame: RecentFrame) {
        let mut q = Self::lock(&self.recent_frames);
        q.push_front(frame);
        q.truncate(RECENT_FRAMES);
    }

    /// The notes service, or `unavailable` (database newer than the build).
    pub fn notes(&self) -> crate::cmd_error::CmdResult<Arc<Notes>> {
        self.notes.get().cloned().ok_or_else(|| {
            crate::cmd_error::CmdError::new(
                "unavailable",
                "Notes are unavailable: the database was written by a newer Mode.",
            )
        })
    }

    pub fn is_db_newer(&self) -> bool {
        self.db_newer.load(Ordering::Relaxed)
    }

    /// `<app_data_dir>/backups` (the pre-migration backups, 09 §12).
    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: mode_core::VERSION,
            dev_build: mode_core::is_dev_build(),
            data_dir: self.data_dir.clone(),
            db_path: self.db_path.clone(),
            endpoint: self.endpoint.describe(),
            ipc_listening: self.ipc_listening.load(Ordering::Relaxed),
            hook_frames_received: self.frames_received.load(Ordering::Relaxed),
        }
    }

    /// Request pushes (debounced by the publisher; dropped before it runs).
    pub fn publish(&self, events: impl IntoIterator<Item = Event>) {
        if let Some(p) = self.publisher.get() {
            p.publish_all(events);
        }
    }

    /// A short confirmation in the notch for an action taken in the app
    /// window (02 §5.4): `overlay.flash`, straight to the overlay (it
    /// bypasses the debounced publisher, like `chat.delta`).
    pub fn flash(&self, text: impl Into<String>, kind: FlashKind, open: Option<FlashOpen>) {
        let mut f = OverlayFlash::new(text, kind);
        f.open = open;
        self.send_flash(&f);
    }

    /// [`Self::flash`] with every field of the payload (`keep_pill`).
    pub fn send_flash(&self, f: &OverlayFlash) {
        match self.flash_sink.get() {
            Some(sink) => sink(f),
            None => tracing::debug!(text = %f.text, "overlay.flash before the sink: dropped"),
        }
    }

    /// The overlay was drawn: decision frames may wait for a card from now on.
    pub fn mark_ui_ready(&self) {
        if !self.ui_ready.swap(true, Ordering::SeqCst) {
            tracing::info!("ui ready: the overlay was drawn; decision frames wait for a card");
            self.ui_ready_notify.notify_waiters();
        }
    }

    pub fn is_ui_ready(&self) -> bool {
        self.ui_ready.load(Ordering::SeqCst)
    }

    pub fn server(&self) -> Option<Arc<Server>> {
        self.server.get().cloned()
    }

    pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
        m.lock().unwrap_or_else(|p| p.into_inner())
    }
}
