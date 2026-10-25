# Tool manifests

Every tool protonhub can install is described by a TOML manifest in
`registry/`. A file holds one or more tools, one table each; the table name
is the tool's id and must be unique across the registry.

```toml
[ge-proton]
name      = "GE-Proton"
source    = { github = "GloriousEggroll/proton-ge-custom" }
asset     = { x86_64 = '^GE-Proton[0-9][0-9.-]*(-x86_64)?\.tar\.gz$', aarch64 = '...' }
checksum  = { pattern = "{asset_stem}.sha512sum", algo = "sha512" }
launchers = { steam = "compatibilitytools.d", heroic = "tools/proton" }
```

| Field | |
|---|---|
| `name` | The name shown to people |
| `source` | Where releases come from; each release is a version. See [Sources](#sources) |
| `tag` | Optional. A regex for the release tags that are versions of this tool, for a repository that publishes several tools (wine-ge-custom's regular and `-LoL` builds). Without it, every release is |
| `asset` | A regex per architecture: `x86_64`, `aarch64`, or `any` for every arch an explicit key doesn't name. Exactly one asset of a release must match |
| `checksum` | Optional. `pattern` is the checksum file's exact name, `algo` is `sha256` or `sha512`. See [Checksums](#checksums) |
| `dir` | Optional. The directory name to install into, `{tag}` being the release's tag: for archives without exactly one top-level directory (DXVK-NVAPI), or to rename the one they have (Steam-Play-None's `Steam-Play-None-main`) |
| `subdir` | Optional, with `dir`. The directory inside the archive's top-level one that is the tool, for archives that hold more: DXVK-Sarek's hold its source, with the DLLs in `build` |
| `requires` | Optional. Programs the tool needs on the host and doesn't bring (Roberta needs `scummvm`). Installing works without them, with a warning |
| `launchers` | For each launcher (`steam`, `heroic`, `lutris`, `bottles`, `winezgui`), the directory under its data directory that versions are extracted into. Relative, without `..` |

Unknown fields, architectures and launchers are errors, so a typo is caught
when the manifest loads rather than ignored.

## Sources

| Source | |
|---|---|
| `{ github = "owner/repo" }` | GitHub releases. The only source `GITHUB_TOKEN` is sent to |
| `{ forgejo = "https://codeberg.org/owner/repo" }` | Releases of a Forgejo or Gitea repository, by its URL |
| `{ gitlab = "https://gitlab.com/group/project" }` | Releases of a GitLab project, by its URL. Their asset links don't give sizes, so these downloads can't be resumed or checked for length |
| `{ url = "https://…/main.tar.gz", version = "main" }` | One archive at a fixed URL, as a single version: a branch snapshot for a tool without releases |

Archives may be `.tar.gz`, `.tar.xz` or `.tar.zst`.

## Choosing a release

`latest` is the newest release among the source's 50 newest that is
neither a draft nor a prerelease (on GitLab, an upcoming release) and
whose tag the `tag` pattern accepts. It is not GitHub's own "latest
release", which can belong to another tool in the same repository. A tag
asked for by name must also match `tag`.

## Resolving a release

`Tool::resolve(assets, arch)` picks the asset and its checksum file:

1. The arch's pattern must match **exactly one** asset. None is
   `NoAsset`, and the error lists the release's assets; several is
   `AmbiguousAsset`, listing the matches.
2. The checksum file is the asset whose name equals the pattern filled in
   for that asset. If it's missing, that's `NoChecksum` with the expected
   name. protonhub never falls back to another checksum file.

## Checksums

A pattern with `{asset}` (the asset's name) or `{asset_stem}` (without
`.tar.gz`, `.tar.xz`, `.tar.zst`, `.zip`, ...) names a checksum file per
asset, like GE-Proton's `GE-Proton11-7-x86_64.sha512sum`. Its line for the
asset is used, or a lone hash if that's all it holds.

A fixed name, like Kron4ek's `sha256sums.txt`, is one list for every
asset, and must name the asset: a lone hash in it could be any asset's.

## Installing

Versions install into the launcher's directory as the archive's top-level
directory, or `dir`. When that directory already holds an older version of
the same tool (Boxtron's is always `boxtron`), the new version replaces
it; a directory of anything else is left alone, and the install fails.

## Adding or fixing a tool

1. Edit or add a file in `registry/`.
2. Record the release's asset names in
   `crates/manifest/tests/registry.rs`, with any neighbouring asset the
   pattern must *not* match (like `dxvk-native-*` or the `-x86_64_v3`
   builds).
3. `just registry-check` and `just test`, then `just registry-verify`,
   which resolves every tool's latest release over the network.

`.github/workflows/registry.yml` runs `registry verify` nightly and on
changes to `registry/`, so an upstream rename breaks CI before it reaches
users. Planned: the app fetches the registry from this repository, so a
fix reaches users without an app release.

## The tools

| Tool | Source | Launchers |
|---|---|---|
| GE-Proton, Proton-CachyOS, Proton-Wineland, DW-Proton, Proton-RTSP, Proton-EM | GitHub | Steam, Heroic, Lutris, Bottles |
| Luxtorpeda | Codeberg | Steam |
| Boxtron, Roberta | GitHub | Steam |
| Steam-Play-None | the main branch's archive | Steam |
| Wine-GE | GitHub | Lutris, Heroic, Bottles, WineZGUI |
| Lutris-Wine, Kron4ek Wine (vanilla, Staging, Staging-Tkg, Proton) | GitHub | Lutris, Bottles, WineZGUI |
| DXVK, DXVK-Sarek, DXVK-NVAPI | GitHub | Lutris, Heroic, Bottles |
| DXVK GPL-Async | GitLab | Lutris, Bottles |
| vkd3d-proton | GitHub | Lutris, Heroic, Bottles |
| vkd3d-lutris | GitHub | Lutris, Heroic |

Not covered, because a manifest can't describe them:

- **Proton-Tkg, Wine-Tkg, DXVK nightly**: built by GitHub Actions, not
  released. Their artifacts can only be downloaded with a token (or
  through a third-party proxy), and they expire.
- **SteamTinkerLaunch**: not an archive to extract but a script set up by
  running it (`steamtinkerlaunch compat add`), with a dozen host programs
  it needs. This is what a code plugin would be for; none exists yet.
