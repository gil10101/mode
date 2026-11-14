//! Pushes to the webviews (08 §5.3): `<area>.changed` events carrying the
//! query results of 09 §13 so the webviews never compose SQL. Pushes are
//! debounced 100 ms per event name and coalesced (latest wins).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

/// Debounce window per event name (08 §5.3).
pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// Every event name the core pushes (08 §5.3), plus `core.changed` for the
/// package-0 smoke path (IPC frame received, database opened), and the three
/// the overlay and app windows added: `overlay.pointer` (hot zone, outside
/// click, drag dwell; only the core can see them), `settings.changed` (the
/// `overlay.*` settings of 02 §17 as a partial object) and `app.open` (the
/// app window's routing request, 02 §16). The `pty.*` / `turn.*` events of
/// the PTY host (05 §8, §14; `ui/src/state/contract-terminal.ts`) are
/// [streams](Event::is_stream): sent straight to the app window with their
/// own payloads (`crate::pty`), never through the debounced [`Publisher`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    Pill,
    Sessions,
    Inbox,
    Notes,
    Agenda,
    Quiet,
    Display,
    Health,
    AiActivity,
    HotkeyFired,
    TerminalExited,
    UpdateAvailable,
    Core,
    OverlayPointer,
    Settings,
    AppOpen,
    PtyOutput,
    PtyStarted,
    PtyExited,
    TurnStarted,
    TurnDelta,
    TurnFinished,
    /// `accounts.changed` (06 §9A): the Integrations view's account rows
    /// after a connect, a status change, pause / resume / remove.
    Accounts,
    /// `alert.fired` (07 §8.3, §8.7): one per delivered alert, a stream so
    /// the overlay plays the rule's cue exactly once per item.
    AlertFired,
    /// `chat.delta` (03 §7.1): reply text and activity rows of a running
    /// Chat turn, in order (`crate::chat`; a stream).
    ChatDelta,
    /// `chat.turn` (03 §7.6): a turn started / finished / failed, with its
    /// tokens (a stream).
    ChatTurn,
    /// `chat.action` (03 §7.5): a proposed action awaiting confirmation, and
    /// its confirmed / rejected result (a stream).
    ChatAction,
    /// `automations.changed` (03 §7B): the Automations list after a save,
    /// a toggle, a run starting or finishing (`crate::scheduler`).
    Automations,
    /// `theme.changed` (10 §3.1): `{tokens, dark}` for both webviews after a
    /// pack activation, an import, an OS appearance or accent change
    /// (`crate::themes`).
    Theme,
    /// `agents.changed` (03 §7E): the Agents list after a save, a delete or
    /// a memory change (`crate::agents`).
    Agents,
    /// `dictation.state` (XC-37, 02 §5.6): listening (with the meter, at
    /// most 10 Hz), transcribing, thinking, done, error (`crate::speech`;
    /// a stream to both windows).
    Dictation,
    /// `dictation.model` (XC-37): a model download's progress (a stream).
    DictationModel,
    /// `overlay.flash` (02 §5.4): a short confirmation in the notch for an
    /// action taken in the app window (a note captured, a session launched,
    /// an account connected, a Chat turn); [`OverlayFlash`] is its payload
    /// (a stream to the overlay).
    OverlayFlash,
    /// `media.changed` (02 §16, OV-37): the local player's track for the
    /// Now playing strip, or `{}` (a stream from `mode-media` to both
    /// windows; `mode_app::media`).
    MediaChanged,
    /// `media.history` (02 §16, ADR-0066): the Music tab's history (distinct
    /// tracks and albums Mode saw playing), pushed by `mode-media` after it
    /// records a play and after "Clear history" (a stream to both windows).
    MediaHistory,
}

