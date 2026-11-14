//! Dictation (XC-37, XC-38; ADR-0061): hold a key, speak, release; the
//! words are transcribed on this machine by whisper.cpp and land in Mode's
//! focused field, become a captured note, or (after "Hey Mode") go to Chat.
//!
//! [`Dictation`] is the state machine the app drives: `start` on press
//! (opens the [`audio::Source`]), `release` on release (closes it, resamples,
//! transcribes on the `mode-speech` thread and hands the text back), and
//! `cancel`. Every change is a [`DictationState`] pushed through the sink as
//! `dictation.state` (≤ 10 Hz while listening, never coalesced). Where the
//! words go is the app's decision ([`wake::classify`], the focused window,
//! `dictation.unfocused`); it reports the outcome with [`Dictation::push`].
//!
//! Without the `speech` feature the engine is not compiled: [`microphone`],
//! the whisper.cpp loader and [`mic_check`] answer
//! [`SpeechError::Unavailable`] and [`AVAILABLE`] is false (the state
//! machine itself still runs, for the tests).

pub mod audio;
pub mod engine;
pub mod models;
pub mod wake;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use audio::Source;
use engine::Recognizer;

/// The speech engine is compiled into this build.
pub const AVAILABLE: bool = cfg!(feature = "speech");

/// What whisper.cpp takes.
pub const SAMPLE_RATE: u32 = 16_000;
/// A shorter hold is a tap: ignored, no text.
pub const MIN_HOLD: Duration = Duration::from_millis(300);
/// The longest hold; past it the recording is dropped with a reason.
pub const MAX_HOLD: Duration = Duration::from_secs(60);
/// The listening pushes' cadence (the meter).
pub const TICK: Duration = Duration::from_millis(100);

/// Where the OS keeps the microphone switch, for the error line.
pub fn mic_settings_path() -> &'static str {
    if cfg!(target_os = "macos") {
        "System Settings › Privacy & Security › Microphone"
    } else if cfg!(windows) {
        "Settings › Privacy › Microphone"
    } else {
        "your sound settings"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpeechError {
    #[error("dictation is not part of this build")]
    Unavailable,
    #[error("no microphone found")]
    NoMicrophone,
    #[error("microphone not allowed · {}", mic_settings_path())]
    MicDenied,
    #[error("no sound from the microphone · check {}", mic_settings_path())]
    Silent,
    #[error("microphone: {0}")]
    Device(String),
    #[error("the {0} model is not downloaded · Settings › Dictation")]
    NoModel(String),
    #[error("{0:?} is not a dictation model")]
    UnknownModel(String),
    #[error("still transcribing")]
    Busy,
    #[error("too long · hold for under a minute")]
    TooLong,
    #[error("model download failed: {0}")]
    Download(String),
    #[error("{0}")]
    Engine(String),
    #[error("{0}")]
    Io(String),
}

/// The state names of `dictation.state`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StateName {
    #[default]
    Idle,
    Listening,
    Transcribing,
    Thinking,
    /// A "Hey Mode" request's reply streaming in (`reply` = the text so far).
    Answering,
    Done,
    Error,
}

/// Where the words went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Field,
    Note,
    Chat,
    None,
}

/// What failed, on an `error` state: the dictation itself (start,
/// microphone, model, transcription) or a "Hey Mode" request to Chat. The
/// Dictation settings page shows request errors under the request row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorSource {
    Dictation,
    Request,
}

/// Where a "Hey Mode" request's answer shows (02 §5.7, ADR-0064): the
/// pill's first words while the app window is in front, else the
/// overlay's reply card. Decided once, when the transcript is routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Pill,
    Card,
}

/// A citation of a request's reply, as the reply card needs it (the
/// `[n]` chip and the item it opens).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Citation {
    pub n: u32,
    pub kind: String,
    pub id: String,
    pub title: String,
}

/// `dictation.state` (`ui/src/state/contract.ts` `DictationState`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DictationState {
    pub state: StateName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// A request's Chat turn (from `thinking` on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// A request's reply: so far on `answering`, all of it on `done`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<Citation>,
    /// A request's surface (02 §5.7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<Surface>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Set on `error` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ErrorSource>,
}

impl DictationState {
    pub fn of(state: StateName) -> DictationState {
        DictationState {
            state,
            ..Default::default()
        }
    }

