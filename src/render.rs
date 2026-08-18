//! Port of `drw.c` — backend-agnostic drawing.
//!
//! All rendering happens into a shared RGBA8 canvas which the X11 and Wayland
//! backends blit to the window. Text rendering uses cosmic-text with the same
//! font-fallback idea as the Xft fontset: the primary font, then the secondary
//! fonts for icon/emoji ranges, then cosmic-text's own fallback.

use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};

use crate::enums::{COL_BG, COL_DETAIL, COL_FG};

/// A parsed Xft-style font description: "Family:size=12:pixelsize=20:..."
#[derive(Debug, Clone)]
pub struct FontSpec {
    pub family: String,
    /// Pixel size (already converted from points for `size=`).
    pub px: f32,
}

pub fn parse_font_name(name: &str) -> FontSpec {
    let mut family = name.to_string();
    let mut px: f32 = 0.0;
    for (i, part) in name.split(':').enumerate() {
        if i == 0 {
            family = part.to_string();
            continue;
        }
        if let Some(value) = part.strip_prefix("pixelsize=") {
            if let Ok(v) = value.parse::<f32>() {
                px = v;
            }
        } else if let Some(value) = part.strip_prefix("size=") {
            if let Ok(v) = value.parse::<f32>() {
                // Xft converts points to pixels via dpi (96 by default).
                px = (v * 96.0 / 72.0).round();
            }
        }
    }
    if px <= 0.0 {
        px = 12.0 * 96.0 / 72.0;
    }
    FontSpec { family, px }
}

/// Parsed color (#RGB, #RRGGBB, #RRGGBBAA or an X11 color name — the common
/// names are built in, anything else falls back to the first scheme color).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 4]); // r, g, b, a

impl Color {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color([r, g, b, 255])
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

/// A color scheme, port of `Clr *scheme` (fg, bg, detail).
pub type SchemeColors = [Color; 3];

pub fn scheme_from_strings(names: &[String; 3]) -> SchemeColors {
    let mut out = [Color::rgb(0, 0, 0); 3];
    for (i, name) in names.iter().enumerate() {
        out[i] = parse_color(name).unwrap_or_else(|| {
            eprintln!("error, cannot allocate color '{name}'");
            Color::rgb(0, 0, 0)
        });
    }
    out
}

/// The pixel canvas both backends present. RGBA8, row-major.
pub struct Canvas {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Self {
        Canvas {
            width,
            height,
            data: vec![0; (width.max(0) as usize) * (height.max(0) as usize) * 4],
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.data = vec![0; (width.max(0) as usize) * (height.max(0) as usize) * 4];
    }

    #[inline]
    pub fn fill_pixel(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let off = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        self.data[off..off + 4].copy_from_slice(&color);
    }

    /// Blit an alpha mask (8bpp) at (x, y) with the given color — used by the
    /// cosmic-text draw callbacks.
    #[inline]
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let off = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        let alpha = color[3] as u32;
        if alpha >= 255 {
            self.data[off..off + 4].copy_from_slice(&color);
        } else if alpha == 0 {
            return;
        } else {
            let inv = 255 - alpha;
            for i in 0..3 {
                let dst = self.data[off + i] as u32;
                let src = color[i] as u32;
                self.data[off + i] = ((src * alpha + dst * inv) / 255) as u8;
            }
            self.data[off + 3] = 255;
        }
    }
}

/// The shared drawing context: fontset + color schemes + canvas, port of
/// `Drw` plus the scheme state of instantmenu.c.
pub struct Renderer {
    pub font_system: FontSystem,
    swash_cache: SwashCache,
    /// Parsed font specs, primary first (drw `fonts`).
    pub fonts: Vec<FontSpec>,
    /// Resolved cosmic-text family names in fontset order.
    families: Vec<String>,
    /// font height of the primary font (`drw->fonts->h` = ascent + descent).
    pub font_height: i32,
    /// sum of left and right padding (`lrpad`)
    pub lrpad: i32,
    /// Color schemes.
    pub schemes: Vec<SchemeColors>,
    /// Currently set scheme (drw_setscheme).
    pub scheme: SchemeColors,