impl Event {
    pub const ALL: &[Event] = &[
        Event::Pill,
        Event::Sessions,
        Event::Inbox,
        Event::Notes,
        Event::Agenda,
        Event::Quiet,
        Event::Display,
        Event::Health,
        Event::AiActivity,
        Event::HotkeyFired,
        Event::TerminalExited,
        Event::UpdateAvailable,
        Event::Core,
        Event::OverlayPointer,
        Event::Settings,
        Event::AppOpen,
        Event::PtyOutput,
        Event::PtyStarted,
        Event::PtyExited,
        Event::TurnStarted,
        Event::TurnDelta,
        Event::TurnFinished,
        Event::Accounts,
        Event::AlertFired,
        Event::ChatDelta,
        Event::ChatTurn,
        Event::ChatAction,
        Event::Automations,
        Event::Theme,
        Event::Agents,
        Event::Dictation,
        Event::DictationModel,
        Event::OverlayFlash,
        Event::MediaChanged,
        Event::MediaHistory,
    ];

    /// The Tauri event name.
    pub fn name(self) -> &'static str {
        match self {
            Event::Pill => "pill.changed",
            Event::Sessions => "sessions.changed",
            Event::Inbox => "inbox.changed",
            Event::Notes => "notes.changed",
            Event::Agenda => "agenda.changed",
            Event::Quiet => "quiet.changed",
            Event::Display => "display.changed",
            Event::Health => "health.changed",
            Event::AiActivity => "ai.activity",
            Event::HotkeyFired => "hotkey.fired",
            Event::TerminalExited => "terminal.exited",
            Event::UpdateAvailable => "update.available",
            Event::Core => "core.changed",
            Event::OverlayPointer => "overlay.pointer",
            Event::Settings => "settings.changed",
            Event::AppOpen => "app.open",
            Event::PtyOutput => "pty.output",
            Event::PtyStarted => "pty.started",
            Event::PtyExited => "pty.exited",
            Event::TurnStarted => "turn.started",
            Event::TurnDelta => "turn.delta",
            Event::TurnFinished => "turn.finished",
            Event::Accounts => "accounts.changed",
            Event::AlertFired => "alert.fired",
            Event::ChatDelta => "chat.delta",
            Event::ChatTurn => "chat.turn",
            Event::ChatAction => "chat.action",
            Event::Automations => "automations.changed",
            Event::Theme => "theme.changed",
            Event::Agents => "agents.changed",
            Event::Dictation => "dictation.state",
            Event::DictationModel => "dictation.model",
            Event::OverlayFlash => "overlay.flash",
            Event::MediaChanged => "media.changed",
            Event::MediaHistory => "media.history",
        }
    }

    /// Per-session streams with their own payloads (every one is delivered,
    /// in order); latest-wins coalescing would lose bytes.
    pub fn is_stream(self) -> bool {
        matches!(
            self,
            Event::PtyOutput
                | Event::PtyStarted
                | Event::PtyExited
                | Event::TurnStarted
                | Event::TurnDelta
                | Event::TurnFinished
                | Event::AlertFired
                | Event::ChatDelta
                | Event::ChatTurn
                | Event::ChatAction
                | Event::Dictation
                | Event::DictationModel
                | Event::OverlayFlash
                | Event::MediaChanged
                | Event::MediaHistory
        )
    }

    pub fn from_name(name: &str) -> Option<Event> {
        Event::ALL.iter().copied().find(|e| e.name() == name)
    }

    /// The name on the Tauri event bus. Tauri 2 accepts only alphanumerics,
    /// `-`, `/`, `:` and `_` in event names (emit and `listen` both refuse
    /// anything else), so the spec's `<area>.changed` travels as
    /// `<area>:changed`; the webview's `listen` wrapper maps `.` to `:`.
    pub fn wire_name(self) -> &'static str {
        match self {
            Event::Pill => "pill:changed",
            Event::Sessions => "sessions:changed",
            Event::Inbox => "inbox:changed",
            Event::Notes => "notes:changed",
            Event::Agenda => "agenda:changed",
            Event::Quiet => "quiet:changed",
            Event::Display => "display:changed",
            Event::Health => "health:changed",
            Event::AiActivity => "ai:activity",
            Event::HotkeyFired => "hotkey:fired",
            Event::TerminalExited => "terminal:exited",
            Event::UpdateAvailable => "update:available",
            Event::Core => "core:changed",
            Event::OverlayPointer => "overlay:pointer",
            Event::Settings => "settings:changed",
            Event::AppOpen => "app:open",
            Event::PtyOutput => "pty:output",
            Event::PtyStarted => "pty:started",
            Event::PtyExited => "pty:exited",
            Event::TurnStarted => "turn:started",
            Event::TurnDelta => "turn:delta",
            Event::TurnFinished => "turn:finished",
            Event::Accounts => "accounts:changed",
            Event::AlertFired => "alert:fired",
            Event::ChatDelta => "chat:delta",
            Event::ChatTurn => "chat:turn",
            Event::ChatAction => "chat:action",
            Event::Automations => "automations:changed",
            Event::Theme => "theme:changed",
            Event::Agents => "agents:changed",
            Event::Dictation => "dictation:state",
            Event::DictationModel => "dictation:model",
            Event::OverlayFlash => "overlay:flash",
            Event::MediaChanged => "media:changed",
            Event::MediaHistory => "media:history",
        }
    }
}

