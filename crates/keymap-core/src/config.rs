//! Niri configuration discovery and loading of the include graph.
//!
//! Loading never modifies anything. It reads the root file and every
//! reachable include with Niri 26.04 resolution rules (relative to the
//! including file, `~/` expansion, `optional=true`, cycle detection) and keeps
//! each file's exact bytes and parsed spans for later patching.

use crate::kdl::{self, Document, Node, ParseError, Span};
use crate::keys::ModKey;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Niri's include recursion limit is small; anything deeper is reported.
pub const RECURSION_LIMIT: usize = 16;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum LocationSource {
    /// Set explicitly by the user (command line or Keymap settings).
    Explicit,
    /// Read from the running Niri process (`-c/--config` or `NIRI_CONFIG`).
    RunningInstance { pid: u32 },
    /// Niri's default lookup applied to the running process environment.
    RunningDefault { pid: u32 },
    /// Niri's default lookup applied to Keymap's own environment.
    Guessed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigLocation {
    pub path: PathBuf,
    pub source: LocationSource,
    /// True when the path is known to be the one the running Niri uses.
    pub verified: bool,
    pub note: Option<String>,
}

/// Candidate file contents keyed by path, used to evaluate a plan before writing.
pub type Overrides = HashMap<PathBuf, String>;

pub fn hash_bytes(b: &[u8]) -> String {
    let d = Sha256::digest(b);
    d.iter().map(|x| format!("{x:02x}")).collect()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Niri's default lookup: `$XDG_CONFIG_HOME/niri/config.kdl` (or
/// `~/.config/niri/config.kdl`), then `/etc/niri/config.kdl`.
fn default_lookup(env: &HashMap<String, String>) -> PathBuf {
    let config_home = env
        .get("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| env.get("HOME").map(|h| Path::new(h).join(".config")));
    if let Some(dir) = config_home {
        let p = dir.join("niri/config.kdl");
        if p.exists() {
            return p;
        }
        let sys = PathBuf::from("/etc/niri/config.kdl");
        if sys.exists() {
            return sys;
        }
        return p;
    }
    PathBuf::from("/etc/niri/config.kdl")
}

fn process_env(pid: u32) -> Option<HashMap<String, String>> {
    let raw = fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(
        raw.split(|b| *b == 0)
            .filter_map(|kv| {
                let s = String::from_utf8_lossy(kv);
                let (k, v) = s.split_once('=')?;
                Some((k.to_string(), v.to_string()))
            })
            .collect(),
    )
}

/// `-c PATH`, `-cPATH`, `--config PATH` or `--config=PATH` from argv.
pub fn config_from_argv(args: &[String]) -> Option<String> {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--" {
            break;
        }
        if a == "-c" || a == "--config" {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix("--config=") {
            return Some(v.to_string());
        }
        if let Some(v) = a.strip_prefix("-c").filter(|v| !v.is_empty() && !a.starts_with("--")) {
            return Some(v.to_string());
        }
    }
    None
}

/// The pid embedded in `$NIRI_SOCKET` (`niri.<display>.<pid>.sock`).
pub fn niri_pid_from_socket(sock: &str) -> Option<u32> {
    let name = Path::new(sock).file_name()?.to_str()?;
    let stem = name.strip_suffix(".sock")?;
    stem.rsplit('.').next()?.parse().ok()
}

/// Resolve the configuration path, preferring facts about the running Niri.
pub fn locate(explicit: Option<&Path>) -> ConfigLocation {
    if let Some(p) = explicit {
        return ConfigLocation { path: p.to_path_buf(), source: LocationSource::Explicit, verified: true, note: None };
    }
    let pid = std::env::var("NIRI_SOCKET").ok().as_deref().and_then(niri_pid_from_socket);
    if let Some(pid) = pid {
        let comm = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        let cmdline = fs::read(format!("/proc/{pid}/cmdline")).ok();
        let env = process_env(pid);
        if comm.trim() == "niri"
            && let (Some(cmdline), Some(env)) = (cmdline, env)
        {
            let args: Vec<String> =
                cmdline.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).into_owned()).collect();
            let cwd = fs::read_link(format!("/proc/{pid}/cwd")).unwrap_or_else(|_| PathBuf::from("/"));
            if let Some(p) = config_from_argv(&args) {
                return ConfigLocation { path: cwd.join(p), source: LocationSource::RunningInstance { pid }, verified: true, note: None };
            }
            if let Some(p) = env.get("NIRI_CONFIG").filter(|s| !s.is_empty()) {
                return ConfigLocation { path: cwd.join(p), source: LocationSource::RunningInstance { pid }, verified: true, note: None };
            }
            return ConfigLocation {
                path: default_lookup(&env),
                source: LocationSource::RunningDefault { pid },
                verified: true,
                note: None,
            };
        }
    }
    let env: HashMap<String, String> = std::env::vars().collect();
    let path = env.get("NIRI_CONFIG").filter(|s| !s.is_empty()).map(PathBuf::from).unwrap_or_else(|| default_lookup(&env));
    ConfigLocation {
        path,
        source: LocationSource::Guessed,
        verified: false,
        note: Some(
            "Could not inspect the running Niri process; this is Niri's default lookup applied to \
             Keymap's environment. Set the path explicitly in Settings if it is wrong."
                .into(),
        ),
    }
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Path as Niri resolves it (may be a symlink).
    pub path: PathBuf,
    /// Final target after following symlinks; writes go here.
    pub real_path: PathBuf,
    pub symlink: bool,
    pub text: String,
    pub hash: String,
    pub writable: bool,
    pub mode: u32,
    pub doc: Option<Document>,
    pub parse_error: Option<ParseError>,
}

