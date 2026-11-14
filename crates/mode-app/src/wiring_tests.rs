//! Wiring tests for the app crate:
//! - the command list: `build.rs` ↔ `generate_handler!` ↔ the capability
//!   files (every command granted to a window, no grant without a command);
//! - the notes commands through [`commands_notes::with_notes`] against a
//!   temp database and a temp vault, with arguments shaped the way the
//!   webview sends them (`contract-notes.ts`);
//! - the Sessions view's commands (history, undismiss, snooze, diff,
//!   transcript, recent folders) and `tokens_week` against a temp database
//!   seeded through the session engine;
//! - the PTY host behind its commands (a fake `claude` in a real PTY):
//!   launch, list, open / replay, refusals as `{code, message, action?}`,
//!   the quit gate, quit-all, forget;
//! - package 9: the hook card extras, the kept round trip, the export →
//!   import round trip, the capture / templates folders, Diagnostics, the
//!   Stats shapes, "Delete all Mode data"'s file sweep, the capabilities;
//! - package 10: each connectors command group against a temp database, the
//!   file keyring and the fixture HTTP client (connect through the loopback,
//!   sync, Inbox triage, important people, the calendar, alerts and quiet hours, the AI
//!   backends, secrets), the alert sink mapping, the session alert watch,
//!   `note.due`, the offline / foreground transitions and the merged health
//!   and quiet payloads;
//! - package 11: every `chat` / `scheduler` / `themes` / `palette` api
//!   function is a command (the overlay gets the theme map and the palette
//!   only), Chat, Automations, theme packs and the palette behind their
//!   command helpers on a temp database, the `automations.changed` /
//!   `theme.changed` payloads, palette invalidation by the pushes, the
//!   `invalid` refusal with its fields, the quit stop.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use mode_core::files::diff;
use mode_core::ipc::Inbound;
use mode_core::notes::api::{self, NoteCaptureArgs, NoteSetFieldArgs, NotesListArgs};
use mode_core::notes::{Notes, NotesOptions};
use mode_core::queries::{self, Clock};
use mode_core::sessions::Engine;
use mode_core::sessions::history::HistoryQuery;
use mode_core::transcript::page;
use mode_wire::{Endpoint, HookEvent, HookFrame, PROTOCOL_VERSION};
use serde_json::{Value, json};

use crate::cmd_error::CmdError;
use crate::commands_notes::with_notes;
use crate::health;
use crate::state::CoreState;

// ------------------------------------------------------------ command list

const BUILD_RS: &str = include_str!("../build.rs");
const LIB_RS: &str = include_str!("lib.rs");
const CAPABILITIES: &[(&str, &str)] = &[
    ("app", include_str!("../capabilities/app.json")),
    ("overlay", include_str!("../capabilities/overlay.json")),
];

fn quoted(s: &str) -> Vec<String> {
    s.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

fn build_commands() -> Vec<String> {
    let start = BUILD_RS
        .find(".commands(&[")
        .expect("command list in build.rs");
    let end = start + BUILD_RS[start..].find("])").expect("end of the list");
    let body: String = BUILD_RS[start..end]
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    quoted(&body)
}

fn handler_commands() -> Vec<String> {
    let start = LIB_RS
        .find("generate_handler![")
        .expect("generate_handler! in lib.rs");
    let end = start + LIB_RS[start..].find("])").expect("end of the handler list");
    LIB_RS[start + "generate_handler![".len()..end]
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.rsplit("::").next().unwrap_or(s).to_string())
        .collect()
}

fn granted(cap: &str) -> BTreeSet<String> {
    let v: Value = serde_json::from_str(cap).expect("capability JSON");
    v["permissions"]
        .as_array()
        .expect("permissions")
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|p| p.strip_prefix("allow-"))
        .map(|p| p.replace('-', "_"))
        .collect()
}

#[test]
fn every_command_is_registered_and_granted_to_a_window() {
    let built = build_commands();
    let handled = handler_commands();
    assert!(
        built.len() > 60,
        "parsed {} commands from build.rs",
        built.len()
    );
    let built_set: BTreeSet<&String> = built.iter().collect();
    assert_eq!(built_set.len(), built.len(), "duplicates in build.rs");
    let handled_set: BTreeSet<&String> = handled.iter().collect();
    assert_eq!(
        built_set, handled_set,
        "build.rs and generate_handler! list the same commands"
    );
    let grants: BTreeMap<&str, BTreeSet<String>> =
        CAPABILITIES.iter().map(|(w, c)| (*w, granted(c))).collect();
    for cmd in &built {
        assert!(
            grants.values().any(|g| g.contains(cmd)),
            "{cmd} has no allow-permission in any capability file"
        );
    }
    for (window, g) in &grants {
        for cmd in g {
            assert!(
                built_set.contains(cmd),
                "{window}.json grants allow-{cmd}, which is not a command"
            );
        }
    }
    let overlay = &grants["overlay"];
    for app_only in [
        "settings_set",
        "hooks_install_apply",
        "hooks_uninstall",
        "notes_import",
        "note_trash",
        // the PTY host and the package 9 commands are the app window's
        "pty_open",
        "pty_write",
        "session_launch",
        "session_prompt",
        "pty_quit_all",
        "app_quit",
        "settings_get_all",
        "settings_import",
        "secret_set",
        "data_delete_all",
        "diagnostics_get",
        "stats_summary",
        // connectors: sign-in, secrets and settings stay in the app window
        "account_add_start",
        "account_add_finish",
        "account_remove",
        "oauth_client_set",
        "ai_key_set",
        "ai_backend_set",
        "keyword_rules_set",
        "vip_add",
        "inbox_list",
        "thread_get",
        "alert_rule_test",
        // dictation: the models and the microphone prompt are the app window's
        "dictation_model_ensure",
        "dictation_model_remove",
        "dictation_models",
        "dictation_mic_check",
        // the onboarding's Star on GitHub link (03 §2.8 row 7)
        "url_open",
        // the launch dialog's folder picker (03 §3.5)
        "sessions_pick_folder",
        // forgetting the Music tab's history is a Settings action (03 §8.3)
        "media_history_clear",
    ] {
        assert!(
            !overlay.contains(app_only),
            "the overlay must not get {app_only}"
        );
    }
    for needed in [
        "note_capture",
        "session_snooze",
        "app_open",
        "overlay_request",
        // the status item's pause and the overlay's inbox / calendar rows
        "quiet_set",
        "alert_dismiss",
        "message_mark_read",
        "thread_archive",
        "thread_snooze",
        "open_in_source",
        "event_join",
        // dictation into the overlay's fields (XC-37)
        "dictation_status",
        "dictation_cancel",
        "dictation_inserted",
        "app_context_set",
        // the dev build's typed transcript, relayed through the overlay (08 §12)
        "dictation_say",
        // the Now playing strip (02 §7.1, OV-37)
        "media_play_pause",
        "media_next",
        "media_previous",
        "media_open",
        // the Music tab (02 §7.10, ADR-0066)
        "media_history",
        "media_play",
    ] {
        assert!(overlay.contains(needed), "the overlay needs {needed}");
    }
    let app = &grants["app"];
    for needed in [
        "notes_list",
        "note_save",
        "sessions_history",
        "session_diff",
        "session_transcript",
        "health_get",
        "pty_open",
        "pty_detach",
        "pty_write",
        "pty_resize",
        "pty_close",
        "session_launch",
        "session_resume",
        "session_prompt",
        "turn_stop",
        "pty_quit_all",
        "pty_list",
        "claude_locate",
        "app_quit",
        "settings_get_all",
        "settings_reset",
        "settings_export",
        "settings_import",
        "platform_info",
        "hotkey_set",
        "notes_folders_set",
        "vault_pick",
        "vault_inspect",
        "profile_picture_pick",
        "data_delete_all",
        "secret_status",
        "secret_set",
        "ai_backends_status",
        "ai_test_call",
        "connectors_status",
        "diagnostics_get",
        "logs_tail",
        "diagnostics_export",
        "db_backup",
        "retention_sweep",
        "stats_days",
        "stats_summary",
        "url_open",
    ] {
        assert!(app.contains(needed), "the app window needs {needed}");
    }
    // Every connectors command is the app window's.
    for cmd in CONNECTOR_COMMANDS {
        assert!(built_set.contains(&cmd.to_string()), "{cmd} in build.rs");
        assert!(app.contains(*cmd), "the app window needs {cmd}");
    }
    assert_eq!(
        overlay
            .iter()
            .filter(|c| CONNECTOR_COMMANDS.contains(&c.as_str()))
            .count(),
        7,
        "the overlay's connectors set is exactly the seven row / status-item commands"
    );
}

/// Every function of `mode_core::connectors::api` as a command (plus the
/// app's `secret_*`).
const CONNECTOR_COMMANDS: &[&str] = &[
    "accounts_list",
    "account_add_start",
    "account_add_finish",
    "account_add_cancel",
    "account_pause",
    "account_resume",
    "account_retry",
    "account_reauthorise",
    "account_remove",
    "connectors_status",
    "claude_code_card",
    "oauth_client_set",
    "slack_manifest",
    "sources_list",
    "sources_refresh",
    "source_set",
    "slack_channels_list",
    "slack_channels_set",
    "inbox_list",
    "thread_get",
    "message_mark_read",
    "thread_archive",
    "thread_snooze",
    "thread_unsnooze",
    "open_in_source",
    "message_set_tier",
    "message_body",
    "keyword_rules_list",
    "keyword_rules_set",
    "urgency_rescore",
    "inbox_refresh",
    "vips_list",
    "vip_add",
    "vip_remove",
    "agenda_list",
    "event_join",
    "event_prepare_notes",
    "event_links",
    "item_link_add",
    "item_link_remove",
    "alerts_recent",
    "alert_dismiss",
    "alert_rule_test",
    "alert_rules_list",
    "alert_rule_set",
    "alert_rule_add",
    "alert_rule_delete",
    "quiet_set",
    "ai_backends_list",
    "ai_backend_set",
    "ai_key_set",
    "ai_health",
    "ai_test_call",
    "ai_backends_status",
    "urgency_diagnostics",
    "secret_status",
    "secret_set",
];

#[test]
fn the_app_window_may_open_only_allow_listed_urls_and_home_paths() {
    let v: Value = serde_json::from_str(CAPABILITIES[0].1).unwrap();
    let perms = v["permissions"].as_array().unwrap();
    assert!(!perms.iter().any(|p| p == "opener:default"));
    let url = perms
        .iter()
        .find(|p| p["identifier"] == "opener:allow-open-url")
        .expect("scoped open_url");
    let schemes: BTreeSet<&str> = url["allow"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["url"].as_str())
        .collect();
    // Plus one exact URL, no wildcard: the macOS Automation privacy pane that
    // Settings → Overlay opens after a refused Now playing permission (03 §8.3).
    assert_eq!(
        schemes,
        BTreeSet::from([
            "http://*",
            "https://*",
            "mailto:*",
            "slack://*",
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation",
        ])
    );
    let paths: BTreeSet<&str> = perms
        .iter()
        .find(|p| p["identifier"] == "opener:allow-open-path")
        .expect("scoped open_path")["allow"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["path"].as_str())
        .collect();
    for needed in [
        "$HOME/**",
        "$HOME/.claude/settings.json",
        "$DATA/Mode/logs",
        "$DATA/Mode-dev/logs",
    ] {
        assert!(paths.contains(needed), "open_path needs {needed}");
    }
    assert!(perms.iter().any(|p| p == "opener:allow-reveal-item-in-dir"));
    assert!(
        !perms
            .iter()
            .filter_map(Value::as_str)
            .any(|p| p.starts_with("dialog:")),
        "the core shows the pickers; the webview gets no dialog grant"
    );
}

#[test]
fn the_asset_protocol_serves_only_the_profile_folder() {
    let conf: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let asset = &conf["app"]["security"]["assetProtocol"];
    assert_eq!(asset["enable"], true);
    let scope: BTreeSet<&str> = asset["scope"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        scope,
        BTreeSet::from(["$DATA/Mode/profile/**", "$DATA/Mode-dev/profile/**"])
    );
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    assert!(csp.contains("img-src 'self' asset: http://asset.localhost"));
    // Features are declared once, in the workspace table (Cargo.toml).
    let cargo = include_str!("../../../Cargo.toml");
    let tauri = cargo
        .lines()
        .find(|l| l.starts_with("tauri = "))
        .expect("tauri in [workspace.dependencies]");
    assert!(
        tauri.contains("\"protocol-asset\""),
        "the tauri feature: {tauri}"
    );
    assert!(
        include_str!("../Cargo.toml").contains("tauri = { workspace = true }"),
        "mode-app takes tauri from the workspace table"
    );
}

// ------------------------------------------------------------------ rigs

pub(crate) fn core_with(dir: &Path) -> Arc<CoreState> {
    let db_path = dir.join("data").join("mode.db");
    let db = Arc::new(mode_core::db::open(&db_path).unwrap());
    let engine = Arc::new(Engine::new(Arc::clone(&db)));
    let core = Arc::new(CoreState::new(
        db,
        engine,
        dir.join("data"),
        db_path,
        Endpoint::at(dir.join("mode.sock").to_string_lossy().into_owned()),
    ));
    // Tests never touch the OS keychain (macOS / Windows CI): the file store
    // under the temp data dir, as a Linux dev build picks it.
    let file = mode_core::keyring::Keyring::file_path(&dir.join("data"));
    let _ = core
        .keyring
        .set(Arc::new(mode_core::keyring::Keyring::file(file)));
    core
}

/// Every `overlay.flash` the core sends, in order.
pub(crate) fn record_flashes(
    core: &CoreState,
) -> Arc<std::sync::Mutex<Vec<mode_core::push::OverlayFlash>>> {
    let seen: Arc<std::sync::Mutex<Vec<mode_core::push::OverlayFlash>>> = Arc::default();
    let s = Arc::clone(&seen);
    assert!(
        core.flash_sink
            .set(Arc::new(move |f| s.lock().unwrap().push(f.clone())))
            .is_ok()
    );
    seen
}

