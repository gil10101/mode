//! The Mode desktop app (08 §3.3): two windows from `tauri.conf.json`, the
//! `#[tauri::command]`s, the `*.changed` pushes and the native overlay glue
//! behind the `OverlayPlatform` trait.

pub mod alerts;
pub mod app_window;
pub mod bundle;
pub mod claude;
pub mod cmd_error;
pub mod commands;
pub mod commands_agents;
pub mod commands_ai;
pub mod commands_alerts;
pub mod commands_automations;
pub mod commands_chat;
pub mod commands_connectors;
pub mod commands_diag;
pub mod commands_dictation;
pub mod commands_inbox;
pub mod commands_notes;
pub mod commands_packs;
pub mod commands_palette;
pub mod commands_pty;
pub mod commands_sessions;
pub mod commands_settings;
pub mod commands_themes;
pub mod dialogs;
pub mod events;
pub mod health;
pub mod hotkeys;
pub mod jump;
pub mod media;
pub mod os_signals;
pub mod overlay;
pub mod platform;
pub mod quit;
pub mod services;
pub mod speech;
pub mod state;
#[cfg(test)]
mod wiring_tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mode_core::connectors::Connectors;
use mode_core::connectors::http::ReqwestClient;
use mode_core::db::DbError;
use mode_core::hooks_install;
use mode_core::ipc::Server;
use mode_core::notes::{Notes, NotesOptions};
use mode_core::pty::{self, Pty};
use mode_core::push::{self, Event, PayloadFn, Publisher};
use mode_core::sessions::Engine;
use mode_wire::Endpoint;
use tauri::{AppHandle, Manager, RunEvent};

use overlay::Overlay;
use overlay::prefs::Prefs;
use platform::OverlayPlatform;
use state::CoreState;

/// Display / theme re-check and (Windows) presentation-state cadence (08 §6.7).
const DISPLAY_WATCH: Duration = Duration::from_secs(2);
/// The snooze time sweep (S2, 07 §6: every minute).
const SNOOZE_SWEEP: Duration = Duration::from_secs(60);

/// Window labels declared in `tauri.conf.json`.
pub const OVERLAY_WINDOW: &str = "overlay";
pub const APP_WINDOW: &str = "app";

/// Where a build may put the relay next to the app binary.
pub const RELAY_ENV: &str = "MODE_HOOK_BIN";

/// A vault root that overrides `settings['notes.vault_path']` for this run
/// (the headless smoke and dev runs keep their notes out of `~/Mode/Notes`).
pub const VAULT_ENV: &str = "MODE_VAULT";

