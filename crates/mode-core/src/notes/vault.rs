//! The vault on disk (04 §2): the root from `settings['notes.vault_path']`,
//! the `.mode/` files (`vault.json`, `folders.json`, §2.3), the walker that
//! honors the ignored paths of §2.5, vault-relative paths with `/`
//! separators, and the file-name ↔ title rules of §2.4.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::fsio;
use crate::paths;

/// `frontmatter` version this build reads and writes (`vault.json`).
pub const FRONTMATTER_VERSION: u32 = 1;
/// The default capture folder (04 §2.2).
pub const CAPTURE_FOLDER: &str = "Unsorted";
/// Mode refuses to create longer vault-relative paths (04 §2.2).
pub const MAX_PATH_CHARS: usize = 240;
/// Longest file name stem derived from a title (04 §2.4).
pub const MAX_NAME_CHARS: usize = 120;
/// Files above this are indexed title-only (04 §2.5).
pub const MAX_INDEX_BYTES: u64 = 10 * 1024 * 1024;
/// Files above this are indexed but not versioned (09 §10).
pub const MAX_VERSIONED_BYTES: u64 = 1024 * 1024;

/// `~/Mode/Notes` (04 §16), `%USERPROFILE%\Mode\Notes` on Windows.
pub fn default_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Mode")
        .join("Notes")
}

/// Expand the setting's value (`~` = the home directory; [`paths::canon`]).
pub fn expand(setting: &str) -> PathBuf {
    let s = setting.trim();
    if s.is_empty() {
        return default_root();
    }
    paths::to_os(&paths::canon(s))
}

/// The vault root configured in `settings['notes.vault_path']`.
pub fn root_from_settings(conn: &rusqlite::Connection) -> crate::db::Result<PathBuf> {
    let v = crate::settings::get_string_in(conn, "notes.vault_path")?;
    Ok(expand(v.as_deref().unwrap_or("~/Mode/Notes")))
}

// ------------------------------------------------------------------ .mode/

pub const MODE_DIR: &str = ".mode";

/// `.mode/vault.json` (04 §2.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultJson {
    pub vault_id: String,
    pub frontmatter: u32,
    pub created_at: String,
    pub created_by_device: String,
    pub app_version: String,
}

fn write_sorted_json(path: &Path, map: &BTreeMap<String, serde_json::Value>) -> io::Result<()> {
    // A BTreeMap serializes with sorted keys whatever serde_json's features.
    let mut text = serde_json::to_string_pretty(map).map_err(io::Error::other)?;
    text.push('\n');
    if std::fs::read_to_string(path).ok().as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    fsio::atomic_write(path, text.as_bytes())
}

