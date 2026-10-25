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
