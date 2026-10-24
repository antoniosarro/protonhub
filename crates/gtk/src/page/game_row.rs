//! A game's row: its picture, where it's from, its ratings, links to it on
//! the web, and the list of tools it can run with.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::{Choice, Game, Library};
use protonhub_core::info::Info;
use protonhub_core::launcher::Launcher;
use protonhub_core::manifest::LauncherKind;

use crate::art;
use crate::i18n::{tr, trn};
use crate::window::{Env, Window, capitalized};

/// A ProtonDB tier as people read it.
pub(super) fn tier_name(tier: &str) -> String {
    match tier {
        "platinum" => tr("Platinum", &[]),
        "gold" => tr("Gold", &[]),
        "silver" => tr("Silver", &[]),
        "bronze" => tr("Bronze", &[]),
        "borked" => tr("Borked", &[]),
        "pending" => tr("Pending", &[]),
        other => capitalized(other),
    }
}

pub(super) fn game_row(
    win: &Rc<Window>,
    env: &Env,
    launcher: &Launcher,
    lib: &Library,
    game: &Game,
    info: Option<&Info>,
    running: bool,
) -> gtk::ListBoxRow {
    let name = gtk::Label::builder()
        .label(&game.name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    // Where it's from, and its anti-cheat's status, colored by how it
    // fares on Linux.
    let under = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let from = gtk::Label::new(Some(&source(game.source)));
    from.add_css_class("dim-label");
    under.append(&from);
    if let Some(a) = info.and_then(|i| i.anticheat.as_ref()) {
        let class = match a.status.as_str() {
            "Supported" | "Running" => "success",
            "Planned" => "warning",
            _ => "error",
        };
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        status.add_css_class(class);
        status.append(&gtk::Image::from_icon_name("security-high-symbolic"));
        status.append(&gtk::Label::new(Some(&a.status)));
        status.set_tooltip_text(Some(&tr(
            "Anti-cheat: {names}",
            &[("names", &a.anticheats.join(", "))],
        )));
        under.append(&status);
    }
    under.add_css_class("caption");
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    text.append(&name);
    text.append(&under);

    // ProtonDB's rating, as a band in its tier's color.
    let rating = info.and_then(|i| i.protondb.as_ref());
    let band = gtk::Box::builder().width_request(4).build();
    band.add_css_class("band");
    match rating {
        Some(r) => {
            band.add_css_class(&format!("tier-{}", r.tier));
            band.set_tooltip_text(Some(&trn(
                "ProtonDB: {tier}, from {n} report",
                "ProtonDB: {tier}, from {n} reports",
                r.total as usize,
                &[("tier", &tier_name(&r.tier))],
            )));
        }
        None => band.add_css_class("tier-none"),
    }
    let content = gtk::Box::builder()
        .spacing(12)
        .margin_top(6)
        .margin_bottom(6)
        .margin_end(12)
        .build();
    content.append(&band);
    content.append(&thumbnail(env, game.art.as_deref(), 192));
    content.append(&text);
    let row = gtk::ListBoxRow::builder()
        .child(&content)
        .activatable(true)
        .name(game.name.to_lowercase())
        .build();

    let (options, labels, selected) = options(launcher.kind, lib, game);
    let tools = gtk::DropDown::builder()
        .model(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .factory(&labels_factory(true))
        .list_factory(&labels_factory(false))
        .valign(gtk::Align::Center)
        .sensitive(!running)
        .tooltip_text(tr("The tool it runs with", &[]))
        .build();
    if let Some(i) = selected {
        tools.set_selected(i as u32);
    }
    let details = gtk::Button::builder()
        .icon_name("info-outline-symbolic")
        .tooltip_text(tr("Details", &[]))
        .valign(gtk::Align::Center)
        .build();
    details.add_css_class("flat");
    details.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        launcher,
        #[strong(rename_to = id)]
        game.id,
        move |b| super::game::open(&win, b, &launcher, &id)
    ));
    let game = game.clone();
    tools.connect_selected_notify(glib::clone!(
        #[strong]
        win,
        #[strong]
        launcher,
        move |d| {
            if let Some(choice) = options.get(d.selected() as usize) {
                win.assign(&launcher, &game, choice.as_ref());
            }
        }
    ));
    content.append(&tools);
    content.append(&details);
    row
}

/// Shows a list of strings as labels; `short` ones all take the same width,
/// ellipsized past it, so the buttons that show them line up.
fn labels_factory(short: bool) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let label = gtk::Label::builder().xalign(0.0).build();
        if short {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_width_chars(20);
            label.set_max_width_chars(20);
        }
        item.downcast_ref::<gtk::ListItem>()
            .expect("a list item")
            .set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        if let (Some(label), Some(text)) = (
            item.child().and_downcast::<gtk::Label>(),
            item.item().and_downcast::<gtk::StringObject>(),
        ) {
            label.set_label(&text.string());
        }
    });
    factory
}

