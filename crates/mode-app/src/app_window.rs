//! The app window (08 §3.3 `mode-app::windows`): `app_open` shows and
//! focuses it and tells its webview where to go with `app.open`; the shell
//! routes on that event (package 6).

use mode_core::push::Event;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::APP_WINDOW;

/// The views `app_open` accepts (`AppView` in the contract).
pub const VIEWS: &[&str] = &[
    "sessions",
    "inbox",
    "notes",
    "calendar",
    "chat",
    "automations",
    "integrations",
    "settings",
    "shortcuts",
    "stats",
];

/// `app.open` (`AppOpen` in `contract-sessions.ts`): the view, an optional
/// item, an optional filter, and `route`, the full path the shell routes on
/// (03 §2.5: `sessions/<id>/terminal`, `notes/<id>`, `shortcuts?q=…`);
/// `tab` is set only by jump to terminal for an embedded session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppOpen {
    pub view: String,
    pub id: Option<String>,
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    pub route: String,
}

impl AppOpen {
    /// The request with its route derived from the other fields.
    pub fn new(view: &str, id: Option<String>, filter: Option<String>, tab: Option<&str>) -> Self {
        AppOpen {
            route: route_for(view, id.as_deref(), filter.as_deref(), tab),
            view: view.to_string(),
            id,
            filter,
            tab: tab.map(str::to_string),
        }
    }
}

/// `encodeURIComponent`: everything but `A–Z a–z 0–9 - _ . ! ~ * ' ( )`.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The canonical route of the shell's router (`formatRoute` in
/// `ui/src/app/router.ts`) for a view, item, filter and session tab.
pub fn route_for(view: &str, id: Option<&str>, filter: Option<&str>, tab: Option<&str>) -> String {
    let mut s = view.to_string();
    if let Some(id) = id {
        s.push('/');
        s.push_str(&encode(id));
        if view == "sessions"
            && let Some(t) = tab.filter(|t| *t != "overview")
        {
            s.push('/');
            s.push_str(&encode(t));
        }
    }
    match (view, filter) {
        ("sessions", Some(f)) => s.push_str(&format!("?filter={}", encode(f))),
        ("shortcuts", Some(f)) => s.push_str(&format!("?q={}", encode(f))),
        _ => {}
    }
    s
}

/// Validate a caller-given route (a path or `mode://` URL for the shell).
pub fn check_route(route: &str) -> Result<(), String> {
    if route.is_empty() || route.len() > 512 || route.chars().any(char::is_control) {
        return Err("the route is empty, too long or not printable".into());
    }
    Ok(())
}

/// Validate an `app_open` request.
pub fn check(view: &str, id: Option<&str>, filter: Option<&str>) -> Result<(), String> {
    if !VIEWS.contains(&view) {
        return Err(format!("{view:?} is not a view of the app window"));
    }
    if id.is_some_and(|i| i.is_empty() || i.len() > 128) {
        return Err("the item id is empty or too long".into());
    }
    if filter.is_some_and(|f| f.len() > 2_000) {
        return Err("the filter is too long".into());
    }
    Ok(())
}

/// Show and focus the app window (the window hotkey, the quit question).
pub fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(APP_WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Hide the app window (03 §2.1: the close button and `⌘/Ctrl+W` hide; Mode
/// keeps running behind the overlay and the status item, and `app_open`
/// shows it again).
pub fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(APP_WINDOW) {
        let _ = w.hide();
    }
}

/// Show and focus the app window, then send it `app.open`.
pub fn open(app: &AppHandle, req: AppOpen) {
    tracing::info!(view = %req.view, id = ?req.id, tab = ?req.tab, "app.open");
    if let Some(w) = app.get_webview_window(APP_WINDOW) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Err(e) = app.emit_to(APP_WINDOW, Event::AppOpen.wire_name(), &req) {
            tracing::warn!(error = %e, "emit app.open failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_ids_and_filters_are_checked() {
        assert!(check("sessions", Some("0193"), None).is_ok());
        assert!(check("integrations", None, None).is_ok());
        assert!(check("chat", None, None).is_ok());
        assert!(check("automations", Some("au-1"), None).is_ok());
        assert!(check("terminal", None, None).is_err());
        assert!(check("notes", Some(""), None).is_err());
        assert!(check("inbox", None, Some(&"x".repeat(2_001))).is_err());
        let v =
            serde_json::to_value(AppOpen::new("sessions", Some("s1".into()), None, None)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"view": "sessions", "id": "s1", "filter": null, "route": "sessions/s1"})
        );
    }

    #[test]
    fn routes_follow_the_shell_router() {
        assert_eq!(route_for("sessions", None, None, None), "sessions");
        assert_eq!(
            route_for("sessions", Some("0193-ab"), None, Some("terminal")),
            "sessions/0193-ab/terminal"
        );
        assert_eq!(
            route_for("sessions", Some("s1"), Some("waiting"), Some("overview")),
            "sessions/s1?filter=waiting"
        );
        assert_eq!(
            route_for("sessions", None, Some("a b/c"), None),
            "sessions?filter=a%20b%2Fc"
        );
        assert_eq!(
            route_for("notes", Some("n 1"), Some("ignored"), None),
            "notes/n%201"
        );
        assert_eq!(
            route_for("shortcuts", None, Some("capture"), None),
            "shortcuts?q=capture"
        );
        let jump = AppOpen::new("sessions", Some("s9".into()), None, Some("terminal"));
        assert_eq!(jump.route, "sessions/s9/terminal");
        assert_eq!(jump.tab.as_deref(), Some("terminal"));
        assert!(check_route("sessions/s1/files").is_ok());
        assert!(check_route("").is_err());
        assert!(check_route("a\nb").is_err());
        assert!(check_route(&"x".repeat(600)).is_err());
        assert!(check_route("bad\u{0}route").is_err());
    }
}
