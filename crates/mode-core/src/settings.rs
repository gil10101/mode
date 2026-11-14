//! Typed access to the `settings` table (09 §5.5): the key registry with a
//! scope and a default for every key, get / set / reset that stamp
//! `updated_at`, `rev` and `device_id`, typed getters, the retention bundle,
//! and the export / import bundle (03 §8.18: user-scope settings only unless
//! asked; device-scope values never leave the machine).
//!
//! A missing row means "default"; resetting deletes the row (the default is
//! the tombstone). `set` takes the scope from the registry; unknown keys are
//! refused unless written through [`set_scoped`].

use std::collections::BTreeMap;

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db::{Db, DbError, Result, now_rfc3339};
use crate::repo::{self, Setting};

/// `device.id`: a UUID v7 minted at first launch (09 §5.5).
pub const DEVICE_ID_KEY: &str = "device.id";

/// When the nightly sweep last completed (device scope; 09 §6.5 "if more than 24 h since the last run").
pub const LAST_SWEEP_KEY: &str = "maintenance.last_sweep_at";

/// The last synthetic hook round trip (`hooks_install::RoundTripReport`),
/// kept so the Claude Code card survives a restart (device scope; this build).
pub const LAST_ROUND_TRIP_KEY: &str = "hooks.last_round_trip";

/// Export bundle format version.
pub const BUNDLE_FORMAT: u32 = 1;
pub const BUNDLE_KIND: &str = "mode-settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Never leaves this machine.
    Device,
    User,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Device => "device",
            Scope::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "device" => Some(Scope::Device),
            "user" => Some(Scope::User),
            _ => None,
        }
    }
}

/// How a key's default is produced.
#[derive(Clone, Copy)]
pub enum DefaultValue {
    /// A JSON literal.
    Json(&'static str),
    /// Computed from the environment at read time.
    Computed(fn() -> Value),
}

/// One registered key.
#[derive(Clone, Copy)]
pub struct KeyDef {
    pub key: &'static str,
    pub scope: Scope,
    pub default: DefaultValue,
    /// The spec that owns the key (09 §5.5 "Owner doc").
    pub owner: &'static str,
}

impl KeyDef {
    pub fn default_value(&self) -> Value {
        match self.default {
            DefaultValue::Json(s) => serde_json::from_str(s).unwrap_or(Value::Null),
            DefaultValue::Computed(f) => f(),
        }
    }
}

impl std::fmt::Debug for KeyDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyDef")
            .field("key", &self.key)
            .field("scope", &self.scope)
            .field("default", &self.default_value())
            .finish()
    }
}

/// Environment-derived defaults (also used by the migration seeds).
pub mod defaults {
    use serde_json::Value;

    /// `device.name`: the hostname.
    pub fn device_name() -> String {
        if let Ok(h) = std::env::var("HOSTNAME")
            && !h.trim().is_empty()
        {
            return h.trim().to_string();
        }
        if let Ok(h) = std::env::var("COMPUTERNAME")
            && !h.trim().is_empty()
        {
            return h.trim().to_string();
        }
        #[cfg(unix)]
        {
            if let Ok(h) = std::fs::read_to_string("/etc/hostname")
                && !h.trim().is_empty()
            {
                return h.trim().to_string();
            }
            let mut buf = [0u8; 256];
            // SAFETY: gethostname writes at most `buf.len()` bytes into a valid buffer.
            let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
            if rc == 0 {
                let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
                if let Ok(s) = std::str::from_utf8(&buf[..end])
                    && !s.trim().is_empty()
                {
                    return s.trim().to_string();
                }
            }
        }
        "This device".to_string()
    }

    /// `profile.name`: the OS user's full name where exposed, else the login name.
    pub fn profile_name() -> String {
        let login = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .ok()
            .filter(|s| !s.trim().is_empty());
        #[cfg(unix)]
        if let Some(login) = &login
            && let Ok(passwd) = std::fs::read_to_string("/etc/passwd")
        {
            for line in passwd.lines() {
                let fields: Vec<&str> = line.split(':').collect();
                if fields.len() >= 5 && fields[0] == login {
                    let full = fields[4].split(',').next().unwrap_or("").trim();
                    if !full.is_empty() {
                        return full.to_string();
                    }
                }
            }
        }
        login.unwrap_or_else(|| "You".to_string())
    }

    /// `capture.hotkey`: ⌥Space on macOS, Ctrl+Alt+Space elsewhere (02 §9).
    pub fn capture_hotkey() -> &'static str {
        if cfg!(target_os = "macos") {
            "Alt+Space"
        } else {
            "Ctrl+Alt+Space"
        }
    }

    /// `overlay.toggle_hotkey`: ⌥⇧Space / Ctrl+Alt+Shift+Space.
    pub fn toggle_hotkey() -> &'static str {
        if cfg!(target_os = "macos") {
            "Alt+Shift+Space"
        } else {
            "Ctrl+Alt+Shift+Space"
        }
    }

    /// `overlay.pin_hotkey`: ⌥⇧P / Ctrl+Alt+Shift+P.
    pub fn pin_hotkey() -> &'static str {
        if cfg!(target_os = "macos") {
            "Alt+Shift+P"
        } else {
            "Ctrl+Alt+Shift+P"
        }
    }

    pub(super) fn device_name_json() -> Value {
        Value::String(device_name())
    }
    pub(super) fn profile_name_json() -> Value {
        Value::String(profile_name())
    }
    pub(super) fn capture_hotkey_json() -> Value {
        Value::String(capture_hotkey().into())
    }
    pub(super) fn toggle_hotkey_json() -> Value {
        Value::String(toggle_hotkey().into())
    }
    pub(super) fn pin_hotkey_json() -> Value {
        Value::String(pin_hotkey().into())
    }
    pub(super) fn null() -> Value {
        Value::Null
    }
}

