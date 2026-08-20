//! High-level drawing context bundling [`Renderer`] and [`Canvas`].
//!
//! [`Painter`] resolves the active scheme and canvas into intention-revealing
//! drawing calls, so callers never pass a canvas or scheme color by hand.

use crate::enums::{ColorRole, Scheme};
use crate::geom::Rect;

use super::canvas::Canvas;
use super::color::{Color, SchemeColors};
use super::renderer::Renderer;

/// Height of the detail / accent strip at the bottom of an item or slider bar (pixels).
pub const ACCENT_STRIP_HEIGHT: i32 = 4;

/// How text is styled inside a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextStyle {
    /// Fill the cell with the scheme background, draw the text in the foreground.
    Normal,
    /// Like [`TextStyle::Normal`], plus a bottom detail strip; the text is
    /// centered in the region above the strip.
    Accented,
    /// Fill the cell with the scheme foreground, draw the text in the background.
    Inverted,
}

/// A drawing context that bundles mutable references to [`Renderer`] and [`Canvas`].
pub struct Painter<'a> {
    pub renderer: &'a mut Renderer,
    pub canvas: &'a mut Canvas,
}

impl<'a> Painter<'a> {
    pub fn new(renderer: &'a mut Renderer, canvas: &'a mut Canvas) -> Self {
        Painter { renderer, canvas }
    }

    /// Set the active color scheme on the renderer.
    pub fn set_scheme(&mut self, scheme: Scheme) {
        self.renderer.set_scheme(scheme);
    }

    /// Get the currently active color scheme.
    pub fn scheme(&self) -> SchemeColors {
        self.renderer.scheme
    }

    /// Look up a configured scheme by its enum variant.
    pub fn color_scheme(&self, scheme: Scheme) -> SchemeColors {
        self.renderer.color_scheme(scheme)
    }

    /// Clear the entire canvas with a specific color.
    pub fn clear(&mut self, color: Color) {
        self.renderer.clear(self.canvas, color);
    }

    /// Clear the entire canvas with the background color of the currently active scheme.
    pub fn clear_with_scheme_bg(&mut self) {
        let bg = self.renderer.scheme.bg;
        self.renderer.clear(self.canvas, bg);
    }

    /// Fill a rectangle with a specific [`ColorRole`] from the active scheme.
    pub fn fill_rect(&mut self, rect: Rect, role: ColorRole) {
        let color = self.renderer.scheme.role(role);
        self.renderer.fill_rect(self.canvas, rect, color);
    }

    /// Fill a rectangle with an explicit [`Color`].
    pub fn fill_solid_rect(&mut self, rect: Rect, color: Color) {
        self.renderer.fill_rect(self.canvas, rect, color);
    }

    /// Fill a rectangle with the active scheme's background color plus a
    /// bottom [`ACCENT_STRIP_HEIGHT`] detail strip in the active scheme's
    /// detail color (the strip is clamped to the rectangle's height).
    pub fn fill_accented_rect(&mut self, rect: Rect) {
        self.fill_rect(rect, ColorRole::Background);
        let strip_h = ACCENT_STRIP_HEIGHT.min(rect.h);
        if strip_h <= 0 {
            return;
        }
        let strip = Rect::new(rect.x, rect.bottom() - strip_h, rect.w, strip_h);
        let detail = self.renderer.scheme.detail;
        self.renderer.fill_rect(self.canvas, strip, detail);
    }

    /// Draw text in `cell` with [`TextStyle::Normal`], starting `left_padding`
    /// in from the cell's left edge. The cell background is filled.
    pub fn draw_text(&mut self, cell: Rect, left_padding: i32, text: &str) {
        self.draw_text_styled(cell, left_padding, text, TextStyle::Normal);
    }

