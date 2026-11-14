//! Placement geometry (02 §4, §6; 08 §6.2–§6.7) as pure functions: which
//! display the overlay lives on, where its stage window goes, how the
//! webview's frame report maps onto the screen, and where the hot zone is. No Tauri types, so every rule is unit-tested on any host.
//!
//! Units follow Tauri: positions and rects are physical pixels in the desktop
//! space Tauri reports monitors in; frame sizes are logical pixels (CSS px of
//! the webview, including the `overlay.scale` zoom) multiplied by the
//! monitor's scale factor when placed.

use serde::{Deserialize, Serialize};

use crate::platform::NotchGeometry;

/// `overlay.placement` (02 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    Notch,
    TopBar,
    RightEdge,
}

impl Placement {
    pub fn parse(s: &str) -> Option<Placement> {
        match s {
            "notch" => Some(Placement::Notch),
            "top-bar" => Some(Placement::TopBar),
            "right-edge" => Some(Placement::RightEdge),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Placement::Notch => "notch",
            Placement::TopBar => "top-bar",
            Placement::RightEdge => "right-edge",
        }
    }
}

/// The drawable ladder states (02 §2.1); `hidden` has no geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    Peek,
    Compact,
    Expanded,
}

/// Which OS's rules apply where they differ (02 §4 hot zone on macOS sits
/// in the menu bar; elsewhere at the work-area top).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    Linux,
}

impl Os {
    /// The OS this build runs on (Linux stands for every other unix).
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn centre_x(&self) -> f64 {
        self.x + self.w / 2.0
    }

    /// Half-open containment (a point on the right / bottom edge is outside).
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// A logical size (CSS px).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    pub const fn new(width: f64, height: f64) -> Size {
        Size { width, height }
    }
}

/// One display as Tauri reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// The OS display name (`overlay.display = <display id>`).
    pub id: String,
    /// Physical bounds.
    pub rect: Rect,
    /// Physical work area (excludes the taskbar, the Dock and the menu bar).
    pub work: Rect,
    pub scale: f64,
    pub primary: bool,
}

impl Monitor {
    /// The work area in logical pixels (`display.changed.work_area`).
    pub fn work_logical(&self) -> Size {
        Size::new(self.work.w / self.scale, self.work.h / self.scale)
    }
}

/// The camera housing on its monitor (physical), with its logical size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Housing {
    pub monitor: usize,
    pub rect: Rect,
    pub logical: Size,
}

/// Find the monitor an `NSScreen` notch reading belongs to (same size in
/// physical pixels) and place its housing: centered between the auxiliary
/// areas, top edge at y = 0 of that screen (08 §6.2).
pub fn housing(monitors: &[Monitor], g: &NotchGeometry) -> Option<Housing> {
    let same = |m: &Monitor| {
        (m.rect.w - g.screen_width * m.scale).abs() < 2.0
            && (m.rect.h - g.screen_height * m.scale).abs() < 2.0
    };
    let monitor = monitors
        .iter()
        .position(|m| same(m) && m.primary)
        .or_else(|| monitors.iter().position(same))?;
    let m = &monitors[monitor];
    if g.notch_height <= 0.0 || g.notch_width <= 0.0 {
        return None;
    }
    Some(Housing {
        monitor,
        rect: Rect::new(
            m.rect.x + g.left_wing_width * m.scale,
            m.rect.y,
            g.notch_width * m.scale,
            g.notch_height * m.scale,
        ),
        logical: Size::new(g.notch_width, g.notch_height),
    })
}

/// `overlay.display` (02 §6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayRule {
    Main,
    Cursor,
    Id(String),
}

impl DisplayRule {
    pub fn parse(s: &str) -> DisplayRule {
        match s {
            "" | "main" => DisplayRule::Main,
            "cursor" => DisplayRule::Cursor,
            id => DisplayRule::Id(id.to_string()),
        }
    }
}

/// Where the overlay lives right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved {
    /// The geometry in use: `notch` falls back to `top-bar` without a notch
    /// (02 §6.5 clamshell, notch-less screens); `overlay.placement` itself is
    /// never rewritten.
    pub placement: Placement,
    pub monitor: usize,
    pub housing: Option<Housing>,
}

fn main_index(monitors: &[Monitor]) -> usize {
    monitors.iter().position(|m| m.primary).unwrap_or(0)
}

