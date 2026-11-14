//! Pointer events only the core can see (02 §4, §8.1; 08 §6.5): the hot
//! zone while the overlay is `hidden` / `peek`, clicks outside the drawn
//! frame while it is `expanded`, and a payload drag dwelling on the hot zone.
//! A pure tracker over cursor samples; the poller thread in `overlay::mod`
//! feeds it at the cadence [`Tracker::interval`] asks for. While the overlay
//! is shown, [`FrameTracker`] decides whether the stage window takes the
//! pointer (only over the drawn frame, 02 §12) and reports `frame_enter` /
//! `frame_leave`, and [`MoveThrottle`] adds `move` samples near the housing
//! for the mascot's gaze (02 §2.1).

use std::time::Duration;

use serde::Serialize;

use super::geometry::Rect;
use super::ladder::Poll;

/// Drag-dwell before the drop target opens (02 §3).
pub const DRAG_DWELL_MS: u64 = 300;
/// Hot-zone cadence while `hidden` / `peek` (02 §15: nothing faster than 10 Hz while hidden).
pub const HOT_ZONE_INTERVAL: Duration = Duration::from_millis(100);
/// While a button is held inside the zone (a possible drag).
pub const DRAG_INTERVAL: Duration = Duration::from_millis(50);
/// Outside-click cadence while `expanded`: fast enough to see a short click.
pub const OUTSIDE_CLICK_INTERVAL: Duration = Duration::from_millis(40);
/// While shown and the pointer is near the stage: the click-through toggle
/// follows the pointer onto and off the frame within a display frame.
pub const FRAME_INTERVAL: Duration = Duration::from_millis(16);
/// How far around the stage (logical px) counts as near it.
pub const STAGE_MARGIN: f64 = 24.0;
/// `move` cadence near the housing: at most 20 Hz.
pub const MOVE_INTERVAL: Duration = Duration::from_millis(50);
/// Beyond this distance from the housing center (logical px) the gaze has
/// saturated; no `move` is sent and the poller slows to [`HOT_ZONE_INTERVAL`].
pub const GAZE_RADIUS: f64 = 600.0;

/// One reading of the pointer, in the physical desktop space of the monitors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub x: f64,
    pub y: f64,
    /// Any mouse button held.
    pub buttons: bool,
    /// A drag with a payload is in progress (macOS: the drag pasteboard
    /// changed since the button went down; not detectable elsewhere).
    pub dragging: bool,
}

/// `overlay.pointer.kind` (`ui/src/state/contract.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerKind {
    HotEnter,
    HotLeave,
    OutsideClick,
    DragDwell,
    Move,
    /// The pointer moved onto the drawn frame (or an open menu) while shown.
    FrameEnter,
    /// The pointer left the drawn frame while shown.
    FrameLeave,
}

/// Where the gaze is measured, while the overlay window is shown: the
/// window's physical rect (the `move` origin), the housing center (or the
/// drawn frame's, without a notch) and the monitor's scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gaze {
    pub window: Rect,
    pub centre: (f64, f64),
    pub scale: f64,
}

impl Gaze {
    fn scale(&self) -> f64 {
        if self.scale > 0.0 { self.scale } else { 1.0 }
    }

    /// The sample within [`GAZE_RADIUS`] of the center (logical px).
    pub fn near(&self, s: &Sample) -> bool {
        let (dx, dy) = (s.x - self.centre.0, s.y - self.centre.1);
        (dx * dx + dy * dy).sqrt() / self.scale() <= GAZE_RADIUS
    }

    /// The sample relative to the window's top-left, in whole CSS px.
    pub fn relative(&self, s: &Sample) -> (f64, f64) {
        (
            ((s.x - self.window.x) / self.scale()).round(),
            ((s.y - self.window.y) / self.scale()).round(),
        )
    }
}

/// Rate limit and dedupe for `move`: at most one per [`MOVE_INTERVAL`],
/// none while the pointer rests, none away from the housing.
#[derive(Debug, Clone, Default)]
pub struct MoveThrottle {
    last_ms: Option<u64>,
    last: Option<(f64, f64)>,
}

