//! Global hotkeys (02 §8.2, 03 §7D): quick capture, overlay toggle and pin,
//! the optional "open app window" and the dictation hold key (XC-37), read
//! from `capture.hotkey`, `overlay.toggle_hotkey`, `overlay.pin_hotkey`,
//! `window.hotkey` and `dictation.hotkey` (registered only while
//! `dictation.enabled`; it acts on press and on release),
//! registered through `tauri_plugin_global_shortcut` (Carbon
//! `RegisterEventHotKey` on macOS, `RegisterHotKey` on Windows, `XGrabKey`
//! on Linux; no Accessibility permission), re-registered when one of them
//! changes, and delivered to the overlay as `hotkey.fired { hotkey }` (the
//! window hotkey shows and focuses the app window instead).
//!
//! The settings strings are human chords ("Ctrl+Alt+Space", "Alt+Shift+P",
//! or the macOS glyph form "⌥⇧Space"); [`parse_chord`] normalizes them into
//! the plugin's accelerator syntax so both spellings work.

use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::overlay::Overlay;
use crate::state::CoreState;

/// Which overlay hotkey fired (`HotkeyFired.hotkey`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HotkeyKind {
    Toggle,
    Capture,
    Pin,
    /// "Open app window" (`window.hotkey`, unset by default).
    Window,
    /// Hold to dictate (`dictation.hotkey`, XC-37): press starts listening,
    /// release transcribes. Registered only while `dictation.enabled`.
    Dictate,
}

impl HotkeyKind {
    pub const ALL: [HotkeyKind; 5] = [
        HotkeyKind::Capture,
        HotkeyKind::Toggle,
        HotkeyKind::Pin,
        HotkeyKind::Window,
        HotkeyKind::Dictate,
    ];

    /// The settings key that holds its chord.
    pub fn key(self) -> &'static str {
        match self {
            HotkeyKind::Capture => "capture.hotkey",
            HotkeyKind::Toggle => "overlay.toggle_hotkey",
            HotkeyKind::Pin => "overlay.pin_hotkey",
            HotkeyKind::Window => "window.hotkey",
            HotkeyKind::Dictate => "dictation.hotkey",
        }
    }

    /// May be left unset (nothing registered, not a failure).
    pub fn optional(self) -> bool {
        matches!(self, HotkeyKind::Window | HotkeyKind::Dictate)
    }

    pub fn from_key(key: &str) -> Option<HotkeyKind> {
        HotkeyKind::ALL.into_iter().find(|k| k.key() == key)
    }
}

/// A parsed chord: modifiers plus one key (in the plugin's key vocabulary).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// ⌘ on macOS, the Windows key elsewhere.
    pub super_key: bool,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChordError {
    #[error("the hotkey is empty")]
    Empty,
    #[error("the hotkey has no key, only modifiers")]
    NoKey,
    #[error("the hotkey has two keys ({0} and {1})")]
    TwoKeys(String, String),
    #[error("{0:?} is not a key Mode can register")]
    UnknownKey(String),
    #[error(
        "a global hotkey needs a modifier (Ctrl, Alt, Shift or Cmd) unless it is a function key"
    )]
    NoModifier,
}

/// Modifier glyphs of the macOS chord notation.
const GLYPHS: &[(char, &str)] = &[('⌃', "ctrl"), ('⌥', "alt"), ('⇧', "shift"), ('⌘', "cmd")];

fn key_name(token: &str) -> Option<String> {
    let t = token.trim();
    let upper = t.to_ascii_uppercase();
    let named = match upper.as_str() {
        "SPACE" | "␣" => "Space",
        "ENTER" | "RETURN" | "↩" => "Enter",
        "TAB" | "⇥" => "Tab",
        "ESC" | "ESCAPE" | "⎋" => "Escape",
        "BACKSPACE" | "⌫" => "Backspace",
        "DELETE" | "DEL" | "⌦" => "Delete",
        "UP" | "ARROWUP" | "↑" => "ArrowUp",
        "DOWN" | "ARROWDOWN" | "↓" => "ArrowDown",
        "LEFT" | "ARROWLEFT" | "←" => "ArrowLeft",
        "RIGHT" | "ARROWRIGHT" | "→" => "ArrowRight",
        "HOME" => "Home",
        "END" => "End",
        "PAGEUP" => "PageUp",
        "PAGEDOWN" => "PageDown",
        "`" | "BACKQUOTE" => "Backquote",
        "-" | "MINUS" => "Minus",
        "=" | "EQUAL" => "Equal",
        "[" | "BRACKETLEFT" => "BracketLeft",
        "]" | "BRACKETRIGHT" => "BracketRight",
        "\\" | "BACKSLASH" => "Backslash",
        ";" | "SEMICOLON" => "Semicolon",
        "'" | "QUOTE" => "Quote",
        "," | "COMMA" => "Comma",
        "." | "PERIOD" => "Period",
        "/" | "SLASH" => "Slash",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.to_string());
    }
    let mut chars = upper.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => return Some(c.to_string()),
        (Some(c), None) if c.is_ascii_digit() => return Some(c.to_string()),
        _ => {}
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok())
        && (1..=24).contains(&n)
    {
        return Some(format!("F{n}"));
    }
    None
}