fn under(monitors: &[Monitor], cursor: Option<(f64, f64)>) -> Option<usize> {
    let (x, y) = cursor?;
    monitors.iter().position(|m| m.rect.contains(x, y))
}

/// The display and the effective placement (02 §6.5): a `notch` overlay
/// stays on the notched screen, except that `overlay.display = cursor`
/// borrows `top-bar` geometry on another screen while the cursor is there;
/// without a notched screen it uses `top-bar` on the rule's display. Other
/// placements follow `overlay.display`: `main` (the primary display),
/// `cursor`, or a display id that falls back to `main` when absent. An unset
/// placement is `notch` where a notch exists, else `top-bar`.
pub fn resolve(
    setting: Option<Placement>,
    rule: &DisplayRule,
    monitors: &[Monitor],
    housing: Option<Housing>,
    cursor: Option<(f64, f64)>,
) -> Option<Resolved> {
    if monitors.is_empty() {
        return None;
    }
    let main = main_index(monitors);
    let by_rule = match rule {
        DisplayRule::Main => main,
        DisplayRule::Cursor => under(monitors, cursor).unwrap_or(main),
        DisplayRule::Id(id) => monitors.iter().position(|m| &m.id == id).unwrap_or(main),
    };
    let wanted = setting.unwrap_or(if housing.is_some() {
        Placement::Notch
    } else {
        Placement::TopBar
    });
    Some(match (wanted, housing) {
        (Placement::Notch, Some(h)) => {
            let away = matches!(rule, DisplayRule::Cursor)
                && under(monitors, cursor).is_some_and(|i| i != h.monitor);
            if away {
                Resolved {
                    placement: Placement::TopBar,
                    monitor: by_rule,
                    housing: None,
                }
            } else {
                Resolved {
                    placement: Placement::Notch,
                    monitor: h.monitor,
                    housing: Some(h),
                }
            }
        }
        (Placement::Notch, None) => Resolved {
            placement: Placement::TopBar,
            monitor: by_rule,
            housing: None,
        },
        (p, _) => Resolved {
            placement: p,
            monitor: by_rule,
            housing: None,
        },
    })
}

/// Width of one notch wing in `compact` (02 §6.1: 120–166; the webview draws
/// 166, which with the fillet makes 180 of bezel each side of the housing).
pub const NOTCH_WING: f64 = 166.0;
/// The concave fillet outside each top corner of the notch frame (the kit's
/// `.m-notch`, 02 §6.1), drawn by the webview and so part of the window.
pub const NOTCH_FILLET: f64 = 14.0;
/// The right-edge panel's offset below the work-area top (02 §6.3).
pub const RIGHT_EDGE_TOP: f64 = 12.0;
/// The tallest expanded tab with `overlay.max_rows = 6` (02 §6.4).
pub const EXPANDED_NOMINAL_HEIGHT: f64 = 356.0;

/// The frame a state draws, as the core assumes it until the webview
/// reports the real one (logical, before the `overlay.scale` zoom; the
/// provisional frame of 08 §6.2). The largest of these sizes the stage.
pub fn canvas(placement: Placement, shape: Shape, housing: Option<Size>) -> Size {
    let notch = housing.unwrap_or(Size::new(184.0, 36.0));
    match (placement, shape) {
        (Placement::Notch, Shape::Peek) => {
            Size::new(notch.width + 48.0 + 2.0 * NOTCH_FILLET, notch.height)
        }
        (Placement::Notch, Shape::Compact) => Size::new(
            notch.width + 2.0 * (NOTCH_WING + NOTCH_FILLET),
            notch.height + 4.0,
        ),
        // max(520, housing + 2 × 180), at most 640: the widest the webview draws.
        (Placement::Notch, Shape::Expanded) => Size::new(640.0, EXPANDED_NOMINAL_HEIGHT),
        (Placement::TopBar, Shape::Peek) => Size::new(160.0, 28.0),
        (Placement::TopBar, Shape::Compact) => Size::new(440.0, 36.0),
        (Placement::TopBar, Shape::Expanded) => Size::new(600.0, EXPANDED_NOMINAL_HEIGHT),
        (Placement::RightEdge, Shape::Peek) => Size::new(28.0, 64.0),
        (Placement::RightEdge, Shape::Compact) => Size::new(400.0, 36.0),
        (Placement::RightEdge, Shape::Expanded) => Size::new(360.0, EXPANDED_NOMINAL_HEIGHT),
    }
}

