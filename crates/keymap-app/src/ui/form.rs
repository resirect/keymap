//! New-action form: run a program (argv, no shell), an explicit shell
//! command, or a Niri action with parameters.

use super::widgets::{self, banner, button, label};
use super::{Pending, Ui, View};
use gtk::prelude::*;
use keymap_core::catalog::{self, ParamKind};
use keymap_core::kdl::Value;
use keymap_core::model::Payload;
use keymap_core::store::{CustomAction, Label};
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Program,
    Shell,
    Niri,
}

#[derive(Debug, Clone)]
pub struct FormState {
    pub mode: Mode,
    pub name: String,
    pub category: String,
    pub program: String,
    pub shell: String,
    /// Index into `catalog::ACTIONS`.
    pub niri: usize,
    pub values: Vec<String>,
    pub dirty: bool,
}

impl Default for FormState {
    fn default() -> Self {
        FormState {
            mode: Mode::Program,
            name: String::new(),
            category: String::new(),
            program: String::new(),
            shell: String::new(),
            niri: 0,
            values: vec![],
            dirty: false,
        }
    }
}

/// Split a command line into argv with simple quoting (`"..."`, `'...'`,
/// backslash). No variables, globs or other shell features.
pub fn split_argv(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = s.chars();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => match chars.next() {
                Some(n @ ('"' | '\\')) => cur.push(n),
                Some(n) => {
                    cur.push('\\');
                    cur.push(n);
                }
                None => return Err("ends with a backslash".into()),
            },
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => match chars.next() {
                Some(n) => {
                    cur.push(n);
                    in_word = true;
                }
                None => return Err("ends with a backslash".into()),
            },
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("has an unclosed quote".into());
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

fn parametric() -> Vec<usize> {
    let mut v: Vec<usize> = (0..catalog::ACTIONS.len()).filter(|i| !matches!(catalog::ACTIONS[*i].name, "spawn" | "spawn-sh")).collect();
    v.sort_by_key(|i| (catalog::ACTIONS[*i].category, catalog::ACTIONS[*i].label));
    v
}

fn value_for(kind: ParamKind, raw: &str) -> Result<Option<Value>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    let int = |s: &str| s.parse::<i64>().ok();
    Ok(Some(match kind {
        ParamKind::Int { min, max } => match int(raw) {
            Some(n) if (min..=max).contains(&n) => Value::Number(n.to_string()),
            _ => return Err(format!("must be a whole number from {min} to {max}")),
        },
        ParamKind::Workspace => match int(raw) {
            Some(n) if (0..=255).contains(&n) => Value::Number(n.to_string()),
            Some(_) => return Err("workspace index must be 0–255".into()),
            None => Value::String(raw.into()),
        },
        ParamKind::Size => {
            let ok = regex_like_size(raw);
            if !ok {
                return Err("use a size like 50%, +10%, -10% or 1200".into());
            }
            Value::String(raw.into())
        }
        ParamKind::Bool => match raw {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err("must be true or false".into()),
        },
        ParamKind::Choice(opts) => {
            if !opts.contains(&raw) {
                return Err(format!("must be one of {}", opts.join(", ")));
            }
            match int(raw) {
                Some(n) => Value::Number(n.to_string()),
                None => Value::String(raw.into()),
            }
        }
        ParamKind::Text | ParamKind::Argv | ParamKind::Shell => Value::String(raw.into()),
    }))
}

fn regex_like_size(s: &str) -> bool {
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let body = body.strip_suffix('%').unwrap_or(body);
    !body.is_empty() && body.parse::<f64>().is_ok()
}

