//! Menu drawing: `draw_item` and `draw_menu`.

use super::Menu;
use crate::enums::{output_offset, ItemCategory, Scheme};

/// Byte-slice a string safely (C pointer arithmetic on the item text).
fn safe_slice(s: &str, from: usize, to: usize) -> &str {
    if from >= to || from > s.len() {
        return "";
    }
    if s.is_char_boundary(from) && s.is_char_boundary(to.min(s.len())) {
        &s[from..to.min(s.len())]
    } else {
        ""
    }
}

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
            // single letter display
            &text[..text.len().min(1)]
        } else {
            &text
        };
        let offset = output_offset(category);
        let shown = safe_slice(output, offset, output.len());

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
                    Some(s) => {
                        let sc = self.renderer.schemes[s as usize];
                        self.renderer.scheme = sc;
                    }
                    None => {
                        category = ItemCategory::Comment;
                        let sc = self.renderer.schemes[Scheme::Normal as usize];
                        self.renderer.scheme = sc;
                    }
                }
            } else {
                let sc = self.renderer.schemes[Scheme::Normal as usize];
                self.renderer.scheme = sc;
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
                    Some(s) => {
                        let sc = self.renderer.schemes[s as usize];
                        self.renderer.scheme = sc;
                    }
                    None => {
                        let sc = self.renderer.schemes[Scheme::Selected as usize];
                        self.renderer.scheme = sc;
                        category = ItemCategory::Normal;
                    }
                }
            } else {
                let sc = self.renderer.schemes[Scheme::Normal as usize];
                self.renderer.scheme = sc;
            }
        } else {
            let sc = if is_selected {
                self.renderer.schemes[Scheme::Selected as usize]
            } else if self.items[self.matches[pos]].already_output {
                self.renderer.schemes[Scheme::Output as usize]
            } else {
                self.renderer.schemes[Scheme::Normal as usize]
            };
            self.renderer.scheme = sc;
        }
        category
    }

    /// Draw the icon of a `:X ` item; returns the horizontal padding it used.
    fn draw_icon(&mut self, text: &str, is_selected: bool, x: i32, y: i32) -> i32 {
        let temp_padding = self.renderer.font_height * 3;
        self.cfg.animated = true;
        // draw the icon (first 6 bytes of text, drawn from byte 3)
        let end = text
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|i| *i < 6)
            .last()
            .unwrap_or(0);
        let end = if text.is_char_boundary(6) {
            6
        } else {
            end.max(3)
        };
        let icon: String = text.chars().skip(3).take_while(|_| false).collect();
        let _ = icon;
        let icon_text = safe_slice(text, 3, end);
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
        let sc = if is_selected {
            self.renderer.schemes[Scheme::Hover as usize]
        } else {
            self.renderer.schemes[Scheme::Normal as usize]
        };
        self.renderer.scheme = sc;
        temp_padding
    }

    pub(super) fn draw_menu(&mut self) {
        let font_height = self.renderer.font_height;
        let mut x = 0;
        let y = 0;
        let arrow_width = self.text_width("");

        self.update_commented_prompt();

        let scheme_norm = self.renderer.schemes[Scheme::Normal as usize];
        self.renderer.scheme = scheme_norm;
        self.renderer.clear(&mut self.canvas, scheme_norm[crate::enums::COLOR_BG]);

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
            let stripped = safe_slice(&selected_text, 1, selected_text.len());
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
            let sc = self.renderer.schemes[Scheme::Selected as usize];
            self.renderer.scheme = sc;
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
        let sc = self.renderer.schemes[Scheme::Normal as usize];
        self.renderer.scheme = sc;

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
                &self.text.clone(),
                false,
                false,
            );
        } else if let Some(search_text) = self.cfg.search_text.clone() {
            let sc = self.renderer.schemes[Scheme::Fade as usize];
            self.renderer.scheme = sc;
            self.renderer.text(
                &mut self.canvas,
                x + if self.cfg.left_command.is_some() { arrow_width } else { 0 },
                0,
                w,
                self.bar_height,
                self.renderer.horizontal_padding / 2,
                &search_text,
                false,
                false,
            );
            let sc = self.renderer.schemes[Scheme::Normal as usize];
            self.renderer.scheme = sc;
        }

        let mut cursor_position = if self.cfg.commented {
            self.bar_height
        } else {
            self.renderer.text_width(&self.text[..self.cursor])
                + self.renderer.horizontal_padding
        };
        cursor_position += self.renderer.horizontal_padding / 2 - 1;
        if cursor_position < w {
            let sc = self.renderer.schemes[Scheme::Normal as usize];
            self.renderer.scheme = sc;
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
        let mut i = 0;
        let mut pos = self.current;
        while let Some(p) = pos {
            if Some(p) == self.next {
                break;
            }
            let column_width = (self.menu_width - x) / self.cfg.columns;
            let item_x = x + (i / self.cfg.lines) * column_width;
            let item_y = y + ((i % self.cfg.lines) + 1) * self.bar_height;
            self.draw_item(p, item_x, item_y, column_width);
            i += 1;
            pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
        }
    }

    /// Draw the horizontal list of items with the paging arrows.
    fn draw_horizontal_list(&mut self, mut x: i32) {
        x += self.input_width;
        let mut arrow_width = self.text_width("<");
        if self.current.map(|c| c > 0).unwrap_or(false) {
            let sc = self.renderer.schemes[Scheme::Normal as usize];
            self.renderer.scheme = sc;
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

        let mut pos = self.current;
        while let Some(p) = pos {
            if Some(p) == self.next {
                break;
            }
            let budget = self.menu_width - x - self.text_width(">") - self.text_width(&self.numbers.clone());
            let text = self.items[self.matches[p]].text.clone();
            let item_width = self.text_width_clamp(&text, budget);
            x = self.draw_item(p, x, 0, item_width);
            pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
        }

        if self.next.is_some() {
            arrow_width = self.text_width(">");
            let sc = self.renderer.schemes[Scheme::Normal as usize];
            self.renderer.scheme = sc;
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
        let sc = self.renderer.schemes[Scheme::Normal as usize];
        self.renderer.scheme = sc;
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
                let sc = self.renderer.schemes[Scheme::Highlight as usize];
                self.renderer.scheme = sc;
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
                let sc = self.renderer.schemes[Scheme::Highlight as usize];
                self.renderer.scheme = sc;
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