impl SourceFile {
    pub fn read(path: &Path) -> std::io::Result<SourceFile> {
        Self::read_with(path, &Overrides::new())
    }

    /// Read a file, substituting candidate text from `overrides` (keyed by the
    /// path Niri resolves) while keeping the real file's metadata.
    pub fn read_with(path: &Path, overrides: &Overrides) -> std::io::Result<SourceFile> {
        let text = match overrides.get(path) {
            Some(t) => t.clone(),
            None => String::from_utf8(fs::read(path)?)
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "file is not valid UTF-8"))?,
        };
        let symlink = fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        let real_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let meta = fs::metadata(&real_path)?;
        let writable = is_writable(&real_path, &meta);
        let (doc, parse_error) = match kdl::parse(&text) {
            Ok(d) => (Some(d), None),
            Err(e) => (None, Some(e)),
        };
        Ok(SourceFile {
            path: path.to_path_buf(),
            real_path,
            symlink,
            hash: hash_bytes(text.as_bytes()),
            text,
            writable,
            mode: meta.mode() & 0o7777,
            doc,
            parse_error,
        })
    }

    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        kdl::line_col(&self.text, offset)
    }
}

/// Keymap only edits files owned by the current user in writable directories
/// (atomic replacement needs to create a sibling temporary file).
fn is_writable(real: &Path, meta: &fs::Metadata) -> bool {
    let uid = unsafe_uid();
    if meta.uid() != uid || meta.mode() & 0o200 == 0 {
        return false;
    }
    let Some(dir) = real.parent() else { return false };
    match fs::metadata(dir) {
        Ok(d) => d.uid() == uid && d.mode() & 0o200 != 0,
        Err(_) => false,
    }
}

fn unsafe_uid() -> u32 {
    // The owner of our own /proc entry is our real uid; avoids a libc dependency.
    fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub message: String,
    /// Errors make Niri reject the configuration; Keymap goes read-only.
    pub error: bool,
}

#[derive(Debug, Clone)]
pub struct IncludeRef {
    pub from_file: usize,
    /// The path argument entry (patched when staging).
    pub path_span: Span,
    pub raw: String,
    pub resolved: PathBuf,
    pub optional: bool,
    pub file: Option<usize>,
}

/// A top-level node in Niri's merged order, with the file it came from.
#[derive(Debug, Clone, Copy)]
pub struct TopNode {
    pub file: usize,
    pub index: usize,
}

#[derive(Debug, Clone, Default)]
pub struct XkbSettings {
    pub rules: String,
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: Option<String>,
    pub file: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub location: ConfigLocation,
    pub files: Vec<SourceFile>,
    pub includes: Vec<IncludeRef>,
    /// Top-level nodes in effective order (includes expanded in place).
    pub order: Vec<TopNode>,
    pub diagnostics: Vec<Diagnostic>,
    pub mod_key: ModKey,
    pub xkb: XkbSettings,
    pub blur_off: bool,
    /// The last top-level `blur` section (the one whose values win), if any.
    pub blur_node: Option<TopNode>,
    /// Its `passes` and `offset`, as written (Niri's defaults apply when absent).
    pub blur_passes: Option<String>,
    pub blur_offset: Option<String>,
}