/// How a flash reads: a success, a neutral note, or a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlashKind {
    Ok,
    Info,
    Bad,
}

/// Where a click on the flash goes in the app window (`app.open`'s view
/// and item).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlashOpen {
    pub view: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// The `overlay.flash` payload (`ui/src/state/contract.ts` `OverlayFlash`):
/// the pill text, its kind, whether the pill stays up after the hold (a
/// Chat turn still thinking), and where a click opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayFlash {
    pub text: String,
    pub kind: FlashKind,
    #[serde(default)]
    pub keep_pill: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<FlashOpen>,
}

impl OverlayFlash {
    pub fn new(text: impl Into<String>, kind: FlashKind) -> Self {
        Self {
            text: text.into(),
            kind,
            keep_pill: false,
            open: None,
        }
    }

    /// A click on the flash opens `view` (and `id` in it).
    pub fn opening(mut self, view: &str, id: Option<&str>) -> Self {
        self.open = Some(FlashOpen {
            view: view.to_string(),
            id: id.map(str::to_string),
        });
        self
    }

    /// The pill stays after the hold (the overlay settles it later).
    pub fn keeping_pill(mut self) -> Self {
        self.keep_pill = true;
        self
    }
}

/// One push: the event and its payload (a 09 §13 query result).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Push {
    pub event: Event,
    pub payload: Value,
}

/// Fan-out from the core to whoever emits to the windows (`mode-app::events`).
#[derive(Debug, Clone)]
pub struct Bus {
    tx: broadcast::Sender<Push>,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new(256)
    }
}

impl Bus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    /// Publish; returns how many subscribers received it.
    pub fn publish(&self, event: Event, payload: Value) -> usize {
        self.tx.send(Push { event, payload }).unwrap_or(0)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Push> {
        self.tx.subscribe()
    }
}

/// Latest-wins coalescing per event name between two flushes.
#[derive(Debug, Default)]
pub struct Coalescer {
    pending: BTreeMap<Event, Value>,
}

impl Coalescer {
    pub fn push(&mut self, event: Event, payload: Value) {
        self.pending.insert(event, payload);
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Take everything pending, one push per event.
    pub fn drain(&mut self) -> Vec<Push> {
        std::mem::take(&mut self.pending)
            .into_iter()
            .map(|(event, payload)| Push { event, payload })
            .collect()
    }
}

// ---------------------------------------------------------------- publisher

/// Builds the payload of an event from the live tables (a 09 §13 query);
/// `None` means nothing to push. Runs on the blocking pool.
pub type PayloadFn = Arc<dyn Fn(Event) -> Option<Value> + Send + Sync>;
/// Delivers a push to the windows (`app.emit` in `mode-app`).
pub type SinkFn = Arc<dyn Fn(&Push) + Send + Sync>;

/// Debounced, coalescing publisher (08 §5.3): each event name waits
/// [`DEBOUNCE`] after its first request, then its payload is computed once
/// and delivered once, however many times it was requested meanwhile.
#[derive(Debug, Clone)]
pub struct Publisher {
    tx: mpsc::UnboundedSender<Event>,
}

impl Publisher {
    /// Start the publisher task on the current tokio runtime.
    pub fn spawn(payloads: PayloadFn, sink: SinkFn, debounce: Duration) -> Publisher {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(rx, payloads, sink, debounce));
        Publisher { tx }
    }