/// Parse a settings chord. Tokens are separated by `+` (or written as
/// leading glyphs, `⌥⇧Space`); modifiers are case-insensitive aliases
/// (`Ctrl`/`Control`/`⌃`, `Alt`/`Option`/`Opt`/`⌥`, `Shift`/`⇧`,
/// `Cmd`/`Command`/`Super`/`Meta`/`Win`/`⌘`; `CmdOrCtrl` resolves per OS).
pub fn parse_chord(s: &str) -> Result<Chord, ChordError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ChordError::Empty);
    }
    let mut tokens: Vec<String> = Vec::new();
    for part in s.split('+') {
        let mut rest = part.trim();
        // Peel leading modifier glyphs: "⌥⇧Space" → alt, shift, "Space".
        while let Some((g, name)) = rest
            .chars()
            .next()
            .and_then(|c| GLYPHS.iter().find(|(g, _)| *g == c))
        {
            tokens.push((*name).to_string());
            rest = rest[g.len_utf8()..].trim_start();
        }
        if !rest.is_empty() {
            tokens.push(rest.to_string());
        }
    }
    let mut chord = Chord {
        ctrl: false,
        alt: false,
        shift: false,
        super_key: false,
        key: String::new(),
    };
    for t in tokens {
        match t.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => chord.ctrl = true,
            "alt" | "option" | "opt" => chord.alt = true,
            "shift" => chord.shift = true,
            "cmd" | "command" | "super" | "meta" | "win" => chord.super_key = true,
            "cmdorctrl" | "commandorcontrol" | "cmdorcontrol" | "commandorctrl" => {
                if cfg!(target_os = "macos") {
                    chord.super_key = true
                } else {
                    chord.ctrl = true
                }
            }
            _ => {
                let key = key_name(&t).ok_or_else(|| ChordError::UnknownKey(t.clone()))?;
                if !chord.key.is_empty() {
                    return Err(ChordError::TwoKeys(chord.key, key));
                }
                chord.key = key;
            }
        }
    }
    if chord.key.is_empty() {
        return Err(ChordError::NoKey);
    }
    let function_key = chord.key.starts_with('F') && chord.key.len() > 1;
    if !(chord.ctrl || chord.alt || chord.shift || chord.super_key) && !function_key {
        return Err(ChordError::NoModifier);
    }
    Ok(chord)
}

impl Chord {
    /// The plugin's accelerator string ("Ctrl+Alt+Shift+Space").
    pub fn accelerator(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        if self.super_key {
            parts.push("Super");
        }
        parts.push(&self.key);
        parts.join("+")
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.accelerator())
    }
}

impl FromStr for Chord {
    type Err = ChordError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_chord(s)
    }
}

/// The chords from the settings, or why one cannot be used; an unset
/// optional hotkey is left out. Two hotkeys on the same chord are refused
/// (the later one in [`HotkeyKind::ALL`]).
pub fn chords_from(
    get: impl Fn(&str) -> Option<String>,
) -> Vec<(HotkeyKind, Result<Chord, String>)> {
    let mut seen: Vec<(HotkeyKind, Chord)> = Vec::new();
    HotkeyKind::ALL
        .into_iter()
        .filter_map(|kind| {
            let raw = get(kind.key()).unwrap_or_default();
            (!(kind.optional() && raw.trim().is_empty())).then_some((kind, raw))
        })
        .map(|(kind, raw)| {
            let parsed = parse_chord(&raw).map_err(|e| e.to_string()).and_then(|c| {
                match seen.iter().find(|(_, other)| *other == c) {
                    Some((other, _)) => Err(format!("{c} is already the {other:?} hotkey")),
                    None => {
                        seen.push((kind, c.clone()));
                        Ok(c)
                    }
                }
            });
            (kind, parsed)
        })
        .collect()
}

/// What is registered right now, for re-registration and conflict reports.
#[derive(Debug, Default)]
pub struct Registered {
    pub chords: Vec<(HotkeyKind, Chord)>,
    /// Hotkeys that could not be registered, with the reason (02 §8.2 conflicts).
    pub failed: Vec<(HotkeyKind, String)>,
}

