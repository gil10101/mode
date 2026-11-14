//! The Settings, Shortcuts, onboarding and profile commands (package 9;
//! `ui/src/state/contract-settings.ts`): the settings registry
//! (`settings_get_all`, `settings_reset`), export / import (03 §8.18),
//! `platform_info`, `hotkey_set` (03 §7D), the capture / templates folders
//! (03 §8.7), the vault picker (03 §2.8), the profile picture (03 §8.2),
//! "Delete all Mode data" (03 §8.15: accounts revoked and every keyring
//! entry removed first). Secrets, AI and connectors status live in
//! `commands_ai` / `commands_connectors`. App window only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mode_core::notes::Notes;
use mode_core::notes::api;
use mode_core::notes::views::FoldersList;
use mode_core::push::Event;
use mode_core::settings::{self, ImportOutcome};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::cmd_error::{CmdError, CmdResult, blocking};
use crate::commands::{self, Stored};
use crate::dialogs::{self, Picked};
use crate::events::{self, CoreChanged};
use crate::hotkeys;
use crate::overlay::Overlay;
use crate::overlay::geometry::Os;
use crate::state::CoreState;
use crate::{bundle, health, jump};

type Core<'a> = State<'a, Arc<CoreState>>;

fn from_string(e: String) -> CmdError {
    CmdError::invalid(e)
}

// ---------------------------------------------------------------- registry

/// Every registered key with its effective value (device and user scope).
#[tauri::command(rename_all = "snake_case")]
pub async fn settings_get_all(state: Core<'_>) -> CmdResult<BTreeMap<String, Value>> {
    let db = Arc::clone(&state.db);
    blocking(move || Ok(settings::all_effective(&db)?)).await
}

/// Delete the row so the default applies again; returns the default. A
/// hotkey is re-registered; the windows get `settings.changed`.
#[tauri::command(rename_all = "snake_case")]
pub async fn settings_reset(app: AppHandle, state: Core<'_>, key: String) -> CmdResult<Value> {
    let core = Arc::clone(&state);
    if let Stored::Refused(why) = commands::write_setting(&app, &core, &key, None)
        .await
        .map_err(from_string)?
    {
        return Err(CmdError::invalid(format!(
            "the default could not be registered: {why}"
        )));
    }
    let db = Arc::clone(&state.db);
    blocking(move || Ok(settings::get(&db, &key)?)).await
}

// ----------------------------------------------------------- export / import

/// `settings_export` / `diagnostics_export`: where the file went (`null`
/// when the save dialog was canceled).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WrittenFile {
    pub path: Option<String>,
}

fn local_today(core: &CoreState) -> String {
    core.db
        .read(|c| Ok(mode_core::queries::Clock::current(c)?.today))
        .unwrap_or_else(|_| mode_core::db::now_rfc3339()[..10].to_string())
}

/// Write `mode-export-<date>.json` (03 §8.18) with the sections picked:
/// the save dialog when one can be shown, else `<data_dir>/exports/`.
#[tauri::command(rename_all = "snake_case")]
pub async fn settings_export(
    app: AppHandle,
    state: Core<'_>,
    sections: Vec<String>,
    include_device: Option<bool>,
) -> CmdResult<WrittenFile> {
    let core = Arc::clone(&state);
    blocking(move || {
        let include_device = include_device.unwrap_or(false);
        let b = core
            .db
            .read(|c| bundle::export_in(c, &sections, include_device))?;
        let text = serde_json::to_string_pretty(&b).map_err(CmdError::internal)?;
        let name = format!("mode-export-{}.json", local_today(&core));
        let path = match dialogs::save_file(
            &app,
            "Export Mode settings",
            &name,
            dirs::download_dir().as_deref(),
            "Mode export",
            &["json"],
        ) {
            Picked::Path(p) => p,
            Picked::Cancelled => return Ok(WrittenFile { path: None }),
            Picked::Unavailable => {
                let dir = core.data_dir.join("exports");
                std::fs::create_dir_all(&dir)?;
                dialogs::unique_in(&dir, &name)
            }
        };
        std::fs::write(&path, text)?;
        tracing::info!(path = %path.display(), sections = ?b.sections, "settings exported");
        Ok(WrittenFile {
            path: Some(path.to_string_lossy().into_owned()),
        })
    })
    .await
}

