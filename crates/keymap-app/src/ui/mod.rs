//! GTK overlay: window/focus lifecycle, state, navigation and persistence.

mod browser;
mod form;
mod keyboard;
mod opening;
mod settings;
mod widgets;

use crate::demo::Demo;
use crate::sound::Cue;
use crate::style;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use keymap::Options;
use keymap::instance::{self, Command};
use keymap::setup;
use keymap_core::config::hash_bytes;
use keymap_core::keys::{Combo, Mods, Trigger};
use keymap_core::model::Model;
use keymap_core::plan::{self, AssignResult, Existing, Others, Plan, Replacement, Target};
use keymap_core::store::{self, Settings, Store};
use keymap_core::txn::{CommitError, NiriValidator, Receipt, Transactions, Validator};
use keymap_core::xkb::Resolver;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::rc::Rc;
use std::time::{Duration, Instant};

static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// With KEYMAP_DEBUG set, log a startup step and the time since launch.
pub(crate) fn trace(step: &str) {
    if std::env::var_os("KEYMAP_DEBUG").is_some() {
        let ms = START.get_or_init(Instant::now).elapsed().as_millis();
        eprintln!("keymap: {ms:>5} ms  {step}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Keyboard,
    Browser,
    Form,
    Confirm,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    This,
    All,
}

/// What a confirmation panel is asking about.
#[derive(Debug, Clone)]
pub enum Pending {
    /// The combination runs another action. `others` is the choice already
    /// made about the target action's own shortcuts.
    Replace {
        rep: Replacement,
        target: String,
        others: Others,
    },
    /// The action already has shortcuts: replace one, or add another.
    Existing {
        info: Existing,
        target: String,
        pick: Option<keymap_core::keys::Combo>,
    },
    Remove {
        binding: usize,
        combo: Combo,
        reveals: Option<String>,
        everywhere: bool,
    },
    DiscardForm,
}

/// An in-progress edit. Survives external config reloads.
#[derive(Debug, Clone)]
pub struct Draft {
    pub combo: Combo,
    /// Canonical payload of the chosen action.
    pub chosen: Option<String>,
    pub tab: Tab,
    pub search: String,
    pub error: Option<String>,
    pub info: Option<String>,
    pub form: Option<form::FormState>,
    pub pending: Option<Pending>,
    pub return_to: View,
}

pub struct State {
    pub settings: Settings,
    pub store: Store,
    pub model: Model,
    pub resolver: Resolver,
    /// Selected on-screen keys (evdev codes) in click order.
    pub selection: Vec<u32>,
    pub view: View,
    pub draft: Option<Draft>,
    pub hint: Option<(String, bool)>,
    pub invalid_keys: Vec<u32>,
    pub changed_on_disk: bool,
    pub settings_message: Option<String>,
    /// Bumped whenever `model` is replaced, so kept panels know to rebuild.
    pub generation: u64,
}

pub struct Win {
    pub window: gtk::ApplicationWindow,
    pub overlay: gtk::Overlay,
    /// Holds only the keyboard, centred on its own bounds.
    pub keyboard_slot: gtk::Box,
    /// Hints and messages, placed above the keyboard (see `place_hint`).
    pub hint: gtk::Box,
    /// The panel on screen, if any.
    pub panel: RefCell<Option<gtk::Widget>>,
    /// Kept panels: hidden while another view is shown, reused when reopened.
    pub settings: RefCell<Option<gtk::Widget>>,
    pub browser: RefCell<Option<browser::Browser>>,
    pub scrim: gtk::Box,
    pub tip: gtk::Box,
    pub toast: gtk::Box,
    pub monitor: RefCell<Option<gdk::Monitor>>,
    monitor_handlers: RefCell<Vec<glib::SignalHandlerId>>,
    pub keys: RefCell<HashMap<u32, gtk::Widget>>,
    /// The keys in layout order, with their opening-ripple distances.
    pub key_boxes: RefCell<Vec<opening::KeyBox>>,
}

pub struct Ui {
    pub app: gtk::Application,
    pub opts: Options,
    pub demo: Option<Demo>,
    pub st: RefCell<State>,
    pub win: RefCell<Option<Rc<Win>>>,
    /// Keeps the application running while the overlay is hidden.
    _hold: gio::ApplicationHoldGuard,
    watchers: RefCell<Vec<gio::FileMonitor>>,
    reload_pending: RefCell<Option<glib::SourceId>>,
    /// An output size/scale change is waiting to be applied (coalesced).
    geometry_pending: std::cell::Cell<bool>,
    toast_timer: RefCell<Option<glib::SourceId>>,
    /// Tells auto-repeat apart from new presses of navigation keys, so holding
    /// Esc goes back exactly one level while navigation still acts on press.
    nav_repeat: RefCell<RepeatGuard>,
    pub sound: crate::sound::Sound,
    /// The running opening timeline, if any (see `opening`).
    ripple: RefCell<Option<gtk::TickCallbackId>>,
    /// The hidden warm-up after preload (`opening::warm_renderer`), while it runs.
    warming: RefCell<Option<(gtk::TickCallbackId, glib::SourceId)>>,
    /// The warm-up has run once; the renderer is ready for a smooth opening.
    warmed: std::cell::Cell<bool>,
    /// An opening asked for during the warm-up, shown when it ends.
    open_after_warm: std::cell::Cell<bool>,
    /// This process already had a visible opening.
    opened_in_session: std::cell::Cell<bool>,
    pub tx: Transactions,
    validator: Box<dyn Validator>,
    pub niri_version: Option<String>,
    css: gtk::CssProvider,
}

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

fn with_ui(f: impl FnOnce(&Rc<Ui>)) {
    let ui = UI.with(|u| u.borrow().clone());
    if let Some(ui) = ui {
        f(&ui);
    }
}

fn handle_command(cmd: Command) {
    with_ui(|ui| {
        trace(&format!("command: {}", cmd.as_str()));
        match cmd {
            Command::Show => ui.request_show(),
            Command::Hide => ui.hide(),
            Command::Toggle => {
                if ui.is_shown() {
                    ui.hide()
                } else {
                    ui.request_show()
                }
            }
            Command::Preload => ui.preload(),
            Command::Quit => ui.app.quit(),
        }
    });
}

/// Validator for demo mode when niri is not installed: parse-only.
struct ParseOnly;
impl Validator for ParseOnly {
    fn validate(&self, _root: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
}

/// Run the resident UI. It stays running while hidden until `keymap quit`.
/// `show` is false for `keymap preload`, which prepares everything hidden.
pub fn run(opts: Options, listener: UnixListener, show: bool) -> i32 {
    START.get_or_init(Instant::now);
    let app = gtk::Application::builder().application_id("io.github.resirect.Keymap").flags(gio::ApplicationFlags::NON_UNIQUE).build();
    let demo_flag = opts.demo;
    let opts = RefCell::new(Some(opts));
    let listener = RefCell::new(Some(listener));
    // Quit cleanly on SIGTERM/SIGINT so the socket and demo files are removed.
    for sig in [15, 2] {
        let app = app.clone();
        glib_unix::unix_signal_add_local(sig, move || {
            app.quit();
            glib::ControlFlow::Break
        });
    }
    app.connect_activate(move |app| {
        let Some(opts) = opts.borrow_mut().take() else { return };
        trace("gtk activated");
        let ui = Ui::new(app.clone(), opts);
        trace("model loaded");
        UI.with(|u| *u.borrow_mut() = Some(ui.clone()));
        if let Some(l) = listener.borrow_mut().take() {
            serve(l, handle_command);
        }
        if show && !ui.blocked_by_fullscreen() {
            ui.show();
        } else {
            ui.preload();
        }
    });
    let code = app.run_with_args::<&str>(&[]);
    UI.with(|u| u.borrow_mut().take());
    instance::cleanup(demo_flag);
    code.into()
}

impl Ui {
    fn new(app: gtk::Application, opts: Options) -> Rc<Ui> {
        let demo = if opts.demo {
            match Demo::create() {
                Ok(d) => Some(d),
                Err(e) => {
                    eprintln!("keymap: cannot create demo directory: {e}");
                    std::process::exit(1);
                }
            }
        } else {
            None
        };
        let (settings, store) =
            if demo.is_some() { (Settings::default(), Store::default()) } else { (store::load_settings(), store::load_store()) };
        let explicit = demo.as_ref().map(|d| d.config()).or(opts.config.clone()).or(settings.config_path.clone());
        let model = setup::load_model(explicit.as_deref(), &store);
        trace("config parsed");
        let resolver = Resolver::new(&model.config.xkb);
        let tx = match &demo {
            Some(d) => Transactions { dir: d.state_dir().join("backups") },
            None => Transactions::default(),
        };
        let niri_available = std::process::Command::new("niri").arg("--version").output().is_ok();
        let validator: Box<dyn Validator> =
            if niri_available || demo.is_none() { Box::new(NiriValidator::default()) } else { Box::new(ParseOnly) };
        let hold = app.hold();
        // Decodes the sounds on its own thread; never blocks start-up.
        let sound = crate::sound::Sound::start(settings.volume);
        if !settings.open_over_fullscreen {
            crate::toplevels::start();
        }
        let ui = Rc::new(Ui {
            app,
            opts,
            demo,
            st: RefCell::new(State {
                settings,
                store,
                model,
                resolver,
                selection: Vec::new(),
                view: View::Keyboard,
                draft: None,
                hint: None,
                invalid_keys: Vec::new(),
                changed_on_disk: false,
                settings_message: None,
                generation: 0,
            }),
            win: RefCell::new(None),
            _hold: hold,
            watchers: RefCell::new(Vec::new()),
            reload_pending: RefCell::new(None),
            geometry_pending: std::cell::Cell::new(false),
            toast_timer: RefCell::new(None),
            nav_repeat: RefCell::new(RepeatGuard::default()),
            sound,
            ripple: RefCell::new(None),
            warming: RefCell::new(None),
            warmed: std::cell::Cell::new(false),
            open_after_warm: std::cell::Cell::new(false),
            opened_in_session: std::cell::Cell::new(false),
            tx,
            validator,
            niri_version: {
                trace("validator ready");
                keymap_core::niri_ipc::version()
            },
            css: gtk::CssProvider::new(),
        });
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &ui.css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            let weak = Rc::downgrade(&ui);
            display.monitors().connect_items_changed(move |_, _, _, _| {
                if let Some(ui) = weak.upgrade() {
                    ui.on_monitors_changed();
                }
            });
        }
        ui.watch_files();
        ui
    }

    pub fn is_demo(&self) -> bool {
        self.demo.is_some()
    }

    fn explicit_path(&self) -> Option<std::path::PathBuf> {
        self.demo.as_ref().map(|d| d.config()).or(self.opts.config.clone()).or(self.st.borrow().settings.config_path.clone())
    }

    // ---------------------------------------------------------------- window

    fn target_monitor(&self) -> Option<gdk::Monitor> {
        let display = gdk::Display::default()?;
        let name = keymap_core::niri_ipc::focused_output();
        let monitors = display.monitors();
        let all: Vec<gdk::Monitor> = (0..monitors.n_items()).filter_map(|i| monitors.item(i).and_downcast()).collect();
        match name {
            Some(n) => all.into_iter().find(|m| m.connector().as_deref() == Some(n.as_str())),
            // Without IPC, let the compositor place the surface on the active output.
            None => None,
        }
    }

    /// Whether the overlay is on screen.
    pub fn is_shown(&self) -> bool {
        self.open_after_warm.get() || (self.warming.borrow().is_none() && self.win.borrow().as_ref().is_some_and(|w| w.window.is_visible()))
    }

    /// Prepare everything that does not need a visible surface: the window
    /// and its widgets, the keyboard, and the settings and browser panels.
    /// Nothing is mapped, so nothing appears on screen and no input is taken.
    pub fn preload(self: &Rc<Self>) {
        if self.win.borrow().is_none() {
            let monitor = self.target_monitor();
            self.build_window(monitor);
        }
        if let Some(win) = self.win.borrow().clone() {
            self.ensure_settings(&win);
            self.ensure_browser(&win);
            // Create the surface and GTK's renderer now (the renderer's start-up
            // is most of a cold show). Realizing does not map: the compositor
            // shows nothing and gives the surface no input until `show`.
            if !win.window.is_realized() {
                WidgetExt::realize(&win.window);
                trace("surface and renderer ready");
                self.warm_renderer();
            }
        }
        trace("preloaded (hidden)");
    }

    /// Whether an opening request is ignored: a fullscreen window has focus
    /// and opening over fullscreen apps is off. Decided before anything is
    /// shown, queued, animated or played, so a blocked press leaves no trace
    /// (and does not use up the first opening's ripple).
    pub fn blocked_by_fullscreen(&self) -> bool {
        if self.st.borrow().settings.open_over_fullscreen {
            return false;
        }
        let output = keymap_core::niri_ipc::focused_output();
        match crate::toplevels::focus(std::time::Duration::from_millis(300), output.as_deref()) {
            crate::toplevels::Focus::Fullscreen => {
                trace("open ignored: the focused window is fullscreen");
                true
            }
            crate::toplevels::Focus::NotFullscreen => false,
            crate::toplevels::Focus::Unknown(why) => {
                trace(&format!("fullscreen state unknown ({why}); opening"));
                false
            }
        }
    }

    /// `show`/`toggle` from the shortcut: opens unless a focused fullscreen
    /// window blocks it. Already visible, it just stays visible.
    pub fn request_show(self: &Rc<Self>) {
        if self.is_shown() || !self.blocked_by_fullscreen() {
            self.show();
        }
    }

    /// Show the overlay on the focused output, reusing the resident window.
    pub fn show(self: &Rc<Self>) {
        // Opening cold (e.g. the first toggle after `keymap quit`) would run
        // the ripple on the renderer's slow first frames: warm up first and
        // open when it is done, a fraction of a second later.
        if self.warming.borrow().is_some() {
            self.open_after_warm.set(true);
            trace("show: after the warm-up");
            return;
        }
        if let Some(w) = self.win.borrow().as_ref()
            && w.window.is_visible()
        {
            w.window.present();
            return;
        }
        trace("show");
        let monitor = self.target_monitor();
        trace("focused output resolved");
        let existing = self.win.borrow().clone();
        match existing {
            Some(w) => {
                let moved = *w.monitor.borrow() != monitor;
                if moved {
                    self.move_to(&w, monitor);
                }
            }
            None => self.build_window(monitor),
        }
        if !self.warmed.get()
            && let Some(win) = self.win.borrow().clone()
        {
            if !win.window.is_realized() {
                WidgetExt::realize(&win.window);
                trace("surface and renderer ready");
            }
            self.warm_renderer();
            if self.warming.borrow().is_some() {
                self.open_after_warm.set(true);
                trace("show: after the warm-up");
                return;
            }
        }
        self.prepare_reopen();
        self.begin_opening();
        self.map_window();
    }

    /// Present the (built) window and warm the kept panels after its first
    /// frame, so showing the keyboard never waits for them.
    fn map_window(self: &Rc<Self>) {
        let Some(win) = self.win.borrow().clone() else { return };
        win.window.present();
        self.trace_frame("shown: first frame");
        let weak = Rc::downgrade(self);
        let frames = std::cell::Cell::new(0);
        win.window.add_tick_callback(move |_, _| {
            frames.set(frames.get() + 1);
            let Some(ui) = weak.upgrade() else { return glib::ControlFlow::Break };
            // One panel per frame, starting with the second frame, and never
            // while the opening ripple runs (it would take its frames).
            if frames.get() < 2 || ui.ripple.borrow().is_some() || ui.warm_one() {
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
    }

    fn build_window(self: &Rc<Self>, monitor: Option<gdk::Monitor>) {
        let blur = self.st.borrow().settings.blur;
        let window = gtk::ApplicationWindow::new(&self.app);
        window.add_css_class("keymap-overlay");
        window.set_decorated(false);
        window.init_layer_shell();
        window.set_namespace(Some(if blur { setup::NAMESPACE_BLUR } else { setup::NAMESPACE_PLAIN }));
        window.set_layer(Layer::Overlay);
        for e in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(e, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_monitor(monitor.as_ref());

        let overlay = gtk::Overlay::new();
        let backdrop = gtk::Box::new(gtk::Orientation::Vertical, 0);
        backdrop.add_css_class("km-backdrop");
        backdrop.set_hexpand(true);
        backdrop.set_vexpand(true);
        overlay.set_child(Some(&backdrop));
        // A click during the opening ripple completes it, then reaches its
        // target as usual (never claimed here).
        let skip = gtk::GestureClick::new();
        skip.set_button(0);
        skip.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        skip.connect_pressed(move |_, _, _, _| {
            if let Some(ui) = weak.upgrade() {
                ui.finish_ripple("click");
            }
        });
        overlay.add_controller(skip);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        click.connect_released(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(ui) = weak.upgrade() {
                ui.on_backdrop_click();
            }
        });
        backdrop.add_controller(click);

        let keyboard_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        keyboard_slot.set_halign(gtk::Align::Center);
        keyboard_slot.set_valign(gtk::Align::Center);
        // Resting focus when no key is selected: the container draws no focus
        // ring, so the opening shortcut's leftover modifier events (which make
        // GTK show focus for a few seconds) cannot light up a key. The first
        // arrow key moves focus onto a key as usual.
        keyboard_slot.set_focusable(true);
        keyboard_slot.update_property(&[gtk::accessible::Property::Label("Keyboard")]);
        overlay.add_overlay(&keyboard_slot);

        let hint = gtk::Box::new(gtk::Orientation::Vertical, 6);
        hint.add_css_class("km-hint-area");
        hint.set_valign(gtk::Align::End);
        hint.set_overflow(gtk::Overflow::Hidden);
        hint.set_can_target(false);
        overlay.add_overlay(&hint);
        let (h, slot) = (hint.clone(), keyboard_slot.clone());
        overlay.connect_get_child_position(move |ov, child| (child == h.upcast_ref::<gtk::Widget>()).then(|| place_hint(ov, &slot, child)));

        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("km-scrim");
        scrim.set_visible(false);
        let sclick = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        sclick.connect_released(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(ui) = weak.upgrade() {
                ui.back();
            }
        });
        scrim.add_controller(sclick);
        overlay.add_overlay(&scrim);

        let tip = gtk::Box::new(gtk::Orientation::Vertical, 4);
        tip.add_css_class("km-tip");
        tip.set_halign(gtk::Align::Start);
        tip.set_valign(gtk::Align::Start);
        tip.set_can_target(false);
        tip.set_visible(false);
        overlay.add_overlay(&tip);

        let toast = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        toast.add_css_class("km-toast");
        toast.set_halign(gtk::Align::Center);
        toast.set_valign(gtk::Align::End);
        toast.set_margin_bottom(48);
        toast.set_visible(false);
        overlay.add_overlay(&toast);

        window.set_child(Some(&overlay));

        let keys = gtk::EventControllerKey::new();
        // Capture phase: Tab/Enter/Esc keep their meaning even when an
        // on-screen key (a button) has focus.
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |c, key, _, state| match weak.upgrade() {
            Some(ui) => {
                let modifier = c.current_event().and_then(|e| e.downcast::<gdk::KeyEvent>().ok()).is_some_and(|e| e.is_modifier());
                ui.on_physical_key(key, state, c.current_event_time(), modifier)
            }
            None => glib::Propagation::Proceed,
        });
        let weak = Rc::downgrade(self);
        keys.connect_key_released(move |_, key, _, _| {
            if let Some(ui) = weak.upgrade() {
                ui.nav_repeat.borrow_mut().released(key);
            }
        });
        window.add_controller(keys);

        window.connect_map(|w| {
            if w.has_css_class("km-warming") {
                return;
            }
            trace("mapped");
            // Keep Niri's own bindings from firing behind the overlay
            // (keyboard-shortcuts-inhibit; binds with allow-inhibiting=false still work).
            if let Some(top) = w.surface().and_downcast::<gdk::Toplevel>() {
                if std::env::var_os("KEYMAP_DEBUG").is_some() {
                    top.connect_shortcuts_inhibited_notify(|t| eprintln!("keymap: shortcuts inhibited: {}", t.is_shortcuts_inhibited()));
                }
                top.inhibit_system_shortcuts(None::<&gdk::Event>);
            }
        });
        // Closing (should the compositor ask) only hides; Quit ends the process.
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.hide();
            }
            glib::Propagation::Stop
        });

        let win = Rc::new(Win {
            window: window.clone(),
            overlay,
            keyboard_slot,
            hint,
            panel: RefCell::new(None),
            settings: RefCell::new(None),
            browser: RefCell::new(None),
            scrim,
            tip,
            toast,
            monitor: RefCell::new(None),
            monitor_handlers: RefCell::new(Vec::new()),
            keys: RefCell::new(HashMap::new()),
            key_boxes: RefCell::new(Vec::new()),
        });
        *self.win.borrow_mut() = Some(win.clone());
        self.watch_monitor(&win, monitor);
        self.apply_css();
        self.render_all();
        trace("widgets built");
    }

    /// Hide: unmap the surface. The compositor drops its blur, dimming and
    /// input focus with it, and GTK stops drawing; the widgets stay built.
    pub fn hide(self: &Rc<Self>) {
        if self.warming.borrow().is_some() {
            // Closing before the warm-up ended: just don't open afterwards.
            if self.open_after_warm.replace(false) {
                trace("show cancelled before the warm-up ended");
            }
            return;
        }
        let Some(w) = self.win.borrow().clone() else { return };
        if !w.window.is_visible() {
            return;
        }
        self.cancel_ripple("hidden");
        self.sound.stop();
        w.window.set_visible(false);
        w.tip.set_visible(false);
        // A toast reports this opening's action; Undo stays in Settings → Recent changes.
        w.toast.set_visible(false);
        if let Some(t) = self.toast_timer.borrow_mut().take() {
            t.remove();
        }
        // Keyboard-focus feedback belongs to this opening only, and so do
        // held keys: a key that hid the overlay is released elsewhere.
        w.window.set_focus_visible(false);
        *self.nav_repeat.borrow_mut() = RepeatGuard::default();
        trace("hidden");
    }

    /// State for a reopened overlay; see `reopen_view`.
    fn prepare_reopen(self: &Rc<Self>) {
        let resume = {
            let mut st = self.st.borrow_mut();
            let dirty = st.draft.as_ref().and_then(|d| d.form.as_ref()).is_some_and(|f| f.dirty);
            let next = reopen_view(st.view, dirty);
            if next == View::Form {
                if let Some(d) = st.draft.as_mut() {
                    // A confirmation left open is cancelled, as Esc would.
                    d.pending = None;
                    d.return_to = View::Form;
                }
            } else {
                st.selection.clear();
                st.draft = None;
                st.hint = None;
                st.invalid_keys.clear();
                st.settings_message = None;
            }
            let changed = st.view != next;
            st.view = next;
            changed
        };
        keyboard::refresh_states(self);
        if resume {
            self.show_panel(false);
        } else if self.st.borrow().view == View::Keyboard {
            self.focus_keyboard();
        }
    }

    /// Rebuild the surface (namespace change, output removed) keeping state.
    pub fn recreate_window(self: &Rc<Self>) {
        let shown = self.is_shown();
        let w = self.win.borrow_mut().take();
        if let Some(w) = w {
            let guard = self.app.hold();
            w.window.destroy();
            let monitor = self.target_monitor();
            self.build_window(monitor);
            if shown {
                self.map_window();
            }
            drop(guard);
        }
    }

    /// Put the hidden window on another output before showing it there.
    fn move_to(self: &Rc<Self>, win: &Rc<Win>, monitor: Option<gdk::Monitor>) {
        win.window.set_monitor(monitor.as_ref());
        let old = self.monitor_size();
        self.watch_monitor(win, monitor);
        if self.monitor_size() != old {
            self.on_geometry_changed();
        }
    }

    fn watch_monitor(self: &Rc<Self>, win: &Rc<Win>, monitor: Option<gdk::Monitor>) {
        if let Some(old) = win.monitor.borrow_mut().take() {
            for h in win.monitor_handlers.borrow_mut().drain(..) {
                old.disconnect(h);
            }
        }
        if let Some(m) = &monitor {
            let weak = Rc::downgrade(self);
            let geometry = m.connect_geometry_notify(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.on_geometry_changed();
                }
            });
            let weak = Rc::downgrade(self);
            let scale = m.connect_scale_notify(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.on_geometry_changed();
                }
            });
            *win.monitor_handlers.borrow_mut() = vec![geometry, scale];
        }
        *win.monitor.borrow_mut() = monitor;
    }

    /// Output size or scale changed: resize the keyboard and panels once,
    /// however many notifications arrive together.
    fn on_geometry_changed(self: &Rc<Self>) {
        if self.geometry_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(ui) = weak.upgrade() {
                ui.geometry_pending.set(false);
                trace("output geometry changed");
                ui.apply_css();
                ui.render_all();
            }
        });
    }

    fn on_monitors_changed(self: &Rc<Self>) {
        let gone = self.win.borrow().as_ref().and_then(|w| w.monitor.borrow().clone()).is_some_and(|m| !m.is_valid());
        if gone {
            self.recreate_window();
        }
    }

    pub fn monitor_size(&self) -> (f64, f64) {
        let w = self.win.borrow();
        let mon = w.as_ref().and_then(|w| w.monitor.borrow().clone()).or_else(|| {
            let d = gdk::Display::default()?;
            d.monitors().item(0).and_downcast::<gdk::Monitor>()
        });
        mon.map(|m| {
            let g = m.geometry();
            (g.width() as f64, g.height() as f64)
        })
        .unwrap_or((1920.0, 1080.0))
    }

    /// With KEYMAP_DEBUG set, log the keyboard's and the hint's rectangles
    /// (overlay coordinates) once the next frame has been laid out, to check
    /// that text changes never move the keyboard.
    pub fn trace_bounds(&self) {
        if std::env::var_os("KEYMAP_DEBUG").is_none() {
            return;
        }
        let Some(win) = self.win.borrow().clone() else { return };
        let Some(fc) = win.window.frame_clock() else { return };
        let id: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        let id2 = id.clone();
        *id.borrow_mut() = Some(fc.connect_after_paint(move |fc| {
            let r = |w: &gtk::Widget| w.compute_bounds(&win.overlay).map(|b| (b.x(), b.y(), b.width(), b.height())).unwrap_or_default();
            let (x, y, w, h) = r(win.keyboard_slot.upcast_ref());
            let (hx, hy, hw, hh) = r(win.hint.upcast_ref());
            trace(&format!(
                "bounds: overlay {}x{}; keyboard {x}+{y} {w}x{h} centre ({:.1}, {:.1}); hint {hx}+{hy} {hw}x{hh}",
                win.overlay.width(),
                win.overlay.height(),
                x + w / 2.0,
                y + h / 2.0
            ));
            if let Some(h) = id2.borrow_mut().take() {
                fc.disconnect(h);
            }
        }));
    }

    /// With KEYMAP_DEBUG set, log when the next frame has been drawn.
    fn trace_frame(&self, what: &str) {
        if std::env::var_os("KEYMAP_DEBUG").is_none() {
            return;
        }
        let Some(fc) = self.win.borrow().as_ref().and_then(|w| w.window.frame_clock()) else { return };
        let what = what.to_string();
        let id: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        let id2 = id.clone();
        *id.borrow_mut() = Some(fc.connect_after_paint(move |fc| {
            trace(&what);
            if let Some(h) = id2.borrow_mut().take() {
                fc.disconnect(h);
            }
        }));
    }

    /// Whether the compositor actually blurs behind the overlay right now.
    pub fn blur_active(&self) -> bool {
        if !self.st.borrow().settings.blur {
            return false;
        }
        let info = if self.is_demo() {
            setup::blur_info(&setup::load_model(None, &Store::default()), self.niri_version.as_deref())
        } else {
            setup::blur_info(&self.st.borrow().model, self.niri_version.as_deref())
        };
        info.state == setup::BlurState::Ready
    }

    pub fn apply_css(&self) {
        let blurred = self.blur_active();
        let st = self.st.borrow();
        let (mw, mh) = self.monitor_size();
        let unit = keyboard::unit_for(&st.settings, mw, mh);
        self.css.load_from_string(&style::css(unit, st.settings.dim, blurred, st.settings.reduced_motion));
    }

    // ------------------------------------------------------------- rendering

    pub fn render_all(self: &Rc<Self>) {
        self.render_keyboard();
        self.drop_kept_panels();
        self.render_panel();
        if self.is_shown() {
            // Warm the replacements in the background, a panel per frame.
            let weak = Rc::downgrade(self);
            if let Some(w) = self.win.borrow().as_ref() {
                w.window.add_tick_callback(move |_, _| match weak.upgrade() {
                    Some(ui) if ui.warm_one() => glib::ControlFlow::Continue,
                    _ => glib::ControlFlow::Break,
                });
            }
        }
    }

    pub fn render_keyboard(self: &Rc<Self>) {
        let Some(win) = self.win.borrow().clone() else { return };
        self.cancel_ripple("keyboard rebuilt");
        while let Some(c) = win.keyboard_slot.first_child() {
            win.keyboard_slot.remove(&c);
        }
        keyboard::build(self, &win);
    }

    /// Forget the kept settings and browser panels (model or size changed).
    pub fn drop_kept_panels(&self) {
        let Some(win) = self.win.borrow().clone() else { return };
        self.drop_settings(&win);
        if let Some(b) = win.browser.borrow_mut().take() {
            forget_panel(&win, b.root.upcast_ref());
        }
    }

    fn drop_settings(&self, win: &Win) {
        if let Some(p) = win.settings.borrow_mut().take() {
            forget_panel(win, &p);
        }
    }

    fn ensure_settings(self: &Rc<Self>, win: &Win) {
        if win.settings.borrow().is_none() {
            let p: gtk::Widget = settings::build(self).upcast();
            keep_panel(win, &p);
            *win.settings.borrow_mut() = Some(p);
        }
    }

    fn ensure_browser(self: &Rc<Self>, win: &Win) {
        let fresh = win.browser.borrow().as_ref().is_some_and(|b| b.key == browser::key(self));
        if !fresh {
            if let Some(old) = win.browser.borrow_mut().take() {
                forget_panel(win, old.root.upcast_ref());
            }
            let b = browser::build(self);
            keep_panel(win, b.root.upcast_ref());
            *win.browser.borrow_mut() = Some(b);
        }
    }

    /// Lay out one kept panel that has not been laid out yet, keeping it
    /// invisible. Returns whether there may be more to do.
    fn warm_one(self: &Rc<Self>) -> bool {
        let Some(win) = self.win.borrow().clone() else { return false };
        if !win.window.is_visible() {
            return false;
        }
        self.ensure_settings(&win);
        self.ensure_browser(&win);
        let cold = [win.settings.borrow().clone(), win.browser.borrow().as_ref().map(|b| b.root.clone().upcast())]
            .into_iter()
            .flatten()
            .find(|p| !p.is_visible());
        match cold {
            Some(p) => {
                trace("warming a panel");
                p.set_visible(true);
                // The browser's list fills itself over the next frames.
                if let Some(b) = win.browser.borrow().as_ref().filter(|b| b.root.upcast_ref::<gtk::Widget>() == &p) {
                    browser::attach(b);
                }
                true
            }
            None => false,
        }
    }

    /// Show the panel for the current view, rebuilding its contents.
    pub fn render_panel(self: &Rc<Self>) {
        self.show_panel(true);
    }

    /// Show the panel for the current view. The settings and browser panels
    /// are kept: they stay laid out in the overlay, parked (transparent, not
    /// clickable or focusable), and opening one only refreshes what depends
    /// on the selection (browser) or nothing at all (settings, unless
    /// `rebuild`). Forms and confirmations are built fresh and removed after.
    fn show_panel(self: &Rc<Self>, rebuild: bool) {
        let Some(win) = self.win.borrow().clone() else { return };
        self.finish_ripple("panel opened");
        let view = self.st.borrow().view;
        let panel: Option<gtk::Widget> = match view {
            View::Keyboard => None,
            View::Browser => {
                self.ensure_browser(&win);
                let b = win.browser.borrow();
                let b = b.as_ref().expect("ensured");
                browser::refresh(self, b);
                Some(b.root.clone().upcast())
            }
            View::Settings => {
                if rebuild {
                    self.drop_settings(&win);
                }
                self.ensure_settings(&win);
                win.settings.borrow().clone()
            }
            View::Form => Some(form::build(self).upcast()),
            View::Confirm => Some(widgets::confirm_panel(self).upcast()),
        };
        let old = win.panel.borrow_mut().take();
        if let Some(old) = old
            && panel.as_ref() != Some(&old)
        {
            if is_kept(&win, &old) {
                park(&old, true);
            } else {
                win.overlay.remove_overlay(&old);
            }
        }
        win.scrim.set_visible(panel.is_some());
        for w in [win.keyboard_slot.upcast_ref::<gtk::Widget>(), win.hint.upcast_ref()] {
            w.set_opacity(if panel.is_some() { 0.35 } else { 1.0 });
            w.set_can_target(panel.is_none() && w != win.hint.upcast_ref::<gtk::Widget>());
        }
        win.keyboard_slot.set_can_focus(panel.is_none());
        win.tip.set_visible(false);
        if let Some(p) = &panel {
            if p.parent().is_none() {
                win.overlay.add_overlay(p);
            }
            p.set_visible(true);
            park(p, false);
            *win.panel.borrow_mut() = Some(p.clone());
            // Move focus into the panel; confirmation defaults to Cancel.
            widgets::focus_default(p);
        } else {
            self.focus_keyboard();
        }
    }

    /// Focus the last selected key, or else the keyboard itself (never a
    /// particular key), so focus never stays behind in a parked panel.
    fn focus_keyboard(&self) {
        let Some(win) = self.win.borrow().clone() else { return };
        let last = self.st.borrow().selection.last().and_then(|k| win.keys.borrow().get(k).cloned());
        match last {
            Some(k) => {
                k.grab_focus();
            }
            None => {
                win.keyboard_slot.grab_focus();
            }
        }
    }

    /// Switch views. Kept panels are shown as they were (settings) or only
    /// refreshed for the new combination (browser), never rebuilt here.
    pub fn set_view(self: &Rc<Self>, v: View) {
        self.st.borrow_mut().view = v;
        self.show_panel(false);
        trace(&format!("view {v:?}: state and widgets ready"));
        self.trace_frame(&format!("view {v:?}: first frame"));
    }

    pub fn toast(self: &Rc<Self>, text: &str, undo: Option<String>) {
        let Some(win) = self.win.borrow().clone() else { return };
        while let Some(c) = win.toast.first_child() {
            win.toast.remove(&c);
        }
        let l = gtk::Label::new(Some(text));
        l.set_wrap(true);
        l.set_max_width_chars(70);
        l.set_width_chars(text.lines().map(|x| x.chars().count()).max().unwrap_or(0).min(60) as i32);
        win.toast.append(&l);
        if let Some(txid) = undo {
            let b = gtk::Button::with_label("Undo");
            b.add_css_class("flat");
            let weak = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.undo(&txid);
                }
            });
            win.toast.append(&b);
            win.toast.add_css_class("km-panel");
        } else {
            win.toast.remove_css_class("km-panel");
        }
        win.toast.set_visible(true);
        if let Some(t) = self.toast_timer.borrow_mut().take() {
            t.remove();
        }
        let weak = Rc::downgrade(self);
        let secs = if win.toast.has_css_class("km-panel") { 20 } else { 8 };
        let id = glib::timeout_add_local_once(Duration::from_secs(secs), move || {
            if let Some(ui) = weak.upgrade() {
                ui.toast_timer.borrow_mut().take();
                if let Some(w) = ui.win.borrow().as_ref() {
                    w.toast.set_visible(false);
                }
            }
        });
        *self.toast_timer.borrow_mut() = Some(id);
    }

    // ------------------------------------------------------------ navigation

    /// Navigation acts on the key press; its auto-repeats are consumed.
    fn on_physical_key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType, time: u32, modifier: bool) -> glib::Propagation {
        let view = self.st.borrow().view;
        let nav = nav_for(view, key, state);
        // A key press completes the opening ripple and then acts as usual in
        // the same event; Esc hides at once instead (hiding cancels it). GTK
        // sends no press for keys already held when the overlay takes focus,
        // so the opening shortcut never skips it; bare modifiers are ignored.
        if nav != Nav::Hide && !modifier {
            self.finish_ripple("key press");
        }
        if nav == Nav::Pass {
            // Holding Space on a focused key would toggle it on every
            // auto-repeat; only the first press counts. In a confirmation,
            // auto-repeats of the Enter or Space press that opened it must
            // not activate a button in it.
            let space = matches!(key, gdk::Key::space | gdk::Key::KP_Space);
            let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter);
            if (space || enter)
                && self.nav_repeat.borrow_mut().is_repeat(key, time)
                && (view == View::Confirm || (space && view == View::Keyboard))
            {
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if self.nav_repeat.borrow_mut().is_repeat(key, time) {
            return glib::Propagation::Stop;
        }
        trace(&format!("key: {nav:?} in {view:?}{}", input_age(time)));
        match nav {
            Nav::OpenBrowser => self.open_browser(),
            Nav::OpenSettings => self.open_settings(),
            Nav::Hide => self.hide(),
            Nav::Back => self.back(),
            Nav::NewAction => {
                if self.st.borrow().model.read_only.is_none() {
                    form::open(self);
                }
            }
            Nav::Pass => {}
        }
        glib::Propagation::Stop
    }

    fn on_backdrop_click(self: &Rc<Self>) {
        if self.st.borrow().view == View::Keyboard {
            self.hide();
        } else {
            self.back();
        }
    }

    /// One level back, protecting unsaved form edits.
    pub fn back(self: &Rc<Self>) {
        let view = self.st.borrow().view;
        match view {
            View::Keyboard => {}
            View::Browser | View::Settings => {
                if view == View::Browser {
                    // Keep the combination selected on the keyboard.
                    self.st.borrow_mut().draft = None;
                }
                if self.st.borrow_mut().settings_message.take().is_some()
                    && let Some(w) = self.win.borrow().clone()
                {
                    // The kept panel shows the message; rebuild it next time.
                    self.drop_settings(&w);
                }
                self.set_view(View::Keyboard);
            }
            View::Form => form::request_close(self),
            View::Confirm => {
                let ret = {
                    let mut st = self.st.borrow_mut();
                    let d = st.draft.as_mut();
                    d.map(|d| {
                        d.pending = None;
                        d.return_to
                    })
                };
                self.set_view(ret.unwrap_or(View::Keyboard));
            }
        }
    }

    pub fn open_settings(self: &Rc<Self>) {
        self.set_view(View::Settings);
    }

    // ------------------------------------------------------------- selection

    /// The user clicked (or activated) a key: the one place selection changes
    /// by hand, so the one place that ticks. Programmatic clears are silent.
    pub fn toggle_key(self: &Rc<Self>, evdev: u32) {
        let selected = {
            let mut st = self.st.borrow_mut();
            let selected = match st.selection.iter().position(|k| *k == evdev) {
                Some(i) => {
                    st.selection.remove(i);
                    false
                }
                None => {
                    st.selection.push(evdev);
                    true
                }
            };
            st.hint = None;
            st.invalid_keys.clear();
            selected
        };
        self.effect(if selected { Cue::Select } else { Cue::Deselect });
        keyboard::refresh_states(self);
    }

    /// An interaction sound, when sounds are on (never blocks: the sound
    /// thread mixes it).
    fn effect(&self, cue: Cue) {
        if self.st.borrow().settings.sounds {
            trace(&format!("sound: {cue:?}"));
            self.sound.play(cue);
        }
    }

    /// The combination formed by the selected keys, or why there is none.
    pub fn selection_combo(&self) -> Result<Combo, (String, Vec<u32>)> {
        let st = self.st.borrow();
        let mut mods = Mods::empty();
        let mut ordinary = Vec::new();
        for k in &st.selection {
            let info = st.resolver.key(*k);
            match (info.modifier, info.sym) {
                (Some(m), _) => mods = mods.with(m),
                (None, Some(sym)) => ordinary.push((*k, sym, info.label)),
                (None, None) => {}
            }
        }
        match ordinary.as_slice() {
            [] if st.selection.is_empty() => Err((String::new(), vec![])),
            [] => Err(("Add one non-modifier key to complete the shortcut.".into(), st.selection.clone())),
            [(_, sym, _)] => Ok(Combo { mods, trigger: Trigger::Keysym(*sym) }),
            many => Err((
                format!(
                    "Niri shortcuts use exactly one non-modifier key. Deselect all but one of: {}.",
                    many.iter().map(|o| o.2.as_str()).collect::<Vec<_>>().join(", ")
                ),
                many.iter().map(|o| o.0).collect(),
            )),
        }
    }

    pub fn open_browser(self: &Rc<Self>) {
        match self.selection_combo() {
            Ok(combo) => {
                let assigned = self.st.borrow().model.effective_for(&combo).is_some();
                let chosen = {
                    let st = self.st.borrow();
                    st.model.effective_for(&combo).and_then(|b| st.model.action_of(b)).map(|a| a.canonical.clone())
                };
                self.st.borrow_mut().draft = Some(Draft {
                    combo,
                    chosen,
                    tab: if assigned { Tab::This } else { Tab::All },
                    search: String::new(),
                    error: None,
                    info: None,
                    form: None,
                    pending: None,
                    return_to: View::Browser,
                });
                self.set_view(View::Browser);
            }
            Err((msg, keys)) => {
                if msg.is_empty() {
                    return;
                }
                {
                    let mut st = self.st.borrow_mut();
                    st.hint = Some((msg, true));
                    st.invalid_keys = keys;
                }
                keyboard::refresh_states(self);
            }
        }
    }

    // ----------------------------------------------------------- persistence

    pub fn save_settings(&self) {
        if self.is_demo() {
            return;
        }
        if let Err(e) = store::save_settings(&self.st.borrow().settings) {
            eprintln!("keymap: cannot save settings: {e}");
        }
    }

    pub fn save_store(&self) {
        if self.is_demo() {
            return;
        }
        if let Err(e) = store::save_store(&self.st.borrow().store) {
            eprintln!("keymap: cannot save custom actions: {e}");
        }
    }

    /// Reload the configuration model from disk (keeps any draft).
    pub fn reload(self: &Rc<Self>) {
        let explicit = self.explicit_path();
        {
            let mut st = self.st.borrow_mut();
            let model = setup::load_model(explicit.as_deref(), &st.store);
            st.resolver = Resolver::new(&model.config.xkb);
            st.model = model;
            st.generation += 1;
            trace(&format!("configuration reloaded: {} bindings", st.model.bindings.len()));
        }
        self.watch_files();
        self.render_all();
    }

    /// Rebuild only the derived model after store (label/custom) changes.
    pub fn rebuild_model(self: &Rc<Self>) {
        let mut st = self.st.borrow_mut();
        let cfg = st.model.config.clone();
        st.model = Model::build(cfg, &st.store);
        st.generation += 1;
    }

    fn watch_files(self: &Rc<Self>) {
        let mut paths: Vec<std::path::PathBuf> = {
            let st = self.st.borrow();
            let mut v: Vec<_> = st.model.config.files.iter().map(|f| f.path.clone()).collect();
            v.extend(st.model.config.includes.iter().map(|i| i.resolved.clone()));
            v.push(st.model.config.location.path.clone());
            v
        };
        paths.sort();
        paths.dedup();
        let mut monitors = Vec::new();
        for p in paths {
            let file = gio::File::for_path(&p);
            match file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
                Ok(m) => {
                    let weak = Rc::downgrade(self);
                    m.connect_changed(move |_, _, _, _| {
                        if let Some(ui) = weak.upgrade() {
                            ui.schedule_reload();
                        }
                    });
                    monitors.push(m);
                }
                Err(e) => eprintln!("keymap: cannot watch {}: {e}", p.display()),
            }
        }
        *self.watchers.borrow_mut() = monitors;
    }

    fn schedule_reload(self: &Rc<Self>) {
        if let Some(id) = self.reload_pending.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(ui) = weak.upgrade() {
                ui.reload_pending.borrow_mut().take();
                ui.on_files_changed();
            }
        });
        *self.reload_pending.borrow_mut() = Some(id);
    }

    fn on_files_changed(self: &Rc<Self>) {
        // Ignore our own writes: if every file still has the bytes we loaded, nothing changed.
        let same = {
            let st = self.st.borrow();
            let files_same =
                st.model.config.files.iter().all(|f| std::fs::read(&f.path).map(|b| hash_bytes(&b) == f.hash).unwrap_or(false));
            let missing_same = st.model.config.includes.iter().filter(|i| i.file.is_none()).all(|i| !i.resolved.exists());
            files_same && missing_same
        };
        if same {
            return;
        }
        let pending = self.st.borrow().draft.is_some();
        if pending {
            self.st.borrow_mut().changed_on_disk = true;
        }
        self.reload();
    }

    /// Commit a plan; on success show a toast with Undo and return to the keyboard.
    pub fn commit(self: &Rc<Self>, plan: &Plan) -> Result<Receipt, String> {
        let model = self.st.borrow().model.clone();
        match self.tx.commit(&model, plan, self.validator.as_ref()) {
            Ok(r) => {
                self.st.borrow_mut().changed_on_disk = false;
                self.reload();
                let mut msg = if self.is_demo() {
                    format!("Demo copy saved (your Niri config was not touched): {}", r.description)
                } else {
                    format!("Saved: {}", r.description)
                };
                for n in &plan.notes {
                    msg.push_str(&format!("\n{n}"));
                }
                self.toast(&msg, Some(r.txid.clone()));
                Ok(r)
            }
            Err(CommitError::Conflict(paths)) => {
                self.reload();
                self.st.borrow_mut().changed_on_disk = true;
                Err(format!(
                    "{} changed on disk after it was loaded. Keymap reloaded it and kept your selection; review and save again.",
                    paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
                ))
            }
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn undo(self: &Rc<Self>, txid: &str) {
        let model = self.st.borrow().model.clone();
        match self.tx.undo(&model, txid, self.validator.as_ref()) {
            Ok(_) => {
                self.reload();
                self.toast("Undone. The previous configuration was restored.", None);
            }
            Err(CommitError::Conflict(p)) => self.toast(
                &format!(
                    "Cannot undo: {} changed after this edit, and undo never overwrites newer changes.",
                    p.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
                ),
                None,
            ),
            Err(e) => self.toast(&format!("Cannot undo: {e}"), None),
        }
    }

    /// Save the chosen action for the draft's combination.
    pub fn save_draft(self: &Rc<Self>, confirmed: bool) {
        let (combo, chosen) = match self.st.borrow().draft.as_ref() {
            Some(d) => (d.combo.clone(), d.chosen.clone()),
            None => return,
        };
        let Some(chosen) = chosen else { return };
        self.assign(&combo, &chosen, &Others::Ask, confirmed);
    }

    pub fn assign(self: &Rc<Self>, combo: &Combo, canonical: &str, others: &Others, confirmed: bool) {
        let result = {
            let st = self.st.borrow();
            match st.model.find_action(canonical) {
                None => Err("That action no longer exists in the configuration.".to_string()),
                Some(a) => {
                    let target = Target::of_action(&st.model, a.id);
                    plan::assign(&st.model, &st.store, combo, &target, others, confirmed).map_err(|e| e.0)
                }
            }
        };
        let confirm = |pending: Pending| {
            let ret = self.st.borrow().view;
            if let Some(d) = self.st.borrow_mut().draft.as_mut() {
                d.pending = Some(pending);
                d.return_to = if ret == View::Confirm { d.return_to } else { ret };
            }
            self.set_view(View::Confirm);
        };
        match result {
            Ok(AssignResult::NoOp(msg)) => {
                if self.st.borrow().view == View::Confirm {
                    self.back();
                }
                self.set_draft_info(msg)
            }
            Ok(AssignResult::HasShortcuts(info)) => {
                confirm(Pending::Existing { info, target: canonical.to_string(), pick: None });
            }
            Ok(AssignResult::NeedsConfirm(rep)) => {
                confirm(Pending::Replace { rep, target: canonical.to_string(), others: others.clone() });
            }
            Ok(AssignResult::Ready(plan)) => match self.commit(&plan) {
                Ok(_) => {
                    self.effect(Cue::Saved);
                    self.finish_edit();
                }
                Err(e) => self.set_draft_error(e),
            },
            Err(e) => self.set_draft_error(e),
        }
    }

    pub fn remove_binding(self: &Rc<Self>, binding: usize, everywhere: bool) {
        let result = {
            let st = self.st.borrow();
            if everywhere {
                let combo = st.model.bindings[binding].combo.clone();
                combo.ok_or_else(|| "invalid key".to_string()).and_then(|c| plan::unbind_all(&st.model, &st.store, &c).map_err(|e| e.0))
            } else {
                plan::remove(&st.model, &st.store, binding).map(|r| r.plan).map_err(|e| e.0)
            }
        };
        match result.and_then(|p| self.commit(&p)) {
            Ok(_) => {
                self.effect(Cue::Saved);
                self.finish_edit();
            }
            Err(e) => self.set_draft_error(e),
        }
    }

    fn finish_edit(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.draft = None;
            st.selection.clear();
        }
        self.st.borrow_mut().view = View::Keyboard;
        self.render_all();
    }

    pub fn set_draft_error(self: &Rc<Self>, e: String) {
        let back_to = {
            let mut st = self.st.borrow_mut();
            let ret = st.draft.as_ref().map(|d| d.return_to).unwrap_or(View::Browser);
            if let Some(d) = st.draft.as_mut() {
                d.error = Some(e);
                d.info = None;
                d.pending = None;
            }
            ret
        };
        let v = self.st.borrow().view;
        self.set_view(if v == View::Confirm { back_to } else { v });
    }

    fn set_draft_info(self: &Rc<Self>, msg: String) {
        if let Some(d) = self.st.borrow_mut().draft.as_mut() {
            d.info = Some(msg);
            d.error = None;
        }
        self.render_panel();
    }
}

