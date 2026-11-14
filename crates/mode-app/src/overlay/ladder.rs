//! The native half of the overlay ladder (02 §2, §6.6, §9) as a pure state
//! table. The webview owns the ladder's logic (timers, pill rules, keys) and
//! tells the core what it wants with `overlay_request`; this decides what
//! the window does about it: shown or not, which geometry, key input or
//! not, which pointer polling, whether `overlay.pinned` must be written, and
//! when a card-capable overlay has first been drawn (`ui_ready`).
//!
//! Presentation mode (02 §6.6) suspends the overlay unless
//! `overlay.fullscreen = show`: it stays hidden whatever the webview asks,
//! except for an opening that follows a hotkey ("the hotkeys still open it").

use serde::{Deserialize, Serialize};

use super::geometry::Shape;

/// `overlay_request.state` (02 §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Hidden,
    Peek,
    Compact,
    Expanded,
}

impl State {
    pub fn parse(s: &str) -> Option<State> {
        match s {
            "hidden" => Some(State::Hidden),
            "peek" => Some(State::Peek),
            "compact" => Some(State::Compact),
            "expanded" => Some(State::Expanded),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            State::Hidden => "hidden",
            State::Peek => "peek",
            State::Compact => "compact",
            State::Expanded => "expanded",
        }
    }

    pub fn shape(self) -> Option<Shape> {
        match self {
            State::Hidden => None,
            State::Peek => Some(Shape::Peek),
            State::Compact => Some(Shape::Compact),
            State::Expanded => Some(Shape::Expanded),
        }
    }
}

/// `overlay.fullscreen` (02 §17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fullscreen {
    #[default]
    PresentationHides,
    Show,
    /// Stricter than the default; with only presentation detection available
    /// it behaves the same.
    Hide,
}

impl Fullscreen {
    pub fn parse(s: &str) -> Fullscreen {
        match s {
            "show" => Fullscreen::Show,
            "hide" => Fullscreen::Hide,
            _ => Fullscreen::PresentationHides,
        }
    }
}

/// One `overlay_request` (02 §16 `overlay.state`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub state: State,
    pub pinned: bool,
    /// The capture box holds key input (02 §9.2).
    pub keyboard: bool,
    /// The webview's reason (`launch`, `arrival`, `expand:hotkey`, `collapse:…`).
    pub reason: String,
}

impl Request {
    /// The webview (re)started and does not know `overlay.pinned` yet.
    pub fn is_launch(&self) -> bool {
        self.reason == "launch"
    }
}

/// Which pointer polling the window needs (02 §15, 08 §6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Poll {
    #[default]
    Off,
    /// `hidden` / `peek`: the hot zone (plus the peek frame), at a modest rate.
    HotZone,
    /// `expanded` and collapsible (or holding key input): clicks outside the frame.
    OutsideClick,
}

/// What the window should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub visible: bool,
    /// The geometry while visible.
    pub shape: Option<Shape>,
    pub key: bool,
    pub poll: Poll,
}

/// What the controller does, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Size and place the window for this state (before `Show`).
    Layout(Shape),
    /// Order the window in without activating Mode.
    Show,
    /// Order it out (macOS, Linux) / shrink it to a click-through pixel (Windows).
    Hide,
    /// Take (true) or release (false) key input for the capture box.
    Key(bool),
    Poll(Poll),
    /// Write `overlay.pinned`.
    PersistPinned(bool),
    /// The webview's pin differs from `overlay.pinned` (launch, or a Settings
    /// change): send it the pin hotkey so it toggles (02 §2.2 "launch, pinned").
    SyncPin,
    /// The overlay was drawn for the first time: decision frames may now wait
    /// for a card (08 §5.2).
    UiReady,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ladder {
    pub requested: State,
    pub pinned: bool,
    pub keyboard: bool,
    pub presentation: bool,
    pub fullscreen: Fullscreen,
    /// A hotkey opened the overlay while suspended.
    pub hotkey_open: bool,
    /// `overlay.pinned` as stored.
    pub stored_pinned: bool,
    /// The overlay has been drawn at least once.
    pub ready: bool,
}

