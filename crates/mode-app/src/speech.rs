//! Dictation in the app (XC-37, XC-38; ADR-0061): the hold key's press and
//! release drive `mode_core::speech::Dictation` on one worker thread (so a
//! release can never overtake its press), and the transcript is routed:
//!
//! - **Dictation** (no wake phrase, or `dictation.commands` off): to the
//!   focused Mode window as `dictation:text`; it inserts the words at the
//!   caret and answers `dictation_inserted {ok}`. No Mode window focused,
//!   or no editable field (`ok = false`): `dictation.unfocused` decides,
//!   a captured note (`capture`, the default) or nothing.
//! - **"Hey Mode …"**: the rest goes to Chat as a message in the voice
//!   conversation (`dictation.thread_id`, created on first use) with what
//!   the app window shows attached (`app_context_set`). Where the answer
//!   shows is decided here, once (02 §5.7, ADR-0064): the app window in
//!   front → the pill (`thinking`, then the reply's first words); anything
//!   else → the overlay's reply card, fed by `answering` states carrying
//!   the reply as Chat streams it, then `done` with all of it.
//!
//! Every state is pushed to both windows as `dictation:state`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use mode_core::chat::api::{self as chat_api, AttachmentArg, RenameArgs, SendArgs};
use mode_core::notes::api::{self as notes_api, NoteCaptureArgs};
use mode_core::push::{Event, FlashKind, Push};
use mode_core::settings;
use mode_core::speech::models::{self, ModelProgress};
use mode_core::speech::wake::{self, Utterance};
use mode_core::speech::{
    self, Citation, DictationState, SpeechError, StartContext, StateName, Surface, Target,
    first_words,
};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::platform::{self, MicAccess};
use crate::state::CoreState;

/// The per-window event that carries the words (only the focused window).
pub const DICTATION_TEXT: &str = "dictation:text";
/// How long a window has to answer `dictation_inserted` before the words
/// count as typed.
const INSERT_GRACE: Duration = Duration::from_secs(2);
/// How long the Settings switch waits for the answer to the OS microphone
/// prompt before it gives up.
const PROMPT_WAIT: Duration = Duration::from_secs(120);
/// The voice conversation's title in Chat.
const VOICE_THREAD_TITLE: &str = "Voice";
/// The kinds `app_context_set` accepts (Chat's attachment kinds).
pub const CONTEXT_KINDS: &[&str] = &["note", "message", "session", "event"];
/// The fastest a streaming reply is pushed as `answering` (02 §5.7).
pub const ANSWER_PUSH: Duration = Duration::from_millis(80);

/// The "Hey Mode" request's Chat turn in flight: its end (and, for the
/// reply card, its text as it streams) becomes the dictation state.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceTurn {
    pub turn_id: String,
    pub thread_id: String,
    pub surface: Surface,
    /// The reply so far (the turn's `chat:delta` text).
    pub reply: String,
    /// When `answering` was last pushed; `None` before the first chunk.
    pub last_push: Option<Instant>,
}

enum Op {
    Press,
    Release,
    Cancel,
}

static WORKER: OnceLock<mpsc::Sender<(AppHandle, Op)>> = OnceLock::new();
static SEQ: AtomicU64 = AtomicU64::new(0);

fn send(app: &AppHandle, op: Op) {
    let tx = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<(AppHandle, Op)>();
        let spawned = std::thread::Builder::new()
            .name("mode-dictation".into())
            .spawn(move || {
                while let Ok((app, op)) = rx.recv() {
                    match op {
                        Op::Press => on_press(&app),
                        Op::Release => on_release(&app),
                        Op::Cancel => {
                            if let Some(core) = core(&app) {
                                *CoreState::lock(&core.dictation_pending) = None;
                                core.dictation.cancel();
                            }
                        }
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "dictation worker did not start");
        }
        tx
    });
    let _ = tx.send((app.clone(), op));
}

/// The hold key went down (or `dictation_press`).
pub fn press(app: &AppHandle) {
    send(app, Op::Press);
}

/// The hold key came up (or `dictation_release`).
pub fn release(app: &AppHandle) {
    send(app, Op::Release);
}

/// Esc while listening (or `dictation_cancel`).
pub fn cancel(app: &AppHandle) {
    send(app, Op::Cancel);
}

