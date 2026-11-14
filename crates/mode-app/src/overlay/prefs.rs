//! The settings the overlay window and its webview read (02 §17): the
//! window-side snapshot ([`Prefs`]) and the `settings.changed` payload, a
//! partial `OverlaySettings` object (`ui/src/state/contract.ts`).

use serde_json::{Map, Value};

use super::geometry::{DisplayRule, Placement};
use super::ladder::Fullscreen;

/// `overlay.motion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    System,
    Reduced,
    Full,
}

/// `theme.appearance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Auto,
    Light,
    Dark,
}

/// What the native side needs from the settings table.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// `None`: not chosen yet (notch where there is one, else top-bar).
    pub placement: Option<Placement>,
    pub display: DisplayRule,
    pub fullscreen: Fullscreen,
    /// `overlay.scale` / 100: the webview's zoom (02 §13), sent as `display.changed.scale`.
    pub zoom: f64,
    pub motion: Motion,
    pub appearance: Appearance,
    pub pinned: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs::from_get(mode_core::settings::default_for)
    }
}

/// Keys whose change moves the window or changes `display.changed`.
pub const DISPLAY_KEYS: &[&str] = &[
    "overlay.placement",
    "overlay.display",
    "overlay.scale",
    "overlay.motion",
    "theme.appearance",
];

/// Keys the window controller reads ([`Prefs`]).
pub const WINDOW_KEYS: &[&str] = &[
    "overlay.placement",
    "overlay.display",
    "overlay.fullscreen",
    "overlay.scale",
    "overlay.motion",
    "theme.appearance",
    "overlay.pinned",
];

impl Prefs {
    /// Build from a getter that returns the effective value of a key.
    pub fn from_get(get: impl Fn(&str) -> Value) -> Prefs {
        let s = |k: &str| get(k).as_str().map(str::to_string);
        Prefs {
            placement: s("overlay.placement").as_deref().and_then(Placement::parse),
            display: DisplayRule::parse(s("overlay.display").as_deref().unwrap_or("main")),
            fullscreen: Fullscreen::parse(s("overlay.fullscreen").as_deref().unwrap_or("")),
            zoom: get("overlay.scale")
                .as_f64()
                .filter(|v| (100.0..=150.0).contains(v))
                .unwrap_or(100.0)
                / 100.0,
            motion: match s("overlay.motion").as_deref() {
                Some("reduced") => Motion::Reduced,
                Some("full") => Motion::Full,
                _ => Motion::System,
            },
            appearance: match s("theme.appearance").as_deref() {
                Some("light") => Appearance::Light,
                Some("dark") => Appearance::Dark,
                _ => Appearance::Auto,
            },
            pinned: get("overlay.pinned").as_bool().unwrap_or(false),
        }
    }

    pub fn load(db: &mode_core::db::Db) -> Prefs {
        Prefs::from_get(|k| {
            mode_core::settings::get(db, k).unwrap_or_else(|_| mode_core::settings::default_for(k))
        })
    }

    /// `display.changed.reduced_motion`, given what the OS says.
    pub fn reduced_motion(&self, os: Option<bool>) -> bool {
        match self.motion {
            Motion::Reduced => true,
            Motion::Full => false,
            Motion::System => os.unwrap_or(false),
        }
    }

    /// `display.changed.dark`, given the window's theme.
    pub fn dark(&self, os_dark: bool) -> bool {
        match self.appearance {
            Appearance::Light => false,
            Appearance::Dark => true,
            Appearance::Auto => os_dark,
        }
    }
}

/// `OverlaySettings` field names by settings key (contract order).
pub const OVERLAY_SETTINGS: &[(&str, &str)] = &[
    ("overlay.peek_delay_ms", "peek_delay_ms"),
    ("overlay.expand_on_hover", "expand_on_hover"),
    ("overlay.expand_hover_ms", "expand_hover_ms"),
    ("overlay.collapse_delay_ms", "collapse_delay_ms"),
    ("overlay.auto_collapse_ms", "auto_collapse_ms"),
    ("overlay.pill_min_ms", "pill_min_ms"),
    ("overlay.max_rows", "max_rows"),
    ("overlay.default_tab", "default_tab"),
    ("overlay.show_mascot", "show_mascot"),
    ("sessions.fold_after_minutes", "fold_after_minutes"),
    ("overlay.idle", "idle"),
    ("overlay.media", "media"),
];

