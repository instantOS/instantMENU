//! The resolved window geometry: computed during setup and recomputed on
//! demand when streamed-in items change the derived grid shape. Owns the
//! effective `-l`/`-g` values (after item-count adjustment and monitor
//! clamping), so runtime code never re-reads them from the config — and the
//! [`Header`], the one place where the header-row geometry (command cells,
//! prompt, input field, paging arrows) is computed, shared by drawing and
//! mouse hit-testing.

use super::measure::Measure;
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
    /// Width of a left/right command cell (the arrow glyph), measured once
    /// at layout time so draw and hit-testing never re-measure it apart.
    pub command_width: i32,
    /// effective -l: 0 = horizontal list, >0 = vertical list/grid rows.
    pub lines: i32,
    /// effective -g: grid columns.
    pub columns: i32,
}

impl Layout {
    /// y band of the i-th visible row: row 0 is the input/header row, items
    /// start at row 1. Single source for grid cells and vertical hit-tests.
    fn row_top(&self, i: usize) -> i32 {
        ((i as i32 % self.lines) + 1) * self.bar_height
    }

    /// Grid cell rect for the i-th visible item (shared by draw + hover).
    pub fn grid_cell_rect(&self, i: usize, x: i32) -> Rect {
        let column_width = (self.menu_width - x) / self.columns;
        Rect::new(
            x + (i as i32 / self.lines) * column_width,
            self.row_top(i),
            column_width,
            self.bar_height,
        )
    }

    /// Inclusive y-band of the i-th visible item row, for hit-tests that
    /// only care about rows (the vertical list selects by y alone).
    pub fn row_band(&self, i: usize) -> (i32, i32) {
        let top = self.row_top(i);
        (top, top + self.bar_height)
    }
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

/// The resolved header-row geometry: command cells, prompt, input field and
/// paging arrows. Computed from the [`Layout`] plus the flags that shape it;
/// drawing and mouse hit-testing both consume these rects, so what you see
/// is exactly what is clickable.
#[derive(Debug, Clone, Copy)]
pub(super) struct Header {
    /// Width of one command cell (the arrow glyph).
    pub command_width: i32,
    /// Left/right command-cell rects; `None` when unconfigured.
    pub left_command: Option<Rect>,
    pub right_command: Option<Rect>,
    /// Prompt rect; `None` without a prompt. In short menus the prompt
    /// block spans all rows (`lines < 8`).
    pub prompt: Option<Rect>,
    /// Input field rect.
    pub input: Rect,
    /// Origin of the content after the prompt: where the item area (grid
    /// cells / horizontal list) begins.
    pub content_x: i32,
    /// Paging arrows of the horizontal list. Always positioned; whether an
    /// arrow is active/drawn depends on the paging state.
    pub left_arrow: Rect,
    pub right_arrow: Rect,
}

impl Header {
    /// Resolve the header geometry. Measured widths come in through the
    /// [`Measure`] seam; `counter_width` is pre-measured by the caller (it
    /// only exists while the match counter is shown).
    #[allow(clippy::too_many_arguments)] // every input shapes exactly one rect
    pub(super) fn compute(
        layout: &Layout,
        has_left_command: bool,
        has_right_command: bool,
        has_prompt: bool,
        has_matches: bool,
        show_match_counter: bool,
        counter_width: i32,
        measure: &mut dyn Measure,
        single_key: bool,
    ) -> Self {
        let bar = layout.bar_height;
        let command_width = layout.command_width;

        let left_command = has_left_command.then(|| Rect::new(0, 0, command_width, bar));
        let right_command = has_right_command
            .then(|| Rect::new(layout.menu_width - command_width, 0, command_width, bar));

        /* the left command cell owns the first `command_width` pixels; the
         * prompt follows it */
        let prompt_x = if has_left_command { command_width } else { 0 };
        let prompt_height = if layout.lines < 8 {
            bar * (layout.lines + 1)
        } else {
            bar
        };
        let prompt = has_prompt.then(|| Rect::new(prompt_x, 0, layout.prompt_width, prompt_height));
        let content_x = prompt_x + if has_prompt { layout.prompt_width } else { 0 };

        /* the input field spans the rest of the header row in list modes
         * with nothing listed; in horizontal mode it keeps its fixed width.
         * Single-key mode has no editable field, so its width is 0 and the
         * horizontal items start right after the prompt. */
        let input_width = if single_key {
            0
        } else if layout.lines > 0 || !has_matches {
            layout.menu_width - content_x
        } else {
            layout.input_width
        };
        let input = Rect::new(content_x, 0, input_width, bar);

        let left_arrow = Rect::new(
            content_x + if single_key { 0 } else { layout.input_width },
            0,
            measure.cell_width("<"),
            bar,
        );
        let right_arrow_width = measure.cell_width(">");
        /* with the match counter shown, the ">" sits left of it — that is
         * also its click target */
        let right_arrow = Rect::new(
            layout.menu_width
                - right_arrow_width
                - if show_match_counter { counter_width } else { 0 },
            0,
            right_arrow_width,
            bar,
        );

        Header {
            command_width,
            left_command,
            right_command,
            prompt,
            input,
            content_x,
            left_arrow,
            right_arrow,
        }
    }
}
