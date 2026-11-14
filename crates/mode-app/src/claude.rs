//! Is Claude Code on this machine (03 §2.7 "Claude Code not installed": no
//! `~/.claude` and no `claude` to run)? The binary is
//! `settings['sessions.claude_path']` when set, else the first `claude` on
//! `PATH`, else the usual install locations (a GUI app's `PATH` is short on
//! macOS). Its version comes from `claude --version` (5 s cap), cached until
//! the binary's path or mtime changes.

use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;

use crate::state::CoreState;

/// How long `claude --version` may take.
pub const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// What the health push says about Claude Code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Probe {
    /// The binary, when one was found.
    pub path: Option<PathBuf>,
    /// `~/.claude` exists.
    pub config_dir: bool,
    /// `2.1.305` from `claude --version`, when it answered.
    pub version: Option<String>,
    #[serde(skip)]
    mtime: Option<SystemTime>,
}

impl Probe {
    /// `found` unless both the binary and `~/.claude` are missing (03 §2.7).
    pub fn word(&self) -> &'static str {
        if self.path.is_some() || self.config_dir {
            "found"
        } else {
            "missing"
        }
    }
}

fn exe_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["claude.exe", "claude.cmd", "claude.bat", "claude"]
    } else {
        &["claude"]
    }
}

fn is_runnable(p: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Where `claude` is: the configured path, then `PATH`, then the install
/// locations under `home`.
pub fn locate(
    configured: Option<&str>,
    path_var: Option<&OsStr>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(c) = configured.map(str::trim).filter(|c| !c.is_empty()) {
        let p = match (c.strip_prefix("~/"), home) {
            (Some(rest), Some(h)) => h.join(rest),
            _ => PathBuf::from(c),
        };
        return is_runnable(&p).then_some(p);
    }
    if let Some(paths) = path_var {
        for dir in std::env::split_paths(paths) {
            for name in exe_names() {
                let p = dir.join(name);
                if is_runnable(&p) {
                    return Some(p);
                }
            }
        }
    }
    let mut extra: Vec<PathBuf> = Vec::new();
    if let Some(h) = home {
        extra.push(h.join(".claude/local/claude"));
        extra.push(h.join(".local/bin/claude"));
        extra.push(h.join(".npm-global/bin/claude"));
    }
    if cfg!(target_os = "macos") {
        extra.push(PathBuf::from("/opt/homebrew/bin/claude"));
        extra.push(PathBuf::from("/usr/local/bin/claude"));
    }
    extra.into_iter().find(|p| is_runnable(p))
}

/// The version token of `claude --version` output (`2.1.305 (Claude Code)`).
pub fn parse_version(out: &str) -> Option<String> {
    out.split_whitespace()
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))
        .map(|t| {
            t.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-')
                .to_string()
        })
}

/// Run `<bin> --version` with a deadline; `None` when it fails or hangs.
pub fn version(bin: &Path) -> Option<String> {
    let mut child = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < VERSION_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(25));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    parse_version(&reader.join().ok()?)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

/// Probe (blocking), reusing the cached version while the binary is the
/// same file.
pub fn probe(core: &CoreState) -> Probe {
    let configured = mode_core::settings::get_string(&core.db, "sessions.claude_path")
        .ok()
        .flatten();
    let home = dirs::home_dir();
    let path_var = std::env::var_os("PATH");
    let path = locate(configured.as_deref(), path_var.as_deref(), home.as_deref());
    let config_dir = home.as_ref().is_some_and(|h| h.join(".claude").is_dir())
        || std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|d| Path::new(&d).is_dir());
    let m = path.as_deref().and_then(mtime);
    let cached = CoreState::lock(&core.claude).clone();
    let version = match cached {
        Some(c) if c.path == path && c.mtime == m => c.version,
        _ => path.as_deref().and_then(version),
    };
    let p = Probe {
        path,
        config_dir,
        version,
        mtime: m,
    };
    *CoreState::lock(&core.claude) = Some(p.clone());
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_from_the_cli_banner() {
        assert_eq!(
            parse_version("2.1.305 (Claude Code)\n").as_deref(),
            Some("2.1.305")
        );
        assert_eq!(parse_version("claude v1.0.3").as_deref(), None);
        assert_eq!(parse_version("").as_deref(), None);
    }

    #[cfg(unix)]
    #[test]
    fn locates_configured_then_path_then_install_dirs_and_runs_version() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = bin.join("claude");
        std::fs::write(&script, "#!/bin/sh\necho '9.8.7 (Claude Code)'\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path_var = std::env::join_paths([bin.clone()]).unwrap();
        assert_eq!(locate(None, Some(&path_var), None), Some(script.clone()));
        assert_eq!(
            locate(Some(script.to_str().unwrap()), None, None),
            Some(script.clone())
        );
        assert_eq!(
            locate(Some("/no/such/claude"), Some(&path_var), None),
            None,
            "a configured path wins, even when wrong"
        );
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        let local = home.join(".local/bin/claude");
        std::fs::copy(&script, &local).unwrap();
        std::fs::set_permissions(&local, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(locate(None, None, Some(&home)), Some(local));
        assert_eq!(version(&script).as_deref(), Some("9.8.7"));
        let probe = Probe {
            path: None,
            config_dir: false,
            version: None,
            mtime: None,
        };
        assert_eq!(probe.word(), "missing");
        assert_eq!(
            Probe {
                config_dir: true,
                ..probe
            }
            .word(),
            "found"
        );
    }
}
