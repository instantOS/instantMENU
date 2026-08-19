//! Mouse handling: hover selection, button presses and paste.

use super::transition::Transition;
use super::Menu;
use crate::backend::MouseButton;
use crate::enums::{EditOp, ExitStatus, ItemCategory, Side};
use crate::geom::Point;

impl Menu {
    /// x offset after the prompt (0 when there is no prompt).
    fn prompt_offset(&self) -> i32 {
        if let Some(prompt) = self.prompt() {
            if !prompt.is_empty() {
                return self.layout.prompt_width;
            }
        }
        0
    }

    /// Width of the input field given the x offset after the prompt.
    fn input_field_width(&self, x: i32) -> i32 {
        if self.layout.lines > 0 || self.matcher.matches.is_empty() {
            self.layout.menu_width - x
        } else {
            self.layout.input_width
        }
    }

    /// set_selection — hover selection on motion.
    pub(super) fn set_selection(&mut self, pos: Point) -> Transition {
        let x = self.prompt_offset();
        if self.layout.lines > 0 {
            if self.layout.columns > 0 {
                self.hover_columns(pos, x)
            } else {
                self.hover_vertical(pos)
            }
        } else if !self.matcher.matches.is_empty() {
            self.hover_horizontal(pos, x)
        } else {
            Transition::Nop
        }
    }

    /// Column/grid hover selection: hit-test the same cell layout as draw_grid.
    fn hover_columns(&mut self, pos: Point, x: i32) -> Transition {
        let start = self.selection.current.unwrap_or(0);
        let end = self.paging.next.unwrap_or(self.matcher.matches.len());
        for (i, item) in (start..end).enumerate() {
            let cell = self.layout.grid_cell_rect(i, x, 0);
            if cell.contains(pos) {
                if self.selection.selected == Some(item) {
                    return Transition::Nop;
                }
                self.selection.selected = Some(item);
                return Transition::Redraw;
            }
        }
        Transition::Nop
    }

    /// Vertical list hover selection.
    fn hover_vertical(&mut self, pos: Point) -> Transition {
        let mut y = 0;
        let row_height = self.layout.bar_height;
        let start = self.selection.current.unwrap_or(0);
        let end = self.paging.next.unwrap_or(self.matcher.matches.len());
        for item in start..end {
            y += row_height;
            if pos.y >= y && pos.y <= y + row_height {
                if self.selection.selected == Some(item) {
                    return Transition::Nop;
                }
                self.selection.selected = Some(item);
                return Transition::Redraw;
            }
        }
        Transition::Nop
    }

    /// Horizontal list hover selection.
    fn hover_horizontal(&mut self, pos: Point, x: i32) -> Transition {
        for (item, rect) in self.horizontal_item_rects(x) {
            if rect.contains(pos) {
                if self.selection.selected == Some(item) {
                    return Transition::Nop;
                }
                self.selection.selected = Some(item);
                return Transition::Redraw;
            }
        }
        Transition::Nop
    }

    /// button_press
    pub(super) fn button_press(
        &mut self,
        button: MouseButton,
        state: u32,
        pos: Point,
    ) -> Transition {
        /* right-click: exit */
        if button == MouseButton::Right {
            return Transition::Exit(ExitStatus::Failure);
        }

        let x = self.prompt_offset();
        let w = self.input_field_width(x);

        match button {
            MouseButton::Left => self.left_click(state, pos, x, w),
            /* middle-mouse click: paste selection */
            MouseButton::Middle => {
                self.request_paste(state);
                Transition::Redraw
            }
            /* scroll up */
            MouseButton::ScrollUp
                if self.paging.prev != 0
                    || self.selection.current.map(|c| c > 0).unwrap_or(false) =>
            {
                self.selection.selected = self.selection.current;
                self.selection.current = Some(self.paging.prev);
                self.recalc_paging();
                Transition::Redraw
            }
            /* scroll down */
            MouseButton::ScrollDown if self.paging.next.is_some() => {
                let next = self.paging.next.unwrap();
                self.selection.selected = Some(next);
                self.selection.current = Some(next);
                self.recalc_paging();
                Transition::Redraw
            }
            _ => Transition::Nop,
        }
    }

