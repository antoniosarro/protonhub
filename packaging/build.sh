#!/bin/sh
# Builds the Arch, Debian or Fedora package of this checkout into dist/, at
# the version in Cargo.toml. Run it on that distribution, as a user (not
# root, for makepkg), with the build dependencies installed
# (packaging/README.md); the release workflow runs it in their containers.
#
#   packaging/build.sh arch|deb|rpm [ARGS...]
#
# ARGS go to makepkg, dpkg-buildpackage or rpmbuild.
set -eu
cd "$(dirname "$0")/.."
kind=${1:?usage: packaging/build.sh arch|deb|rpm [ARGS...]}
shift
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
out=$PWD/dist
work=$(mktemp -d)
mkdir -p "$out"

# The checkout as a source tarball whose top directory is $1.
tarball() {
	tar -czf "$2" --exclude=./target --exclude=./dist --exclude=./debian \
		--exclude=./result --exclude=./.git --exclude=./.direnv \
		--exclude=./.flatpak-builder --exclude=./.claude --transform "s,^\.,$1," .
}

case $kind in
arch)
	# packaging/arch/PKGBUILD builds main's newest commit; this is the
	# same package as a release of the checkout.
	tarball protonhub "$work/protonhub-$version.tar.gz"
	sed -e 's/^pkgname=.*/pkgname=protonhub/' -e "s/^pkgver=.*/pkgver=$version/" \
		-e '/^provides=/d' -e '/^conflicts=/d' -e '/^pkgver() {/,/^}/d' \
		-e "s/^source=.*/source=(\"protonhub-$version.tar.gz\")/" \
		packaging/arch/PKGBUILD >"$work/PKGBUILD"
	(cd "$work" && PKGDEST=$out makepkg --noconfirm "$@")
	;;
deb)
	rm -rf debian
	cp -r packaging/debian debian
	sed -i "1s/(.*)/($version)/" debian/changelog
	dpkg-buildpackage -us -uc -b "$@"
	mv ../protonhub_"$version"_*.deb "$out/"
	;;
rpm)
	mkdir "$work/SOURCES"
	tarball "protonhub-$version" "$work/SOURCES/protonhub-$version.tar.gz"
	sed "s/^Version:.*/Version:        $version/" packaging/rpm/protonhub.spec >"$work/protonhub.spec"
	rpmbuild -bb --define "_topdir $work" --define "_rpmdir $out" \
		--define '_build_name_fmt %%{NAME}-%%{VERSION}-%%{RELEASE}.%%{ARCH}.rpm' \
		"$@" "$work/protonhub.spec"
	;;
*)
	echo "packaging/build.sh: unknown package $kind: arch, deb or rpm" >&2
	exit 2
	;;
esac
rm -rf "$work"
ls -l "$out"
