//! Roster identity (05 §11.1–§11.2), event summaries (05 §4.2 "summary"),
//! terminal detection from the relay's side information (05 §9.1–§9.2) and
//! the git lookups a `SessionStart` / `CwdChanged` needs (`repo_root`,
//! `git_branch`).

use std::collections::BTreeMap;

use serde_json::Value;

use crate::paths;

/// The twelve avatar hues of 05 §11.2.
pub const AVATAR_HUES: u64 = 12;

/// The session-avatar variants of 05 §11.2 (ADR-0050): the bot mark wearing one
/// accessory, drawn in `assets/logo/final/bot-<variant>-{light,dark}.png`, in
/// assignment order. `''` on a row draws the plain mark.
pub const AVATAR_VARIANTS: [&str; 8] = [
    "beret", "specs", "tophat", "mustache", "beanie", "cap", "squares", "monocle",
];

/// The variant with the lowest `count`, the earlier one in [`AVATAR_VARIANTS`]
/// on a tie.
pub fn least_used_variant(count: impl Fn(&str) -> i64) -> &'static str {
    AVATAR_VARIANTS
        .iter()
        .copied()
        .min_by_key(|v| count(v))
        .unwrap_or(AVATAR_VARIANTS[0])
}

/// `display_name` (05 §11.1): title, else the repo folder, else the cwd
/// folder, else the first 8 characters of the Claude session id.
pub fn display_name(
    title: Option<&str>,
    repo_root: Option<&str>,
    cwd: &str,
    claude_session_id: &str,
) -> String {
    if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
        return t.to_string();
    }
    if let Some(name) = repo_root.and_then(basename) {
        return name;
    }
    if let Some(name) = basename(cwd) {
        return name;
    }
    claude_session_id.chars().take(8).collect()
}

/// The last path component, for display.
pub fn basename(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let name = trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())?;
    Some(name.to_string())
}

/// FNV-1a 64 of the bytes.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// `avatar.<k>`, `k = FNV-1a 64 of (repo_root ?? cwd) mod 12 + 1` (05 §11.2).
pub fn avatar_color(repo_root: Option<&str>, cwd: &str) -> String {
    let key = repo_root.unwrap_or(cwd);
    format!("avatar.{}", fnv1a_64(key.as_bytes()) % AVATAR_HUES + 1)
}

/// `<server>` when `tool_name` is `mcp__<server>__<tool>`.
pub fn mcp_server(tool_name: &str) -> Option<&str> {
    let rest = tool_name.strip_prefix("mcp__")?;
    let (server, _tool) = rest.split_once("__")?;
    (!server.is_empty()).then_some(server)
}

/// `<tool>` when `tool_name` is `mcp__<server>__<tool>`.
pub fn mcp_tool(tool_name: &str) -> Option<&str> {
    let rest = tool_name.strip_prefix("mcp__")?;
    let (_server, tool) = rest.split_once("__")?;
    (!tool.is_empty()).then_some(tool)
}

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    truncate_chars(line, max)
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

/// Canonicalize the path fields of a hook payload at ingress (05 §6.1):
/// `cwd`, `transcript_path` and `CwdChanged`'s `new_cwd`. The
/// tool's own `tool_input` stays as Claude Code sent it (an answer echoes it
/// back as `updatedInput`); its `file_path` is resolved where it is used
/// ([`crate::files::apply_hook_change`], [`tool_summary`]).
pub fn canonical_paths(ev: &mut mode_wire::HookEvent) {
    if !ev.cwd.is_empty() {
        ev.cwd = paths::canon(&ev.cwd);
    }
    if !ev.transcript_path.is_empty() {
        ev.transcript_path = paths::canon(&ev.transcript_path);
    }
    if let Some(Value::String(p)) = ev.extra.get_mut("new_cwd")
        && !p.is_empty()
    {
        *p = paths::canon(p);
    }
}