    /// A dictation failure (start, microphone, model, transcription).
    pub fn error(reason: impl Into<String>) -> DictationState {
        DictationState {
            state: StateName::Error,
            error: Some(reason.into()),
            source: Some(ErrorSource::Dictation),
            ..Default::default()
        }
    }

    /// A "Hey Mode" request that Chat could not take or answer.
    pub fn request_error(reason: impl Into<String>) -> DictationState {
        DictationState {
            source: Some(ErrorSource::Request),
            ..DictationState::error(reason)
        }
    }

    /// `done` with where the words went and how many.
    pub fn done(target: Target, text: &str) -> DictationState {
        DictationState {
            state: StateName::Done,
            target: Some(target),
            words: Some(word_count(text)),
            text: (!text.is_empty()).then(|| first_words(text, 8)),
            ..Default::default()
        }
    }
}

pub fn word_count(text: &str) -> u32 {
    text.split_whitespace().count().min(u32::MAX as usize) as u32
}

/// The first `n` words, with an ellipsis when there were more.
pub fn first_words(text: &str, n: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= n {
        words.join(" ")
    } else {
        format!("{}…", words[..n].join(" "))
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Delivers every state change (the app emits `dictation:state`).
pub type StateSink = Arc<dyn Fn(&DictationState) + Send + Sync>;
/// Loads a recognizer for a model file (whisper.cpp; tests pass a fake).
pub type Loader = Arc<dyn Fn(&Path) -> Result<Arc<dyn Recognizer>, SpeechError> + Send + Sync>;

/// What one hold transcribes with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartContext {
    pub model_path: PathBuf,
    /// `None` for the English models; an ISO code or `auto` otherwise.
    pub language: Option<String>,
}

struct Capture {
    source: Box<dyn Source>,
    samples: Arc<Mutex<Vec<f32>>>,
    rate: u32,
    started: Instant,
    ctx: StartContext,
}

#[derive(Default)]
struct Inner {
    state: DictationState,
    capture: Option<Capture>,
}

/// The dictation service (one per core).
pub struct Dictation {
    inner: Mutex<Inner>,
    sink: Mutex<Option<StateSink>>,
    loader: Loader,
    recognizer: Mutex<Option<(PathBuf, Arc<dyn Recognizer>)>>,
    /// Bumped by every start and cancel: a stale ticker or transcription
    /// sees a different number and stops.
    generation: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Dictation {
    /// The service with whisper.cpp as its engine.
    pub fn new() -> Arc<Dictation> {
        Dictation::with_loader(Arc::new(engine::load))
    }

    pub fn with_loader(loader: Loader) -> Arc<Dictation> {
        Arc::new(Dictation {
            inner: Mutex::new(Inner::default()),
            sink: Mutex::new(None),
            loader,
            recognizer: Mutex::new(None),
            generation: AtomicU64::new(0),
        })
    }

    pub fn set_sink(&self, sink: impl Fn(&DictationState) + Send + Sync + 'static) {
        *lock(&self.sink) = Some(Arc::new(sink));
    }

    /// The current state (`dictation_status`).
    pub fn state(&self) -> DictationState {
        lock(&self.inner).state.clone()
    }

    pub fn is_listening(&self) -> bool {
        lock(&self.inner).capture.is_some()
    }

    /// Set and push a state (the app reports `thinking`, `done`, `error`).
    pub fn push(&self, mut s: DictationState) {
        if s.since_ms.is_none() {
            s.since_ms = Some(now_ms());
        }
        lock(&self.inner).state = s.clone();
        self.emit(&s);
    }

    fn emit(&self, s: &DictationState) {
        let sink = lock(&self.sink).clone();
        if let Some(f) = sink {
            f(s);
        }
    }

    /// Press: open `source` and start listening. Refused while a clip is
    /// still being transcribed; a second press while listening is ignored.
    pub fn start(
        self: &Arc<Self>,
        mut source: Box<dyn Source>,
        ctx: StartContext,
    ) -> Result<(), SpeechError> {
        let mut inner = lock(&self.inner);
        if inner.capture.is_some() {
            return Ok(());
        }
        if inner.state.state == StateName::Transcribing {
            return Err(SpeechError::Busy);
        }
        if !ctx.model_path.is_file() {
            let name = ctx
                .model_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let id = name
                .strip_prefix("ggml-")
                .and_then(|n| n.strip_suffix("-q5_1.bin"))
                .unwrap_or(&name)
                .to_string();
            return Err(SpeechError::NoModel(id));
        }
        let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let buf = Arc::clone(&samples);
        let rate = source.start(Box::new(move |chunk: &[f32]| {
            lock(&buf).extend_from_slice(chunk);
        }))?;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        inner.capture = Some(Capture {
            source,
            samples,
            rate: rate.max(1),
            started: Instant::now(),
            ctx,
        });
        let s = DictationState {
            state: StateName::Listening,
            since_ms: Some(now_ms()),
            seconds: Some(0.0),
            level: Some(0.0),
            ..Default::default()
        };
        inner.state = s.clone();
        drop(inner);
        self.emit(&s);
        let me = Arc::clone(self);
        let _ = std::thread::Builder::new()
            .name("mode-speech-level".into())
            .spawn(move || me.tick(generation));
        Ok(())
    }

    /// The meter and the 60 s cap, every [`TICK`] while listening.
    fn tick(&self, generation: u64) {
        loop {
            std::thread::sleep(TICK);
            if self.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            let mut inner = lock(&self.inner);
            let Some(cap) = inner.capture.as_mut() else {
                return;
            };
            let held = cap.started.elapsed();
            if held >= MAX_HOLD {
                cap.source.stop();
                inner.capture = None;
                drop(inner);
                self.generation.fetch_add(1, Ordering::SeqCst);
                self.push(DictationState::error(SpeechError::TooLong.to_string()));
                return;
            }
            let level = {
                let samples = lock(&cap.samples);
                let window = (cap.rate / 10) as usize;
                audio::level(&samples[samples.len().saturating_sub(window)..])
            };
            let mut s = inner.state.clone();
            s.seconds = Some((held.as_millis() as f32 / 100.0).round() / 10.0);
            s.level = Some((level * 100.0).round() / 100.0);
            inner.state = s.clone();
            drop(inner);
            self.emit(&s);
        }
    }

    /// Release: stop listening; a tap (under [`MIN_HOLD`]) goes back to
    /// idle with no text. Otherwise push `transcribing` and, on the
    /// `mode-speech` thread, transcribe and call `done` with the text
    /// (`None` for a tap or a release without a press; an error has already
    /// been pushed as `error` when `done` gets `Err`).
    pub fn release(
        self: &Arc<Self>,
        done: impl FnOnce(Result<Option<String>, SpeechError>) + Send + 'static,
    ) {
        let cap = lock(&self.inner).capture.take();
        let Some(mut cap) = cap else {
            done(Ok(None));
            return;
        };
        cap.source.stop();
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let held = cap.started.elapsed();
        let samples = std::mem::take(&mut *lock(&cap.samples));
        if held < MIN_HOLD || samples.is_empty() {
            self.push(DictationState::of(StateName::Idle));
            done(Ok(None));
            return;
        }
        if samples.iter().all(|s| *s == 0.0) {
            let e = SpeechError::Silent;
            self.push(DictationState::error(e.to_string()));
            done(Err(e));
            return;
        }
        self.push(DictationState::of(StateName::Transcribing));
        let me = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("mode-speech".into())
            .spawn(move || {
                let out = me.transcribe(&samples, cap.rate, &cap.ctx);
                if me.generation.load(Ordering::SeqCst) != generation {
                    tracing::info!("dictation: canceled while transcribing");
                    return;
                }
                match out {
                    Ok(text) => done(Ok(Some(text))),
                    Err(e) => {
                        me.push(DictationState::error(e.to_string()));
                        done(Err(e));
                    }
                }
            });
        if let Err(e) = spawned {
            self.push(DictationState::error(e.to_string()));
        }
    }

    /// Resample and transcribe one clip (blocking; the `mode-speech` thread).
    pub fn transcribe(
        &self,
        samples: &[f32],
        rate: u32,
        ctx: &StartContext,
    ) -> Result<String, SpeechError> {
        let pcm = audio::to_16k(samples, rate)?;
        let rec = self.recognizer(&ctx.model_path)?;
        let started = Instant::now();
        let text = rec.transcribe(&pcm, ctx.language.as_deref())?;
        tracing::info!(
            seconds = pcm.len() as f32 / SAMPLE_RATE as f32,
            ms = started.elapsed().as_millis() as u64,
            words = word_count(&text),
            "dictation transcribed"
        );
        Ok(text)
    }

    /// The recognizer for `path`, loaded on first use and kept.
    fn recognizer(&self, path: &Path) -> Result<Arc<dyn Recognizer>, SpeechError> {
        let mut slot = lock(&self.recognizer);
        if let Some((p, r)) = slot.as_ref()
            && p == path
        {
            return Ok(Arc::clone(r));
        }
        let r = (self.loader)(path)?;
        *slot = Some((path.to_path_buf(), Arc::clone(&r)));
        Ok(r)
    }

    /// Drop the loaded model (a removed or switched model).
    pub fn unload(&self) {
        *lock(&self.recognizer) = None;
    }

    /// Esc, or the app going away: stop listening, discard the clip or a
    /// transcription in flight, back to idle.
    pub fn cancel(&self) {
        let cap = lock(&self.inner).capture.take();
        if let Some(mut cap) = cap {
            cap.source.stop();
        }
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.push(DictationState::of(StateName::Idle));
    }
}

/// Open the default microphone for `hold` (the OS asks for access the
/// first time), then close it.
pub fn mic_check(hold: Duration) -> Result<(), SpeechError> {
    #[cfg(feature = "speech")]
    {
        let mut mic = audio::Microphone::new();
        mic.start(Box::new(|_: &[f32]| {}))?;
        std::thread::sleep(hold);
        mic.stop();
        Ok(())
    }
    #[cfg(not(feature = "speech"))]
    {
        let _ = hold;
        Err(SpeechError::Unavailable)
    }
}

/// The default microphone as a [`Source`].
pub fn microphone() -> Result<Box<dyn Source>, SpeechError> {
    #[cfg(feature = "speech")]
    {
        Ok(Box::new(audio::Microphone::new()))
    }
    #[cfg(not(feature = "speech"))]
    {
        Err(SpeechError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_serialise_to_the_contract_shape() {
        let v = serde_json::to_value(DictationState::done(Target::Field, "one two three")).unwrap();
        assert_eq!(v["state"], "done");
        assert_eq!(v["target"], "field");
        assert_eq!(v["words"], 3);
        assert_eq!(v["text"], "one two three");
        assert!(v.get("error").is_none() && v.get("level").is_none());
        let e = serde_json::to_value(DictationState::error("x")).unwrap();
        assert_eq!(
            e,
            serde_json::json!({"state": "error", "error": "x", "source": "dictation"})
        );
        let r = serde_json::to_value(DictationState::request_error("y")).unwrap();
        assert_eq!(
            r,
            serde_json::json!({"state": "error", "error": "y", "source": "request"})
        );
        assert_eq!(first_words("a b c d", 2), "a b…");
        assert!(SpeechError::MicDenied.to_string().contains("Microphone"));
    }

    #[test]
    fn a_voice_reply_serialises_its_surface_and_citations() {
        let s = DictationState {
            state: StateName::Answering,
            target: Some(Target::Chat),
            thread_id: Some("th".into()),
            turn_id: Some("t1".into()),
            reply: Some("Today you have".into()),
            surface: Some(Surface::Card),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_value(&s).unwrap(),
            serde_json::json!({"state": "answering", "target": "chat", "thread_id": "th",
                "turn_id": "t1", "reply": "Today you have", "surface": "card"})
        );
        let mut done = DictationState::done(Target::Chat, "one two");
        done.citations = vec![Citation {
            n: 1,
            kind: "note".into(),
            id: "n1".into(),
            title: "Groceries".into(),
        }];
        done.surface = Some(Surface::Pill);
        let v = serde_json::to_value(&done).unwrap();
        assert_eq!(v["surface"], "pill");
        assert_eq!(
            v["citations"],
            serde_json::json!([{"n": 1, "kind": "note", "id": "n1", "title": "Groceries"}])
        );
        // Nothing new on a plain dictation state.
        let plain = serde_json::to_value(DictationState::done(Target::Field, "a")).unwrap();
        assert!(plain.get("citations").is_none() && plain.get("surface").is_none());
        let back: DictationState = serde_json::from_value(v).unwrap();
        assert_eq!(back, done);
    }
}