/// `settings_import`: the merge preview or what was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportView {
    pub path: Option<String>,
    pub applied: bool,
    pub entries: BTreeMap<String, ImportOutcome>,
}

/// Read an export (the open dialog when `path` is null) and merge it:
/// `apply: false` is the preview, `apply: true` writes it (03 §8.18).
#[tauri::command(rename_all = "snake_case")]
pub async fn settings_import(
    app: AppHandle,
    state: Core<'_>,
    path: Option<String>,
    apply: Option<bool>,
    include_device: Option<bool>,
) -> CmdResult<ImportView> {
    let apply = apply.unwrap_or(false);
    let core = Arc::clone(&state);
    let a = app.clone();
    let view = blocking(move || {
        let path: PathBuf = match path.filter(|p| !p.trim().is_empty()) {
            Some(p) => dialogs::expand_home(&p),
            None => {
                match dialogs::pick_file(&a, "Import Mode settings", "Mode export", &["json"]) {
                    Picked::Path(p) => p,
                    Picked::Cancelled => {
                        return Ok(ImportView {
                            path: None,
                            applied: false,
                            entries: BTreeMap::new(),
                        });
                    }
                    Picked::Unavailable => {
                        return Err(CmdError::new(
                            "unavailable",
                            "The file picker is not available here; pass the file's path.",
                        ));
                    }
                }
            }
        };
        let meta = std::fs::metadata(&path)
            .map_err(|e| CmdError::not_found(format!("{}: {e}", path.display())))?;
        if meta.len() > 32 * 1024 * 1024 {
            return Err(CmdError::invalid(
                "the file is too large to be a Mode export",
            ));
        }
        let text = std::fs::read_to_string(&path)?;
        let b = bundle::parse(&text)?;
        let device = core.db.device_id().to_string();
        let include_device = include_device.unwrap_or(false);
        let report = if apply {
            core.db
                .write(move |c| bundle::import_in(c, &device, &b, include_device, true))?
        } else {
            core.db
                .read(|c| bundle::import_in(c, &device, &b, include_device, false))?
        };
        Ok(ImportView {
            path: Some(path.to_string_lossy().into_owned()),
            applied: report.applied,
            entries: report.entries,
        })
    })
    .await?;
    if view.applied {
        let written = |o: &ImportOutcome| matches!(o, ImportOutcome::New | ImportOutcome::Updated);
        let keys: Vec<String> = view
            .entries
            .iter()
            .filter(|(k, o)| written(o) && settings::key_def(k).is_some())
            .map(|(k, _)| k.clone())
            .collect();
        tracing::info!(
            settings = keys.len(),
            rows = view
                .entries
                .iter()
                .filter(|(k, o)| written(o) && k.contains('/'))
                .count(),
            "settings imported"
        );
        if keys
            .iter()
            .any(|k| hotkeys::HotkeyKind::from_key(k).is_some() || k == "dictation.enabled")
        {
            hotkeys::register_from_settings(app.clone(), Arc::clone(&state)).await;
        }
        if !keys.is_empty() {
            commands::after_settings_change(&app, &state, keys).await;
        }
        if view.entries.keys().any(|k| k.starts_with("groupings/")) {
            state.publish([Event::Notes]);
        }
    }
    Ok(view)
}

// --------------------------------------------------------------- platform

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DisplayInfo {
    pub id: String,
    pub name: String,
    pub main: bool,
    pub notch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformInfo {
    /// `macos` | `windows` | `linux`.
    pub os: &'static str,
    pub notch: bool,
    pub displays: Vec<DisplayInfo>,
    pub reduced_motion: bool,
    pub terminal_apps: Vec<String>,
}

pub fn os_word(os: Os) -> &'static str {
    match os {
        Os::MacOs => "macos",
        Os::Windows => "windows",
        Os::Linux => "linux",
    }
}

