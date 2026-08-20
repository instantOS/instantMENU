//! Text measurement seam.
//!
//! Paging and layout math need cell widths but must stay independent of the
//! renderer; they take a `&mut dyn Measure` instead. Tests substitute
//! fixed-width fakes.

use crate::render::Renderer;

/// Cell width measurement with dmenu's padding and clamping semantics.
pub(super) trait Measure {
    /// Cell width: the glyph width plus the horizontal padding.
    fn cell_width(&mut self, s: &str) -> i32;
    /// Cell width of `s` clamped to `n`. 0 yields 0, negatives are unclamped.
    fn cell_width_clamp(&mut self, s: &str, n: i32) -> i32;
}

/// [`Measure`] over the shared renderer. In commented mode every text is a
/// square cell of the bar height.
pub(super) struct TextMeasurer<'a> {
    renderer: &'a mut Renderer,
    commented: bool,
    bar_height: i32,
}

impl<'a> TextMeasurer<'a> {
    /// Built from disjoint menu fields so callers can hold it alongside
    /// borrows of the item and selection state.
    pub(super) fn new(renderer: &'a mut Renderer, commented: bool, bar_height: i32) -> Self {
        TextMeasurer {
            renderer,
            commented,
            bar_height,
        }
    }
}

impl Measure for TextMeasurer<'_> {
    fn cell_width(&mut self, s: &str) -> i32 {
        if self.commented {
            self.bar_height
        } else {
            self.renderer.text_width(s) + self.renderer.horizontal_padding
        }
    }

    fn cell_width_clamp(&mut self, s: &str, n: i32) -> i32 {
        if self.commented {
            return self.bar_height;
        }
        if n == 0 {
            return 0;
        }
        if n < 0 {
            return self.cell_width(s);
        }
        (self.renderer.text_width(s) + self.renderer.horizontal_padding).min(n)
    }
}
