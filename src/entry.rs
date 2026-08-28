//! Parsing for instantMENU's line-oriented item format.
//!
//! A plain line is both the visible label and the value written to stdout:
//!
//! ```text
//! Display
//! ```
//!
//! An optional leading attribute block adds presentation and matching
//! metadata without becoming part of the label or output:
//!
//! ```text
//! {blue icon=display key=d match="monitor screen"} Display
//! {heading green} System actions
//! {value="/tmp/a b"} My File
//! ```
//!
//! Attributes are whitespace-separated. Values containing whitespace may be
//! single- or double-quoted; a backslash quotes the following character.
//! Known color names are shorthand for `color=<name>`, so `{red}` and
//! `{color=red}` are equivalent. A line is interpreted as markup only when
//! the complete leading block is valid; otherwise it remains literal text.

use crate::enums::Scheme;
use crate::icons;

/// Parsed metadata which affects one menu item.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemEntry {
    /// Optional item color. Ordinary items use it while selected; a
    /// heading carries it at all times.
    pub scheme: Option<Scheme>,
    /// Optional glyph drawn in the icon gutter.
    pub icon: Option<char>,
    /// Explicit activation key used by `--single-key` mode.
    pub key: Option<char>,
    /// Structural, non-selectable heading row.
    pub heading: bool,
}

impl ItemEntry {
    pub fn is_heading(self) -> bool {
        self.heading
    }
}

/// The borrowed result of parsing one source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedItem<'a> {
    /// Visible label and default output value.
    pub label: &'a str,
    /// Extra text used only for regular-menu matching.
    pub match_text: Option<String>,
    /// Optional output value printed when selected.
    pub value: Option<String>,
    pub entry: ItemEntry,
}

impl<'a> ParsedItem<'a> {
    fn plain(text: &'a str) -> Self {
        ParsedItem {
            label: text,
            match_text: None,
            value: None,
            entry: ItemEntry::default(),
        }
    }
}

/// Parse one item line. Invalid or incomplete markup is deliberately literal:
/// arbitrary command output remains safe to pipe into instantMENU, while a
/// valid block has strict names, values and duplicate checking.
pub fn parse(text: &str) -> ParsedItem<'_> {
    if text.starts_with("{{") {
        return ParsedItem::plain(&text[1..]);
    }
    parse_markup(text).unwrap_or_else(|| ParsedItem::plain(text))
}

fn parse_markup(text: &str) -> Option<ParsedItem<'_>> {
    let body = text.strip_prefix('{')?;
    let end = closing_brace(body)?;
    let after = &body[end + 1..];
    if !after.is_empty() && !after.starts_with(char::is_whitespace) {
        return None;
    }

    let mut scanner = AttributeScanner::new(&body[..end]);
    let mut entry = ItemEntry::default();
    let mut match_text = None;
    let mut value: Option<String> = None;
    let mut saw_attribute = false;
    let mut saw_color = false;
    let mut saw_icon = false;
    let mut saw_key = false;
    let mut saw_heading = false;

    while let Some(attribute) = scanner.next_attribute().ok()? {
        saw_attribute = true;
        match (attribute.name.as_str(), attribute.value) {
            ("color", Some(value)) if !saw_color => {
                entry.scheme = Some(parse_color(&value)?);
                saw_color = true;
            }
            ("icon", Some(value)) if !saw_icon => {
                entry.icon = Some(icons::lookup(&value)?);
                saw_icon = true;
            }
            ("key", Some(value)) if !saw_key => {
                let mut chars = value.chars();
                let key = chars.next()?;
                if chars.next().is_some() || key.is_control() || key.is_whitespace() {
                    return None;
                }
                entry.key = Some(key);
                saw_key = true;
            }
            ("match", Some(value)) if match_text.is_none() && !value.is_empty() => {
                match_text = Some(value);
            }
            ("value", Some(v)) if value.is_none() && !v.is_empty() => {
                value = Some(v);
            }
            ("heading", None) if !saw_heading => {
                entry.heading = true;
                saw_heading = true;
            }
            (name, None) if !saw_color => {
                entry.scheme = Some(parse_color(name)?);
                saw_color = true;
            }
            _ => return None,
        }
    }

    if !saw_attribute
        || (entry.heading && (entry.key.is_some() || match_text.is_some() || value.is_some()))
    {
        return None;
    }

    Some(ParsedItem {
        label: after.trim_start(),
        match_text,
        value,
        entry,
    })
}

