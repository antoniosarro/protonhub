//! Translations, through gettext. Message ids are the English text, with
//! `{name}` placeholders that a translation can move.

use std::path::PathBuf;

use gettextrs::{
    LocaleCategory, bind_textdomain_codeset, bindtextdomain, gettext, ngettext, setlocale,
    textdomain,
};

const DOMAIN: &str = "protonhub";

/// Picks the user's language. Catalogs are in `$PROTONHUB_LOCALEDIR`, or
/// installed with the binary (`<prefix>/share/locale`). Without them, or
/// for a language with none, the text stays English.
/// Call it first thing in `main`.
pub fn init() {
    // SAFETY: setlocale races with other threads reading the locale; this
    // runs first in main, before GTK or anything else starts a thread.
    unsafe {
        setlocale(LocaleCategory::LcAll, "");
    }
    let dir = std::env::var_os("PROTONHUB_LOCALEDIR")
        .map(PathBuf::from)
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            Some(exe.parent()?.parent()?.join("share/locale")).filter(|d| d.is_dir())
        });
    // A failure here only leaves the English text.
    if let Some(dir) = dir {
        let _ = bindtextdomain(DOMAIN, dir);
    }
    let _ = bind_textdomain_codeset(DOMAIN, "UTF-8");
    let _ = textdomain(DOMAIN);
}

/// `msgid` translated, with each `{name}` replaced by its value.
pub fn tr(msgid: &str, args: &[(&str, &str)]) -> String {
    fill(gettext(msgid), args)
}

/// The singular or plural of a message for `n`, translated; `{n}` is `n`.
pub fn trn(one: &str, many: &str, n: usize, args: &[(&str, &str)]) -> String {
    let count = n.to_string();
    let text = fill(ngettext(one, many, n as u32), args);
    fill(text, &[("n", &count)])
}

fn fill(mut text: String, args: &[(&str, &str)]) -> String {
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

#[cfg(test)]
mod tests {
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
}
