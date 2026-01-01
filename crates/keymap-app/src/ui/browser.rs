//! Action browser: what this combination does, and every action available.

use super::widgets::{self, banner, button, label};
use super::{Pending, Tab, Ui, View};
use gtk::prelude::*;
use gtk::{gio, glib};
use keymap_core::model::{ActionOrigin, BindStatus};
use keymap_core::plan;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const CATEGORY_ORDER: &[&str] = &[
    "Applications",
    "Commands",
    "Session",
    "Screenshots",
    "Windows",
    "Floating",
    "Columns",
    "Sizing",
    "Workspaces",
    "Monitors",
    "Keyboard",
    "Overview",
    "Screencast",
    "Debug",
];

fn category_rank(c: &str) -> usize {
    CATEGORY_ORDER.iter().position(|x| *x == c).unwrap_or(CATEGORY_ORDER.len())
}

/// One action of the "All actions" list, prepared once per model.
struct Item {
    canonical: String,
    name: String,
    category: String,
    origin: Option<&'static str>,
    shortcuts: Vec<String>,
    text: String,
    unsupported: Option<String>,
    places: Vec<String>,
    hay: String,
}

/// The widgets of one on-screen row. The list view creates only enough of
/// these to fill the viewport and rebinds them to other items as it scrolls.
#[derive(Clone)]
struct RowWidgets {
    root: gtk::Box,
    name: gtk::Label,
    origin: gtk::Label,
    current: gtk::Label,
    chips: gtk::FlowBox,
    details: gtk::Box,
}

/// What the kept list needs to know about the combination being edited.
#[derive(Default)]
struct Context {
    current: Option<String>,
    chosen: Option<String>,
    save: Option<gtk::Button>,
    read_only: bool,
    /// Rows on screen by item index, so a choice updates them in place.
    bound: HashMap<usize, (gtk::ListItem, RowWidgets)>,
}

impl Context {
    fn apply(&self, item: &Item, li: &gtk::ListItem, rw: &RowWidgets) {
        let current = self.current.as_deref() == Some(item.canonical.as_str());
        let chosen = self.chosen.as_deref() == Some(item.canonical.as_str());
        rw.current.set_visible(current);
        let enabled = item.unsupported.is_none() || current;
        li.set_activatable(enabled);
        rw.root.set_sensitive(enabled);
        if chosen {
            rw.root.add_css_class("km-chosen");
        } else {
            rw.root.remove_css_class("km-chosen");
        }
        rw.details.set_visible(chosen);
    }

    fn apply_bound(&self, items: &[Item]) {
        for (i, (li, rw)) in &self.bound {
            self.apply(&items[*i], li, rw);
        }
    }
}

/// The action browser panel, kept across openings. The "All actions" list is
/// a `gtk::ListView` over a model prepared once per configuration: only the
/// rows in view exist as widgets, so opening the browser does not lay out
/// every action. Each opening rebuilds just the header, the (short) "This
/// combination" list and the footer.
pub struct Browser {
    pub root: gtk::Box,
    pub key: (u64, i32, i32),
    top: gtk::Box,
    this: gtk::Box,
    all: gtk::Box,
    search: gtk::SearchEntry,
    list: gtk::ListView,
    /// The filtered actions, of which the list shows a growing prefix while
    /// it fills (see `attach`).
    slice: gtk::SliceListModel,
    /// Where to scroll once the model is attached.
    scroll_to: Rc<std::cell::Cell<Option<u32>>>,
    filter: gtk::CustomFilter,
    query: Rc<RefCell<String>>,
    items: Rc<Vec<Item>>,
    ctx: Rc<RefCell<Context>>,
    footer: gtk::Box,
}

/// Cache key: the model generation and the panel's size.
pub fn key(ui: &Ui) -> (u64, i32, i32) {
    let (mw, mh) = ui.monitor_size();
    (ui.st.borrow().generation, mw as i32, mh as i32)
}

fn index_of(obj: Option<glib::Object>) -> Option<usize> {
    obj.and_downcast::<glib::BoxedAnyObject>().map(|o| *o.borrow::<usize>())
}

