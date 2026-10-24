#!/bin/sh
# Installs everything but the binaries under PREFIX, the same for every
# package: the tool manifests (share/protonhub/registry, where the binaries
# look for them), the compiled translations (share/locale), and the desktop
# entry, AppStream metainfo and icon. Needs msgfmt (gettext).
#
#   packaging/install-data.sh PREFIX     e.g. "$pkgdir/usr", /app, $out
#
# Run from anywhere; paths are relative to the repository.
set -eu
prefix=${1:?usage: packaging/install-data.sh PREFIX}
case $prefix in /*) ;; *) prefix=$PWD/$prefix ;; esac
cd "$(dirname "$0")/.."
id=dev.antoniosarro.ProtonHub

install -d "$prefix/share/protonhub"
cp -r registry "$prefix/share/protonhub/"
for po in po/*.po; do
	lang=$(basename "$po" .po)
	install -d "$prefix/share/locale/$lang/LC_MESSAGES"
	msgfmt --check -o "$prefix/share/locale/$lang/LC_MESSAGES/protonhub.mo" "$po"
done
install -Dm644 "packaging/linux/$id.desktop" -t "$prefix/share/applications"
install -Dm644 "packaging/linux/$id.metainfo.xml" -t "$prefix/share/metainfo"
install -Dm644 "packaging/linux/$id.svg" -t "$prefix/share/icons/hicolor/scalable/apps"