fn core(app: &AppHandle) -> Option<Arc<CoreState>> {
    app.try_state::<Arc<CoreState>>().map(|s| Arc::clone(&s))
}

/// Every dictation state goes to both windows. Dictation on with its model
/// file missing (a download that never finished) fetches the model now, so
/// the first hold after launch works.
pub fn install(app: &AppHandle, core: &Arc<CoreState>) {
    let a = app.clone();
    core.dictation.set_sink(move |s| {
        if let Err(e) = a.emit(Event::Dictation.wire_name(), s) {
            tracing::warn!(error = %e, "emit dictation.state failed");
        }
    });
    let m = model(core);
    if setting_bool(core, "dictation.enabled") && !models::is_installed(&core.data_dir, m) {
        tracing::info!(
            model = m.id,
            "dictation is on but its model is missing; downloading"
        );
        if let Err(e) = ensure_model(app, core, m.id) {
            tracing::warn!(model = m.id, error = %e, "dictation model download did not start");
        }
    }
}

fn setting(core: &CoreState, key: &str) -> Value {
    settings::get(&core.db, key).unwrap_or_else(|_| settings::default_for(key))
}

fn setting_bool(core: &CoreState, key: &str) -> bool {
    setting(core, key)
        .as_bool()
        .or_else(|| settings::default_for(key).as_bool())
        .unwrap_or(false)
}

fn setting_str(core: &CoreState, key: &str) -> Option<String> {
    setting(core, key)
        .as_str()
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

/// The configured model (the default when the setting names none we know).
pub fn model(core: &CoreState) -> &'static models::ModelDef {
    setting_str(core, "dictation.model")
        .and_then(|id| models::find(&id))
        .or_else(|| models::find(models::DEFAULT_MODEL))
        .expect("the default model is in the catalog")
}

/// What a hold transcribes with: the model file, and the language for the
/// multilingual model (`auto` detects).
pub fn start_context(core: &CoreState) -> StartContext {
    let m = model(core);
    StartContext {
        model_path: models::path(&core.data_dir, m),
        language: m
            .multilingual
            .then(|| setting_str(core, "dictation.language").unwrap_or_else(|| "auto".into())),
    }
}

fn on_press(app: &AppHandle) {
    let Some(core) = core(app) else {
        return;
    };
    if !setting_bool(&core, "dictation.enabled") {
        return;
    }
    // A hold before the model is there starts the download and says so; the
    // pill shows the reason for a moment (02 §5.6).
    let m = model(&core);
    if !models::is_installed(&core.data_dir, m) {
        tracing::info!(
            model = m.id,
            "hold before the model is downloaded; fetching it"
        );
        let reason = match ensure_model(app, &core, m.id) {
            Ok(()) => format!("downloading {} · hold again soon", m.id),
            Err(e) => e.to_string(),
        };
        core.dictation.push(DictationState::error(reason));
        return;
    }
    if let Err(e) = mic_allowed(false) {
        tracing::info!(error = %e, "dictation did not start");
        core.dictation.push(DictationState::error(e));
        return;
    }
    let started =
        speech::microphone().and_then(|mic| core.dictation.start(mic, start_context(&core)));
    if let Err(e) = started {
        tracing::info!(error = %e, "dictation did not start");
        core.dictation.push(DictationState::error(e.to_string()));
    }
}

/// The microphone as the OS allows it, before the device opens (08 §6.2):
/// macOS hands a refused process silence rather than an error, so TCC is
/// read first. While it is undecided, `wait` blocks for the prompt's answer
/// (the Settings switch, "Asking for the microphone…"); a press only raises
/// the prompt and asks for another hold.
pub fn mic_allowed(wait: bool) -> Result<(), String> {
    match platform::mic_access() {
        MicAccess::Allowed => Ok(()),
        MicAccess::Undecided if wait => match platform::request_mic(PROMPT_WAIT) {
            Some(true) => Ok(()),
            Some(false) => Err(mic_denied()),
            None => Err("the microphone prompt was not answered".into()),
        },
        MicAccess::Undecided => {
            std::thread::spawn(|| platform::request_mic(PROMPT_WAIT));
            Err("allow the microphone in the prompt, then hold again".into())
        }
        MicAccess::Denied | MicAccess::Restricted => Err(mic_denied()),
    }
}

