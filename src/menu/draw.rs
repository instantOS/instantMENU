//! Menu drawing: `draw_item` and `draw_menu`.

use super::Menu;
use crate::enums::{output_offset, ItemCategory, Scheme};

impl Menu {
    /// recalculate_numbers
    fn recalculate_numbers(&mut self) {
        let match_count = self.matches.len();
        if self.cfg.toast != 0 {
            self.show_numbers = false;
            return;
        }
        let item_count = self.items.len();
        if match_count > 1 {
            if self.cfg.lines > 1 {
                self.show_numbers = match_count > self.cfg.lines as usize;
            } else {
                self.show_numbers = true;
            }
        } else {
            self.show_numbers = false;
        }
        self.numbers = format!("{match_count}/{item_count}");
    }

    /// draw_item — draws one item at (x, y, w), returns the advanced x.
    fn draw_item(&mut self, pos: usize, x: i32, y: i32, w: i32) -> i32 {
        let is_selected = self.selected == Some(pos);
        let text = self.items[self.matches[pos]].text.clone();
        let bytes = text.as_bytes();

        let mut category = self.classify_item(pos, bytes, is_selected);

        let mut temp_padding = 0;
        if category == ItemCategory::Colored && bytes.get(2) == Some(&b' ') {
            temp_padding = self.draw_icon(&text, is_selected, x, y);
            category = ItemCategory::Icon;
        }

        let output: &str = if self.cfg.commented {
            // single letter display (the full first UTF-8 char; a raw byte
            // cut would panic on multi-byte text)
            match text.chars().next() {
                Some(c) => &text[..c.len_utf8()],
                None => "",
            }
        } else {
            &text
        };
        let offset = output_offset(category);
        let shown = output.get(offset..).unwrap_or("");

        if is_selected {
            self.selected_y = y;
        }

        let x_in = x + if category == ItemCategory::Icon { temp_padding } else { 0 };
        let w_in = if self.cfg.commented {
            self.bar_height
        } else {
            w - if category == ItemCategory::Icon { temp_padding } else { 0 }
        };
        let left_padding = if self.cfg.commented {
            (self.bar_height - self.renderer.text_width(output)) / 2
        } else {
            self.renderer.horizontal_padding / 2
        };

        self.renderer.text(
            &mut self.canvas,
            x_in,
            y,
            w_in,
            self.bar_height,
            left_padding,
            shown,
            false,
            category == ItemCategory::ColoredComment || is_selected,
        )
    }

    /// Classify an item by its `>`/`:` prefix and set the matching scheme.
    fn classify_item(&mut self, pos: usize, bytes: &[u8], is_selected: bool) -> ItemCategory {
        let mut category = ItemCategory::Normal;
        if bytes.first() == Some(&b'>') {
            if bytes.get(1) == Some(&b'>') {
                category = ItemCategory::ColoredComment;
                let scheme = match bytes.get(2) {
                    Some(b'r') => Some(Scheme::Red),
                    Some(b'g') => Some(Scheme::Green),
                    Some(b'y') => Some(Scheme::Yellow),
                    Some(b'h') => Some(Scheme::Highlight),
                    Some(b'b') => Some(Scheme::Selected),
                    _ => None,
                };
                match scheme {
                    Some(s) => self.renderer.set_scheme(s),
                    None => {
                        category = ItemCategory::Comment;
                        self.renderer.set_scheme(Scheme::Normal);
                    }
                }
            } else {
                self.renderer.set_scheme(Scheme::Normal);
                category = ItemCategory::Comment;
            }
        } else if bytes.first() == Some(&b':') {
            category = ItemCategory::Colored;
            if is_selected {
                let scheme = match bytes.get(1) {
                    Some(b'r') => Some(Scheme::Red),
                    Some(b'g') => Some(Scheme::Green),
                    Some(b'y') => Some(Scheme::Yellow),
                    Some(b'b') => Some(Scheme::Selected),
                    _ => None,
                };
                match scheme {
                    Some(s) => self.renderer.set_scheme(s),
                    None => {
                        self.renderer.set_scheme(Scheme::Selected);
                        category = ItemCategory::Normal;
                    }
                }
            } else {
                self.renderer.set_scheme(Scheme::Normal);
            }
        } else {
            let scheme = if is_selected {
                Scheme::Selected
            } else if self.items[self.matches[pos]].already_output {
                Scheme::Output
            } else {
                Scheme::Normal
            };
            self.renderer.set_scheme(scheme);
        }
        category
    }

