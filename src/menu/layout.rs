//! The resolved window geometry: computed once during setup, immutable
//! afterwards. Owns the effective `-l`/`-g` values (after stdin adjustment
//! and monitor clamping), so runtime code never re-reads them from the
//! config.

use crate::geom::Rect;

/// Window geometry and effective list shape.
#[derive(Debug, Clone, Default)]
pub(super) struct Layout {
    pub x: i32,
    pub y: i32,
    pub menu_width: i32,
    pub menu_height: i32,
    pub bar_height: i32,
    pub input_width: i32,
    pub prompt_width: i32,
    /// effective -l: 0 = horizontal list, >0 = vertical list/grid rows.
    pub lines: i32,
    /// effective -g: grid columns.
    pub columns: i32,
}

/// The -l/-g pair before monitor clamping: as adjusted by stdin's item
/// count, consumed by [`Menu::setup`](super::Menu::setup).
#[derive(Debug, Clone, Copy, Default)]
pub struct GridShape {
    pub lines: i32,
    pub columns: i32,
}

impl Layout {
    /// Grid cell rect for the i-th visible item (shared by draw + hover).
    pub fn grid_cell_rect(&self, i: usize, x: i32, y: i32) -> Rect {
        let column_width = (self.menu_width - x) / self.columns;
        Rect::new(
            x + (i as i32 / self.lines) * column_width,
            y + ((i as i32 % self.lines) + 1) * self.bar_height,
            column_width,
            self.bar_height,
        )
    }
}