/// The one-line summary of a tool call: "Bash: npm test", "Edit src/main.rs",
/// "github: create_issue", "asks: Which database?".
pub fn tool_summary(tool_name: &str, input: Option<&Value>, base: Option<&str>) -> String {
    let get = |k: &str| input.and_then(|i| i.get(k)).and_then(Value::as_str);
    match tool_name {
        "Bash" => match get("command") {
            Some(cmd) => format!("Bash: {}", first_line(cmd, 80)),
            None => "Bash".into(),
        },
        "Edit" | "Write" | "MultiEdit" | "Read" | "NotebookEdit" => {
            match get("file_path").or_else(|| get("notebook_path")) {
                Some(p) => format!("{tool_name} {}", paths::display(p, base)),
                None => tool_name.into(),
            }
        }
        "Glob" | "Grep" => match get("pattern") {
            Some(p) => format!("{tool_name} {}", truncate_chars(p, 60)),
            None => tool_name.into(),
        },
        "WebFetch" => match get("url") {
            Some(u) => format!("WebFetch {}", truncate_chars(u, 80)),
            None => "WebFetch".into(),
        },
        "WebSearch" => match get("query") {
            Some(q) => format!("WebSearch {}", truncate_chars(q, 60)),
            None => "WebSearch".into(),
        },
        "AskUserQuestion" => match first_question(input) {
            Some(q) => format!("asks: {}", truncate_chars(&q, 120)),
            None => "asks a question".into(),
        },
        "Agent" | "Task" => match get("description").or_else(|| get("subagent_type")) {
            Some(d) => format!("{tool_name}: {}", truncate_chars(d, 80)),
            None => tool_name.into(),
        },
        _ => match (mcp_server(tool_name), mcp_tool(tool_name)) {
            (Some(server), Some(tool)) => format!("{server}: {tool}"),
            _ => tool_name.to_string(),
        },
    }
}

/// The first question text of an `AskUserQuestion` input.
pub fn first_question(input: Option<&Value>) -> Option<String> {
    input?
        .get("questions")?
        .as_array()?
        .first()?
        .get("question")?
        .as_str()
        .map(str::to_string)
}

/// Line counts for the `+N −M` roster text.
pub fn diff_text(added: i64, removed: i64) -> String {
    format!("+{added} \u{2212}{removed}")
}

/// Elapsed as "12s", "15m 12s", "1h 04m", "1d 02h" (05 §11.3), as the UI writes it.
pub fn elapsed_text(seconds: i64) -> String {
    let s = seconds.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else if s < 86_400 {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d {:02}h", s / 86_400, (s % 86_400) / 3600)
    }
}

// ------------------------------------------------------------------ terminal

/// Which terminal app hosts a session and how to find its tab (05 §9.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRef {
    /// `terminal.app` | `iterm2` | `warp` | `ghostty` | `windows_terminal` | `wsl` | `powershell`
    pub app: String,
    /// `iterm2:w0t1p0:…`, `terminal.app:/dev/ttys004`, `wt:{GUID}`.
    pub reference: String,
}

/// Detect the hosting terminal from the relay's allow-listed environment and tty.
pub fn terminal_from_env(env: &BTreeMap<String, String>, tty: Option<&str>) -> Option<TerminalRef> {
    let get = |k: &str| env.get(k).map(String::as_str).filter(|v| !v.is_empty());
    if let Some(wt) = get("WT_SESSION") {
        let app = if get("WSL_DISTRO_NAME").is_some() {
            "wsl"
        } else {
            "windows_terminal"
        };
        return Some(TerminalRef {
            app: app.into(),
            reference: format!("wt:{wt}"),
        });
    }
    match get("TERM_PROGRAM") {
        Some("Apple_Terminal") => {
            return Some(TerminalRef {
                app: "terminal.app".into(),
                reference: format!(
                    "terminal.app:{}",
                    tty.or(get("TERM_SESSION_ID")).unwrap_or("")
                ),
            });
        }
        Some("iTerm.app") => {
            return Some(TerminalRef {
                app: "iterm2".into(),
                reference: format!("iterm2:{}", get("ITERM_SESSION_ID").unwrap_or("")),
            });
        }
        Some("WarpTerminal") => {
            return Some(TerminalRef {
                app: "warp".into(),
                reference: format!("warp:{}", tty.unwrap_or("")),
            });
        }
        Some("ghostty") => {
            return Some(TerminalRef {
                app: "ghostty".into(),
                reference: format!("ghostty:{}", tty.unwrap_or("")),
            });
        }
        _ => {}
    }
    if get("GHOSTTY_RESOURCES_DIR").is_some() {
        return Some(TerminalRef {
            app: "ghostty".into(),
            reference: format!("ghostty:{}", tty.unwrap_or("")),
        });
    }
    if get("WSL_DISTRO_NAME").is_some() {
        return Some(TerminalRef {
            app: "wsl".into(),
            reference: format!("wsl:{}", get("WSL_DISTRO_NAME").unwrap_or("")),
        });
    }
    // Any other host that names itself (an editor terminal, another emulator):
    // kept so jump to terminal can say where the session runs (05 §9.3
    // "Unsupported host").
    if let Some(program) = get("TERM_PROGRAM").and_then(term_program_name) {
        return Some(TerminalRef {
            reference: format!("{OTHER_TERMINAL_PREFIX}{program}"),
            app: program,
        });
    }
    None
}