    /// Draw the icon of a `:X ` item; returns the horizontal padding it used.
    fn draw_icon(&mut self, text: &str, is_selected: bool, x: i32, y: i32) -> i32 {
        let temp_padding = self.renderer.font_height * 3;
        self.cfg.animated = true;
        // draw the icon (the three bytes after the ":X " prefix)
        let end = (3..=6)
            .rev()
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(3);
        let icon_text = text.get(3..end).unwrap_or("");
        let left_padding = (temp_padding as f64 / 2.6) as i32;
        self.renderer.text(
            &mut self.canvas,
            x,
            y,
            temp_padding,
            self.cfg.line_height,
            left_padding,
            icon_text,
            false,
            is_selected,
        );
        let sc = if is_selected { Scheme::Hover } else { Scheme::Normal };
        self.renderer.set_scheme(sc);
        temp_padding
    }

    pub(super) fn draw_menu(&mut self) {
        let font_height = self.renderer.font_height;
        let mut x = 0;
        let y = 0;
        let arrow_width = self.text_width("");

        self.update_commented_prompt();

        let scheme_norm = self.renderer.color_scheme(Scheme::Normal);
        self.renderer.set_scheme(Scheme::Normal);
        self.renderer.clear(&mut self.canvas, scheme_norm.bg);

        self.draw_prompt(&mut x, arrow_width);
        self.draw_input_field(x, arrow_width, font_height);

        self.recalculate_numbers();

        if self.cfg.lines > 0 {
            self.draw_grid(x, y);
        } else if !self.matches.is_empty() {
            self.draw_horizontal_list(x);
        }

        self.draw_footer(arrow_width);
        self.backend.present(&self.canvas);
    }

    /// commented mode: the prompt follows the selected item.
    fn update_commented_prompt(&mut self) {
        if self.cfg.commented && !self.matches.is_empty() {
            let selected_text = self.selected_text().unwrap_or_default();
            let stripped = selected_text.get(1..).unwrap_or("");
            self.comment_prompt = Some(stripped.to_string());
        }
    }

    /// Draw the prompt, advancing `x` past it.
    fn draw_prompt(&mut self, x: &mut i32, arrow_width: i32) {
        let prompt = self.prompt().map(|p| p.to_string());
        if let Some(prompt) = prompt.filter(|p| !p.is_empty()) {
            if self.cfg.left_command.is_some() {
                *x += arrow_width;
            }
            self.renderer.set_scheme(Scheme::Selected);
            if self.cfg.lines < 8 {
                *x = self.renderer.text(
                    &mut self.canvas,
                    *x,
                    0,
                    self.prompt_width,
                    self.bar_height * (self.cfg.lines + 1),
                    self.renderer.horizontal_padding / 2,
                    &prompt,
                    true,
                    false,
                );
            } else {
                *x = self.renderer.text(
                    &mut self.canvas,
                    *x,
                    0,
                    self.prompt_width,
                    self.bar_height,
                    self.renderer.horizontal_padding / 2,
                    &prompt,
                    true,
                    false,
                );
            }
        }
    }

    /// Draw the input field (text, search_text placeholder or password dots)
    /// and the cursor.
    fn draw_input_field(&mut self, x: i32, arrow_width: i32, font_height: i32) {
        let w = if self.cfg.lines > 0 || self.matches.is_empty() {
            self.menu_width - x
        } else {
            self.input_width
        };
        self.renderer.set_scheme(Scheme::Normal);

        if self.cfg.password {
            let dots = ".".repeat(self.text.len());
            self.renderer.text(
                &mut self.canvas,
                x,
                0,
                w,
                self.bar_height,
                self.renderer.horizontal_padding / 2,
                &dots,
                false,
                false,
            );
        } else if !self.text.is_empty() {
            self.renderer.text(
                &mut self.canvas,
                x + if self.cfg.left_command.is_some() { arrow_width } else { 0 },
                0,
                w,
                self.bar_height,
                self.renderer.horizontal_padding / 2,
                &self.text,
                false,
                false,
            );
        } else if let Some(search_text) = self.cfg.search_text.as_deref() {
            self.renderer.set_scheme(Scheme::Fade);
            self.renderer.text(
                &mut self.canvas,
                x + if self.cfg.left_command.is_some() { arrow_width } else { 0 },
                0,
                w,
                self.bar_height,
                self.renderer.horizontal_padding / 2,
                search_text,
                false,
                false,
            );
            self.renderer.set_scheme(Scheme::Normal);
        }

        let mut cursor_position = if self.cfg.commented {
            self.bar_height
        } else {
            self.renderer.text_width(&self.text[..self.cursor])
                + self.renderer.horizontal_padding
        };
        cursor_position += self.renderer.horizontal_padding / 2 - 1;
        if cursor_position < w {
            self.renderer.set_scheme(Scheme::Normal);
            // disable cursor on password prompt
            if !self.cfg.password && self.cfg.toast == 0 {
                self.renderer.rect(
                    &mut self.canvas,
                    x + if self.cfg.left_command.is_some() { arrow_width } else { 0 }
                        + cursor_position,
                    2 + (self.bar_height - font_height) / 2,
                    2,
                    font_height - 4,
                    true,
                    false,
                    false,
                );
            }
        }
    }

