//! Change plans: minimal byte-span edits against a specific snapshot.
//!
//! A plan never touches disk. Each plan is verified by reloading the
//! candidate texts in memory and checking that the intended combination ends
//! up with the intended effective action (or none), using the same merge and
//! first-match rules as `Model`.

use crate::config::{self, Overrides};
use crate::kdl::Span;
use crate::keys::Combo;
use crate::model::{BindStatus, Binding, Model};
use crate::store::Store;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

pub fn apply_edits(text: &str, edits: &[TextEdit]) -> String {
    let mut edits = edits.to_vec();
    edits.sort_by(|a, b| b.start.cmp(&a.start).then(b.end.cmp(&a.end)));
    let mut out = text.to_string();
    for e in edits {
        out.replace_range(e.start..e.end, &e.text);
    }
    out
}

#[derive(Debug, Clone, Serialize)]
pub struct FileChange {
    /// Path as Niri resolves it.
    pub path: PathBuf,
    /// Symlink target that is actually rewritten.
    pub real_path: PathBuf,
    pub expected_hash: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub changes: Vec<FileChange>,
    pub description: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Replacement {
    pub combo: Combo,
    pub binding: usize,
    pub current: String,
    pub proposed: String,
    /// The action's own shortcut that the same save removes, if chosen.
    pub also_removes: Option<ExistingShortcut>,
}

/// What to do with the shortcuts the target action already has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Others {
    /// Ask first (returns `HasShortcuts` when there are any).
    Ask,
    /// Keep them and add the new shortcut too.
    Keep,
    /// Remove this one (identified by combination, so it survives reloads).
    Replace(Combo),
}

/// One active shortcut of the action being assigned.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExistingShortcut {
    pub combo: Combo,
    /// Why Keymap cannot remove it, if it cannot.
    pub blocked: Option<String>,
    /// What the combination runs once this declaration is removed.
    pub reveals: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Existing {
    pub action: String,
    pub combo: Combo,
    pub shortcuts: Vec<ExistingShortcut>,
}

#[derive(Debug, Clone, Serialize)]
pub enum AssignResult {
    /// Nothing to write (e.g. the combination already runs this action).
    NoOp(String),
    /// The action already has other shortcuts; ask whether to replace one.
    HasShortcuts(Existing),
    /// The combination runs another action; ask before replacing.
    NeedsConfirm(Replacement),
    Ready(Plan),
}

#[derive(Debug, Clone, Serialize)]
pub struct RemovePlan {
    pub plan: Plan,
    /// What the combination will do after the removal, if anything.
    pub reveals: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlanError(pub String);

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PlanError {}

fn err<T>(s: impl Into<String>) -> Result<T, PlanError> {
    Err(PlanError(s.into()))
}

/// The action an assignment points to: its KDL text and canonical identity.
#[derive(Debug, Clone)]
pub struct Target {
    pub text: String,
    pub canonical: String,
    pub name: String,
    pub is_spawn: bool,
}

impl Target {
    pub fn of_action(model: &Model, action: usize) -> Target {
        let a = &model.actions[action];
        Target { text: a.text(), canonical: a.canonical.clone(), name: a.name.clone(), is_spawn: a.payload.is_spawn() }
    }
}

struct Edits<'m> {
    model: &'m Model,
    per_file: Vec<(usize, Vec<TextEdit>)>,
}

impl<'m> Edits<'m> {
    fn new(model: &'m Model) -> Self {
        Edits { model, per_file: Vec::new() }
    }

    fn push(&mut self, file: usize, e: TextEdit) -> Result<(), PlanError> {
        let f = &self.model.config.files[file];
        if !f.writable {
            return err(format!(
                "{} is not writable by you (owned by another user or read-only); Keymap does not escalate privileges.",
                f.path.display()
            ));
        }
        if self.model.config.files.iter().filter(|o| o.path == f.path).count() > 1 {
            return err(format!("{} is included more than once; edit it by hand.", f.path.display()));
        }
        match self.per_file.iter_mut().find(|(i, _)| *i == file) {
            Some((_, v)) => v.push(e),
            None => self.per_file.push((file, vec![e])),
        }
        Ok(())
    }

