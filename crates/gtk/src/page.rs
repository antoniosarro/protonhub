//! One launcher's page: its games, each with the tool it runs with, and
//! its tools: what's installed, and what can be.

use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::games::{self, Choice, Game, Library, Usage};
use protonhub_core::info::Info;
use protonhub_core::install::human_size;
use protonhub_core::launcher::Launcher;
use protonhub_core::manifest::{LauncherKind, Tool};

use crate::i18n::{tr, trn};
use crate::window::{Env, LauncherState, Window, capitalized, named};
use crate::{art, details, errors, tool};

pub fn build(win: &Rc<Window>, env: &Env, state: &LauncherState) -> gtk::Widget {
    let launcher = &state.launcher;
    let view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    view.add_top_bar(&header);

    let library = state.library.as_ref().map(Option::as_ref);
    if library.is_ok_and(|l| l.is_some()) {
        if state.running {
            view.add_top_bar(&banner(&tr(
                "{launcher} is running: quit it to change a game's tool, or it will undo the change",
                &[("launcher", &launcher.title())],
            )));
        } else if !games::sees_launchers() {
            view.add_top_bar(&banner(&tr(
                "Quit {launcher} before changing a game's tool: from inside a Flatpak, ProtonHub can't tell whether it's running",
                &[("launcher", &launcher.title())],
            )));
        }
    }

    let stack = adw::ViewStack::new();
    match library {
        Ok(Some(lib)) => {
            stack.add_titled_with_icon(
                &games_view(win, env, launcher, lib, state),
                Some("games"),
                &tr("Games", &[]),
                "applications-games-symbolic",
            );
        }
        Ok(None) => {}
        Err(e) => {
            stack.add_titled_with_icon(
                &failed(&tr("Can't read the games", &[]), e),
                Some("games"),
                &tr("Games", &[]),
                "applications-games-symbolic",
            );
        }
    }
    let tools = match &state.usage {
        Ok(usage) => tools_view(win, env, state, usage),
        Err(e) => failed(&tr("Can't read the tool directories", &[]), e),
    };
    stack.add_titled_with_icon(
        &tools,
        Some("tools"),
        &tr("Tools", &[]),
        "applications-utilities-symbolic",
    );

    if stack.pages().n_items() > 1 {
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        header.set_title_widget(Some(&switcher));
        if let Some(name) = win.view.borrow().as_deref() {
            stack.set_visible_child_name(name);
        }
        stack.connect_visible_child_name_notify(glib::clone!(
            #[strong]
            win,
            move |s| {
                win.view.replace(s.visible_child_name().map(Into::into));
            }
        ));
    } else {
        header.set_title_widget(Some(&adw::WindowTitle::new(&launcher.title(), "")));
    }
    view.set_content(Some(&stack));
    view.upcast()
}

fn banner(text: &str) -> adw::Banner {
    adw::Banner::builder().title(text).revealed(true).build()
}

fn failed(what: &str, message: &str) -> gtk::Widget {
    adw::StatusPage::builder()
        .icon_name("dialog-error-symbolic")
        .title(what)
        .description(glib::markup_escape_text(message))
        .build()
        .upcast()
}

/// A view's content: scrolled, and no wider than reads well.
fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let clamp = adw::Clamp::builder()
        .maximum_size(880)
        .tightening_threshold(600)
        .child(child)
        .margin_top(18)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .vexpand(true)
        .build()
        .upcast()
}

/// A small rounded label, colored by `class`.
pub fn pill(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .valign(gtk::Align::Center)
        .build();
    label.add_css_class("pill");
    if !class.is_empty() {
        label.add_css_class(class);
    }
    label
}