/// What a physical key press does in a view. Everything not listed passes
/// through to the focused widget (Tab/Shift+Tab focus navigation inside
/// panels, Enter in fields and buttons, typing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nav {
    OpenBrowser,
    OpenSettings,
    Hide,
    Back,
    NewAction,
    Pass,
}

/// Accept commands on a background thread and hand them to `handle` on the
/// GTK main thread. The thread blocks in `accept`; there is no polling.
fn serve(listener: UnixListener, handle: fn(Command)) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            let mut reader = BufReader::new(&stream);
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let reply = match Command::parse(&line) {
                Some(cmd) => {
                    glib::MainContext::default().invoke(move || handle(cmd));
                    "ok\n"
                }
                None => "error unknown command\n",
            };
            let _ = (&stream).write_all(reply.as_bytes());
        }
    });
}

/// Where the hint sits: centred above the keyboard's own bounds, as wide as
/// the keyboard (at least a readable width), growing upward as it wraps, and
/// clipped rather than ever overlapping the keyboard. The keyboard itself is
/// centred by the overlay and never measured against the hint.
fn place_hint(overlay: &gtk::Overlay, keyboard: &gtk::Box, hint: &gtk::Widget) -> gdk::Rectangle {
    const GAP: i32 = 14;
    let (ow, oh) = (overlay.width(), overlay.height());
    let (_, kw, _, _) = keyboard.measure(gtk::Orientation::Horizontal, -1);
    let (_, kh, _, _) = keyboard.measure(gtk::Orientation::Vertical, kw);
    let top = (oh - kh) / 2;
    let w = kw.max(420).min(ow - 32).max(0);
    let (_, nh, _, _) = hint.measure(gtk::Orientation::Vertical, w);
    let h = nh.min(top - GAP - 8).max(0);
    gdk::Rectangle::new((ow - w) / 2, top - GAP - h, w, h)
}