pub fn read_vault_json(root: &Path) -> Option<VaultJson> {
    let text = std::fs::read_to_string(root.join(MODE_DIR).join("vault.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_vault_json(root: &Path, v: &VaultJson) -> io::Result<()> {
    let value = serde_json::to_value(v).map_err(io::Error::other)?;
    let map: BTreeMap<String, serde_json::Value> = match value {
        serde_json::Value::Object(m) => m.into_iter().collect(),
        _ => BTreeMap::new(),
    };
    write_sorted_json(&root.join(MODE_DIR).join("vault.json"), &map)
}

/// Read or create `vault.json`; `app_version` is refreshed when it moved.
pub fn ensure_vault_json(root: &Path, device_id: &str, now: &str) -> io::Result<VaultJson> {
    match read_vault_json(root) {
        Some(mut v) => {
            if v.app_version != crate::VERSION && v.frontmatter <= FRONTMATTER_VERSION {
                v.app_version = crate::VERSION.to_string();
                write_vault_json(root, &v)?;
            }
            Ok(v)
        }
        None => {
            let v = VaultJson {
                vault_id: crate::repo::new_id(),
                frontmatter: FRONTMATTER_VERSION,
                created_at: now.to_string(),
                created_by_device: device_id.to_string(),
                app_version: crate::VERSION.to_string(),
            };
            write_vault_json(root, &v)?;
            Ok(v)
        }
    }
}

/// `.mode/folders.json`: folder id → vault-relative path.
pub fn read_folders_json(root: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(root.join(MODE_DIR).join("folders.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn write_folders_json(root: &Path, map: &BTreeMap<String, String>) -> io::Result<()> {
    let m: BTreeMap<String, serde_json::Value> = map
        .iter()
        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
        .collect();
    write_sorted_json(&root.join(MODE_DIR).join("folders.json"), &m)
}

// ------------------------------------------------------------------ paths

/// A vault-relative path with `/` separators (`""` for the root itself),
/// or `None` outside the vault or for a name that is not UTF-8. Compared in
/// the canonical form of [`paths`], so a watcher's spelling of the vault
/// (another drive-letter case, `\\?\` on Windows) still matches.
pub fn rel_path(root: &Path, abs: &Path) -> Option<String> {
    abs.to_str()?;
    let (root, abs) = (paths::from_os(root), paths::from_os(abs));
    if paths::same(&root, &abs) {
        return Some(String::new());
    }
    paths::relative(&root, &abs)
}

/// The absolute path of a vault-relative path.
pub fn abs_path(root: &Path, rel: &str) -> PathBuf {
    paths::to_os(&paths::join_rel(&paths::from_os(root), rel))
}

/// The parent directory of a vault-relative path (`""` = the root).
pub fn parent_dir(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

pub fn file_name(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(_, f)| f).unwrap_or(rel)
}

/// Names never indexed, written or watched (04 §2.5): dotfiles and
/// dot-directories (`.mode/`, `.obsidian/`, `.git/`, `.trash/`,
/// `.DS_Store`), editor temp files, the Windows recycle bin.
pub fn ignored_name(name: &str) -> bool {
    name.starts_with('.')
        || name.ends_with('~')
        || name.ends_with(".tmp")
        || name.ends_with(".swp")
        || name.contains(".md.tmp-")
        || name.eq_ignore_ascii_case("$RECYCLE.BIN")
        || name.eq_ignore_ascii_case("System Volume Information")
}

/// Directories at the vault root reserved for the roadmap (04 §2.2).
pub fn reserved_root_dir(name: &str) -> bool {
    name == "attachments"
}

/// Whether a vault-relative path is a note path (04 §2.2, §2.5).
pub fn is_note_path(rel: &str) -> bool {
    if !rel.ends_with(".md") || rel.len() <= 3 {
        return false;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.len() > 1 && reserved_root_dir(parts[0]) {
        return false;
    }
    parts.iter().all(|p| !p.is_empty() && !ignored_name(p))
}

/// Whether a vault-relative directory path can be a folder row.
pub fn is_folder_path(rel: &str) -> bool {
    if rel.is_empty() {
        return false;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    !reserved_root_dir(parts[0]) && parts.iter().all(|p| !p.is_empty() && !ignored_name(p))
}

// ------------------------------------------------------------------ names

/// A file name stem from a title (04 §2.4): `/ \ : * ? " < > |` and control
/// characters → `-`, whitespace collapsed, leading / trailing dots and
/// spaces trimmed, at most 120 characters; empty → `Untitled`.
pub fn stem_for_title(title: &str) -> String {
    let replaced: String = title
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c| c == '.' || c == ' ');
    let mut stem: String = trimmed.chars().take(MAX_NAME_CHARS).collect();
    stem = stem.trim_end_matches(['.', ' ']).to_string();
    if stem.is_empty() {
        "Untitled".to_string()
    } else {
        stem
    }
}

/// The title of a note without an H1: its file name without `.md`.
pub fn title_from_path(rel: &str) -> String {
    let name = file_name(rel);
    name.strip_suffix(".md").unwrap_or(name).to_string()
}

/// A free `<stem>.md` (then `<stem> 2.md`, `<stem> 3.md`, …) in vault
/// directory `dir`, compared case-insensitively (04 §2.2, §2.4). Errors when
/// the resulting path is longer than 240 characters.
pub fn unique_name(root: &Path, dir: &str, stem: &str) -> io::Result<String> {
    let taken: Vec<String> = std::fs::read_dir(abs_path(root, dir))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default();
    for n in 1..10_000 {
        let name = if n == 1 {
            format!("{stem}.md")
        } else {
            format!("{stem} {n}.md")
        };
        if taken.contains(&name.to_lowercase()) {
            continue;
        }
        let rel = if dir.is_empty() {
            name.clone()
        } else {
            format!("{dir}/{name}")
        };
        if rel.chars().count() > MAX_PATH_CHARS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("the path would be longer than {MAX_PATH_CHARS} characters"),
            ));
        }
        return Ok(rel);
    }
    Err(io::Error::other("no free file name"))
}

// ------------------------------------------------------------------ walker

/// A `.md` file found by the walker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub rel: String,
    pub size: u64,
}

/// What a full walk saw.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Walk {
    /// Vault-relative directories (the root excluded), parents before children.
    pub dirs: Vec<String>,
    pub files: Vec<FileEntry>,
    /// Diagnostics → Notes entries (symlinks skipped, unreadable directories).
    pub diagnostics: Vec<(String, String)>,
}

