//! `keymap`: the command-line client. It sends show/hide/toggle/preload/quit
//! to the resident overlay over its socket, and starts `keymap-ui` only when
//! none is running.

use keymap::instance::Command;
use keymap::{deliver, die, parse_args, setup};
use std::os::unix::process::CommandExt;

fn main() {
    let (opts, command) = parse_args();
    let setup_command = ["blur ", "shortcut ", "autostart "].iter().any(|p| command.starts_with(p));
    if setup_command || matches!(command.as_str(), "check" | "blur-rule") {
        // Printing commands exit quietly when piped into `head` and the like,
        // instead of panicking on a closed stdout.
        // SAFETY: called before any threads are started.
        unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    }
    if let Some(action) = command.strip_prefix("blur ") {
        std::process::exit(setup::blur(&opts, action));
    }
    if let Some(action) = command.strip_prefix("shortcut ") {
        std::process::exit(setup::shortcut(&opts, action));
    }
    if let Some(action) = command.strip_prefix("autostart ") {
        std::process::exit(setup::autostart(&opts, action));
    }
    match command.as_str() {
        "check" => std::process::exit(setup::check(&opts)),
        "blur-rule" => {
            print!("{}", setup::BLUR_RULE);
            return;
        }
        _ => {}
    }
    let cmd = Command::parse(&command).expect("validated above");
    let Some(start) = deliver(cmd, &opts) else { return };
    // Nothing running: become the resident overlay process.
    let mut ui = std::process::Command::new(ui_binary());
    if let Some(c) = &opts.config {
        ui.arg("--config").arg(c);
    }
    if opts.demo {
        ui.arg("--demo");
    }
    let err = ui.arg(start.as_str()).exec();
    die(&format!("cannot start keymap-ui: {err}"));
}

/// `keymap-ui` next to this executable (installed together), else from PATH.
fn ui_binary() -> std::path::PathBuf {
    std::env::current_exe().ok().map(|p| p.with_file_name("keymap-ui")).filter(|p| p.is_file()).unwrap_or_else(|| "keymap-ui".into())
}