    /// Draw text in `cell` with the given [`TextStyle`]. Text wider than the
    /// cell is truncated with an ellipsis.
    pub fn draw_text_styled(
        &mut self,
        cell: Rect,
        left_padding: i32,
        text: &str,
        style: TextStyle,
    ) {
        if cell.w <= 0 || cell.h <= 0 {
            return;
        }

        // Resolve the background fill and the band the text is centered in.
        let (text_color, text_band) = match style {
            TextStyle::Normal => {
                self.renderer
                    .fill_rect(self.canvas, cell, self.renderer.scheme.bg);
                (self.renderer.scheme.fg, cell)
            }
            TextStyle::Inverted => {
                self.renderer
                    .fill_rect(self.canvas, cell, self.renderer.scheme.fg);
                (self.renderer.scheme.bg, cell)
            }
            TextStyle::Accented => {
                self.fill_accented_rect(cell);
                let body_h = (cell.h - ACCENT_STRIP_HEIGHT.min(cell.h)).max(0);
                let body = Rect::new(cell.x, cell.y, cell.w, body_h);
                (self.renderer.scheme.fg, body)
            }
        };

        if text_band.h <= 0 || cell.w < left_padding {
            return;
        }
        let available = cell.w - left_padding;
        let full = self.renderer.text_width(text);
        let shown = if full <= available {
            text
        } else {
            let ellipsis_width = self.renderer.text_width("...");
            let (prefix, prefix_width) = self
                .renderer
                .fit_text(text, (available - ellipsis_width).max(0));
            self.renderer.draw_shaped_text(
                self.canvas,
                text_band,
                left_padding + prefix_width,
                "...",
                text_color,
            );
            prefix
        };

        if !shown.is_empty() {
            self.renderer
                .draw_shaped_text(self.canvas, text_band, left_padding, shown, text_color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;
    use crate::render::renderer::make_test_renderer;
    use crate::render::Color;

    #[test]
    fn fill_accented_rect_draws_detail_strip() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(20, 20));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Selected);
        painter.fill_accented_rect(Rect::new(0, 0, 20, 20));

        let sel = painter.scheme();
        let bg = sel.bg;
        let detail = sel.detail;
        let bgra = |c: Color| [c.b(), c.g(), c.r(), c.a()];

        // Above the detail strip (y = 10): background color
        let pixel_top: [u8; 4] = painter.canvas.data[(10 * 20 + 5) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(pixel_top, bgra(bg));

        // In the bottom 4px detail strip (y = 18): detail color
        let pixel_bottom: [u8; 4] = painter.canvas.data[(18 * 20 + 5) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(pixel_bottom, bgra(detail));
    }

    #[test]
    fn fill_accented_rect_draws_strip_on_large_rectangles() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(50, 50));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Selected);
        painter.fill_accented_rect(Rect::new(0, 0, 50, 50));

        let sel = painter.scheme();
        let detail = sel.detail;
        let bgra = |c: Color| [c.b(), c.g(), c.r(), c.a()];

        // The bottom 4px are the detail strip regardless of the rect height.
        let pixel_bottom: [u8; 4] = painter.canvas.data[(48 * 50 + 25) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(pixel_bottom, bgra(detail));
    }