fn prepare_items(ui: &Ui) -> Vec<Item> {
    let st = ui.st.borrow();
    let model = &st.model;
    let mut order: Vec<usize> = (0..model.actions.len()).collect();
    order.sort_by(|a, b| {
        let (x, y) = (&model.actions[*a], &model.actions[*b]);
        (category_rank(&x.category), &x.category, x.bindings.is_empty(), x.name.to_lowercase()).cmp(&(
            category_rank(&y.category),
            &y.category,
            y.bindings.is_empty(),
            y.name.to_lowercase(),
        ))
    });
    order
        .into_iter()
        .map(|i| {
            let a = &model.actions[i];
            let shortcuts: Vec<String> = model.shortcuts(a.id).iter().filter_map(|b| b.combo.as_ref().map(|c| c.label())).collect();
            let text = a.text();
            Item {
                canonical: a.canonical.clone(),
                hay: format!("{} {} {} {}", a.name, a.category, text, shortcuts.join(" ")).to_lowercase(),
                name: a.name.clone(),
                category: a.category.clone(),
                origin: match &a.origin {
                    ActionOrigin::Config => None,
                    ActionOrigin::Custom { .. } => Some("saved, not bound"),
                    ActionOrigin::Builtin => Some("Niri action"),
                },
                shortcuts,
                text,
                unsupported: a.unsupported.clone(),
                places: a
                    .bindings
                    .iter()
                    .map(|b| {
                        let b = &model.bindings[*b];
                        format!("{} — {}:{}", b.key_text, short_path(&b.path), b.line)
                    })
                    .collect(),
            }
        })
        .collect()
}

fn row_widgets() -> RowWidgets {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 3);
    root.add_css_class("km-item");
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = label("", &["km-row-name"]);
    name.set_hexpand(true);
    top.append(&name);
    let origin = label("", &["km-muted", "km-small"]);
    top.append(&origin);
    let current = label("current", &["km-mint", "km-small"]);
    top.append(&current);
    root.append(&top);
    // Chips wrap inside the row instead of widening it. The flow box is
    // display-only: clicks fall through to the row, focus skips it.
    let chips = gtk::FlowBox::new();
    chips.set_selection_mode(gtk::SelectionMode::None);
    chips.set_column_spacing(10);
    chips.set_row_spacing(4);
    chips.set_max_children_per_line(6);
    chips.set_halign(gtk::Align::Start);
    chips.set_can_target(false);
    chips.set_can_focus(false);
    root.append(&chips);
    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    details.add_css_class("km-details");
    root.append(&details);
    RowWidgets { root, name, origin, current, chips, details }
}

fn fill_row(item: &Item, rw: &RowWidgets) {
    rw.name.set_text(&item.name);
    rw.origin.set_text(item.origin.unwrap_or(""));
    rw.origin.set_visible(item.origin.is_some());
    rw.chips.remove_all();
    for s in item.shortcuts.iter().take(5) {
        rw.chips.append(&widgets::caps_from(&s.split(" + ").map(String::from).collect::<Vec<_>>()));
    }
    if item.shortcuts.len() > 5 {
        rw.chips.append(&label(&format!("+{}", item.shortcuts.len() - 5), &["km-muted", "km-small"]));
    }
    let mut c = rw.chips.first_child();
    while let Some(w) = c {
        w.set_can_focus(false);
        w.set_halign(gtk::Align::Start);
        c = w.next_sibling();
    }
    rw.chips.set_visible(!item.shortcuts.is_empty());
    clear(&rw.details);
    let t = label(&item.text, &["km-mono"]);
    t.set_selectable(true);
    rw.details.append(&t);
    if let Some(u) = &item.unsupported {
        rw.details.append(&label(u, &["km-small", "km-error-text"]));
    }
    for p in &item.places {
        rw.details.append(&label(p, &["km-small", "km-muted"]));
    }
}