impl MoveThrottle {
    /// The `move` position for this sample, if one is due.
    pub fn step(&mut self, gaze: Option<&Gaze>, s: &Sample, now_ms: u64) -> Option<(f64, f64)> {
        let g = gaze.filter(|g| g.near(s))?;
        let p = g.relative(s);
        if self.last == Some(p) {
            return None;
        }
        let interval = MOVE_INTERVAL.as_millis() as u64;
        if self
            .last_ms
            .is_some_and(|t| now_ms.saturating_sub(t) < interval)
        {
            return None;
        }
        self.last_ms = Some(now_ms);
        self.last = Some(p);
        Some(p)
    }

    /// The cadence the gaze needs: 20 Hz near the housing, 10 Hz farther
    /// away (to notice the pointer coming back), nothing while hidden.
    pub fn interval(gaze: Option<&Gaze>, s: Option<&Sample>) -> Option<Duration> {
        let g = gaze?;
        Some(if s.is_some_and(|s| g.near(s)) {
            MOVE_INTERVAL
        } else {
            HOT_ZONE_INTERVAL
        })
    }
}

/// The rects the trackers test against.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Zones {
    pub hot: Option<Rect>,
    /// The drawn frame with its open menus, while the window is shown.
    pub frame: Option<Rect>,
    /// The stage (the window's rect), while the window is shown.
    pub stage: Option<Rect>,
    /// The home monitor's scale factor (for [`STAGE_MARGIN`]).
    pub scale: f64,
    /// Counts the times the window was shown: a new showing starts the
    /// click-through state afresh, even when the poller missed the hide.
    pub shown: u64,
}

impl Zones {
    /// The sample within [`STAGE_MARGIN`] of the stage.
    fn near_stage(&self, x: f64, y: f64) -> bool {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        let m = STAGE_MARGIN * s;
        self.stage.is_some_and(|r| {
            Rect::new(r.x - m, r.y - m, r.w + 2.0 * m, r.h + 2.0 * m).contains(x, y)
        })
    }

    fn in_hover(&self, x: f64, y: f64) -> bool {
        self.hot.is_some_and(|r| r.contains(x, y)) || self.in_frame(x, y)
    }

    fn in_frame(&self, x: f64, y: f64) -> bool {
        self.frame.is_some_and(|r| r.contains(x, y))
    }
}

/// What one sample changes about the stage window: a new click-through
/// value to apply, and the edge to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameStep {
    /// `Some(true)`: let the pointer through (outside the frame);
    /// `Some(false)`: take it (on the frame). `None`: nothing to change.
    pub click_through: Option<bool>,
    pub edge: Option<PointerKind>,
}

/// Whether the pointer is on the drawn frame while the stage is shown
/// (02 §12, 08 §6.5). The window is click-through when shown (the
/// controller sets it with `Show`); the first sample of a showing applies
/// the real value, later ones only a change. A pointer pressed on the frame
/// stays on it until released, so a drag or a text selection that leaves
/// the frame keeps its window.
#[derive(Debug, Clone, Default)]
pub struct FrameTracker {
    shown: u64,
    in_frame: Option<bool>,
}

impl FrameTracker {
    /// One sample (or none when the pointer could not be read).
    pub fn step(&mut self, zones: &Zones, sample: Option<&Sample>) -> FrameStep {
        if zones.stage.is_none() {
            self.in_frame = None;
            return FrameStep::default();
        }
        if zones.shown != self.shown {
            self.shown = zones.shown;
            self.in_frame = None;
        }
        let Some(s) = sample else {
            return FrameStep::default();
        };
        let on = zones.in_frame(s.x, s.y);
        let inside = on || (s.buttons && self.in_frame == Some(true));
        match self.in_frame.replace(inside) {
            None => FrameStep {
                click_through: Some(!inside),
                edge: inside.then_some(PointerKind::FrameEnter),
            },
            Some(was) if was != inside => FrameStep {
                click_through: Some(!inside),
                edge: Some(if inside {
                    PointerKind::FrameEnter
                } else {
                    PointerKind::FrameLeave
                }),
            },
            Some(_) => FrameStep::default(),
        }
    }

