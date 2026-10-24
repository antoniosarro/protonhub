# packages for Linux distributions and Nix: just pkg <recipe> (packaging/README.md)
mod pkg 'packaging'

default:
    @just --list

# compile the CLI to target/debug/protonhub
build:
    cargo build -p protonhub-cli

# build and run the CLI, forwarding any extra arguments
run *args:
    cargo run -q -p protonhub-cli -- {{args}}

# run the core tests, forwarding any extra arguments (e.g. a test name filter)
test *args:
    cargo test {{args}}

# check every manifest in registry/ against the schema
registry-check:
    cargo run -q -p protonhub-cli -- registry check registry

# resolve every tool's latest release over the network (what CI runs nightly)
registry-verify:
    cargo run -q -p protonhub-cli -- registry verify registry

# format Rust sources in place
fmt:
    cargo fmt --all

# run clippy on the core crates, warnings as errors
lint:
    cargo clippy --all-targets -- -D warnings

# run fmt + lint + test + registry-check, the full pre-commit check
check: fmt lint test registry-check

# build and run the GUI, with the translations compiled
gui *args: locale
    PROTONHUB_LOCALEDIR=target/locale cargo run -q -p protonhub-gtk -- {{args}}

# extract the GUI's strings into po/protonhub.pot, and merge them into each translation
pot:
    xgettext --language=Rust --from-code=UTF-8 --keyword=tr:1 --keyword=trn:1,2 \
        --add-comments=TRANSLATORS: --package-name=protonhub --sort-by-file \
        -o po/protonhub.pot crates/gtk/src/*.rs crates/gtk/src/*/*.rs
    for po in po/*.po; do msgmerge --quiet --update --backup=none "$po" po/protonhub.pot; done

# compile the translations into target/locale (the package installs them to share/locale)
locale:
    #!/usr/bin/env bash
    set -euo pipefail
    for po in po/*.po; do
        lang=$(basename "$po" .po)
        mkdir -p "target/locale/$lang/LC_MESSAGES"
        msgfmt --check -o "target/locale/$lang/LC_MESSAGES/protonhub.mo" "$po"
    done

# screenshot the GUI in a private headless display: just screenshot OUT.png [STEP...] (scripts/screenshot.sh)
[positional-arguments]
screenshot *args:
    ./scripts/screenshot.sh "$@"

# lint and test the GUI crate too (it is outside the default members)
gui-check:
    cargo clippy -p protonhub-gtk --all-targets -- -D warnings
    cargo test -p protonhub-gtk