pub fn run() {
    // `<app_data_dir>/logs/mode.log`, 5 files × 5 MB (08 §12).
    let data_dir = mode_core::data_dir(mode_core::is_dev_build());
    mode_core::diag::init_tracing_to(Some(&mode_core::diag::log_dir(&data_dir)));
    let app = tauri::Builder::default()
        // Must be the first plugin so a second launch is forwarded before anything else runs.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window(APP_WINDOW) {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // The core shows the pickers; the webviews get no `dialog:*` grant.
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::core_status,
            commands::sessions_list,
            commands::session_detail,
            commands::session_decide,
            commands::session_acknowledge,
            commands::session_dismiss,
            commands::session_jump,
            commands::hooks_status,
            commands::hooks_install_preview,
            commands::hooks_install_apply,
            commands::hooks_round_trip,
            commands::hooks_uninstall,
            commands::overlay_request,
            commands::overlay_measure,
            commands::app_open,
            commands::app_hide,
            commands::settings_set,
            commands::media_play_pause,
            commands::media_next,
            commands::media_previous,
            commands::media_open,
            commands::media_history,
            commands::media_play,
            commands::media_history_clear,
            commands_sessions::sessions_history,
            commands_sessions::session_undismiss,
            commands_sessions::session_snooze,
            commands_sessions::session_files_refresh,
            commands_sessions::session_diff,
            commands_sessions::session_transcript,
            commands_sessions::sessions_recent_folders,
            commands_sessions::sessions_pick_folder,
            commands_sessions::health_get,
            commands_pty::pty_open,
            commands_pty::pty_detach,
            commands_pty::pty_write,
            commands_pty::pty_resize,
            commands_pty::pty_close,
            commands_pty::session_launch,
            commands_pty::session_resume,
            commands_pty::session_prompt,
            commands_pty::turn_stop,
            commands_pty::pty_quit_all,
            commands_pty::pty_list,
            commands_pty::claude_locate,
            commands_pty::app_quit,
            commands_settings::settings_get_all,
            commands_settings::settings_reset,
            commands_settings::settings_export,
            commands_settings::settings_import,
            commands_settings::platform_info,
            commands_settings::hotkey_set,
            commands_settings::notes_folders_set,
            commands_settings::vault_pick,
            commands_settings::vault_inspect,
            commands_settings::profile_picture_pick,
            commands_settings::data_delete_all,
            commands_ai::secret_status,
            commands_ai::secret_set,
            commands_ai::ai_backends_status,
            commands_ai::ai_test_call,
            commands_ai::ai_backends_list,
            commands_ai::ai_backend_set,
            commands_ai::ai_key_set,
            commands_ai::ai_health,
            commands_ai::ai_providers_list,
            commands_ai::ai_provider_save,
            commands_ai::ai_provider_delete,
            commands_ai::ai_provider_key_set,
            commands_ai::ai_provider_test,
            commands_ai::ai_provider_models,
            commands_ai::ai_provider_kinds,
            commands_ai::ai_routes_list,
            commands_ai::ai_route_set,
            commands_connectors::connectors_status,
            commands_connectors::accounts_list,
            commands_connectors::account_add_start,
            commands_connectors::account_add_finish,
            commands_connectors::account_add_cancel,
            commands_connectors::account_pause,
            commands_connectors::account_resume,
            commands_connectors::account_retry,
            commands_connectors::account_reauthorise,
            commands_connectors::account_remove,
            commands_connectors::claude_code_card,
            commands_connectors::oauth_client_set,
            commands_connectors::slack_manifest,
            commands_connectors::sources_list,
            commands_connectors::sources_refresh,
            commands_connectors::source_set,
            commands_connectors::slack_channels_list,
            commands_connectors::slack_channels_set,
            commands_connectors::url_open,
            commands_inbox::inbox_list,
            commands_inbox::thread_get,
            commands_inbox::message_mark_read,
            commands_inbox::thread_archive,
            commands_inbox::thread_snooze,
            commands_inbox::thread_unsnooze,
            commands_inbox::open_in_source,
            commands_inbox::message_set_tier,
            commands_inbox::message_body,
            commands_inbox::inbox_refresh,
            commands_inbox::keyword_rules_list,
            commands_inbox::keyword_rules_set,
            commands_inbox::urgency_rescore,
            commands_inbox::urgency_diagnostics,
            commands_inbox::vips_list,
            commands_inbox::vip_add,
            commands_inbox::vip_remove,
            commands_inbox::agenda_list,
            commands_inbox::event_join,
            commands_inbox::event_prepare_notes,
            commands_inbox::event_links,
            commands_inbox::item_link_add,
            commands_inbox::item_link_remove,
            commands_alerts::alerts_recent,
            commands_alerts::alert_dismiss,
            commands_alerts::alert_rule_test,
            commands_alerts::alert_rules_list,
            commands_alerts::alert_rule_set,
            commands_alerts::alert_rule_add,
            commands_alerts::alert_rule_delete,
            commands_alerts::quiet_set,
            commands_diag::diagnostics_get,
            commands_diag::logs_tail,
            commands_diag::diagnostics_export,
            commands_diag::db_backup,
            commands_diag::retention_sweep,
            commands_diag::stats_days,
            commands_diag::stats_summary,
            commands_notes::notes_list,
            commands_notes::note_get,
            commands_notes::note_editor,
            commands_notes::note_create,
            commands_notes::note_save,
            commands_notes::note_set_field,
            commands_notes::note_cycle_status,
            commands_notes::note_archive,
            commands_notes::note_rename,
            commands_notes::note_trash,
            commands_notes::note_untrash,
            commands_notes::folders_list,
            commands_notes::groupings_list,
            commands_notes::grouping_save,
            commands_notes::grouping_delete,
            commands_notes::note_drop,
            commands_notes::add_task_plan,
            commands_notes::tags_list,
            commands_notes::tag_rename,
            commands_notes::tag_delete,
            commands_notes::query_resolve,
            commands_notes::query_run,
            commands_notes::note_history,
            commands_notes::note_revision_get,
            commands_notes::note_restore,
            commands_notes::note_link_add,
            commands_notes::note_capture,
            commands_notes::note_capture_links,
            commands_notes::note_from_session,
            commands_notes::note_from_message,
            commands_notes::note_prepare,
            commands_notes::notes_reindex,
            commands_notes::notes_import,
            commands_notes::notes_state_export,
            commands_notes::notes_state_import,
            commands_notes::statuses_save,
            commands_notes::status_move,
            commands_chat::chat_threads_list,
            commands_chat::chat_thread_get,
            commands_chat::chat_send,
            commands_chat::chat_stop,
            commands_chat::chat_action_confirm,
            commands_chat::chat_thread_delete,
            commands_chat::chat_sources,
            commands_chat::chat_status,
            commands_chat::chat_thread_rename,
            commands_chat::chat_pin,
            commands_chat::chat_thread_set,
            commands_chat::chat_pick_file,
            commands_automations::automations_list,
            commands_automations::automation_get,
            commands_automations::automation_save,
            commands_automations::automation_delete,
            commands_automations::automation_set_enabled,
            commands_automations::automation_run_now,
            commands_automations::automation_runs,
            commands_automations::automation_run_get,
            commands_automations::automation_duplicate,
            commands_automations::automation_examples,
            commands_automations::schedule_preview,
            commands_agents::agents_list,
            commands_agents::agent_get,
            commands_agents::agent_save,
            commands_agents::agent_delete,
            commands_agents::agent_memory_save,
            commands_agents::agent_memory_delete,
            commands_agents::skills_list,
            commands_agents::mcp_servers_list,
            commands_themes::themes_list,
            commands_themes::theme_get,
            commands_themes::theme_set_active,
            commands_themes::theme_import,
            commands_themes::theme_export,
            commands_themes::theme_delete,
            commands_themes::theme_resolve,
            commands_themes::theme_duplicate,
            commands_themes::theme_update,
            commands_packs::pack_export,
            commands_packs::pack_import,
            commands_packs::catalogue_get,
            commands_palette::palette_query,
            commands_dictation::dictation_status,
            commands_dictation::dictation_press,
            commands_dictation::dictation_release,
            commands_dictation::dictation_cancel,
            commands_dictation::dictation_inserted,
            commands_dictation::dictation_models,
            commands_dictation::dictation_model_ensure,
            commands_dictation::dictation_model_remove,
            commands_dictation::dictation_mic_check,
            commands_dictation::app_context_set,
            commands_dictation::dictation_say,
        ])
        .setup(|app| setup(app.handle().clone()).map_err(Into::into))
        .build(tauri::generate_context!())
        .expect("error while building Mode");
    app.run(|app, event| match event {
        // Quit with live embedded sessions asks first (03 §2.1).
        RunEvent::ExitRequested { code, api, .. } => quit::on_exit_requested(app, &api, code),
        RunEvent::Exit => quit::on_exit(app),
        _ => {}
    });
}