impl LoadedConfig {
    pub fn node(&self, t: TopNode) -> &Node {
        &self.files[t.file].doc.as_ref().expect("ordered nodes come from parsed files").nodes[t.index]
    }
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.error)
    }
    pub fn file_index(&self, path: &Path) -> Option<usize> {
        self.files.iter().position(|f| f.path == path)
    }
    pub fn diag(&mut self, file: usize, offset: usize, message: impl Into<String>, error: bool) {
        let (line, column) = self.files[file].line_col(offset);
        let path = self.files[file].path.clone();
        self.diagnostics.push(Diagnostic { path, line, column, message: message.into(), error });
    }
}

pub fn expand_include(raw: &str, base: &Path) -> PathBuf {
    let p = Path::new(raw);
    if let Ok(rest) = p.strip_prefix("~")
        && let Some(home) = home_dir()
    {
        return home.join(rest);
    }
    base.join(p)
}

/// Load the root file and its include graph.
pub fn load(location: ConfigLocation) -> LoadedConfig {
    load_with(location, &Overrides::new())
}

/// Load with some files replaced by candidate text (nothing is written).
pub fn load_with(location: ConfigLocation, overrides: &Overrides) -> LoadedConfig {
    let mut cfg = LoadedConfig {
        location: location.clone(),
        files: Vec::new(),
        includes: Vec::new(),
        order: Vec::new(),
        diagnostics: Vec::new(),
        mod_key: ModKey::Super,
        xkb: XkbSettings::default(),
        blur_off: false,
        blur_node: None,
        blur_passes: None,
        blur_offset: None,
    };
    match SourceFile::read_with(&location.path, overrides) {
        Ok(f) => {
            cfg.files.push(f);
            walk(&mut cfg, 0, &mut vec![location.path.clone()], overrides);
        }
        Err(e) => cfg.diagnostics.push(Diagnostic {
            path: location.path.clone(),
            line: 0,
            column: 0,
            message: format!("cannot read configuration: {e}"),
            error: true,
        }),
    }
    scan_settings(&mut cfg);
    cfg
}

