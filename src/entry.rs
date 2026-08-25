//! Parsing of item line prefixes: how a line renders — a plain item, a
//! `>` comment, a colored item, or an icon entry — and where its label
//! starts inside the text.
//!
//! Styled entries use colon-delimited fields (readable by humans and
//! unambiguous for the scripts and agents that produce them):
//!
//! ```text
//! :red:power: Shutdown
//! : green : power-off :     Shutdown now
//! :green: A colored entry without an icon
//! ```
//!
//! Whitespace around each colon is tolerated and leading label whitespace is
//! skipped. The color is a scheme name or single-letter code; the optional
//! icon is a Nerd Fonts glyph, its name, or a hex codepoint (see
//! [`crate::icons`]). A styled entry whose color or icon does not resolve is
//! a plain item showing the **exact** original line.

use crate::enums::Scheme;
use crate::icons;

/// How an item line renders.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ItemKind {
    /// A plain item: the whole line is the label.
    #[default]
    Plain,
    /// `>comment`: drawn like an item but cannot be selected or activated.
    Comment,
    /// `>>c comment`: a comment drawn in its color scheme.
    ColoredComment,
    /// `:color: label`: an item in its color scheme.
    Colored,
    /// An icon entry: a colored icon gutter left of the label.
    Icon,
}

impl ItemKind {
    /// `>`-prefixed items can not be selected/activated.
    pub fn is_comment(self) -> bool {
        matches!(self, ItemKind::Comment | ItemKind::ColoredComment)
    }
}

/// A parsed item line. [`Copy`]: drawn once per frame per visible item.
/// The default is the plain item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemEntry {
    pub kind: ItemKind,
    /// The scheme the prefix selects (None for plain items and plain
    /// comments — their scheme follows selection/output state).
    pub scheme: Option<Scheme>,
    /// The resolved icon glyph of an [`ItemKind::Icon`] entry.
    pub icon: Option<char>,
    /// Byte offset of the label inside the item text.
    pub label: usize,
}

/// The plain item: label = the whole line.
fn plain() -> ItemEntry {
    ItemEntry {
        kind: ItemKind::Plain,
        scheme: None,
        icon: None,
        label: 0,
    }
}

/// Parse an item line. See the module docs for the accepted spellings.
pub fn parse(text: &str) -> ItemEntry {
    match text.as_bytes().first() {
        Some(b'>') => parse_comment(text),
        Some(b':') => parse_colored(text),
        _ => plain(),
    }
}

/// `>comment` (label starts after `>`) and `>>c comment` (label after the
/// color code; an unknown code falls back to a plain comment).
fn parse_comment(text: &str) -> ItemEntry {
    let bytes = text.as_bytes();
    if bytes.get(1) == Some(&b'>') {
        match comment_scheme(bytes.get(2).copied()) {
            Some(scheme) => ItemEntry {
                kind: ItemKind::ColoredComment,
                scheme: Some(scheme),
                icon: None,
                label: 4,
            },
            None => ItemEntry {
                kind: ItemKind::Comment,
                scheme: Some(Scheme::Normal),
                icon: None,
                label: 1,
            },
        }
    } else {
        ItemEntry {
            kind: ItemKind::Comment,
            scheme: Some(Scheme::Normal),
            icon: None,
            label: 1,
        }
    }
}

/// A `:` line: `:color: label` or `:color:icon: label`.
fn parse_colored(text: &str) -> ItemEntry {
    let further_colons = text.as_bytes()[1..].iter().filter(|&&b| b == b':').count();
    if further_colons >= 2 {
        /* the icon-entry shape: valid fields make an icon entry, anything
         * else renders the exact text — a coincidence of shape must not
         * eat anything */
        return parse_icon_entry(text).unwrap_or_else(plain);
    }
    if further_colons == 1 {
        return parse_colored_entry(text).unwrap_or_else(plain);
    }
    plain()
}

/// `:color: label`, with whitespace tolerated around the color delimiter.
fn parse_colored_entry(text: &str) -> Option<ItemEntry> {
    let (color, label) = text[1..].split_once(':')?;
    Some(ItemEntry {
        kind: ItemKind::Colored,
        scheme: Some(parse_color(color)?),
        icon: None,
        label: text.len() - label.trim_start().len(),
    })
}

/// `:color:icon: label`, with whitespace tolerated around every colon.
fn parse_icon_entry(text: &str) -> Option<ItemEntry> {
    let mut fields = text[1..].splitn(3, ':');
    let color = fields.next()?;
    let icon = fields.next()?;
    let label = fields.next()?; // absent: fewer than two further colons
    let scheme = parse_color(color)?;
    let icon = icons::lookup(icon)?;
    Some(ItemEntry {
        kind: ItemKind::Icon,
        scheme: Some(scheme),
        icon: Some(icon),
        label: text.len() - label.trim_start().len(),
    })
}

/// The scheme a `:color:` field names: full names or single-letter codes.
fn parse_color(name: &str) -> Option<Scheme> {
    match name.trim().to_lowercase().as_str() {
        "red" | "r" => Some(Scheme::Red),
        "green" | "g" => Some(Scheme::Green),
        "yellow" | "y" => Some(Scheme::Yellow),
        "blue" | "b" => Some(Scheme::Selected),
        "highlight" | "h" => Some(Scheme::Highlight),
        "normal" | "default" => Some(Scheme::Normal),
        _ => None,
    }
}

