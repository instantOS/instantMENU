//! Icon resolution for `:color:icon: label` entries.
//!
//! The icon field of an entry may spell a glyph three ways, all resolved by
//! [`lookup`]:
//!
//! - the literal glyph (a Nerd Fonts codepoint or an emoji) — for producers
//!   that already speak UTF-8,
//! - its name: `power-off`, `power_off`, `poweroff` and `power off` all
//!   resolve, with or without a family qualifier (`md-`, `fa-`, ... and the
//!   old `nf-` prefix). The ~11k official Nerd Fonts names ship as generated
//!   catalog (see [`names`]); [`ALIASES`] adds the launcher-vocabulary
//!   synonyms that are not Nerd Fonts names (`shutdown`, `reboot`, ...).
//! - a hex codepoint (`f011`, `0xf011`, `u+f011`) — must land in an icon or
//!   emoji range, so incidental hex words do not resolve.

mod names;

/// Whether `c` is in the private-use ranges Nerd Fonts icons live in
/// (the ranges the renderer routes to the secondary font).
pub fn is_icon_char(c: char) -> bool {
    matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// Whether `c` is in the emoji/symbol ranges (routed to the emoji font).
pub fn is_emoji_char(c: char) -> bool {
    (c as u32) >= 0x1F000 || matches!(c as u32, 0x2600..=0x27BF | 0x2190..=0x21FF | 0x2B00..=0x2BFF)
}

/// Launcher-vocabulary synonyms that are not Nerd Fonts names, mapping to
/// the normalized bare name of the icon they mean. Keys and targets are
/// stored normalized (separator-free); checked before the generated tables,
/// so an alias also overrides an unfortunate bare-name collision (`volume`
/// alone is Font Awesome's muted speaker).
const ALIASES: &[(&str, &str)] = &[
    ("app", "apps"),
    ("back", "arrowleft"),
    ("brightness", "brightness5"),
    ("computer", "laptop"),
    ("display", "monitor"),
    ("down", "arrowdown"),
    ("exit", "logout"),
    ("forward", "arrowright"),
    ("headphone", "headphones"),
    ("left", "arrowleft"),
    ("next", "skipnext"),
    ("options", "settings"),
    ("poweron", "power"),
    ("preferences", "settings"),
    ("prev", "skipprevious"),
    ("previous", "skipprevious"),
    ("quit", "logout"),
    ("reboot", "restart"),
    ("right", "arrowright"),
    ("shutdown", "poweroff"),
    ("sound", "volumehigh"),
    ("temperature", "thermometer"),
    ("up", "arrowup"),
    ("volume", "volumehigh"),
];

/// Resolve an icon field to the glyph it names, or `None` when it spells
/// no icon (the entry then renders its exact text instead).
pub fn lookup(spec: &str) -> Option<char> {
    let spec = spec.trim();

    /* the glyph itself: one character inside the icon/emoji ranges */
    let mut chars = spec.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if is_icon_char(c) || is_emoji_char(c) {
            return Some(c);
        }
    }

    let key = normalize(spec);
    if key.is_empty() {
        return None;
    }

    /* names: aliases, then the bare name, then the family-qualified one
     * (the old `nf-` prefix normalizes onto the qualified key) */
    let code = ALIASES
        .iter()
        .find(|(alias, _)| *alias == key)
        .and_then(|(_, target)| names::bare().find(target))
        .or_else(|| names::bare().find(&key))
        .or_else(|| names::qualified().find(&key))
        .or_else(|| {
            key.strip_prefix("nf")
                .and_then(|rest| names::qualified().find(rest))
        });

    if let Some(code) = code {
        return char::from_u32(code);
    }

    /* a hex codepoint: `f011`, `0xf011`, `u+f011`, `\u{f011}` — everything
     * but the digits normalizes away. Must land in an icon or emoji range
     * so ordinary words that happen to be hex do not resolve. */
    let digits = key.strip_prefix("0x").unwrap_or(&key);
    let digits = digits.strip_prefix('u').unwrap_or(digits);
    let value = (3..=6)
        .contains(&digits.len())
        .then(|| u32::from_str_radix(digits, 16).ok())
        .flatten()?;
    let c = char::from_u32(value)?;
    (is_icon_char(c) || is_emoji_char(c)).then_some(c)
}

/// The lookup key of a name: lowercase, separators dropped (`power-off`,
/// `power_off` and `poweroff` share one key). Matches the generator in
/// `utils/gen_icons.py`.
fn normalize(spec: &str) -> String {
    spec.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// One discoverable Nerd Fonts icon. Names are normalized, family-qualified
/// lookup keys such as `mdpoweroff`; separators are optional when using them
/// in an icon entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogIcon {
    pub name: &'static str,
    pub glyph: char,
}