/// Where this build's relay may sit, most preferred first:
/// 1. `MODE_HOOK_BIN` (smoke runs, tests);
/// 2. the Tauri sidecar next to the app binary: `bundle.externalBin`
///    ships `binaries/mode-hook-<triple>` as `mode-hook[.exe]` beside the
///    executable (`Mode.app/Contents/MacOS/`, `/usr/bin/`, the AppImage's
///    `usr/bin/`, the NSIS install dir), and tauri-build copies it to
///    `target/<profile>/` for `tauri dev`, where cargo's own relay also sits;
/// 3. the same file with its triple suffix still on (a binary run from
///    beside an unbundled `binaries/` copy);
/// 4. the resource dir (`BaseDirectory::Resource`): `bin/mode-hook`, then
///    `mode-hook`.
///
/// Whichever is found is copied to `<app_data_dir>/bin/` (`stage_relay`):
/// that copy, not the bundle path, is what `~/.claude/settings.json` names,
/// so the hook command survives app moves and updates (05 §3.6, 08 §7).
pub(crate) fn relay_candidates(
    env_override: Option<PathBuf>,
    exe_dir: Option<&Path>,
    resource_dir: Option<&Path>,
    triple: Option<&str>,
) -> Vec<PathBuf> {
    let name = hooks_install::RELAY_FILE_NAME;
    let mut candidates: Vec<PathBuf> = env_override.into_iter().collect();
    if let Some(dir) = exe_dir {
        candidates.push(dir.join(name));
        if let Some(triple) = triple {
            let (stem, ext) = match name.rsplit_once('.') {
                Some((s, e)) => (s.to_string(), format!(".{e}")),
                None => (name.to_string(), String::new()),
            };
            candidates.push(dir.join(format!("{stem}-{triple}{ext}")));
        }
    }
    if let Some(res) = resource_dir {
        candidates.push(res.join("bin").join(name));
        candidates.push(res.join(name));
    }
    candidates
}