/// A game's picture, from a file or a URL, or an icon where it has none.
/// `width` is what it's decoded to.
pub(super) fn thumbnail(env: &Env, art: Option<&str>, width: i32) -> gtk::Widget {
    let frame = gtk::Overlay::builder()
        .valign(gtk::Align::Center)
        .overflow(gtk::Overflow::Hidden)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    frame.add_css_class("thumbnail");
    frame.set_child(Some(&gtk::Image::from_icon_name(
        "applications-games-symbolic",
    )));
    if let Some(art) = art {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .build();
        frame.add_overlay(&picture);
        let weak = picture.downgrade();
        art::load(&env.client, art, width, move |t| {
            if let Some(p) = weak.upgrade() {
                p.set_paintable(Some(t));
            }
        });
    }
    frame.upcast()
}

/// The tools a game's list offers, their labels, and which is the game's
/// current one.
pub(super) fn options(
    kind: LauncherKind,
    lib: &Library,
    game: &Game,
) -> (Vec<Option<Choice>>, Vec<String>, Option<usize>) {
    // Steam games can go back to the default; Heroic's always name one.
    let mut options: Vec<Option<Choice>> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if kind == LauncherKind::Steam {
        options.push(None);
        labels.push(match &lib.default {
            Some(d) => tr("Default ({tool})", &[("tool", &d.name)]),
            None => tr("Default (none)", &[]),
        });
    }
    for c in &lib.choices {
        options.push(Some(c.clone()));
        labels.push(c.name.clone());
    }
    let current = game.tool.as_ref().map(|t| t.id.as_str());
    let mut selected = options
        .iter()
        .position(|o| match o {
            None => !game.explicit,
            Some(c) => game.explicit && Some(c.id.as_str()) == current,
        })
        .or_else(|| {
            // Heroic games on the default name it like any other tool.
            options
                .iter()
                .position(|o| o.as_ref().map(|c| c.id.as_str()) == current)
        });
    if selected.is_none()
        && let Some(tool) = &game.tool
    {
        // A tool the launcher doesn't list, like a system Wine.
        options.push(Some(tool.clone()));
        labels.push(tool.name.clone());
        selected = Some(options.len() - 1);
    }
    (options, labels, selected)
}

pub(super) fn source(source: &str) -> String {
    match source {
        "steam" => "Steam".into(),
        "shortcut" => tr("Non-Steam game", &[]),
        "epic" => "Epic Games".into(),
        "gog" => "GOG".into(),
        "amazon" => "Amazon".into(),
        "sideload" => tr("Sideloaded", &[]),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: &str) -> Choice {
        Choice {
            id: id.into(),
            name: format!("{id} name"),
            dir: None,
        }
    }

    fn game(tool: Option<&str>, explicit: bool) -> Game {
        Game {
            id: "1".into(),
            name: "G".into(),
            source: "steam",
            tool: tool.map(choice),
            explicit,
            epic_namespace: None,
            art: None,
        }
    }

    fn lib(default: Option<&str>) -> Library {
        Library {
            games: Vec::new(),
            default: default.map(choice),
            choices: vec![choice("a"), choice("b")],
            problems: Vec::new(),
        }
    }

    fn ids(options: &[Option<Choice>]) -> Vec<Option<&str>> {
        options
            .iter()
            .map(|o| o.as_ref().map(|c| c.id.as_str()))
            .collect()
    }

    #[test]
    fn steam_offers_the_default_first() {
        let l = lib(Some("b"));
        let (o, labels, sel) = options(LauncherKind::Steam, &l, &game(Some("b"), false));
        assert_eq!(ids(&o), [None, Some("a"), Some("b")]);
        assert_eq!(labels[0], "Default (b name)");
        assert_eq!(sel, Some(0), "on the default, though it is b too");
        let (_, _, sel) = options(LauncherKind::Steam, &l, &game(Some("b"), true));
        assert_eq!(sel, Some(2), "b chosen for the game itself");
    }

    #[test]
    fn heroic_has_no_default_entry() {
        let l = lib(Some("a"));
        let (o, _, sel) = options(LauncherKind::Heroic, &l, &game(Some("a"), false));
        assert_eq!(ids(&o), [Some("a"), Some("b")]);
        assert_eq!(sel, Some(0), "the default, named like any tool");
    }

    #[test]
    fn an_unlisted_tool_is_added() {
        let (o, labels, sel) = options(
            LauncherKind::Heroic,
            &lib(None),
            &game(Some("/usr/bin/wine"), true),
        );
        assert_eq!(ids(&o).last(), Some(&Some("/usr/bin/wine")));
        assert_eq!(labels.last().unwrap(), "/usr/bin/wine name");
        assert_eq!(sel, Some(2));
        let (_, _, sel) = options(LauncherKind::Heroic, &lib(None), &game(None, false));
        assert_eq!(sel, None, "nothing to show for a game without a tool");
    }
}