/// The scheme for a `>>` color code: r/g/y/h/b.
fn comment_scheme(code: Option<u8>) -> Option<Scheme> {
    match code {
        Some(b'r') => Some(Scheme::Red),
        Some(b'g') => Some(Scheme::Green),
        Some(b'y') => Some(Scheme::Yellow),
        Some(b'h') => Some(Scheme::Highlight),
        Some(b'b') => Some(Scheme::Selected),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> ItemEntry {
        parse(text)
    }

    /* ── the icon entry ─────────────────────────────────────────────── */

    /// The canonical spellings parse to the icon, color and label.
    #[test]
    fn icon_entries_parse() {
        assert_eq!(
            parsed(":red:power: Shutdown"),
            ItemEntry {
                kind: ItemKind::Icon,
                scheme: Some(Scheme::Red),
                icon: Some('\u{f0425}'),
                label: ":red:power: ".len(),
            }
        );
        assert_eq!(
            parsed(":green:shutdown:Shutdown"),
            ItemEntry {
                kind: ItemKind::Icon,
                scheme: Some(Scheme::Green),
                icon: Some('\u{f0902}'),
                label: ":green:shutdown:".len(),
            }
        );
        // the icon may be the literal glyph
        assert_eq!(
            parsed(":b:\u{f011}: x"),
            ItemEntry {
                kind: ItemKind::Icon,
                scheme: Some(Scheme::Selected),
                icon: Some('\u{f011}'),
                label: ":b:\u{f011}: ".len(),
            }
        );
    }

    /// Whitespace around every colon is tolerated; only leading whitespace
    /// of the label goes away.
    #[test]
    fn icon_entries_tolerate_whitespace() {
        let text = ": green : power-off :    Shutdown now";
        let entry = parsed(text);
        assert_eq!(entry.kind, ItemKind::Icon);
        assert_eq!(entry.scheme, Some(Scheme::Green));
        assert_eq!(entry.icon, Some('\u{f0902}'));
        assert_eq!(&text[entry.label..], "Shutdown now");
    }

    /// Colors: full names, the legacy letters, case-insensitive.
    #[test]
    fn icon_entry_colors() {
        for (name, scheme) in [
            ("red", Scheme::Red),
            ("green", Scheme::Green),
            ("yellow", Scheme::Yellow),
            ("blue", Scheme::Selected),
            ("highlight", Scheme::Highlight),
            ("normal", Scheme::Normal),
            ("default", Scheme::Normal),
            ("g", Scheme::Green),
            ("b", Scheme::Selected),
            ("Yellow", Scheme::Yellow),
        ] {
            assert_eq!(
                parsed(&format!(":{name}:power: x")).scheme,
                Some(scheme),
                "{name}"
            );
        }
    }

    /// An invalid color or icon renders the exact text: a plain item whose
    /// label is the whole line.
    #[test]
    fn invalid_icon_entries_render_the_exact_text() {
        for text in [
            ":bogus:power: Shutdown",
            ":red:not-an-icon: Shutdown",
            ":red:: Shutdown",
            "::power: Shutdown",
            ": green :   : Shutdown",
            "h:i: not an entry",
        ] {
            let entry = parsed(text);
            assert_eq!(entry, plain(), "{text}");
            assert_eq!(&text[entry.label..], text);
        }
    }

    /// An icon entry may have an empty label (an icon-only item).
    #[test]
    fn icon_entries_may_have_an_empty_label() {
        let entry = parsed(":green:power:");
        assert_eq!(entry.kind, ItemKind::Icon);
        assert_eq!(entry.label, ":green:power:".len());
        assert_eq!(entry.icon, Some('\u{f0425}'));
    }

    /// The label keeps colons and inner whitespace; only leading
    /// whitespace after the last colon is skipped.
    #[test]
    fn icon_entry_labels_keep_their_colons() {
        let text = ":red:power: note: one  two";
        let entry = parsed(text);
        assert_eq!(&text[entry.label..], "note: one  two");
    }

    /* ── colored entries ────────────────────────────────────────────── */

    #[test]
    fn colored_entries_parse() {
        assert_eq!(
            parsed(":green: Shutdown"),
            ItemEntry {
                kind: ItemKind::Colored,
                scheme: Some(Scheme::Green),
                icon: None,
                label: ":green: ".len(),
            }
        );
        assert_eq!(
            parsed(": g :    Shutdown"),
            ItemEntry {
                kind: ItemKind::Colored,
                scheme: Some(Scheme::Green),
                icon: None,
                label: ": g :    ".len(),
            }
        );
    }

    /// Removed prefix layouts render literally rather than being partially
    /// interpreted as styled entries.
    #[test]
    fn unsupported_colored_layouts_render_exactly() {
        for text in [
            ":rtext",
            ":g label",
            ":b \u{f011}Shutdown",
            ":q: label",
            ":",
        ] {
            assert_eq!(parsed(text), plain(), "{text}");
        }
    }

    /* ── comments and plain items ───────────────────────────────────── */

    #[test]
    fn comments_parse() {
        assert_eq!(
            parsed(">note"),
            ItemEntry {
                kind: ItemKind::Comment,
                scheme: Some(Scheme::Normal),
                icon: None,
                label: 1,
            }
        );
        assert_eq!(
            parsed(">>g green note"),
            ItemEntry {
                kind: ItemKind::ColoredComment,
                scheme: Some(Scheme::Green),
                icon: None,
                label: 4,
            }
        );
        assert_eq!(parsed(">>? note").kind, ItemKind::Comment);
        assert_eq!(parsed(">>? note").scheme, Some(Scheme::Normal));
    }

    #[test]
    fn plain_items_parse() {
        let entry = parsed("Shutdown");
        assert_eq!(entry, plain());
        assert!(!entry.kind.is_comment());
        assert_eq!(parsed(""), plain());
        // only a leading : or > is special
        assert_eq!(parsed("a:b:c").kind, ItemKind::Plain);
    }
}