/// The refused-microphone line; a dev build adds where TCC lists an
/// unbundled binary (under the terminal or editor that launched it).
fn mic_denied() -> String {
    let mut reason = SpeechError::MicDenied.to_string();
    if cfg!(debug_assertions) {
        reason
            .push_str(" · a dev run is listed there under the terminal or editor that launched it");
    }
    reason
}

fn on_release(app: &AppHandle) {
    let Some(core) = core(app) else {
        return;
    };
    let (a, c) = (app.clone(), Arc::clone(&core));
    core.dictation.release(move |result| match result {
        Ok(Some(text)) => route(&a, &c, text),
        Ok(None) => {}
        Err(e) => tracing::info!(error = %e, "dictation ended with an error"),
    });
}

/// `dictation_say` (dev builds only): `text` as if a hold had produced it.
#[cfg(debug_assertions)]
pub fn say(app: &AppHandle, core: &Arc<CoreState>, text: String) {
    tracing::info!("dictation_say: routing a typed transcript (dev build)");
    route(app, core, text);
}

/// A transcript: dictation or a request ("Hey Mode").
fn route(app: &AppHandle, core: &Arc<CoreState>, text: String) {
    let text = text.trim().to_string();
    if text.is_empty() {
        core.dictation.push(DictationState::done(Target::None, ""));
        return;
    }
    let utterance = if setting_bool(core, "dictation.commands") {
        wake::classify(&text)
    } else {
        Utterance::Dictation(text)
    };
    match utterance {
        Utterance::Dictation(words) => deliver(app, core, words),
        Utterance::Command(request) => ask_chat(core, request),
    }
}

/// The words to the focused Mode window; without one, the unfocused rule.
fn deliver(app: &AppHandle, core: &Arc<CoreState>, words: String) {
    let focused = *CoreState::lock(&core.focused_window);
    let Some(label) = focused else {
        unfocused(core, words);
        return;
    };
    let seq = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    *CoreState::lock(&core.dictation_pending) = Some((seq, words.clone()));
    if let Err(e) = app.emit_to(label, DICTATION_TEXT, json!({ "text": words })) {
        tracing::warn!(window = label, error = %e, "emit dictation:text failed");
        *CoreState::lock(&core.dictation_pending) = None;
        unfocused(core, words);
        return;
    }
    // A window that never answers (closed mid-way) still ends the pill.
    let c = Arc::clone(core);
    let _ = std::thread::Builder::new()
        .name("mode-dictation-grace".into())
        .spawn(move || {
            std::thread::sleep(INSERT_GRACE);
            let still = {
                let mut p = CoreState::lock(&c.dictation_pending);
                match p.as_ref() {
                    Some((s, _)) if *s == seq => p.take(),
                    _ => None,
                }
            };
            if let Some((_, words)) = still {
                tracing::info!("dictation: no dictation_inserted answer; counted as typed");
                c.dictation
                    .push(DictationState::done(Target::Field, &words));
            }
        });
}

/// `dictation_inserted {ok}`: the window typed the words, or had no field.
pub fn inserted(core: &Arc<CoreState>, ok: bool) {
    let Some((_, words)) = CoreState::lock(&core.dictation_pending).take() else {
        return;
    };
    if ok {
        core.dictation
            .push(DictationState::done(Target::Field, &words));
    } else {
        let c = Arc::clone(core);
        let _ = std::thread::Builder::new()
            .name("mode-dictation".into())
            .spawn(move || unfocused(&c, words));
    }
}

/// Nothing in Mode can take the words: `dictation.unfocused`.
fn unfocused(core: &Arc<CoreState>, words: String) {
    if setting_str(core, "dictation.unfocused").as_deref() == Some("nothing") {
        core.dictation
            .push(DictationState::done(Target::None, &words));
        return;
    }
    match capture(core, &words) {
        Ok(note_id) => {
            let mut s = DictationState::done(Target::Note, &words);
            s.note_id = Some(note_id);
            core.dictation.push(s);
        }
        Err(e) => core.dictation.push(DictationState::error(e)),
    }
}

