//! The shared drawing context: fontset + color schemes + canvas, port of
//! `Drw` plus the scheme state of instantmenu.c.

use std::collections::{HashMap, HashSet};

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};

use crate::enums::Scheme;

use super::canvas::Canvas;
use super::color::{scheme_from_strings, Color, SchemeColors, SchemeStrings};
use super::font::{parse_font_name, primary_font_height, resolve_family, FontSpec};
use super::fontconfig;

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
    pub horizontal_padding: i32,
    /// Color schemes.
    pub schemes: Vec<SchemeColors>,
    /// Currently set scheme (drw_setscheme).
    pub scheme: SchemeColors,

    frame_background: Option<Color>,

    // Shaped text is reusable for both measurement and drawing.
    layout_cache: HashMap<String, TextLayout>,
    // Characters whose fallback coverage has already been checked. This lets
    // pasted text extend the small startup database without repeated queries.
    checked_chars: HashSet<char>,
}

struct TextLayout {
    width: i32,
    buffer: Buffer,
}

impl Renderer {
    /// Create the renderer and load the fontset. Mirrors `drw_fontset_create`
    /// + `drw_scm_create` + `lrpad = drw->fonts->h`.
    pub fn new(
        fonts: &[String],
        scheme_strings: &[SchemeStrings; 9],
        required_chars: &HashSet<char>,
    ) -> Self {
        let specs: Vec<FontSpec> = fonts.iter().map(|f| parse_font_name(f)).collect();
        let mut font_system = match fontconfig::database_for(&specs, required_chars) {
            Some(db) => FontSystem::new_with_locale_and_db(detect_locale(), db),
            None => FontSystem::new(),
        };

        // Resolve each family against the system font database, loosely
        // (fontconfig-style), falling back to the raw name.
        let mut families = Vec::new();
        for spec in &specs {
            let resolved = resolve_family(&font_system, &spec.family);
            families.push(resolved);
        }

        let font_height = primary_font_height(&mut font_system, &families, specs[0].pixel_size);

        let mut checked_chars = required_chars.clone();
        checked_chars.extend(' '..='~');
        let mut renderer = Renderer {
            schemes: scheme_strings.iter().map(scheme_from_strings).collect(),
            swash_cache: SwashCache::new(),
            font_system,
            fonts: specs,
            families,
            font_height,
            horizontal_padding: font_height,
            scheme: SchemeColors::default(),
            frame_background: None,
            layout_cache: HashMap::new(),
            checked_chars,
        };
        renderer.scheme = renderer.schemes.first().copied().unwrap_or_default();
        renderer
    }

    /// Look up a configured scheme by its enum variant.
    pub fn color_scheme(&self, scheme: Scheme) -> SchemeColors {
        self.schemes[scheme as usize]
    }

    /// Make `scheme` the current drawing scheme.
    pub fn set_scheme(&mut self, scheme: Scheme) {
        self.scheme = self.color_scheme(scheme);
    }

    pub fn clear(&mut self, canvas: &mut Canvas, color: Color) {
        canvas.fill_rect(0, 0, canvas.width, canvas.height, color.channels());
        self.frame_background = Some(color);
    }

    /// drw_rect — filled rect in the current scheme; `invert` swaps fg/bg,
    /// `rounded` paints the bottom 4px strip with the detail color.
    pub fn rect(&mut self, canvas: &mut Canvas, x: i32, y: i32, w: i32, h: i32, filled: bool, invert: bool, rounded: bool) {
        let color = if invert { self.scheme.bg } else { self.scheme.fg };
        if filled && h < 40 {
            if rounded {
                self.fill_rect(canvas, x, y, w, h - 4, color);
                self.fill_rect(canvas, x, y + h - 4, w, 4, self.scheme.detail);
            } else {
                self.fill_rect(canvas, x, y, w, h, color);
            }
        } else {
            self.fill_rect(canvas, x, y, w, h, color);
        }
    }

    fn fill_rect(&self, canvas: &mut Canvas, x: i32, y: i32, w: i32, h: i32, color: Color) {
        canvas.fill_rect(x, y, w, h, color.channels());
    }

