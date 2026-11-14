//! Launch, resume and close of embedded sessions (05 §8.1, §8.2, §8.3):
//! finding `claude` (the `sessions.claude_path` setting, the login shell's
//! `PATH` on macOS, `where claude` on Windows preferring `claude.exe`, a
//! `.cmd` shim through `cmd.exe /d /c`), its arguments (never `--bare`,
//! ADR-0021), its environment (`MODE_LAUNCH_ID`, `TERM`, `COLORTERM`,
//! `LANG`), the `launched` row before any hook, the resume preconditions,
//! and "Close terminal" / quit with SIGINT (never SIGTERM, ADR-0005).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::api::{ApiError, LaunchArgs, LaunchResult, ResumeArgs};
use super::driver::{self, Action, HandOff};
use super::host::{self, PtyProc, SpawnSpec};
use super::{DEFAULT_SIZE, Events, FirstTurnOpts, Inner, Pty, PtyStarted};
use crate::agents::{self, LaunchFlags, PlanDirs};
use crate::push;
use crate::sessions::embedded::{self, NewLaunch, ProcessExit, ResumePrep};
use crate::sessions::liveness;
use crate::settings;

/// The permission modes the launch dialog offers (03 §3.5).
pub const PERMISSION_MODES: &[&str] = &[
    "default",
    "plan",
    "acceptEdits",
    "auto",
    "dontAsk",
    "bypassPermissions",
];

/// Variables removed from the inherited environment: a Mode started from a
/// Claude Code shell must not make `claude` think it is nested.
pub const ENV_REMOVE: &[&str] = &["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "MODE_LAUNCH_ID"];

/// A prompt longer than this goes through stdin, never argv.
pub const PROMPT_ARG_MAX: usize = 16 * 1024;

pub const NOT_FOUND: &str =
    "claude was not found on PATH; set its location on the Claude Code card in Integrations";

// ----------------------------------------------------------------- binary

/// How to start `claude`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCommand {
    /// What is executed (`cmd.exe` for a `.cmd` shim).
    pub program: PathBuf,
    /// Arguments before `claude`'s own (`/d /c <shim>`).
    pub prefix: Vec<OsString>,
    /// The `claude` binary or shim.
    pub path: PathBuf,
    /// `setting` | `path` | `where` | `fallback` | `config`.
    pub source: &'static str,
}

impl ClaudeCommand {
    pub fn direct(path: impl Into<PathBuf>, source: &'static str) -> Self {
        let path = path.into();
        ClaudeCommand {
            program: path.clone(),
            prefix: Vec::new(),
            path,
            source,
        }
    }

    /// A `.cmd` / `.bat` shim runs through `cmd.exe /d /c` (05 §8.1).
    pub fn via_cmd(path: impl Into<PathBuf>, source: &'static str) -> Self {
        let path = path.into();
        let comspec = std::env::var_os("ComSpec")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cmd.exe"));
        ClaudeCommand {
            program: comspec,
            prefix: vec!["/d".into(), "/c".into(), path.clone().into_os_string()],
            path,
            source,
        }
    }

    /// The right form for a path (shims by extension).
    pub fn for_path(path: impl Into<PathBuf>, source: &'static str) -> Self {
        let path = path.into();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        match ext.as_deref() {
            Some("cmd" | "bat") => Self::via_cmd(path, source),
            _ => Self::direct(path, source),
        }
    }

    /// Arguments pass through `cmd.exe` (prompts must not).
    pub fn is_shim(&self) -> bool {
        !self.prefix.is_empty()
    }
}

