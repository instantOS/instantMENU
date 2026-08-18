//! Color parsing: hex colors, common X11 color names and color schemes.

use crate::enums::ColorRole;

/// Parsed color (#RGB, #RRGGBB, #RRGGBBAA or an X11 color name — the common
/// names are built in, anything else falls back to the first scheme color).
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

    pub fn parse(name: &str) -> Option<Self> {
        let hex = name.strip_prefix('#')?;
        match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()?;
                Some(Color([r * 17, g * 17, b * 17, 255]))
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Color([r, g, b, 255]))
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(Color([r, g, b, a]))
            }
            _ => None,
        }
    }

}

/// Table of common X11 color names (the set typically used for menu theming).
const X11_COLORS: &[(&str, &str)] = &[
    ("black", "#000000"),
    ("white", "#FFFFFF"),
    ("red", "#FF0000"),
    ("green", "#00FF00"),
    ("blue", "#0000FF"),
    ("yellow", "#FFFF00"),
    ("magenta", "#FF00FF"),
    ("cyan", "#00FFFF"),
    ("gray", "#BEBEBE"),
    ("grey", "#BEBEBE"),
    ("orange", "#FFA500"),
    ("pink", "#FFC0CB"),
    ("purple", "#A020F0"),
    ("brown", "#A52A2A"),
    ("darkred", "#8B0000"),
    ("darkgreen", "#006400"),
    ("darkblue", "#00008B"),
    ("lightgray", "#D3D3D3"),
    ("lightgrey", "#D3D3D3"),
];

pub fn parse_color(name: &str) -> Option<Color> {
    if let Some(c) = Color::parse(name) {
        return Some(c);
    }
    let lower = name.trim().to_ascii_lowercase();
    for (n, hex) in X11_COLORS {
        if *n == lower {
            return Color::parse(hex);
        }
    }
    None
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
