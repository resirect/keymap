//! Small shared widgets and the confirmation panel.

use super::{Pending, Ui, View};
use gtk::prelude::*;
use keymap_core::keys::Combo;
use keymap_core::plan::{ExistingShortcut, Others};
use std::rc::Rc;

/// Widget name of the button that receives focus when a panel opens.
pub const DEFAULT_FOCUS: &str = "km-default";

pub fn label(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    // Bound the natural width so long text wraps instead of widening panels.
    l.set_max_width_chars(64);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

/// Keycap chips: `[Super] + [Shift] + [S]`.
pub fn caps(combo: &Combo) -> gtk::Box {
    caps_from(&combo.label_parts())
}

pub fn caps_from(parts: &[String]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            let plus = gtk::Label::new(Some("+"));
            plus.add_css_class("km-plus");
            b.append(&plus);
        }
        let cap = gtk::Label::new(Some(p));
        cap.add_css_class("km-cap");
        b.append(&cap);
    }
    b
}

pub fn button(text: &str, classes: &[&str]) -> gtk::Button {
    let b = gtk::Button::with_label(text);
    for c in classes {
        b.add_css_class(c);
    }
    b
}

/// A centred glass panel of the given width, capped to the screen.
pub fn panel(ui: &Rc<Ui>, width: f64, height_frac: Option<f64>) -> gtk::Box {
    let (mw, mh) = ui.monitor_size();
    let p = gtk::Box::new(gtk::Orientation::Vertical, 12);
    p.add_css_class("km-panel");
    p.set_halign(gtk::Align::Center);
    p.set_valign(gtk::Align::Center);
    p.set_size_request(width.min(mw - 32.0) as i32, -1);
    if let Some(f) = height_frac {
        p.set_size_request(width.min(mw - 32.0) as i32, (mh * f) as i32);
    }
    // Clicks inside the panel must not reach the scrim behind it.
    let g = gtk::GestureClick::new();
    g.connect_released(|g, _, _, _| {
        g.set_state(gtk::EventSequenceState::Claimed);
    });
    p.add_controller(g);
    p
}

/// A vertically scrolling panel area. The scrollbar has its own gutter beside
/// the content (no overlay scrollbars drawn over rows), the content is clipped
/// to the viewport, and the gutter keeps its width when nothing overflows, so
/// rows never jump sideways while a search filters the list.
pub fn scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new();
    scroll.add_css_class("km-scroll");
    scroll.set_vexpand(true);
    scroll.set_overlay_scrolling(false);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Always);
    scroll.set_child(Some(child));
    let adj = scroll.vadjustment();
    let update = {
        let scroll = scroll.downgrade();
        move |a: &gtk::Adjustment| {
            let Some(scroll) = scroll.upgrade() else { return };
            let fits = a.upper() - a.lower() <= a.page_size() + 0.5;
            if fits {
                scroll.add_css_class("km-fits");
            } else {
                scroll.remove_css_class("km-fits");
            }
            // An invisible track must not take clicks either.
            scroll.vscrollbar().set_can_target(!fits);
        }
    };
    update(&adj);
    adj.connect_changed(update);
    scroll
}

pub fn button_row() -> gtk::Box {
    let r = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    r.set_halign(gtk::Align::End);
    r.set_margin_top(4);
    r
}

pub fn spacer() -> gtk::Box {
    let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    s.set_hexpand(true);
    s
}

/// A wrapping label with Pango markup (callers escape their own text).
fn markup(text: &str, classes: &[&str]) -> gtk::Label {
    let l = label("", classes);
    l.set_markup(text);
    l
}

/// Bold, escaped combination for use inside sentences.
fn combo_markup(c: &Combo) -> String {
    format!("<b>{}</b>", gtk::glib::markup_escape_text(&c.label()))
}

