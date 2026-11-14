//! File I/O rules of 04 §9.3 and §12: atomic writes through a temp file in
//! the same directory (`<name>.md.tmp-<pid>-<n>`, fsync, rename; retried on
//! Windows while a sync client or antivirus holds the file), stat in the
//! 09 §4.1 timestamp format, SHA-256 content hashes, and the OS trash.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// SHA-256, lowercase hex (09 §4.1).
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What reconcile compares first (09 §8.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    /// mtime, `2026-10-18T20:17:03.123Z`.
    pub mtime: String,
    pub size: i64,
    /// Birth time where the OS reports one (macOS, Windows; Linux `statx`).
    pub birth: Option<String>,
}

fn fmt_system_time(t: std::time::SystemTime) -> String {
    crate::db::format_timestamp(time::OffsetDateTime::from(t))
}

pub fn stat(path: &Path) -> io::Result<Stat> {
    let m = std::fs::metadata(path)?;
    Ok(Stat {
        mtime: m
            .modified()
            .map(fmt_system_time)
            .unwrap_or_else(|_| crate::db::now_rfc3339()),
        size: m.len() as i64,
        birth: m.created().ok().map(fmt_system_time),
    })
}

fn tmp_path(target: &Path) -> PathBuf {
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("note.md");
    let base = if name.ends_with(".md") {
        name.to_string()
    } else {
        format!("{name}.md")
    };
    target.with_file_name(format!("{base}.tmp-{}-{n}", std::process::id()))
}

fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    let mut last = None;
    for attempt in 0..5 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if cfg!(windows) && e.kind() == io::ErrorKind::PermissionDenied => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(50 * (attempt + 1)));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("rename failed")))
}

fn write_tmp(target: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let tmp = tmp_path(target);
    let res = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(tmp)
}

fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Write `bytes` to `target` atomically: temp file in the same directory,
/// fsync, rename over the target (04 §9.3). The temp file is removed on
/// failure; the target is never left half-written.
pub fn atomic_write(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = write_tmp(target, bytes)?;
    if let Err(e) = rename_retrying(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Some(dir) = target.parent() {
        sync_dir(dir);
    }
    Ok(())
}

/// Like [`atomic_write`] for a file that must not exist yet (capture, new
/// note, conflict copy): fails with `AlreadyExists` instead of replacing.
pub fn atomic_create(target: &Path, bytes: &[u8]) -> io::Result<()> {
    if std::fs::symlink_metadata(target).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the file exists",
        ));
    }
    let tmp = write_tmp(target, bytes)?;
    // A hard link refuses an existing target; fall back to rename where
    // the volume has no hard links (FAT, some network shares).
    let res = match std::fs::hard_link(&tmp, target) {
        Ok(()) => {
            let _ = std::fs::remove_file(&tmp);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
        Err(_) => {
            if std::fs::symlink_metadata(target).is_ok() {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "the file exists",
                ))
            } else {
                rename_retrying(&tmp, target)
            }
        }
    };
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    if let Some(dir) = target.parent() {
        sync_dir(dir);
    }
    res
}

/// Move `from` to `to` (a note move or rename); refuses to replace.
pub fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the target exists",
        ));
    }
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    rename_retrying(from, to)
}

// ------------------------------------------------------------------- trash

/// Where deleted notes go (04 §12: the OS trash, never `unlink`).
pub trait Trash: Send + Sync {
    /// Move `path` to the trash.
    fn trash(&self, path: &Path) -> io::Result<()>;
    /// Put the most recent trashed copy of `original` back; `false` when
    /// the platform cannot (the caller then rewrites the saved bytes).
    fn restore(&self, original: &Path) -> io::Result<bool>;
}

/// The OS trash through the `trash` crate (macOS Trash, Windows Recycle
/// Bin, the freedesktop trash on Linux).
#[derive(Debug, Default, Clone, Copy)]
pub struct OsTrash;

impl Trash for OsTrash {
    fn trash(&self, path: &Path) -> io::Result<()> {
        trash::delete(path).map_err(|e| io::Error::other(e.to_string()))
    }

    #[cfg(any(
        windows,
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )
    ))]
    fn restore(&self, original: &Path) -> io::Result<bool> {
        let items = trash::os_limited::list().map_err(|e| io::Error::other(e.to_string()))?;
        let name = original
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let parent = original.parent().map(Path::to_path_buf);
        let latest = items
            .into_iter()
            .filter(|i| {
                Some(i.name.to_string_lossy().to_string()) == name
                    && Some(i.original_parent.clone()) == parent
            })
            .max_by_key(|i| i.time_deleted);
        match latest {
            Some(item) => trash::os_limited::restore_all([item])
                .map(|_| true)
                .map_err(|e| io::Error::other(e.to_string())),
            None => Ok(false),
        }
    }

    #[cfg(not(any(
        windows,
        all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        )
    )))]
    fn restore(&self, _original: &Path) -> io::Result<bool> {
        Ok(false)
    }
}

/// A trash directory (tests and dry runs): files are moved under it with
/// their vault-relative name and restored from there.
#[derive(Debug, Clone)]
pub struct DirTrash {
    pub dir: PathBuf,
}

impl DirTrash {
    fn slot(&self, path: &Path) -> PathBuf {
        let flat = path.to_string_lossy().replace(['/', '\\', ':'], "_");
        self.dir.join(flat)
    }
}

impl Trash for DirTrash {
    fn trash(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let slot = self.slot(path);
        if slot.exists() {
            std::fs::remove_file(&slot)?;
        }
        std::fs::rename(path, slot)
    }

    fn restore(&self, original: &Path) -> io::Result<bool> {
        let slot = self.slot(original);
        if !slot.exists() || original.exists() {
            return Ok(false);
        }
        if let Some(dir) = original.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::rename(slot, original)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_and_create() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.md");
        atomic_create(&p, b"one").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"one");
        assert_eq!(
            atomic_create(&p, b"two").unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        atomic_write(&p, b"three").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"three");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
        let s = stat(&p).unwrap();
        assert_eq!(s.size, 5);
        assert_eq!(s.mtime.len(), 24);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let t = DirTrash {
            dir: dir.path().join("trash"),
        };
        t.trash(&p).unwrap();
        assert!(!p.exists());
        assert!(t.restore(&p).unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), b"three");
    }
}