#[test]
fn flashes_name_the_action_and_where_a_click_goes() {
    use mode_core::connectors::Provider;
    use mode_core::push::{Event, FlashKind, Push};

    let dir = tempfile::tempdir().unwrap();
    let core = core_with(dir.path());
    core.flash("before the sink", FlashKind::Info, None);
    let seen = record_flashes(&core);
    let last = || seen.lock().unwrap().last().cloned();
    let count = || seen.lock().unwrap().len();

    // Accounts: connected, failed, canceled.
    let failed: crate::cmd_error::CmdResult<mode_core::connectors::accounts::AccountRow> =
        Err(CmdError::new("exchange", "token exchange failed"));
    crate::commands_connectors::flash_connect(&core, Some(Provider::Google), &failed);
    let f = last().unwrap();
    assert_eq!(
        (f.text.as_str(), f.kind),
        ("Gmail · sign-in failed", FlashKind::Bad)
    );
    let open = f.open.unwrap();
    assert_eq!(
        (open.view.as_str(), open.id.as_deref()),
        ("integrations", Some("gmail"))
    );
    let cancelled = Err(CmdError::new("cancelled", "cancelled"));
    crate::commands_connectors::flash_connect(&core, Some(Provider::Slack), &cancelled);
    assert_eq!(count(), 1, "a canceled sign-in says nothing");

    // Launch: a session without a row falls back to "session"; an
    // already-open resume or a failed start says nothing.
    let mut launched = mode_core::pty::api::LaunchResult {
        session_id: "s-1".into(),
        hooks_installed: true,
        error: None,
        already_open: false,
    };
    crate::commands_pty::flash_launching(&core, &launched);
    let f = last().unwrap();
    assert_eq!(
        (f.text.as_str(), f.kind),
        ("launching · session", FlashKind::Info)
    );
    assert!(f.keep_pill, "held until the session's own pill takes over");
    let open = f.open.unwrap();
    assert_eq!(
        (open.view.as_str(), open.id.as_deref()),
        ("sessions", Some("s-1"))
    );
    launched.already_open = true;
    crate::commands_pty::flash_launching(&core, &launched);
    launched.already_open = false;
    launched.error = Some("claude not found".into());
    crate::commands_pty::flash_launching(&core, &launched);
    assert_eq!(count(), 2);

    // Chat: started (held), answered (first five words, opens the
    // conversation), failed; other streams are not flashes.
    let turn = |payload: Value| Push {
        event: Event::ChatTurn,
        payload,
    };
    let started = crate::services::chat_flash(&turn(
        json!({"turn_id": "t", "thread_id": "th", "status": "started"}),
    ))
    .unwrap();
    assert_eq!(started.text, "thinking · Mode");
    assert!(started.keep_pill);
    let answered = crate::services::chat_flash(&turn(json!({
        "turn_id": "t", "thread_id": "th", "status": "finished",
        "turn": {"text": "Three attempts with backoff, then give up."}
    })))
    .unwrap();
    assert_eq!(
        answered.text,
        "answered · Three attempts with backoff, then…"
    );
    assert_eq!((answered.kind, answered.keep_pill), (FlashKind::Ok, false));
    let open = answered.open.unwrap();
    assert_eq!(
        (open.view.as_str(), open.id.as_deref()),
        ("chat", Some("th"))
    );
    let failed = crate::services::chat_flash(&turn(
        json!({"turn_id": "t", "thread_id": "th", "status": "failed", "error": "no key"}),
    ))
    .unwrap();
    assert_eq!(failed.kind, FlashKind::Bad);
    let stopped = crate::services::chat_flash(&turn(
        json!({"turn_id": "t", "thread_id": "th", "status": "finished", "stopped": true}),
    ))
    .unwrap();
    assert_eq!(stopped.text, "stopped · Mode");
    assert!(
        crate::services::chat_flash(&Push {
            event: Event::ChatDelta,
            payload: json!({"turn_id": "t"}),
        })
        .is_none()
    );
}

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Rig {
    _dir: tempfile::TempDir,
    root: PathBuf,
    core: Arc<CoreState>,
    frames: u64,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let core = core_with(&root);
        Rig {
            _dir: dir,
            root,
            core,
            frames: 0,
        }
    }

    fn send(&mut self, payload: HookEvent) -> String {
        self.frames += 1;
        let inbound = Inbound {
            frame: HookFrame {
                v: PROTOCOL_VERSION,
                id: format!("frame-{}", self.frames),
                event: payload.hook_event_name.clone(),
                relay_version: mode_core::ipc::EXPECTED_RELAY_VERSION.into(),
                wants_decision: false,
                sent_at: mode_core::db::now_rfc3339(),
                ppid: None,
                tty: None,
                env: BTreeMap::new(),
                launch_id: None,
                truncated: false,
                decision_wait_ms: None,
                payload,
            },
            received_at: Instant::now(),
            relay_stale: false,
        };
        self.core
            .engine
            .apply(&inbound)
            .unwrap()
            .expect("applied")
            .session_id
    }

    fn event(name: &str, claude_id: &str, cwd: &Path, transcript: &Path) -> HookEvent {
        let mut ev = HookEvent::new(name, claude_id);
        ev.cwd = cwd.to_string_lossy().into_owned();
        ev.transcript_path = transcript.to_string_lossy().into_owned();
        ev
    }

    fn edit(
        claude_id: &str,
        cwd: &Path,
        transcript: &Path,
        file: &Path,
        old: &str,
        new: &str,
        tool_use_id: &str,
    ) -> HookEvent {
        let mut ev = Self::event("PostToolUse", claude_id, cwd, transcript);
        ev.set("tool_name", "Edit");
        ev.set("tool_use_id", tool_use_id);
        ev.set(
            "tool_input",
            json!({"file_path": file.to_string_lossy(), "old_string": old, "new_string": new}),
        );
        ev.set("tool_response", json!({"filePath": file.to_string_lossy()}));
        ev
    }

    fn file_id(&self, session_id: &str, path: &Path) -> String {
        let (sid, p) = (session_id.to_string(), path.to_string_lossy().into_owned());
        self.core
            .db
            .read(move |c| {
                Ok(c.query_row(
                    "SELECT id FROM file_changes WHERE session_id = ? AND path = ?",
                    [&sid, &p],
                    |r| r.get::<_, String>(0),
                )?)
            })
            .unwrap()
    }
}

// ----------------------------------------------------------------- notes

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tauri::async_runtime::block_on(f)
}

