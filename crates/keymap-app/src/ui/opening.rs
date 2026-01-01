//! The opening: keys ripple in from the keyboard's centre while one quiet
//! sound plays. Everything is prepared before the window maps, so the first
//! frame already shows the starting state; the timeline then runs on the
//! window's frame clock and starts (with the sound) on that first frame.
//! Any real interaction ends it at once; hiding cancels it.
//!
//! The ripple only changes key opacity, so keys keep their size and the
//! layout never changes while they appear.

use super::{State, Ui, View, trace};
use crate::setup;
use crate::sound::Cue;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use keymap::motion;
use std::cell::Cell;
use std::rc::Rc;

/// A key on the keyboard and its ripple distance (`motion::distances`),
/// computed when the keyboard is built.
pub struct KeyBox {
    pub widget: gtk::Widget,
    pub distance: f64,
}

impl Ui {
    /// Called by `show` just before the window maps.
    pub(super) fn begin_opening(self: &Rc<Self>) {
        self.cancel_ripple("replaced by a new opening");
        let Some(win) = self.win.borrow().clone() else { return };
        let first = self.claim_first_open();
        let (motion_on, sounds, keyboard) = {
            let st: std::cell::Ref<State> = self.st.borrow();
            (st.settings.animations && !st.settings.reduced_motion, st.settings.sounds, st.view == View::Keyboard)
        };
        // Reopening into an unfinished form skips the ripple: the keyboard is
        // dimmed behind the panel there.
        let ripple = motion_on && keyboard && !win.key_boxes.borrow().is_empty();
        let total = if first { motion::FIRST_OPEN_SECS } else { motion::LATER_OPEN_SECS };
        let cue = match (sounds, ripple, first) {
            (false, _, _) => None,
            (true, true, true) => Some(Cue::First),
            (true, true, false) => Some(Cue::Later),
            (true, false, _) => Some(Cue::Quick),
        };
        trace(&format!(
            "opening: {}{}, sound {}",
            if ripple { format!("ripple {:.0} ms", total * 1000.0) } else { "no ripple".into() },
            if first { " (first in session)" } else { "" },
            cue.map_or("off".into(), |c| format!("{c:?}")),
        ));
        // Starting state, applied while still unmapped: no flash of the
        // finished keyboard.
        if ripple {
            self.apply_ripple(0.0, total);
        }
        // Debugging aid: pause the ripple at a given moment (it still counts
        // as running, so skipping and cancelling can be inspected).
        let hold = std::env::var("KEYMAP_DEBUG_RIPPLE_AT").ok().and_then(|v| v.parse::<f64>().ok()).filter(|_| ripple);
        if let Some(t) = hold {
            trace(&format!("ripple: held at {t} s"));
        }
        let start: Cell<Option<i64>> = Cell::new(None);
        let frames = Cell::new(0u32);
        let stats = FrameStats::attach(&win.window);
        let weak = Rc::downgrade(self);
        let id = win.window.add_tick_callback(move |_, clock| {
            let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
            let now = clock.frame_time();
            frames.set(frames.get() + 1);
            let update = std::time::Instant::now();
            let t0 = match start.get() {
                Some(t0) => t0,
                None => {
                    // The first frame on screen: start the timeline and the sound together.
                    trace("ripple: first frame");
                    if let Some(c) = cue {
                        ui.sound.play(c);
                    }
                    start.set(Some(now));
                    now
                }
            };
            let t = hold.unwrap_or((now - t0) as f64 / 1e6);
            if !ripple || t >= total {
                if ripple {
                    trace(&format!("ripple: finished after {} frames", frames.get()));
                }
                // Returning Break removes this callback; just forget its id.
                ui.ripple.borrow_mut().take();
                ui.rest_keys();
                return glib::ControlFlow::Break;
            }
            ui.apply_ripple(t, total);
            if let Some(s) = &stats {
                s.updated(now, update.elapsed());
            }
            glib::ControlFlow::Continue
        });
        *self.ripple.borrow_mut() = Some(id);
    }

    /// Opacity only: GTK skips drawing keys at 0 (they still receive clicks)
    /// and ignores unchanged values, so each frame redraws only keys in motion.
    fn apply_ripple(&self, t: f64, total: f64) {
        let Some(win) = self.win.borrow().clone() else { return };
        for k in win.key_boxes.borrow().iter() {
            k.widget.set_opacity(motion::key_opacity(t, k.distance, total));
        }
    }

