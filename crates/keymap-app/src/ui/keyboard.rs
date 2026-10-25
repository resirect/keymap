//! The glass keyboard: geometry, legends, assignment state and hover details.

use super::{Ui, Win};
use gtk::prelude::*;
use keymap_core::geometry;
use keymap_core::keys::{Combo, Mods, Trigger};
use keymap_core::store::{FormFactor, Settings};
use std::rc::Rc;

/// Size of one keyboard unit in pixels for the given output size. The 100%
/// layout keeps the 75% layout's key size and is simply wider (the Size
/// setting is the 75% keyboard's share of the screen); it shrinks only when
/// it would not fit the output.
pub fn unit_for(settings: &Settings, mw: f64, mh: f64) -> f64 {
    let fit = |lay: &geometry::Layout, share: f64| (mw * share / lay.width).min(mh * 0.62 / lay.height);
    let unit = match settings.form_factor {
        FormFactor::Full => {
            fit(&geometry::layout(FormFactor::SeventyFive), settings.scale).min(fit(&geometry::layout(FormFactor::Full), 0.96))
        }
        ff => fit(&geometry::layout(ff), settings.scale),
    };
    unit.max(18.0)
}

/// Modifiers among the selected keys.
fn selected_mods(ui: &Ui) -> Mods {
    let st = ui.st.borrow();
    st.selection.iter().filter_map(|k| st.resolver.key(*k).modifier).fold(Mods::empty(), |m, x| m.with(x))
}

fn combo_for(ui: &Ui, evdev: u32, mods: Mods) -> Option<Combo> {
    let st = ui.st.borrow();
    let info = st.resolver.key(evdev);
    info.sym.filter(|_| info.modifier.is_none()).map(|s| Combo { mods, trigger: Trigger::Keysym(s) })
}

/// Whether a key does something together with the selected modifiers. With
/// nothing selected, whether it takes part in any active shortcut (a key that
/// only types its character does not).
fn is_assigned(ui: &Ui, evdev: u32, mods: Mods) -> bool {
    let st = ui.st.borrow();
    let info = st.resolver.key(evdev);
    if st.selection.is_empty() {
        return match (info.modifier, info.sym) {
            (Some(m), _) => st.model.effective.keys().any(|c| c.is_keyboard() && c.mods.contains(m)),
            (None, Some(sym)) => st.model.effective.keys().any(|c| c.trigger == Trigger::Keysym(sym)),
            _ => false,
        };
    }
    match (info.modifier, info.sym) {
        (Some(m), _) => {
            let want = mods.with(m);
            st.model.effective.keys().any(|c| c.is_keyboard() && c.mods.contains(want))
        }
        (None, Some(sym)) => st.model.effective.contains_key(&Combo { mods, trigger: Trigger::Keysym(sym) }),
        _ => false,
    }
}

