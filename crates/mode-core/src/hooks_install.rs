//! Hook install for `~/.claude/settings.json` (05 §3.1, §3.6; 03 §7C.2–§7C.3;
//! 09 `hook_installs`).
//!
//! The only time Mode writes a file it does not own, so the rules are strict:
//!
//! * Mode's entries are computed from the event table of 05 §3.1 (exec form
//!   with `args: ["<Event>", "<timeout>"]`, explicit `timeout`, `async` on
//!   every telemetry event).
//! * The file is edited as **text**: Mode's own entries are spliced in or
//!   out with the span scanner of `mode_wire::scan`, so every byte outside
//!   them (other keys, foreign hooks, key order, indentation, comments-free
//!   formatting) stays as it was. A file `serde_json` cannot parse (comments,
//!   trailing commas) is refused with a clear error.
//! * A foreign hook on one of Mode's events is kept and reported as a
//!   [`Conflict`]; it is never rewritten.
//! * `plan_*` computes the before/after text and a unified diff for the
//!   preview; `apply` backs the file up, writes atomically (temp file +
//!   rename), re-reads and verifies by `block_hash`, restoring the backup
//!   when verification fails.
//! * [`round_trip`] spawns the real relay with a synthetic `SessionStart`
//!   against a running [`crate::ipc::Server`] and waits for the frame.
//! * Project files (`.claude/settings.json`, `.claude/settings.local.json`)
//!   are never read or written.
//!
//! Development builds refuse to touch the real `~/.claude` unless
//! `MODE_CLAUDE_SETTINGS` or `CLAUDE_CONFIG_DIR` points somewhere else
//! (dev hook installs use a scratch `$HOME` only); tests always pass explicit paths.

use std::collections::BTreeMap;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mode_wire::{Endpoint, HookEvent, ROUND_TRIP_SESSION_PREFIX, events, scan};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::ipc::Server;

/// The relay version this core ships with (the workspace version).
pub const RELAY_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The relay's file name next to the app data (09 §2: `<app_data_dir>/bin/mode-hook`).
pub const RELAY_FILE_NAME: &str = if cfg!(windows) {
    "mode-hook.exe"
} else {
    "mode-hook"
};
/// Backups kept per directory (05 §3.6).
pub const BACKUPS_KEPT: usize = 5;
/// Overrides the settings file (tests, dev builds).
pub const SETTINGS_ENV_OVERRIDE: &str = "MODE_CLAUDE_SETTINGS";
/// Claude Code's own override of `~/.claude` (05 §9.2).
pub const CLAUDE_CONFIG_DIR: &str = "CLAUDE_CONFIG_DIR";
/// The synthetic round trip must complete within this (05 §3.6 S10).
pub const ROUND_TRIP_TIMEOUT: Duration = Duration::from_secs(1);

// ------------------------------------------------------------ event table

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Synchronous: the relay may wait for a decision (120 s).
    Decision,
    /// Telemetry: `async`, 5 s.
    Telemetry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventSpec {
    pub name: &'static str,
    /// `Some("")` writes `"matcher": ""`; `None` writes no matcher (05 §3.1).
    pub matcher: Option<&'static str>,
    pub kind: EventKind,
}

/// The event table of 05 §3.1, in the order the block is written.
pub const EVENTS: &[EventSpec] = &[
    EventSpec {
        name: events::PERMISSION_REQUEST,
        matcher: Some(""),
        kind: EventKind::Decision,
    },
    EventSpec {
        name: events::PRE_TOOL_USE,
        matcher: Some(""),
        kind: EventKind::Decision,
    },
    EventSpec {
        name: events::SESSION_START,
        matcher: Some(""),
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::SESSION_END,
        matcher: None,
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::USER_PROMPT_SUBMIT,
        matcher: None,
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::POST_TOOL_USE,
        matcher: Some(""),
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::POST_TOOL_USE_FAILURE,
        matcher: Some(""),
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::NOTIFICATION,
        matcher: Some(""),
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::STOP,
        matcher: None,
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::STOP_FAILURE,
        matcher: None,
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::SUBAGENT_START,
        matcher: Some(""),
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::SUBAGENT_STOP,
        matcher: Some(""),
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::PRE_COMPACT,
        matcher: Some(""),
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::POST_COMPACT,
        matcher: Some(""),
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::CWD_CHANGED,
        matcher: None,
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::PRE_MODEL_SWITCH,
        matcher: None,
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::POST_MODEL_SWITCH,
        matcher: None,
        kind: EventKind::Telemetry,
    },
    EventSpec {
        name: events::ELICITATION,
        matcher: None,
        kind: EventKind::Telemetry,
    },
];

fn event_spec(name: &str) -> Option<&'static EventSpec> {
    EVENTS.iter().find(|e| e.name == name)
}

/// `hooks.decision_timeout_seconds` / `hooks.async_timeout_seconds` (03 §7C.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timeouts {
    pub decision_secs: u64,
    pub async_secs: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            decision_secs: 120,
            async_secs: 5,
        }
    }
}

impl Timeouts {
    pub fn for_kind(&self, kind: EventKind) -> u64 {
        match kind {
            EventKind::Decision => self.decision_secs,
            EventKind::Telemetry => self.async_secs,
        }
    }
}

/// One row of the events list in the preview dialog (03 §7C.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLine {
    pub event: String,
    pub timeout: u64,
    #[serde(rename = "async")]
    pub is_async: bool,
    pub matcher: Option<String>,
}

pub fn event_lines(timeouts: &Timeouts) -> Vec<EventLine> {
    EVENTS
        .iter()
        .map(|e| EventLine {
            event: e.name.into(),
            timeout: timeouts.for_kind(e.kind),
            is_async: e.kind == EventKind::Telemetry,
            matcher: e.matcher.map(str::to_string),
        })
        .collect()
}

/// Mode's own entries, rendered in the key order of 05 §3.1 (`type`,
/// `command`, `args`, `timeout`, `async`). `serde_json::Value` sorts keys,
/// which is fine for hashing and lookups but not for the text Mode writes.
#[derive(Debug, Clone)]
enum Node {
    Object(Vec<(&'static str, Node)>),
    Array(Vec<Node>),
    Leaf(Value),
}

impl Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match self {
            Node::Object(members) => {
                let mut m = s.serialize_map(Some(members.len()))?;
                for (k, v) in members {
                    m.serialize_entry(k, v)?;
                }
                m.end()
            }
            Node::Array(items) => {
                let mut q = s.serialize_seq(Some(items.len()))?;
                for v in items {
                    q.serialize_element(v)?;
                }
                q.end()
            }
            Node::Leaf(v) => v.serialize(s),
        }
    }
}

fn hook_node(spec: &EventSpec, command: &str, timeouts: &Timeouts) -> Node {
    let timeout = timeouts.for_kind(spec.kind);
    let mut members = vec![
        ("type", Node::Leaf(Value::from("command"))),
        ("command", Node::Leaf(Value::from(command))),
        (
            "args",
            Node::Array(vec![
                Node::Leaf(Value::from(spec.name)),
                Node::Leaf(Value::from(timeout.to_string())),
            ]),
        ),
        ("timeout", Node::Leaf(Value::from(timeout))),
    ];
    if spec.kind == EventKind::Telemetry {
        members.push(("async", Node::Leaf(Value::Bool(true))));
    }
    Node::Object(members)
}

fn group_node(spec: &EventSpec, command: &str, timeouts: &Timeouts) -> Node {
    let mut members = Vec::new();
    if let Some(matcher) = spec.matcher {
        members.push(("matcher", Node::Leaf(Value::from(matcher))));
    }
    members.push((
        "hooks",
        Node::Array(vec![hook_node(spec, command, timeouts)]),
    ));
    Node::Object(members)
}

/// Mode's block as written: event → `[group]`, in table order.
fn block_nodes(command: &str, timeouts: &Timeouts) -> Vec<(&'static str, Node)> {
    EVENTS
        .iter()
        .map(|e| (e.name, Node::Array(vec![group_node(e, command, timeouts)])))
        .collect()
}

/// The group inside an event's `[group]` node.
fn first_group(node: &Node) -> &Node {
    match node {
        Node::Array(items) if !items.is_empty() => &items[0],
        other => other,
    }
}

fn to_value(node: &Node) -> Value {
    serde_json::to_value(node).expect("a node is plain JSON")
}

/// The command entry Mode registers for one event (exec form, 05 §3.1).
pub fn hook_entry(spec: &EventSpec, command: &str, timeouts: &Timeouts) -> Value {
    to_value(&hook_node(spec, command, timeouts))
}

