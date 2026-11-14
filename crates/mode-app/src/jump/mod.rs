//! Jump to terminal (05 §9.3, ADR-0009): classify where a session runs from
//! what the relay forwarded (the `terminal_app` / `terminal_ref` columns
//! filled from `TERM_PROGRAM`, `ITERM_SESSION_ID`, `WT_SESSION`, the tty and
//! the `claude` pid), then focus that host the way its row of the table
//! says. The classifier and the Windows window choice are pure and tested
//! on every host; the per-OS executors are thin.
//!
//! The overlay collapses itself after a jump (unless pinned); the app
//! window is not raised, except for embedded sessions, whose terminal is
//! the app window's Terminal tab.

#[cfg(not(any(target_os = "macos", windows)))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use mode_core::sessions::JumpInfo;

use crate::overlay::geometry::Os;

/// How to bring a session's terminal forward.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `launch_mode = embedded`: the app window's Terminal tab.
    Embedded,
    /// `untracked` (no process known): a new window of the default terminal in `cwd`.
    NewTerminal { cwd: String },
    /// Terminal.app: the tab whose `tty` matches, through Apple Events.
    AppleTerminal { tty: Option<String> },
    /// iTerm2: the session with this unique id, through Apple Events.
    ITerm { session: Option<String> },
    /// Warp, Ghostty on macOS: window-level activation by bundle id.
    Bundle {
        app: &'static str,
        bundle_id: &'static str,
    },
    /// Windows Terminal, a console host, WSL: the top-level window of the
    /// session's process tree.
    ProcessWindow {
        app: &'static str,
        pid: Option<u32>,
        windows_terminal: bool,
    },
    /// Linux dev: activation by window class.
    WmClass { app: String, class: &'static str },
    /// A host Mode cannot focus (an editor terminal, another emulator), or
    /// none known: the default terminal is activated and a hint is returned.
    Unsupported { app: Option<String> },
}

/// The iTerm2 unique id from `ITERM_SESSION_ID` (`w0t1p0:<UUID>`).
fn iterm_unique_id(reference: Option<&str>) -> Option<String> {
    let id = reference?.strip_prefix("iterm2:")?;
    let uuid = id.rsplit(':').next()?;
    let ok = !uuid.is_empty() && uuid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then(|| uuid.to_string())
}

/// A tty fit for an Apple Events comparison.
fn clean_tty(tty: Option<&str>) -> Option<String> {
    let t = tty?;
    let ok = t.starts_with("/dev/")
        && t.len() < 64
        && t[5..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '/');
    ok.then(|| t.to_string())
}

/// Which row of 05 §9.3 applies to a session on this OS.
pub fn classify(info: &JumpInfo, os: Os) -> Target {
    if info.launch_mode == "embedded" {
        return Target::Embedded;
    }
    if info.state == "untracked" || (info.tracking == "discovered" && info.pid.is_none()) {
        return Target::NewTerminal {
            cwd: info.cwd.clone(),
        };
    }
    let app = info.terminal_app.as_deref();
    let reference = info.terminal_ref.as_deref();
    match os {
        Os::MacOs => match app {
            Some("terminal.app") => Target::AppleTerminal {
                tty: clean_tty(info.tty.as_deref()),
            },
            Some("iterm2") => Target::ITerm {
                session: iterm_unique_id(reference),
            },
            Some("warp") => Target::Bundle {
                app: "Warp",
                bundle_id: "dev.warp.Warp-Stable",
            },
            Some("ghostty") => Target::Bundle {
                app: "Ghostty",
                bundle_id: "com.mitchellh.ghostty",
            },
            other => Target::Unsupported {
                app: other.map(str::to_string),
            },
        },
        Os::Windows => match app {
            Some("windows_terminal") => Target::ProcessWindow {
                app: "Windows Terminal",
                pid: info.pid,
                windows_terminal: true,
            },
            Some("wsl") => {
                let wt = reference.is_some_and(|r| r.starts_with("wt:"));
                Target::ProcessWindow {
                    app: if wt {
                        "Windows Terminal"
                    } else {
                        "the WSL console"
                    },
                    pid: info.pid,
                    windows_terminal: wt,
                }
            }
            Some("powershell") => Target::ProcessWindow {
                app: "the console",
                pid: info.pid,
                windows_terminal: false,
            },
            None if info.pid.is_some() => Target::ProcessWindow {
                app: "the console",
                pid: info.pid,
                windows_terminal: false,
            },
            other => Target::Unsupported {
                app: other.map(str::to_string),
            },
        },
        Os::Linux => match app {
            Some("ghostty") => Target::WmClass {
                app: "Ghostty".into(),
                class: "com.mitchellh.ghostty",
            },
            Some(a) if a.eq_ignore_ascii_case("wezterm") => Target::WmClass {
                app: "WezTerm".into(),
                class: "org.wezfurlong.wezterm",
            },
            Some("vscode") => Target::WmClass {
                app: "VS Code".into(),
                class: "code",
            },
            other => Target::Unsupported {
                app: other.map(str::to_string),
            },
        },
    }
}

