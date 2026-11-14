//! The package 11 services in the app (03 §7, §7B, 10 §3–§5, §7.4.15):
//! Chat over the connectors' AI backends, the automations scheduler, the
//! theme packs and the command palette. [`create`] builds them at setup
//! (before the publisher exists); [`start`] attaches the PTY host, the
//! publisher and the run loop once the publisher task runs. The Chat
//! streams go straight to the app window; `automations:changed` and
//! `theme:changed` go through the debounced publisher (see
//! [`crate::payloads`]), and both are pushed once at launch.

use std::path::PathBuf;
use std::sync::Arc;

use mode_core::chat::actions::LiveEffects;
use mode_core::chat::{Chat, ChatOptions};
use mode_core::connectors::Connectors;
use mode_core::notes::Notes;
use mode_core::packs::api::Packs;
use mode_core::palette::Palette;
use mode_core::pty::Pty;
use mode_core::push::{Event, FlashKind, OverlayFlash, Publisher, Push};
use mode_core::scheduler::dispatch::LiveDispatch;
use mode_core::scheduler::{FailedRun, Scheduler, SchedulerOptions, Wake};
use mode_core::themes::{OsAppearance, Themes};
use mode_core::urgency::alerts::{AlertEvent, AutomationFailed};
use tauri::{AppHandle, Emitter, Manager, Theme};
use tokio::sync::mpsc;

use crate::state::CoreState;

/// Where `theme_export` writes without a path: Downloads, else the data dir.
pub fn export_dir(data_dir: &std::path::Path) -> PathBuf {
    dirs::download_dir().unwrap_or_else(|| data_dir.join("exports"))
}

/// The OS appearance the themes resolver reads: the app window's theme
/// (tao follows the OS), the platform's reduced-motion flag.
pub fn os_appearance(app: &AppHandle, reduce_motion: Option<bool>) -> OsAppearance {
    let dark = app
        .get_webview_window(crate::APP_WINDOW)
        .and_then(|w| w.theme().ok())
        .is_some_and(|t| t == Theme::Dark);
    OsAppearance {
        dark,
        reduce_motion: reduce_motion.unwrap_or(false),
        ..Default::default()
    }
}

/// Report the OS appearance; `theme:changed` follows when it changed.
pub fn feed_appearance(app: &AppHandle, core: &CoreState, reduce_motion: Option<bool>) {
    if let Some(t) = core.themes.get()
        && t.set_os(os_appearance(app, reduce_motion))
    {
        tracing::info!("OS appearance changed: theme.changed requested");
    }
}

/// A failed run with `notify_on_failure` → `automation.failed` (07 §8.1).
pub fn failed_event(r: &FailedRun) -> AlertEvent {
    AlertEvent::AutomationFailed(AutomationFailed {
        automation_id: r.automation_id.clone(),
        automation_name: r.automation_name.clone(),
        run_id: r.run_id.clone(),
        error: r.error.clone(),
    })
}

/// The Chat stream sink: every event to the app window, in order; a turn
/// that ended may have created a conversation, so the palette goes stale.
fn chat_sink(app: AppHandle, core: Arc<CoreState>) -> impl Fn(&Push) + Send + Sync + 'static {
    move |push: &Push| {
        if let Err(e) = app.emit_to(crate::APP_WINDOW, push.event.wire_name(), &push.payload) {
            tracing::warn!(event = push.event.name(), error = %e, "emit failed");
        }
        if push.event == Event::ChatTurn && push.payload["status"] != "started" {
            core.invalidate_palette();
        }
        // A voice request's turn has the dictation pill (thinking, done).
        let voice = push.event == Event::ChatTurn
            && CoreState::lock(&core.voice_turn)
                .as_ref()
                .map(|v| v.turn_id.as_str())
                == push.payload["turn_id"].as_str();
        if !voice && let Some(f) = chat_flash(push) {
            core.send_flash(&f);
        }
        // A "Hey Mode" request's reply and end are the dictation state (XC-38, 02 §5.7).
        crate::speech::on_chat_push(&core, push);
    }
}

/// The notch's view of a Chat turn (02 §5.4, §11.2): "thinking · Mode"
/// held while it runs, then "answered · <first five words>" (a click opens
/// the conversation), or why it ended without an answer.
pub fn chat_flash(push: &Push) -> Option<OverlayFlash> {
    if push.event != Event::ChatTurn {
        return None;
    }
    let p = &push.payload;
    let thread = p["thread_id"].as_str();
    let status = p["status"].as_str()?;
    let f = match status {
        "started" => OverlayFlash::new("thinking · Mode", FlashKind::Info).keeping_pill(),
        _ if p["stopped"] == true => OverlayFlash::new("stopped · Mode", FlashKind::Info),
        "finished" => {
            let reply = p["turn"]["text"].as_str().unwrap_or_default();
            let words = mode_core::speech::first_words(reply.trim(), 5);
            let text = if words.is_empty() {
                "answered · Mode".to_string()
            } else {
                format!("answered · {words}")
            };
            OverlayFlash::new(text, FlashKind::Ok)
        }
        _ => OverlayFlash::new("Mode · no answer", FlashKind::Bad),
    };
    Some(f.opening("chat", thread))
}

