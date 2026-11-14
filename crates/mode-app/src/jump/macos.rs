//! macOS executors for 05 §9.3: Apple Events through `osascript` for
//! Terminal.app (tab by tty) and iTerm2 (session by unique id), bundle-id
//! activation through `open -b` for Warp and Ghostty, `open -a` for a new
//! window of the default terminal. Values reach AppleScript as `argv`, never
//! spliced into the script. Compile-checked only.

use std::process::Command;

use super::{Target, unsupported_hint};

const TERMINAL_TAB: &str = r#"on run argv
  set wanted to item 1 of argv
  tell application "Terminal"
    repeat with w in windows
      repeat with t in tabs of w
        if tty of t is wanted then
          set selected tab of w to t
          set index of w to 1
          activate
          return "ok"
        end if
      end repeat
    end repeat
    activate
  end tell
  return "missing"
end run"#;

const ITERM_SESSION: &str = r#"on run argv
  set wanted to item 1 of argv
  tell application "iTerm2"
    repeat with w in windows
      repeat with t in tabs of w
        repeat with s in sessions of t
          if unique id of s is wanted then
            select w
            tell t to select
            tell s to select
            activate
            return "ok"
          end if
        end repeat
      end repeat
    end repeat
    activate
  end tell
  return "missing"
end run"#;

enum Script {
    Found,
    Missing,
    /// Automation (TCC) denied: error -1743.
    Denied,
    Failed,
}

fn osascript(script: &str, arg: &str) -> Script {
    match Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .arg(arg)
        .output()
    {
        Ok(out) if out.status.success() => {
            if String::from_utf8_lossy(&out.stdout).trim() == "ok" {
                Script::Found
            } else {
                Script::Missing
            }
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            tracing::info!(stderr = %err.trim(), "osascript refused");
            if err.contains("-1743") || err.contains("Not authorized") {
                Script::Denied
            } else {
                Script::Failed
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "osascript could not run");
            Script::Failed
        }
    }
}

fn open_bundle(bundle_id: &str) -> bool {
    Command::new("/usr/bin/open")
        .args(["-b", bundle_id])
        .status()
        .is_ok_and(|s| s.success())
}

/// The default terminal's app name and bundle id (`sessions.terminal_app`).
fn default_terminal(app: Option<&str>) -> (&'static str, &'static str) {
    // The card stores display names ("iTerm2", "Terminal.app"); older rows ids.
    let id = app.map(|a| a.trim().trim_end_matches(".app").to_ascii_lowercase());
    match id.as_deref() {
        Some("iterm2" | "iterm") => ("iTerm", "com.googlecode.iterm2"),
        Some("warp") => ("Warp", "dev.warp.Warp-Stable"),
        Some("ghostty") => ("Ghostty", "com.mitchellh.ghostty"),
        _ => ("Terminal", "com.apple.Terminal"),
    }
}

fn denied_hint(app: &str) -> String {
    format!(
        "Allow Mode to control {app} in System Settings › Privacy & Security › Automation to jump to the tab."
    )
}

pub fn run(target: &Target, default: Option<&str>) -> Option<String> {
    match target {
        Target::AppleTerminal { tty } => {
            let Some(tty) = tty else {
                open_bundle("com.apple.Terminal");
                return Some("Terminal is in front; Mode did not know the session's tab.".into());
            };
            match osascript(TERMINAL_TAB, tty) {
                Script::Found => None,
                Script::Missing => Some("Terminal is in front; the session's tab is gone.".into()),
                Script::Denied => {
                    open_bundle("com.apple.Terminal");
                    Some(denied_hint("Terminal"))
                }
                Script::Failed => {
                    open_bundle("com.apple.Terminal");
                    Some("Terminal is in front; Mode could not select the tab.".into())
                }
            }
        }
        Target::ITerm { session } => {
            let Some(id) = session else {
                open_bundle("com.googlecode.iterm2");
                return Some("iTerm2 is in front; Mode did not know the session.".into());
            };
            match osascript(ITERM_SESSION, id) {
                Script::Found => None,
                Script::Missing => Some("iTerm2 is in front; the session is gone.".into()),
                Script::Denied => {
                    open_bundle("com.googlecode.iterm2");
                    Some(denied_hint("iTerm2"))
                }
                Script::Failed => {
                    open_bundle("com.googlecode.iterm2");
                    Some("iTerm2 is in front; Mode could not select the session.".into())
                }
            }
        }
        Target::Bundle { app, bundle_id } => {
            if open_bundle(bundle_id) {
                None
            } else {
                Some(format!("Mode could not bring {app} forward."))
            }
        }
        Target::NewTerminal { cwd } => {
            let (name, _) = default_terminal(default);
            let ok = Command::new("/usr/bin/open")
                .args(["-a", name, cwd.as_str()])
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                None
            } else {
                Some(format!("Mode could not open {name} in {cwd}."))
            }
        }
        Target::Unsupported { app } => {
            let (_, bundle) = default_terminal(default);
            open_bundle(bundle);
            Some(unsupported_hint(app.as_deref()))
        }
        Target::ProcessWindow { app, .. } => Some(unsupported_hint(Some(app))),
        Target::WmClass { app, .. } => Some(unsupported_hint(Some(app))),
        Target::Embedded => None,
    }
}
