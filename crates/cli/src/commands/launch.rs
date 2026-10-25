//! A game's launch options and presets, and its Wine prefix with
//! winetricks.

use protonhub_core::launch;
use protonhub_core::prefix;
use serde::Serialize;

use crate::common::*;

pub fn launch(
    cx: &Cx,
    game: String,
    set: Option<String>,
    on: Vec<String>,
    off: Vec<String>,
    launcher: Option<String>,
) -> Result<()> {
    let (target, g) = locate(launcher.as_deref(), &game)?;
    let current = launch::all(&target)?
        .ok_or_else(|| format!("ProtonHub can't read {target}'s launch options yet"))?
        .into_iter()
        .find(|(id, _)| *id == g.id)
        .map(|(_, o)| o)
        .unwrap_or_default();
    let presets = launch::presets();
    let changed = match set {
        Some(text) => Some(text),
        None if on.is_empty() && off.is_empty() => None,
        None => {
            let mut o = launch::Options::parse(&current).ok_or_else(|| {
                format!(
                    "{}'s launch options have an unclosed quote; replace them with --set",
                    g.name
                )
            })?;
            for (ids, state) in [(&on, true), (&off, false)] {
                for id in ids {
                    let p = presets
                        .iter()
                        .find(|p| p.id == *id)
                        .ok_or_else(|| format!("no preset {id}; `protonhub presets` lists them"))?;
                    o.set(p, state);
                }
            }
            Some(o.to_string())
        }
    };
    if let Some(text) = &changed {
        warn_unseen(&target);
        launch::set(&target, &g.id, text)?;
    }
    let options = changed.unwrap_or(current);
    let active: Vec<&str> = launch::Options::parse(&options)
        .map(|o| {
            presets
                .iter()
                .filter(|p| o.has(p))
                .map(|p| p.id.as_str())
                .collect()
        })
        .unwrap_or_default();
    #[derive(Serialize)]
    struct Out<'a> {
        game: &'a str,
        options: &'a str,
        presets: &'a [&'a str],
    }
    let shown = if options.is_empty() { "-" } else { &options };
    let text = format!("options\t{shown}\npresets\t{}", active.join(", "));
    cx.print(
        text,
        &Out {
            game: &g.id,
            options: &options,
            presets: &active,
        },
    )
}

pub fn presets(cx: &Cx) -> Result<()> {
    let presets = launch::presets();
    let text: Vec<_> = presets
        .iter()
        .map(|p| {
            let mut sets: Vec<_> = p.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
            sets.extend(p.wrapper.clone());
            format!(
                "{}\t{}\t{}\t{}",
                p.id,
                p.name,
                sets.join(" "),
                p.description
            )
        })
        .collect();
    cx.print(text.join("\n"), &presets)
}

pub fn tricks(
    cx: &Cx,
    game: String,
    verbs: Vec<String>,
    run: Option<String>,
    launcher: Option<String>,
) -> Result<()> {
    let (target, g) = locate(launcher.as_deref(), &game)?;
    let prefix = prefix::find(&target, &g)?;
    if let Some(bad) = verbs.iter().find(|v| !prefix::is_verb(v)) {
        return Err(format!("{bad} isn't a winetricks verb").into());
    }
    // Straight to the terminal, where installers can ask and Ctrl+C
    // reaches them.
    let cmd = match run.as_deref() {
        Some(tool @ ("winecfg" | "regedit" | "control")) => Some(prefix.wine_tool(tool)?),
        Some(path) if std::path::Path::new(path).is_file() => {
            Some(prefix.program(&std::path::absolute(path)?)?)
        }
        Some(path) => return Err(format!("{path}: no such program").into()),
        None if verbs.is_empty() => None,
        None => Some(prefix.winetricks(&verbs)?),
    };
    if let Some(mut cmd) = cmd {
        let status = cmd.status()?;
        if !status.success() {
            return Err(format!("{} ended with {status}", cmd.get_program().display()).into());
        }
    }
    #[derive(Serialize)]
    struct Out<'a> {
        prefix: &'a std::path::Path,
        wine: &'a std::path::Path,
        installed: Vec<String>,
    }
    let out = Out {
        prefix: &prefix.dir,
        wine: &prefix.wine,
        installed: prefix.installed(),
    };
    let text = format!(
        "prefix\t{}\nwine\t{}\ninstalled\t{}",
        out.prefix.display(),
        out.wine.display(),
        if out.installed.is_empty() {
            "-".to_owned()
        } else {
            out.installed.join(" ")
        }
    );
    cx.print(text, &out)
}

pub fn verbs(cx: &Cx) -> Result<()> {
    let groups = prefix::verbs();
    let text: Vec<_> = groups
        .iter()
        .flat_map(|g| {
            g.verb.iter().map(|v| {
                format!(
                    "{}\t{}\t{}\t{}",
                    v.id,
                    g.title,
                    v.name,
                    v.description.as_deref().unwrap_or("")
                )
            })
        })
        .collect();
    cx.print(text.join("\n"), &groups)
}
