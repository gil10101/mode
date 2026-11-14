//! Linux dev executors: window activation by WM class through `wmctrl`
//! when it is installed, a new terminal in `cwd` through `$TERMINAL` or
//! `x-terminal-emulator`; everything else is logged with its hint.

use std::process::{Command, Stdio};

use super::{Target, unsupported_hint};

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
}

pub fn run(target: &Target, _default: Option<&str>) -> Option<String> {
    match target {
        Target::WmClass { app, class } => {
            if !on_path("wmctrl") {
                tracing::info!(app = %app, "jump: wmctrl not installed; cannot focus");
                return Some(format!(
                    "Mode cannot focus {app} here (wmctrl is not installed)."
                ));
            }
            let ok = Command::new("wmctrl")
                .args(["-x", "-a", class])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                None
            } else {
                Some(format!("No {app} window was found."))
            }
        }
        Target::NewTerminal { cwd } => {
            let program = std::env::var("TERMINAL")
                .ok()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "x-terminal-emulator".into());
            match Command::new(&program)
                .current_dir(cwd)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(_) => None,
                Err(e) => {
                    tracing::info!(program = %program, error = %e, "jump: no terminal to open");
                    Some(format!("Mode could not open a terminal in {cwd}."))
                }
            }
        }
        Target::Unsupported { app } => {
            tracing::info!(app = ?app, "jump: host cannot be focused");
            Some(unsupported_hint(app.as_deref()))
        }
        Target::AppleTerminal { .. } | Target::ITerm { .. } | Target::Bundle { .. } => {
            Some("That terminal runs on another computer's OS; Mode cannot focus it here.".into())
        }
        Target::ProcessWindow { app, .. } => Some(unsupported_hint(Some(app))),
        Target::Embedded => None,
    }
}