/// Walk the vault: directories and `.md` files that are not ignored;
/// symlinks are never followed (04 §2.2).
pub fn walk(root: &Path) -> io::Result<Walk> {
    let mut out = Walk::default();
    let meta = std::fs::metadata(root)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "the vault is not a directory",
        ));
    }
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => {
                out.diagnostics
                    .push((rel.clone(), format!("unreadable directory: {e}")));
                continue;
            }
        };
        let mut children: Vec<(PathBuf, String)> = Vec::new();
        for entry in rd.filter_map(|e| e.ok()) {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if ignored_name(&name) || (rel.is_empty() && reserved_root_dir(&name)) {
                continue;
            }
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                if name.ends_with(".md")
                    || std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir())
                {
                    out.diagnostics
                        .push((child_rel, "symlink not followed".to_string()));
                }
                continue;
            }
            if ft.is_dir() {
                children.push((entry.path(), child_rel));
            } else if ft.is_file() && is_note_path(&child_rel) {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                out.files.push(FileEntry {
                    rel: child_rel,
                    size,
                });
            }
        }
        children.sort_by(|a, b| b.1.cmp(&a.1));
        for (p, r) in children {
            out.dirs.push(r.clone());
            stack.push((p, r));
        }
    }
    out.dirs.sort();
    out.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert_eq!(stem_for_title("Re: Q4 budget?"), "Re- Q4 budget-");
        assert_eq!(stem_for_title("  ..a   b.. "), "a b");
        assert_eq!(stem_for_title(""), "Untitled");
        assert_eq!(stem_for_title(&"x".repeat(200)).chars().count(), 120);
        assert_eq!(
            title_from_path("Projects/mode/Overlay spike.md"),
            "Overlay spike"
        );
        assert!(is_note_path("Projects/a.md"));
        assert!(!is_note_path(".obsidian/a.md"));
        assert!(!is_note_path("a/.hidden.md"));
        assert!(!is_note_path("attachments/a.md"));
        assert!(!is_note_path("a.md.tmp-1-2"));
        assert!(!is_note_path("a.txt"));
        assert!(is_folder_path("Projects/mode"));
        assert!(!is_folder_path(".mode"));
        assert_eq!(parent_dir("a/b/c.md"), "a/b");
        assert_eq!(parent_dir("c.md"), "");
        assert_eq!(
            abs_path(Path::new("/v"), "a/b.md"),
            PathBuf::from("/v/a/b.md")
        );
        assert_eq!(
            rel_path(Path::new("/v"), Path::new("/v/a/b.md")).as_deref(),
            Some("a/b.md")
        );
        assert_eq!(rel_path(Path::new("/v"), Path::new("/w/a.md")), None);
        assert_eq!(rel_path(Path::new("/v"), Path::new("/vault/a.md")), None);
        assert_eq!(
            rel_path(Path::new("/v/"), Path::new("/v")).as_deref(),
            Some("")
        );
        // The setting's `~` is the home directory, in either separator.
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expand("~"), paths::to_os(&paths::from_os(&home)));
            assert_eq!(
                paths::from_os(&expand("~/Mode/Notes/")),
                paths::join_rel(&paths::from_os(&home), "Mode/Notes")
            );
        }
        assert_eq!(expand("  "), default_root());
    }

    #[test]
    fn walk_skips_ignored_and_collides_case_insensitively() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for p in [
            "a.md",
            "Todo.md",
            "x/b.md",
            "x/y/c.md",
            ".obsidian/z.md",
            ".mode/vault.json",
            "x/img.png",
            "x/b.md~",
            "attachments/n.md",
        ] {
            let path = abs_path(root, p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        let w = walk(root).unwrap();
        assert_eq!(w.dirs, vec!["x", "x/y"]);
        let files: Vec<&str> = w.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(files, vec!["Todo.md", "a.md", "x/b.md", "x/y/c.md"]);
        assert_eq!(unique_name(root, "", "todo").unwrap(), "todo 2.md");
        assert_eq!(unique_name(root, "x", "New").unwrap(), "x/New.md");
        assert!(unique_name(root, "", &"y".repeat(239)).is_err());
        let v = ensure_vault_json(root, "dev", "2026-10-18T00:00:00.000Z").unwrap();
        let text = std::fs::read_to_string(root.join(".mode/vault.json")).unwrap();
        assert!(text.ends_with("}\n"));
        assert!(
            text.find("app_version").unwrap() < text.find("vault_id").unwrap(),
            "sorted keys"
        );
        assert_eq!(
            ensure_vault_json(root, "dev2", "later").unwrap().vault_id,
            v.vault_id
        );
        let mut m = BTreeMap::new();
        m.insert("id2".to_string(), "x".to_string());
        m.insert("id1".to_string(), "x/y".to_string());
        write_folders_json(root, &m).unwrap();
        assert_eq!(read_folders_json(root), m);
    }
}
