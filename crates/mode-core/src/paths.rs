//! One internal form for every path the core stores, compares or shows.
//!
//! Paths reach the core from places that disagree on spelling: hook payloads
//! (`cwd`, `transcript_path`, `tool_input.file_path`) carry the OS's
//! separators, `git` prints forward slashes and its own drive-letter case,
//! transcripts and the notes vault come from directory walks, settings are
//! typed by the user (`~/Mode/Notes`). Comparing those as raw strings made a
//! hook row and a git row for the same file two rows on Windows. The rule is
//! therefore: convert at every ingress with [`canon`] / [`from_os`], keep the
//! canonical string everywhere inside (database rows, maps, comparisons), and
//! convert back with [`to_os`] only at the OS boundary (opening, statting,
//! spawning in a directory).
//!
//! The canonical form:
//!
//! | Input | Canonical |
//! |---|---|
//! | `C:\Users\a\x.rs`, `c:/Users/a/x.rs` | `C:/Users/a/x.rs` (forward slashes, drive letter upper-cased) |
//! | `C:\`, `C:` | `C:/` (a root keeps its separator) |
//! | `\\server\share\dir\` | `//server/share/dir` (UNC: the `//server/share` prefix is kept intact and never climbed above) |
//! | `\\?\C:\x`, `\\?\UNC\server\share\x` | `C:/x`, `//server/share/x` (verbatim prefixes dropped) |
//! | `/u/mode/`, `/u//mode/./src/../a.rs` | `/u/mode`, `/u/mode/a.rs` (no trailing slash, `.` and `..` resolved lexically) |
//! | `~`, `~/Notes` | the home directory, `<home>/Notes` ([`canon`] only: a setting the user typed, 04 §2.2) |
//!
//! A `\` is a separator in a Windows-shaped path (a drive letter, a UNC or
//! `\`-rooted path) and on a Windows host; in a Unix path it stays part of a
//! file name. The functions never touch the file system (no symlink
//! resolution), so they behave the same on every OS and the Windows cases
//! are tested on Linux. Comparisons ([`key`], [`same`], [`relative`]) fold
//! ASCII case for Windows-shaped paths, whose file systems ignore case.
//!
//! What the UI shows (05 §6.1, 03 §3.9 "`rel_path` when inside `repo_root`,
//! else `path`") is [`display`].

use std::path::{Path, PathBuf};

/// The canonical form of `p` (see the module table); a leading `~` is the
/// home directory. `""` stays `""`; a relative path stays relative
/// (normalized: `./a/../b` is `b`, leading `..` are kept).
pub fn canon(p: &str) -> String {
    let home = dirs::home_dir().map(|h| normalise(&h.to_string_lossy()));
    canon_with_home(p, home.as_deref())
}

/// [`canon`] with an explicit home directory (`None`: `~` stays literal).
pub fn canon_with_home(p: &str, home: Option<&str>) -> String {
    let win = windows_flavour(p) || home.is_some_and(windows_flavour);
    let tilde_rest = if p == "~" {
        Some("")
    } else {
        p.strip_prefix("~/")
            .or_else(|| if win { p.strip_prefix("~\\") } else { None })
    };
    match (tilde_rest, home) {
        (Some(rest), Some(home)) if !home.is_empty() => normalise(&format!(
            "{home}/{}",
            if win {
                rest.replace('\\', "/")
            } else {
                rest.into()
            }
        )),
        _ => normalise(p),
    }
}

/// `p` resolved against `base` (05 §6.1: a `cwd`-relative hook path is made
/// absolute against the session's `cwd`). An absolute or `~`-rooted `p` is
/// returned canonical as is; an empty `base` leaves `p` as given (canonical).
/// `\` in `p` is a separator when `base` is Windows-shaped.
pub fn join(base: &str, p: &str) -> String {
    if p.is_empty() {
        return canon(base);
    }
    if is_absolute(p) || p == "~" || p.starts_with("~/") || p.starts_with("~\\") {
        return canon(p);
    }
    join_rel(base, p)
}

