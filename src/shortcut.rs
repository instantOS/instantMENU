//! Per-entry accelerators, independent of single-key menus and global actions.

use crate::backend::Modifiers;
use xkbcommon::xkb::{self, keysyms as ks};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    sym: u32,
    mods: Modifiers,
}

impl std::str::FromStr for Shortcut {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let spec = spec.to_lowercase();
        let mut suffix = spec.as_str();
        let mut mods = Modifiers::default();
        loop {
            let mut consumed = false;
            for (prefix, flag) in [
                ("ctrl-", &mut mods.ctrl),
                ("alt-", &mut mods.alt),
                ("shift-", &mut mods.shift),
                ("super-", &mut mods.logo),
            ] {
                if let Some(rest) = suffix.strip_prefix(prefix) {
                    if *flag {
                        return Err("duplicate shortcut modifier".into());
                    }
                    *flag = true;
                    suffix = rest;
                    consumed = true;
                    break;
                }
            }
            if !consumed {
                break;
            }
        }

        let sym = if let Some(c) = suffix
            .chars()
            .next()
            .filter(|_| suffix.chars().count() == 1)
        {
            if c.is_whitespace() || c.is_control() {
                return Err("shortcut character must be printable".into());
            }
            xkb::utf32_to_keysym(c as u32).raw()
        } else {
            let name = match suffix {
                "enter" => "Return",
                "esc" | "escape" => "Escape",
                "tab" => "Tab",
                "space" => "space",
                "backspace" => "BackSpace",
                "delete" => "Delete",
                "insert" => "Insert",
                "left" => "Left",
                "right" => "Right",
                "up" => "Up",
                "down" => "Down",
                "home" => "Home",
                "end" => "End",
                "page-up" => "Prior",
                "page-down" => "Next",
                function
                    if function
                        .strip_prefix('f')
                        .is_some_and(|n| n.parse::<u8>().is_ok_and(|n| (1..=35).contains(&n))) =>
                {
                    function
                }
                _ => return Err(format!("unknown shortcut key {suffix:?}")),
            };
            xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE).raw()
        };

        if sym == ks::KEY_NoSymbol {
            return Err(format!("unknown shortcut key {suffix:?}"));
        }

        // Keep dismissal available even when a producer supplies shortcuts.
        if sym == ks::KEY_Escape
            || (mods == Modifiers::default() && matches!(sym, ks::KEY_Return | ks::KEY_Tab))
            || (mods.ctrl && !mods.alt && !mods.logo && matches!(sym, ks::KEY_c | ks::KEY_g))
        {
            return Err("reserved menu key".into());
        }
        Ok(Self { sym, mods })
    }
}

impl Shortcut {
    pub fn matches(self, sym: u32, mods: Modifiers) -> bool {
        if self.mods != mods {
            return false;
        }
        if self.sym == sym {
            return true;
        }
        // Shift and Caps Lock can change letter keysyms. Modifiers are checked
        // separately, so shift-o matches Shift+O without matching plain o.
        match (character(self.sym), character(sym)) {
            (Some(a), Some(b)) => a.to_lowercase().eq(b.to_lowercase()),
            _ => false,
        }
    }

    /// Compact modifier symbols and a readable key name for the row's keycap.
    pub fn hint(self) -> String {
        let mut parts = Vec::new();
        for (active, symbol) in [
            (self.mods.ctrl, "⌃"),
            (self.mods.alt, "⎇"),
            (self.mods.shift, "⇧"),
            (self.mods.logo, "⊞"),
        ] {
            if active {
                parts.push(symbol.to_owned());
            }
        }
        let key = match self.sym {
            ks::KEY_Return => "↵".into(),
            ks::KEY_Tab => "Tab".into(),
            ks::KEY_space => "Space".into(),
            ks::KEY_BackSpace => "⌫".into(),
            ks::KEY_Delete => "Del".into(),
            ks::KEY_Insert => "Ins".into(),
            ks::KEY_Left => "←".into(),
            ks::KEY_Right => "→".into(),
            ks::KEY_Up => "↑".into(),
            ks::KEY_Down => "↓".into(),
            ks::KEY_Home => "Home".into(),
            ks::KEY_End => "End".into(),
            ks::KEY_Prior => "PgUp".into(),
            ks::KEY_Next => "PgDn".into(),
            _ => character(self.sym)
                .map(|c| c.to_uppercase().collect())
                .unwrap_or_else(|| xkb::keysym_get_name(self.sym.into())),
        };
        parts.push(key);
        parts.join(" ")
    }
}

fn character(sym: u32) -> Option<char> {
    char::from_u32(xkb::keysym_to_utf32(sym.into())).filter(|c| !c.is_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_are_exact_and_shifted_letters_match() {
        let shortcut: Shortcut = "super-ctrl-shift-o".parse().unwrap();
        let mods = Modifiers {
            logo: true,
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        };
        assert!(shortcut.matches(ks::KEY_O, mods));
        assert!(shortcut.matches(ks::KEY_o, mods));
        assert!(!shortcut.matches(ks::KEY_O, Modifiers { alt: true, ..mods }));
        assert!(!shortcut.matches(ks::KEY_o, Modifiers::default()));
        assert_eq!(shortcut.hint(), "⌃ ⇧ ⊞ O");
    }

    #[test]
    fn named_keys_unicode_and_function_keys() {
        for (spec, sym) in [
            ("f3", ks::KEY_F3),
            ("alt-page-down", ks::KEY_Next),
            ("λ", xkb::utf32_to_keysym('λ' as u32).raw()),
        ] {
            let shortcut: Shortcut = spec.parse().unwrap();
            assert!(shortcut.matches(
                sym,
                Modifiers {
                    alt: spec.starts_with("alt-"),
                    ..Modifiers::default()
                }
            ));
        }
    }

    #[test]
    fn malformed_and_reserved_shortcuts_are_rejected() {
        for spec in [
            "",
            "ctrl-",
            "super-super-o",
            "ctrl-unknown",
            "esc",
            "enter",
            "tab",
            "ctrl-c",
            "ctrl-g",
            "f36",
            "f01",
            " ",
        ] {
            assert!(spec.parse::<Shortcut>().is_err(), "{spec}");
        }
    }
}