/// The first relay of [`relay_candidates`] that exists.
fn bundled_relay(app: &AppHandle) -> Option<PathBuf> {
    let env_override = std::env::var_os(RELAY_ENV)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from));
    // `BaseDirectory::Resource`.
    let resource_dir = app.path().resource_dir().ok();
    let triple = tauri::utils::platform::target_triple().ok();
    relay_candidates(
        env_override,
        exe_dir.as_deref(),
        resource_dir.as_deref(),
        triple.as_deref(),
    )
    .into_iter()
    .find(|p| p.is_file())
}

/// Stage `bundled` into `<data_dir>/bin/` (copied when the bytes differ,
/// so an app update replaces it at the next launch and `mode-hook
/// --version` names the new version). When the copy cannot be replaced
/// (Windows keeps a running `mode-hook.exe` locked while a hook is in
/// flight), the copy already staged keeps serving and the relay check
/// reports a version that differs.
pub(crate) fn stage_bundled_relay(bundled: Option<&Path>, data_dir: &Path) -> Option<PathBuf> {
    let Some(bundled) = bundled else {
        tracing::warn!("no relay binary found next to the app; hook install is unavailable");
        return None;
    };
    match hooks_install::stage_relay(bundled, data_dir) {
        Ok(staged) => {
            tracing::info!(bundled = %bundled.display(), relay = %staged.display(), "relay staged");
            Some(staged)
        }
        Err(e) => {
            let staged = hooks_install::default_relay_path(data_dir);
            if staged.is_file() {
                tracing::warn!(bundled = %bundled.display(), error = %e, relay = %staged.display(), "relay could not be replaced; keeping the staged copy");
                Some(staged)
            } else {
                tracing::warn!(bundled = %bundled.display(), error = %e, "relay could not be staged");
                None
            }
        }
    }
}