/// `rel` under `base`, where `rel` is relative by contract (a path `git`
/// reports from the repository root, a vault-relative path): never treated
/// as absolute or home-relative, so a directory literally named `~` stays one.
pub fn join_rel(base: &str, rel: &str) -> String {
    if base.is_empty() {
        return normalise(rel);
    }
    if rel.is_empty() {
        return normalise(base);
    }
    normalise(&format!("{base}/{rel}"))
}

/// `abs` relative to `base` when it lies strictly inside it (`/u/mode`,
/// `/u/mode/src/a.rs` → `src/a.rs`); `None` for `base` itself, for a path
/// outside it (`/u/model/x` is not inside `/u/mode`) and for relative input.
pub fn relative(base: &str, abs: &str) -> Option<String> {
    let base = normalise(base);
    let abs = normalise(abs);
    if base.is_empty() || !is_absolute(&abs) {
        return None;
    }
    let (kb, ka) = (key(&base), key(&abs));
    let prefix = if kb.ends_with('/') {
        kb.clone()
    } else {
        format!("{kb}/")
    };
    if ka.len() > prefix.len() && ka.starts_with(&prefix) {
        // ASCII folding keeps byte offsets, so the original spelling is sliced.
        Some(abs[prefix.len()..].to_string())
    } else {
        None
    }
}

/// The short form the UI shows for a path: relative to `repo_root` when
/// inside it, else the whole canonical path (03 §3.9, 05 §6.1).
pub fn display(abs: &str, repo_root: Option<&str>) -> String {
    repo_root
        .and_then(|root| relative(root, abs))
        .unwrap_or_else(|| normalise(abs))
}

/// The OS path for a canonical one, at the boundary (open, stat, spawn).
pub fn to_os(p: &str) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(p.replace('/', "\\"))
    } else {
        PathBuf::from(p)
    }
}

/// The canonical form of a path the OS handed over (a directory walk, a
/// watcher event, `current_dir`). A leading `~` here is a directory of
/// that name, never the home directory.
pub fn from_os(p: &Path) -> String {
    normalise(&p.to_string_lossy())
}

/// Whether `p` is absolute in its own flavor: `/x`, `\x`, `C:\x`, `C:/x`,
/// `\\server\share`, `//server/share`.
pub fn is_absolute(p: &str) -> bool {
    p.starts_with(['/', '\\']) || drive_prefix(p).is_some()
}

/// The key two spellings of one path share: canonical, ASCII case folded
/// for Windows-shaped paths. For maps and equality, never for display.
pub fn key(p: &str) -> String {
    let c = normalise(p);
    if drive_prefix(&c).is_some() || c.starts_with("//") {
        c.to_ascii_lowercase()
    } else {
        c
    }
}

/// Whether two spellings name the same path ([`key`] equality).
pub fn same(a: &str, b: &str) -> bool {
    key(a) == key(b)
}

// ------------------------------------------------------------------ internals

/// The drive letter of `C:`, `C:\…`, `C:/…` (not `C:foo`, which a Unix name can be).
fn drive_prefix(p: &str) -> Option<char> {
    let mut chars = p.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    match chars.next() {
        None | Some('/') | Some('\\') => Some(letter),
        _ => None,
    }
}

/// Whether `\` separates components in `p`.
fn windows_flavour(p: &str) -> bool {
    cfg!(windows) || p.starts_with('\\') || drive_prefix(p).is_some()
}

#[derive(Debug, PartialEq, Eq)]
enum Root {
    Relative,
    Unix,
    Drive(char),
    Unc(String, String),
}

/// Separators, roots, `.` / `..`, duplicate and trailing separators; no `~`.
fn normalise(p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }
    let s = if windows_flavour(p) {
        p.replace('\\', "/")
    } else {
        p.to_string()
    };
    let (root, rest) = split_root(&s);
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => match parts.last() {
                Some(&last) if last != ".." => {
                    parts.pop();
                }
                // Never above a root; a relative path keeps its leading `..`.
                _ if root == Root::Relative => parts.push(".."),
                _ => {}
            },
            other => parts.push(other),
        }
    }
    let tail = parts.join("/");
    match root {
        Root::Relative if tail.is_empty() => ".".into(),
        Root::Relative => tail,
        Root::Unix => format!("/{tail}"),
        Root::Drive(d) => format!("{}:/{tail}", d.to_ascii_uppercase()),
        Root::Unc(server, share) => {
            let mut out = format!("//{server}");
            for part in [share.as_str(), tail.as_str()] {
                if !part.is_empty() {
                    out.push('/');
                    out.push_str(part);
                }
            }
            out
        }
    }
}

