//! Raw AppKit glue for the overlay (08 §6.2), written against `objc2` /
//! `objc2-app-kit`; no Tauri types. tauri-nspanel is a reference only: the
//! approach is reused, the crate is not depended on, and the style mask is
//! OR-ed, never replaced.
//!
//! Every function here must run on the main thread (the `MainThreadMarker`
//! argument proves it). Type-checked against the real crates on
//! `aarch64-apple-darwin`; untested on a real notch until the owner runs it.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSColor, NSEvent, NSMainMenuWindowLevel, NSPanel,
    NSPasteboard, NSPasteboardNameDrag, NSScreen, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};

use super::super::geometry::NotchGeometry;

define_class!(
    /// The `NSPanel` subclass the overlay's `NSWindow` is converted to. It has
    /// no ivars, so its instance layout is the `NSWindow` it replaces.
    // SAFETY:
    // - NSPanel has no subclassing requirements beyond NSWindow's.
    // - `OverlayPanel` does not implement `Drop` and has no ivars.
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "ModeOverlayPanel"]
    pub struct OverlayPanel;

    impl OverlayPanel {
        /// A non-activating panel may still take key for the capture box (08 §6.6).
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }

        /// Return the frame unchanged so the panel may sit over the menu bar / in the notch area.
        #[unsafe(method(constrainFrameRect:toScreen:))]
        fn constrain_frame_rect(&self, frame: NSRect, _screen: Option<&NSScreen>) -> NSRect {
            frame
        }
    }
);

/// Just above the menu bar (`CGWindowLevelForKey(.mainMenuWindow) + 3`).
pub fn overlay_level() -> isize {
    NSMainMenuWindowLevel + 3
}

/// Convert the `NSWindow` at `address` (from `WebviewWindow::ns_window()`) to
/// [`OverlayPanel`] and apply the 08 §6.2 table.
///
/// # Safety
/// `address` must be a live `NSWindow` owned by tao for the life of the Tauri
/// window, and the caller must be on the main thread (`mtm`).
pub unsafe fn convert_to_panel(address: usize, mtm: MainThreadMarker) {
    // SAFETY: by the caller's contract the address is a live NSWindow.
    let ns_window: &NSWindow = unsafe { &*(address as *const NSWindow) };
    let object: &AnyObject = ns_window;
    let target: &AnyClass = OverlayPanel::class();
    // SAFETY: the new class is an NSPanel subclass with no extra ivars, so the
    // instance size is unchanged (the conversion tauri-nspanel performs).
    unsafe {
        objc2::ffi::object_setClass(object as *const AnyObject as *mut AnyObject, target);
    }
    // SAFETY: the class was just set, so the object now is an OverlayPanel.
    let panel: &OverlayPanel = unsafe { &*(object as *const AnyObject).cast::<OverlayPanel>() };

    // Add `.nonactivatingPanel` onto the existing `.borderless`; never replace the mask.
    panel.setStyleMask(panel.styleMask() | NSWindowStyleMask::NonactivatingPanel);
    panel.setLevel(overlay_level());
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setHidesOnDeactivate(false);
    panel.setMovableByWindowBackground(false);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setHasShadow(false);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));

    // No Dock tile, no menu bar of its own, the panel never activates Mode.
    NSApplication::sharedApplication(mtm)
        .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

/// The first screen whose `safeAreaInsets.top > 0`, with that inset.
pub fn notch_screen(mtm: MainThreadMarker) -> Option<(Retained<NSScreen>, f64)> {
    // `NSArray::iter` hands out retained screens; keep the first notched one.
    NSScreen::screens(mtm).iter().find_map(|screen| {
        let top = screen.safeAreaInsets().top;
        (top > 0.0).then_some((screen, top))
    })
}

/// Fresh notch geometry from the OS, or `None` without a notched screen.
pub fn notch_geometry(mtm: MainThreadMarker) -> Option<NotchGeometry> {
    let (screen, notch_height) = notch_screen(mtm)?;
    let frame = screen.frame();
    let visible = screen.visibleFrame();
    let left = screen.auxiliaryTopLeftArea();
    let right = screen.auxiliaryTopRightArea();
    Some(NotchGeometry {
        screen_width: frame.size.width,
        screen_height: frame.size.height,
        notch_height,
        notch_width: frame.size.width - left.size.width - right.size.width,
        left_wing_width: left.size.width,
        right_wing_width: right.size.width,
        menu_bar_height: (frame.origin.y + frame.size.height)
            - (visible.origin.y + visible.size.height),
        scale: screen.backingScaleFactor(),
    })
}

