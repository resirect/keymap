//! Everything `keymap` (the command-line client) and `keymap-ui` (the
//! resident overlay) share. Nothing here uses GTK, so the client starts in a
//! few milliseconds instead of loading the GUI toolkit for every key press.

pub mod instance;
pub mod motion;
pub mod setup;

use std::path::PathBuf;

pub const USAGE: &str = "\
keymap — glass keyboard shortcut manager for Niri

Usage:
  keymap [show]        Show the overlay (starts Keymap if needed)
  keymap toggle        Show or hide the overlay (starts Keymap if needed)
  keymap hide          Hide the overlay and release input (Keymap keeps running)
  keymap preload       Start Keymap hidden and prepare it, if it is not running
                       (for Niri's spawn-at-startup; never shows the overlay)
  keymap quit          Stop the running Keymap instance
  keymap autostart [enable|disable|status]
                       Add or remove `spawn-at-startup \"keymap\" \"preload\"` in
                       your Niri config (validated, backed up, undoable)
  keymap shortcut [status|set [COMBO]|remove]
                       Show, set or remove the Niri shortcut that opens Keymap
                       (default Mod+Shift+M = Super+Shift+M). `set` moves Keymap's own bind,
                       keeping its properties; refuses a combination another
                       action uses
  keymap check         Print what Keymap reads from your Niri config (no UI, read-only)
  keymap blur [status|enable|subtle|light|medium|strong]
                       `enable` adds Keymap's blur layer rule (and, if your config
                       sets no blur strength, Light); a strength sets Niri's global
                       blur { passes offset } (validated, backed up, undoable)
  keymap blur-rule     Print the app-scoped Niri layer rule used for background blur

Keymap stays running in the background after the overlay hides, so opening it
again is immediate. `keymap quit` ends it; the next show starts it again.

Options:
  --config PATH        Use this Niri config instead of the discovered one
  --demo               Demo mode: edit a temporary copy of a synthetic config
  -h, --help           Show this help
";

pub struct Options {
    pub config: Option<PathBuf>,
    pub demo: bool,
}

/// Parse the command line into options and the command (default `show`).
pub fn parse_args() -> (Options, String) {
    let mut args = std::env::args().skip(1);
    let mut opts = Options { config: None, demo: false };
    let mut command = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "--demo" => opts.demo = true,
            "--config" => match args.next() {
                Some(p) => opts.config = Some(PathBuf::from(p)),
                None => die("--config needs a path"),
            },
            s if s.starts_with("--config=") => opts.config = Some(PathBuf::from(&s[9..])),
            "show" | "hide" | "toggle" | "preload" | "quit" | "check" | "blur-rule" if command.is_none() => command = Some(a),
            "blur" if command.is_none() => {
                let action = args.next().unwrap_or_else(|| "status".into());
                command = Some(format!("blur {action}"));
            }
            "shortcut" if command.is_none() => {
                let action = args.next().unwrap_or_else(|| "status".into());
                command = Some(match action.as_str() {
                    "set" => format!("shortcut set {}", args.next().unwrap_or_else(|| setup::DEFAULT_LAUNCH.into())),
                    _ => format!("shortcut {action}"),
                });
            }
            "autostart" if command.is_none() => {
                let action = args.next().unwrap_or_else(|| "status".into());
                command = Some(format!("autostart {action}"));
            }
            other => die(&format!("unknown argument `{other}`\n\n{USAGE}")),
        }
    }
    (opts, command.unwrap_or_else(|| "show".into()))
}

/// Send `cmd` to the running instance. Returns the command a new instance
/// should start with, or `None` when there is nothing more to do.
pub fn deliver(cmd: instance::Command, opts: &Options) -> Option<instance::Command> {
    use instance::Command;
    match instance::send(cmd, opts.demo) {
        Ok(true) => None,
        // Nothing running: plain hide and quit have nothing to do.
        Ok(false) => match cmd {
            Command::Hide | Command::Quit => None,
            c => Some(c),
        },
        Err(e) => die(&format!("cannot reach the running instance: {e}")),
    }
}

pub fn die(msg: &str) -> ! {
    eprintln!("keymap: {msg}");
    std::process::exit(2);
}