impl Ladder {
    pub fn new(stored_pinned: bool, fullscreen: Fullscreen) -> Ladder {
        Ladder {
            requested: State::Hidden,
            pinned: false,
            keyboard: false,
            presentation: false,
            fullscreen,
            hotkey_open: false,
            stored_pinned,
            ready: false,
        }
    }

    pub fn suspended(&self) -> bool {
        self.presentation && self.fullscreen != Fullscreen::Show
    }

    pub fn plan(&self) -> Plan {
        let visible = self.requested != State::Hidden && (!self.suspended() || self.hotkey_open);
        let shape = if visible {
            self.requested.shape()
        } else {
            None
        };
        let key = visible && self.keyboard && self.requested == State::Expanded;
        let poll = if !visible {
            if self.suspended() {
                Poll::Off
            } else {
                Poll::HotZone
            }
        } else {
            match self.requested {
                State::Peek | State::Hidden => Poll::HotZone,
                State::Compact => Poll::Off,
                State::Expanded if !self.pinned || self.keyboard => Poll::OutsideClick,
                State::Expanded => Poll::Off,
            }
        };
        Plan {
            visible,
            shape,
            key,
            poll,
        }
    }

    /// The webview asked for a state. `hotkey_recent`: a global hotkey fired
    /// moments ago (it may open the overlay through a suspension).
    pub fn request(&mut self, r: &Request, hotkey_recent: bool) -> Vec<Action> {
        let before = self.plan();
        let mut out = Vec::new();
        if r.is_launch() {
            if self.stored_pinned != r.pinned {
                out.push(Action::SyncPin);
            }
        } else if r.pinned != self.stored_pinned {
            self.stored_pinned = r.pinned;
            out.push(Action::PersistPinned(r.pinned));
        }
        self.hotkey_open =
            self.suspended() && r.state == State::Expanded && (hotkey_recent || self.hotkey_open);
        self.requested = r.state;
        self.pinned = r.pinned;
        self.keyboard = r.keyboard && r.state == State::Expanded;
        self.diff(before, &mut out);
        out
    }

    /// `quiet.changed.presentation`.
    pub fn set_presentation(&mut self, on: bool) -> Vec<Action> {
        let before = self.plan();
        self.presentation = on;
        if !self.suspended() {
            self.hotkey_open = false;
        }
        let mut out = Vec::new();
        self.diff(before, &mut out);
        out
    }

    /// `overlay.fullscreen` changed.
    pub fn set_fullscreen(&mut self, f: Fullscreen) -> Vec<Action> {
        let before = self.plan();
        self.fullscreen = f;
        let mut out = Vec::new();
        self.diff(before, &mut out);
        out
    }

    /// `overlay.pinned` was written from elsewhere (Settings, the status item).
    pub fn set_stored_pinned(&mut self, pinned: bool) -> Vec<Action> {
        self.stored_pinned = pinned;
        if pinned != self.pinned {
            vec![Action::SyncPin]
        } else {
            Vec::new()
        }
    }