/// A shortcut's consequence or limitation, shown under it.
fn shortcut_note(s: &ExistingShortcut) -> Option<String> {
    match (&s.blocked, &s.reveals) {
        (Some(b), _) => Some(b.clone()),
        (None, Some(r)) => Some(format!("Removing it reveals an earlier declaration: {r}")),
        (None, None) => None,
    }
}

pub fn banner(text: &str, error: bool) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("km-banner");
    if error {
        b.add_css_class("error");
    }
    b.append(&label(text, &["km-small"]));
    b
}

fn find_named(w: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if w.widget_name() == name {
        return Some(w.clone());
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        // Hidden parts of a kept panel (e.g. the other browser tab) are skipped.
        if ch.is_visible()
            && let Some(f) = find_named(&ch, name)
        {
            return Some(f);
        }
        c = ch.next_sibling();
    }
    None
}

/// Focus the panel's designated default widget, else its first focusable one.
pub fn focus_default(panel: &gtk::Widget) {
    let p = panel.clone();
    gtk::glib::idle_add_local_once(move || {
        if let Some(w) = find_named(&p, DEFAULT_FOCUS) {
            w.grab_focus();
        } else {
            p.child_focus(gtk::DirectionType::TabForward);
        }
    });
}

/// "This action already has a shortcut": replace one of them, or add the new
/// combination as another. With several, the user picks which to replace;
/// nothing is preselected, so none is ever removed by default.
fn existing_shortcuts(
    ui: &Rc<Ui>,
    p: &gtk::Box,
    row: &gtk::Box,
    cancel: &gtk::Button,
    info: keymap_core::plan::Existing,
    target: String,
    pick: Option<Combo>,
) {
    let several = info.shortcuts.len() > 1;
    p.append(&label(if several { "This action already has shortcuts" } else { "This action already has a shortcut" }, &["km-title"]));
    let replace = button("Replace existing", &["km-danger"]);
    // One fact per line: the action, what it has now, what is proposed. The
    // phrases share a width so the keycaps line up beside them, whatever the
    // length of the action's name.
    let facts = gtk::Box::new(gtk::Orientation::Vertical, 10);
    facts.append(&label(&format!("“{}”", info.action), &["km-dialog-action"]));
    let column = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
    let phrase = |t: &str| {
        let l = label(t, &["km-muted"]);
        l.set_valign(gtk::Align::Center);
        l
    };
    let pair = |t: &str, keys: &gtk::Box| {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let l = phrase(t);
        column.add_widget(&l);
        keys.set_valign(gtk::Align::Center);
        b.append(&l);
        b.append(keys);
        b
    };
    if several {
        facts.append(&phrase("is currently assigned to"));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let mut group: Option<gtk::CheckButton> = None;
        for s in &info.shortcuts {
            let item = gtk::Box::new(gtk::Orientation::Vertical, 2);
            item.append(&caps(&s.combo));
            if let Some(n) = shortcut_note(s) {
                item.append(&label(&n, &["km-muted", "km-small"]));
            }
            let choice = gtk::CheckButton::new();
            choice.set_child(Some(&item));
            choice.set_group(group.as_ref());
            choice.set_active(pick.as_ref() == Some(&s.combo));
            choice.set_sensitive(s.blocked.is_none());
            let weak = Rc::downgrade(ui);
            let (combo, replace) = (s.combo.clone(), replace.clone());
            choice.connect_toggled(move |c| {
                let Some(ui) = weak.upgrade() else { return };
                if c.is_active() {
                    if let Some(Pending::Existing { pick, .. }) = ui.st.borrow_mut().draft.as_mut().and_then(|d| d.pending.as_mut()) {
                        *pick = Some(combo.clone());
                    }
                    replace.set_sensitive(true);
                }
            });
            list.append(&choice);
            group.get_or_insert(choice);
        }
        facts.append(&list);
        replace.set_sensitive(pick.is_some());
    } else {
        let s = &info.shortcuts[0];
        facts.append(&pair("is currently assigned to", &caps(&s.combo)));
        if let Some(n) = shortcut_note(s) {
            facts.append(&banner(&n, false));
        }
        replace.set_sensitive(s.blocked.is_none());
    }
    let new = caps(&info.combo);
    new.append(&gtk::Label::new(Some("?")));
    facts.append(&pair("How would you like to use", &new));
    if several {
        facts.append(&label("To replace one, choose it above.", &["km-muted", "km-small"]));
    }
    p.append(&facts);
    let only = (!several).then(|| info.shortcuts[0].combo.clone());
    let weak = Rc::downgrade(ui);
    let (combo, t) = (info.combo.clone(), target.clone());
    replace.connect_clicked(move |_| {
        let Some(ui) = weak.upgrade() else { return };
        let chosen = only.clone().or_else(|| match ui.st.borrow().draft.as_ref().and_then(|d| d.pending.as_ref()) {
            Some(Pending::Existing { pick, .. }) => pick.clone(),
            _ => None,
        });
        if let Some(c) = chosen {
            ui.assign(&combo, &t, &Others::Replace(c), false);
        }
    });
    let add = button("Add another shortcut", &[]);
    let weak = Rc::downgrade(ui);
    let combo = info.combo.clone();
    add.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.assign(&combo, &target, &Others::Keep, false);
        }
    });
    row.append(cancel);
    row.append(&add);
    row.append(&replace);
}