/// The matcher group holding Mode's command entry.
pub fn group_entry(spec: &EventSpec, command: &str, timeouts: &Timeouts) -> Value {
    to_value(&group_node(spec, command, timeouts))
}

/// Mode's block: event name → `[group]`, in table order.
pub fn mode_entries(command: &str, timeouts: &Timeouts) -> Map<String, Value> {
    EVENTS
        .iter()
        .map(|e| {
            (
                e.name.to_string(),
                Value::Array(vec![group_entry(e, command, timeouts)]),
            )
        })
        .collect()
}

/// The block for **Copy JSON** (03 §7C.2), pretty-printed.
pub fn copy_json(command: &str, timeouts: &Timeouts) -> String {
    let root = Node::Object(vec![(
        "hooks",
        Node::Object(block_nodes(command, timeouts)),
    )]);
    render(&root, "  ", "")
}

fn canonical(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let sorted: BTreeMap<&String, Value> =
                m.iter().map(|(k, x)| (k, canonical(x))).collect();
            Value::Object(sorted.into_iter().map(|(k, x)| (k.clone(), x)).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

/// `hook_installs.block_hash`: sha256 over Mode's entries in canonical form
/// (sorted keys, compact), so key order and whitespace never matter.
pub fn block_hash(entries: &Map<String, Value>) -> String {
    let canonical = canonical(&Value::Object(entries.clone()));
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ------------------------------------------------------------------ targets

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetKind {
    Native,
    /// A WSL distribution's home on Windows (05 §3.6 "Targets").
    Wsl {
        distro: String,
    },
}

/// One settings file Mode edits (`hook_installs.scope = 'user'`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub settings_path: PathBuf,
    /// `hook_installs.relay_path`: the relay as the app sees it.
    pub relay_path: PathBuf,
    /// The `command` written into the file: the relay path, or its `/mnt/<drive>/…` form for WSL.
    pub command: String,
    pub kind: TargetKind,
}

impl Target {
    pub fn native(settings_path: impl Into<PathBuf>, relay_path: impl Into<PathBuf>) -> Self {
        let relay_path = relay_path.into();
        Self {
            settings_path: settings_path.into(),
            command: relay_path.to_string_lossy().into_owned(),
            relay_path,
            kind: TargetKind::Native,
        }
    }
}

/// `<app_data_dir>/bin/mode-hook[.exe]` (09 §2).
pub fn default_relay_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bin").join(RELAY_FILE_NAME)
}

/// The settings file of the current OS user: `MODE_CLAUDE_SETTINGS`, else
/// `$CLAUDE_CONFIG_DIR/settings.json`, else `~/.claude/settings.json`.
pub fn default_settings_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(SETTINGS_ENV_OVERRIDE).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    if let Some(dir) = std::env::var_os(CLAUDE_CONFIG_DIR).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(dir).join("settings.json"));
    }
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

/// The native target. Dev builds get one only with an override in the
/// environment (never the real `~/.claude`).
pub fn default_target(data_dir: &Path, dev: bool) -> Result<Target, InstallError> {
    let overridden = std::env::var_os(SETTINGS_ENV_OVERRIDE).is_some_and(|p| !p.is_empty())
        || std::env::var_os(CLAUDE_CONFIG_DIR).is_some_and(|p| !p.is_empty());
    if dev && !overridden {
        return Err(InstallError::DevBuildNeedsScratchHome);
    }
    let settings = default_settings_path().ok_or(InstallError::NoHome)?;
    Ok(Target::native(settings, default_relay_path(data_dir)))
}

/// Every install target: the native one, plus each WSL distribution with a
/// `~/.claude` when `include_wsl` (Windows only; a flag because `wsl.exe`
/// discovery spawns processes).
pub fn discover_targets(
    data_dir: &Path,
    dev: bool,
    include_wsl: bool,
) -> Result<Vec<Target>, InstallError> {
    let targets = vec![default_target(data_dir, dev)?];
    #[cfg(windows)]
    let targets = {
        let mut targets = targets;
        if include_wsl {
            targets.extend(wsl::discover(&default_relay_path(data_dir)));
        }
        targets
    };
    #[cfg(not(windows))]
    let _ = include_wsl;
    Ok(targets)
}

/// Copy the bundled relay to `<app_data_dir>/bin/` when missing or different
/// (08 §7 "copied from the bundle on every launch when the bundled version differs").
pub fn stage_relay(bundled: &Path, data_dir: &Path) -> io::Result<PathBuf> {
    let dest = default_relay_path(data_dir);
    let src_bytes = std::fs::read(bundled)?;
    let same = std::fs::read(&dest)
        .map(|b| b == src_bytes)
        .unwrap_or(false);
    if !same {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = dest.with_extension("staging");
        std::fs::write(&tmp, &src_bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(dest)
}

// ------------------------------------------------------------------- errors

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("{path}: not strict JSON (comments and trailing commas are not supported): {detail}")]
    Unparseable { path: PathBuf, detail: String },
    #[error("{path}: the top level is not a JSON object")]
    NotAnObject { path: PathBuf },
    #[error("{path}: `hooks` is not an object")]
    HooksNotAnObject { path: PathBuf },
    #[error("{path}: `hooks.{event}` is not an array")]
    EventNotAnArray { path: PathBuf, event: String },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("verification after writing failed ({detail}); {}", match restored { Some(p) => format!("restored the backup {}", p.display()), None => "nothing to restore".to_string() })]
    Verify {
        detail: String,
        restored: Option<PathBuf>,
    },
    #[error(
        "dev builds install hooks only into a scratch home: set MODE_CLAUDE_SETTINGS or CLAUDE_CONFIG_DIR"
    )]
    DevBuildNeedsScratchHome,
    #[error("no home directory to locate ~/.claude/settings.json")]
    NoHome,
}

fn io_err(path: &Path, source: io::Error) -> InstallError {
    InstallError::Io {
        path: path.to_path_buf(),
        source,
    }
}

// --------------------------------------------------------------- the editor

/// A hook on one of Mode's events that is not Mode's: kept as is, shown with
/// the gutter note "kept as is — not Mode's" (05 §3.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub event: String,
    pub command: String,
    pub matcher: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    /// `command` equals the target's relay command.
    Mode,
    /// A `mode-hook` at another path (an older install location).
    ModeElsewhere,
    Foreign,
}

fn is_relay_name(command: &str) -> bool {
    let base = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .trim_matches('"');
    base == "mode-hook" || base == "mode-hook.exe"
}

fn classify(hook: &Value, command: &str) -> Owner {
    let Some(cmd) = hook.get("command").and_then(Value::as_str) else {
        return Owner::Foreign;
    };
    if cmd == command {
        Owner::Mode
    } else if is_relay_name(cmd) {
        Owner::ModeElsewhere
    } else {
        Owner::Foreign
    }
}

fn hook_command(hook: &Value) -> String {
    hook.get("command")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(hook).unwrap_or_default())
}

struct Edit {
    range: Range<usize>,
    replacement: String,
}

fn apply_edits(text: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|e| e.range.start);
    let mut out = String::with_capacity(text.len() + 1024);
    let mut pos = 0;
    for e in edits {
        debug_assert!(
            e.range.start >= pos,
            "edits overlap: {}..{} after {pos}: {:?}",
            e.range.start,
            e.range.end,
            e.replacement
        );
        out.push_str(&text[pos..e.range.start]);
        out.push_str(&e.replacement);
        pos = e.range.end;
    }
    out.push_str(&text[pos..]);
    out
}

/// The indentation unit of the file: the whitespace before its first
/// indented member, else two spaces.
fn detect_indent_unit(text: &str) -> String {
    for line in text.lines() {
        let ws: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if !ws.is_empty() && line.trim_start().starts_with('"') {
            return ws;
        }
    }
    "  ".to_string()
}

fn line_start(text: &str, i: usize) -> usize {
    text[..i].rfind('\n').map(|p| p + 1).unwrap_or(0)
}

/// Leading whitespace of the line containing byte `i`.
fn line_indent(text: &str, i: usize) -> &str {
    let ls = line_start(text, i);
    let end = text[ls..]
        .find(|c: char| c != ' ' && c != '\t')
        .map(|p| ls + p)
        .unwrap_or(text.len());
    &text[ls..end.min(i.max(ls))]
}

/// Whether the token at `i` is the first thing on its line (block layout).
fn starts_line(text: &str, i: usize) -> bool {
    text[line_start(text, i)..i].trim().is_empty()
}