#[test]
fn notes_commands_answer_against_a_temp_vault() {
    let dir = tempfile::tempdir().unwrap();
    let core = core_with(dir.path());
    let vault = dir.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(
        vault.join("Existing.md"),
        "---\nstatus: doing\ntags: [home]\n---\n\nBuy paint\n",
    )
    .unwrap();

    // Before the service is set (a newer database): `unavailable`.
    let err = block_on(with_notes(&core, api::folders_list)).unwrap_err();
    assert_eq!(err.code, "unavailable");

    let notes = Notes::new(
        Arc::clone(&core.db),
        NotesOptions {
            root: Some(vault.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let report = notes.init().unwrap();
    assert_eq!(report.files, 1);
    let pushes = Arc::new(std::sync::Mutex::new(0usize));
    {
        let pushes = Arc::clone(&pushes);
        notes.set_publisher(move |e| {
            assert_eq!(e, mode_core::push::Event::Notes);
            *pushes.lock().unwrap() += 1;
        });
    }
    assert!(core.notes.set(Arc::clone(&notes)).is_ok());

    // The overlay's capture box: `{text, created_from?}`.
    let capture: NoteCaptureArgs =
        serde_json::from_value(json!({"text": "Call the plumber #home !"})).unwrap();
    let captured = block_on(with_notes(&core, move |n| api::note_capture(n, capture))).unwrap();
    assert!(captured.path.ends_with(".md"), "{}", captured.path);
    assert!(vault.join(&captured.path).is_file());
    assert!(
        *pushes.lock().unwrap() >= 1,
        "a capture requests notes.changed"
    );

    // The Notes view's list: `{scope: {kind: 'folder', folder_id: 'root'}, show_archived}`.
    let list: NotesListArgs = serde_json::from_value(
        json!({"scope": {"kind": "folder", "folder_id": "root", "recursive": true}, "show_archived": false}),
    )
    .unwrap();
    let rows = block_on(with_notes(&core, move |n| api::notes_list(n, list))).unwrap();
    assert_eq!(rows.count, 2);
    let titles: BTreeSet<String> = rows.rows.iter().map(|r| r.title.clone()).collect();
    assert!(titles.contains("Existing"), "{titles:?}");

    // note_get then a one-field write (`{note_id, edit: {field, value}}`).
    let id = captured.note_id.clone();
    let detail = block_on(with_notes(&core, move |n| {
        api::note_get(n, serde_json::from_value(json!({"note_id": id}))?)
    }))
    .unwrap();
    assert_eq!(detail.row.id, captured.note_id);
    let set: NoteSetFieldArgs = serde_json::from_value(
        json!({"note_id": captured.note_id, "edit": {"field": "status", "value": "doing"}}),
    )
    .unwrap();
    let written = block_on(with_notes(&core, move |n| api::note_set_field(n, set))).unwrap();
    let v = serde_json::to_value(&written).unwrap();
    assert_eq!(v["outcome"], "saved");
    assert_eq!(v["row"]["status"], "doing");

    // Lists the sidebar loads.
    let g = block_on(with_notes(&core, api::groupings_list)).unwrap();
    assert!(g.groupings.iter().any(|r| r.name == "Doing"));
    let f = block_on(with_notes(&core, api::folders_list)).unwrap();
    assert_eq!(f.vault_path, vault.to_string_lossy());
    let tags = block_on(with_notes(&core, api::tags_list)).unwrap();
    assert!(tags.iter().any(|t| t.name == "home"));

    // An unknown note rejects as {code, message}.
    let err: CmdError = block_on(with_notes(&core, |n| {
        api::note_get(
            n,
            api::NoteIdArgs {
                note_id: "nope".into(),
            },
        )
    }))
    .unwrap_err();
    assert_eq!(err.code, "not_found");
    let v = serde_json::to_value(&err).unwrap();
    assert!(v["message"].is_string() && v.as_object().unwrap().len() == 2);

    // The vault feeds health: present now, missing once it is gone.
    assert!(!health::compute(&core).vault_missing);
    std::fs::remove_dir_all(&vault).unwrap();
    let _ = notes.scan();
    assert!(notes.vault_status().missing);
    assert!(health::compute(&core).vault_missing);
}

// -------------------------------------------------------------- sessions

#[test]
fn sessions_history_undismiss_snooze_and_recent_folders() {
    let mut rig = Rig::new();
    let cwd = rig.root.join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let transcript = rig.root.join("t.jsonl");
    let mut start = Rig::event("SessionStart", "claude-a", &cwd, &transcript);
    start.set("source", "startup");
    let sid = rig.send(start);
    rig.send(Rig::event("Stop", "claude-a", &cwd, &transcript));
    let engine = Arc::clone(&rig.core.engine);
    let roster = engine.roster().unwrap();
    assert_eq!(roster[0].row.state, "unread");

    // Snooze hides a finished session from the pill (03 §3.4); null cancels.
    let pill = |e: &Engine| e.pill().unwrap().candidate.map(|c| c.kind);
    assert_eq!(pill(&engine).as_deref(), Some("session_finished"));
    let (until, pushes) = engine.snooze(&sid, Some("tomorrow"), None).unwrap();
    let until = until.expect("a snooze end");
    assert!(until > mode_core::db::now_rfc3339());
    assert!(pushes.contains(&mode_core::push::Event::Pill));
    assert_ne!(pill(&engine).as_deref(), Some("session_finished"));
    assert!(engine.snooze(&sid, Some("whenever"), None).is_err());
    assert!(
        engine
            .snooze(&sid, None, Some("2001-01-01T00:00:00Z"))
            .is_err()
    );
    let err = engine
        .snooze(&sid, Some("someday"), Some("2999-01-01T00:00:00Z"))
        .unwrap_err();
    assert_eq!(CmdError::from(err).code, "invalid");
    let (picked, _) = engine
        .snooze(&sid, None, Some("2999-01-01T09:00:00Z"))
        .unwrap();
    assert_eq!(picked.as_deref(), Some("2999-01-01T09:00:00.000Z"));
    let (none, _) = engine.snooze(&sid, None, None).unwrap();
    assert!(none.is_none());
    assert_eq!(pill(&engine).as_deref(), Some("session_finished"));
    assert!(engine.snooze("missing", None, None).is_err());

    // Dismissed rows leave the roster for the history (05 §12).
    assert!(engine.history(&HistoryQuery::default()).unwrap().is_empty());
    engine.dismiss(&sid).unwrap();
    assert!(engine.roster().unwrap().is_empty());
    let h = engine.history(&HistoryQuery::default()).unwrap();
    assert_eq!(h.len(), 1);
    let v = serde_json::to_value(&h[0]).unwrap();
    assert_eq!(v["id"], sid);
    assert!(v["dismissed_at"].is_string());
    assert_eq!(v["display_name"], "proj");
    assert!(v.get("claude_session_id").is_some() && v.get("state_seconds").is_some());
    let q = |s: &str| HistoryQuery {
        query: Some(s.into()),
        ..Default::default()
    };
    assert_eq!(engine.history(&q("PRO")).unwrap().len(), 1);
    assert_eq!(engine.history(&q("50%_")).unwrap().len(), 0);
    let before = HistoryQuery {
        before: v["dismissed_at"].as_str().map(str::to_string),
        ..Default::default()
    };
    assert!(
        engine.history(&before).unwrap().is_empty(),
        "paging excludes the cursor row"
    );
    assert_eq!(engine.undismiss(&sid).unwrap().len(), 2);
    assert!(engine.history(&HistoryQuery::default()).unwrap().is_empty());
    assert_eq!(engine.roster().unwrap().len(), 1);

    // Recent folders: distinct cwd, newest first.
    let other = rig.root.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let mut s2 = Rig::event("SessionStart", "claude-b", &other, &transcript);
    s2.set("source", "startup");
    rig.send(s2);
    let folders = engine.recent_folders(20).unwrap();
    assert_eq!(folders.len(), 2);
    assert!(folders.contains(&cwd.to_string_lossy().into_owned()));
}

#[test]
fn session_diff_uses_git_in_a_repository_and_the_edit_outside_one() {
    let mut rig = Rig::new();
    let repo = rig.root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "seed"]);
    let transcript = rig.root.join("t.jsonl");
    let mut start = Rig::event("SessionStart", "claude-r", &repo, &transcript);
    start.set("source", "startup");
    let sid = rig.send(start);
    std::fs::write(repo.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    let file = repo.join("a.txt");
    rig.send(Rig::edit(
        "claude-r",
        &repo,
        &transcript,
        &file,
        "two",
        "TWO",
        "tu-1",
    ));
    let fid = rig.file_id(&sid, &file);
    let d = diff::diff_for(&rig.core.db, &sid, &fid)
        .unwrap()
        .expect("a diff");
    assert_eq!(d.basis, "working tree vs HEAD");
    let texts: Vec<&str> = d.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(
        texts.contains(&"-two") && texts.contains(&"+TWO"),
        "{texts:?}"
    );
    let add = d.lines.iter().find(|l| l.kind == "add").unwrap();
    assert_eq!(add.new_no, Some(2));
    assert!(
        diff::diff_for(&rig.core.db, &sid, "no-such-row")
            .unwrap()
            .is_none()
    );
    assert!(
        diff::diff_for(&rig.core.db, "no-such-session", &fid)
            .unwrap()
            .is_none()
    );

    // Outside a repository: the stored edit, placed at its line.
    let plain = rig.root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let mut s2 = Rig::event("SessionStart", "claude-p", &plain, &transcript);
    s2.set("source", "startup");
    let sid2 = rig.send(s2);
    let notes = plain.join("notes.txt");
    std::fs::write(&notes, "alpha\nbeta\ngamma\n").unwrap();
    rig.send(Rig::edit(
        "claude-p",
        &plain,
        &transcript,
        &notes,
        "beta\n",
        "BETA\nbeta2\n",
        "tu-2",
    ));
    std::fs::write(&notes, "alpha\nBETA\nbeta2\ngamma\n").unwrap();
    let fid2 = rig.file_id(&sid2, &notes);
    let d = diff::diff_for(&rig.core.db, &sid2, &fid2).unwrap().unwrap();
    assert_eq!(d.basis, "last edit");
    let kinds: Vec<(&str, Option<u32>, Option<u32>)> = d
        .lines
        .iter()
        .map(|l| (l.kind, l.old_no, l.new_no))
        .collect();
    assert_eq!(
        kinds,
        [
            ("hunk", None, None),
            ("del", Some(2), None),
            ("add", None, Some(2)),
            ("add", None, Some(3))
        ]
    );
    assert_eq!(d.lines[0].text, "@@ -2 +2,2 @@");
}

#[test]
fn session_transcript_pages_the_session_file_with_its_subagents() {
    let mut rig = Rig::new();
    let cwd = rig.root.join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let transcript = rig.root.join("claude-t.jsonl");
    let mut lines = String::new();
    for i in 1..=7 {
        lines.push_str(&json!({"type": "user", "timestamp": "2026-10-21T12:00:00.000Z", "message": {"role": "user", "content": format!("prompt {i}")}}).to_string());
        lines.push('\n');
    }
    lines.push_str(&json!({"type": "cost-state", "totalCostUSD": 0.5}).to_string());
    lines.push('\n');
    lines.push_str(&json!({"type": "assistant", "message": {"role": "assistant", "usage": {"input_tokens": 3, "output_tokens": 4}, "content": [{"type": "tool_use", "id": "x", "name": "Read", "input": {"file_path": "src/a.rs"}}]}}).to_string());
    lines.push('\n');
    std::fs::write(&transcript, lines).unwrap();
    let subs = rig.root.join("claude-t").join("subagents");
    std::fs::create_dir_all(&subs).unwrap();
    std::fs::write(
        subs.join("agent-a1.jsonl"),
        format!("{}\n", json!({"type": "user", "agentId": "a1", "message": {"role": "user", "content": "check"}})),
    )
    .unwrap();
    std::fs::write(
        subs.join("agent-a1.meta.json"),
        r#"{"agentType":"test-runner"}"#,
    )
    .unwrap();
    let mut start = Rig::event("SessionStart", "claude-t", &cwd, &transcript);
    start.set("source", "startup");
    let sid = rig.send(start);
    let path = rig
        .core
        .db
        .read(|c| mode_core::repo::get_by_id::<mode_core::repo::Session>(c, &sid))
        .unwrap()
        .unwrap()
        .transcript_path;

    let first = page::page(path.as_deref(), None, 4);
    assert!(first.exists && first.has_more);
    assert_eq!(first.subagent_files, 1);
    let main: Vec<u64> = first
        .lines
        .iter()
        .filter(|l| l.agent_id.is_none())
        .map(|l| l.line)
        .collect();
    assert_eq!(main, [6, 7, 9], "line 8 (cost-state) is left out");
    let tool = first.lines.iter().find(|l| l.line == 9).unwrap();
    assert_eq!(tool.label, "tool_use · Read");
    assert_eq!(tool.usage.unwrap().output_tokens, 4);
    let sub = first.lines.iter().find(|l| l.agent_id.is_some()).unwrap();
    assert_eq!(sub.agent_type.as_deref(), Some("test-runner"));
    let v = serde_json::to_value(&first).unwrap();
    assert!(!v.to_string().contains("CostUSD"));
    assert_eq!(v["lines"][0]["type"], "user");

    let earlier = page::page(path.as_deref(), Some(6), 4);
    let nums: Vec<u64> = earlier.lines.iter().map(|l| l.line).collect();
    assert_eq!(nums, [2, 3, 4, 5]);
    assert!(
        earlier.lines.iter().all(|l| l.agent_id.is_none()),
        "subagents only on the first page"
    );
    assert!(earlier.has_more);
}

#[test]
fn tokens_week_sums_claude_code_tokens_since_the_week_start() {
    let rig = Rig::new();
    let now = mode_core::db::now_rfc3339();
    let long_ago = "2020-01-01T00:00:00.000Z".to_string();
    rig.core
        .db
        .write(move |c| {
            for (id, at, source, purpose, n) in [
                ("u1", now.as_str(), "claude_code", "session", 100),
                ("u2", now.as_str(), "claude_code", "session", 20),
                ("u3", long_ago.as_str(), "claude_code", "session", 5_000),
                ("u4", now.as_str(), "claude_api", "scoring", 7_000),
            ] {
                c.execute(
                    "INSERT INTO usage_samples (id, sampled_at, source, purpose, model, input_tokens, output_tokens,
                                                cache_read_tokens, cache_write_tokens, created_at)
                     VALUES (?1, ?2, ?3, ?4, 'm', ?5, 1, 2, 3, ?2)",
                    (id, at, source, purpose, n),
                )?;
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(health::tokens_week(&rig.core.db), (100 + 6) + (20 + 6));
    let start = rig
        .core
        .db
        .read(|c| {
            let clock = Clock::current(c)?;
            queries::usage_week_start(c, &clock.now, 1)
        })
        .unwrap();
    assert!(start <= mode_core::db::now_rfc3339());
    let h = health::compute(&rig.core);
    assert_eq!(h.tokens_week, 132);
    assert!(h.first_run, "onboarding.completed_at is unset");
    assert!(!h.db_newer);
    assert!(h.backups_dir.ends_with("backups"));
    assert!(matches!(h.claude_code, "found" | "missing"));
}

// ------------------------------------------------------------- PTY host

/// A stand-in `claude`: prints a banner, echoes lines, exits on SIGINT
/// (130) or end of input (0).
#[cfg(unix)]
const FAKE_CLAUDE: &str = r#"#!/usr/bin/env bash
printf 'fake claude ready\n'
trap 'printf "BYE\n"; exit 130' INT
while IFS= read -r line; do printf 'GOT:%s\n' "$line"; done
exit 0
"#;

#[cfg(unix)]
struct PtyRig {
    rig: Rig,
    pushes: Arc<std::sync::Mutex<Vec<mode_core::push::Push>>>,
    work: PathBuf,
}

#[cfg(unix)]
impl PtyRig {
    fn new() -> PtyRig {
        use std::os::unix::fs::PermissionsExt;
        let rig = Rig::new();
        let fake = rig.root.join("claude");
        std::fs::write(&fake, FAKE_CLAUDE).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        mode_core::settings::set(
            &rig.core.db,
            "sessions.claude_path",
            &Value::String(fake.to_string_lossy().into_owned()),
        )
        .unwrap();
        let work = rig.root.join("my-project");
        std::fs::create_dir_all(&work).unwrap();
        let pushes: Arc<std::sync::Mutex<Vec<mode_core::push::Push>>> = Arc::default();
        let p = Arc::clone(&pushes);
        let core = Arc::clone(&rig.core);
        let pty = mode_core::pty::Pty::new(
            Arc::clone(&rig.core.engine),
            Arc::new(move |push: &mode_core::push::Push| p.lock().unwrap().push(push.clone())),
            Arc::new(move |events: &[mode_core::push::Event]| core.publish(events.iter().copied())),
            mode_core::pty::Config {
                timings: mode_core::pty::Timings {
                    session_end_grace: std::time::Duration::from_millis(150),
                    turn_wait: std::time::Duration::from_secs(5),
                    exit_step: std::time::Duration::from_millis(600),
                    tick: std::time::Duration::from_millis(25),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        assert!(rig.core.pty.set(pty).is_ok());
        PtyRig { rig, pushes, work }
    }

    fn events(&self) -> Vec<mode_core::push::Event> {
        self.pushes
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.event)
            .collect()
    }
}

#[cfg(unix)]
fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(unix)]
#[test]
fn the_pty_host_runs_behind_its_commands_and_gates_the_quit() {
    use mode_core::pty::api::{
        self, LaunchArgs, PromptArgs, PtyOpenArgs, QuitAllArgs, SessionArgs,
    };

    use crate::commands_pty::with_pty;
    use crate::quit::{self, ExitDecision, QuitGate, QuitStep};

    // Before the host is set: `unavailable`.
    let bare = Rig::new();
    let err = block_on(with_pty(&bare.core, api::pty_list)).unwrap_err();
    assert_eq!(err.code, "unavailable");
    assert!(quit::live_sessions(&bare.core).is_empty());

    let r = PtyRig::new();
    let core = Arc::clone(&r.rig.core);
    assert!(block_on(with_pty(&core, api::pty_list)).unwrap().is_empty());
    assert_eq!(
        QuitGate::default().on_exit_requested(|| quit::live_sessions(&core)),
        ExitDecision::Allow,
        "nothing live: quit goes straight through"
    );
    let found = block_on(with_pty(&core, api::claude_locate)).unwrap();
    assert!(found.found);
    assert_eq!(found.source.as_deref(), Some("setting"));

    // Launch through the command wrapper, the contract's argument shape.
    let args: LaunchArgs = serde_json::from_value(
        json!({"cwd": r.work.to_string_lossy(), "open": "terminal", "cols": 90, "rows": 25}),
    )
    .unwrap();
    let launched = block_on(with_pty(&core, move |p| api::session_launch(p, args))).unwrap();
    assert!(launched.error.is_none(), "{:?}", launched.error);
    assert!(!launched.hooks_installed);
    let sid = launched.session_id.clone();
    let v = serde_json::to_value(&launched).unwrap();
    assert_eq!(v["session_id"], sid);

    let live = block_on(with_pty(&core, api::pty_list)).unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].driver, "pty");

    // Attach: the replay carries the banner (base64), then input echoes.
    let s = sid.clone();
    wait_until("the banner", || {
        let s = s.clone();
        let o = block_on(with_pty(&core, move |p| {
            api::pty_open(
                p,
                PtyOpenArgs {
                    session_id: s,
                    cols: 100,
                    rows: 30,
                },
            )
        }))
        .unwrap();
        let bytes = mode_core::pty::b64_decode(&o.data).unwrap();
        String::from_utf8_lossy(&bytes).contains("fake claude ready")
    });
    let s = sid.clone();
    block_on(with_pty(&core, move |p| {
        api::pty_write(
            p,
            mode_core::pty::api::PtyWriteArgs {
                session_id: s,
                data: mode_core::pty::b64(b"hello\r"),
            },
        )
    }))
    .unwrap();
    wait_until("the echo on pty.output", || {
        r.pushes.lock().unwrap().iter().any(|p| {
            p.event == mode_core::push::Event::PtyOutput
                && p.payload["data"]
                    .as_str()
                    .and_then(|d| mode_core::pty::b64_decode(d).ok())
                    .is_some_and(|b| String::from_utf8_lossy(&b).contains("GOT:hello"))
        })
    });
    let events = r.events();
    assert!(events.contains(&mode_core::push::Event::PtyStarted));
    assert!(
        events.iter().all(|e| e.is_stream()),
        "only streams reach the sink"
    );

    // Errors keep their code; an unknown session has no terminal.
    let err = block_on(with_pty(&core, |p| {
        api::pty_open(
            p,
            PtyOpenArgs {
                session_id: "nope".into(),
                cols: 80,
                rows: 24,
            },
        )
    }))
    .unwrap_err();
    assert_eq!(err.code, "not_found");
    let err = block_on(with_pty(&core, |p| {
        api::session_prompt(
            p,
            PromptArgs {
                session_id: "nope".into(),
                text: "hi".into(),
            },
        )
    }))
    .unwrap_err();
    assert_eq!(err.code, "not_found");
    let s = sid.clone();
    let err = block_on(with_pty(&core, move |p| {
        api::session_prompt(
            p,
            PromptArgs {
                session_id: s,
                text: "   ".into(),
            },
        )
    }))
    .unwrap_err();
    assert_eq!(err.code, "invalid");

    // ⌘Q with a live terminal: held and asked; the profile row's Quit too.
    let gate = QuitGate::default();
    match gate.on_exit_requested(|| quit::live_sessions(&core)) {
        ExitDecision::Confirm(sessions) => assert_eq!(sessions[0].session_id, sid),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        gate.on_app_quit(None, || quit::live_sessions(&core)),
        QuitStep::Confirm(_)
    ));
    assert_eq!(
        gate.on_app_quit(Some(true), Vec::new),
        QuitStep::StopThenExit
    );

    // What confirming runs: SIGINT, wait; nothing remains.
    let done = block_on(with_pty(&core, |p| {
        api::pty_quit_all(
            p,
            QuitAllArgs {
                wait_ms: Some(5_000),
            },
        )
    }))
    .unwrap();
    assert_eq!(done.signalled, std::slice::from_ref(&sid));
    assert!(done.remaining.is_empty(), "{:?}", done.remaining);
    assert!(quit::live_sessions(&core).is_empty());
    wait_until("pty.exited", || {
        r.events().contains(&mode_core::push::Event::PtyExited)
    });

    // The exited terminal still replays until the row is dismissed.
    let s = sid.clone();
    let o = block_on(with_pty(&core, move |p| {
        api::pty_open(
            p,
            PtyOpenArgs {
                session_id: s,
                cols: 80,
                rows: 24,
            },
        )
    }))
    .unwrap();
    assert!(o.exited);
    let s = sid.clone();
    block_on(with_pty(&core, move |p| {
        api::pty_detach(p, SessionArgs { session_id: s })
    }))
    .unwrap();
    assert!(
        core.pty.get().unwrap().forget(&sid) > 0,
        "dismiss forgets the replay"
    );
    assert!(core.pty.get().unwrap().proc(&sid).is_none());
}

// -------------------------------------------------------- package 9: hooks

#[test]
fn hook_card_extras_and_the_kept_round_trip() {
    use mode_core::hooks_install::{self, Target, Timeouts};

    let rig = Rig::new();
    let target = Target::native(
        rig.root.join("home/.claude/settings.json"),
        rig.root.join("data/bin/mode-hook"),
    );
    let plan = hooks_install::plan_install(&target, &Timeouts::default()).unwrap();
    let v = serde_json::to_value(crate::commands::PlanView {
        plan,
        backup_dir: rig.core.backups_dir().to_string_lossy().into_owned(),
        relay_version: Some(hooks_install::RELAY_VERSION.into()),
    })
    .unwrap();
    assert_eq!(v["action"], "install");
    assert!(v["diff"].is_string() && v["events"].is_array());
    assert!(v["backup_dir"].as_str().unwrap().ends_with("backups"));
    assert_eq!(v["relay_version"], hooks_install::RELAY_VERSION);
    let un = hooks_install::plan_uninstall(&target).unwrap();
    assert_eq!(serde_json::to_value(&un).unwrap()["action"], "uninstall");

    // Applied → the status carries the `hook_installs` facts.
    let backups = rig.core.backups_dir();
    let applied = hooks_install::install(&target, &Timeouts::default(), Some(&backups)).unwrap();
    let (t, a) = (target.clone(), applied.clone());
    rig.core
        .db
        .write(move |c| {
            mode_core::hooks_db::record_install(c, &t, &a, &[], &mode_core::db::now_rfc3339())
        })
        .unwrap();
    let status = hooks_install::inspect(&target, &Timeouts::default(), true).unwrap();
    let view = serde_json::to_value(crate::commands::status_view(&rig.core, status)).unwrap();
    assert_eq!(view["status"], "installed");
    assert!(view["installed_at"].is_string() && view["verified_at"].is_string());
    assert!(view.get("backup_path").is_some());
    assert!(
        view.get("last_round_trip").is_some(),
        "flattened Status fields stay"
    );

    // The round trip survives a restart through the setting.
    assert!(crate::commands::last_round_trip(&rig.core).is_none());
    let report = json!({"session_id": "mode-round-trip-1", "latency_ms": 9, "acked": true,
                        "relay_version": "0.1.0", "at": "2026-10-22T10:00:00.000Z"});
    mode_core::settings::set(
        &rig.core.db,
        mode_core::settings::LAST_ROUND_TRIP_KEY,
        &report,
    )
    .unwrap();
    let kept = crate::commands::last_round_trip(&rig.core).expect("kept");
    assert_eq!(kept.latency_ms, 9);
    assert!(
        mode_core::settings::validate(mode_core::settings::LAST_ROUND_TRIP_KEY, &report).is_err(),
        "the webview cannot write it"
    );
}

// -------------------------------------------------- package 9: settings

fn custom_grouping(name: &str) -> mode_core::repo::Grouping {
    mode_core::repo::Grouping {
        id: mode_core::repo::new_id(),
        name: name.into(),
        slug: name.to_lowercase().replace(' ', "-"),
        kind: "custom".into(),
        filter_json: r#"{"tags":{"any":["home"]}}"#.into(),
        sort_json: mode_core::db::seeds::DEFAULT_SORT.into(),
        position: 50,
        show_in_sidebar: true,
        ..Default::default()
    }
}

#[test]
fn settings_export_then_import_round_trips_with_a_preview() {
    use mode_core::settings::ImportOutcome;

    use crate::bundle;

    let a = Rig::new();
    let b = Rig::new();
    mode_core::settings::set(&a.core.db, "sounds.volume", &json!(80)).unwrap();
    mode_core::settings::set(&a.core.db, "theme.active", &json!("nord")).unwrap();
    let mut g = custom_grouping("Home jobs");
    let device = a.core.db.device_id().to_string();
    let g2 = g.clone();
    a.core
        .db
        .write(move |c| {
            let mut g = g2;
            mode_core::repo::create(c, &device, &mut g)
        })
        .unwrap();

    let sections = vec!["settings".to_string(), "groupings".to_string()];
    let exported = a
        .core
        .db
        .read(|c| bundle::export_in(c, &sections, false))
        .unwrap();
    let text = serde_json::to_string_pretty(&exported).unwrap();
    assert!(!text.contains("theme.active"), "device scope stays home");
    assert!(!text.contains("device.id"));
    let parsed = bundle::parse(&text).unwrap();
    assert_eq!(parsed.sections, sections);
    assert!(parsed.alert_rules.is_none() && parsed.themes.is_none());

    // The preview writes nothing.
    let dev_b = b.core.db.device_id().to_string();
    let preview = b
        .core
        .db
        .read(|c| bundle::import_in(c, &dev_b, &parsed, false, false))
        .unwrap();
    assert!(!preview.applied);
    assert_eq!(preview.entries["sounds.volume"], ImportOutcome::Updated);
    assert_eq!(preview.entries["groupings/Home jobs"], ImportOutcome::New);
    assert_eq!(preview.entries["groupings/Todo"], ImportOutcome::Unchanged);
    assert_eq!(
        mode_core::settings::get(&b.core.db, "sounds.volume").unwrap(),
        json!(60)
    );

    // Apply; the view shape is `{path, applied, entries}`.
    let (p, d) = (parsed.clone(), dev_b.clone());
    let applied = b
        .core
        .db
        .write(move |c| bundle::import_in(c, &d, &p, false, true))
        .unwrap();
    let view = serde_json::to_value(crate::commands_settings::ImportView {
        path: Some("/x/mode-export.json".into()),
        applied: applied.applied,
        entries: applied.entries.clone(),
    })
    .unwrap();
    assert_eq!(view["applied"], true);
    assert_eq!(view["entries"]["groupings/Home jobs"]["outcome"], "new");
    assert_eq!(
        mode_core::settings::get(&b.core.db, "sounds.volume").unwrap(),
        json!(80)
    );
    let id = g.id.clone();
    let row = b
        .core
        .db
        .read(move |c| mode_core::repo::get_by_id::<mode_core::repo::Grouping>(c, &id))
        .unwrap()
        .expect("imported");
    assert_eq!(row.name, "Home jobs");
    assert_eq!(row.device_id, dev_b, "stamped by this device");

    // Again: unchanged. A row edited here since is kept.
    let (p, d) = (parsed.clone(), dev_b.clone());
    let again = b
        .core
        .db
        .read(move |c| bundle::import_in(c, &d, &p, false, false))
        .unwrap();
    assert_eq!(
        again.entries["groupings/Home jobs"],
        ImportOutcome::Unchanged
    );
    let mut edited = row.clone();
    edited.name = "Home jobs (edited)".into();
    let d = dev_b.clone();
    b.core
        .db
        .write(move |c| {
            mode_core::repo::save(c, &d, &mut edited)?;
            let mut twice =
                mode_core::repo::get_by_id::<mode_core::repo::Grouping>(c, &edited.id)?.unwrap();
            mode_core::repo::save(c, &d, &mut twice)
        })
        .unwrap();
    let (p, d) = (parsed.clone(), dev_b.clone());
    let stale = b
        .core
        .db
        .read(move |c| bundle::import_in(c, &d, &p, false, false))
        .unwrap();
    assert!(matches!(
        &stale.entries["groupings/Home jobs"],
        ImportOutcome::Skipped(r) if r.contains("changed here")
    ));
    // A newer export wins.
    g.name = "Home jobs v2".into();
    g.rev = 10;
    let mut newer = parsed.clone();
    newer.groupings = Some(vec![g]);
    let d = dev_b.clone();
    let r2 = b
        .core
        .db
        .write(move |c| bundle::import_in(c, &d, &newer, false, true))
        .unwrap();
    assert_eq!(r2.entries["groupings/Home jobs v2"], ImportOutcome::Updated);
    assert!(
        bundle::export_in(&rusqlite::Connection::open_in_memory().unwrap(), &[], false).is_err()
    );
}

#[test]
fn capture_and_templates_folders_must_live_in_the_vault() {
    use crate::commands_settings::{folder_in_vault, set_note_folders};

    let dir = tempfile::tempdir().unwrap();
    let core = core_with(dir.path());
    let vault = std::fs::canonicalize(dir.path()).unwrap().join("vault");
    for d in ["Inbox", "Templates", "Projects/Mode", ".hidden"] {
        std::fs::create_dir_all(vault.join(d)).unwrap();
    }
    assert_eq!(folder_in_vault(&vault, "Inbox").unwrap(), "Inbox");
    assert_eq!(
        folder_in_vault(&vault, "Projects/Mode/").unwrap(),
        "Projects/Mode"
    );
    assert_eq!(
        folder_in_vault(&vault, &vault.join("Templates").to_string_lossy()).unwrap(),
        "Templates"
    );
    assert_eq!(
        folder_in_vault(&vault, "Nope").unwrap_err().code,
        "not_found"
    );
    assert_eq!(
        folder_in_vault(&vault, ".hidden").unwrap_err().code,
        "invalid"
    );
    assert_eq!(folder_in_vault(&vault, "").unwrap_err().code, "invalid");
    assert_eq!(
        folder_in_vault(&vault, &dir.path().to_string_lossy())
            .unwrap_err()
            .code,
        "invalid",
        "outside the vault"
    );

    let notes = Notes::new(
        Arc::clone(&core.db),
        NotesOptions {
            root: Some(vault.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    notes.init().unwrap();
    let list = set_note_folders(&notes, Some("Inbox".into()), Some("Templates".into())).unwrap();
    let kind_of = |l: &mode_core::notes::views::FoldersList, path: &str| {
        let v = serde_json::to_value(l).unwrap();
        v["folders"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["path"] == path)
            .map(|f| f["kind"].as_str().unwrap().to_string())
    };
    assert_eq!(kind_of(&list, "Inbox").as_deref(), Some("capture"));
    assert_eq!(kind_of(&list, "Templates").as_deref(), Some("templates"));
    assert_ne!(
        kind_of(&list, "Unsorted").as_deref(),
        Some("capture"),
        "one capture folder"
    );
    // A capture lands in the new folder.
    let cap = api::note_capture(
        &notes,
        serde_json::from_value(json!({"text": "Water the plants"})).unwrap(),
    )
    .unwrap();
    assert!(cap.path.starts_with("Inbox/"), "{}", cap.path);
    // Clearing the templates folder; a folder made after the last scan is found.
    std::fs::create_dir_all(vault.join("Later")).unwrap();
    let list = set_note_folders(&notes, Some("Later".into()), Some(String::new())).unwrap();
    assert_eq!(kind_of(&list, "Later").as_deref(), Some("capture"));
    assert_eq!(kind_of(&list, "Templates").as_deref(), Some("normal"));
    assert_eq!(
        set_note_folders(&notes, None, None).unwrap_err().code,
        "invalid"
    );
    assert_eq!(
        set_note_folders(&notes, Some("Inbox".into()), Some("Inbox".into()))
            .unwrap_err()
            .code,
        "invalid"
    );
}

#[test]
fn vault_inspection_and_the_profile_picture() {
    use crate::commands_settings::{count_md_files, store_picture};

    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    for (p, body) in [
        ("a.md", "x"),
        ("sub/b.MD", "x"),
        ("sub/c.txt", "x"),
        (".mode/d.md", "x"),
        ("node_modules/e.md", "x"),
    ] {
        let f = vault.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, body).unwrap();
    }
    assert_eq!(count_md_files(&vault), 2);
    assert_eq!(count_md_files(&dir.path().join("missing")), 0);

    let data = dir.path().join("data");
    let png = dir.path().join("me.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n....").unwrap();
    let first = store_picture(&data, &png).unwrap();
    assert!(first.starts_with(data.join("profile")));
    assert_eq!(first.extension().unwrap(), "png");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = store_picture(&data, &png).unwrap();
    assert!(
        second.is_file() && !first.exists(),
        "the previous picture is removed"
    );
    let svg = dir.path().join("x.png");
    std::fs::write(&svg, "<svg/>").unwrap();
    assert_eq!(store_picture(&data, &svg).unwrap_err().code, "invalid");
}

#[test]
fn delete_all_keeps_logs_relay_and_database_files() {
    use crate::commands_settings::clear_data_dir;

    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    for f in [
        "mode.db",
        "mode.db-wal",
        "secrets-dev.json",
        "logs/mode.log",
        "bin/mode-hook",
        "backups/mode-backup-1.db",
        "profile/picture-1.png",
        "exports/mode-export.json",
    ] {
        let p = d.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }
    let removed = clear_data_dir(d).unwrap();
    assert_eq!(
        removed,
        ["backups", "exports", "profile", "secrets-dev.json"]
    );
    for kept in ["mode.db", "mode.db-wal", "logs/mode.log", "bin/mode-hook"] {
        assert!(d.join(kept).exists(), "{kept} kept");
    }
    assert!(clear_data_dir(&d.join("missing")).unwrap().is_empty());
}

// --------------------------------------------- package 9: diagnostics

#[test]
fn diagnostics_have_the_contract_shape() {
    let rig = Rig::new();
    rig.core.record_frame(crate::events::recent_frame(
        "PreToolUse",
        &mode_core::db::now_rfc3339(),
    ));
    rig.core.db.backup_now().unwrap();
    let d = crate::commands_diag::gather(&rig.core).unwrap();
    let v = serde_json::to_value(&d).unwrap();
    for k in [
        "version",
        "dev_build",
        "data_dir",
        "db_path",
        "endpoint",
        "ipc_listening",
        "hook_frames_received",
    ] {
        assert!(v["core"].get(k).is_some(), "core.{k}");
    }
    for k in ["found", "path", "version"] {
        assert!(v["claude"].get(k).is_some(), "claude.{k}");
    }
    assert!(v["relay"].get("path").is_some() && v["relay"].get("version").is_some());
    assert!(v["hooks"].as_array().unwrap().is_empty());
    assert_eq!(v["ipc"]["recent"][0]["event"], "PreToolUse");
    assert!(v["ipc"]["recent"][0]["latency_ms"].as_u64().unwrap() < 60_000);
    assert_eq!(v["ipc"]["last_event_at"], v["ipc"]["recent"][0]["at"]);
    assert!(v["ipc"].get("listening").is_some() && v["ipc"].get("endpoint").is_some());
    for k in [
        "vault_path",
        "files_indexed",
        "last_full_scan_at",
        "pending_touchups",
    ] {
        assert!(v["index"].get(k).is_some(), "index.{k}");
    }
    let db = &v["database"];
    assert!(db["size_bytes"].as_u64().unwrap() > 0);
    assert!(db.get("wal_bytes").is_some() && db.get("last_sweep_at").is_some());
    let tables = db["tables"].as_array().unwrap();
    assert!(
        tables
            .iter()
            .any(|t| t["name"] == "sessions" && t["rows"] == 0)
    );
    assert!(
        !tables
            .iter()
            .any(|t| t["name"].as_str().unwrap().contains("_fts"))
    );
    assert_eq!(db["backups"].as_array().unwrap().len(), 1);
    for k in ["path", "size_bytes", "created_at"] {
        assert!(db["backups"][0].get(k).is_some(), "backups[].{k}");
    }
    assert_eq!(v["sessions_tracked"], 0);
    assert_eq!(v["sessions_today"], 0);
    assert_eq!(v["secrets_store"], "file");
    for _ in 0..25 {
        rig.core
            .record_frame(crate::events::recent_frame("Stop", "garbage"));
    }
    let q = CoreState::lock(&rig.core.recent_frames);
    assert_eq!(q.len(), crate::state::RECENT_FRAMES);
    assert!(
        q[0].latency_ms.is_none(),
        "an unreadable stamp has no latency"
    );
}

#[test]
fn sweep_report_counts_rows_per_table() {
    let rig = Rig::new();
    let report = mode_core::retention::run(&rig.core.db, true).unwrap();
    let v = serde_json::to_value(crate::commands_diag::SweepResult {
        deleted: crate::commands_diag::deleted_by_table(&report),
        report,
    })
    .unwrap();
    assert_eq!(v["dry_run"], true);
    assert!(v["deleted"].is_object());
    assert!(
        v.get("session_events_deleted").is_some(),
        "the report is flattened"
    );
}

// --------------------------------------------------- package 9: stats

#[test]
fn stats_days_and_summary_have_the_contract_shapes() {
    use crate::commands_diag::{stats_days_in, stats_summary_in};

    let mut rig = Rig::new();
    let cwd = rig.root.join("proj");
    std::fs::create_dir_all(&cwd).unwrap();
    let transcript = rig.root.join("t.jsonl");
    let mut start = Rig::event("SessionStart", "claude-s", &cwd, &transcript);
    start.set("source", "startup");
    rig.send(start);
    rig.send(Rig::event("Stop", "claude-s", &cwd, &transcript));
    let now = mode_core::db::now_rfc3339();
    rig.core
        .db
        .write(move |c| {
            for (id, source, model, n) in [
                ("u1", "claude_code", "model-a", 1_000),
                ("u2", "claude_api", "model-b", 10),
            ] {
                c.execute(
                    "INSERT INTO usage_samples (id, sampled_at, source, purpose, model, input_tokens, output_tokens,
                                                cache_read_tokens, cache_write_tokens, created_at)
                     VALUES (?1, ?2, ?3, 'session', ?4, ?5, 0, 0, 0, ?2)",
                    (id, now.as_str(), source, model, n),
                )?;
            }
            Ok(())
        })
        .unwrap();
    let today = rig.core.db.read(|c| Ok(Clock::current(c)?.today)).unwrap();
    let from = crate::commands_diag::date_of(queries::day_number(&today).unwrap() - 6);

    let days = rig
        .core
        .db
        .read(|c| Ok(stats_days_in(c, &from, &today)))
        .unwrap()
        .unwrap();
    assert_eq!(days.len(), 7, "every day of the window");
    let v = serde_json::to_value(&days).unwrap();
    assert_eq!(v[6]["date"], today);
    assert_eq!(v[6]["sessions"], 1);
    assert_eq!(v[6]["tokens"], 1_000, "Claude Code tokens only");
    assert_eq!(v[0]["sessions"], 0);
    for k in ["date", "sessions", "files", "tokens"] {
        assert!(v[0].get(k).is_some(), "StatsDay.{k}");
    }

    let s = rig
        .core
        .db
        .read(|c| Ok(stats_summary_in(c, &from, &today)))
        .unwrap()
        .unwrap();
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["from"], from);
    assert_eq!(v["tokens"], 1_010, "every source");
    assert_eq!(v["sessions"], 1);
    assert_eq!(v["days"].as_array().unwrap().len(), 7);
    let split = v["days"][6]["split"].as_array().unwrap();
    assert_eq!(split.len(), 2);
    for k in ["model", "source", "tokens"] {
        assert!(split[0].get(k).is_some(), "split.{k}");
    }
    let by_state = v["by_state"].as_object().unwrap();
    assert_eq!(
        by_state.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "waiting",
            "working",
            "failed",
            "finished",
            "idle",
            "untracked"
        ])
    );
    assert_eq!(
        v["by_state"]["finished"], 1,
        "an unread session is finished"
    );
    assert!(!v.to_string().contains("cost"), "tokens only");
    let bad = rig
        .core
        .db
        .read(|c| Ok(stats_days_in(c, &today, &from)))
        .unwrap();
    assert_eq!(bad.unwrap_err().code, "invalid");
}