/// Each game with its picture, ratings, and a list of the tools it can
/// run with, the current one chosen; choosing another sets it.
fn games_view(
    win: &Rc<Window>,
    env: &Env,
    launcher: &Launcher,
    lib: &Library,
    state: &LauncherState,
) -> gtk::Widget {
    let running = state.running;
    if lib.games.is_empty() && lib.problems.is_empty() {
        return adw::StatusPage::builder()
            .icon_name("applications-games-symbolic")
            .title(tr("No games installed", &[]))
            .description(tr(
                "Games installed with {launcher} show up here",
                &[("launcher", &launcher.title())],
            ))
            .build()
            .upcast();
    }
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);

    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr("Search games", &[]))
        .hexpand(true)
        .build();
    let move_games = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("media-playlist-shuffle-symbolic")
                .label(tr("Move Games…", &[]))
                .can_shrink(true)
                .build(),
        )
        .tooltip_text(tr("Move every game set to one tool to another", &[]))
        .sensitive(!running && lib.games.iter().any(|g| g.explicit))
        .build();
    move_games.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        launcher,
        #[strong]
        lib,
        move |_| move_dialog(&win, &launcher, &lib)
    ));
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bar.append(&search);
    bar.append(&move_games);
    content.append(&bar);
    if state.info.iter().any(|(_, i)| i.protondb.is_some()) {
        content.append(&legend());
    }

    if let Some(e) = &state.info_error {
        // Not an error of ProtonHub's: a source was down or broken.
        let row = adw::ActionRow::builder()
            .title(tr("Some ratings or anti-cheat statuses are missing", &[]))
            .subtitle(glib::markup_escape_text(e))
            .subtitle_lines(1)
            .tooltip_text(e)
            .build();
        let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        icon.add_css_class("warning");
        row.add_prefix(&icon);
        content.append(&boxed(&row));
    }
    for problem in &lib.problems {
        content.append(&boxed(&errors::row(
            &tr("Left out: a file couldn't be read", &[]),
            problem,
        )));
    }

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    let none = adw::ActionRow::builder()
        .title(tr("No games match", &[]))
        .build();
    none.add_css_class("dim-label");
    list.set_placeholder(Some(&none));
    for game in &lib.games {
        let info = state
            .info
            .iter()
            .find(|(id, _)| *id == game.id)
            .map(|(_, i)| i);
        list.append(&game_row(win, env, launcher, lib, game, info, running));
    }
    list.set_filter_func(glib::clone!(
        #[weak]
        search,
        #[upgrade_or]
        true,
        move |row| {
            let query = search.text().to_lowercase();
            query.is_empty() || row.widget_name().contains(query.trim())
        }
    ));
    search.connect_search_changed(glib::clone!(
        #[weak]
        list,
        move |_| list.invalidate_filter()
    ));
    content.append(&list);
    scrolled(&content)
}

/// What the colors of the bands at the start of the game rows mean.
fn legend() -> gtk::Box {
    let legend = gtk::Box::builder()
        .spacing(14)
        .halign(gtk::Align::End)
        .build();
    legend.add_css_class("caption");
    let title = gtk::Label::new(Some(&tr("ProtonDB rating:", &[])));
    title.add_css_class("dim-label");
    legend.append(&title);
    for tier in ["platinum", "gold", "silver", "bronze", "borked"] {
        let dot = gtk::Box::builder()
            .width_request(10)
            .height_request(10)
            .valign(gtk::Align::Center)
            .build();
        dot.add_css_class("dot");
        dot.add_css_class(&format!("tier-{tier}"));
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        item.append(&dot);
        item.append(&gtk::Label::new(Some(&tier_name(tier))));
        legend.append(&item);
    }
    legend
}

/// A ProtonDB tier as people read it.
fn tier_name(tier: &str) -> String {
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

/// A boxed list of one row.
fn boxed(row: &impl IsA<gtk::Widget>) -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    list.append(row);
    list
}

fn game_row(
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
    content.append(&thumbnail(env, game));
    content.append(&text);
    let row = gtk::ListBoxRow::builder()
        .child(&content)
        .activatable(false)
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
    let links = links(win, game);
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
    if let Some(links) = links {
        content.append(&links);
    }
    content.append(&tools);
    row
}

/// A menu of a Steam game's pages on the web: ProtonDB's reports, SteamDB
/// and the store; none for other games, which those sites don't know.
fn links(win: &Rc<Window>, game: &Game) -> Option<gtk::MenuButton> {
    if game.source != "steam" {
        return None;
    }
    let id = &game.id;
    let menu = gio::Menu::new();
    for (label, uri) in [
        (
            tr("ProtonDB Reports", &[]),
            format!("https://www.protondb.com/app/{id}"),
        ),
        (
            tr("SteamDB", &[]),
            format!("https://steamdb.info/app/{id}/"),
        ),
        (
            tr("Steam Store", &[]),
            format!("https://store.steampowered.com/app/{id}/"),
        ),
    ] {
        let item = gio::MenuItem::new(Some(&label), None);
        item.set_action_and_target_value(Some("game.open"), Some(&uri.to_variant()));
        menu.append_item(&item);
    }
    let open = gio::SimpleAction::new("open", Some(glib::VariantTy::STRING));
    open.connect_activate(glib::clone!(
        #[strong]
        win,
        move |_, uri| {
            if let Some(uri) = uri.and_then(|u| u.get::<String>()) {
                gtk::UriLauncher::new(&uri).launch(
                    Some(&win.window),
                    None::<&gio::Cancellable>,
                    |_| {},
                );
            }
        }
    ));
    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&open);
    let button = gtk::MenuButton::builder()
        .icon_name("web-browser-symbolic")
        .tooltip_text(tr("Open on the web", &[]))
        .menu_model(&menu)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    button.insert_action_group("game", Some(&actions));
    Some(button)
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