/// The window at `address` (from `WebviewWindow::ns_window()`).
///
/// # Safety
/// `address` must be a live `NSWindow` owned by tao, used on the main thread.
unsafe fn window_at<'a>(address: usize, _mtm: MainThreadMarker) -> &'a NSWindow {
    // SAFETY: by the caller's contract.
    unsafe { &*(address as *const NSWindow) }
}

/// Show the panel without activating Mode or making it key (08 §6.2
/// "Showing": `orderFrontRegardless`, never `makeKeyAndOrderFront`).
///
/// # Safety
/// As [`convert_to_panel`].
pub unsafe fn order_front(address: usize, mtm: MainThreadMarker) {
    // SAFETY: by the caller's contract.
    unsafe { window_at(address, mtm) }.orderFrontRegardless();
}

/// Move and size the panel in one `setFrame:display:` (08 §6.2 "Frame motion and the stage"), so a
/// move of the stage changes its size and origin together, never in two composited steps.
/// `x, y, w, h` are physical px with a top-left origin, as tao's `set_position` and
/// `set_size` take them; tao converts with the window's backing scale and flips y
/// against the primary screen, and so does this.
///
/// # Safety
/// As [`convert_to_panel`].
pub unsafe fn set_frame(address: usize, rect: (f64, f64, f64, f64), mtm: MainThreadMarker) {
    // SAFETY: by the caller's contract.
    let w = unsafe { window_at(address, mtm) };
    let Some(primary) = NSScreen::screens(mtm).iter().next() else {
        return;
    };
    let scale = w.backingScaleFactor().max(1.0);
    let (x, y, width, height) = (
        rect.0 / scale,
        rect.1 / scale,
        rect.2 / scale,
        rect.3 / scale,
    );
    let top = primary.frame().size.height;
    let frame = NSRect::new(
        NSPoint::new(x, top - y - height),
        NSSize::new(width, height),
    );
    w.setFrame_display(frame, true);
}

/// The panel's frame size in physical px (for a move that keeps its size).
///
/// # Safety
/// As [`convert_to_panel`].
pub unsafe fn frame_size(address: usize, mtm: MainThreadMarker) -> (f64, f64) {
    // SAFETY: by the caller's contract.
    let w = unsafe { window_at(address, mtm) };
    let scale = w.backingScaleFactor().max(1.0);
    let f = w.frame();
    (f.size.width * scale, f.size.height * scale)
}

/// Order the panel out so WKWebView stops compositing (02 §15).
///
/// # Safety
/// As [`convert_to_panel`].
pub unsafe fn order_out(address: usize, mtm: MainThreadMarker) {
    // SAFETY: by the caller's contract.
    unsafe { window_at(address, mtm) }.orderOut(None);
}

/// Key input for the capture box (08 §6.6): the non-activating panel
/// becomes key while `NSApp` stays inactive; releasing resigns key and
/// AppKit restores the front app's key window because Mode never activated.
///
/// # Safety
/// As [`convert_to_panel`].
pub unsafe fn set_key(address: usize, key: bool, mtm: MainThreadMarker) {
    // SAFETY: by the caller's contract.
    let w = unsafe { window_at(address, mtm) };
    if key {
        w.makeKeyWindow();
    } else if w.isKeyWindow() {
        w.resignKeyWindow();
    }
}

/// Any mouse button held, system-wide (`NSEvent.pressedMouseButtons`; no
/// Accessibility permission).
pub fn mouse_buttons_down() -> bool {
    NSEvent::pressedMouseButtons() != 0
}

/// The drag pasteboard's change count: it moves when a drag with a payload
/// starts anywhere, which tells a file or text drag from a window drag.
pub fn drag_change_count() -> isize {
    // SAFETY: `NSPasteboardNameDrag` is an AppKit constant, valid for the
    // life of the process.
    let name = unsafe { NSPasteboardNameDrag };
    NSPasteboard::pasteboardWithName(name).changeCount()
}

/// `NSWorkspace.accessibilityDisplayShouldReduceMotion`.
pub fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}
