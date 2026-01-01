//! End-to-end tests of loading, planning and committing against synthetic
//! fixtures copied into temporary directories.

use keymap_core::config::{self, ConfigLocation, LocationSource, hash_bytes};
use keymap_core::keys::{Combo, Key, ModKey};
use keymap_core::model::{BindStatus, Model, Payload};
use keymap_core::plan::{self, AssignResult, Others, Target};
use keymap_core::store::Store;
use keymap_core::txn::{CommitError, NiriValidator, Transactions, TxState, Validator};
use std::fs;
use std::path::{Path, PathBuf};

struct Accept;
impl Validator for Accept {
    fn validate(&self, root: &Path) -> Result<(), String> {
        assert!(root.exists());
        Ok(())
    }
}

struct Reject;
impl Validator for Reject {
    fn validate(&self, _: &Path) -> Result<(), String> {
        Err("synthetic validation failure".into())
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/configs/basic")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            fs::copy(e.path(), dst).unwrap();
        }
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    tx: Transactions,
}

impl Env {
    fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("niri");
        copy_dir(&fixture_dir(), &cfg);
        let tx = Transactions { dir: tmp.path().join("state/backups") };
        Env { root: cfg.join("config.kdl"), _tmp: tmp, tx }
    }

    fn model(&self) -> Model {
        let loc = ConfigLocation { path: self.root.clone(), source: LocationSource::Explicit, verified: true, note: None };
        Model::build(config::load(loc), &Store::default())
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root.parent().unwrap().join(rel)).unwrap()
    }

    fn write(&self, rel: &str, s: &str) {
        fs::write(self.root.parent().unwrap().join(rel), s).unwrap();
    }
}

fn combo(m: &Model, s: &str) -> Combo {
    Key::parse(s).unwrap().resolve(m.mod_key)
}

fn effective_action(m: &Model, s: &str) -> Option<String> {
    m.effective_for(&combo(m, s)).and_then(|b| m.action_of(b)).map(|a| a.payload.to_kdl())
}

fn action_named(m: &Model, kdl_text: &str) -> usize {
    let p = Payload::from_node(&keymap_core::kdl::parse(kdl_text).unwrap().nodes[0]);
    m.find_action(&p.canonical()).unwrap_or_else(|| panic!("no action {kdl_text}")).id
}

fn shortcut_labels(m: &Model, action: usize) -> Vec<String> {
    m.shortcuts(action).iter().map(|b| b.combo.as_ref().unwrap().label()).collect()
}

fn diff_lines(a: &str, b: &str) -> (Vec<String>, Vec<String>) {
    let al: Vec<&str> = a.lines().collect();
    let bl: Vec<&str> = b.lines().collect();
    let removed = al.iter().filter(|l| !bl.contains(l)).map(|s| s.to_string()).collect();
    let added = bl.iter().filter(|l| !al.contains(l)).map(|s| s.to_string()).collect();
    (removed, added)
}

