//! Windows executors for 05 §9.3: the top-level window of the session's
//! process tree (Windows Terminal, a console host, WSL) through
//! `SetForegroundWindow`; a new Windows Terminal (or console) in `cwd` for
//! untracked sessions. Compile-checked only.

use std::process::Command;

use super::{Proc, Target, pick_window, unsupported_hint};
use crate::platform::windows::win32;

pub fn run(target: &Target, _default: Option<&str>) -> Option<String> {
    match target {
        Target::ProcessWindow {
            app,
            pid,
            windows_terminal,
        } => {
            let Some(pid) = pid else {
                return Some(format!(
                    "The {app} window for this session is no longer known."
                ));
            };
            let procs: Vec<Proc> = win32::processes()
                .into_iter()
                .map(|p| Proc {
                    pid: p.pid,
                    parent: p.parent,
                    exe: p.exe,
                })
                .collect();
            let windows = win32::top_level_windows();
            match pick_window(*pid, &procs, &windows, *windows_terminal) {
                Some(h) if win32::focus_window(h) => None,
                Some(_) => Some(format!(
                    "Windows kept {app} in the background; its taskbar button is flashing."
                )),
                None => Some(format!("The {app} window for this session was not found.")),
            }
        }
        Target::NewTerminal { cwd } => {
            let wt = Command::new("wt.exe").args(["-d", cwd.as_str()]).spawn();
            if wt.is_ok() {
                return None;
            }
            let console = Command::new("cmd.exe")
                .args(["/C", "start", "", "/D", cwd.as_str(), "cmd.exe"])
                .spawn();
            match console {
                Ok(_) => None,
                Err(_) => Some(format!("Mode could not open a terminal in {cwd}.")),
            }
        }
        Target::Unsupported { app } => Some(unsupported_hint(app.as_deref())),
        Target::AppleTerminal { .. } | Target::ITerm { .. } | Target::Bundle { .. } => {
            Some(unsupported_hint(None))
        }
        Target::WmClass { app, .. } => Some(unsupported_hint(Some(app))),
        Target::Embedded => None,
    }
}
