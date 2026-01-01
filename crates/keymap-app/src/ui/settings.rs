//! Settings: keyboard shape and size, background, configuration source,
//! history (undo) and recovery of interrupted saves.

use super::browser::short_path;
use super::widgets::{self, banner, button, label};
use super::{Ui, View};
use crate::setup::{self, BlurState};
use gtk::prelude::*;
use keymap_core::config::LocationSource;
use keymap_core::store::FormFactor;
use keymap_core::txn::TxState;
use std::rc::Rc;

fn switch_row(text: &str, note: Option<&str>, on: bool, f: impl Fn(bool) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
    v.set_hexpand(true);
    v.append(&label(text, &[]));
    if let Some(n) = note {
        v.append(&label(n, &["km-small", "km-muted"]));
    }
    row.append(&v);
    let s = gtk::Switch::new();
    s.set_active(on);
    s.set_valign(gtk::Align::Center);
    s.update_property(&[gtk::accessible::Property::Label(text)]);
    s.connect_active_notify(move |s| f(s.is_active()));
    row.append(&s);
    row
}

fn slider(text: &str, min: f64, max: f64, value: f64, fmt: fn(f64) -> String, f: impl Fn(f64) + 'static) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let l = label(text, &[]);
    l.set_size_request(150, -1);
    row.append(&l);
    let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 0.01);
    s.set_value(value);
    s.set_hexpand(true);
    s.set_draw_value(false);
    s.update_property(&[gtk::accessible::Property::Label(text)]);
    let v = label(&fmt(value), &["km-muted", "km-small"]);
    v.set_size_request(48, -1);
    let v2 = v.clone();
    // Apply after the user pauses, so dragging stays smooth.
    let pending: Rc<std::cell::RefCell<Option<gtk::glib::SourceId>>> = Rc::default();
    let f = Rc::new(f);
    s.connect_value_changed(move |s| {
        let x = s.value();
        v2.set_text(&fmt(x));
        if let Some(id) = pending.borrow_mut().take() {
            id.remove();
        }
        let (f, p2) = (f.clone(), pending.clone());
        let id = gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(120), move || {
            p2.borrow_mut().take();
            f(x);
        });
        *pending.borrow_mut() = Some(id);
    });
    row.append(&s);
    row.append(&v);
    row
}

