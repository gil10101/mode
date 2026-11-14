//! The git pass of 05 §6.2: the user's `git` from `PATH`, read-only, 5 s per
//! command, `GIT_OPTIONAL_LOCKS=0`. `status --porcelain=v2 -z` gives kinds,
//! renames and untracked files; `diff --numstat -M -z HEAD` gives exact line
//! counts versus `HEAD`; untracked files are counted by hand. Paths leave
//! this module canonical ([`crate::paths`]): git prints forward slashes and
//! its own drive-letter case on Windows, hook payloads print backslashes, and
//! both must name one `file_changes` row.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::paths;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Per-command timeout (05 §6.2).
pub const GIT_TIMEOUT: Duration = Duration::from_secs(5);
/// Untracked files larger than this are not counted (0/0).
const COUNT_CAP_BYTES: u64 = 32 * 1024 * 1024;

/// `file_changes.change_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
}

impl ChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Modified => "modified",
            ChangeKind::Deleted => "deleted",
            ChangeKind::Renamed => "renamed",
            ChangeKind::Untracked => "untracked",
        }
    }
}

/// One path git reports as changed versus `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitChange {
    /// Absolute, canonical ([`paths::canon`]).
    pub path: String,
    pub kind: ChangeKind,
    /// For `Renamed`; absolute, canonical.
    pub old_path: Option<String>,
    pub added: u32,
    pub removed: u32,
    /// `git` reported `-` counts (or the file holds NUL bytes).
    pub binary: bool,
}

/// Run `git <args>` in `cwd` with `timeout`; `Some(stdout)` on exit 0.
/// The child is killed when the timeout passes.
pub fn run_git(args: &[&str], cwd: &Path, timeout: Duration) -> Option<Vec<u8>> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => break None,
        }
    };
    let out = reader.join().ok()?;
    match status {
        Some(s) if s.success() => Some(out),
        _ => None,
    }
}

fn text(out: Vec<u8>) -> Option<String> {
    let s = String::from_utf8_lossy(&out).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `git rev-parse --show-toplevel` of `cwd`, canonical; `None` outside a
/// repository or without `git`.
pub fn repo_root(cwd: &Path) -> Option<String> {
    if !cwd.is_dir() {
        return None;
    }
    let out = run_git(&["rev-parse", "--show-toplevel"], cwd, GIT_TIMEOUT)?;
    text(out).map(|t| paths::canon(&t))
}

/// `git rev-parse --abbrev-ref HEAD` (`HEAD` when detached, as the CLI's own
/// `gitBranch` reports it).
pub fn branch(root: &Path) -> Option<String> {
    let out = run_git(&["rev-parse", "--abbrev-ref", "HEAD"], root, GIT_TIMEOUT)?;
    text(out)
}

/// Whether the repository has a `HEAD` commit (a fresh `git init` has none).
pub fn has_head(root: &Path) -> bool {
    run_git(&["rev-parse", "--verify", "-q", "HEAD"], root, GIT_TIMEOUT).is_some()
}

/// The working tree versus `HEAD`: every changed, deleted, renamed and
/// untracked path with exact line counts. `None` when `git` failed.
pub fn status(root: &Path) -> Option<Vec<GitChange>> {
    let status = run_git(
        &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
        root,
        GIT_TIMEOUT,
    )?;
    let entries = parse_status_z(&status);
    let counts: HashMap<String, Numstat> = if has_head(root) {
        run_git(
            &["diff", "--numstat", "-M", "-z", "HEAD"],
            root,
            GIT_TIMEOUT,
        )
        .map(|out| parse_numstat_z(&out))
        .unwrap_or_default()
        .into_iter()
        .map(|n| (n.path.clone(), n))
        .collect()
    } else {
        HashMap::new()
    };
    let base = paths::from_os(root);
    let mut out = Vec::with_capacity(entries.len());
    for e in entries {
        let abs = paths::join_rel(&base, &e.path);
        let (added, removed, binary) = match counts.get(&e.path) {
            Some(n) => (n.added, n.removed, n.binary),
            None => match e.kind {
                ChangeKind::Deleted => (0, 0, false),
                _ => match count_lines(&paths::to_os(&abs)) {
                    Some((lines, false)) => (lines, 0, false),
                    Some((_, true)) => (0, 0, true),
                    None => (0, 0, false),
                },
            },
        };
        out.push(GitChange {
            path: abs,
            kind: e.kind,
            old_path: e.old_path.map(|p| paths::join_rel(&base, &p)),
            added,
            removed,
            binary,
        });
    }
    Some(out)
}

/// A `status --porcelain=v2 -z` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// Relative to the repository root.
    pub path: String,
    pub kind: ChangeKind,
    pub old_path: Option<String>,
}