/// `value` pretty-printed with `unit`, every line after the first prefixed by `base`.
fn render<T: Serialize + ?Sized>(value: &T, unit: &str, base: &str) -> String {
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(
        &mut buf,
        serde_json::ser::PrettyFormatter::with_indent(unit.as_bytes()),
    );
    value.serialize(&mut ser).expect("JSON value serializes");
    let s = String::from_utf8(buf).expect("serde_json writes UTF-8");
    let mut out = String::with_capacity(s.len() + 64);
    for (n, line) in s.lines().enumerate() {
        if n > 0 {
            out.push('\n');
            out.push_str(base);
        }
        out.push_str(line);
    }
    out
}

/// An edit that appends `items` (rendered at the container's indentation)
/// to a container whose children are at `children`, open bracket at `open`
/// and close bracket at `close`. `key_for(i)` names the member key for an
/// object container, `None` for an array.
fn append_to_container<T: Serialize>(
    text: &str,
    unit: &str,
    open: usize,
    close: usize,
    children: &[Range<usize>],
    items: &[T],
    key_for: impl Fn(usize) -> Option<String>,
) -> Option<Edit> {
    if items.is_empty() {
        return None;
    }
    let as_text = |i: usize, base: &str| {
        let body = render(&items[i], unit, base);
        match key_for(i) {
            Some(k) => format!("{}: {body}", serde_json::to_string(&k).unwrap_or_default()),
            None => body,
        }
    };
    match children.last() {
        None => {
            let outer = line_indent(text, open).to_string();
            let inner = format!("{outer}{unit}");
            let body: Vec<String> = (0..items.len())
                .map(|i| format!("{inner}{}", as_text(i, &inner)))
                .collect();
            Some(Edit {
                range: open..close + 1,
                replacement: format!(
                    "{}\n{}\n{outer}{}",
                    &text[open..open + 1],
                    body.join(",\n"),
                    &text[close..close + 1]
                ),
            })
        }
        Some(last) => {
            if starts_line(text, last.start) {
                let indent = line_indent(text, last.start).to_string();
                let body: Vec<String> = (0..items.len())
                    .map(|i| format!("{indent}{}", as_text(i, &indent)))
                    .collect();
                Some(Edit {
                    range: last.end..last.end,
                    replacement: format!(",\n{}", body.join(",\n")),
                })
            } else {
                let body: Vec<String> = (0..items.len()).map(|i| as_text(i, "")).collect();
                Some(Edit {
                    range: last.end..last.end,
                    replacement: format!(", {}", body.join(", ")),
                })
            }
        }
    }
}

/// Ranges that remove the children at `indices` (ascending) with the layout
/// kept: a run followed by a kept child takes the separator after it, a run
/// at the end takes the separator before it. The caller never asks for all
/// children (it removes the container instead).
fn removal_ranges(children: &[Range<usize>], indices: &[usize]) -> Vec<Range<usize>> {
    let n = children.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < indices.len() {
        let a = indices[i];
        let mut b = a;
        while i + 1 < indices.len() && indices[i + 1] == b + 1 {
            i += 1;
            b += 1;
        }
        i += 1;
        if b + 1 < n {
            out.push(children[a].start..children[b + 1].start);
        } else if a > 0 {
            out.push(children[a - 1].end..children[b].end);
        } else {
            out.push(children[a].start..children[b].end);
        }
    }
    out
}

struct Edited {
    text: String,
    conflicts: Vec<Conflict>,
}

/// The whole editing engine. `block` is Mode's block to install (replacing
/// what is there), or `None` to uninstall.
fn edit_text(
    path: &Path,
    text: &str,
    command: &str,
    block: Option<&[(&'static str, Node)]>,
) -> Result<Edited, InstallError> {
    let trimmed = text.trim();
    let text: std::borrow::Cow<str> = if trimmed.is_empty() {
        "{}\n".into()
    } else {
        text.into()
    };
    let parsed: Value = serde_json::from_str(&text).map_err(|e| InstallError::Unparseable {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })?;
    if !parsed.is_object() {
        return Err(InstallError::NotAnObject {
            path: path.to_path_buf(),
        });
    }
    let unit = detect_indent_unit(&text);
    let src = text.as_bytes();
    let root = scan::first_value(src)
        .and_then(|i| scan::scan_object(src, i))
        .filter(scan::Object::is_complete)
        .ok_or_else(|| InstallError::NotAnObject {
            path: path.to_path_buf(),
        })?;

    let mut edits = Vec::new();
    let mut conflicts = Vec::new();
    let is_mode_event = |name: &str| event_spec(name).is_some();

    let Some(hooks_member) = root.member("hooks") else {
        // No hooks at all: install appends the member; uninstall has nothing to do.
        if let Some(block) = block {
            let value = Node::Object(block.to_vec());
            let children: Vec<Range<usize>> = root.members.iter().map(scan::Member::span).collect();
            edits.extend(append_to_container(
                &text,
                &unit,
                root.open,
                root.close.expect("complete"),
                &children,
                &[value],
                |_| Some("hooks".to_string()),
            ));
        }
        return Ok(Edited {
            text: apply_edits(&text, edits),
            conflicts,
        });
    };

    let hooks = scan::scan_object(src, hooks_member.value.start)
        .filter(scan::Object::is_complete)
        .ok_or_else(|| InstallError::HooksNotAnObject {
            path: path.to_path_buf(),
        })?;

    let mut removed_members: Vec<usize> = Vec::new();
    let mut placed: Vec<&str> = Vec::new();

    for (mi, member) in hooks.members.iter().enumerate() {
        let event = member.key.as_str();
        let wanted: Option<&Node> = block.and_then(|b| {
            b.iter()
                .find(|(name, _)| *name == event)
                .map(|(_, node)| first_group(node))
        });
        let Some(array) =
            scan::scan_array(src, member.value.start).filter(scan::Array::is_complete)
        else {
            if is_mode_event(event) {
                return Err(InstallError::EventNotAnArray {
                    path: path.to_path_buf(),
                    event: event.to_string(),
                });
            }
            continue;
        };
        let mut group_edits: Vec<Edit> = Vec::new();
        let mut removed_groups: Vec<usize> = Vec::new();
        let mut replaced = false;
        for (gi, span) in array.elements.iter().enumerate() {
            let group: Value = match serde_json::from_slice(&src[span.clone()]) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let matcher = group
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string);
            let Some(inner) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            let owners: Vec<Owner> = inner.iter().map(|h| classify(h, command)).collect();
            for (h, owner) in inner.iter().zip(&owners) {
                if *owner == Owner::Foreign && is_mode_event(event) {
                    conflicts.push(Conflict {
                        event: event.to_string(),
                        command: hook_command(h),
                        matcher: matcher.clone(),
                    });
                }
            }
            let mode_count = owners.iter().filter(|o| **o != Owner::Foreign).count();
            if mode_count == 0 {
                continue;
            }
            if mode_count == owners.len() {
                // A group of Mode's own: replace the first with the current entry, drop the rest.
                if let (Some(group), false) = (wanted, replaced) {
                    let base = if starts_line(&text, span.start) {
                        line_indent(&text, span.start).to_string()
                    } else {
                        String::new()
                    };
                    group_edits.push(Edit {
                        range: span.clone(),
                        replacement: render(group, &unit, &base),
                    });
                    replaced = true;
                } else {
                    removed_groups.push(gi);
                }
            } else {
                // Mixed group (user-edited): take only Mode's hooks out of it.
                let inner_scan = scan::scan_object(src, span.start)
                    .and_then(|g| g.member("hooks").map(|m| m.value.start))
                    .and_then(|i| scan::scan_array(src, i))
                    .filter(scan::Array::is_complete);
                if let Some(inner_scan) = inner_scan {
                    let ours: Vec<usize> = owners
                        .iter()
                        .enumerate()
                        .filter(|(_, o)| **o != Owner::Foreign)
                        .map(|(i, _)| i)
                        .collect();
                    group_edits.extend(
                        removal_ranges(&inner_scan.elements, &ours)
                            .into_iter()
                            .map(|range| Edit {
                                range,
                                replacement: String::new(),
                            }),
                    );
                }
            }
        }
        if removed_groups.len() == array.elements.len() && !replaced {
            // Nothing of this event would be left: drop the member instead.
            removed_members.push(mi);
            continue;
        }
        group_edits.extend(
            removal_ranges(&array.elements, &removed_groups)
                .into_iter()
                .map(|range| Edit {
                    range,
                    replacement: String::new(),
                }),
        );
        if let (Some(group), false) = (wanted, replaced) {
            // Mode's event exists with foreign groups only: append Mode's group.
            group_edits.extend(append_to_container(
                &text,
                &unit,
                array.open,
                array.close.expect("complete"),
                &array.elements,
                std::slice::from_ref(group),
                |_| None,
            ));
        }
        if wanted.is_some() {
            placed.push(event);
        }
        edits.extend(group_edits);
    }

    let mut remaining_members = hooks.members.len() - removed_members.len();

    // Events of Mode's that the file does not have yet.
    if let Some(block) = block {
        let missing: Vec<(&str, &Node)> = EVENTS
            .iter()
            .filter(|e| !placed.contains(&e.name) && hooks.member(e.name).is_none())
            .filter_map(|e| {
                block
                    .iter()
                    .find(|(name, _)| *name == e.name)
                    .map(|(name, node)| (*name, node))
            })
            .collect();
        if !missing.is_empty() {
            let children: Vec<Range<usize>> =
                hooks.members.iter().map(scan::Member::span).collect();
            let items: Vec<&Node> = missing.iter().map(|(_, n)| *n).collect();
            edits.extend(append_to_container(
                &text,
                &unit,
                hooks.open,
                hooks.close.expect("complete"),
                &children,
                &items,
                |i| missing.get(i).map(|(name, _)| name.to_string()),
            ));
            remaining_members += items.len();
        }
    }

    if remaining_members == 0 {
        // The hooks object would be empty: drop the member entirely (and, when
        // nothing else is in the file, leave `{}`).
        let spans: Vec<Range<usize>> = root.members.iter().map(scan::Member::span).collect();
        let hi = root
            .members
            .iter()
            .position(|m| m.key == "hooks")
            .expect("present");
        edits.retain(|e| {
            e.range.start < hooks_member.start || e.range.start > hooks_member.value.end
        });
        if root.members.len() == 1 {
            let close = root.close.expect("complete");
            edits.push(Edit {
                range: root.open..close + 1,
                replacement: "{}".to_string(),
            });
        } else {
            edits.extend(removal_ranges(&spans, &[hi]).into_iter().map(|range| Edit {
                range,
                replacement: String::new(),
            }));
        }
    } else {
        let spans: Vec<Range<usize>> = hooks.members.iter().map(scan::Member::span).collect();
        edits.extend(
            removal_ranges(&spans, &removed_members)
                .into_iter()
                .map(|range| Edit {
                    range,
                    replacement: String::new(),
                }),
        );
    }

    Ok(Edited {
        text: apply_edits(&text, edits),
        conflicts,
    })
}

// --------------------------------------------------------------------- plan

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Install,
    Uninstall,
}

