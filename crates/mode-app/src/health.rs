//! `health.changed` (02 §14, §16; contract `HealthChanged` + `AppHealth` /
//! `SettingsHealth`): offline, paused and broken accounts, the hook relay's
//! install status, a missing vault, a read-only or newer database, Claude
//! Code on this machine, the backups folder, this week's Claude Code tokens
//! and whether onboarding ever finished. Pushed on launch, after the notes
//! launch scan, after hook install / uninstall / re-inspection, and by the
//! 5-minute relay check; `health_get` answers windows that open later.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mode_core::hooks_install::{InstallStatus, Status};
use mode_core::push::Event;
use mode_core::queries::{self, Clock};
use mode_core::repo::{self, Note};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::state::CoreState;

/// The relay re-verification cadence (05 §3.6, §6A.1: the relay reads
/// `failed` after 5 minutes without a frame).
pub const RELAY_CHECK: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthChanged {
    /// The connectors' reachability (08 §11; `os_signals`).
    pub offline: bool,
    pub paused_accounts: Vec<String>,
    /// `installed` | `stale` | `missing` | `conflict` | `none`.
    pub hooks: &'static str,
    pub vault_missing: bool,
    pub read_only: bool,
    /// `needs_reauth` / `error` / `revoked` accounts, by label.
    pub broken_accounts: Vec<String>,
    /// `found` | `missing`: `missing` when neither `~/.claude` nor a `claude`
    /// binary exists (03 §2.7).
    pub claude_code: &'static str,
    /// `claude --version` (`2.1.305`), when the binary answered.
    pub claude_code_version: Option<String>,
    /// The database's `user_version` is ahead of this build (XC-33).
    pub db_newer: bool,
    /// Where the pre-migration backups are (the "update Mode" screen).
    pub backups_dir: String,
    /// Claude Code tokens this usage week (05 §13; the profile row's second
    /// line). Tokens only, never currency.
    pub tokens_week: i64,
    /// Onboarding never finished (`onboarding.completed_at` unset, 03 §2.8).
    pub first_run: bool,
}

/// The contract's `HookStatus` word for an install status.
pub fn hooks_word(status: Option<&Status>) -> &'static str {
    match status.map(|s| s.status) {
        Some(InstallStatus::Installed) => "installed",
        Some(InstallStatus::Stale) => "stale",
        Some(InstallStatus::Missing) => "missing",
        Some(InstallStatus::Conflict) => "conflict",
        Some(InstallStatus::Removed | InstallStatus::NotInstalled) | None => "none",
    }
}

fn expand_home(path: &str) -> Option<PathBuf> {
    match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map(|h| h.join(rest)),
        None if path == "~" => dirs::home_dir(),
        None => Some(PathBuf::from(path)),
    }
}

/// A vault is missing when it was in use (notes are indexed) or the user
/// chose a location other than the default, and the folder is not there:
/// moved or unmounted (02 §14). The default `~/Mode/Notes` before any note
/// exists is created by the notes package on first use, so its absence is
/// not a missing vault (the migration seeds the setting, so "stored" does
/// not mean "chosen").
pub fn vault_missing(
    value: &serde_json::Value,
    default: &serde_json::Value,
    notes_indexed: bool,
) -> bool {
    let Some(path) = value.as_str().and_then(expand_home) else {
        return false;
    };
    (notes_indexed || value != default) && !path.is_dir()
}

/// The weekday a usage week starts on, SQLite numbering (0 = Sunday …
/// 6 = Saturday): `usage.reset_weekday` (a weekday name, or 0–6 with
/// 0 = Sunday, or 7 = Sunday), else `notes.week_start`, else Monday.
pub fn week_start_day(reset_weekday: &serde_json::Value, week_start: &serde_json::Value) -> u8 {
    fn named(v: &serde_json::Value) -> Option<u8> {
        if let Some(n) = v.as_i64() {
            return (0..=7).contains(&n).then_some((n % 7) as u8);
        }
        let s = v.as_str()?.trim().to_ascii_lowercase();
        let day = match s.get(..3)? {
            "sun" => 0,
            "mon" => 1,
            "tue" => 2,
            "wed" => 3,
            "thu" => 4,
            "fri" => 5,
            "sat" => 6,
            _ => return None,
        };
        Some(day)
    }
    named(reset_weekday)
        .or_else(|| named(week_start))
        .unwrap_or(1)
}

