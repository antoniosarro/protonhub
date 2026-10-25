# Fedora. Builds the release tagged v%{version}; the crates
# come from crates.io during %%prep, so on COPR enable networking.
# Locally: spectool -g -R packaging/rpm/protonhub.spec && rpmbuild -bb packaging/rpm/protonhub.spec
Name:           protonhub
Version:        0.1.0
Release:        1%{?dist}
Summary:        Install and manage Proton, Wine, DXVK and VKD3D builds across game launchers
License:        MIT
URL:            https://github.com/antoniosarro/protonhub
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

# Release builds carry no debug info to split out.
%global debug_package %{nil}

ExclusiveArch:  x86_64 aarch64
BuildRequires:  cargo >= 1.92
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk4) >= 4.12
BuildRequires:  pkgconfig(libadwaita-1) >= 1.6
BuildRequires:  pkgconfig(libudev)
BuildRequires:  gettext
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib
Recommends:     libnotify
Suggests:       mangohud
Suggests:       vulkan-tools
Suggests:       winetricks

%description
ProtonHub installs and removes compatibility tools such as GE-Proton,
Proton-CachyOS, Wine-GE, DXVK and vkd3d-proton for Steam, Heroic, Lutris,
Bottles and WineZGUI, verifying every download against its published
checksum. It sets the tool each Steam and Heroic game runs with, finds the
tools nothing uses, and keeps tools at their latest release with a daily
systemd user timer. It comes as a GTK 4 + libadwaita app (protonhub-gtk)
and a scriptable command line (protonhub).

%prep
%autosetup -n %{name}-%{version}
cargo fetch --locked

%build
cargo build --frozen --release --workspace

%install
install -Dm755 target/release/protonhub target/release/protonhub-gtk -t %{buildroot}%{_bindir}
sh packaging/install-data.sh %{buildroot}%{_prefix}
%find_lang %{name}

%check
cargo test --frozen --release --workspace
desktop-file-validate %{buildroot}%{_datadir}/applications/dev.antoniosarro.ProtonHub.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/dev.antoniosarro.ProtonHub.metainfo.xml

%files -f %{name}.lang
%license LICENSE
%doc README.md
%{_bindir}/protonhub
%{_bindir}/protonhub-gtk
%{_datadir}/protonhub/
%{_datadir}/applications/dev.antoniosarro.ProtonHub.desktop
%{_metainfodir}/dev.antoniosarro.ProtonHub.metainfo.xml
%{_datadir}/icons/hicolor/512x512/apps/dev.antoniosarro.ProtonHub.png

%changelog
* Sat Oct 24 2026 Antonio Sarro <tech@antoniosarro.dev> - 0.1.0-1
- First release