    #[test]
    fn draw_text_clips_glyphs_to_the_cell() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(40, 20));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Normal);
        // Cell only 8px wide with 2px padding: even the ellipsis cannot fit,
        // so whatever is drawn must be clipped at the cell's right edge.
        painter.draw_text(Rect::new(0, 0, 8, 20), 2, "abcdefghijklmno");

        // Pixels right of the cell are untouched (canvas still zeroed).
        let right_of_cell: [u8; 4] = painter.canvas.data[(10 * 40 + 12) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(right_of_cell, [0, 0, 0, 0]);
        // Something was drawn inside the cell: some pixel between the padding
        // and the cell edge differs from the pure background fill (the
        // ellipsis dots sit near the baseline, so scan all rows).
        let norm = painter.scheme();
        let bgra = |c: Color| [c.b(), c.g(), c.r(), c.a()];
        let bg = bgra(norm.bg);
        let drew = (2..8).any(|x| {
            (0..20).any(|y| {
                let pixel: [u8; 4] = painter.canvas.data[(y * 40 + x) * 4..][..4]
                    .try_into()
                    .unwrap();
                pixel != bg
            })
        });
        assert!(drew, "expected clipped ellipsis glyphs inside the cell");
    }

    #[test]
    fn fill_rect_with_color_roles() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(10, 10));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Normal);

        let norm = painter.scheme();
        let bgra = |c: Color| [c.b(), c.g(), c.r(), c.a()];

        painter.fill_rect(Rect::new(0, 0, 5, 10), ColorRole::Foreground);
        painter.fill_rect(Rect::new(5, 0, 5, 10), ColorRole::Background);

        let fg_pixel: [u8; 4] = painter.canvas.data[(5 * 10 + 2) * 4..][..4]
            .try_into()
            .unwrap();
        let bg_pixel: [u8; 4] = painter.canvas.data[(5 * 10 + 7) * 4..][..4]
            .try_into()
            .unwrap();

        assert_eq!(fg_pixel, bgra(norm.fg));
        assert_eq!(bg_pixel, bgra(norm.bg));
    }

    #[test]
    fn clear_with_scheme_bg_clears_canvas() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(10, 10));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Selected);
        painter.clear_with_scheme_bg();

        let sel = painter.scheme();
        let bgra = |c: Color| [c.b(), c.g(), c.r(), c.a()];

        for pixel in painter.canvas.data.chunks_exact(4) {
            assert_eq!(pixel, bgra(sel.bg));
        }
    }

    #[test]
    fn draw_text_fills_the_cell_background() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(40, 20));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Normal);
        painter.draw_text(Rect::new(0, 0, 40, 20), 6, "x");

        let norm = painter.scheme();
        let bgra = |c: Color| [c.b(), c.g(), c.r(), c.a()];
        // Far from the glyph: the cell is pure background.
        let pixel: [u8; 4] = painter.canvas.data[(10 * 40 + 35) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(pixel, bgra(norm.bg));
    }

    #[test]
    fn draw_text_styled_accent_draws_the_detail_strip() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(40, 20));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Selected);
        painter.draw_text_styled(Rect::new(0, 0, 40, 20), 6, "x", TextStyle::Accented);

        let sel = painter.scheme();
        let bgra = |c: Color| [c.b(), c.g(), c.r(), c.a()];
        // Body above the strip, away from the glyph.
        let body: [u8; 4] = painter.canvas.data[(5 * 40 + 35) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(body, bgra(sel.bg));
        // The bottom 4px are the detail strip.
        let strip: [u8; 4] = painter.canvas.data[(18 * 40 + 35) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(strip, bgra(sel.detail));
    }

    #[test]
    fn fill_accented_rect_clamps_strip_to_small_height() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(20, 20));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Selected);
        // Fill a small rect at y=5..7 (height = 2, less than ACCENT_STRIP_HEIGHT 4)
        painter.fill_accented_rect(Rect::new(0, 5, 20, 2));

        let sel = painter.scheme();
        let detail = sel.detail;
        let bgra = |c: Color| [c.b(), c.g(), c.r(), c.a()];

        // Above the rect (y = 4) must NOT be painted (still 0)
        let pixel_above: [u8; 4] = painter.canvas.data[(4 * 20 + 5) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(pixel_above, [0, 0, 0, 0]);

        // Inside the rect (y = 5, 6) is the detail strip clamped to 2px
        let pixel_inside: [u8; 4] = painter.canvas.data[(5 * 20 + 5) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(pixel_inside, bgra(detail));

        // Below the rect (y = 7) must NOT be painted (still 0)
        let pixel_below: [u8; 4] = painter.canvas.data[(7 * 20 + 5) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(pixel_below, [0, 0, 0, 0]);
    }

    #[test]
    fn draw_text_styled_accent_clamps_on_small_cells() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(20, 20));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Selected);
        // Cell height = 2 at y = 10
        painter.draw_text_styled(Rect::new(0, 10, 20, 2), 0, "x", TextStyle::Accented);

        // Above the cell (y = 9) must NOT be painted
        let pixel_above: [u8; 4] = painter.canvas.data[(9 * 20 + 5) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(pixel_above, [0, 0, 0, 0]);
    }
}