/// Replacement, removal and discard confirmations. Cancel has the focus, so a
/// stray Enter never replaces or deletes anything.
pub fn confirm_panel(ui: &Rc<Ui>) -> gtk::Box {
    let width = 520f64.min(ui.monitor_size().0 - 32.0);
    let p = panel(ui, width, None);
    p.add_css_class("km-dialog");
    p.set_spacing(16);
    let pending = ui.st.borrow().draft.as_ref().and_then(|d| d.pending.clone());
    let cancel = button("Cancel", &[]);
    cancel.set_widget_name(DEFAULT_FOCUS);
    let weak = Rc::downgrade(ui);
    cancel.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.back();
        }
    });
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_top(4);
    match pending {
        Some(Pending::Replace { rep, target, others }) => {
            p.append(&label("Replace this shortcut?", &["km-title"]));
            p.append(&caps(&rep.combo));
            p.append(&label(&format!("It currently runs “{}”.", rep.current), &[]));
            p.append(&label(&format!("After saving it will run “{}”.", rep.proposed), &[]));
            match &rep.also_removes {
                Some(r) => {
                    let esc = gtk::glib::markup_escape_text(&rep.proposed);
                    p.append(&markup(&format!("The same save removes {} from “{esc}”.", combo_markup(&r.combo)), &[]));
                    if let Some(n) = shortcut_note(r) {
                        p.append(&banner(&n, false));
                    }
                    p.append(&label("Every other shortcut stays as it is.", &["km-muted", "km-small"]));
                }
                None => p.append(&label("Other shortcuts of both actions stay as they are.", &["km-muted", "km-small"])),
            }
            let ok = button("Replace", &["km-danger"]);
            let weak = Rc::downgrade(ui);
            let combo = rep.combo.clone();
            ok.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.assign(&combo, &target, &others, true);
                }
            });
            row.append(&cancel);
            row.append(&ok);
        }
        Some(Pending::Existing { info, target, pick }) => existing_shortcuts(ui, &p, &row, &cancel, info, target, pick),
        Some(Pending::Remove { binding, combo, reveals, everywhere }) => {
            let (name, place) = {
                let st = ui.st.borrow();
                // A reload (the removal's own commit, or an outside edit)
                // may have renumbered the bindings under this panel.
                match st.model.bindings.get(binding) {
                    Some(b) => (
                        st.model.action_of(b).map(|a| a.name.clone()).unwrap_or_else(|| "this action".into()),
                        format!("{}:{}", b.path.display(), b.line),
                    ),
                    None => ("this action".into(), "no longer in the configuration".into()),
                }
            };
            p.append(&label(if everywhere { "Remove every declaration?" } else { "Remove this shortcut?" }, &["km-title"]));
            p.append(&caps(&combo));
            p.append(&label(&format!("It runs “{name}” ({place}). The action itself is kept."), &[]));
            match reveals {
                Some(r) => p.append(&banner(&format!("Removing it reveals an earlier declaration: {r}"), false)),
                None => p.append(&label("Afterwards this combination does nothing.", &["km-muted", "km-small"])),
            }
            let ok = button("Remove", &["km-danger"]);
            let weak = Rc::downgrade(ui);
            ok.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.remove_binding(binding, everywhere);
                }
            });
            row.append(&cancel);
            row.append(&ok);
        }
        Some(Pending::DiscardForm) => {
            p.append(&label("Discard this new action?", &["km-title"]));
            p.append(&label("The details you entered will be lost. Nothing has been saved.", &[]));
            let ok = button("Discard", &["km-danger"]);
            let weak = Rc::downgrade(ui);
            ok.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    if let Some(d) = ui.st.borrow_mut().draft.as_mut() {
                        d.form = None;
                        d.pending = None;
                    }
                    ui.set_view(View::Browser);
                }
            });
            row.append(&cancel);
            row.append(&ok);
        }
        None => {
            p.append(&label("Nothing to confirm.", &[]));
            row.append(&cancel);
        }
    }
    // Content width: the panel less its 24 px padding and 1 px border.
    let content = (width - 50.0) as i32;
    fit_text(&p, content, width as i32);
    fit_dialog_buttons(&row, content);
    p.append(&row);
    p
}