// -------------------------------------------- package 10: connectors

mod connectors {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use mode_core::connectors::Connectors;
    use mode_core::connectors::ai::fixture::FixtureBackend;
    use mode_core::connectors::ai::{AiBackend, BackendFactory, BackendKind};
    use mode_core::connectors::api::{self, AccountAddStartArgs, IdArgs, StateArgs};
    use mode_core::connectors::http::{FixtureClient, HttpClient};
    use mode_core::connectors::service::{AlertCommand, ConnectorsOptions};
    use mode_core::db::parse_timestamp;
    use mode_core::notes::ctx::TzMode;
    use mode_core::push::Event;
    use mode_core::repo::{self, CalendarEvent, Note};
    use mode_wire::HookEvent;
    use serde_json::{Value, json};

    use super::{Rig, block_on, core_with};
    use crate::alerts::{self, SessionSignal, SinkStep};
    use crate::cmd_error::CmdError;
    use crate::commands_connectors::{self, with_connectors};
    use crate::state::CoreState;
    use crate::{commands_ai, health, os_signals};

    const NOW: &str = "2026-10-21T12:00:00.000Z";
    const ME: &str = "me@example.com";

    fn fixture(path: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/connectors")
            .join(path)
    }

    struct Factory {
        claude: Arc<FixtureBackend>,
        local: Arc<FixtureBackend>,
    }

