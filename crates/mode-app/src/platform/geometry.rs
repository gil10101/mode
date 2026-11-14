//! Geometry types shared by the platform implementations (no Tauri types, so
//! the raw glue can be type-checked on every target without the app).

use serde::Serialize;

/// The notch screen, in logical points (08 §6.2; re-read on every screen
/// change, never cached across one).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct NotchGeometry {
    pub screen_width: f64,
    pub screen_height: f64,
    /// `safeAreaInsets.top`: the camera housing height.
    pub notch_height: f64,
    /// Width of the housing between the two wings.
    pub notch_width: f64,
    pub left_wing_width: f64,
    pub right_wing_width: f64,
    pub menu_bar_height: f64,
    pub scale: f64,
}
