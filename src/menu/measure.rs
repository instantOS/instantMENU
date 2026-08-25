//! Text measurement seam.
//!
//! Paging and layout math need cell widths but must stay independent of the
//! renderer; they take a `&mut dyn Measure` instead. Tests substitute
//! fixed-width fakes.

use super::matcher::Item;
use crate::render::Renderer;

/// Width reserved for the colored icon block at the start of an icon item.
/// Drawing and every item-measurement path use this helper so their geometry
/// cannot drift apart.
pub(super) fn icon_gutter_width(font_height: i32) -> i32 {
    font_height * 3
}

/// Cell width measurement with dmenu's padding and clamping semantics.
pub(super) trait Measure {
    /// Cell width: the glyph width plus the horizontal padding.
    fn cell_width(&mut self, s: &str) -> i32;

    /// Width of an item as rendered: its visible label plus an icon gutter
    /// when the parsed entry is an icon item.
    fn item_cell_width(&mut self, item: &Item) -> i32 {
        self.cell_width(item.label())
            + if item.entry.icon.is_some() {
                self.icon_gutter_width()
            } else {
                0
            }
    }

    /// Rendered item width clamped to the available horizontal budget.
    fn item_cell_width_clamp(&mut self, item: &Item, n: i32) -> i32 {
        if n == 0 {
            return 0;
        }
        let width = self.item_cell_width(item);
        if n < 0 {
            width
        } else {
            width.min(n)
        }
    }

    /// Width of the icon gutter for this measurement context. Pure paging
    /// test measurers have no gutter unless they opt into one.
    fn icon_gutter_width(&self) -> i32 {
        0
    }
}

/// [`Measure`] over the shared renderer. In single-key mode every key is a
/// square cell of the bar height.
pub(super) struct TextMeasurer<'a> {
    renderer: &'a mut Renderer,
    single_key: bool,
    bar_height: i32,
}

impl<'a> TextMeasurer<'a> {
    /// Built from disjoint menu fields so callers can hold it alongside
    /// borrows of the item and selection state.
    pub(super) fn new(renderer: &'a mut Renderer, single_key: bool, bar_height: i32) -> Self {
        TextMeasurer {
            renderer,
            single_key,
            bar_height,
        }
    }
}

impl Measure for TextMeasurer<'_> {
    fn cell_width(&mut self, s: &str) -> i32 {
        if self.single_key {
            self.bar_height
        } else {
            self.renderer.text_width(s) + self.renderer.horizontal_padding
        }
    }

    fn icon_gutter_width(&self) -> i32 {
        icon_gutter_width(self.renderer.font_height)
    }
}