/// The words as a captured note (04 §7.1, the capture box's path): a long
/// dictation keeps its first words as the title and all of it as the body.
fn capture(core: &CoreState, words: &str) -> Result<String, String> {
    let notes = core.notes().map_err(|e| e.message)?;
    let text = if speech::word_count(words) > 8 {
        let title = first_words(words, 8);
        format!("{}\n{words}", title.trim_end_matches('…'))
    } else {
        words.to_string()
    };
    let r = notes_api::note_capture(
        &notes,
        NoteCaptureArgs {
            text,
            created_from: None,
        },
    )
    .map_err(|e| e.message)?;
    tracing::info!(note = %r.note_id, "dictation captured as a note");
    crate::commands_notes::flash_captured(core, &r.note_id);
    Ok(r.note_id)
}

/// Where a request's answer shows (02 §5.7): the pill while the app
/// window has keyboard focus, else the overlay's reply card, unless
/// `dictation.reply_card` is off.
pub fn surface(core: &CoreState) -> Surface {
    let app_in_front = *CoreState::lock(&core.focused_window) == Some(crate::APP_WINDOW);
    if app_in_front || !setting_bool(core, "dictation.reply_card") {
        Surface::Pill
    } else {
        Surface::Card
    }
}

/// "Hey Mode …": a message in the voice conversation, with what the app
/// window shows attached. A voice turn still running there is stopped
/// first (a conversation runs one turn at a time; the latest request wins).
pub(crate) fn ask_chat(core: &Arc<CoreState>, request: String) {
    if request.is_empty() {
        let mut s = DictationState::done(Target::Chat, "");
        s.text = Some("What would you like?".into());
        core.dictation.push(s);
        return;
    }
    let chat = match core.chat() {
        Ok(c) => c,
        Err(_) => {
            core.dictation
                .push(DictationState::request_error("Chat is off"));
            return;
        }
    };
    let surface = surface(core);
    let previous = CoreState::lock(&core.voice_turn).take();
    if let Some(prev) = previous {
        tracing::info!(turn = %prev.turn_id, "a new voice request stops the previous one");
        let _ = chat_api::chat_stop(
            &chat,
            chat_api::TurnIdArgs {
                turn_id: prev.turn_id,
            },
        );
    }
    let attachments = context_item(core).map(|(kind, id)| vec![AttachmentArg { kind, id }]);
    let remembered = setting_str(core, "dictation.thread_id");
    let send = |thread_id: Option<String>, attachments: Option<Vec<AttachmentArg>>| {
        chat_api::chat_send(
            &chat,
            SendArgs {
                thread_id,
                text: request.clone(),
                effort: None,
                attachments,
                connectors: None,
            },
        )
    };
    let mut result = send(remembered.clone(), attachments.clone());
    if let Err(e) = &result
        && e.code == "not_found"
        && remembered.is_some()
    {
        // The voice conversation was deleted: start a new one.
        result = send(None, attachments.clone());
    }
    if let Err(e) = &result
        && e.code == "invalid"
        && attachments.is_some()
    {
        // The item on screen cannot be attached (gone, or a kind Chat refuses).
        result = send(remembered.clone(), None);
    }
    let sent = match result {
        Ok(r) => r,
        Err(e) => {
            core.dictation
                .push(DictationState::request_error(e.message));
            return;
        }
    };
    if remembered.as_deref() != Some(sent.thread_id.as_str()) {
        let _ = chat_api::chat_thread_rename(
            &chat,
            RenameArgs {
                id: sent.thread_id.clone(),
                title: VOICE_THREAD_TITLE.into(),
            },
        );
        if let Err(e) = settings::set(
            &core.db,
            "dictation.thread_id",
            &Value::String(sent.thread_id.clone()),
        ) {
            tracing::warn!(error = %e, "dictation.thread_id not saved");
        }
    }
    core.invalidate_palette();
    thinking(core, &sent.turn_id, &sent.thread_id, surface);
    catch_up(core, &chat, &sent.turn_id, &sent.thread_id);
}

