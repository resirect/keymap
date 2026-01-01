#!/bin/sh
# Remove the files installed by scripts/install.sh.
#
# Keymap's own settings, saved custom actions and backups are kept unless you
# pass --purge. In your Niri configuration only Keymap's session startup entry
# (`keymap autostart`) and the shortcut that opens Keymap (`keymap shortcut`)
# are removed, each through the same validated and backed-up transaction as
# any Keymap edit; nothing else there is touched.
set -eu

PREFIX=${PREFIX:-"$HOME/.local"}
purge=no
[ "${1:-}" = "--purge" ] && purge=yes

if [ -x "$PREFIX/bin/keymap" ]; then
    # Stop a running instance (nothing to do if none is running).
    "$PREFIX/bin/keymap" quit >/dev/null 2>&1 || true
    # Remove the session startup entry, if there is one.
    if "$PREFIX/bin/keymap" autostart status 2>/dev/null | grep -q '^Starts with the session'; then
        "$PREFIX/bin/keymap" autostart disable || echo "Could not remove the startup entry; see 'keymap autostart status'."
    fi
    # Remove the shortcut that opens Keymap, which would otherwise run a missing program.
    if "$PREFIX/bin/keymap" shortcut status 2>/dev/null | grep -q '^Opens Keymap'; then
        "$PREFIX/bin/keymap" shortcut remove || echo "Could not remove the launch shortcut; see 'keymap shortcut status'."
    fi
fi

for f in "$PREFIX/bin/keymap" "$PREFIX/bin/keymap-ui" "$PREFIX/share/applications/io.github.resirect.Keymap.desktop"; do
    if [ -e "$f" ]; then
        rm -- "$f"
        echo "Removed $f"
    fi
done

config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/keymap"
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/keymap"
if [ "$purge" = yes ]; then
    for d in "$config_dir" "$state_dir"; do
        if [ -e "$d" ]; then
            rm -rf -- "$d"
            echo "Removed $d"
        fi
    done
elif [ -e "$config_dir" ] || [ -e "$state_dir" ]; then
    echo "Kept app data (remove with --purge):"
    [ -e "$config_dir" ] && echo "  $config_dir   settings and saved custom actions"
    [ -e "$state_dir" ] && echo "  $state_dir    backups and history of config edits"
fi
echo
echo "Bindings you created with Keymap remain in your Niri configuration."
echo "If you added the Keymap blur rule (namespace \"^keymap\$\") or the blur strength"
echo "section (\"// Keymap: blur strength\"), remove those lines by hand if you no"
echo "longer want them."