/// A game's picture, or an icon where it has none.
fn thumbnail(env: &Env, game: &Game) -> gtk::Widget {
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
    if let Some(art) = &game.art {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .build();
        frame.add_overlay(&picture);
        let weak = picture.downgrade();
        art::load(&env.client, art, 192, move |t| {
            if let Some(p) = weak.upgrade() {
                p.set_paintable(Some(t));
            }
        });
    }
    frame.upcast()
}

/// The tools: a cleanup of the unused ones, what's installed, and what
/// can be.
fn tools_view(win: &Rc<Window>, env: &Env, state: &LauncherState, usage: &[Usage]) -> gtk::Widget {
    let launcher = &state.launcher;
    let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    if let Some(group) = unused_group(win, usage) {
        content.append(&group);
    }
    content.append(&installed_group(win, env, launcher, usage));
    if let Some(group) = officials_group(win, env, state) {
        content.append(&group);
    }
    content.append(&available_group(win, env, launcher, usage));
    scrolled(&content)
}

/// How much the tools nothing uses take, and a button to delete them.
fn unused_group(win: &Rc<Window>, usage: &[Usage]) -> Option<adw::PreferencesGroup> {
    let unused: Vec<_> = usage.iter().filter(|u| u.unused()).collect();
    if unused.is_empty() {
        return None;
    }
    let total: u64 = unused.iter().map(|u| u.size).sum();
    let names: Vec<String> = unused.iter().map(|u| dir_name(&u.entry.dir)).collect();
    let row = adw::ActionRow::builder()
        .title(trn(
            "{n} unused tool takes {size}",
            "{n} unused tools take {size}",
            unused.len(),
            &[("size", &human_size(total))],
        ))
        .subtitle(tr("No game in any launcher runs it", &[]))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("user-trash-symbolic"));
    let clean = gtk::Button::builder()
        .label(tr("Clean Up…", &[]))
        .valign(gtk::Align::Center)
        .sensitive(!win.busy())
        .build();
    clean.add_css_class("destructive-action");
    let dirs: Vec<_> = unused.iter().map(|u| u.entry.dir.clone()).collect();
    clean.connect_clicked(glib::clone!(
        #[strong]
        win,
        move |_| confirm_cleanup(&win, &names, dirs.clone(), total)
    ));
    row.add_suffix(&clean);
    let group = adw::PreferencesGroup::new();
    group.add(&row);
    Some(group)
}

/// Every tool in the launcher's tool directories, ProtonHub's or not,
/// with its size and what uses it.
fn installed_group(
    win: &Rc<Window>,
    env: &Env,
    launcher: &Launcher,
    usage: &[Usage],
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Installed", &[]))
        .build();
    if usage.is_empty() {
        let row = adw::ActionRow::builder()
            .title(tr("No tools installed yet", &[]))
            .subtitle(tr("Install one from the list below", &[]))
            .build();
        row.add_css_class("dim-label");
        group.add(&row);
        return group;
    }
    let total: u64 = usage.iter().map(|u| u.size).sum();
    group.set_description(Some(&trn(
        "{n} tool · {size}",
        "{n} tools · {size}",
        usage.len(),
        &[("size", &human_size(total))],
    )));
    let mut sorted: Vec<_> = usage.iter().collect();
    sorted.sort_by_key(|u| dir_name(&u.entry.dir).to_lowercase());
    for u in sorted {
        let record = u.entry.record.as_ref();
        let tool = match record {
            Some(r) => env.registry.tools.get(&r.tool),
            None => guess(env, launcher, &u.entry.dir),
        };
        let dir = dir_name(&u.entry.dir);
        let name = match (tool, record) {
            (Some(t), Some(r)) => named(&t.name, &r.version),
            (Some(t), None) if !simple(&dir).contains(&simple(&t.name)) => named(&t.name, &dir),
            _ => dir.clone(),
        };
        let family = tool.map_or(name.as_str(), |t| t.name.as_str());
        let avatar = art::tool_avatar(&env.client, tool, family, 36);
        group.add(&usage_row(
            win,
            u,
            &name,
            &avatar,
            tool,
            details::Removal::Folder,
        ));
    }
    group
}