fn is_executable(p: &Path) -> bool {
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

/// The first `names` entry found in a `PATH`-style list.
pub fn find_in_path(path_var: &OsStr, names: &[&str]) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for n in names {
            let candidate = dir.join(n);
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// `where claude` output → `claude.exe` first, else a `.cmd` / `.bat` shim;
/// extensionless shell scripts (npm also writes one) cannot be started.
pub fn pick_where(output: &str) -> Option<PathBuf> {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let with = |ext: &[&str]| {
        lines
            .iter()
            .find(|l| {
                let lower = l.to_ascii_lowercase();
                ext.iter().any(|e| lower.ends_with(e))
            })
            .map(PathBuf::from)
    };
    with(&[".exe"]).or_else(|| with(&[".cmd", ".bat"]))
}

/// `PATH` from `<shell> -l -c …`, between markers so login banners do not
/// matter. Bounded by `timeout`; `None` when the shell fails.
pub fn capture_login_path(shell: &str, timeout: Duration) -> Option<OsString> {
    use std::io::Read;
    let mut child = std::process::Command::new(shell)
        .args(["-l", "-c", "printf '\\n__MODE_PATH__=%s\\n' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                // A hung login shell (not `claude`) is ended outright.
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    parse_marked_path(&out).map(OsString::from)
}

/// The value after the `__MODE_PATH__=` marker.
pub fn parse_marked_path(out: &str) -> Option<String> {
    out.lines()
        .rev()
        .find_map(|l| l.strip_prefix("__MODE_PATH__="))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
}

/// The login shell's `PATH`, captured once (macOS: apps start with a
/// minimal `PATH`; elsewhere the process `PATH` is the user's).
pub fn login_path() -> Option<OsString> {
    #[cfg(target_os = "macos")]
    {
        static CAPTURED: std::sync::OnceLock<Option<OsString>> = std::sync::OnceLock::new();
        CAPTURED
            .get_or_init(|| {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
                capture_login_path(&shell, Duration::from_secs(5))
            })
            .clone()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(windows)]
fn where_claude() -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("where")
        .arg("claude")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    pick_where(&String::from_utf8_lossy(&out.stdout))
}

fn claude_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["claude.exe", "claude.cmd"]
    } else {
        &["claude"]
    }
}

fn fallback_locations() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    if cfg!(windows) {
        vec![home.join(".local").join("bin").join("claude.exe")]
    } else {
        vec![
            home.join(".local").join("bin").join("claude"),
            home.join(".claude").join("local").join("claude"),
        ]
    }
}

/// 05 §8.1 "Binary": the setting if set, else the login `PATH` (`where` on
/// Windows), else the installer's usual places.
pub fn resolve(setting: Option<&str>) -> Result<ClaudeCommand, String> {
    if let Some(p) = setting.map(str::trim).filter(|p| !p.is_empty()) {
        let path = match p.strip_prefix("~/") {
            Some(rest) => dirs::home_dir()
                .map(|h| h.join(rest))
                .unwrap_or_else(|| PathBuf::from(p)),
            None => PathBuf::from(p),
        };
        if !path.is_file() {
            return Err(format!(
                "the claude path set on the Claude Code card does not exist: {p}"
            ));
        }
        return Ok(ClaudeCommand::for_path(path, "setting"));
    }
    #[cfg(windows)]
    if let Some(p) = where_claude() {
        return Ok(ClaudeCommand::for_path(p, "where"));
    }
    let path_var = login_path().or_else(|| std::env::var_os("PATH"));
    if let Some(p) = path_var
        .as_deref()
        .and_then(|v| find_in_path(v, claude_names()))
    {
        return Ok(ClaudeCommand::for_path(p, "path"));
    }
    fallback_locations()
        .into_iter()
        .find(|p| is_executable(p))
        .map(|p| ClaudeCommand::for_path(p, "fallback"))
        .ok_or_else(|| NOT_FOUND.to_string())
}

// -------------------------------------------------------------- arguments

pub fn check_permission_mode(m: &str) -> Result<(), String> {
    if PERMISSION_MODES.contains(&m) {
        Ok(())
    } else {
        Err(format!(
            "{m:?} is not a permission mode ({})",
            PERMISSION_MODES.join(", ")
        ))
    }
}

/// A model override is one plain token (an alias or an id, `[1m]` style
/// suffixes allowed); never something argv or `cmd.exe` could read as a flag
/// or an operator.
pub fn check_model(m: &str) -> Result<(), String> {
    let ok = !m.is_empty()
        && m.len() <= 100
        && m.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && m.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '[' | ']' | '/')
        });
    if ok {
        Ok(())
    } else {
        Err(format!("{m:?} is not a model name"))
    }
}

/// How the prompt reaches a headless `claude -p`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptVia {
    /// `claude -p "<prompt>"` (05 §8.5).
    Arg,
    /// Piped on stdin: through a `cmd.exe` shim (no safe quoting for `%` /
    /// `&`), a leading `-` (read as a flag), or longer than argv allows.
    Stdin,
}

pub fn prompt_via(prompt: &str, shim: bool) -> PromptVia {
    if shim || prompt.starts_with('-') || prompt.len() > PROMPT_ARG_MAX || prompt.contains('\0') {
        PromptVia::Stdin
    } else {
        PromptVia::Arg
    }
}