/// Build the payload described by the form, or explain what is missing.
pub fn payload(f: &FormState) -> Result<Payload, String> {
    match f.mode {
        Mode::Program => {
            let argv = split_argv(&f.program).map_err(|e| format!("The command {e}."))?;
            if argv.is_empty() {
                return Err("Enter the program to run.".into());
            }
            Ok(Payload::spawn(&argv))
        }
        Mode::Shell => {
            if f.shell.trim().is_empty() {
                return Err("Enter the shell command.".into());
            }
            Ok(Payload::spawn_sh(f.shell.trim()))
        }
        Mode::Niri => {
            let def = &catalog::ACTIONS[f.niri];
            let mut p = Payload { name: def.name.into(), args: vec![], props: vec![], has_children: false };
            for (i, spec) in def.params.iter().enumerate() {
                let raw = f.values.get(i).map(String::as_str).unwrap_or("");
                match value_for(spec.kind, raw).map_err(|e| format!("{} {e}.", spec.label))? {
                    None if spec.required => return Err(format!("{} is required.", spec.label)),
                    None => {}
                    Some(v) => match spec.prop {
                        Some(name) => p.props.push((name.into(), v)),
                        None => p.args.push(v),
                    },
                }
            }
            Ok(p)
        }
    }
}

pub fn open(ui: &Rc<Ui>) {
    if let Some(d) = ui.st.borrow_mut().draft.as_mut() {
        if d.form.is_none() {
            d.form = Some(FormState::default());
        }
        d.error = None;
        d.return_to = View::Form;
    }
    ui.set_view(View::Form);
}

/// Esc / Cancel: ask before throwing away typed details.
pub fn request_close(ui: &Rc<Ui>) {
    let dirty = ui.st.borrow().draft.as_ref().and_then(|d| d.form.as_ref()).is_some_and(|f| f.dirty);
    if dirty {
        if let Some(d) = ui.st.borrow_mut().draft.as_mut() {
            d.pending = Some(Pending::DiscardForm);
            d.return_to = View::Form;
        }
        ui.set_view(View::Confirm);
    } else {
        if let Some(d) = ui.st.borrow_mut().draft.as_mut() {
            d.form = None;
            d.return_to = View::Browser;
        }
        ui.set_view(View::Browser);
    }
}

fn update(ui: &Rc<Ui>, f: impl FnOnce(&mut FormState)) {
    if let Some(form) = ui.st.borrow_mut().draft.as_mut().and_then(|d| d.form.as_mut()) {
        f(form);
        form.dirty = true;
    }
}

fn entry(ui: &Rc<Ui>, text: &str, placeholder: &str, set: fn(&mut FormState, String)) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.set_text(text);
    e.set_placeholder_text(Some(placeholder));
    let weak = Rc::downgrade(ui);
    e.connect_changed(move |e| {
        if let Some(ui) = weak.upgrade() {
            let t = e.text().to_string();
            update(&ui, |f| set(f, t));
        }
    });
    e
}