/// What the preview dialog shows and what `apply` writes (05 §3.6 S1–S5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub target: Target,
    pub action: Action,
    pub file_existed: bool,
    pub before: String,
    pub after: String,
    /// Unified diff of `before` → `after`.
    pub diff: String,
    pub changed: bool,
    pub conflicts: Vec<Conflict>,
    pub events: Vec<EventLine>,
    /// Hash of Mode's entries after the write (empty for uninstall).
    pub block_hash: String,
}

fn read_settings(path: &Path) -> Result<(String, bool), InstallError> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok((s, true)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((String::new(), false)),
        Err(e) => Err(io_err(path, e)),
    }
}

/// A unified diff for the preview (03 §7C.3), three lines of context.
pub fn diff(before: &str, after: &str, label: &str) -> String {
    similar::TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header(&format!("{label} (before)"), &format!("{label} (after)"))
        .to_string()
}

fn plan(target: &Target, action: Action, timeouts: &Timeouts) -> Result<Plan, InstallError> {
    let (before, file_existed) = read_settings(&target.settings_path)?;
    let block = match action {
        Action::Install => Some(block_nodes(&target.command, timeouts)),
        Action::Uninstall => None,
    };
    let edited = edit_text(
        &target.settings_path,
        &before,
        &target.command,
        block.as_deref(),
    )?;
    let mut after = edited.text;
    if !file_existed && !after.ends_with('\n') {
        after.push('\n');
    }
    let changed = after != before;
    let label = target.settings_path.display().to_string();
    Ok(Plan {
        target: target.clone(),
        action,
        file_existed,
        diff: diff(&before, &after, &label),
        changed,
        conflicts: edited.conflicts,
        events: match action {
            Action::Install => event_lines(timeouts),
            Action::Uninstall => Vec::new(),
        },
        block_hash: match action {
            Action::Install => block_hash(&mode_entries(&target.command, timeouts)),
            Action::Uninstall => String::new(),
        },
        before,
        after,
    })
}

/// Install or update: Mode's entries replace whatever Mode entries the file
/// has; foreign hooks stay and are listed as conflicts.
pub fn plan_install(target: &Target, timeouts: &Timeouts) -> Result<Plan, InstallError> {
    plan(target, Action::Install, timeouts)
}

/// Remove exactly Mode's entries (05 §3.6 "Uninstall").
pub fn plan_uninstall(target: &Target) -> Result<Plan, InstallError> {
    plan(target, Action::Uninstall, &Timeouts::default())
}

// -------------------------------------------------------------------- apply

/// What `apply` did (`hook_installs.backup_path`, `block_hash`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Applied {
    pub settings_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub block_hash: String,
    pub bytes_written: usize,
    pub written: bool,
}

fn timestamp() -> String {
    mode_wire::time::now_rfc3339()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect()
}

/// Copy the settings file to `settings.json.mode-backup-<UTC timestamp>` in
/// `backup_dir` (default: next to the file), keeping the newest
/// [`BACKUPS_KEPT`]. `None` when there is no file to back up.
pub fn backup(settings_path: &Path, backup_dir: Option<&Path>) -> io::Result<Option<PathBuf>> {
    if !settings_path.exists() {
        return Ok(None);
    }
    let dir = match backup_dir {
        Some(d) => d.to_path_buf(),
        None => settings_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    std::fs::create_dir_all(&dir)?;
    let file_name = settings_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "settings.json".into());
    let prefix = format!("{file_name}.mode-backup-");
    let dest = dir.join(format!("{prefix}{}", timestamp()));
    std::fs::copy(settings_path, &dest)?;
    // Prune: names sort by timestamp.
    let mut old: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
        })
        .collect();
    old.sort();
    while old.len() > BACKUPS_KEPT {
        let victim = old.remove(0);
        let _ = std::fs::remove_file(victim);
    }
    Ok(Some(dest))
}

/// Temp file in the same directory + rename, permissions of the original kept.
fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(dir) = dir {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.mode-tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    let result = (|| {
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(contents.as_bytes())?;
            f.sync_all()?;
        }
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Mode's entries as found in a file: event → the groups that are Mode's.
/// Mode hooks inside a mixed group are reported as their own group.
fn found_entries(root: &Value, command: &str) -> (Map<String, Value>, Vec<Conflict>, Vec<String>) {
    let mut found = Map::new();
    let mut conflicts = Vec::new();
    let mut commands = Vec::new();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return (found, conflicts, commands);
    };
    for (event, groups) in hooks {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        let mut mine = Vec::new();
        for group in groups {
            let Some(inner) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            let matcher = group.get("matcher").cloned();
            let owners: Vec<Owner> = inner.iter().map(|h| classify(h, command)).collect();
            for (h, o) in inner.iter().zip(&owners) {
                match o {
                    Owner::Foreign => {
                        if event_spec(event).is_some() {
                            conflicts.push(Conflict {
                                event: event.clone(),
                                command: hook_command(h),
                                matcher: matcher
                                    .as_ref()
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                            });
                        }
                    }
                    _ => commands.push(hook_command(h)),
                }
            }
            let all_mine = owners.iter().all(|o| *o != Owner::Foreign);
            if all_mine {
                mine.push(group.clone());
            } else {
                let ours: Vec<Value> = inner
                    .iter()
                    .zip(&owners)
                    .filter(|(_, o)| **o != Owner::Foreign)
                    .map(|(h, _)| h.clone())
                    .collect();
                if !ours.is_empty() {
                    let mut g = Map::new();
                    if let Some(m) = &matcher {
                        g.insert("matcher".into(), m.clone());
                    }
                    g.insert("hooks".into(), Value::Array(ours));
                    mine.push(Value::Object(g));
                }
            }
        }
        if !mine.is_empty() {
            found.insert(event.clone(), Value::Array(mine));
        }
    }
    (found, conflicts, commands)
}

/// S7–S9 of 05 §3.6: backup, atomic write, re-read and verify. A plan with
/// nothing to change writes nothing.
pub fn apply(plan: &Plan, backup_dir: Option<&Path>) -> Result<Applied, InstallError> {
    let path = &plan.target.settings_path;
    if !plan.changed {
        return Ok(Applied {
            settings_path: path.clone(),
            backup_path: None,
            block_hash: plan.block_hash.clone(),
            bytes_written: 0,
            written: false,
        });
    }
    let backup_path = backup(path, backup_dir).map_err(|e| io_err(path, e))?;
    write_atomically(path, &plan.after).map_err(|e| io_err(path, e))?;

    // Re-read and verify by hash; restore the backup when the result is not what was planned.
    let verify = (|| -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        if text != plan.after {
            return Err("file content differs from the plan".into());
        }
        let root: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let (found, _, _) = found_entries(&root, &plan.target.command);
        match plan.action {
            Action::Install if block_hash(&found) != plan.block_hash => {
                Err("block hash differs after the write".into())
            }
            Action::Uninstall if !found.is_empty() => Err("Mode entries still present".into()),
            _ => Ok(()),
        }
    })();
    if let Err(detail) = verify {
        let restored = match &backup_path {
            Some(b) if std::fs::copy(b, path).is_ok() => Some(b.clone()),
            _ => None,
        };
        if backup_path.is_none() {
            let _ = std::fs::remove_file(path);
        }
        return Err(InstallError::Verify { detail, restored });
    }
    Ok(Applied {
        settings_path: path.clone(),
        backup_path,
        block_hash: plan.block_hash.clone(),
        bytes_written: plan.after.len(),
        written: true,
    })
}