/// `terminal_ref` prefix for a host Mode cannot focus; the rest is its `TERM_PROGRAM`.
pub const OTHER_TERMINAL_PREFIX: &str = "term_program:";

/// A `TERM_PROGRAM` value fit to store and show: printable, at most 40 characters.
fn term_program_name(raw: &str) -> Option<String> {
    let clean: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .take(40)
        .collect();
    let clean = clean.trim().to_string();
    (!clean.is_empty()).then_some(clean)
}

// ----------------------------------------------------------------------- git

/// `repo_root` and `git_branch` of a working directory (05 §4.2 `SessionStart`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoInfo {
    pub repo_root: Option<String>,
    pub git_branch: Option<String>,
}

/// Look the repository up with the user's `git` (2 s per command, never
/// writing). A missing directory or `git` gives an empty answer.
pub fn repo_info(cwd: &str) -> RepoInfo {
    if cwd.is_empty() || !paths::to_os(cwd).is_dir() {
        return RepoInfo::default();
    }
    let root = crate::files::git::repo_root(&paths::to_os(cwd));
    let branch = root
        .as_deref()
        .and_then(|r| crate::files::git::branch(&paths::to_os(r)));
    RepoInfo {
        repo_root: root,
        git_branch: branch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_precedence() {
        assert_eq!(
            display_name(
                Some("Fix auth"),
                Some("/u/mode"),
                "/u/mode/src",
                "7f3a1c2e-rest"
            ),
            "Fix auth"
        );
        assert_eq!(
            display_name(Some("  "), Some("/u/mode/"), "/u/mode/src", "7f3a"),
            "mode"
        );
        assert_eq!(display_name(None, None, "/u/scratch", "7f3a"), "scratch");
        assert_eq!(display_name(None, None, "", "7f3a1c2e-4b5d"), "7f3a1c2e");
        assert_eq!(basename(r"C:\Users\ana\code\mode"), Some("mode".into()));
    }

    #[test]
    fn avatar_colour_is_stable_and_within_the_palette() {
        let a = avatar_color(Some("/u/mode"), "/u/mode/src");
        assert_eq!(a, avatar_color(Some("/u/mode"), "/elsewhere"));
        let k: u64 = a.strip_prefix("avatar.").unwrap().parse().unwrap();
        assert!((1..=12).contains(&k));
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn least_used_variant_prefers_the_lowest_count_then_list_order() {
        assert_eq!(least_used_variant(|_| 0), "beret");
        let worn = |v: &str| match v {
            "beret" | "specs" | "tophat" => 2,
            "mustache" => 1,
            _ => 1,
        };
        assert_eq!(least_used_variant(worn), "mustache");
        let only_monocle_free = |v: &str| i64::from(v != "monocle");
        assert_eq!(least_used_variant(only_monocle_free), "monocle");
    }

    #[test]
    fn summaries_name_the_tool_and_its_target() {
        let bash = serde_json::json!({"command": "npm test\n&& echo"});
        assert_eq!(tool_summary("Bash", Some(&bash), None), "Bash: npm test");
        let edit = serde_json::json!({"file_path": "/u/mode/src/main.rs"});
        assert_eq!(
            tool_summary("Edit", Some(&edit), Some("/u/mode")),
            "Edit src/main.rs"
        );
        assert_eq!(
            tool_summary("Edit", Some(&edit), Some("/other")),
            "Edit /u/mode/src/main.rs"
        );
        let q = serde_json::json!({"questions": [{"question": "Which db?", "options": []}]});
        assert_eq!(
            tool_summary("AskUserQuestion", Some(&q), None),
            "asks: Which db?"
        );
        assert_eq!(
            tool_summary("mcp__github__create_issue", None, None),
            "github: create_issue"
        );
        assert_eq!(mcp_server("mcp__github__create_issue"), Some("github"));
        assert_eq!(mcp_server("Bash"), None);
        assert_eq!(tool_summary("Weird", None, None), "Weird");
        // Windows spellings name the target relative to the repo too.
        let win = serde_json::json!({"file_path": r"C:\Users\a\mode\src\main.rs"});
        assert_eq!(
            tool_summary("Edit", Some(&win), Some("C:/Users/a/mode")),
            "Edit src/main.rs"
        );
        let sibling = serde_json::json!({"file_path": "/u/model/x.rs"});
        assert_eq!(
            tool_summary("Write", Some(&sibling), Some("/u/mode")),
            "Write /u/model/x.rs",
            "a sibling directory with a common prefix is outside"
        );
        assert_eq!(elapsed_text(12), "12s");
        assert_eq!(elapsed_text(240), "4m 00s");
        assert_eq!(elapsed_text(124), "2m 04s");
        assert_eq!(elapsed_text(912), "15m 12s");
        assert_eq!(elapsed_text(3840), "1h 04m");
        assert_eq!(elapsed_text(4800), "1h 20m");
        assert_eq!(elapsed_text(93_600), "1d 02h");
        assert_eq!(elapsed_text(200_000), "2d 07h");
        assert_eq!(diff_text(12, 3), "+12 \u{2212}3");
    }

    #[test]
    fn hook_payload_paths_are_canonical_at_ingress_and_tool_input_is_verbatim() {
        let mut ev = mode_wire::HookEvent::new("CwdChanged", "s");
        ev.cwd = r"c:\Users\ana\mode\".into();
        ev.transcript_path = r"C:\Users\ana\.claude\projects\C--Users-ana-mode\s.jsonl".into();
        ev.set("new_cwd", r"C:\Users\ana\mode\src\..\docs");
        ev.set(
            "tool_input",
            serde_json::json!({"file_path": r"C:\Users\ana\mode\a.rs"}),
        );
        canonical_paths(&mut ev);
        assert_eq!(ev.cwd, "C:/Users/ana/mode");
        assert_eq!(
            ev.transcript_path,
            "C:/Users/ana/.claude/projects/C--Users-ana-mode/s.jsonl"
        );
        assert_eq!(ev.get("new_cwd").unwrap(), "C:/Users/ana/mode/docs");
        assert_eq!(
            ev.get("tool_input").unwrap()["file_path"],
            r"C:\Users\ana\mode\a.rs",
            "answers echo tool_input back verbatim"
        );
        let mut unix = mode_wire::HookEvent::new("SessionStart", "s");
        unix.cwd = "/u/mode/".into();
        canonical_paths(&mut unix);
        assert_eq!(
            (unix.cwd.as_str(), unix.transcript_path.as_str()),
            ("/u/mode", "")
        );
    }

    #[test]
    fn terminal_detection_prefers_windows_terminal_then_term_program() {
        let mut env = BTreeMap::new();
        assert_eq!(terminal_from_env(&env, Some("/dev/ttys004")), None);
        env.insert("TERM_PROGRAM".into(), "iTerm.app".into());
        env.insert("ITERM_SESSION_ID".into(), "w0t1p0:6D1C".into());
        let t = terminal_from_env(&env, Some("/dev/ttys004")).unwrap();
        assert_eq!(t.app, "iterm2");
        assert_eq!(t.reference, "iterm2:w0t1p0:6D1C");
        env.insert("TERM_PROGRAM".into(), "Apple_Terminal".into());
        let t = terminal_from_env(&env, Some("/dev/ttys004")).unwrap();
        assert_eq!(
            (t.app.as_str(), t.reference.as_str()),
            ("terminal.app", "terminal.app:/dev/ttys004")
        );
        env.insert("WT_SESSION".into(), "{guid}".into());
        assert_eq!(
            terminal_from_env(&env, None).unwrap().app,
            "windows_terminal"
        );
        env.insert("WSL_DISTRO_NAME".into(), "Ubuntu".into());
        assert_eq!(terminal_from_env(&env, None).unwrap().app, "wsl");
        let mut g = BTreeMap::new();
        g.insert("GHOSTTY_RESOURCES_DIR".into(), "/x".into());
        assert_eq!(terminal_from_env(&g, None).unwrap().app, "ghostty");
        let mut v = BTreeMap::new();
        v.insert("TERM_PROGRAM".into(), "vscode".into());
        let t = terminal_from_env(&v, Some("/dev/pts/3")).unwrap();
        assert_eq!(
            (t.app.as_str(), t.reference.as_str()),
            ("vscode", "term_program:vscode")
        );
        v.insert("TERM_PROGRAM".into(), "  \u{1b}\u{7}/ ".into());
        assert_eq!(terminal_from_env(&v, None), None, "nothing printable left");
    }

    #[test]
    fn repo_info_is_empty_outside_a_repository() {
        assert_eq!(repo_info("/definitely/not/here"), RepoInfo::default());
        assert_eq!(repo_info(""), RepoInfo::default());
    }
}