pub fn build(ui: &Rc<Ui>) -> gtk::Box {
    let p = widgets::panel(ui, 620.0, None);
    let Some(d) = ui.st.borrow().draft.clone() else { return p };
    let f = d.form.clone().unwrap_or_default();
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    head.append(&label("New action for", &["km-title"]));
    head.append(&widgets::caps(&d.combo));
    p.append(&head);
    if let Some(e) = &d.error {
        p.append(&banner(e, true));
    }

    let modes = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let mut group: Option<gtk::ToggleButton> = None;
    for (m, text) in [(Mode::Program, "Run a program"), (Mode::Shell, "Shell command"), (Mode::Niri, "Niri action")] {
        let b = gtk::ToggleButton::with_label(text);
        b.add_css_class("km-tab");
        b.set_group(group.as_ref());
        b.set_active(f.mode == m);
        let weak = Rc::downgrade(ui);
        b.connect_toggled(move |b| {
            if b.is_active()
                && let Some(ui) = weak.upgrade()
            {
                update(&ui, |f| f.mode = m);
                ui.render_panel();
            }
        });
        modes.append(&b);
        group.get_or_insert(b);
    }
    p.append(&modes);

    p.append(&label("Name", &["km-section"]));
    let names = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = entry(ui, &f.name, "Shown in Keymap (optional)", |f, t| f.name = t);
    name.set_hexpand(true);
    names.append(&name);
    let category = entry(ui, &f.category, "Category (optional)", |f, t| f.category = t);
    category.update_property(&[gtk::accessible::Property::Label("Category")]);
    names.append(&category);
    p.append(&names);

    let preview = label("", &["km-mono", "km-muted"]);
    match f.mode {
        Mode::Program => {
            p.append(&label("Command", &["km-section"]));
            let e = entry(ui, &f.program, "e.g. foot -e htop", |f, t| f.program = t);
            e.set_widget_name(widgets::DEFAULT_FOCUS);
            p.append(&e);
            p.append(&label(
                "Runs the program directly with these arguments. Quotes group words; there is no shell, so $VARS, ~, pipes and globs are passed literally.",
                &["km-small", "km-muted"],
            ));
        }
        Mode::Shell => {
            p.append(&label("Shell command", &["km-section"]));
            let e = entry(ui, &f.shell, "e.g. grim -g \"$(slurp)\" - | wl-copy", |f, t| f.shell = t);
            e.set_widget_name(widgets::DEFAULT_FOCUS);
            p.append(&e);
            p.append(&label(
                "Runs through `sh -c`, so pipes, variables and redirection work. Only use commands you trust.",
                &["km-small", "km-muted"],
            ));
        }
        Mode::Niri => {
            let ids = parametric();
            let labels: Vec<String> =
                ids.iter().map(|i| format!("{} — {}", catalog::ACTIONS[*i].category, catalog::ACTIONS[*i].label)).collect();
            let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
            let dd = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
            dd.set_enable_search(true);
            let pos = ids.iter().position(|i| *i == f.niri).unwrap_or(0);
            dd.set_selected(pos as u32);
            if !ids.contains(&f.niri) {
                update(ui, |s| {
                    s.niri = ids[0];
                    s.dirty = false;
                });
            }
            dd.set_widget_name(widgets::DEFAULT_FOCUS);
            let weak = Rc::downgrade(ui);
            let ids2 = ids.clone();
            dd.connect_selected_notify(move |dd| {
                if let Some(ui) = weak.upgrade() {
                    let i = ids2[dd.selected() as usize];
                    update(&ui, |f| {
                        f.niri = i;
                        f.values.clear();
                    });
                    ui.render_panel();
                }
            });
            p.append(&label("Action", &["km-section"]));
            p.append(&dd);
            let def = &catalog::ACTIONS[ids[pos]];
            for (i, spec) in def.params.iter().enumerate() {
                let hint = match spec.kind {
                    ParamKind::Int { min, max } => format!("{min}–{max}"),
                    ParamKind::Workspace => "index (1, 2…) or workspace name".into(),
                    ParamKind::Size => "50%, +10%, -10% or pixels".into(),
                    ParamKind::Bool => "true or false".into(),
                    ParamKind::Choice(o) => o.join(" / "),
                    _ => String::new(),
                };
                let hint = match spec.default {
                    Some(d) => format!("{hint} (default {d})"),
                    None if !spec.required => format!("{hint} (optional)"),
                    None => hint,
                };
                p.append(&label(spec.label, &["km-section"]));
                let e = gtk::Entry::new();
                e.set_text(f.values.get(i).map(String::as_str).unwrap_or(""));
                e.set_placeholder_text(Some(&hint));
                let weak = Rc::downgrade(ui);
                e.connect_changed(move |e| {
                    if let Some(ui) = weak.upgrade() {
                        let t = e.text().to_string();
                        update(&ui, |f| {
                            if f.values.len() <= i {
                                f.values.resize(i + 1, String::new());
                            }
                            f.values[i] = t;
                        });
                    }
                });
                p.append(&e);
            }
        }
    }
    p.append(&preview);

    let row = widgets::button_row();
    let cancel = button("Cancel", &[]);
    let weak = Rc::downgrade(ui);
    cancel.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            request_close(&ui);
        }
    });
    let save = button("Save", &["km-primary"]);
    let weak = Rc::downgrade(ui);
    save.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            save_form(&ui);
        }
    });
    row.append(&cancel);
    row.append(&save);
    p.append(&row);

    // Live preview of exactly what will be written.
    let refresh = {
        let weak = Rc::downgrade(ui);
        let (preview, save) = (preview.clone(), save.clone());
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let f = ui.st.borrow().draft.as_ref().and_then(|d| d.form.clone()).unwrap_or_default();
            match payload(&f) {
                Ok(pl) => {
                    preview.set_text(&format!("Will write: {}", pl.to_kdl()));
                    preview.remove_css_class("km-error-text");
                    save.set_sensitive(true);
                }
                Err(e) => {
                    preview.set_text(&e);
                    preview.add_css_class("km-error-text");
                    save.set_sensitive(false);
                }
            }
        }
    };
    refresh();
    let refresh = Rc::new(refresh);
    connect_all_entries(p.upcast_ref(), &refresh);
    p
}

