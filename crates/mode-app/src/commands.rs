//! `#[tauri::command]`s (08 §5.3): validation and capability scoping, no
//! business logic. Argument names are the snake_case wire names of
//! `ui/src/state/contract.ts`.

use std::path::PathBuf;
use std::sync::Arc;

use mode_core::diag::Snapshot;
use mode_core::hooks_install::{self, Plan, RoundTripReport, Status, Target, Timeouts};
use mode_core::push::Event;
use mode_core::sessions::{DecideOutcome, Decision, SessionDetail, SessionItem};
use mode_core::{hooks_db, settings};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::app_window::{self, AppOpen};
use crate::hotkeys::{self, HotkeyKind};
use crate::media::{Command as MediaCommand, Media};
use crate::overlay::geometry::Os;
use crate::overlay::ladder::{self, Request};
use crate::overlay::{Overlay, prefs};
use crate::state::CoreState;
use crate::{health, jump};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Round trip through the IPC layer; the overlay calls it at start.
#[tauri::command]
pub fn ping() -> &'static str {
    "pong"
}

/// What the Diagnostics view shows about the core.
#[tauri::command]
pub fn core_status(state: State<'_, Arc<CoreState>>) -> Snapshot {
    state.snapshot()
}

// ----------------------------------------------------------------- sessions