    /// `drw_fontset_getwidth` — width of `text` (without lrpad).
    pub fn text_width(&mut self, text: &str) -> i32 {
        if text.is_empty() {
            return 0;
        }
        if let Some(layout) = self.layout_cache.get(text) {
            return layout.width;
        }
        let buffer = self.make_buffer(text, None);
        let width = Self::buffer_width(&buffer);
        if self.layout_cache.len() >= 1024 {
            self.layout_cache.clear();
        }
        self.layout_cache.insert(text.to_owned(), TextLayout { width, buffer });
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
        let new_chars: HashSet<char> = text
            .chars()
            .filter(|ch| !self.checked_chars.contains(ch))
            .collect();
        if !new_chars.is_empty() {
            fontconfig::add_fallbacks(self.font_system.db_mut(), &new_chars);
            self.checked_chars.extend(new_chars);
        }
        let pixel_size = self.fonts[0].pixel_size;
        let metrics = Metrics::new(pixel_size, self.font_height as f32);
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
        buffer.set_rich_text(spans, &default_attrs, Shaping::Advanced, None);
    }

    fn buffer_width(buffer: &Buffer) -> i32 {
        buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max)
            .ceil() as i32
    }

    /// drw_text — draw `text` at (x, y, w, h) with `left_padding` padding.
    /// `invert` swaps fg/bg, `rounded` paints a 4px detail strip at the
    /// bottom and shifts the text up by 2px. Text that does not fit is
    /// truncated with an ellipsis ("..."). Returns the x position after the
    /// drawn text.
    pub fn text(
        &mut self,
        canvas: &mut Canvas,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        left_padding: i32,
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
        let fill = if invert { self.scheme.fg } else { self.scheme.bg };
        if rounded {
            self.fill_rect(canvas, x, y, w, h - 4, fill);
            self.fill_rect(canvas, x, y + h - 4, w, 4, self.scheme.detail);
        } else if self.frame_background != Some(fill) {
            self.fill_rect(canvas, x, y, w, h, fill);
        }
        if w < left_padding {
            return x + w;
        }

        let color = if invert { self.scheme.bg } else { self.scheme.fg };
        let cosmic_color = CosmicColor::rgba(color.r(), color.g(), color.b(), color.a());

        let available = w - left_padding;
        let mut display_text = text;
        let ellipsis_width = self.text_width("...");
        let full_width = self.text_width(text);
        let mut drawn_width = full_width;
        if full_width > available {
            // find the longest prefix after which an ellipsis still fits
            let max_text_width = (available - ellipsis_width).max(0);
            let chars: Vec<(usize, char)> = text.char_indices().collect();
            // binary search over char count
            let mut lo = 0usize;
            let mut hi = chars.len();
            while lo < hi {
                let mid = (lo + hi + 1) / 2;
                let end = if mid < chars.len() { chars[mid].0 } else { text.len() };
                if self.text_width(&text[..end]) <= max_text_width {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            let end = if lo < chars.len() { chars[lo].0 } else { text.len() };
            display_text = &text[..end];
            drawn_width = self.text_width(display_text) + ellipsis_width;
            // draw ellipsis right after the truncated text
            let ellipsis_x = x + left_padding + self.text_width(display_text);
            self.draw_run(canvas, ellipsis_x, y, h, "...", cosmic_color, rounded);
        }
        if !display_text.is_empty() {
            self.draw_run(canvas, x + left_padding, y, h, display_text, cosmic_color, rounded);
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
        let mut layout = self.layout_cache.remove(text).unwrap_or_else(|| {
            let buffer = self.make_buffer(text, None);
            TextLayout {
                width: Self::buffer_width(&buffer),
                buffer,
            }
        });

        let width = canvas.width;
        let height = canvas.height;
        // vertical centering like drw_text:
        // ty = y + (h - usedfont->h)/2 + ascent, here the buffer baseline sits
        // at ascent within font_height rows.
        let y_off = y + (h - self.font_height) / 2 - if rounded { 2 } else { 0 };

        layout.buffer.draw(
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
        if self.layout_cache.len() >= 1024 {
            self.layout_cache.clear();
        }
        self.layout_cache.insert(text.to_owned(), layout);
    }
}

fn detect_locale() -> String {
    std::env::var("LC_ALL")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var("LC_CTYPE").ok().filter(|value| !value.is_empty()))
        .or_else(|| std::env::var("LANG").ok().filter(|value| !value.is_empty()))
        .map(|value| {
            value
                .split('.')
                .next()
                .unwrap_or("en_US")
                .replace('_', "-")
        })
        .unwrap_or_else(|| "en-US".to_string())
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