/// What `claude` is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation<'a> {
    /// `claude [--permission-mode m] [--model x]` (05 §8.1).
    Interactive {
        permission_mode: Option<&'a str>,
        model: Option<&'a str>,
    },
    /// `claude --resume <id> [--permission-mode m] [--model x]` (05 §8.2).
    Resume {
        claude_session_id: &'a str,
        permission_mode: Option<&'a str>,
        model: Option<&'a str>,
    },
    /// `claude -p ["<prompt>"] [--resume <id>] --output-format stream-json
    /// --verbose --include-partial-messages` (05 §8.5).
    Headless {
        prompt: Option<&'a str>,
        resume: Option<&'a str>,
        permission_mode: Option<&'a str>,
        model: Option<&'a str>,
    },
}

/// The argument list after the binary. Never `--bare` (ADR-0021).
pub fn build_args(inv: &Invocation<'_>) -> Vec<String> {
    build_args_with(inv, &LaunchFlags::default())
}

/// [`build_args`] followed by an agent profile's flags (03 §7E, 05 §8.1):
/// `--effort`, `--fallback-model a,b`, `--allowedTools` / `--disallowedTools`,
/// the system prompt (file or inline), `--add-dir` per table and per
/// repository other than the session's folder, `--mcp-config <file>
/// --strict-mcp-config` for the agent's MCP servers, `--settings`
/// for the skills turned off, then the extra arguments as given. `model`
/// and `permission_mode` of the flags are not read here: the caller merges
/// them into the invocation (a launch's own choice wins).
pub fn build_args_with(inv: &Invocation<'_>, flags: &LaunchFlags) -> Vec<String> {
    let mut a = invocation_args(inv);
    let opt = |a: &mut Vec<String>, flag: &str, v: Option<&str>| {
        if let Some(v) = v {
            a.push(flag.into());
            a.push(v.into());
        }
    };
    opt(&mut a, "--effort", flags.effort.as_deref());
    if !flags.fallback_models.is_empty() {
        opt(
            &mut a,
            "--fallback-model",
            Some(&flags.fallback_models.join(",")),
        );
    }
    if !flags.allowed_tools.is_empty() {
        a.push("--allowedTools".into());
        a.extend(flags.allowed_tools.iter().cloned());
    }
    if !flags.denied_tools.is_empty() {
        a.push("--disallowedTools".into());
        a.extend(flags.denied_tools.iter().cloned());
    }
    if let Some(f) = &flags.system_prompt_file {
        opt(
            &mut a,
            "--append-system-prompt-file",
            Some(&f.to_string_lossy()),
        );
    } else {
        opt(
            &mut a,
            "--append-system-prompt",
            flags.system_prompt.as_deref(),
        );
    }
    for d in &flags.add_dirs {
        opt(&mut a, "--add-dir", Some(d));
    }
    opt(&mut a, "--mcp-config", flags.mcp_config.as_deref());
    if flags.strict_mcp_config && flags.mcp_config.is_some() {
        a.push("--strict-mcp-config".into());
    }
    opt(&mut a, "--settings", flags.settings_json.as_deref());
    a.extend(
        flags
            .extra_args
            .iter()
            .filter(|x| x.as_str() != "--bare")
            .cloned(),
    );
    debug_assert!(!a.iter().any(|x| x == "--bare"));
    a
}

fn invocation_args(inv: &Invocation<'_>) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let opt = |a: &mut Vec<String>, flag: &str, v: Option<&str>| {
        if let Some(v) = v {
            a.push(flag.into());
            a.push(v.into());
        }
    };
    match inv {
        Invocation::Interactive {
            permission_mode,
            model,
        } => {
            opt(&mut a, "--permission-mode", *permission_mode);
            opt(&mut a, "--model", *model);
        }
        Invocation::Resume {
            claude_session_id,
            permission_mode,
            model,
        } => {
            a.push("--resume".into());
            a.push((*claude_session_id).into());
            opt(&mut a, "--permission-mode", *permission_mode);
            opt(&mut a, "--model", *model);
        }
        Invocation::Headless {
            prompt,
            resume,
            permission_mode,
            model,
        } => {
            a.push("-p".into());
            if let Some(p) = prompt {
                a.push((*p).into());
            }
            opt(&mut a, "--resume", *resume);
            a.extend(
                [
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--include-partial-messages",
                ]
                .map(String::from),
            );
            opt(&mut a, "--permission-mode", *permission_mode);
            opt(&mut a, "--model", *model);
        }
    }
    a
}

/// 05 §8.1 "Environment": the inherited (login) environment plus these.
pub fn launch_env(
    launch_id: &str,
    lang_set: bool,
    path: Option<&OsStr>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = vec![
        ("MODE_LAUNCH_ID".into(), launch_id.into()),
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
    ];
    if !lang_set {
        env.push(("LANG".into(), "en_US.UTF-8".into()));
    }
    if let Some(p) = path {
        env.push(("PATH".into(), p.to_os_string()));
    }
    env
}