/// Claude Code tokens since the start of this usage week.
pub fn tokens_week(db: &mode_core::db::Db) -> i64 {
    db.read(|c| {
        let get = |k: &str| {
            mode_core::settings::get_in(c, k)
                .unwrap_or_else(|_| mode_core::settings::default_for(k))
        };
        let wd = week_start_day(&get("usage.reset_weekday"), &get("notes.week_start"));
        let clock = Clock::current(c)?;
        let start = queries::usage_week_start(c, &clock.now, wd)?;
        queries::claude_code_tokens_since(c, &start)
    })
    .unwrap_or(0)
}

/// Compute the payload (blocking: database reads, maybe `claude --version`).
pub fn compute(core: &CoreState) -> HealthChanged {
    // `offline`, `paused_accounts`, `broken_accounts` are the connectors'.
    let offline = core.connectors.get().is_some_and(|c| c.is_offline());
    let accounts = core
        .db
        .read(|c| mode_core::connectors::health::health_fields(c, offline))
        .unwrap_or(mode_core::connectors::health::HealthFields {
            offline,
            ..Default::default()
        });
    let vault = mode_core::settings::get(&core.db, "notes.vault_path").unwrap_or_default();
    let vault_default = mode_core::settings::default_for("notes.vault_path");
    let notes_indexed = core
        .db
        .read(|c| repo::count::<Note>(c, "1", &[]))
        .is_ok_and(|n| n > 0);
    let hooks = CoreState::lock(&core.hooks).clone();
    // The notes service knows (launch scan, every scan since); without it
    // (a newer database) the settings heuristic stands in.
    let notes_missing = core.notes.get().map(|n| n.vault_status().missing);
    let db_newer = core.is_db_newer();
    let claude = crate::claude::probe(core);
    let onboarded = mode_core::settings::get(&core.db, "onboarding.completed_at")
        .map(|v| !v.is_null())
        .unwrap_or(false);
    HealthChanged {
        offline: accounts.offline,
        paused_accounts: accounts.paused_accounts,
        hooks: hooks_word(hooks.as_ref()),
        vault_missing: !db_newer
            && notes_missing
                .unwrap_or_else(|| vault_missing(&vault, &vault_default, notes_indexed)),
        read_only: core.db.is_read_only(),
        broken_accounts: accounts.broken_accounts,
        claude_code: claude.word(),
        claude_code_version: claude.version,
        db_newer,
        backups_dir: core.backups_dir().to_string_lossy().into_owned(),
        tokens_week: if db_newer { 0 } else { tokens_week(&core.db) },
        first_run: !db_newer && !onboarded,
    }
}

/// Compute `health.changed` and remember it (the publisher's payload for
/// [`Event::Health`] requests, e.g. the connectors' offline changes).
pub fn payload(core: &CoreState) -> Option<serde_json::Value> {
    let h = compute(core);
    *CoreState::lock(&core.last_health) = Some(h.clone());
    serde_json::to_value(h).ok()
}

/// Compute and push `health.changed` to both windows.
pub async fn emit(app: AppHandle, core: Arc<CoreState>) {
    let c = Arc::clone(&core);
    match tauri::async_runtime::spawn_blocking(move || compute(&c)).await {
        Ok(h) => {
            tracing::info!(
                hooks = h.hooks,
                read_only = h.read_only,
                vault_missing = h.vault_missing,
                claude_code = h.claude_code,
                db_newer = h.db_newer,
                first_run = h.first_run,
                tokens_week = h.tokens_week,
                "health.changed"
            );
            *CoreState::lock(&core.last_health) = Some(h.clone());
            if let Err(e) = app.emit(Event::Health.wire_name(), h) {
                tracing::warn!(error = %e, "emit health.changed failed");
            }
        }
        Err(e) => tracing::warn!(error = %e, "health task failed"),
    }
}

