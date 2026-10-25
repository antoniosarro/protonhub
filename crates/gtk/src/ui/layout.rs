//! How views and pages are laid out: grids of cards, sections, columns
//! that stack where the view is narrow, a page's top, scrolled content.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::window::Window;

/// What activating a card does.
pub type Action = Rc<dyn Fn()>;

/// A grid of cards that fills the width, as many to a line as fit; a card
/// with an action does it when activated.
#[derive(Clone)]
pub struct Grid {
    pub flow: gtk::FlowBox,
    /// Each card's, in their order.
    actions: Rc<RefCell<Vec<Option<Action>>>>,
}

impl Grid {
    pub fn new() -> Self {
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .homogeneous(true)
            .min_children_per_line(1)
            .max_children_per_line(12)
            .column_spacing(12)
            .row_spacing(12)
            .valign(gtk::Align::Start)
            .activate_on_single_click(true)
            .build();
        flow.add_css_class("grid");
        let actions: Rc<RefCell<Vec<Option<Action>>>> = Rc::default();
        flow.connect_child_activated(glib::clone!(
            #[strong]
            actions,
            move |_, child| {
                let action = usize::try_from(child.index())
                    .ok()
                    .and_then(|i| actions.borrow().get(i).cloned().flatten());
                if let Some(action) = action {
                    action();
                }
            }
        ));
        Self { flow, actions }
    }

    /// Adds `card`; activating it does `action`.
    pub fn add(&self, card: &impl IsA<gtk::Widget>, action: Option<Action>) -> gtk::FlowBoxChild {
        if action.is_some() {
            card.add_css_class("activatable");
        }
        let child = gtk::FlowBoxChild::builder().child(card).build();
        self.flow.append(&child);
        self.actions.borrow_mut().push(action);
        child
    }
}

/// A card: a rounded box, `width` wide at least, its corners clipping what's in it.
pub fn card(orientation: gtk::Orientation, width: i32) -> gtk::Box {
    let card = gtk::Box::builder()
        .orientation(orientation)
        .width_request(width)
        .overflow(gtk::Overflow::Hidden)
        .build();
    card.add_css_class("card");
    card
}

/// A section of a view: a heading with a few words beside it, an optional
/// widget at its end, and what's under it.
pub fn section(
    title: &str,
    detail: Option<&str>,
    end: Option<&gtk::Widget>,
    content: &impl IsA<gtk::Widget>,
) -> gtk::Box {
    let heading = gtk::Box::builder().spacing(12).build();
    let name = gtk::Label::builder().label(title).xalign(0.0).build();
    name.add_css_class("title-4");
    heading.append(&name);
    let detail = gtk::Label::builder()
        .label(detail.unwrap_or_default())
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    detail.add_css_class("dim-label");
    heading.append(&detail);
    if let Some(end) = end {
        heading.append(end);
    }
    let section = gtk::Box::new(gtk::Orientation::Vertical, 12);
    section.append(&heading);
    section.append(content);
    section
}

/// A box that turns vertical where its view is narrower than a width, in
/// pixels.
pub type Flip<'a> = (&'a gtk::Box, u32);

