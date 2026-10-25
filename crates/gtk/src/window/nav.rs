//! Where the window goes: the view the sidebar chooses, the pages pushed
//! over it and back, and drawing it all again with what was just read.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use super::{ERRORS, MANGOHUD, PREFERENCES, STATUS, Window, sidebar};
use crate::i18n::tr;
use crate::pages::{self, Route};
use crate::{errors, ui, views};

impl Window {
    /// The sidebar chooses the view; a page popped is forgotten.
    pub(super) fn connect_navigation(self: &Rc<Self>) {
        // The window lives as long as the app, so its handlers can hold it.
        self.launchers.connect_row_selected(glib::clone!(
            #[strong(rename_to = win)]
            self,
            move |_, row| {
                let Some(row) = row else { return };
                win.mark_place(None);
                win.show(&row.widget_name(), &row.tooltip_text().unwrap_or_default());
            }
        ));
        self.launchers.connect_row_activated(glib::clone!(
            #[strong(rename_to = win)]
            self,
            move |_, _| win.split.set_show_content(true)
        ));
        self.places.connect_row_activated(glib::clone!(
            #[strong(rename_to = win)]
            self,
            move |_, row| win.choose_place(&row.widget_name())
        ));
        self.nav.connect_popped(glib::clone!(
            #[strong(rename_to = win)]
            self,
            move |_, page| win.pushed.borrow_mut().retain(|(_, p)| p != page)
        ));
    }

    /// Chooses one of the app's places in the sidebar, showing it.
    pub(super) fn choose_place(self: &Rc<Self>, name: &str) {
        let Some(row) = self.mark_place(Some(name)) else {
            return;
        };
        row.set_visible(true);
        self.launchers.unselect_all();
        self.show(name, &row.tooltip_text().unwrap_or_default());
        self.split.set_show_content(true);
    }

    /// Marks the place `name` as the one chosen, and no other; gives its row.
    pub(super) fn mark_place(&self, name: Option<&str>) -> Option<gtk::ListBoxRow> {
        let mut chosen = None;
        let mut i = 0;
        while let Some(row) = self.places.row_at_index(i) {
            if Some(row.widget_name().as_str()) == name {
                row.add_css_class("chosen");
                chosen = Some(row);
            } else {
                row.remove_css_class("chosen");
            }
            i += 1;
        }
        chosen
    }

    /// Shows the view `name`, titled `title`, building a place's first;
    /// the pages over the last one shown go, unless it's shown again
    /// after a redraw.
    pub(super) fn show(self: &Rc<Self>, name: &str, title: &str) {
        if !self.redrawing.get() {
            self.nav.pop_to_page(&self.root);
        }
        let place = match name {
            // Built once: it keeps changes not saved yet.
            MANGOHUD if self.views.child_by_name(name).is_none() => {
                Some(pages::mangohud::global(self))
            }
            // Built each time, with what's there now.
            PREFERENCES => Some(pages::preferences::view(self)),
            ERRORS => Some(errors::view(self)),
            _ => None,
        };
        if let Some(view) = place {
            if let Some(old) = self.views.child_by_name(name) {
                self.views.remove(&old);
            }
            self.views.add_named(&view, Some(name));
        }
        if self.views.child_by_name(name).is_some() {
            self.views.set_visible_child_name(name);
        } else if self.views.child_by_name(STATUS).is_some() {
            self.views.set_visible_child_name(STATUS);
        }
        self.root.set_title(title);
        self.shown.replace(name.to_owned());
    }

    fn is_place(name: &str) -> bool {
        [MANGOHUD, PREFERENCES, ERRORS].contains(&name)
    }

    /// Shows the launcher `delta` away from the current one, wrapping around.
    pub fn switch_tab(&self, delta: i32) {
        let mut n = 0;
        while self.launchers.row_at_index(n).is_some() {
            n += 1;
        }
        if n == 0 {
            return;
        }
        let current = self.launchers.selected_row().map_or(0, |r| r.index());
        let next = (current + delta).rem_euclid(n);
        if let Some(row) = self.launchers.row_at_index(next) {
            self.launchers.select_row(Some(&row));
        }
    }

