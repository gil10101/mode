//! `mode-core`: everything that is not pixels (08 §3.2). A library with no
//! Tauri dependency; it compiles and is tested on every OS without a window
//! system. The modules below are the skeleton of 08 §3.2; each one carries the
//! minimal real behavior package 0 needs and grows in later packages.

pub mod agents;
pub mod calendar;
pub mod chat;
pub mod connectors;
pub mod db;
pub mod diag;
pub mod files;
pub mod hooks_db;
pub mod hooks_install;
pub mod ipc;
pub mod keyring;
pub mod links;
pub mod media_history;
pub mod notes;
pub mod packs;
pub mod palette;
pub mod paths;
pub mod pty;
pub mod push;
pub mod queries;
pub mod redact;
pub mod repo;
pub mod retention;
pub mod scheduler;
pub mod sessions;
pub mod settings;
pub mod speech;
pub mod stats;
pub mod themes;
pub mod transcript;
pub mod urgency;

use std::path::PathBuf;

/// The core's version, stamped into `schema_migrations.app_version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `<app_data_dir>` (08 §7): `Mode-dev` for dev builds so a dev build and the
/// installed app coexist (ADR-0034). macOS: `~/Library/Application
/// Support`, Windows: `%APPDATA%`, Linux: `$XDG_DATA_HOME` or `~/.local/share`.
pub fn data_dir(dev: bool) -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    base.join(if dev { "Mode-dev" } else { "Mode" })
}

/// Whether this build is a dev build (`-dev` data dir, `mode-dev` endpoint).
pub const fn is_dev_build() -> bool {
    cfg!(debug_assertions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_suffixed_for_dev() {
        assert!(data_dir(true).ends_with("Mode-dev"));
        assert!(data_dir(false).ends_with("Mode"));
    }
}