/// Find the first unquoted closing brace. Metadata is intentionally flat;
/// nested blocks are not part of the language.
fn closing_brace(body: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '}' => return Some(i),
            '{' => return None,
            _ => {}
        }
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
struct Attribute {
    name: String,
    value: Option<String>,
}

/// A small shell-like scanner for the flat contents of `{...}`. It accepts
/// `name=value`, quoted values, and bare shorthand words; whitespace around
/// `=` is intentionally not accepted so the grammar remains unambiguous.
struct AttributeScanner<'a> {
    text: &'a str,
    cursor: usize,
}

impl<'a> AttributeScanner<'a> {
    fn new(text: &'a str) -> Self {
        AttributeScanner { text, cursor: 0 }
    }

    fn next_attribute(&mut self) -> Result<Option<Attribute>, ()> {
        self.skip_whitespace();
        if self.cursor == self.text.len() {
            return Ok(None);
        }

        let name_start = self.cursor;
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == '=' {
                break;
            }
            if matches!(c, '{' | '}' | '\'' | '"' | '\\') {
                return Err(());
            }
            self.bump(c);
        }
        if self.cursor == name_start {
            return Err(());
        }
        let name = self.text[name_start..self.cursor].to_ascii_lowercase();

        let value = if self.peek() == Some('=') {
            self.cursor += 1;
            Some(self.value().ok_or(())?)
        } else {
            None
        };
        if self.peek().is_some_and(|c| !c.is_whitespace()) {
            return Err(());
        }
        Ok(Some(Attribute { name, value }))
    }

    fn value(&mut self) -> Option<String> {
        let first = self.peek()?;
        if matches!(first, '\'' | '"') {
            self.bump(first);
            let quote = first;
            let mut value = String::new();
            let mut escaped = false;
            while let Some(c) = self.peek() {
                self.bump(c);
                if escaped {
                    value.push(c);
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == quote {
                    return Some(value);
                } else {
                    value.push(c);
                }
            }
            None
        } else {
            let start = self.cursor;
            while let Some(c) = self.peek() {
                if c.is_whitespace() {
                    break;
                }
                if matches!(c, '=' | '{' | '}' | '\'' | '"' | '\\') {
                    return None;
                }
                self.bump(c);
            }
            (self.cursor > start).then(|| self.text[start..self.cursor].to_string())
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(c) = self.peek() {
            if !c.is_whitespace() {
                break;
            }
            self.bump(c);
        }
    }

    fn peek(&self) -> Option<char> {
        self.text[self.cursor..].chars().next()
    }

    fn bump(&mut self, c: char) {
        self.cursor += c.len_utf8();
    }
}

/// Named item schemes. `blue` is the friendly alias for the palette's
/// historical `selected` role; `default` aliases `normal`.
fn parse_color(name: &str) -> Option<Scheme> {
    match name.to_ascii_lowercase().as_str() {
        "normal" | "default" => Some(Scheme::Normal),
        "fade" => Some(Scheme::Fade),
        "highlight" => Some(Scheme::Highlight),
        "hover" => Some(Scheme::Hover),
        "selected" | "blue" => Some(Scheme::Selected),
        "output" => Some(Scheme::Output),
        "green" => Some(Scheme::Green),
        "yellow" => Some(Scheme::Yellow),
        "red" => Some(Scheme::Red),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_lines_are_unchanged() {
        for text in ["Display", "", ":red:power: Shutdown", ">Heading"] {
            assert_eq!(parse(text), ParsedItem::plain(text), "{text}");
        }
    }

    #[test]
    fn doubled_opening_brace_escapes_a_literal_markup_like_label() {
        let parsed = parse("{{red} Literal braces");
        assert_eq!(parsed, ParsedItem::plain("{red} Literal braces"));
    }

    #[test]
    fn color_shorthand_and_explicit_color_are_equivalent() {
        let short = parse("{red} Shut down");
        let explicit = parse("{color=red} Shut down");
        assert_eq!(short, explicit);
        assert_eq!(short.label, "Shut down");
        assert_eq!(short.entry.scheme, Some(Scheme::Red));
    }

    #[test]
    fn every_named_scheme_parses() {
        for (name, scheme) in [
            ("normal", Scheme::Normal),
            ("default", Scheme::Normal),
            ("fade", Scheme::Fade),
            ("highlight", Scheme::Highlight),
            ("hover", Scheme::Hover),
            ("selected", Scheme::Selected),
            ("blue", Scheme::Selected),
            ("output", Scheme::Output),
            ("green", Scheme::Green),
            ("yellow", Scheme::Yellow),
            ("red", Scheme::Red),
        ] {
            assert_eq!(parse(&format!("{{{name}}} x")).entry.scheme, Some(scheme));
            assert_eq!(
                parse(&format!("{{color={name}}} x")).entry.scheme,
                Some(scheme)
            );
        }
    }

    #[test]
    fn attributes_compose_in_any_order() {
        let parsed = parse("{key=d blue icon=display match='monitor screen'} Display");
        assert_eq!(parsed.label, "Display");
        assert_eq!(parsed.match_text.as_deref(), Some("monitor screen"));
        assert_eq!(parsed.entry.scheme, Some(Scheme::Selected));
        assert_eq!(parsed.entry.icon, icons::lookup("display"));
        assert_eq!(parsed.entry.key, Some('d'));
        assert!(!parsed.entry.heading);
        assert_eq!(parsed.value, None);
    }

    #[test]
    fn value_is_hidden_output_without_changing_label_or_search() {
        let parsed = parse("{value=one} same");
        assert_eq!(parsed.label, "same");
        assert_eq!(parsed.value.as_deref(), Some("one"));
        assert_eq!(parsed.match_text, None);

        let with_match = parse("{value=two match=alt} Label");
        assert_eq!(with_match.label, "Label");
        assert_eq!(with_match.value.as_deref(), Some("two"));
        assert_eq!(with_match.match_text.as_deref(), Some("alt"));

        let quoted = parse(r#"{value="file:/tmp/a b"} My File"#);
        assert_eq!(quoted.label, "My File");
        assert_eq!(quoted.value.as_deref(), Some("file:/tmp/a b"));

        let icon_value = parse("{value=one icon=star} same");
        assert_eq!(icon_value.label, "same");
        assert_eq!(icon_value.value.as_deref(), Some("one"));
        assert_eq!(icon_value.entry.icon, icons::lookup("star"));

        // value composes in any order with other attributes
        let any_order = parse("{icon=display value=out match=alt red} X");
        assert_eq!(any_order.label, "X");
        assert_eq!(any_order.value.as_deref(), Some("out"));
        assert_eq!(any_order.match_text.as_deref(), Some("alt"));
        assert_eq!(any_order.entry.scheme, Some(Scheme::Red));
    }

    #[test]
    fn quoted_values_support_spaces_braces_and_escapes() {
        let parsed = parse(r#"{match="one \"two\" } three" color=green} Label"#);
        assert_eq!(parsed.label, "Label");
        assert_eq!(parsed.match_text.as_deref(), Some("one \"two\" } three"));
    }

    #[test]
    fn unicode_keys_are_single_characters() {
        assert_eq!(parse("{key=λ} Lambda").entry.key, Some('λ'));
        for invalid in ["{key=} x", "{key=ab} x", "{key=' '} x", "{key='\t'} x"] {
            assert_eq!(parse(invalid), ParsedItem::plain(invalid), "{invalid}");
        }
    }

    #[test]
    fn headings_may_be_styled_but_not_searchable_or_activated() {
        let heading = parse("{heading green icon=display} Displays");
        assert!(heading.entry.heading);
        assert_eq!(heading.entry.scheme, Some(Scheme::Green));
        assert_eq!(heading.entry.icon, icons::lookup("display"));

        for invalid in [
            "{heading key=d} Displays",
            "{heading match=monitor} Displays",
            "{heading value=one} Displays",
        ] {
            assert_eq!(parse(invalid), ParsedItem::plain(invalid), "{invalid}");
        }
    }

    #[test]
    fn invalid_markup_is_literal_and_never_partially_consumed() {
        for text in [
            "{} Empty",
            "{purple} Unknown color",
            "{color=gren} Typo",
            "{icon=not-an-icon} Unknown icon",
            "{wat=yes} Unknown attribute",
            "{red green} Duplicate color",
            "{color=red color=green} Duplicate field",
            "{heading heading} Duplicate flag",
            "{match=} Empty match",
            "{value=} Empty value",
            "{value=one value=two} Duplicate value",
            "{value =one} Space before equals",
            "{red}No separator",
            "{red Nested { block} Label",
            "{red Unclosed",
            "{match=\"unclosed} Label",
            "{match =value} Label",
            "{value=\"unclosed} Label",
        ] {
            assert_eq!(parse(text), ParsedItem::plain(text), "{text}");
        }
    }

    #[test]
    fn labels_may_be_empty_and_leading_separator_space_is_not_rendered() {
        assert_eq!(parse("{icon=power}").label, "");
        assert_eq!(parse("{red}    padded").label, "padded");
    }
}