pub fn build(ui: &Rc<Ui>) -> Browser {
    let root = widgets::panel(ui, 720.0, Some(0.72));
    let top = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let this = gtk::Box::new(gtk::Orientation::Vertical, 12);
    this.set_vexpand(true);
    let all = gtk::Box::new(gtk::Orientation::Vertical, 12);
    all.set_vexpand(true);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for w in [&top, &this, &all, &footer] {
        root.append(w);
    }

    let items = Rc::new(prepare_items(ui));
    let ctx: Rc<RefCell<Context>> = Rc::default();
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    store.extend_from_slice(&(0..items.len()).map(glib::BoxedAnyObject::new).collect::<Vec<_>>());
    // Items are already in display order; the section sorter only tells the
    // view where each category starts, for its headers.
    let sorted = gtk::SortListModel::new(Some(store), None::<gtk::Sorter>);
    let it = items.clone();
    sorted.set_section_sorter(Some(&gtk::CustomSorter::new(move |a, b| {
        let (Some(a), Some(b)) = (index_of(Some(a.clone())), index_of(Some(b.clone()))) else { return gtk::Ordering::Equal };
        let (x, y) = (&it[a].category, &it[b].category);
        (category_rank(x), x).cmp(&(category_rank(y), y)).into()
    })));
    let query: Rc<RefCell<String>> = Rc::default();
    let (it, q) = (items.clone(), query.clone());
    let filter = gtk::CustomFilter::new(move |o| {
        let q = q.borrow();
        q.is_empty() || index_of(Some(o.clone())).is_some_and(|i| q.split_whitespace().all(|w| it[i].hay.contains(w)))
    });
    let filtered = gtk::FilterListModel::new(Some(sorted), Some(filter.clone()));

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, li| {
        let Some(li) = li.downcast_ref::<gtk::ListItem>() else { return };
        let rw = row_widgets();
        li.set_child(Some(&rw.root));
        unsafe { li.set_data("km-row", rw) };
    });
    let (it, c) = (items.clone(), ctx.clone());
    factory.connect_bind(move |_, li| {
        let Some(li) = li.downcast_ref::<gtk::ListItem>() else { return };
        let (Some(i), Some(rw)) = (index_of(li.item()), unsafe { li.data::<RowWidgets>("km-row").map(|p| p.as_ref().clone()) }) else {
            return;
        };
        fill_row(&it[i], &rw);
        let mut ctx = c.borrow_mut();
        ctx.apply(&it[i], li, &rw);
        ctx.bound.insert(i, (li.clone(), rw));
    });
    let c = ctx.clone();
    factory.connect_unbind(move |_, li| {
        let Some(li) = li.downcast_ref::<gtk::ListItem>() else { return };
        c.borrow_mut().bound.retain(|_, (l, _)| l != li);
    });
    let headers = gtk::SignalListItemFactory::new();
    headers.connect_setup(|_, h| {
        let Some(h) = h.downcast_ref::<gtk::ListHeader>() else { return };
        let l = label("", &["km-section"]);
        l.set_margin_top(8);
        l.set_margin_bottom(4);
        h.set_child(Some(&l));
    });
    let it = items.clone();
    headers.connect_bind(move |_, h| {
        let Some(h) = h.downcast_ref::<gtk::ListHeader>() else { return };
        if let (Some(i), Some(l)) = (index_of(h.item()), h.child().and_downcast::<gtk::Label>()) {
            l.set_text(&it[i].category);
        }
    });
    let slice = gtk::SliceListModel::new(Some(filtered.clone()), 0, 0);
    let list = gtk::ListView::new(None::<gtk::NoSelection>, Some(factory));
    list.set_header_factory(Some(&headers));
    list.add_css_class("km-list");
    list.set_single_click_activate(true);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search actions, commands or categories"));
    search.set_widget_name(widgets::DEFAULT_FOCUS);
    search.set_search_delay(60);
    all.append(&search);
    all.append(&widgets::scroller(&list));

    // Choosing an action marks it and enables Save; the model is unchanged.
    let weak = Rc::downgrade(ui);
    let (it, c, fm) = (items.clone(), ctx.clone(), filtered.clone());
    let choose = Rc::new(move |pos: u32| -> bool {
        let Some(i) = index_of(fm.item(pos)) else { return false };
        let item = &it[i];
        let mut ctx = c.borrow_mut();
        let current = ctx.current.as_deref() == Some(item.canonical.as_str());
        if item.unsupported.is_some() && !current {
            return false;
        }
        ctx.chosen = Some(item.canonical.clone());
        ctx.apply_bound(&it);
        if let Some(save) = &ctx.save {
            save.set_sensitive(!ctx.read_only && !current);
        }
        drop(ctx);
        if let Some(ui) = weak.upgrade()
            && let Some(d) = ui.st.borrow_mut().draft.as_mut()
        {
            d.chosen = Some(item.canonical.clone());
            d.info = None;
        }
        true
    });
    let ch = choose.clone();
    list.connect_activate(move |_, pos| {
        ch(pos);
    });

    let weak = Rc::downgrade(ui);
    let (f2, q2, s2) = (filter.clone(), query.clone(), slice.clone());
    search.connect_search_changed(move |e| {
        let text = e.text().to_string();
        if *q2.borrow() != text.to_lowercase() {
            *q2.borrow_mut() = text.to_lowercase();
            f2.changed(gtk::FilterChange::Different);
            // Matches must not hide behind a list that is still filling.
            s2.set_size(u32::MAX);
        }
        if let Some(ui) = weak.upgrade()
            && let Some(d) = ui.st.borrow_mut().draft.as_mut()
        {
            d.search = text;
        }
    });
    let l2 = list.clone();
    search.connect_activate(move |_| {
        // Enter in the search field picks the first available match.
        if let Some(pos) = (0..filtered.n_items()).find(|p| choose(*p)) {
            l2.scroll_to(pos, gtk::ListScrollFlags::FOCUS, None);
        }
    });

    Browser {
        root,
        key: key(ui),
        top,
        this,
        all,
        search,
        list,
        slice: slice.clone(),
        scroll_to: Rc::default(),
        filter,
        query,
        items,
        ctx,
        footer,
    }
}