/// Folder checks shared by launch and resume.
pub fn check_dir(cwd: &str) -> Result<PathBuf, String> {
    let p = crate::paths::to_os(cwd);
    if cwd.trim().is_empty() || !p.is_absolute() {
        return Err("choose a folder (an absolute path)".into());
    }
    if !p.is_dir() {
        return Err(format!("the folder no longer exists: {cwd}"));
    }
    Ok(p)
}

fn check_size(cols: Option<u16>, rows: Option<u16>) -> Result<(u16, u16), ApiError> {
    let c = cols.unwrap_or(DEFAULT_SIZE.0);
    let r = rows.unwrap_or(DEFAULT_SIZE.1);
    if !(2..=1000).contains(&c) || !(1..=1000).contains(&r) {
        return Err(ApiError::invalid(format!(
            "terminal size {c}×{r} is out of range"
        )));
    }
    Ok((c, r))
}

// ---------------------------------------------------------------- service

/// What [`Inner::spawn_pty`] starts.
pub(crate) struct PtyStart<'a> {
    pub row_id: &'a str,
    pub launch_id: &'a str,
    pub claude: &'a ClaudeCommand,
    pub args: Vec<String>,
    pub cwd: &'a Path,
    pub size: (u16, u16),
    /// Infer the state from output (hooks not installed).
    pub infer: bool,
    /// The agent profile's environment (03 §7E), after Mode's own.
    pub env: Vec<(String, String)>,
}

impl Inner {
    /// The configured or discovered `claude`.
    pub(crate) fn claude(&self) -> Result<ClaudeCommand, ApiError> {
        if let Some(c) = &self.config.claude {
            return Ok(c.clone());
        }
        let setting = settings::get_string(self.engine.db(), "sessions.claude_path")
            .ok()
            .flatten();
        resolve(setting.as_deref()).map_err(ApiError::claude_missing)
    }

    /// The `claude` an agent profile names, else [`Inner::claude`].
    pub(crate) fn claude_for(&self, flags: &LaunchFlags) -> Result<ClaudeCommand, ApiError> {
        if self.config.claude.is_none()
            && let Some(p) = flags.claude_path.as_deref()
        {
            return resolve(Some(p)).map_err(|e| {
                ApiError::claude_missing(
                    e.replace("set on the Claude Code card", "set on the agent"),
                )
            });
        }
        self.claude()
    }

    /// The flags of the agent profile a launch names; nothing for none.
    pub(crate) fn flags_for(&self, profile_id: Option<&str>) -> Result<LaunchFlags, ApiError> {
        let Some(id) = profile_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(LaunchFlags::default());
        };
        let vault = settings::get_string(self.engine.db(), "notes.vault_path")
            .ok()
            .flatten()
            .map(|v| crate::paths::to_os(&crate::paths::canon(&v)));
        let dirs = PlanDirs {
            agents_dir: self.config.agents_dir.clone(),
            vault,
            keyring: self.config.keyring.clone(),
            claude_json: None,
        };
        let id = id.to_string();
        self.engine
            .db()
            .read(|c| agents::plan(c, &id, &dirs))?
            .ok_or_else(|| ApiError::not_found("no such agent"))
    }

    /// Environment of every `claude` Mode starts.
    pub(crate) fn env_for(&self, launch_id: &str) -> Vec<(OsString, OsString)> {
        let lang_set = std::env::var_os("LANG").is_some_and(|v| !v.is_empty());
        let mut env = launch_env(launch_id, lang_set, login_path().as_deref());
        env.extend(
            self.config
                .extra_env
                .iter()
                .map(|(k, v)| (OsString::from(k), OsString::from(v))),
        );
        env
    }

    /// Spawn `claude` in a new PTY for `row_id` and announce it.
    pub(crate) fn spawn_pty(self: &Arc<Self>, s: PtyStart<'_>) -> Result<Arc<PtyProc>, String> {
        let PtyStart {
            row_id,
            launch_id,
            claude,
            args,
            cwd,
            size,
            infer,
            env: extra_env,
        } = s;
        self.engine
            .embedded()
            .register(launch_id, row_id, "embedded");
        let mut argv: Vec<OsString> = claude.prefix.clone();
        argv.extend(args.into_iter().map(OsString::from));
        let mut env = self.env_for(launch_id);
        env.extend(
            extra_env
                .into_iter()
                .map(|(k, v)| (OsString::from(k), OsString::from(v))),
        );
        let spec = SpawnSpec {
            program: claude.program.clone().into_os_string(),
            args: argv,
            cwd: cwd.to_path_buf(),
            env,
            env_remove: ENV_REMOVE.iter().map(OsString::from).collect(),
            cols: size.0,
            rows: size.1,
        };
        let events = Arc::new(Events(Arc::clone(self)));
        let proc = match host::spawn(&spec, launch_id, row_id, &self.proc_opts(infer), events) {
            Ok(p) => p,
            Err(e) => {
                self.engine.embedded().forget(launch_id);
                return Err(e);
            }
        };
        if let Some(pid) = proc.pid() {
            self.engine.record_process(row_id, pid);
        }
        self.host.insert(Arc::clone(&proc));
        self.emit(
            push::Event::PtyStarted,
            &PtyStarted {
                session_id: row_id.to_string(),
                cols: size.0,
                rows: size.1,
            },
        );
        tracing::info!(target: "pty", session = %row_id, pid = ?proc.pid(), source = claude.source, "terminal started");
        Ok(proc)
    }

    /// 05 §16 "PTY spawn fails": the row turns `failed` with the error line.
    fn spawn_failed(&self, row_id: &str, generation: u64, error: &str) {
        self.drivers.release(row_id, HandOff::PtyExited, generation);
        let exit = ProcessExit {
            source: "pty",
            error: Some(error.to_string()),
            ..Default::default()
        };
        self.engine_pushes(
            "spawn failure",
            self.engine.record_process_exit(row_id, &exit),
        );
    }
}

