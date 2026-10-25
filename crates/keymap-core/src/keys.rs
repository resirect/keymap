//! Key combinations with Niri's identity rules (niri-config `Key::from_str`,
//! niri 26.04): modifiers are case-insensitive words joined by `+`, the last
//! part is a mouse/wheel/touchpad trigger or an XKB keysym name resolved
//! case-insensitively.

use serde::{Deserialize, Serialize};
use std::fmt;
use xkbcommon::xkb;

/// Modifier bit set. `MOD` is Niri's `Mod` alias and is distinct from the
/// concrete modifier it resolves to, exactly as in Niri's own config model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, PartialOrd, Ord)]
pub struct Mods(pub u8);

impl Mods {
    pub const CTRL: Mods = Mods(1);
    pub const SHIFT: Mods = Mods(2);
    pub const ALT: Mods = Mods(4);
    pub const SUPER: Mods = Mods(8);
    pub const LEVEL3: Mods = Mods(16);
    pub const LEVEL5: Mods = Mods(32);
    pub const MOD: Mods = Mods(64);

    pub fn empty() -> Self {
        Mods(0)
    }
    pub fn contains(self, o: Mods) -> bool {
        self.0 & o.0 == o.0
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
    /// How many modifier keys are held (`Mod` counts as one).
    pub fn count(self) -> u32 {
        self.0.count_ones()
    }
    pub fn with(self, o: Mods) -> Mods {
        Mods(self.0 | o.0)
    }
    pub fn without(self, o: Mods) -> Mods {
        Mods(self.0 & !o.0)
    }
    /// Concrete modifiers in display order.
    pub const ORDER: [(Mods, &'static str); 6] = [
        (Mods::SUPER, "Super"),
        (Mods::CTRL, "Ctrl"),
        (Mods::ALT, "Alt"),
        (Mods::SHIFT, "Shift"),
        (Mods::LEVEL3, "ISO_Level3_Shift"),
        (Mods::LEVEL5, "ISO_Level5_Shift"),
    ];
}

/// Which concrete modifier Niri's `Mod` resolves to (`input { mod-key }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModKey {
    Ctrl,
    Shift,
    Alt,
    Super,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

impl ModKey {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ModKey::Ctrl,
            "shift" => ModKey::Shift,
            "alt" => ModKey::Alt,
            "super" | "win" => ModKey::Super,
            "iso_level3_shift" | "mod5" => ModKey::IsoLevel3Shift,
            "iso_level5_shift" | "mod3" => ModKey::IsoLevel5Shift,
            _ => return None,
        })
    }
    pub fn mods(self) -> Mods {
        match self {
            ModKey::Ctrl => Mods::CTRL,
            ModKey::Shift => Mods::SHIFT,
            ModKey::Alt => Mods::ALT,
            ModKey::Super => Mods::SUPER,
            ModKey::IsoLevel3Shift => Mods::LEVEL3,
            ModKey::IsoLevel5Shift => Mods::LEVEL5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Trigger {
    /// Raw keysym value as resolved by xkbcommon.
    Keysym(u32),
    /// Mouse buttons, wheel and touchpad scroll, by Niri's canonical name.
    Pointer(String),
}

pub const POINTER_TRIGGERS: [&str; 13] = [
    "MouseLeft",
    "MouseRight",
    "MouseMiddle",
    "MouseBack",
    "MouseForward",
    "WheelScrollDown",
    "WheelScrollUp",
    "WheelScrollLeft",
    "WheelScrollRight",
    "TouchpadScrollDown",
    "TouchpadScrollUp",
    "TouchpadScrollLeft",
    "TouchpadScrollRight",
];

/// A key exactly as Niri's config layer identifies it (used for duplicate
/// detection and include overrides).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct Key {
    pub mods: Mods,
    pub trigger: Trigger,
}

/// A key after resolving `Mod`: what actually matches at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct Combo {
    pub mods: Mods,
    pub trigger: Trigger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyError(pub String);

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn keysym_from_name(name: &str) -> Option<u32> {
    let mut sym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
    // Same special case as Niri (niri-wm/niri#1969).
    let lower_saver = xkb::keysym_from_name("XF86Screensaver", xkb::KEYSYM_NO_FLAGS).raw();
    if lower_saver != xkb::keysyms::KEY_NoSymbol && sym.raw() == lower_saver {
        sym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
        if sym.raw() == xkb::keysyms::KEY_NoSymbol {
            sym = xkb::Keysym::new(xkb::keysyms::KEY_XF86ScreenSaver);
        }
    }
    (sym.raw() != xkb::keysyms::KEY_NoSymbol).then_some(sym.raw())
}

pub fn keysym_name(sym: u32) -> String {
    xkb::keysym_get_name(xkb::Keysym::new(sym))
}

impl Key {
    pub fn parse(s: &str) -> Result<Key, KeyError> {
        let mut parts: Vec<&str> = s.split('+').collect();
        let key = parts.pop().unwrap_or_default();
        let mut mods = Mods::empty();
        for part in parts {
            let p = part.trim().to_ascii_lowercase();
            let m = match p.as_str() {
                "mod" => Mods::MOD,
                "ctrl" | "control" => Mods::CTRL,
                "shift" => Mods::SHIFT,
                "alt" => Mods::ALT,
                "super" | "win" => Mods::SUPER,
                "iso_level3_shift" | "mod5" => Mods::LEVEL3,
                "iso_level5_shift" | "mod3" => Mods::LEVEL5,
                _ => return Err(KeyError(format!("invalid modifier: {part}"))),
            };
            mods = mods.with(m);
        }
        let trigger = if let Some(p) = POINTER_TRIGGERS.iter().find(|p| p.eq_ignore_ascii_case(key)) {
            Trigger::Pointer((*p).to_string())
        } else {
            Trigger::Keysym(keysym_from_name(key).ok_or_else(|| KeyError(format!("invalid key: {key}")))?)
        };
        Ok(Key { mods, trigger })
    }

    pub fn resolve(&self, mod_key: ModKey) -> Combo {
        let mods = if self.mods.contains(Mods::MOD) { self.mods.without(Mods::MOD).with(mod_key.mods()) } else { self.mods };
        Combo { mods, trigger: self.trigger.clone() }
    }

    pub fn is_keyboard(&self) -> bool {
        matches!(self.trigger, Trigger::Keysym(_))
    }
}

impl Combo {
    pub fn is_keyboard(&self) -> bool {
        matches!(self.trigger, Trigger::Keysym(_))
    }

    pub fn trigger_label(&self) -> String {
        trigger_label(&self.trigger)
    }

    /// Human-readable parts: modifiers in a fixed order, then the trigger.
    pub fn label_parts(&self) -> Vec<String> {
        let mut out: Vec<String> = Mods::ORDER
            .iter()
            .filter(|(m, _)| self.mods.contains(*m))
            .map(|(_, n)| match *n {
                "ISO_Level3_Shift" => "AltGr".to_string(),
                "ISO_Level5_Shift" => "Level5".to_string(),
                n => n.to_string(),
            })
            .collect();
        out.push(self.trigger_label());
        out
    }

    pub fn label(&self) -> String {
        self.label_parts().join(" + ")
    }

    /// Config spelling for a new binding. `use_mod` writes Niri's `Mod`
    /// alias for the resolved mod key, matching the user's prevailing style.
    pub fn to_config(&self, mod_key: ModKey, use_mod: bool) -> String {
        let mut parts = Vec::new();
        let mut mods = self.mods;
        if use_mod && mods.contains(mod_key.mods()) {
            parts.push("Mod".to_string());
            mods = mods.without(mod_key.mods());
        }
        for (m, n) in Mods::ORDER {
            if mods.contains(m) {
                parts.push(n.to_string());
            }
        }
        parts.push(match &self.trigger {
            Trigger::Keysym(s) => config_keysym_name(*s),
            Trigger::Pointer(p) => p.clone(),
        });
        parts.join("+")
    }
}

/// Keysym name for writing into the config. Letters are written uppercase as
/// is conventional in Niri configs (`Mod+T`); Niri resolves them
/// case-insensitively to the same keysym.
pub fn config_keysym_name(sym: u32) -> String {
    let name = keysym_name(sym);
    if name.len() == 1 && name.chars().all(|c| c.is_ascii_lowercase()) { name.to_ascii_uppercase() } else { name }
}

pub fn trigger_label(t: &Trigger) -> String {
    match t {
        Trigger::Pointer(p) => pointer_label(p),
        Trigger::Keysym(s) => keysym_label(*s),
    }
}

fn pointer_label(p: &str) -> String {
    match p {
        "MouseLeft" => "Left click".into(),
        "MouseRight" => "Right click".into(),
        "MouseMiddle" => "Middle click".into(),
        "MouseBack" => "Mouse back".into(),
        "MouseForward" => "Mouse forward".into(),
        "WheelScrollDown" => "Wheel down".into(),
        "WheelScrollUp" => "Wheel up".into(),
        "WheelScrollLeft" => "Wheel left".into(),
        "WheelScrollRight" => "Wheel right".into(),
        "TouchpadScrollDown" => "Touchpad down".into(),
        "TouchpadScrollUp" => "Touchpad up".into(),
        "TouchpadScrollLeft" => "Touchpad left".into(),
        "TouchpadScrollRight" => "Touchpad right".into(),
        p => p.into(),
    }
}

/// Short label for a keysym as shown on keycaps and chips.
pub fn keysym_label(sym: u32) -> String {
    let name = keysym_name(sym);
    let fixed = match name.as_str() {
        "Return" => "Enter",
        "Escape" => "Esc",
        "BackSpace" => "Backspace",
        "Prior" | "Page_Up" => "PgUp",
        "Next" | "Page_Down" => "PgDn",
        "Print" => "Print",
        "Scroll_Lock" => "Scroll",
        "Caps_Lock" => "Caps",
        "Num_Lock" => "Num",
        "space" => "Space",
        "Left" => "←",
        "Right" => "→",
        "Up" => "↑",
        "Down" => "↓",
        "Delete" => "Delete",
        "Insert" => "Insert",
        "KP_Enter" => "Num Enter",
        "KP_Add" => "Num +",
        "KP_Subtract" => "Num −",
        "KP_Multiply" => "Num *",
        "KP_Divide" => "Num /",
        "KP_Decimal" | "KP_Delete" => "Num .",
        "KP_Home" | "KP_7" => "Num 7",
        "KP_Up" | "KP_8" => "Num 8",
        "KP_Prior" | "KP_9" => "Num 9",
        "KP_Left" | "KP_4" => "Num 4",
        "KP_Begin" | "KP_5" => "Num 5",
        "KP_Right" | "KP_6" => "Num 6",
        "KP_End" | "KP_1" => "Num 1",
        "KP_Down" | "KP_2" => "Num 2",
        "KP_Next" | "KP_3" => "Num 3",
        "KP_Insert" | "KP_0" => "Num 0",
        "XF86AudioRaiseVolume" => "Volume up",
        "XF86AudioLowerVolume" => "Volume down",
        "XF86AudioMute" => "Mute",
        "XF86AudioMicMute" => "Mic mute",
        "XF86AudioPlay" => "Play",
        "XF86AudioPause" => "Pause media",
        "XF86AudioNext" => "Next track",
        "XF86AudioPrev" => "Previous track",
        "XF86AudioStop" => "Stop media",
        "XF86MonBrightnessUp" => "Brightness up",
        "XF86MonBrightnessDown" => "Brightness down",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    let ch = xkb::keysym_to_utf8(xkb::Keysym::new(sym));
    let ch = ch.trim_end_matches('\0');
    let printable = ch.chars().count() == 1 && ch.chars().all(|c| !c.is_control() && !c.is_whitespace());
    if printable && !name.starts_with("KP_") {
        return ch.to_uppercase();
    }
    name.strip_prefix("XF86").unwrap_or(&name).replace('_', " ")
}

/// Keypad keysyms that are the same physical key (NumLock off/on levels).
pub fn keypad_twin(sym: u32) -> Option<u32> {
    let name = keysym_name(sym);
    let twin = match name.as_str() {
        "KP_Home" => "KP_7",
        "KP_7" => "KP_Home",
        "KP_Up" => "KP_8",
        "KP_8" => "KP_Up",
        "KP_Prior" => "KP_9",
        "KP_9" => "KP_Prior",
        "KP_Left" => "KP_4",
        "KP_4" => "KP_Left",
        "KP_Begin" => "KP_5",
        "KP_5" => "KP_Begin",
        "KP_Right" => "KP_6",
        "KP_6" => "KP_Right",
        "KP_End" => "KP_1",
        "KP_1" => "KP_End",
        "KP_Down" => "KP_2",
        "KP_2" => "KP_Down",
        "KP_Next" => "KP_3",
        "KP_3" => "KP_Next",
        "KP_Insert" => "KP_0",
        "KP_0" => "KP_Insert",
        "KP_Delete" => "KP_Decimal",
        "KP_Decimal" => "KP_Delete",
        _ => return None,
    };
    keysym_from_name(twin)
}

/// The keysym produced by the same key without Shift, for shifted symbols
/// on the US layout (Niri matches binds against the unshifted "raw" keysym,
/// so `Mod+Exclam` never fires on a US keyboard; it has to be `Mod+Shift+1`).
pub fn us_unshifted(sym: u32) -> Option<u32> {
    let name = keysym_name(sym);
    let base = match name.as_str() {
        "exclam" => "1",
        "at" => "2",
        "numbersign" => "3",
        "dollar" => "4",
        "percent" => "5",
        "asciicircum" => "6",
        "ampersand" => "7",
        "asterisk" => "8",
        "parenleft" => "9",
        "parenright" => "0",
        "underscore" => "minus",
        "plus" => "equal",
        "braceleft" => "bracketleft",
        "braceright" => "bracketright",
        "bar" => "backslash",
        "colon" => "semicolon",
        "quotedbl" => "apostrophe",
        "less" => "comma",
        "greater" => "period",
        "question" => "slash",
        "asciitilde" => "grave",
        _ => return None,
    };
    keysym_from_name(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_matches_niri_rules() {
        let a = Key::parse("Mod+Shift+T").unwrap();
        let b = Key::parse("mod+shift+t").unwrap();
        assert_eq!(a, b, "case-insensitive keysym and modifiers");
        let c = Key::parse("Super+Shift+T").unwrap();
        assert_ne!(a, c, "Mod is distinct from Super at config level");
        assert_eq!(a.resolve(ModKey::Super), c.resolve(ModKey::Super));
        assert_ne!(a.resolve(ModKey::Alt), c.resolve(ModKey::Alt));
        assert_eq!(Key::parse("Ctrl+Alt+Delete").unwrap(), Key::parse("Control+Alt+Delete").unwrap());
        assert!(Key::parse("Hyper+T").is_err());
        assert!(Key::parse("Mod+NotAKey").is_err());
        assert_eq!(Key::parse("Mod+WheelScrollDown").unwrap().trigger, Trigger::Pointer("WheelScrollDown".into()));
        assert_eq!(Key::parse("Mod5+A").unwrap(), Key::parse("ISO_Level3_Shift+a").unwrap());
    }

    #[test]
    fn numpad_is_not_number_row() {
        let row = Key::parse("Mod+1").unwrap();
        let pad = Key::parse("Mod+KP_1").unwrap();
        assert_ne!(row, pad);
        assert_eq!(keypad_twin(keysym_from_name("KP_End").unwrap()), keysym_from_name("KP_1"));
        assert_eq!(keysym_label(keysym_from_name("KP_1").unwrap()), "Num 1");
        assert_eq!(keysym_label(keysym_from_name("1").unwrap()), "1");
    }

    #[test]
    fn shifted_symbols_map_to_base_key() {
        let excl = keysym_from_name("exclam").unwrap();
        assert_eq!(us_unshifted(excl), keysym_from_name("1"));
        assert_eq!(us_unshifted(keysym_from_name("a").unwrap()), None);
    }

    #[test]
    fn config_spelling() {
        let c = Key::parse("Super+Shift+s").unwrap().resolve(ModKey::Super);
        assert_eq!(c.to_config(ModKey::Super, true), "Mod+Shift+S");
        assert_eq!(c.to_config(ModKey::Super, false), "Super+Shift+S");
        assert_eq!(c.to_config(ModKey::Alt, true), "Super+Shift+S");
        assert_eq!(c.label(), "Super + Shift + S");
        let p = Key::parse("Print").unwrap().resolve(ModKey::Super);
        assert_eq!(p.to_config(ModKey::Super, true), "Print");
    }
}