/// Parked panels stay laid out but are invisible and inert: no drawing (GTK
/// skips fully transparent widgets), no pointer hits, no focus, and hidden
/// from assistive technologies.
fn park(w: &gtk::Widget, parked: bool) {
    w.set_opacity(if parked { 0.0 } else { 1.0 });
    w.set_can_target(!parked);
    w.set_can_focus(!parked);
    w.update_state(&[gtk::accessible::State::Hidden(parked)]);
}

fn keep_panel(win: &Win, p: &gtk::Widget) {
    // Not laid out until `warm_one` (or opening it) makes it visible.
    p.set_visible(false);
    park(p, true);
    win.overlay.add_overlay(p);
}

fn forget_panel(win: &Win, p: &gtk::Widget) {
    if win.panel.borrow().as_ref() == Some(p) {
        win.panel.borrow_mut().take();
    }
    win.overlay.remove_overlay(p);
}

fn is_kept(win: &Win, p: &gtk::Widget) -> bool {
    win.settings.borrow().as_ref() == Some(p) || win.browser.borrow().as_ref().is_some_and(|b| b.root.upcast_ref::<gtk::Widget>() == p)
}

/// Where a reopened overlay starts. Hiding never saves or discards anything
/// by itself: a form with typed details comes back (with any confirmation
/// that was open cancelled, as Esc would), so the usual discard rules apply;
/// everything else starts on the keyboard.
fn reopen_view(view: View, form_dirty: bool) -> View {
    match view {
        View::Form | View::Confirm if form_dirty => View::Form,
        _ => View::Keyboard,
    }
}

