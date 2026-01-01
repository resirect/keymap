# Architecture

Keymap is one Rust binary built from two crates.

```
crates/keymap-core   no GUI; all logic that can corrupt a config or mislead the user
crates/keymap-app    GTK 4 + gtk4-layer-shell overlay, CLI and single-instance control
```

## Frontend

The overlay uses GTK 4 with gtk4-layer-shell, which provides everything it
needs:

- a layer surface with a namespace, so a Niri layer rule can blur behind it;
- exclusive keyboard focus;
- keyboard-shortcuts-inhibit.

Keymap does not depend on any desktop shell. All logic lives in the UI-agnostic
`keymap-core` crate.

## keymap-core

| Module | Responsibility |
| --- | --- |
| `kdl` | Lossless KDL v1 parser. Every node, entry and children block keeps its byte span. Handles slashdash, nested comments, raw strings and type annotations. It never re-serialises a document. |
| `config` | Finds the running Niri's config (`/proc` of the `$NIRI_SOCKET` owner) and walks includes with Niri 26.04 rules: top-level only, relative to the including file, `~/`, `optional=true`, cycle detection. Also reads `mod-key`, XKB settings, `blur { off }` and layer rules. |
| `keys` | Niri key syntax: modifiers, the `Mod` alias, keysyms (case-insensitive xkb lookup) and pointer triggers. `Combo` is the runtime identity Niri matches against, after resolving `Mod`. |
| `xkb` | Builds the user's keymap with libxkbcommon. For each physical key it computes the keysym Niri would match (Smithay's raw Latin rule) and modifier roles. Legends come separately from a plain US keymap, so the drawing is English whatever the layouts; `binds_as` records when the shortcut identity differs from the legend. |
| `geometry` | Data-driven ANSI 60/75/100 geometry, keyed by evdev code. |
| `catalog` | Every config-usable Niri 26.04 action, with parameters and documented defaults. |
| `model` | Emulates Niri's merge. Duplicates inside one block are errors, later blocks replace exact keys, and the first runtime match wins. Groups actions by canonical payload and attaches names (labels, `hotkey-overlay-title`, derived). |
| `plan` | Turns intents (assign, remove, unbind everywhere) into text edits on spans. Verifies each plan by re-parsing the edited text and rebuilding the model, so the intended combination must end up running the intended action. |
| `txn` | Hash checks, staging with remapped includes, `niri validate`, backups and journal, atomic replace, symlink and mode preservation, conditional undo, and recovery of interrupted saves. |
| `store` | Keymap's own data in XDG dirs: settings and display metadata for custom actions. It holds no binding data; Niri's config stays the source of truth. |
| `niri_ipc` | Minimal JSON IPC: version, focused output. |

`tests/pipeline.rs` drives the whole path against `examples/configs/basic`.

## keymap-app

