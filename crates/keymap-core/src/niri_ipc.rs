//! Minimal client for Niri's JSON IPC socket (`$NIRI_SOCKET`): one request
//! per connection, newline-delimited JSON, as documented for niri-ipc.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub fn request(req: &str) -> Result<Value, String> {
    let path = std::env::var("NIRI_SOCKET").map_err(|_| "NIRI_SOCKET is not set (not running under Niri?)".to_string())?;
    let mut s = UnixStream::connect(&path).map_err(|e| format!("cannot connect to Niri IPC: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok();
    s.write_all(format!("{}\n", serde_json::to_string(req).unwrap_or_default()).as_bytes()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&line).map_err(|e| format!("bad IPC reply: {e}"))?;
    match v.get("Ok") {
        Some(ok) => Ok(ok.get(req).cloned().unwrap_or(Value::Null)),
        None => Err(v.get("Err").and_then(Value::as_str).unwrap_or("IPC error").to_string()),
    }
}

/// Name of the output with the focused window, or the focused output.
pub fn focused_output() -> Option<String> {
    let v = request("FocusedOutput").ok()?;
    v.get("name").and_then(Value::as_str).map(str::to_string)
}

pub fn version() -> Option<String> {
    request("Version").ok().and_then(|v| v.as_str().map(str::to_string))
}
