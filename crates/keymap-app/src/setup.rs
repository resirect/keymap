//! Configuration helpers shared by the CLI and the settings panel:
//! loading, the read-only `check` report, blur-rule status/installation, and
//! the session startup entry (`keymap autostart`).

use crate::Options;
use keymap_core::config::{self, layer_effect};
use keymap_core::model::Model;
use keymap_core::plan::{self, FileChange, Plan, TextEdit};
use keymap_core::store::{self, Store};

pub const NAMESPACE_BLUR: &str = "keymap";
pub const NAMESPACE_PLAIN: &str = "keymap-noblur";

pub const BLUR_RULE: &str = r#"// Keymap: blur the real windows behind Keymap's overlay, and nothing else.
// Matches only the "keymap" layer namespace; other surfaces are unaffected.
// Non-xray effects are experimental in Niri 26.04 (they can briefly
// disappear during some animations).
layer-rule {
    match namespace="^keymap$"
    background-effect {
        blur true
        xray false
    }
}
"#;

pub fn load_model(explicit: Option<&std::path::Path>, store: &Store) -> Model {
    let loc = config::locate(explicit);
    Model::build(config::load(loc), store)
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlurState {
    /// Blur of the windows below is configured for the overlay namespace.
    Ready,
    /// A rule enables blur, but in xray mode (wallpaper only).
    XrayOnly,
    /// `blur { off }` disables all blur.
    GlobalOff,
    /// No rule enables blur for the overlay namespace.
    NotConfigured,
    /// Running Niri is older than 26.04 (no background effects).
    Unsupported(String),
}

pub struct BlurInfo {
    pub state: BlurState,
    /// A broad user rule also blurs the "no blur" namespace.
    pub cannot_disable: bool,
    pub message: String,
}

fn version_ok(v: &str) -> bool {
    let mut it = v.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty());
    let major: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor) >= (26, 4)
}

pub fn blur_info(model: &Model, niri_version: Option<&str>) -> BlurInfo {
    let eff = layer_effect(&model.config, NAMESPACE_BLUR, "overlay");
    let plain = layer_effect(&model.config, NAMESPACE_PLAIN, "overlay");
    let cannot_disable = plain.blur == Some(true);
    let state = if let Some(v) = niri_version.filter(|v| !version_ok(v)) {
        BlurState::Unsupported(v.to_string())
    } else if model.config.blur_off {
        BlurState::GlobalOff
    } else if eff.blur == Some(true) && eff.xray == Some(false) {
        BlurState::Ready
    } else if eff.blur == Some(true) {
        BlurState::XrayOnly
    } else {
        BlurState::NotConfigured
    };
    let message = match &state {
        BlurState::Ready => {
            "Available: Niri blurs the windows behind the overlay. Its strength is Niri's global blur setting (below).".into()
        }
        BlurState::XrayOnly => {
            "A rule enables blur for Keymap in xray mode, which shows only the wallpaper. Add `xray false` to that rule.".into()
        }
        BlurState::GlobalOff => "Unavailable: blur is turned off globally in your Niri config (blur { off }).".into(),
        BlurState::NotConfigured => {
            "Not set up: Niri needs a layer rule for the \"keymap\" namespace. The overlay is dimmed instead.".into()
        }
        BlurState::Unsupported(v) => format!("Unavailable: Niri {v} has no background effects (needs 26.04 or newer)."),
    };
    BlurInfo { state, cannot_disable, message }
}

/// Plan appending the blur rule to the root config file.
pub fn blur_rule_plan(model: &Model, store: &Store) -> Result<Plan, String> {
    if let Some(r) = &model.read_only {
        return Err(r.clone());
    }
    let root = model.config.files.first().ok_or("no configuration file")?;
    if !root.writable {
        return Err(format!("{} is not writable", root.path.display()));
    }
    let sep = if root.text.ends_with('\n') || root.text.is_empty() { "\n" } else { "\n\n" };
    let edit = TextEdit { start: root.text.len(), end: root.text.len(), text: format!("{sep}{BLUR_RULE}") };
    let plan = Plan {
        changes: vec![FileChange {
            path: root.path.clone(),
            real_path: root.real_path.clone(),
            expected_hash: root.hash.clone(),
            before: root.text.clone(),
            after: plan::apply_edits(&root.text, &[edit]),
        }],
        description: "Add Keymap's background blur layer rule".into(),
        notes: vec![],
    };
    let cand = plan::candidate(model, &plan, store);
    if cand.config.has_errors() {
        return Err("the rule would make the configuration invalid".into());
    }
    let eff = layer_effect(&cand.config, NAMESPACE_BLUR, "overlay");
    if eff.blur != Some(true) || eff.xray != Some(false) {
        return Err(format!("a later rule overrides it ({}); edit your layer rules by hand", eff.sources.join(", ")));
    }
    Ok(plan)
}