    fn finish(self, description: String, notes: Vec<String>) -> Plan {
        let changes = self
            .per_file
            .into_iter()
            .map(|(i, edits)| {
                let f = &self.model.config.files[i];
                FileChange {
                    path: f.path.clone(),
                    real_path: f.real_path.clone(),
                    expected_hash: f.hash.clone(),
                    before: f.text.clone(),
                    after: apply_edits(&f.text, &edits),
                }
            })
            .collect();
        Plan { changes, description, notes }
    }
}

fn line_start(text: &str, pos: usize) -> usize {
    text[..pos].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, pos: usize) -> usize {
    text[pos..].find('\n').map_or(text.len(), |i| pos + i + 1)
}

fn indent_of(text: &str, pos: usize) -> String {
    let ls = line_start(text, pos);
    text[ls..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// Edit that deletes a node; removes its whole line when the line holds
/// nothing else but whitespace and an optional trailing `//` comment.
fn delete_node(text: &str, full: Span) -> TextEdit {
    let ls = line_start(text, full.start);
    let le = line_end(text, full.end);
    let before = &text[ls..full.start];
    let after = text[full.end..le].trim();
    if before.trim().is_empty() && (after.is_empty() || after.starts_with("//")) {
        TextEdit { start: ls, end: le, text: String::new() }
    } else {
        let mut start = full.start;
        while start > ls && matches!(text.as_bytes()[start - 1], b' ' | b'\t') {
            start -= 1;
        }
        TextEdit { start, end: full.end, text: String::new() }
    }
}

/// Edit that removes one property entry together with its leading space.
fn delete_entry(text: &str, span: Span) -> TextEdit {
    let mut start = span.start;
    while start > 0 && matches!(text.as_bytes()[start - 1], b' ' | b'\t') {
        start -= 1;
    }
    TextEdit { start, end: span.end, text: String::new() }
}

fn bind_line(key: &str, action_text: &str) -> String {
    format!("{key} {{ {action_text}; }}")
}

/// Insert a bind line at the end of a `binds` section.
/// Add a bind line for a combination nothing else declares, so any section
/// works: the user's main binds block (the one with the most bindings), or a
/// new `binds {}` block.
fn push_new_bind(model: &Model, edits: &mut Edits, line: &str) -> Result<(), PlanError> {
    let main = (0..model.sections.len())
        .filter(|s| writable_unique(model, model.sections[*s].file))
        .max_by_key(|s| (model.bindings.iter().filter(|b| b.section == *s).count(), *s));
    match main {
        Some(s) => {
            let (file, e) = insert_into_section(model, s, line);
            edits.push(file, e)
        }
        None => {
            if model.config.files.is_empty() || !writable_unique(model, 0) {
                return err("No writable binds section or configuration file is available.");
            }
            let text = &model.config.files[0].text;
            let sep = if text.ends_with('\n') || text.is_empty() { "" } else { "\n" };
            edits.push(0, TextEdit { start: text.len(), end: text.len(), text: format!("{sep}\nbinds {{\n    {line}\n}}\n") })
        }
    }
}

/// Plan adding one complete bind line (key, properties and action) for a
/// combination that nothing declares yet.
pub fn add_bind_line(model: &Model, line: &str, description: String) -> Result<Plan, PlanError> {
    check_editable(model)?;
    let mut edits = Edits::new(model);
    push_new_bind(model, &mut edits, line)?;
    Ok(edits.finish(description, vec![]))
}

/// Plan changing only the key of binding `b` to `key`, keeping its
/// properties and action byte for byte.
pub fn rename_bind(model: &Model, b: &Binding, key: &str, description: String) -> Result<Plan, PlanError> {
    check_editable(model)?;
    if !writable_unique(model, b.file) {
        return err(format!("{} cannot be edited by Keymap.", b.path.display()));
    }
    let text = &model.config.files[b.file].text;
    let start = b.node_span.start;
    let end = start + b.key_text.len();
    if text.get(start..end) != Some(b.key_text.as_str()) {
        return err("The binding's key is not written plainly; change it by hand.");
    }
    let mut edits = Edits::new(model);
    edits.push(b.file, TextEdit { start, end, text: key.to_string() })?;
    Ok(edits.finish(description, vec![]))
}

fn insert_into_section(model: &Model, section: usize, line: &str) -> (usize, TextEdit) {
    let s = &model.sections[section];
    let text = &model.config.files[s.file].text;
    let close = s.block.end - 1;
    let last_child = model.bindings.iter().filter(|b| b.section == section).map(|b| b.node_span).next_back();
    let brace_indent = indent_of(text, close);
    let indent = match last_child {
        Some(sp) => indent_of(text, sp.start),
        None => format!("{brace_indent}    "),
    };
    let ls = line_start(text, close);
    let block_has_newline = text[s.block.start..close].contains('\n');
    let edit = if block_has_newline && text[ls..close].trim().is_empty() {
        TextEdit { start: ls, end: ls, text: format!("{indent}{line}\n") }
    } else {
        TextEdit { start: close, end: close, text: format!("\n{indent}{line}\n{brace_indent}") }
    };
    (s.file, edit)
}

fn writable_unique(model: &Model, file: usize) -> bool {
    let f = &model.config.files[file];
    f.writable && model.config.files.iter().filter(|o| o.path == f.path).count() == 1
}

/// Load the candidate configuration produced by a plan.
pub fn candidate(model: &Model, plan: &Plan, store: &Store) -> Model {
    let overrides: Overrides = plan.changes.iter().map(|c| (c.path.clone(), c.after.clone())).collect();
    Model::build(config::load_with(model.config.location.clone(), &overrides), store)
}

fn verify(model: &Model, plan: &Plan, store: &Store, combo: &Combo, expect: Option<&str>) -> Result<Model, PlanError> {
    let cand = candidate(model, plan, store);
    if let Some(d) = cand.config.diagnostics.iter().find(|d| d.error) {
        return err(format!("The change would make the configuration invalid: {}:{}: {}", d.path.display(), d.line, d.message));
    }
    let got = cand.effective_for(combo).and_then(|b| cand.action_of(b)).map(|a| a.canonical.as_str());
    if got != expect {
        return err(format!("The change would not take effect as intended for {}; another declaration takes precedence.", combo.label()));
    }
    Ok(cand)
}

fn check_editable(model: &Model) -> Result<(), PlanError> {
    match &model.read_only {
        Some(r) => err(r.clone()),
        None => Ok(()),
    }
}

/// Why an active binding cannot be deleted by Keymap, if it cannot.
fn removal_blocked(model: &Model, b: &Binding) -> Option<String> {
    (!writable_unique(model, b.file))
        .then(|| format!("Defined in {}, which Keymap cannot edit (Niri has no \"unbind\").", b.path.display()))
}

/// The target action's active shortcuts other than `combo`, as the user
/// would choose among them. Actions match by canonical identity (action and
/// arguments), never by display name.
pub fn existing_shortcuts(model: &Model, store: &Store, combo: &Combo, target: &Target) -> Vec<ExistingShortcut> {
    let Some(action) = model.find_action(&target.canonical) else { return Vec::new() };
    model
        .shortcuts(action.id)
        .into_iter()
        .filter(|b| b.combo.as_ref().is_some_and(|c| c != combo))
        .map(|b| {
            let c = b.combo.clone().expect("filtered");
            let blocked = removal_blocked(model, b);
            let reveals = if blocked.is_none() { remove(model, store, b.id).ok().and_then(|r| r.reveals) } else { None };
            ExistingShortcut { combo: c, blocked, reveals }
        })
        .collect()
}

/// Plan binding `combo` to `target`. `others` says what happens to the
/// shortcuts the action already has; `confirmed` approves replacing what
/// `combo` currently runs. Everything lands in one plan, so it is validated
/// and committed (or rejected) together.
pub fn assign(
    model: &Model,
    store: &Store,
    combo: &Combo,
    target: &Target,
    others: &Others,
    confirmed: bool,
) -> Result<AssignResult, PlanError> {
    check_editable(model)?;
    if !combo.is_keyboard() {
        return err("Only keyboard combinations can be assigned here.");
    }
    if combo.mods.contains(crate::keys::Mods::MOD) {
        return err("Internal error: combination is not resolved.");
    }
    let cur = model.effective_for(combo);
    let cur_action = cur.and_then(|b| model.action_of(b));
    if cur_action.map(|a| a.canonical.as_str()) == Some(target.canonical.as_str()) {
        return Ok(AssignResult::NoOp(format!("This shortcut is already assigned to {}.", target.name)));
    }
    let existing = existing_shortcuts(model, store, combo, target);
    let removed = match others {
        Others::Ask if !existing.is_empty() => {
            return Ok(AssignResult::HasShortcuts(Existing { action: target.name.clone(), combo: combo.clone(), shortcuts: existing }));
        }
        Others::Ask | Others::Keep => None,
        Others::Replace(c) => match existing.into_iter().find(|e| &e.combo == c) {
            None => return err(format!("{} is no longer a shortcut of {}; review and save again.", c.label(), target.name)),
            Some(e) if e.blocked.is_some() => return err(format!("{} cannot be removed: {}", c.label(), e.blocked.unwrap())),
            Some(e) => Some(e),
        },
    };
    if let (Some(cur), false) = (cur, confirmed) {
        return Ok(AssignResult::NeedsConfirm(Replacement {
            combo: combo.clone(),
            binding: cur.id,
            current: cur_action.map_or("(no action)".into(), |a| a.name.clone()),
            proposed: target.name.clone(),
            also_removes: removed,
        }));
    }
    let mut notes = Vec::new();
    let mut edits = Edits::new(model);
    let mut description;
    if let Some(cur) = cur {
        description = format!("Replace {}: {} → {}", combo.label(), cur_action.map_or("?", |a| a.name.as_str()), target.name);
        if writable_unique(model, cur.file) {
            replace_in_place(model, cur, target, &mut edits, &mut notes)?;
        } else {
            override_later(model, cur, target, &mut edits)?;
            notes.push(format!(
                "{} is defined in {} which Keymap cannot edit; an override with the same key was added to a later binds section.",
                combo.label(),
                cur.path.display()
            ));
        }
    } else {
        description = format!("Assign {} → {}", combo.label(), target.name);
        let key = combo.to_config(model.mod_key, model.prefer_mod);
        push_new_bind(model, &mut edits, &bind_line(&key, &target.text))?;
    }
    if let Some(r) = &removed {
        let b = model.effective_for(&r.combo).expect("an existing shortcut is effective");
        edits.push(b.file, delete_node(&model.config.files[b.file].text, b.full_span))?;
        description.push_str(&format!(", removing {}", r.combo.label()));
    }
    let plan = edits.finish(description, notes);
    let cand = verify(model, &plan, store, combo, Some(&target.canonical))?;
    if let Some(r) = &removed {
        let after = cand.effective_for(&r.combo).and_then(|b| cand.action_of(b)).map(|a| a.canonical.as_str());
        if after == Some(target.canonical.as_str()) {
            return err(format!("{} would still run {} from another declaration; edit it by hand.", r.combo.label(), target.name));
        }
    }
    Ok(AssignResult::Ready(plan))
}

fn replace_in_place(model: &Model, cur: &Binding, target: &Target, edits: &mut Edits, notes: &mut Vec<String>) -> Result<(), PlanError> {
    let text = &model.config.files[cur.file].text;
    let Some(span) = cur.action_span else { return err("The existing binding has no action node.") };
    edits.push(cur.file, TextEdit { start: span.start, end: span.end, text: target.text.clone() })?;
    for o in &cur.options {
        let drop = match o.name.as_str() {
            "hotkey-overlay-title" => {
                notes.push("Removed hotkey-overlay-title, which described the previous action.".into());
                true
            }
            "allow-when-locked" if !target.is_spawn => {
                notes.push("Removed allow-when-locked, which Niri only accepts on spawn actions.".into());
                true
            }
            _ => false,
        };
        if drop {
            edits.push(cur.file, delete_entry(text, o.span))?;
        }
    }
    Ok(())
}

fn override_later(model: &Model, cur: &Binding, target: &Target, edits: &mut Edits) -> Result<(), PlanError> {
    let after = model.sections[cur.section].order;
    let Some(s) =
        (0..model.sections.len()).rev().find(|s| model.sections[*s].order > after && writable_unique(model, model.sections[*s].file))
    else {
        return err(format!(
            "{} is defined in {} which you cannot edit, and no writable binds section comes after it to override it.",
            cur.key_text,
            cur.path.display()
        ));
    };
    let (file, e) = insert_into_section(model, s, &bind_line(&cur.key_text, &target.text));
    edits.push(file, e)
}

/// Plan removing one effective binding, reporting any declaration it reveals.
pub fn remove(model: &Model, store: &Store, binding: usize) -> Result<RemovePlan, PlanError> {
    check_editable(model)?;
    let b = &model.bindings[binding];
    let Some(combo) = b.combo.clone() else { return err("This binding has an invalid key.") };
    if !b.is_effective() {
        return err("Only the active declaration of a combination can be removed here.");
    }
    if !writable_unique(model, b.file) {
        return err(format!(
            "{} is defined in {}, which Keymap cannot edit. Niri has no \"unbind\" action, so it cannot be removed from here.",
            combo.label(),
            b.path.display()
        ));
    }
    let mut edits = Edits::new(model);
    edits.push(b.file, delete_node(&model.config.files[b.file].text, b.full_span))?;
    let name = model.action_of(b).map_or("?".to_string(), |a| a.name.clone());
    let plan = edits.finish(format!("Remove {} ({name})", combo.label()), vec![]);
    let cand = candidate(model, &plan, store);
    if let Some(d) = cand.config.diagnostics.iter().find(|d| d.error) {
        return err(format!("The removal would make the configuration invalid: {}", d.message));
    }
    let reveals = cand.effective_for(&combo).map(|nb| {
        let an = cand.action_of(nb).map_or("?".to_string(), |a| a.name.clone());
        format!("{an} (from {}:{})", nb.path.display(), nb.line)
    });
    Ok(RemovePlan { plan, reveals })
}

/// Plan removing every declaration of a combination so it is truly unbound.
pub fn unbind_all(model: &Model, store: &Store, combo: &Combo) -> Result<Plan, PlanError> {
    check_editable(model)?;
    let mut edits = Edits::new(model);
    let targets: Vec<&Binding> =
        model.bindings.iter().filter(|b| b.combo.as_ref() == Some(combo) && !matches!(b.status, BindStatus::Invalid(_))).collect();
    if targets.is_empty() {
        return err(format!("{} is not bound.", combo.label()));
    }
    for b in &targets {
        edits.push(b.file, delete_node(&model.config.files[b.file].text, b.full_span))?;
    }
    let plan = edits.finish(format!("Unbind {} everywhere ({} declarations)", combo.label(), targets.len()), vec![]);
    verify(model, &plan, store, combo, None)?;
    Ok(plan)
}
