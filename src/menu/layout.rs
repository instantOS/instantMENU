//! The resolved window geometry: computed during setup and recomputed on
//! demand when streamed-in items change the derived grid shape. Owns the
//! effective `-l`/`-g` values (after item-count adjustment and monitor
//! clamping), so runtime code never re-reads them from the config.

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

/// The -l/-g pair before monitor clamping: as adjusted for the current item
/// count. Recomputed whenever items stream in; the window is resized to fit
/// when the shape changes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GridShape {
    pub lines: i32,
    pub columns: i32,
}

/// The -l/-g adjustment the C version made after reading stdin: lines shrink
/// to fit the item count, then columns shrink to the widest actually-needed
/// grid (only when the grid is multi-column).
pub(super) fn adjusted_grid(lines: i32, columns: i32, count: i32) -> GridShape {
    let i = count;
    let lines = lines.min(i / columns + (i % columns != 0) as i32);
    let columns = if columns != 1 && lines != 0 {
        (i / lines + (i % lines != 0) as i32).min(columns)
    } else {
        columns
    };
    GridShape { lines, columns }
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