pub fn build(ui: &Rc<Ui>, win: &Rc<Win>) {
    let (mw, mh) = ui.monitor_size();
    let settings = ui.st.borrow().settings.clone();
    let unit = unit_for(&settings, mw, mh);
    let gap = (unit * 0.08).round().max(2.0);
    let lay = geometry::layout(settings.form_factor);

    let fixed = gtk::Fixed::new();
    fixed.set_size_request((lay.width * unit) as i32, (lay.height * unit) as i32);
    fixed.set_halign(gtk::Align::Center);
    let mut keys = win.keys.borrow_mut();
    keys.clear();
    let mut boxes = win.key_boxes.borrow_mut();
    boxes.clear();
    let mut centers = Vec::with_capacity(lay.keys.len());
    for g in &lay.keys {
        let info = ui.st.borrow().resolver.key(g.evdev);
        let b = gtk::Button::new();
        b.add_css_class("km-key");
        b.set_size_request(((g.w * unit) - gap).max(8.0) as i32, ((g.h * unit) - gap).max(8.0) as i32);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_valign(gtk::Align::Center);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        // Small keys show only the main legend.
        let roomy = unit >= 34.0;
        if let Some(s) = info.shifted.as_ref().filter(|_| roomy) {
            let l = gtk::Label::new(Some(s));
            l.add_css_class("km-shifted");
            top.append(&l);
        }
        if top.first_child().is_some() {
            content.append(&top);
        }
        let legend = gtk::Label::new(None);
        let (legend_px, pad_h, bw) = crate::style::key_text_metrics(unit);
        let room = (g.w * unit) - gap - 2.0 * (pad_h + bw);
        legend.set_label(&short_legend(&info.label, |t| text_width(&legend, t, legend_px) <= room));
        legend.add_css_class("km-legend");
        legend.set_ellipsize(gtk::pango::EllipsizeMode::End);
        content.append(&legend);
        b.set_child(Some(&content));
        let name = if info.label.is_empty() { format!("key {}", g.evdev) } else { info.label.clone() };
        b.update_property(&[gtk::accessible::Property::Label(&name)]);
        if info.sym.is_none() && info.modifier.is_none() {
            b.add_css_class("disabled");
            b.set_sensitive(false);
        }
        let evdev = g.evdev;
        let weak = Rc::downgrade(ui);
        b.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.toggle_key(evdev);
            }
        });
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(ui);
        let bw = b.clone();
        motion.connect_enter(move |_, _, _| {
            if let Some(ui) = weak.upgrade() {
                show_tip(&ui, evdev, bw.upcast_ref());
            }
        });
        let weak = Rc::downgrade(ui);
        motion.connect_leave(move |_| {
            if let Some(ui) = weak.upgrade()
                && let Some(w) = ui.win.borrow().as_ref()
            {
                w.tip.set_visible(false);
            }
        });
        b.add_controller(motion);
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(ui);
        let bw = b.clone();
        focus.connect_enter(move |_| {
            // Only for keyboard navigation, not the initial focus at startup.
            let visible = bw.root().and_downcast::<gtk::Window>().is_some_and(|w| w.gets_focus_visible());
            if let (true, Some(ui)) = (visible, weak.upgrade()) {
                show_tip(&ui, evdev, bw.upcast_ref());
            }
        });
        let weak = Rc::downgrade(ui);
        focus.connect_leave(move |_| {
            if let Some(ui) = weak.upgrade()
                && let Some(w) = ui.win.borrow().as_ref()
            {
                w.tip.set_visible(false);
            }
        });
        b.add_controller(focus);
        fixed.put(&b, g.x * unit + gap / 2.0, g.y * unit + gap / 2.0);
        centers.push(((g.x + g.w / 2.0) * unit, (g.y + g.h / 2.0) * unit));
        boxes.push(super::opening::KeyBox { widget: b.clone().upcast(), distance: 0.0 });
        keys.insert(g.evdev, b.upcast());
    }
    // The opening ripple's stagger, once per layout rather than per frame.
    for (k, d) in boxes.iter_mut().zip(keymap::motion::distances(&centers, lay.width * unit, lay.height * unit)) {
        k.distance = d;
    }
    drop(keys);
    drop(boxes);
    win.keyboard_slot.append(&fixed);
    refresh_states(ui);
}

/// Update selection/assignment classes and the hint without rebuilding keys.
pub fn refresh_states(ui: &Rc<Ui>) {
    let Some(win) = ui.win.borrow().clone() else { return };
    let mods = selected_mods(ui);
    let (selection, invalid, assigned_only) = {
        let st = ui.st.borrow();
        (st.selection.clone(), st.invalid_keys.clone(), st.settings.assigned_only)
    };
    for (k, w) in win.keys.borrow().iter() {
        let selected = selection.contains(k);
        let assigned = is_assigned(ui, *k, mods);
        set_class(w, "selected", selected);
        set_class(w, "invalid", invalid.contains(k));
        set_class(w, "assigned", assigned);
        // The filter pauses while a combination is being built.
        set_class(w, "faded", assigned_only && selection.is_empty() && !assigned);
        w.update_state(&[gtk::accessible::State::Pressed(if selected {
            gtk::AccessibleTristate::True
        } else {
            gtk::AccessibleTristate::False
        })]);
    }
    // The hint lives in its own overlay child above the keyboard (see
    // Ui::position_hint), so its length never moves the keyboard.
    let row = &win.hint;
    while let Some(c) = row.first_child() {
        row.remove(&c);
    }
    if ui.is_demo() {
        let d = gtk::Label::new(Some("Demo — editing a temporary copy"));
        d.add_css_class("km-demo");
        d.set_halign(gtk::Align::Center);
        row.append(&d);
    }
    let hint = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    hint.add_css_class("km-hint");
    hint.set_halign(gtk::Align::Center);
    let st = ui.st.borrow();
    if let Some((msg, err)) = &st.hint {
        if *err {
            hint.add_css_class("error");
        }
        hint.append(&hint_label(msg));
    } else if !st.selection.is_empty() {
        drop(st);
        match ui.selection_combo() {
            Ok(c) => {
                hint.append(&super::widgets::caps(&c));
                let st = ui.st.borrow();
                let what = st
                    .model
                    .effective_for(&c)
                    .and_then(|b| st.model.action_of(b))
                    .map(|a| format!("runs “{}”", a.name))
                    .unwrap_or_else(|| "unassigned".into());
                let l = hint_label(&format!("{what} · Enter to choose an action"));
                l.add_css_class("km-muted");
                hint.append(&l);
            }
            Err(_) => {
                let l = hint_label("Add one non-modifier key · Enter to continue");
                l.add_css_class("km-muted");
                hint.append(&l);
            }
        }
    } else {
        let text = if st.model.read_only.is_some() {
            "Read-only: the configuration has errors (Tab → Settings for details)"
        } else {
            "Click keys to build a shortcut · Enter: choose action · Tab: settings · Esc: close"
        };
        let l = hint_label(text);
        l.add_css_class("km-muted");
        hint.append(&l);
    }
    row.append(&hint);
    ui.trace_bounds();
}

