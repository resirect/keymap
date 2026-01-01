//! Single-instance control socket in `$XDG_RUNTIME_DIR`.
//!
//! The first `keymap` process binds the socket and owns the overlay; later
//! invocations send one line (`show`, `hide`, `toggle`, `preload`, `quit`)
//! and exit.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Show,
    Hide,
    Toggle,
    Preload,
    Quit,
}

impl Command {
    pub fn parse(s: &str) -> Option<Command> {
        Some(match s.trim() {
            "show" => Command::Show,
            "hide" => Command::Hide,
            "toggle" => Command::Toggle,
            "preload" => Command::Preload,
            "quit" => Command::Quit,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Command::Show => "show",
            Command::Hide => "hide",
            Command::Toggle => "toggle",
            Command::Preload => "preload",
            Command::Quit => "quit",
        }
    }
}

pub fn socket_path(demo: bool) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    let display = display.rsplit('/').next().unwrap_or("wayland-0").to_string();
    dir.join(format!("keymap-{display}{}.sock", if demo { "-demo" } else { "" }))
}

/// Send a command to a running instance. `Ok(false)` means none is running.
pub fn send(cmd: Command, demo: bool) -> std::io::Result<bool> {
    let path = socket_path(demo);
    let mut s = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    s.write_all(format!("{}\n", cmd.as_str()).as_bytes())?;
    let mut reply = String::new();
    BufReader::new(&s).read_line(&mut reply)?;
    Ok(reply.trim() == "ok")
}

/// Bind the control socket, replacing a stale one left by a crashed process.
pub fn bind(demo: bool) -> std::io::Result<UnixListener> {
    let path = socket_path(demo);
    match UnixListener::bind(&path) {
        Ok(l) => {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            Ok(l)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if UnixStream::connect(&path).is_ok() {
                return Err(e);
            }
            std::fs::remove_file(&path)?;
            let l = UnixListener::bind(&path)?;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            Ok(l)
        }
        Err(e) => Err(e),
    }
}

pub fn cleanup(demo: bool) {
    let _ = std::fs::remove_file(socket_path(demo));
}
