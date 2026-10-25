use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

/// CPU architectures a tool can ship builds for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    /// The architecture this binary was built for, if a tool can target it.
    pub fn host() -> Option<Self> {
        match std::env::consts::ARCH {
            "x86_64" => Some(Self::X86_64),
            "aarch64" => Some(Self::Aarch64),
            _ => None,
        }
    }
}

/// Launchers a tool can be installed into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LauncherKind {
    Steam,
    Heroic,
    Lutris,
    Bottles,
    Winezgui,
}

impl LauncherKind {
    /// The name manifests use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Steam => "steam",
            Self::Heroic => "heroic",
            Self::Lutris => "lutris",
            Self::Bottles => "bottles",
            Self::Winezgui => "winezgui",
        }
    }

    /// The name people know it by.
    pub fn title(self) -> &'static str {
        match self {
            Self::Steam => "Steam",
            Self::Heroic => "Heroic",
            Self::Lutris => "Lutris",
            Self::Bottles => "Bottles",
            Self::Winezgui => "WineZGUI",
        }
    }
}

/// Where a tool's releases are published; each release is a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// `owner/repo` on GitHub.
    Github(String),
    /// A Forgejo or Gitea repository by its URL, like
    /// `https://codeberg.org/luxtorpeda/luxtorpeda`.
    Forgejo(String),
    /// A GitLab project by its URL, like `https://gitlab.com/group/project`.
    Gitlab(String),
    /// One archive at a fixed URL, as a single version: a branch snapshot.
    Url { url: String, version: String },
}

