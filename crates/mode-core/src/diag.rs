//! `tracing` setup and the Diagnostics data (08 §12): a stderr subscriber
//! with `MODE_LOG` / `RUST_LOG` filtering, plus (when the app names a log
//! folder) a size-rotated file `logs/mode.log` in the plain
//! `<RFC 3339>  LEVEL target: message` format the Logs view parses (03
//! §8.17; the target names the component of 08 §12); a panic hook that logs
//! and writes `logs/panic-<ts>.log`; the bounded tail the Logs view reads;
//! and the status snapshot `core_status` returns.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once, OnceLock};

use serde::Serialize;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

static INIT: Once = Once::new();
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The log file inside the log folder.
pub const LOG_FILE: &str = "mode.log";
/// Files kept: `mode.log` plus `mode.log.1` … `mode.log.4`.
pub const LOG_FILES_KEPT: usize = 5;
/// `mode.log` rotates when the next record would take it past this size.
pub const LOG_FILE_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// The most lines [`tail`] returns.
pub const TAIL_MAX_LINES: usize = 10_000;
/// The most bytes [`tail`] reads from the end of the files.
const TAIL_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// `<app_data_dir>/logs`.
pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// The live log file, once [`init_tracing_to`] opened one.
pub fn current_log_file() -> Option<PathBuf> {
    LOG_DIR.get().map(|d| d.join(LOG_FILE))
}

/// Install the global subscriber once, stderr only. Returns `true` when this
/// call installed it (later calls, and a subscriber set elsewhere, return
/// `false`).
pub fn init_tracing() -> bool {
    init_tracing_to(None)
}

/// Install the global subscriber once: stderr, plus the rotated file in
/// `log_dir` when given (a folder that cannot be created leaves stderr only,
/// with a warning).
pub fn init_tracing_to(log_dir: Option<&Path>) -> bool {
    let mut installed = false;
    INIT.call_once(|| {
        let filter = EnvFilter::try_from_env("MODE_LOG")
            .or_else(|_| EnvFilter::try_from_default_env())
            .unwrap_or_else(|_| EnvFilter::new("info"));
        let mut file_error = None;
        let file = match log_dir.map(RollingFile::open) {
            Some(Ok(f)) => Some(f),
            Some(Err(e)) => {
                file_error = Some(e.to_string());
                None
            }
            None => None,
        };
        let file_dir = file.as_ref().map(|f| f.dir.clone());
        let stderr = tracing_subscriber::fmt::layer()
            .with_target(true)
            .with_writer(io::stderr);
        let file_layer = file.map(|f| {
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_ansi(false)
                .with_writer(Mutex::new(f))
        });
        installed = tracing_subscriber::registry()
            .with(filter)
            .with(stderr)
            .with(file_layer)
            .try_init()
            .is_ok();
        if installed {
            if let Some(d) = file_dir {
                let _ = LOG_DIR.set(d);
                tracing::info!(target: "mode_core::diag", file = %LOG_DIR.get().map(|d| d.join(LOG_FILE).display().to_string()).unwrap_or_default(), "logging to file");
            }
            if let Some(e) = file_error {
                tracing::warn!(target: "mode_core::diag", error = %e, "no log file; logging to stderr only");
            }
            install_panic_hook();
        }
    });
    installed
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!(target: "mode::panic", "{info}");
        if let Some(dir) = LOG_DIR.get() {
            let _ = write_panic_file(dir, &info.to_string());
        }
        previous(info);
    }));
}

/// `logs/panic-<ts>.log` with the message and a backtrace (08 §2.6 "Crash").
pub fn write_panic_file(dir: &Path, message: &str) -> io::Result<PathBuf> {
    let stamp: String = crate::db::now_rfc3339()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let path = dir.join(format!("panic-{stamp}.log"));
    let bt = std::backtrace::Backtrace::force_capture();
    std::fs::write(
        &path,
        format!("Mode {} panicked\n{message}\n\n{bt}\n", crate::VERSION),
    )?;
    Ok(path)
}

// ------------------------------------------------------------ rolling file

/// `mode.log` with size rotation: when a record would take the file past
/// `max_bytes`, `mode.log.(n-1)` → `mode.log.n` … `mode.log` → `mode.log.1`
/// and a new `mode.log` starts; `keep` files in all. A record is written
/// whole (the fmt layer hands over one formatted line per write).
#[derive(Debug)]
pub struct RollingFile {
    dir: PathBuf,
    file: Option<File>,
    size: u64,
    max_bytes: u64,
    keep: usize,
}

impl RollingFile {
    /// `dir/mode.log` with the default limits (5 files × 5 MB).
    pub fn open(dir: &Path) -> io::Result<RollingFile> {
        Self::with_limits(dir, LOG_FILE_MAX_BYTES, LOG_FILES_KEPT)
    }

    pub fn with_limits(dir: &Path, max_bytes: u64, keep: usize) -> io::Result<RollingFile> {
        std::fs::create_dir_all(dir)?;
        let mut f = RollingFile {
            dir: dir.to_path_buf(),
            file: None,
            size: 0,
            max_bytes: max_bytes.max(1),
            keep: keep.max(1),
        };
        f.reopen()?;
        Ok(f)
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(LOG_FILE)
    }

    fn numbered(&self, n: usize) -> PathBuf {
        self.dir.join(format!("{LOG_FILE}.{n}"))
    }