/// A turn that ended before it was remembered as the voice turn (a refusal
/// is immediate: no key, the budget) had its `chat.turn` pass by unseen;
/// replay its end from the stored reply so the pill or card is not left
/// thinking. A turn ending meanwhile is handled once: whoever takes the
/// voice turn first.
fn catch_up(core: &CoreState, chat: &mode_core::chat::Chat, turn_id: &str, thread_id: &str) {
    let (thread, turn) = (thread_id.to_string(), turn_id.to_string());
    let stored = chat
        .db()
        .read(move |c| mode_core::chat::edge::reply(c, &thread, &turn));
    let Ok(Some(reply)) = stored else {
        return;
    };
    let status = match reply.status.as_deref() {
        Some("streaming") | None => return,
        Some("failed") => "failed",
        Some(_) => "finished",
    };
    let payload = json!({
        "turn_id": turn_id,
        "thread_id": thread_id,
        "status": status,
        "error": reply.error,
        "citations": reply.citations,
        "turn": reply,
    });
    on_voice_end(
        core,
        turn_id,
        &Push {
            event: Event::ChatTurn,
            payload,
        },
    );
}

/// The request's turn is running: remember it and push `thinking`.
fn thinking(core: &CoreState, turn_id: &str, thread_id: &str, surface: Surface) {
    *CoreState::lock(&core.voice_turn) = Some(VoiceTurn {
        turn_id: turn_id.to_string(),
        thread_id: thread_id.to_string(),
        surface,
        reply: String::new(),
        last_push: None,
    });
    core.dictation.push(DictationState {
        state: StateName::Thinking,
        target: Some(Target::Chat),
        thread_id: Some(thread_id.to_string()),
        turn_id: Some(turn_id.to_string()),
        surface: Some(surface),
        ..Default::default()
    });
}

/// The item a request carries: the overlay's (its panel is open) if it
/// set one, else the app window's.
pub fn context_item(core: &CoreState) -> Option<(String, String)> {
    let ctx = CoreState::lock(&core.context);
    ctx.get(crate::OVERLAY_WINDOW)
        .or_else(|| ctx.get(crate::APP_WINDOW))
        .cloned()
}

/// Chat's stream sink calls this for every push: the voice request's
/// reply text streams into `answering` (at most every [`ANSWER_PUSH`], the
/// first chunk at once), and its turn ending is `done` with the whole
/// reply and its citations (or `error`).
pub fn on_chat_push(core: &CoreState, push: &Push) {
    let turn = push.payload["turn_id"].as_str().unwrap_or_default();
    match push.event {
        Event::ChatDelta => on_voice_delta(core, turn, push),
        Event::ChatTurn => on_voice_end(core, turn, push),
        _ => {}
    }
}

fn on_voice_delta(core: &CoreState, turn: &str, push: &Push) {
    let Some(text) = push.payload["text"].as_str().filter(|t| !t.is_empty()) else {
        return;
    };
    let state = {
        let mut voice = CoreState::lock(&core.voice_turn);
        let Some(v) = voice.as_mut().filter(|v| v.turn_id == turn) else {
            return;
        };
        v.reply.push_str(text);
        let due = v.last_push.is_none_or(|t| t.elapsed() >= ANSWER_PUSH);
        if !due {
            return;
        }
        v.last_push = Some(Instant::now());
        DictationState {
            state: StateName::Answering,
            target: Some(Target::Chat),
            thread_id: Some(v.thread_id.clone()),
            turn_id: Some(v.turn_id.clone()),
            reply: Some(v.reply.clone()),
            surface: Some(v.surface),
            ..Default::default()
        }
    };
    core.dictation.push(state);
}

fn on_voice_end(core: &CoreState, turn: &str, push: &Push) {
    let status = push.payload["status"].as_str().unwrap_or_default();
    if status == "started" {
        return;
    }
    let voice = {
        let mut voice = CoreState::lock(&core.voice_turn);
        if voice.as_ref().map(|v| v.turn_id.as_str()) != Some(turn) {
            return;
        }
        voice.take().expect("checked above")
    };
    let thread_id = push.payload["thread_id"]
        .as_str()
        .map(str::to_string)
        .or(Some(voice.thread_id.clone()));
    if status == "failed" {
        let why = push.payload["error"]
            .as_str()
            .unwrap_or("Chat could not answer")
            .to_string();
        core.dictation.push(DictationState {
            thread_id,
            turn_id: Some(voice.turn_id),
            surface: Some(voice.surface),
            ..DictationState::request_error(why)
        });
        return;
    }
    let reply = push.payload["turn"]["text"]
        .as_str()
        .map(str::to_string)
        .unwrap_or(voice.reply);
    let mut s = DictationState::done(Target::Chat, &reply);
    s.thread_id = thread_id;
    s.turn_id = Some(voice.turn_id);
    s.surface = Some(voice.surface);
    s.citations = citations(push);
    s.reply = Some(reply);
    core.dictation.push(s);
}