/// Blur strength presets: Niri's global `blur { passes; offset }`. Niri has
/// no per-surface strength, so this is the only lever; dual-kawase radius
/// grows roughly with `offset × 2^passes` (4, 10, 16, 24).
pub const BLUR_STRENGTHS: [(&str, u32, &str); 4] = [("Subtle", 1, "2"), ("Light", 2, "2.5"), ("Medium", 3, "2"), ("Strong", 3, "3")];
/// What a fresh install uses: clearly lighter than Niri's default (Strong).
pub const DEFAULT_BLUR_STRENGTH: &str = "Light";
/// Niri's own defaults when the config has no `blur` section.
const NIRI_BLUR_DEFAULT: (u32, &str) = (3, "3");

/// The preset matching the config's current blur, if any.
pub fn blur_strength(cfg: &keymap_core::config::LoadedConfig) -> Option<&'static str> {
    let passes = cfg.blur_passes.as_deref().and_then(|p| p.parse::<u32>().ok()).unwrap_or(NIRI_BLUR_DEFAULT.0);
    let offset = cfg.blur_offset.as_deref().and_then(|o| o.parse::<f64>().ok()).unwrap_or(3.0);
    BLUR_STRENGTHS.iter().find(|(_, p, o)| *p == passes && o.parse::<f64>().ok() == Some(offset)).map(|(n, _, _)| *n)
}

/// Plan setting Niri's global blur to preset `name`: edits `passes` and
/// `offset` in the last `blur` section (keeping everything else in it), or
/// appends a commented section to the root file when there is none.
pub fn blur_strength_plan(model: &Model, store: &Store, name: &str) -> Result<Plan, String> {
    if let Some(r) = &model.read_only {
        return Err(r.clone());
    }
    let &(_, passes, offset) = BLUR_STRENGTHS.iter().find(|(n, _, _)| *n == name).ok_or("unknown blur strength")?;
    let cfg = &model.config;
    let (file, edits) = match cfg.blur_node {
        None => {
            let root = cfg.files.first().ok_or("no configuration file")?;
            let sep = if root.text.ends_with('\n') || root.text.is_empty() { "\n" } else { "\n\n" };
            let text = format!(
                "{sep}// Keymap: blur strength (Settings → Blur strength). This is Niri's global\n\
                 // blur, so it also applies to anything else that uses blur.\nblur {{\n    passes {passes}\n    offset {offset}\n}}\n"
            );
            (0, vec![TextEdit { start: root.text.len(), end: root.text.len(), text }])
        }
        Some(t) => {
            let node = cfg.node(t);
            let text = &cfg.files[t.file].text;
            let block = node.children_block().ok_or("the blur section has no { } block; edit it by hand")?;
            let close = block.span.end - 1;
            let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
            let own_line = text[line_start..close].trim().is_empty();
            let indent = block
                .nodes
                .first()
                .map(|c| {
                    let ls = text[..c.span.start].rfind('\n').map_or(0, |i| i + 1);
                    text[ls..c.span.start].to_string()
                })
                .unwrap_or_else(|| format!("{}    ", &text[line_start..close]));
            let (mut edits, mut added) = (Vec::new(), String::new());
            for (key, value) in [("passes", passes.to_string()), ("offset", offset.to_string())] {
                match node.child_nodes().filter(|c| c.name == key).last() {
                    Some(c) => edits.push(TextEdit { start: c.span.start, end: c.span.end, text: format!("{key} {value}") }),
                    None if own_line => added.push_str(&format!("{indent}{key} {value}\n")),
                    None => return Err("the blur section is on one line; edit it by hand".into()),
                }
            }
            if !added.is_empty() {
                edits.push(TextEdit { start: line_start, end: line_start, text: added });
            }
            (t.file, edits)
        }
    };
    let f = &cfg.files[file];
    if !f.writable {
        return Err(format!("{} is not writable", f.path.display()));
    }
    let plan = Plan {
        changes: vec![FileChange {
            path: f.path.clone(),
            real_path: f.real_path.clone(),
            expected_hash: f.hash.clone(),
            before: f.text.clone(),
            after: plan::apply_edits(&f.text, &edits),
        }],
        description: format!("Set Niri's blur strength to {name} (passes {passes}, offset {offset})"),
        notes: vec![],
    };
    let cand = plan::candidate(model, &plan, store);
    if cand.config.has_errors() {
        return Err("the change would make the configuration invalid".into());
    }
    if blur_strength(&cand.config) != Some(name) {
        return Err("a later blur section overrides it; edit your blur settings by hand".into());
    }
    Ok(plan)
}