    /// left-click: clear the input, or click an item/arrow.
    fn left_click(&mut self, state: u32, pos: Point, x: i32, w: i32) -> Transition {
        let y = 0;
        let row_height = self.layout.bar_height;

        /* left-click on input: clear input,
         * NOTE: if there is no left-arrow the space for < is reserved so
         *       add that to the input width */
        let command_cell_width = self.command_cell_width();
        let input_hit = (self.layout.lines <= 0
            && pos.x >= 0
            && pos.x
                <= x + w
                    + if self.paging.prev == 0
                        || self.selection.current.map(|c| c == 0).unwrap_or(true)
                    {
                        self.text_width("<")
                    } else {
                        0
                    })
            || (self.layout.lines > 0 && pos.y >= y && pos.y <= y + row_height);
        if input_hit {
            if self.cfg.left_command.is_some() && pos.x < command_cell_width {
                return self.trigger_command(Side::Left);
            } else if pos.x > self.layout.menu_width - command_cell_width {
                return self.trigger_command(Side::Right);
            }
            let t = self.insert(EditOp::Delete(self.editor.cursor));
            return t.at_least_redraw();
        } else if self.layout.lines > 0 {
            return self.vertical_click(state);
        } else if !self.matcher.matches.is_empty() {
            return self.horizontal_click(state, pos, x);
        }
        Transition::Nop
    }

    /// left-click on a vertical list item.
    fn vertical_click(&mut self, state: u32) -> Transition {
        if self.selected_is_comment() {
            return Transition::Nop;
        }
        let out = self
            .selected_text()
            .unwrap_or_else(|| self.editor.text.clone());
        self.confirm(&out, state).at_least_redraw()
    }

    /// left-click on the horizontal list: arrows and items.
    fn horizontal_click(&mut self, state: u32, pos: Point, x: i32) -> Transition {
        let arrow_width = self.text_width("<");
        let left_arrow_x = x + self.layout.input_width;
        if (self.paging.prev != 0 || self.selection.current.map(|c| c > 0).unwrap_or(false))
            && pos.x >= left_arrow_x
            && pos.x <= left_arrow_x + arrow_width
        {
            self.selection.selected = self.selection.current;
            self.selection.current = Some(self.paging.prev);
            self.recalc_paging();
            return Transition::Redraw;
        }
        'items: for (item, rect) in self.horizontal_item_rects(x) {
            if rect.contains(pos) {
                let item_text = self.matcher.text_of_match(item).to_string();
                if ItemCategory::from_prefix(&item_text, false).0.is_comment()
                    && self.selected_text_ref().is_some_and(|t| !t.is_empty())
                {
                    break 'items;
                }
                self.selection.selected = Some(item);
                return self.confirm(&item_text, state).at_least_redraw();
            }
        }
        /* left-click on right arrow */
        let right_arrow_width = self.text_width(">");
        let right_arrow_x = self.layout.menu_width - right_arrow_width;
        if self.paging.next.is_some()
            && pos.x >= right_arrow_x
            && pos.x <= right_arrow_x + right_arrow_width
        {
            let next = self.paging.next.unwrap();
            self.selection.selected = Some(next);
            self.selection.current = Some(next);
            self.recalc_paging();
            return Transition::Redraw;
        }
        Transition::Nop
    }

    /// paste — insert selection text.
    pub(super) fn paste(&mut self, text: &str) -> Transition {
        /* we have been given the current selection, now insert it into input */
        let line = text.split('\n').next().unwrap_or("");
        self.insert(EditOp::Insert(line)).at_least_redraw()
    }
}