/// The ended turn's citations (`chat.turn`'s, else the stored reply's).
fn citations(push: &Push) -> Vec<Citation> {
    let list = push.payload["citations"]
        .as_array()
        .filter(|a| !a.is_empty())
        .or_else(|| push.payload["turn"]["citations"].as_array());
    list.map(|a| {
        a.iter()
            .filter_map(|c| {
                Some(Citation {
                    n: u32::try_from(c["n"].as_u64()?).ok()?,
                    kind: c["kind"].as_str()?.to_string(),
                    id: c["id"].as_str()?.to_string(),
                    title: c["title"].as_str().unwrap_or_default().to_string(),
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

/// `dictation_model_ensure`: download in the background; progress on
/// `dictation:model`. A model already downloading is not started twice.
pub fn ensure_model(app: &AppHandle, core: &Arc<CoreState>, id: &str) -> Result<(), SpeechError> {
    let m = models::find(id).ok_or_else(|| SpeechError::UnknownModel(id.into()))?;
    if !CoreState::lock(&core.model_downloads).insert(m.id.to_string()) {
        return Ok(());
    }
    let (a, c) = (app.clone(), Arc::clone(core));
    let spawned = std::thread::Builder::new()
        .name("mode-model".into())
        .spawn(move || {
            let emit = |p: ModelProgress| {
                if let Err(e) = a.emit(Event::DictationModel.wire_name(), &p) {
                    tracing::warn!(error = %e, "emit dictation.model failed");
                }
            };
            let result = mode_core::connectors::http::ReqwestClient::new()
                .map_err(|e| SpeechError::Download(e.to_string()))
                .and_then(|client| models::ensure(&c.data_dir, m.id, &client, &mut |p| emit(p)));
            match result {
                Ok(path) => {
                    tracing::info!(model = m.id, path = %path.display(), "dictation model ready");
                    c.flash(format!("model ready · {}", m.id), FlashKind::Ok, None);
                }
                Err(e) => {
                    tracing::warn!(model = m.id, error = %e, "dictation model download failed");
                    emit(ModelProgress {
                        model: m.id.into(),
                        downloaded: 0,
                        total: m.size_bytes,
                        done: true,
                        error: Some(e.to_string()),
                    });
                }
            }
            CoreState::lock(&c.model_downloads).remove(m.id);
        });
    if let Err(e) = spawned {
        CoreState::lock(&core.model_downloads).remove(m.id);
        return Err(SpeechError::Io(e.to_string()));
    }
    Ok(())
}

/// A Mode window gained or lost keyboard focus.
pub fn on_focus(core: &CoreState, label: &'static str, focused: bool) {
    let mut f = CoreState::lock(&core.focused_window);
    if focused {
        *f = Some(label);
    } else if *f == Some(label) {
        *f = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(turn: &str, surface: Surface) -> VoiceTurn {
        VoiceTurn {
            turn_id: turn.into(),
            thread_id: "th".into(),
            surface,
            reply: String::new(),
            last_push: None,
        }
    }

    fn recorder(core: &CoreState) -> Arc<std::sync::Mutex<Vec<DictationState>>> {
        let seen: Arc<std::sync::Mutex<Vec<DictationState>>> = Arc::default();
        let s = Arc::clone(&seen);
        core.dictation
            .set_sink(move |st| s.lock().unwrap().push(st.clone()));
        seen
    }

    fn push(event: Event, payload: Value) -> Push {
        Push { event, payload }
    }

    #[test]
    fn a_voice_turn_ending_is_the_pills_done() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::wiring_tests::core_with(dir.path());
        let seen = recorder(&core);
        *CoreState::lock(&core.voice_turn) = Some(voice("t1", Surface::Pill));
        on_chat_push(
            &core,
            &push(
                Event::ChatTurn,
                json!({"turn_id": "t1", "thread_id": "th", "status": "started"}),
            ),
        );
        on_chat_push(
            &core,
            &push(
                Event::ChatTurn,
                json!({"turn_id": "t0", "thread_id": "th", "status": "finished"}),
            ),
        );
        on_chat_push(
            &core,
            &push(
                Event::ChatDelta,
                json!({"turn_id": "t0", "thread_id": "th", "text": "other"}),
            ),
        );
        assert!(
            seen.lock().unwrap().is_empty(),
            "only the voice turn's text and end count"
        );
        on_chat_push(
            &core,
            &push(
                Event::ChatTurn,
                json!({"turn_id": "t1", "thread_id": "th", "status": "finished",
                "citations": [{"n": 1, "kind": "note", "id": "n1", "title": "Retry policy", "snippet": "…"}],
                "turn": {"text": "The retry policy note says three attempts with backoff, then give up. [1]"}}),
            ),
        );
        let last = seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.state, StateName::Done);
        assert_eq!(last.target, Some(Target::Chat));
        assert_eq!(last.thread_id.as_deref(), Some("th"));
        assert_eq!(last.turn_id.as_deref(), Some("t1"));
        assert_eq!(last.surface, Some(Surface::Pill));
        assert_eq!(
            last.text.as_deref(),
            Some("The retry policy note says three attempts with…")
        );
        assert_eq!(
            last.reply.as_deref(),
            Some("The retry policy note says three attempts with backoff, then give up. [1]")
        );
        assert_eq!(
            last.citations,
            vec![Citation {
                n: 1,
                kind: "note".into(),
                id: "n1".into(),
                title: "Retry policy".into()
            }]
        );
        assert!(CoreState::lock(&core.voice_turn).is_none());

        // A request Chat could not answer is a request error, not a dictation one.
        *CoreState::lock(&core.voice_turn) = Some(voice("t2", Surface::Card));
        on_chat_push(
            &core,
            &push(
                Event::ChatTurn,
                json!({"turn_id": "t2", "thread_id": "th", "status": "failed", "error": "no key"}),
            ),
        );
        let last = seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.state, StateName::Error);
        assert_eq!(last.error.as_deref(), Some("no key"));
        assert_eq!(last.source, Some(speech::ErrorSource::Request));
        assert_eq!(last.turn_id.as_deref(), Some("t2"));
        assert_eq!(last.surface, Some(Surface::Card), "the card shows why");
    }

    #[test]
    fn the_voice_replys_text_streams_as_answering_at_most_every_80_ms() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::wiring_tests::core_with(dir.path());
        let seen = recorder(&core);
        *CoreState::lock(&core.voice_turn) = Some(voice("t1", Surface::Card));
        let delta = |text: &str| {
            push(
                Event::ChatDelta,
                json!({"turn_id": "t1", "thread_id": "th", "text": text, "kind": "text"}),
            )
        };
        on_chat_push(&core, &delta("Today you have "));
        on_chat_push(&core, &delta("three tasks"));
        // An activity chunk carries no text.
        on_chat_push(
            &core,
            &push(
                Event::ChatDelta,
                json!({"turn_id": "t1", "thread_id": "th", "kind": "activity"}),
            ),
        );
        {
            let states = seen.lock().unwrap();
            assert_eq!(states.len(), 1, "the first chunk at once, the next held");
            assert_eq!(states[0].state, StateName::Answering);
            assert_eq!(states[0].reply.as_deref(), Some("Today you have "));
            assert_eq!(states[0].surface, Some(Surface::Card));
            assert_eq!(states[0].turn_id.as_deref(), Some("t1"));
            assert_eq!(states[0].thread_id.as_deref(), Some("th"));
        }
        std::thread::sleep(ANSWER_PUSH + Duration::from_millis(10));
        on_chat_push(&core, &delta(": groceries."));
        let last = seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(
            last.reply.as_deref(),
            Some("Today you have three tasks: groceries."),
            "the whole reply so far, never a delta"
        );
        // The end carries everything, even text that was never pushed.
        on_chat_push(&core, &delta(" More."));
        on_chat_push(
            &core,
            &push(
                Event::ChatTurn,
                json!({"turn_id": "t1", "thread_id": "th", "status": "stopped", "stopped": true}),
            ),
        );
        let last = seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last.state, StateName::Done);
        assert_eq!(
            last.reply.as_deref(),
            Some("Today you have three tasks: groceries. More.")
        );
    }

    #[test]
    fn the_answer_shows_where_the_user_is_looking() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::wiring_tests::core_with(dir.path());
        assert_eq!(surface(&core), Surface::Card, "no Mode window in front");
        on_focus(&core, crate::OVERLAY_WINDOW, true);
        assert_eq!(surface(&core), Surface::Card, "the overlay's capture box");
        on_focus(&core, crate::APP_WINDOW, true);
        assert_eq!(surface(&core), Surface::Pill, "the app window in front");
        settings::set(&core.db, "dictation.reply_card", &json!(false)).unwrap();
        on_focus(&core, crate::APP_WINDOW, false);
        assert_eq!(surface(&core), Surface::Pill, "the card switched off");
    }

    #[test]
    fn the_overlays_item_wins_over_the_app_windows() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::wiring_tests::core_with(dir.path());
        assert_eq!(context_item(&core), None);
        let item = |k: &str, id: &str| (k.to_string(), id.to_string());
        CoreState::lock(&core.context).insert(crate::APP_WINDOW.into(), item("note", "n1"));
        assert_eq!(context_item(&core), Some(item("note", "n1")));
        CoreState::lock(&core.context).insert(crate::OVERLAY_WINDOW.into(), item("message", "m1"));
        assert_eq!(context_item(&core), Some(item("message", "m1")));
        CoreState::lock(&core.context).remove(crate::OVERLAY_WINDOW);
        assert_eq!(context_item(&core), Some(item("note", "n1")));
    }

    #[test]
    fn focus_follows_the_last_window_that_gained_it() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::wiring_tests::core_with(dir.path());
        on_focus(&core, crate::APP_WINDOW, true);
        on_focus(&core, crate::OVERLAY_WINDOW, true);
        on_focus(&core, crate::APP_WINDOW, false);
        assert_eq!(
            *CoreState::lock(&core.focused_window),
            Some(crate::OVERLAY_WINDOW)
        );
        on_focus(&core, crate::OVERLAY_WINDOW, false);
        assert_eq!(*CoreState::lock(&core.focused_window), None);
    }

    #[test]
    fn the_start_context_follows_the_model_setting() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::wiring_tests::core_with(dir.path());
        let ctx = start_context(&core);
        assert!(
            ctx.model_path
                .ends_with("models/whisper/ggml-base.en-q5_1.bin")
        );
        assert_eq!(ctx.language, None);
        settings::set(&core.db, "dictation.model", &json!("base")).unwrap();
        settings::set(&core.db, "dictation.language", &json!("de")).unwrap();
        assert_eq!(start_context(&core).language.as_deref(), Some("de"));
    }

    #[test]
    fn unfocused_words_become_a_note_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::wiring_tests::core_with(dir.path());
        let flashes = crate::wiring_tests::record_flashes(&core);
        settings::set(&core.db, "dictation.unfocused", &json!("nothing")).unwrap();
        unfocused(&core, "call Priya".into());
        let s = core.dictation.state();
        assert_eq!((s.state, s.target), (StateName::Done, Some(Target::None)));
        assert!(
            flashes.lock().unwrap().is_empty(),
            "nothing captured, no flash"
        );

        // The default, `capture`: a note, and "captured · note" in the notch.
        let vault = dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let notes = mode_core::notes::Notes::new(
            Arc::clone(&core.db),
            mode_core::notes::NotesOptions {
                root: Some(vault),
                ..Default::default()
            },
        )
        .unwrap();
        notes.init().unwrap();
        assert!(core.notes.set(notes).is_ok());
        settings::set(&core.db, "dictation.unfocused", &json!("capture")).unwrap();
        unfocused(&core, "call Priya".into());
        let s = core.dictation.state();
        assert_eq!((s.state, s.target), (StateName::Done, Some(Target::Note)));
        let f = flashes.lock().unwrap().last().cloned().unwrap();
        assert_eq!(f.text, "captured · note");
        assert_eq!(f.kind, mode_core::push::FlashKind::Ok);
        let open = f.open.unwrap();
        assert_eq!((open.view.as_str(), open.id), ("notes", s.note_id));
    }
}