/// An installed tool: its size and what uses it, marked Default or Unused,
/// and a button to its details.
fn usage_row(
    win: &Rc<Window>,
    u: &Usage,
    name: &str,
    avatar: &adw::Avatar,
    tool: Option<&Tool>,
    removal: details::Removal,
) -> adw::ActionRow {
    let mut subtitle = vec![human_size(u.size)];
    // The Unused mark says that already.
    subtitle.extend(users(u).filter(|_| !u.unused()));
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(name))
        .subtitle(glib::markup_escape_text(&subtitle.join(" · ")))
        .tooltip_text(u.entry.dir.display().to_string())
        .build();
    row.add_prefix(avatar);
    if u.users.as_ref().is_some_and(|u| u.default) {
        row.add_suffix(&pill(&tr("Default", &[]), "accent"));
    } else if u.unused() {
        row.add_suffix(&pill(&tr("Unused", &[]), "warning"));
    }
    let info = gtk::Button::builder()
        .icon_name("info-outline-symbolic")
        .tooltip_text(tr("Details", &[]))
        .valign(gtk::Align::Center)
        .build();
    info.add_css_class("flat");
    row.add_suffix(&info);
    row.set_activatable_widget(Some(&info));
    let (u, name, tool) = (u.clone(), name.to_owned(), tool.cloned());
    info.connect_clicked(glib::clone!(
        #[strong]
        win,
        move |_| {
            if let Ok(env) = &win.env {
                details::show(&win, env, &u, &name, tool.as_ref(), removal.clone());
            }
        }
    ));
    row
}

/// Valve's Protons, which Steam installs as apps: each with its size, what
/// uses it, and its details, where it can be uninstalled.
fn officials_group(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
) -> Option<adw::PreferencesGroup> {
    let group = adw::PreferencesGroup::builder()
        .title(tr("From Steam", &[]))
        .description(tr("Valve's Protons, which Steam installs and updates", &[]))
        .build();
    let found = match &state.officials {
        Ok(found) if found.is_empty() => return None,
        Ok(found) => found,
        Err(e) => {
            group.add(&errors::row(&tr("Can't read Steam's Protons", &[]), e));
            return Some(group);
        }
    };
    for o in found {
        let usage = Usage {
            entry: protonhub_core::install::Entry {
                dir: o.official.dir.clone(),
                record: None,
            },
            size: o.size,
            users: Some(o.users.clone()),
        };
        let avatar = art::avatar(&env.client, "Proton", Some(art::VALVE.into()), 36);
        let removal = details::Removal::Steam {
            official: o.official.clone(),
            launcher: state.launcher.clone(),
            running: state.running,
        };
        group.add(&usage_row(
            win,
            &usage,
            &o.official.name,
            &avatar,
            None,
            removal,
        ));
    }
    Some(group)
}

/// The registry's tools this launcher takes; each opens its releases.
fn available_group(
    win: &Rc<Window>,
    env: &Env,
    launcher: &Launcher,
    usage: &[Usage],
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Available", &[]))
        .description(tr("Install a release, or keep a tool up to date", &[]))
        .build();
    let channels = win.channels();
    for tool in env.registry.tools.values() {
        let Some(dir) = launcher.tool_dir(tool) else {
            continue;
        };
        let here = installed(usage, &dir, tool);
        let mut subtitle = vec![tool.source.origin().to_owned()];
        if here > 0 {
            subtitle.push(trn("{n} installed", "{n} installed", here, &[]));
        }
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&tool.name))
            .subtitle(glib::markup_escape_text(&subtitle.join(" · ")))
            .activatable(true)
            .build();
        row.add_prefix(&art::tool_avatar(&env.client, Some(tool), &tool.name, 36));
        if channels
            .list
            .iter()
            .any(|c| c.tool == tool.id && launcher.matches(&c.launcher))
        {
            row.add_suffix(&pill(&tr("Auto-update", &[]), "accent"));
        }
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.connect_activated(glib::clone!(
            #[strong]
            win,
            #[strong]
            launcher,
            #[strong]
            tool,
            move |_| tool::show(&win, &tool, &launcher)
        ));
        group.add(&row);
    }
    group
}

