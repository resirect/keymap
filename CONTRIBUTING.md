# Contributing

Bug reports and pull requests are welcome.

## Reporting a problem

Please include:

- your distribution, and the versions of Niri and GTK;
- what you did, what you expected and what happened;
- the output of `keymap check`, if it is relevant.

Remove anything private before posting. That includes your home directory
name in paths and personal commands from your Niri config.

## Building and testing

The build dependencies are listed in the [README](README.md#requirements).

```sh
cargo test --workspace                 # tests; those that need niri skip themselves without it
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run -p keymap --bin keymap -- --demo   # practice on the synthetic config
```

To work in an isolated nested Niri session that never touches your real
configuration:

```sh
scripts/dev/nested-niri.sh /tmp/keymap-nested
```

Other debugging aids:

- `KEYMAP_DEBUG=1` prints a timing trace and explains opening decisions.
- `KEYMAP_DEBUG_RIPPLE_AT=0.3` pauses the opening animation at 0.3 s.
- `python3 scripts/gen-sounds.py` regenerates the sound files
  deterministically.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how the code is
organized.

## Pull requests

- Keep changes focused, and add a test for any change to how configs are read
  or written.
- Use test data that is clearly synthetic, never a real personal config.
- By contributing, you dedicate your contribution to the public domain under
  the same [CC0 1.0](LICENSE) terms as the rest of Keymap.
