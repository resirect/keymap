//! Actions and bindings derived from a loaded configuration.
//!
//! An *action* is an action node's kind plus its full payload; a *binding* is
//! one key declaration pointing at an action, with its own options and
//! provenance. Actions are grouped only when their payloads are equal after
//! applying Niri's documented defaults; names never cause grouping.

use crate::catalog::{self, ParamKind};
use crate::config::{LoadedConfig, TopNode};
use crate::kdl::{self, Node, Span, Value};
use crate::keys::{self, Combo, Key, ModKey, Mods, Trigger};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Payload {
    pub name: String,
    pub args: Vec<Value>,
    pub props: Vec<(String, Value)>,
    /// Action nodes with children are not produced by any Niri 26.04 action;
    /// they are preserved but never edited.
    #[serde(default)]
    pub has_children: bool,
}

impl Payload {
    pub fn simple(name: &str) -> Self {
        Payload { name: name.into(), args: vec![], props: vec![], has_children: false }
    }

    pub fn spawn(argv: &[String]) -> Self {
        Payload { name: "spawn".into(), args: argv.iter().map(|a| Value::String(a.clone())).collect(), props: vec![], has_children: false }
    }

    pub fn spawn_sh(cmd: &str) -> Self {
        Payload { name: "spawn-sh".into(), args: vec![Value::String(cmd.into())], props: vec![], has_children: false }
    }

    pub fn from_node(n: &Node) -> Self {
        Payload {
            name: n.name.clone(),
            args: n.args().map(|e| e.value.clone()).collect(),
            props: n.props().map(|e| (e.name.clone().unwrap_or_default(), e.value.clone())).collect(),
            has_children: n.children.iter().any(|c| !c.disabled),
        }
    }

    /// Identity used for grouping. Properties are order-insensitive (last
    /// occurrence wins, as in KDL) and documented defaults are dropped, so
    /// `screenshot` equals `screenshot show-pointer=true`.
    pub fn canonical(&self) -> String {
        let def = catalog::find(&self.name);
        let mut props: HashMap<&str, String> = HashMap::new();
        for (k, v) in &self.props {
            props.insert(k, v.canonical());
        }
        if let Some(def) = def {
            for p in def.params {
                if let (Some(name), Some(d)) = (p.prop, p.default) {
                    let dv = match p.kind {
                        ParamKind::Bool => Value::Bool(d == "true").canonical(),
                        _ => Value::Number(d.into()).canonical(),
                    };
                    if props.get(name) == Some(&dv) {
                        props.remove(name);
                    }
                }
            }
        }
        let mut props: Vec<_> = props.into_iter().collect();
        props.sort();
        let args: Vec<String> = self.args.iter().map(Value::canonical).collect();
        let props: Vec<String> = props.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!(
            "{}\u{1f}{}\u{1f}{}{}",
            self.name,
            args.join("\u{1e}"),
            props.join("\u{1e}"),
            if self.has_children { "\u{1f}children" } else { "" }
        )
    }

    /// KDL text for a freshly written action node.
    pub fn to_kdl(&self) -> String {
        let mut s = kdl::ident(&self.name);
        for a in &self.args {
            s.push(' ');
            s.push_str(&a.to_kdl());
        }
        for (k, v) in &self.props {
            s.push(' ');
            s.push_str(&kdl::ident(k));
            s.push('=');
            s.push_str(&v.to_kdl());
        }
        s
    }

    pub fn argv(&self) -> Vec<String> {
        self.args.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_kdl())).collect()
    }

    pub fn is_spawn(&self) -> bool {
        self.name == "spawn" || self.name == "spawn-sh"
    }

    /// Why this payload cannot be edited with forms, if it cannot.
    pub fn unsupported_reason(&self) -> Option<String> {
        if self.has_children {
            return Some("action has child nodes".into());
        }
        if catalog::find(&self.name).is_none() {
            return Some(format!("`{}` is not a Niri 26.04 bind action known to Keymap", self.name));
        }
        None
    }

    /// Conservative display name derived from the payload itself.
    pub fn derived_name(&self) -> String {
        match self.name.as_str() {
            "spawn" => {
                let argv = self.argv();
                let Some(first) = argv.first() else { return "Run (empty command)".into() };
                let base = first.rsplit('/').next().unwrap_or(first);
                let rest = argv[1..].join(" ");
                let mut s = if rest.is_empty() { format!("Run {base}") } else { format!("Run {base} {rest}") };
                truncate(&mut s, 60);
                s
            }
            "spawn-sh" => {
                let mut s = format!("Shell: {}", self.argv().first().cloned().unwrap_or_default());
                truncate(&mut s, 60);
                s
            }
            name => {
                let Some(def) = catalog::find(name) else { return name.to_string() };
                let mut s = def.label.to_string();
                for a in &self.args {
                    s.push(' ');
                    s.push_str(a.as_str().unwrap_or(&a.to_kdl()));
                }
                let extras: Vec<String> = self
                    .props
                    .iter()
                    .filter(|(k, v)| {
                        let d = def.params.iter().find(|p| p.prop == Some(k.as_str())).and_then(|p| p.default);
                        d.map(|d| v.to_kdl() != d).unwrap_or(true)
                    })
                    .map(|(k, v)| format!("{k}={}", v.to_kdl()))
                    .collect();
                if !extras.is_empty() {
                    s.push_str(&format!(" ({})", extras.join(", ")));
                }
                s
            }
        }
    }

    pub fn category(&self) -> &'static str {
        catalog::find(&self.name).map_or("Other", |d| d.category)
    }
}

