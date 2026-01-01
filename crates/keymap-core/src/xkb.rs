//! Physical key → binding identity (from the user's XKB setup) and legend
//! (always US English).
//!
//! Identity: Niri matches binds against Smithay's
//! `raw_latin_sym_or_raw_current_sym`: the level-0 keysym of the active layout
//! if it is ASCII or non-printable, otherwise the level-0 keysym of the first
//! other layout that produces a printable ASCII character. Keymap applies the
//! same rule with the first configured layout taken as active, so `us,ua` and
//! `ua,us` both bind the Latin letters.
//!
//! Legends are presentation only: they come from a fixed US keymap by
//! physical position, so the drawing never shows Cyrillic or other
//! alternate-layout characters and does not change when the input language
//! is switched. Nothing is ever derived from a legend's text.

use crate::config::XkbSettings;
use crate::keys::{self, Mods};
use xkbcommon::xkb;

#[derive(Debug, Clone, PartialEq)]
pub struct KeyInfo {
    pub evdev: u32,
    /// Keysym Niri would match for this key.
    pub sym: Option<u32>,
    /// English legend for this physical key (US position).
    pub label: String,
    /// Legend printed above the main one (e.g. `!` over `1`).
    pub shifted: Option<String>,
    /// The modifier this key acts as, if any.
    pub modifier: Option<Mods>,
    /// How shortcuts name this key when that differs from the legend (e.g.
    /// `Super` for a Caps Lock key remapped by an XKB option).
    pub binds_as: Option<String>,
}

pub struct Resolver {
    keymap: Option<xkb::Keymap>,
    /// Plain US keymap, used only for legends.
    legends: Option<xkb::Keymap>,
    pub layout_names: Vec<String>,
    pub error: Option<String>,
}

fn key_char(sym: xkb::Keysym) -> Option<char> {
    let s = xkb::keysym_to_utf8(sym);
    let s = s.trim_end_matches('\0');
    let mut it = s.chars();
    let c = it.next()?;
    if it.next().is_some() { None } else { Some(c) }
}

fn modifier_of(name: &str) -> Option<Mods> {
    Some(match name {
        "Super_L" | "Super_R" | "Hyper_L" | "Hyper_R" => Mods::SUPER,
        "Control_L" | "Control_R" => Mods::CTRL,
        "Alt_L" | "Alt_R" | "Meta_L" | "Meta_R" => Mods::ALT,
        "Shift_L" | "Shift_R" => Mods::SHIFT,
        "ISO_Level3_Shift" | "Mode_switch" => Mods::LEVEL3,
        "ISO_Level5_Shift" => Mods::LEVEL5,
        _ => return None,
    })
}

impl Resolver {
    pub fn new(settings: &XkbSettings) -> Resolver {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let mut r = Self::identity(&ctx, settings);
        r.legends = xkb::Keymap::new_from_names(&ctx, "", "", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS);
        r
    }

