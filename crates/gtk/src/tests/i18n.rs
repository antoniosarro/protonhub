use super::*;

#[test]
fn fills_placeholders() {
    assert_eq!(
        tr(
            "{game} now runs with {tool}",
            &[("game", "Celeste"), ("tool", "GE-Proton11-7")]
        ),
        "Celeste now runs with GE-Proton11-7"
    );
    assert_eq!(trn("{n} game", "{n} games", 1, &[]), "1 game");
    assert_eq!(trn("{n} game", "{n} games", 4, &[]), "4 games");
}
