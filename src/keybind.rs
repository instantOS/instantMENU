//! Global accept bindings. Parsed once at startup, shared by X11 and Wayland.
use crate::backend::Modifiers;
use xkbcommon::xkb::{self, keysyms as ks};

#[derive(Debug, Clone)]
pub struct Keybind {
    pub key: String,
    pub label: String,
    sym: u32,
    mods: Modifiers,
}
impl std::str::FromStr for Keybind {
    type Err = String;
    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let (key, label) = spec
            .split_once(':')
            .ok_or("expected KEY:LABEL (for example ctrl-e:Edit)")?;
        if !is_supported_menu_key(key) || RESERVED.contains(&key) {
            return Err(format!("unsupported or reserved menu key {key:?}"));
        }
        if label.trim().is_empty() || label.chars().any(char::is_control) {
            return Err("binding labels must be nonempty and contain no control characters".into());
        }
        let mut suffix = key;
        let mut mods = Modifiers::default();
        for (prefix, flag) in [
            ("ctrl-", &mut mods.ctrl),
            ("alt-", &mut mods.alt),
            ("shift-", &mut mods.shift),
        ] {
            if let Some(rest) = suffix.strip_prefix(prefix) {
                *flag = true;
                suffix = rest;
            }
        }
        let name = match suffix {
            "enter" => "Return",
            "space" => "space",
            "backspace" => "BackSpace",
            "delete" => "Delete",
            "left" => "Left",
            "right" => "Right",
            "up" => "Up",
            "down" => "Down",
            "home" => "Home",
            "end" => "End",
            "insert" => "Insert",
            "page-up" => "Prior",
            "page-down" => "Next",
            other => other,
        };
        if suffix.len() == 1 && suffix.as_bytes()[0].is_ascii_uppercase() {
            mods.shift = true;
        }
        let sym = xkb::keysym_from_name(
            name,
            if suffix.len() == 1 {
                xkb::KEYSYM_NO_FLAGS
            } else {
                xkb::KEYSYM_CASE_INSENSITIVE
            },
        )
        .raw();
        if sym == ks::KEY_NoSymbol {
            return Err(format!("unknown key {key:?}"));
        }
        Ok(Self {
            key: key.into(),
            label: label.into(),
            sym,
            mods,
        })
    }
}
impl Keybind {
    pub fn matches(&self, sym: u32, mods: Modifiers) -> bool {
        self.sym == sym && self.mods == mods
    }
    pub fn hint(&self) -> String {
        format!(
            "{}  {}",
            self.key
                .split('-')
                .map(|part| {
                    let mut chars = part.chars();
                    chars
                        .next()
                        .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join("+"),
            self.label
        )
    }
}
const RESERVED: &[&str] = &[
    // Abort / dismissal.
    "esc",
    "ctrl-c",
    "ctrl-g",
    "ctrl-q",
    // Submit (enter and its aliases).
    "enter",
    "ctrl-m",
    // Multi-select toggles.
    "tab",
    "btab",
    "shift-tab",
    // Cursor navigation (including fzf's emacs-mode defaults).
    "up",
    "down",
    "ctrl-p",
    "ctrl-n",
    "ctrl-k",
    "ctrl-j",
];

fn is_supported_menu_key(key: &str) -> bool {
    const NAMED_KEYS: &[&str] = &[
        "enter",
        "space",
        "backspace",
        "tab",
        "shift-tab",
        "esc",
        "delete",
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "insert",
        "page-up",
        "page-down",
    ];
    const MODIFIED_NAMED_KEYS: &[&str] = &[
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "backspace",
        "delete",
        "page-up",
        "page-down",
        "enter",
        "space",
    ];

    if NAMED_KEYS.contains(&key) {
        return true;
    }
    if let Some(number) = key.strip_prefix('f') {
        return number
            .parse::<u8>()
            .is_ok_and(|number| (1..=12).contains(&number));
    }
    if key.len() == 1 {
        return key
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric());
    }

    let is_named_suffix = |suffix| MODIFIED_NAMED_KEYS.contains(&suffix);
    let is_ascii_letter = |suffix: &str| {
        suffix.len() == 1
            && suffix
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
    };
    let is_safe_alt_character = |suffix: &str| {
        suffix.len() == 1
            && suffix
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
    };

    if let Some(suffix) = key.strip_prefix("ctrl-alt-shift-") {
        return is_named_suffix(suffix);
    }
    if let Some(suffix) = key.strip_prefix("ctrl-alt-") {
        return is_ascii_letter(suffix) || is_named_suffix(suffix);
    }
    if let Some(suffix) = key.strip_prefix("ctrl-shift-") {
        return is_named_suffix(suffix);
    }
    if let Some(suffix) = key.strip_prefix("ctrl-") {
        return is_ascii_letter(suffix) || is_named_suffix(suffix);
    }
    if let Some(suffix) = key.strip_prefix("alt-shift-") {
        return is_named_suffix(suffix);
    }
    if let Some(suffix) = key.strip_prefix("alt-") {
        return is_safe_alt_character(suffix) || is_named_suffix(suffix);
    }
    key.strip_prefix("shift-").is_some_and(is_named_suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unsafe_reserved_and_malformed_bindings() {
        for spec in [
            "ctrl-c:Quit",
            "enter:Submit",
            "ctrl-j:Jump",
            "ctrl-e",
            "ctrl-e:",
            "ctrl-e:bad\nlabel",
            "load:Oops",
            "ctrl-e,alt-s:Oops",
        ] {
            assert!(spec.parse::<Keybind>().is_err(), "{spec:?}");
        }
    }
    #[test]
    fn matches_exact_modifiers_and_function_keys() {
        let binding: Keybind = "ctrl-alt-e:Edit: details".parse().unwrap();
        assert!(binding.matches(
            ks::KEY_e,
            Modifiers {
                ctrl: true,
                alt: true,
                ..Modifiers::default()
            }
        ));
        assert!(!binding.matches(
            ks::KEY_e,
            Modifiers {
                ctrl: true,
                ..Modifiers::default()
            }
        ));
        assert!("f3:Open"
            .parse::<Keybind>()
            .unwrap()
            .matches(ks::KEY_F3, Modifiers::default()));
        assert!("A:Add".parse::<Keybind>().unwrap().matches(
            ks::KEY_A,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            }
        ));
    }
}