/// The process-wide registry of Mode's hotkeys (managed state).
#[derive(Debug, Default)]
pub struct Hotkeys {
    pub state: Mutex<Registered>,
}

/// Register the chords with the OS through the plugin, replacing what
/// was registered before. Blocking (the plugin waits for the main thread):
/// never call it on the main thread. Returns the hotkeys that failed.
pub fn register(
    app: &AppHandle,
    wanted: Vec<(HotkeyKind, Result<Chord, String>)>,
) -> Vec<(HotkeyKind, String)> {
    let Some(hotkeys) = app.try_state::<Hotkeys>() else {
        return Vec::new();
    };
    let gs = app.global_shortcut();
    let mut reg = hotkeys.state.lock().unwrap_or_else(|e| e.into_inner());
    for (_, c) in reg.chords.drain(..) {
        if let Ok(sc) = Shortcut::from_str(&c.accelerator()) {
            let _ = gs.unregister(sc);
        }
    }
    reg.failed.clear();
    for (kind, chord) in wanted {
        let result = chord.and_then(|c| {
            let sc = Shortcut::from_str(&c.accelerator()).map_err(|e| e.to_string())?;
            gs.on_shortcut(sc, move |app, _shortcut, event| match (kind, event.state) {
                (HotkeyKind::Dictate, ShortcutState::Pressed) => crate::speech::press(app),
                (HotkeyKind::Dictate, ShortcutState::Released) => crate::speech::release(app),
                (_, ShortcutState::Pressed) => fired(app, kind),
                (_, ShortcutState::Released) => {}
            })
            .map_err(|e| e.to_string())?;
            Ok(c)
        });
        match result {
            Ok(c) => {
                tracing::info!(hotkey = ?kind, chord = %c, "hotkey registered");
                reg.chords.push((kind, c));
            }
            Err(e) => {
                tracing::warn!(hotkey = ?kind, error = %e, "hotkey not registered");
                reg.failed.push((kind, e));
            }
        }
    }
    reg.failed.clone()
}

/// A hotkey was pressed: the overlay controller records it (main thread)
/// and emits `hotkey.fired`; the window hotkey shows the app window.
fn fired(app: &AppHandle, kind: HotkeyKind) {
    if kind == HotkeyKind::Window {
        crate::app_window::show(app);
        return;
    }
    if let Some(overlay) = app.try_state::<Arc<Overlay>>() {
        overlay.on_main(move |o| o.hotkey(kind));
    }
}