    /// The cadence while shown: [`FRAME_INTERVAL`] near the stage,
    /// [`HOT_ZONE_INTERVAL`] farther away; nothing while hidden.
    pub fn interval(zones: &Zones, sample: Option<&Sample>) -> Option<Duration> {
        zones.stage?;
        Some(if sample.is_some_and(|s| zones.near_stage(s.x, s.y)) {
            FRAME_INTERVAL
        } else {
            HOT_ZONE_INTERVAL
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Tracker {
    mode: Poll,
    inside: bool,
    pressed: bool,
    drag_since: Option<u64>,
    drag_fired: bool,
}

impl Tracker {
    pub fn mode(&self) -> Poll {
        self.mode
    }

    /// Switch modes. Entering hot-zone tracking adopts the current
    /// containment silently (only a real entry arms the peek dwell, as in the
    /// webview); leaving it while the cursor is in the zone but off the frame
    /// reports `hot_leave`, so the webview's hover flag is not left set.
    pub fn set_mode(
        &mut self,
        mode: Poll,
        zones: &Zones,
        sample: Option<Sample>,
    ) -> Vec<PointerKind> {
        let mut out = Vec::new();
        if mode == self.mode {
            return out;
        }
        if self.mode == Poll::HotZone
            && self.inside
            && sample.is_none_or(|s| !zones.in_frame(s.x, s.y))
        {
            out.push(PointerKind::HotLeave);
        }
        self.mode = mode;
        self.inside = mode == Poll::HotZone && sample.is_some_and(|s| zones.in_hover(s.x, s.y));
        self.pressed = sample.is_some_and(|s| s.buttons);
        self.drag_since = None;
        self.drag_fired = false;
        out
    }

    /// One sample at `now_ms` (any monotonic clock).
    pub fn step(&mut self, s: Sample, zones: &Zones, now_ms: u64) -> Vec<PointerKind> {
        let mut out = Vec::new();
        match self.mode {
            Poll::Off => {}
            Poll::HotZone => {
                let inside = zones.in_hover(s.x, s.y);
                if inside && !self.inside {
                    out.push(PointerKind::HotEnter);
                } else if !inside && self.inside {
                    out.push(PointerKind::HotLeave);
                }
                self.inside = inside;
                if s.dragging && inside {
                    let since = *self.drag_since.get_or_insert(now_ms);
                    if !self.drag_fired && now_ms.saturating_sub(since) >= DRAG_DWELL_MS {
                        self.drag_fired = true;
                        out.push(PointerKind::DragDwell);
                    }
                } else {
                    self.drag_since = None;
                    if !s.dragging {
                        self.drag_fired = false;
                    }
                }
            }
            Poll::OutsideClick => {
                if s.buttons && !self.pressed && !zones.in_frame(s.x, s.y) {
                    out.push(PointerKind::OutsideClick);
                }
            }
        }
        self.pressed = s.buttons;
        out
    }

    /// How long until the next sample; `None` when polling is off.
    pub fn interval(&self) -> Option<Duration> {
        match self.mode {
            Poll::Off => None,
            Poll::HotZone if self.pressed && self.inside => Some(DRAG_INTERVAL),
            Poll::HotZone => Some(HOT_ZONE_INTERVAL),
            Poll::OutsideClick => Some(OUTSIDE_CLICK_INTERVAL),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64) -> Sample {
        Sample {
            x,
            y,
            buttons: false,
            dragging: false,
        }
    }

    fn zones() -> Zones {
        Zones {
            hot: Some(Rect::new(100.0, 0.0, 360.0, 6.0)),
            ..Zones::default()
        }
    }

    #[test]
    fn hot_zone_enter_and_leave_are_edges() {
        let z = zones();
        let mut t = Tracker::default();
        assert!(
            t.set_mode(Poll::HotZone, &z, Some(at(0.0, 500.0)))
                .is_empty()
        );
        assert_eq!(t.interval(), Some(HOT_ZONE_INTERVAL));
        assert!(t.step(at(10.0, 400.0), &z, 0).is_empty());
        assert_eq!(t.step(at(200.0, 2.0), &z, 100), vec![PointerKind::HotEnter]);
        assert!(
            t.step(at(220.0, 3.0), &z, 200).is_empty(),
            "no repeat while inside"
        );
        assert_eq!(
            t.step(at(220.0, 30.0), &z, 300),
            vec![PointerKind::HotLeave]
        );
    }

    #[test]
    fn a_cursor_already_resting_in_the_zone_is_not_an_entry() {
        let z = zones();
        let mut t = Tracker::default();
        t.set_mode(Poll::HotZone, &z, Some(at(200.0, 2.0)));
        assert!(t.step(at(201.0, 2.0), &z, 0).is_empty());
        assert_eq!(
            t.step(at(201.0, 50.0), &z, 100),
            vec![PointerKind::HotLeave]
        );
    }

    #[test]
    fn the_peek_frame_extends_the_hover_area() {
        let mut z = zones();
        let mut t = Tracker::default();
        t.set_mode(Poll::HotZone, &z, Some(at(0.0, 500.0)));
        assert_eq!(t.step(at(200.0, 2.0), &z, 0), vec![PointerKind::HotEnter]);
        // Peek shows a 120 × 26 frame under the zone: moving down onto it is not a leave.
        z.frame = Some(Rect::new(220.0, 0.0, 120.0, 26.0));
        assert!(t.step(at(230.0, 20.0), &z, 100).is_empty());
        assert_eq!(
            t.step(at(230.0, 40.0), &z, 200),
            vec![PointerKind::HotLeave]
        );
    }

    #[test]
    fn leaving_hot_zone_mode_off_the_frame_reports_a_leave() {
        let mut z = zones();
        let mut t = Tracker::default();
        t.set_mode(Poll::HotZone, &z, Some(at(0.0, 500.0)));
        t.step(at(150.0, 2.0), &z, 0);
        z.frame = Some(Rect::new(220.0, 0.0, 120.0, 36.0));
        // Compact: polling stops; the cursor sits in the zone left of the pill.
        assert_eq!(
            t.set_mode(Poll::Off, &z, Some(at(150.0, 2.0))),
            vec![PointerKind::HotLeave]
        );
        assert_eq!(t.interval(), None);
        // On the pill instead: the webview's own mouseleave covers it.
        let mut t = Tracker::default();
        t.set_mode(Poll::HotZone, &z, Some(at(0.0, 500.0)));
        t.step(at(230.0, 2.0), &z, 0);
        assert!(t.set_mode(Poll::Off, &z, Some(at(230.0, 2.0))).is_empty());
    }

    #[test]
    fn outside_click_is_a_press_outside_the_frame() {
        let z = Zones {
            frame: Some(Rect::new(100.0, 0.0, 520.0, 356.0)),
            ..Zones::default()
        };
        let mut t = Tracker::default();
        let down = |x, y| Sample {
            x,
            y,
            buttons: true,
            dragging: false,
        };
        // A button already held when the mode starts (the click that expanded it) is not one.
        t.set_mode(Poll::OutsideClick, &z, Some(down(200.0, 100.0)));
        assert_eq!(t.interval(), Some(OUTSIDE_CLICK_INTERVAL));
        assert!(
            t.step(down(900.0, 100.0), &z, 0).is_empty(),
            "a drag that left the frame"
        );
        assert!(t.step(at(900.0, 100.0), &z, 40).is_empty());
        assert_eq!(
            t.step(down(900.0, 100.0), &z, 80),
            vec![PointerKind::OutsideClick]
        );
        assert!(
            t.step(down(900.0, 120.0), &z, 120).is_empty(),
            "held, not a new click"
        );
        t.step(at(300.0, 100.0), &z, 160);
        assert!(
            t.step(down(300.0, 100.0), &z, 200).is_empty(),
            "inside: the webview's click"
        );
    }

    #[test]
    fn a_payload_drag_dwelling_300_ms_fires_once() {
        let z = zones();
        let mut t = Tracker::default();
        t.set_mode(Poll::HotZone, &z, Some(at(0.0, 500.0)));
        let drag = |x, y| Sample {
            x,
            y,
            buttons: true,
            dragging: true,
        };
        assert_eq!(
            t.step(drag(200.0, 2.0), &z, 1_000),
            vec![PointerKind::HotEnter]
        );
        assert_eq!(t.interval(), Some(DRAG_INTERVAL));
        assert!(t.step(drag(201.0, 2.0), &z, 1_200).is_empty());
        assert_eq!(
            t.step(drag(202.0, 2.0), &z, 1_300),
            vec![PointerKind::DragDwell]
        );
        assert!(
            t.step(drag(203.0, 2.0), &z, 1_600).is_empty(),
            "once per drag"
        );
        // Button held without a payload (a window drag): never a drag dwell.
        let mut t = Tracker::default();
        t.set_mode(Poll::HotZone, &z, Some(at(0.0, 500.0)));
        let held = Sample {
            x: 200.0,
            y: 2.0,
            buttons: true,
            dragging: false,
        };
        t.step(held, &z, 0);
        assert!(t.step(held, &z, 1_000).is_empty());
        // Leaving and re-entering restarts the dwell.
        let mut t = Tracker::default();
        t.set_mode(Poll::HotZone, &z, Some(at(0.0, 500.0)));
        t.step(drag(200.0, 2.0), &z, 0);
        t.step(drag(200.0, 50.0), &z, 200);
        t.step(drag(200.0, 2.0), &z, 250);
        assert!(t.step(drag(200.0, 2.0), &z, 400).is_empty());
        assert!(
            t.step(drag(200.0, 2.0), &z, 550)
                .contains(&PointerKind::DragDwell)
        );
    }

    /// A shown 2x stage (1163, 0, 1280, 960) with a compact frame in its top-center.
    fn shown(shown: u64) -> Zones {
        Zones {
            hot: None,
            frame: Some(Rect::new(1258.0, 0.0, 1090.0, 84.0)),
            stage: Some(Rect::new(1163.0, 0.0, 1280.0, 960.0)),
            scale: 2.0,
            shown,
        }
    }

    fn step(ct: Option<bool>, edge: Option<PointerKind>) -> FrameStep {
        FrameStep {
            click_through: ct,
            edge,
        }
    }

    #[test]
    fn the_window_takes_the_pointer_only_over_the_frame() {
        let z = shown(1);
        let mut f = FrameTracker::default();
        // The first sample of a showing applies the real value: off the frame, let it through.
        assert_eq!(f.step(&z, Some(&at(1200.0, 500.0))), step(Some(true), None));
        assert_eq!(
            f.step(&z, Some(&at(1210.0, 500.0))),
            FrameStep::default(),
            "no repeat"
        );
        assert_eq!(
            f.step(&z, Some(&at(1300.0, 40.0))),
            step(Some(false), Some(PointerKind::FrameEnter))
        );
        assert_eq!(f.step(&z, Some(&at(1310.0, 40.0))), FrameStep::default());
        assert_eq!(
            f.step(&z, Some(&at(1300.0, 200.0))),
            step(Some(true), Some(PointerKind::FrameLeave)),
            "the clear stage below the pill passes clicks through"
        );
        // The frame grows under a resting pointer (an open menu, a morph's union): it is on it.
        let mut grown = z;
        grown.frame = Some(Rect::new(1258.0, 0.0, 1090.0, 400.0));
        assert_eq!(
            f.step(&grown, Some(&at(1300.0, 200.0))),
            step(Some(false), Some(PointerKind::FrameEnter))
        );
        // Hidden: nothing, and the next showing starts afresh.
        assert_eq!(
            f.step(&Zones::default(), Some(&at(1300.0, 40.0))),
            FrameStep::default()
        );
        assert_eq!(
            f.step(&shown(2), Some(&at(1300.0, 40.0))),
            step(Some(false), Some(PointerKind::FrameEnter)),
            "already on the frame when it appears"
        );
        // A hide and a show between two samples still start afresh.
        assert_eq!(
            f.step(&shown(3), Some(&at(1300.0, 40.0))),
            step(Some(false), Some(PointerKind::FrameEnter))
        );
        // No reading: no change.
        assert_eq!(f.step(&shown(3), None), FrameStep::default());
    }

    #[test]
    fn a_press_on_the_frame_keeps_the_window_until_release() {
        let z = shown(1);
        let mut f = FrameTracker::default();
        f.step(&z, Some(&at(1300.0, 40.0)));
        let held = |x, y| Sample {
            x,
            y,
            buttons: true,
            dragging: false,
        };
        assert_eq!(f.step(&z, Some(&held(1300.0, 300.0))), FrameStep::default());
        assert_eq!(
            f.step(&z, Some(&at(1300.0, 300.0))),
            step(Some(true), Some(PointerKind::FrameLeave))
        );
        // A drag coming from outside (a payload) is taken as soon as it is on the frame.
        assert_eq!(
            f.step(&z, Some(&held(1300.0, 40.0))),
            step(Some(false), Some(PointerKind::FrameEnter))
        );
    }

    #[test]
    fn the_cadence_is_fast_near_the_stage_only() {
        let z = shown(1);
        assert_eq!(
            FrameTracker::interval(&Zones::default(), Some(&at(0.0, 0.0))),
            None
        );
        assert_eq!(
            FrameTracker::interval(&z, Some(&at(1300.0, 500.0))),
            Some(FRAME_INTERVAL)
        );
        // 24 pt = 48 px around the stage still counts; farther does not.
        assert_eq!(
            FrameTracker::interval(&z, Some(&at(1163.0 - 47.0, 500.0))),
            Some(FRAME_INTERVAL)
        );
        assert_eq!(
            FrameTracker::interval(&z, Some(&at(1163.0 - 49.0, 500.0))),
            Some(HOT_ZONE_INTERVAL)
        );
        assert_eq!(
            FrameTracker::interval(&z, Some(&at(1300.0, 1009.0))),
            Some(HOT_ZONE_INTERVAL)
        );
        assert_eq!(FrameTracker::interval(&z, None), Some(HOT_ZONE_INTERVAL));
        assert_eq!(
            serde_json::to_value(PointerKind::FrameEnter).unwrap(),
            serde_json::json!("frame_enter")
        );
        assert_eq!(
            serde_json::to_value(PointerKind::FrameLeave).unwrap(),
            serde_json::json!("frame_leave")
        );
    }

    #[test]
    fn moves_are_window_relative_throttled_and_only_near_the_housing() {
        // A 2x display: window at (1000, 0) physical, housing center (1400, 32).
        let g = Gaze {
            window: Rect::new(1000.0, 0.0, 800.0, 200.0),
            centre: (1400.0, 32.0),
            scale: 2.0,
        };
        let mut m = MoveThrottle::default();
        assert_eq!(m.step(None, &at(1400.0, 40.0), 0), None, "hidden: nothing");
        assert_eq!(m.step(Some(&g), &at(1400.0, 40.0), 0), Some((200.0, 20.0)));
        assert_eq!(
            m.step(Some(&g), &at(1500.0, 40.0), 20),
            None,
            "at most every 50 ms"
        );
        assert_eq!(m.step(Some(&g), &at(1500.0, 40.0), 60), Some((250.0, 20.0)));
        assert_eq!(
            m.step(Some(&g), &at(1500.0, 40.0), 200),
            None,
            "a resting pointer sends nothing"
        );
        // 1 300 physical px = 650 logical px away: past the radius.
        assert_eq!(m.step(Some(&g), &at(2700.0, 32.0), 300), None);
        assert_eq!(
            m.step(Some(&g), &at(600.0, 300.0), 400),
            Some((-200.0, 150.0)),
            "left of the window, still within 600 px"
        );
        assert_eq!(MoveThrottle::interval(None, Some(&at(1400.0, 40.0))), None);
        assert_eq!(
            MoveThrottle::interval(Some(&g), Some(&at(1400.0, 40.0))),
            Some(MOVE_INTERVAL)
        );
        assert_eq!(
            MoveThrottle::interval(Some(&g), Some(&at(2700.0, 32.0))),
            Some(HOT_ZONE_INTERVAL)
        );
        assert_eq!(
            serde_json::to_value(PointerKind::Move).unwrap(),
            serde_json::json!("move")
        );
    }
}
