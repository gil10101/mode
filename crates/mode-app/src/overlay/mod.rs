//! The overlay window controller (08 §3.3 `mode-app::overlay`): acts on the
//! webview's `overlay_request` / `overlay_measure`, owns the placement, the
//! geometry, show / hide, the capture-box key exception and the pointer
//! poller, and pushes `display.changed`, `overlay.pointer`, `hotkey.fired`
//! and `quiet.changed` to the overlay webview.
//!
//! The decisions are pure ([`ladder`], [`geometry`], [`pointer`], [`prefs`]);
//! this module applies them to the Tauri window through the
//! [`OverlayPlatform`] glue. Window work runs on the main thread: the sync
//! commands already do, everything else goes through [`Overlay::on_main`].

pub mod geometry;
pub mod ladder;
pub mod pointer;
pub mod prefs;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

use mode_core::push::Event;
use serde::Serialize;
use tauri::{
    AppHandle, DragDropEvent, Emitter, Manager, PhysicalPosition, PhysicalSize, Theme,
    WebviewWindow, WindowEvent,
};

use crate::OVERLAY_WINDOW;
use crate::hotkeys::HotkeyKind;
use crate::platform::OverlayPlatform;
use crate::state::CoreState;
use geometry::{Housing, Monitor, Placement, Rect, Resolved, Shape, Size};
use ladder::{Action, Ladder, Poll, Request};
use pointer::{FrameTracker, Gaze, MoveThrottle, PointerKind, Tracker, Zones};
use prefs::Prefs;

/// The webview's `listen` calls resolve after a dynamic import, so its
/// launch request can reach the core before its listeners exist: what the
/// launch needs is sent again after these delays.
pub const LAUNCH_RESEND: [Duration; 2] = [Duration::from_millis(400), Duration::from_millis(1_500)];
/// A hotkey within this long before a request may open a suspended overlay.
const HOTKEY_GRACE: Duration = Duration::from_millis(1_500);

/// `display.changed` (`DisplayChanged` in the contract).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DisplayChanged {
    /// The geometry in use (a `notch` setting without a notch reads `top-bar`).
    pub placement: Placement,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notch: Option<Size>,
    /// The webview zoom, `overlay.scale` / 100 (the webview applies it as
    /// CSS `zoom`; device pixel ratios are the webview's own business).
    pub scale: f64,
    pub dark: bool,
    pub reduced_motion: bool,
    /// The home display's work area in logical pixels (02 §6 height maxima).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_area: Option<Size>,
}

/// `quiet.changed` (`QuietChanged`); only `presentation` is detected here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct QuietChanged {
    pub quiet_hours: bool,
    pub focus: bool,
    pub presentation: bool,
}

#[derive(Debug, Clone, Serialize)]
struct PointerPayload {
    kind: PointerKind,
    /// `move` only: window-relative CSS px.
    #[serde(skip_serializing_if = "Option::is_none")]
    x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    y: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct HotkeyPayload {
    hotkey: HotkeyKind,
}

/// What the poller thread reads.
#[derive(Debug, Default)]
struct PollCfg {
    mode: Poll,
    zones: Zones,
    /// Set while the overlay window is shown (the mascot's gaze).
    gaze: Option<Gaze>,
    generation: u64,
}

#[derive(Default)]
struct PollShared {
    cfg: Mutex<PollCfg>,
    wake: Condvar,
}

impl PollShared {
    fn lock(&self) -> MutexGuard<'_, PollCfg> {
        self.cfg.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, mode: Option<Poll>, zones: Zones, gaze: Option<Gaze>) {
        let mut c = self.lock();
        let changed = mode.is_some_and(|m| m != c.mode) || c.zones != zones || c.gaze != gaze;
        if let Some(m) = mode {
            c.mode = m;
        }
        c.zones = zones;
        c.gaze = gaze;
        if changed {
            c.generation += 1;
            self.wake.notify_all();
        }
    }
}

