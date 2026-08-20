//! The shared drawing context: fonts, color schemes and the shaped-text cache.
//! Drawing primitives take the target [`Canvas`] explicitly; the backends blit
//! that canvas to the window.

use std::collections::{HashMap, HashSet};

use cosmic_text::{
    Attrs, Buffer, Color as CosmicColor, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};

use crate::enums::Scheme;
use crate::geom::Rect;

use super::canvas::Canvas;
use super::color::{scheme_from_strings, Color, SchemeColors, SchemeStrings};
use super::font::{parse_font_name, primary_font_height, resolve_family, FontSpec};
use super::fontconfig;
use super::painter::Painter;

/// The shared drawing context: fonts, color schemes and the shaped-text cache.
pub struct Renderer {
    pub font_system: FontSystem,
    swash_cache: SwashCache,
    /// Parsed font specs, primary first.
    pub fonts: Vec<FontSpec>,
    /// Resolved cosmic-text family names in fontset order.
    families: Vec<String>,
    /// Height of the primary font (ascent + descent).
    pub font_height: i32,
    /// Sum of the left and right text padding inside a cell.
    pub horizontal_padding: i32,
    /// Color schemes.
    pub schemes: Vec<SchemeColors>,
    /// Currently active color scheme.
    pub scheme: SchemeColors,

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
    /// Create the renderer: resolve the font families, load the schemes and
    /// seed the fallback set for the required characters.
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
        canvas.fill_rect(Rect::with_size(canvas.size()), color.channels());
    }

    /// Create a [`Painter`] drawing context bundling this renderer and the given canvas.
    pub fn painter<'a>(&'a mut self, canvas: &'a mut Canvas) -> Painter<'a> {
        Painter::new(self, canvas)
    }

    /// Fill a rectangle with a color directly into the canvas.
    pub fn fill_rect(&self, canvas: &mut Canvas, rect: Rect, color: Color) {
        canvas.fill_rect(rect, color.channels());
    }

    /// Half of the horizontal padding: the left (and right) text inset inside
    /// a cell.
    pub fn cell_inset(&self) -> i32 {
        self.horizontal_padding / 2
    }

    /// Raw glyph width of `text`, with no padding applied.
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
        self.layout_cache
            .insert(text.to_owned(), TextLayout { width, buffer });
        width
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
                spans.push((
                    &text[start..index],
                    Attrs::new().family(Family::Name(family)),
                ));
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

    /// Longest UTF-8-boundary prefix of `text` whose glyph width is at most
    /// `max_width`, together with that prefix's width. Returns the whole text
    /// when it already fits, and an empty prefix when even the first glyph
    /// does not.
    pub(super) fn fit_text<'a>(&mut self, text: &'a str, max_width: i32) -> (&'a str, i32) {
        let full = self.text_width(text);
        if full <= max_width {
            return (text, full);
        }
        if max_width <= 0 {
            return ("", 0);
        }

        // Byte offset of every char boundary, plus the end. Glyph width is
        // monotonic in prefix length, so `partition_point` yields the longest
        // prefix that fits — always cut on a UTF-8 boundary.
        let mut boundaries: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        boundaries.push(text.len());
        let fit = boundaries.partition_point(|&end| self.text_width(&text[..end]) <= max_width);
        let prefix = &text[..boundaries[fit - 1]];
        (prefix, self.text_width(prefix))
    }

    /// Draw the glyphs of `text` (shaped on demand) at `band.x + x_offset`,
    /// vertically centered within `band`. Foreground only — the cell
    /// background is left untouched, so callers must fill it first.
    pub(super) fn draw_shaped_text(
        &mut self,
        canvas: &mut Canvas,
        band: Rect,
        x_offset: i32,
        text: &str,
        color: Color,
    ) {
        if band.w <= 0 || band.h <= 0 {
            return;
        }

        // Remove-then-reinsert: the miss path (`make_buffer`) needs `&mut
        // self`, so we can't hold a cache entry across it. Take the layout
        // out, draw from it, then put it back.
        let mut layout = self.layout_cache.remove(text).unwrap_or_else(|| {
            let buffer = self.make_buffer(text, None);
            TextLayout {
                width: Self::buffer_width(&buffer),
                buffer,
            }
        });

        let width = canvas.width;
        let height = canvas.height;
        let min_x = band.x.max(0);
        let max_x = band.right().min(width);
        // The shaped buffer occupies `font_height` rows; center that box in
        // the band. Glyph `gy` is relative to the buffer's top edge.
        let x = band.x + x_offset;
        let y = band.y + (band.h - self.font_height) / 2;
        let cosmic_color = CosmicColor::rgba(color.r(), color.g(), color.b(), color.a());

        layout.buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            cosmic_color,
            |gx, gy, _gw, _gh, c| {
                let px_color = [c.r(), c.g(), c.b(), c.a()];
                let cx = x + gx;
                let cy = y + gy;
                if cx < min_x || cx >= max_x || cy < 0 || cy >= height {
                    return;
                }
                canvas.blend_pixel(crate::geom::Point::new(cx, cy), px_color);
            },
        );
        if self.layout_cache.len() >= 1024 {
            self.layout_cache.clear();
        }
        self.layout_cache.insert(text.to_owned(), layout);
    }
}

