//! Whether the focused window is fullscreen, from the compositor's own state.
//!
//! Niri's IPC reports window sizes but not fullscreen, and a maximized window
//! can be exactly as large as a fullscreen one. The wlr-foreign-toplevel
//! protocol, which Niri implements, carries each window's real states
//! (activated, maximized, fullscreen) and the outputs it is on. A thread with
//! its own Wayland connection follows those events as they happen, so asking
//! costs a mutex lock: no polling, no subprocess.
//!
//! When the protocol is missing or the connection fails, the answer is
//! `Unknown` and Keymap opens as if nothing were fullscreen.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1 as handle, zwlr_foreign_toplevel_manager_v1 as manager,
};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Toplevel {
    pub activated: bool,
    pub fullscreen: bool,
    /// Names of the outputs the window is on (as Niri names them, e.g. "DP-1").
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Fullscreen,
    NotFullscreen,
    /// No window state is available; the reason is for the trace.
    Unknown(String),
}

/// Whether the focused window is fullscreen. Only the activated (focused)
/// window counts, so a fullscreen window on another monitor does not; when
/// the focused output is known, the window must also be on it.
pub fn focused_fullscreen<'a>(toplevels: impl IntoIterator<Item = &'a Toplevel>, focused_output: Option<&str>) -> bool {
    toplevels
        .into_iter()
        .any(|t| t.activated && t.fullscreen && focused_output.is_none_or(|o| t.outputs.is_empty() || t.outputs.iter().any(|x| x == o)))
}

#[derive(Default)]
struct Shared {
    /// The initial state has arrived.
    ready: bool,
    unavailable: Option<String>,
    toplevels: HashMap<ObjectId, Toplevel>,
}

type Tracker = Arc<(Mutex<Shared>, Condvar)>;
static TRACKER: OnceLock<Tracker> = OnceLock::new();

/// Start following window state (once; later calls do nothing).
pub fn start() {
    TRACKER.get_or_init(|| {
        let t: Tracker = Arc::default();
        let thread = t.clone();
        let spawned = std::thread::Builder::new().name("keymap-toplevels".into()).spawn(move || {
            let err = run(&thread).err().unwrap_or_else(|| "connection closed".into());
            let (m, cv) = &*thread;
            if let Ok(mut s) = m.lock() {
                s.unavailable = Some(err);
                s.toplevels.clear();
            }
            cv.notify_all();
        });
        if let Err(e) = spawned {
            t.0.lock().map(|mut s| s.unavailable = Some(e.to_string())).ok();
        }
        t
    });
}

/// The focused window's state. Waits at most `wait` for the first state
/// after start-up (a resident Keymap has it long before any key press).
pub fn focus(wait: Duration, focused_output: Option<&str>) -> Focus {
    start();
    let (m, cv) = &**TRACKER.get().expect("started");
    let Ok(guard) = m.lock() else { return Focus::Unknown("window state lock poisoned".into()) };
    let Ok((s, _)) = cv.wait_timeout_while(guard, wait, |s| !s.ready && s.unavailable.is_none()) else {
        return Focus::Unknown("window state lock poisoned".into());
    };
    if let Some(e) = &s.unavailable {
        return Focus::Unknown(e.clone());
    }
    if !s.ready {
        return Focus::Unknown(format!("no window state within {} ms", wait.as_millis()));
    }
    if focused_fullscreen(s.toplevels.values(), focused_output) { Focus::Fullscreen } else { Focus::NotFullscreen }
}

struct State {
    shared: Tracker,
    manager: bool,
    outputs: HashMap<ObjectId, String>,
    /// States are double-buffered: applied on each handle's `done`.
    pending: HashMap<ObjectId, Toplevel>,
}