/// The OS, the notch, the displays, reduced motion and the terminal apps
/// found (onboarding step 1, the Claude Code card's terminal picker).
#[tauri::command(rename_all = "snake_case")]
pub async fn platform_info(app: AppHandle) -> CmdResult<PlatformInfo> {
    let os = Os::current();
    // AppKit wants the main thread.
    let (notch, reduced) = match app.try_state::<Arc<Overlay>>() {
        Some(overlay) => {
            let platform = overlay.platform();
            let (tx, rx) = tokio::sync::oneshot::channel();
            app.run_on_main_thread(move || {
                let _ = tx.send((
                    platform.notch_geometry().is_some(),
                    platform.reduced_motion(),
                ));
            })?;
            rx.await.unwrap_or((false, None))
        }
        None => (false, None),
    };
    let primary = app.primary_monitor().ok().flatten();
    let displays = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let main = primary
                .as_ref()
                .is_some_and(|p| p.position() == m.position() && p.name() == m.name());
            let name = m
                .name()
                .cloned()
                .unwrap_or_else(|| format!("Display {}", i + 1));
            DisplayInfo {
                id: name.clone(),
                name,
                main,
                // The notch is on the built-in display, the main one when present.
                notch: notch && main,
            }
        })
        .collect();
    let terminal_apps = blocking(move || Ok(jump::terminal_apps(os))).await?;
    Ok(PlatformInfo {
        os: os_word(os),
        notch,
        displays,
        reduced_motion: reduced.unwrap_or(false),
        terminal_apps,
    })
}

// --------------------------------------------------------------- shortcuts

/// The global hotkeys by Shortcuts-view id (the settings key works too).
pub const GLOBAL_HOTKEYS: &[(&str, &str)] = &[
    ("global.capture", "capture.hotkey"),
    ("global.toggle_overlay", "overlay.toggle_hotkey"),
    ("global.pin_overlay", "overlay.pin_hotkey"),
    ("global.open_app", "window.hotkey"),
    ("global.dictate", "dictation.hotkey"),
];

/// The settings key of a global hotkey name.
pub fn global_key(name: &str) -> Option<&'static str> {
    GLOBAL_HOTKEYS
        .iter()
        .find(|(id, key)| *id == name || *key == name)
        .map(|(_, key)| *key)
}

/// An app-window shortcut id (`sessions.jump`, `notes.new_folder`).
pub fn valid_shortcut_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HotkeySetResult {
    pub name: String,
    /// The chord now stored (`null` when cleared).
    pub keys: Option<String>,
    /// Global hotkeys: whether the OS accepted it (always true for the rest).
    pub registered: bool,
    pub reason: Option<String>,
}

/// Re-bind a shortcut (03 §7D): global names write their settings key and
/// re-register (a chord the OS refuses answers `registered: false` with
/// the reason and keeps the old one; `null` clears `window.hotkey` and
/// resets the other three to their defaults); every other name writes
/// `shortcuts.bindings[name]` (`null` = unbound).
#[tauri::command(rename_all = "snake_case")]
pub async fn hotkey_set(
    app: AppHandle,
    state: Core<'_>,
    name: String,
    keys: Option<String>,
) -> CmdResult<HotkeySetResult> {
    let core = Arc::clone(&state);
    let keys = keys.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
    if let Some(k) = &keys
        && k.chars().count() > 64
    {
        return Err(CmdError::invalid("the shortcut is too long"));
    }
    let stored_chord = |db: Arc<mode_core::db::Db>, key: &'static str| async move {
        blocking(move || Ok(settings::get_string(&db, key)?)).await
    };
    if let Some(key) = global_key(&name) {
        if let Some(k) = &keys
            && let Err(e) = hotkeys::parse_chord(k)
        {
            return Ok(HotkeySetResult {
                name,
                keys: stored_chord(Arc::clone(&state.db), key).await?,
                registered: false,
                reason: Some(e.to_string()),
            });
        }
        let value = match &keys {
            Some(k) => Some(Value::String(k.clone())),
            None if key == "window.hotkey" => Some(Value::Null),
            None => None,
        };
        let stored = commands::write_setting(&app, &core, key, value)
            .await
            .map_err(from_string)?;
        let now = stored_chord(Arc::clone(&state.db), key).await?;
        return Ok(match stored {
            Stored::Done => HotkeySetResult {
                name,
                keys: now,
                registered: true,
                reason: None,
            },
            Stored::Refused(why) => HotkeySetResult {
                name,
                keys: now,
                registered: false,
                reason: Some(why),
            },
        });
    }
    if !valid_shortcut_name(&name) {
        return Err(CmdError::not_found(format!("{name:?} is not a shortcut")));
    }
    let db = Arc::clone(&state.db);
    let mut map = blocking(move || Ok(settings::get(&db, "shortcuts.bindings")?)).await?;
    if !map.is_object() {
        map = Value::Object(Default::default());
    }
    map.as_object_mut().expect("an object").insert(
        name.clone(),
        keys.clone().map(Value::String).unwrap_or(Value::Null),
    );
    commands::write_setting(&app, &core, "shortcuts.bindings", Some(map))
        .await
        .map_err(from_string)?;
    Ok(HotkeySetResult {
        name,
        keys,
        registered: true,
        reason: None,
    })
}