/// Terminal apps found on this machine, by the names the Claude Code card
/// offers for `sessions.terminal_app` (`PlatformInfo.terminal_apps`).
pub fn terminal_apps(os: Os) -> Vec<String> {
    let on_path = |program: &str| {
        std::env::var_os("PATH")
            .is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
    };
    let home = dirs::home_dir();
    let app_bundle = |name: &str| {
        [
            Some(std::path::PathBuf::from("/Applications")),
            Some(std::path::PathBuf::from("/System/Applications/Utilities")),
            home.as_ref().map(|h| h.join("Applications")),
        ]
        .into_iter()
        .flatten()
        .any(|d| d.join(name).is_dir())
    };
    let found: Vec<&str> = match os {
        Os::MacOs => [
            ("Terminal.app", "Terminal.app"),
            ("iTerm2", "iTerm.app"),
            ("Warp", "Warp.app"),
            ("Ghostty", "Ghostty.app"),
        ]
        .into_iter()
        .filter(|(_, bundle)| app_bundle(bundle))
        .map(|(n, _)| n)
        .collect(),
        Os::Windows => {
            let mut v = Vec::new();
            if on_path("wt.exe") {
                v.push("Windows Terminal");
            }
            v.push("PowerShell");
            if on_path("wsl.exe") {
                v.push("WSL");
            }
            v
        }
        Os::Linux => [
            ("GNOME Terminal", "gnome-terminal"),
            ("Konsole", "konsole"),
            ("Ghostty", "ghostty"),
            ("Alacritty", "alacritty"),
            ("Kitty", "kitty"),
            ("WezTerm", "wezterm"),
            ("xterm", "xterm"),
        ]
        .into_iter()
        .filter(|(_, bin)| on_path(bin))
        .map(|(n, _)| n)
        .collect(),
    };
    found.into_iter().map(str::to_string).collect()
}

/// The one-line hint for a host Mode cannot focus (05 §9.3 last rows).
pub fn unsupported_hint(app: Option<&str>) -> String {
    match app {
        Some(a) => format!("This session runs in {a}, which Mode cannot focus."),
        None => "Mode could not tell which terminal runs this session.".to_string(),
    }
}

/// One process of a snapshot (Windows ancestry walk).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proc {
    pub pid: u32,
    pub parent: u32,
    /// Lowercase executable name.
    pub exe: String,
}

/// `pid` and its ancestors, nearest first (cycles and pid 0 end the walk).
pub fn ancestry(pid: u32, procs: &[Proc]) -> Vec<u32> {
    let mut chain = vec![pid];
    let mut cur = pid;
    while chain.len() < 32 {
        let Some(p) = procs.iter().find(|p| p.pid == cur) else {
            break;
        };
        if p.parent == 0 || chain.contains(&p.parent) {
            break;
        }
        chain.push(p.parent);
        cur = p.parent;
    }
    chain
}

fn is_console_host(exe: &str) -> bool {
    matches!(exe, "conhost.exe" | "openconsole.exe")
}

/// Shell and system processes whose windows are never a session's terminal
/// (the walk reaches `explorer.exe` for anything started from the desktop).
fn is_system(exe: &str) -> bool {
    matches!(
        exe,
        "explorer.exe"
            | "svchost.exe"
            | "services.exe"
            | "wininit.exe"
            | "winlogon.exe"
            | "sihost.exe"
            | "runtimebroker.exe"
            | "startmenuexperiencehost.exe"
    )
}