/// The tallest window a placement allows (logical): `notch` min(480, 60 % of
/// the screen), `top-bar` min(480, 60 % of the work area), `right-edge` 70 %
/// of the work area (02 §6.1–§6.3). Every state uses it, so an open menu
/// below a pill still fits.
pub fn max_height(placement: Placement, monitor: &Monitor) -> f64 {
    let screen_h = monitor.rect.h / monitor.scale;
    let work_h = monitor.work.h / monitor.scale;
    match placement {
        Placement::Notch => (0.6 * screen_h).min(480.0),
        Placement::TopBar => (0.6 * work_h).min(480.0),
        Placement::RightEdge => 0.7 * work_h,
    }
}

/// The stage: the overlay window's fixed size for a placement while it is
/// shown (logical, before the `overlay.scale` zoom; 02 §12, 08 §6.2). The
/// frame morphs inside it, so it holds the largest shape the placement draws:
/// `notch` the widest canvas (640, or the compact pill on a housing so wide
/// it needs more), `top-bar` 600, `right-edge` 400; all as tall as
/// [`max_height`]. The width never exceeds the work area.
pub fn stage(placement: Placement, housing: Option<Size>, monitor: &Monitor) -> Size {
    let width = match placement {
        Placement::Notch => canvas(placement, Shape::Expanded, housing)
            .width
            .max(canvas(placement, Shape::Compact, housing).width),
        Placement::TopBar => canvas(placement, Shape::Expanded, housing).width,
        Placement::RightEdge => canvas(placement, Shape::Compact, housing).width,
    };
    Size::new(
        width.min(monitor.work_logical().width),
        max_height(placement, monitor),
    )
}

/// The window rect for a logical size (02 §6, 08 §6.2–§6.4): `notch`
/// centered on the housing at the top of the screen; `top-bar` centered on the
/// work area, hanging from its top (below the menu bar on macOS, flush on
/// Windows); `right-edge` flush with the work area's right edge, 12 px below
/// its top. Kept inside the monitor (`notch`) or the work area.
pub fn window_rect(r: &Resolved, monitors: &[Monitor], size: Size) -> Option<Rect> {
    let m = monitors.get(r.monitor)?;
    let s = m.scale;
    let w = (size.width * s).min(m.work.w.max(1.0)).round();
    let h = (size.height * s).round();
    let rect = match (r.placement, r.housing) {
        (Placement::Notch, Some(hs)) => {
            let x = (hs.rect.centre_x() - w / 2.0).clamp(m.rect.x, m.rect.right() - w);
            Rect::new(x.round(), m.rect.y, w, h)
        }
        (Placement::RightEdge, _) => Rect::new(
            m.work.right() - w,
            (m.work.y + RIGHT_EDGE_TOP * s).round(),
            w,
            h,
        ),
        _ => {
            let x = (m.work.centre_x() - w / 2.0).clamp(m.work.x, m.work.right() - w);
            Rect::new(x.round(), m.work.y, w, h)
        }
    };
    Some(rect)
}

/// The drawn frame the webview reports (`overlay_measure`: a rect in CSS px
/// relative to the stage's top-left, the CSS zoom already folded in) as a
/// physical rect, clamped to the stage. An empty or non-finite report is the
/// empty rect at the stage's origin.
pub fn frame_from_css(stage: Rect, scale: f64, x: f64, y: f64, w: f64, h: f64) -> Rect {
    let finite = [x, y, w, h].iter().all(|v| v.is_finite());
    if !finite || w <= 0.0 || h <= 0.0 {
        return Rect::new(stage.x, stage.y, 0.0, 0.0);
    }
    let s = if scale > 0.0 { scale } else { 1.0 };
    let left = (stage.x + x * s).clamp(stage.x, stage.right());
    let top = (stage.y + y * s).clamp(stage.y, stage.bottom());
    let right = (stage.x + (x + w) * s).clamp(left, stage.right());
    let bottom = (stage.y + (y + h) * s).clamp(top, stage.bottom());
    Rect::new(left, top, right - left, bottom - top)
}

/// The smallest rect holding both (an empty rect adds nothing).
pub fn union(a: Rect, b: Rect) -> Rect {
    let empty = |r: &Rect| r.w <= 0.0 || r.h <= 0.0;
    match (empty(&a), empty(&b)) {
        (true, _) => b,
        (_, true) => a,
        _ => {
            let (x, y) = (a.x.min(b.x), a.y.min(b.y));
            Rect::new(
                x,
                y,
                a.right().max(b.right()) - x,
                a.bottom().max(b.bottom()) - y,
            )
        }
    }
}