/// The `settings.changed` payload: every field (`keys = None`, launch) or
/// the fields of the given keys; keys that are not overlay settings are
/// skipped. A value of the wrong kind falls back to the key's default.
pub fn settings_payload(get: impl Fn(&str) -> Value, keys: Option<&[&str]>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, field) in OVERLAY_SETTINGS {
        if keys.is_some_and(|ks| !ks.contains(key)) {
            continue;
        }
        let v = get(key);
        let default = mode_core::settings::default_for(key);
        let ok = match &default {
            Value::Bool(_) => v.is_boolean(),
            Value::Number(_) => v.is_i64() || v.is_u64(),
            Value::String(_) => v.is_string(),
            _ => true,
        };
        out.insert((*field).to_string(), if ok { v } else { default });
    }
    out
}

/// Whether a settings key is pushed to the overlay as `settings.changed`.
pub fn is_overlay_setting(key: &str) -> bool {
    OVERLAY_SETTINGS.iter().any(|(k, _)| *k == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn launch_payload_is_the_full_contract_object_with_defaults() {
        let p = settings_payload(mode_core::settings::default_for, None);
        assert_eq!(
            Value::Object(p),
            json!({
                "peek_delay_ms": 150,
                "expand_on_hover": true,
                "expand_hover_ms": 600,
                "collapse_delay_ms": 2500,
                "auto_collapse_ms": 8000,
                "pill_min_ms": 2000,
                "max_rows": 6,
                "default_tab": "auto",
                "show_mascot": true,
                "fold_after_minutes": 10,
                "idle": "mascot",
                "media": true
            }),
            "equals DEFAULT_SETTINGS in ui/src/state/contract.ts"
        );
    }

    #[test]
    fn change_payload_is_partial_and_type_safe() {
        let get = |k: &str| match k {
            "overlay.max_rows" => json!(8),
            "overlay.show_mascot" => json!("nope"),
            other => mode_core::settings::default_for(other),
        };
        let p = settings_payload(get, Some(&["overlay.max_rows", "overlay.placement"]));
        assert_eq!(Value::Object(p), json!({ "max_rows": 8 }));
        let p = settings_payload(get, Some(&["overlay.show_mascot"]));
        assert_eq!(
            Value::Object(p),
            json!({ "show_mascot": true }),
            "bad kind → default"
        );
        assert!(is_overlay_setting("sessions.fold_after_minutes"));
        assert!(!is_overlay_setting("overlay.placement"));
    }

    #[test]
    fn prefs_read_the_window_keys() {
        let p = Prefs::default();
        assert_eq!(p.placement, None, "unset until onboarding");
        assert_eq!(p.display, DisplayRule::Main);
        assert_eq!(p.fullscreen, Fullscreen::PresentationHides);
        assert_eq!(p.zoom, 1.0);
        assert!(!p.pinned);
        let p = Prefs::from_get(|k| match k {
            "overlay.placement" => json!("right-edge"),
            "overlay.display" => json!("cursor"),
            "overlay.fullscreen" => json!("show"),
            "overlay.scale" => json!(125),
            "overlay.motion" => json!("reduced"),
            "theme.appearance" => json!("dark"),
            "overlay.pinned" => json!(true),
            other => mode_core::settings::default_for(other),
        });
        assert_eq!(p.placement, Some(Placement::RightEdge));
        assert_eq!(p.display, DisplayRule::Cursor);
        assert_eq!(p.fullscreen, Fullscreen::Show);
        assert_eq!(p.zoom, 1.25);
        assert!(p.reduced_motion(Some(false)));
        assert!(p.dark(false));
        assert!(p.pinned);
        let auto = Prefs::default();
        assert!(auto.reduced_motion(Some(true)) && !auto.reduced_motion(None));
        assert!(auto.dark(true) && !auto.dark(false));
    }
}
