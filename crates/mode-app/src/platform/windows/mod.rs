//! Windows adapter: hops to the window's thread and calls the raw glue in
//! `win32`. Compile-checked on `x86_64-pc-windows-msvc`; untested on Windows.

pub(crate) mod win32;

use std::sync::Mutex;

use tauri::{AppHandle, PhysicalSize, WebviewWindow};

use super::{NotchGeometry, OverlayPlatform, PlatformError};
use crate::overlay::geometry::Os;
use crate::overlay::pointer::Sample;

#[derive(Default)]
pub struct Windows {
    /// The foreground window before the capture box took key input.
    previous_foreground: Mutex<Option<isize>>,
}

fn raw(window: &WebviewWindow) -> Result<isize, PlatformError> {
    // HWND is not Send; carry the raw value to the window's thread.
    Ok(window.hwnd()?.0 as isize)
}

impl OverlayPlatform for Windows {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn os(&self) -> Os {
        Os::Windows
    }

    fn apply_overlay_style(&self, window: &WebviewWindow) -> Result<(), PlatformError> {
        let h = raw(window)?;
        window.run_on_main_thread(move || {
            // SAFETY: tao keeps the window alive for the life of the Tauri
            // window and this closure runs on its thread.
            unsafe {
                if let Err(e) = win32::make_tool_window(h) {
                    tracing::warn!(error = %e, "could not apply the tool-window style");
                }
                if !win32::install_no_activate(h) {
                    tracing::warn!("could not subclass the overlay for WM_MOUSEACTIVATE");
                }
            }
        })?;
        // Never hidden (08 §6.3: tao focuses a window on `set_visible(true)`):
        // shown once now as a click-through pixel, which is `hidden`.
        self.hide(window)?;
        window.show()?;
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
        // tao: WS_EX_TRANSPARENT | WS_EX_LAYERED, whole window (08 §6.3).
        Ok(window.set_ignore_cursor_events(enabled)?)
    }

    fn show(&self, window: &WebviewWindow) -> Result<(), PlatformError> {
        // The controller sized and placed it already; take clicks again.
        self.set_click_through(window, false)
    }

    fn hide(&self, window: &WebviewWindow) -> Result<(), PlatformError> {
        self.set_click_through(window, true)?;
        window.set_size(PhysicalSize::new(1u32, 1u32))?;
        Ok(())
    }

    fn set_key(&self, window: &WebviewWindow, key: bool) -> Result<(), PlatformError> {
        let h = raw(window)?;
        let previous = if key {
            None
        } else {
            self.previous_foreground
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take()
        };
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        window.run_on_main_thread(move || {
            // SAFETY: as in `apply_overlay_style`.
            let prev = unsafe {
                if key {
                    win32::begin_key(h)
                } else {
                    win32::end_key(h, previous);
                    None
                }
            };
            let _ = tx.send(prev);
        })?;
        if key && let Ok(prev) = rx.recv_timeout(std::time::Duration::from_millis(500)) {
            *self
                .previous_foreground
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = prev;
        }
        Ok(())
    }

    fn pointer(&self, _app: &AppHandle) -> Option<Sample> {
        let (x, y, buttons) = win32::cursor_and_buttons()?;
        Some(Sample {
            x,
            y,
            buttons,
            // A payload drag over a hidden overlay is not detectable without
            // an OLE drop target; a drag over the shown window arrives as
            // Tauri's drag-drop event.
            dragging: false,
        })
    }

    fn reduced_motion(&self) -> Option<bool> {
        win32::reduced_motion()
    }

    fn presentation(&self) -> Option<bool> {
        win32::presentation()
    }

    fn focus(&self) -> Option<bool> {
        win32::focus_assist()
    }

    fn screen_locked(&self) -> Option<bool> {
        Some(win32::screen_locked())
    }

    fn take_resumed(&self) -> bool {
        win32::take_resumed()
    }
}
