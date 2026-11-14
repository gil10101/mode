//! Native glue per OS behind one trait (08 §3.4, §6.1): `linux` is the dev
//! implementation (works under xvfb), `macos` and `windows` are compiled only
//! on their target and verified in CI until they run on real hardware.
//!
//! Each OS directory splits the raw glue (`macos/appkit.rs`, `windows/win32.rs`:
//! handles in, no Tauri types) from the thin `OverlayPlatform` adapter in its
//! `mod.rs`, so the raw glue is type-checked against `objc2` / `windows` on
//! its target without the whole app.

mod geometry;
#[cfg(not(any(target_os = "macos", windows)))]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
pub(crate) mod windows;

use tauri::{AppHandle, PhysicalPosition, PhysicalSize, WebviewWindow};

pub use geometry::NotchGeometry;

/// What the OS says about the microphone before the device is opened
/// (08 §6.2, XC-37). macOS reads TCC, because CoreAudio gives a refused
/// process silence rather than an error; Windows and Linux have no answer
/// ahead of the device and report `Allowed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicAccess {
    /// Never asked: a prompt is possible.
    Undecided,
    Allowed,
    Denied,
    /// Blocked by policy (a profile, parental controls); no prompt will help.
    Restricted,
}

/// The microphone's status now, without prompting.
pub fn mic_access() -> MicAccess {
    #[cfg(target_os = "macos")]
    {
        macos::mic::access()
    }
    #[cfg(not(target_os = "macos"))]
    {
        MicAccess::Allowed
    }
}

/// Raise the OS prompt while the status is undecided and wait up to `wait`
/// for the answer: `Some(true)` allowed, `Some(false)` refused, `None` not
/// answered in time. Where there is no prompt (Windows, Linux): `Some(true)`.
pub fn request_mic(wait: std::time::Duration) -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        macos::mic::request(wait)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = wait;
        Some(true)
    }
}

use crate::overlay::geometry::Os;
use crate::overlay::pointer::Sample;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("tauri: {0}")]
    Tauri(#[from] tauri::Error),
    #[error("native: {0}")]
    Native(String),
}

/// Everything OS-specific the overlay controller needs. Methods that take a
/// window may be called from any thread (they hop to the main thread where
/// the OS requires it) unless they say otherwise.
pub trait OverlayPlatform: Send + Sync {
    fn name(&self) -> &'static str;

    /// Which OS's geometry rules apply.
    fn os(&self) -> Os;

    /// Make the overlay window a non-activating, always-visible panel
    /// (NSPanel conversion on macOS, tool-window bits and the
    /// `WM_MOUSEACTIVATE` answer on Windows).
    fn apply_overlay_style(&self, window: &WebviewWindow) -> Result<(), PlatformError>;

    /// The first screen with a notch, or `None`. Main thread only (returns
    /// `None` elsewhere); never cached across a screen change (08 §6.2).
    fn notch_geometry(&self) -> Option<NotchGeometry>;

    /// Whole-window click-through (08 §6.5).
    fn set_click_through(&self, window: &WebviewWindow, enabled: bool)
    -> Result<(), PlatformError>;

    /// Size and place the overlay window (physical pixels). The default is
    /// Tauri's `set_size` + `set_position`; Linux pins the size with min /
    /// max constraints because GTK ignores `resize` on a non-resizable window.
    fn set_frame(
        &self,
        window: &WebviewWindow,
        size: Option<PhysicalSize<u32>>,
        position: PhysicalPosition<i32>,
    ) -> Result<(), PlatformError> {
        if let Some(size) = size {
            window.set_size(size)?;
        }
        window.set_position(position)?;
        Ok(())
    }

    /// Show the overlay without activating Mode or taking key (02 §9.1):
    /// `orderFrontRegardless` on macOS, un-shrink on Windows, `show` on Linux.
    fn show(&self, window: &WebviewWindow) -> Result<(), PlatformError>;

    /// `hidden`: `orderOut` on macOS (WKWebView stops compositing), a 1 × 1
    /// click-through window on Windows (never hidden, 08 §6.3), `hide` on Linux.
    fn hide(&self, window: &WebviewWindow) -> Result<(), PlatformError>;

    /// Take (true) or release (false) key input for the capture box (02 §9.2,
    /// 08 §6.6): `makeKeyWindow` / `resignKeyWindow` on the non-activating
    /// panel; `set_activating` on Windows; focusable + focus on Linux.
    fn set_key(&self, window: &WebviewWindow, key: bool) -> Result<(), PlatformError>;

    /// One pointer sample in Tauri's physical desktop space. Called from the
    /// poller thread, never from the main thread (it may wait on it).
    fn pointer(&self, app: &AppHandle) -> Option<Sample>;

    /// The OS asks for reduced motion (`None`: unknown; the webview's
    /// `prefers-reduced-motion` still applies).
    fn reduced_motion(&self) -> Option<bool>;

    /// Presentation / exclusive full screen (02 §6.6); `None`: not detectable.
    fn presentation(&self) -> Option<bool>;

    /// Focus / Do Not Disturb as the OS reports it (07 §8.5.2); `None`:
    /// unknown here (macOS Focus is read by the core from its assertions file).
    fn focus(&self) -> Option<bool> {
        None
    }

    /// The screen is locked (07 §8.4 row 8); `None`: not detectable.
    fn screen_locked(&self) -> Option<bool> {
        None
    }

    /// The OS reported a resume from sleep since the last call (Windows
    /// `WM_POWERBROADCAST`); elsewhere the poll timer detects it.
    fn take_resumed(&self) -> bool {
        false
    }
}

/// The implementation for this build's target.
pub fn current() -> Box<dyn OverlayPlatform> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOs::default())
    }
    #[cfg(windows)]
    {
        Box::new(windows::Windows::default())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Box::new(linux::Linux)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// The AVFoundation call must answer (any status) without throwing;
    /// `request_mic` is not exercised here because it can raise the prompt.
    #[test]
    fn the_microphone_status_is_readable() {
        let status = super::mic_access();
        eprintln!("microphone status for this test run: {status:?}");
    }
}
