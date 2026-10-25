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
        deck: None,
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

fn ids(choices: &[Option<Choice>]) -> Vec<Option<&str>> {
    choices
        .iter()
        .map(|o| o.as_ref().map(|c| c.id.as_str()))
        .collect()
}

#[test]
fn steam_offers_the_default_first() {
    let l = lib(Some("b"));
    let o = for_game(LauncherKind::Steam, &l, &game(Some("b"), false));
    assert_eq!(ids(&o.choices), [None, Some("a"), Some("b")]);
    assert_eq!(o.labels[0], "Default (b name)");
    assert_eq!(o.selected, Some(0), "on the default, though it is b too");
    let o = for_game(LauncherKind::Steam, &l, &game(Some("b"), true));
    assert_eq!(o.selected, Some(2), "b chosen for the game itself");
}

#[test]
fn heroic_has_no_default_entry() {
    let l = lib(Some("a"));
    let o = for_game(LauncherKind::Heroic, &l, &game(Some("a"), false));
    assert_eq!(ids(&o.choices), [Some("a"), Some("b")]);
    assert_eq!(o.selected, Some(0), "the default, named like any tool");
}

#[test]
fn an_unlisted_tool_is_added() {
    let o = for_game(
        LauncherKind::Heroic,
        &lib(None),
        &game(Some("/usr/bin/wine"), true),
    );
    assert_eq!(ids(&o.choices).last(), Some(&Some("/usr/bin/wine")));
    assert_eq!(o.labels.last().unwrap(), "/usr/bin/wine name");
    assert_eq!(o.selected, Some(2));
    let o = for_game(LauncherKind::Heroic, &lib(None), &game(None, false));
    assert_eq!(
        o.selected, None,
        "nothing to show for a game without a tool"
    );
}

#[test]
fn steam_default_can_be_none_and_keeps_a_missing_one() {
    let mut l = lib(Some("gone"));
    let o = for_default(LauncherKind::Steam, &l);
    assert_eq!(ids(&o.choices), [None, Some("a"), Some("b"), Some("gone")]);
    assert_eq!(o.selected, Some(3));
    l.default = None;
    let o = for_default(LauncherKind::Faugus, &l);
    assert_eq!(ids(&o.choices), [Some("a"), Some("b")]);
    assert_eq!(o.selected, None);
}

#[test]
fn in_use_counts_games_with_a_tool_of_their_own() {
    let mut l = lib(Some("a"));
    l.games = vec![
        game(Some("a"), true),
        game(Some("a"), true),
        game(Some("b"), true),
        game(Some("a"), false),
    ];
    let used = in_use(&l);
    let labels: Vec<_> = used.iter().map(|(_, l)| l.as_str()).collect();
    assert_eq!(labels, ["a name (2 games)", "b name (1 game)"]);
}