#[test]
fn imports_nested_includes_with_niri_precedence() {
    let env = Env::new();
    let m = env.model();
    assert!(m.read_only.is_none(), "{:?}", m.config.diagnostics);
    assert_eq!(m.config.files.len(), 3, "root, binds-extra, nested/apps");
    assert_eq!(effective_action(&m, "Mod+E").as_deref(), Some(r#"spawn "librewolf""#));
    let old_e = m.bindings.iter().find(|b| b.key_text == "Mod+E" && b.path.ends_with("config.kdl")).unwrap();
    assert!(matches!(old_e.status, BindStatus::Overridden { .. }));
    let super_q = m.bindings.iter().find(|b| b.key_text == "Super+Q").unwrap();
    assert!(matches!(super_q.status, BindStatus::Shadowed { .. }), "first match wins at runtime");
    assert_eq!(effective_action(&m, "Super+Q").as_deref(), Some("close-window"));
    assert!(m.bindings.iter().all(|b| b.key_text != "Mod+D"), "slash-dashed binds are not active");
    let wheel = m.bindings.iter().find(|b| b.key_text == "Mod+WheelScrollDown").unwrap();
    assert!(wheel.is_effective() && !wheel.combo.as_ref().unwrap().is_keyboard());
    assert_eq!(wheel.option("cooldown-ms").unwrap().value, keymap_core::kdl::Value::Number("150".into()));
}

#[test]
fn groups_actions_by_payload_not_name() {
    let env = Env::new();
    let m = env.model();
    let term = action_named(&m, r#"spawn "foot""#);
    assert_eq!(m.actions[term].name, "Open terminal", "hotkey-overlay-title labels the action");
    assert_eq!(shortcut_labels(&m, term), vec!["Super + T", "Super + Enter", "Ctrl + Alt + T"]);
    let ff = action_named(&m, r#"spawn "firefox""#);
    let ffp = action_named(&m, r#"spawn "firefox" "--private-window""#);
    assert_ne!(ff, ffp, "different argv stays separate");
    assert!(shortcut_labels(&m, ff).is_empty(), "overridden Mod+E is not an active shortcut");
    let shot = action_named(&m, "screenshot");
    let shot_window = action_named(&m, "screenshot-window write-to-disk=false");
    assert_ne!(shot, shot_window, "different parameters are a different action");
    assert_eq!(action_named(&m, "screenshot show-pointer=true"), shot, "documented default is equal");
}

#[test]
fn second_shortcut_keeps_the_first_and_touches_one_line() {
    let env = Env::new();
    let m = env.model();
    let before = env.read("config.kdl");
    let shot = action_named(&m, "screenshot");
    let c = combo(&m, "Super+Shift+S");
    let AssignResult::Ready(p) = plan::assign(&m, &Store::default(), &c, &Target::of_action(&m, shot), &Others::Keep, false).unwrap()
    else {
        panic!("expected a ready plan")
    };
    assert_eq!(p.changes.len(), 1);
    env.tx.commit(&m, &p, &Accept).unwrap();
    let after = env.read("config.kdl");
    let (removed, added) = diff_lines(&before, &after);
    assert!(removed.is_empty(), "{removed:?}");
    assert_eq!(added, vec!["    Mod+Shift+S { screenshot; }"]);
    assert_eq!(after.replace("    Mod+Shift+S { screenshot; }\n", ""), before, "every other byte is preserved");
    let m2 = env.model();
    assert_eq!(shortcut_labels(&m2, action_named(&m2, "screenshot")), vec!["Print", "Super + Shift + S"]);
}

#[test]
fn reuses_verbatim_action_text() {
    let env = Env::new();
    let m = env.model();
    let run = action_named(&m, r##"spawn-sh r#"fuzzel --prompt "Run: ""#"##);
    let c = combo(&m, "Super+R");
    let AssignResult::Ready(p) = plan::assign(&m, &Store::default(), &c, &Target::of_action(&m, run), &Others::Keep, false).unwrap() else {
        panic!()
    };
    assert!(p.changes[0].after.contains(r##"Mod+R { spawn-sh r#"fuzzel --prompt "Run: ""#; }"##), "raw string kept as written");
}

#[test]
fn identical_assignment_is_a_noop() {
    let env = Env::new();
    let m = env.model();
    let shot = action_named(&m, "screenshot");
    let r = plan::assign(&m, &Store::default(), &combo(&m, "Print"), &Target::of_action(&m, shot), &Others::Keep, false).unwrap();
    assert!(matches!(r, AssignResult::NoOp(_)));
}

#[test]
fn replacement_needs_confirmation_and_changes_only_its_target() {
    let env = Env::new();
    let m = env.model();
    let hashes: Vec<String> = m.config.files.iter().map(|f| hash_bytes(&fs::read(&f.real_path).unwrap())).collect();
    let browser = action_named(&m, r#"spawn "librewolf""#);
    let c = combo(&m, "Super+T");
    let r = plan::assign(&m, &Store::default(), &c, &Target::of_action(&m, browser), &Others::Keep, false).unwrap();
    let AssignResult::NeedsConfirm(rep) = r else { panic!("expected confirmation") };
    assert_eq!(rep.current, "Open terminal");
    // Cancelling is simply not proceeding: nothing has been written.
    let now: Vec<String> = m.config.files.iter().map(|f| hash_bytes(&fs::read(&f.real_path).unwrap())).collect();
    assert_eq!(hashes, now);
    let AssignResult::Ready(p) = plan::assign(&m, &Store::default(), &c, &Target::of_action(&m, browser), &Others::Keep, true).unwrap()
    else {
        panic!()
    };
    assert!(p.notes.iter().any(|n| n.contains("hotkey-overlay-title")));
    env.tx.commit(&m, &p, &Accept).unwrap();
    let m2 = env.model();
    assert_eq!(effective_action(&m2, "Mod+T").as_deref(), Some(r#"spawn "librewolf""#));
    assert_eq!(effective_action(&m2, "Ctrl+Alt+T").as_deref(), Some(r#"spawn "foot""#), "other shortcut of the old action kept");
    assert_eq!(effective_action(&m2, "Mod+E").as_deref(), Some(r#"spawn "librewolf""#), "other shortcut of the new action kept");
    let (removed, added) = diff_lines(&env.read("config.kdl"), &fs::read_to_string(&m.config.files[0].path).unwrap());
    assert!(removed.is_empty() && added.is_empty());
}

#[test]
fn removing_an_override_reports_what_it_reveals() {
    let env = Env::new();
    let m = env.model();
    let b = m.effective_for(&combo(&m, "Mod+E")).unwrap().id;
    let rp = plan::remove(&m, &Store::default(), b).unwrap();
    let rev = rp.reveals.expect("the config.kdl declaration becomes active");
    assert!(rev.contains("Open browser") && rev.contains("config.kdl"), "{rev}");
    // Unbinding everywhere removes both declarations (two files).
    let all = plan::unbind_all(&m, &Store::default(), &combo(&m, "Mod+E")).unwrap();
    assert_eq!(all.changes.len(), 2);
    env.tx.commit(&m, &all, &Accept).unwrap();
    let m2 = env.model();
    assert!(m2.effective_for(&combo(&m2, "Mod+E")).is_none());
    assert_eq!(effective_action(&m2, "Mod+Shift+E").as_deref(), Some(r#"spawn "firefox" "--private-window""#));
}

#[test]
fn removing_one_shortcut_keeps_the_others() {
    let env = Env::new();
    let m = env.model();
    let b = m.effective_for(&combo(&m, "Ctrl+Alt+T")).unwrap().id;
    let rp = plan::remove(&m, &Store::default(), b).unwrap();
    assert!(rp.reveals.is_none());
    env.tx.commit(&m, &rp.plan, &Accept).unwrap();
    let m2 = env.model();
    let term = action_named(&m2, r#"spawn "foot""#);
    assert_eq!(shortcut_labels(&m2, term), vec!["Super + T", "Super + Enter"]);
}

#[test]
fn validation_failure_leaves_files_untouched() {
    let env = Env::new();
    let m = env.model();
    let before = env.read("config.kdl");
    let target = Target { text: "focus-workspace 3".into(), canonical: "x".into(), name: "x".into(), is_spawn: false };
    let p = plan::assign(&m, &Store::default(), &combo(&m, "Super+3"), &target, &Others::Keep, false);
    // The canonical id is bogus, so verification itself refuses the plan.
    assert!(p.is_err());
    let shot = action_named(&m, "screenshot");
    let AssignResult::Ready(p) =
        plan::assign(&m, &Store::default(), &combo(&m, "Super+3"), &Target::of_action(&m, shot), &Others::Keep, false).unwrap()
    else {
        panic!()
    };
    let e = env.tx.commit(&m, &p, &Reject).unwrap_err();
    assert!(matches!(e, CommitError::Validation(_)));
    assert_eq!(env.read("config.kdl"), before);
    assert!(env.tx.history().is_empty(), "no transaction recorded for a rejected plan");
}

#[test]
fn external_edit_is_detected_and_not_overwritten() {
    let env = Env::new();
    let m = env.model();
    let shot = action_named(&m, "screenshot");
    let AssignResult::Ready(p) =
        plan::assign(&m, &Store::default(), &combo(&m, "Super+3"), &Target::of_action(&m, shot), &Others::Keep, false).unwrap()
    else {
        panic!()
    };
    let edited = env.read("config.kdl").replace("gaps 12", "gaps 16");
    env.write("config.kdl", &edited);
    let e = env.tx.commit(&m, &p, &Accept).unwrap_err();
    assert!(matches!(e, CommitError::Conflict(_)), "{e:?}");
    assert_eq!(env.read("config.kdl"), edited);
}

#[test]
fn undo_restores_bytes_but_never_clobbers_later_edits() {
    let env = Env::new();
    let m = env.model();
    let original = env.read("config.kdl");
    let shot = action_named(&m, "screenshot");
    let AssignResult::Ready(p) =
        plan::assign(&m, &Store::default(), &combo(&m, "Super+3"), &Target::of_action(&m, shot), &Others::Keep, false).unwrap()
    else {
        panic!()
    };
    let r = env.tx.commit(&m, &p, &Accept).unwrap();
    // A later external edit blocks undo.
    let edited = env.read("config.kdl").replace("gaps 12", "gaps 16");
    env.write("config.kdl", &edited);
    assert!(matches!(env.tx.undo_plan(&r.txid), Err(CommitError::Conflict(_))));
    assert_eq!(env.read("config.kdl"), edited);
    // Without the external edit, undo restores the exact original bytes.
    env.write("config.kdl", &p.changes[0].after);
    let m2 = env.model();
    env.tx.undo(&m2, &r.txid, &Accept).unwrap();
    assert_eq!(env.read("config.kdl"), original);
    assert_eq!(env.tx.journal(&r.txid).unwrap().state, TxState::Undone);
}

#[test]
fn interrupted_transaction_can_be_recovered() {
    let env = Env::new();
    let m = env.model();
    let original = env.read("config.kdl");
    let shot = action_named(&m, "screenshot");
    let AssignResult::Ready(p) =
        plan::assign(&m, &Store::default(), &combo(&m, "Super+3"), &Target::of_action(&m, shot), &Others::Keep, false).unwrap()
    else {
        panic!()
    };
    let r = env.tx.commit(&m, &p, &Accept).unwrap();
    // Simulate a crash after the rename but before the journal was finalised.
    let jpath = env.tx.dir.join(&r.txid).join("journal.json");
    let j = fs::read_to_string(&jpath).unwrap().replace("\"committed\"", "\"prepared\"");
    fs::write(&jpath, j).unwrap();
    assert_eq!(env.tx.pending_recoveries().len(), 1);
    let report = env.tx.recover(&r.txid).unwrap();
    assert!(report[0].contains("restored"), "{report:?}");
    assert_eq!(env.read("config.kdl"), original);
    assert!(env.tx.pending_recoveries().is_empty());
}

#[test]
fn read_only_source_is_explained_not_escalated() {
    use std::os::unix::fs::PermissionsExt;
    let env = Env::new();
    let extra = env.root.parent().unwrap().join("binds-extra.kdl");
    fs::set_permissions(&extra, fs::Permissions::from_mode(0o444)).unwrap();
    let m = env.model();
    let b = m.effective_for(&combo(&m, "Mod+E")).unwrap().id;
    let e = plan::remove(&m, &Store::default(), b).unwrap_err();
    assert!(e.0.contains("not writable") || e.0.contains("cannot edit"), "{e}");
    // Replacing it adds an override in a later writable section.
    let shot = action_named(&m, "screenshot");
    let r = plan::assign(&m, &Store::default(), &combo(&m, "Mod+E"), &Target::of_action(&m, shot), &Others::Keep, true).unwrap();
    let AssignResult::Ready(p) = r else { panic!() };
    assert!(p.changes[0].path.ends_with("nested/apps.kdl"));
    assert!(p.changes[0].after.contains("Mod+E { screenshot; }"));
}

#[test]
fn symlinked_config_stays_a_symlink() {
    let env = Env::new();
    let dir = env.root.parent().unwrap();
    let real = dir.join("real.kdl");
    fs::rename(&env.root, &real).unwrap();
    std::os::unix::fs::symlink("real.kdl", &env.root).unwrap();
    let m = env.model();
    let shot = action_named(&m, "screenshot");
    let AssignResult::Ready(p) =
        plan::assign(&m, &Store::default(), &combo(&m, "Super+3"), &Target::of_action(&m, shot), &Others::Keep, false).unwrap()
    else {
        panic!()
    };
    env.tx.commit(&m, &p, &Accept).unwrap();
    assert!(fs::symlink_metadata(&env.root).unwrap().file_type().is_symlink());
    assert!(fs::read_to_string(&real).unwrap().contains("Mod+3 { screenshot; }"));
}

#[test]
fn configured_mod_key_and_shifted_symbols() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("config.kdl");
    fs::write(
        &p,
        "input {\n    mod-key \"Alt\"\n}\nbinds {\n    Mod+T { spawn \"foot\"; }\n    Super+T { spawn \"kitty\"; }\n    Mod+Exclam { close-window; }\n}\n",
    )
    .unwrap();
    let loc = ConfigLocation { path: p, source: LocationSource::Explicit, verified: true, note: None };
    let m = Model::build(config::load(loc), &Store::default());
    assert_eq!(m.mod_key, ModKey::Alt);
    assert_eq!(effective_action(&m, "Alt+T").as_deref(), Some(r#"spawn "foot""#));
    assert_eq!(effective_action(&m, "Super+T").as_deref(), Some(r#"spawn "kitty""#));
    let excl = m.bindings.iter().find(|b| b.key_text == "Mod+Exclam").unwrap();
    assert!(excl.warning.as_deref().unwrap().contains("Shift + 1"));
    let c = combo(&m, "Alt+Shift+S");
    assert_eq!(c.to_config(m.mod_key, m.prefer_mod), "Mod+Shift+S");
}

#[test]
fn invalid_configs_are_read_only_with_locations() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("config.kdl");
    fs::write(&p, "binds {\n    Mod+T { spawn \"a\"; }\n    Mod+T { spawn \"b\"; }\n}\ninclude \"nope.kdl\"\n").unwrap();
    let loc = ConfigLocation { path: p.clone(), source: LocationSource::Explicit, verified: true, note: None };
    let m = Model::build(config::load(loc.clone()), &Store::default());
    assert!(m.read_only.is_some());
    let msgs: Vec<String> = m.config.diagnostics.iter().map(|d| format!("{}: {}", d.line, d.message)).collect();
    assert!(msgs.iter().any(|s| s.starts_with("3: duplicate keybind")), "{msgs:?}");
    assert!(msgs.iter().any(|s| s.starts_with("5: cannot read include")), "{msgs:?}");
    fs::write(&p, "binds {\n    Mod+T { spawn \"a\"\n").unwrap();
    let m = Model::build(config::load(loc), &Store::default());
    assert!(m.read_only.is_some());
    assert!(m.config.diagnostics[0].message.contains("KDL syntax error"));
    // Cyclic includes are reported, not followed forever.
    let a = tmp.path().join("a.kdl");
    fs::write(&a, "include \"b.kdl\"\n").unwrap();
    fs::write(tmp.path().join("b.kdl"), "include \"a.kdl\"\n").unwrap();
    let m = Model::build(
        config::load(ConfigLocation { path: a, source: LocationSource::Explicit, verified: true, note: None }),
        &Store::default(),
    );
    assert!(m.config.diagnostics.iter().any(|d| d.message.contains("recursive include")));
}

/// Uses the installed niri, when present, to check staging of the whole include graph.
#[test]
fn staged_include_graph_validates_with_real_niri() {
    if std::process::Command::new("niri").arg("--version").output().is_err() {
        eprintln!("niri not installed; skipping");
        return;
    }
    let env = Env::new();
    let m = env.model();
    let before: Vec<String> = m.config.files.iter().map(|f| f.text.clone()).collect();
    let shot = action_named(&m, "screenshot");
    let AssignResult::Ready(p) =
        plan::assign(&m, &Store::default(), &combo(&m, "Super+3"), &Target::of_action(&m, shot), &Others::Keep, false).unwrap()
    else {
        panic!()
    };
    // A broken edit in an *included* file must be caught by validating the staged graph.
    let mut bad = p.clone();
    let mut change = bad.changes[0].clone();
    change.path = m.config.files[2].path.clone();
    change.real_path = m.config.files[2].real_path.clone();
    change.expected_hash = m.config.files[2].hash.clone();
    change.before = m.config.files[2].text.clone();
    change.after = m.config.files[2].text.replace("spawn \"alacritty\"", "not-a-real-action");
    bad.changes = vec![change];
    let e = env.tx.commit(&m, &bad, &NiriValidator::default()).unwrap_err();
    let CommitError::Validation(msg) = e else { panic!("{e:?}") };
    assert!(msg.contains("apps.kdl") || msg.contains("not-a-real-action") || msg.contains("unknown"), "{msg}");
    let after: Vec<String> = m.config.files.iter().map(|f| fs::read_to_string(&f.path).unwrap()).collect();
    assert_eq!(before, after, "live files untouched by validation");
    env.tx.commit(&m, &p, &NiriValidator::default()).unwrap();
    assert!(env.read("config.kdl").contains("Mod+3 { screenshot; }"));
}

fn ask(m: &Model, key: &str, action: usize) -> AssignResult {
    plan::assign(m, &Store::default(), &combo(m, key), &Target::of_action(m, action), &Others::Ask, false).unwrap()
}

#[test]
fn an_action_with_shortcuts_asks_first_and_writes_nothing() {
    let env = Env::new();
    let m = env.model();
    let before = env.read("config.kdl");
    let AssignResult::HasShortcuts(e) = ask(&m, "Super+Shift+S", action_named(&m, "screenshot")) else { panic!("expected a question") };
    assert_eq!(e.action, m.actions[action_named(&m, "screenshot")].name);
    assert_eq!(e.shortcuts.iter().map(|s| s.combo.label()).collect::<Vec<_>>(), vec!["Print"]);
    let AssignResult::HasShortcuts(e) = ask(&m, "Super+Y", action_named(&m, r#"spawn "foot""#)) else { panic!() };
    assert_eq!(e.shortcuts.len(), 3, "every shortcut is offered, none chosen for the user");
    // Identity includes arguments: screenshot-window's question never lists screenshot's Print.
    if let AssignResult::HasShortcuts(e) = ask(&m, "Super+Y", action_named(&m, "screenshot-window write-to-disk=false")) {
        assert!(e.shortcuts.iter().all(|s| s.combo.label() != "Print"));
    }
    assert_eq!(env.read("config.kdl"), before);
    let r = ask(&m, "Print", action_named(&m, "screenshot"));
    let AssignResult::NoOp(msg) = r else { panic!() };
    assert_eq!(msg, format!("This shortcut is already assigned to {}.", m.actions[action_named(&m, "screenshot")].name));
}

#[test]
fn replacing_removes_only_the_chosen_shortcut_in_the_same_save() {
    let env = Env::new();
    let m = env.model();
    let foot = action_named(&m, r#"spawn "foot""#);
    let AssignResult::HasShortcuts(e) = ask(&m, "Super+Y", foot) else { panic!() };
    let pick = e.shortcuts.iter().find(|s| s.blocked.is_none()).expect("a removable shortcut").combo.clone();
    let others: Vec<String> = shortcut_labels(&m, foot).into_iter().filter(|l| *l != pick.label()).collect();
    let r = plan::assign(&m, &Store::default(), &combo(&m, "Super+Y"), &Target::of_action(&m, foot), &Others::Replace(pick.clone()), false);
    let AssignResult::Ready(p) = r.unwrap() else { panic!("a free combination needs no second confirmation") };
    assert!(p.description.contains(&format!("removing {}", pick.label())), "{}", p.description);
    env.tx.commit(&m, &p, &Accept).unwrap();
    let m2 = env.model();
    let mut want = others.clone();
    want.push("Super + Y".into());
    let mut got = shortcut_labels(&m2, action_named(&m2, r#"spawn "foot""#));
    got.sort();
    want.sort();
    assert_eq!(got, want);
    assert!(
        m2.effective_for(&pick)
            .and_then(|b| m2.action_of(b))
            .is_none_or(|a| a.canonical != m2.actions[action_named(&m2, r#"spawn "foot""#)].canonical)
    );
}

#[test]
fn a_taken_combination_still_confirms_and_lists_the_removal() {
    let env = Env::new();
    let m = env.model();
    let foot = action_named(&m, r#"spawn "foot""#);
    let AssignResult::HasShortcuts(e) = ask(&m, "Mod+E", foot) else { panic!() };
    let pick = e.shortcuts.iter().find(|s| s.blocked.is_none()).unwrap().combo.clone();
    let t = Target::of_action(&m, foot);
    let c = combo(&m, "Mod+E");
    let AssignResult::NeedsConfirm(rep) = plan::assign(&m, &Store::default(), &c, &t, &Others::Replace(pick.clone()), false).unwrap()
    else {
        panic!("the conflict confirmation is kept")
    };
    assert_eq!(rep.also_removes.as_ref().map(|r| r.combo.clone()), Some(pick.clone()));
    let AssignResult::Ready(p) = plan::assign(&m, &Store::default(), &c, &t, &Others::Replace(pick.clone()), true).unwrap() else {
        panic!()
    };
    env.tx.commit(&m, &p, &Accept).unwrap();
    let m2 = env.model();
    assert_eq!(effective_action(&m2, "Mod+E").as_deref(), Some(r#"spawn "foot""#));
    assert_eq!(shortcut_labels(&m2, action_named(&m2, r#"spawn "foot""#)).len(), 3, "one replaced, one added");
}

#[test]
fn a_stale_choice_is_refused() {
    let env = Env::new();
    let m = env.model();
    let foot = action_named(&m, r#"spawn "foot""#);
    let gone = combo(&m, "Super+F12");
    let r = plan::assign(&m, &Store::default(), &combo(&m, "Super+Y"), &Target::of_action(&m, foot), &Others::Replace(gone), false);
    assert!(r.is_err());
}
