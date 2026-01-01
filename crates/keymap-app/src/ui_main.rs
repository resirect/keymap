//! `keymap-ui`: the resident overlay process. Normally started by `keymap`
//! when no instance is running; it can also be run directly with the same
//! arguments.

mod demo;
mod sound;
mod style;
mod toplevels;
mod ui;

use keymap::instance::{self, Command};
use keymap::{deliver, die, parse_args};
// The UI modules refer to it as `crate::setup`.
use keymap::setup;

fn main() {
    let (opts, command) = parse_args();
    let Some(cmd) = Command::parse(&command) else { die(&format!("`{command}` is handled by `keymap`, not keymap-ui")) };
    let Some(cmd) = deliver(cmd, &opts) else { return };
    let listener = match instance::bind(opts.demo) {
        Ok(l) => l,
        Err(_) => {
            // Lost a start-up race: another instance just bound the socket.
            match instance::send(cmd, opts.demo) {
                Ok(true) => return,
                _ => die("another Keymap instance is starting; try again"),
            }
        }
    };
    let code = ui::run(opts, listener, cmd != Command::Preload);
    std::process::exit(code);
}