    // text width cache
    width_cache: HashMap<(String, u32), i32>,
}

impl Renderer {
    /// Create the renderer and load the fontset. Mirrors `drw_fontset_create`
    /// + `drw_scm_create` + `lrpad = drw->fonts->h`.
    pub fn new(fonts: &[String], scheme_strings: &[[String; 3]; 9]) -> Self {
        let mut font_system = FontSystem::new();
        let specs: Vec<FontSpec> = fonts.iter().map(|f| parse_font_name(f)).collect();

        // Resolve each family against the system font database, loosely
        // (fontconfig-style), falling back to the raw name.
        let mut families = Vec::new();
        for spec in &specs {
            let resolved = resolve_family(&font_system, &spec.family);
            families.push(resolved);
        }

        let font_height = primary_font_height(&mut font_system, &families, specs[0].px);

        let mut renderer = Renderer {
            schemes: scheme_strings.iter().map(scheme_from_strings).collect(),
            swash_cache: SwashCache::new(),
            font_system,
            fonts: specs,
            families,
            font_height,
            lrpad: font_height,
            scheme: [Color::rgb(0, 0, 0); 3],
            width_cache: HashMap::new(),
        };
        renderer.scheme = renderer.schemes[0];
        renderer
    }

    /// drw_setscheme
    pub fn setscheme(&mut self, scheme: SchemeColors) {
        self.scheme = scheme;
    }

    pub fn scheme(&self, index: usize) -> SchemeColors {
        self.schemes[index]
    }

    /// drw_rect — filled rect in the current scheme; `invert` swaps fg/bg,
    /// `rounded` paints the bottom 4px strip with the detail color.
    pub fn rect(&mut self, canvas: &mut Canvas, x: i32, y: i32, w: i32, h: i32, filled: bool, invert: bool, rounded: bool) {
        let color = if invert { self.scheme[COL_BG] } else { self.scheme[COL_FG] };
        if filled && h < 40 {
            if rounded {
                self.fill_rect(canvas, x, y, w, h - 4, color);
                self.fill_rect(canvas, x, y + h - 4, w, 4, self.scheme[COL_DETAIL]);
            } else {
                self.fill_rect(canvas, x, y, w, h, color);
            }
        } else {
            self.fill_rect(canvas, x, y, w, h, color);
        }
    }

    fn fill_rect(&self, canvas: &mut Canvas, x: i32, y: i32, w: i32, h: i32, color: Color) {
        if w <= 0 || h <= 0 {
            return;
        }
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(canvas.width);
        let y1 = (y + h).min(canvas.height);
        let px = [color.r(), color.g(), color.b(), color.a()];
        for yy in y0..y1 {
            let row_start = (yy as usize * canvas.width as usize + x0 as usize) * 4;
            let row_end = row_start + (x1 - x0) as usize * 4;
            for off in (row_start..row_end).step_by(4) {
                canvas.data[off..off + 4].copy_from_slice(&px);
            }
        }
    }

    /// `drw_fontset_getwidth` — width of `text` (without lrpad).
    pub fn text_width(&mut self, text: &str) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let px = self.fonts[0].px;
        let key = (text.to_string(), px.to_bits());
        if let Some(w) = self.width_cache.get(&key) {
            return *w;
        }
        let buffer = self.make_buffer(text, None);
        let width = self.shape_width(buffer);
        if self.width_cache.len() > 8192 {
            self.width_cache.clear();
        }
        self.width_cache.insert(key, width);
        width
    }

    /// `drw_fontset_getwidth_clamp` — width of `text` clamped to `n`.
    pub fn text_width_clamp(&mut self, text: &str, n: i32) -> i32 {
        if n == 0 {
            return 0;
        }
        self.text_width(text).min(n)
    }

    fn make_buffer(&mut self, text: &str, max_width: Option<f32>) -> Buffer {
        let px = self.fonts[0].px;
        let metrics = Metrics::new(px, self.font_height as f32);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_wrap(Wrap::None);
        if let Some(w) = max_width {
            buffer.set_size(Some(w), Some(self.font_height as f32));
        } else {
            buffer.set_size(None, None);
        }
        self.set_buffer_text(&mut buffer, text);
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
    }