    fn identity(ctx: &xkb::Context, settings: &XkbSettings) -> Resolver {
        if let Some(file) = &settings.file {
            let path = crate::config::expand_include(file, std::path::Path::new("/"));
            let km = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| xkb::Keymap::new_from_string(ctx, s, xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS));
            if let Some(km) = km {
                return Self::from_keymap(km, None);
            }
            let fallback = Self::identity(ctx, &XkbSettings { file: None, ..settings.clone() });
            return Resolver { error: Some(format!("could not compile xkb file {file}; using layout names")), ..fallback };
        }
        let km = xkb::Keymap::new_from_names(
            ctx,
            &settings.rules,
            &settings.model,
            &settings.layout,
            &settings.variant,
            settings.options.clone(),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        );
        match km {
            Some(km) => Self::from_keymap(km, None),
            None => {
                let us = xkb::Keymap::new_from_names(ctx, "", "", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS);
                let msg = format!("could not compile XKB layout \"{}\"; showing US legends", settings.layout);
                match us {
                    Some(km) => Self::from_keymap(km, Some(msg)),
                    None => Resolver { keymap: None, legends: None, layout_names: vec![], error: Some(msg) },
                }
            }
        }
    }

    fn from_keymap(km: xkb::Keymap, error: Option<String>) -> Resolver {
        let layout_names = (0..km.num_layouts()).map(|i| km.layout_get_name(i).to_string()).collect();
        Resolver { keymap: Some(km), legends: None, layout_names, error }
    }

    fn level_sym(&self, code: xkb::Keycode, layout: u32, level: u32) -> Option<xkb::Keysym> {
        self.keymap.as_ref()?.key_get_syms_by_level(code, layout, level).first().copied()
    }

    /// The English legend and shifted legend of a physical key.
    fn legend(&self, code: xkb::Keycode) -> Option<(String, Option<String>)> {
        let km = self.legends.as_ref()?;
        let at = |level| km.key_get_syms_by_level(code, 0, level).first().copied();
        let base = at(0)?;
        Some((sym_label(base), shifted_legend(base, at(1))))
    }

    pub fn key(&self, evdev: u32) -> KeyInfo {
        let code = xkb::Keycode::new(evdev + 8);
        let Some(km) = &self.keymap else {
            return KeyInfo { evdev, sym: None, label: format!("#{evdev}"), shifted: None, modifier: None, binds_as: None };
        };
        let n = km.num_layouts_for_key(code).max(1);
        let base = self.level_sym(code, 0, 0);
        let identity = match base {
            Some(b) if key_char(b).is_none_or(|c| c.is_ascii()) => Some(b),
            Some(b) => (1..n)
                .filter_map(|l| self.level_sym(code, l, 0))
                .find(|s| key_char(*s).is_some_and(|c| c.is_ascii() && !c.is_ascii_control()))
                .or(Some(b)),
            None => None,
        };
        let Some(sym) = identity else {
            let (label, shifted) = self.legend(code).unwrap_or_default();
            return KeyInfo { evdev, sym: None, label, shifted, modifier: None, binds_as: None };
        };
        let modifier = modifier_of(&keys::keysym_name(sym.raw()));
        let id_label = sym_label(sym);
        let (label, shifted) = self.legend(code).unwrap_or_else(|| {
            // Not on a US keyboard: show the identity, but only if it is English.
            let id_layout = (0..n).find(|l| self.level_sym(code, *l, 0) == Some(sym)).unwrap_or(0);
            let label = if id_label.is_ascii() { id_label.clone() } else { String::new() };
            (label, shifted_legend(sym, self.level_sym(code, id_layout, 1)))
        });
        let binds_as = (!id_label.eq_ignore_ascii_case(&label)).then_some(id_label);
        KeyInfo { evdev, sym: Some(sym.raw()), label, shifted, modifier, binds_as }
    }
}

/// Display name of a keysym: friendly modifier names, else the key label.
fn sym_label(sym: xkb::Keysym) -> String {
    match keys::keysym_name(sym.raw()).as_str() {
        "Super_L" | "Super_R" => "Super".into(),
        "Control_L" | "Control_R" => "Ctrl".into(),
        "Alt_L" | "Alt_R" | "Meta_L" | "Meta_R" => "Alt".into(),
        "Shift_L" | "Shift_R" => "Shift".into(),
        "ISO_Level3_Shift" | "Mode_switch" => "AltGr".into(),
        _ => keys::keysym_label(sym.raw()),
    }
}