/// The roster with each open card (the overlay calls it on mount).
#[tauri::command(rename_all = "snake_case")]
pub async fn sessions_list(state: State<'_, Arc<CoreState>>) -> CmdResult<Vec<SessionItem>> {
    let engine = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(move || engine.roster())
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_detail(
    state: State<'_, Arc<CoreState>>,
    session_id: String,
) -> CmdResult<Option<SessionDetail>> {
    let engine = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(move || engine.detail(&session_id))
        .await
        .map_err(err)?
        .map_err(err)
}

/// Answer the open card (05 §7.1, §7.3); the first decision wins (§7.4).
#[tauri::command(rename_all = "snake_case")]
pub async fn session_decide(
    state: State<'_, Arc<CoreState>>,
    session_id: String,
    decision: Decision,
) -> CmdResult<DecideOutcome> {
    let engine = Arc::clone(&state.engine);
    let outcome =
        tauri::async_runtime::spawn_blocking(move || engine.decide(&session_id, decision))
            .await
            .map_err(err)?
            .map_err(err)?;
    state.publish([Event::Sessions, Event::Pill]);
    Ok(outcome)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_acknowledge(
    state: State<'_, Arc<CoreState>>,
    session_id: String,
) -> CmdResult<()> {
    let engine = Arc::clone(&state.engine);
    let pushes = tauri::async_runtime::spawn_blocking(move || engine.acknowledge(&session_id))
        .await
        .map_err(err)?
        .map_err(err)?;
    state.publish(pushes);
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn session_dismiss(
    state: State<'_, Arc<CoreState>>,
    session_id: String,
) -> CmdResult<()> {
    let engine = Arc::clone(&state.engine);
    let sid = session_id.clone();
    let pushes = tauri::async_runtime::spawn_blocking(move || engine.dismiss(&sid))
        .await
        .map_err(err)?
        .map_err(err)?;
    // A dismissed row's terminal replay goes with it (package 8).
    if let Some(pty) = state.pty.get() {
        let dropped = pty.forget(&session_id);
        if dropped > 0 {
            tracing::info!(session = %session_id, dropped, "pty replay buffers forgotten");
        }
    }
    state.publish(pushes);
    Ok(())
}

/// Jump to terminal (05 §9.3): focus the session's terminal tab or window.
/// `None` when Mode focused it (or opened the app window's Terminal tab for
/// an embedded session); otherwise a one-line hint saying why it could not.
#[tauri::command(rename_all = "snake_case")]
pub async fn session_jump(
    app: AppHandle,
    state: State<'_, Arc<CoreState>>,
    session_id: String,
) -> CmdResult<Option<String>> {
    let engine = Arc::clone(&state.engine);
    let sid = session_id.clone();
    let info = tauri::async_runtime::spawn_blocking(move || engine.jump_info(&sid))
        .await
        .map_err(err)?
        .map_err(err)?
        .ok_or_else(|| format!("no session {session_id}"))?;
    let target = jump::classify(&info, Os::current());
    tracing::info!(session = %session_id, target = ?target, "session_jump");
    if target == jump::Target::Embedded {
        app_window::open(
            &app,
            AppOpen::new("sessions", Some(info.session_id), None, Some("terminal")),
        );
        return Ok(None);
    }
    let db = Arc::clone(&state.db);
    let hint = tauri::async_runtime::spawn_blocking(move || {
        let default = settings::get_string(&db, "sessions.terminal_app")
            .ok()
            .flatten();
        jump::run(&target, default.as_deref())
    })
    .await
    .map_err(err)?;
    if let Some(h) = &hint {
        tracing::info!(session = %session_id, hint = %h, "session_jump could not focus the host");
    }
    Ok(hint)
}

// -------------------------------------------------------------------- hooks

fn target(state: &CoreState) -> CmdResult<Target> {
    hooks_install::default_target(&state.data_dir, mode_core::is_dev_build()).map_err(err)
}

fn timeouts(state: &CoreState) -> Timeouts {
    let get = |key: &str, default: u64| {
        settings::get_i64(&state.db, key)
            .ok()
            .flatten()
            .and_then(|v| u64::try_from(v).ok())
            .unwrap_or(default)
    };
    Timeouts {
        decision_secs: get("hooks.decision_timeout_seconds", 120),
        async_secs: get("hooks.async_timeout_seconds", 5),
    }
}

/// The last synthetic round trip: this run's, else the one kept in
/// `settings['hooks.last_round_trip']` (the Integrations card survives a
/// restart).
pub fn last_round_trip(state: &CoreState) -> Option<RoundTripReport> {
    if let Some(r) = CoreState::lock(&state.hooks)
        .as_ref()
        .and_then(|s| s.last_round_trip.clone())
    {
        return Some(r);
    }
    settings::get(&state.db, settings::LAST_ROUND_TRIP_KEY)
        .ok()
        .filter(|v| !v.is_null())
        .and_then(|v| serde_json::from_value(v).ok())
}

/// `hooks_status` (`HooksStatus`): the inspection plus the `hook_installs`
/// facts of the Claude Code card (03 §7C.3).
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusView {
    #[serde(flatten)]
    pub status: Status,
    pub installed_at: Option<String>,
    pub verified_at: Option<String>,
    pub backup_path: Option<String>,
}

pub fn status_view(state: &CoreState, status: Status) -> StatusView {
    let sp = status.path.to_string_lossy().into_owned();
    let row = state
        .db
        .read(|c| hooks_db::row_for(c, &sp))
        .ok()
        .flatten()
        .filter(|r| r.removed_at.is_none());
    StatusView {
        installed_at: row.as_ref().map(|r| r.installed_at.clone()),
        verified_at: row.as_ref().and_then(|r| r.verified_at.clone()),
        backup_path: row.and_then(|r| r.backup_path),
        status,
    }
}

/// `hooks_install_preview` (`HooksPlan`): the plan plus where the backup
/// goes and the relay version, for the dialog header.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlanView {
    #[serde(flatten)]
    pub plan: Plan,
    pub backup_dir: String,
    pub relay_version: Option<String>,
}

/// Re-read the hook install and cache it (the 5-minute relay check, 05 §3.6).
pub fn reinspect_hooks(state: &CoreState) -> CmdResult<Status> {
    let target = target(state)?;
    inspect_and_cache(state, &target)
}

fn inspect_and_cache(state: &CoreState, target: &Target) -> CmdResult<Status> {
    let settings_path = target.settings_path.to_string_lossy().into_owned();
    let sp = settings_path.clone();
    let previously = state
        .db
        .read(|c| hooks_db::previously_installed(c, &sp))
        .map_err(err)?;
    let mut status = hooks_install::inspect(target, &timeouts(state), previously).map_err(err)?;
    status.last_round_trip = last_round_trip(state);
    let s = status.clone();
    let now = mode_core::db::now_rfc3339();
    let _ = state
        .db
        .write(move |c| hooks_db::record_status(c, &s, &now));
    *CoreState::lock(&state.hooks) = Some(status.clone());
    let _ = settings_path;
    Ok(status)
}

/// The Claude Code card (03 §7C.2): re-read the settings file and classify.
#[tauri::command(rename_all = "snake_case")]
pub async fn hooks_status(
    app: AppHandle,
    state: State<'_, Arc<CoreState>>,
) -> CmdResult<StatusView> {
    let st = Arc::clone(&state);
    let status = tauri::async_runtime::spawn_blocking(move || {
        let s = reinspect_hooks(&st)?;
        Ok::<_, String>(status_view(&st, s))
    })
    .await
    .map_err(err)??;
    health::emit(app, Arc::clone(&state)).await;
    Ok(status)
}

/// The install (or, `action: "uninstall"`, the uninstall) preview: unified
/// diff, events, conflicts (05 §3.6 S1–S5), the backup folder and the relay
/// version.
#[tauri::command(rename_all = "snake_case")]
pub async fn hooks_install_preview(
    state: State<'_, Arc<CoreState>>,
    action: Option<String>,
) -> CmdResult<PlanView> {
    let uninstall = match action.as_deref() {
        None | Some("install") => false,
        Some("uninstall") => true,
        Some(other) => return Err(format!("{other:?} is not install or uninstall")),
    };
    let st = Arc::clone(&state);
    tauri::async_runtime::spawn_blocking(move || {
        let target = target(&st)?;
        let plan = if uninstall {
            hooks_install::plan_uninstall(&target)
        } else {
            hooks_install::plan_install(&target, &timeouts(&st))
        }
        .map_err(err)?;
        Ok(PlanView {
            plan,
            backup_dir: st.backups_dir().to_string_lossy().into_owned(),
            relay_version: Some(hooks_install::RELAY_VERSION.to_string()),
        })
    })
    .await
    .map_err(err)?
}

/// Backup, write, re-read and verify (05 §3.6 S7–S9); records the
/// `hook_installs` row.
#[tauri::command(rename_all = "snake_case")]
pub async fn hooks_install_apply(
    app: AppHandle,
    state: State<'_, Arc<CoreState>>,
) -> CmdResult<hooks_install::Applied> {
    let st = Arc::clone(&state);
    let applied = tauri::async_runtime::spawn_blocking(move || {
        let target = target(&st)?;
        let backups = st.data_dir.join("backups");
        let applied =
            hooks_install::install(&target, &timeouts(&st), Some(&backups)).map_err(err)?;
        let events: Vec<String> = hooks_install::EVENTS
            .iter()
            .map(|e| e.name.to_string())
            .collect();
        let (t, a) = (target.clone(), applied.clone());
        st.db
            .write(move |c| {
                hooks_db::record_install(c, &t, &a, &events, &mode_core::db::now_rfc3339())
            })
            .map_err(err)?;
        inspect_and_cache(&st, &target)?;
        Ok::<_, String>(applied)
    })
    .await
    .map_err(err)??;
    health::emit(app, Arc::clone(&state)).await;
    state.flash(
        "hooks installed",
        mode_core::push::FlashKind::Ok,
        Some(mode_core::push::FlashOpen {
            view: "integrations".into(),
            id: Some("claude-code".into()),
        }),
    );
    Ok(applied)
}

/// Remove Mode's entries from the settings file (05 §3.6, 03 §7C.2), with a
/// backup; records the uninstall and pushes `health.changed`.
#[tauri::command(rename_all = "snake_case")]
pub async fn hooks_uninstall(
    app: AppHandle,
    state: State<'_, Arc<CoreState>>,
) -> CmdResult<hooks_install::Applied> {
    let st = Arc::clone(&state);
    let applied = tauri::async_runtime::spawn_blocking(move || {
        let target = target(&st)?;
        let backups = st.data_dir.join("backups");
        let applied = hooks_install::uninstall(&target, Some(&backups)).map_err(err)?;
        let settings_path = target.settings_path.to_string_lossy().into_owned();
        st.db
            .write(move |c| {
                hooks_db::record_uninstall(c, &settings_path, &mode_core::db::now_rfc3339())
            })
            .map_err(err)?;
        inspect_and_cache(&st, &target)?;
        Ok::<_, String>(applied)
    })
    .await
    .map_err(err)??;
    health::emit(app, Arc::clone(&state)).await;
    Ok(applied)
}

/// The synthetic round trip through the real relay (05 §3.6 S10).
#[tauri::command(rename_all = "snake_case")]
pub async fn hooks_round_trip(state: State<'_, Arc<CoreState>>) -> CmdResult<RoundTripReport> {
    let relay: PathBuf = CoreState::lock(&state.relay_path)
        .clone()
        .ok_or_else(|| "the relay binary is not staged".to_string())?;
    let server = state
        .server()
        .ok_or_else(|| "the hook IPC server is not listening".to_string())?;
    let report = hooks_install::round_trip(&relay, &server, hooks_install::ROUND_TRIP_TIMEOUT)
        .await
        .map_err(err)?;
    if let Some(s) = CoreState::lock(&state.hooks).as_mut() {
        s.last_round_trip = Some(report.clone());
    }
    let (db, value) = (
        Arc::clone(&state.db),
        serde_json::to_value(&report).map_err(err)?,
    );
    let kept = tauri::async_runtime::spawn_blocking(move || {
        settings::set(&db, settings::LAST_ROUND_TRIP_KEY, &value)
    })
    .await;
    if !matches!(kept, Ok(Ok(()))) {
        tracing::warn!("the round trip could not be kept for the next launch");
    }
    Ok(report)
}

// ------------------------------------------------------------------ overlay

/// What the overlay webview wants to be (02 §16 `overlay.state`): the core
/// shows, hides, sizes and places the window, takes or releases key input,
/// and persists `overlay.pinned`. Runs on the main thread (a sync command).
/// A `launch` request also re-sends the display, the overlay settings and
/// the health state, since the webview just (re)started.
#[tauri::command(rename_all = "snake_case")]
pub fn overlay_request(
    app: AppHandle,
    overlay: State<'_, Arc<Overlay>>,
    core: State<'_, Arc<CoreState>>,
    state: String,
    pinned: Option<bool>,
    keyboard: Option<bool>,
    reason: Option<String>,
) -> CmdResult<()> {
    let st =
        ladder::State::parse(&state).ok_or_else(|| format!("{state:?} is not an overlay state"))?;
    let reason: String = reason.unwrap_or_default().chars().take(64).collect();
    let r = Request {
        state: st,
        pinned: pinned.unwrap_or(false),
        keyboard: keyboard.unwrap_or(false),
        reason,
    };
    let launch = r.is_launch();
    overlay.request(r);
    if launch {
        // Sent after the webview's listeners are up (they register after a
        // dynamic import, possibly after this request).
        let core = Arc::clone(&core);
        tauri::async_runtime::spawn(async move {
            for delay in crate::overlay::LAUNCH_RESEND {
                tokio::time::sleep(delay).await;
                emit_overlay_settings(&app, &core, None).await;
                health::emit(app.clone(), Arc::clone(&core)).await;
                // The Notes tab renders from `notes.changed` only.
                core.publish([Event::Notes]);
            }
        });
    }
    Ok(())
}

/// The drawn frame's rect (with any open menu) in CSS px relative to the
/// stage's top-left: the target with `settled: false` when a morph starts,
/// the rect at rest with `settled: true` (02 §12). The core keeps it for the
/// click-through toggle, `frame_enter` / `frame_leave` and outside clicks;
/// the window itself never changes size. Main thread (sync command).
#[tauri::command(rename_all = "snake_case")]
pub fn overlay_measure(
    overlay: State<'_, Arc<Overlay>>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    settled: bool,
) {
    overlay.measure(x, y, width, height, settled);
}

/// Push `settings.changed`: every overlay setting (`keys = None`) or the
/// given ones (a partial `OverlaySettings`).
pub async fn emit_overlay_settings(
    app: &AppHandle,
    core: &Arc<CoreState>,
    keys: Option<Vec<String>>,
) {
    let db = Arc::clone(&core.db);
    let payload = tauri::async_runtime::spawn_blocking(move || {
        let get = |k: &str| settings::get(&db, k).unwrap_or_else(|_| settings::default_for(k));
        let refs: Option<Vec<&str>> = keys
            .as_ref()
            .map(|v| v.iter().map(String::as_str).collect());
        prefs::settings_payload(get, refs.as_deref())
    })
    .await;
    match payload {
        Ok(p) if !p.is_empty() => {
            tracing::info!(fields = p.len(), "settings.changed");
            if let Err(e) = app.emit_to(crate::OVERLAY_WINDOW, Event::Settings.wire_name(), p) {
                tracing::warn!(error = %e, "emit settings.changed failed");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "settings payload task failed"),
    }
}

// -------------------------------------------------------------- now playing

/// ⏯ on the Now playing strip (02 §7.1): the local player toggles; the
/// poller pushes the new state at once.
#[tauri::command]
pub async fn media_play_pause(media: State<'_, Arc<Media>>) -> CmdResult<()> {
    media.run(MediaCommand::PlayPause).await
}

/// ⏭ on the Now playing strip.
#[tauri::command]
pub async fn media_next(media: State<'_, Arc<Media>>) -> CmdResult<()> {
    media.run(MediaCommand::Next).await
}

/// ⏮ on the Now playing strip.
#[tauri::command]
pub async fn media_previous(media: State<'_, Arc<Media>>) -> CmdResult<()> {
    media.run(MediaCommand::Previous).await
}

/// The strip's artwork or title: the player comes to the front.
#[tauri::command]
pub async fn media_open(media: State<'_, Arc<Media>>) -> CmdResult<()> {
    media.run(MediaCommand::Open).await
}

/// The Music tab (02 §7.10, ADR-0066): the distinct tracks and albums Mode
/// saw playing, newest first.
#[tauri::command]
pub async fn media_history(
    state: State<'_, Arc<CoreState>>,
) -> CmdResult<mode_core::media_history::History> {
    let core = Arc::clone(&state);
    tauri::async_runtime::spawn_blocking(move || crate::media::history(&core))
        .await
        .map_err(|e| e.to_string())?
}

/// A Music-tab row: the play `id` starts its track in the player it came
/// from, or brings that player to the front where it cannot (02 §7.10).
#[tauri::command]
pub async fn media_play(
    state: State<'_, Arc<CoreState>>,
    media: State<'_, Arc<Media>>,
    id: String,
) -> CmdResult<crate::media::PlayOutcome> {
    let core = Arc::clone(&state);
    let row = tauri::async_runtime::spawn_blocking(move || {
        core.db
            .read(|c| mode_core::media_history::get_in(c, &id))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??
    .ok_or_else(|| "That track is no longer in the history".to_string())?;
    media
        .play(crate::media::PlayItem {
            app: row.app,
            track_id: row.track_id,
        })
        .await
}

/// Settings → Overlay "Clear history" (03 §8.3): forget every play, then
/// push the empty history to both windows.
#[tauri::command]
pub async fn media_history_clear(
    app: AppHandle,
    state: State<'_, Arc<CoreState>>,
) -> CmdResult<()> {
    let core = Arc::clone(&state);
    let n = tauri::async_runtime::spawn_blocking(move || {
        core.db
            .write(|c| mode_core::media_history::clear_in(c))
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    tracing::info!(plays = n, "media history cleared");
    crate::media::emit_history(&app, &state);
    Ok(())
}

// ----------------------------------------------------------------- settings

/// The outcome of [`write_setting`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stored {
    Done,
    /// A global hotkey the OS refused (02 §8.2); the previous chord is back.
    Refused(String),
}

/// Write (`Some`) or reset (`None`) one setting: validated against the
/// registry (02 §17 ranges; internal keys refused), hotkeys parsed and
/// re-registered (a chord another app holds is rolled back), then the
/// windows are told ([`after_settings_change`]).
pub async fn write_setting(
    app: &AppHandle,
    state: &Arc<CoreState>,
    key: &str,
    value: Option<Value>,
) -> CmdResult<Stored> {
    match &value {
        Some(v) => settings::validate(key, v)?,
        None => {
            if settings::key_def(key).is_none() {
                return Err(format!("{key} is not a registered setting"));
            }
            if settings::INTERNAL_KEYS.contains(&key) {
                return Err(format!("{key} is managed by Mode"));
            }
        }
    }
    let hotkey = HotkeyKind::from_key(key);
    if hotkey.is_some()
        && let Some(chord) = value.as_ref().and_then(Value::as_str)
    {
        hotkeys::parse_chord(chord).map_err(err)?;
    }
    let core = Arc::clone(state);
    let (k, v) = (key.to_string(), value.clone());
    let previous = tauri::async_runtime::spawn_blocking(move || {
        let previous = settings::stored(&core.db, &k)?;
        match &v {
            Some(v) => settings::set(&core.db, &k, v)?,
            None => {
                settings::reset(&core.db, &k)?;
            }
        }
        Ok::<_, mode_core::db::DbError>(previous)
    })
    .await
    .map_err(err)?
    .map_err(err)?;

    if let Some(kind) = hotkey {
        let failed = hotkeys::register_from_settings(app.clone(), Arc::clone(state)).await;
        if let Some((_, why)) = failed.iter().find(|(k, _)| *k == kind) {
            let why = why.clone();
            let core = Arc::clone(state);
            let k = key.to_string();
            let _ = tauri::async_runtime::spawn_blocking(move || match previous {
                Some(p) => settings::set(&core.db, &k, &p).map(|_| ()),
                None => settings::reset(&core.db, &k).map(|_| ()),
            })
            .await;
            hotkeys::register_from_settings(app.clone(), Arc::clone(state)).await;
            return Ok(Stored::Refused(why));
        }
    }
    if key == "dictation.model" {
        state.dictation.unload();
    }
    if key == "onboarding.completed_at" && value.as_ref().is_some_and(|v| !v.is_null()) {
        // The welcome pill (03 §2.8), as the first flash (02 §5.4).
        state.flash(
            "Mode is ready · hover to open",
            mode_core::push::FlashKind::Info,
            None,
        );
    }
    if key == "dictation.enabled" {
        // The hold key is registered only while dictation is on (XC-37).
        hotkeys::register_from_settings(app.clone(), Arc::clone(state)).await;
        if value.as_ref().and_then(Value::as_bool) != Some(true) {
            state.dictation.cancel();
        }
    }
    after_settings_change(app, state, vec![key.to_string()]).await;
    Ok(Stored::Done)
}

/// Tell the windows about changed settings: the overlay's partial
/// `OverlaySettings`, the window placement, `health.changed` for the keys
/// it reads, and the app window's `settings.changed` with the registry keys.
pub async fn after_settings_change(app: &AppHandle, state: &Arc<CoreState>, keys: Vec<String>) {
    let overlay_keys: Vec<String> = keys
        .iter()
        .filter(|k| prefs::is_overlay_setting(k))
        .cloned()
        .collect();
    if !overlay_keys.is_empty() {
        emit_overlay_settings(app, state, Some(overlay_keys)).await;
    }
    if keys
        .iter()
        .any(|k| prefs::WINDOW_KEYS.contains(&k.as_str()))
    {
        let db = Arc::clone(&state.db);
        if let Ok(p) = tauri::async_runtime::spawn_blocking(move || prefs::Prefs::load(&db)).await
            && let Some(overlay) = app.try_state::<Arc<Overlay>>()
        {
            overlay.on_main(move |o| o.set_prefs(p));
        }
    }
    if keys.iter().any(|k| {
        matches!(
            k.as_str(),
            "notes.vault_path" | "onboarding.completed_at" | "sessions.claude_path"
        )
    }) {
        health::emit(app.clone(), Arc::clone(state)).await;
    }
    // Both webviews repaint from `theme.changed` (the pack, the appearance,
    // the font scale, the terminal pack).
    if keys.iter().any(|k| k.starts_with("theme."))
        && let Some(t) = state.themes.get()
    {
        t.changed();
    }
    // Now playing re-reads its switch and quiet hours at once.
    if keys
        .iter()
        .any(|k| k == crate::media::SETTING || k == "notifications.quiet_hours")
        && let Some(m) = app.try_state::<Arc<Media>>()
    {
        m.wake();
    }
    if keys.iter().any(|k| k.starts_with("automations.")) {
        state.wake_scheduler(mode_core::scheduler::Wake::Changed);
    }
    if keys.iter().any(|k| k.starts_with("ai.")) {
        // The scheduler's `problem` (no AI backend) may have changed.
        state.publish([Event::Automations]);
    }
    emit_app_settings(app, state, Some(keys)).await;
}

/// Push `settings.changed` to the app window: `{key: effective value}` for
/// the given registry keys (every key when `None`).
pub async fn emit_app_settings(app: &AppHandle, core: &Arc<CoreState>, keys: Option<Vec<String>>) {
    let db = Arc::clone(&core.db);
    let payload = tauri::async_runtime::spawn_blocking(move || match keys {
        None => settings::all_effective(&db).unwrap_or_default(),
        Some(keys) => keys
            .into_iter()
            .filter(|k| settings::key_def(k).is_some())
            .map(|k| {
                let v = settings::get(&db, &k).unwrap_or_else(|_| settings::default_for(&k));
                (k, v)
            })
            .collect(),
    })
    .await;
    match payload {
        Ok(p) if !p.is_empty() => {
            tracing::info!(keys = p.len(), "settings.changed (app window)");
            if let Err(e) = app.emit_to(crate::APP_WINDOW, Event::Settings.wire_name(), p) {
                tracing::warn!(error = %e, "emit settings.changed failed");
            }
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "settings payload task failed"),
    }
}

/// Write one setting (the Settings and Shortcuts views): validated against
/// the registry (02 §17 ranges), hotkeys parsed and re-registered (a chord
/// another app holds is refused and the old one kept, 02 §8.2), then the
/// overlay and the app window are told (`settings.changed`, window
/// placement, display). Returns the stored value.
#[tauri::command(rename_all = "snake_case")]
pub async fn settings_set(
    app: AppHandle,
    state: State<'_, Arc<CoreState>>,
    key: String,
    value: Value,
) -> CmdResult<Value> {
    let core = Arc::clone(&state);
    if let Stored::Refused(why) = write_setting(&app, &core, &key, Some(value.clone())).await? {
        return Err(format!("{value} could not be registered: {why}"));
    }
    let db = Arc::clone(&state.db);
    let k = key.clone();
    tauri::async_runtime::spawn_blocking(move || settings::get(&db, &k))
        .await
        .map_err(err)?
        .map_err(err)
}

/// Open the app window on a view (02 §16): show and focus it, then send it
/// `app.open { view, id, filter, route }` (the shell routes on `route`). A
/// caller may pass `route` itself (a deep link, 03 §2.5); otherwise it is
/// derived from the other fields.
#[tauri::command(rename_all = "snake_case")]
pub fn app_open(
    app: AppHandle,
    view: String,
    id: Option<String>,
    filter: Option<String>,
    route: Option<String>,
) -> CmdResult<()> {
    app_window::check(&view, id.as_deref(), filter.as_deref())?;
    let mut req = AppOpen::new(&view, id, filter, None);
    if let Some(r) = route {
        app_window::check_route(&r)?;
        req.route = r;
    }
    app_window::open(&app, req);
    Ok(())
}

/// `⌘/Ctrl+W` and the close button (03 §2.1): hide the app window; Mode keeps
/// running.
#[tauri::command]
pub fn app_hide(app: AppHandle) -> CmdResult<()> {
    app_window::hide(&app);
    Ok(())
}