/// Rows added to the list per frame while it fills.
const ROWS_PER_FRAME: u32 = 12;

/// Give the list its rows once it has a size, a few per frame. GTK's list
/// view creates a widget for every item of a list this short (it keeps up to
/// 200), and styling and measuring ~150 action rows costs about 300 ms here;
/// done at once, that would stall input for a frame that long. Growing the
/// list over consecutive frames keeps every frame short. The kept panel is
/// filled in the background (see `Ui::warm_panels`), so this normally finishes
/// before the browser is opened; a search shows every match at once.
pub fn attach(br: &Browser) {
    if br.list.model().is_some() {
        return;
    }
    let (slice, target) = (br.slice.clone(), br.scroll_to.clone());
    let selection = gtk::NoSelection::new(Some(slice.clone()));
    br.list.add_tick_callback(move |list, _| {
        if list.model().is_none() {
            if !list.is_mapped() || list.height() == 0 {
                return glib::ControlFlow::Continue;
            }
            super::trace("browser: filling the action list");
            list.set_model(Some(&selection));
        }
        let total = slice.model().map(|m| m.n_items()).unwrap_or(0);
        if slice.size() < total {
            slice.set_size(slice.size().saturating_add(ROWS_PER_FRAME));
            return glib::ControlFlow::Continue;
        }
        super::trace("browser: action list ready");
        if let Some(pos) = target.take() {
            list.scroll_to(pos, gtk::ListScrollFlags::NONE, None);
        }
        glib::ControlFlow::Break
    });
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// Fill the kept panel for the current draft.
pub fn refresh(ui: &Rc<Ui>, br: &Browser) {
    clear(&br.top);
    clear(&br.this);
    clear(&br.footer);
    let Some(draft) = ui.st.borrow().draft.clone() else {
        br.top.append(&label("Select keys first.", &[]));
        br.all.set_visible(false);
        return;
    };
    let combo = draft.combo.clone();
    let st = ui.st.borrow();
    let model = &st.model;
    let current = model.effective_for(&combo).and_then(|b| model.action_of(b)).map(|a| a.canonical.clone());
    let decls: Vec<usize> = model.bindings.iter().filter(|b| b.combo.as_ref() == Some(&combo)).map(|b| b.id).collect();
    let p = &br.top;

    // Header: title, combination and what it currently does, and a way back.
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    title.append(&label("Choose an action", &["km-title"]));
    title.append(&widgets::spacer());
    let back = button("Esc  Back", &["flat"]);
    back.set_tooltip_text(Some("Back to the keyboard; the selection is kept"));
    let weak = Rc::downgrade(ui);
    back.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.back();
        }
    });
    title.append(&back);
    p.append(&title);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    head.append(&widgets::caps(&combo));
    let now = match model.effective_for(&combo).and_then(|b| model.action_of(b)) {
        Some(a) => format!("runs “{}”", a.name),
        None => "is unassigned".into(),
    };
    head.append(&label(&now, &["km-muted"]));
    p.append(&head);
    if let Some(r) = &model.read_only {
        p.append(&banner(&format!("Read-only: {r}"), true));
    }
    if st.changed_on_disk {
        p.append(&banner("The configuration changed on disk and was reloaded. Your selection was kept.", false));
    }
    if let Some(w) = model.bindings.iter().find(|b| b.combo.as_ref() == Some(&combo)).and_then(|b| b.warning.clone()) {
        p.append(&banner(&w, false));
    }
    if let Some(e) = &draft.error {
        p.append(&banner(e, true));
    }
    if let Some(i) = &draft.info {
        p.append(&label(i, &["km-mint", "km-small"]));
    }

    // Tabs.
    let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    // Counts the effective assignment (0 or 1); replaced declarations are details.
    let n = usize::from(model.effective_for(&combo).is_some());
    let t_this = gtk::ToggleButton::with_label(&format!("This combination ({n})"));
    let t_all = gtk::ToggleButton::with_label("All actions");
    t_all.set_group(Some(&t_this));
    t_this.add_css_class("km-tab");
    t_all.add_css_class("km-tab");
    t_this.set_active(draft.tab == Tab::This);
    t_all.set_active(draft.tab == Tab::All);
    tabs.append(&t_this);
    tabs.append(&t_all);
    p.append(&tabs);
    for (btn, tab) in [(&t_this, Tab::This), (&t_all, Tab::All)] {
        let weak = Rc::downgrade(ui);
        btn.connect_toggled(move |b| {
            if !b.is_active() {
                return;
            }
            if let Some(ui) = weak.upgrade() {
                let changed = ui.st.borrow_mut().draft.as_mut().map(|d| std::mem::replace(&mut d.tab, tab) != tab);
                if changed == Some(true) {
                    ui.render_panel();
                }
            }
        });
    }

    br.this.set_visible(draft.tab == Tab::This);
    br.all.set_visible(draft.tab == Tab::All);
    if draft.tab == Tab::This {
        let list = gtk::ListBox::new();
        list.add_css_class("km-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        if decls.is_empty() {
            br.this.append(&label("Nothing uses this combination yet. Pick an action under “All actions”.", &["km-muted"]));
        }
        let mut ordered = decls.clone();
        ordered.sort_by_key(|id| !model.bindings[*id].is_effective());
        let mut noted = false;
        for id in &ordered {
            let b = &model.bindings[*id];
            if !b.is_effective() && !noted {
                noted = true;
                let h = label("Not active (kept in your config for reference)", &["km-section"]);
                h.set_margin_top(8);
                let hr = gtk::ListBoxRow::new();
                hr.set_activatable(false);
                hr.set_selectable(false);
                hr.set_can_focus(false);
                hr.add_css_class("km-header-row");
                hr.set_child(Some(&h));
                list.append(&hr);
            }
            let row = gtk::ListBoxRow::new();
            row.set_activatable(false);
            if !b.is_effective() {
                row.set_opacity(0.7);
            }
            let v = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let name = model.action_of(b).map(|a| a.name.clone()).unwrap_or_else(|| "(unreadable action)".into());
            let n = label(&name, &["km-row-name"]);
            n.set_hexpand(true);
            top.append(&n);
            let status = match &b.status {
                BindStatus::Effective => "active".to_string(),
                BindStatus::Overridden { by } => {
                    format!("replaced by {}:{}", file_name(&model.bindings[*by].path), model.bindings[*by].line)
                }
                BindStatus::Shadowed { by } => format!("never fires: {} comes first", model.bindings[*by].key_text),
                BindStatus::Duplicate { .. } => "duplicate — Niri rejects this".into(),
                BindStatus::Invalid(e) => format!("invalid: {e}"),
            };
            let sl = label(&status, &["km-muted", "km-small"]);
            sl.set_wrap(false);
            top.append(&sl);
            if b.is_effective() && model.read_only.is_none() {
                let rm = button("Remove", &["flat"]);
                let weak = Rc::downgrade(ui);
                let bid = *id;
                let everywhere = decls.len() > 1;
                rm.connect_clicked(move |_| {
                    if let Some(ui) = weak.upgrade() {
                        request_remove(&ui, bid, false);
                    }
                });
                top.append(&rm);
                if everywhere {
                    let all = button("Remove all", &["flat"]);
                    let weak = Rc::downgrade(ui);
                    all.set_tooltip_text(Some("Remove every declaration of this combination"));
                    all.connect_clicked(move |_| {
                        if let Some(ui) = weak.upgrade() {
                            request_remove(&ui, bid, true);
                        }
                    });
                    top.append(&all);
                }
            }
            v.append(&top);
            let src = format!(
                "{}:{}  {} {{ {} }}",
                short_path(&b.path),
                b.line,
                b.key_text,
                model.action_of(b).map(|a| a.text()).unwrap_or_default()
            );
            let s = label(&src, &["km-mono", "km-muted"]);
            s.set_selectable(true);
            v.append(&s);
            if let Some(a) = model.action_of(b) {
                let others: Vec<String> =
                    model.shortcuts(a.id).iter().filter(|o| o.id != *id).filter_map(|o| o.combo.as_ref().map(|c| c.label())).collect();
                if !others.is_empty() {
                    v.append(&label(&format!("Also on: {}", others.join(", ")), &["km-small", "km-muted"]));
                }
            }
            row.set_child(Some(&v));
            list.append(&row);
        }
        br.this.append(&widgets::scroller(&list));
    } else {
        // Update the kept list for this combination.
        {
            let mut ctx = br.ctx.borrow_mut();
            ctx.current = current.clone();
            ctx.chosen = draft.chosen.clone();
            ctx.apply_bound(&br.items);
        }
        if br.search.text() != draft.search {
            br.search.set_text(&draft.search);
        }
        // Filter now rather than after the entry's typing delay.
        let q = draft.search.to_lowercase();
        if *br.query.borrow() != q {
            *br.query.borrow_mut() = q;
            br.filter.changed(gtk::FilterChange::Different);
        }
        if draft.search.is_empty() && !br.items.is_empty() {
            // Start at the top, or at the action this combination runs.
            let pos = draft.chosen.as_ref().and_then(|c| br.items.iter().position(|i| &i.canonical == c));
            if br.list.model().is_none() || br.slice.size() < br.slice.model().map(|m| m.n_items()).unwrap_or(0) {
                br.scroll_to.set(pos.map(|p| p as u32));
            } else if let Some(pos) = pos {
                br.list.scroll_to(pos as u32, gtk::ListScrollFlags::NONE, None);
            } else if let Some(adj) = br.list.vadjustment() {
                // The very top, including the first category header.
                adj.set_value(adj.lower());
            }
        }
        attach(br);
    }

    // Footer.
    let row = &br.footer;
    let new = button("New action", &[]);
    new.set_tooltip_text(Some("Ctrl+N"));
    new.set_sensitive(model.read_only.is_none());
    let weak = Rc::downgrade(ui);
    new.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            super::form::open(&ui);
        }
    });
    row.append(&new);
    row.append(&widgets::spacer());
    let cancel = button("Cancel", &[]);
    let weak = Rc::downgrade(ui);
    cancel.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.back();
        }
    });
    row.append(&cancel);
    let save = button("Save", &["km-primary"]);
    let ch = draft.chosen.clone();
    save.set_sensitive(model.read_only.is_none() && draft.tab == Tab::All && ch.is_some() && ch != current);
    let weak = Rc::downgrade(ui);
    save.connect_clicked(move |_| {
        if let Some(ui) = weak.upgrade() {
            ui.save_draft(false);
        }
    });
    row.append(&save);
    let mut ctx = br.ctx.borrow_mut();
    ctx.current = current;
    ctx.chosen = draft.chosen.clone();
    ctx.save = Some(save);
    ctx.read_only = model.read_only.is_some();
}

fn request_remove(ui: &Rc<Ui>, binding: usize, everywhere: bool) {
    let r = {
        let st = ui.st.borrow();
        let combo = st.model.bindings[binding].combo.clone();
        if everywhere { Ok(None) } else { plan::remove(&st.model, &st.store, binding).map(|r| r.reveals).map_err(|e| e.0) }
            .map(|rev| (combo, rev))
    };
    match r {
        Ok((Some(combo), reveals)) => {
            if let Some(d) = ui.st.borrow_mut().draft.as_mut() {
                d.pending = Some(Pending::Remove { binding, combo, reveals, everywhere });
                d.return_to = View::Browser;
            }
            ui.set_view(View::Confirm);
        }
        Ok((None, _)) => ui.set_draft_error("This declaration has no valid key.".into()),
        Err(e) => ui.set_draft_error(e),
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Paths under the home directory are shown with `~`.
pub fn short_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    match std::env::var("HOME") {
        Ok(h) if !h.is_empty() && s.starts_with(&h) => format!("~{}", &s[h.len()..]),
        _ => s,
    }
}