    impl BackendFactory for Factory {
        fn claude(&self, _api_key: String) -> Arc<dyn AiBackend> {
            Arc::clone(&self.claude) as Arc<dyn AiBackend>
        }
        fn local(&self, _k: BackendKind, _url: &str, _t: Option<String>) -> Arc<dyn AiBackend> {
            Arc::clone(&self.local) as Arc<dyn AiBackend>
        }
    }

    struct ConnRig {
        _dir: tempfile::TempDir,
        core: Arc<CoreState>,
        c: Arc<Connectors>,
        http: Arc<FixtureClient>,
        ai: Arc<Factory>,
        pushes: Arc<Mutex<Vec<Event>>>,
        alerts: Arc<Mutex<Vec<AlertCommand>>>,
    }

    impl ConnRig {
        /// A temp database, the Linux dev file keyring under the data dir,
        /// the fixture HTTP client and AI backends; `fixed` pins the clock to
        /// the fixtures' Wed 2026-10-21 12:00 UTC (the device at UTC+2).
        fn new(fixed: bool) -> ConnRig {
            let dir = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(dir.path()).unwrap();
            let core = core_with(&root);
            let http = Arc::new(FixtureClient::new());
            let ai = Arc::new(Factory {
                claude: Arc::new(FixtureBackend::canned(BackendKind::ClaudeApi)),
                local: Arc::new(FixtureBackend::canned(BackendKind::Ollama)),
            });
            let now = parse_timestamp(NOW).unwrap();
            let c = Connectors::with_options(
                Arc::clone(&core.db),
                core.keyring(),
                Arc::clone(&http) as Arc<dyn HttpClient>,
                ConnectorsOptions {
                    clock: fixed.then(|| {
                        Arc::new(move || now) as Arc<dyn Fn() -> time::OffsetDateTime + Send + Sync>
                    }),
                    tz: TzMode::Fixed(120),
                    ai_factory: Some(Arc::clone(&ai) as Arc<dyn BackendFactory>),
                },
            );
            let pushes: Arc<Mutex<Vec<Event>>> = Arc::default();
            let p = Arc::clone(&pushes);
            c.set_publisher(move |e| p.lock().unwrap().push(e));
            let alerts: Arc<Mutex<Vec<AlertCommand>>> = Arc::default();
            let a = Arc::clone(&alerts);
            c.set_alert_sink(move |cmd| a.lock().unwrap().push(cmd.clone()));
            core.connectors.set(Arc::clone(&c)).unwrap();
            core.session_watch.prime(&core.db);
            ConnRig {
                _dir: dir,
                core,
                c,
                http,
                ai,
                pushes,
                alerts,
            }
        }

        fn run<T: Send + 'static>(
            &self,
            f: impl FnOnce(&Connectors) -> api::ApiResult<T> + Send + 'static,
        ) -> Result<T, CmdError> {
            block_on(with_connectors(&self.core, f))
        }

        fn take_pushes(&self) -> Vec<Event> {
            std::mem::take(&mut *self.pushes.lock().unwrap())
        }

        fn take_alerts(&self) -> Vec<AlertCommand> {
            std::mem::take(&mut *self.alerts.lock().unwrap())
        }

        fn setting(&self, key: &str, v: Value) {
            mode_core::settings::set(&self.core.db, key, &v).unwrap();
        }

        fn exec(&self, sql: &str) {
            let sql = sql.to_string();
            self.core
                .db
                .write(move |c| {
                    c.execute_batch(&sql)?;
                    Ok(())
                })
                .unwrap();
        }

        fn count(&self, sql: &str) -> i64 {
            let sql = sql.to_string();
            self.core
                .db
                .read(move |c| Ok(c.query_row(&sql, [], |r| r.get(0))?))
                .unwrap()
        }