/// Plan + apply for the common cases.
pub fn install(
    target: &Target,
    timeouts: &Timeouts,
    backup_dir: Option<&Path>,
) -> Result<Applied, InstallError> {
    apply(&plan_install(target, timeouts)?, backup_dir)
}

/// "Update hooks": the same as install (Mode's entries are replaced with the current ones).
pub fn update(
    target: &Target,
    timeouts: &Timeouts,
    backup_dir: Option<&Path>,
) -> Result<Applied, InstallError> {
    install(target, timeouts, backup_dir)
}

pub fn uninstall(target: &Target, backup_dir: Option<&Path>) -> Result<Applied, InstallError> {
    apply(&plan_uninstall(target)?, backup_dir)
}

// ------------------------------------------------------------------- status

/// `hook_installs.status` (09 §5.5) plus "not installed" for no row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStatus {
    Installed,
    Stale,
    Missing,
    Conflict,
    Removed,
    NotInstalled,
}

/// What the Claude Code card shows (03 §7C.2).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    pub installed: bool,
    pub status: InstallStatus,
    /// The relay's version as the file's command reports it (`mode-hook --version`).
    pub version: Option<String>,
    pub path: PathBuf,
    /// The relay command found in the file, when any.
    pub relay_path: Option<PathBuf>,
    pub conflicts: Vec<Conflict>,
    /// Events that carry a Mode entry.
    pub events: Vec<String>,
    pub block_hash: Option<String>,
    pub detail: Option<String>,
    pub last_round_trip: Option<RoundTripReport>,
}

/// Re-verification (05 §3.6): re-read the file, recompute the block hash and
/// classify. Pure apart from `mode-hook --version` on a relay that is not the
/// expected one (to name the stale version).
pub fn inspect(
    target: &Target,
    timeouts: &Timeouts,
    previously_installed: bool,
) -> Result<Status, InstallError> {
    let path = target.settings_path.clone();
    let absent = |status: InstallStatus, detail: &str| Status {
        installed: false,
        status,
        version: None,
        path: path.clone(),
        relay_path: None,
        conflicts: Vec::new(),
        events: Vec::new(),
        block_hash: None,
        detail: Some(detail.to_string()),
        last_round_trip: None,
    };
    let (text, exists) = read_settings(&target.settings_path)?;
    if !exists {
        let status = if previously_installed {
            InstallStatus::Missing
        } else {
            InstallStatus::NotInstalled
        };
        return Ok(absent(status, "settings file does not exist"));
    }
    let root: Value = serde_json::from_str(&text).map_err(|e| InstallError::Unparseable {
        path: path.clone(),
        detail: e.to_string(),
    })?;
    let (found, conflicts, commands) = found_entries(&root, &target.command);
    let expected = block_hash(&mode_entries(&target.command, timeouts));
    let found_hash = block_hash(&found);
    let mut events: Vec<String> = found.keys().cloned().collect();
    events.sort();
    let relay_path = commands.first().map(PathBuf::from);
    let other_path = commands.iter().any(|c| c != &target.command);

    let (status, detail) = if found.is_empty() {
        if !conflicts.is_empty() {
            (
                InstallStatus::Conflict,
                "another hook is registered".to_string(),
            )
        } else if previously_installed {
            (InstallStatus::Missing, "hooks were removed".to_string())
        } else {
            (
                InstallStatus::NotInstalled,
                "no Mode hooks in the file".to_string(),
            )
        }
    } else if found_hash == expected {
        (InstallStatus::Installed, "hooks installed".to_string())
    } else if other_path {
        (InstallStatus::Stale, "relay path differs".to_string())
    } else {
        (
            InstallStatus::Stale,
            "events or timeouts differ".to_string(),
        )
    };
    let version = match status {
        InstallStatus::Installed => Some(RELAY_VERSION.to_string()),
        InstallStatus::Stale => relay_path.as_deref().and_then(relay_version),
        _ => None,
    };
    Ok(Status {
        installed: status == InstallStatus::Installed,
        status,
        version,
        path,
        relay_path,
        conflicts,
        events,
        block_hash: (!found.is_empty()).then_some(found_hash),
        detail: Some(detail),
        last_round_trip: None,
    })
}

/// `mode-hook --version` → `0.1.0`.
pub fn relay_version(relay: &Path) -> Option<String> {
    let out = Command::new(relay)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim().strip_prefix("mode-hook ").map(str::to_string)
}

// --------------------------------------------------------------- round trip

/// The result of the synthetic round trip (05 §3.6 S10: "round trip 9 ms").
/// Deserializable so the app can keep the last one across restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundTripReport {
    pub session_id: String,
    pub latency_ms: u64,
    /// The relay confirmed the core's ack (`MODE_HOOK_DEBUG` line).
    pub acked: bool,
    pub relay_version: Option<String>,
    /// RFC 3339.
    pub at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RoundTripError {
    #[error("could not run {relay}: {source}")]
    Spawn {
        relay: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("no frame reached the core within {timeout_ms} ms (relay said: {stderr:?})")]
    NoFrame { timeout_ms: u64, stderr: String },
}

/// Spawn `relay SessionStart` with a synthetic payload against `server`
/// and wait for the frame. Proves exec form, connect and parse end to end.
pub async fn round_trip(
    relay: &Path,
    server: &Server,
    timeout: Duration,
) -> Result<RoundTripReport, RoundTripError> {
    let session_id = format!("{ROUND_TRIP_SESSION_PREFIX}{}", mode_wire::id::uuid_v7());
    let mut payload = HookEvent::new(events::SESSION_START, session_id.clone());
    payload.cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    payload.source = Some("mode-round-trip".into());
    let body = serde_json::to_vec(&payload).unwrap_or_default();

    let endpoint: &Endpoint = server.endpoint();
    let started = Instant::now();
    let mut child = Command::new(relay)
        .arg(events::SESSION_START)
        .arg("5")
        .env(mode_wire::endpoint::ENV_OVERRIDE, endpoint.describe())
        .env("MODE_HOOK_DEBUG", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| RoundTripError::Spawn {
            relay: relay.to_path_buf(),
            source,
        })?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(&body);
    }
    let seen = server.await_round_trip(&session_id, timeout).await;
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .ok()
        .and_then(Result::ok);
    let stderr = output
        .as_ref()
        .map(|o| String::from_utf8_lossy(&o.stderr).trim().to_string())
        .unwrap_or_default();
    let Some(arrived) = seen else {
        return Err(RoundTripError::NoFrame {
            timeout_ms: timeout.as_millis() as u64,
            stderr,
        });
    };
    Ok(RoundTripReport {
        session_id,
        latency_ms: arrived.saturating_duration_since(started).as_millis() as u64,
        acked: stderr.lines().any(|l| l.trim() == "mode-hook: acked"),
        relay_version: relay_version(relay),
        at: mode_wire::time::now_rfc3339(),
    })
}

// ---------------------------------------------------------------------- WSL

/// Path mapping for WSL targets (05 §3.2, §3.6). Pure functions; discovery
/// (`wsl.exe --list --quiet`) is Windows-only.
pub mod wsl {
    use super::*;