    /// Draw the vertical list / grid of items.
    fn draw_grid(&mut self, x: i32, y: i32) {
        let start = self.current.unwrap_or(0);
        let end = self.next.unwrap_or(self.matches.len());
        let column_width = (self.menu_width - x) / self.cfg.columns;
        for (i, pos) in (start..end).enumerate() {
            let item_x = x + (i as i32 / self.cfg.lines) * column_width;
            let item_y = y + ((i as i32 % self.cfg.lines) + 1) * self.bar_height;
            self.draw_item(pos, item_x, item_y, column_width);
        }
    }

    /// Draw the horizontal list of items with the paging arrows.
    fn draw_horizontal_list(&mut self, mut x: i32) {
        x += self.input_width;
        let mut arrow_width = self.text_width("<");
        if self.current.map(|c| c > 0).unwrap_or(false) {
            self.renderer.set_scheme(Scheme::Normal);
            self.renderer.text(
                &mut self.canvas,
                x,
                0,
                arrow_width,
                self.bar_height,
                self.renderer.horizontal_padding / 2,
                "<",
                false,
                false,
            );
        }
        x += arrow_width;

        let start = self.current.unwrap_or(0);
        let end = self.next.unwrap_or(self.matches.len());
        for pos in start..end {
            let budget = self.menu_width - x - self.text_width(">") - self.text_width(&self.numbers.clone());
            let text = self.items[self.matches[pos]].text.clone();
            let item_width = self.text_width_clamp(&text, budget);
            x = self.draw_item(pos, x, 0, item_width);
        }

        if self.next.is_some() {
            arrow_width = self.text_width(">");
            self.renderer.set_scheme(Scheme::Normal);
            if self.show_numbers {
                let numbers = self.numbers.clone();
                let numbers_width = self.text_width(&numbers);
                self.renderer.text(
                    &mut self.canvas,
                    self.menu_width - arrow_width - numbers_width,
                    0,
                    arrow_width,
                    self.bar_height,
                    self.renderer.horizontal_padding / 2,
                    ">",
                    false,
                    false,
                );
            }
        }
    }

    /// Draw the item counter and the left/right command cells.
    fn draw_footer(&mut self, arrow_width: i32) {
        self.renderer.set_scheme(Scheme::Normal);
        if self.show_numbers {
            let numbers = self.numbers.clone();
            let numbers_width = self.text_width(&numbers);
            let right_padding = if self.cfg.right_command.is_some() { arrow_width } else { 0 };
            self.renderer.text(
                &mut self.canvas,
                self.menu_width - numbers_width - right_padding,
                0,
                numbers_width,
                self.bar_height,
                self.renderer.horizontal_padding / 2,
                &numbers,
                false,
                false,
            );
        }
        if self.cfg.lines > 0 {
            if self.cfg.left_command.is_some() {
                self.renderer.set_scheme(Scheme::Highlight);
                self.renderer.text(
                    &mut self.canvas,
                    0,
                    0,
                    arrow_width,
                    self.bar_height,
                    self.renderer.horizontal_padding / 2,
                    "",
                    false,
                    false,
                );
            }
            if self.cfg.right_command.is_some() {
                self.renderer.set_scheme(Scheme::Highlight);
                self.renderer.text(
                    &mut self.canvas,
                    self.menu_width - arrow_width,
                    0,
                    arrow_width,
                    self.bar_height,
                    self.renderer.horizontal_padding / 2,
                    "",
                    false,
                    false,
                );
            }
        }
    }
}