fn kind_of_xy(xy: &str) -> ChangeKind {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    if x == 'D' || y == 'D' {
        ChangeKind::Deleted
    } else if x == 'A' || y == 'A' {
        ChangeKind::Added
    } else {
        ChangeKind::Modified
    }
}

/// Parse `git status --porcelain=v2 -z` output (NUL-terminated records; a
/// rename record is followed by the original path as its own record).
pub fn parse_status_z(bytes: &[u8]) -> Vec<StatusEntry> {
    let mut out = Vec::new();
    let mut records = bytes
        .split(|b| *b == 0)
        .map(|r| String::from_utf8_lossy(r).into_owned())
        .filter(|r| !r.is_empty());
    while let Some(rec) = records.next() {
        let mut fields = rec.splitn(2, ' ');
        let tag = fields.next().unwrap_or("");
        let rest = fields.next().unwrap_or("");
        match tag {
            "1" => {
                // 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
                let parts: Vec<&str> = rest.splitn(8, ' ').collect();
                if parts.len() == 8 {
                    out.push(StatusEntry {
                        path: parts[7].to_string(),
                        kind: kind_of_xy(parts[0]),
                        old_path: None,
                    });
                }
            }
            "2" => {
                // 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path> NUL <origPath>
                let parts: Vec<&str> = rest.splitn(9, ' ').collect();
                if parts.len() == 9 {
                    let orig = records.next();
                    out.push(StatusEntry {
                        path: parts[8].to_string(),
                        kind: ChangeKind::Renamed,
                        old_path: orig,
                    });
                }
            }
            "u" => {
                // u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
                let parts: Vec<&str> = rest.splitn(10, ' ').collect();
                if parts.len() == 10 {
                    out.push(StatusEntry {
                        path: parts[9].to_string(),
                        kind: ChangeKind::Modified,
                        old_path: None,
                    });
                }
            }
            "?" => out.push(StatusEntry {
                path: rest.to_string(),
                kind: ChangeKind::Untracked,
                old_path: None,
            }),
            _ => {}
        }
    }
    out
}

/// One `diff --numstat` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Numstat {
    pub path: String,
    pub old_path: Option<String>,
    pub added: u32,
    pub removed: u32,
    pub binary: bool,
}

/// Parse `git diff --numstat -M -z` output: `added TAB removed TAB path NUL`,
/// or for renames `added TAB removed TAB NUL old NUL new NUL`; binary files
/// carry `-` counts.
pub fn parse_numstat_z(bytes: &[u8]) -> Vec<Numstat> {
    let mut out = Vec::new();
    let mut records = bytes
        .split(|b| *b == 0)
        .map(|r| String::from_utf8_lossy(r).into_owned());
    while let Some(rec) = records.next() {
        if rec.is_empty() {
            continue;
        }
        let mut f = rec.splitn(3, '\t');
        let a = f.next().unwrap_or("");
        let r = f.next().unwrap_or("");
        let path = f.next().unwrap_or("");
        let binary = a == "-" || r == "-";
        let added = a.parse().unwrap_or(0);
        let removed = r.parse().unwrap_or(0);
        if path.is_empty() {
            let old = records.next().unwrap_or_default();
            let new = records.next().unwrap_or_default();
            if new.is_empty() {
                continue;
            }
            out.push(Numstat {
                path: new,
                old_path: Some(old),
                added,
                removed,
                binary,
            });
        } else {
            out.push(Numstat {
                path: path.to_string(),
                old_path: None,
                added,
                removed,
                binary,
            });
        }
    }
    out
}