    /// Request a push of `event` (coalesced with others in the window).
    pub fn publish(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn publish_all(&self, events: impl IntoIterator<Item = Event>) {
        for e in events {
            self.publish(e);
        }
    }
}

async fn run(
    mut rx: mpsc::UnboundedReceiver<Event>,
    payloads: PayloadFn,
    sink: SinkFn,
    debounce: Duration,
) {
    let far = Duration::from_secs(3600);
    let mut due: BTreeMap<Event, tokio::time::Instant> = BTreeMap::new();
    loop {
        let next = due.values().min().copied();
        let open = tokio::select! {
            ev = rx.recv() => match ev {
                Some(e) => {
                    due.entry(e).or_insert_with(|| tokio::time::Instant::now() + debounce);
                    true
                }
                None => false,
            },
            _ = tokio::time::sleep_until(next.unwrap_or_else(|| tokio::time::Instant::now() + far)), if next.is_some() => true,
        };
        let now = tokio::time::Instant::now();
        let ready: Vec<Event> = due
            .iter()
            .filter(|(_, t)| !open || **t <= now)
            .map(|(e, _)| *e)
            .collect();
        for e in ready {
            due.remove(&e);
            let p = Arc::clone(&payloads);
            let payload = tokio::task::spawn_blocking(move || p(e))
                .await
                .ok()
                .flatten();
            if let Some(payload) = payload {
                sink(&Push { event: e, payload });
            }
        }
        if !open {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_the_spec_names_and_round_trip() {
        assert_eq!(Event::Pill.name(), "pill.changed");
        assert_eq!(Event::AiActivity.name(), "ai.activity");
        assert_eq!(Event::Core.name(), "core.changed");
        assert_eq!(Event::OverlayPointer.name(), "overlay.pointer");
        assert_eq!(Event::Settings.name(), "settings.changed");
        assert_eq!(Event::AppOpen.name(), "app.open");
        assert_eq!(Event::PtyOutput.name(), "pty.output");
        assert_eq!(Event::PtyOutput.wire_name(), "pty:output");
        assert_eq!(Event::TurnFinished.wire_name(), "turn:finished");
        assert!(Event::PtyOutput.is_stream());
        assert!(!Event::Sessions.is_stream());
        assert_eq!(
            Event::ALL.iter().filter(|e| e.is_stream()).count(),
            15,
            "pty.output / started / exited, turn.started / delta / finished, alert.fired, chat.delta / turn / action, dictation.state / model, overlay.flash, media.changed / history"
        );
        assert_eq!(Event::MediaChanged.wire_name(), "media:changed");
        assert_eq!(Event::MediaHistory.name(), "media.history");
        assert_eq!(Event::MediaHistory.wire_name(), "media:history");
        assert_eq!(Event::Dictation.wire_name(), "dictation:state");
        assert_eq!(Event::DictationModel.wire_name(), "dictation:model");
        assert_eq!(Event::ChatDelta.wire_name(), "chat:delta");
        assert_eq!(Event::OverlayFlash.name(), "overlay.flash");
        assert_eq!(Event::OverlayFlash.wire_name(), "overlay:flash");
        assert_eq!(Event::Automations.wire_name(), "automations:changed");
        assert_eq!(Event::Theme.wire_name(), "theme:changed");
        assert!(!Event::Automations.is_stream() && !Event::Theme.is_stream());
        for e in Event::ALL {
            assert_eq!(Event::from_name(e.name()), Some(*e));
        }
        assert_eq!(Event::from_name("nope"), None);
        assert_eq!(
            serde_json::to_string(&Event::HotkeyFired).unwrap(),
            "\"hotkey_fired\""
        );
    }

    #[test]
    fn a_flash_serialises_to_the_contract_shape() {
        let plain =
            serde_json::to_value(OverlayFlash::new("captured · note", FlashKind::Ok)).unwrap();
        assert_eq!(
            plain,
            serde_json::json!({"text": "captured · note", "kind": "ok", "keep_pill": false})
        );
        let full = OverlayFlash::new("answered · the retry policy says", FlashKind::Info)
            .opening("chat", Some("th-1"))
            .keeping_pill();
        let v = serde_json::to_value(&full).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "text": "answered · the retry policy says",
                "kind": "info",
                "keep_pill": true,
                "open": {"view": "chat", "id": "th-1"}
            })
        );
        let notes = OverlayFlash::new("x", FlashKind::Bad).opening("notes", None);
        assert_eq!(
            serde_json::to_value(&notes).unwrap()["open"],
            serde_json::json!({"view": "notes"})
        );
        let back: OverlayFlash =
            serde_json::from_value(serde_json::json!({"text": "t", "kind": "bad"})).unwrap();
        assert_eq!(back, OverlayFlash::new("t", FlashKind::Bad));
    }