/// `keymap blur ...`: turn Keymap's blur on (`enable`, run by install.sh) or
/// set its strength. Each change is its own validated, backed-up transaction.
pub fn blur(opts: &Options, action: &str) -> i32 {
    let settings = store::load_settings();
    let explicit = opts.config.clone().or(settings.config_path.clone());
    let st = store::load_store();
    let load = || load_model(explicit.as_deref(), &st);
    let model = load();
    let version = keymap_core::niri_ipc::version();
    if let Some(r) = &model.read_only {
        eprintln!("keymap: cannot tell or change blur: {r}");
        return 1;
    }
    let info = blur_info(&model, version.as_deref());
    let strength = |m: &Model| blur_strength(&m.config).unwrap_or("custom");
    let commit = |model: &Model, plan: Result<Plan, String>| -> bool {
        let plan = match plan {
            Ok(p) => p,
            Err(e) => {
                eprintln!("keymap: cannot change the configuration: {e}");
                return false;
            }
        };
        match keymap_core::txn::Transactions::default().commit(model, &plan, &keymap_core::txn::NiriValidator::default()) {
            Ok(r) => {
                println!("{} ({}).", r.description, r.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", "));
                println!("Validated with `niri validate`; backed up as transaction {}. Undo from Settings → Recent changes.", r.txid);
                true
            }
            Err(e) => {
                eprintln!("keymap: nothing was written: {e}");
                false
            }
        }
    };
    match action {
        "status" => {
            println!("{}", info.message);
            println!("Strength: {} (Niri's global blur; `{} blur subtle|light|medium|strong`)", strength(&model), shell_program());
            0
        }
        "enable" => {
            if let BlurState::Unsupported(_) | BlurState::GlobalOff = info.state {
                println!("{} Nothing changed.", info.message);
                return 0;
            }
            let model = if info.state == BlurState::NotConfigured {
                if !commit(&model, blur_rule_plan(&model, &st)) {
                    return 1;
                }
                load()
            } else {
                println!("The blur rule is already set up (nothing changed).");
                model
            };
            if blur_info(&model, version.as_deref()).state != BlurState::Ready {
                println!("{}", blur_info(&model, version.as_deref()).message);
            }
            // Keep a strength the user already chose; only an unset one gets Keymap's default.
            if model.config.blur_node.is_none() {
                if !commit(&model, blur_strength_plan(&model, &st, DEFAULT_BLUR_STRENGTH)) {
                    return 1;
                }
            } else {
                println!("Blur strength stays as configured ({}).", strength(&model));
            }
            0
        }
        other => match BLUR_STRENGTHS.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(other)) {
            Some((name, _, _)) if blur_strength(&model.config) == Some(name) => {
                println!("Blur strength is already {name} (nothing changed).");
                0
            }
            Some((name, _, _)) => {
                if commit(&model, blur_strength_plan(&model, &st, name)) {
                    0
                } else {
                    1
                }
            }
            None => {
                eprintln!("keymap: unknown blur action `{other}` (use status, enable, subtle, light, medium or strong)");
                2
            }
        },
    }
}

/// `keymap check`: a read-only summary without printing commands.
pub fn check(opts: &Options) -> i32 {
    let settings = store::load_settings();
    let explicit = opts.config.clone().or(settings.config_path.clone());
    let model = load_model(explicit.as_deref(), &store::load_store());
    let loc = &model.config.location;
    println!("Config:       {}", loc.path.display());
    println!("Found via:    {:?}{}", loc.source, if loc.verified { "" } else { " (not verified)" });
    if let Some(n) = &loc.note {
        println!("Note:         {n}");
    }
    for f in &model.config.files {
        println!(
            "  file        {}{}{}",
            f.path.display(),
            if f.symlink { format!(" -> {}", f.real_path.display()) } else { String::new() },
            if f.writable { "" } else { " (read-only)" }
        );
    }
    let kb = model.bindings.iter().filter(|b| b.combo.as_ref().is_some_and(|c| c.is_keyboard()));
    let effective = kb.clone().filter(|b| b.is_effective()).count();
    let pointer = model.bindings.iter().filter(|b| b.combo.as_ref().is_some_and(|c| !c.is_keyboard())).count();
    let overridden = model.bindings.iter().filter(|b| matches!(b.status, keymap_core::model::BindStatus::Overridden { .. })).count();
    let shadowed = model.bindings.iter().filter(|b| matches!(b.status, keymap_core::model::BindStatus::Shadowed { .. })).count();
    let warnings = model.bindings.iter().filter(|b| b.warning.is_some()).count();
    println!("Mod key:      {:?}", model.mod_key);
    println!("XKB layout:   {}", if model.config.xkb.layout.is_empty() { "(default)" } else { &model.config.xkb.layout });
    println!(
        "Bindings:     {effective} active keyboard, {pointer} pointer, {overridden} overridden, {shadowed} shadowed, {warnings} that cannot fire"
    );
    let configured = model.actions.iter().filter(|a| a.origin == keymap_core::model::ActionOrigin::Config).count();
    println!("Actions:      {configured} configured, {} total available", model.actions.len());
    let version = keymap_core::niri_ipc::version();
    println!("Niri:         {}", version.as_deref().unwrap_or("(not reachable over IPC)"));
    println!("Blur:         {}", blur_info(&model, version.as_deref()).message);
    let launch = launch_binds(&model);
    match suggest_launch_bind(&model) {
        _ if !launch.is_empty() => {
            let keys: Vec<String> = launch.iter().map(|b| b.combo.as_ref().map_or(b.key_text.clone(), |c| c.label())).collect();
            println!("Launch bind:  {} opens Keymap (`keymap shortcut set COMBO` moves it)", keys.join(", "))
        }
        Some(k) => {
            // Niri spawns with its own PATH, which may lack ~/.local/bin.
            let program = startup_program();
            println!("Launch bind:  {k} is free. Example: {k} repeat=false allow-inhibiting=false {{ spawn \"{program}\" \"toggle\"; }}")
        }
        None => println!("Launch bind:  the suggested combinations are taken; choose a free one in Keymap itself"),
    }
    let tx = keymap_core::txn::Transactions::default();
    for j in tx.pending_recoveries() {
        println!("Interrupted:  {} — {} (recover it from Settings)", j.txid, j.description);
    }
    match &model.read_only {
        None => println!("Editable:     yes"),
        Some(r) => println!("Editable:     no — {r}"),
    }
    for d in &model.config.diagnostics {
        println!("  {} {}:{}:{}: {}", if d.error { "error" } else { "note " }, d.path.display(), d.line, d.column, d.message);
    }
    if model.read_only.is_some() { 1 } else { 0 }
}