/// Build the services at setup. Chat needs the connectors service (its AI
/// backends); the scheduler is not started on a scratch database. Returns
/// the scheduler's wake channel for [`start`].
pub fn create(
    app: &AppHandle,
    core: &Arc<CoreState>,
    notes: Option<&Arc<Notes>>,
    connectors: Option<&Arc<Connectors>>,
    reduce_motion: Option<bool>,
) -> Option<mpsc::UnboundedReceiver<Wake>> {
    let db = Arc::clone(&core.db);
    match Themes::new(Arc::clone(&db), export_dir(&core.data_dir)) {
        Ok(t) => {
            t.set_os(os_appearance(app, reduce_motion));
            let _ = core.themes.set(t);
        }
        Err(e) => tracing::error!(error = %e, "theme packs unavailable"),
    }
    let palette = Palette::new(Arc::clone(&db));
    palette.set_background(true);
    let _ = core.palette.set(palette);
    let chat = connectors.map(|c| {
        let chat = Chat::new(
            Arc::clone(&db),
            core.keyring(),
            Arc::clone(c.ai_factory()),
            ChatOptions::default(),
        );
        chat.set_sink(chat_sink(app.clone(), Arc::clone(core)));
        let _ = core.chat.set(Arc::clone(&chat));
        chat
    });
    if core.is_db_newer() {
        tracing::info!("automations not started: the database is newer than this build");
        create_packs(core, connectors, None);
        return None;
    }
    let dispatch = LiveDispatch {
        notes: notes.cloned(),
        pty: None,
        chat,
    };
    let s = Scheduler::new(db, Arc::new(dispatch), SchedulerOptions::default());
    if let Some(c) = connectors {
        let c = Arc::clone(c);
        s.set_failure_sink(move |r| c.alert(failed_event(r)));
    }
    let _ = core.scheduler.set(Arc::clone(&s));
    create_packs(core, connectors, Some(s));
    let (tx, rx) = mpsc::unbounded_channel();
    let _ = core.scheduler_wake.set(tx);
    tracing::info!(
        chat = core.chat.get().is_some(),
        "chat, automations, themes and palette created"
    );
    Some(rx)
}

/// The packs service (03 §7C.6): the themes service, the scheduler when it
/// runs, the connectors' HTTP client for the catalog, the export folder.
fn create_packs(
    core: &Arc<CoreState>,
    connectors: Option<&Arc<Connectors>>,
    scheduler: Option<Arc<Scheduler>>,
) {
    let (Some(t), Some(c)) = (core.themes.get(), connectors) else {
        tracing::info!("marketplace not started: no themes service or no HTTP client");
        return;
    };
    let p = Packs::new(
        Arc::clone(&core.db),
        scheduler,
        Arc::clone(t),
        Arc::clone(c.http()),
        export_dir(&core.data_dir),
    );
    let _ = core.packs.set(p);
}

/// Once the publisher and the PTY host exist: Chat's confirmed actions and
/// the scheduler's dispatch get the PTY host, the scheduler and the themes
/// get the publisher, the scheduler's loop starts (catch-up first), and
/// `theme:changed` / `automations:changed` are pushed once.
pub fn start(
    core: &Arc<CoreState>,
    publisher: &Publisher,
    pty: Option<Pty>,
    wake: Option<mpsc::UnboundedReceiver<Wake>>,
) {
    let notes = core.notes.get().cloned();
    let connectors = core.connectors.get().cloned();
    let pty = pty.map(Arc::new);
    let chat = core.chat.get().cloned();
    let scheduler = core.scheduler.get().cloned();
    if let Some(chat) = &chat {
        chat.set_effects(Arc::new(LiveEffects {
            notes: notes.clone(),
            connectors,
            pty: pty.clone(),
            scheduler: scheduler.as_ref().map(Arc::downgrade),
        }));
    }
    if let Some(s) = &scheduler {
        s.set_dispatch(Arc::new(LiveDispatch { notes, pty, chat }));
        let p = publisher.clone();
        s.set_publisher(move |e| p.publish(e));
        if let Some(rx) = wake {
            tauri::async_runtime::spawn(Arc::clone(s).run(rx));
            tracing::info!("automations scheduler started");
        }
    }
    if let Some(t) = core.themes.get() {
        let p = publisher.clone();
        t.set_publisher(move |e| p.publish(e));
    }
    core.publish([Event::Theme, Event::Automations]);
}

/// Quit (03 §2.1, 08 §2.6): stop the running Chat turns (their partial
/// text kept) and the automation runs still executing.
pub fn stop(core: &CoreState) {
    let turns = core.chat.get().map(|c| c.stop_all()).unwrap_or(0);
    let runs = core.scheduler.get().map(|s| s.shutdown()).unwrap_or(0);
    if turns + runs > 0 {
        tracing::info!(turns, runs, "quit: chat turns and automation runs stopped");
    }
}

/// The app window's `ThemeChanged` (tao follows the OS appearance).
pub fn on_theme_changed(core: &CoreState, dark: bool, reduce_motion: Option<bool>) {
    if let Some(t) = core.themes.get() {
        let os = OsAppearance {
            dark,
            reduce_motion: reduce_motion.unwrap_or(false),
            ..t.os()
        };
        if t.set_os(os) {
            tracing::info!(dark, "OS appearance changed: theme.changed requested");
        }
    }
}