struct Inner {
    ladder: Ladder,
    prefs: Prefs,
    monitors: Vec<Monitor>,
    resolved: Option<Resolved>,
    visible: bool,
    shape: Option<Shape>,
    /// The stage: the window's physical rect (02 §12, 08 §6.2), fixed while shown.
    window: Option<Rect>,
    /// The drawn frame in CSS px relative to the stage's top-left: the
    /// provisional one after a change of state, the webview's report after,
    /// and while it morphs (`settled: false`) the union of the old and the new.
    frame_css: Option<Rect>,
    /// Counts the showings (the poller restarts its click-through state on each).
    shown: u64,
    hot: Option<Rect>,
    /// The drawn frame, physical, while shown.
    frame: Option<Rect>,
    last_display: Option<DisplayChanged>,
    last_hotkey: Option<Instant>,
    /// `overlay_request`s received (the launch pin sync is not repeated once
    /// the webview answered).
    requests: u64,
    /// Where the cursor was at the last hotkey or opening (`overlay.display = cursor`).
    cursor_hint: Option<(f64, f64)>,
    presentation: Option<bool>,
}

pub struct Overlay {
    app: AppHandle,
    platform: Arc<dyn OverlayPlatform>,
    core: Arc<CoreState>,
    inner: Mutex<Inner>,
    poll: Arc<PollShared>,
    drag_generation: AtomicU64,
}

