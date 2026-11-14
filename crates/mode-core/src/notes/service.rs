//! The notes service: one per app. It owns the vault root, runs every
//! operation on the single writer (or a reader) with a [`Cx`], remembers
//! which notes are open in the editor (no touch-up there), coalesces editor
//! revisions to one per 60 s, keeps the bytes of the last trashes for the
//! 8 s Undo, rebuilds Diagnostics → Notes per scan, and requests
//! `notes.changed` through the publisher hook after every change (08 §4.3).
//!
//! [`Notes::run`] is the background loop: launch scan, watcher (native
//! with a polling fallback, 250 ms debounce), the 10-minute full scan, the
//! touch-up queue, and the day rollover push (04 §5.3).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::ctx::{Cx, Env, TzMode, invalid};
use super::fsio::Trash;
use super::reconcile::{self, ScanReport};
use super::vault;
use super::watcher::{self, VaultWatcher, WatchMode};
use super::write::Trashed;
use crate::db::{Db, Result};
use crate::push::Event;
use crate::queries::Clock;

/// How often the full scan runs (04 §11.1).
pub const SCAN_EVERY: Duration = Duration::from_secs(600);
/// How often the touch-up queue is looked at.
pub const TOUCHUP_EVERY: Duration = Duration::from_secs(10);
/// Editor revisions are coalesced to one per this window (04 §9.3).
pub const REVISION_WINDOW: Duration = Duration::from_secs(60);
/// How long a trashed note's bytes are kept for Undo (04 §12: 8 s, with slack).
pub const UNDO_WINDOW: Duration = Duration::from_secs(60);

/// Options for [`Notes::new`].
#[derive(Clone, Default)]
pub struct NotesOptions {
    /// The vault root; default `settings['notes.vault_path']`.
    pub root: Option<PathBuf>,
    /// A fixed clock (tests).
    pub clock: Option<Clock>,
    pub tz: TzMode,
    /// The trash; default the OS trash.
    pub trash: Option<Arc<dyn Trash>>,
}

/// The vault's health for `health.changed` and the Notes view banners.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct VaultStatus {
    pub root: PathBuf,
    /// "Vault not found" (02 §14).
    pub missing: bool,
    /// "Update Mode": `vault.json` names a newer frontmatter version (04 §2.3).
    pub read_only: Option<String>,
    pub vault_id: Option<String>,
}

type PublishFn = Arc<dyn Fn(Event) + Send + Sync>;

pub struct Notes {
    db: Arc<Db>,
    env: RwLock<Env>,
    open: Mutex<HashSet<String>>,
    last_revision: Mutex<HashMap<String, Instant>>,
    trashed: Mutex<Vec<(Instant, Trashed)>>,
    diagnostics: Mutex<BTreeMap<String, Vec<String>>>,
    status: Mutex<VaultStatus>,
    publish: Mutex<Option<PublishFn>>,
    /// Flips to `true` once [`Notes::run`] finished the launch scan.
    launched: tokio::sync::watch::Sender<bool>,
}