/// The hot zone (02 §4): the housing plus 8 px each side at notch height;
/// a 360 × 6 strip centered at the top edge (in the menu bar on macOS, at the
/// work-area top elsewhere); a 6 px strip on the right edge from 12 px below
/// the work-area top, 240 px tall.
pub fn hot_zone(r: &Resolved, monitors: &[Monitor], os: Os) -> Option<Rect> {
    let m = monitors.get(r.monitor)?;
    let s = m.scale;
    Some(match (r.placement, r.housing) {
        (Placement::Notch, Some(h)) => {
            Rect::new(h.rect.x - 8.0 * s, h.rect.y, h.rect.w + 16.0 * s, h.rect.h)
        }
        (Placement::RightEdge, _) => Rect::new(
            m.work.right() - 6.0 * s,
            m.work.y + RIGHT_EDGE_TOP * s,
            6.0 * s,
            240.0 * s,
        ),
        _ => {
            let top = if os == Os::MacOs { m.rect.y } else { m.work.y };
            Rect::new(m.work.centre_x() - 180.0 * s, top, 360.0 * s, 6.0 * s)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(id: &str, x: f64, y: f64, w: f64, h: f64, scale: f64, primary: bool) -> Monitor {
        Monitor {
            id: id.into(),
            rect: Rect::new(x, y, w, h),
            work: Rect::new(x, y, w, h),
            scale,
            primary,
        }
    }

    /// A 14" MacBook Pro: 1512 × 982 points at 2×, 185 pt housing, 32 pt tall.
    fn macbook() -> (Monitor, NotchGeometry) {
        let mut m = mon(
            "Built-in Retina Display",
            0.0,
            0.0,
            3024.0,
            1964.0,
            2.0,
            true,
        );
        m.work = Rect::new(0.0, 64.0, 3024.0, 1900.0);
        let g = NotchGeometry {
            screen_width: 1512.0,
            screen_height: 982.0,
            notch_height: 32.0,
            notch_width: 185.0,
            left_wing_width: 663.5,
            right_wing_width: 663.5,
            menu_bar_height: 32.0,
            scale: 2.0,
        };
        (m, g)
    }

    #[test]
    fn housing_is_matched_by_physical_size_and_centred_between_the_wings() {
        let (m, g) = macbook();
        let ext = mon("LG", 3024.0, 0.0, 2560.0, 1440.0, 1.0, false);
        let monitors = vec![ext, m];
        let h = housing(&monitors, &g).expect("notch screen found");
        assert_eq!(h.monitor, 1);
        assert_eq!(h.rect, Rect::new(1327.0, 0.0, 370.0, 64.0));
        assert_eq!(h.logical, Size::new(185.0, 32.0));
        assert!((h.rect.centre_x() - 1512.0).abs() < 0.01);
        // No screen of that size: no housing.
        assert!(housing(&monitors[..1], &g).is_none());
    }

    /// A 14" MacBook Pro at "More Space": NSScreen reports 1800 × 1169 points at 2x, the
    /// auxiliary areas 791 and 788 wide (a 221 × 38 housing at x 791..1012); tao reports the
    /// monitor as CGDisplayPixelsWide/High (points) × the backing scale, 3600 × 2338.
    #[test]
    fn more_space_macbook_peek_sits_beside_the_housing() {
        let m = mon(
            "Built-in Retina Display",
            0.0,
            0.0,
            3600.0,
            2338.0,
            2.0,
            true,
        );
        let g = NotchGeometry {
            screen_width: 1800.0,
            screen_height: 1169.0,
            notch_height: 38.0,
            notch_width: 1800.0 - 791.0 - 788.0,
            left_wing_width: 791.0,
            right_wing_width: 788.0,
            menu_bar_height: 38.0,
            scale: 2.0,
        };
        let monitors = vec![m];
        let h = housing(&monitors, &g).expect("the housing is matched at More Space");
        assert_eq!(h.logical, Size::new(221.0, 38.0));
        assert_eq!(h.rect, Rect::new(1582.0, 0.0, 442.0, 76.0));
        let r = resolve(None, &DisplayRule::Main, &monitors, Some(h), None).unwrap();
        let peek = canvas(Placement::Notch, Shape::Peek, Some(h.logical));
        assert_eq!(peek, Size::new(297.0, 38.0));
        let w = window_rect(&r, &monitors, peek).unwrap();
        // Logical (753, 0, 297, 38): the right wing spans 1012..1036, beside the housing.
        assert_eq!(w, Rect::new(1506.0, 0.0, 594.0, 76.0));
        assert_eq!(
            (w.x / 2.0, w.y, w.w / 2.0, w.h / 2.0),
            (753.0, 0.0, 297.0, 38.0)
        );
    }

    #[test]
    fn notch_rect_to_window_rect_per_state() {
        let (m, g) = macbook();
        let monitors = vec![m];
        let h = housing(&monitors, &g).unwrap();
        let r = resolve(None, &DisplayRule::Main, &monitors, Some(h), None).unwrap();
        assert_eq!(r.placement, Placement::Notch);
        let peek = canvas(Placement::Notch, Shape::Peek, Some(h.logical));
        assert_eq!(
            peek,
            Size::new(261.0, 32.0),
            "housing + two 24 px wings + two 14 px fillets"
        );
        let w = window_rect(&r, &monitors, peek).unwrap();
        assert_eq!(
            w,
            Rect::new(1251.0, 0.0, 522.0, 64.0),
            "centered on the housing, y = 0"
        );
        let compact = canvas(Placement::Notch, Shape::Compact, Some(h.logical));
        assert_eq!(
            compact,
            Size::new(545.0, 36.0),
            "housing + two 166 px wings + two 14 px fillets, 4 px hang"
        );
        let expanded = canvas(Placement::Notch, Shape::Expanded, Some(h.logical));
        assert_eq!(expanded.width, 640.0, "the 640 maximum");
        let w = window_rect(&r, &monitors, expanded).unwrap();
        assert_eq!((w.x, w.y, w.w), (1512.0 - 640.0, 0.0, 1280.0));
    }

    #[test]
    fn top_bar_and_right_edge_follow_the_work_area_and_scale() {
        // Windows, 150 %, taskbar at the bottom (work area 1032 px of 1080 logical).
        let mut m = mon("\\\\.\\DISPLAY1", 0.0, 0.0, 2880.0, 1620.0, 1.5, true);
        m.work = Rect::new(0.0, 0.0, 2880.0, 1548.0);
        let monitors = vec![m];
        let top = resolve(
            Some(Placement::TopBar),
            &DisplayRule::Main,
            &monitors,
            None,
            None,
        )
        .unwrap();
        let w = window_rect(&top, &monitors, Size::new(320.0, 36.0)).unwrap();
        assert_eq!(w, Rect::new(1200.0, 0.0, 480.0, 54.0));
        let right = resolve(
            Some(Placement::RightEdge),
            &DisplayRule::Main,
            &monitors,
            None,
            None,
        )
        .unwrap();
        let w = window_rect(&right, &monitors, Size::new(360.0, 300.0)).unwrap();
        assert_eq!(w, Rect::new(2880.0 - 540.0, 18.0, 540.0, 450.0));
        // macOS top-bar hangs below the menu bar: the work area starts at 25 pt.
        let mut mac = mon("Studio Display", 0.0, 0.0, 5120.0, 2880.0, 2.0, true);
        mac.work = Rect::new(0.0, 50.0, 5120.0, 2830.0);
        let monitors = vec![mac];
        let r = resolve(
            Some(Placement::TopBar),
            &DisplayRule::Main,
            &monitors,
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            window_rect(&r, &monitors, Size::new(160.0, 28.0))
                .unwrap()
                .y,
            50.0
        );
    }

    #[test]
    fn work_area_clamps_width_and_position() {
        // Taskbar docked on the left: the work area starts at x = 100.
        let mut m = mon("A", 0.0, 0.0, 800.0, 600.0, 1.0, true);
        m.work = Rect::new(100.0, 0.0, 500.0, 560.0);
        let monitors = vec![m];
        let r = resolve(
            Some(Placement::TopBar),
            &DisplayRule::Main,
            &monitors,
            None,
            None,
        )
        .unwrap();
        let w = window_rect(&r, &monitors, Size::new(600.0, 300.0)).unwrap();
        assert_eq!((w.x, w.w), (100.0, 500.0), "never wider than the work area");
        let w = window_rect(&r, &monitors, Size::new(440.0, 36.0)).unwrap();
        assert_eq!(w.x, 130.0, "centered on the work area, not the screen");
        assert_eq!(max_height(Placement::TopBar, &monitors[0]), 336.0);
        assert_eq!(max_height(Placement::RightEdge, &monitors[0]), 392.0);
        let tall = mon("B", 0.0, 0.0, 1920.0, 1200.0, 1.0, true);
        assert_eq!(max_height(Placement::Notch, &tall), 480.0);
    }

    #[test]
    fn multi_display_choice() {
        let (mac, g) = macbook();
        let ext = mon("LG", 3024.0, 0.0, 2560.0, 1440.0, 1.0, false);
        let monitors = vec![mac, ext];
        let h = housing(&monitors, &g);
        // Notch stays on the notched screen with `main`, even when the cursor is elsewhere.
        let r = resolve(
            Some(Placement::Notch),
            &DisplayRule::Main,
            &monitors,
            h,
            Some((4000.0, 10.0)),
        )
        .unwrap();
        assert_eq!((r.placement, r.monitor), (Placement::Notch, 0));
        // `cursor` borrows top-bar geometry on the external display.
        let r = resolve(
            Some(Placement::Notch),
            &DisplayRule::Cursor,
            &monitors,
            h,
            Some((4000.0, 10.0)),
        )
        .unwrap();
        assert_eq!((r.placement, r.monitor), (Placement::TopBar, 1));
        // ... and returns to the notch when the cursor is back.
        let r = resolve(
            Some(Placement::Notch),
            &DisplayRule::Cursor,
            &monitors,
            h,
            Some((100.0, 10.0)),
        )
        .unwrap();
        assert_eq!((r.placement, r.monitor), (Placement::Notch, 0));
        // Clamshell: no notch screen, top-bar on main, the setting untouched.
        let r = resolve(
            Some(Placement::Notch),
            &DisplayRule::Main,
            &monitors[1..],
            None,
            None,
        )
        .unwrap();
        assert_eq!((r.placement, r.monitor), (Placement::TopBar, 0));
        // Unset placement: notch where there is one, else top-bar.
        assert_eq!(
            resolve(None, &DisplayRule::Main, &monitors, h, None)
                .unwrap()
                .placement,
            Placement::Notch
        );
        assert_eq!(
            resolve(None, &DisplayRule::Main, &monitors, None, None)
                .unwrap()
                .placement,
            Placement::TopBar
        );
        // A fixed display id, falling back to main when it is gone.
        let r = resolve(
            Some(Placement::RightEdge),
            &DisplayRule::Id("LG".into()),
            &monitors,
            h,
            None,
        )
        .unwrap();
        assert_eq!(r.monitor, 1);
        let r = resolve(
            Some(Placement::RightEdge),
            &DisplayRule::Id("gone".into()),
            &monitors,
            h,
            None,
        )
        .unwrap();
        assert_eq!(r.monitor, 0);
        assert!(resolve(None, &DisplayRule::Main, &[], None, None).is_none());
        assert_eq!(DisplayRule::parse("cursor"), DisplayRule::Cursor);
        assert_eq!(DisplayRule::parse("main"), DisplayRule::Main);
        assert_eq!(DisplayRule::parse("LG"), DisplayRule::Id("LG".into()));
    }

    #[test]
    fn the_stage_holds_the_largest_shape_of_each_placement() {
        // This Mac at More Space: 1800 x 1169 points at 2x, a 221 x 38 housing at x 791..1012.
        let m = mon("Built-in", 0.0, 0.0, 3600.0, 2338.0, 2.0, true);
        let g = NotchGeometry {
            screen_width: 1800.0,
            screen_height: 1169.0,
            notch_height: 38.0,
            notch_width: 221.0,
            left_wing_width: 791.0,
            right_wing_width: 788.0,
            menu_bar_height: 38.0,
            scale: 2.0,
        };
        let monitors = vec![m.clone()];
        let h = housing(&monitors, &g).unwrap();
        let r = resolve(None, &DisplayRule::Main, &monitors, Some(h), None).unwrap();
        let s = stage(Placement::Notch, Some(h.logical), &m);
        assert_eq!(
            s,
            Size::new(640.0, 480.0),
            "640 wide, min(480, 0.6 x 1169) tall"
        );
        // Centered on the housing (center 901.5 pt = 1803 px): x = 1803 - 640.
        assert_eq!(
            window_rect(&r, &monitors, s).unwrap(),
            Rect::new(1163.0, 0.0, 1280.0, 960.0)
        );
        // A housing wider than 280 pt: the compact pill sets the width.
        let wide = stage(Placement::Notch, Some(Size::new(300.0, 38.0)), &m);
        assert_eq!(wide.width, 300.0 + 2.0 * (NOTCH_WING + NOTCH_FILLET));
        assert_eq!(stage(Placement::TopBar, None, &m), Size::new(600.0, 480.0));
        let right = stage(Placement::RightEdge, None, &m);
        assert_eq!(right.width, 400.0);
        assert!((right.height - 0.7 * 1169.0).abs() < 1e-9);
        // Never wider than the work area.
        let small = mon("Small", 0.0, 0.0, 500.0, 400.0, 1.0, true);
        assert_eq!(stage(Placement::TopBar, None, &small).width, 500.0);
    }

    #[test]
    fn frame_reports_convert_to_physical_and_clamp_to_the_stage() {
        let stage = Rect::new(1163.0, 0.0, 1280.0, 960.0);
        // A 545 x 42 compact frame centered in the 640 pt stage.
        let f = frame_from_css(stage, 2.0, 47.5, 0.0, 545.0, 42.0);
        assert_eq!(f, Rect::new(1258.0, 0.0, 1090.0, 84.0));
        // A menu hanging past the stage's bottom is cut at the stage.
        let f = frame_from_css(stage, 2.0, 100.0, 300.0, 200.0, 400.0);
        assert_eq!(f, Rect::new(1363.0, 600.0, 400.0, 360.0));
        // Left of the stage: clamped to its left edge.
        let f = frame_from_css(stage, 2.0, -20.0, 0.0, 40.0, 10.0);
        assert_eq!(f, Rect::new(1163.0, 0.0, 40.0, 20.0));
        // Nothing drawn.
        let f = frame_from_css(stage, 2.0, 10.0, 10.0, 0.0, 10.0);
        assert_eq!((f.w, f.h), (0.0, 0.0));
        assert_eq!(frame_from_css(stage, 2.0, f64::NAN, 0.0, 5.0, 5.0).w, 0.0);
    }

    #[test]
    fn union_holds_both_rects() {
        let a = Rect::new(100.0, 0.0, 200.0, 40.0);
        let b = Rect::new(50.0, 0.0, 300.0, 300.0);
        assert_eq!(union(a, b), Rect::new(50.0, 0.0, 300.0, 300.0));
        assert_eq!(
            union(a, Rect::new(400.0, 10.0, 10.0, 10.0)),
            Rect::new(100.0, 0.0, 310.0, 40.0)
        );
        assert_eq!(union(Rect::default(), a), a);
        assert_eq!(union(a, Rect::default()), a);
    }

    #[test]
    fn hot_zones_per_placement_and_os() {
        let (mac, g) = macbook();
        let monitors = vec![mac.clone()];
        let h = housing(&monitors, &g);
        let r = resolve(None, &DisplayRule::Main, &monitors, h, None).unwrap();
        let z = hot_zone(&r, &monitors, Os::MacOs).unwrap();
        assert_eq!(
            z,
            Rect::new(1311.0, 0.0, 402.0, 64.0),
            "housing + 8 pt each side"
        );
        assert!(z.contains(1512.0, 10.0));
        assert!(
            !z.contains(200.0, 10.0),
            "the menu bar outside the housing is not hot"
        );

        let top = resolve(
            Some(Placement::TopBar),
            &DisplayRule::Main,
            &monitors,
            None,
            None,
        )
        .unwrap();
        let z = hot_zone(&top, &monitors, Os::MacOs).unwrap();
        assert_eq!(
            z,
            Rect::new(1152.0, 0.0, 720.0, 12.0),
            "in the menu bar's center on macOS"
        );
        let z = hot_zone(&top, &monitors, Os::Windows).unwrap();
        assert_eq!(z.y, 64.0, "at the work-area top elsewhere");

        let right = resolve(
            Some(Placement::RightEdge),
            &DisplayRule::Main,
            &monitors,
            None,
            None,
        )
        .unwrap();
        let z = hot_zone(&right, &monitors, Os::Linux).unwrap();
        assert_eq!(z, Rect::new(3012.0, 88.0, 12.0, 480.0));
        assert!(z.contains(3023.0, 100.0));
        assert!(!z.contains(3024.0, 100.0), "half-open on the right edge");
    }
}