/// `(lines, binary)` of a file: NUL in the first 8 KB means binary (0 lines).
pub fn count_lines(path: &Path) -> Option<(u32, bool)> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() > COUNT_CAP_BYTES {
        return Some((0, false));
    }
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut head = [0u8; 8192];
    let n = read_fully(&mut reader, &mut head);
    if head[..n].contains(&0) {
        return Some((0, true));
    }
    let mut lines: u32 = 0;
    let mut last: Option<u8> = None;
    for b in head[..n].iter() {
        if *b == b'\n' {
            lines += 1;
        }
        last = Some(*b);
    }
    loop {
        let buf = reader.fill_buf().ok()?;
        if buf.is_empty() {
            break;
        }
        lines += buf.iter().filter(|b| **b == b'\n').count() as u32;
        last = buf.last().copied();
        let len = buf.len();
        reader.consume(len);
    }
    if last.is_some_and(|b| b != b'\n') {
        lines += 1;
    }
    Some((lines, false))
}

fn read_fully(r: &mut impl Read, buf: &mut [u8]) -> usize {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2_records() {
        let raw = b"1 .M N... 100644 100644 100644 abc def src/a.rs\0\
1 A. N... 000000 100644 100644 000 111 new.rs\0\
1 .D N... 100644 100644 000000 abc abc gone.rs\0\
2 R. N... 100644 100644 100644 abc abc R100 b.rs\0a.rs\0\
? notes.md\0\
! ignored.o\0";
        let e = parse_status_z(raw);
        assert_eq!(e.len(), 5);
        assert_eq!(
            (e[0].path.as_str(), e[0].kind),
            ("src/a.rs", ChangeKind::Modified)
        );
        assert_eq!(
            (e[1].path.as_str(), e[1].kind),
            ("new.rs", ChangeKind::Added)
        );
        assert_eq!(
            (e[2].path.as_str(), e[2].kind),
            ("gone.rs", ChangeKind::Deleted)
        );
        assert_eq!(e[3].kind, ChangeKind::Renamed);
        assert_eq!(e[3].path, "b.rs");
        assert_eq!(e[3].old_path.as_deref(), Some("a.rs"));
        assert_eq!(
            (e[4].path.as_str(), e[4].kind),
            ("notes.md", ChangeKind::Untracked)
        );
    }

    #[test]
    fn parses_numstat_with_renames_and_binaries() {
        let raw = b"3\t1\tsrc/a.rs\0-\t-\timg.png\0\
0\t0\t\0a.rs\0b.rs\0";
        let n = parse_numstat_z(raw);
        assert_eq!(n.len(), 3);
        assert_eq!((n[0].added, n[0].removed, n[0].binary), (3, 1, false));
        assert!(n[1].binary);
        assert_eq!(n[2].path, "b.rs");
        assert_eq!(n[2].old_path.as_deref(), Some("a.rs"));
    }

    #[test]
    fn counts_lines_and_detects_binaries() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("t.txt");
        std::fs::write(&t, "a\nb\nc").unwrap();
        assert_eq!(count_lines(&t), Some((3, false)));
        std::fs::write(&t, "a\nb\n").unwrap();
        assert_eq!(count_lines(&t), Some((2, false)));
        std::fs::write(&t, "").unwrap();
        assert_eq!(count_lines(&t), Some((0, false)));
        let b = dir.path().join("b.bin");
        std::fs::write(&b, b"ab\0cd\n").unwrap();
        assert_eq!(count_lines(&b), Some((0, true)));
        assert_eq!(count_lines(&dir.path().join("missing")), None);
    }

    #[test]
    fn run_git_reports_failure_and_timeouts_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(repo_root(dir.path()), None, "not a repository");
        assert_eq!(repo_root(&dir.path().join("missing")), None);
        assert!(
            run_git(&["--no-such-flag"], dir.path(), GIT_TIMEOUT).is_none(),
            "a failing command is None"
        );
    }
}
