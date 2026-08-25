//! Parsing of item line prefixes: how a line renders — a plain item, a
//! `>` comment, a colored item, or an icon entry — and where its label
//! starts inside the text.
//!
//! Icon entries spell `:color:icon: label` (readable by humans and
//! unambiguous for the scripts and agents that produce them):
//!
//! ```text
//! :red:power: Shutdown
//! : green : power-off :     Shutdown now
//! ```
//!
//! Whitespace around each colon is tolerated; everything after the third
//! colon is the label (leading whitespace skipped, the rest kept verbatim).
//! The color is a scheme name, the icon a Nerd Fonts glyph, its name, or a
//! hex codepoint (see [`crate::icons`]). An icon entry whose color or icon
//! does not resolve is a plain item showing the **exact** original line —
//! a line that only coincidentally has the shape must not lose text.
//!
//! The old spellings stay working: `:x label` colors the item, and
//! `:x <glyph>label` (a literal glyph right after the space) keeps its icon
//! gutter. What no longer happens is the old reading of `:x label` as
//! "icon = first letter of the label", which ate the letter and glued the
//! icon onto the word.

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
    /// `:x label` / `:xlabel` (legacy): an item in its color scheme.
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

/// A `:` line: the `:color:icon: label` shape, else the legacy `:x` forms.
fn parse_colored(text: &str) -> ItemEntry {
    let further_colons = text.as_bytes()[1..].iter().filter(|&&b| b == b':').count();
    if further_colons >= 2 {
        /* the icon-entry shape: valid fields make an icon entry, anything
         * else renders the exact text — a coincidence of shape must not
         * eat anything */
        return parse_icon_entry(text).unwrap_or_else(plain);
    }
    legacy_colored(text).unwrap_or_else(plain)
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

/// Legacy `:x…`: `x` is a color code (r/g/y/b). `:x <glyph>…` with a
/// literal icon glyph right after the space stays an icon entry (the old
/// icon syntax); `:x label` is a colored item showing its whole label.
fn legacy_colored(text: &str) -> Option<ItemEntry> {
    let scheme = colored_scheme(text.as_bytes().get(1).copied())?;
    if let Some(rest) = text[2..].strip_prefix(char::is_whitespace) {
        if let Some(glyph) = rest.chars().next() {
            if icons::is_icon_char(glyph) || icons::is_emoji_char(glyph) {
                return Some(ItemEntry {
                    kind: ItemKind::Icon,
                    scheme: Some(scheme),
                    icon: Some(glyph),
                    label: 3 + glyph.len_utf8(),
                });
            }
        }
        return Some(ItemEntry {
            kind: ItemKind::Colored,
            scheme: Some(scheme),
            icon: None,
            label: text.len() - rest.trim_start().len(),
        });
    }
    Some(ItemEntry {
        kind: ItemKind::Colored,
        scheme: Some(scheme),
        icon: None,
        label: 2,
    })
}

/// The scheme a `:color:` field names: the full names, plus the legacy
/// single-letter codes.
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

/// The scheme for a legacy `:x` colored item: r/g/y/b only — like the C
/// `drawitem` `:x` branch, a selected item with an unknown code falls back
/// to a plain item (which is what [`parse`] makes of it).
fn colored_scheme(code: Option<u8>) -> Option<Scheme> {
    match code {
        Some(b'r') => Some(Scheme::Red),
        Some(b'g') => Some(Scheme::Green),
        Some(b'y') => Some(Scheme::Yellow),
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

    /* ── legacy ─────────────────────────────────────────────────────── */

    /// `:x <glyph>label` (the old icon syntax) keeps its gutter.
    #[test]
    fn legacy_glyph_icons_still_parse() {
        assert_eq!(
            parsed(":b \u{f011}Shutdown"),
            ItemEntry {
                kind: ItemKind::Icon,
                scheme: Some(Scheme::Selected),
                icon: Some('\u{f011}'),
                label: 3 + '\u{f011}'.len_utf8(),
            }
        );
        // a non-glyph after the space is a label now, not an eaten letter
        assert_eq!(
            parsed(":g shutdown"),
            ItemEntry {
                kind: ItemKind::Colored,
                scheme: Some(Scheme::Green),
                icon: None,
                label: ":g ".len(),
            }
        );
        // several spaces of separation collapse for the label start
        assert_eq!(
            &":r   red item"[parsed(":r   red item").label..],
            "red item"
        );
    }

    /// `:xlabel` without the space is the colored item it always was.
    #[test]
    fn legacy_colored_items() {
        assert_eq!(
            parsed(":rtext"),
            ItemEntry {
                kind: ItemKind::Colored,
                scheme: Some(Scheme::Red),
                icon: None,
                label: 2,
            }
        );
        // unknown color code: exact text, selected or not
        assert_eq!(parsed(":q text"), plain());
        assert_eq!(parsed(":h text"), plain()); // h is comment-only
        assert_eq!(parsed(":"), plain());
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