// ------------------------------------------------------------------- notes

/// A vault-relative folder path (`a/b`), checked to exist inside `root`.
pub fn folder_in_vault(root: &Path, input: &str) -> CmdResult<String> {
    let raw = input.trim();
    let candidate = if Path::new(raw).is_absolute() || raw.starts_with('~') {
        dialogs::expand_home(raw)
    } else {
        root.join(raw.trim_matches('/'))
    };
    let canon_root = std::fs::canonicalize(root)
        .map_err(|_| CmdError::not_found("the notes folder is missing"))?;
    let canon = std::fs::canonicalize(&candidate)
        .map_err(|_| CmdError::not_found(format!("{raw} does not exist in the notes folder")))?;
    if !canon.is_dir() {
        return Err(CmdError::invalid(format!("{raw} is not a folder")));
    }
    let rel = canon
        .strip_prefix(&canon_root)
        .map_err(|_| CmdError::invalid(format!("{raw} is outside the notes folder")))?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    if parts.is_empty() {
        return Err(CmdError::invalid(
            "pick a folder inside the notes folder, not the folder itself",
        ));
    }
    if parts.iter().any(|p| p.starts_with('.')) {
        return Err(CmdError::invalid("hidden folders cannot hold notes"));
    }
    Ok(parts.join("/"))
}

/// Set the capture folder and / or the templates folder (03 §8.7): each a
/// folder inside the vault (vault-relative or absolute); `templates: ""`
/// clears the templates folder. At most one folder of each kind; the
/// change is stamped and `notes.changed` requested. Returns the folders.
pub fn set_note_folders(
    n: &Notes,
    capture: Option<String>,
    templates: Option<String>,
) -> CmdResult<FoldersList> {
    let root = n.root();
    let mut wanted: Vec<(&'static str, Option<String>)> = Vec::new();
    if let Some(c) = capture {
        wanted.push(("capture", Some(folder_in_vault(&root, &c)?)));
    }
    if let Some(t) = templates {
        let t = t.trim();
        wanted.push((
            "templates",
            if t.is_empty() {
                None
            } else {
                Some(folder_in_vault(&root, t)?)
            },
        ));
    }
    if wanted.is_empty() {
        return Err(CmdError::invalid("name a capture or templates folder"));
    }
    if wanted.iter().filter_map(|(_, p)| p.as_ref()).count() == 2 && wanted[0].1 == wanted[1].1 {
        return Err(CmdError::invalid(
            "the capture and templates folders must differ",
        ));
    }
    // A folder made a moment ago may not be indexed yet.
    let missing = |w: &[(&str, Option<String>)]| -> CmdResult<bool> {
        for p in w.iter().filter_map(|(_, p)| p.clone()) {
            let found: bool = n.db().read(move |c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM folders WHERE path = ?1 AND deleted_at IS NULL",
                    [&p],
                    |r| r.get::<_, i64>(0),
                )? > 0)
            })?;
            if !found {
                return Ok(true);
            }
        }
        Ok(false)
    };
    if missing(&wanted)? {
        n.scan().map_err(CmdError::from)?;
        if missing(&wanted)? {
            return Err(CmdError::not_found("that folder is not indexed"));
        }
    }
    let device = n.db().device_id().to_string();
    n.db().write(move |c| {
        let now = mode_core::db::now_rfc3339();
        for (kind, path) in &wanted {
            match path {
                Some(p) => {
                    c.execute(
                        "UPDATE folders SET kind = 'normal', updated_at = ?2, rev = rev + 1, device_id = ?3
                          WHERE kind = ?1 AND path <> ?4",
                        rusqlite::params![kind, now, device, p],
                    )?;
                    c.execute(
                        "UPDATE folders SET kind = ?1, updated_at = ?2, rev = rev + 1, device_id = ?3
                          WHERE path = ?4 AND deleted_at IS NULL AND kind <> ?1",
                        rusqlite::params![kind, now, device, p],
                    )?;
                }
                None => {
                    c.execute(
                        "UPDATE folders SET kind = 'normal', updated_at = ?2, rev = rev + 1, device_id = ?3
                          WHERE kind = ?1",
                        rusqlite::params![kind, now, device],
                    )?;
                }
            }
        }
        Ok(())
    })?;
    n.changed();
    tracing::info!("notes folders set");
    api::folders_list(n).map_err(CmdError::from)
}