fn connect_all_entries(w: &gtk::Widget, refresh: &Rc<impl Fn() + 'static>) {
    if let Some(e) = w.downcast_ref::<gtk::Entry>() {
        let r = refresh.clone();
        // Runs after the state update handler connected earlier.
        e.connect_changed(move |_| r());
    }
    let mut c = w.first_child();
    while let Some(ch) = c {
        connect_all_entries(&ch, refresh);
        c = ch.next_sibling();
    }
}

fn save_form(ui: &Rc<Ui>) {
    let Some(d) = ui.st.borrow().draft.clone() else { return };
    let Some(f) = d.form.clone() else { return };
    let pl = match payload(&f) {
        Ok(p) => p,
        Err(e) => return ui.set_draft_error(e),
    };
    let canonical = pl.canonical();
    let name = f.name.trim().to_string();
    let category = Some(f.category.trim().to_string()).filter(|c| !c.is_empty());
    {
        let mut st = ui.st.borrow_mut();
        let exists = st.model.find_action(&canonical).is_some();
        if exists {
            if !name.is_empty() || category.is_some() {
                let name =
                    if name.is_empty() { st.model.find_action(&canonical).map(|a| a.name.clone()).unwrap_or_default() } else { name };
                st.store.labels.insert(canonical.clone(), Label { name, category });
            }
        } else {
            let id = st.store.new_id();
            let name = if name.is_empty() { pl.derived_name() } else { name };
            st.store.custom_actions.push(CustomAction { id, name, category, payload: pl });
        }
    }
    ui.save_store();
    ui.rebuild_model();
    if let Some(d) = ui.st.borrow_mut().draft.as_mut() {
        d.chosen = Some(canonical.clone());
        d.form = None;
        d.return_to = View::Browser;
        d.tab = super::Tab::All;
    }
    ui.st.borrow_mut().view = View::Browser;
    let combo = d.combo.clone();
    ui.assign(&combo, &canonical, &keymap_core::plan::Others::Ask, false);
    if ui.st.borrow().view == View::Browser {
        ui.render_panel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_splitting() {
        assert_eq!(split_argv("foot -e htop").unwrap(), ["foot", "-e", "htop"]);
        assert_eq!(split_argv(r#"sh "a b" 'c d' e\ f"#).unwrap(), ["sh", "a b", "c d", "e f"]);
        assert_eq!(split_argv(r#""" x"#).unwrap(), ["", "x"]);
        assert_eq!(split_argv("echo $HOME ~").unwrap(), ["echo", "$HOME", "~"]);
        assert!(split_argv("'open").is_err());
    }

    #[test]
    fn niri_payloads() {
        let i = catalog::ACTIONS.iter().position(|a| a.name == "focus-workspace").unwrap();
        let mut f = FormState { mode: Mode::Niri, niri: i, values: vec!["3".into()], ..Default::default() };
        assert_eq!(payload(&f).unwrap().to_kdl(), "focus-workspace 3");
        f.values = vec!["chat".into()];
        assert_eq!(payload(&f).unwrap().to_kdl(), r#"focus-workspace "chat""#);
        f.values = vec![];
        assert!(payload(&f).is_err());
    }
}