impl Pty {
    /// `session_launch` (05 §8.1, 03 §3.5).
    pub fn launch(&self, args: &LaunchArgs) -> Result<LaunchResult, ApiError> {
        let inner = &self.inner;
        // The agent profile's flags; the launch's own model and permission
        // mode win over the profile's defaults (03 §7E.3). No folder: the
        // agent's first repository (03 §7E.2 Does).
        let flags = inner.flags_for(args.profile_id.as_deref())?;
        let folder = match args.cwd.trim() {
            "" => flags
                .start_dir()
                .map(str::to_string)
                .ok_or_else(|| ApiError::invalid("choose a folder".to_string()))?,
            f => f.to_string(),
        };
        let cwd = check_dir(&folder).map_err(ApiError::invalid)?;
        let flags = flags.for_cwd(&cwd);
        let permission_mode = args
            .permission_mode
            .clone()
            .or_else(|| flags.permission_mode.clone());
        let model = args.model.clone().or_else(|| flags.model.clone());
        if let Some(m) = permission_mode.as_deref() {
            check_permission_mode(m).map_err(ApiError::invalid)?;
        }
        if let Some(m) = model.as_deref() {
            check_model(m).map_err(ApiError::invalid)?;
        }
        let size = check_size(args.cols, args.rows)?;
        let claude = inner.claude_for(&flags)?;
        let hooks = inner.engine.hooks_installed().unwrap_or(false);
        let launch_id = uuid::Uuid::now_v7().to_string();
        let (row, pushes) = inner.engine.create_launched(&NewLaunch {
            launch_id: launch_id.clone(),
            cwd: crate::paths::canon(&folder),
            permission_mode: permission_mode.clone(),
            profile_id: args
                .profile_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        })?;
        inner.publish(&pushes);
        if args.overview_only.unwrap_or(false) {
            // 05 §8.5: no terminal; the first prompt starts a headless turn.
            inner.engine.embedded().forget(&launch_id);
            super::lock(&inner.first_turn).insert(
                row.id.clone(),
                FirstTurnOpts {
                    permission_mode,
                    model,
                    flags,
                },
            );
            return Ok(LaunchResult {
                session_id: row.id,
                hooks_installed: hooks,
                error: None,
                already_open: false,
            });
        }
        let (_, generation) = inner
            .drivers
            .apply(&row.id, HandOff::Launch)
            .map_err(ApiError::refused_plain)?;
        let argv = build_args_with(
            &Invocation::Interactive {
                permission_mode: permission_mode.as_deref(),
                model: model.as_deref(),
            },
            &flags,
        );
        let start = PtyStart {
            row_id: &row.id,
            launch_id: &launch_id,
            claude: &claude,
            args: argv,
            cwd: &cwd,
            size,
            infer: !hooks,
            env: flags.env.clone(),
        };
        let error = match inner.spawn_pty(start) {
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(target: "pty", session = %row.id, error = %e, "terminal did not start");
                inner.spawn_failed(&row.id, generation, &e);
                Some(e)
            }
        };
        Ok(LaunchResult {
            session_id: row.id,
            hooks_installed: hooks,
            error,
            already_open: false,
        })
    }

    /// `session_resume` (05 §8.2, §14): `claude --resume <id>` in the
    /// session's folder; the live terminal when one is already open; a fresh
    /// `claude` for an "Overview only" row that never ran ("Open a terminal").
    pub fn resume(&self, args: &ResumeArgs) -> Result<LaunchResult, ApiError> {
        let inner = &self.inner;
        let row = inner
            .engine
            .session_row(&args.session_id)?
            .ok_or_else(|| ApiError::not_found("no such session"))?;
        if row.parent_session_id.is_some() {
            return Err(ApiError::invalid("resume the parent session"));
        }
        let size = check_size(args.cols, args.rows)?;
        if let Some(m) = args.permission_mode.as_deref() {
            check_permission_mode(m).map_err(ApiError::invalid)?;
        }
        if let Some(m) = args.model.as_deref() {
            check_model(m).map_err(ApiError::invalid)?;
        }
        let hooks = inner.engine.hooks_installed().unwrap_or(false);
        match inner.drivers.get(&row.id) {
            driver::Driver::Headless => {
                return Err(ApiError::refused_plain(driver::STOP_TURN_FIRST));
            }
            driver::Driver::Pty if inner.host.live(&row.id).is_some() => {
                return Ok(LaunchResult {
                    session_id: row.id,
                    hooks_installed: hooks,
                    error: None,
                    already_open: true,
                });
            }
            _ => {}
        }
        let foreign = row.ended_at.is_none()
            && inner
                .engine
                .process(&row.id)
                .is_some_and(liveness::pid_alive);
        if foreign {
            return Err(ApiError::refused(
                driver::say::ELSEWHERE,
                Some(Action::Jump),
            ));
        }
        let folder = args.cwd.clone().unwrap_or_else(|| row.cwd.clone());
        let cwd = check_dir(&folder).map_err(|e| ApiError::new("cwd_missing", e))?;
        let placeholder = embedded::is_placeholder(&row.claude_session_id);
        if !placeholder {
            if let Some(t) = row.transcript_path.as_deref() {
                if !crate::paths::to_os(t).exists() {
                    return Err(ApiError::new(
                        "transcript_missing",
                        format!("the transcript is gone, so this session cannot be resumed: {t}"),
                    ));
                }
            }
        }
        let first = super::lock(&inner.first_turn)
            .get(&row.id)
            .cloned()
            .unwrap_or_default();
        // A placeholder row keeps the flags of its launch; a session that ran
        // is resumed with its agent's current flags (03 §7E.3).
        let flags = if placeholder {
            first.flags.clone()
        } else {
            inner.flags_for(row.profile_id.as_deref())?
        }
        .for_cwd(&cwd);
        let claude = inner.claude_for(&flags)?;
        let (_, generation) = inner
            .drivers
            .apply(&row.id, HandOff::Resume)
            .map_err(ApiError::refused_plain)?;
        let pushes = inner.engine.prepare_resume(
            &row.id,
            &ResumePrep {
                launch_mode: "embedded".into(),
                permission_mode: args.permission_mode.clone(),
                cwd: args.cwd.as_deref().map(crate::paths::canon),
                reopen: !hooks || placeholder,
            },
        );
        inner.engine_pushes("resume", pushes);
        super::lock(&inner.first_turn).remove(&row.id);
        let argv = if placeholder {
            build_args_with(
                &Invocation::Interactive {
                    permission_mode: args
                        .permission_mode
                        .as_deref()
                        .or(first.permission_mode.as_deref()),
                    model: args.model.as_deref().or(first.model.as_deref()),
                },
                &flags,
            )
        } else {
            build_args_with(
                &Invocation::Resume {
                    claude_session_id: &row.claude_session_id,
                    permission_mode: args.permission_mode.as_deref(),
                    model: args.model.as_deref(),
                },
                &flags,
            )
        };
        let launch_id = uuid::Uuid::now_v7().to_string();
        let start = PtyStart {
            row_id: &row.id,
            launch_id: &launch_id,
            claude: &claude,
            args: argv,
            cwd: &cwd,
            size,
            infer: !hooks,
            env: flags.env.clone(),
        };
        let error = match inner.spawn_pty(start) {
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(target: "pty", session = %row.id, error = %e, "terminal did not start");
                inner.spawn_failed(&row.id, generation, &e);
                Some(e)
            }
        };
        Ok(LaunchResult {
            session_id: row.id,
            hooks_installed: hooks,
            error,
            already_open: false,
        })
    }

    /// "Close terminal" (05 §8.3, 03 §3.8): SIGINT, wait for the turn to end
    /// (the engine says when: the row leaves `working`; or the output has
    /// been silent for the idle window), then let `claude` exit — a second
    /// SIGINT at the idle prompt, then end-of-input, then the hang-up a
    /// closed terminal sends. Never SIGTERM. Returns at once; `pty.exited`
    /// follows.
    pub fn close(&self, session_id: &str) -> Result<(), ApiError> {
        let proc = self
            .inner
            .host
            .live(session_id)
            .ok_or_else(|| ApiError::not_found("no terminal is open for this session"))?;
        if proc.closing.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        proc.sigint().map_err(ApiError::io)?;
        let inner = Arc::clone(&self.inner);
        std::thread::Builder::new()
            .name("mode-pty-close".into())
            .spawn(move || close_ladder(&inner, &proc))
            .map_err(ApiError::io)?;
        Ok(())
    }

    /// Quit (03 §2.1, 08 §2.6): close every live terminal and stop every
    /// headless turn, then wait up to `wait` for them to end. Returns the
    /// sessions signaled and those still running.
    pub fn quit_all(&self, wait: Duration) -> (Vec<String>, Vec<String>) {
        let mut signalled = Vec::new();
        for p in self.inner.host.all_live() {
            let id = p.row_id();
            if self.close(&id).is_ok() {
                signalled.push(id);
            }
        }
        for id in self.inner.turns.running() {
            if self.stop_turn(&id).is_ok() {
                signalled.push(id);
            }
        }
        let deadline = Instant::now() + wait;
        loop {
            let mut remaining: Vec<String> = self
                .inner
                .host
                .all_live()
                .iter()
                .map(|p| p.row_id())
                .collect();
            remaining.extend(self.inner.turns.running());
            if remaining.is_empty() || Instant::now() >= deadline {
                return (signalled, remaining);
            }
            std::thread::sleep(self.inner.config.timings.tick);
        }
    }
}