        /// Connect Google through the real loopback with the recorded
        /// token exchange (06 §4), then run the Gmail initial sync.
        ///
        /// The loopback gets a port from the OS (no fixed list, so parallel
        /// test processes never race for one) and the "browser" calls the
        /// port the redirect URI names, as a real browser does. A watchdog
        /// cancels the flow after [`LOOPBACK_WAIT`], so a callback that
        /// never arrives fails the test instead of holding it for the
        /// flow's five-minute timeout.
        fn connect_google(&self) -> api::SourceRow {
            self.setting("connectors.oauth_ports", json!([]));
            let status = self
                .run(|c| {
                    api::oauth_client_set(
                        c,
                        api::OAuthClientArgs {
                            provider: "google".into(),
                            client_id: "fixture-client.apps.googleusercontent.com".into(),
                            client_secret: Some("fixture-secret".into()),
                            project_id: None,
                            team_id: None,
                        },
                    )
                })
                .unwrap();
            assert!(status.stored);
            let v = commands_ai::secret_status_of(&self.core, "oauth_client:google").unwrap();
            assert!(
                v.stored && v.last4.is_none(),
                "an OAuth client shows stored only"
            );
            let start = self
                .run(|c| {
                    api::account_add_start(
                        c,
                        AccountAddStartArgs {
                            provider: "google".into(),
                            options: None,
                        },
                    )
                })
                .unwrap();
            assert!(start.auth_url.starts_with("https://accounts.google.com/"));
            assert!(commands_connectors::external_url_ok(&start.auth_url));
            assert!(!start.auth_url.contains("fixture-secret"));
            self.http.load_more(&fixture("gmail/connect.json")).unwrap();
            let port = redirect_port(&start.redirect_uri);
            let state = start.state.clone();
            let browser = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                get(port, &format!("/callback?code=fixture-code&state={state}"))
            });
            let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let watchdog = {
                let (c, state, done) =
                    (Arc::clone(&self.c), start.state.clone(), Arc::clone(&done));
                std::thread::spawn(move || {
                    let end = std::time::Instant::now() + LOOPBACK_WAIT;
                    while std::time::Instant::now() < end {
                        if done.load(std::sync::atomic::Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    let _ = api::account_add_cancel(&c, StateArgs { state });
                })
            };
            let state = start.state.clone();
            // The command runs the wait on the blocking pool.
            let finished = self.run(move |c| api::account_add_finish(c, StateArgs { state }));
            done.store(true, std::sync::atomic::Ordering::SeqCst);
            watchdog.join().unwrap();
            let row = finished.unwrap_or_else(|e| {
                panic!("the loopback callback did not arrive on port {port}: {e:?}")
            });
            assert_eq!(browser.join().unwrap().0, 200);
            assert_eq!(
                (row.provider.as_str(), row.email.as_deref()),
                ("google", Some(ME))
            );
            let account = row.id.clone();
            let sources = self
                .run(move |c| {
                    api::sources_list(
                        c,
                        api::SourcesListArgs {
                            account_id: Some(account),
                        },
                    )
                })
                .unwrap();
            let inbox = sources
                .into_iter()
                .find(|s| s.kind == "gmail_inbox")
                .expect("the inbox source");
            self.http
                .load_more(&fixture("gmail/initial_sync.json"))
                .unwrap();
            let out = self.c.sync_source(&row.id, &inbox.id).unwrap();
            assert_eq!(out.inserted.len(), 6, "{out:?}");
            inbox
        }
    }

    /// How long a test waits for its own loopback callback.
    const LOOPBACK_WAIT: Duration = Duration::from_secs(20);

    /// The port of `http://127.0.0.1:<port>/callback`.
    fn redirect_port(uri: &str) -> u16 {
        uri.strip_prefix("http://127.0.0.1:")
            .and_then(|r| r.split('/').next())
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("a loopback redirect URI: {uri}"))
    }

    fn get(port: u16, path: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        let code = out
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        (code, out)
    }

    #[test]
    fn every_api_function_is_a_command() {
        let api_rs = include_str!("../../mode-core/src/connectors/api.rs");
        let handled = super::handler_commands();
        for line in api_rs.lines() {
            let Some(rest) = line.strip_prefix("pub fn ") else {
                continue;
            };
            let name = rest.split(['(', '<']).next().unwrap();
            if matches!(
                name,
                "connector_error" | "oauth_error" | "ai_test_call_with"
            ) {
                continue;
            }
            assert!(
                handled.iter().any(|h| h == name),
                "connectors::api::{name} has no command"
            );
        }
    }

    #[test]
    fn accounts_connect_sync_and_the_integrations_cards() {
        let rig = ConnRig::new(true);
        // Before any account: empty lists, "not connected" cards, the pushes empty.
        assert!(rig.run(api::accounts_list).unwrap().is_empty());
        let st = rig
            .run(commands_connectors::connectors_status_view)
            .unwrap();
        assert_eq!(st.gmail.state, "not_connected");
        let payload = crate::payloads(Arc::clone(&rig.core), None, Some(Arc::clone(&rig.c)));
        assert_eq!(
            payload(Event::Accounts).unwrap(),
            json!({"accounts": [], "offline": false})
        );
        assert!(
            payload(Event::Agenda).is_some(),
            "agenda.changed is pushed empty"
        );

        rig.connect_google();
        let accounts = rig.run(api::accounts_list).unwrap();
        assert_eq!(accounts.len(), 1);
        let id = accounts[0].id.clone();
        // connectors_status: the core's report plus the contract's AccountSummary fields.
        let st = rig
            .run(commands_connectors::connectors_status_view)
            .unwrap();
        let v = serde_json::to_value(&st).unwrap();
        assert_eq!(v["shipped"], true);
        assert_eq!(v["gmail"]["state"], "connected");
        let a = &v["accounts"][0];
        assert_eq!(a["id"], id.as_str());
        assert_eq!(a["account_id"], id.as_str());
        assert_eq!(a["services"], json!(["gmail", "calendar"]));
        assert!(a.get("status_detail").is_some() && a.get("health").is_some());
        assert_eq!(a["status"], "active");

        // Pause / resume / retry / reauthorize.
        let i = id.clone();
        let row = rig
            .run(move |c| api::account_pause(c, IdArgs { id: i }))
            .unwrap();
        assert_eq!(row.status, "paused");
        let pushes = rig.take_pushes();
        assert!(pushes.contains(&Event::Accounts) && pushes.contains(&Event::Health));
        let h = health::compute(&rig.core);
        assert_eq!(h.paused_accounts, vec![ME.to_string()]);
        let i = id.clone();
        let row = rig
            .run(move |c| api::account_resume(c, IdArgs { id: i }))
            .unwrap();
        assert_eq!(row.status, "active");
        let i = id.clone();
        let re = rig
            .run(move |c| api::account_reauthorise(c, IdArgs { id: i }))
            .unwrap();
        assert!(re.auth_url.contains("login_hint="), "{}", re.auth_url);
        let s = re.state.clone();
        assert!(
            rig.run(move |c| api::account_add_cancel(c, StateArgs { state: s }))
                .unwrap()
        );

        // Sources, the Slack manifest, the Claude Code card.
        let i = id.clone();
        let sources = rig
            .run(move |c| {
                api::sources_list(
                    c,
                    api::SourcesListArgs {
                        account_id: Some(i),
                    },
                )
            })
            .unwrap();
        let cal = sources
            .iter()
            .find(|s| s.kind == "calendar")
            .unwrap()
            .id
            .clone();
        let row = rig
            .run(move |c| {
                api::source_set(
                    c,
                    api::SourceSetArgs {
                        id: cal,
                        enabled: Some(false),
                        vip: None,
                        poll_seconds: Some(Some(600)),
                    },
                )
            })
            .unwrap();
        assert!(!row.enabled && row.poll_seconds == Some(600));
        let manifest = rig
            .run(|c| api::slack_manifest(c, api::ManifestArgs { writeback: true }))
            .unwrap();
        assert!(manifest.contains("redirect_urls"), "{manifest}");
        let card = rig
            .run(|c| api::claude_code_card(c, api::ClaudeCodeCardArgs { binary_found: true }))
            .unwrap();
        assert!(!card.state.is_empty());

        // Remove: the rows and the keyring entry go; health and the views are pushed.
        rig.take_pushes();
        let i = id.clone();
        rig.run(move |c| api::account_remove(c, IdArgs { id: i }))
            .unwrap();
        assert!(rig.run(api::accounts_list).unwrap().is_empty());
        assert!(!rig.core.keyring().present(&format!("account:{id}")));
        let pushes = rig.take_pushes();
        for e in [Event::Accounts, Event::Inbox, Event::Agenda, Event::Health] {
            assert!(pushes.contains(&e), "{e:?} after remove");
        }
        assert_eq!(rig.count("SELECT COUNT(*) FROM messages"), 0);
        // A command before the service exists answers `unavailable`.
        let dir = tempfile::tempdir().unwrap();
        let bare = core_with(dir.path());
        let err = block_on(with_connectors(&bare, api::accounts_list)).unwrap_err();
        assert_eq!(err.code, "unavailable");
    }

    #[test]
    fn inbox_triage_snooze_vips_and_rules() {
        let rig = ConnRig::new(true);
        rig.connect_google();
        let list = rig
            .run(|c| api::inbox_list(c, api::InboxListArgs::default()))
            .unwrap();
        let v = serde_json::to_value(&list).unwrap();
        let rows = v["rows"].as_array().unwrap();
        assert!(!rows.is_empty(), "{v}");
        let thread = rows[0]["thread_id"].as_str().unwrap().to_string();
        let t = thread.clone();
        let detail = rig
            .run(move |c| api::thread_get(c, api::ThreadGetArgs { thread_id: t }))
            .unwrap();
        let d = serde_json::to_value(&detail).unwrap();
        let message = d["messages"][0]["id"].as_str().unwrap().to_string();
        // A message id resolves to its thread (`inbox/message/<id>`).
        let m = message.clone();
        let by_message = rig
            .run(move |c| api::thread_get(c, api::ThreadGetArgs { thread_id: m }))
            .unwrap();
        assert_eq!(by_message.id, thread);
        let missing = rig
            .run(|c| {
                api::thread_get(
                    c,
                    api::ThreadGetArgs {
                        thread_id: "nope".into(),
                    },
                )
            })
            .unwrap_err();
        assert_eq!(missing.code, "not_found");

        // Mark read: local first, a queued write-back, the views pushed.
        rig.take_pushes();
        let m = message.clone();
        let r = rig
            .run(move |c| {
                api::message_mark_read(
                    c,
                    api::MarkReadArgs {
                        id: m,
                        kind: None,
                        read: true,
                    },
                )
            })
            .unwrap();
        let r = serde_json::to_value(r).unwrap();
        assert_eq!(r["messages"].as_array().unwrap().len(), 1, "{r}");
        assert!(rig.count("SELECT COUNT(*) FROM connector_writebacks") >= 1);
        assert!(rig.take_pushes().contains(&Event::Inbox));

        // Snooze until tomorrow, then cancel; archive and unarchive.
        let t = thread.clone();
        let s = rig
            .run(move |c| {
                api::thread_snooze(
                    c,
                    api::SnoozeArgs {
                        id: t,
                        kind: None,
                        preset: Some(mode_core::urgency::snooze::Preset::Tomorrow),
                        until: None,
                        wake_on_reply: None,
                    },
                )
            })
            .unwrap();
        assert!(s.until.as_str() > NOW && s.targets >= 1);
        let t = thread.clone();
        assert!(
            rig.run(move |c| api::thread_unsnooze(c, api::UnsnoozeArgs { id: t, kind: None }))
                .unwrap()
                >= 1
        );
        let t = thread.clone();
        rig.run(move |c| {
            api::thread_archive(
                c,
                api::ArchiveArgs {
                    id: t,
                    kind: None,
                    archived: true,
                },
            )
        })
        .unwrap();
        let archived = rig
            .run(|c| {
                api::inbox_list(
                    c,
                    api::InboxListArgs {
                        filter: mode_core::urgency::route::InboxFilter {
                            show_archived: true,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
            })
            .unwrap();
        assert!(
            serde_json::to_value(&archived).unwrap()["rows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["thread_id"] == thread.as_str() && r["is_archived"] == true)
        );

        // Open in source: the Gmail web link (http / https only for the opener).
        let m = message.clone();
        let link = rig
            .run(move |c| api::open_in_source(c, IdArgs { id: m }))
            .unwrap();
        assert!(
            link.url.starts_with("https://mail.google.com/"),
            "{}",
            link.url
        );
        assert!(commands_connectors::external_url_ok(&link.url));

        // The badge override, the body on demand (cached), the counters.
        let m = message.clone();
        rig.run(move |c| {
            api::message_set_tier(
                c,
                api::SetTierArgs {
                    id: m,
                    tier: mode_core::urgency::Tier::Low,
                },
            )
        })
        .unwrap();
        let diag = rig.run(api::urgency_diagnostics).unwrap();
        assert!(diag.writebacks_queued >= 1);

        // Important people by the sender of a message.
        let m = message.clone();
        let added = rig
            .run(move |c| {
                api::vip_add(
                    c,
                    mode_core::urgency::vips::VipAdd {
                        message_id: Some(m),
                        ..Default::default()
                    },
                )
            })
            .unwrap();
        let vips = rig.run(api::vips_list).unwrap();
        assert_eq!(vips.len(), 1);
        let contact = serde_json::to_value(&added).unwrap()["contact_id"]
            .as_str()
            .unwrap()
            .to_string();
        rig.run(move |c| {
            api::vip_remove(
                c,
                api::ContactArgs {
                    contact_id: contact,
                },
            )
        })
        .unwrap();
        assert!(rig.run(api::vips_list).unwrap().is_empty());

        // Keyword rules: seeded examples, validated writes; Re-score runs.
        let rules = rig.run(api::keyword_rules_list).unwrap();
        assert!(!rules.is_empty());
        let mut bad = rules.clone();
        bad[1].pattern = "(".into(); // the second premade rule is a regex
        let err = rig
            .run(move |c| api::keyword_rules_set(c, api::KeywordRulesSetArgs { rules: bad }))
            .unwrap_err();
        assert_eq!(err.code, "invalid");
        rig.run(api::urgency_rescore).unwrap();
        // inbox.changed carries the rows the overlay renders.
        let payload = crate::payloads(Arc::clone(&rig.core), None, Some(Arc::clone(&rig.c)));
        let inbox = payload(Event::Inbox).unwrap();
        assert!(inbox.get("rows").is_some(), "{inbox}");
    }

    #[test]
    fn calendar_agenda_join_and_prepare() {
        let rig = ConnRig::new(true);
        rig.connect_google();
        let account = rig.run(api::accounts_list).unwrap()[0].id.clone();
        let ev = CalendarEvent {
            id: "ev-1".into(),
            account_id: account,
            calendar_id: ME.into(),
            calendar_name: Some(ME.into()),
            external_id: "gcal-ev-1".into(),
            title: "Design review".into(),
            starts_at: "2026-10-21T12:30:00.000Z".into(),
            ends_at: "2026-10-21T13:00:00.000Z".into(),
            status: "confirmed".into(),
            my_response: Some("accepted".into()),
            attendees_json: "[]".into(),
            join_url: Some("https://meet.google.com/abc-defg-hij".into()),
            conference: Some("meet".into()),
            html_link: Some("https://www.google.com/calendar/event?eid=x".into()),
            synced_at: NOW.into(),
            created_at: NOW.into(),
            updated_at: NOW.into(),
            ..Default::default()
        };
        rig.core
            .db
            .write(move |c| {
                repo::insert(c, &ev)?;
                Ok(())
            })
            .unwrap();
        let rows = rig
            .run(|c| {
                api::agenda_list(
                    c,
                    api::AgendaArgs {
                        from: "2026-10-21T00:00:00.000Z".into(),
                        to: "2026-10-22T00:00:00.000Z".into(),
                    },
                )
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        let url = rig
            .run(|c| api::event_join(c, IdArgs { id: "ev-1".into() }))
            .unwrap();
        assert!(commands_connectors::external_url_ok(&url) && url.contains("meet.google.com"));
        let link = rig
            .run(|c| api::open_in_source(c, IdArgs { id: "ev-1".into() }))
            .unwrap();
        assert!(link.url.contains("calendar"));
        let payload = crate::payloads(Arc::clone(&rig.core), None, Some(Arc::clone(&rig.c)));
        let agenda = payload(Event::Agenda).unwrap();
        assert_eq!(agenda["rows"].as_array().unwrap().len(), 1, "{agenda}");
        // Prepare notes needs the notes service: `unavailable` without it.
        assert_eq!(rig.core.notes().unwrap_err().code, "unavailable");
    }

    #[test]
    fn alerts_quiet_hours_and_the_sink_mapping() {
        let rig = ConnRig::new(false);
        // The status item's pause: quiet.changed `paused`, pushed.
        rig.take_pushes();
        let q = rig
            .run(|c| {
                api::quiet_set(
                    c,
                    api::QuietSetArgs {
                        pause_minutes: Some(60),
                        ..Default::default()
                    },
                )
            })
            .unwrap();
        assert!(q.paused);
        assert!(rig.take_pushes().contains(&Event::Quiet));
        let payload = crate::payloads(Arc::clone(&rig.core), None, Some(Arc::clone(&rig.c)));
        assert_eq!(payload(Event::Quiet).unwrap()["paused"], true);
        let q = rig
            .run(|c| {
                api::quiet_set(
                    c,
                    api::QuietSetArgs {
                        pause_minutes: Some(0),
                        ..Default::default()
                    },
                )
            })
            .unwrap();
        assert!(!q.paused);
        let err = rig
            .run(|c| {
                api::quiet_set(
                    c,
                    api::QuietSetArgs {
                        quiet_hours: Some(mode_core::urgency::alerts::QuietHours {
                            enabled: true,
                            from: "25:00".into(),
                            to: "08:00".into(),
                        }),
                        ..Default::default()
                    },
                )
            })
            .unwrap_err();
        assert_eq!(err.code, "invalid");

        // Settings → Alerts → Test on "Session needs you" with sound and an
        // OS notification: the sink gets one Fired; the app plays the cue on
        // the overlay and posts one silent notification with Allow / Deny.
        rig.setting("sounds.enabled", json!(true));
        rig.exec(
            "UPDATE alert_rules SET channels_json = '[\"overlay\",\"sound\",\"os_notification\"]'
              WHERE id = '00000000-0000-7000-8000-000000000201'",
        );
        let out = rig
            .run(|c| {
                api::alert_rule_test(
                    c,
                    api::AlertTestArgs {
                        rule_id: "00000000-0000-7000-8000-000000000201".into(),
                    },
                )
            })
            .unwrap();
        assert_eq!(out.sound.as_deref(), Some("waiting"));
        let fired = rig.take_alerts();
        assert_eq!(fired.len(), 1);
        let steps = alerts::plan(&fired[0]);
        let SinkStep::Overlay(o) = &steps[0] else {
            panic!("{steps:?}")
        };
        assert_eq!(o.sound.as_deref(), Some("waiting"));
        let SinkStep::Post(p) = &steps[1] else {
            panic!("{steps:?}")
        };
        assert_eq!(p.action_type.as_deref(), Some("MODE_APPROVAL"));
        assert_eq!(
            p.actions,
            vec![
                ("allow".to_string(), "Allow".to_string()),
                ("deny".to_string(), "Deny".to_string())
            ]
        );
        assert_eq!(
            p.sound, None,
            "the overlay plays the cue; the OS stays silent"
        );
        assert_eq!(p.delivered.route.view, "sessions");
        assert_eq!(
            alerts::action_plan("allow", &p.delivered),
            alerts::ActionPlan::Decide {
                session_id: "test".into(),
                allow: true
            }
        );
        assert_eq!(
            rig.count("SELECT COUNT(*) FROM alert_log"),
            0,
            "a test writes nothing"
        );

        // alert_dismiss withdraws by item key.
        let ids = rig
            .run(|c| {
                api::alert_dismiss(
                    c,
                    api::AlertDismissArgs {
                        item_key: Some("gmail:<x@y>".into()),
                        target_kind: None,
                        target_id: None,
                    },
                )
            })
            .unwrap();
        assert_eq!(ids, ["gmail:<x@y>"]);
        let w = rig.take_alerts();
        assert_eq!(
            alerts::plan(&w[0]),
            vec![SinkStep::Remove {
                identifiers: ids.clone(),
                ids: vec![alerts::notification_id("gmail:<x@y>")],
            }]
        );
        assert!(
            rig.run(|c| api::alerts_recent(c, api::AlertsRecentArgs { limit: Some(5) }))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn session_transitions_raise_and_withdraw_session_alerts() {
        let mut sessions = Rig::new();
        let core = Arc::clone(&sessions.core);
        let cwd = sessions.root.join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        let tr = sessions.root.join("t.jsonl");
        let mut start = Rig::event("SessionStart", "claude-w", &cwd, &tr);
        start.set("source", "startup");
        // Primed after the start: an existing session raises nothing.
        let sid = sessions.send(start);
        core.session_watch.prime(&core.db);
        assert!(core.session_watch.observe(&core.db).is_empty());

        let mut perm: HookEvent = Rig::event("PermissionRequest", "claude-w", &cwd, &tr);
        perm.set("tool_name", "Bash");
        perm.set("tool_use_id", "toolu_1");
        perm.set("tool_input", json!({"command": "npm test"}));
        sessions.send(perm);
        let sig = core.session_watch.observe(&core.db);
        assert_eq!(sig.len(), 1, "{sig:?}");
        let SessionSignal::Alert(ev) = &sig[0] else {
            panic!("{sig:?}")
        };
        let mode_core::urgency::alerts::AlertEvent::SessionWaiting(w) = ev.as_ref() else {
            panic!("{ev:?}")
        };
        assert_eq!(
            (w.session_id.as_str(), w.waiting_kind.as_str()),
            (sid.as_str(), "permission")
        );
        assert_eq!(w.tool_use_id.as_deref(), Some("toolu_1"));
        assert!(
            core.session_watch.observe(&core.db).is_empty(),
            "once per card"
        );

        // The alert pipeline on the same database: the rule fires.
        let c2 = Connectors::new(
            Arc::clone(&core.db),
            core.keyring(),
            Arc::new(FixtureClient::new()) as Arc<dyn HttpClient>,
        );
        let got: Arc<Mutex<Vec<AlertCommand>>> = Arc::default();
        let g = Arc::clone(&got);
        c2.set_alert_sink(move |cmd| g.lock().unwrap().push(cmd.clone()));
        mode_core::settings::set(&core.db, "sounds.enabled", &json!(true)).unwrap();
        core.db
            .write(|c| {
                c.execute_batch(
                    "UPDATE alert_rules SET channels_json = '[\"overlay\",\"sound\",\"os_notification\"]'
                      WHERE id IN ('00000000-0000-7000-8000-000000000201', '00000000-0000-7000-8000-000000000203')",
                )?;
                Ok(())
            })
            .unwrap();
        alerts::dispatch_session_signals(&c2, sig);
        let fired = std::mem::take(&mut *got.lock().unwrap());
        assert_eq!(fired.len(), 1, "{fired:?}");
        let AlertCommand::Fired(out) = &fired[0] else {
            panic!()
        };
        assert_eq!(out.event, "session.waiting");
        assert_eq!(out.sound.as_deref(), Some("waiting"));
        assert!(out.notification.is_some());

        // The tool ran (card answered in the terminal), then the turn ended:
        // the waiting notification is withdrawn, `session.finished` raised.
        let mut post = Rig::event("PostToolUse", "claude-w", &cwd, &tr);
        post.set("tool_name", "Bash");
        post.set("tool_use_id", "toolu_1");
        post.set("tool_input", json!({"command": "npm test"}));
        post.set("tool_response", json!({"stdout": "ok"}));
        sessions.send(post);
        let sig = core.session_watch.observe(&core.db);
        assert_eq!(sig, vec![SessionSignal::Withdraw(sid.clone())]);
        alerts::dispatch_session_signals(&c2, sig);
        let w = std::mem::take(&mut *got.lock().unwrap());
        let AlertCommand::Withdraw { identifiers } = &w[0] else {
            panic!("{w:?}")
        };
        assert_eq!(identifiers.len(), 1);
        assert!(identifiers[0].contains("toolu_1"), "{identifiers:?}");
        sessions.send(Rig::event("Stop", "claude-w", &cwd, &tr));
        let sig = core.session_watch.observe(&core.db);
        assert!(
            matches!(&sig[..], [SessionSignal::Alert(e)] if e.name() == "session.finished"),
            "{sig:?}"
        );
        // Acknowledged: the finished notification goes too.
        core.engine.acknowledge(&sid).unwrap();
        assert_eq!(
            core.session_watch.observe(&core.db),
            vec![SessionSignal::Withdraw(sid.clone())]
        );
        // Dismissed rows withdraw only when they were alertable.
        core.engine.dismiss(&sid).unwrap();
        assert!(core.session_watch.observe(&core.db).is_empty());
    }

    #[test]
    fn note_due_fires_once_a_day() {
        let rig = ConnRig::new(true);
        let note = Note {
            id: "n1".into(),
            path: "n1.md".into(),
            title: "Send the contract".into(),
            status: "todo".into(),
            urgent: true,
            when_date: Some("2026-10-21".into()),
            source_kind: "manual".into(),
            created_at: NOW.into(),
            updated_at: NOW.into(),
            device_id: "d".into(),
            ..Default::default()
        };
        rig.core
            .db
            .write(move |c| {
                repo::insert(c, &note)?;
                Ok(())
            })
            .unwrap();
        assert_eq!(alerts::note_due_once(&rig.c), 1);
        assert_eq!(
            rig.count("SELECT COUNT(*) FROM alert_log WHERE event = 'note.due'"),
            1,
            "the premade Due today rule logged it (overlay only)"
        );
        assert!(
            rig.take_alerts().is_empty(),
            "overlay only: nothing to post"
        );
        assert_eq!(alerts::note_due_once(&rig.c), 1);
        assert_eq!(
            rig.count("SELECT COUNT(*) FROM alert_log WHERE event = 'note.due'"),
            1,
            "a second firing the same day is de-duplicated"
        );
    }

    #[test]
    fn ai_backends_secrets_and_the_test_call() {
        let rig = ConnRig::new(true);
        let rows = rig.run(api::ai_backends_list).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(
            rows.iter()
                .any(|r| r.selected && r.backend == BackendKind::ClaudeApi)
        );
        // secret_set through the file keyring: never the value back.
        let key = "sk-ant-fixture-0123456789";
        let v = commands_ai::secret_set_of(&rig.core, "anthropic_api_key", Some(key)).unwrap();
        assert!(v.stored && v.updated_at.is_some());
        assert_eq!(v.name, "anthropic_api_key");
        assert_eq!(v.last4.as_deref(), Some("6789"));
        let text = serde_json::to_string(&v).unwrap();
        assert!(!text.contains(key));
        let file = rig.core.data_dir.join("secrets-dev.json");
        assert!(file.is_file(), "the Linux dev store is the file keyring");
        assert_eq!(rig.core.keyring().kind(), "file");
        assert!(
            commands_ai::secret_status_of(&rig.core, "ai:anthropic_api_key")
                .unwrap()
                .stored
        );
        assert_eq!(
            commands_ai::secret_set_of(&rig.core, "anthropic_api_key", Some("two\nlines"))
                .unwrap_err()
                .code,
            "invalid"
        );
        assert_eq!(
            commands_ai::secret_set_of(&rig.core, "oauth_client:google", Some("x"))
                .unwrap_err()
                .code,
            "invalid"
        );
        let status = rig.run(api::ai_backends_status).unwrap();
        let s = serde_json::to_value(&status).unwrap();
        assert_eq!(s["claude_api"]["key_stored"], true);
        assert_eq!(s["ollama"]["reachable"], true);
        assert_eq!(s["ollama"]["models"], json!(["fixture-model"]));

        // The test call: the selected backend (Claude API, canned reply), then a local one.
        rig.ai.claude.push_text("ok");
        let r = rig
            .run(|c| api::ai_test_call_with(c, api::AiTestArgs::default()))
            .unwrap();
        assert!(r.ok && r.latency_ms.is_some(), "{r:?}");
        assert_eq!(rig.ai.claude.requests().len(), 1);
        let r = rig
            .run(|c| {
                api::ai_test_call_with(
                    c,
                    api::AiTestArgs {
                        backend: Some(BackendKind::Ollama),
                        provider: None,
                        model: None,
                    },
                )
            })
            .unwrap();
        assert!(!r.ok, "the local fixture has no reply queued");
        assert_eq!(rig.ai.local.requests().len(), 1, "tested the local backend");
        let rows = rig.run(api::ai_backends_list).unwrap();
        assert!(
            rows.iter()
                .any(|r| r.selected && r.backend == BackendKind::ClaudeApi),
            "testing another backend stores nothing"
        );
        let h = rig
            .run(|c| {
                api::ai_backend_set(
                    c,
                    api::AiBackendSetArgs {
                        local_url: Some("https://example.com".into()),
                        ..Default::default()
                    },
                )
            })
            .unwrap_err();
        assert_eq!(h.code, "invalid", "a public address is refused");
        let st = rig
            .run(|c| {
                api::ai_key_set(
                    c,
                    api::AiKeySetArgs {
                        backend: BackendKind::ClaudeApi,
                        key: None,
                    },
                )
            })
            .unwrap();
        assert!(!st.stored);
        let _ = rig
            .run(|c| api::ai_health(c, api::AiHealthArgs { probe: false }))
            .unwrap();

        // "Delete all Mode data" removes every keyring entry.
        commands_ai::secret_set_of(&rig.core, "local_token", Some("local-bearer-1234")).unwrap();
        assert!(commands_ai::forget_all_secrets(&rig.core) >= 1);
        assert!(
            !commands_ai::secret_status_of(&rig.core, "local_token")
                .unwrap()
                .stored
        );
    }

    #[test]
    fn offline_and_foreground_transitions() {
        let rig = ConnRig::new(false);
        let payload = crate::payloads(Arc::clone(&rig.core), None, Some(Arc::clone(&rig.c)));
        assert_eq!(payload(Event::Health).unwrap()["offline"], false);
        // Two failed reachability checks → offline: health and accounts pushed.
        let mut t = os_signals::OfflineTracker::default();
        assert_eq!(t.observe(false), None);
        let off = t.observe(false).unwrap();
        rig.c.set_offline(off);
        assert!(rig.c.is_offline());
        let pushes = rig.take_pushes();
        assert!(pushes.contains(&Event::Health) && pushes.contains(&Event::Accounts));
        let h = payload(Event::Health).unwrap();
        assert_eq!(h["offline"], true);
        assert!(h.get("first_run").is_some(), "the app's own fields stay");
        assert_eq!(
            rig.core
                .last_health
                .lock()
                .unwrap()
                .as_ref()
                .map(|h| h.offline),
            Some(true)
        );
        assert_eq!(payload(Event::Accounts).unwrap()["offline"], true);
        let st = rig
            .run(commands_connectors::connectors_status_view)
            .unwrap();
        assert!(st.offline);
        rig.c.set_offline(t.observe(true).unwrap());
        assert!(!rig.c.is_offline());
        assert!(rig.take_pushes().contains(&Event::Health));
        // The same state twice pushes nothing.
        rig.c.set_offline(false);
        assert!(rig.take_pushes().is_empty());

        // The app window's focus drives the foreground boost.
        assert!(!rig.c.is_foreground());
        os_signals::on_app_window_event(&rig.core, &tauri::WindowEvent::Focused(true));
        assert!(rig.c.is_foreground());
        os_signals::on_app_window_event(&rig.core, &tauri::WindowEvent::Focused(false));
        assert!(!rig.c.is_foreground());

        // OS facts → quiet.changed (presentation here; Focus on Windows).
        let mut os = rig.c.os_state();
        os.presentation = true;
        rig.c.set_os_state(os);
        assert!(rig.take_pushes().contains(&Event::Quiet));
        assert_eq!(payload(Event::Quiet).unwrap()["presentation"], true);
    }

    #[test]
    fn the_secret_names_keep_their_entries() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with(dir.path());
        // Without the connectors service the keyring is written directly.
        let v =
            commands_ai::secret_set_of(&core, "ai:local_token", Some("bearer-abcdefgh")).unwrap();
        assert!(v.stored);
        let raw: HashMap<String, String> = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("data/secrets-dev.json")).unwrap(),
        )
        .unwrap();
        assert!(raw.contains_key("ai:local_token"), "06 §13 entry name");
        assert!(
            !commands_ai::secret_set_of(&core, "local_token", None)
                .unwrap()
                .stored
        );
        assert_eq!(
            commands_ai::secret_status_of(&core, "account:x")
                .unwrap_err()
                .code,
            "invalid"
        );
    }
}

// ------------------------------------------------------------- package 11

mod v1 {
    use std::sync::{Arc, Mutex};

    use mode_core::chat::api::{self as chat_api, SendArgs, StatusArgs, ThreadIdArgs};
    use mode_core::chat::{Chat, ChatOptions};
    use mode_core::connectors::ai::fixture::FixtureBackend;
    use mode_core::connectors::ai::{AiBackend, BackendFactory, BackendKind};
    use mode_core::keyring::names;
    use mode_core::palette::{Palette, QueryArgs, palette_query};
    use mode_core::push::{Event, Push};
    use mode_core::scheduler::api::{self as sched_api, IdArgs, SaveArgs};
    use mode_core::scheduler::dispatch::LiveDispatch;
    use mode_core::scheduler::store::AutomationDraft;
    use mode_core::scheduler::{FailedRun, Scheduler, SchedulerOptions, Wake};
    use mode_core::themes::Themes;
    use mode_core::themes::api::{self as theme_api, ImportArgs, SetActiveArgs};
    use mode_core::urgency::alerts::AlertEvent;
    use serde_json::{Value, json};

    use super::{block_on, core_with, handler_commands};
    use crate::cmd_error::CmdError;
    use crate::commands_automations::with_scheduler;
    use crate::commands_chat::with_chat;
    use crate::commands_themes::with_themes;
    use crate::services;
    use crate::state::CoreState;

    const API_FILES: &[(&str, &str, &[&str])] = &[
        ("chat", include_str!("../../mode-core/src/chat/api.rs"), &[]),
        (
            "scheduler",
            include_str!("../../mode-core/src/scheduler/api.rs"),
            &[],
        ),
        (
            "themes",
            include_str!("../../mode-core/src/themes/api.rs"),
            &["swatches"],
        ),
        (
            "palette",
            include_str!("../../mode-core/src/palette/api.rs"),
            &["score_text", "recency"],
        ),
        (
            "connectors",
            include_str!("../../mode-core/src/connectors/api.rs"),
            &["connector_error", "oauth_error", "ai_test_call_with"],
        ),
    ];

    #[test]
    fn every_api_function_is_a_command_and_the_overlay_gets_two() {
        let handled = handler_commands();
        for (area, src, helpers) in API_FILES {
            let mut n = 0;
            for line in src.lines() {
                let Some(rest) = line.strip_prefix("pub fn ") else {
                    continue;
                };
                let name = rest.split(['(', '<']).next().unwrap();
                if helpers.contains(&name) {
                    continue;
                }
                n += 1;
                assert!(
                    handled.iter().any(|h| h == name),
                    "{area}::api::{name} has no command"
                );
            }
            assert!(n > 0, "{area}: no api functions parsed");
        }
        for extra in [
            "chat_pick_file",
            "alert_rules_list",
            "alert_rule_set",
            "alert_rule_add",
            "alert_rule_delete",
        ] {
            assert!(handled.iter().any(|h| h == extra), "{extra}");
        }
        let overlay = super::granted(super::CAPABILITIES[1].1);
        let app = super::granted(super::CAPABILITIES[0].1);
        for both in ["theme_resolve", "palette_query"] {
            assert!(overlay.contains(both) && app.contains(both), "{both}");
        }
        for app_only in [
            "chat_send",
            "chat_action_confirm",
            "chat_pick_file",
            "automation_save",
            "automation_run_now",
            "theme_import",
            "theme_set_active",
            "alert_rule_set",
            "alert_rule_add",
            "alert_rule_delete",
        ] {
            assert!(
                !overlay.contains(app_only),
                "the overlay must not get {app_only}"
            );
            assert!(app.contains(app_only), "the app window needs {app_only}");
        }
    }

    struct Factory(Arc<FixtureBackend>);

    impl BackendFactory for Factory {
        fn claude(&self, _api_key: String) -> Arc<dyn AiBackend> {
            Arc::clone(&self.0) as Arc<dyn AiBackend>
        }
        fn local(&self, _k: BackendKind, _url: &str, _t: Option<String>) -> Arc<dyn AiBackend> {
            Arc::clone(&self.0) as Arc<dyn AiBackend>
        }
    }

    struct V1Rig {
        _dir: tempfile::TempDir,
        core: Arc<CoreState>,
        ai: Arc<FixtureBackend>,
        streams: Arc<Mutex<Vec<Push>>>,
        failed: Arc<Mutex<Vec<FailedRun>>>,
        wake: tokio::sync::mpsc::UnboundedReceiver<Wake>,
    }

    /// The four services as `services::create` builds them, on a temp
    /// database, Chat inline over the fixture backend.
    fn rig() -> V1Rig {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let core = core_with(&root);
        core.keyring()
            .set(names::AI_ANTHROPIC_API_KEY, "sk-test-0000")
            .unwrap();
        let db = Arc::clone(&core.db);
        let t = Themes::new(Arc::clone(&db), root.join("exports")).unwrap();
        let _ = core.themes.set(t);
        let _ = core.palette.set(Palette::new(Arc::clone(&db)));
        let ai = Arc::new(FixtureBackend::canned(BackendKind::ClaudeApi));
        let chat = Chat::new(
            Arc::clone(&db),
            core.keyring(),
            Arc::new(Factory(Arc::clone(&ai))),
            ChatOptions {
                inline: true,
                ..Default::default()
            },
        );
        let streams: Arc<Mutex<Vec<Push>>> = Arc::default();
        let s = Arc::clone(&streams);
        chat.set_sink(move |p| s.lock().unwrap().push(p.clone()));
        let _ = core.chat.set(Arc::clone(&chat));
        let sched = Scheduler::new(
            db,
            Arc::new(LiveDispatch {
                notes: None,
                pty: None,
                chat: Some(chat),
            }),
            SchedulerOptions {
                inline: true,
                ..Default::default()
            },
        );
        let failed: Arc<Mutex<Vec<FailedRun>>> = Arc::default();
        let f = Arc::clone(&failed);
        sched.set_failure_sink(move |r| f.lock().unwrap().push(r.clone()));
        let _ = core.scheduler.set(sched);
        let (tx, wake) = tokio::sync::mpsc::unbounded_channel();
        let _ = core.scheduler_wake.set(tx);
        V1Rig {
            _dir: dir,
            core,
            ai,
            streams,
            failed,
            wake,
        }
    }

    #[test]
    fn chat_runs_behind_its_commands_and_streams_contract_events() {
        let r = rig();
        r.ai.push_text(
            &json!({"tool_calls": [], "answer": "Nothing is due today.", "citations": [], "actions": []})
                .to_string(),
        );
        let sent = block_on(with_chat(&r.core, |c| {
            chat_api::chat_send(
                c,
                SendArgs {
                    thread_id: None,
                    text: "What is due today?".into(),
                    effort: None,
                    attachments: None,
                    connectors: None,
                },
            )
        }))
        .unwrap();
        assert!(!sent.user_turn_id.is_empty());
        let streams = r.streams.lock().unwrap().clone();
        let names: Vec<&str> = streams.iter().map(|p| p.event.wire_name()).collect();
        assert!(
            names.contains(&"chat:delta") && names.contains(&"chat:turn"),
            "{names:?}"
        );
        let fin = streams
            .iter()
            .rev()
            .find(|p| p.event == Event::ChatTurn)
            .unwrap();
        assert_eq!(fin.payload["status"], "finished");
        assert_eq!(fin.payload["turn"]["text"], "Nothing is due today.");
        let id = sent.thread_id.clone();
        let t = block_on(with_chat(&r.core, move |c| {
            chat_api::chat_thread_get(c, ThreadIdArgs { id })
        }))
        .unwrap();
        assert_eq!(t.turns.len(), 2);
        let st = block_on(with_chat(&r.core, |c| {
            chat_api::chat_status(c, StatusArgs::default())
        }))
        .unwrap();
        assert_eq!(
            (st.ready.as_str(), st.backend.as_str()),
            ("ok", "claude_api")
        );
        // Quit stops nothing here (inline turns are over) and does not fail.
        services::stop(&r.core);
        // Without the service: `unavailable`.
        let dir = tempfile::tempdir().unwrap();
        let bare = core_with(dir.path());
        let e = block_on(with_chat(&bare, chat_api::chat_threads_list)).unwrap_err();
        assert_eq!(e.code, "unavailable");
    }

    /// XC-38, 02 §5.7: a "Hey Mode" request with no Mode window in front
    /// is answered on the reply card: `thinking` with `surface: card`,
    /// then `done` with the whole reply, even when the turn ended inside
    /// `chat_send` (inline here, as a refusal is in the app) before it was
    /// remembered as the voice turn. A voice turn left running is stopped.
    #[test]
    fn a_voice_request_reaches_the_reply_card_even_when_its_turn_ends_at_once() {
        use mode_core::speech::{DictationState, StateName, Surface, Target};
        let r = rig();
        let chat = Arc::clone(r.core.chat.get().unwrap());
        let core = Arc::clone(&r.core);
        chat.set_sink(move |p| crate::speech::on_chat_push(&core, p));
        let seen: Arc<Mutex<Vec<DictationState>>> = Arc::default();
        let s = Arc::clone(&seen);
        r.core
            .dictation
            .set_sink(move |st| s.lock().unwrap().push(st.clone()));
        r.ai.push_text(
            &json!({"tool_calls": [], "answer": "Today you have three tasks.", "citations": [], "actions": []})
                .to_string(),
        );
        *crate::state::CoreState::lock(&r.core.voice_turn) = Some(crate::speech::VoiceTurn {
            turn_id: "t-old".into(),
            thread_id: "th-old".into(),
            surface: Surface::Pill,
            reply: String::new(),
            last_push: None,
        });
        crate::speech::ask_chat(&r.core, "show me my todo for today".into());
        let states = seen.lock().unwrap().clone();
        let thinking = states
            .iter()
            .find(|s| s.state == StateName::Thinking)
            .expect("thinking");
        assert_eq!(thinking.surface, Some(Surface::Card));
        assert!(thinking.turn_id.is_some());
        let done = states.last().unwrap();
        assert_eq!(done.state, StateName::Done, "{states:?}");
        assert_eq!(done.target, Some(Target::Chat));
        assert_eq!(done.surface, Some(Surface::Card));
        assert_eq!(done.reply.as_deref(), Some("Today you have three tasks."));
        assert_eq!(done.turn_id, thinking.turn_id);
        assert_eq!(
            states.iter().filter(|s| s.state == StateName::Done).count(),
            1,
            "the end is reported once"
        );
        assert!(crate::state::CoreState::lock(&r.core.voice_turn).is_none());
    }

    fn draft(v: Value) -> AutomationDraft {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn automations_refuse_with_fields_push_rows_and_alert_on_failure() {
        let mut r = rig();
        let payload = crate::payloads(Arc::clone(&r.core), None, None);
        assert_eq!(payload(Event::Automations).unwrap(), json!([]));
        // Invalid: `{code: invalid, message, fields}`.
        let bad = draft(
            json!({"name": "", "schedule": {"kind": "cron", "cron": "0 8 * *"},
                               "action_kind": "ask_chat", "action": {"prompt": ""}}),
        );
        let e: CmdError = block_on(with_scheduler(&r.core, move |s| {
            sched_api::automation_save(s, SaveArgs { draft: bad })
        }))
        .unwrap_err();
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "invalid");
        let fields: Vec<&str> = v["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["field"].as_str().unwrap())
            .collect();
        assert!(
            fields.contains(&"name")
                && fields.contains(&"schedule.cron")
                && fields.contains(&"action.prompt"),
            "{fields:?}"
        );
        // Valid: the row (with its problem: no notes service here).
        let ok = draft(
            json!({"name": "Weekly changelog", "schedule": {"kind": "weekly", "time": "17:00", "weekday": 5},
                              "action_kind": "ask_chat", "action": {"prompt": "What changed?", "effort": "auto"}}),
        );
        let row = block_on(with_scheduler(&r.core, move |s| {
            sched_api::automation_save(s, SaveArgs { draft: ok })
        }))
        .unwrap();
        assert_eq!(row.problem.as_deref(), Some("folder_missing"));
        let rows = payload(Event::Automations).unwrap();
        assert_eq!(rows[0]["id"], row.id.as_str());
        assert_eq!(rows[0]["trigger_phrase"], "Weekly · Fri 17:00");
        // A run without the notes service fails; the failure reaches the sink
        // and maps onto `automation.failed`.
        let id = row.id.clone();
        let run = block_on(with_scheduler(&r.core, move |s| {
            sched_api::automation_run_now(s, IdArgs { id })
        }))
        .unwrap();
        assert_eq!(run.state, "skipped", "{run:?}");
        r.core.wake_scheduler(Wake::Changed);
        assert_eq!(r.wake.try_recv().unwrap(), Wake::Changed);
        let fr = FailedRun {
            automation_id: row.id.clone(),
            automation_name: row.name.clone(),
            run_id: "run-1".into(),
            error: "the notes folder is not available".into(),
        };
        let AlertEvent::AutomationFailed(a) = services::failed_event(&fr) else {
            panic!("automation.failed");
        };
        assert_eq!(
            (a.automation_id, a.run_id),
            (row.id.clone(), "run-1".to_string())
        );
        assert!(r.failed.lock().unwrap().is_empty(), "a skip is no failure");
        // Quit: nothing fires any more.
        services::stop(&r.core);
        let id = row.id.clone();
        let after = block_on(with_scheduler(&r.core, move |s| {
            sched_api::automation_run_now(s, IdArgs { id })
        }))
        .unwrap();
        assert_ne!(after.state, "running");
    }

    #[test]
    fn theme_changed_and_the_palette_follow_the_pushes() {
        let r = rig();
        let payload = crate::payloads(Arc::clone(&r.core), None, None);
        let theme = payload(Event::Theme).unwrap();
        assert_eq!(theme["slug"], "system");
        assert!(theme["tokens"]["--m-r-ctl"].is_string());
        assert!(theme["tokens"]["--m-terminal-cursor-accent"].is_string());
        let resolved = block_on(with_themes(&r.core, |t| {
            theme_api::theme_set_active(
                t,
                SetActiveArgs {
                    id: "nord".into(),
                    mode: Some("dark".into()),
                },
            )
        }))
        .unwrap();
        assert!(resolved.dark);
        assert_eq!(payload(Event::Theme).unwrap()["slug"], "nord");
        let pre = block_on(with_themes(&r.core, |t| {
            theme_api::theme_import(
                t,
                ImportArgs {
                    text: Some("not json".into()),
                    ..Default::default()
                },
            )
        }))
        .unwrap();
        assert!(!pre.ok && !pre.saved && !pre.issues.is_empty());
        // The palette: a pushed change makes the next query see new rows.
        let p = r.core.palette.get().unwrap().clone();
        let q = |text: &str| {
            palette_query(
                &p,
                QueryArgs {
                    text: text.into(),
                    limit: None,
                    kinds: None,
                },
            )
            .unwrap()
        };
        assert!(q("quokka").is_empty());
        r.core
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO notes (id, path, title, created_at, updated_at, device_id)
                     VALUES ('n-q', 'Quokka.md', 'Quokka facts', ?1, ?1, 'dev')",
                    [mode_core::db::now_rfc3339()],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(
            q("quokka").is_empty(),
            "the warm index answers until a push"
        );
        let _ = payload(Event::Notes);
        assert_eq!(q("quokka")[0].id, "n-q");
        assert!(
            q("go to automations")
                .iter()
                .any(|h| h.route == "automations")
        );
    }
}

// ----------------------------------------------------------- relay sidecar

/// Writes a stand-in relay: a script answering `--version` like `mode-hook`.
fn fake_relay(path: &Path, version: &str) {
    std::fs::write(path, format!("#!/bin/sh\necho 'mode-hook {version}'\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// `mode-hook --version` of `relay`, retried briefly: a process another test
/// forks while a script is being written can hold it open (ETXTBSY).
#[cfg(unix)]
fn version_of(relay: &Path) -> Option<String> {
    for _ in 0..20 {
        if let Some(v) = mode_core::hooks_install::relay_version(relay) {
            return Some(v);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    None
}

#[test]
fn the_sidecar_beside_the_binary_is_preferred_and_staged_under_the_data_dir() {
    use mode_core::hooks_install::{RELAY_FILE_NAME, default_relay_path};
    let dir = tempfile::tempdir().unwrap();
    // A bundle's layout: the sidecar beside the executable, resources apart.
    let exe_dir = dir.path().join("Mode.app").join("Contents").join("MacOS");
    let res_dir = dir
        .path()
        .join("Mode.app")
        .join("Contents")
        .join("Resources");
    std::fs::create_dir_all(&exe_dir).unwrap();
    std::fs::create_dir_all(res_dir.join("bin")).unwrap();
    let data = dir.path().join("data").join("Mode");
    let triple = "x86_64-unknown-linux-gnu";
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let sidecar = exe_dir.join(RELAY_FILE_NAME);
    let suffixed = exe_dir.join(format!("mode-hook-{triple}{ext}"));
    let in_resources = res_dir.join("bin").join(RELAY_FILE_NAME);
    fake_relay(&in_resources, "0.0.1");
    fake_relay(&suffixed, "0.0.2");
    fake_relay(&sidecar, "0.1.0");
    let find = |env: Option<PathBuf>| {
        crate::relay_candidates(env, Some(&exe_dir), Some(&res_dir), Some(triple))
            .into_iter()
            .find(|p| p.is_file())
    };

    assert_eq!(find(None).as_ref(), Some(&sidecar), "the sidecar wins");
    let explicit = dir.path().join("explicit-relay");
    fake_relay(&explicit, "9.9.9");
    assert_eq!(
        find(Some(explicit.clone())).as_ref(),
        Some(&explicit),
        "MODE_HOOK_BIN overrides the bundle"
    );

    // Staged under the data dir: the path written into settings.json.
    let staged = crate::stage_bundled_relay(find(None).as_deref(), &data).unwrap();
    assert_eq!(staged, default_relay_path(&data));
    assert_eq!(
        std::fs::read(&staged).unwrap(),
        std::fs::read(&sidecar).unwrap()
    );
    #[cfg(unix)]
    assert_eq!(version_of(&staged).as_deref(), Some("0.1.0"));

    // An app update ships a new sidecar: the next launch replaces the staged
    // copy in place, so the hook command stays valid and names the new version.
    fake_relay(&sidecar, "0.2.0");
    let updated = crate::stage_bundled_relay(find(None).as_deref(), &data).unwrap();
    assert_eq!(updated, staged, "the relay path does not move");
    assert_eq!(
        std::fs::read(&updated).unwrap(),
        std::fs::read(&sidecar).unwrap()
    );
    #[cfg(unix)]
    assert_eq!(version_of(&updated).as_deref(), Some("0.2.0"));

    // Fallbacks: the triple-suffixed file, then the resource dir, then none.
    std::fs::remove_file(&sidecar).unwrap();
    assert_eq!(find(None).as_ref(), Some(&suffixed));
    std::fs::remove_file(&suffixed).unwrap();
    assert_eq!(find(None).as_ref(), Some(&in_resources));
    std::fs::remove_dir_all(&res_dir).unwrap();
    assert_eq!(find(None), None);
    assert_eq!(crate::stage_bundled_relay(None, &data), None);
    assert!(
        staged.is_file(),
        "a missing bundle leaves the staged copy alone"
    );
}

/// The relay cargo built for this profile (`target/<profile>/mode-hook`, or
/// tauri-build's copy of the sidecar there), staged, passes the version
/// check the hook status uses.
#[cfg(unix)]
#[test]
fn the_staged_workspace_relay_reports_this_build_s_version() {
    use mode_core::hooks_install::{RELAY_FILE_NAME, RELAY_VERSION};
    let relay = std::env::current_exe()
        .ok()
        .and_then(|e| Some(e.parent()?.parent()?.join(RELAY_FILE_NAME)))
        .filter(|p| p.is_file());
    let Some(relay) = relay else {
        eprintln!("mode-hook not built for this profile; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let staged = crate::stage_bundled_relay(Some(&relay), dir.path()).unwrap();
    assert_eq!(version_of(&staged).as_deref(), Some(RELAY_VERSION));
}