/// Re-inspect the hook install every 5 minutes; push `health.changed` when
/// the hook word changed. The first run happens at launch.
pub async fn relay_check_task(app: AppHandle, core: Arc<CoreState>) {
    loop {
        let before = hooks_word(CoreState::lock(&core.hooks).as_ref());
        let c = Arc::clone(&core);
        let _ = tauri::async_runtime::spawn_blocking(move || crate::commands::reinspect_hooks(&c))
            .await;
        let after = hooks_word(CoreState::lock(&core.hooks).as_ref());
        let first = CoreState::lock(&core.last_health).is_none();
        if first || before != after {
            emit(app.clone(), Arc::clone(&core)).await;
        }
        tokio::time::sleep(RELAY_CHECK).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hook_words_follow_the_contract() {
        assert_eq!(hooks_word(None), "none");
        let mut s = Status {
            installed: true,
            status: InstallStatus::Installed,
            version: None,
            path: PathBuf::from("/x"),
            relay_path: None,
            conflicts: vec![],
            events: vec![],
            block_hash: None,
            detail: None,
            last_round_trip: None,
        };
        assert_eq!(hooks_word(Some(&s)), "installed");
        for (st, word) in [
            (InstallStatus::Stale, "stale"),
            (InstallStatus::Missing, "missing"),
            (InstallStatus::Conflict, "conflict"),
            (InstallStatus::Removed, "none"),
            (InstallStatus::NotInstalled, "none"),
        ] {
            s.status = st;
            assert_eq!(hooks_word(Some(&s)), word);
        }
    }

    #[test]
    fn a_vault_is_missing_only_once_chosen_or_used() {
        let default = json!("~/Mode/Notes-that-does-not-exist");
        assert!(
            !vault_missing(&default, &default, false),
            "default, unused: created on first use"
        );
        assert!(vault_missing(&default, &default, true), "in use and gone");
        let dir = json!(std::env::temp_dir().to_string_lossy());
        assert!(!vault_missing(&dir, &default, true), "present");
        assert!(
            vault_missing(&json!("/definitely/not/a/vault/here"), &default, false),
            "chosen and gone"
        );
        assert!(!vault_missing(&json!(null), &default, true));
    }

    #[test]
    fn payload_shape_matches_the_contract() {
        let h = HealthChanged {
            offline: false,
            paused_accounts: vec!["Google (work)".into()],
            hooks: "stale",
            vault_missing: false,
            read_only: true,
            broken_accounts: vec![],
            claude_code: "found",
            claude_code_version: Some("2.1.305".into()),
            db_newer: false,
            backups_dir: "/d/Mode/backups".into(),
            tokens_week: 1_200_000,
            first_run: true,
        };
        assert_eq!(
            serde_json::to_value(h).unwrap(),
            json!({
                "offline": false,
                "paused_accounts": ["Google (work)"],
                "hooks": "stale",
                "vault_missing": false,
                "read_only": true,
                "broken_accounts": [],
                "claude_code": "found",
                "claude_code_version": "2.1.305",
                "db_newer": false,
                "backups_dir": "/d/Mode/backups",
                "tokens_week": 1_200_000,
                "first_run": true
            })
        );
    }

    #[test]
    fn usage_weeks_start_on_the_reset_weekday_else_the_notes_week() {
        assert_eq!(week_start_day(&json!(null), &json!("monday")), 1);
        assert_eq!(week_start_day(&json!(null), &json!("sunday")), 0);
        assert_eq!(week_start_day(&json!("Thursday"), &json!("monday")), 4);
        assert_eq!(week_start_day(&json!("sat"), &json!(null)), 6);
        assert_eq!(week_start_day(&json!(7), &json!("monday")), 0);
        assert_eq!(week_start_day(&json!(2), &json!("monday")), 2);
        assert_eq!(week_start_day(&json!(9), &json!("garbage")), 1);
    }
}
