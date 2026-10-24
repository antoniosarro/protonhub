#!/usr/bin/env bash
# Screenshots the GUI in a private headless sway, so nothing shows on or
# types into your session:
#
#   scripts/screenshot.sh OUT.png [STEP...]
#
# Each STEP is one wtype call (e.g. "-k Tab", "-k Return",
# "-M ctrl -k r -m ctrl", "-s 2000" to wait), a second apart once the
# window is up; the shot is taken a second after the last. There is no
# pointer: the headless display has no pointer device.
#
# SHOT_HOME is the HOME the GUI sees (default: an empty one, so no
# launchers), SHOT_SIZE the display size (default 900x700). Needs the dev
# shell, which has sway, grim and wtype.
set -euo pipefail
cd "$(dirname "$0")/.."

[ $# -ge 1 ] || { sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
OUT=$(realpath -m "$1")
shift
SIZE=${SHOT_SIZE:-900x700}

cargo build -q -p protonhub-gtk
TMP=$(mktemp -d)
cleanup() {
	kill "${APP_PID:-}" "${SWAY_PID:-}" 2>/dev/null || true
	rm -rf "$TMP"
}
trap cleanup EXIT
HOME_DIR=${SHOT_HOME:-$TMP/home}
mkdir -p "$HOME_DIR"

cat >"$TMP/sway.cfg" <<CFG
output HEADLESS-1 resolution $SIZE
default_border none
exec sh -c 'echo \$WAYLAND_DISPLAY >$TMP/display'
CFG
WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 sway -c "$TMP/sway.cfg" >"$TMP/sway.log" 2>&1 &
SWAY_PID=$!
for _ in $(seq 40); do [ -s "$TMP/display" ] && break; sleep 0.25; done
[ -s "$TMP/display" ] || { echo "screenshot.sh: headless sway didn't start:" >&2; cat "$TMP/sway.log" >&2; exit 1; }
export WAYLAND_DISPLAY
WAYLAND_DISPLAY=$(cat "$TMP/display")

# The real cache keeps API responses between shots; HOME and the config
# (channels) are the fixture's.
XDG_CACHE_HOME=${XDG_CACHE_HOME:-$HOME/.cache} XDG_CONFIG_HOME=$HOME_DIR/.config \
	HOME=$HOME_DIR GDK_BACKEND=wayland \
	target/debug/protonhub-gtk >"$TMP/app.log" 2>&1 &
APP_PID=$!
sleep 2
kill -0 "$APP_PID" 2>/dev/null || { echo "screenshot.sh: the GUI exited:" >&2; cat "$TMP/app.log" >&2; exit 1; }

# Each wtype is a new virtual keyboard: a key sent before the GUI has its
# keymap is lost, so every call waits a moment first.
for step in "$@"; do
	# shellcheck disable=SC2086 # each step is a list of wtype options
	wtype -s 150 $step
	sleep 1
done
sleep 1
grim "$OUT"
echo "$OUT"
# Warnings and criticals from GTK are worth seeing with the shot.
[ -s "$TMP/app.log" ] && { echo "--- the GUI's log:" >&2; cat "$TMP/app.log" >&2; }
true