    /// Every key exactly at rest.
    fn rest_keys(&self) {
        let Some(win) = self.win.borrow().clone() else { return };
        for k in win.key_boxes.borrow().iter() {
            k.widget.set_opacity(1.0);
        }
    }

    /// A window's first frames are slow (first layout, GL shaders, glyph
    /// uploads) and would freeze the start of the ripple. After preload,
    /// draw a few frames while effectively invisible instead: opacity
    /// 0.004 (rounds to nothing on screen), on the background layer under
    /// all windows, no blur, no keyboard input. An opening asked for meanwhile
    /// waits for it (`open_after_warm`).
    pub(super) fn warm_renderer(self: &Rc<Self>) {
        let Some(win) = self.win.borrow().clone() else { return };
        if win.window.is_visible() || self.warming.borrow().is_some() || std::env::var_os("KEYMAP_NO_WARMUP").is_some() {
            return;
        }
        let w = &win.window;
        w.set_namespace(Some(setup::NAMESPACE_PLAIN));
        w.set_layer(Layer::Background);
        w.set_keyboard_mode(KeyboardMode::None);
        w.add_css_class("km-warming");
        // Half-transparent keys first (the ripple's drawing path), then at rest.
        for k in win.key_boxes.borrow().iter() {
            k.widget.set_opacity(0.5);
        }
        trace("warm-up: drawing hidden frames");
        let frames = Cell::new(0u32);
        let weak = Rc::downgrade(self);
        let tick = w.add_tick_callback(move |_, _| {
            let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
            frames.set(frames.get() + 1);
            match frames.get() {
                1 => glib::ControlFlow::Continue,
                2 => {
                    ui.rest_keys();
                    glib::ControlFlow::Continue
                }
                _ => {
                    if let Some((_, timeout)) = ui.warming.borrow_mut().take() {
                        timeout.remove();
                    }
                    ui.end_warm("done");
                    glib::ControlFlow::Break
                }
            }
        });
        // Niri may not send frame callbacks to a fully covered surface; never
        // stay mapped for long.
        let weak = Rc::downgrade(self);
        let timeout = glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
            if let Some(ui) = weak.upgrade()
                && let Some((tick, _)) = ui.warming.borrow_mut().take()
            {
                tick.remove();
                ui.end_warm("timed out");
            }
        });
        *self.warming.borrow_mut() = Some((tick, timeout));
        w.set_visible(true);
    }

    /// Unmap and restore the overlay's real surface settings.
    fn end_warm(self: &Rc<Self>, reason: &str) {
        let Some(win) = self.win.borrow().clone() else { return };
        let w = &win.window;
        w.set_visible(false);
        let blur = self.st.borrow().settings.blur;
        w.set_namespace(Some(if blur { setup::NAMESPACE_BLUR } else { setup::NAMESPACE_PLAIN }));
        w.set_layer(Layer::Overlay);
        w.set_keyboard_mode(KeyboardMode::Exclusive);
        w.remove_css_class("km-warming");
        self.rest_keys();
        self.warmed.set(true);
        trace(&format!("warm-up: {reason}"));
        if self.open_after_warm.replace(false) {
            // After this unmap is processed, so the opening maps afresh.
            let weak = Rc::downgrade(self);
            glib::idle_add_local_once(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.show();
                }
            });
        }
    }

    /// Real interaction during the ripple: show the finished keyboard now
    /// (the caller then handles the input as usual) and fade the sound out.
    pub(super) fn finish_ripple(&self, reason: &str) {
        if let Some(id) = self.ripple.borrow_mut().take() {
            id.remove();
            trace(&format!("ripple: skipped ({reason})"));
            self.rest_keys();
            self.sound.stop();
        }
    }

    /// Hiding or rebuilding: drop the timeline and its sound. (Other sounds,
    /// such as the saved tone as the keyboard is rebuilt, play on.)
    pub(super) fn cancel_ripple(&self, reason: &str) {
        if let Some(id) = self.ripple.borrow_mut().take() {
            id.remove();
            trace(&format!("ripple: cancelled ({reason})"));
            self.rest_keys();
            self.sound.stop();
        }
    }

    /// The first visible opening in this desktop session plays the slow
    /// ripple. Preloading never gets here, so it does not use it up.
    fn claim_first_open(&self) -> bool {
        if self.opened_in_session.replace(true) {
            return false;
        }
        match (motion::marker_dir(), motion::session_key()) {
            (Some(dir), Some(key)) => motion::claim_first_open(&dir, &key),
            // No session to tie the marker to: count per process.
            _ => true,
        }
    }
}

