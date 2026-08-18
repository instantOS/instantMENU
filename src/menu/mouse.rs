//! Mouse handling: hover selection, button presses and paste.

use super::Menu;
use crate::backend::{MouseButton, CONTROL_MASK, SHIFT_MASK};

impl Menu {
    /// x offset after the prompt (0 when there is no prompt).
    fn prompt_offset(&self) -> i32 {
        if let Some(prompt) = self.prompt() {
            if !prompt.is_empty() {
                return self.prompt_width;
            }
        }
        0
    }

    /// Width of the input field given the x offset after the prompt.
    fn input_field_width(&self, x: i32) -> i32 {
        if self.cfg.lines > 0 || self.matches.is_empty() {
            self.menu_width - x
        } else {
            self.input_width
        }
    }

    /// set_selection — hover selection on motion.
    pub(super) fn set_selection(&mut self, ev_x: i32, ev_y: i32) {
        let x = self.prompt_offset();
        if self.cfg.lines > 0 {
            if self.cfg.columns > 0 {
                self.hover_columns(ev_x, ev_y, x);
            } else {
                self.hover_vertical(ev_y);
            }
        } else if !self.matches.is_empty() {
            self.hover_horizontal(ev_x, x);
        }
    }

    /// Column/grid hover selection: hit-test the same cell layout as draw_grid.
    fn hover_columns(&mut self, ev_x: i32, ev_y: i32, x: i32) {
        let row_height = self.bar_height;
        let column_width = self.menu_width / self.cfg.columns;
        let start = self.current.unwrap_or(0);
        let end = self.next.unwrap_or(self.matches.len());
        for (i, pos) in (start..end).enumerate() {
            let check_x = x + (i as i32 / self.cfg.lines) * column_width;
            let check_y = (i as i32 % self.cfg.lines + 1) * row_height;
            if ev_y >= check_y
                && ev_y <= check_y + row_height
                && ev_x >= check_x
                && ev_x <= check_x + column_width
            {
                if self.selected == Some(pos) {
                    return;
                }
                self.selected = Some(pos);
                self.draw_menu();
                return;
            }
        }
    }

    /// Vertical list hover selection.
    fn hover_vertical(&mut self, ev_y: i32) {
        let mut y = 0;
        let row_height = self.bar_height;
        let start = self.current.unwrap_or(0);
        let end = self.next.unwrap_or(self.matches.len());
        for pos in start..end {
            y += row_height;
            if ev_y >= y && ev_y <= (y + row_height) {
                if self.selected == Some(pos) {
                    return;
                }
                self.selected = Some(pos);
                self.draw_menu();
                return;
            }
        }
    }

    /// Horizontal list hover selection.
    fn hover_horizontal(&mut self, ev_x: i32, mut x: i32) {
        x += self.input_width;
        let mut arrow_width = self.text_width("<");
        let start = self.current.unwrap_or(0);
        let end = self.next.unwrap_or(self.matches.len());
        for pos in start..end {
            x += arrow_width;
            let item_text = self.items[self.matches[pos]].text.clone();
            let right_arrow_width = self.text_width(">");
            arrow_width = self.text_width(&item_text).min(self.menu_width - x - right_arrow_width);
            if ev_x >= x && ev_x <= x + arrow_width {
                if self.selected == Some(pos) {
                    return;
                }
                self.selected = Some(pos);
                self.draw_menu();
                return;
            }
        }
    }

    /// button_press
    pub(super) fn button_press(&mut self, button: MouseButton, state: u32, ev_x: i32, ev_y: i32) {
        /* right-click: exit */
        if button == MouseButton::Right {
            std::process::exit(1);
        }

        let x = self.prompt_offset();
        let w = self.input_field_width(x);

        match button {
            MouseButton::Left => self.left_click(state, ev_x, ev_y, x, w),
            /* middle-mouse click: paste selection */
            MouseButton::Middle => {
                self.backend.request_selection(state & SHIFT_MASK != 0);
                self.draw_menu();
            }
            /* scroll up */
            MouseButton::ScrollUp
                if self.prev != 0 || self.current.map(|c| c > 0).unwrap_or(false) =>
            {
                self.selected = self.current;
                self.current = Some(self.prev);
                self.calc_offsets();
                self.draw_menu();
            }
            /* scroll down */
            MouseButton::ScrollDown if self.next.is_some() => {
                let next = self.next.unwrap();
                self.selected = Some(next);
                self.current = Some(next);
                self.calc_offsets();
                self.draw_menu();
            }
            _ => {}
        }
    }