/// The first unused combination among the default and a few alternatives.
pub fn suggest_launch_bind(model: &Model) -> Option<&'static str> {
    [DEFAULT_LAUNCH, "Mod+Shift+K", "Mod+Ctrl+K", "Mod+Alt+K"].into_iter().find(|k| {
        let combo = keymap_core::keys::Key::parse(k).expect("valid key").resolve(model.mod_key);
        model.bindings.iter().all(|b| b.combo.as_ref() != Some(&combo))
    })
}

// ------------------------------------------------------- launch shortcut

/// The shortcut that opens Keymap by default (Super+Shift+M).
pub const DEFAULT_LAUNCH: &str = "Mod+Shift+M";

/// The string arguments of an action's KDL text, unescaped.
fn quoted_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut cur = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => cur.extend(chars.next()),
                '"' => break,
                c => cur.push(c),
            }
        }
        out.push(cur);
    }
    out
}

/// Whether an action's KDL text opens Keymap (`keymap toggle` or `show`,
/// directly, through `env`, or as a `spawn-sh` command line).
fn opens_keymap(action_text: &str) -> bool {
    let args = quoted_args(action_text);
    let words: Vec<&str> = match action_text.split_whitespace().next() {
        Some("spawn") => args.iter().map(String::as_str).collect(),
        Some("spawn-sh") => args.first().map(|c| c.split_whitespace().collect()).unwrap_or_default(),
        _ => return false,
    };
    let Some(program) = program_after_env(words.iter().copied()) else { return false };
    let rest = words.iter().skip_while(|w| **w != program).skip(1);
    is_keymap_program(program) && rest.into_iter().any(|w| *w == "toggle" || *w == "show")
}

/// Active bindings that open Keymap.
pub fn launch_binds(model: &Model) -> Vec<&keymap_core::model::Binding> {
    model
        .bindings
        .iter()
        .filter(|b| b.is_effective() && b.combo.is_some() && model.action_of(b).is_some_and(|a| opens_keymap(&a.text())))
        .collect()
}

/// Plan making `key` open Keymap. Keymap's existing launch bind is moved
/// (only its key changes, so properties such as `repeat=false` and a custom
/// command stay); without one, a new bind is added. Either way it is a single
/// validated transaction: the old shortcut goes only if the new one is saved.
/// A combination used by anything else is refused, never overwritten.
pub fn launch_plan(model: &Model, key: &str, program: &str) -> Result<Option<Plan>, String> {
    if let Some(r) = &model.read_only {
        return Err(r.clone());
    }
    let combo = keymap_core::keys::Key::parse(key).map_err(|e| format!("`{key}` is not a key combination: {e}"))?.resolve(model.mod_key);
    let current = launch_binds(model);
    if current.iter().any(|b| b.combo.as_ref() == Some(&combo)) {
        return Ok(None);
    }
    if let Some(b) = model.bindings.iter().find(|b| b.combo.as_ref() == Some(&combo)) {
        let what = model.action_of(b).map_or("(no action)".to_string(), |a| format!("“{}”", a.name));
        return Err(format!(
            "{} is already bound to {what} ({}:{}). Nothing was changed. Choose another combination, or rebind that one first.",
            combo.label(),
            b.path.display(),
            b.line
        ));
    }
    let written = combo.to_config(model.mod_key, model.prefer_mod);
    let plan = match current.as_slice() {
        [] => {
            let program = program.replace('\\', "\\\\").replace('"', "\\\"");
            let line = format!(
                "{written} repeat=false allow-inhibiting=false hotkey-overlay-title=\"Keymap: edit shortcuts\" {{ spawn \"{program}\" \"toggle\"; }}"
            );
            plan::add_bind_line(model, &line, format!("Open Keymap with {}", combo.label()))
        }
        [old] => plan::rename_bind(
            model,
            old,
            &written,
            format!("Open Keymap with {} instead of {}", combo.label(), old.combo.as_ref().map_or(String::new(), |c| c.label())),
        ),
        several => {
            let list: Vec<String> = several.iter().map(|b| format!("{} ({}:{})", b.key_text, b.path.display(), b.line)).collect();
            return Err(format!("Keymap opens from several shortcuts: {}. Remove the extra ones in Keymap first.", list.join(", ")));
        }
    };
    plan.map(Some).map_err(|e| e.0)
}