/// Every family-qualified Nerd Fonts icon, sorted by name.
pub fn catalog() -> impl Iterator<Item = CatalogIcon> {
    names::qualified().iter().filter_map(|(name, codepoint)| {
        char::from_u32(codepoint).map(|glyph| CatalogIcon { name, glyph })
    })
}

/// Search every icon name. Search uses the same case- and separator-insensitive
/// normalization as entry parsing; an empty query returns the whole catalog.
pub fn search(query: &str) -> Vec<CatalogIcon> {
    let query = normalize(query);
    catalog()
        .filter(|icon| icon.name.contains(&query))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup_str(spec: &str) -> Option<String> {
        lookup(spec).map(|c| c.to_string())
    }

    /// The literal glyph passes through untouched.
    #[test]
    fn literal_glyphs_resolve_to_themselves() {
        assert_eq!(lookup("\u{f011}"), Some('\u{f011}')); // nf-fa-power_off
        assert_eq!(lookup("\u{f0425}"), Some('\u{f0425}')); // nf-md-power
        assert_eq!(lookup("\u{1f5a5}"), Some('\u{1f5a5}')); // emoji
        assert_eq!(lookup(" \u{f011} "), Some('\u{f011}'));
    }

    /// Plain words that name no icon do not resolve ("x" does — it is
    /// octicons' X logo).
    #[test]
    fn plain_characters_do_not_resolve() {
        assert_eq!(lookup("a"), None);
        assert_eq!(lookup("qqq"), None);
        assert_eq!(lookup("zzz"), None);
    }

    /// Separator and case variants of a name all resolve to the same glyph.
    #[test]
    fn name_separators_and_case_do_not_matter() {
        let power = lookup_str("md-power").unwrap(); // f0425
        for spec in ["power", "Power", "md-power", "md_power", "MD POWER"] {
            assert_eq!(lookup_str(spec), Some(power.clone()), "{spec}");
        }
        let power_off = lookup_str("md-power_off").unwrap(); // f0902
        for spec in [
            "power-off",
            "power_off",
            "poweroff",
            "power off",
            "nf-md-power_off",
        ] {
            assert_eq!(lookup_str(spec), Some(power_off.clone()), "{spec}");
        }
    }

    /// The bare name prefers Material Design; a family qualifier selects
    /// that family's glyph instead.
    #[test]
    fn qualified_names_select_their_family() {
        // bare "poweroff" is md-power_off (f0902), not fa-power_off (f011)
        assert_eq!(lookup("poweroff"), Some('\u{f0902}'));
        assert_eq!(lookup("fa-power_off"), Some('\u{f011}'));
        assert_eq!(lookup("nf-fa-power_off"), Some('\u{f011}'));
    }

    /// The aliases resolve, and only to glyphs the tables actually contain.
    #[test]
    fn aliases_resolve_to_existing_names() {
        assert_eq!(lookup("shutdown"), lookup("poweroff"));
        assert_eq!(lookup("reboot"), lookup("restart"));
        assert_eq!(lookup("brightness"), lookup("brightness5"));
        for (alias, target) in ALIASES {
            // keys and targets must already be normalized to be reachable
            assert_eq!(normalize(alias), *alias, "{alias}");
            assert_eq!(normalize(target), *target, "{target}");
            let resolved = lookup(alias).unwrap_or_else(|| panic!("{alias}"));
            let expected = lookup(target).unwrap_or_else(|| panic!("{target}"));
            assert_eq!(resolved, expected, "{alias}");
        }
    }

    #[test]
    fn catalog_search_is_normalized_and_complete() {
        assert_eq!(catalog().count(), names::qualified().len());
        let results = search("power-off");
        assert!(results.iter().any(|icon| icon.name == "fapoweroff"));
        assert!(results.iter().any(|icon| icon.name == "mdpoweroff"));
        assert_eq!(search("MD POWER_OFF"), search("mdpoweroff"));
        assert_eq!(search("").len(), catalog().count());
    }

    /// Hex codepoints in the icon ranges resolve; hex-looking words outside
    /// them and unknown names do not.
    #[test]
    fn hex_codepoints_resolve_only_inside_icon_ranges() {
        assert_eq!(lookup("f011"), Some('\u{f011}'));
        assert_eq!(lookup("0xf011"), Some('\u{f011}'));
        assert_eq!(lookup("u+f011"), Some('\u{f011}'));
        assert_eq!(lookup("\\u{f0425}"), Some('\u{f0425}'));
        assert_eq!(lookup("1f5a5"), Some('\u{1f5a5}'));
        // valid hex, but 0xcafe is not an icon codepoint
        assert_eq!(lookup("cafe"), None);
        assert_eq!(lookup("no-such-icon"), None);
        assert_eq!(lookup(""), None);
        assert_eq!(lookup("   "), None);
    }
}
