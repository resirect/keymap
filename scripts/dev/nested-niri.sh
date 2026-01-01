#!/bin/sh
# Start an isolated, nested Niri session (a window inside your current
# session) for manual and scripted integration checks. It never touches your
# real configuration: the config and runtime files live in $1 (a scratch dir).
#
#   scripts/dev/nested-niri.sh /tmp/keymap-nested [extra.kdl]
#
# Writes $1/env with WAYLAND_DISPLAY and NIRI_SOCKET of the nested session.
set -eu
dir=${1:?usage: nested-niri.sh SCRATCH_DIR [extra.kdl]}
mkdir -p "$dir"
cat > "$dir/config.kdl" <<KDL
input {
    keyboard {
        xkb {
            layout "us,de"
        }
    }
}
hotkey-overlay {
    skip-at-startup
}
prefer-no-csd
screenshot-path null
spawn-at-startup "sh" "-c" "env | grep -E '^(WAYLAND_DISPLAY|NIRI_SOCKET)=' > '$dir/env'"
spawn-at-startup "foot" "-e" "sh" "-c" "ls -la --color=always /usr/bin | head -200; exec sleep 100000"
binds {
    Mod+T { spawn "foot"; }
    Mod+Shift+E { quit skip-confirmation=true; }
}
KDL
[ -n "${2:-}" ] && cat "$2" >> "$dir/config.kdl"
rm -f "$dir/env"
exec niri -c "$dir/config.kdl" > "$dir/niri.log" 2>&1