impl Overlay {
    /// The controller for the `overlay` window, with its pointer poller.
    pub fn new(
        app: AppHandle,
        platform: Arc<dyn OverlayPlatform>,
        core: Arc<CoreState>,
        prefs: Prefs,
    ) -> Arc<Overlay> {
        let ladder = Ladder::new(prefs.pinned, prefs.fullscreen);
        let overlay = Arc::new(Overlay {
            app: app.clone(),
            platform: Arc::clone(&platform),
            core,
            inner: Mutex::new(Inner {
                ladder,
                prefs,
                monitors: Vec::new(),
                resolved: None,
                visible: false,
                shape: None,
                window: None,
                frame_css: None,
                shown: 0,
                hot: None,
                frame: None,
                last_display: None,
                last_hotkey: None,
                requests: 0,
                cursor_hint: None,
                presentation: None,
            }),
            poll: Arc::new(PollShared::default()),
            drag_generation: AtomicU64::new(0),
        });
        let poll = Arc::clone(&overlay.poll);
        let spawned = std::thread::Builder::new()
            .name("mode-overlay-pointer".into())
            .spawn(move || poller(app, platform, poll));
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "pointer poller not started; no hot zone");
        }
        overlay
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn window(&self) -> Option<WebviewWindow> {
        self.app.get_webview_window(OVERLAY_WINDOW)
    }

    /// Run `f` on the main thread (inline when already there).
    pub fn on_main(self: &Arc<Self>, f: impl FnOnce(&Overlay) + Send + 'static) {
        let me = Arc::clone(self);
        if let Err(e) = self.app.run_on_main_thread(move || f(&me)) {
            tracing::warn!(error = %e, "overlay: main thread unavailable");
        }
    }

    /// The first layout: read the displays, apply the hidden state, start
    /// hot-zone polling. Main thread.
    pub fn start(&self) {
        let mut inner = self.lock();
        self.read_displays(&mut inner);
        if let Some(w) = self.window()
            && let Err(e) = self.platform.hide(&w)
        {
            tracing::warn!(error = %e, "overlay: initial hide failed");
        }
        let plan = inner.ladder.plan();
        self.update_zones(&mut inner, Some(plan.poll));
        self.emit_display(&mut inner, true);
    }

    // ------------------------------------------------------------ requests

    /// `overlay_request` (main thread).
    pub fn request(&self, r: Request) {
        tracing::info!(
            state = r.state.as_str(),
            pinned = r.pinned,
            keyboard = r.keyboard,
            reason = %r.reason,
            "overlay_request"
        );
        let mut inner = self.lock();
        inner.requests += 1;
        let hotkey_recent = inner
            .last_hotkey
            .is_some_and(|t| t.elapsed() < HOTKEY_GRACE);
        if r.is_launch() || inner.monitors.is_empty() {
            self.read_displays(&mut inner);
        }
        let actions = inner.ladder.request(&r, hotkey_recent);
        self.run(&mut inner, actions);
        if r.is_launch() {
            // The webview (re)started: it needs the display before it draws
            // (again shortly, in case its listeners were not up yet).
            self.emit_display(&mut inner, true);
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                for delay in LAUNCH_RESEND {
                    tokio::time::sleep(delay).await;
                    if let Some(o) = app.try_state::<Arc<Overlay>>() {
                        o.on_main(|o| {
                            let mut inner = o.lock();
                            o.emit_display(&mut inner, true);
                        });
                    }
                }
            });
        }
    }

    /// Tell the webview to toggle its pin (it differs from `overlay.pinned`),
    /// once its listeners are up; repeated only if it has not answered.
    fn sync_pin_later(&self) {
        let app = self.app.clone();
        let asked_at = self.lock().requests;
        tauri::async_runtime::spawn(async move {
            for (i, delay) in LAUNCH_RESEND.into_iter().enumerate() {
                tokio::time::sleep(delay).await;
                let Some(o) = app.try_state::<Arc<Overlay>>() else {
                    return;
                };
                o.on_main(move |o| {
                    let inner = o.lock();
                    let differs = inner.ladder.pinned != inner.ladder.stored_pinned;
                    let answered = inner.requests > asked_at;
                    drop(inner);
                    if differs && (i == 0 || !answered) {
                        tracing::info!(
                            "overlay.pinned differs from the webview: sending the pin hotkey"
                        );
                        o.emit_hotkey(HotkeyKind::Pin);
                    }
                });
            }
        });
    }

    /// `overlay_measure` (main thread): the drawn frame's rect (with any open
    /// menu) in CSS px relative to the stage's top-left. `settled: false` is
    /// the target at the start of a morph: until it ends the frame is the
    /// union of the old rect and the target, so the pointer keeps the window
    /// over every part the frame passes through; `settled: true` is the rect
    /// at rest (02 §12).
    pub fn measure(&self, x: f64, y: f64, width: f64, height: f64, settled: bool) {
        let finite = [x, y, width, height].iter().all(|v| v.is_finite());
        if !finite || width <= 0.0 || height <= 0.0 {
            return;
        }
        let mut inner = self.lock();
        if inner.shape.is_none() {
            return;
        }
        let rect = Rect::new(x, y, width, height);
        inner.frame_css = Some(match inner.frame_css {
            Some(old) if !settled => geometry::union(old, rect),
            _ => rect,
        });
        tracing::debug!(x, y, width, height, settled, "overlay_measure");
        self.update_zones(&mut inner, None);
    }

    /// A global hotkey fired (main thread): remember it, tell the webview.
    pub fn hotkey(&self, kind: HotkeyKind) {
        {
            let mut inner = self.lock();
            inner.last_hotkey = Some(Instant::now());
            inner.cursor_hint = self.app.cursor_position().ok().map(|p| (p.x, p.y));
        }
        tracing::info!(hotkey = ?kind, "hotkey.fired");
        self.emit_hotkey(kind);
    }

    fn emit_hotkey(&self, kind: HotkeyKind) {
        if let Err(e) = self.app.emit_to(
            OVERLAY_WINDOW,
            Event::HotkeyFired.wire_name(),
            HotkeyPayload { hotkey: kind },
        ) {
            tracing::warn!(error = %e, "emit hotkey.fired failed");
        }
    }

    /// Presentation mode started or ended (02 §6.6). Main thread.
    pub fn set_presentation(&self, on: bool) {
        let mut inner = self.lock();
        if inner.presentation == Some(on) {
            return;
        }
        inner.presentation = Some(on);
        let actions = inner.ladder.set_presentation(on);
        self.run(&mut inner, actions);
        drop(inner);
        tracing::info!(presentation = on, "quiet.changed");
        // With the connectors service the merged state (quiet hours, pause,
        // Focus, presentation) is pushed through the publisher.
        if let Some(c) = self.core.connectors.get() {
            let mut os = c.os_state();
            os.presentation = on;
            c.set_os_state(os);
            return;
        }
        let q = QuietChanged {
            quiet_hours: false,
            focus: false,
            presentation: on,
        };
        if let Err(e) = self.app.emit(Event::Quiet.wire_name(), q) {
            tracing::warn!(error = %e, "emit quiet.changed failed");
        }
    }

    /// Settings the window reads changed (main thread).
    pub fn set_prefs(&self, prefs: Prefs) {
        let mut inner = self.lock();
        let old = std::mem::replace(&mut inner.prefs, prefs.clone());
        if old.fullscreen != prefs.fullscreen {
            let actions = inner.ladder.set_fullscreen(prefs.fullscreen);
            self.run(&mut inner, actions);
        }
        if prefs.pinned != inner.ladder.stored_pinned {
            let actions = inner.ladder.set_stored_pinned(prefs.pinned);
            self.run(&mut inner, actions);
        }
        let moved = old.placement != prefs.placement
            || old.display != prefs.display
            || old.zoom != prefs.zoom;
        if moved {
            self.read_displays(&mut inner);
            if let Some(shape) = inner.shape {
                // Another stage: the frame is the provisional one until the webview reports.
                inner.frame_css = None;
                self.layout(&mut inner, shape);
            } else {
                let plan = inner.ladder.plan();
                self.update_zones(&mut inner, Some(plan.poll));
            }
        }
        self.emit_display(&mut inner, false);
    }

    /// Re-read the displays and theme; re-anchor and push `display.changed`
    /// when something changed (08 §6.7). Main thread.
    pub fn refresh_display(&self, force: bool) {
        let mut inner = self.lock();
        let before = (inner.monitors.clone(), inner.resolved);
        self.read_displays(&mut inner);
        if (inner.monitors.clone(), inner.resolved) != before {
            tracing::info!(monitors = inner.monitors.len(), "displays changed");
            if inner.visible {
                self.place(&mut inner);
            } else {
                let plan = inner.ladder.plan();
                self.update_zones(&mut inner, Some(plan.poll));
            }
        }
        self.emit_display(&mut inner, force);
    }

    /// Whether the overlay is shown (for the presentation poll cadence).
    /// The native glue (Settings' `platform_info`).
    pub fn platform(&self) -> Arc<dyn OverlayPlatform> {
        Arc::clone(&self.platform)
    }

    pub fn is_visible(&self) -> bool {
        self.lock().visible
    }

    /// Window events of the overlay (main thread). Some OSes report a
    /// resize we applied while the controller still holds its state (AppKit
    /// delegates fire inside `setFrame:`); those events are handled on a
    /// later turn of the main loop instead of deadlocking.
    pub fn on_window_event(self: &Arc<Self>, event: &WindowEvent) {
        if matches!(self.inner.try_lock(), Err(TryLockError::WouldBlock)) {
            let me = Arc::clone(self);
            let event = event.clone();
            tauri::async_runtime::spawn(async move {
                let again = Arc::clone(&me);
                me.on_main(move |_| again.handle_window_event(&event));
            });
            return;
        }
        self.handle_window_event(event);
    }

    fn handle_window_event(&self, event: &WindowEvent) {
        match event {
            WindowEvent::Resized(size) => {
                tracing::trace!(w = size.width, h = size.height, "overlay resized");
            }
            WindowEvent::Moved(p) => {
                let ours = self.lock().window.is_some_and(|r| {
                    (r.x - f64::from(p.x)).abs() <= 1.0 && (r.y - f64::from(p.y)).abs() <= 1.0
                });
                if !ours {
                    self.refresh_display(false);
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => self.refresh_display(false),
            WindowEvent::ThemeChanged(_) => {
                let mut inner = self.lock();
                self.emit_display(&mut inner, false);
            }
            WindowEvent::DragDrop(drag) => self.on_drag(drag),
            _ => {}
        }
    }

    /// A payload dragged over the shown window for 300 ms opens the drop
    /// target (02 §8.1, §8.5).
    fn on_drag(&self, drag: &DragDropEvent) {
        match drag {
            DragDropEvent::Enter { paths, .. } => {
                let generation = self.drag_generation.fetch_add(1, Ordering::SeqCst) + 1;
                tracing::debug!(paths = paths.len(), "drag entered the overlay");
                let app = self.app.clone();
                let me = self.app.state::<Arc<Overlay>>().inner().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(pointer::DRAG_DWELL_MS)).await;
                    if me.drag_generation.load(Ordering::SeqCst) == generation {
                        emit_pointer(&app, PointerKind::DragDwell);
                    }
                });
            }
            DragDropEvent::Over { .. } => {}
            DragDropEvent::Drop { paths, .. } => {
                self.drag_generation.fetch_add(1, Ordering::SeqCst);
                tracing::info!(
                    paths = paths.len(),
                    "drop on the overlay (capture from drops lands with notes)"
                );
            }
            DragDropEvent::Leave => {
                self.drag_generation.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------- actions

    fn run(&self, inner: &mut Inner, actions: Vec<Action>) {
        let window = self.window();
        for a in actions {
            tracing::debug!(action = ?a, "overlay action");
            match a {
                Action::Layout(shape) => self.layout(inner, shape),
                Action::Show => {
                    inner.visible = true;
                    inner.shown += 1;
                    if let Some(w) = &window {
                        if let Err(e) = self.platform.show(w) {
                            tracing::warn!(error = %e, "overlay: show failed");
                        }
                        // The clear stage lets the pointer through until the poller
                        // sees it on the frame (02 §12, 08 §6.5).
                        if let Err(e) = self.platform.set_click_through(w, true) {
                            tracing::warn!(error = %e, "overlay: click-through failed");
                        }
                    }
                    tracing::info!(shape = ?inner.shape, "overlay shown");
                    self.update_zones(inner, None);
                }
                Action::Hide => {
                    inner.visible = false;
                    inner.shape = None;
                    // Windows shrinks the hidden window to a pixel: the next showing sets the stage again.
                    inner.window = None;
                    inner.frame_css = None;
                    if let Some(w) = &window
                        && let Err(e) = self.platform.hide(w)
                    {
                        tracing::warn!(error = %e, "overlay: hide failed");
                    }
                    tracing::info!("overlay hidden");
                    self.update_zones(inner, None);
                }
                Action::Key(key) => {
                    if let Some(w) = &window
                        && let Err(e) = self.platform.set_key(w, key)
                    {
                        tracing::warn!(error = %e, key, "overlay: key input change failed");
                    }
                }
                Action::Poll(mode) => self.update_zones(inner, Some(mode)),
                Action::PersistPinned(pinned) => {
                    inner.prefs.pinned = pinned;
                    let db = Arc::clone(&self.core.db);
                    tauri::async_runtime::spawn_blocking(move || {
                        let v = serde_json::Value::Bool(pinned);
                        if let Err(e) = mode_core::settings::set(&db, "overlay.pinned", &v) {
                            tracing::warn!(error = %e, "overlay.pinned not stored");
                        }
                    });
                }
                Action::SyncPin => self.sync_pin_later(),
                Action::UiReady => self.core.mark_ui_ready(),
            }
        }
    }

    /// A state to draw: record the shape, resolve the placement on a change,
    /// put the stage in place and assume the state's canvas as the frame
    /// until the webview reports it (while shown, together with the frame on
    /// screen, which the morph starts from).
    fn layout(&self, inner: &mut Inner, shape: Shape) {
        let changed = inner.shape != Some(shape);
        if changed {
            if !inner.visible
                && matches!(inner.prefs.display, geometry::DisplayRule::Cursor)
                && let Ok(p) = self.app.cursor_position()
            {
                inner.cursor_hint = Some((p.x, p.y));
            }
            self.resolve(inner);
        }
        inner.shape = Some(shape);
        let before = inner.window;
        let Some(stage) = self.stage_rect(inner) else {
            return;
        };
        if changed || inner.frame_css.is_none() || before != Some(stage) {
            let provisional = self.provisional_frame(inner, stage, shape);
            inner.frame_css = match inner.frame_css {
                Some(old) if inner.visible && before == Some(stage) => {
                    provisional.map(|p| geometry::union(old, p))
                }
                _ => provisional,
            };
        }
        self.place(inner);
    }

    /// The stage's physical rect for the resolved placement and zoom.
    fn stage_rect(&self, inner: &Inner) -> Option<Rect> {
        let r = inner.resolved?;
        let m = inner.monitors.get(r.monitor)?;
        let base = geometry::stage(r.placement, r.housing.map(|h| h.logical), m);
        let z = inner.prefs.zoom;
        let size = Size::new(base.width * z, (base.height * z).min(m.rect.h / m.scale));
        geometry::window_rect(&r, &inner.monitors, size)
    }

    /// The state's canvas anchored as the stage is, in CSS px relative to the stage.
    fn provisional_frame(&self, inner: &Inner, stage: Rect, shape: Shape) -> Option<Rect> {
        let r = inner.resolved?;
        let scale = monitor_scale(inner, &r);
        let base = geometry::canvas(r.placement, shape, r.housing.map(|h| h.logical));
        let z = inner.prefs.zoom;
        let rect = geometry::window_rect(
            &r,
            &inner.monitors,
            Size::new(base.width * z, base.height * z),
        )?;
        Some(Rect::new(
            (rect.x - stage.x) / scale,
            (rect.y - stage.y) / scale,
            rect.w / scale,
            rect.h / scale,
        ))
    }

    /// Put the window on the stage rect (one native call, only when it
    /// differs; never per frame, 02 §12) and recompute the zones.
    fn place(&self, inner: &mut Inner) {
        let Some(rect) = self.stage_rect(inner) else {
            return;
        };
        if inner.window != Some(rect)
            && let Some(w) = self.window()
        {
            self.apply_frame(&w, inner.window, rect);
            tracing::debug!(
                x = rect.x,
                y = rect.y,
                w = rect.w,
                h = rect.h,
                "overlay stage placed"
            );
        }
        inner.window = Some(rect);
        self.update_zones(inner, None);
    }

    /// Set the native frame (physical px) in one call.
    fn apply_frame(&self, w: &WebviewWindow, before: Option<Rect>, rect: Rect) {
        let resized = before.is_none_or(|o| o.w != rect.w || o.h != rect.h);
        let size = resized.then(|| PhysicalSize::new(rect.w as u32, rect.h as u32));
        let position = PhysicalPosition::new(rect.x as i32, rect.y as i32);
        if let Err(e) = self.platform.set_frame(w, size, position) {
            tracing::warn!(error = %e, "overlay: placing the window failed");
        }
    }

    /// Recompute the hot zone and the drawn frame and hand them (and maybe a
    /// new mode) to the poller.
    fn update_zones(&self, inner: &mut Inner, mode: Option<Poll>) {
        inner.hot = inner
            .resolved
            .and_then(|r| geometry::hot_zone(&r, &inner.monitors, self.platform.os()));
        let scale = inner.resolved.map_or(1.0, |r| monitor_scale(inner, &r));
        let stage = inner.window.filter(|_| inner.visible);
        inner.frame = match (stage, inner.frame_css) {
            (Some(st), Some(f)) => Some(geometry::frame_from_css(st, scale, f.x, f.y, f.w, f.h)),
            (Some(st), None) => Some(st),
            _ => None,
        };
        let gaze = match (inner.visible, inner.window, inner.resolved) {
            (true, Some(win), Some(r)) => {
                let around = r
                    .housing
                    .filter(|_| r.placement == Placement::Notch)
                    .map(|h| h.rect)
                    .or(inner.frame)
                    .unwrap_or(win);
                Some(Gaze {
                    window: win,
                    centre: (around.centre_x(), around.y + around.h / 2.0),
                    scale,
                })
            }
            _ => None,
        };
        self.poll.set(
            mode,
            Zones {
                hot: inner.hot,
                frame: inner.frame,
                stage,
                scale,
                shown: inner.shown,
            },
            gaze,
        );
    }

    // ------------------------------------------------------------- displays

    fn read_displays(&self, inner: &mut Inner) {
        let primary = self.app.primary_monitor().ok().flatten();
        let list = self.app.available_monitors().unwrap_or_default();
        inner.monitors = list
            .iter()
            .map(|m| {
                let pos = m.position();
                let size = m.size();
                let work = m.work_area();
                Monitor {
                    id: m.name().cloned().unwrap_or_default(),
                    rect: Rect::new(
                        f64::from(pos.x),
                        f64::from(pos.y),
                        f64::from(size.width),
                        f64::from(size.height),
                    ),
                    work: Rect::new(
                        f64::from(work.position.x),
                        f64::from(work.position.y),
                        f64::from(work.size.width),
                        f64::from(work.size.height),
                    ),
                    scale: m.scale_factor(),
                    primary: primary
                        .as_ref()
                        .is_some_and(|p| p.position() == pos && p.name() == m.name()),
                }
            })
            .collect();
        self.resolve(inner);
    }

    fn resolve(&self, inner: &mut Inner) {
        let housing: Option<Housing> = self
            .platform
            .notch_geometry()
            .and_then(|g| geometry::housing(&inner.monitors, &g));
        inner.resolved = geometry::resolve(
            inner.prefs.placement,
            &inner.prefs.display,
            &inner.monitors,
            housing,
            inner.cursor_hint,
        );
    }

    fn display_payload(&self, inner: &Inner) -> Option<DisplayChanged> {
        let r = inner.resolved?;
        let m = inner.monitors.get(r.monitor)?;
        let os_dark = self
            .window()
            .and_then(|w| w.theme().ok())
            .is_some_and(|t| t == Theme::Dark);
        Some(DisplayChanged {
            placement: r.placement,
            notch: r
                .housing
                .filter(|_| r.placement == Placement::Notch)
                .map(|h| h.logical),
            scale: inner.prefs.zoom,
            dark: inner.prefs.dark(os_dark),
            reduced_motion: inner.prefs.reduced_motion(self.platform.reduced_motion()),
            work_area: Some(m.work_logical()),
        })
    }

    fn emit_display(&self, inner: &mut Inner, force: bool) {
        let Some(payload) = self.display_payload(inner) else {
            return;
        };
        if !force && inner.last_display.as_ref() == Some(&payload) {
            return;
        }
        let housing = inner.resolved.and_then(|r| r.housing);
        tracing::info!(
            placement = payload.placement.as_str(),
            housing = ?housing.map(|h| (h.rect, h.logical)),
            notch = ?payload.notch,
            scale = payload.scale,
            dark = payload.dark,
            reduced_motion = payload.reduced_motion,
            "display.changed"
        );
        if let Err(e) = self
            .app
            .emit_to(OVERLAY_WINDOW, Event::Display.wire_name(), &payload)
        {
            tracing::warn!(error = %e, "emit display.changed failed");
        }
        inner.last_display = Some(payload);
    }
}

fn monitor_scale(inner: &Inner, r: &Resolved) -> f64 {
    inner.monitors.get(r.monitor).map_or(1.0, |m| m.scale)
}

fn emit_pointer(app: &AppHandle, kind: PointerKind) {
    tracing::debug!(kind = ?kind, "overlay.pointer");
    send_pointer(
        app,
        PointerPayload {
            kind,
            x: None,
            y: None,
        },
    );
}

fn send_pointer(app: &AppHandle, payload: PointerPayload) {
    if let Err(e) = app.emit_to(OVERLAY_WINDOW, Event::OverlayPointer.wire_name(), payload) {
        tracing::warn!(error = %e, "emit overlay.pointer failed");
    }
}

/// The pointer poller (02 §15, 08 §6.5): idle while polling is off and the
/// window hidden; samples at the trackers' cadence otherwise, emits what it
/// sees and, while shown, lets the stage take the pointer only over the frame.
fn poller(app: AppHandle, platform: Arc<dyn OverlayPlatform>, shared: Arc<PollShared>) {
    let start = Instant::now();
    let mut tracker = Tracker::default();
    let mut frame = FrameTracker::default();
    let mut moves = MoveThrottle::default();
    let mut seen = 0u64;
    loop {
        let (mode, zones, gaze, generation) = {
            let mut c = shared.lock();
            while c.mode == Poll::Off
                && tracker.mode() == Poll::Off
                && c.gaze.is_none()
                && c.zones.stage.is_none()
            {
                c = shared.wake.wait(c).unwrap_or_else(|e| e.into_inner());
            }
            (c.mode, c.zones, c.gaze, c.generation)
        };
        let sample = platform.pointer(&app);
        let now = start.elapsed().as_millis() as u64;
        let events = if generation != seen {
            seen = generation;
            let mut ev = tracker.set_mode(mode, &zones, sample);
            if let Some(s) = sample
                && tracker.mode() != Poll::Off
            {
                ev.extend(tracker.step(s, &zones, now));
            }
            ev
        } else {
            sample.map_or_else(Vec::new, |s| tracker.step(s, &zones, now))
        };
        let edge = frame.step(&zones, sample.as_ref());
        if let Some(through) = edge.click_through
            && let Some(w) = app.get_webview_window(OVERLAY_WINDOW)
            && let Err(e) = platform.set_click_through(&w, through)
        {
            tracing::warn!(error = %e, "overlay: click-through failed");
        }
        for kind in events.into_iter().chain(edge.edge) {
            emit_pointer(&app, kind);
        }
        if let Some(s) = sample
            && let Some((x, y)) = moves.step(gaze.as_ref(), &s, now)
        {
            send_pointer(
                &app,
                PointerPayload {
                    kind: PointerKind::Move,
                    x: Some(x),
                    y: Some(y),
                },
            );
        }
        let waits = [
            tracker.interval(),
            MoveThrottle::interval(gaze.as_ref(), sample.as_ref()),
            FrameTracker::interval(&zones, sample.as_ref()),
        ];
        let Some(wait) = waits.into_iter().flatten().min() else {
            continue;
        };
        let c = shared.lock();
        if c.generation == seen {
            let _ = shared.wake.wait_timeout(c, wait);
        }
    }
}