/// `keymap shortcut [status|set COMBO]`.
pub fn shortcut(opts: &Options, action: &str) -> i32 {
    let settings = store::load_settings();
    let explicit = opts.config.clone().or(settings.config_path.clone());
    let st = store::load_store();
    let model = load_model(explicit.as_deref(), &st);
    if let Some(r) = &model.read_only {
        eprintln!("keymap: cannot read or change the shortcut: {r}");
        return 1;
    }
    let key = match action.split_once(' ') {
        Some(("set", k)) => k.to_string(),
        None if action == "status" => {
            let binds = launch_binds(&model);
            if binds.is_empty() {
                println!("No shortcut opens Keymap. `{} shortcut set` binds {DEFAULT_LAUNCH} (Super+Shift+M).", shell_program());
            }
            for b in binds {
                println!("Opens Keymap: {}  ({}:{})", b.combo.as_ref().map_or(b.key_text.clone(), |c| c.label()), b.path.display(), b.line);
            }
            return 0;
        }
        None if action == "remove" => return remove_launch_binds(explicit.as_deref(), &st),
        _ => {
            eprintln!("keymap: unknown shortcut action `{action}` (use status, set [COMBO] or remove)");
            return 2;
        }
    };
    let plan = match launch_plan(&model, &key, &startup_program()) {
        Ok(Some(p)) => p,
        Ok(None) => {
            println!("{key} already opens Keymap (nothing changed).");
            return 0;
        }
        Err(e) => {
            eprintln!("keymap: {e}");
            return 1;
        }
    };
    let cand = plan::candidate(&model, &plan, &st);
    let combo = keymap_core::keys::Key::parse(&key).expect("parsed above").resolve(model.mod_key);
    if cand.config.has_errors() || !launch_binds(&cand).iter().any(|b| b.combo.as_ref() == Some(&combo)) {
        eprintln!("keymap: the change would not leave {key} opening Keymap; nothing was written");
        return 1;
    }
    let tx = keymap_core::txn::Transactions::default();
    match tx.commit(&model, &plan, &keymap_core::txn::NiriValidator::default()) {
        Ok(r) => {
            println!("{} ({}).", r.description, r.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", "));
            println!("Validated with `niri validate`; the previous file is backed up (transaction {}).", r.txid);
            println!("Niri reloads it right away. Undo from Keymap's Settings → Recent changes.");
            0
        }
        Err(e) => {
            eprintln!("keymap: nothing was written: {e}");
            1
        }
    }
}

/// `keymap shortcut remove`: remove every bind that opens Keymap, one
/// validated, backed-up transaction per bind.
fn remove_launch_binds(explicit: Option<&std::path::Path>, st: &Store) -> i32 {
    let mut removed = 0;
    loop {
        let model = load_model(explicit, st);
        if let Some(r) = &model.read_only {
            eprintln!("keymap: cannot read or change the shortcut: {r}");
            return 1;
        }
        let launch = launch_binds(&model);
        let Some(i) = model.bindings.iter().position(|b| launch.iter().any(|l| std::ptr::eq(*l, b))) else { break };
        let rp = match plan::remove(&model, st, i) {
            Ok(rp) => rp,
            Err(e) => {
                eprintln!("keymap: cannot remove {}: {}", model.bindings[i].key_text, e.0);
                return 1;
            }
        };
        let tx = keymap_core::txn::Transactions::default();
        match tx.commit(&model, &rp.plan, &keymap_core::txn::NiriValidator::default()) {
            Ok(r) => {
                println!("{} ({}).", r.description, r.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", "));
                println!("Validated with `niri validate`; the previous file is backed up (transaction {}).", r.txid);
                if let Some(rev) = rp.reveals {
                    println!("That combination now runs {rev}.");
                }
                removed += 1;
            }
            Err(e) => {
                eprintln!("keymap: nothing was written: {e}");
                return 1;
            }
        }
    }
    if removed == 0 {
        println!("No shortcut opens Keymap (nothing changed).");
    }
    0
}

// ------------------------------------------------------------- autostart

/// Comment placed above the startup entry Keymap adds.
pub const AUTOSTART_COMMENT: &str =
    "// Keymap: start hidden with the session so it opens instantly. `keymap autostart disable` removes this.";

/// A `spawn-at-startup` / `spawn-sh-at-startup` node that starts Keymap.
#[derive(Debug, Clone, PartialEq)]
pub struct StartupEntry {
    pub file: usize,
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// The program a command line runs, skipping an `env` wrapper and
/// `NAME=value` assignments (`env GSK_RENDERER=gl keymap preload`).
fn program_after_env<'a>(mut words: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut w = words.next()?;
    if std::path::Path::new(w).file_name().is_some_and(|n| n == "env") {
        w = words.next()?;
    }
    while w.contains('=') && !w.starts_with('=') {
        w = words.next()?;
    }
    Some(w)
}

fn is_keymap_program(p: &str) -> bool {
    std::path::Path::new(p).file_name().is_some_and(|n| n == "keymap")
}