/// How long ago (compositor clock) an input event happened, for traces.
fn input_age(time: u32) -> String {
    if time == 0 || std::env::var_os("KEYMAP_DEBUG").is_none() {
        return String::new();
    }
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid pointer to a timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    let now = (ts.tv_sec * 1000 + ts.tv_nsec / 1_000_000) as u32;
    format!(" (event {} ms ago)", now.wrapping_sub(time))
}

/// GDK synthesises key repeats on the client and gives them the timestamp of
/// the original press (Niri's wl_seat v9 sends no compositor repeats), so a
/// press of the same key with the same timestamp is a repeat. This needs no
/// release event, which GTK may not deliver once focus has moved to a new
/// panel. Virtual keyboards send timestamp 0; for them the release decides.
#[derive(Debug, Default)]
struct RepeatGuard {
    last: Option<(gdk::Key, u32)>,
    held: bool,
}

impl RepeatGuard {
    fn is_repeat(&mut self, key: gdk::Key, time: u32) -> bool {
        let key = key.to_lower();
        let repeat = match self.last {
            Some((k, t)) if k == key => {
                if time != 0 {
                    time == t
                } else {
                    self.held
                }
            }
            _ => false,
        };
        if !repeat {
            self.last = Some((key, time));
            self.held = true;
        }
        repeat
    }