/// The shifted character, when it is a different printable non-letter.
fn shifted_legend(base: xkb::Keysym, shifted: Option<xkb::Keysym>) -> Option<String> {
    shifted.and_then(key_char).filter(|c| !c.is_control() && !c.is_whitespace()).map(|c| c.to_string()).filter(|s| {
        // Keypad keys are labelled by their digit already (KP_Home is "Num 7").
        let plain = key_char(base).map(|c| c.to_uppercase().to_string());
        plain.as_deref() != Some(s.as_str()) && !keys::keysym_name(base.raw()).starts_with("KP_") && !s.chars().all(char::is_alphabetic)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(layout: &str) -> Resolver {
        Resolver::new(&XkbSettings { layout: layout.into(), ..Default::default() })
    }

    #[test]
    fn latin_identity_across_layout_order() {
        for layouts in ["us,ua", "ua,us", "us,ru", "ru,us"] {
            let res = r(layouts);
            assert!(res.error.is_none(), "{layouts}: {:?}", res.error);
            let q = res.key(16); // KEY_Q
            assert_eq!(q.sym, keys::keysym_from_name("q"), "{layouts}");
            assert_eq!(q.label, "Q");
            assert_eq!(q.binds_as, None);
        }
    }

    #[test]
    fn keypad_digits_are_not_painted_twice() {
        let r = r("us,ua");
        // KP_7 (evdev 71): one "7", no small shifted "7".
        let k = r.key(71);
        assert_eq!(k.label, "Num 7");
        assert_eq!(k.shifted, None);
        // The main row keeps its shifted legend.
        assert_eq!(r.key(8).shifted.as_deref(), Some("&"));
    }

    #[test]
    fn legends_are_english_whatever_the_layouts() {
        // Only Cyrillic layouts: the binding identity stays Cyrillic (that is
        // what Niri matches), the legend is the US one for that position.
        for layouts in ["ua", "ru", "ua,ru"] {
            let res = r(layouts);
            let q = res.key(16);
            assert_eq!(q.label, "Q", "{layouts}");
            assert_eq!(q.sym, keys::keysym_from_name("Cyrillic_shorti"), "{layouts}");
            assert_eq!(q.binds_as.as_deref(), Some(keys::keysym_label(q.sym.unwrap()).as_str()));
            let semi = res.key(39); // KEY_SEMICOLON: ж in ua
            assert_eq!(semi.label, ";", "{layouts}");
            assert_eq!(semi.shifted.as_deref(), Some(":"), "{layouts}");
            let one = res.key(2);
            assert_eq!((one.label.as_str(), one.shifted.as_deref()), ("1", Some("!")), "{layouts}");
        }
        // No legend on any key has a non-English letter (symbols such as the
        // keypad's "−" are fine), and none depends on the layouts configured.
        let english = |t: &str| t.chars().all(|c| c.is_ascii() || !c.is_alphabetic());
        let us = r("us");
        for layouts in ["us,ua", "ua,us", "ua", "de", "fr"] {
            let res = r(layouts);
            for code in 1..=200 {
                let (k, u) = (res.key(code), us.key(code));
                assert!(english(&k.label), "{layouts} key {code}: {:?}", k.label);
                assert!(k.shifted.as_deref().is_none_or(english), "{layouts} key {code}: {:?}", k.shifted);
                if u.sym.is_some() {
                    assert_eq!((&k.label, &k.shifted), (&u.label, &u.shifted), "{layouts} key {code}");
                }
            }
        }
    }

    #[test]
    fn legend_differs_from_identity_on_other_latin_layouts() {
        // German QWERTZ: the US "Y" position types z, so it binds as Z.
        let de = r("de");
        let k = de.key(21); // KEY_Y
        assert_eq!(k.label, "Y");
        assert_eq!(k.sym, keys::keysym_from_name("z"));
        assert_eq!(k.binds_as.as_deref(), Some("Z"));
        // An XKB option that turns Caps Lock into Super keeps the Caps legend.
        let caps = Resolver::new(&XkbSettings { layout: "us".into(), options: Some("caps:super".into()), ..Default::default() }).key(58);
        assert_eq!(caps.modifier, Some(Mods::SUPER));
        assert_eq!(caps.binds_as.as_deref(), Some("Super"));
        assert_ne!(caps.label, "Super");
    }

    #[test]
    fn modifiers_numbers_and_numpad() {
        let res = r("us");
        assert_eq!(res.key(125).modifier, Some(Mods::SUPER));
        assert_eq!(res.key(42).modifier, Some(Mods::SHIFT));
        let one = res.key(2);
        assert_eq!(one.label, "1");
        assert_eq!(one.shifted.as_deref(), Some("!"));
        let kp1 = res.key(79);
        assert_ne!(kp1.sym, one.sym, "number pad 1 differs from the number row");
        assert_eq!(kp1.label, "Num 1");
        assert_eq!(res.key(28).label, "Enter");
        assert_eq!(res.key(30).shifted, None, "letters have no separate shifted legend");
    }

    #[test]
    fn bad_layout_falls_back() {
        let res = r("definitely-not-a-layout");
        assert!(res.error.is_some());
        assert_eq!(res.key(16).label, "Q");
    }
}