    /// `C:\Users\ana\AppData\Roaming\Mode\bin\mode-hook.exe` →
    /// `/mnt/c/Users/ana/AppData/Roaming/Mode/bin/mode-hook.exe`.
    pub fn relay_command(windows_path: &str) -> Option<String> {
        let mut chars = windows_path.chars();
        let drive = chars.next()?.to_ascii_lowercase();
        if !drive.is_ascii_alphabetic() || chars.next() != Some(':') {
            return None;
        }
        let rest: String = chars.collect();
        let rest = rest.replace('\\', "/");
        let rest = rest.trim_start_matches('/');
        Some(format!("/mnt/{drive}/{rest}"))
    }

    /// `\\wsl$\<distro>\<home>\.claude\settings.json` for a Linux `$HOME`.
    pub fn settings_path_for_home(distro: &str, home: &str) -> PathBuf {
        let home = home.trim_end_matches('/').replace('/', "\\");
        PathBuf::from(format!(r"\\wsl$\{distro}{home}\.claude\settings.json"))
    }

    /// `\\wsl$\<distro>\home\<user>\.claude\settings.json`.
    pub fn settings_path(distro: &str, user: &str) -> PathBuf {
        settings_path_for_home(distro, &format!("/home/{user}"))
    }

    /// The target for one distribution, or `None` when the relay path has no drive letter.
    pub fn target(distro: &str, home: &str, relay_path: &Path) -> Option<Target> {
        let command = relay_command(&relay_path.to_string_lossy())?;
        Some(Target {
            settings_path: settings_path_for_home(distro, home),
            relay_path: relay_path.to_path_buf(),
            command,
            kind: TargetKind::Wsl {
                distro: distro.to_string(),
            },
        })
    }