/// A renderer over a fixed nine-scheme palette and `monospace:size=12`, for
/// the pixel- and measurement-level tests of this module and `painter`.
#[cfg(test)]
pub(super) fn make_test_renderer() -> Renderer {
    let scheme_strings = [
        SchemeStrings {
            fg: "#ffffff".to_string(),
            bg: "#111111".to_string(),
            detail: "#333333".to_string(),
        },
        SchemeStrings {
            fg: "#aaaaaa".to_string(),
            bg: "#222222".to_string(),
            detail: "#444444".to_string(),
        },
        SchemeStrings {
            fg: "#bbbbbb".to_string(),
            bg: "#333333".to_string(),
            detail: "#555555".to_string(),
        },
        SchemeStrings {
            fg: "#cccccc".to_string(),
            bg: "#444444".to_string(),
            detail: "#666666".to_string(),
        },
        SchemeStrings {
            fg: "#000000".to_string(),
            bg: "#0055ff".to_string(),
            detail: "#00aaff".to_string(),
        },
        SchemeStrings {
            fg: "#dddddd".to_string(),
            bg: "#555555".to_string(),
            detail: "#777777".to_string(),
        },
        SchemeStrings {
            fg: "#00ff00".to_string(),
            bg: "#003300".to_string(),
            detail: "#006600".to_string(),
        },
        SchemeStrings {
            fg: "#ffff00".to_string(),
            bg: "#333300".to_string(),
            detail: "#666600".to_string(),
        },
        SchemeStrings {
            fg: "#ff0000".to_string(),
            bg: "#330000".to_string(),
            detail: "#660000".to_string(),
        },
    ];
    Renderer::new(
        &["monospace:size=12".to_string()],
        &scheme_strings,
        &HashSet::new(),
    )
}

fn detect_locale() -> String {
    std::env::var("LC_ALL")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("LC_CTYPE")
                .ok()
                .filter(|value| !value.is_empty())
        })
        .or_else(|| std::env::var("LANG").ok().filter(|value| !value.is_empty()))
        .map(|value| value.split('.').next().unwrap_or("en_US").replace('_', "-"))
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
        Some(c)
            if (c as u32) >= 0x1F000
                || matches!(c as u32, 0x2600..=0x27BF | 0x2190..=0x21FF | 0x2B00..=0x2BFF) =>
        {
            CharClass::Emoji
        }
        _ => CharClass::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_text_returns_the_whole_text_when_it_fits() {
        let mut r = make_test_renderer();
        let text = "abcdefghij";
        let full = r.text_width(text);
        assert_eq!(r.fit_text(text, full), (text, full));
        assert_eq!(r.fit_text(text, full + 100), (text, full));
    }

    #[test]
    fn fit_text_cuts_on_a_char_boundary() {
        let mut r = make_test_renderer();
        // Widths are taken from the renderer itself so the test holds for any
        // font: the prefix of five ASCII chars must be the longest that fits
        // in exactly its own width.
        let text = "aaaaaaaaaaaaaa";
        let w5 = r.text_width("aaaaa");
        let (prefix, width) = r.fit_text(text, w5);
        assert_eq!(prefix, "aaaaa");
        assert_eq!(width, w5);
    }

    #[test]
    fn fit_text_cuts_multi_byte_text_on_a_char_boundary() {
        let mut r = make_test_renderer();
        let text = "αααααααα";
        let w3 = r.text_width("ααα");
        let (prefix, width) = r.fit_text(text, w3);
        assert_eq!(prefix, "ααα");
        assert_eq!(width, w3);
    }

    #[test]
    fn fit_text_returns_empty_prefix_when_nothing_fits() {
        let mut r = make_test_renderer();
        // Any single glyph at size 12 is wider than 1px.
        assert_eq!(r.fit_text("abc", 1), ("", 0));
        assert_eq!(r.fit_text("abc", 0), ("", 0));
        assert_eq!(r.fit_text("abc", -5), ("", 0));
    }
}