/// A view's or page's content, scrolled, no wider than `max`; each box in
/// `flips` turns vertical where the view is narrower than its width.
/// Where it was scrolled to is kept under `key`, to scroll back to after a
/// redraw.
pub fn scrolled(
    win: &Rc<Window>,
    key: String,
    child: &impl IsA<gtk::Widget>,
    max: i32,
    flips: &[Flip],
) -> gtk::Widget {
    let clamp = adw::Clamp::builder()
        .maximum_size(max)
        .tightening_threshold(max)
        .child(child)
        .margin_top(18)
        .margin_bottom(32)
        .margin_start(18)
        .margin_end(18)
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .vexpand(true)
        .build();
    let adjustment = scrolled.vadjustment();
    if let Some(&to) = win.scrolls.borrow().get(&key) {
        // Once the content is measured and that far can be scrolled to.
        let handler: Rc<Cell<Option<glib::SignalHandlerId>>> = Rc::default();
        handler.set(Some(adjustment.connect_changed(glib::clone!(
            #[strong]
            handler,
            move |a| {
                if a.upper() - a.page_size() >= to {
                    a.set_value(to);
                    if let Some(h) = handler.take() {
                        a.disconnect(h);
                    }
                }
            }
        ))));
    }
    adjustment.connect_value_changed(glib::clone!(
        #[weak]
        win,
        move |a| {
            win.scrolls.borrow_mut().insert(key.clone(), a.value());
        }
    ));
    if flips.is_empty() {
        return scrolled.upcast();
    }
    // Outside the scrolled window, where its width is the view's: inside,
    // it would cut the content short instead of letting it scroll.
    let bin = adw::BreakpointBin::builder()
        .child(&scrolled)
        .width_request(300)
        .height_request(200)
        .build();
    // Only the last breakpoint that applies does, so each turns the boxes
    // of the wider ones too.
    let mut widths: Vec<u32> = flips.iter().map(|(_, w)| *w).collect();
    widths.sort_unstable_by(|a, b| b.cmp(a));
    widths.dedup();
    for width in widths {
        let narrow = adw::Breakpoint::new(
            adw::BreakpointCondition::parse(&format!("max-width: {width}sp"))
                .expect("a valid condition"),
        );
        for (row, _) in flips.iter().filter(|(_, w)| *w >= width) {
            narrow.add_setter(
                *row,
                "orientation",
                Some(&gtk::Orientation::Vertical.to_value()),
            );
            narrow.add_setter(*row, "homogeneous", Some(&false.to_value()));
        }
        bin.add_breakpoint(narrow);
    }
    bin.upcast()
}

/// Where columns turn into one: narrower than reads well side by side.
pub const COLUMNS: u32 = 820;

/// `first` and `second` side by side, each as wide; to flip at [`COLUMNS`],
/// one above the other.
pub fn columns(first: &[gtk::Widget], second: &[gtk::Widget]) -> gtk::Box {
    let row = gtk::Box::builder().spacing(24).homogeneous(true).build();
    for widgets in [first, second] {
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(24)
            .valign(gtk::Align::Start)
            .build();
        for w in widgets {
            column.append(w);
        }
        row.append(&column);
    }
    row
}

/// A page to push: `content` under a header bar with a back button.
pub fn page(title: &str, content: &impl IsA<gtk::Widget>) -> (adw::NavigationPage, adw::HeaderBar) {
    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(content));
    let page = adw::NavigationPage::builder()
        .title(title)
        .child(&view)
        .build();
    (page, header)
}

/// Where a page's top puts its picture over the rest.
pub const HERO: u32 = 640;

/// The top of a page: a picture beside a title and a line under it, and a
/// box under them for the page to add to. To flip at [`HERO`].
pub fn hero(picture: &impl IsA<gtk::Widget>, title: &str, subtitle: &str) -> (gtk::Box, gtk::Box) {
    let name = gtk::Label::builder()
        .label(title)
        .wrap(true)
        .xalign(0.0)
        .build();
    name.add_css_class("title-1");
    let under = gtk::Label::builder()
        .label(subtitle)
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        // Selectable with the pointer, but not where the focus starts.
        .focusable(false)
        .build();
    under.add_css_class("dim-label");
    let info = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    info.append(&name);
    info.append(&under);
    let extra = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(6)
        .build();
    info.append(&extra);
    picture.set_valign(gtk::Align::Start);
    picture.set_halign(gtk::Align::Start);
    let row = gtk::Box::builder().spacing(24).build();
    row.append(picture);
    row.append(&info);
    (row, extra)
}

/// Buttons in a line that wraps, as many to a line as fit.
pub fn wrap(widgets: &[gtk::Widget]) -> gtk::FlowBox {
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(1)
        .max_children_per_line(widgets.len().max(1) as u32)
        .column_spacing(6)
        .row_spacing(6)
        .halign(gtk::Align::Start)
        .build();
    for w in widgets {
        let child = gtk::FlowBoxChild::builder()
            .child(w)
            .focusable(false)
            .build();
        flow.append(&child);
    }
    flow
}