/// Hint text wraps (upward, see Ui::position_hint) instead of widening.
fn hint_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_justify(gtk::Justification::Center);
    l.set_max_width_chars(90);
    l
}

/// A legend that fits the key: numpad keys drop the "Num" prefix (their
/// position says it) and keys too narrow for the full word (`fits`) use
/// common abbreviations.
fn short_legend(label: &str, fits: impl Fn(&str) -> bool) -> String {
    let l = match label.strip_prefix("Num ") {
        Some(rest) if rest != "Lock" => rest,
        _ => label,
    };
    if l.chars().count() <= 3 || fits(l) {
        return l.to_string();
    }
    match l {
        "Super" => "Sup",
        "Print" => "Prt",
        "Scroll Lock" | "Scroll" => "ScrL",
        "Pause" => "Pse",
        "Insert" => "Ins",
        "Delete" => "Del",
        "Home" => "Hm",
        "Menu" => "Mnu",
        "PgUp" => "PgU",
        "PgDn" => "PgD",
        "Num Lock" => "Num",
        "Backspace" => "Bksp",
        "Enter" => "Ent",
        _ => l,
    }
    .to_string()
}

/// Width in pixels of `text` in the legend font (the `.km-legend` size and
/// weight, in the label's font family).
fn text_width(label: &gtk::Label, text: &str, px: f64) -> f64 {
    let layout = label.create_pango_layout(Some(text));
    let mut font = layout.context().font_description().unwrap_or_default();
    font.set_absolute_size(px * f64::from(gtk::pango::SCALE));
    font.set_weight(gtk::pango::Weight::Medium);
    layout.set_font_description(Some(&font));
    f64::from(layout.pixel_size().0)
}

fn set_class(w: &gtk::Widget, c: &str, on: bool) {
    if on {
        w.add_css_class(c);
    } else {
        w.remove_css_class(c);
    }
}

fn show_tip(ui: &Rc<Ui>, evdev: u32, key: &gtk::Widget) {
    let Some(win) = ui.win.borrow().clone() else { return };
    if ui.st.borrow().view != super::View::Keyboard {
        return;
    }
    let tip = &win.tip;
    while let Some(c) = tip.first_child() {
        tip.remove(&c);
    }
    let mods = selected_mods(ui);
    let st = ui.st.borrow();
    let info = st.resolver.key(evdev);
    let title = gtk::Label::new(Some(&if info.label.is_empty() { format!("Key {evdev}") } else { info.label.clone() }));
    title.add_css_class("km-tip-title");
    title.set_xalign(0.0);
    tip.append(&title);
    let row = |text: &str, muted: bool| {
        let l = gtk::Label::new(Some(text));
        l.add_css_class("km-tip-row");
        if muted {
            l.add_css_class("km-muted");
        }
        l.set_xalign(0.0);
        l
    };
    // Say in words what the outline colour means, so colour is never the only cue.
    let invalid = st.invalid_keys.contains(&evdev);
    let with = if mods.is_empty() { String::new() } else { format!(" with {}", mods_label(mods)) };
    drop(st);
    let status = if invalid {
        "Red outline: this key keeps the selection from being a shortcut".to_string()
    } else if is_assigned(ui, evdev, mods) {
        format!("Mint outline: used by an active shortcut{with}")
    } else if info.modifier.is_some() || info.sym.is_some() {
        format!("No active shortcut{with}")
    } else {
        "Not available for shortcuts".to_string()
    };
    let st = ui.st.borrow();
    let s = row(&status, !invalid);
    if invalid {
        s.add_css_class("km-error-text");
    }
    tip.append(&s);
    if let Some(id) = &info.binds_as {
        // The legend is the US one; say what shortcuts actually use.
        let what = if info.modifier.is_some() { "Acts as" } else { "Shortcuts name it" };
        tip.append(&row(&format!("{what} {id} in your keyboard layout"), true));
    }
    if info.modifier.is_some() {
        tip.append(&row("Modifier — combine with another key", true));
    } else if let Some(sym) = info.sym {
        drop(st);
        let combo = combo_for(ui, evdev, mods);
        let st = ui.st.borrow();
        if let Some(c) = &combo {
            let current = st.model.effective_for(c).and_then(|b| st.model.action_of(b)).map(|a| a.name.clone());
            tip.append(&row(&format!("{}: {}", c.label(), current.as_deref().unwrap_or("unassigned")), current.is_none()));
        }
        let mut others: Vec<(&Combo, String)> = st
            .model
            .effective
            .iter()
            .filter(|(c, _)| c.trigger == Trigger::Keysym(sym) && Some(*c) != combo.as_ref())
            .filter_map(|(c, b)| st.model.action_of(&st.model.bindings[*b]).map(|a| (c, a.name.clone())))
            .collect();
        others.sort_by_cached_key(|(c, _)| shortest_first(c));
        let more = others.len().saturating_sub(6);
        for (c, a) in others.into_iter().take(6) {
            tip.append(&row(&format!("{}: {a}", c.label()), false));
        }
        if more > 0 {
            tip.append(&row(&format!("and {more} more"), true));
        }
    }
    let Some(bounds) = key.compute_bounds(&win.overlay).filter(|b| b.width() > 0.0) else { return };
    let (tw, _, _, _) = tip.measure(gtk::Orientation::Horizontal, -1);
    let (th, _, _, _) = tip.measure(gtk::Orientation::Vertical, tw);
    let ow = win.overlay.width() as f32;
    let oh = win.overlay.height() as f32;
    let mut x = bounds.x();
    let mut y = bounds.y() + bounds.height() + 8.0;
    if y + th as f32 > oh - 8.0 {
        y = bounds.y() - th as f32 - 8.0;
    }
    if x + tw as f32 > ow - 8.0 {
        x = ow - tw as f32 - 8.0;
    }
    tip.set_margin_start(x.max(8.0) as i32);
    tip.set_margin_top(y.max(8.0) as i32);
    tip.set_visible(true);
}

