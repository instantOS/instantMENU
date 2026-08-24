//! Parsed colors and the named palette used by the renderer.
//!
//! Parsing is delegated to `csscolorparser`, which covers the same surface
//! the C version got for free from Xft: `#RGB`/`#RRGGBB`/`#RRGGBBAA` hex and
//! the full X11/CSS named color palette (the hand-rolled table only shipped
//! ~18 names, silently falling back to black for everything else).

use crate::enums::{ColorRole, Scheme};

/// Parsed color (#RGB, #RRGGBB, #RRGGBBAA or an X11/CSS color name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color([u8; 4]); // r, g, b, a

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color([r, g, b, 255])
    }

    /// Construct an opaque color from a `0xRRGGBB` literal.
    pub const fn hex(rgb: u32) -> Self {
        Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
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

impl std::str::FromStr for Color {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_color(value).ok_or_else(|| format!("invalid color: `{value}`"))
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
    pub const fn new(fg: Color, bg: Color, detail: Color) -> Self {
        SchemeColors { fg, bg, detail }
    }

    pub const fn hex(fg: u32, bg: u32, detail: u32) -> Self {
        SchemeColors::new(Color::hex(fg), Color::hex(bg), Color::hex(detail))
    }

    pub fn role(self, role: ColorRole) -> Color {
        match role {
            ColorRole::Foreground => self.fg,
            ColorRole::Background => self.bg,
            ColorRole::Detail => self.detail,
        }
    }
}

/// Complete set of semantic menu colors. Named fields make theme definitions
/// independent of enum declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub normal: SchemeColors,
    pub fade: SchemeColors,
    pub highlight: SchemeColors,
    pub hover: SchemeColors,
    pub selected: SchemeColors,
    pub output: SchemeColors,
    pub green: SchemeColors,
    pub yellow: SchemeColors,
    pub red: SchemeColors,
}

impl Palette {
    /// Return the colors for a runtime-selected semantic scheme.
    pub const fn scheme(&self, scheme: Scheme) -> SchemeColors {
        match scheme {
            Scheme::Normal => self.normal,
            Scheme::Fade => self.fade,
            Scheme::Highlight => self.highlight,
            Scheme::Hover => self.hover,
            Scheme::Selected => self.selected,
            Scheme::Output => self.output,
            Scheme::Green => self.green,
            Scheme::Yellow => self.yellow,
            Scheme::Red => self.red,
        }
    }
}