/// Read the chords from the settings and register them (off the main thread).
pub async fn register_from_settings(
    app: AppHandle,
    core: Arc<CoreState>,
) -> Vec<(HotkeyKind, String)> {
    let job = tauri::async_runtime::spawn_blocking(move || {
        let dictation = mode_core::settings::get(&core.db, "dictation.enabled")
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let wanted = chords_from(|k| {
            if k == HotkeyKind::Dictate.key() && !dictation {
                return None;
            }
            mode_core::settings::get_string(&core.db, k).ok().flatten()
        });
        register(&app, wanted)
    });
    job.await.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_defaults_parse() {
        let c = parse_chord("Ctrl+Alt+Space").unwrap();
        assert_eq!(c.accelerator(), "Ctrl+Alt+Space");
        assert_eq!(
            parse_chord("Ctrl+Alt+Shift+Space").unwrap().accelerator(),
            "Ctrl+Alt+Shift+Space"
        );
        assert_eq!(
            parse_chord("Ctrl+Alt+Shift+P").unwrap().accelerator(),
            "Ctrl+Alt+Shift+P"
        );
        assert_eq!(parse_chord("Alt+Space").unwrap().accelerator(), "Alt+Space");
        assert_eq!(
            parse_chord("Alt+Shift+P").unwrap().accelerator(),
            "Alt+Shift+P"
        );
        // Every computed default of the registry parses.
        for key in [
            "capture.hotkey",
            "overlay.toggle_hotkey",
            "overlay.pin_hotkey",
            "dictation.hotkey",
        ] {
            let v = mode_core::settings::default_for(key);
            assert!(parse_chord(v.as_str().unwrap()).is_ok(), "{key} = {v}");
        }
    }

    #[test]
    fn aliases_glyphs_and_case_normalise() {
        let a = parse_chord("option+shift+p").unwrap();
        let b = parse_chord("⌥⇧P").unwrap();
        let c = parse_chord(" Alt + Shift + p ").unwrap();
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(parse_chord("⌥Space").unwrap().accelerator(), "Alt+Space");
        assert_eq!(parse_chord("⌘⇧K").unwrap().accelerator(), "Shift+Super+K");
        assert_eq!(
            parse_chord("Control+Option+Command+Return")
                .unwrap()
                .accelerator(),
            "Ctrl+Alt+Super+Enter"
        );
        assert_eq!(parse_chord("Win+F12").unwrap().key, "F12");
        assert_eq!(parse_chord("Ctrl+/").unwrap().key, "Slash");
        let cmd_or_ctrl = parse_chord("CmdOrCtrl+K").unwrap();
        assert_eq!(cmd_or_ctrl.ctrl, !cfg!(target_os = "macos"));
        assert_eq!(cmd_or_ctrl.super_key, cfg!(target_os = "macos"));
        assert_eq!("Ctrl+1".parse::<Chord>().unwrap().key, "1");
    }

    #[test]
    fn bad_chords_are_refused_with_a_reason() {
        assert_eq!(parse_chord(""), Err(ChordError::Empty));
        assert_eq!(parse_chord("Ctrl+Alt"), Err(ChordError::NoKey));
        assert_eq!(parse_chord("Space"), Err(ChordError::NoModifier));
        assert!(parse_chord("F5").is_ok(), "function keys may stand alone");
        assert!(matches!(
            parse_chord("Ctrl+A+B"),
            Err(ChordError::TwoKeys(..))
        ));
        assert!(matches!(
            parse_chord("Ctrl+Banana"),
            Err(ChordError::UnknownKey(_))
        ));
        assert!(matches!(
            parse_chord("Ctrl+F25"),
            Err(ChordError::UnknownKey(_))
        ));
    }

    #[test]
    fn accelerators_are_accepted_by_the_plugin_parser() {
        use tauri_plugin_global_shortcut::Shortcut;
        for s in [
            "Ctrl+Alt+Space",
            "⌥⇧P",
            "Ctrl+Alt+Shift+Space",
            "Cmd+Shift+K",
            "Ctrl+/",
            "F12",
        ] {
            let acc = parse_chord(s).unwrap().accelerator();
            assert!(Shortcut::from_str(&acc).is_ok(), "{s} → {acc}");
        }
    }

    #[test]
    fn duplicate_and_missing_chords_are_reported_per_hotkey() {
        let values = |k: &str| match k {
            "capture.hotkey" => Some("Ctrl+Alt+Space".to_string()),
            "overlay.toggle_hotkey" => Some("ctrl+alt+space".to_string()),
            _ => None,
        };
        let out = chords_from(values);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].0, HotkeyKind::Capture);
        assert!(out[0].1.is_ok());
        assert!(out[1].1.as_ref().unwrap_err().contains("already"));
        assert!(out[2].1.is_err(), "pin: empty");
        assert_eq!(
            HotkeyKind::from_key("overlay.pin_hotkey"),
            Some(HotkeyKind::Pin)
        );
        assert_eq!(
            HotkeyKind::from_key("window.hotkey"),
            Some(HotkeyKind::Window)
        );
        // The window hotkey joins when set, and clashes like the others.
        let with_window = |k: &str| match k {
            "capture.hotkey" => Some("Ctrl+Alt+Space".to_string()),
            "window.hotkey" => Some("Ctrl+Alt+Space".to_string()),
            _ => Some("Alt+Shift+P".to_string()),
        };
        let out = chords_from(with_window);
        assert_eq!(out.len(), 5);
        assert_eq!(out[3].0, HotkeyKind::Window);
        assert!(out[3].1.as_ref().unwrap_err().contains("already"));
        assert_eq!(out[4].0, HotkeyKind::Dictate);
        assert!(out[4].1.as_ref().unwrap_err().contains("already"));
        // The dictation key is optional: unset (dictation off) means not registered.
        let dictation = |k: &str| match k {
            "dictation.hotkey" => Some("Ctrl+Alt+D".to_string()),
            "capture.hotkey" => Some("Ctrl+Alt+Space".to_string()),
            "overlay.toggle_hotkey" => Some("Ctrl+Alt+Shift+Space".to_string()),
            "overlay.pin_hotkey" => Some("Ctrl+Alt+Shift+P".to_string()),
            _ => None,
        };
        let out = chords_from(dictation);
        assert_eq!(out.len(), 4, "window unset");
        assert_eq!(out[3].0, HotkeyKind::Dictate);
        assert_eq!(out[3].1.as_ref().unwrap().accelerator(), "Ctrl+Alt+D");
        assert_eq!(
            HotkeyKind::from_key("dictation.hotkey"),
            Some(HotkeyKind::Dictate)
        );
        assert_eq!(
            serde_json::to_value(HotkeyKind::Dictate).unwrap(),
            "dictate"
        );
        assert_eq!(serde_json::to_value(HotkeyKind::Toggle).unwrap(), "toggle");
    }
}
