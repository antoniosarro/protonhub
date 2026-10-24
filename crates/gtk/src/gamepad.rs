//! Gamepad navigation, for the Steam Deck and the couch: the D-pad or left
//! stick moves the focus, A activates, B closes a dialog, and the shoulder
//! buttons switch tabs. Pads are read on the main loop, between frames.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gilrs::{Axis, Button, EventType, Gilrs};
use gtk::glib;

use crate::window::Window;

/// What a gamepad input does in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Focus(gtk::DirectionType),
    Activate,
    Back,
    /// The next (1) or previous (-1) tab.
    Tab(i32),
}

/// How far a stick goes before it counts as a press, and how far back
/// before it can press again.
const PRESS: f32 = 0.6;
const RELEASE: f32 = 0.3;

/// Turns events into navigation; it remembers where the stick is, so
/// holding it moves the focus once.
#[derive(Default)]
pub struct Mapper {
    x: f32,
    y: f32,
}

impl Mapper {
    pub fn nav(&mut self, event: &EventType) -> Option<Nav> {
        match *event {
            EventType::ButtonPressed(button, _) => button_nav(button),
            EventType::AxisChanged(axis, v, _) => self.axis_nav(axis, v),
            _ => None,
        }
    }

    fn axis_nav(&mut self, axis: Axis, v: f32) -> Option<Nav> {
        use gtk::DirectionType::{Down, Left, Right, Up};
        match axis {
            Axis::LeftStickX => {
                stick(&mut self.x, v).map(|d| Nav::Focus(if d > 0 { Right } else { Left }))
            }
            // Up is positive.
            Axis::LeftStickY => {
                stick(&mut self.y, v).map(|d| Nav::Focus(if d > 0 { Up } else { Down }))
            }
            _ => None,
        }
    }
}

fn button_nav(button: Button) -> Option<Nav> {
    use gtk::DirectionType::{Down, Left, Right, Up};
    match button {
        Button::DPadUp => Some(Nav::Focus(Up)),
        Button::DPadDown => Some(Nav::Focus(Down)),
        Button::DPadLeft => Some(Nav::Focus(Left)),
        Button::DPadRight => Some(Nav::Focus(Right)),
        Button::South => Some(Nav::Activate),
        Button::East => Some(Nav::Back),
        Button::LeftTrigger => Some(Nav::Tab(-1)),
        Button::RightTrigger => Some(Nav::Tab(1)),
        _ => None,
    }
}

/// A press (its sign) when the stick crosses PRESS from rest.
fn stick(last: &mut f32, v: f32) -> Option<i32> {
    let was = *last;
    if v.abs() < RELEASE {
        *last = 0.0;
        return None;
    }
    if v.abs() >= PRESS && was == 0.0 {
        *last = v;
        return Some(if v > 0.0 { 1 } else { -1 });
    }
    None
}

/// Reads the pads every frame and drives `win`. Without gamepad support
/// (no udev), it says so once and does nothing.
pub fn attach(win: &Rc<Window>) {
    let mut gilrs = match Gilrs::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("protonhub-gtk: no gamepad support: {e}");
            return;
        }
    };
    let mut mapper = Mapper::default();
    let win = Rc::downgrade(win);
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(win) = win.upgrade() else {
            return glib::ControlFlow::Break;
        };
        while let Some(event) = gilrs.next_event() {
            if let Some(nav) = mapper.nav(&event.event) {
                apply(&win, nav);
            }
        }
        glib::ControlFlow::Continue
    });
}

fn apply(win: &Rc<Window>, nav: Nav) {
    let window = &win.window;
    match nav {
        Nav::Focus(dir) => {
            // Within an open dialog, or else the window.
            match window.visible_dialog() {
                Some(d) => d.child_focus(dir),
                None => window.child_focus(dir),
            };
        }
        Nav::Activate => {
            if let Some(w) = gtk::prelude::RootExt::focus(window) {
                w.activate();
            }
        }
        Nav::Back => {
            if let Some(d) = window.visible_dialog() {
                d.close();
            }
        }
        Nav::Tab(delta) => win.switch_tab(delta),
    }
}

#[cfg(test)]
mod tests {
    use gtk::DirectionType::{Down, Right, Up};

    use super::*;

    #[test]
    fn buttons_navigate() {
        assert_eq!(button_nav(Button::DPadDown), Some(Nav::Focus(Down)));
        assert_eq!(button_nav(Button::South), Some(Nav::Activate));
        assert_eq!(button_nav(Button::East), Some(Nav::Back));
        assert_eq!(button_nav(Button::RightTrigger), Some(Nav::Tab(1)));
        assert_eq!(button_nav(Button::LeftTrigger), Some(Nav::Tab(-1)));
        assert_eq!(button_nav(Button::Start), None);
    }

    #[test]
    fn holding_the_stick_moves_once() {
        let mut m = Mapper::default();
        let mut y = |v| m.axis_nav(Axis::LeftStickY, v);
        assert_eq!(y(0.4), None, "not far enough");
        assert_eq!(y(0.9), Some(Nav::Focus(Up)));
        assert_eq!(y(1.0), None, "still held");
        assert_eq!(y(0.1), None, "back to rest");
        assert_eq!(y(-0.8), Some(Nav::Focus(Down)));
        assert_eq!(
            m.axis_nav(Axis::LeftStickX, 0.7),
            Some(Nav::Focus(Right)),
            "axes are apart"
        );
    }
}