/// Wrap every label within the content width (in this font and text scale),
/// so long names and messages wrap instead of widening the panel past
/// `panel_width`. Character widths are estimates (bold text runs wider), so
/// the panel is measured and the limit tightened until it fits.
fn fit_text(p: &gtk::Box, content: i32, panel_width: i32) {
    let mut labels = Vec::new();
    let mut stack = vec![p.clone().upcast::<gtk::Widget>()];
    while let Some(w) = stack.pop() {
        if let Some(l) = w.downcast_ref::<gtk::Label>()
            && l.wraps()
        {
            labels.push((l.clone(), l.max_width_chars()));
        }
        let mut c = w.first_child();
        while let Some(k) = c {
            c = k.next_sibling();
            stack.push(k);
        }
    }
    let char_px = p.pango_context().metrics(None, None).approximate_char_width() as f64 / gtk::pango::SCALE as f64;
    let mut chars = ((content as f64 / char_px.max(1.0)) as i32).max(8);
    loop {
        for (l, max) in &labels {
            l.set_max_width_chars((*max).min(chars));
        }
        let (_, natural, _, _) = p.measure(gtk::Orientation::Horizontal, -1);
        if natural <= panel_width || chars <= 8 {
            break;
        }
        chars = (chars * 9 / 10).max(8);
    }
}

/// Confirmation buttons span the content width: Cancel keeps its natural
/// width on the left edge, the actions share the rest, so both ends sit on
/// the content's edges with the same 12 px gaps between. When they do not fit
/// side by side (a narrow output, large text), they stack at full width
/// rather than clip.
fn fit_dialog_buttons(row: &gtk::Box, width: i32) {
    let mut kids = Vec::new();
    let mut c = row.first_child();
    while let Some(k) = c {
        c = k.next_sibling();
        kids.push(k);
    }
    let alone = kids.len() == 1;
    for (i, k) in kids.iter().enumerate() {
        k.set_hexpand(i > 0 || alone);
    }
    let (_, natural, _, _) = row.measure(gtk::Orientation::Horizontal, -1);
    if natural > width {
        row.set_orientation(gtk::Orientation::Vertical);
        row.set_spacing(8);
        for k in &kids {
            k.set_hexpand(true);
        }
    }
}