impl std::fmt::Debug for Notes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Notes")
            .field("status", &self.vault_status())
            .finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Notes {
    pub fn new(db: Arc<Db>, opts: NotesOptions) -> Result<Arc<Notes>> {
        let root = match opts.root {
            Some(r) => r,
            None => db.read(vault::root_from_settings)?,
        };
        let mut env = Env::new(root.clone(), db.device_id());
        env.clock = opts.clock;
        env.tz = opts.tz;
        if let Some(t) = opts.trash {
            env.trash = t;
        }
        Ok(Arc::new(Notes {
            db,
            env: RwLock::new(env),
            open: Mutex::new(HashSet::new()),
            last_revision: Mutex::new(HashMap::new()),
            trashed: Mutex::new(Vec::new()),
            diagnostics: Mutex::new(BTreeMap::new()),
            status: Mutex::new(VaultStatus {
                root,
                ..Default::default()
            }),
            publish: Mutex::new(None),
            launched: tokio::sync::watch::channel(false).0,
        }))
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    pub fn env(&self) -> Env {
        self.env.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn root(&self) -> PathBuf {
        self.env().root
    }

    /// Replace the clock (tests: a fixed `now` / `today`).
    pub fn set_clock(&self, clock: Option<Clock>) {
        self.env.write().unwrap_or_else(|p| p.into_inner()).clock = clock;
    }

    /// Where `notes.changed` requests go (`Publisher::publish` in the app).
    pub fn set_publisher(&self, f: impl Fn(Event) + Send + Sync + 'static) {
        *lock(&self.publish) = Some(Arc::new(f));
    }

    pub fn changed(&self) {
        let f = lock(&self.publish).clone();
        if let Some(f) = f {
            f(Event::Notes);
        }
    }

    /// Run `f` on the writer inside one transaction.
    pub fn write<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Cx) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let env = self.env();
        self.db.write(move |conn| {
            crate::repo::tx(conn, |tx| {
                let cx = Cx::new(tx, &env)?;
                f(&cx)
            })
        })
    }

    /// Run `f` on a reader (no writes).
    pub fn read<T>(&self, f: impl FnOnce(&Cx) -> Result<T>) -> Result<T> {
        let env = self.env();
        self.db.read(|conn| {
            let cx = Cx::new(conn, &env)?;
            f(&cx)
        })
    }

    /// Refuse writes while the vault is newer than this build (04 §2.3).
    pub fn ensure_writable(&self) -> Result<()> {
        let s = self.vault_status();
        if s.missing {
            return Err(invalid(
                "vault",
                format!("Vault not found at {}", s.root.display()),
            ));
        }
        match s.read_only {
            Some(r) => Err(invalid("vault", r)),
            None => Ok(()),
        }
    }

    pub fn vault_status(&self) -> VaultStatus {
        lock(&self.status).clone()
    }

    /// Diagnostics → Notes: `(path, message)`, rebuilt per scan.
    pub fn diagnostics(&self) -> Vec<(String, String)> {
        lock(&self.diagnostics)
            .iter()
            .flat_map(|(p, ms)| ms.iter().map(move |m| (p.clone(), m.clone())))
            .collect()
    }

    fn absorb_diagnostics(&self, report: &ScanReport, full: bool, paths: &[String]) {
        let mut d = lock(&self.diagnostics);
        if full {
            d.clear();
        } else {
            for p in paths {
                d.remove(p);
            }
        }
        for (p, m) in &report.diagnostics {
            d.entry(p.clone()).or_default().push(m.clone());
        }
    }

    // ---------------------------------------------------------- the vault

    /// Check the vault (create `.mode/`, rebuild the index for another
    /// vault), make sure the capture folder exists, then scan. A missing
    /// root is created only for a first vault: once the index is bound to a
    /// vault, its absence (moved, renamed, unmounted) is reported as
    /// "Vault not found" (02 §14) rather than replaced by an empty vault,
    /// which would drop the index and its revisions.
    pub fn init(&self) -> Result<ScanReport> {
        let root = self.root();
        if !root.exists() {
            let bound = self
                .db
                .read(|c| crate::settings::get_string_in(c, reconcile::INDEXED_VAULT_KEY))?
                .is_some();
            if bound {
                let mut s = lock(&self.status);
                s.root = root.clone();
                s.missing = true;
                return Err(invalid(
                    "vault",
                    format!("Vault not found at {}", root.display()),
                ));
            }
            if let Err(e) = std::fs::create_dir_all(&root) {
                tracing::warn!(error = %e, root = %root.display(), "vault cannot be created");
            }
        }
        let v = self.write(|cx| {
            let v = reconcile::check_vault(cx)?;
            if v.frontmatter <= vault::FRONTMATTER_VERSION {
                reconcile::capture_folder(cx)?;
            }
            Ok(v)
        });
        {
            let mut s = lock(&self.status);
            s.root = root.clone();
            match &v {
                Ok(v) => {
                    s.missing = false;
                    s.vault_id = Some(v.vault_id.clone());
                    s.read_only = (v.frontmatter > vault::FRONTMATTER_VERSION).then(|| {
                        "This vault was written by a newer Mode. Update Mode to edit it."
                            .to_string()
                    });
                }
                Err(_) => s.missing = !root.is_dir(),
            }
        }
        v?;
        self.scan()
    }

    /// A full scan (launch, every 10 minutes, Reindex).
    pub fn scan(&self) -> Result<ScanReport> {
        let root = self.root();
        let walk = match vault::walk(&root) {
            Ok(w) => w,
            Err(e) => {
                lock(&self.status).missing = true;
                return Err(e.into());
            }
        };
        lock(&self.status).missing = false;
        let snapshot = self.db.read(reconcile::row_stats)?;
        let prepared: Vec<_> = walk
            .files
            .iter()
            .map(|f| reconcile::prepare(&root, &f.rel, snapshot.get(&f.rel)))
            .collect();
        let report = self.write(move |cx| reconcile::scan(cx, &walk, &prepared))?;
        self.absorb_diagnostics(&report, true, &[]);
        if report.any_change() {
            self.changed();
        }
        Ok(report)
    }

    /// Reconcile watcher paths (absolute). Directory events run a full scan.
    pub fn reconcile_paths(&self, paths: &[PathBuf]) -> Result<ScanReport> {
        let root = self.root();
        let mut rels: Vec<String> = Vec::new();
        let mut dirs = false;
        let known_folders: HashSet<String> = self
            .db
            .read(|c| {
                let mut s = c.prepare("SELECT path FROM folders WHERE deleted_at IS NULL")?;
                let r = s.query_map([], |r| r.get::<_, String>(0))?;
                Ok(r.collect::<rusqlite::Result<HashSet<_>>>()?)
            })
            .unwrap_or_default();
        for p in paths {
            let Some(rel) = vault::rel_path(&root, p) else {
                continue;
            };
            if p.is_dir() || known_folders.contains(&rel) {
                dirs = true;
            } else if vault::is_note_path(&rel) && !rels.contains(&rel) {
                rels.push(rel);
            }
        }
        if dirs {
            return self.scan();
        }
        if rels.is_empty() {
            return Ok(ScanReport::default());
        }
        let snapshot = self.db.read(reconcile::row_stats)?;
        let prepared: Vec<_> = rels
            .iter()
            .map(|r| reconcile::prepare(&root, r, snapshot.get(r)))
            .collect();
        let report = self.write(move |cx| reconcile::reconcile_paths(cx, &prepared))?;
        self.absorb_diagnostics(&report, false, &rels);
        if report.any_change() {
            self.changed();
        }
        Ok(report)
    }

    /// One pass over the touch-up queue (04 §11.2). Returns files written.
    pub fn touch_up_pass(&self) -> Result<usize> {
        let enabled = self
            .db
            .read(|c| crate::settings::get_bool_in(c, "notes.touchup_enabled"))?
            .unwrap_or(true);
        if !enabled || self.vault_status().read_only.is_some() {
            return Ok(0);
        }
        let delay = self
            .db
            .read(|c| crate::settings::get_i64_in(c, "notes.touchup_delay_seconds"))?
            .unwrap_or(30);
        let open = lock(&self.open).clone();
        let written = self.write(move |cx| {
            let mut n = 0;
            for note in reconcile::due_touchups(cx, delay, &open)? {
                if reconcile::touch_up(cx, &note)? == reconcile::TouchUp::Written {
                    n += 1;
                }
            }
            Ok(n)
        })?;
        Ok(written)
    }

    // -------------------------------------------------------- the editor

    /// The editor opened (`true`) or closed a note: no touch-up while open.
    pub fn set_open(&self, note_id: &str, open: bool) {
        let mut o = lock(&self.open);
        if open {
            o.insert(note_id.to_string());
        } else {
            o.remove(note_id);
            lock(&self.last_revision).remove(note_id);
        }
    }

    /// Whether an editor save records a revision now (04 §9.3: at most once
    /// per 60 s of editing, always on blur / close).
    pub fn take_revision_slot(&self, note_id: &str, checkpoint: bool) -> bool {
        let mut m = lock(&self.last_revision);
        let now = Instant::now();
        let due = checkpoint
            || m.get(note_id)
                .is_none_or(|t| now.duration_since(*t) >= REVISION_WINDOW);
        if due {
            m.insert(note_id.to_string(), now);
        }
        due
    }

    pub fn remember_trash(&self, t: Trashed) {
        let mut v = lock(&self.trashed);
        v.retain(|(at, x)| at.elapsed() < UNDO_WINDOW && x.note_id != t.note_id);
        v.push((Instant::now(), t));
    }

    pub fn take_trash(&self, note_id: &str) -> Option<Trashed> {
        let mut v = lock(&self.trashed);
        let i = v.iter().position(|(_, t)| t.note_id == note_id)?;
        Some(v.remove(i).1)
    }

    // ------------------------------------------------------------- push

    /// The payload of `notes.changed` (`None` for other events), for the
    /// app's `PayloadFn`.
    pub fn payload(&self, event: Event) -> Result<Option<Value>> {
        if event != Event::Notes {
            return Ok(None);
        }
        let p = self.read(super::push::payload)?;
        Ok(Some(serde_json::to_value(p)?))
    }

    // ---------------------------------------------------------- the loop

    /// Resolves once [`Notes::run`] has finished its launch scan (whether
    /// it succeeded or found the vault missing): `vault_status` is current
    /// from then on.
    pub async fn launched(&self) {
        let mut rx = self.launched.subscribe();
        let _ = rx.wait_for(|done| *done).await;
    }

    pub fn is_launched(&self) -> bool {
        *self.launched.borrow()
    }

    /// The background loop: launch scan, watcher, periodic scan, touch-ups,
    /// day rollover. Runs until the task is dropped.
    pub async fn run(self: Arc<Self>, mode: Option<WatchMode>) {
        let me = Arc::clone(&self);
        match tokio::task::spawn_blocking(move || me.init()).await {
            Ok(Ok(r)) => tracing::info!(
                root = %self.root().display(),
                files = r.files,
                changed = r.changed.len(),
                tombstoned = r.tombstoned,
                "notes: launch scan"
            ),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, root = %self.root().display(), "notes: launch scan failed")
            }
            Err(e) => tracing::warn!(error = %e, "notes: launch scan task failed"),
        }
        self.launched.send_replace(true);
        let start = |root: &PathBuf| -> Option<VaultWatcher> {
            let w = match mode {
                Some(m) => VaultWatcher::start(root, m, watcher::DEBOUNCE),
                None => VaultWatcher::start_with_fallback(root, watcher::DEBOUNCE),
            };
            w.map_err(|e| tracing::warn!(error = %e, "notes: vault watcher not started"))
                .ok()
        };
        let mut w = start(&self.root());
        let mut scan_tick = tokio::time::interval(SCAN_EVERY);
        scan_tick.tick().await;
        let mut touch_tick = tokio::time::interval(TOUCHUP_EVERY);
        let mut day_tick = tokio::time::interval(Duration::from_secs(60));
        let mut today = self
            .read(|cx| Ok(cx.clock.today.clone()))
            .unwrap_or_default();
        loop {
            tokio::select! {
                p = async { w.as_mut().unwrap().recv().await }, if w.is_some() => {
                    match p {
                        Some(p) => {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            let mut batch = vec![p];
                            if let Some(w) = w.as_mut() {
                                batch.extend(w.drain());
                            }
                            let me = Arc::clone(&self);
                            let _ = tokio::task::spawn_blocking(move || {
                                if let Err(e) = me.reconcile_paths(&batch) {
                                    tracing::warn!(error = %e, "notes: reconcile failed");
                                }
                            }).await;
                        }
                        None => w = None,
                    }
                }
                _ = scan_tick.tick() => {
                    let root = self.root();
                    if w.as_ref().is_none_or(|x| x.root != root) {
                        w = start(&root);
                    }
                    let me = Arc::clone(&self);
                    let _ = tokio::task::spawn_blocking(move || me.scan()).await;
                }
                _ = touch_tick.tick() => {
                    let me = Arc::clone(&self);
                    let _ = tokio::task::spawn_blocking(move || me.touch_up_pass()).await;
                }
                _ = day_tick.tick() => {
                    let now = self.read(|cx| Ok(cx.clock.today.clone())).unwrap_or_default();
                    if now != today {
                        today = now;
                        self.changed();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vanished_vault_is_missing_not_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(crate::db::open(&dir.path().join("mode.db")).unwrap());
        let vault = dir.path().join("vault");
        let opts = || NotesOptions {
            root: Some(vault.clone()),
            ..Default::default()
        };
        // A first vault is created on first use.
        let notes = Notes::new(Arc::clone(&db), opts()).unwrap();
        notes.init().unwrap();
        assert!(vault.join(".mode").is_dir());
        assert!(!notes.vault_status().missing);
        std::fs::write(vault.join("Keep.md"), "Keep me\n").unwrap();
        notes.scan().unwrap();

        // Gone at the next launch: reported, not replaced, index kept.
        std::fs::rename(&vault, dir.path().join("moved")).unwrap();
        let notes = Notes::new(Arc::clone(&db), opts()).unwrap();
        assert!(notes.init().is_err());
        assert!(notes.vault_status().missing);
        assert!(!vault.exists(), "no empty vault in its place");
        let kept: i64 = db
            .read(|c| Ok(c.query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(kept, 1);
    }
}