    /// Set text on a buffer, splitting runs by Unicode range so icon and emoji
    /// ranges use the secondary fonts of the fontset (fontset fallback).
    fn set_buffer_text(&self, buffer: &mut Buffer, text: &str) {
        let primary = self.families.first().cloned().unwrap_or_default();
        let secondary = self.families.get(1).cloned().unwrap_or(primary.clone());
        let emoji = self.families.get(2).cloned().unwrap_or(secondary.clone());

        let default_attrs = Attrs::new().family(Family::Name(&primary));
        let mut spans: Vec<(&str, Attrs)> = Vec::new();
        let mut start = 0usize;
        let mut current = char_class(text.chars().next());
        for (index, ch) in text.char_indices().skip(1) {
            let class = char_class(Some(ch));
            if class != current {
                let family = match current {
                    CharClass::Icon => &secondary,
                    CharClass::Emoji => &emoji,
                    CharClass::Normal => &primary,
                };
                spans.push((&text[start..index], Attrs::new().family(Family::Name(family))));
                start = index;
                current = class;
            }
        }
        if start < text.len() {
            let family = match current {
                CharClass::Icon => &secondary,
                CharClass::Emoji => &emoji,
                CharClass::Normal => &primary,
            };
            spans.push((&text[start..], Attrs::new().family(Family::Name(family))));
        }
        buffer.set_rich_text(spans, &default_attrs, Shaping::Basic, None);
    }

    fn shape_width(&mut self, mut buffer: Buffer) -> i32 {
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
            .ceil() as i32
    }

    /// drw_text — draw `text` at (x, y, w, h) with `lpad` padding. `invert`
    /// swaps fg/bg, `rounded` paints a 4px detail strip at the bottom and
    /// shifts the text up by 2px. Text that does not fit is truncated with an
    /// ellipsis ("..."). Returns the x position after the drawn text.
    pub fn text(
        &mut self,
        canvas: &mut Canvas,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        lpad: i32,
        text: &str,
        invert: bool,
        rounded: bool,
    ) -> i32 {
        let render = x != 0 || y != 0 || w != 0 || h != 0;
        if !render {
            // measuring call: width of the full text
            return self.text_width(text);
        }
        if w == 0 {
            return x;
        }

        // background
        let fill = if invert { self.scheme[COL_FG] } else { self.scheme[COL_BG] };
        if rounded {
            self.fill_rect(canvas, x, y, w, h - 4, fill);
            self.fill_rect(canvas, x, y + h - 4, w, 4, self.scheme[COL_DETAIL]);
        } else {
            self.fill_rect(canvas, x, y, w, h, fill);
        }
        if w < lpad {
            return x + w;
        }

        let color = if invert { self.scheme[COL_BG] } else { self.scheme[COL_FG] };
        let cosmic_color =
            CosmicColor::rgba(color.r(), color.g(), color.b(), color.a());

        let avail = w - lpad;
        let mut display_text = text;
        let ellipsis_width = self.text_width("...");
        let full_width = self.text_width(text);
        let mut drawn_width = full_width;
        if full_width > avail {
            // find the longest prefix after which an ellipsis still fits
            let max = (avail - ellipsis_width).max(0);
            let chars: Vec<(usize, char)> = text.char_indices().collect();
            // binary search over char count
            let mut lo = 0usize;
            let mut hi = chars.len();
            while lo < hi {
                let mid = (lo + hi + 1) / 2;
                let end = if mid < chars.len() { chars[mid].0 } else { text.len() };
                if self.text_width(&text[..end]) <= max {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            let end = if lo < chars.len() { chars[lo].0 } else { text.len() };
            display_text = &text[..end];
            drawn_width = self.text_width(display_text) + ellipsis_width;
            // draw ellipsis right after the truncated text
            let ell_x = x + lpad + self.text_width(display_text);
            self.draw_run(canvas, ell_x, y, h, "...", cosmic_color, rounded);
        }
        if !display_text.is_empty() {
            self.draw_run(canvas, x + lpad, y, h, display_text, cosmic_color, rounded);
        }

        // drw_text returns the advanced x plus remaining w, which is x + w for
        // the non-overflow case; overflow callers still get the cell edge.
        let _ = drawn_width;
        x + w
    }

    fn draw_run(
        &mut self,
        canvas: &mut Canvas,
        x: i32,
        y: i32,
        h: i32,
        text: &str,
        color: CosmicColor,
        rounded: bool,
    ) {
        let px = self.fonts[0].px;
        let metrics = Metrics::new(px, self.font_height as f32);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, Some(self.font_height as f32));
        self.set_buffer_text(&mut buffer, text);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let width = canvas.width;
        let height = canvas.height;
        // vertical centering like drw_text:
        // ty = y + (h - usedfont->h)/2 + ascent, here the buffer baseline sits
        // at ascent within font_height rows.
        let y_off = y + (h - self.font_height) / 2 - if rounded { 2 } else { 0 };

        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            color,
            |gx, gy, _gw, _gh, c| {
                let px_color = [c.r(), c.g(), c.b(), c.a()];
                let cx = x + gx;
                let cy = y_off + gy;
                if cx < 0 || cy < 0 || cx >= width || cy >= height {
                    return;
                }
                canvas.blend_pixel(cx, cy, px_color);
            },
        );
    }
}