    #[test]
    fn wire_names_are_tauri_legal_and_map_one_to_one() {
        for e in Event::ALL {
            let wire = e.wire_name();
            assert_eq!(wire, e.name().replace('.', ":"), "{e:?}");
            assert!(
                wire.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '/' | ':' | '_')),
                "{wire} would be refused by Tauri"
            );
        }
    }

    #[tokio::test]
    async fn bus_delivers_to_subscribers() {
        let bus = Bus::default();
        assert_eq!(
            bus.publish(Event::Sessions, Value::Null),
            0,
            "no subscriber yet"
        );
        let mut rx = bus.subscribe();
        assert_eq!(bus.publish(Event::Sessions, serde_json::json!({"n": 1})), 1);
        let push = rx.recv().await.unwrap();
        assert_eq!(push.event, Event::Sessions);
        assert_eq!(push.payload["n"], 1);
    }

    #[test]
    fn coalescer_keeps_the_latest_per_event() {
        let mut c = Coalescer::default();
        assert!(c.is_empty());
        c.push(Event::Notes, serde_json::json!(1));
        c.push(Event::Notes, serde_json::json!(2));
        c.push(Event::Pill, serde_json::json!("p"));
        let drained = c.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(
            drained
                .iter()
                .find(|p| p.event == Event::Notes)
                .unwrap()
                .payload,
            2
        );
        assert!(c.is_empty());
    }

    #[tokio::test]
    async fn publisher_debounces_and_computes_the_payload_once() {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let computed = Arc::new(AtomicUsize::new(0));
        let got: Arc<Mutex<Vec<Push>>> = Arc::new(Mutex::new(Vec::new()));
        let c = Arc::clone(&computed);
        let payloads: PayloadFn = Arc::new(move |e| {
            c.fetch_add(1, Ordering::SeqCst);
            (e != Event::Notes).then(|| serde_json::json!({"event": e.name()}))
        });
        let g = Arc::clone(&got);
        let sink: SinkFn = Arc::new(move |p| g.lock().unwrap().push(p.clone()));
        let publisher = Publisher::spawn(payloads, sink, Duration::from_millis(50));
        publisher.publish(Event::Sessions);
        publisher.publish(Event::Sessions);
        publisher.publish_all([Event::Pill, Event::Sessions, Event::Notes]);
        tokio::time::sleep(Duration::from_millis(200)).await;
        let pushes = got.lock().unwrap().clone();
        assert_eq!(
            pushes.len(),
            2,
            "sessions once, pill once, notes had no payload"
        );
        assert_eq!(pushes[0].event, Event::Pill);
        assert_eq!(pushes[1].event, Event::Sessions);
        assert_eq!(pushes[1].payload["event"], "sessions.changed");
        assert_eq!(computed.load(Ordering::SeqCst), 3);
        publisher.publish(Event::Sessions);
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(
            got.lock().unwrap().len(),
            3,
            "a later request is a new push"
        );
    }
}