/// Per-frame cost of the ripple, collected only with `KEYMAP_DEBUG`: the
/// update (key properties), layout and paint phases measured from the frame
/// clock's signals, and the interval between frames.
struct FrameStats {
    rows: std::rc::Rc<std::cell::RefCell<Vec<[f64; 5]>>>,
    mark: std::rc::Rc<Cell<Option<std::time::Instant>>>,
    cpu_mark: std::rc::Rc<Cell<f64>>,
    handlers: Vec<glib::SignalHandlerId>,
    clock: gtk::gdk::FrameClock,
    last: Cell<Option<i64>>,
}

impl FrameStats {
    fn attach(window: &gtk::ApplicationWindow) -> Option<FrameStats> {
        std::env::var_os("KEYMAP_DEBUG")?;
        let clock = window.frame_clock()?;
        let rows: std::rc::Rc<std::cell::RefCell<Vec<[f64; 5]>>> = Default::default();
        let mark: std::rc::Rc<Cell<Option<std::time::Instant>>> = Default::default();
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        // Connected after GDK's own handlers, so each fires when that phase ends.
        let cpu_mark: std::rc::Rc<Cell<f64>> = Default::default();
        let (r, m, c) = (rows.clone(), mark.clone(), cpu_mark.clone());
        let layout = clock.connect_layout(move |_| {
            if let (Some(t), Some(row)) = (m.get(), r.borrow_mut().last_mut()) {
                row[1] = ms(t.elapsed());
            }
            m.set(Some(std::time::Instant::now()));
            c.set(thread_cpu_ms());
        });
        let (r, m, c) = (rows.clone(), mark.clone(), cpu_mark.clone());
        let paint = clock.connect_paint(move |_| {
            if let (Some(t), Some(row)) = (m.get(), r.borrow_mut().last_mut()) {
                row[2] = ms(t.elapsed());
                row[4] = thread_cpu_ms() - c.get();
            }
            m.set(None);
        });
        Some(FrameStats { rows, mark, cpu_mark, handlers: vec![layout, paint], clock, last: Cell::new(None) })
    }

    fn updated(&self, frame_time: i64, update: std::time::Duration) {
        let interval = self.last.replace(Some(frame_time)).map_or(0.0, |l| (frame_time - l) as f64 / 1000.0);
        self.rows.borrow_mut().push([update.as_secs_f64() * 1000.0, 0.0, 0.0, interval, 0.0]);
        self.mark.set(Some(std::time::Instant::now()));
        self.cpu_mark.set(thread_cpu_ms());
    }

    fn report(&self) {
        let rows = self.rows.borrow();
        if rows.is_empty() {
            return;
        }
        let (refresh, _) = self.clock.refresh_info(0);
        let refresh_ms = if refresh > 0 { refresh as f64 / 1000.0 } else { 1000.0 / 60.0 };
        let col = |i: usize| {
            let mut v: Vec<f64> = rows.iter().map(|r| r[i]).filter(|x| *x > 0.0).collect();
            v.sort_by(f64::total_cmp);
            let pick = |q: f64| v.get(((v.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0);
            format!("median {:.2} p95 {:.2} max {:.2}", pick(0.5), pick(0.95), pick(1.0))
        };
        if std::env::var_os("KEYMAP_DEBUG_FRAMES").is_some() {
            for (i, r) in rows.iter().enumerate() {
                trace(&format!(
                    "  frame {i:>3}: update {:.2} layout {:.2} paint {:.2} (cpu {:.2}) interval {:.2}",
                    r[0], r[1], r[2], r[4], r[3]
                ));
            }
        }
        let missed: usize =
            rows.iter().map(|r| r[3]).filter(|i| *i > 0.0).map(|i| ((i / refresh_ms).round() as usize).saturating_sub(1)).sum();
        trace(&format!(
            "ripple frames: {} at refresh {:.2} ms, missed vblanks {missed}; update [{}] layout [{}] paint [{}] paint-cpu [{}] interval [{}] ms",
            rows.len(),
            refresh_ms,
            col(0),
            col(1),
            col(2),
            col(4),
            col(3)
        ));
    }
}

impl Drop for FrameStats {
    fn drop(&mut self) {
        // Finished, skipped or cancelled: report once.
        self.report();
        for h in self.handlers.drain(..) {
            self.clock.disconnect(h);
        }
    }
}

/// CPU time of the calling thread, in milliseconds.
fn thread_cpu_ms() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid pointer to a timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1e6
}
