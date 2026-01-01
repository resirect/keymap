//! Niri actions usable in `binds {}`, transcribed from the `Action` enum of
//! niri-config at tag v26.04 (variants marked `#[knuffel(skip)]` are IPC-only
//! and excluded). Labels and categories are Keymap's own display metadata.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// `spawn "prog" "arg"...`: executable plus argv, no shell.
    Argv,
    /// `spawn-sh "cmd"`: explicit shell command line.
    Shell,
    Int {
        min: i64,
        max: i64,
    },
    Text,
    /// Workspace index (number) or name (string).
    Workspace,
    /// Niri SizeChange, e.g. `"+10%"`, `"-10"`, `"50%"`, `"1200"`.
    Size,
    Choice(&'static [&'static str]),
    Bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    /// `Some(name)` for a property, `None` for a positional argument.
    pub prop: Option<&'static str>,
    pub label: &'static str,
    pub kind: ParamKind,
    pub required: bool,
    /// Niri's default when the property is omitted (for payload comparison).
    pub default: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub struct ActionDef {
    pub name: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub params: &'static [ParamSpec],
}

const fn arg(label: &'static str, kind: ParamKind) -> ParamSpec {
    ParamSpec { prop: None, label, kind, required: true, default: None }
}
const fn prop(name: &'static str, label: &'static str, kind: ParamKind, default: Option<&'static str>) -> ParamSpec {
    ParamSpec { prop: Some(name), label, kind, required: false, default }
}

const NONE: &[ParamSpec] = &[];
const INDEX: &[ParamSpec] = &[arg("Index", ParamKind::Int { min: 1, max: 255 })];
const OUTPUT: &[ParamSpec] = &[arg("Output name", ParamKind::Text)];
const SIZE: &[ParamSpec] = &[arg("Size change", ParamKind::Size)];
const FOCUS: &[ParamSpec] = &[prop("focus", "Follow the window", ParamKind::Bool, Some("true"))];
const WS_FOCUS: &[ParamSpec] = &[arg("Workspace", ParamKind::Workspace), prop("focus", "Follow the window", ParamKind::Bool, Some("true"))];

macro_rules! a {
    ($name:literal, $label:literal, $cat:literal) => {
        ActionDef { name: $name, label: $label, category: $cat, params: NONE }
    };
    ($name:literal, $label:literal, $cat:literal, $params:expr) => {
        ActionDef { name: $name, label: $label, category: $cat, params: $params }
    };
}

pub const APPS: &str = "Applications";
pub const COMMANDS: &str = "Commands";