/// Every enabled startup node, in any writable or read-only config file,
/// that runs Keymap.
pub fn startup_entries(model: &Model) -> Vec<StartupEntry> {
    let mut out = Vec::new();
    for (i, f) in model.config.files.iter().enumerate() {
        let Some(doc) = &f.doc else { continue };
        for n in doc.enabled() {
            let args: Vec<&str> = n
                .args()
                .filter_map(|e| match &e.value {
                    keymap_core::kdl::Value::String(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect();
            let runs_keymap = match n.name.as_str() {
                "spawn-at-startup" => program_after_env(args.iter().copied()).is_some_and(is_keymap_program),
                "spawn-sh-at-startup" => args.first().and_then(|c| program_after_env(c.split_whitespace())).is_some_and(is_keymap_program),
                _ => false,
            };
            if runs_keymap {
                out.push(StartupEntry {
                    file: i,
                    line: keymap_core::kdl::line_col(&f.text, n.span.start).0,
                    start: n.span.start,
                    end: n.full_span.end,
                    text: f.text[n.span.start..n.full_span.end].to_string(),
                });
            }
        }
    }
    out
}

/// How the startup entry and launch bind name the program: `keymap` when a
/// directory outside `$HOME` on Niri's own PATH finds this same executable,
/// otherwise this executable's absolute path. Directories under `$HOME` (such
/// as `~/.local/bin`) are usually on PATH only through a shell profile, which a
/// session started by a display manager may not read.
fn startup_program() -> String {
    let path = niri_env_path().or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    program_on(&system_dirs(&path, &std::env::var("HOME").unwrap_or_default()))
}

/// The entries of `path` that are not under `home`.
fn system_dirs(path: &str, home: &str) -> String {
    path.split(':').filter(|d| home.is_empty() || !std::path::Path::new(d).starts_with(home)).collect::<Vec<_>>().join(":")
}

/// How to name this binary in hints printed to the user's shell.
fn shell_program() -> String {
    program_on(&std::env::var("PATH").unwrap_or_default())
}

/// `keymap` when that name on `path` is this binary, else its full path.
fn program_on(path: &str) -> String {
    let me = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let found = path.split(':').map(|d| std::path::Path::new(d).join("keymap")).find(|p| p.is_file());
    match (me, found) {
        (Some(me), Some(f)) if f.canonicalize().ok().as_ref() == Some(&me) => "keymap".into(),
        (Some(me), _) => me.display().to_string(),
        (None, _) => "keymap".into(),
    }
}

/// PATH of the running Niri, from the process behind `$NIRI_SOCKET`
/// (`niri.<display>.<pid>.sock`).
fn niri_env_path() -> Option<String> {
    let sock = std::env::var("NIRI_SOCKET").ok()?;
    let pid: u32 = sock.rsplit('.').nth(1)?.parse().ok()?;
    let env = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    env.split(|b| *b == 0).find_map(|kv| kv.strip_prefix(b"PATH=")).map(|v| String::from_utf8_lossy(v).into_owned())
}

/// Plan adding `spawn-at-startup "keymap" "preload"` to the root file.
pub fn autostart_plan(model: &Model, program: &str) -> Result<Plan, String> {
    if let Some(r) = &model.read_only {
        return Err(r.clone());
    }
    let root = model.config.files.first().ok_or("no configuration file")?;
    if !root.writable {
        return Err(format!("{} is not writable", root.path.display()));
    }
    let sep = if root.text.ends_with('\n') || root.text.is_empty() { "\n" } else { "\n\n" };
    let quoted = program.replace('\\', "\\\\").replace('"', "\\\"");
    let text = format!("{sep}{AUTOSTART_COMMENT}\nspawn-at-startup \"{quoted}\" \"preload\"\n");
    let edit = TextEdit { start: root.text.len(), end: root.text.len(), text };
    Ok(Plan {
        changes: vec![FileChange {
            path: root.path.clone(),
            real_path: root.real_path.clone(),
            expected_hash: root.hash.clone(),
            before: root.text.clone(),
            after: plan::apply_edits(&root.text, &[edit]),
        }],
        description: "Start Keymap hidden with the Niri session".into(),
        notes: vec![],
    })
}

/// Plan removing every startup entry that runs Keymap (and the comment Keymap
/// put above its own), leaving everything else byte-for-byte.
pub fn autostart_removal_plan(model: &Model) -> Result<Plan, String> {
    if let Some(r) = &model.read_only {
        return Err(r.clone());
    }
    let entries = startup_entries(model);
    let mut changes = Vec::new();
    for (i, f) in model.config.files.iter().enumerate() {
        let mine: Vec<&StartupEntry> = entries.iter().filter(|e| e.file == i).collect();
        if mine.is_empty() {
            continue;
        }
        if !f.writable {
            return Err(format!("{} is not writable", f.path.display()));
        }
        let t = &f.text;
        let edits: Vec<TextEdit> = mine
            .iter()
            .map(|e| {
                // Whole lines when the entry is alone on its line(s).
                let line_start = t[..e.start].rfind('\n').map_or(0, |i| i + 1);
                let line_end = t[e.end..].find('\n').map_or(t.len(), |i| e.end + i + 1);
                let alone = t[line_start..e.start].trim().is_empty() && t[e.end..line_end].trim().is_empty();
                let (mut start, end) = if alone { (line_start, line_end) } else { (e.start, e.end) };
                if alone && start > 0 {
                    let prev_start = t[..start - 1].rfind('\n').map_or(0, |i| i + 1);
                    if t[prev_start..start].trim_end() == AUTOSTART_COMMENT {
                        start = prev_start;
                        // And the blank line Keymap put before its comment.
                        if start > 0 && t[..start].ends_with("\n\n") {
                            start -= 1;
                        }
                    }
                }
                TextEdit { start, end, text: String::new() }
            })
            .collect();
        changes.push(FileChange {
            path: f.path.clone(),
            real_path: f.real_path.clone(),
            expected_hash: f.hash.clone(),
            before: t.clone(),
            after: plan::apply_edits(t, &edits),
        });
    }
    Ok(Plan { changes, description: "Stop starting Keymap with the Niri session".into(), notes: vec![] })
}

/// `keymap autostart [enable|disable|status]`.
pub fn autostart(opts: &Options, action: &str) -> i32 {
    let settings = store::load_settings();
    let explicit = opts.config.clone().or(settings.config_path.clone());
    let st = store::load_store();
    let model = load_model(explicit.as_deref(), &st);
    // An unreadable config hides any entry in it: never report "not enabled".
    if let Some(r) = &model.read_only {
        eprintln!("keymap: cannot tell or change autostart: {r}");
        return 1;
    }
    let entries = startup_entries(&model);
    let show = |e: &StartupEntry| format!("{}:{}  {}", model.config.files[e.file].path.display(), e.line, e.text);
    let plan = match action {
        "status" => {
            if entries.is_empty() {
                println!("Keymap does not start with the session. `{} autostart enable` adds it.", shell_program());
            } else {
                for e in &entries {
                    println!("Starts with the session: {}", show(e));
                }
            }
            return 0;
        }
        "enable" if !entries.is_empty() => {
            println!("Already enabled (nothing changed): {}", show(&entries[0]));
            return 0;
        }
        "enable" => autostart_plan(&model, &startup_program()),
        "disable" if entries.is_empty() => {
            println!("Not enabled (nothing changed).");
            return 0;
        }
        "disable" => autostart_removal_plan(&model),
        other => {
            eprintln!("keymap: unknown autostart action `{other}` (use enable, disable or status)");
            return 2;
        }
    };
    let plan = match plan {
        Ok(p) => p,
        Err(e) => {
            eprintln!("keymap: cannot change the configuration: {e}");
            return 1;
        }
    };
    let cand = plan::candidate(&model, &plan, &st);
    if cand.config.has_errors() {
        eprintln!("keymap: the change would make the configuration invalid; nothing was written");
        return 1;
    }
    let tx = keymap_core::txn::Transactions::default();
    match tx.commit(&model, &plan, &keymap_core::txn::NiriValidator::default()) {
        Ok(r) => {
            println!("{} ({}).", r.description, r.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", "));
            println!("Validated with `niri validate`; the previous file is backed up (transaction {}).", r.txid);
            if action == "enable" {
                println!("Niri runs it at the next login. To start Keymap hidden right now: {} preload", shell_program());
            }
            println!(
                "Undo from Keymap's Settings → Recent changes, or run `{} autostart {}`.",
                shell_program(),
                if action == "enable" { "disable" } else { "enable" }
            );
            0
        }
        Err(e) => {
            eprintln!("keymap: nothing was written: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keymap_core::config::{ConfigLocation, LocationSource};

    fn model(text: &str) -> (tempfile::TempDir, Model) {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.kdl");
        std::fs::write(&p, text).unwrap();
        let loc = ConfigLocation { path: p, source: LocationSource::Explicit, verified: true, note: None };
        let m = Model::build(config::load(loc), &Store::default());
        (tmp, m)
    }

    const USER: &str =
        "spawn-at-startup \"waybar\"\nspawn-at-startup \"/usr/bin/keymapper\" \"--daemon\"\nbinds {\n    Mod+T { spawn \"foot\"; }\n}\n";

    #[test]
    fn enabling_then_disabling_restores_the_file_exactly() {
        let (_t, m) = model(USER);
        assert!(startup_entries(&m).is_empty(), "keymapper is not keymap");
        let added = autostart_plan(&m, "keymap").unwrap().changes[0].after.clone();
        assert!(added.starts_with(USER), "only appends");
        assert!(added.ends_with("spawn-at-startup \"keymap\" \"preload\"\n"));
        let (_t2, m2) = model(&added);
        assert_eq!(startup_entries(&m2).len(), 1);
        assert_eq!(autostart_removal_plan(&m2).unwrap().changes[0].after, USER);
    }

    #[test]
    fn disabling_removes_only_keymap_entries_in_any_form() {
        let text = format!(
            "{USER}spawn-sh-at-startup \"keymap preload\"\nspawn-at-startup \"/home/u/.local/bin/keymap\" \"--resident\" \"hide\"\n/-spawn-at-startup \"keymap\" \"preload\"\n"
        );
        let (_t, m) = model(&text);
        assert_eq!(startup_entries(&m).len(), 2, "commented-out entries are not running ones");
        let after = autostart_removal_plan(&m).unwrap().changes[0].after.clone();
        assert_eq!(after, format!("{USER}/-spawn-at-startup \"keymap\" \"preload\"\n"));
        let (_t2, wrapped) = model(
            "spawn-at-startup \"env\" \"GSK_RENDERER=gl\" \"keymap\" \"preload\"\nspawn-sh-at-startup \"GSK_RENDERER=gl keymap preload\"\nspawn-at-startup \"env\" \"A=1\" \"foot\"\n",
        );
        assert_eq!(startup_entries(&wrapped).len(), 2, "env-wrapped entries count; other programs do not");
        let after = autostart_removal_plan(&wrapped).unwrap().changes[0].after.clone();
        assert_eq!(after, "spawn-at-startup \"env\" \"A=1\" \"foot\"\n");
    }

    #[test]
    fn blur_strength_appends_a_section_when_there_is_none() {
        let (_t, m) = model(USER);
        assert_eq!(blur_strength(&m.config), Some("Strong"), "Niri's defaults are the strongest preset");
        let after = blur_strength_plan(&m, &Store::default(), "Light").unwrap().changes[0].after.clone();
        assert!(after.starts_with(USER), "only appends");
        assert!(after.ends_with("blur {\n    passes 2\n    offset 2.5\n}\n"));
        let (_t2, m2) = model(&after);
        assert_eq!(blur_strength(&m2.config), Some("Light"));
    }

    #[test]
    fn blur_strength_edits_only_passes_and_offset_in_the_last_section() {
        let text = "blur {\n    passes 5\n}\n// mine\nblur {\n    noise 0.05\n    passes 4\n    saturation 1.2\n}\n";
        let (_t, m) = model(text);
        assert_eq!(blur_strength(&m.config), None, "custom values match no preset");
        let after = blur_strength_plan(&m, &Store::default(), "Subtle").unwrap().changes[0].after.clone();
        assert_eq!(after, "blur {\n    passes 5\n}\n// mine\nblur {\n    noise 0.05\n    passes 1\n    saturation 1.2\n    offset 2\n}\n");
        let (_t2, m2) = model(&after);
        assert_eq!(blur_strength(&m2.config), Some("Subtle"));
        let back = blur_strength_plan(&m2, &Store::default(), "Medium").unwrap().changes[0].after.clone();
        assert!(back.contains("passes 3\n    saturation 1.2\n    offset 2\n"));
        let (_t3, one_line) = model("blur { noise 0.1; }\n");
        assert!(blur_strength_plan(&one_line, &Store::default(), "Light").is_err(), "one-line sections are left alone");
    }

    const LAUNCH: &str = "binds {\n    Mod+T { spawn \"foot\"; }\n    Mod+I { focus-column-right; }\n    Mod+Shift+K allow-inhibiting=false repeat=false { spawn \"env\" \"GSK_RENDERER=gl\" \"keymap\" \"toggle\"; }\n}\n";

    #[test]
    fn moving_the_launch_shortcut_changes_only_its_key() {
        let (_t, m) = model(LAUNCH);
        assert_eq!(launch_binds(&m).len(), 1, "an env-wrapped `keymap toggle` is Keymap's bind");
        let after = launch_plan(&m, "Mod+M", "keymap").unwrap().unwrap().changes[0].after.clone();
        assert_eq!(after, LAUNCH.replace("Mod+Shift+K allow", "Mod+M allow"), "properties and command are kept");
        let (_t2, m2) = model(&after);
        assert!(launch_plan(&m2, "Mod+M", "keymap").unwrap().is_none(), "setting it again changes nothing");
    }

    #[test]
    fn a_taken_combination_is_refused_and_named() {
        let (_t, m) = model(LAUNCH);
        let e = launch_plan(&m, "Mod+I", "keymap").unwrap_err();
        assert!(e.contains("Super + I") && e.contains("Nothing was changed"), "{e}");
        assert!(e.to_lowercase().contains("focus"), "names the action: {e}");
    }

    #[test]
    fn without_a_launch_bind_one_is_added_once() {
        let (_t, m) = model(USER);
        assert!(launch_binds(&m).is_empty(), "keymapper and foot do not open Keymap");
        let after = launch_plan(&m, DEFAULT_LAUNCH, "keymap").unwrap().unwrap().changes[0].after.clone();
        assert!(after.contains("Mod+Shift+M repeat=false allow-inhibiting=false"), "{after}");
        let (_t2, m2) = model(&after);
        assert_eq!(launch_binds(&m2).len(), 1);
        assert!(launch_plan(&m2, DEFAULT_LAUNCH, "keymap").unwrap().is_none(), "no duplicate");
    }

    #[test]
    fn launch_binds_are_recognised_in_any_form() {
        assert!(opens_keymap(r#"spawn "keymap" "toggle""#));
        assert!(opens_keymap(r#"spawn "/home/u/.local/bin/keymap" "show""#));
        assert!(opens_keymap(r#"spawn-sh "GSK_RENDERER=gl keymap toggle""#));
        assert!(!opens_keymap(r#"spawn "keymap" "quit""#));
        assert!(!opens_keymap(r#"spawn "keymapper" "toggle""#));
        assert!(!opens_keymap(r#"spawn "foot" "keymap" "toggle""#), "foot is the program here");
    }

    #[test]
    fn only_directories_outside_home_can_find_keymap_by_name() {
        let path = "/home/u/.local/bin:/home/u/bin:/usr/local/bin:/usr/bin:/home/user2/bin";
        assert_eq!(system_dirs(path, "/home/u"), "/usr/local/bin:/usr/bin:/home/user2/bin");
        assert_eq!(system_dirs(path, ""), path);
    }
}
