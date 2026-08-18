//! Color parsing: hex colors, X11/CSS color names and color schemes.
//!
//! Parsing is delegated to `csscolorparser`, which covers the same surface
//! the C version got for free from Xft: `#RGB`/`#RRGGBB`/`#RRGGBBAA` hex and
//! the full X11/CSS named color palette (the hand-rolled table only shipped
//! ~18 names, silently falling back to black for everything else).

use crate::enums::ColorRole;

/// Parsed color (#RGB, #RRGGBB, #RRGGBBAA or an X11 color name — anything
/// else falls back to the first scheme color).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color([u8; 4]); // r, g, b, a

impl Color {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color([r, g, b, 255])
    }

    pub fn r(self) -> u8 {
        self.0[0]
    }

    pub fn g(self) -> u8 {
        self.0[1]
    }

    pub fn b(self) -> u8 {
        self.0[2]
    }

    pub fn a(self) -> u8 {
        self.0[3]
    }

    /// The raw RGBA channel array, for blitting into a canvas.
    pub fn channels(self) -> [u8; 4] {
        self.0
    }
}

pub fn parse_color(name: &str) -> Option<Color> {
    let parsed = csscolorparser::parse(name.trim()).ok()?;
    let [r, g, b, a] = parsed.to_rgba8();
    Some(Color([r, g, b, a]))
}

/// A color scheme as configured (fg/bg/detail color strings), before parsing.
#[derive(Debug, Clone)]
pub struct SchemeStrings {
    pub fg: String,
    pub bg: String,
    pub detail: String,
}

impl SchemeStrings {
    pub fn role_mut(&mut self, role: ColorRole) -> &mut String {
        match role {
            ColorRole::Foreground => &mut self.fg,
            ColorRole::Background => &mut self.bg,
            ColorRole::Detail => &mut self.detail,
        }
    }
}

/// A parsed color scheme (port of `Clr *scheme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemeColors {
    pub fg: Color,
    pub bg: Color,
    pub detail: Color,
}

impl SchemeColors {
    pub fn role(self, role: ColorRole) -> Color {
        match role {
            ColorRole::Foreground => self.fg,
            ColorRole::Background => self.bg,
            ColorRole::Detail => self.detail,
        }
    }
}

impl Default for SchemeColors {
    fn default() -> Self {
        SchemeColors {
            fg: Color::rgb(0, 0, 0),
            bg: Color::rgb(0, 0, 0),
            detail: Color::rgb(0, 0, 0),
        }
    }
}

pub fn scheme_from_strings(strings: &SchemeStrings) -> SchemeColors {
    let parse = |name: &str| {
        parse_color(name).unwrap_or_else(|| {
            eprintln!("error, cannot allocate color '{name}'");
            Color::rgb(0, 0, 0)
        })
    };
    SchemeColors {
        fg: parse(&strings.fg),
        bg: parse(&strings.bg),
        detail: parse(&strings.detail),
    }
}
