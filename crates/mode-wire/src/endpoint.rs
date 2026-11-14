//! Where the relay finds the core (05 §3.2; 08 §5.2).
//!
//! | OS | Endpoint |
//! |---|---|
//! | macOS | Unix socket `~/Library/Application Support/<Mode\|Mode-dev>/mode.sock` (the `app_data_dir` of 09 §2; reachable from any terminal, unlike `$TMPDIR`) |
//! | Linux | Unix socket `$XDG_RUNTIME_DIR/<name>.sock`, else `$TMPDIR`, else `/tmp` |
//! | Windows | Named pipe `\\.\pipe\<name>-<sid>` with the current user's SID string |
//!
//! `<name>` is `mode-dev` for debug builds and `mode` otherwise, so a dev
//! build and the installed app coexist. `MODE_ENDPOINT` overrides the full
//! socket path / pipe name (tests, diagnostics, the smoke script).

#[cfg(unix)]
use std::path::PathBuf;

/// Environment variable that overrides the computed endpoint.
pub const ENV_OVERRIDE: &str = "MODE_ENDPOINT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    name: String,
    override_path: Option<String>,
}

impl Endpoint {
    pub const DEV_NAME: &str = "mode-dev";
    pub const RELEASE_NAME: &str = "mode";

    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            override_path: None,
        }
    }

    pub fn dev() -> Self {
        Self::named(Self::DEV_NAME)
    }

    pub fn release() -> Self {
        Self::named(Self::RELEASE_NAME)
    }

    /// `dev` in debug builds, `release` otherwise.
    pub fn for_build() -> Self {
        if cfg!(debug_assertions) {
            Self::dev()
        } else {
            Self::release()
        }
    }

    /// An endpoint at an explicit socket path (unix) or pipe name (windows).
    pub fn at(path: impl Into<String>) -> Self {
        Self {
            name: Self::RELEASE_NAME.into(),
            override_path: Some(path.into()),
        }
    }

    /// `MODE_ENDPOINT` when set and non-empty, else [`Endpoint::for_build`].
    pub fn from_env() -> Self {
        match std::env::var(ENV_OVERRIDE) {
            Ok(p) if !p.trim().is_empty() => Self::at(p),
            _ => Self::for_build(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the endpoint was given explicitly (`MODE_ENDPOINT` or [`Endpoint::at`]).
    pub fn is_override(&self) -> bool {
        self.override_path.is_some()
    }

    /// The `app_data_dir` folder name this endpoint belongs to (09 §2, 08 §7):
    /// `Mode` for the release name, `Mode-dev` for the dev name.
    pub fn app_dir_name(&self) -> String {
        match self.name.as_str() {
            Self::RELEASE_NAME => "Mode".into(),
            Self::DEV_NAME => "Mode-dev".into(),
            other => format!("Mode-{other}"),
        }
    }

    /// Linux: the directory the socket lives in (`$XDG_RUNTIME_DIR`, `$TMPDIR`, `/tmp`).
    #[cfg(unix)]
    pub fn runtime_dir() -> PathBuf {
        for var in ["XDG_RUNTIME_DIR", "TMPDIR"] {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() {
                    return PathBuf::from(v);
                }
            }
        }
        PathBuf::from("/tmp")
    }

    /// macOS: `~/Library/Application Support/<Mode|Mode-dev>` from `$HOME`
    /// (no config, no `$TMPDIR`: the relay inherits the terminal's environment,
    /// which may differ from the app's). `None` without a `$HOME`.
    #[cfg(unix)]
    pub fn macos_app_data_dir(&self) -> Option<PathBuf> {
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
        Some(
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(self.app_dir_name()),
        )
    }

    #[cfg(unix)]
    pub fn socket_path(&self) -> PathBuf {
        if let Some(p) = &self.override_path {
            return PathBuf::from(p);
        }
        if cfg!(target_os = "macos") {
            if let Some(dir) = self.macos_app_data_dir() {
                return dir.join("mode.sock");
            }
        }
        Self::runtime_dir().join(format!("{}.sock", self.name))
    }

    /// `\\.\pipe\<name>-<sid>`; without a resolvable SID the bare name (the
    /// core's DACL still restricts the pipe to the user).
    #[cfg(windows)]
    pub fn pipe_name(&self) -> String {
        match &self.override_path {
            Some(p) => p.clone(),
            None => match crate::sid::current_user_sid() {
                Some(sid) => format!(r"\\.\pipe\{}-{sid}", self.name),
                None => format!(r"\\.\pipe\{}", self.name),
            },
        }
    }

    /// The endpoint as a string for diagnostics.
    pub fn describe(&self) -> String {
        #[cfg(unix)]
        {
            self.socket_path().display().to_string()
        }
        #[cfg(windows)]
        {
            self.pipe_name()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_build_profile() {
        assert_eq!(Endpoint::dev().name(), "mode-dev");
        assert_eq!(Endpoint::release().name(), "mode");
        let expected = if cfg!(debug_assertions) {
            "mode-dev"
        } else {
            "mode"
        };
        assert_eq!(Endpoint::for_build().name(), expected);
        assert_eq!(Endpoint::dev().app_dir_name(), "Mode-dev");
        assert_eq!(Endpoint::release().app_dir_name(), "Mode");
        assert_eq!(Endpoint::named("x").app_dir_name(), "Mode-x");
        assert!(Endpoint::at("/x").is_override());
        assert!(!Endpoint::dev().is_override());
    }

    #[cfg(unix)]
    #[test]
    fn unix_socket_path_uses_name_and_override() {
        let p = Endpoint::named("mode-test").socket_path();
        if cfg!(target_os = "macos") {
            assert!(
                p.ends_with("Application Support/Mode-mode-test/mode.sock"),
                "{}",
                p.display()
            );
        } else {
            assert!(p.ends_with("mode-test.sock"), "{}", p.display());
        }
        assert_eq!(
            Endpoint::at("/x/y.sock").socket_path(),
            PathBuf::from("/x/y.sock")
        );
        assert_eq!(Endpoint::at("/x/y.sock").describe(), "/x/y.sock");
        let dir = Endpoint::dev().macos_app_data_dir();
        if std::env::var_os("HOME").is_some_and(|h| !h.is_empty()) {
            let dir = dir.unwrap();
            assert!(
                dir.ends_with("Library/Application Support/Mode-dev"),
                "{}",
                dir.display()
            );
            // 104-byte sun_path limit (05 §3.2): typical home dirs leave ample room.
            assert!(dir.join("mode.sock").as_os_str().len() < 104);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_pipe_name_uses_name_sid_and_override() {
        let name = Endpoint::named("mode-test").pipe_name();
        assert!(name.starts_with(r"\\.\pipe\mode-test"), "{name}");
        if let Some(sid) = crate::sid::current_user_sid() {
            assert!(sid.starts_with("S-1-"), "{sid}");
            assert_eq!(name, format!(r"\\.\pipe\mode-test-{sid}"));
        }
        assert_eq!(Endpoint::at(r"\\.\pipe\x").pipe_name(), r"\\.\pipe\x");
    }
}
