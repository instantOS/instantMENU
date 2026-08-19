//! High-level drawing context bundling [`Renderer`] and [`Canvas`].
//!
//! [`Painter`] eliminates boolean blindness and repetitive canvas passing by
//! providing intention-revealing drawing methods for rectangles, text, and
//! UI elements.

use crate::enums::{ColorRole, Scheme};
use crate::geom::Rect;

use super::canvas::Canvas;
use super::color::{Color, SchemeColors};
use super::renderer::Renderer;

/// Height of the detail / accent strip at the bottom of an item or slider bar (pixels).
pub const ACCENT_STRIP_HEIGHT: i32 = 4;
/// Maximum height of a rectangle that receives a detail / accent strip.
pub const ACCENT_MAX_HEIGHT: i32 = 40;
/// Vertical shift (pixels) for text rendered in an accented cell to keep it visually centered.
pub const ACCENT_TEXT_Y_OFFSET: i32 = 2;

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

    /// Fill a rectangle with the active scheme's background color and, if the
    /// rectangle height is within [`ACCENT_MAX_HEIGHT`], a bottom [`ACCENT_STRIP_HEIGHT`]
    /// detail strip using the active scheme's detail color.
    pub fn fill_accented_rect(&mut self, rect: Rect) {
        self.fill_rect(rect, ColorRole::Background);
        if rect.h < ACCENT_MAX_HEIGHT {
            let strip = Rect::new(
                rect.x,
                rect.bottom() - ACCENT_STRIP_HEIGHT,
                rect.w,
                ACCENT_STRIP_HEIGHT,
            );
            let detail = self.renderer.scheme.detail;
            self.renderer.fill_rect(self.canvas, strip, detail);
        }
    }

    /// Text width in pixels (delegates to [`Renderer::text_width`]).
    pub fn text_width(&mut self, text: &str) -> i32 {
        self.renderer.text_width(text)
    }

    /// Font height of primary font.
    pub fn font_height(&self) -> i32 {
        self.renderer.font_height
    }

    /// Horizontal padding (`lrpad`).
    pub fn horizontal_padding(&self) -> i32 {
        self.renderer.horizontal_padding
    }

    /// Draw standard text in `cell` with `left_padding`.
    pub fn draw_text(&mut self, cell: Rect, left_padding: i32, text: &str) -> i32 {
        self.renderer
            .draw_text(self.canvas, cell, left_padding, text, false, false)
    }

    /// Draw a menu item cell with optional accent styling.
    ///
    /// If `is_accented` is true, paints an accented background (with detail strip)
    /// and shifts the text up by [`ACCENT_TEXT_Y_OFFSET`] to keep it visually centered.
    pub fn draw_item(
        &mut self,
        cell: Rect,
        left_padding: i32,
        text: &str,
        is_accented: bool,
    ) -> i32 {
        self.renderer
            .draw_text(self.canvas, cell, left_padding, text, false, is_accented)
    }

    /// Draw inverted text: background is painted with `fg` and text is
    /// painted with `bg`. (Not used by the prompt — the C original draws
    /// that non-inverted — but available for inverted cells.)
    pub fn draw_inverted_text(&mut self, cell: Rect, left_padding: i32, text: &str) -> i32 {
        self.renderer
            .draw_text(self.canvas, cell, left_padding, text, true, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Size;
    use crate::render::{Color, SchemeStrings};
    use std::collections::HashSet;

    fn make_test_renderer() -> Renderer {
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
    fn fill_accented_rect_skips_detail_strip_on_large_rectangles() {
        let mut renderer = make_test_renderer();
        let mut canvas = Canvas::new(Size::new(50, 50));
        let mut painter = Painter::new(&mut renderer, &mut canvas);
        painter.set_scheme(Scheme::Selected);
        // Height 50 >= ACCENT_MAX_HEIGHT (40)
        painter.fill_accented_rect(Rect::new(0, 0, 50, 50));

        let sel = painter.scheme();
        let bg = sel.bg;
        let bgra = |c: Color| [c.b(), c.g(), c.r(), c.a()];

        // Bottom rows should remain background color, no detail strip
        let pixel_bottom: [u8; 4] = painter.canvas.data[(48 * 50 + 25) * 4..][..4]
            .try_into()
            .unwrap();
        assert_eq!(pixel_bottom, bgra(bg));
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
}