impl Source {
    /// Where the releases are, in a few words: `owner/repo` on GitHub, else
    /// the URL without its scheme.
    pub fn origin(&self) -> &str {
        match self {
            Self::Github(repo) => repo,
            Self::Forgejo(url) | Self::Gitlab(url) | Self::Url { url, .. } => {
                url.split_once("://").map_or(url, |(_, rest)| rest)
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    github: Option<String>,
    forgejo: Option<String>,
    gitlab: Option<String>,
    url: Option<String>,
    version: Option<String>,
}

impl RawSource {
    fn compile(self) -> Result<Source, &'static str> {
        let web = |u: &str| u.starts_with("https://") || u.starts_with("http://");
        match (
            self.github,
            self.forgejo,
            self.gitlab,
            self.url,
            self.version,
        ) {
            (Some(repo), None, None, None, None) if repo.split('/').count() == 2 => {
                Ok(Source::Github(repo))
            }
            (None, Some(u), None, None, None) if web(&u) => {
                Ok(Source::Forgejo(u.trim_end_matches('/').into()))
            }
            (None, None, Some(u), None, None) if web(&u) => {
                Ok(Source::Gitlab(u.trim_end_matches('/').into()))
            }
            (None, None, None, Some(url), Some(version)) if web(&url) => {
                Ok(Source::Url { url, version })
            }
            _ => Err(
                "source must be one of { github = \"owner/repo\" }, { forgejo = URL }, { gitlab = URL }, or { url = URL, version = NAME }",
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Algo {
    Sha256,
    Sha512,
}

/// How to find an asset's checksum among the release's other assets.
///
/// A pattern with `{asset}` or `{asset_stem}` names a file per asset; a
/// fixed name is one list of every asset's checksum, which must then name
/// the asset (a lone hash there could be any asset's).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checksum {
    /// The checksum file's exact name, with `{asset}` (the asset's name) and
    /// `{asset_stem}` (its name without the archive extension) filled in.
    pub pattern: String,
    pub algo: Algo,
}

impl Checksum {
    /// Whether each asset has a checksum file of its own.
    pub fn per_asset(&self) -> bool {
        self.pattern.contains("{asset")
    }
}

/// One tool, as its manifest describes it, with the asset patterns compiled.
#[derive(Debug, Clone)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub source: Source,
    /// Which of the source's releases are versions of this tool; all of
    /// them without one. A repository can publish several tools, like
    /// wine-ge-custom's regular and LoL builds.
    pub tag: Option<Regex>,
    /// Exactly one asset of a release must match the pattern for an arch.
    pub asset: BTreeMap<Arch, Regex>,
    pub checksum: Option<Checksum>,
    /// The directory to install into, for archives without exactly one
    /// top-level directory, or to rename it; `{tag}` is the release's tag.
    pub dir: Option<String>,
    /// With `dir`: the directory inside the archive's top-level one that is
    /// the tool, for archives that hold more (DXVK-Sarek's hold its source,
    /// with the tool in `build`).
    pub subdir: Option<String>,
    /// Programs the tool needs on the host, which it doesn't bring.
    pub requires: Vec<String>,
    /// The directory, relative to the launcher's data directory, the tool is
    /// extracted into.
    pub launchers: BTreeMap<LauncherKind, PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTool {
    name: String,
    source: RawSource,
    tag: Option<String>,
    /// Keyed by arch, or `any` for every arch.
    asset: BTreeMap<String, String>,
    checksum: Option<Checksum>,
    dir: Option<String>,
    subdir: Option<String>,
    #[serde(default)]
    requires: Vec<String>,
    launchers: BTreeMap<LauncherKind, PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
    #[error("{path}: tool {tool}: {field} pattern: {source}")]
    Pattern {
        path: PathBuf,
        tool: String,
        field: String,
        source: regex::Error,
    },
    #[error("{path}: tool {tool}: {reason}")]
    Invalid {
        path: PathBuf,
        tool: String,
        reason: String,
    },
    #[error("{path}: tool {tool} is already defined in {first}")]
    Duplicate {
        path: PathBuf,
        tool: String,
        first: PathBuf,
    },
}

/// Every tool, keyed by id (the manifest's table name).
#[derive(Debug, Default)]
pub struct Registry {
    pub tools: BTreeMap<String, Tool>,
    origin: BTreeMap<String, PathBuf>,
}

impl Registry {
    /// Loads every `*.toml` in `dir`, in name order.
    pub fn load_dir(dir: &Path) -> Result<Self, LoadError> {
        let io = |source| LoadError::Io {
            path: dir.to_owned(),
            source,
        };
        let mut paths = std::fs::read_dir(dir)
            .map_err(io)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(io)?;
        paths.retain(|p| p.extension().is_some_and(|e| e == "toml"));
        paths.sort();

        let mut registry = Self::default();
        for path in paths {
            let text = std::fs::read_to_string(&path).map_err(|source| LoadError::Io {
                path: path.clone(),
                source,
            })?;
            registry.add(&path, &text)?;
        }
        Ok(registry)
    }

    /// Adds the tools in one manifest file; `path` is only for errors.
    pub fn add(&mut self, path: &Path, text: &str) -> Result<(), LoadError> {
        let raw: BTreeMap<String, RawTool> =
            toml::from_str(text).map_err(|source| LoadError::Parse {
                path: path.to_owned(),
                source: Box::new(source),
            })?;
        for (id, raw) in raw {
            if let Some(first) = self.origin.get(&id) {
                return Err(LoadError::Duplicate {
                    path: path.to_owned(),
                    tool: id,
                    first: first.clone(),
                });
            }
            let tool = compile(path, &id, raw)?;
            self.origin.insert(id.clone(), path.to_owned());
            self.tools.insert(id, tool);
        }
        Ok(())
    }
}

fn compile(path: &Path, id: &str, raw: RawTool) -> Result<Tool, LoadError> {
    let invalid = |reason: &str| LoadError::Invalid {
        path: path.to_owned(),
        tool: id.to_owned(),
        reason: reason.to_owned(),
    };
    if raw.asset.is_empty() {
        return Err(invalid("asset names no architecture"));
    }
    if raw.launchers.is_empty() {
        return Err(invalid("launchers names no launcher"));
    }
    let source = raw.source.compile().map_err(invalid)?;
    if let Some(dir) = &raw.dir
        && (dir.is_empty() || dir.starts_with('.') || dir.contains('/'))
    {
        return Err(invalid("dir must be a plain directory name"));
    }
    if let Some(subdir) = &raw.subdir
        && (raw.dir.is_none() || subdir.is_empty() || !is_plain_relative(Path::new(subdir)))
    {
        return Err(invalid(
            "subdir must be relative, without .., and comes with a dir",
        ));
    }
    if raw.launchers.values().any(|d| !is_plain_relative(d)) {
        return Err(invalid("launcher directories must be relative, without .."));
    }
    let regex = |field: String, pattern: &str| {
        Regex::new(pattern).map_err(|source| LoadError::Pattern {
            path: path.to_owned(),
            tool: id.to_owned(),
            field,
            source,
        })
    };
    let tag = raw.tag.map(|t| regex("tag".into(), &t)).transpose()?;
    // `any` applies to every arch an explicit key doesn't name.
    let mut asset = BTreeMap::new();
    for (key, pattern) in &raw.asset {
        let arches: &[Arch] = match key.as_str() {
            "x86_64" => &[Arch::X86_64],
            "aarch64" => &[Arch::Aarch64],
            "any" => &[Arch::X86_64, Arch::Aarch64],
            _ => {
                return Err(invalid(&format!(
                    "unknown asset arch {key}: x86_64, aarch64 or any"
                )));
            }
        };
        let re = regex(format!("{key} asset"), pattern)?;
        for &arch in arches {
            if key != "any" || !asset.contains_key(&arch) {
                asset.insert(arch, re.clone());
            }
        }
    }
    Ok(Tool {
        id: id.to_owned(),
        name: raw.name,
        source,
        tag,
        asset,
        checksum: raw.checksum,
        dir: raw.dir,
        subdir: raw.subdir,
        requires: raw.requires,
        launchers: raw.launchers,
    })
}

impl Tool {
    /// Whether the release tagged `tag` is a version of this tool.
    pub fn is_version(&self, tag: &str) -> bool {
        self.tag.as_ref().is_none_or(|re| re.is_match(tag))
    }
}

fn is_plain_relative(dir: &Path) -> bool {
    use std::path::Component;
    dir.components().all(|c| matches!(c, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GE: &str = r#"
        [ge-proton]
        name      = "GE-Proton"
        source    = { github = "GloriousEggroll/proton-ge-custom" }
        asset     = { x86_64 = '^GE-Proton.*-x86_64\.tar\.gz$' }
        checksum  = { pattern = "{asset_stem}.sha512sum", algo = "sha512" }
        launchers = { steam = "compatibilitytools.d", heroic = "tools/proton" }
    "#;

    fn add(text: &str) -> Result<Registry, LoadError> {
        let mut r = Registry::default();
        r.add(Path::new("t.toml"), text)?;
        Ok(r)
    }

    #[test]
    fn loads_a_tool() {
        let r = add(GE).unwrap();
        let t = &r.tools["ge-proton"];
        assert_eq!(
            t.source,
            Source::Github("GloriousEggroll/proton-ge-custom".into())
        );
        assert!(t.asset[&Arch::X86_64].is_match("GE-Proton11-7-x86_64.tar.gz"));
        assert_eq!(
            t.launchers[&LauncherKind::Heroic],
            Path::new("tools/proton")
        );
    }

    #[test]
    fn rejects_bad_manifests() {
        let cases = [
            ("unknown field", GE.replace("name ", "nmae ")),
            ("bad regex", GE.replace(r"\.tar", r"(\.tar")),
            ("bad tag regex", GE.replace("name ", "tag = '(' \n name ")),
            ("unknown arch", GE.replace("x86_64 =", "riscv =")),
            ("unknown launcher", GE.replace("heroic =", "origin =")),
            (
                "dir with a slash",
                GE.replace("name ", "dir = 'a/b'\n name "),
            ),
            (
                "two sources",
                GE.replace("source    = {", "source    = { forgejo = 'https://x/o/r',"),
            ),
            (
                "forgejo without a URL",
                GE.replace(
                    "github = \"GloriousEggroll/proton-ge-custom\"",
                    "forgejo = \"codeberg.org/o/r\"",
                ),
            ),
            (
                "url without a version",
                GE.replace(
                    "github = \"GloriousEggroll/proton-ge-custom\"",
                    "url = \"https://x/a.tar.gz\"",
                ),
            ),
            (
                "subdir without a dir",
                GE.replace("name ", "subdir = 'build'\n name "),
            ),
            (
                "escaping subdir",
                GE.replace("name ", "dir = 'a'\n subdir = '../b'\n name "),
            ),
            ("escaping dir", GE.replace("tools/proton", "../proton")),
            ("absolute dir", GE.replace("tools/proton", "/opt/proton")),
            (
                "no arch",
                GE.replace(r#"x86_64 = '^GE-Proton.*-x86_64\.tar\.gz$'"#, ""),
            ),
        ];
        for (what, text) in cases {
            assert!(add(&text).is_err(), "{what}: accepted");
        }
    }

    #[test]
    fn any_arch_and_other_sources() {
        let r = add(r#"
            [spn]
            name = "Steam-Play-None"
            source = { url = "https://example.org/main.tar.gz", version = "main" }
            asset = { any = '^main\.tar\.gz$', aarch64 = '^other$' }
            dir = "Steam-Play-None"
            requires = ["sh"]
            launchers = { steam = "compatibilitytools.d" }
            [lux]
            name = "Luxtorpeda"
            source = { forgejo = "https://codeberg.org/luxtorpeda/luxtorpeda/" }
            asset = { x86_64 = 'x' }
            launchers = { steam = "compatibilitytools.d" }
            "#)
        .unwrap();
        let spn = &r.tools["spn"];
        assert!(spn.asset[&Arch::X86_64].is_match("main.tar.gz"));
        assert!(
            spn.asset[&Arch::Aarch64].is_match("other"),
            "an explicit arch wins"
        );
        assert_eq!(spn.dir.as_deref(), Some("Steam-Play-None"));
        assert_eq!(spn.requires, ["sh"]);
        assert_eq!(
            r.tools["lux"].source,
            Source::Forgejo("https://codeberg.org/luxtorpeda/luxtorpeda".into())
        );
    }

    #[test]
    fn rejects_a_tool_defined_twice() {
        let mut r = add(GE).unwrap();
        let err = r.add(Path::new("other.toml"), GE).unwrap_err();
        assert!(matches!(err, LoadError::Duplicate { .. }), "{err}");
    }
}