    fn reopen(&mut self) -> io::Result<()> {
        let path = self.path();
        let mut opts = OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&path)?;
        self.size = file.metadata().map(|m| m.len()).unwrap_or(0);
        self.file = Some(file);
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file = None;
        let last = self.keep - 1;
        if last == 0 {
            let _ = std::fs::remove_file(self.path());
        } else {
            let _ = std::fs::remove_file(self.numbered(last));
            for n in (1..last).rev() {
                let from = self.numbered(n);
                if from.exists() {
                    std::fs::rename(&from, self.numbered(n + 1))?;
                }
            }
            if self.path().exists() {
                std::fs::rename(self.path(), self.numbered(1))?;
            }
        }
        self.reopen()
    }
}

impl Write for RollingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.size > 0 && self.size + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        if self.file.is_none() {
            self.reopen()?;
        }
        let f = self.file.as_mut().expect("reopened");
        f.write_all(buf)?;
        self.size += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

// --------------------------------------------------------------------- tail

/// The last `lines` lines of `path` (oldest first), reading at most 4 MB
/// from the end; when `path` holds fewer, the rest come from `path.1` (the
/// file rotated last). A missing file reads as no lines.
pub fn tail(path: &Path, lines: usize) -> io::Result<Vec<String>> {
    let want = lines.min(TAIL_MAX_LINES);
    if want == 0 {
        return Ok(Vec::new());
    }
    let mut out = tail_one(path, want, TAIL_MAX_BYTES)?;
    if out.len() < want {
        let mut rotated = path.as_os_str().to_owned();
        rotated.push(".1");
        let mut older = tail_one(Path::new(&rotated), want - out.len(), TAIL_MAX_BYTES)?;
        older.append(&mut out);
        out = older;
    }
    Ok(out)
}

fn tail_one(path: &Path, want: usize, max_bytes: u64) -> io::Result<Vec<String>> {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let len = f.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut all: Vec<&str> = text.lines().collect();
    if start > 0 && !all.is_empty() {
        // The first line was cut by the byte window.
        all.remove(0);
    }
    let from = all.len().saturating_sub(want);
    Ok(all[from..].iter().map(|l| l.to_string()).collect())
}

/// What the Diagnostics view and `core_status` show (03 §8.16 subset).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    pub version: &'static str,
    pub dev_build: bool,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub endpoint: String,
    pub ipc_listening: bool,
    pub hook_frames_received: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        let first = init_tracing();
        let second = init_tracing();
        assert!(!second, "the second call never installs");
        // The first call installs unless another test set a subscriber first.
        tracing::info!(first, "tracing initialized");
    }

    #[test]
    fn snapshot_serialises() {
        let s = Snapshot {
            version: crate::VERSION,
            dev_build: true,
            data_dir: "/tmp/Mode-dev".into(),
            db_path: "/tmp/Mode-dev/mode.db".into(),
            endpoint: "/tmp/mode-dev.sock".into(),
            ipc_listening: true,
            hook_frames_received: 3,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["ipc_listening"], true);
        assert_eq!(v["hook_frames_received"], 3);
    }

    #[test]
    fn the_log_file_rotates_by_size_and_keeps_n_files() {
        let dir = tempfile::tempdir().unwrap();
        let logs = log_dir(dir.path());
        let mut f = RollingFile::with_limits(&logs, 100, 3).unwrap();
        // One record per write, as the fmt layer does.
        for i in 0..20 {
            let line =
                format!("2026-10-22T10:00:00.000000Z  INFO mode_core::db: line {i:02} xxxxxxxx\n");
            f.write_all(line.as_bytes()).unwrap();
        }
        f.flush().unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&logs)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["mode.log", "mode.log.1", "mode.log.2"]);
        for n in &names {
            let len = std::fs::metadata(logs.join(n)).unwrap().len();
            assert!(len <= 100, "{n} is {len} bytes");
        }
        // The newest lines are in mode.log, whole.
        let current = std::fs::read_to_string(logs.join("mode.log")).unwrap();
        assert!(current.ends_with("line 19 xxxxxxxx\n"), "{current}");
        assert!(current.lines().all(|l| l.contains(": line ")));
        // Reopening appends to the existing file.
        drop(f);
        let mut again = RollingFile::with_limits(&logs, 100, 3).unwrap();
        assert!(again.size > 0);
        again.write_all(b"x\n").unwrap();
    }

    #[test]
    fn tail_reads_the_last_lines_across_the_rotated_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE);
        assert!(tail(&path, 10).unwrap().is_empty(), "missing file");
        std::fs::write(dir.path().join("mode.log.1"), "a\nb\nc\n").unwrap();
        std::fs::write(&path, "d\ne\n").unwrap();
        assert_eq!(tail(&path, 2).unwrap(), ["d", "e"]);
        assert_eq!(tail(&path, 4).unwrap(), ["b", "c", "d", "e"]);
        assert_eq!(tail(&path, 100).unwrap(), ["a", "b", "c", "d", "e"]);
        assert!(tail(&path, 0).unwrap().is_empty());
        // A byte window that cuts a line drops the partial line.
        let long = dir.path().join("long.log");
        std::fs::write(&long, "first line\nsecond\nthird\n").unwrap();
        assert_eq!(tail_one(&long, 10, 12).unwrap(), ["third"]);
    }

    #[test]
    fn panic_files_land_in_the_log_folder() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_panic_file(dir.path(), "boom").unwrap();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("panic-") && name.ends_with(".log"));
        assert!(std::fs::read_to_string(p).unwrap().contains("boom"));
    }
}