/// The window to bring forward for a Windows session (05 §9.3): walking up
/// from the `claude` process, the first process (or console host child of
/// one) that owns a visible top-level window; for a Windows Terminal session
/// whose tree does not reach the terminal (default-terminal hand-off), any
/// `WindowsTerminal.exe` window.
pub fn pick_window(
    pid: u32,
    procs: &[Proc],
    windows: &[(isize, u32)],
    windows_terminal: bool,
) -> Option<isize> {
    let owned = |p: u32| {
        windows
            .iter()
            .find(|(_, owner)| *owner == p)
            .map(|(h, _)| *h)
    };
    let exe = |p: u32| procs.iter().find(|x| x.pid == p).map(|x| x.exe.as_str());
    for level in ancestry(pid, procs) {
        // An exited parent (its pid may be recycled) or the desktop shell ends the walk.
        if exe(level).is_none_or(is_system) {
            break;
        }
        if let Some(h) = owned(level) {
            return Some(h);
        }
        let hosts = procs
            .iter()
            .filter(|p| p.parent == level && is_console_host(&p.exe));
        for host in hosts {
            if let Some(h) = owned(host.pid) {
                return Some(h);
            }
        }
    }
    if windows_terminal {
        return procs
            .iter()
            .filter(|p| p.exe == "windowsterminal.exe")
            .find_map(|p| owned(p.pid));
    }
    None
}

