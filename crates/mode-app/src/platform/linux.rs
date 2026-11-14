//! Linux: the dev target. Tauri's own attributes (`decorations: false`,
//! `transparent`, `alwaysOnTop`, `skipTaskbar`, `focusable: false`) are the
//! window; there is no notch. The pointer is read through GDK on the main
//! thread (position and button mask of the default seat's pointer, X11);
//! payload drags cannot be told from other drags here, so `dragging` is
//! always false (a file drag over the shown window still arrives as Tauri's
//! drag-drop event). Wayland exposes no global pointer: no hot zone there.

use std::sync::mpsc;
use std::time::Duration;

use gtk::gdk;
use gtk::prelude::*;
use tauri::{AppHandle, PhysicalPosition, PhysicalSize, WebviewWindow};

use super::{NotchGeometry, OverlayPlatform, PlatformError};
use crate::overlay::geometry::Os;
use crate::overlay::pointer::Sample;

pub struct Linux;

/// Pointer position and button state from GDK. Main thread only.
fn gdk_sample() -> Option<Sample> {
    let display = gdk::Display::default()?;
    let pointer = display.default_seat()?.pointer()?;
    let root = display.default_screen().root_window()?;
    let (_, x, y, mask) = root.device_position_double(&pointer);
    let scale = f64::from(root.scale_factor().max(1));
    let buttons = gdk::ModifierType::BUTTON1_MASK
        | gdk::ModifierType::BUTTON2_MASK
        | gdk::ModifierType::BUTTON3_MASK;
    Some(Sample {
        x: x * scale,
        y: y * scale,
        buttons: mask.intersects(buttons),
        dragging: false,
    })
}

impl OverlayPlatform for Linux {
    fn name(&self) -> &'static str {
        "linux"
    }

    fn os(&self) -> Os {
        Os::Linux
    }

    fn apply_overlay_style(&self, window: &WebviewWindow) -> Result<(), PlatformError> {
        // GTK ignores `resize` on a non-resizable window (it keeps the default
        // size); the overlay is undecorated, so this only lets Mode size it.
        window.set_resizable(true)?;
        tracing::debug!(
            label = window.label(),
            "linux overlay: resizable for the core, no other glue"
        );
        Ok(())
    }

    fn notch_geometry(&self) -> Option<NotchGeometry> {
        None
    }

    fn set_click_through(
        &self,
        window: &WebviewWindow,
        enabled: bool,
    ) -> Result<(), PlatformError> {
        Ok(window.set_ignore_cursor_events(enabled)?)
    }

    fn set_frame(
        &self,
        window: &WebviewWindow,
        size: Option<PhysicalSize<u32>>,
        position: PhysicalPosition<i32>,
    ) -> Result<(), PlatformError> {
        if let Some(size) = size {
            // tao re-applies the configured `resizable: false` on the first
            // configure event, after which GTK keeps the default size.
            if !window.is_resizable()? {
                window.set_resizable(true)?;
            }
            window.set_size(size)?;
        }
        window.set_position(position)?;
        Ok(())
    }

    fn show(&self, window: &WebviewWindow) -> Result<(), PlatformError> {
        Ok(window.show()?)
    }

    fn hide(&self, window: &WebviewWindow) -> Result<(), PlatformError> {
        Ok(window.hide()?)
    }

    fn set_key(&self, window: &WebviewWindow, key: bool) -> Result<(), PlatformError> {
        if key {
            window.set_focusable(true)?;
            window.set_focus()?;
        } else {
            window.set_focusable(false)?;
        }
        Ok(())
    }

    fn pointer(&self, app: &AppHandle) -> Option<Sample> {
        let (tx, rx) = mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let _ = tx.send(gdk_sample());
        })
        .ok()?;
        rx.recv_timeout(Duration::from_millis(250)).ok().flatten()
    }

    fn reduced_motion(&self) -> Option<bool> {
        None
    }

    fn presentation(&self) -> Option<bool> {
        None
    }
}
