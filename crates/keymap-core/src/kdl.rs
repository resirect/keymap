//! A small lossless KDL v1 reader.
//!
//! Niri parses its configuration with knuffel (KDL v1). Keymap never
//! re-serialises a parsed document: it only records byte spans so edits can
//! patch the original text and leave every other byte untouched. The parser
//! therefore keeps spans for nodes, names, entries and children blocks, and
//! marks slash-dashed (`/-`) items as disabled instead of dropping them.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
    pub fn slice<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start..self.end]
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    String(String),
    /// Numbers are kept as their source text; Keymap never needs arithmetic.
    Number(String),
    Bool(bool),
    Null,
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Canonical text used for payload comparison (not for writing).
    pub fn canonical(&self) -> String {
        match self {
            Value::String(s) => format!("s:{s}"),
            Value::Number(n) => format!("n:{}", normalize_number(n)),
            Value::Bool(b) => format!("b:{b}"),
            Value::Null => "null".into(),
        }
    }

    /// KDL v1 source text for this value.
    pub fn to_kdl(&self) -> String {
        match self {
            Value::String(s) => quote(s),
            Value::Number(n) => n.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".into(),
        }
    }
}

fn normalize_number(n: &str) -> String {
    let n = n.replace('_', "");
    if let Some(h) = n.strip_prefix("0x")
        && let Ok(v) = i128::from_str_radix(h, 16)
    {
        return v.to_string();
    }
    if let Some(o) = n.strip_prefix("0o")
        && let Ok(v) = i128::from_str_radix(o, 8)
    {
        return v.to_string();
    }
    if let Some(b) = n.strip_prefix("0b")
        && let Ok(v) = i128::from_str_radix(b, 2)
    {
        return v.to_string();
    }
    if let Ok(v) = n.parse::<i128>() {
        return v.to_string();
    }
    n
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// `Some(key)` for a property (`key=value`), `None` for an argument.
    pub name: Option<String>,
    pub ty: Option<String>,
    pub value: Value,
    pub span: Span,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Children {
    /// From `{` to `}` inclusive (and a leading `/-` when disabled).
    pub span: Span,
    pub nodes: Vec<Node>,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// From the first byte of the node (including `/-`) to the end of its last token.
    pub span: Span,
    /// `span` extended over a trailing `;` if present.
    pub full_span: Span,
    pub disabled: bool,
    pub ty: Option<String>,
    pub name: String,
    pub name_span: Span,
    pub entries: Vec<Entry>,
    pub children: Vec<Children>,
}

impl Node {
    pub fn args(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.name.is_none() && !e.disabled)
    }
    pub fn props(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.name.is_some() && !e.disabled)
    }
    pub fn prop(&self, key: &str) -> Option<&Entry> {
        // In KDL the last occurrence of a property wins.
        self.props().filter(|e| e.name.as_deref() == Some(key)).last()
    }
    /// Enabled child nodes across all enabled children blocks.
    pub fn child_nodes(&self) -> impl Iterator<Item = &Node> {
        self.children.iter().filter(|c| !c.disabled).flat_map(|c| c.nodes.iter()).filter(|n| !n.disabled)
    }
    pub fn children_block(&self) -> Option<&Children> {
        self.children.iter().find(|c| !c.disabled)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub nodes: Vec<Node>,
}

impl Document {
    pub fn enabled(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|n| !n.disabled)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

/// 1-based line and column of a byte offset.
pub fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

pub fn parse(text: &str) -> Result<Document, ParseError> {
    let mut p = Parser { s: text, b: text.as_bytes(), i: 0 };
    let nodes = p.nodes(false)?;
    p.skip_line_space()?;
    if p.i < p.b.len() {
        return Err(p.err("unexpected input"));
    }
    Ok(Document { nodes })
}

/// Quote a string as a KDL v1 escaped string.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Format an identifier, quoting it when it is not a valid bare identifier.
pub fn ident(s: &str) -> String {
    let bare = !s.is_empty() && s.chars().all(is_ident_char) && !starts_like_number(s) && !matches!(s, "true" | "false" | "null");
    if bare { s.to_string() } else { quote(s) }
}

fn is_ident_char(c: char) -> bool {
    !(c.is_whitespace()
        || c.is_control()
        || matches!(c, '\\' | '/' | '(' | ')' | '{' | '}' | '<' | '>' | ';' | '[' | ']' | '=' | ',' | '"')
        || c == '\u{FEFF}')
}

fn starts_like_number(s: &str) -> bool {
    let mut it = s.chars();
    match it.next() {
        Some(c) if c.is_ascii_digit() => true,
        Some('+' | '-') => it.next().is_some_and(|c| c.is_ascii_digit()),
        _ => false,
    }
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, msg: impl Into<String>) -> ParseError {
        let (line, column) = line_col(self.s, self.i);
        ParseError { offset: self.i, line, column, message: msg.into() }
    }

    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn starts(&self, pat: &str) -> bool {
        self.s[self.i..].starts_with(pat)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }

    fn is_newline(c: char) -> bool {
        matches!(c, '\n' | '\r' | '\u{85}' | '\u{0C}' | '\u{2028}' | '\u{2029}')
    }

    fn is_ws(c: char) -> bool {
        matches!(c, '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
    }

    fn block_comment(&mut self) -> Result<(), ParseError> {
        // Assumes we are at "/*". Block comments nest in KDL.
        let start = self.i;
        self.i += 2;
        let mut depth = 1;
        while depth > 0 {
            if self.starts("/*") {
                depth += 1;
                self.i += 2;
            } else if self.starts("*/") {
                depth -= 1;
                self.i += 2;
            } else if self.bump().is_none() {
                self.i = start;
                return Err(self.err("unterminated block comment"));
            }
        }
        Ok(())
    }

    fn line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if Self::is_newline(c) {
                break;
            }
            self.bump();
        }
    }

    /// Whitespace, block comments and line continuations within a node.
    fn skip_node_space(&mut self) -> Result<bool, ParseError> {
        let start = self.i;
        loop {
            match self.peek() {
                Some(c) if Self::is_ws(c) => {
                    self.bump();
                }
                Some('/') if self.starts("/*") => self.block_comment()?,
                Some('\\') => {
                    // Escaped line: `\` ws* (comment)? newline
                    let save = self.i;
                    self.bump();
                    while let Some(c) = self.peek() {
                        if Self::is_ws(c) {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    if self.starts("//") {
                        self.line_comment();
                    }
                    match self.peek() {
                        Some(c) if Self::is_newline(c) => {
                            self.eat_newline();
                        }
                        None => {}
                        _ => {
                            self.i = save;
                            return Err(self.err("expected newline after line continuation"));
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(self.i > start)
    }

    fn eat_newline(&mut self) {
        if self.starts("\r\n") {
            self.i += 2;
        } else {
            self.bump();
        }
    }

    /// Whitespace, newlines and comments between nodes.
    fn skip_line_space(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek() {
                Some(c) if Self::is_ws(c) || Self::is_newline(c) => {
                    self.bump();
                }
                Some('/') if self.starts("//") => self.line_comment(),
                Some('/') if self.starts("/*") => self.block_comment()?,
                _ => return Ok(()),
            }
        }
    }

    fn nodes(&mut self, in_block: bool) -> Result<Vec<Node>, ParseError> {
        let mut out = Vec::new();
        loop {
            self.skip_line_space()?;
            match self.peek() {
                None => {
                    if in_block {
                        return Err(self.err("unclosed children block, expected `}`"));
                    }
                    return Ok(out);
                }
                Some('}') if in_block => return Ok(out),
                Some(';') => {
                    self.bump();
                }
                _ => out.push(self.node()?),
            }
        }
    }

    fn node(&mut self) -> Result<Node, ParseError> {
        let start = self.i;
        let mut disabled = false;
        if self.starts("/-") {
            disabled = true;
            self.i += 2;
            self.skip_node_space()?;
        }
        let ty = self.type_annotation()?;
        let name_start = self.i;
        let name = self.identifier()?;
        let name_span = Span::new(name_start, self.i);
        let mut entries = Vec::new();
        let mut children = Vec::new();
        let mut end = self.i;
        loop {
            let had_space = self.skip_node_space()?;
            let Some(c) = self.peek() else { break };
            if Self::is_newline(c) || c == ';' || c == '}' {
                break;
            }
            if self.starts("//") {
                break;
            }
            let item_start = self.i;
            let mut item_disabled = false;
            if self.starts("/-") {
                item_disabled = true;
                self.i += 2;
                self.skip_node_space()?;
            }
            if self.peek() == Some('{') {
                let nodes_start = self.i;
                self.bump();
                let nodes = self.nodes(true)?;
                if self.peek() != Some('}') {
                    return Err(self.err("expected `}`"));
                }
                self.bump();
                let _ = nodes_start;
                children.push(Children { span: Span::new(item_start, self.i), nodes, disabled: item_disabled });
                end = self.i;
                continue;
            }
            if !children.is_empty() && !item_disabled {
                return Err(self.err("arguments and properties must come before children"));
            }
            if !had_space {
                return Err(self.err("expected whitespace before argument or property"));
            }
            let entry = self.entry(item_start, item_disabled)?;
            end = entry.span.end;
            entries.push(entry);
        }
        let mut full_end = end;
        if self.peek() == Some(';') {
            self.bump();
            full_end = self.i;
        }
        Ok(Node { span: Span::new(start, end), full_span: Span::new(start, full_end), disabled, ty, name, name_span, entries, children })
    }

    fn type_annotation(&mut self) -> Result<Option<String>, ParseError> {
        if self.peek() != Some('(') {
            return Ok(None);
        }
        self.bump();
        let t = self.identifier()?;
        if self.peek() != Some(')') {
            return Err(self.err("expected `)` after type annotation"));
        }
        self.bump();
        Ok(Some(t))
    }

    fn entry(&mut self, start: usize, disabled: bool) -> Result<Entry, ParseError> {
        let ty = self.type_annotation()?;
        // A property key is an identifier (bare or string) followed by `=`.
        if ty.is_none() {
            let save = self.i;
            let key = match self.peek() {
                Some('"') => Some(self.string()?),
                Some('r') if self.starts("r\"") || self.starts("r#") => Some(self.raw_string()?),
                Some(c) if is_ident_char(c) && !c.is_ascii_digit() => {
                    let id = self.bare_identifier();
                    if starts_like_number(&id) { None } else { Some(id) }
                }
                _ => None,
            };
            if let Some(key) = key
                && self.peek() == Some('=')
            {
                self.bump();
                let vty = self.type_annotation()?;
                let value = self.value()?;
                return Ok(Entry { name: Some(key), ty: vty, value, span: Span::new(start, self.i), disabled });
            }
            self.i = save;
        }
        let value = self.value()?;
        Ok(Entry { name: None, ty, value, span: Span::new(start, self.i), disabled })
    }

    fn value(&mut self) -> Result<Value, ParseError> {
        match self.peek() {
            Some('"') => Ok(Value::String(self.string()?)),
            Some('r') if self.starts("r\"") || self.starts("r#") => Ok(Value::String(self.raw_string()?)),
            Some(c) if c.is_ascii_digit() || ((c == '-' || c == '+') && self.next_is_digit()) => Ok(Value::Number(self.number()?)),
            Some(_) => {
                let save = self.i;
                let id = self.bare_identifier();
                match id.as_str() {
                    "true" => Ok(Value::Bool(true)),
                    "false" => Ok(Value::Bool(false)),
                    "null" => Ok(Value::Null),
                    _ => {
                        self.i = save;
                        Err(self.err("expected a value (string, number, true, false or null)"))
                    }
                }
            }
            None => Err(self.err("unexpected end of input, expected a value")),
        }
    }

    fn next_is_digit(&self) -> bool {
        self.s[self.i..].chars().nth(1).is_some_and(|c| c.is_ascii_digit())
    }

    fn number(&mut self) -> Result<String, ParseError> {
        let start = self.i;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-') {
                self.bump();
            } else {
                break;
            }
        }
        let raw = &self.s[start..self.i];
        let body = raw.trim_start_matches(['+', '-']);
        let ok = if let Some(h) = body.strip_prefix("0x") {
            !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit() || c == '_')
        } else if let Some(o) = body.strip_prefix("0o") {
            !o.is_empty() && o.chars().all(|c| matches!(c, '0'..='7' | '_'))
        } else if let Some(b) = body.strip_prefix("0b") {
            !b.is_empty() && b.chars().all(|c| matches!(c, '0' | '1' | '_'))
        } else {
            body.replace('_', "").parse::<f64>().is_ok()
        };
        if !ok {
            self.i = start;
            return Err(self.err(format!("invalid number `{raw}`")));
        }
        Ok(raw.to_string())
    }

    fn identifier(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Some('"') => self.string(),
            Some('r') if self.starts("r\"") || self.starts("r#") => self.raw_string(),
            Some(c) if is_ident_char(c) => {
                let id = self.bare_identifier();
                if starts_like_number(&id) {
                    return Err(self.err("identifier cannot start with a digit"));
                }
                Ok(id)
            }
            _ => Err(self.err("expected a node name")),
        }
    }

    fn bare_identifier(&mut self) -> String {
        let start = self.i;
        while let Some(c) = self.peek() {
            if is_ident_char(c) {
                self.bump();
            } else {
                break;
            }
        }
        self.s[start..self.i].to_string()
    }

    fn string(&mut self) -> Result<String, ParseError> {
        let start = self.i;
        self.bump(); // opening quote
        let mut out = String::new();
        loop {
            match self.bump() {
                None => {
                    self.i = start;
                    return Err(self.err("unterminated string"));
                }
                Some('"') => return Ok(out),
                Some('\\') => match self.bump() {
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('\\') => out.push('\\'),
                    Some('/') => out.push('/'),
                    Some('"') => out.push('"'),
                    Some('b') => out.push('\u{08}'),
                    Some('f') => out.push('\u{0C}'),
                    Some('u') => {
                        if self.bump() != Some('{') {
                            return Err(self.err("expected `{` in unicode escape"));
                        }
                        let hs = self.i;
                        while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                            self.bump();
                        }
                        let hex = &self.s[hs..self.i];
                        if self.bump() != Some('}') || hex.is_empty() || hex.len() > 6 {
                            return Err(self.err("invalid unicode escape"));
                        }
                        let c = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
                        out.push(c.ok_or_else(|| self.err("invalid unicode scalar"))?);
                    }
                    _ => return Err(self.err("invalid escape in string")),
                },
                Some(c) => out.push(c),
            }
        }
    }

    fn raw_string(&mut self) -> Result<String, ParseError> {
        let start = self.i;
        self.bump(); // r
        let mut hashes = 0;
        while self.peek() == Some('#') {
            hashes += 1;
            self.bump();
        }
        if self.bump() != Some('"') {
            self.i = start;
            return Err(self.err("invalid raw string"));
        }
        let close: String = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
        let body_start = self.i;
        match self.s[self.i..].find(&close) {
            Some(rel) => {
                let body = self.s[body_start..body_start + rel].to_string();
                self.i = body_start + rel + close.len();
                Ok(body)
            }
            None => {
                self.i = start;
                Err(self.err("unterminated raw string"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binds_with_spans_and_trivia() {
        let src = r##"// top comment
binds {
    Mod+T hotkey-overlay-title="Open a Terminal: alacritty" { spawn "alacritty"; }
    /-Mod+D { spawn "fuzzel"; }
    Mod+Shift+S repeat=false { screenshot show-pointer=false; }
    Print { spawn-sh r#"grim -g "$(slurp)" - | wl-copy"#; }
    Mod+1 { focus-workspace 1; } /* trailing */
}
"##;
        let doc = parse(src).unwrap();
        let binds = &doc.nodes[0];
        assert_eq!(binds.name, "binds");
        let kids: Vec<_> = binds.children[0].nodes.iter().collect();
        assert_eq!(kids.len(), 5);
        assert!(kids[1].disabled);
        assert_eq!(kids[0].name, "Mod+T");
        assert_eq!(kids[0].prop("hotkey-overlay-title").unwrap().value.as_str(), Some("Open a Terminal: alacritty"));
        let shot = &kids[2];
        assert_eq!(shot.prop("repeat").unwrap().value, Value::Bool(false));
        let act = shot.child_nodes().next().unwrap();
        assert_eq!(act.name, "screenshot");
        assert_eq!(act.span.slice(src), "screenshot show-pointer=false");
        let raw = kids[3].child_nodes().next().unwrap();
        assert_eq!(raw.args().next().unwrap().value.as_str(), Some(r#"grim -g "$(slurp)" - | wl-copy"#));
        assert_eq!(kids[4].full_span.slice(src), "Mod+1 { focus-workspace 1; }");
    }

    #[test]
    fn line_continuation_types_and_errors() {
        let src = "node 1 \\\n  2 (u8)3 key=(t)\"v\" 0x1F -2.5e3\n";
        let doc = parse(src).unwrap();
        let n = &doc.nodes[0];
        assert_eq!(n.args().count(), 5);
        assert_eq!(n.prop("key").unwrap().ty.as_deref(), Some("t"));
        assert!(parse("a {").is_err());
        assert!(parse("a \"unterminated").is_err());
        let e = parse("binds {\n  Mod+T { spawn \"x\" }\n  }}").unwrap_err();
        assert_eq!(e.line, 3);
    }

    #[test]
    fn quoting_round_trips() {
        for s in ["plain", "with \"quotes\"", "back\\slash", "tab\tnew\nline", "ünïcode"] {
            let doc = parse(&format!("n {}", quote(s))).unwrap();
            assert_eq!(doc.nodes[0].args().next().unwrap().value.as_str(), Some(s));
        }
        assert_eq!(ident("Mod+T"), "Mod+T");
        assert_eq!(ident("1"), "\"1\"");
    }
}