    /// left-click: clear the input, or click an item/arrow.
    fn left_click(&mut self, state: u32, ev_x: i32, ev_y: i32, x: i32, w: i32) {
        let y = 0;
        let row_height = self.bar_height;

        /* left-click on input: clear input,
         * NOTE: if there is no left-arrow the space for < is reserved so
         *       add that to the input width */
        let _arrow_width = self.text_width("");
        let input_hit = (self.cfg.lines <= 0
            && ev_x >= 0
            && ev_x
                <= x + w
                    + if self.prev == 0 || self.current.map(|c| c == 0).unwrap_or(true) {
                        self.text_width("<")
                    } else {
                        0
                    })
            || (self.cfg.lines > 0 && ev_y >= y && ev_y <= y + row_height);
        if input_hit {
            if self.cfg.left_command.is_some() && ev_x < self.text_width("") {
                self.trigger_command(0);
            } else if ev_x > self.menu_width - self.text_width("") {
                self.trigger_command(1);
            } else {
                let cursor = self.cursor as i32;
                self.insert(None, -cursor);
                self.draw_menu();
            }
            return;
        } else if self.cfg.lines > 0 {
            self.vertical_click(state);
            return;
        } else if !self.matches.is_empty() {
            self.horizontal_click(state, ev_x, x);
            return;
        }
    }

    /// left-click on a vertical list item.
    fn vertical_click(&mut self, state: u32) {
        let item = self.selected_text();
        if let Some(text) = &item {
            if text.starts_with('>') {
                return;
            }
        }
        self.animate_selection();
        let out = item.unwrap_or_else(|| self.text.clone());
        self.println(&out);
        if state & CONTROL_MASK == 0 {
            std::process::exit(0);
        }
        if let Some(s) = self.selected {
            self.items[self.matches[s]].already_output = true;
        }
        self.draw_menu();
    }

    /// left-click on the horizontal list: arrows and items.
    fn horizontal_click(&mut self, state: u32, ev_x: i32, mut x: i32) {
        x += self.input_width;
        let mut arrow_width = self.text_width("<");
        if self.prev != 0 || self.current.map(|c| c > 0).unwrap_or(false) {
            if ev_x >= x && ev_x <= x + arrow_width {
                self.selected = self.current;
                self.current = Some(self.prev);
                self.calc_offsets();
                self.draw_menu();
                return;
            }
        }
        let start = self.current.unwrap_or(0);
        let end = self.next.unwrap_or(self.matches.len());
        for pos in start..end {
            x += arrow_width;
            let item_text = self.items[self.matches[pos]].text.clone();
            let right_arrow_width = self.text_width(">");
            arrow_width = self.text_width(&item_text).min(self.menu_width - x - right_arrow_width);
            if ev_x >= x && ev_x <= x + arrow_width {
                if let Some(text) = self.selected_text() {
                    if item_text.starts_with('>') && !text.is_empty() {
                        break;
                    }
                }
                self.animate_selection();
                self.println(&item_text);
                if state & CONTROL_MASK == 0 {
                    std::process::exit(0);
                }
                self.selected = Some(pos);
                self.items[self.matches[pos]].already_output = true;
                self.draw_menu();
                return;
            }
        }
        /* left-click on right arrow */
        let right_arrow_width = self.text_width(">");
        arrow_width = right_arrow_width;
        x = self.menu_width - arrow_width;
        if self.next.is_some() && ev_x >= x && ev_x <= x + arrow_width {
            let next = self.next.unwrap();
            self.selected = Some(next);
            self.current = Some(next);
            self.calc_offsets();
            self.draw_menu();
            return;
        }
    }

    /// paste — insert selection text.
    pub(super) fn paste(&mut self, text: &str) {
        /* we have been given the current selection, now insert it into input */
        let line = text.split('\n').next().unwrap_or("");
        self.insert(Some(line), line.len() as i32);
        self.draw_menu();
    }
}