pub fn build(ui: &Rc<Ui>) -> gtk::Box {
    let p = widgets::panel(ui, 680.0, Some(0.8));
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.append(&label("Keymap settings", &["km-title"]));
    head.append(&widgets::spacer());
    let close = button("Close overlay", &[]);
    let weak = Rc::downgrade(ui);
    close.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.hide();
        }
    });
    close.set_tooltip_text(Some("Hide Keymap (Esc on the keyboard)"));
    let back = button("Back to keyboard", &[]);
    back.set_tooltip_text(Some("Esc"));
    back.set_widget_name(widgets::DEFAULT_FOCUS);
    let weak = Rc::downgrade(ui);
    back.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.back();
        }
    });
    head.append(&back);
    head.append(&close);
    p.append(&head);
    if let Some(m) = ui.st.borrow().settings_message.clone() {
        p.append(&banner(&m, m.starts_with("Cannot") || m.starts_with("Could not")));
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    p.append(&widgets::scroller(&body));
    let s = ui.st.borrow().settings.clone();

    // --- Keyboard
    body.append(&label("Keyboard", &["km-section"]));
    let ff = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let mut group: Option<gtk::ToggleButton> = None;
    for (f, text) in [(FormFactor::Sixty, "60%"), (FormFactor::SeventyFive, "75%"), (FormFactor::Full, "100%")] {
        let b = gtk::ToggleButton::with_label(text);
        b.add_css_class("km-tab");
        b.set_group(group.as_ref());
        b.set_active(s.form_factor == f);
        let weak = Rc::downgrade(ui);
        b.connect_toggled(move |b| {
            if let (true, Some(ui)) = (b.is_active(), weak.upgrade()) {
                ui.st.borrow_mut().settings.form_factor = f;
                ui.st.borrow_mut().selection.clear();
                ui.save_settings();
                ui.apply_css();
                ui.render_keyboard();
            }
        });
        ff.append(&b);
        group.get_or_insert(b);
    }
    body.append(&ff);
    let weak = Rc::downgrade(ui);
    body.append(&slider(
        "Size",
        0.35,
        0.95,
        s.scale,
        |x| format!("{:.0}%", x * 100.0),
        move |x| {
            if let Some(ui) = weak.upgrade() {
                ui.st.borrow_mut().settings.scale = x;
                ui.save_settings();
                ui.apply_css();
                ui.render_keyboard();
            }
        },
    ));
    let weak = Rc::downgrade(ui);
    body.append(&switch_row(
        "Show only assigned keys",
        Some("Fades keys that do nothing with the selected modifiers."),
        s.assigned_only,
        move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.st.borrow_mut().settings.assigned_only = on;
                ui.save_settings();
                super::keyboard::refresh_states(&ui);
            }
        },
    ));
    let weak = Rc::downgrade(ui);
    body.append(&switch_row("Reduce motion", None, s.reduced_motion, move |on| {
        if let Some(ui) = weak.upgrade() {
            ui.st.borrow_mut().settings.reduced_motion = on;
            ui.save_settings();
            ui.apply_css();
        }
    }));
    let weak = Rc::downgrade(ui);
    body.append(&switch_row("Animations", Some("Keys ripple in from the centre when the keyboard opens."), s.animations, move |on| {
        if let Some(ui) = weak.upgrade() {
            ui.st.borrow_mut().settings.animations = on;
            ui.save_settings();
        }
    }));

    // --- Sound
    body.append(&label("Sound", &["km-section"]));
    let weak = Rc::downgrade(ui);
    body.append(&switch_row(
        "Sound effects",
        Some("Quiet sounds when the keyboard opens, keys are selected and bindings are saved."),
        s.sounds,
        move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.st.borrow_mut().settings.sounds = on;
                ui.save_settings();
                if !on {
                    ui.sound.stop();
                }
            }
        },
    ));
    let weak = Rc::downgrade(ui);
    body.append(&slider(
        "Volume",
        0.0,
        1.0,
        s.volume,
        |x| format!("{:.0}%", x * 100.0),
        move |x| {
            if let Some(ui) = weak.upgrade() {
                ui.st.borrow_mut().settings.volume = x;
                ui.sound.set_volume(x);
                ui.save_settings();
            }
        },
    ));

    // --- Background
    body.append(&label("Background", &["km-section"]));
    // Blur depends on the running Niri's config, even in demo mode (read-only).
    let info = if ui.is_demo() {
        let live = setup::load_model(None, &keymap_core::store::Store::default());
        setup::blur_info(&live, ui.niri_version.as_deref())
    } else {
        setup::blur_info(&ui.st.borrow().model, ui.niri_version.as_deref())
    };
    let weak = Rc::downgrade(ui);
    let mut note = info.message.clone();
    if info.cannot_disable && !s.blur {
        note.push_str(" A broad layer rule in your config blurs all layers, so Keymap cannot turn blur off.");
    }
    body.append(&switch_row("Blur the windows behind", Some(&note), s.blur, move |on| {
        if let Some(ui) = weak.upgrade() {
            ui.st.borrow_mut().settings.blur = on;
            ui.save_settings();
            // The layer namespace is fixed per surface, so recreate it.
            ui.recreate_window();
            ui.st.borrow_mut().view = View::Settings;
            ui.render_panel();
        }
    }));
    if matches!(info.state, BlurState::NotConfigured | BlurState::XrayOnly) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let add = button("Add blur rule to Niri config", &[]);
        add.set_tooltip_text(Some("Appends one layer-rule that matches only Keymap's overlay"));
        add.set_sensitive(!ui.is_demo() && ui.st.borrow().model.read_only.is_none() && info.state == BlurState::NotConfigured);
        let weak = Rc::downgrade(ui);
        add.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                install_rule(&ui);
            }
        });
        let copy = button("Copy rule", &[]);
        copy.connect_clicked(|b| b.clipboard().set_text(setup::BLUR_RULE));
        row.append(&add);
        row.append(&copy);
        body.append(&row);
        let rule = label(setup::BLUR_RULE.trim_end(), &["km-mono", "km-muted"]);
        rule.set_selectable(true);
        body.append(&rule);
    }
    if s.blur && !matches!(info.state, BlurState::Unsupported(_) | BlurState::GlobalOff) {
        let current = setup::blur_strength(&ui.st.borrow().model.config);
        body.append(&label("Blur strength", &[]));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let editable = !ui.is_demo() && ui.st.borrow().model.read_only.is_none();
        let mut group: Option<gtk::ToggleButton> = None;
        for (name, passes, offset) in setup::BLUR_STRENGTHS {
            let b = gtk::ToggleButton::with_label(name);
            b.add_css_class("km-tab");
            b.set_tooltip_text(Some(&format!("passes {passes}, offset {offset}")));
            b.set_group(group.as_ref());
            b.set_active(current == Some(name));
            b.set_sensitive(editable);
            let weak = Rc::downgrade(ui);
            b.connect_toggled(move |b| {
                if let (true, Some(ui)) = (b.is_active(), weak.upgrade()) {
                    set_blur_strength(&ui, name);
                }
            });
            row.append(&b);
            group.get_or_insert(b);
        }
        body.append(&row);
        let mut note = String::from(
            "Sets passes and offset in Niri's blur section. That section is global, so this also changes blur for anything else that uses it. Each change is validated and can be undone in Recent changes.",
        );
        if current.is_none() {
            note.insert_str(0, "Your config uses custom blur values. ");
        }
        body.append(&label(&note, &["km-muted", "km-small"]));
    }
    let weak = Rc::downgrade(ui);
    body.append(&slider(
        "Dimming",
        0.0,
        0.7,
        s.dim,
        |x| format!("{:.0}%", x * 100.0),
        move |x| {
            if let Some(ui) = weak.upgrade() {
                ui.st.borrow_mut().settings.dim = x;
                ui.save_settings();
                ui.apply_css();
            }
        },
    ));

    // --- Opening
    body.append(&label("Opening", &["km-section"]));
    let weak = Rc::downgrade(ui);
    body.append(&switch_row(
        "Allow opening over fullscreen apps",
        Some("Let the shortcut open Keymap while a fullscreen app is focused."),
        s.open_over_fullscreen,
        move |on| {
            if let Some(ui) = weak.upgrade() {
                ui.st.borrow_mut().settings.open_over_fullscreen = on;
                ui.save_settings();
                if !on {
                    crate::toplevels::start();
                }
            }
        },
    ));

    // --- Source
    body.append(&label("Niri configuration", &["km-section"]));
    {
        let st = ui.st.borrow();
        let loc = &st.model.config.location;
        let how = match &loc.source {
            LocationSource::Explicit => "chosen explicitly".to_string(),
            LocationSource::RunningInstance { pid } => format!("used by the running Niri (pid {pid})"),
            LocationSource::RunningDefault { pid } => format!("Niri's default path; running Niri (pid {pid}) has no -c"),
            LocationSource::Guessed => "Niri's default path (no running Niri found)".to_string(),
        };
        if ui.is_demo() {
            body.append(&banner("Demo mode: a temporary copy of a synthetic config. Your Niri config is never read or written.", false));
        }
        body.append(&label(&format!("{} — {how}", short_path(&loc.path)), &[]));
        if let Some(n) = &loc.note {
            body.append(&label(n, &["km-small", "km-muted"]));
        }
        for f in &st.model.config.files {
            let mut t = format!("• {}", short_path(&f.path));
            if f.symlink {
                t.push_str(&format!(" → {}", short_path(&f.real_path)));
            }
            if !f.writable {
                t.push_str("  (read-only)");
            }
            body.append(&label(&t, &["km-small", "km-muted"]));
        }
        if let Some(r) = &st.model.read_only {
            body.append(&banner(&format!("Editing is disabled: {r}"), true));
        }
        for d in st.model.config.diagnostics.iter().take(8) {
            let t = format!("{}:{}:{}: {}", short_path(&d.path), d.line, d.column, d.message);
            body.append(&label(&t, &["km-small", if d.error { "km-error-text" } else { "km-muted" }]));
        }
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let reload = button("Reload", &[]);
    let weak = Rc::downgrade(ui);
    reload.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.reload();
            ui.st.borrow_mut().settings_message = Some("Reloaded from disk.".into());
            ui.render_panel();
        }
    });
    row.append(&reload);
    body.append(&row);

    // --- Recovery and history
    let pending = ui.tx.pending_recoveries();
    if !pending.is_empty() {
        body.append(&label("Interrupted saves", &["km-section"]));
        for j in pending {
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let l = label(
                &format!("{} — {}", j.description, j.files.iter().map(|f| short_path(&f.path)).collect::<Vec<_>>().join(", ")),
                &["km-small"],
            );
            l.set_hexpand(true);
            r.append(&l);
            let b = button("Restore previous files", &[]);
            let weak = Rc::downgrade(ui);
            let id = j.txid.clone();
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    let msg = match ui.tx.recover(&id) {
                        Ok(lines) => lines.join(" "),
                        Err(e) => format!("Could not recover: {e}"),
                    };
                    ui.reload();
                    ui.st.borrow_mut().settings_message = Some(msg);
                    ui.render_panel();
                }
            });
            r.append(&b);
            body.append(&r);
        }
    }
    body.append(&label("Recent changes", &["km-section"]));
    let history = ui.tx.history();
    if history.is_empty() {
        body.append(&label("No changes made by Keymap yet.", &["km-small", "km-muted"]));
    }
    for j in history.iter().take(6) {
        let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let state = match j.state {
            TxState::Committed => "",
            TxState::Undone => " (undone)",
            TxState::RolledBack => " (rolled back)",
            TxState::Prepared => " (interrupted)",
        };
        let l = label(&format!("{}{state}", j.description), &["km-small"]);
        l.set_hexpand(true);
        r.append(&l);
        if j.state == TxState::Committed && j.undoes.is_none() {
            let b = button("Undo", &["flat"]);
            let weak = Rc::downgrade(ui);
            let id = j.txid.clone();
            b.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.undo(&id);
                    ui.render_panel();
                }
            });
            r.append(&b);
        }
        body.append(&r);
    }
    body.append(&label(&format!("Backups of every changed file: {}", short_path(&ui.tx.dir)), &["km-small", "km-muted"]));
    p
}

fn set_blur_strength(ui: &Rc<Ui>, name: &str) {
    let plan = {
        let st = ui.st.borrow();
        setup::blur_strength_plan(&st.model, &st.store, name)
    };
    let msg = match plan.and_then(|p| ui.commit(&p)) {
        Ok(_) => format!("Blur strength set to {name}. Niri applies it immediately."),
        Err(e) => format!("Cannot change the blur strength: {e}"),
    };
    ui.st.borrow_mut().settings_message = Some(msg);
    ui.st.borrow_mut().view = View::Settings;
    ui.render_panel();
}

fn install_rule(ui: &Rc<Ui>) {
    let plan = {
        let st = ui.st.borrow();
        setup::blur_rule_plan(&st.model, &st.store)
    };
    let msg = match plan.and_then(|p| ui.commit(&p)) {
        Ok(_) => "Added the Keymap layer rule. Niri applies it automatically.".to_string(),
        Err(e) => format!("Cannot add the rule: {e}"),
    };
    ui.st.borrow_mut().settings_message = Some(msg);
    ui.recreate_window();
    ui.st.borrow_mut().view = View::Settings;
    ui.render_panel();
}