/// The capture / templates folder setter of Settings → Notes (03 §8.7).
#[tauri::command(rename_all = "snake_case")]
pub async fn notes_folders_set(
    state: Core<'_>,
    capture: Option<String>,
    templates: Option<String>,
) -> CmdResult<FoldersList> {
    let notes = state.notes()?;
    blocking(move || set_note_folders(&notes, capture, templates)).await
}

// ------------------------------------------------------------------- vault

/// Directories skipped when counting notes.
const SKIP_DIRS: &[&str] = &["node_modules", "target"];
/// At most this many directory entries are looked at.
pub const COUNT_ENTRIES_MAX: usize = 50_000;

/// `.md` files under `root` (hidden folders and `node_modules` skipped),
/// looking at no more than [`COUNT_ENTRIES_MAX`] entries.
pub fn count_md_files(root: &Path) -> usize {
    let mut stack = vec![root.to_path_buf()];
    let (mut seen, mut md) = (0usize, 0usize);
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            seen += 1;
            if seen > COUNT_ENTRIES_MAX {
                return md;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(e.path());
                }
            } else if ft.is_file() && name.to_ascii_lowercase().ends_with(".md") {
                md += 1;
            }
        }
    }
    md
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VaultPick {
    /// `null`: the dialog was canceled.
    pub path: Option<String>,
    pub md_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VaultInspect {
    pub path: String,
    pub exists: bool,
    pub md_files: usize,
}

/// The notes folder picker (onboarding step 2, Settings → Notes): the
/// folder and how many `.md` files it already holds.
#[tauri::command(rename_all = "snake_case")]
pub async fn vault_pick(app: AppHandle, current: Option<String>) -> CmdResult<VaultPick> {
    blocking(move || {
        let start = current.as_deref().map(dialogs::expand_home);
        match dialogs::pick_folder(&app, "Choose the notes folder", start.as_deref()) {
            Picked::Path(p) => Ok(VaultPick {
                md_files: count_md_files(&p),
                path: Some(p.to_string_lossy().into_owned()),
            }),
            Picked::Cancelled => Ok(VaultPick {
                path: None,
                md_files: 0,
            }),
            Picked::Unavailable => Err(CmdError::new(
                "unavailable",
                "The folder picker is not available; type the path instead.",
            )),
        }
    })
    .await
}

/// Does a typed notes folder exist, and how many notes does it hold.
#[tauri::command(rename_all = "snake_case")]
pub async fn vault_inspect(path: String) -> CmdResult<VaultInspect> {
    blocking(move || {
        let p = dialogs::expand_home(path.trim());
        let exists = p.is_dir();
        Ok(VaultInspect {
            md_files: if exists { count_md_files(&p) } else { 0 },
            exists,
            path: p.to_string_lossy().into_owned(),
        })
    })
    .await
}

// ----------------------------------------------------------------- profile

/// Largest picture accepted.
pub const PICTURE_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// The image type of `bytes` by its signature: `png`, `jpg`, `gif`, `webp`.
pub fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// The webview URL of a local file (Tauri's asset protocol, as
/// `convertFileSrc` builds it).
pub fn asset_url(path: &Path) -> String {
    let enc: String = path
        .to_string_lossy()
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    if cfg!(windows) {
        format!("http://asset.localhost/{enc}")
    } else {
        format!("asset://localhost/{enc}")
    }
}

/// Copy a picture into `<data_dir>/profile/picture-<stamp>.<ext>`, removing
/// the previous ones. Returns the copy.
pub fn store_picture(data_dir: &Path, source: &Path) -> CmdResult<PathBuf> {
    let meta = std::fs::metadata(source)?;
    if meta.len() > PICTURE_MAX_BYTES {
        return Err(CmdError::invalid("the picture is larger than 10 MB"));
    }
    let bytes = std::fs::read(source)?;
    let ext = image_kind(&bytes)
        .ok_or_else(|| CmdError::invalid("that file is not a PNG, JPEG, GIF or WebP picture"))?;
    let dir = data_dir.join("profile");
    std::fs::create_dir_all(&dir)?;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with("picture") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let stamp: String = mode_core::db::now_rfc3339()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let dest = dir.join(format!("picture-{stamp}.{ext}"));
    std::fs::write(&dest, bytes)?;
    Ok(dest)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PicturePick {
    /// The copy under the data dir (what `profile.picture_path` stores);
    /// `null` when canceled.
    pub path: Option<String>,
    /// Its asset-protocol URL for an `<img>`.
    pub url: Option<String>,
}

/// The profile picture (03 §8.2): a PNG / JPEG / GIF / WebP picker; the file
/// is copied under `<app_data_dir>/profile/` (served by the asset protocol).
#[tauri::command(rename_all = "snake_case")]
pub async fn profile_picture_pick(app: AppHandle, state: Core<'_>) -> CmdResult<PicturePick> {
    let data_dir = state.data_dir.clone();
    blocking(move || {
        let source = match dialogs::pick_file(
            &app,
            "Choose a profile picture",
            "Pictures",
            &["png", "jpg", "jpeg", "gif", "webp"],
        ) {
            Picked::Path(p) => p,
            Picked::Cancelled => {
                return Ok(PicturePick {
                    path: None,
                    url: None,
                });
            }
            Picked::Unavailable => {
                return Err(CmdError::new(
                    "unavailable",
                    "The file picker is not available here.",
                ));
            }
        };
        let dest = store_picture(&data_dir, &source)?;
        tracing::info!(path = %dest.display(), "profile picture stored");
        Ok(PicturePick {
            url: Some(asset_url(&dest)),
            path: Some(dest.to_string_lossy().into_owned()),
        })
    })
    .await
}

// ------------------------------------------------------------- delete all

/// What stays in the data dir after "Delete all Mode data": the logs, the
/// staged relay (installed hooks point at it), the database files (emptied
/// in place) and the hook socket.
const KEEP: &[&str] = &[
    "logs",
    "bin",
    "mode.db",
    "mode.db-wal",
    "mode.db-shm",
    "mode.sock",
];

/// Remove everything in `data_dir` but [`KEEP`] and sockets; the names removed.
pub fn clear_data_dir(data_dir: &Path) -> std::io::Result<Vec<String>> {
    let mut removed = Vec::new();
    let rd = match std::fs::read_dir(data_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(removed),
        Err(e) => return Err(e),
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if KEEP.contains(&name.as_str()) || name.ends_with(".sock") {
            continue;
        }
        let ft = e.file_type()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            if ft.is_socket() {
                continue;
            }
        }
        if ft.is_dir() {
            std::fs::remove_dir_all(e.path())?;
        } else {
            std::fs::remove_file(e.path())?;
        }
        removed.push(name);
    }
    removed.sort();
    Ok(removed)
}

/// "Delete all Mode data" (03 §8.15): `confirm` must be `delete`. Refused
/// while embedded sessions run. Deletes the keyring entries, empties the
/// database in place (device id and premade rows stay), removes the data
/// dir's other contents but the logs and the relay, re-indexes the
/// untouched vault, and pushes `core.changed` plus every view's push.
#[tauri::command(rename_all = "snake_case")]
pub async fn data_delete_all(app: AppHandle, state: Core<'_>, confirm: String) -> CmdResult<()> {
    if confirm != "delete" {
        return Err(CmdError::invalid("type delete to confirm"));
    }
    if state.is_db_newer() {
        return Err(CmdError::new(
            "read_only",
            "The database belongs to a newer Mode; nothing was deleted.",
        ));
    }
    let core = Arc::clone(&state);
    let live = blocking(move || Ok(crate::quit::live_sessions(&core))).await?;
    if !live.is_empty() {
        return Err(CmdError::new(
            "busy",
            format!(
                "Close the {} running terminal{} first.",
                live.len(),
                if live.len() == 1 { "" } else { "s" }
            ),
        ));
    }
    let core = Arc::clone(&state);
    blocking(move || {
        let secrets = crate::commands_ai::forget_all_secrets(&core);
        let report = core.db.reset_contents()?;
        let removed = clear_data_dir(&core.data_dir)?;
        *CoreState::lock(&core.hooks) = None;
        *CoreState::lock(&core.last_health) = None;
        CoreState::lock(&core.recent_frames).clear();
        *CoreState::lock(&core.last_full_scan_at) = None;
        if let Some(n) = core.notes.get() {
            match n.scan() {
                Ok(r) => {
                    *CoreState::lock(&core.last_full_scan_at) = Some(mode_core::db::now_rfc3339());
                    tracing::info!(files = r.files, "notes re-indexed after the data reset");
                }
                Err(e) => tracing::warn!(error = %e, "notes re-index failed after the data reset"),
            }
        }
        tracing::warn!(
            tables = report.deleted.len(),
            secrets,
            removed = ?removed,
            "all Mode data deleted"
        );
        Ok(())
    })
    .await?;
    let c = Arc::clone(&state);
    let _ = blocking(move || {
        let _ = commands::reinspect_hooks(&c);
        Ok(())
    })
    .await;
    events::emit_core_changed(&app, CoreChanged::reason("data_deleted", &state));
    state.publish([
        Event::Sessions,
        Event::Pill,
        Event::Notes,
        Event::Accounts,
        Event::Inbox,
        Event::Agenda,
        Event::Quiet,
    ]);
    commands::emit_overlay_settings(&app, &state, None).await;
    commands::emit_app_settings(&app, &state, None).await;
    hotkeys::register_from_settings(app.clone(), Arc::clone(&state)).await;
    health::emit(app, Arc::clone(&state)).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_hotkeys_map_by_id_or_key() {
        assert_eq!(global_key("global.capture"), Some("capture.hotkey"));
        assert_eq!(global_key("window.hotkey"), Some("window.hotkey"));
        assert_eq!(global_key("sessions.jump"), None);
        assert!(valid_shortcut_name("notes.new_folder"));
        assert!(!valid_shortcut_name("bad name"));
        assert!(!valid_shortcut_name(""));
    }

    #[test]
    fn pictures_are_sniffed_and_urls_encoded() {
        assert_eq!(image_kind(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(image_kind(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_kind(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_kind(b"<svg/>"), None);
        let url = asset_url(Path::new("/d/Mode Dev/profile/picture-1.png"));
        if cfg!(windows) {
            assert!(url.starts_with("http://asset.localhost/"));
        } else {
            assert_eq!(
                url,
                "asset://localhost/%2Fd%2FMode%20Dev%2Fprofile%2Fpicture-1.png"
            );
        }
    }

    #[test]
    fn os_words_follow_the_contract() {
        assert_eq!(os_word(Os::MacOs), "macos");
        assert_eq!(os_word(Os::Windows), "windows");
        assert_eq!(os_word(Os::Linux), "linux");
    }
}