    fn diff(&mut self, before: Plan, out: &mut Vec<Action>) {
        let after = self.plan();
        if before.key && !after.key {
            out.push(Action::Key(false));
        }
        match (before.visible, after.visible) {
            (false, true) => {
                if let Some(s) = after.shape {
                    out.push(Action::Layout(s));
                }
                out.push(Action::Show);
            }
            (true, true) if before.shape != after.shape => {
                if let Some(s) = after.shape {
                    out.push(Action::Layout(s));
                }
            }
            (true, false) => out.push(Action::Hide),
            _ => {}
        }
        if after.key && !before.key {
            out.push(Action::Key(true));
        }
        if before.poll != after.poll {
            out.push(Action::Poll(after.poll));
        }
        if after.visible && !self.ready {
            self.ready = true;
            out.push(Action::UiReady);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Action::{Hide, Key, Layout, PersistPinned, Show, SyncPin, UiReady};

    fn req(state: State, pinned: bool, keyboard: bool, reason: &str) -> Request {
        Request {
            state,
            pinned,
            keyboard,
            reason: reason.into(),
        }
    }

    #[test]
    fn launch_hidden_polls_the_hot_zone_and_syncs_a_stored_pin() {
        let mut l = Ladder::new(false, Fullscreen::default());
        assert_eq!(l.plan().poll, Poll::HotZone, "hidden at start: hot zone");
        let a = l.request(&req(State::Hidden, false, false, "launch"), false);
        assert!(a.is_empty(), "nothing to do: {a:?}");

        let mut l = Ladder::new(true, Fullscreen::default());
        let a = l.request(&req(State::Hidden, false, false, "launch"), false);
        assert_eq!(
            a,
            vec![SyncPin],
            "the webview starts unpinned; it is told to pin"
        );
        assert!(l.stored_pinned, "a launch request never writes the setting");
        let a = l.request(&req(State::Expanded, true, false, "expand:pin"), false);
        assert_eq!(
            a,
            vec![
                Layout(Shape::Expanded),
                Show,
                Action::Poll(Poll::Off),
                UiReady
            ],
            "pinned and expanded: no outside-click polling"
        );
    }

    #[test]
    fn the_ladder_states_map_to_show_hide_and_size() {
        let mut l = Ladder::new(false, Fullscreen::default());
        let a = l.request(&req(State::Peek, false, false, "arrival"), false);
        assert_eq!(
            a,
            vec![Layout(Shape::Peek), Show, UiReady],
            "peek keeps the hot-zone poll"
        );
        let a = l.request(&req(State::Compact, false, false, "arrival"), false);
        assert_eq!(a, vec![Layout(Shape::Compact), Action::Poll(Poll::Off)]);
        let a = l.request(&req(State::Compact, false, false, "hover_out"), false);
        assert!(a.is_empty(), "same state: nothing");
        let a = l.request(&req(State::Expanded, false, false, "expand:click"), false);
        assert_eq!(
            a,
            vec![Layout(Shape::Expanded), Action::Poll(Poll::OutsideClick)]
        );
        let a = l.request(
            &req(State::Hidden, false, false, "collapse:outside_click"),
            false,
        );
        assert_eq!(a, vec![Hide, Action::Poll(Poll::HotZone)]);
        let a = l.request(&req(State::Compact, false, false, "arrival"), false);
        assert_eq!(
            a,
            vec![Layout(Shape::Compact), Show, Action::Poll(Poll::Off)],
            "ready only once"
        );
    }

    #[test]
    fn keyboard_takes_and_releases_key_input_around_show_and_hide() {
        let mut l = Ladder::new(false, Fullscreen::default());
        let a = l.request(&req(State::Expanded, false, true, "expand:hotkey"), true);
        assert_eq!(
            a,
            vec![
                Layout(Shape::Expanded),
                Show,
                Key(true),
                Action::Poll(Poll::OutsideClick),
                UiReady
            ]
        );
        let a = l.request(
            &req(State::Expanded, false, false, "keyboard_released"),
            false,
        );
        assert_eq!(a, vec![Key(false)]);
        let a = l.request(&req(State::Expanded, false, true, "keyboard"), false);
        assert_eq!(a, vec![Key(true)]);
        let a = l.request(
            &req(State::Hidden, false, true, "collapse:keyboard_idle"),
            false,
        );
        assert_eq!(
            a,
            vec![Key(false), Hide, Action::Poll(Poll::HotZone)],
            "release before hiding"
        );
        // Key input only exists in `expanded`.
        let a = l.request(&req(State::Compact, false, true, "odd"), false);
        assert!(!a.contains(&Key(true)));
    }

    #[test]
    fn pinned_is_persisted_and_changes_outside_click_polling() {
        let mut l = Ladder::new(false, Fullscreen::default());
        l.request(&req(State::Expanded, false, false, "expand:click"), false);
        let a = l.request(&req(State::Expanded, true, false, "pin"), false);
        assert_eq!(a, vec![PersistPinned(true), Action::Poll(Poll::Off)]);
        // A pinned overlay in a keyboard session still watches outside clicks (to release keys).
        let a = l.request(&req(State::Expanded, true, true, "keyboard"), false);
        assert_eq!(a, vec![Key(true), Action::Poll(Poll::OutsideClick)]);
        let a = l.request(&req(State::Compact, false, false, "collapse:escape"), false);
        assert_eq!(
            a,
            vec![
                PersistPinned(false),
                Key(false),
                Layout(Shape::Compact),
                Action::Poll(Poll::Off)
            ]
        );
        // Settings wrote overlay.pinned = true while the webview is unpinned.
        assert_eq!(l.set_stored_pinned(true), vec![SyncPin]);
        assert_eq!(l.set_stored_pinned(false), vec![]);
    }

    #[test]
    fn presentation_suspends_unless_fullscreen_is_show() {
        // request × pinned × quiet → visible
        let table: &[(State, bool, bool, Fullscreen, bool)] = &[
            // (state, pinned, presentation, fullscreen, visible)
            (
                State::Compact,
                false,
                false,
                Fullscreen::PresentationHides,
                true,
            ),
            (
                State::Compact,
                false,
                true,
                Fullscreen::PresentationHides,
                false,
            ),
            (
                State::Expanded,
                true,
                true,
                Fullscreen::PresentationHides,
                false,
            ),
            (State::Expanded, true, true, Fullscreen::Hide, false),
            (State::Expanded, true, true, Fullscreen::Show, true),
            (State::Peek, false, true, Fullscreen::Show, true),
            (State::Hidden, true, false, Fullscreen::Show, false),
        ];
        for &(state, pinned, presentation, fullscreen, visible) in table {
            let mut l = Ladder::new(pinned, fullscreen);
            l.set_presentation(presentation);
            l.request(&req(state, pinned, false, "test"), false);
            assert_eq!(
                l.plan().visible,
                visible,
                "{state:?} pinned={pinned} presentation={presentation} {fullscreen:?}"
            );
            if presentation && fullscreen != Fullscreen::Show && !visible {
                assert_eq!(l.plan().poll, Poll::Off, "no hot zone while suspended");
            }
        }
    }

    #[test]
    fn suspension_hides_and_resumes_and_hotkeys_still_open() {
        let mut l = Ladder::new(false, Fullscreen::default());
        l.request(&req(State::Compact, false, false, "arrival"), false);
        assert_eq!(l.set_presentation(true), vec![Hide]);
        // An arrival while suspended stays hidden.
        let a = l.request(&req(State::Compact, false, false, "arrival"), false);
        assert!(a.is_empty());
        // A hotkey opens it anyway, and it stays open through follow-up requests.
        let a = l.request(&req(State::Expanded, false, true, "expand:hotkey"), true);
        assert_eq!(
            a,
            vec![
                Layout(Shape::Expanded),
                Show,
                Key(true),
                Action::Poll(Poll::OutsideClick)
            ]
        );
        let a = l.request(&req(State::Expanded, true, true, "pin"), false);
        assert_eq!(a, vec![PersistPinned(true)]);
        // Collapsing ends the exception.
        let a = l.request(&req(State::Hidden, false, false, "collapse:escape"), false);
        assert_eq!(
            a,
            vec![
                PersistPinned(false),
                Key(false),
                Hide,
                Action::Poll(Poll::Off)
            ]
        );
        let a = l.request(&req(State::Expanded, false, false, "expand:click"), false);
        assert!(a.is_empty(), "no hotkey: stays hidden while suspended");
        // The presentation ends: the requested state comes back.
        let a = l.set_presentation(false);
        assert_eq!(
            a,
            vec![
                Layout(Shape::Expanded),
                Show,
                Action::Poll(Poll::OutsideClick)
            ]
        );
        // fullscreen = show lifts a suspension at once.
        l.set_presentation(true);
        let a = l.set_fullscreen(Fullscreen::Show);
        assert_eq!(
            a,
            vec![
                Layout(Shape::Expanded),
                Show,
                Action::Poll(Poll::OutsideClick)
            ]
        );
    }

    #[test]
    fn states_parse_from_the_wire() {
        for s in [State::Hidden, State::Peek, State::Compact, State::Expanded] {
            assert_eq!(State::parse(s.as_str()), Some(s));
        }
        assert_eq!(State::parse("open"), None);
        assert_eq!(Fullscreen::parse("show"), Fullscreen::Show);
        assert_eq!(
            Fullscreen::parse("presentation-hides"),
            Fullscreen::PresentationHides
        );
        assert_eq!(Fullscreen::parse("hide"), Fullscreen::Hide);
    }
}