#[derive(PartialEq, Clone, Copy)]
enum CharClass {
    Normal,
    Icon,
    Emoji,
}

fn char_class(ch: Option<char>) -> CharClass {
    match ch {
        Some(c) if matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD) => {
            CharClass::Icon
        }
        Some(c) if (c as u32) >= 0x1F000 || matches!(c as u32, 0x2600..=0x27BF | 0x2190..=0x21FF | 0x2B00..=0x2BFF) => {
            CharClass::Emoji
        }
        _ => CharClass::Normal,
    }
}

/// Resolve an Xft-style family name to a family present in the font database
/// (fontconfig-style loose matching, like the instantWM text rasterizer).
fn resolve_family(font_system: &FontSystem, configured: &str) -> String {
    let db = font_system.db();
    // exact match first
    let exact = db
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name))
        .find(|name| name.eq_ignore_ascii_case(configured))
        .cloned();
    if let Some(name) = exact {
        return name;
    }
    let wanted = normalized_family(configured);
    let loose = db
        .faces()
        .flat_map(|face| face.families.iter().map(|(name, _)| name))
        .find(|name| normalized_family(name) == wanted)
        .cloned();
    if let Some(name) = loose {
        return name;
    }
    // family names may embed the style: "Inter-Regular" -> try "Inter"
    if let Some((base, _rest)) = configured.rsplit_once('-') {
        if !base.is_empty() {
            let base_wanted = normalized_family(base);
            let base_hit = db
                .faces()
                .flat_map(|face| face.families.iter().map(|(name, _)| name))
                .find(|name| normalized_family(name) == base_wanted)
                .cloned();
            if let Some(name) = base_hit {
                return name;
            }
        }
    }
    configured.to_string()
}

fn normalized_family(family: &str) -> String {
    family
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Height (ascent + descent) of the primary font at `px`, matching Xft's
/// `font->h = ascent + descent`.
fn primary_font_height(font_system: &mut FontSystem, families: &[String], px: f32) -> i32 {
    let db = font_system.db();
    let mut height = (px * 1.2).ceil() as i32;
    for family in families {
        let query = fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        };
        let Some(face_id) = db.query(&query) else { continue };
        /* FaceInfo carries no metrics; read them from the face data itself */
        let h = db
            .with_face_data(face_id, |data, index| {
                ttf_parser::Face::parse(data, index).map_or(0, |face| {
                    let upem = face.units_per_em().max(1) as f32;
                    let ascent = face.ascender() as f32;
                    let descent = face.descender() as f32; // negative
                    ((ascent - descent) / upem * px).round() as i32
                })
            })
            .unwrap_or(0);
        if h <= 0 {
            continue;
        }
        height = h;
        break;
    }
    height
}