/// Sort key for a key's shortcuts: fewest modifiers first, so the shortest
/// ones are at the top (and survive the cut-off); equal lengths by name.
fn shortest_first(c: &Combo) -> (u32, String) {
    (c.mods.count(), c.label())
}

/// "Super+Shift" style label for the selected modifiers.
fn mods_label(mods: Mods) -> String {
    let mut parts = Combo { mods, trigger: Trigger::Keysym(0) }.label_parts();
    parts.pop();
    parts.join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(ff: FormFactor, scale: f64) -> f64 {
        unit_for(&Settings { form_factor: ff, scale, ..Settings::default() }, 1920.0, 1080.0)
    }

    #[test]
    fn full_keyboard_has_the_75_key_size_and_is_only_wider() {
        assert_eq!(unit(FormFactor::Full, 0.55), unit(FormFactor::SeventyFive, 0.55));
        let full = geometry::layout(FormFactor::Full).width * unit(FormFactor::Full, 0.55);
        let tkl = geometry::layout(FormFactor::SeventyFive).width * unit(FormFactor::SeventyFive, 0.55);
        assert!(full > tkl * 1.3);
        // At a size where it would not fit, it shrinks to the screen instead.
        let big = unit(FormFactor::Full, 0.9);
        assert!(big < unit(FormFactor::SeventyFive, 0.9));
        assert!(geometry::layout(FormFactor::Full).width * big <= 1920.0 * 0.96 + 1e-9);
    }

    #[test]
    fn hover_list_puts_the_shortest_shortcuts_first() {
        let m = |mods: &[Mods]| Combo { mods: mods.iter().fold(Mods::empty(), |a, b| a.with(*b)), trigger: Trigger::Keysym(0x6d) };
        let mut combos = [
            m(&[Mods::CTRL, Mods::SHIFT, Mods::ALT, Mods::SUPER]),
            m(&[Mods::SUPER, Mods::CTRL]),
            m(&[Mods::ALT]),
            m(&[Mods::SUPER, Mods::SHIFT, Mods::CTRL]),
            m(&[]),
            m(&[Mods::SUPER, Mods::SHIFT]),
            m(&[Mods::CTRL]),
            m(&[Mods::LEVEL3, Mods::SHIFT]),
        ];
        combos.sort_by_cached_key(shortest_first);
        let counts: Vec<u32> = combos.iter().map(|c| c.mods.count()).collect();
        assert_eq!(counts, [0, 1, 1, 2, 2, 2, 3, 4]);
        // Equal lengths keep a stable, readable order.
        let labels: Vec<String> = combos.iter().map(Combo::label).collect();
        assert_eq!(labels[1..3], ["Alt + M", "Ctrl + M"]);
    }
}
