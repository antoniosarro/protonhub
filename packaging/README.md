# Packaging

Everything that turns protonhub into something to install. The recipes in
`packaging/justfile` run from the repository root as `just pkg <recipe>`
(`just pkg` lists them), in the dev shell.

```
packaging/
├── install-data.sh    installs everything but the binaries; every package calls it
├── justfile           just pkg: version, install, nix, flatpak, flatpak-sources, clean
├── linux/             desktop entry, AppStream metainfo, icon
├── arch/PKGBUILD      Arch Linux: protonhub-git
├── rpm/protonhub.spec Fedora
├── debian/            Debian and Ubuntu
├── nix/               package, NixOS module, home-manager module
└── flatpak/           manifest, and the crates for its offline build
```

## What every package installs

| Path | |
|---|---|
| `bin/protonhub`, `bin/protonhub-gtk` | the CLI and the GUI |
| `share/protonhub/registry/` | the tool manifests; the binaries look for them at `../share/protonhub/registry` |
| `share/locale/*/LC_MESSAGES/protonhub.mo` | the translations; the GUI looks for them at `../share/locale` |
| `share/applications/dev.antoniosarro.ProtonHub.desktop` | the desktop entry |
| `share/metainfo/dev.antoniosarro.ProtonHub.metainfo.xml` | the AppStream metadata |
| `share/icons/hicolor/scalable/apps/dev.antoniosarro.ProtonHub.svg` | the icon |

`install-data.sh PREFIX` installs all but `bin/`, so a package builds with
`cargo build --release --workspace`, installs the two binaries, and runs
it. The GUI links GTK 4 (4.12+), libadwaita (1.6+) and libudev (gamepads);
building needs Rust 1.92+ and gettext's `msgfmt`. `notify-send` (libnotify)
is optional, for the update timer's notifications.

## Version

`just pkg version` prints the version from git: `1.2.3` at the tag
`v1.2.3`, `1.2.3.r4.gabc1234` four commits past it, and
`0.0.0.r<commits>.g<hash>` before the first tag. The Arch package uses the
same scheme, with `.r0.g<hash>` at a tag. The Nix package reads `VERSION` (flakes can't see tags); the
RPM spec and `debian/changelog` name it themselves.

## From source

```sh
just pkg install                 # to ~/.local
just pkg install ~/apps          # any PREFIX you can write to
```

## Arch Linux

`arch/PKGBUILD` builds the newest commit, as AUR's `protonhub-git` would:

```sh
cd packaging/arch && makepkg -si
```

## Fedora

`rpm/protonhub.spec` builds the release tagged `v<Version>`. The crates
come from crates.io during `%prep`, so the build needs the network (on
COPR, enable it for the project).

```sh
sudo dnf install rpm-build rpmdevtools
sudo dnf builddep packaging/rpm/protonhub.spec
spectool -g -R packaging/rpm/protonhub.spec
rpmbuild -bb packaging/rpm/protonhub.spec
```

## Debian and Ubuntu

`debian/` builds a `.deb` from the checkout. It needs Rust 1.92 or newer
and libadwaita 1.6 or newer: Ubuntu 26.10 has both; Debian 13 has the
libraries but an older Rust, so install a toolchain with
[rustup](https://rustup.rs) and pass `-d` to skip the check for the
distribution's `cargo`. The crates come from crates.io, so the build needs
the network; this is for building your own package, not for the Debian
archive.

```sh
sudo apt install build-essential debhelper cargo rustc pkgconf \
    libgtk-4-dev libadwaita-1-dev libudev-dev gettext
cp -r packaging/debian . && dpkg-buildpackage -us -uc -b    # add -d with rustup
sudo apt install ../protonhub_*.deb
```

## Nix

| Output | |
|---|---|
| `packages.<system>.default` | `protonhub` and `protonhub-gtk`; `nix build`, `nix run` |
| `overlays.default` | adds `pkgs.protonhub` |
| `nixosModules.default` | `programs.protonhub.enable`: installs it for every user |
| `homeManagerModules.default` | `programs.protonhub`: installs it, and can declare channels and the timer |

```sh
nix run github:antoniosarro/protonhub -- --help
nix shell github:antoniosarro/protonhub -c protonhub-gtk
just pkg nix                   # build ./result from the checkout
```

In a system flake:

```nix
inputs.protonhub.url = "github:antoniosarro/protonhub";

# NixOS
imports = [ inputs.protonhub.nixosModules.default ];
programs.protonhub.enable = true;

# home-manager
imports = [ inputs.protonhub.homeManagerModules.default ];
programs.protonhub = {
  enable = true;
  channels = [
    { tool = "ge-proton"; launcher = "steam"; keep = 2; moveGames = true; }
    { tool = "dxvk"; launcher = "lutris"; }
  ];
  timer.enable = true;          # protonhub update --notify, daily
};
```

With `channels` set, home-manager writes
`~/.config/protonhub/channels.toml` read-only: the GUI's "Keep up to date"
and `protonhub channel` can't change it, and say so. Leave `channels`
unset to manage them there instead. `timer.enable` declares what
`protonhub timer enable` would write; use one or the other.

## Flatpak

`just pkg flatpak` builds the manifest and installs the app for your user.
It needs `flatpak-builder` and the Flathub remote, for the GNOME 51
runtime and the Rust SDK extension. The CLI is inside:

```sh
flatpak run dev.antoniosarro.ProtonHub                       # the GUI
flatpak run --command=protonhub dev.antoniosarro.ProtonHub list
```

The sandbox can read and write each launcher's data directory (native,
Flatpak and Snap), reach the network, read gamepads, and send
notifications. It can't:

- see processes outside it, so it can't tell whether Steam or Heroic is
  running before changing a game's tool (the GUI says so);
- install the systemd timer (`timer enable` says so). A host timer can run
  `flatpak run --command=protonhub dev.antoniosarro.ProtonHub update --notify`.

After changing `Cargo.lock`, `just pkg flatpak-sources` regenerates
`flatpak/cargo-sources.json` with flatpak-builder-tools' generator.