/// Open the database; one written by a newer Mode (XC-33) is left alone
/// and the app runs on a scratch database so the window can say so
/// (`health.changed.db_newer`). Returns the database and whether that
/// happened.
fn open_db(db_path: &std::path::Path) -> anyhow::Result<(mode_core::db::Db, bool)> {
    match mode_core::db::open(db_path) {
        Ok(db) => Ok((db, false)),
        Err(e @ DbError::TooNew { .. }) => {
            tracing::error!(error = %e, "the database is newer than this build; running on a scratch database");
            let scratch = std::env::temp_dir()
                .join(format!("mode-db-newer-{}", std::process::id()))
                .join("mode.db");
            Ok((mode_core::db::open(&scratch)?, true))
        }
        Err(e) => Err(e.into()),
    }
}

/// The publisher's payload function: `health.changed` from the app (with
/// the connectors' fields), `automations.changed` (the rows) from the
/// scheduler and `theme.changed` (the token map) from the themes service,
/// `sessions.changed` / `pill.changed` from the engine (a sessions push
/// also runs the session alert watch), then `notes.changed` from the notes
/// service, then `inbox.changed`, `agenda.changed`, `quiet.changed` and
/// `accounts.changed` from the connectors service. A sessions, notes,
/// inbox, agenda or automations push marks the palette index stale.
pub fn payloads(
    core: Arc<CoreState>,
    notes: Option<Arc<Notes>>,
    connectors: Option<Arc<Connectors>>,
) -> PayloadFn {
    Arc::new(move |e| {
        if matches!(
            e,
            Event::Sessions | Event::Notes | Event::Inbox | Event::Agenda | Event::Automations
        ) {
            core.invalidate_palette();
        }
        match e {
            Event::Health => return health::payload(&core),
            Event::Automations => return core.scheduler.get().and_then(|s| s.payload()),
            Event::Theme => return core.themes.get().and_then(|t| t.payload()),
            Event::Agents => return commands_agents::payload(&core),
            _ => {}
        }
        match core.engine.payload(e) {
            Ok(Some(v)) => {
                if e == Event::Sessions {
                    alerts::after_sessions_push(&core);
                }
                return Some(v);
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(event = e.name(), error = %err, "payload failed");
                return None;
            }
        }
        match notes.as_ref().map(|n| n.payload(e)) {
            Some(Ok(Some(v))) => return Some(v),
            Some(Ok(None)) | None => {}
            Some(Err(err)) => {
                tracing::warn!(event = e.name(), error = %err, "notes payload failed");
                return None;
            }
        }
        match connectors.as_ref().map(|c| c.payload(e)) {
            Some(Ok(v)) => v,
            Some(Err(err)) => {
                tracing::warn!(event = e.name(), error = %err, "connectors payload failed");
                None
            }
            None => None,
        }
    })
}

/// The connectors service (06, 07): the keyring, the HTTP client, the alert
/// sink (OS notifications, the overlay cue) and its action handler. Not on
/// a scratch database. The publisher, the run loop and the OS signals are
/// attached once the publisher exists.
fn start_connectors(app: &AppHandle, state: &Arc<CoreState>) -> Option<Arc<Connectors>> {
    let http = match ReqwestClient::new() {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "connectors service unavailable: the HTTP client failed");
            return None;
        }
    };
    let c = Connectors::new(Arc::clone(&state.db), state.keyring(), Arc::new(http));
    let notifier = alerts::Notifier::new(app.clone());
    notifier.listen_actions(Arc::clone(state));
    {
        let n = Arc::clone(&notifier);
        c.set_alert_sink(move |cmd| n.handle(cmd));
    }
    app.manage(notifier);
    let _ = state.connectors.set(Arc::clone(&c));
    state.session_watch.prime(&state.db);
    tracing::info!(
        keyring = state.keyring().kind(),
        "connectors service created"
    );
    Some(c)
}

