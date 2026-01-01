#!/bin/sh
# User-local install of Keymap (no root, no services).
#
#   scripts/install.sh            build in release mode and install to ~/.local
#   PREFIX=/some/prefix scripts/install.sh
#   KEYMAP_NO_BLUR=1 scripts/install.sh   leave the Niri config untouched
#
# Installs exactly three files:
#   $PREFIX/bin/keymap        command-line client (show/hide/toggle/quit, check)
#   $PREFIX/bin/keymap-ui     the resident overlay, started by `keymap` on demand
#   $PREFIX/share/applications/io.github.resirect.Keymap.desktop
# Then it turns background blur on with `keymap blur enable`: unless they are
# already there, it adds Keymap's own blur layer rule and, only if your config
# has no `blur { }` section, one setting the Light strength. Each is validated
# with `niri validate`, backed up, and undoable in Settings → Recent changes.
# See README.md for the optional launch bind and session startup entry.
set -eu

PREFIX=${PREFIX:-"$HOME/.local"}
root=$(cd "$(dirname "$0")/.." && pwd)

fail() {
    echo "Keymap was not installed: $1" >&2
    exit 1
}
command -v cargo >/dev/null 2>&1 ||
    fail "cargo was not found. Install the build dependencies listed in README.md first."
cargo build --release --locked --manifest-path "$root/Cargo.toml" -p keymap ||
    fail "the build failed (errors above). Check the build dependencies in README.md."

# A running instance keeps using the old binary until it restarts.
if [ -x "$PREFIX/bin/keymap" ]; then
    "$PREFIX/bin/keymap" quit >/dev/null 2>&1 || true
fi
install -Dm755 "$root/target/release/keymap" "$PREFIX/bin/keymap"
install -Dm755 "$root/target/release/keymap-ui" "$PREFIX/bin/keymap-ui"
install -Dm644 "$root/packaging/io.github.resirect.Keymap.desktop" \
    "$PREFIX/share/applications/io.github.resirect.Keymap.desktop"

echo "Installed $PREFIX/bin/keymap and $PREFIX/bin/keymap-ui"
echo "Installed $PREFIX/share/applications/io.github.resirect.Keymap.desktop"
keymap=keymap
case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *)
        keymap="$PREFIX/bin/keymap"
        echo "Note: $PREFIX/bin is not on your PATH, so commands below use the full path."
        echo "Niri starts programs with the same PATH: use the full path in a launch bind too,"
        echo "or add $PREFIX/bin to PATH in your shell profile and log in again."
        ;;
esac
if [ "${KEYMAP_NO_BLUR:-}" = 1 ]; then
    echo "Left the Niri config untouched (KEYMAP_NO_BLUR=1). '$PREFIX/bin/keymap blur enable' turns blur on later."
else
    echo
    echo "Turning on background blur:"
    "$PREFIX/bin/keymap" blur enable || echo "Blur was not set up; Settings → Background in Keymap explains why."
fi
echo
echo "Next: run '$keymap check' to see what Keymap reads and a free launch combination."
echo "To open Keymap with Super+Shift+M: '$keymap shortcut set'. It refuses if that is taken;"
echo "'$keymap check' then suggests a free combination for '$keymap shortcut set COMBO'."
echo "Optional: '$keymap autostart enable' starts Keymap hidden with your Niri session"
echo "so the first open is instant (adds one validated, backed-up line; 'disable' removes it)."