fn truncate(s: &mut String, max: usize) {
    if s.chars().count() > max {
        *s = s.chars().take(max - 1).collect::<String>() + "…";
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum BindStatus {
    /// This declaration is what the key does.
    Effective,
    /// A later `binds` section declared the exact same key, replacing this one.
    Overridden { by: usize },
    /// Same runtime combination as an earlier declaration (e.g. `Mod+T` and
    /// `Super+T`); Niri uses the first match, so this one never fires.
    Shadowed { by: usize },
    /// Duplicate key inside one `binds` block: Niri rejects the config.
    Duplicate { first: usize },
    /// The key or action could not be understood.
    Invalid(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct BindOption {
    pub name: String,
    pub value: Value,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct Binding {
    pub id: usize,
    pub file: usize,
    pub path: PathBuf,
    pub line: usize,
    pub section: usize,
    pub node_span: Span,
    pub full_span: Span,
    pub key_text: String,
    pub key: Option<Key>,
    pub combo: Option<Combo>,
    pub action: Option<usize>,
    pub action_span: Option<Span>,
    pub options: Vec<BindOption>,
    pub status: BindStatus,
    /// A configuration that Niri accepts but that cannot fire as written.
    pub warning: Option<String>,
}

impl Binding {
    pub fn is_effective(&self) -> bool {
        self.status == BindStatus::Effective
    }
    pub fn option(&self, name: &str) -> Option<&BindOption> {
        self.options.iter().rfind(|o| o.name == name)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BindsSection {
    pub file: usize,
    pub node_span: Span,
    /// The `{ ... }` block.
    pub block: Span,
    pub order: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ActionOrigin {
    /// Appears in the Niri configuration.
    Config,
    /// Saved in Keymap's own store; not bound in Niri.
    Custom { id: String },
    /// A parameterless Niri built-in that is not configured yet.
    Builtin,
}

#[derive(Debug, Clone, Serialize)]
pub struct Action {
    pub id: usize,
    pub payload: Payload,
    pub canonical: String,
    /// Verbatim action node text from the config, reused when adding another
    /// shortcut so quoting and spelling are preserved byte for byte.
    pub source_text: Option<String>,
    pub name: String,
    pub category: String,
    pub origin: ActionOrigin,
    pub custom_id: Option<String>,
    pub unsupported: Option<String>,
    /// All bindings (any status) that reference this action.
    pub bindings: Vec<usize>,
}

impl Action {
    pub fn text(&self) -> String {
        self.source_text.clone().unwrap_or_else(|| self.payload.to_kdl())
    }
}

#[derive(Debug, Clone)]
pub struct Model {
    pub config: LoadedConfig,
    pub sections: Vec<BindsSection>,
    pub bindings: Vec<Binding>,
    pub actions: Vec<Action>,
    pub effective: HashMap<Combo, usize>,
    pub mod_key: ModKey,
    /// Most existing bindings spell the mod key as `Mod`.
    pub prefer_mod: bool,
    /// Why the configuration cannot be edited right now, if it cannot.
    pub read_only: Option<String>,
}

impl Model {
    pub fn build(config: LoadedConfig, store: &Store) -> Model {
        let mut m = Model {
            mod_key: config.mod_key,
            config,
            sections: Vec::new(),
            bindings: Vec::new(),
            actions: Vec::new(),
            effective: HashMap::new(),
            prefer_mod: true,
            read_only: None,
        };
        let mut by_canon: HashMap<String, usize> = HashMap::new();
        let mut merged: Vec<usize> = Vec::new();
        let order: Vec<TopNode> = m.config.order.clone();
        for (oi, t) in order.iter().enumerate() {
            let node = m.config.node(*t).clone();
            if node.name != "binds" {
                continue;
            }
            let Some(block) = node.children_block() else { continue };
            let section = m.sections.len();
            m.sections.push(BindsSection { file: t.file, node_span: node.span, block: block.span, order: oi });
            let mut seen: HashMap<Key, usize> = HashMap::new();
            let mut new_ids: Vec<usize> = Vec::new();
            for child in node.child_nodes() {
                let id = m.add_binding(t.file, section, child, &mut by_canon);
                let Some(key) = m.bindings[id].key.clone() else { continue };
                if let Some(first) = seen.get(&key) {
                    m.bindings[id].status = BindStatus::Duplicate { first: *first };
                    let line = m.bindings[id].line;
                    m.config.diag(t.file, child.span.start, format!("duplicate keybind `{}` (line {line})", child.name), true);
                    continue;
                }
                seen.insert(key, id);
                new_ids.push(id);
            }
            // Later sections replace earlier declarations of the exact same key.
            merged.retain(|old| {
                let k = m.bindings[*old].key.clone();
                match new_ids.iter().find(|n| m.bindings[**n].key == k) {
                    Some(by) => {
                        m.bindings[*old].status = BindStatus::Overridden { by: *by };
                        false
                    }
                    None => true,
                }
            });
            merged.extend(new_ids);
        }
        // Runtime matching takes the first declaration whose resolved
        // modifiers equal the pressed ones.
        for id in merged {
            let Some(combo) = m.bindings[id].combo.clone() else { continue };
            if let Some(first) = m.effective.get(&combo) {
                m.bindings[id].status = BindStatus::Shadowed { by: *first };
            } else {
                m.effective.insert(combo, id);
            }
        }
        let (with_mod, with_literal) = m.bindings.iter().filter_map(|b| b.key.as_ref()).fold((0, 0), |(a, b), k| {
            if k.mods.contains(Mods::MOD) {
                (a + 1, b)
            } else if k.mods.contains(m.mod_key.mods()) {
                (a, b + 1)
            } else {
                (a, b)
            }
        });
        m.prefer_mod = with_mod >= with_literal;
        m.add_store_and_builtins(store, &mut by_canon);
        m.name_actions(store);
        if m.config.has_errors() {
            m.read_only = Some("The configuration has errors; fix them and reload before editing.".into());
        } else if m.config.files.is_empty() {
            m.read_only = Some("No configuration file could be read.".into());
        }
        m
    }

    fn add_binding(&mut self, file: usize, section: usize, node: &Node, by_canon: &mut HashMap<String, usize>) -> usize {
        let id = self.bindings.len();
        let (line, _) = self.config.files[file].line_col(node.span.start);
        let mut status = BindStatus::Effective;
        let key = match Key::parse(&node.name) {
            Ok(k) => Some(k),
            Err(e) => {
                status = BindStatus::Invalid(e.0.clone());
                self.config.diag(file, node.span.start, format!("invalid keybind `{}`: {}", node.name, e.0), true);
                None
            }
        };
        let combo = key.as_ref().map(|k| k.resolve(self.mod_key));
        let mut warning = None;
        if let Some(Trigger::Keysym(sym)) = key.as_ref().map(|k| &k.trigger)
            && let Some(base) = keys::us_unshifted(*sym)
        {
            warning = Some(format!(
                "Niri matches the unshifted key, so `{}` cannot fire on a US-style layout; use Shift + {}",
                keys::keysym_name(*sym),
                keys::keysym_label(base)
            ));
        }
        let action_node = node.child_nodes().next();
        let action = action_node.map(|an| {
            let payload = Payload::from_node(an);
            let canon = payload.canonical();
            *by_canon.entry(canon.clone()).or_insert_with(|| {
                let aid = self.actions.len();
                let text = an.span.slice(&self.config.files[file].text).to_string();
                self.actions.push(Action {
                    id: aid,
                    unsupported: payload.unsupported_reason(),
                    canonical: canon,
                    source_text: Some(text),
                    name: String::new(),
                    category: String::new(),
                    origin: ActionOrigin::Config,
                    custom_id: None,
                    bindings: vec![],
                    payload,
                });
                aid
            })
        });
        if action_node.is_none() && status == BindStatus::Effective {
            status = BindStatus::Invalid("no action".into());
        }
        if let Some(a) = action {
            self.actions[a].bindings.push(id);
        }
        self.bindings.push(Binding {
            id,
            file,
            path: self.config.files[file].path.clone(),
            line,
            section,
            node_span: node.span,
            full_span: node.full_span,
            key_text: node.name.clone(),
            key,
            combo,
            action,
            action_span: action_node.map(|a| a.span),
            options: node
                .props()
                .map(|e| BindOption { name: e.name.clone().unwrap_or_default(), value: e.value.clone(), span: e.span })
                .collect(),
            status,
            warning,
        });
        id
    }

    fn add_store_and_builtins(&mut self, store: &Store, by_canon: &mut HashMap<String, usize>) {
        for c in &store.custom_actions {
            let canon = c.payload.canonical();
            if let Some(&aid) = by_canon.get(&canon) {
                self.actions[aid].custom_id.get_or_insert(c.id.clone());
                continue;
            }
            let aid = self.actions.len();
            self.actions.push(Action {
                id: aid,
                unsupported: c.payload.unsupported_reason(),
                canonical: canon.clone(),
                source_text: None,
                name: String::new(),
                category: String::new(),
                origin: ActionOrigin::Custom { id: c.id.clone() },
                custom_id: Some(c.id.clone()),
                bindings: vec![],
                payload: c.payload.clone(),
            });
            by_canon.insert(canon, aid);
        }
        for def in catalog::ACTIONS {
            if def.params.iter().any(|p| p.required) || def.name == "spawn" || def.name == "spawn-sh" {
                continue;
            }
            let payload = Payload::simple(def.name);
            let canon = payload.canonical();
            if by_canon.contains_key(&canon) {
                continue;
            }
            let aid = self.actions.len();
            self.actions.push(Action {
                id: aid,
                unsupported: None,
                canonical: canon.clone(),
                source_text: None,
                name: String::new(),
                category: String::new(),
                origin: ActionOrigin::Builtin,
                custom_id: None,
                bindings: vec![],
                payload,
            });
            by_canon.insert(canon, aid);
        }
    }

    fn name_actions(&mut self, store: &Store) {
        for i in 0..self.actions.len() {
            let a = &self.actions[i];
            let meta = store.label_for(&a.canonical, a.custom_id.as_deref());
            // A hotkey-overlay-title is the user's own label for what the bind does.
            let title = a
                .bindings
                .iter()
                .find_map(|b| self.bindings[*b].option("hotkey-overlay-title").and_then(|o| o.value.as_str().map(str::to_string)));
            let name = meta.as_ref().map(|m| m.0.clone()).or(title.filter(|t| !t.is_empty())).unwrap_or_else(|| a.payload.derived_name());
            let category = meta.and_then(|m| m.1).filter(|c| !c.is_empty()).unwrap_or_else(|| a.payload.category().to_string());
            self.actions[i].name = name;
            self.actions[i].category = category;
        }
    }

    pub fn effective_for(&self, combo: &Combo) -> Option<&Binding> {
        self.effective.get(combo).map(|i| &self.bindings[*i])
    }

    pub fn action_of(&self, b: &Binding) -> Option<&Action> {
        b.action.map(|a| &self.actions[a])
    }

    /// Effective keyboard shortcuts for an action, sorted for display.
    pub fn shortcuts(&self, action: usize) -> Vec<&Binding> {
        let mut v: Vec<&Binding> = self.actions[action].bindings.iter().map(|b| &self.bindings[*b]).filter(|b| b.is_effective()).collect();
        v.sort_by_key(|b| (b.combo.as_ref().map(|c| c.mods.0.count_ones()), b.id));
        v
    }

    pub fn find_action(&self, canonical: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.canonical == canonical)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(src: &str) -> Payload {
        Payload::from_node(&kdl::parse(src).unwrap().nodes[0])
    }

    #[test]
    fn payload_identity() {
        assert_eq!(p("screenshot").canonical(), p("screenshot show-pointer=true").canonical());
        assert_ne!(p("screenshot").canonical(), p("screenshot show-pointer=false").canonical());
        assert_eq!(p(r#"spawn "foot""#).canonical(), p(r#"spawn r"foot""#).canonical());
        assert_ne!(p(r#"spawn "foot""#).canonical(), p(r#"spawn "foot" "-e" "htop""#).canonical());
        assert_ne!(p(r#"spawn "a b""#).canonical(), p(r#"spawn "a" "b""#).canonical());
        assert_ne!(p(r#"spawn "foot""#).canonical(), p(r#"spawn-sh "foot""#).canonical());
        assert_eq!(p("focus-workspace 0x3").canonical(), p("focus-workspace 3").canonical());
        assert_ne!(p("focus-workspace 3").canonical(), p(r#"focus-workspace "3""#).canonical());
    }

    #[test]
    fn writes_payloads_as_kdl() {
        let s = Payload::spawn(&["sh".into(), "-c".into(), "echo \"hi\" $HOME".into()]);
        let back = p(&s.to_kdl());
        assert_eq!(back, s, "argv survives quoting without expansion");
        assert_eq!(Payload::spawn_sh("grim -g \"$(slurp)\"").to_kdl(), r#"spawn-sh "grim -g \"$(slurp)\"""#);
    }
}
