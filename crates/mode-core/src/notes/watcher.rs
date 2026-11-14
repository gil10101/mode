//! The vault watcher (04 §11.1, 08 §4.3): FSEvents / inotify /
//! ReadDirectoryChangesW through `notify`, or polling (network volumes,
//! tests), debounced 250 ms per path. Paths under ignored names (§2.5) are
//! dropped; `.md` files and extension-less paths (directories, which may be
//! gone by the time the event is read) are forwarded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use notify::{Config, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;

pub use crate::transcript::watch::WatchMode;

use super::vault;

/// Per-path debounce (04 §11.1).
pub const DEBOUNCE: Duration = Duration::from_millis(250);

/// A running vault watcher; dropping it stops the watcher and its thread.
pub struct VaultWatcher {
    _inner: Box<dyn notify::Watcher + Send>,
    rx: mpsc::UnboundedReceiver<PathBuf>,
    pub root: PathBuf,
}

impl std::fmt::Debug for VaultWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultWatcher")
            .field("root", &self.root)
            .finish()
    }
}

/// Whether an event path is worth reconciling.
pub fn interesting(root: &Path, path: &Path) -> bool {
    let Some(rel) = vault::rel_path(root, path) else {
        return false;
    };
    if rel.is_empty() {
        return false;
    }
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.iter().any(|p| vault::ignored_name(p)) || vault::reserved_root_dir(parts[0]) {
        return false;
    }
    match path.extension() {
        None => true,
        Some(x) => x == "md",
    }
}

impl VaultWatcher {
    /// Watch `root` recursively (it must exist).
    pub fn start(root: &Path, mode: WatchMode, debounce: Duration) -> notify::Result<VaultWatcher> {
        let (raw_tx, raw_rx) = std_mpsc::channel::<PathBuf>();
        let r = root.to_path_buf();
        let handler = move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for p in ev.paths {
                    if interesting(&r, &p) {
                        let _ = raw_tx.send(p);
                    }
                }
            }
        };
        let mut inner: Box<dyn notify::Watcher + Send> = match mode {
            WatchMode::Native => Box::new(RecommendedWatcher::new(handler, Config::default())?),
            WatchMode::Poll(interval) => Box::new(PollWatcher::new(
                handler,
                Config::default().with_poll_interval(interval),
            )?),
        };
        inner.watch(root, RecursiveMode::Recursive)?;
        let (tx, rx) = mpsc::unbounded_channel();
        std::thread::Builder::new()
            .name("mode-vault-watch".into())
            .spawn(move || debounce_loop(raw_rx, tx, debounce))
            .map_err(notify::Error::io)?;
        Ok(VaultWatcher {
            _inner: inner,
            rx,
            root: root.to_path_buf(),
        })
    }

    /// Native, falling back to polling every 2 s when the native watcher
    /// cannot start (inotify limits, network volumes).
    pub fn start_with_fallback(root: &Path, debounce: Duration) -> notify::Result<VaultWatcher> {
        match VaultWatcher::start(root, WatchMode::Native, debounce) {
            Ok(w) => Ok(w),
            Err(e) => {
                tracing::warn!(error = %e, "native vault watcher failed; polling");
                VaultWatcher::start(root, WatchMode::Poll(Duration::from_secs(2)), debounce)
            }
        }
    }

    pub fn try_recv(&mut self) -> Option<PathBuf> {
        self.rx.try_recv().ok()
    }

    pub async fn recv(&mut self) -> Option<PathBuf> {
        self.rx.recv().await
    }

    /// Everything ready now (a batch for one reconcile pass).
    pub fn drain(&mut self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        while let Some(p) = self.try_recv() {
            if !out.contains(&p) {
                out.push(p);
            }
        }
        out
    }
}

fn debounce_loop(
    raw: std_mpsc::Receiver<PathBuf>,
    out: mpsc::UnboundedSender<PathBuf>,
    debounce: Duration,
) {
    let mut last_seen: HashMap<PathBuf, Instant> = HashMap::new();
    loop {
        let wait = last_seen
            .values()
            .map(|t| (*t + debounce).saturating_duration_since(Instant::now()))
            .min()
            .unwrap_or(Duration::from_secs(3600));
        match raw.recv_timeout(wait) {
            Ok(p) => {
                last_seen.insert(p, Instant::now());
            }
            Err(std_mpsc::RecvTimeoutError::Timeout) => {}
            Err(std_mpsc::RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let due: Vec<PathBuf> = last_seen
            .iter()
            .filter(|(_, t)| **t + debounce <= now)
            .map(|(p, _)| p.clone())
            .collect();
        for p in due {
            last_seen.remove(&p);
            if out.send(p).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_ignored_paths() {
        let root = Path::new("/v");
        assert!(interesting(root, Path::new("/v/a.md")));
        assert!(interesting(root, Path::new("/v/Projects")));
        assert!(!interesting(root, Path::new("/v/.mode/folders.json")));
        assert!(!interesting(root, Path::new("/v/.obsidian/x.md")));
        assert!(!interesting(root, Path::new("/v/a.md.tmp-1-2")));
        assert!(!interesting(root, Path::new("/v/img.png")));
        assert!(!interesting(root, Path::new("/w/a.md")));
    }

    #[tokio::test]
    async fn poll_watcher_reports_a_note_once_per_burst() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = VaultWatcher::start(
            dir.path(),
            WatchMode::Poll(Duration::from_millis(50)),
            Duration::from_millis(100),
        )
        .unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        let file = dir.path().join("n.md");
        std::fs::write(&file, "a").unwrap();
        std::fs::write(dir.path().join("x.png"), "a").unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), w.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, file);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(w.drain().is_empty(), "one burst, one path; .png ignored");
    }
}