fn setup(app: AppHandle) -> anyhow::Result<()> {
    let dev = mode_core::is_dev_build();
    let data_dir = mode_core::data_dir(dev);
    let db_path = data_dir.join("mode.db");
    let (db, db_newer) = open_db(&db_path)?;
    let db = Arc::new(db);
    let endpoint = Endpoint::from_env();
    tracing::info!(dev, data_dir = %data_dir.display(), endpoint = %endpoint.describe(), "core starting");

    let engine = Arc::new(Engine::new(Arc::clone(&db)));
    if !db_newer {
        // 05 §4.2 "Mode starts": settle what the previous run left open before
        // anything is published.
        match engine.reconcile_startup() {
            Ok(r) => tracing::info!(
                closed = r.closed,
                reset = r.reset,
                tracked = r.tracked,
                boot = engine.boot(),
                "startup reconciliation"
            ),
            Err(e) => tracing::warn!(error = %e, "startup reconciliation failed"),
        }
    }
    let state = Arc::new(CoreState::new(
        db,
        engine,
        data_dir.clone(),
        db_path,
        endpoint.clone(),
    ));
    state
        .db_newer
        .store(db_newer, std::sync::atomic::Ordering::Relaxed);
    // The secret store (08 §8), chosen once and logged.
    let _ = state
        .keyring
        .set(Arc::new(state::open_keyring(&data_dir, dev)));
    app.manage(Arc::clone(&state));
    // `overlay.flash` (02 §5.4): to the overlay only.
    {
        use tauri::Emitter;
        let a = app.clone();
        let _ = state.flash_sink.set(Arc::new(move |f| {
            if let Err(e) = a.emit_to(OVERLAY_WINDOW, Event::OverlayFlash.wire_name(), f) {
                tracing::warn!(error = %e, "emit overlay.flash failed");
            }
        }));
    }
    // Dictation (XC-37): every state to both windows.
    speech::install(&app, &state);

    // A GUI app's PATH is short on macOS: capture the login shell's PATH
    // now, off the main thread, so the first launch does not wait for it.
    std::thread::Builder::new()
        .name("mode-login-path".into())
        .spawn(|| {
            let found = pty::launch::login_path().is_some();
            tracing::debug!(found, "login PATH warmed");
        })?;

    // The notes service (04, 08 §4.3); never on a scratch database, where
    // its launch scan would index (and touch up) the vault for nothing.
    let notes = if db_newer {
        None
    } else {
        let opts = NotesOptions {
            root: std::env::var_os(VAULT_ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            ..Default::default()
        };
        match Notes::new(Arc::clone(&state.db), opts) {
            Ok(n) => {
                tracing::info!(root = %n.root().display(), "notes service created");
                let _ = state.notes.set(Arc::clone(&n));
                Some(n)
            }
            Err(e) => {
                tracing::error!(error = %e, "notes service unavailable");
                None
            }
        }
    };

    let connectors = if db_newer {
        None
    } else {
        start_connectors(&app, &state)
    };

    // The relay: staged into `<app_data_dir>/bin/` when the bundled copy differs (08 §7).
    if let Some(staged) = stage_bundled_relay(bundled_relay(&app).as_deref(), &data_dir) {
        *CoreState::lock(&state.relay_path) = Some(staged);
    }

    // Native overlay glue, then the window controller (hidden until the
    // webview asks for a drawable state).
    let platform: Arc<dyn OverlayPlatform> = Arc::from(platform::current());
    if let Some(window) = app.get_webview_window(OVERLAY_WINDOW) {
        platform.apply_overlay_style(&window)?;
        tracing::info!(platform = platform.name(), "overlay style applied");
    }
    let overlay = Overlay::new(
        app.clone(),
        Arc::clone(&platform),
        Arc::clone(&state),
        Prefs::load(&state.db),
    );
    app.manage(Arc::clone(&overlay));
    app.manage(hotkeys::Hotkeys::default());
    if let Some(window) = app.get_webview_window(OVERLAY_WINDOW) {
        let ov = Arc::clone(&overlay);
        let core = Arc::clone(&state);
        window.on_window_event(move |e| {
            if let tauri::WindowEvent::Focused(f) = e {
                speech::on_focus(&core, OVERLAY_WINDOW, *f);
            }
            ov.on_window_event(e)
        });
    }
    // Chat, automations, theme packs and the palette (package 11).
    let scheduler_wake = services::create(
        &app,
        &state,
        notes.as_ref(),
        connectors.as_ref(),
        platform.reduced_motion(),
    );
    if let Some(window) = app.get_webview_window(APP_WINDOW) {
        let (core, pf) = (Arc::clone(&state), Arc::clone(&platform));
        let handle = app.clone();
        window.on_window_event(move |e| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                // 03 §2.1: closing hides the window; the overlay, the status
                // item and the sessions keep running, and the window comes
                // back on the next `app_open`, pill click or hotkey.
                api.prevent_close();
                app_window::hide(&handle);
                return;
            }
            if let tauri::WindowEvent::Focused(f) = e {
                speech::on_focus(&core, APP_WINDOW, *f);
            }
            os_signals::on_app_window_event(&core, e);
            if let tauri::WindowEvent::ThemeChanged(t) = e {
                services::on_theme_changed(&core, *t == tauri::Theme::Dark, pf.reduced_motion());
            }
        });
    }
    overlay.start();
    // Now playing (OV-37): the `mode-media` poller, after the overlay it reads.
    app.manage(media::start(
        app.clone(),
        Arc::clone(&state),
        Arc::clone(&overlay),
    ));
    if state.smoke {
        tracing::info!("MODE_SMOKE=1: decision frames are answered no_opinion at once");
    }
    tauri::async_runtime::spawn(hotkeys::register_from_settings(
        app.clone(),
        Arc::clone(&state),
    ));
    tauri::async_runtime::spawn(display_watch(
        app.clone(),
        Arc::clone(&state),
        Arc::clone(&overlay),
        Arc::clone(&platform),
    ));
    if !db_newer {
        tauri::async_runtime::spawn(os_signals::signals_task(
            Arc::clone(&state),
            Arc::clone(&platform),
        ));
    }
    tauri::async_runtime::spawn(health::relay_check_task(app.clone(), Arc::clone(&state)));
    {
        let (app, st) = (app.clone(), Arc::clone(&state));
        tauri::async_runtime::spawn(async move {
            commands::emit_overlay_settings(&app, &st, None).await;
        });
    }

    // Pushes, the notes loop, hook IPC and the frame pump.
    let handle = app.clone();
    let st = Arc::clone(&state);
    tauri::async_runtime::spawn(async move {
        let publisher = Publisher::spawn(
            payloads(Arc::clone(&st), notes.clone(), connectors.clone()),
            events::sink(handle.clone()),
            push::DEBOUNCE,
        );
        let _ = st.publisher.set(publisher.clone());
        // The PTY host (package 8): its `pty.*` / `turn.*` streams go straight
        // to the app window; it requests the debounced pushes through the publisher.
        let pty_publish: pty::PublishFn = {
            let core = Arc::clone(&st);
            Arc::new(move |events: &[Event]| core.publish(events.iter().copied()))
        };
        let host = Pty::new(
            Arc::clone(&st.engine),
            events::pty_sink(handle.clone()),
            pty_publish,
            pty::Config {
                agents_dir: Some(st.data_dir.join("agents")),
                keyring: Some(st.keyring()),
                ..pty::Config::default()
            },
        );
        let _ = st.pty.set(host.clone());
        tracing::info!("pty host ready");
        services::start(&st, &publisher, Some(host), scheduler_wake);
        if let Some(c) = connectors {
            let p = publisher.clone();
            c.set_publisher(move |e| p.publish(e));
            tauri::async_runtime::spawn(Arc::clone(&c).run());
            tauri::async_runtime::spawn(alerts::note_due_task(Arc::clone(&st)));
            tracing::info!("connectors service started");
            // The Integrations, Inbox and Calendar views and the overlay tabs
            // render from these pushes (empty without accounts).
            st.publish([Event::Accounts, Event::Agenda, Event::Inbox, Event::Quiet]);
        }
        if let Some(n) = notes {
            n.set_publisher(move |e| publisher.publish(e));
            tauri::async_runtime::spawn(Arc::clone(&n).run(None));
            let (app, core) = (handle.clone(), Arc::clone(&st));
            tauri::async_runtime::spawn(async move {
                n.launched().await;
                *CoreState::lock(&core.last_full_scan_at) = Some(mode_core::db::now_rfc3339());
                core.publish([Event::Notes]);
                health::emit(app, core).await;
            });
        }
        if st.is_db_newer() {
            tracing::warn!("hook IPC not started: the database is newer than this build");
            return;
        }
        match Server::bind(endpoint).await {
            Ok((server, rx)) => {
                let server = Arc::new(server);
                st.engine.set_server(Arc::clone(&server));
                let _ = st.server.set(Arc::clone(&server));
                st.ipc_listening
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                events::emit_core_changed(
                    &handle,
                    events::CoreChanged::reason("ipc_listening", &st),
                );
                tauri::async_runtime::spawn(events::resolutions_task(
                    Arc::clone(&st),
                    Arc::clone(&server),
                ));
                events::pump(handle, server, rx, st).await;
            }
            Err(e) => {
                tracing::error!(error = %e, "hook IPC could not bind");
                events::emit_core_changed(&handle, events::CoreChanged::reason("ipc_error", &st));
            }
        }
    });
    if !db_newer {
        tauri::async_runtime::spawn(events::tailer_task(Arc::clone(&state)));
        tauri::async_runtime::spawn(events::liveness_task(Arc::clone(&state)));
        tauri::async_runtime::spawn(events::scan_task(Arc::clone(&state)));
        tauri::async_runtime::spawn(snooze_sweep(Arc::clone(&state)));
    }

    Ok(())
}

