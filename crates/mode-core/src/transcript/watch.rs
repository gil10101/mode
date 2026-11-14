//! The projects-root watcher (05 §5.2): FSEvents / inotify /
//! ReadDirectoryChangesW through `notify`, or polling for `\\wsl$` paths and
//! tests; events debounced 250 ms per file, only `.jsonl` paths forwarded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use notify::{Config, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher as _};
use tokio::sync::mpsc;

/// Per-file debounce (05 §5.2).
pub const DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchMode {
    /// The platform's native watcher.
    Native,
    /// `PollWatcher` with this interval (WSL paths, tests).
    Poll(Duration),
}

/// A running watcher; dropping it stops the native watcher and the debounce
/// thread.
pub struct Watcher {
    _inner: Box<dyn notify::Watcher + Send>,
    rx: mpsc::UnboundedReceiver<PathBuf>,
    pub root: PathBuf,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").field("root", &self.root).finish()
    }
}

fn interesting(path: &Path) -> bool {
    path.extension().is_some_and(|x| x == "jsonl")
}

impl Watcher {
    /// Watch `root` recursively. The root must exist.
    pub fn start(root: &Path, mode: WatchMode, debounce: Duration) -> notify::Result<Watcher> {
        let (raw_tx, raw_rx) = std_mpsc::channel::<PathBuf>();
        let handler = move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for p in ev.paths {
                    if interesting(&p) {
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
            .name("mode-transcript-watch".into())
            .spawn(move || debounce_loop(raw_rx, tx, debounce))
            .map_err(notify::Error::io)?;
        Ok(Watcher {
            _inner: inner,
            rx,
            root: root.to_path_buf(),
        })
    }

    /// The next debounced path, when one is ready.
    pub fn try_recv(&mut self) -> Option<PathBuf> {
        self.rx.try_recv().ok()
    }

    /// Wait for the next debounced path (`None` once the watcher is gone).
    pub async fn recv(&mut self) -> Option<PathBuf> {
        self.rx.recv().await
    }
}

/// Forward each path once per quiet period of `debounce`.
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

    #[tokio::test]
    async fn poll_watcher_reports_a_growing_transcript_once_per_burst() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        std::fs::write(&file, "{}\n").unwrap();
        // The poll watcher compares whole-second mtimes: age the file first.
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(5))
            .unwrap();
        let mut w = Watcher::start(
            dir.path(),
            WatchMode::Poll(Duration::from_millis(50)),
            Duration::from_millis(100),
        )
        .unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        for i in 0..3 {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            writeln!(f, "{{\"n\":{i}}}").unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        std::fs::write(dir.path().join("notes.md"), "x").unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), w.recv())
            .await
            .expect("a change within 5 s")
            .unwrap();
        assert_eq!(got, file);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            w.try_recv().is_none(),
            "one burst is one notification, and .md files are ignored"
        );
    }
}