macro_rules! key {
    ($key:literal, $scope:ident, json $default:literal, $owner:literal) => {
        KeyDef {
            key: $key,
            scope: Scope::$scope,
            default: DefaultValue::Json($default),
            owner: $owner,
        }
    };
    ($key:literal, $scope:ident, computed $f:path, $owner:literal) => {
        KeyDef {
            key: $key,
            scope: Scope::$scope,
            default: DefaultValue::Computed($f),
            owner: $owner,
        }
    };
}

/// The key inventory of 09 §5.5, in the document's order, plus
/// `maintenance.last_sweep_at` (device scope) for the nightly sweep.
pub static KEYS: &[KeyDef] = &[
    key!("device.id", Device, computed defaults::null, "09"),
    key!("device.name", Device, computed defaults::device_name_json, "09"),
    key!("notes.vault_path", Device, json r#""~/Mode/Notes""#, "04"),
    key!("notes.statuses", User, json r#"[{"slug":"todo","label":"Todo","glyph":"○","color":null,"closed":false},{"slug":"doing","label":"Doing","glyph":"◑","color":null,"closed":false},{"slug":"done","label":"Done","glyph":"✓","color":null,"closed":true}]"#, "04"),
    key!("notes.touchup_delay_seconds", User, json "30", "09"),
    key!("overlay.placement", Device, computed defaults::null, "02"),
    key!("overlay.pinned", Device, json "false", "02"),
    key!("overlay.peek_delay_ms", User, json "150", "02"),
    key!("capture.hotkey", Device, computed defaults::capture_hotkey_json, "03"),
    key!("sessions.fold_after_minutes", User, json "10", "05"),
    key!("sessions.store_prompt_previews", User, json "false", "09"),
    key!("sessions.terminal_app", Device, computed defaults::null, "05"),
    key!("polling.gmail_seconds", User, json "60", "06"),
    key!("polling.slack_seconds", User, json "30", "06"),
    key!("polling.slack_budget_per_minute", User, json "40", "06"),
    key!("ai.backend", User, json r#""claude_api""#, "06"),
    key!("ai.local_only", User, json "false", "06"),
    key!("ai.model_chat", User, json r#""claude-opus-5-5""#, "06"),
    key!("ai.model_scoring", User, json r#""claude-haiku-5-5""#, "07"),
    key!("ai.scoring_chars", User, json "500", "07"),
    key!("ai.budget_tokens_month", User, json "2000000", "07"),
    key!("urgency.keyword_rules", User, json "[]", "07"),
    key!("notifications.quiet_hours", User, json r#"{"enabled":false,"from":"22:00","to":"08:00"}"#, "07"),
    key!("retention.session_events_days", User, json "7", "09"),
    key!("retention.sessions_days", User, json "90", "09"),
    key!("retention.messages_days", User, json "90", "09"),
    key!("retention.message_body_days", User, json "7", "09"),
    key!("retention.usage_days", User, json "180", "09"),
    key!("retention.chat_days", User, json "90", "09"),
    key!("usage.budget_tokens_week", User, json "null", "05"),
    key!("usage.reset_weekday", User, json "null", "05"),
    key!("theme.active", Device, json r#""system""#, "10"),
    key!("theme.appearance", Device, json r#""auto""#, "10"),
    key!("theme.typography_style", User, json r#""pack""#, "10"),
    key!("theme.font_scale", User, json "1.0", "10"),
    key!("theme.terminal_pack", Device, json r#""follow""#, "10"),
    key!("sounds.enabled", User, json "false", "10"),
    key!("sounds.volume", User, json "60", "10"),
    key!("overlay.expand_on_hover", User, json "true", "02"),
    key!("overlay.expand_hover_ms", User, json "600", "02"),
    key!("overlay.collapse_delay_ms", User, json "2500", "02"),
    key!("overlay.auto_collapse_ms", User, json "8000", "02"),
    key!("overlay.pill_min_ms", User, json "2000", "02"),
    key!("overlay.max_rows", User, json "6", "02"),
    key!("overlay.default_tab", User, json r#""auto""#, "02"),
    key!("overlay.display", Device, json r#""main""#, "02"),
    key!("overlay.fullscreen", User, json r#""presentation-hides""#, "02"),
    key!("overlay.right_edge_dock", Device, json r#""float""#, "02"),
    key!("overlay.motion", User, json r#""system""#, "02"),
    key!("overlay.show_mascot", User, json "true", "02"),
    key!("overlay.scale", Device, json "100", "02"),
    key!("overlay.toggle_hotkey", Device, computed defaults::toggle_hotkey_json, "02"),
    key!("overlay.pin_hotkey", Device, computed defaults::pin_hotkey_json, "02"),
    key!("overlay.capture_sound", User, json "null", "02"),
    key!("overlay.idle", Device, json r#""mascot""#, "02"),
    key!("overlay.media", Device, json "true", "02"),
    key!("onboarding.steps", Device, json "{}", "03"),
    key!("onboarding.completed_at", Device, json "null", "03"),
    key!("window.bounds", Device, computed defaults::null, "03"),
    key!("window.sidebar_collapsed", Device, json "false", "03"),
    key!("window.detail_open", Device, json "true", "03"),
    key!("window.last_route", Device, json r#""sessions""#, "03"),
    key!("window.hotkey", Device, computed defaults::null, "03"),
    key!("general.launch_at_login", Device, json "false", "03"),
    key!("general.status_item", Device, json r#"{"badge":true,"pause_entry":true}"#, "03"),
    key!("general.compact_rows", Device, json "false", "03"),
    key!("general.auto_update", Device, json "true", "03"),
    key!("general.language", User, json r#""en""#, "03"),
    key!("sessions.filter", Device, json "{}", "03"),
    key!("sessions.group_by_repo", Device, json "false", "03"),
    key!("sessions.claude_path", Device, json "null", "05"),
    key!("sessions.custom_names", Device, json "{}", "05"),
    key!("hooks.decision_timeout_seconds", User, json "120", "05"),
    key!("hooks.async_timeout_seconds", User, json "5", "05"),
    key!("inbox.saved_filters", User, json "[]", "03"),
    key!("notes.sort", Device, json "{}", "03"),
    key!("notes.board_groupings", Device, json r#"["todo","doing","urgent","done"]"#, "03"),
    key!("notes.rename_with_title", User, json "false", "03"),
    key!("notes.touchup_enabled", User, json "true", "04"),
    key!("notes.week_start", User, json r#""monday""#, "04"),
    key!("notes.autosave_delay_ms", User, json "500", "04"),
    key!("notes.show_archived_in_search", User, json "false", "04"),
    key!("cards.open", Device, json "[]", "03"),
    key!("polling.calendar_seconds", User, json "300", "06"),
    key!("gmail.initial_sync_days", User, json "14", "06"),
    key!("gmail.initial_sync_max", User, json "500", "06"),
    key!("gmail.body_fetch", User, json r#""auto""#, "06"),
    key!("slack.initial_sync_days", User, json "7", "06"),
    key!("slack.search_mentions_seconds", User, json "60", "06"),
    key!("connectors.oauth_ports", Device, json "[48311,48312,48313,48314,48315]", "06"),
    key!("ai.local_url", User, json r#""http://127.0.0.1:11434""#, "06"),
    key!("ai.model_summary", User, json r#""claude-haiku-5-5""#, "06"),
    key!("ai.local_model_chat", User, computed defaults::null, "06"),
    key!("ai.local_model_scoring", User, computed defaults::null, "06"),
    key!("ai.local_model_summary", User, computed defaults::null, "06"),
    key!("ai.timeout_seconds", User, json r#"{"scoring":30,"summary":120,"chat":300}"#, "06"),
    key!("marketplace.index_url", User, json r#""https://raw.githubusercontent.com/gil10101/mode_cat/main/index.json""#, "03"),
    key!("urgency.direct_email_requires_known_contact", User, json "true", "07"),
    key!("urgency.pill_message_max_age_hours", User, json "24", "07"),
    key!("notifications.max_alert_age_minutes", User, json "30", "07"),
    key!("notifications.snooze_wake_on_reply", User, json "true", "07"),
    key!("notifications.pause_until", Device, json "null", "07"),
    key!("notifications.os_preview", User, json r#""full""#, "07"),
    key!("notifications.os_sound", User, json "false", "07"),
    key!("notifications.focus_detection", User, json r#"{"macos_assertions":true,"windows_wnf":true}"#, "07"),
    key!("profile.name", User, computed defaults::profile_name_json, "03"),
    key!("profile.picture_path", Device, json "null", "03"),
    key!("shortcuts.bindings", User, json "{}", "03"),
    key!("automations.catch_up_minutes", User, json "30", "03"),
    key!("dictation.enabled", Device, json "false", "03"),
    key!("dictation.hotkey", Device, json r#""Ctrl+Alt+D""#, "03"),
    key!("dictation.model", Device, json r#""base.en""#, "03"),
    key!("dictation.language", Device, json r#""auto""#, "03"),
    key!("dictation.commands", Device, json "true", "03"),
    key!("dictation.reply_card", Device, json "true", "03"),
    key!("dictation.unfocused", Device, json r#""capture""#, "03"),
    key!("dictation.thread_id", Device, json "null", "03"),
    key!("chat.sources_open", Device, json "false", "03"),
    key!("maintenance.last_sweep_at", Device, json "null", "09 §6.5 (this build)"),
    key!("hooks.last_round_trip", Device, json "null", "05 §3.6 (this build)"),
];

/// The registry entry for `key`.
pub fn key_def(key: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|k| k.key == key)
}

/// The default for `key` (`Null` for unknown keys).
pub fn default_for(key: &str) -> Value {
    key_def(key)
        .map(|k| k.default_value())
        .unwrap_or(Value::Null)
}

/// The registered scope of `key`.
pub fn scope_of(key: &str) -> Option<Scope> {
    key_def(key).map(|k| k.scope)
}

// ---------------------------------------------------------------------------
// Validation (the `settings_set` command)
// ---------------------------------------------------------------------------

/// Keys only the core writes; a webview may not set them.
pub const INTERNAL_KEYS: &[&str] = &[DEVICE_ID_KEY, LAST_SWEEP_KEY, LAST_ROUND_TRIP_KEY];

/// The global hotkey keys (02 §8.2, 03 §7D): a key chord string each; the
/// dictation hold key (XC-37) last.
pub const HOTKEY_KEYS: &[&str] = &[
    "capture.hotkey",
    "overlay.toggle_hotkey",
    "overlay.pin_hotkey",
    "window.hotkey",
    "dictation.hotkey",
];

fn int_in(value: &Value, lo: i64, hi: i64) -> std::result::Result<(), String> {
    match value.as_i64() {
        Some(n) if (lo..=hi).contains(&n) => Ok(()),
        Some(n) => Err(format!("{n} is outside {lo}–{hi}")),
        None => Err("expects a whole number".into()),
    }
}

fn one_of(value: &Value, allowed: &[&str]) -> std::result::Result<(), String> {
    match value.as_str() {
        Some(v) if allowed.contains(&v) => Ok(()),
        _ => Err(format!("expects one of {}", allowed.join(", "))),
    }
}

fn short_string(value: &Value, max_chars: usize) -> std::result::Result<(), String> {
    match value.as_str() {
        Some(v) if !v.trim().is_empty() && v.chars().count() <= max_chars => Ok(()),
        Some(_) => Err(format!("expects 1–{max_chars} characters")),
        None => Err("expects a string".into()),
    }
}

fn same_kind(a: &Value, b: &Value) -> bool {
    matches!(
        (a, b),
        (Value::Bool(_), Value::Bool(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::String(_), Value::String(_))
            | (Value::Array(_), Value::Array(_))
            | (Value::Object(_), Value::Object(_))
    )
}

/// Check a value a webview wants to store under `key`: the key must be
/// registered and not internal; the `overlay.*` keys, the hotkeys and
/// `sessions.fold_after_minutes` get the ranges and vocabularies of 02 §17;
/// every other key must keep the JSON kind of its default (any kind when the
/// default is `null`). The hotkey check here is structural only (a non-empty
/// chord of at most 64 characters); the app parses it for the OS.
pub fn validate(key: &str, value: &Value) -> std::result::Result<(), String> {
    let def = key_def(key).ok_or_else(|| format!("{key} is not a registered setting"))?;
    if INTERNAL_KEYS.contains(&key) {
        return Err(format!("{key} is managed by Mode"));
    }
    let checked = match key {
        "overlay.placement" if value.is_null() => Ok(()),
        "overlay.placement" => one_of(value, &["notch", "top-bar", "right-edge"]),
        "overlay.pinned" | "overlay.expand_on_hover" | "overlay.show_mascot" | "overlay.media" => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err("expects true or false".into())
            }
        }
        "overlay.peek_delay_ms" => int_in(value, 0, 1_000),
        "overlay.expand_hover_ms" => int_in(value, 300, 2_000),
        "overlay.collapse_delay_ms" => int_in(value, 500, 10_000),
        "overlay.auto_collapse_ms" if value.as_i64() == Some(0) => Ok(()),
        "overlay.auto_collapse_ms" => {
            int_in(value, 3_000, 60_000).map_err(|e| format!("{e} (or 0 for never)"))
        }
        "overlay.pill_min_ms" => int_in(value, 500, 10_000),
        "overlay.max_rows" => int_in(value, 4, 10),
        "overlay.default_tab" => one_of(
            value,
            &["auto", "sessions", "inbox", "notes", "calendar", "music"],
        ),
        "overlay.display" => short_string(value, 200),
        "overlay.fullscreen" => one_of(value, &["presentation-hides", "show", "hide"]),
        "overlay.right_edge_dock" => one_of(value, &["float", "appbar"]),
        "overlay.motion" => one_of(value, &["system", "reduced", "full"]),
        "overlay.scale" => int_in(value, 100, 150),
        "overlay.capture_sound" if value.is_null() => Ok(()),
        "overlay.capture_sound" => short_string(value, 64),
        "overlay.idle" => one_of(value, &["mascot", "hidden"]),
        "retention.chat_days" => int_in(value, 7, 3_650),
        "window.hotkey" if value.is_null() => Ok(()),
        k if HOTKEY_KEYS.contains(&k) => short_string(value, 64),
        "sessions.fold_after_minutes" => int_in(value, 1, 1_440),
        "dictation.enabled"
        | "dictation.commands"
        | "dictation.reply_card"
        | "chat.sources_open" => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err("expects true or false".into())
            }
        }
        "dictation.model" => one_of(value, &["tiny.en", "base.en", "small.en", "base"]),
        "dictation.language" => short_string(value, 16),
        "dictation.unfocused" => one_of(value, &["capture", "nothing"]),
        "dictation.thread_id" if value.is_null() => Ok(()),
        "dictation.thread_id" => short_string(value, 64),
        _ => {
            let default = def.default_value();
            if default.is_null() || value.is_null() || same_kind(&default, value) {
                Ok(())
            } else {
                Err("does not match the kind of its default".into())
            }
        }
    };
    checked.map_err(|e| format!("{key}: {e}"))
}

// ---------------------------------------------------------------------------
// Connection-level API (for use inside write jobs and reads)
// ---------------------------------------------------------------------------

/// The stored row for `key`.
pub fn stored_in(conn: &Connection, key: &str) -> Result<Option<Setting>> {
    repo::get_by_id::<Setting>(conn, key)
}

/// The stored value for `key`, or `None` when the default applies.
pub fn stored_value_in(conn: &Connection, key: &str) -> Result<Option<Value>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value_json FROM settings WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

/// The effective value: stored, else the registered default.
pub fn get_in(conn: &Connection, key: &str) -> Result<Value> {
    Ok(stored_value_in(conn, key)?.unwrap_or_else(|| default_for(key)))
}

/// Upsert `key` with its registered scope; an existing row keeps its scope and
/// gets `rev + 1`. Unknown keys are refused.
pub fn set_in(conn: &Connection, device_id: &str, key: &str, value: &Value) -> Result<()> {
    let scope = scope_of(key).ok_or_else(|| DbError::Invalid {
        what: "settings key",
        detail: format!("{key} is not a registered key"),
    })?;
    set_scoped_in(conn, device_id, key, value, scope)
}

/// Upsert `key` with an explicit scope (for keys other docs add later).
pub fn set_scoped_in(
    conn: &Connection,
    device_id: &str,
    key: &str,
    value: &Value,
    scope: Scope,
) -> Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value_json, scope, updated_at, rev, device_id)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)
         ON CONFLICT(key) DO UPDATE SET
           value_json = excluded.value_json,
           updated_at = excluded.updated_at,
           rev = settings.rev + 1,
           device_id = excluded.device_id",
        rusqlite::params![
            key,
            value.to_string(),
            scope.as_str(),
            now_rfc3339(),
            device_id
        ],
    )?;
    Ok(())
}

/// Delete the row so the default applies again; `true` when a row existed.
pub fn reset_in(conn: &Connection, key: &str) -> Result<bool> {
    Ok(conn.execute("DELETE FROM settings WHERE key = ?1", [key])? > 0)
}

/// Every stored row, by key.
pub fn all_in(conn: &Connection) -> Result<Vec<Setting>> {
    repo::query::<Setting>(conn, "1 ORDER BY key", &[])
}

/// Every registered key with its effective value (stored, else the
/// default), for the app window's `settings_get_all`.
pub fn all_effective_in(conn: &Connection) -> Result<BTreeMap<String, Value>> {
    let stored: BTreeMap<String, Value> = all_in(conn)?
        .into_iter()
        .filter_map(|row| {
            serde_json::from_str::<Value>(&row.value_json)
                .ok()
                .map(|v| (row.key, v))
        })
        .collect();
    Ok(KEYS
        .iter()
        .map(|k| {
            let v = stored
                .get(k.key)
                .cloned()
                .unwrap_or_else(|| k.default_value());
            (k.key.to_string(), v)
        })
        .collect())
}

/// The `rev` of a key, when stored.
pub fn rev_in(conn: &Connection, key: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT rev FROM settings WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?)
}

/// Typed read with the default as fallback for a missing or mistyped value.
pub fn get_as_in<T: serde::de::DeserializeOwned>(conn: &Connection, key: &str) -> Result<T> {
    let v = get_in(conn, key)?;
    match serde_json::from_value::<T>(v) {
        Ok(t) => Ok(t),
        Err(e) => match serde_json::from_value::<T>(default_for(key)) {
            Ok(t) => Ok(t),
            Err(_) => Err(DbError::Json(e)),
        },
    }
}

pub fn get_i64_in(conn: &Connection, key: &str) -> Result<Option<i64>> {
    Ok(get_in(conn, key)?.as_i64())
}

pub fn get_bool_in(conn: &Connection, key: &str) -> Result<Option<bool>> {
    Ok(get_in(conn, key)?.as_bool())
}

pub fn get_string_in(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(get_in(conn, key)?.as_str().map(str::to_string))
}

/// One entry of `notes.statuses`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusDef {
    pub slug: String,
    pub label: String,
    #[serde(default)]
    pub glyph: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub closed: bool,
}

/// `notes.statuses` as a list.
pub fn statuses_in(conn: &Connection) -> Result<Vec<StatusDef>> {
    get_as_in(conn, "notes.statuses")
}

/// The slugs of the statuses whose `closed` flag is set (`"closed"` in a filter).
pub fn closed_statuses_in(conn: &Connection) -> Result<Vec<String>> {
    Ok(statuses_in(conn)?
        .into_iter()
        .filter(|s| s.closed)
        .map(|s| s.slug)
        .collect())
}

/// The `retention.*` values (09 §10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retention {
    pub session_events_days: i64,
    pub sessions_days: i64,
    pub messages_days: i64,
    pub message_body_days: i64,
    pub usage_days: i64,
    /// Chat turns (and the conversations they leave empty), 03 §7.6.
    pub chat_days: i64,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            session_events_days: 7,
            sessions_days: 90,
            messages_days: 90,
            message_body_days: 7,
            usage_days: 180,
            chat_days: 90,
        }
    }
}

pub fn retention_in(conn: &Connection) -> Result<Retention> {
    let d = Retention::default();
    Ok(Retention {
        session_events_days: get_i64_in(conn, "retention.session_events_days")?
            .unwrap_or(d.session_events_days),
        sessions_days: get_i64_in(conn, "retention.sessions_days")?.unwrap_or(d.sessions_days),
        messages_days: get_i64_in(conn, "retention.messages_days")?.unwrap_or(d.messages_days),
        message_body_days: get_i64_in(conn, "retention.message_body_days")?
            .unwrap_or(d.message_body_days),
        usage_days: get_i64_in(conn, "retention.usage_days")?.unwrap_or(d.usage_days),
        chat_days: get_i64_in(conn, "retention.chat_days")?.unwrap_or(d.chat_days),
    })
}

// ---------------------------------------------------------------------------
// Db-level API
// ---------------------------------------------------------------------------

/// The effective value: stored, else the registered default.
pub fn get(db: &Db, key: &str) -> Result<Value> {
    let key = key.to_string();
    db.read(|c| get_in(c, &key))
}

/// The stored value for `key`, or `None` when the default applies.
pub fn stored(db: &Db, key: &str) -> Result<Option<Value>> {
    db.read(|c| stored_value_in(c, key))
}

/// Upsert `key` with its registered scope.
pub fn set(db: &Db, key: &str, value: &Value) -> Result<()> {
    let (key, value, device) = (key.to_string(), value.clone(), db.device_id().to_string());
    db.write(move |c| set_in(c, &device, &key, &value))
}

/// Upsert `key` with an explicit scope.
pub fn set_scoped(db: &Db, key: &str, value: &Value, scope: Scope) -> Result<()> {
    let (key, value, device) = (key.to_string(), value.clone(), db.device_id().to_string());
    db.write(move |c| set_scoped_in(c, &device, &key, &value, scope))
}

/// Delete the row so the default applies again; `true` when a row existed.
pub fn reset(db: &Db, key: &str) -> Result<bool> {
    let key = key.to_string();
    db.write(move |c| reset_in(c, &key))
}

/// Every registered key with its effective value.
pub fn all_effective(db: &Db) -> Result<BTreeMap<String, Value>> {
    db.read(all_effective_in)
}

/// The `rev` of a key, when stored.
pub fn rev(db: &Db, key: &str) -> Result<Option<i64>> {
    db.read(|c| rev_in(c, key))
}

pub fn get_as<T: serde::de::DeserializeOwned>(db: &Db, key: &str) -> Result<T> {
    db.read(|c| get_as_in(c, key))
}

pub fn get_i64(db: &Db, key: &str) -> Result<Option<i64>> {
    db.read(|c| get_i64_in(c, key))
}

pub fn get_bool(db: &Db, key: &str) -> Result<Option<bool>> {
    db.read(|c| get_bool_in(c, key))
}

pub fn get_string(db: &Db, key: &str) -> Result<Option<String>> {
    db.read(|c| get_string_in(c, key))
}

pub fn retention(db: &Db) -> Result<Retention> {
    db.read(retention_in)
}

/// This device's id (minted at open).
pub fn device_id(db: &Db) -> String {
    db.device_id().to_string()
}

// ---------------------------------------------------------------------------
// Export / import bundle (03 §8.18)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleEntry {
    pub value: Value,
    pub scope: Scope,
    pub rev: i64,
    pub updated_at: String,
}

/// `mode-export-<date>.json`, settings section: user-scope keys by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
    pub format: u32,
    pub kind: String,
    pub exported_at: String,
    pub app_version: String,
    pub settings: BTreeMap<String, BundleEntry>,
}

/// Export stored settings; device-scope rows only when `include_device`
/// (`device.id` never).
pub fn export_in(conn: &Connection, include_device: bool) -> Result<Bundle> {
    let mut settings = BTreeMap::new();
    for row in all_in(conn)? {
        let scope = Scope::parse(&row.scope).unwrap_or(Scope::User);
        if row.key == DEVICE_ID_KEY || (scope == Scope::Device && !include_device) {
            continue;
        }
        settings.insert(
            row.key.clone(),
            BundleEntry {
                value: serde_json::from_str(&row.value_json)?,
                scope,
                rev: row.rev,
                updated_at: row.updated_at,
            },
        );
    }
    Ok(Bundle {
        format: BUNDLE_FORMAT,
        kind: BUNDLE_KIND.into(),
        exported_at: now_rfc3339(),
        app_version: crate::VERSION.into(),
        settings,
    })
}

pub fn export(db: &Db, include_device: bool) -> Result<Bundle> {
    db.read(|c| export_in(c, include_device))
}

/// The merge preview row of 03 §8.18.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "outcome", content = "reason")]
pub enum ImportOutcome {
    New,
    Updated,
    Unchanged,
    Skipped(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub applied: bool,
    pub entries: BTreeMap<String, ImportOutcome>,
}

impl ImportReport {
    pub fn count(&self, pred: impl Fn(&ImportOutcome) -> bool) -> usize {
        self.entries.values().filter(|o| pred(o)).count()
    }
}

/// Import a bundle. With `apply = false` only the preview is computed. Unknown
/// keys and (unless `include_device`) device-scope keys are skipped; a value
/// equal to the stored one is unchanged; otherwise the row is written through
/// [`set_scoped_in`] (so it gets `rev + 1` and a `change_log` entry).
pub fn import_in(
    conn: &Connection,
    device_id: &str,
    bundle: &Bundle,
    include_device: bool,
    apply: bool,
) -> Result<ImportReport> {
    if bundle.kind != BUNDLE_KIND {
        return Err(DbError::Invalid {
            what: "settings bundle",
            detail: format!("kind {:?} is not {BUNDLE_KIND}", bundle.kind),
        });
    }
    if bundle.format > BUNDLE_FORMAT {
        return Err(DbError::Invalid {
            what: "settings bundle",
            detail: format!(
                "format {} needs a newer Mode (this build reads {BUNDLE_FORMAT})",
                bundle.format
            ),
        });
    }
    let mut report = ImportReport {
        applied: apply,
        entries: BTreeMap::new(),
    };
    for (key, entry) in &bundle.settings {
        let Some(def) = key_def(key) else {
            report
                .entries
                .insert(key.clone(), ImportOutcome::Skipped("unknown key".into()));
            continue;
        };
        if key == DEVICE_ID_KEY {
            report.entries.insert(
                key.clone(),
                ImportOutcome::Skipped("device id never imported".into()),
            );
            continue;
        }
        if def.scope == Scope::Device && !include_device {
            report
                .entries
                .insert(key.clone(), ImportOutcome::Skipped("device scope".into()));
            continue;
        }
        let outcome = match stored_value_in(conn, key)? {
            Some(current) if current == entry.value => ImportOutcome::Unchanged,
            Some(_) => ImportOutcome::Updated,
            None => ImportOutcome::New,
        };
        if apply && matches!(outcome, ImportOutcome::New | ImportOutcome::Updated) {
            set_scoped_in(conn, device_id, key, &entry.value, def.scope)?;
        }
        report.entries.insert(key.clone(), outcome);
    }
    Ok(report)
}

pub fn import(db: &Db, bundle: &Bundle, include_device: bool, apply: bool) -> Result<ImportReport> {
    let (bundle, device) = (bundle.clone(), db.device_id().to_string());
    db.write(move |c| import_in(c, &device, &bundle, include_device, apply))
}

/// A bundle as a JSON document.
pub fn bundle_to_json(bundle: &Bundle) -> Result<String> {
    Ok(serde_json::to_string_pretty(bundle)?)
}

pub fn bundle_from_json(text: &str) -> Result<Bundle> {
    Ok(serde_json::from_str(text)?)
}

/// `json!`-style helper for callers that set values from Rust literals.
pub fn value<T: Serialize>(v: T) -> Value {
    json!(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_applies_the_overlay_ranges_and_vocabularies() {
        assert!(validate("overlay.peek_delay_ms", &json!(150)).is_ok());
        assert!(validate("overlay.peek_delay_ms", &json!(1_001)).is_err());
        assert!(validate("overlay.peek_delay_ms", &json!(150.5)).is_err());
        assert!(validate("overlay.auto_collapse_ms", &json!(0)).is_ok());
        assert!(validate("overlay.auto_collapse_ms", &json!(2_000)).is_err());
        assert!(validate("overlay.auto_collapse_ms", &json!(60_000)).is_ok());
        assert!(validate("overlay.max_rows", &json!(10)).is_ok());
        assert!(validate("overlay.max_rows", &json!(3)).is_err());
        assert!(validate("overlay.placement", &json!("right-edge")).is_ok());
        assert!(validate("overlay.placement", &Value::Null).is_ok());
        assert!(validate("overlay.placement", &json!("left-edge")).is_err());
        assert!(validate("overlay.pinned", &json!(true)).is_ok());
        assert!(validate("overlay.pinned", &json!("yes")).is_err());
        assert!(validate("overlay.media", &json!(false)).is_ok());
        assert!(validate("overlay.media", &json!("off")).is_err());
        assert!(validate("overlay.default_tab", &json!("notes")).is_ok());
        assert!(validate("overlay.fullscreen", &json!("show")).is_ok());
        assert!(validate("overlay.motion", &json!("slow")).is_err());
        assert!(validate("overlay.scale", &json!(125)).is_ok());
        assert!(validate("overlay.scale", &json!(200)).is_err());
        assert!(validate("overlay.display", &json!("cursor")).is_ok());
        assert!(validate("overlay.display", &json!("")).is_err());
        assert!(validate("sessions.fold_after_minutes", &json!(0)).is_err());
        assert!(validate("overlay.idle", &json!("mascot")).is_ok());
        assert!(validate("overlay.idle", &json!("hidden")).is_ok());
        assert!(validate("overlay.idle", &json!("eyes")).is_err());
        assert!(validate("chat.sources_open", &json!(true)).is_ok());
        assert!(validate("chat.sources_open", &json!("open")).is_err());
    }

    #[test]
    fn chat_retention_defaults_to_90_days_within_7_to_3650() {
        assert_eq!(default_for("retention.chat_days"), json!(90));
        assert_eq!(scope_of("retention.chat_days"), Some(Scope::User));
        assert_eq!(default_for("overlay.idle"), json!("mascot"));
        assert_eq!(scope_of("overlay.idle"), Some(Scope::Device));
        assert_eq!(default_for("chat.sources_open"), json!(false));
        assert_eq!(scope_of("chat.sources_open"), Some(Scope::Device));
        assert!(validate("retention.chat_days", &json!(7)).is_ok());
        assert!(validate("retention.chat_days", &json!(3_650)).is_ok());
        assert!(validate("retention.chat_days", &json!(6)).is_err());
        assert!(validate("retention.chat_days", &json!(3_651)).is_err());
        assert!(validate("retention.chat_days", &json!(30.5)).is_err());
        assert_eq!(Retention::default().chat_days, 90);
    }

    #[test]
    fn validate_checks_hotkeys_kinds_and_refuses_unknown_or_internal_keys() {
        assert!(validate("capture.hotkey", &json!("Ctrl+Alt+Space")).is_ok());
        assert!(validate("overlay.pin_hotkey", &json!("")).is_err());
        assert!(validate("overlay.toggle_hotkey", &json!(5)).is_err());
        assert!(validate("window.hotkey", &Value::Null).is_ok());
        assert!(
            validate("nope.key", &json!(1))
                .unwrap_err()
                .contains("not a registered")
        );
        assert!(
            validate(DEVICE_ID_KEY, &json!("x"))
                .unwrap_err()
                .contains("managed")
        );
        assert!(validate(LAST_SWEEP_KEY, &json!("x")).is_err());
        // Generic keys keep the kind of their default.
        assert!(validate("sounds.enabled", &json!(true)).is_ok());
        assert!(validate("sounds.enabled", &json!("on")).is_err());
        assert!(validate("notes.vault_path", &json!("~/Notes")).is_ok());
        // A null default takes any kind.
        assert!(validate("sessions.terminal_app", &json!("iterm2")).is_ok());
    }

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::open(&dir.path().join("mode.db")).unwrap();
        (dir, db)
    }

    #[test]
    fn registry_has_no_duplicates_and_parses_every_default() {
        let mut seen = std::collections::BTreeSet::new();
        for k in KEYS {
            assert!(seen.insert(k.key), "duplicate key {}", k.key);
            let _ = k.default_value();
            assert!(
                k.key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_')
            );
        }
        assert_eq!(default_for("sessions.fold_after_minutes"), json!(10));
        assert_eq!(default_for("ai.budget_tokens_month"), json!(2_000_000));
        assert_eq!(default_for("usage.budget_tokens_week"), Value::Null);
        assert_eq!(default_for("shortcuts.bindings"), json!({}));
        assert_eq!(default_for("automations.catch_up_minutes"), json!(30));
        assert_eq!(default_for("profile.picture_path"), Value::Null);
        assert!(default_for("profile.name").is_string());
        assert_eq!(scope_of("device.name"), Some(Scope::Device));
        assert_eq!(scope_of("notes.statuses"), Some(Scope::User));
        assert_eq!(default_for("nope"), Value::Null);
    }

    #[test]
    fn set_get_reset_round_trip_with_stamping() {
        let (_d, db) = db();
        // Seeded: the migration wrote the default row.
        assert_eq!(stored(&db, "overlay.pinned").unwrap(), Some(json!(false)));
        assert_eq!(rev(&db, "overlay.pinned").unwrap(), Some(1));
        set(&db, "overlay.pinned", &json!(true)).unwrap();
        assert_eq!(get(&db, "overlay.pinned").unwrap(), json!(true));
        assert_eq!(rev(&db, "overlay.pinned").unwrap(), Some(2));
        assert!(reset(&db, "overlay.pinned").unwrap());
        assert!(!reset(&db, "overlay.pinned").unwrap());
        assert_eq!(stored(&db, "overlay.pinned").unwrap(), None);
        assert_eq!(get(&db, "overlay.pinned").unwrap(), json!(false), "default");

        // Not seeded (no literal default): missing row, computed default.
        assert_eq!(stored(&db, "window.bounds").unwrap(), None);
        assert_eq!(get(&db, "window.bounds").unwrap(), Value::Null);
        set(&db, "window.bounds", &json!({"x": 1})).unwrap();
        assert_eq!(rev(&db, "window.bounds").unwrap(), Some(1));
        let row = db.read(|c| stored_in(c, "window.bounds")).unwrap().unwrap();
        assert_eq!(row.scope, "device");
        assert_eq!(row.device_id, db.device_id());
        assert_eq!(row.updated_at.len(), 24);

        assert!(
            set(&db, "made.up", &json!(1)).is_err(),
            "unknown keys refused"
        );
        set_scoped(&db, "made.up", &json!(1), Scope::User).unwrap();
        assert_eq!(get(&db, "made.up").unwrap(), json!(1));
    }

    #[test]
    fn typed_getters_and_retention() {
        let (_d, db) = db();
        assert_eq!(
            get_i64(&db, "sessions.fold_after_minutes").unwrap(),
            Some(10)
        );
        assert_eq!(get_bool(&db, "sounds.enabled").unwrap(), Some(false));
        assert_eq!(
            get_string(&db, "ai.backend").unwrap().as_deref(),
            Some("claude_api")
        );
        let statuses: Vec<StatusDef> = get_as(&db, "notes.statuses").unwrap();
        assert_eq!(statuses.len(), 3);
        assert!(statuses[2].closed);
        assert_eq!(
            db.read(closed_statuses_in).unwrap(),
            vec!["done".to_string()]
        );
        assert_eq!(retention(&db).unwrap(), Retention::default());
        set(&db, "retention.sessions_days", &json!(30)).unwrap();
        assert_eq!(retention(&db).unwrap().sessions_days, 30);
        set(&db, "retention.chat_days", &json!(14)).unwrap();
        assert_eq!(retention(&db).unwrap().chat_days, 14);
        // A mistyped stored value falls back to the default.
        set(&db, "retention.usage_days", &json!("lots")).unwrap();
        assert_eq!(retention(&db).unwrap().usage_days, 180);
        let s: Vec<StatusDef> = {
            set(&db, "notes.statuses", &json!("broken")).unwrap();
            get_as(&db, "notes.statuses").unwrap()
        };
        assert_eq!(
            s.len(),
            3,
            "default statuses when the stored value is unusable"
        );
    }

    #[test]
    fn export_import_round_trip_and_preview() {
        let (_d, a) = db();
        set(&a, "sounds.volume", &json!(80)).unwrap();
        set(&a, "shortcuts.bindings", &json!({"capture": "F9"})).unwrap();
        set(&a, "theme.active", &json!("paper")).unwrap(); // device scope
        let bundle = export(&a, false).unwrap();
        assert_eq!(bundle.kind, BUNDLE_KIND);
        assert!(bundle.settings.contains_key("sounds.volume"));
        assert!(
            bundle.settings.contains_key("ai.budget_tokens_month"),
            "seeded user keys export too"
        );
        assert!(
            !bundle.settings.contains_key("theme.active"),
            "device scope stays home"
        );
        assert!(!bundle.settings.contains_key(DEVICE_ID_KEY));
        let with_device = export(&a, true).unwrap();
        assert!(with_device.settings.contains_key("theme.active"));
        assert!(
            !with_device.settings.contains_key(DEVICE_ID_KEY),
            "never the device id"
        );

        let text = bundle_to_json(&bundle).unwrap();
        let parsed = bundle_from_json(&text).unwrap();
        assert_eq!(parsed, bundle);

        let (_e, b) = db();
        reset(&b, "sounds.volume").unwrap();
        let preview = import(&b, &parsed, false, false).unwrap();
        assert!(!preview.applied);
        assert_eq!(preview.entries["sounds.volume"], ImportOutcome::New);
        assert_eq!(
            preview.entries["shortcuts.bindings"],
            ImportOutcome::Updated
        );
        assert_eq!(
            preview.entries["ai.budget_tokens_month"],
            ImportOutcome::Unchanged
        );
        assert_eq!(
            get(&b, "sounds.volume").unwrap(),
            json!(60),
            "preview writes nothing"
        );

        let report = import(&b, &parsed, false, true).unwrap();
        assert!(report.applied);
        assert_eq!(get(&b, "sounds.volume").unwrap(), json!(80));
        assert_eq!(
            get(&b, "shortcuts.bindings").unwrap(),
            json!({"capture": "F9"})
        );
        assert_eq!(rev(&b, "shortcuts.bindings").unwrap(), Some(2));
        assert_eq!(
            report.count(|o| matches!(o, ImportOutcome::Unchanged)),
            parsed.settings.len() - 2
        );

        let mut foreign = parsed.clone();
        foreign.settings.insert(
            "evil.key".into(),
            BundleEntry {
                value: json!(1),
                scope: Scope::User,
                rev: 1,
                updated_at: "t".into(),
            },
        );
        foreign.settings.insert(
            "theme.active".into(),
            BundleEntry {
                value: json!("nord"),
                scope: Scope::Device,
                rev: 1,
                updated_at: "t".into(),
            },
        );
        let r = import(&b, &foreign, false, true).unwrap();
        assert!(matches!(r.entries["evil.key"], ImportOutcome::Skipped(_)));
        assert!(matches!(
            r.entries["theme.active"],
            ImportOutcome::Skipped(_)
        ));
        assert_eq!(get(&b, "theme.active").unwrap(), json!("system"));
        let r = import(&b, &foreign, true, true).unwrap();
        assert_eq!(r.entries["theme.active"], ImportOutcome::Updated);
        assert_eq!(get(&b, "theme.active").unwrap(), json!("nord"));

        let mut newer = parsed.clone();
        newer.format = 99;
        assert!(import(&b, &newer, false, false).is_err());
    }

    #[test]
    fn all_effective_resolves_every_registered_key() {
        let (_d, db) = db();
        set(&db, "sounds.volume", &json!(80)).unwrap();
        reset(&db, "overlay.pinned").unwrap();
        set_scoped(&db, "made.up", &json!(1), Scope::User).unwrap();
        let all = all_effective(&db).unwrap();
        assert_eq!(all.len(), KEYS.len());
        assert_eq!(all["sounds.volume"], json!(80), "stored wins");
        assert_eq!(all["overlay.pinned"], json!(false), "default after a reset");
        assert!(all["profile.name"].is_string(), "computed default");
        assert_eq!(all[LAST_ROUND_TRIP_KEY], Value::Null);
        assert!(
            !all.contains_key("made.up"),
            "only registered keys, even when an unknown row exists"
        );
        assert!(
            validate(LAST_ROUND_TRIP_KEY, &json!({})).is_err(),
            "internal"
        );
        assert_eq!(scope_of(LAST_ROUND_TRIP_KEY), Some(Scope::Device));
    }

    #[test]
    fn the_reply_card_switch_is_a_device_bool_on_by_default() {
        assert_eq!(default_for("dictation.reply_card"), json!(true));
        assert_eq!(scope_of("dictation.reply_card"), Some(Scope::Device));
        assert!(validate("dictation.reply_card", &json!(false)).is_ok());
        assert!(validate("dictation.reply_card", &json!("off")).is_err());
    }

    #[test]
    fn environment_defaults_are_non_empty() {
        assert!(!defaults::device_name().is_empty());
        assert!(!defaults::profile_name().is_empty());
        assert!(defaults::capture_hotkey().contains("Space"));
        assert!(defaults::pin_hotkey().ends_with('P'));
    }
}