fn walk(cfg: &mut LoadedConfig, file: usize, stack: &mut Vec<PathBuf>, overrides: &Overrides) {
    if let Some(e) = cfg.files[file].parse_error.clone() {
        cfg.diag(file, e.offset, format!("KDL syntax error: {}", e.message), true);
        return;
    }
    let base = cfg.files[file].path.parent().map(Path::to_path_buf).unwrap_or_default();
    let n = cfg.files[file].doc.as_ref().map_or(0, |d| d.nodes.len());
    for index in 0..n {
        let node = cfg.files[file].doc.as_ref().unwrap().nodes[index].clone();
        if node.disabled {
            continue;
        }
        if node.name != "include" {
            cfg.order.push(TopNode { file, index });
            continue;
        }
        let Some(arg) = node.args().next().cloned() else {
            cfg.diag(file, node.span.start, "include needs a path argument", true);
            continue;
        };
        let Some(raw) = arg.value.as_str().map(str::to_string) else {
            cfg.diag(file, arg.span.start, "include path must be a string", true);
            continue;
        };
        let optional = matches!(node.prop("optional").map(|e| &e.value), Some(kdl::Value::Bool(true)));
        let resolved = expand_include(&raw, &base);
        let mut inc = IncludeRef { from_file: file, path_span: arg.span, raw, resolved: resolved.clone(), optional, file: None };
        if stack.len() >= RECURSION_LIMIT {
            cfg.diag(file, node.span.start, "include recursion limit reached", true);
            cfg.includes.push(inc);
            continue;
        }
        if stack.contains(&resolved) {
            cfg.diag(file, node.span.start, "recursive include (file includes itself)", true);
            cfg.includes.push(inc);
            continue;
        }
        match SourceFile::read_with(&resolved, overrides) {
            Ok(sf) => {
                cfg.files.push(sf);
                let idx = cfg.files.len() - 1;
                inc.file = Some(idx);
                cfg.includes.push(inc);
                stack.push(resolved);
                walk(cfg, idx, stack, overrides);
                stack.pop();
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && optional => {
                cfg.includes.push(inc);
            }
            Err(e) => {
                cfg.diag(file, node.span.start, format!("cannot read include {}: {e}", resolved.display()), true);
                cfg.includes.push(inc);
            }
        }
    }
}

fn string_arg(n: &Node) -> Option<String> {
    n.args().next().and_then(|e| e.value.as_str()).map(str::to_string)
}

fn scan_settings(cfg: &mut LoadedConfig) {
    let mut mod_key = None;
    let mut xkb = XkbSettings::default();
    let mut blur_off = false;
    let (mut blur_node, mut passes, mut offset) = (None, None, None);
    for t in cfg.order.clone() {
        let node = cfg.node(t).clone();
        match node.name.as_str() {
            "input" => {
                for c in node.child_nodes() {
                    match c.name.as_str() {
                        "mod-key" => match string_arg(c).as_deref().and_then(ModKey::parse) {
                            Some(m) => mod_key = Some(m),
                            None => cfg.diag(t.file, c.span.start, "invalid mod-key", true),
                        },
                        "keyboard" => {
                            for x in c.child_nodes().filter(|n| n.name == "xkb") {
                                for f in x.child_nodes() {
                                    let v = string_arg(f).unwrap_or_default();
                                    match f.name.as_str() {
                                        "rules" => xkb.rules = v,
                                        "model" => xkb.model = v,
                                        "layout" => xkb.layout = v,
                                        "variant" => xkb.variant = v,
                                        "options" => xkb.options = Some(v),
                                        "file" => xkb.file = Some(v),
                                        _ => {}
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "blur" => {
                blur_node = Some(t);
                for c in node.child_nodes() {
                    let number = || match c.args().next().map(|e| &e.value) {
                        Some(kdl::Value::Number(n)) => Some(n.clone()),
                        _ => None,
                    };
                    match c.name.as_str() {
                        "off" => blur_off = !matches!(c.args().next().map(|e| &e.value), Some(kdl::Value::Bool(false))),
                        "passes" => passes = number(),
                        "offset" => offset = number(),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    cfg.mod_key = mod_key.unwrap_or(ModKey::Super);
    cfg.xkb = xkb;
    cfg.blur_off = blur_off;
    cfg.blur_node = blur_node;
    cfg.blur_passes = passes;
    cfg.blur_offset = offset;
}

/// Effective background effect for a layer-shell namespace after applying
/// every matching `layer-rule` in order (later rules override earlier ones).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LayerEffect {
    pub blur: Option<bool>,
    pub xray: Option<bool>,
    /// Files/lines of the rules that set these values.
    pub sources: Vec<String>,
}

pub fn layer_effect(cfg: &LoadedConfig, namespace: &str, layer: &str) -> LayerEffect {
    let mut eff = LayerEffect::default();
    for t in &cfg.order {
        let node = cfg.node(*t);
        if node.name != "layer-rule" {
            continue;
        }
        let matches: Vec<&Node> = node.child_nodes().filter(|c| c.name == "match").collect();
        let excludes: Vec<&Node> = node.child_nodes().filter(|c| c.name == "exclude").collect();
        let hit = |m: &Node| {
            m.props().all(|p| match (p.name.as_deref(), &p.value) {
                (Some("namespace"), kdl::Value::String(re)) => regex::Regex::new(re).map(|r| r.is_match(namespace)).unwrap_or(false),
                (Some("layer"), kdl::Value::String(l)) => l == layer,
                // Other matchers (e.g. at-startup) cannot be evaluated statically; assume a match.
                _ => true,
            })
        };
        let included = matches.is_empty() || matches.iter().any(|m| hit(m));
        let excluded = excludes.iter().any(|m| m.props().count() > 0 && hit(m));
        if !included || excluded {
            continue;
        }
        for be in node.child_nodes().filter(|c| c.name == "background-effect") {
            for f in be.child_nodes() {
                let v = match f.args().next().map(|e| &e.value) {
                    Some(kdl::Value::Bool(b)) => *b,
                    None => true,
                    _ => continue,
                };
                let (line, _) = cfg.files[t.file].line_col(f.span.start);
                let src = format!("{}:{}", cfg.files[t.file].path.display(), line);
                match f.name.as_str() {
                    "blur" => {
                        eff.blur = Some(v);
                        eff.sources.push(src);
                    }
                    "xray" => {
                        eff.xray = Some(v);
                        eff.sources.push(src);
                    }
                    _ => {}
                }
            }
        }
    }
    eff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_and_socket_parsing() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(config_from_argv(&a(&["niri", "-c", "/x.kdl"])), Some("/x.kdl".into()));
        assert_eq!(config_from_argv(&a(&["niri", "--config=/y"])), Some("/y".into()));
        assert_eq!(config_from_argv(&a(&["niri", "--session"])), None);
        assert_eq!(niri_pid_from_socket("/run/user/1000/niri.wayland-1.1476.sock"), Some(1476));
    }
}
