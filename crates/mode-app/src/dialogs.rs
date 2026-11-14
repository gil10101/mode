//! The OS file and folder pickers the core shows for the app window (the
//! vault, export / import, the profile picture) through the dialog plugin.
//! Blocking: call them on the blocking pool, never on the main thread. No
//! dialog is shown in the headless smoke run (`MODE_SMOKE=1`) or with
//! `MODE_NO_DIALOGS=1`: callers get [`Picked::Unavailable`] and fall back.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    Path(PathBuf),
    Cancelled,
    /// No dialog can be shown here (headless run, plugin missing).
    Unavailable,
}

/// Whether a dialog may be shown.
pub fn available(app: &AppHandle) -> bool {
    let off = |k: &str| std::env::var(k).is_ok_and(|v| v == "1");
    !off("MODE_SMOKE")
        && !off("MODE_NO_DIALOGS")
        && app
            .try_state::<tauri_plugin_dialog::Dialog<Wry>>()
            .is_some()
}

fn builder(app: &AppHandle, title: &str) -> FileDialogBuilder<Wry> {
    let mut b = app.dialog().file().set_title(title);
    if let Some(w) = app.get_webview_window(crate::APP_WINDOW) {
        b = b.set_parent(&w);
    }
    b
}

fn outcome(p: Option<tauri_plugin_dialog::FilePath>) -> Picked {
    match p.map(|p| p.into_path()) {
        Some(Ok(path)) => Picked::Path(path),
        Some(Err(e)) => {
            tracing::warn!(error = %e, "the picked item is not a local path");
            Picked::Cancelled
        }
        None => Picked::Cancelled,
    }
}

/// A folder picker starting in `start` when it exists.
pub fn pick_folder(app: &AppHandle, title: &str, start: Option<&Path>) -> Picked {
    if !available(app) {
        return Picked::Unavailable;
    }
    let mut b = builder(app, title);
    if let Some(s) = start.filter(|s| s.is_dir()) {
        b = b.set_directory(s);
    }
    outcome(b.blocking_pick_folder())
}

/// A file picker limited to `extensions`.
pub fn pick_file(app: &AppHandle, title: &str, filter: &str, extensions: &[&str]) -> Picked {
    if !available(app) {
        return Picked::Unavailable;
    }
    outcome(
        builder(app, title)
            .add_filter(filter, extensions)
            .blocking_pick_file(),
    )
}

/// A file picker over any file (Chat's `+` → Add file).
pub fn pick_any_file(app: &AppHandle, title: &str) -> Picked {
    if !available(app) {
        return Picked::Unavailable;
    }
    outcome(builder(app, title).blocking_pick_file())
}

/// A save dialog proposing `file_name` (in `start` when it exists).
pub fn save_file(
    app: &AppHandle,
    title: &str,
    file_name: &str,
    start: Option<&Path>,
    filter: &str,
    extensions: &[&str],
) -> Picked {
    if !available(app) {
        return Picked::Unavailable;
    }
    let mut b = builder(app, title)
        .set_file_name(file_name)
        .add_filter(filter, extensions);
    if let Some(s) = start.filter(|s| s.is_dir()) {
        b = b.set_directory(s);
    }
    outcome(b.blocking_save_file())
}

/// `~/x` → `<home>/x`.
pub fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None if path == "~" => dirs::home_dir().unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

/// `path` in `dir`, or `name-2.ext`, `name-3.ext` … when taken.
pub fn unique_in(dir: &Path, file_name: &str) -> PathBuf {
    let first = dir.join(file_name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (file_name.to_string(), String::new()),
    };
    (2..)
        .map(|n| dir.join(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .expect("an unused name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_names_and_home_expansion() {
        let dir = tempfile::tempdir().unwrap();
        let a = unique_in(dir.path(), "mode-export-2026-10-22.json");
        assert!(a.ends_with("mode-export-2026-10-22.json"));
        std::fs::write(&a, "{}").unwrap();
        let b = unique_in(dir.path(), "mode-export-2026-10-22.json");
        assert!(b.ends_with("mode-export-2026-10-22-2.json"));
        if let Some(h) = dirs::home_dir() {
            assert_eq!(expand_home("~/Notes"), h.join("Notes"));
        }
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