    /// Why there are no launchers to show, or that they're loading.
    fn status(&self) -> Option<gtk::Widget> {
        let snapshot = self.snapshot.borrow();
        let child = match (&self.env, snapshot.as_deref()) {
            (Err(e), _) => ui::status(
                "dialog-error-symbolic",
                &tr("ProtonHub can't start", &[]),
                e,
            ),
            (Ok(_), None) => adw::Spinner::builder()
                .width_request(32)
                .height_request(32)
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .build()
                .upcast(),
            (Ok(_), Some(s)) if s.launchers.is_empty() => ui::status(
                "applications-games-symbolic",
                &tr("No launchers found", &[]),
                &tr(
                    "ProtonHub looks for Steam, Heroic, Lutris, Bottles and WineZGUI, native, Flatpak or Snap.",
                    &[],
                ),
            ),
            _ => return None,
        };
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&child));
        Some(view.upcast())
    }

    /// Draws the sidebar's launchers and their views again from the last
    /// snapshot, keeping what's shown, and builds the pages pushed over
    /// them again.
    pub fn redraw(self: &Rc<Self>) {
        self.redrawing.set(true);
        self.launchers.remove_all();
        let mut child = self.views.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            let name = self.views.page(&c).name().unwrap_or_default();
            if !Self::is_place(&name) {
                self.views.remove(&c);
            }
        }
        let shown = self.shown.borrow().clone();
        if let Some(status) = self.status() {
            self.views.add_named(&status, Some(STATUS));
            if !Self::is_place(&shown) {
                self.show(STATUS, "ProtonHub");
            }
        } else if let (Ok(env), Some(snapshot)) = (&self.env, self.snapshot()) {
            for state in &snapshot.launchers {
                let name = state.launcher.to_string();
                self.views
                    .add_named(&views::launcher(self, env, state), Some(&name));
                self.launchers.append(&sidebar::launcher(state));
            }
            if !Self::is_place(&shown) {
                let mut row = self.launchers.row_at_index(0);
                let mut i = 0;
                while let Some(r) = self.launchers.row_at_index(i) {
                    if r.widget_name() == shown {
                        row = Some(r);
                    }
                    i += 1;
                }
                if row.as_ref().is_some_and(|r| r.widget_name() != shown) {
                    // The launcher shown is gone, and its pages with it.
                    self.nav.pop_to_page(&self.root);
                }
                self.launchers.select_row(row.as_ref());
            }
        }
        self.build_pushed();
        self.redrawing.set(false);
    }

    /// Builds the pages pushed again where they show what was read, up to
    /// the first that shows something no longer there.
    fn build_pushed(self: &Rc<Self>) {
        let old = self.pushed.borrow().clone();
        if old.is_empty() {
            return;
        }
        let mut kept = Vec::new();
        for (route, page) in old {
            let page = match &route {
                Some(r) => match pages::build(self, r) {
                    Some(p) => p,
                    None => break,
                },
                None => page,
            };
            kept.push((route, page));
        }
        let mut stack = vec![self.root.clone()];
        stack.extend(kept.iter().map(|(_, p)| p.clone()));
        self.nav.replace(&stack);
        self.pushed.replace(kept);
    }

    /// Pushes the page `route` shows, which a redraw builds again.
    pub fn open(self: &Rc<Self>, route: Route) {
        if let Some(page) = pages::build(self, &route) {
            self.pushed.borrow_mut().push((Some(route), page.clone()));
            self.nav.push(&page);
            self.split.set_show_content(true);
        }
    }

    /// Pushes `page` as it is, kept as it is across redraws.
    pub fn push(&self, page: &adw::NavigationPage) {
        self.pushed.borrow_mut().push((None, page.clone()));
        self.nav.push(page);
        self.split.set_show_content(true);
    }

    /// Goes back from the page shown, or in a narrow window from the
    /// launchers' views to the sidebar.
    pub fn back(&self) {
        if !self.pushed.borrow().is_empty() {
            self.nav.pop();
        } else if self.split.is_collapsed() {
            self.split.set_show_content(false);
        }
    }
}