fn wait_exit(proc: &PtyProc, d: Duration, tick: Duration) -> bool {
    let until = Instant::now() + d;
    while Instant::now() < until {
        if !proc.is_alive() {
            return true;
        }
        std::thread::sleep(tick);
    }
    !proc.is_alive()
}

fn close_ladder(inner: &Inner, proc: &Arc<PtyProc>) {
    let t = &inner.config.timings;
    let row = proc.row_id();
    let deadline = Instant::now() + t.turn_wait;
    std::thread::sleep(t.tick.min(Duration::from_millis(200)));
    while proc.is_alive() && Instant::now() < deadline {
        let over =
            inner.engine.turn_over(&row).unwrap_or(true) || proc.silent_for() >= t.idle_after;
        if over {
            break;
        }
        std::thread::sleep(t.tick);
    }
    if wait_exit(proc, Duration::ZERO, t.tick) {
        return;
    }
    let steps: [(&str, &dyn Fn()); 3] = [
        ("sigint", &|| {
            let _ = proc.sigint();
        }),
        ("eof", &|| {
            let _ = proc.eof();
        }),
        ("hangup", &|| proc.hangup()),
    ];
    for (name, step) in steps {
        if !proc.is_alive() {
            return;
        }
        tracing::debug!(target: "pty", session = %row, step = name, "close terminal");
        step();
        if wait_exit(proc, t.exit_step, t.tick) {
            return;
        }
    }
    tracing::warn!(target: "pty", session = %row, "claude did not exit after close; left running");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_output_prefers_the_exe() {
        let out = "C:\\Users\\a b\\AppData\\Roaming\\npm\\claude\r\nC:\\Users\\a b\\AppData\\Roaming\\npm\\claude.cmd\r\nC:\\Users\\a b\\.local\\bin\\claude.exe\r\n";
        assert_eq!(
            pick_where(out).unwrap(),
            PathBuf::from("C:\\Users\\a b\\.local\\bin\\claude.exe")
        );
        let shim_only = "C:\\npm\\claude\nC:\\npm\\claude.CMD\n";
        assert_eq!(
            pick_where(shim_only).unwrap(),
            PathBuf::from("C:\\npm\\claude.CMD")
        );
        assert_eq!(
            pick_where("C:\\npm\\claude\n"),
            None,
            "a sh script cannot start"
        );
        assert_eq!(pick_where(""), None);
    }

    #[test]
    fn shims_go_through_cmd_exe() {
        let c = ClaudeCommand::for_path("C:\\npm\\claude.cmd", "where");
        assert!(c.is_shim());
        assert_eq!(c.prefix[..2], [OsString::from("/d"), OsString::from("/c")]);
        assert_eq!(c.prefix[2], OsString::from("C:\\npm\\claude.cmd"));
        let c = ClaudeCommand::for_path("/usr/local/bin/claude", "path");
        assert!(!c.is_shim());
        assert_eq!(c.program, PathBuf::from("/usr/local/bin/claude"));
    }

    #[test]
    fn arguments_follow_the_spec_and_never_bare() {
        assert_eq!(
            build_args(&Invocation::Interactive {
                permission_mode: Some("plan"),
                model: Some("test-model")
            }),
            ["--permission-mode", "plan", "--model", "test-model"]
        );
        assert!(
            build_args(&Invocation::Interactive {
                permission_mode: None,
                model: None
            })
            .is_empty()
        );
        assert_eq!(
            build_args(&Invocation::Resume {
                claude_session_id: "abc",
                permission_mode: None,
                model: None,
            }),
            ["--resume", "abc"]
        );
        assert_eq!(
            build_args(&Invocation::Headless {
                prompt: Some("fix it"),
                resume: Some("abc"),
                permission_mode: Some("acceptEdits"),
                model: None,
            }),
            [
                "-p",
                "fix it",
                "--resume",
                "abc",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--permission-mode",
                "acceptEdits"
            ]
        );
        let stdin = build_args(&Invocation::Headless {
            prompt: None,
            resume: None,
            permission_mode: None,
            model: None,
        });
        assert_eq!(stdin[0], "-p");
        assert_eq!(stdin[1], "--output-format");
    }

    #[test]
    fn inputs_are_validated() {
        for m in PERMISSION_MODES {
            assert!(check_permission_mode(m).is_ok());
        }
        assert!(check_permission_mode("yolo").is_err());
        assert!(check_model("test-model").is_ok());
        assert!(check_model("some-model-4-5[1m]").is_ok());
        assert!(check_model("--bare").is_err());
        assert!(check_model("a b").is_err());
        assert!(check_model("x&calc").is_err());
        assert!(check_model("").is_err());
        assert_eq!(prompt_via("fix the tests", false), PromptVia::Arg);
        assert_eq!(prompt_via("-h", false), PromptVia::Stdin);
        assert_eq!(prompt_via("100% & done", true), PromptVia::Stdin);
        assert_eq!(
            prompt_via(&"x".repeat(PROMPT_ARG_MAX + 1), false),
            PromptVia::Stdin
        );
        assert!(check_dir("relative/dir").is_err());
        assert!(check_dir("/definitely/not/here/mode").is_err());
        assert!(check_dir(env!("CARGO_MANIFEST_DIR")).is_ok());
    }

    #[test]
    fn environment_carries_the_join_key() {
        let env = launch_env("L-1", false, Some(OsStr::new("/opt/bin:/usr/bin")));
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        assert_eq!(get("MODE_LAUNCH_ID").as_deref(), Some("L-1"));
        assert_eq!(get("TERM").as_deref(), Some("xterm-256color"));
        assert_eq!(get("COLORTERM").as_deref(), Some("truecolor"));
        assert_eq!(get("LANG").as_deref(), Some("en_US.UTF-8"));
        assert_eq!(get("PATH").as_deref(), Some("/opt/bin:/usr/bin"));
        let env = launch_env("L-2", true, None);
        assert!(env.iter().all(|(k, _)| k != "LANG" && k != "PATH"));
    }

    #[test]
    fn path_search_and_marked_output() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("claude");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                find_in_path(dir.path().as_os_str(), &["claude"]),
                None,
                "not executable yet"
            );
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let joined =
            std::env::join_paths([Path::new("/nonexistent-mode-dir"), dir.path()]).unwrap();
        assert_eq!(find_in_path(&joined, &["claude"]), Some(bin.clone()));
        assert_eq!(
            parse_marked_path("Last login: today\nmotd\n__MODE_PATH__=/a:/b\n").as_deref(),
            Some("/a:/b")
        );
        assert_eq!(parse_marked_path("nothing"), None);
        assert!(resolve(Some("/nonexistent/claude")).is_err());
        assert_eq!(
            resolve(Some(bin.to_str().unwrap())).unwrap().source,
            "setting"
        );
    }

    #[cfg(unix)]
    #[test]
    fn login_shell_path_is_captured_between_markers() {
        let Some(sh) = ["/bin/bash", "/bin/sh"]
            .into_iter()
            .find(|s| Path::new(s).exists())
        else {
            return;
        };
        let p = capture_login_path(sh, Duration::from_secs(10)).expect("a PATH");
        assert!(!p.is_empty());
    }
}