pub static ACTIONS: &[ActionDef] = &[
    a!("spawn", "Run a program", "Applications", &[arg("Command", ParamKind::Argv)]),
    a!("spawn-sh", "Run a shell command", "Commands", &[arg("Shell command", ParamKind::Shell)]),
    a!("quit", "Quit Niri", "Session", &[prop("skip-confirmation", "Skip confirmation", ParamKind::Bool, Some("false"))]),
    a!("suspend", "Suspend", "Session"),
    a!("power-off-monitors", "Power off monitors", "Session"),
    a!("power-on-monitors", "Power on monitors", "Session"),
    a!("toggle-keyboard-shortcuts-inhibit", "Toggle shortcut inhibition", "Session"),
    a!("show-hotkey-overlay", "Show hotkey overlay", "Session"),
    a!(
        "do-screen-transition",
        "Screen transition",
        "Session",
        &[prop("delay-ms", "Delay (ms)", ParamKind::Int { min: 0, max: 65535 }, None)]
    ),
    a!("screenshot", "Take a screenshot", "Screenshots", &[prop("show-pointer", "Show pointer", ParamKind::Bool, Some("true"))]),
    a!(
        "screenshot-screen",
        "Screenshot the screen",
        "Screenshots",
        &[
            prop("write-to-disk", "Write to disk", ParamKind::Bool, Some("true")),
            prop("show-pointer", "Show pointer", ParamKind::Bool, Some("true")),
        ]
    ),
    a!(
        "screenshot-window",
        "Screenshot the window",
        "Screenshots",
        &[
            prop("write-to-disk", "Write to disk", ParamKind::Bool, Some("true")),
            prop("show-pointer", "Show pointer", ParamKind::Bool, Some("false")),
        ]
    ),
    a!("close-window", "Close window", "Windows"),
    a!("fullscreen-window", "Fullscreen window", "Windows"),
    a!("toggle-windowed-fullscreen", "Toggle windowed fullscreen", "Windows"),
    a!("focus-window-in-column", "Focus window in column", "Windows", INDEX),
    a!("focus-window-previous", "Focus previous window", "Windows"),
    a!("focus-window-down", "Focus window down", "Windows"),
    a!("focus-window-up", "Focus window up", "Windows"),
    a!("focus-window-down-or-column-left", "Focus window down or column left", "Windows"),
    a!("focus-window-down-or-column-right", "Focus window down or column right", "Windows"),
    a!("focus-window-up-or-column-left", "Focus window up or column left", "Windows"),
    a!("focus-window-up-or-column-right", "Focus window up or column right", "Windows"),
    a!("focus-window-or-monitor-up", "Focus window or monitor up", "Windows"),
    a!("focus-window-or-monitor-down", "Focus window or monitor down", "Windows"),
    a!("focus-window-or-workspace-down", "Focus window or workspace down", "Windows"),
    a!("focus-window-or-workspace-up", "Focus window or workspace up", "Windows"),
    a!("focus-window-top", "Focus top window", "Windows"),
    a!("focus-window-bottom", "Focus bottom window", "Windows"),
    a!("focus-window-down-or-top", "Focus window down or top", "Windows"),
    a!("focus-window-up-or-bottom", "Focus window up or bottom", "Windows"),
    a!("move-window-down", "Move window down", "Windows"),
    a!("move-window-up", "Move window up", "Windows"),
    a!("move-window-down-or-to-workspace-down", "Move window down or to workspace below", "Windows"),
    a!("move-window-up-or-to-workspace-up", "Move window up or to workspace above", "Windows"),
    a!("consume-or-expel-window-left", "Consume or expel window left", "Windows"),
    a!("consume-or-expel-window-right", "Consume or expel window right", "Windows"),
    a!("consume-window-into-column", "Consume window into column", "Windows"),
    a!("expel-window-from-column", "Expel window from column", "Windows"),
    a!("swap-window-left", "Swap window left", "Windows"),
    a!("swap-window-right", "Swap window right", "Windows"),
    a!("center-window", "Center window", "Windows"),
    a!("toggle-window-floating", "Toggle floating", "Floating"),
    a!("move-window-to-floating", "Move window to floating", "Floating"),
    a!("move-window-to-tiling", "Move window to tiling", "Floating"),
    a!("focus-floating", "Focus floating windows", "Floating"),
    a!("focus-tiling", "Focus tiled windows", "Floating"),
    a!("switch-focus-between-floating-and-tiling", "Switch focus floating/tiling", "Floating"),
    a!("toggle-window-rule-opacity", "Toggle window-rule opacity", "Windows"),
    a!("focus-column-left", "Focus column left", "Columns"),
    a!("focus-column-right", "Focus column right", "Columns"),
    a!("focus-column-first", "Focus first column", "Columns"),
    a!("focus-column-last", "Focus last column", "Columns"),
    a!("focus-column-right-or-first", "Focus column right or first", "Columns"),
    a!("focus-column-left-or-last", "Focus column left or last", "Columns"),
    a!("focus-column", "Focus column", "Columns", INDEX),
    a!("focus-column-or-monitor-left", "Focus column or monitor left", "Columns"),
    a!("focus-column-or-monitor-right", "Focus column or monitor right", "Columns"),
    a!("move-column-left", "Move column left", "Columns"),
    a!("move-column-right", "Move column right", "Columns"),
    a!("move-column-to-first", "Move column to first", "Columns"),
    a!("move-column-to-last", "Move column to last", "Columns"),
    a!("move-column-left-or-to-monitor-left", "Move column left or to monitor", "Columns"),
    a!("move-column-right-or-to-monitor-right", "Move column right or to monitor", "Columns"),
    a!("move-column-to-index", "Move column to index", "Columns", INDEX),
    a!("toggle-column-tabbed-display", "Toggle tabbed column", "Columns"),
    a!("set-column-display", "Set column display", "Columns", &[arg("Display", ParamKind::Choice(&["normal", "tabbed"]))]),
    a!("center-column", "Center column", "Columns"),
    a!("center-visible-columns", "Center visible columns", "Columns"),
    a!("maximize-column", "Maximize column", "Sizing"),
    a!("maximize-window-to-edges", "Maximize window to edges", "Sizing"),
    a!("expand-column-to-available-width", "Expand column to available width", "Sizing"),
    a!("set-column-width", "Set column width", "Sizing", SIZE),
    a!("set-window-width", "Set window width", "Sizing", SIZE),
    a!("set-window-height", "Set window height", "Sizing", SIZE),
    a!("reset-window-height", "Reset window height", "Sizing"),
    a!("switch-preset-column-width", "Next preset column width", "Sizing"),
    a!("switch-preset-column-width-back", "Previous preset column width", "Sizing"),
    a!("switch-preset-window-width", "Next preset window width", "Sizing"),
    a!("switch-preset-window-width-back", "Previous preset window width", "Sizing"),
    a!("switch-preset-window-height", "Next preset window height", "Sizing"),
    a!("switch-preset-window-height-back", "Previous preset window height", "Sizing"),
    a!("focus-workspace-down", "Focus workspace below", "Workspaces"),
    a!("focus-workspace-up", "Focus workspace above", "Workspaces"),
    a!("focus-workspace", "Focus workspace", "Workspaces", &[arg("Workspace", ParamKind::Workspace)]),
    a!("focus-workspace-previous", "Focus previous workspace", "Workspaces"),
    a!("move-window-to-workspace-down", "Move window to workspace below", "Workspaces", FOCUS),
    a!("move-window-to-workspace-up", "Move window to workspace above", "Workspaces", FOCUS),
    a!("move-window-to-workspace", "Move window to workspace", "Workspaces", WS_FOCUS),
    a!("move-column-to-workspace-down", "Move column to workspace below", "Workspaces", FOCUS),
    a!("move-column-to-workspace-up", "Move column to workspace above", "Workspaces", FOCUS),
    a!("move-column-to-workspace", "Move column to workspace", "Workspaces", WS_FOCUS),
    a!("move-workspace-down", "Move workspace down", "Workspaces"),
    a!("move-workspace-up", "Move workspace up", "Workspaces"),
    a!("move-workspace-to-index", "Move workspace to index", "Workspaces", INDEX),
    a!("set-workspace-name", "Set workspace name", "Workspaces", &[arg("Name", ParamKind::Text)]),
    a!("unset-workspace-name", "Unset workspace name", "Workspaces"),
    a!("focus-monitor-left", "Focus monitor left", "Monitors"),
    a!("focus-monitor-right", "Focus monitor right", "Monitors"),
    a!("focus-monitor-down", "Focus monitor down", "Monitors"),
    a!("focus-monitor-up", "Focus monitor up", "Monitors"),
    a!("focus-monitor-previous", "Focus previous monitor", "Monitors"),
    a!("focus-monitor-next", "Focus next monitor", "Monitors"),
    a!("focus-monitor", "Focus monitor", "Monitors", OUTPUT),
    a!("move-window-to-monitor-left", "Move window to monitor left", "Monitors"),
    a!("move-window-to-monitor-right", "Move window to monitor right", "Monitors"),
    a!("move-window-to-monitor-down", "Move window to monitor down", "Monitors"),
    a!("move-window-to-monitor-up", "Move window to monitor up", "Monitors"),
    a!("move-window-to-monitor-previous", "Move window to previous monitor", "Monitors"),
    a!("move-window-to-monitor-next", "Move window to next monitor", "Monitors"),
    a!("move-window-to-monitor", "Move window to monitor", "Monitors", OUTPUT),
    a!("move-column-to-monitor-left", "Move column to monitor left", "Monitors"),
    a!("move-column-to-monitor-right", "Move column to monitor right", "Monitors"),
    a!("move-column-to-monitor-down", "Move column to monitor down", "Monitors"),
    a!("move-column-to-monitor-up", "Move column to monitor up", "Monitors"),
    a!("move-column-to-monitor-previous", "Move column to previous monitor", "Monitors"),
    a!("move-column-to-monitor-next", "Move column to next monitor", "Monitors"),
    a!("move-column-to-monitor", "Move column to monitor", "Monitors", OUTPUT),
    a!("move-workspace-to-monitor-left", "Move workspace to monitor left", "Monitors"),
    a!("move-workspace-to-monitor-right", "Move workspace to monitor right", "Monitors"),
    a!("move-workspace-to-monitor-down", "Move workspace to monitor down", "Monitors"),
    a!("move-workspace-to-monitor-up", "Move workspace to monitor up", "Monitors"),
    a!("move-workspace-to-monitor-previous", "Move workspace to previous monitor", "Monitors"),
    a!("move-workspace-to-monitor-next", "Move workspace to next monitor", "Monitors"),
    a!("move-workspace-to-monitor", "Move workspace to monitor", "Monitors", OUTPUT),
    a!("switch-layout", "Switch keyboard layout", "Keyboard", &[arg("Target", ParamKind::Choice(&["next", "prev", "0", "1", "2", "3"]))]),
    a!("toggle-overview", "Toggle overview", "Overview"),
    a!("open-overview", "Open overview", "Overview"),
    a!("close-overview", "Close overview", "Overview"),
    a!("set-dynamic-cast-window", "Cast focused window", "Screencast"),
    a!(
        "set-dynamic-cast-monitor",
        "Cast monitor",
        "Screencast",
        &[ParamSpec { prop: None, label: "Output name (optional)", kind: ParamKind::Text, required: false, default: None }]
    ),
    a!("clear-dynamic-cast-target", "Clear cast target", "Screencast"),
    a!("toggle-debug-tint", "Toggle debug tint", "Debug"),
    a!("debug-toggle-opaque-regions", "Toggle opaque-region debug", "Debug"),
    a!("debug-toggle-damage", "Toggle damage debug", "Debug"),
];

pub fn find(name: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|d| d.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_and_kebab_case() {
        let mut seen = HashSet::new();
        for d in ACTIONS {
            assert!(seen.insert(d.name), "duplicate {}", d.name);
            assert!(d.name.chars().all(|c| c.is_ascii_lowercase() || c == '-'), "{}", d.name);
        }
        assert!(ACTIONS.len() > 120);
    }
}