/// The registry's tool a directory ProtonHub didn't install is a version
/// of: the only one that goes where it is, the one its name starts with,
/// or the one the others there are variants of (DXVK, for DXVK GPL-Async).
fn guess<'a>(env: &'a Env, launcher: &Launcher, dir: &Path) -> Option<&'a Tool> {
    let here: Vec<_> = env
        .registry
        .tools
        .values()
        .filter(|t| launcher.tool_dir(t).as_deref() == dir.parent())
        .collect();
    if let [only] = here[..] {
        return Some(only);
    }
    let name = simple(&dir_name(dir));
    let named = here
        .iter()
        .filter(|t| name.starts_with(&simple(&t.name)))
        .max_by_key(|t| t.name.len());
    named
        .or_else(|| {
            here.iter().find(|base| {
                here.iter()
                    .all(|t| simple(&t.name).starts_with(&simple(&base.name)))
            })
        })
        .copied()
}

/// A name with only its letters and digits, lowercased, to compare.
fn simple(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// How many versions of `tool` ProtonHub installed in `dir`.
fn installed(usage: &[Usage], dir: &Path, tool: &Tool) -> usize {
    usage
        .iter()
        .filter(|u| u.entry.dir.parent() == Some(dir))
        .filter(|u| u.entry.record.as_ref().is_some_and(|r| r.tool == tool.id))
        .count()
}

/// Moves every game set to one tool to another: the tools games are set
/// to on the left, every tool on the right.
fn move_dialog(win: &Rc<Window>, launcher: &Launcher, lib: &Library) {
    let mut froms: Vec<(Choice, usize)> = Vec::new();
    for tool in lib
        .games
        .iter()
        .filter(|g| g.explicit)
        .filter_map(|g| g.tool.as_ref())
    {
        match froms.iter_mut().find(|(c, _)| c.id == tool.id) {
            Some((_, n)) => *n += 1,
            None => froms.push((tool.clone(), 1)),
        }
    }
    let labels: Vec<String> = froms
        .iter()
        .map(|(c, n)| {
            trn(
                "{tool} ({n} game)",
                "{tool} ({n} games)",
                *n,
                &[("tool", &c.name)],
            )
        })
        .collect();
    let from = adw::ComboRow::builder()
        .title(tr("From", &[]))
        .model(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .build();
    let to_labels: Vec<&str> = lib.choices.iter().map(|c| c.name.as_str()).collect();
    let to = adw::ComboRow::builder()
        .title(tr("To", &[]))
        .model(&gtk::StringList::new(&to_labels))
        .build();
    let group = adw::PreferencesGroup::builder()
        .description(tr("Games on the launcher's default stay on it", &[]))
        .build();
    group.add(&from);
    group.add(&to);
    let page = adw::PreferencesPage::new();
    page.add(&group);

    let go = gtk::Button::builder().label(tr("Move", &[])).build();
    go.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&go);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(tr("Move Games", &[]))
        .content_width(460)
        .child(&view)
        .build();
    let choices = lib.choices.clone();
    go.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        launcher,
        #[weak]
        dialog,
        move |_| {
            let (Some((f, _)), Some(t)) = (
                froms.get(from.selected() as usize),
                choices.get(to.selected() as usize),
            ) else {
                return;
            };
            dialog.close();
            win.reassign(&launcher, f, t);
        }
    ));
    dialog.present(Some(&win.window));
}

/// The tools a game's list offers, their labels, and which is the game's
/// current one.
fn options(
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

/// Who uses a tool, in a few words; none where nobody can tell.
fn users(u: &Usage) -> Option<String> {
    let users = u.users.as_ref()?;
    let games = match &users.games[..] {
        [] => None,
        [one] => Some(one.clone()),
        many => Some(trn("{n} game", "{n} games", many.len(), &[])),
    };
    Some(match (users.default, games) {
        (false, None) => tr("unused", &[]),
        (true, None) => tr("used by the default", &[]),
        (false, Some(g)) => tr("used by {games}", &[("games", &g)]),
        (true, Some(g)) => tr("used by the default and {games}", &[("games", &g)]),
    })
}

fn source(source: &str) -> String {
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

fn confirm_cleanup(win: &Rc<Window>, names: &[String], dirs: Vec<std::path::PathBuf>, total: u64) {
    let dialog = adw::AlertDialog::new(
        Some(&trn(
            "Delete {n} unused tool?",
            "Delete {n} unused tools?",
            names.len(),
            &[],
        )),
        Some(&tr(
            "This frees {size}:\n\n{tools}",
            &[("size", &human_size(total)), ("tools", &names.join("\n"))],
        )),
    );
    dialog.add_responses(&[
        ("cancel", &tr("Cancel", &[])),
        ("delete", &tr("Delete", &[])),
    ]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.connect_response(
        Some("delete"),
        glib::clone!(
            #[strong]
            win,
            move |_, _| win.clean_up(dirs.clone(), total)
        ),
    );
    dialog.present(Some(&win.window));
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// `path` with the home directory shown as `~`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
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