- Two binaries. `keymap` (`main.rs`) is the client: it parses the CLI, runs
  `check`, `blur-rule`, `blur` and `autostart`, and sends show/hide/toggle/preload/quit
  to the running instance through `instance.rs` (a Unix socket in
  `$XDG_RUNTIME_DIR`). It does not link GTK, so it starts in a few
  milliseconds. When nothing is running it execs `keymap-ui` (`ui_main.rs`),
  which binds the socket (replacing a stale one; a process that loses the
  start-up race forwards its command to the winner) and runs the UI.
  `lib.rs` holds what both share (`Options`, argument parsing, `instance`,
  `setup`, and `motion`: the ripple's timing and the session marker).
- Opening (`ui/opening.rs`): before the window maps, every key is put at the
  ripple's starting state (transparent). One tick callback on the window's
  frame clock then fades all keys in from the first frame on, staggered by
  distance from the keyboard centre (`motion::distances`, computed once when
  the keyboard is built), and starts the sound on that same frame. Only
  opacity changes: layout, positions and hit areas never move, and GTK
  redraws only keys whose opacity changed. Panel warming waits until the
  ripple ends. Any click or new non-modifier key press completes it and is
  handled as usual in the same event; Esc hides; hiding or rebuilding the
  keyboard cancels it.
  The first visible opening of a Niri session takes ~1.47 s, later ones
  ~0.73 s. "First" is a marker file in `$XDG_RUNTIME_DIR/keymap/` named after
  the Niri IPC socket file (device, inode, creation time), created with
  `O_EXCL`, so a restarted Keymap stays fast while a new session starts slow,
  and racing instances cannot both claim it. Preloading never touches it.
- Fullscreen gate (`toplevels.rs`): `show`/`toggle` are ignored while the
  focused window is fullscreen, unless Settings allow it. Niri's IPC has no
  fullscreen field, so a thread with its own Wayland connection follows
  `zwlr_foreign_toplevel_manager_v1` (activated/fullscreen states and output
  names, applied on each handle's `done`) and the check is a mutex lookup plus
  the focused-output IPC request `show` makes anyway. The gate runs before
  `show()`, so a blocked request touches no window, warm-up queue, sound or
  first-opening marker. Unavailable state (no protocol, lost connection, no
  answer within 300 ms of start-up) means "not fullscreen".
- Sound (`sound.rs`): six clips generated by `scripts/gen-sounds.py` (three
  opening variants, select/deselect ticks, a saved tone) are compiled in and
  decoded once on a sound thread, which mixes them into one libpulse-simple
  stream (PipeWire provides it; loaded with `dlopen`, so without it Keymap is
  simply silent). The thread sleeps between sounds and closes the stream after
  2 s of silence. A stop or a new opening/saved cue fades what is playing over
  30 ms; a tick fades only an earlier tick, and is skipped if one started
  under 35 ms ago, so rapid clicks never stack or queue (at most 4 voices).
  The server-side buffer is kept at 20 ms. Ticks play only from
  `Ui::toggle_key` (a user's click or Space on a key; held-Space repeats are
  consumed), the saved tone only after a binding commit succeeds (`assign`,
  `remove_binding`); interaction sounds are ~10 dB (ticks) and ~6 dB (saved)
  below the opening sound by 400 ms-window energy.
- `ui/mod.rs` owns the window and focus lifecycle and the view state machine
  (Keyboard → Browser → Form/Confirm, and Keyboard → Settings). It also handles
  commit/undo and file watching (GIO file monitors on every config file and
  missing include, debounced). Watch events for bytes Keymap itself wrote are
  ignored by comparing hashes, so a pending draft survives its own save.
- Physical navigation keys (Tab on the keyboard view, Enter, Esc, Ctrl+N) are
  decided by the pure `nav_for(view, key, modifiers)` and handled by one
  capture-phase controller on the window, on the press. GDK synthesises key
  repeats client-side with the original press's timestamp, so `RepeatGuard`
  drops events that repeat the last key's timestamp (or, for timestamp-less
  virtual keyboards, arrive before its release): holding Esc goes back one
  level only.
- The process is resident. `preload` builds the window, keyboard, settings
  panel and action browser and realizes the surface (creating GTK's renderer).
  It then maps the window for three frames as a warm-up (`warm_renderer`):
  opacity 0.004, background layer, `keymap-noblur`, no keyboard interactivity,
  at most 1.5 s. A show during the warm-up waits for it, and a cold `show`
  (no preload, e.g. the first toggle after `quit`) runs the warm-up first,
  opening ~0.4 s later but smoothly; a hide meanwhile drops the pending show.
  This does the first layout, GL shader compilation and glyph uploads ahead
  of time, so they don't stall the first opening's ripple.
  `KEYMAP_NO_WARMUP=1` skips it. `show` maps it on the focused output (moving it first if
  that output changed) and `hide` unmaps it: with no mapped surface there is no
  blur, dimming, input capture or drawing, and the widgets stay built. The
  window is anchored to all edges with exclusive zone -1 and exclusive keyboard
  focus, in namespace `keymap` (blurred via the layer rule) or `keymap-noblur`,
  and requests keyboard-shortcuts-inhibit. A namespace change or a removed
  output rebuilds the window without touching the view state.
- Reopening follows the pure `reopen_view(view, form_dirty)`: the keyboard,
  unless a new action has typed details, which come back as the form (an open
  confirmation is cancelled). Hiding never saves or discards.
- Panels: the settings panel and the action browser are kept in the overlay.
  After the first frame of a show, a tick callback makes one per frame visible
  but parked (opacity 0, not targetable or focusable, hidden from assistive
  technology), so their layout is done before they are opened; opening one
  only unparks it (the browser first refreshes what depends on the selection).
  They are dropped and rebuilt when the model (`State.generation`) or output
  size changes. Forms and confirmations are built fresh.
- The browser's action list is a `ListView` over a sorted, filtered model with
  category section headers. The model is attached once the list has a size and
  grown 12 rows per frame through a `SliceListModel`, because GTK keeps up to
  200 rows alive and would otherwise lay out every row in one frame.
- The keyboard is the only child of its centred overlay slot, so it is centred
  on its own bounds. The hint above it is a separate overlay child placed by
  `place_hint` (`get-child-position`): at least as wide as the keyboard, ending
  14 px above it, growing upward and clipped before the screen edge. Output
  geometry and scale changes are coalesced into one rebuild. With
  `KEYMAP_DEBUG=1`, `bounds:` lines log both rectangles after each hint change.
- `ui/keyboard.rs`, `ui/browser.rs`, `ui/form.rs`, `ui/settings.rs` and
  `ui/widgets.rs` hold the views. `widgets::scroller` is the one scrolling
  container: a non-overlay scrollbar in its own fixed gutter, so rows can never
  sit under it. `style.rs` builds CSS scaled to the key unit from shared theme
  tokens (surface, line, accent, danger, focus) used by keys and menus alike.
- `setup.rs` produces the blur status and plans the blur rule, the blur
  strength and the session startup entry (`keymap blur`, `keymap autostart`;
  both commit through the same transactions as any edit). Niri has no
  per-surface blur strength, so the strength presets edit only `passes` and
  `offset` in the last global `blur { }` section (or append one). The
  installer's `keymap blur enable` adds a strength only when no section
  exists. `demo.rs`
  provides the temporary demo copy.

## Data locations

| Path | Contents |
| --- | --- |
| `~/.config/keymap/settings.json` | Appearance and layout |
| `~/.config/keymap/actions.json` | Custom actions and display labels |
| `~/.local/state/keymap/backups/<txid>/` | Journal, plus before/after copies of every changed file (last 30) |
| `$XDG_RUNTIME_DIR/keymap-<display>.sock` | Single-instance control socket |