/// The root of a path whose separators are already `/`, and the rest.
fn split_root(s: &str) -> (Root, &str) {
    // Verbatim and device prefixes (`\\?\`, `\\.\`).
    for prefix in ["//?/", "//./"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            if let Some(unc) = rest
                .strip_prefix("UNC/")
                .or_else(|| rest.strip_prefix("unc/"))
            {
                return unc_root(unc);
            }
            if let Some(d) = drive_prefix(rest) {
                return (Root::Drive(d), rest.get(2..).unwrap_or(""));
            }
            // `\\.\pipe\name` and the like: the device is the "server".
            return unc_root(&s[2..]);
        }
    }
    if let Some(rest) = s.strip_prefix("//")
        && !rest.starts_with('/')
        && !rest.is_empty()
    {
        return unc_root(rest);
    }
    if let Some(d) = drive_prefix(s) {
        return (Root::Drive(d), s.get(2..).unwrap_or(""));
    }
    if let Some(rest) = s.strip_prefix('/') {
        return (Root::Unix, rest);
    }
    (Root::Relative, s)
}

/// `server/share/rest` → the UNC root and `rest`.
fn unc_root(s: &str) -> (Root, &str) {
    let mut it = s.splitn(3, '/');
    let server = it.next().unwrap_or("").to_string();
    let share = it.next().unwrap_or("").to_string();
    let rest = it.next().unwrap_or("");
    if share.is_empty() {
        // `//server` alone: nothing to protect.
        return (Root::Unc(server, String::new()), rest);
    }
    (Root::Unc(server, share), rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME_UNIX: &str = "/home/ana";
    const HOME_WIN: &str = "C:/Users/ana";

    /// (input, canonical) on every OS; Windows inputs included on purpose.
    const CANON: &[(&str, &str)] = &[
        // Unix
        ("/u/mode", "/u/mode"),
        ("/u/mode/", "/u/mode"),
        ("/u//mode///src/", "/u/mode/src"),
        ("/u/mode/./src/../a.rs", "/u/mode/a.rs"),
        ("/u/mode/src/../../../..", "/"),
        ("/", "/"),
        ("//", "/"),
        ("///u/mode", "/u/mode"),
        // Windows drive paths
        (r"C:\Users\a\x.rs", "C:/Users/a/x.rs"),
        (r"c:\Users\a\x.rs", "C:/Users/a/x.rs"),
        ("c:/Users/a/x.rs", "C:/Users/a/x.rs"),
        (r"C:\Users/a\mixed/x.rs", "C:/Users/a/mixed/x.rs"),
        (r"C:\Users\a\mode\", "C:/Users/a/mode"),
        (r"C:\Users\a\mode\src\..\.\lib.rs", "C:/Users/a/mode/lib.rs"),
        (r"C:\..\..", "C:/"),
        (r"C:\", "C:/"),
        ("C:", "C:/"),
        (r"\\?\C:\Users\a\x.rs", "C:/Users/a/x.rs"),
        // UNC
        (r"\\server\share\dir\x.rs", "//server/share/dir/x.rs"),
        (r"\\server\share\dir\", "//server/share/dir"),
        (r"\\server\share\..\..\x", "//server/share/x"),
        ("//server/share/dir/x.rs", "//server/share/dir/x.rs"),
        (r"\\?\UNC\server\share\x", "//server/share/x"),
        (
            r"\\wsl$\Ubuntu\home\ana\.claude",
            "//wsl$/Ubuntu/home/ana/.claude",
        ),
        (r"\\.\pipe\mode-dev", "//./pipe/mode-dev"),
        (r"\\server", "//server"),
        // Relative
        ("src/a.rs", "src/a.rs"),
        ("./src/../b.rs", "b.rs"),
        ("../x", "../x"),
        ("a/../..", ".."),
        ("./", "."),
        ("", ""),
        // A Unix name may hold a backslash or start with a drive-like word.
        ("/u/we\\ird.rs", "/u/we\\ird.rs"),
        ("c:notes.md", "c:notes.md"),
    ];

    #[test]
    fn canonical_form_table() {
        for (input, want) in CANON {
            // The Unix-only backslash case depends on the host flavor.
            if cfg!(windows) && input.contains("we\\ird") {
                continue;
            }
            assert_eq!(canon_with_home(input, None), *want, "canon({input:?})");
            // Idempotent.
            assert_eq!(canon_with_home(want, None), *want, "canon({want:?}) again");
        }
    }

    #[test]
    fn home_is_expanded_by_canon_only() {
        let h = Some(HOME_UNIX);
        assert_eq!(canon_with_home("~", h), "/home/ana");
        assert_eq!(canon_with_home("~/", h), "/home/ana");
        assert_eq!(canon_with_home("~/Mode/Notes/", h), "/home/ana/Mode/Notes");
        assert_eq!(canon_with_home("~/a/../b", h), "/home/ana/b");
        // `~user` and a `~` inside a path are names, not the home directory.
        assert_eq!(canon_with_home("~bob/x", h), "~bob/x");
        assert_eq!(canon_with_home("/u/~/x", h), "/u/~/x");
        // A Windows home keeps its drive; `~\` is a separator there.
        let w = Some(HOME_WIN);
        assert_eq!(
            canon_with_home(r"~\Mode\Notes", w),
            "C:/Users/ana/Mode/Notes"
        );
        assert_eq!(
            canon_with_home("~/Mode/Notes", w),
            "C:/Users/ana/Mode/Notes"
        );
        // Without a home `~` stays as written.
        assert_eq!(canon_with_home("~/x", None), "~/x");
        // From the OS, `~` is a directory of that name; comparisons and the
        // display form never expand it either.
        assert_eq!(from_os(Path::new("~/x")), "~/x");
        assert_eq!(display("~/x", None), "~/x");
        assert_eq!(key("~/x"), "~/x");
        // The real home directory is canonical.
        if let Some(home) = dirs::home_dir() {
            assert_eq!(canon("~"), from_os(&home));
        }
    }

    #[test]
    fn join_resolves_against_the_base_in_its_flavour() {
        assert_eq!(join("/u/mode", "src/a.rs"), "/u/mode/src/a.rs");
        assert_eq!(join("/u/mode/", "./src/../a.rs"), "/u/mode/a.rs");
        assert_eq!(join("/u/mode", "/abs/a.rs"), "/abs/a.rs");
        assert_eq!(join("/u/mode", "../other/a.rs"), "/u/other/a.rs");
        assert_eq!(join("", "src/a.rs"), "src/a.rs");
        assert_eq!(join("/u/mode", ""), "/u/mode");
        // A Windows base makes `\` a separator in the relative part.
        assert_eq!(
            join(r"C:\Users\a\mode", r"src\a.rs"),
            "C:/Users/a/mode/src/a.rs"
        );
        assert_eq!(
            join("C:/Users/a/mode", r"src\a.rs"),
            "C:/Users/a/mode/src/a.rs"
        );
        assert_eq!(join("/u/mode", r"C:\x\a.rs"), "C:/x/a.rs");
        assert_eq!(join(r"C:\Users\a", r"\\srv\share\a.rs"), "//srv/share/a.rs");
        assert_eq!(
            join(r"\\srv\share\repo", r"src\a.rs"),
            "//srv/share/repo/src/a.rs"
        );
        // `join_rel` never treats its second part as rooted elsewhere.
        assert_eq!(join_rel("/u/mode", "~/x"), "/u/mode/~/x");
        assert_eq!(join_rel("C:/u/mode", "src/a.rs"), "C:/u/mode/src/a.rs");
    }

    #[test]
    fn relative_is_component_aware_and_case_folds_windows_paths() {
        assert_eq!(
            relative("/u/mode", "/u/mode/src/a.rs").as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(
            relative("/u/mode/", "/u/mode/src/a.rs").as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(
            relative("/u/mode", "/u/model/x.rs"),
            None,
            "a sibling with a common prefix"
        );
        assert_eq!(relative("/u/mode", "/u/mode"), None, "the base itself");
        assert_eq!(relative("/u/mode", "/u/mode/"), None);
        assert_eq!(relative("/u/mode", "src/a.rs"), None, "relative input");
        assert_eq!(relative("/", "/etc/hosts").as_deref(), Some("etc/hosts"));
        // Unix paths are case-sensitive.
        assert_eq!(relative("/u/Mode", "/u/mode/a.rs"), None);
        // Windows: hook spelling vs git spelling.
        assert_eq!(
            relative("C:/Users/a/mode", r"c:\Users\A\Mode\src\a.rs").as_deref(),
            Some("src/a.rs")
        );
        assert_eq!(relative(r"C:\", r"C:\x\y").as_deref(), Some("x/y"));
        assert_eq!(relative(r"C:\a", r"D:\a\x"), None);
        assert_eq!(
            relative(r"\\srv\share\repo", "//SRV/share/repo/a.rs").as_deref(),
            Some("a.rs")
        );
    }

    #[test]
    fn display_is_rel_path_inside_the_repo_else_the_path() {
        assert_eq!(display("/u/mode/src/a.rs", Some("/u/mode")), "src/a.rs");
        assert_eq!(
            display("/elsewhere/a.rs", Some("/u/mode")),
            "/elsewhere/a.rs"
        );
        assert_eq!(display("/u/mode/src/a.rs", None), "/u/mode/src/a.rs");
        assert_eq!(
            display(r"C:\Users\a\mode\src\a.rs", Some("C:/Users/a/mode")),
            "src/a.rs"
        );
        assert_eq!(
            display(r"C:\tmp\a.rs", Some("C:/Users/a/mode")),
            "C:/tmp/a.rs"
        );
    }

    #[test]
    fn keys_match_the_spellings_of_one_path() {
        assert!(same(
            r"C:\Users\a\mode\src\a.rs",
            "C:/Users/a/mode/src/a.rs"
        ));
        assert!(same(
            r"c:\users\a\MODE\src\a.rs",
            "C:/Users/a/mode/src/a.rs"
        ));
        assert!(same(r"\\srv\share\a", "//SRV/Share/a/"));
        assert!(same("/u/mode/src/./a.rs", "/u/mode/src/a.rs"));
        assert!(
            !same("/u/mode/A.rs", "/u/mode/a.rs"),
            "Unix is case-sensitive"
        );
        assert_eq!(key("/u/Mode"), "/u/Mode");
        assert_eq!(key(r"C:\Users\Ana"), "c:/users/ana");
    }

    #[test]
    fn os_round_trip() {
        let p = to_os("/u/mode/src/a.rs");
        assert_eq!(from_os(&p), "/u/mode/src/a.rs");
        if cfg!(windows) {
            assert_eq!(to_os("C:/Users/a/x.rs"), PathBuf::from(r"C:\Users\a\x.rs"));
            assert_eq!(to_os("//srv/share/x"), PathBuf::from(r"\\srv\share\x"));
        } else {
            assert_eq!(to_os("/u/mode"), PathBuf::from("/u/mode"));
        }
        let dir = std::env::temp_dir();
        assert!(is_absolute(&from_os(&dir)));
        assert_eq!(from_os(&to_os(&from_os(&dir))), from_os(&dir));
    }

    #[test]
    fn absolute_in_either_flavour() {
        for p in [
            "/u",
            r"\u",
            r"C:\x",
            "C:/x",
            "C:",
            r"\\srv\share",
            "//srv/share",
        ] {
            assert!(is_absolute(p), "{p}");
        }
        for p in ["src/a.rs", "c:notes.md", "~/x", "", "."] {
            assert!(!is_absolute(p), "{p}");
        }
    }
}