    fn released(&mut self, key: gdk::Key) {
        if self.last.is_some_and(|(k, _)| k == key.to_lower()) {
            self.held = false;
        }
    }
}

fn nav_for(view: View, key: gdk::Key, state: gdk::ModifierType) -> Nav {
    let mods = state
        & (gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK
            | gdk::ModifierType::META_MASK
            | gdk::ModifierType::HYPER_MASK);
    let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter);
    match view {
        View::Keyboard if enter => Nav::OpenBrowser,
        // Bare Tab only; Shift+Tab arrives as ISO_Left_Tab and keeps moving focus.
        View::Keyboard if key == gdk::Key::Tab && mods.is_empty() => Nav::OpenSettings,
        View::Keyboard if key == gdk::Key::Escape => Nav::Hide,
        View::Browser if mods == gdk::ModifierType::CONTROL_MASK && key.to_lower() == gdk::Key::n => Nav::NewAction,
        _ if key == gdk::Key::Escape => Nav::Back,
        _ => Nav::Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdk::Key;
    use gdk::ModifierType as M;

    #[test]
    fn keyboard_view_navigation() {
        assert_eq!(nav_for(View::Keyboard, Key::Tab, M::empty()), Nav::OpenSettings);
        assert_eq!(nav_for(View::Keyboard, Key::Escape, M::empty()), Nav::Hide);
        assert_eq!(nav_for(View::Keyboard, Key::Return, M::empty()), Nav::OpenBrowser);
        assert_eq!(nav_for(View::Keyboard, Key::KP_Enter, M::empty()), Nav::OpenBrowser);
        // Lock keys do not make Tab "modified".
        assert_eq!(nav_for(View::Keyboard, Key::Tab, M::LOCK_MASK), Nav::OpenSettings);
        assert_eq!(nav_for(View::Keyboard, Key::ISO_Left_Tab, M::SHIFT_MASK), Nav::Pass);
        assert_eq!(nav_for(View::Keyboard, Key::Tab, M::CONTROL_MASK), Nav::Pass);
        assert_eq!(nav_for(View::Keyboard, Key::Left, M::empty()), Nav::Pass);
    }

    #[test]
    fn reopening_never_resumes_a_confirmation_or_drops_typed_details() {
        for v in [View::Keyboard, View::Browser, View::Settings, View::Form, View::Confirm] {
            assert_eq!(reopen_view(v, false), View::Keyboard, "{v:?} without typed details");
        }
        // A dirty form (also behind a replace/discard confirmation) comes back as the form.
        assert_eq!(reopen_view(View::Form, true), View::Form);
        assert_eq!(reopen_view(View::Confirm, true), View::Form);
        assert_eq!(reopen_view(View::Browser, true), View::Keyboard, "form state only counts in the form");
    }

    #[test]
    fn repeats_share_the_press_timestamp() {
        let mut g = RepeatGuard::default();
        assert!(!g.is_repeat(Key::Escape, 1000));
        assert!(g.is_repeat(Key::Escape, 1000), "GDK repeat");
        assert!(g.is_repeat(Key::Escape, 1000));
        // A new press has a new timestamp, even if its release was never seen.
        assert!(!g.is_repeat(Key::Escape, 1180));
        g.released(Key::Escape);
        assert!(!g.is_repeat(Key::Escape, 1300));
        // Another key is never a repeat of this one.
        assert!(!g.is_repeat(Key::Tab, 1300));
    }

    #[test]
    fn virtual_keyboards_without_timestamps_use_the_release() {
        let mut g = RepeatGuard::default();
        assert!(!g.is_repeat(Key::Escape, 0));
        assert!(g.is_repeat(Key::Escape, 0));
        g.released(Key::Escape);
        assert!(!g.is_repeat(Key::Escape, 0));
    }

    #[test]
    fn panels_keep_tab_and_enter_and_go_back_one_level_on_esc() {
        for v in [View::Settings, View::Browser, View::Form, View::Confirm] {
            assert_eq!(nav_for(v, Key::Tab, M::empty()), Nav::Pass, "{v:?}");
            assert_eq!(nav_for(v, Key::ISO_Left_Tab, M::SHIFT_MASK), Nav::Pass, "{v:?}");
            assert_eq!(nav_for(v, Key::Return, M::empty()), Nav::Pass, "{v:?}");
            assert_eq!(nav_for(v, Key::Escape, M::empty()), Nav::Back, "{v:?}");
        }
        assert_eq!(nav_for(View::Browser, Key::n, M::CONTROL_MASK), Nav::NewAction);
        assert_eq!(nav_for(View::Form, Key::n, M::CONTROL_MASK), Nav::Pass);
    }
}