/// Focus the host of a classified session. Blocking (spawns `osascript` /
/// enumerates windows); run it off the main thread. `Ok(None)`: focused;
/// `Ok(Some(hint))`: Mode could not focus the host, here is why.
/// `Embedded` is the caller's (it opens the app window).
pub fn run(target: &Target, default_terminal: Option<&str>) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        macos::run(target, default_terminal)
    }
    #[cfg(windows)]
    {
        windows::run(target, default_terminal)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        linux::run(target, default_terminal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(app: Option<&str>, reference: Option<&str>) -> JumpInfo {
        JumpInfo {
            session_id: "s".into(),
            launch_mode: "external".into(),
            tracking: "hooked".into(),
            state: "waiting".into(),
            cwd: "/Users/ana/code/api".into(),
            terminal_app: app.map(str::to_string),
            terminal_ref: reference.map(str::to_string),
            pid: Some(4242),
            tty: Some("/dev/ttys004".into()),
            ended: false,
        }
    }

    #[test]
    fn macos_rows_of_the_table() {
        assert_eq!(
            classify(
                &info(Some("terminal.app"), Some("terminal.app:/dev/ttys004")),
                Os::MacOs
            ),
            Target::AppleTerminal {
                tty: Some("/dev/ttys004".into())
            }
        );
        assert_eq!(
            classify(
                &info(Some("iterm2"), Some("iterm2:w0t1p0:6D1C-AB12")),
                Os::MacOs
            ),
            Target::ITerm {
                session: Some("6D1C-AB12".into())
            }
        );
        assert_eq!(
            classify(
                &info(Some("iterm2"), Some("iterm2:w0t1p0:\"; do shell script")),
                Os::MacOs
            ),
            Target::ITerm { session: None },
            "anything but a plain id is dropped"
        );
        assert_eq!(
            classify(&info(Some("warp"), None), Os::MacOs),
            Target::Bundle {
                app: "Warp",
                bundle_id: "dev.warp.Warp-Stable"
            }
        );
        assert_eq!(
            classify(&info(Some("ghostty"), None), Os::MacOs),
            Target::Bundle {
                app: "Ghostty",
                bundle_id: "com.mitchellh.ghostty"
            }
        );
        assert_eq!(
            classify(
                &info(Some("vscode"), Some("term_program:vscode")),
                Os::MacOs
            ),
            Target::Unsupported {
                app: Some("vscode".into())
            }
        );
        assert_eq!(
            classify(&info(None, None), Os::MacOs),
            Target::Unsupported { app: None }
        );
        let mut bad_tty = info(Some("terminal.app"), None);
        bad_tty.tty = Some("/dev/tty\"x".into());
        assert_eq!(
            classify(&bad_tty, Os::MacOs),
            Target::AppleTerminal { tty: None }
        );
    }

    #[test]
    fn windows_rows_of_the_table() {
        assert_eq!(
            classify(
                &info(Some("windows_terminal"), Some("wt:{GUID}")),
                Os::Windows
            ),
            Target::ProcessWindow {
                app: "Windows Terminal",
                pid: Some(4242),
                windows_terminal: true
            }
        );
        assert_eq!(
            classify(&info(Some("wsl"), Some("wt:{GUID}")), Os::Windows),
            Target::ProcessWindow {
                app: "Windows Terminal",
                pid: Some(4242),
                windows_terminal: true
            },
            "WSL inside Windows Terminal follows the Windows Terminal rule"
        );
        assert_eq!(
            classify(&info(Some("wsl"), Some("wsl:Ubuntu")), Os::Windows),
            Target::ProcessWindow {
                app: "the WSL console",
                pid: Some(4242),
                windows_terminal: false
            }
        );
        assert_eq!(
            classify(&info(None, None), Os::Windows),
            Target::ProcessWindow {
                app: "the console",
                pid: Some(4242),
                windows_terminal: false
            },
            "no env: the console-host rule by ancestry"
        );
        let mut gone = info(None, None);
        gone.pid = None;
        assert_eq!(
            classify(&gone, Os::Windows),
            Target::Unsupported { app: None }
        );
        assert_eq!(
            classify(&info(Some("vscode"), None), Os::Windows),
            Target::Unsupported {
                app: Some("vscode".into())
            }
        );
    }

    #[test]
    fn embedded_untracked_and_linux_rows() {
        let mut e = info(Some("iterm2"), None);
        e.launch_mode = "embedded".into();
        assert_eq!(classify(&e, Os::MacOs), Target::Embedded);
        assert_eq!(classify(&e, Os::Windows), Target::Embedded);
        let mut u = info(None, None);
        u.state = "untracked".into();
        u.tracking = "discovered".into();
        u.pid = None;
        assert_eq!(
            classify(&u, Os::Linux),
            Target::NewTerminal {
                cwd: "/Users/ana/code/api".into()
            }
        );
        assert_eq!(
            classify(&info(Some("ghostty"), None), Os::Linux),
            Target::WmClass {
                app: "Ghostty".into(),
                class: "com.mitchellh.ghostty"
            }
        );
        assert_eq!(
            classify(&info(Some("tmux"), None), Os::Linux),
            Target::Unsupported {
                app: Some("tmux".into())
            }
        );
        assert_eq!(
            unsupported_hint(Some("vscode")),
            "This session runs in vscode, which Mode cannot focus."
        );
        assert!(unsupported_hint(None).contains("could not tell"));
    }

    fn p(pid: u32, parent: u32, exe: &str) -> Proc {
        Proc {
            pid,
            parent,
            exe: exe.into(),
        }
    }

    #[test]
    fn windows_ancestry_finds_the_hosting_window() {
        // WindowsTerminal(10) → OpenConsole(11) → pwsh(12) → claude(13)
        let procs = vec![
            p(1, 0, "explorer.exe"),
            p(10, 1, "windowsterminal.exe"),
            p(11, 10, "openconsole.exe"),
            p(12, 11, "pwsh.exe"),
            p(13, 12, "claude.exe"),
            // A classic console: cmd(20) with its conhost(21) → claude(22)
            p(20, 1, "cmd.exe"),
            p(21, 20, "conhost.exe"),
            p(22, 20, "claude.exe"),
            // A cycle must not hang the walk.
            p(30, 31, "a.exe"),
            p(31, 30, "b.exe"),
        ];
        assert_eq!(ancestry(13, &procs), vec![13, 12, 11, 10, 1]);
        assert_eq!(ancestry(30, &procs), vec![30, 31]);
        let windows = vec![(0x100, 10), (0x200, 21), (0x300, 1)];
        assert_eq!(pick_window(13, &procs, &windows, true), Some(0x100));
        assert_eq!(
            pick_window(22, &procs, &windows, false),
            Some(0x200),
            "the console host child of an ancestor"
        );
        // Default-terminal hand-off: the tree does not reach WT; any WT window.
        let handoff = vec![
            p(40, 1, "cmd.exe"),
            p(41, 40, "claude.exe"),
            p(10, 1, "windowsterminal.exe"),
        ];
        assert_eq!(pick_window(41, &handoff, &[(0x100, 10)], true), Some(0x100));
        assert_eq!(pick_window(41, &handoff, &[(0x100, 10)], false), None);
        // The walk stops at the desktop shell: never focus an Explorer window.
        let mut with_shell = handoff.clone();
        with_shell.push(p(1, 0, "explorer.exe"));
        assert_eq!(pick_window(41, &with_shell, &[(0x300, 1)], false), None);
        // ... and at a parent that is gone (its pid may belong to someone else now).
        assert_eq!(pick_window(41, &handoff, &[(0x300, 1)], false), None);
    }
}