/// Fire snoozes whose time has come (S2) and push what they hid.
async fn snooze_sweep(state: Arc<CoreState>) {
    loop {
        tokio::time::sleep(SNOOZE_SWEEP).await;
        let engine = Arc::clone(&state.engine);
        match tauri::async_runtime::spawn_blocking(move || engine.fire_due_snoozes()).await {
            Ok(Ok(pushes)) => state.publish(pushes),
            Ok(Err(e)) => tracing::warn!(error = %e, "snooze sweep failed"),
            Err(e) => tracing::warn!(error = %e, "snooze sweep task failed"),
        }
    }
}

/// Re-read displays and theme every few seconds (monitor attach / detach /
/// DPI and theme changes the window's own events miss; the OS appearance
/// for the theme packs too), and poll the presentation state while the
/// overlay is shown (08 §6.7: 2 s on Windows; not detectable on Linux and
/// macOS yet).
async fn display_watch(
    app: AppHandle,
    core: Arc<CoreState>,
    overlay: Arc<Overlay>,
    platform: Arc<dyn OverlayPlatform>,
) {
    let mut presenting = false;
    let mut tick: u64 = 0;
    loop {
        tokio::time::sleep(DISPLAY_WATCH).await;
        tick += 1;
        if tick % 2 == 0 {
            overlay.on_main(|o| o.refresh_display(false));
            let (a, c, rm) = (app.clone(), Arc::clone(&core), platform.reduced_motion());
            let _ = app.run_on_main_thread(move || services::feed_appearance(&a, &c, rm));
        }
        if (overlay.is_visible() || presenting)
            && let Some(p) = platform.presentation()
            && p != presenting
        {
            presenting = p;
            overlay.on_main(move |o| o.set_presentation(p));
        }
    }
}
