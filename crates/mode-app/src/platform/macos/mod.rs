//! macOS adapter: hops to the main thread and calls the raw glue in `appkit`.
//! Compile-checked on `aarch64-apple-darwin`; untested on a Mac.

mod appkit;
pub(super) mod mic;

use std::sync::Mutex;

use objc2::MainThreadMarker;
use tauri::{AppHandle, PhysicalPosition, PhysicalSize, WebviewWindow};

use super::{NotchGeometry, OverlayPlatform, PlatformError};
use crate::overlay::geometry::Os;
use crate::overlay::pointer::Sample;

#[derive(Default)]
pub struct MacOs {
    /// The drag pasteboard's change count while no button was held: a drag
    /// with a payload moves it.
    drag_baseline: Mutex<Option<isize>>,
}

/// Run raw glue on the panel on the main thread.
fn on_panel(
    window: &WebviewWindow,
    f: impl FnOnce(usize, MainThreadMarker) + Send + 'static,
) -> Result<(), PlatformError> {
    // `*mut c_void` is not Send; carry the address and rebuild it on the main thread.
    let address = window.ns_window()? as usize;
    window.run_on_main_thread(move || {
        if let Some(mtm) = MainThreadMarker::new() {
            f(address, mtm);
        }
    })?;
    Ok(())
}

impl OverlayPlatform for MacOs {
    fn name(&self) -> &'static str {
        "macos"
    }

    fn os(&self) -> Os {
        Os::MacOs
    }

    fn apply_overlay_style(&self, window: &WebviewWindow) -> Result<(), PlatformError> {
        // SAFETY (closure): tao keeps the NSWindow alive for the life of the
        // Tauri window and the closure runs on the main thread.
        on_panel(window, |address, mtm| unsafe {
            appkit::convert_to_panel(address, mtm)
        })
    }

    fn notch_geometry(&self) -> Option<NotchGeometry> {
        appkit::notch_geometry(MainThreadMarker::new()?)
    }

    fn set_click_through(
        &self,
        window: &WebviewWindow,
        enabled: bool,
    ) -> Result<(), PlatformError> {
        // tao implements this with `setIgnoresMouseEvents:` on the same NSWindow.
        Ok(window.set_ignore_cursor_events(enabled)?)
    }

    fn set_frame(
        &self,
        window: &WebviewWindow,
        size: Option<PhysicalSize<u32>>,
        position: PhysicalPosition<i32>,
    ) -> Result<(), PlatformError> {
        let size = size.map(|s| (f64::from(s.width), f64::from(s.height)));
        let (x, y) = (f64::from(position.x), f64::from(position.y));
        // SAFETY (closure): as above. One `setFrame:display:` per call (08 §6.2 "Frame motion and the stage").
        on_panel(window, move |address, mtm| unsafe {
            let (w, h) = size.unwrap_or_else(|| appkit::frame_size(address, mtm));
            appkit::set_frame(address, (x, y, w, h), mtm)
        })
    }

    fn show(&self, window: &WebviewWindow) -> Result<(), PlatformError> {
        // SAFETY (closure): as above.
        on_panel(window, |address, mtm| unsafe {
            appkit::order_front(address, mtm)
        })
    }

    fn hide(&self, window: &WebviewWindow) -> Result<(), PlatformError> {
        // SAFETY (closure): as above.
        on_panel(window, |address, mtm| unsafe {
            appkit::order_out(address, mtm)
        })
    }

    fn set_key(&self, window: &WebviewWindow, key: bool) -> Result<(), PlatformError> {
        // SAFETY (closure): as above.
        on_panel(window, move |address, mtm| unsafe {
            appkit::set_key(address, key, mtm)
        })
    }

    fn pointer(&self, app: &AppHandle) -> Option<Sample> {
        let p = app.cursor_position().ok()?;
        let buttons = appkit::mouse_buttons_down();
        let count = appkit::drag_change_count();
        let mut baseline = self.drag_baseline.lock().unwrap_or_else(|e| e.into_inner());
        let dragging = if buttons {
            baseline.is_some_and(|b| b != count)
        } else {
            *baseline = Some(count);
            false
        };
        Some(Sample {
            x: p.x,
            y: p.y,
            buttons,
            dragging,
        })
    }

    fn reduced_motion(&self) -> Option<bool> {
        Some(appkit::reduce_motion())
    }

    fn presentation(&self) -> Option<bool> {
        // The presentation heuristic (front window covering the screen at
        // the normal level, from a list of presentation apps) has no clean
        // public API (02 §6.6); not implemented, the manual "Presenting"
        // toggle of the status item will set it.
        None
    }
}
