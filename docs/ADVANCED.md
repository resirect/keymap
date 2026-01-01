# Advanced use

This page covers optional installation methods and details that the
[README](../README.md) leaves out.

## Commands

```sh
keymap              # show Keymap (starts it if needed)
keymap toggle       # show or hide
keymap hide         # hide; Keymap keeps running in the background
keymap preload      # start Keymap hidden, if it is not running yet
keymap quit         # stop the running instance
keymap check        # read-only report on the config Keymap uses
keymap shortcut status|set [COMBO]|remove   # the shortcut that opens Keymap
keymap autostart enable|disable|status      # start hidden with the Niri session
keymap blur status|enable|subtle|light|medium|strong   # blur rule and strength
keymap blur-rule    # print Keymap's blur layer rule
keymap --demo       # practice on a temporary synthetic config
keymap --config PATH   # use this file instead of the discovered one
```

Repeated launches never open a second overlay. The first instance owns a
control socket in `$XDG_RUNTIME_DIR`, and later commands are sent to it. A
stale socket left by a crash is replaced. `keymap` is a small client that does
not load GTK; the overlay itself is `keymap-ui`.

While hidden, Keymap has no surface on screen and captures no input. It uses
no CPU until a config file changes. `keymap quit` really exits.

## Installing elsewhere

`PREFIX` changes the install location (default `~/.local`):

```sh
PREFIX=~/apps/keymap scripts/install.sh
PREFIX=~/apps/keymap scripts/uninstall.sh
```

`scripts/install.sh` installs exactly `$PREFIX/bin/keymap`,
`$PREFIX/bin/keymap-ui` and
`$PREFIX/share/applications/io.github.resirect.Keymap.desktop`.

## Writing the shortcut by hand

Instead of `keymap shortcut set`, you can add a line like this to a
`binds { }` block in your Niri config:

```kdl
Mod+Shift+M repeat=false allow-inhibiting=false { spawn "keymap" "toggle"; }
```

- Use the full path to `keymap` if it is not on the `PATH` Niri starts with.
  A `PATH` entry such as `~/.local/bin` often comes from a shell profile that
  a display manager does not read. `keymap check` prints the line with the
  path that will work.
- `allow-inhibiting=false` keeps the shortcut working while Keymap is open.
  Keymap blocks other Niri shortcuts so that your keys reach it.
- `repeat=false` stops a held shortcut from opening and closing Keymap
  repeatedly.

See [examples/niri/launch-bind.kdl](../examples/niri/launch-bind.kdl).

`keymap shortcut set` refuses a combination that is already in use and
changes nothing. If you already have a Keymap shortcut, only its key changes,
so its options and command (such as an `env` wrapper) are kept.

### Using the GL renderer

If GTK's Vulkan renderer fails on your system, the first open takes about
0.8 s longer. Start Keymap with the GL renderer instead:

```kdl
Mod+Shift+M repeat=false allow-inhibiting=false { spawn "env" "GSK_RENDERER=gl" "keymap" "toggle"; }
spawn-at-startup "env" "GSK_RENDERER=gl" "keymap" "preload"
```

Keymap still recognizes both lines as its own.

## Background blur

Niri blurs layer surfaces only when a layer rule asks for it. Its default
"xray" mode shows only the wallpaper. Keymap needs one rule that matches only
its own `keymap` namespace:

```kdl
layer-rule {
    match namespace="^keymap$"
    background-effect {
        blur true
        xray false
    }
}
```

`keymap blur enable` adds it unless it is already there. `keymap blur-rule`
prints it. Settings → Background can add it too. Niri 26.04 marks non-xray
blur as experimental. See also
[examples/niri/blur-rule.kdl](../examples/niri/blur-rule.kdl).

**Strength.** Niri has no per-window blur strength. Its only strength setting
is the global `blur { passes; offset }` section, which applies to everything
that uses blur. Keymap offers four presets: Subtle (1, 2), Light (2, 2.5),
Medium (3, 2) and Strong (3, 3, Niri's default).

`keymap blur enable` sets Light only if your config has no `blur` section; it
never overrides one you wrote. `keymap blur subtle|light|medium|strong`
changes only `passes` and `offset` in your last `blur` section, or appends a
commented section if there is none.

Without the rule, or with blur turned off in Keymap, Keymap dims the screen
instead and makes the keys more opaque. If your own layer rules already blur
every layer, Keymap cannot turn blur off; Settings says so.

## How edits are made safe

1. **Finds the config.** Keymap uses the config of the running Niri (`-c`,
   `NIRI_CONFIG`, then the default location). Otherwise it uses Niri's default
   lookup. `--config` overrides both.
2. **Parses losslessly.** It follows every include with Niri's rules,
   including `optional=true`, `~/` and cycle detection. A config with errors
   opens read-only, with file and line.
3. **Edits only what changes.** Each change patches only the bytes of the
   affected declaration. Comments, formatting and unrelated settings stay
   byte for byte.
4. **Validates before writing.** The whole include graph is copied to a
   staging directory and checked with `niri validate`. Your files are not
   written until validation passes.
5. **Writes safely.** Keymap refuses to overwrite a file that changed since it
   was loaded. It writes a backup under `~/.local/state/keymap/backups/`
   (the last 30 are kept), then replaces each file atomically. Symlinks and
   file modes are kept.
6. **Reloads.** Niri reloads the config by itself, and Keymap follows edits
   made by other programs.

**Undo** restores the previous content only if the files still match what
Keymap wrote. A later edit by you or another program blocks Undo instead of
being overwritten. If Keymap is interrupted while saving, Settings →
Interrupted saves offers to restore the previous files.

An edit that touches several files is validated as a whole. Each file is
replaced atomically, but the set is not one atomic step.

### Undoing changes by hand

Each change is stored in `~/.local/state/keymap/backups/<id>/`.
`journal.json` lists the files, and `0.before` and `0.after` hold each file's
content before and after. Copy a `.before` file back over the listed path to
restore it.

## Shortcuts and keyboard layouts

Keymap follows Niri: the first XKB layout decides which key a shortcut uses,
or the first layout with Latin letters. Key labels always show US positions,
whatever layouts you use. When a key's shortcut name differs from its label,
hovering the key says so. Non-Latin shortcuts that no key can produce stay in
the action list and remain editable. Left and right modifiers are drawn
separately, but Niri does not tell them apart.

Actions are grouped by their exact content: the action, its arguments and its
options. `spawn "firefox"` and `spawn "firefox" "--private-window"` are
different actions. Custom action names and categories are Keymap's own
labels, stored in `~/.config/keymap/actions.json`. They are never written into
the Niri config.

**Read-only files.** A file you cannot write, such as `/etc/niri/config.kdl`,
is never edited and no root access is requested. If a later, writable `binds`
block exists, replacing a shortcut adds an override there. Removing a shortcut
defined in a read-only file is refused, with an explanation.

## Demo mode

`keymap --demo` copies [examples/configs/basic/](../examples/configs/basic/),
a synthetic config, to a temporary directory and edits that copy. A yellow
badge says so. The copy and its backups are deleted when Keymap exits.

## Performance

These figures come from a release build:

- A cold start shows Keymap after about 0.6 s, mostly GTK start-up.
- When Keymap is already running, it appears about 20–30 ms after the request.
- It uses about 160 MB resident memory (about 75 MB proportional set size)
  while running, shown or hidden. That memory stayed flat over 100 open and
  close cycles.

`KEYMAP_DEBUG=1` prints a timing trace.