    /// Parse `wsl.exe --list --quiet` output (UTF-16LE on Windows).
    pub fn parse_distro_list(bytes: &[u8]) -> Vec<String> {
        let utf16 = bytes.starts_with(&[0xFF, 0xFE])
            || (bytes.len() >= 4 && bytes[1] == 0 && bytes[3] == 0);
        let text = if utf16 {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        };
        text.lines()
            .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0'))
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Every distribution with a `~/.claude` directory, as install targets.
    #[cfg(windows)]
    pub fn discover(relay_path: &Path) -> Vec<Target> {
        let Ok(out) = Command::new("wsl.exe")
            .args(["--list", "--quiet"])
            .stdin(Stdio::null())
            .output()
        else {
            return Vec::new();
        };
        parse_distro_list(&out.stdout)
            .into_iter()
            .filter_map(|distro| {
                let home = Command::new("wsl.exe")
                    .args(["-d", &distro, "--", "sh", "-c", "printf %s \"$HOME\""])
                    .stdin(Stdio::null())
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|h| h.starts_with('/'))?;
                let t = target(&distro, &home, relay_path)?;
                t.settings_path
                    .parent()
                    .is_some_and(Path::exists)
                    .then_some(t)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn target_in(dir: &tempfile::TempDir) -> Target {
        let settings = dir
            .path()
            .join("home")
            .join(".claude")
            .join("settings.json");
        let relay = dir.path().join("Mode").join("bin").join(RELAY_FILE_NAME);
        Target::native(settings, relay)
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn round_trip_reports_survive_a_json_round_trip() {
        let r = RoundTripReport {
            session_id: format!("{ROUND_TRIP_SESSION_PREFIX}1"),
            latency_ms: 9,
            acked: true,
            relay_version: Some(RELAY_VERSION.into()),
            at: "2026-10-22T10:00:00.000Z".into(),
        };
        let v = serde_json::to_value(&r).unwrap();
        let back: RoundTripReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, r);
    }

    const SAMPLE: &str = r#"{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "permissions": {
    "allow": ["Bash(npm test)", "Read"],
    "deny": []
  },
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          { "type": "command", "command": "/usr/local/bin/lint-check", "timeout": 10 }
        ]
      }
    ],
    "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }]
  },
  "env": { "FOO": "bar" },
  "model": "opus"
}
"#;

    #[test]
    fn entries_follow_the_event_table() {
        let t = Timeouts::default();
        let entries = mode_entries("/opt/Mode/bin/mode-hook", &t);
        assert_eq!(entries.len(), EVENTS.len());
        assert_eq!(EVENTS.len(), 18);
        let pr = &entries["PermissionRequest"][0];
        assert_eq!(pr["matcher"], "");
        let hook = &pr["hooks"][0];
        assert_eq!(hook["type"], "command");
        assert_eq!(hook["command"], "/opt/Mode/bin/mode-hook");
        assert_eq!(
            hook["args"],
            serde_json::json!(["PermissionRequest", "120"])
        );
        assert_eq!(hook["timeout"], 120);
        assert!(
            hook.get("async").is_none(),
            "decision events are synchronous"
        );
        let ss = &entries["SessionStart"][0]["hooks"][0];
        assert_eq!(ss["timeout"], 5);
        assert_eq!(ss["async"], true);
        assert_eq!(ss["args"], serde_json::json!(["SessionStart", "5"]));
        assert!(
            entries["SessionEnd"][0].get("matcher").is_none(),
            "no matcher where the table has none"
        );
        assert_eq!(entries["PostToolUse"][0]["matcher"], "");

        let lines = event_lines(&t);
        assert_eq!(
            lines[0],
            EventLine {
                event: "PermissionRequest".into(),
                timeout: 120,
                is_async: false,
                matcher: Some(String::new())
            }
        );
        assert_eq!(
            lines[3],
            EventLine {
                event: "SessionEnd".into(),
                timeout: 5,
                is_async: true,
                matcher: None
            }
        );
        assert_eq!(serde_json::to_value(&lines[3]).unwrap()["async"], true);

        // The hash ignores key order and whitespace, and changes with the content.
        let custom = Timeouts {
            decision_secs: 90,
            async_secs: 5,
        };
        assert_ne!(
            block_hash(&entries),
            block_hash(&mode_entries("/opt/Mode/bin/mode-hook", &custom))
        );
        let reparsed: Map<String, Value> =
            serde_json::from_str(&serde_json::to_string(&entries).unwrap()).unwrap();
        assert_eq!(block_hash(&entries), block_hash(&reparsed));

        let json = copy_json("/opt/Mode/bin/mode-hook", &t);
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v["hooks"]["Stop"][0]["hooks"][0]["args"],
            serde_json::json!(["Stop", "5"])
        );
        assert!(
            json.starts_with("{\n  \"hooks\": {\n    \"PermissionRequest\""),
            "{json}"
        );
    }

    #[test]
    fn install_into_a_missing_file_then_uninstall_leaves_an_empty_object() {
        let dir = tmp();
        let target = target_in(&dir);
        let t = Timeouts::default();
        let plan = plan_install(&target, &t).unwrap();
        assert!(!plan.file_existed);
        assert!(plan.changed);
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.events.len(), 18);
        assert!(plan.diff.contains("+  \"hooks\": {"), "{}", plan.diff);
        let v: Value = serde_json::from_str(&plan.after).unwrap();
        assert_eq!(
            v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"],
            target.command
        );
        assert!(plan.after.ends_with("}\n"));

        let applied = apply(&plan, None).unwrap();
        assert!(applied.written);
        assert!(applied.backup_path.is_none(), "nothing to back up");
        assert_eq!(
            std::fs::read_to_string(&target.settings_path).unwrap(),
            plan.after
        );

        let status = inspect(&target, &t, false).unwrap();
        assert!(status.installed);
        assert_eq!(status.status, InstallStatus::Installed);
        assert_eq!(status.version.as_deref(), Some(RELAY_VERSION));
        assert_eq!(status.events.len(), 18);
        assert_eq!(status.block_hash.as_deref(), Some(plan.block_hash.as_str()));
        assert_eq!(
            status.relay_path.as_deref(),
            Some(target.relay_path.as_path())
        );

        // Installing again changes nothing and writes nothing.
        let again = plan_install(&target, &t).unwrap();
        assert!(!again.changed, "{}", again.diff);
        assert!(!apply(&again, None).unwrap().written);

        let un = plan_uninstall(&target).unwrap();
        assert!(un.changed);
        assert_eq!(un.after, "{}\n");
        let applied = apply(&un, None).unwrap();
        assert!(applied.backup_path.is_some());
        let status = inspect(&target, &t, true).unwrap();
        assert_eq!(status.status, InstallStatus::Missing);
        assert!(!status.installed);
        let status = inspect(&target, &t, false).unwrap();
        assert_eq!(status.status, InstallStatus::NotInstalled);
    }

    #[test]
    fn install_and_uninstall_round_trip_a_sample_file_byte_for_byte() {
        let dir = tmp();
        let target = target_in(&dir);
        write(&target.settings_path, SAMPLE);
        let t = Timeouts::default();

        let plan = plan_install(&target, &t).unwrap();
        assert!(plan.changed);
        assert_eq!(plan.before, SAMPLE);
        // Foreign hooks on Mode's events are conflicts, kept as they are.
        assert_eq!(
            plan.conflicts,
            vec![
                Conflict {
                    event: "PreToolUse".into(),
                    command: "/usr/local/bin/lint-check".into(),
                    matcher: Some("Bash".into())
                },
                Conflict {
                    event: "Stop".into(),
                    command: "say done".into(),
                    matcher: None
                },
            ]
        );
        let after: Value = serde_json::from_str(&plan.after).unwrap();
        // Everything outside hooks is untouched, textually.
        for key in ["$schema", "permissions", "env", "model"] {
            let before: Value = serde_json::from_str(SAMPLE).unwrap();
            assert_eq!(after[key], before[key], "{key}");
        }
        assert!(
            plan.after
                .contains("\"allow\": [\"Bash(npm test)\", \"Read\"],"),
            "inline arrays stay inline"
        );
        assert!(plan.after.contains("{ \"type\": \"command\", \"command\": \"/usr/local/bin/lint-check\", \"timeout\": 10 }"), "foreign hook text untouched");
        // Mode's group was appended after the foreign group of PreToolUse, and Stop too.
        let pre = after["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2);
        assert_eq!(pre[0]["matcher"], "Bash");
        assert_eq!(pre[1]["hooks"][0]["command"], target.command);
        assert_eq!(after["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(after["hooks"].as_object().unwrap().len(), 18);
        // Key order of the file: hooks stays where it was.
        let keys: Vec<&str> = after
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let _ = keys; // (serde_json sorts; the textual check below is the real one)
        assert!(plan.after.find("\"hooks\"").unwrap() < plan.after.find("\"env\"").unwrap());
        assert!(
            plan.after.find("\"permissions\"").unwrap() < plan.after.find("\"hooks\"").unwrap()
        );

        let backups = dir.path().join("backups");
        let applied = apply(&plan, Some(&backups)).unwrap();
        let backup_path = applied.backup_path.clone().unwrap();
        assert!(backup_path.starts_with(&backups));
        assert!(
            backup_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("settings.json.mode-backup-")
        );
        assert_eq!(std::fs::read_to_string(&backup_path).unwrap(), SAMPLE);

        let status = inspect(&target, &t, false).unwrap();
        assert_eq!(
            status.status,
            InstallStatus::Installed,
            "{:?}",
            status.detail
        );
        assert_eq!(
            status.conflicts.len(),
            2,
            "conflicts are reported next to installed"
        );

        // Uninstall gives the original bytes back.
        let un = plan_uninstall(&target).unwrap();
        assert_eq!(un.after, SAMPLE, "{}", un.diff);
        assert!(un.diff.contains("-    \"PermissionRequest\": ["));
        apply(&un, Some(&backups)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&target.settings_path).unwrap(),
            SAMPLE
        );
        let status = inspect(&target, &t, true).unwrap();
        assert_eq!(
            status.status,
            InstallStatus::Conflict,
            "foreign hooks on Mode's events, no Mode block"
        );
        assert_eq!(status.conflicts.len(), 2);
    }

    #[test]
    fn diff_snapshot_for_a_small_file() {
        let dir = tmp();
        let target = Target::native(dir.path().join("settings.json"), "/opt/Mode/bin/mode-hook");
        write(&target.settings_path, "{\n  \"model\": \"opus\"\n}\n");
        let t = Timeouts {
            decision_secs: 120,
            async_secs: 5,
        };
        let plan = plan_install(&target, &t).unwrap();
        let body = diff(&plan.before, &plan.after, "settings.json");
        let expected_head = "--- settings.json (before)\n+++ settings.json (after)\n@@ -1,3 +1,";
        assert!(body.starts_with(expected_head), "{body}");
        let expected_lines = [
            " {",
            "-  \"model\": \"opus\"",
            "+  \"model\": \"opus\",",
            "+  \"hooks\": {",
            "+    \"PermissionRequest\": [",
            "+      {",
            "+        \"matcher\": \"\",",
            "+        \"hooks\": [",
            "+          {",
            "+            \"type\": \"command\",",
            "+            \"command\": \"/opt/Mode/bin/mode-hook\",",
            "+            \"args\": [",
            "+              \"PermissionRequest\",",
            "+              \"120\"",
            "+            ],",
            "+            \"timeout\": 120",
            "+          }",
            "+        ]",
            "+      }",
            "+    ],",
            "+    \"PreToolUse\": [",
        ];
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(
            &lines[3..3 + expected_lines.len()],
            &expected_lines,
            "{body}"
        );
        assert!(body.ends_with("+  }\n }\n"), "{body}");
        assert!(body.contains("+            \"async\": true"));
    }

    #[test]
    fn update_replaces_stale_entries_and_mixed_groups_lose_only_mode_hooks() {
        let dir = tmp();
        let target = target_in(&dir);
        let old = Timeouts {
            decision_secs: 60,
            async_secs: 3,
        };
        let new = Timeouts::default();
        // An older install at another path, plus a user-edited mixed group and a leftover event.
        let old_cmd = "/old/place/mode-hook";
        let mut root = Map::new();
        let mut hooks = mode_entries(old_cmd, &old);
        let mixed = serde_json::json!([{
            "matcher": "",
            "hooks": [
                {"type": "command", "command": "/usr/bin/other"},
                {"type": "command", "command": old_cmd, "args": ["Notification", "3"], "timeout": 3, "async": true}
            ]
        }]);
        hooks.insert("Notification".into(), mixed);
        hooks.insert("SomethingNew".into(), serde_json::json!([{"hooks": [{"type": "command", "command": old_cmd, "args": ["SomethingNew", "3"]}]}]));
        root.insert("hooks".into(), Value::Object(hooks));
        root.insert("theme".into(), Value::from("dark"));
        let text = render(&Value::Object(root), "    ", "") + "\n";
        write(&target.settings_path, &text);

        let status = inspect(&target, &new, true).unwrap();
        assert_eq!(status.status, InstallStatus::Stale);
        assert_eq!(status.detail.as_deref(), Some("relay path differs"));
        assert_eq!(status.relay_path.as_deref(), Some(Path::new(old_cmd)));
        assert_eq!(
            status.version, None,
            "a relay that cannot be run has no version"
        );

        let plan = plan_install(&target, &new).unwrap();
        assert_eq!(
            plan.conflicts,
            vec![Conflict {
                event: "Notification".into(),
                command: "/usr/bin/other".into(),
                matcher: Some(String::new())
            }]
        );
        let after: Value = serde_json::from_str(&plan.after).unwrap();
        let hooks = after["hooks"].as_object().unwrap();
        assert!(
            hooks.get("SomethingNew").is_none(),
            "a leftover Mode event is removed"
        );
        assert_eq!(hooks.len(), 18);
        for (event, groups) in hooks {
            let groups = groups.as_array().unwrap();
            let mode_groups: Vec<&Value> = groups
                .iter()
                .filter(|g| g["hooks"][0]["command"] == target.command)
                .collect();
            assert_eq!(mode_groups.len(), 1, "{event}: exactly one Mode group");
            assert_eq!(
                mode_groups[0]["hooks"][0]["args"][1],
                new.for_kind(event_spec(event).unwrap().kind).to_string()
            );
            assert!(
                !groups.iter().any(|g| g["hooks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|h| h["command"] == old_cmd)),
                "{event}: old relay gone"
            );
        }
        let notification = hooks["Notification"].as_array().unwrap();
        assert_eq!(
            notification[0]["hooks"].as_array().unwrap().len(),
            1,
            "the foreign hook alone stays in the mixed group"
        );
        assert_eq!(notification[0]["hooks"][0]["command"], "/usr/bin/other");
        assert_eq!(after["theme"], "dark");
        assert!(
            plan.after.contains("\n    \"theme\": \"dark\""),
            "four-space indent kept: {}",
            plan.after
        );

        apply(&plan, None).unwrap();
        let status = inspect(&target, &new, true).unwrap();
        assert_eq!(
            status.status,
            InstallStatus::Installed,
            "{:?}",
            status.detail
        );

        // A timeout change alone is stale too ("Update hooks").
        let status = inspect(
            &target,
            &Timeouts {
                decision_secs: 90,
                async_secs: 5,
            },
            true,
        )
        .unwrap();
        assert_eq!(status.status, InstallStatus::Stale);
        assert_eq!(status.detail.as_deref(), Some("events or timeouts differ"));
    }

    #[test]
    fn refuses_files_it_cannot_parse_or_that_have_the_wrong_shape() {
        let dir = tmp();
        let target = target_in(&dir);
        write(
            &target.settings_path,
            "{\n  // a comment\n  \"model\": \"opus\",\n}\n",
        );
        let err = plan_install(&target, &Timeouts::default()).unwrap_err();
        assert!(matches!(err, InstallError::Unparseable { .. }), "{err}");
        assert!(
            err.to_string().contains("comments and trailing commas"),
            "{err}"
        );

        write(&target.settings_path, "[1, 2]\n");
        assert!(matches!(
            plan_install(&target, &Timeouts::default()).unwrap_err(),
            InstallError::NotAnObject { .. }
        ));

        write(&target.settings_path, "{\"hooks\": []}\n");
        assert!(matches!(
            plan_install(&target, &Timeouts::default()).unwrap_err(),
            InstallError::HooksNotAnObject { .. }
        ));

        write(&target.settings_path, "{\"hooks\": {\"Stop\": {}}}\n");
        assert!(
            matches!(plan_install(&target, &Timeouts::default()).unwrap_err(), InstallError::EventNotAnArray { event, .. } if event == "Stop")
        );

        // Empty and whitespace-only files count as `{}`.
        write(&target.settings_path, "  \n");
        let plan = plan_install(&target, &Timeouts::default()).unwrap();
        assert!(plan.file_existed && plan.changed);
        serde_json::from_str::<Value>(&plan.after).unwrap();
    }

    #[test]
    fn backups_are_pruned_to_the_newest_five() {
        let dir = tmp();
        let settings = dir.path().join("settings.json");
        write(&settings, "{}\n");
        let mut made = Vec::new();
        for i in 0..7 {
            // Distinct names: the timestamp has millisecond resolution.
            std::thread::sleep(Duration::from_millis(2));
            let p = backup(&settings, None).unwrap().unwrap();
            assert!(
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("settings.json.mode-backup-"),
                "{i}"
            );
            made.push(p);
        }
        let kept: Vec<PathBuf> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().contains("mode-backup-"))
            .collect();
        assert_eq!(kept.len(), BACKUPS_KEPT);
        assert!(!made[0].exists() && !made[1].exists());
        assert!(made[6].exists());
        assert_eq!(backup(&dir.path().join("nope.json"), None).unwrap(), None);
    }

    #[test]
    fn verification_failure_restores_the_backup() {
        let dir = tmp();
        let target = target_in(&dir);
        write(&target.settings_path, "{\n  \"model\": \"opus\"\n}\n");
        let mut plan = plan_install(&target, &Timeouts::default()).unwrap();
        // Tamper with the plan so the written block does not hash as planned.
        plan.block_hash = "0000".into();
        let err = apply(&plan, None).unwrap_err();
        match err {
            InstallError::Verify {
                restored: Some(_), ..
            } => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(&target.settings_path).unwrap(),
            "{\n  \"model\": \"opus\"\n}\n"
        );
        assert!(
            !std::fs::read_dir(target.settings_path.parent().unwrap())
                .unwrap()
                .any(|e| e
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("mode-tmp")),
            "no temp file left"
        );
    }

    #[test]
    fn default_target_and_wsl_mapping() {
        let dir = tmp();
        assert!(
            matches!(
                default_target(dir.path(), true),
                Err(InstallError::DevBuildNeedsScratchHome)
            ) || std::env::var_os(SETTINGS_ENV_OVERRIDE).is_some()
                || std::env::var_os(CLAUDE_CONFIG_DIR).is_some()
        );
        assert_eq!(
            default_relay_path(dir.path()),
            dir.path().join("bin").join(RELAY_FILE_NAME)
        );

        assert_eq!(
            wsl::relay_command(r"C:\Users\ana\AppData\Roaming\Mode\bin\mode-hook.exe").as_deref(),
            Some("/mnt/c/Users/ana/AppData/Roaming/Mode/bin/mode-hook.exe")
        );
        assert_eq!(wsl::relay_command("/not/windows").as_deref(), None);
        assert_eq!(
            wsl::settings_path("Ubuntu", "ana"),
            PathBuf::from(r"\\wsl$\Ubuntu\home\ana\.claude\settings.json")
        );
        let t = wsl::target(
            "Ubuntu-22.04",
            "/home/ana",
            Path::new(r"D:\Mode\bin\mode-hook.exe"),
        )
        .unwrap();
        assert_eq!(t.command, "/mnt/d/Mode/bin/mode-hook.exe");
        assert_eq!(
            t.kind,
            TargetKind::Wsl {
                distro: "Ubuntu-22.04".into()
            }
        );
        assert_eq!(
            t.settings_path,
            PathBuf::from(r"\\wsl$\Ubuntu-22.04\home\ana\.claude\settings.json")
        );
        let utf16: Vec<u8> = "\u{feff}Ubuntu\r\nDebian\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(wsl::parse_distro_list(&utf16), vec!["Ubuntu", "Debian"]);
        assert_eq!(
            wsl::parse_distro_list(b"Ubuntu\n\nkali\n"),
            vec!["Ubuntu", "kali"]
        );

        // The WSL block is what the Windows side would write, with the /mnt command.
        let entries = mode_entries(&t.command, &Timeouts::default());
        assert_eq!(
            entries["SessionStart"][0]["hooks"][0]["command"],
            "/mnt/d/Mode/bin/mode-hook.exe"
        );
    }

    #[test]
    fn stage_relay_copies_once() {
        let dir = tmp();
        let bundled = dir.path().join("bundle").join("mode-hook");
        write(&bundled, "#!/bin/sh\necho v1\n");
        let data = dir.path().join("Mode");
        let dest = stage_relay(&bundled, &data).unwrap();
        assert_eq!(dest, data.join("bin").join(RELAY_FILE_NAME));
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho v1\n");
        let mtime = std::fs::metadata(&dest).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        stage_relay(&bundled, &data).unwrap();
        assert_eq!(
            std::fs::metadata(&dest).unwrap().modified().unwrap(),
            mtime,
            "same bytes: no copy"
        );
        write(&bundled, "#!/bin/sh\necho v2\n");
        stage_relay(&bundled, &data).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/sh\necho v2\n");
    }

    /// The built relay, building it on demand when `cargo test -p mode-core`
    /// runs before `mode-hook` was compiled.
    fn built_relay() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("MODE_HOOK_BIN").filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(p));
        }
        let exe = std::env::current_exe().ok()?;
        let profile_dir = exe.parent()?.parent()?.to_path_buf();
        let relay = profile_dir.join(RELAY_FILE_NAME);
        if !relay.exists() {
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let status = Command::new(cargo)
                .args(["build", "-p", "mode-hook", "--quiet"])
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .status()
                .ok()?;
            if !status.success() {
                return None;
            }
        }
        relay.exists().then_some(relay)
    }

    #[cfg(unix)]
    fn test_endpoint(dir: &tempfile::TempDir, name: &str) -> Endpoint {
        Endpoint::at(
            dir.path()
                .join(format!("{name}.sock"))
                .display()
                .to_string(),
        )
    }

    #[cfg(windows)]
    fn test_endpoint(_dir: &tempfile::TempDir, name: &str) -> Endpoint {
        Endpoint::at(format!(
            r"\\.\pipe\mode-test-rt-{}-{name}",
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn round_trip_through_the_real_relay() {
        let Some(relay) = built_relay() else {
            eprintln!("mode-hook binary not available; skipping");
            return;
        };
        let dir = tmp();
        let endpoint = test_endpoint(&dir, "rt");
        let (server, mut inbound) = Server::bind(endpoint).await.unwrap();
        let report = round_trip(&relay, &server, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(report.session_id.starts_with(ROUND_TRIP_SESSION_PREFIX));
        assert!(report.acked, "the relay saw the ack");
        assert!(report.latency_ms < 1_000, "{}", report.latency_ms);
        assert_eq!(report.relay_version.as_deref(), Some(RELAY_VERSION));
        assert!(report.at.ends_with('Z'));
        assert_eq!(server.metrics().round_trips, 1);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), inbound.recv())
                .await
                .is_err(),
            "the synthetic frame never reaches the consumer"
        );
        assert_eq!(relay_version(&relay).as_deref(), Some(RELAY_VERSION));

        // Nobody listening: the relay exits 0 silently and the round trip reports it.
        drop(server);
        let dir2 = tmp();
        let endpoint2 = test_endpoint(&dir2, "rt2");
        let (server2, _rx) = Server::bind(endpoint2).await.unwrap();
        let missing = dir2.path().join("no-such-relay");
        assert!(matches!(
            round_trip(&missing, &server2, Duration::from_millis(200)).await,
            Err(RoundTripError::Spawn { .. })
        ));
    }
}