fn run(shared: &Tracker) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland connection: {e}"))?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut st = State { shared: shared.clone(), manager: false, outputs: HashMap::new(), pending: HashMap::new() };
    // Globals, then the outputs' names and the existing windows' states.
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    if !st.manager {
        return Err("the compositor does not offer zwlr_foreign_toplevel_manager_v1".into());
    }
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    {
        let (m, cv) = &**shared;
        m.lock().map_err(|_| "lock poisoned")?.ready = true;
        cv.notify_all();
    }
    loop {
        queue.blocking_dispatch(&mut st).map_err(|e| e.to_string())?;
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(st: &mut State, r: &wl_registry::WlRegistry, e: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<State>) {
        if let wl_registry::Event::Global { name, interface, version } = e {
            match interface.as_str() {
                "zwlr_foreign_toplevel_manager_v1" if !st.manager => {
                    r.bind::<manager::ZwlrForeignToplevelManagerV1, _, _>(name, version.min(3), qh, ());
                    st.manager = true;
                }
                // Output names arrive from version 4.
                "wl_output" if version >= 4 => {
                    r.bind::<wl_output::WlOutput, _, _>(name, 4, qh, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(st: &mut State, o: &wl_output::WlOutput, e: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<State>) {
        if let wl_output::Event::Name { name } = e {
            st.outputs.insert(o.id(), name);
        }
    }
}

impl Dispatch<manager::ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(_: &mut State, _: &manager::ZwlrForeignToplevelManagerV1, _: manager::Event, _: &(), _: &Connection, _: &QueueHandle<State>) {}

    event_created_child!(State, manager::ZwlrForeignToplevelManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (handle::ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<handle::ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(st: &mut State, h: &handle::ZwlrForeignToplevelHandleV1, e: handle::Event, _: &(), _: &Connection, _: &QueueHandle<State>) {
        let id = h.id();
        let commit = |st: &mut State, t: Option<Toplevel>| {
            if let Ok(mut s) = st.shared.0.lock() {
                match t {
                    Some(t) => s.toplevels.insert(id.clone(), t),
                    None => s.toplevels.remove(&id),
                };
            }
        };
        match e {
            handle::Event::State { state } => {
                let states: Vec<u32> = state.as_chunks::<4>().0.iter().map(|c| u32::from_ne_bytes(*c)).collect();
                let t = st.pending.entry(id.clone()).or_default();
                t.activated = states.contains(&(handle::State::Activated as u32));
                t.fullscreen = states.contains(&(handle::State::Fullscreen as u32));
            }
            handle::Event::OutputEnter { output } => {
                let name = st.outputs.get(&output.id()).cloned().unwrap_or_default();
                let t = st.pending.entry(id.clone()).or_default();
                if !t.outputs.contains(&name) {
                    t.outputs.push(name);
                }
            }
            handle::Event::OutputLeave { output } => {
                let name = st.outputs.get(&output.id()).cloned().unwrap_or_default();
                st.pending.entry(id.clone()).or_default().outputs.retain(|x| *x != name);
            }
            handle::Event::Done => {
                let t = st.pending.get(&id).cloned().unwrap_or_default();
                commit(st, Some(t));
            }
            handle::Event::Closed => {
                st.pending.remove(&id);
                commit(st, None);
                h.destroy();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(activated: bool, fullscreen: bool, out: &str) -> Toplevel {
        Toplevel { activated, fullscreen, outputs: vec![out.into()] }
    }

    #[test]
    fn only_a_focused_fullscreen_window_counts() {
        // Nothing open, or an ordinary focused window.
        assert!(!focused_fullscreen(&[], Some("DP-1")));
        assert!(!focused_fullscreen(&[t(true, false, "DP-1")], Some("DP-1")));
        // Fullscreen and focused.
        assert!(focused_fullscreen(&[t(true, true, "DP-1")], Some("DP-1")));
        assert!(focused_fullscreen(&[t(true, true, "DP-1")], None));
        // A fullscreen window on another monitor, focus on an ordinary window here.
        assert!(!focused_fullscreen(&[t(false, true, "HDMI-A-1"), t(true, false, "DP-1")], Some("DP-1")));
        // Focus moved to an empty monitor while the other keeps its (stale) activated fullscreen window.
        assert!(!focused_fullscreen(&[t(true, true, "HDMI-A-1")], Some("DP-1")));
    }
}
