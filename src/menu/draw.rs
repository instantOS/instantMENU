//! Menu drawing: `draw_item` and `draw_menu`.

use super::{Menu, LEFT_GLYPH, RIGHT_GLYPH};
use crate::enums::{output_offset, ColorRole, ItemCategory, Scheme};
use crate::geom::Rect;
use crate::render::Painter;

impl Menu {
    /// recalculate_numbers
    fn recalculate_numbers(&mut self) {
        let match_count = self.matcher.matches.len();
        if self.cfg.toast != 0 {
            self.show_numbers = false;
            return;
        }
        let item_count = self.matcher.items.len();
        if match_count > 1 {
            if self.layout.lines > 1 {
                self.show_numbers = match_count > self.layout.lines as usize;
            } else {
                self.show_numbers = true;
            }
        } else {
            self.show_numbers = false;
        }
        self.numbers = format!("{match_count}/{item_count}");
    }

    /// draw_item — draws one item in `cell`.
    fn draw_item(&mut self, pos: usize, cell: Rect) {
        let is_selected = self.selection.selected == Some(pos);
        let text = self.matcher.text_of_match(pos).to_string();
        let bytes = text.as_bytes();

        let mut category = self.classify_item(pos, &text, is_selected);

        let mut temp_padding = 0;
        let mut label_offset = 0usize;
        if category == ItemCategory::Colored && bytes.get(2) == Some(&b' ') {
            (temp_padding, label_offset) = self.draw_icon(&text, is_selected, cell.x, cell.y);
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
        /* icon items: the label starts after the ":X " prefix plus the actual
         * icon bytes, which output_offset cannot express statically */
        let offset = if category == ItemCategory::Icon {
            label_offset
        } else {
            output_offset(category)
        };
        let shown = output.get(offset..).unwrap_or("");

        if is_selected {
            self.selected_y = cell.y;
        }

        let label_x = cell.x
            + if category == ItemCategory::Icon {
                temp_padding
            } else {
                0
            };
        let label_w = if self.cfg.commented {
            self.layout.bar_height
        } else {
            cell.w
                - if category == ItemCategory::Icon {
                    temp_padding
                } else {
                    0
                }
        };
        let left_padding = if self.cfg.commented {
            (self.layout.bar_height - self.renderer.text_width(output)) / 2
        } else {
            self.renderer.horizontal_padding / 2
        };

        let is_accented = category == ItemCategory::ColoredComment || is_selected;
        let rect = Rect::new(label_x, cell.y, label_w, self.layout.bar_height);
        let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
        p.draw_item(rect, left_padding, shown, is_accented);
    }

    /// Classify an item by its `>`/`:` prefix and set the matching scheme.
    fn classify_item(&mut self, pos: usize, text: &str, is_selected: bool) -> ItemCategory {
        let (category, prefixed) = ItemCategory::from_prefix(text, is_selected);
        let scheme = match prefixed {
            Some(s) => s,
            None if category == ItemCategory::Normal => {
                // plain item: the scheme follows selection/output state
                if is_selected {
                    Scheme::Selected
                } else if self.matcher.items[self.matcher.matches[pos]].already_output {
                    Scheme::Output
                } else {
                    Scheme::Normal
                }
            }
            None => Scheme::Normal, // unselected `:c` item
        };
        self.renderer.set_scheme(scheme);
        category
    }

    /// Draw the icon of a `:X ` item; returns the cell padding it used and
    /// the byte offset of the label inside `text` (the 3-byte prefix plus
    /// the icon glyph — everything actually drawn as the icon).
    fn draw_icon(&mut self, text: &str, is_selected: bool, x: i32, y: i32) -> (i32, usize) {
        let temp_padding = self.renderer.font_height * 3;
        // the icon is the first UTF-8 char after the ":X " prefix; the label
        // starts right after it, however many bytes the glyph is
        let end = text
            .get(3..)
            .and_then(|rest| rest.chars().next())
            .map_or(3, |c| 3 + c.len_utf8());
        let icon_text = text.get(3..end).unwrap_or("");
        let left_padding = (temp_padding as f64 / 2.6) as i32;
        // the icon cell spans the full bar height (the `--line-height` value
        // only sets the *minimum* row height), so the glyph is vertically
        // centered like the label and the accent strip sits at the row's
        // bottom edge instead of 4px from its top
        let rect = Rect::new(x, y, temp_padding, self.layout.bar_height);
        let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
        p.draw_item(rect, left_padding, icon_text, is_selected);
        let sc = if is_selected {
            Scheme::Hover
        } else {
            Scheme::Normal
        };
        self.renderer.set_scheme(sc);
        (temp_padding, end)
    }

    pub(super) fn draw_menu(&mut self) {
        if self.slider.is_some() {
            self.draw_slide();
            return;
        }
        let font_height = self.renderer.font_height;
        let mut x = 0;
        let y = 0;
        let arrow_width = self.command_cell_width();

        self.update_commented_prompt();

        let scheme_norm = self.renderer.color_scheme(Scheme::Normal);
        let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
        p.set_scheme(Scheme::Normal);
        p.clear(scheme_norm.bg);

        self.draw_prompt(&mut x, arrow_width);
        self.draw_input_field(x, arrow_width, font_height);

        self.recalculate_numbers();

        if self.layout.lines > 0 {
            self.draw_grid(x, y);
        } else if !self.matcher.matches.is_empty() {
            self.draw_horizontal_list(x);
        }

        self.draw_footer(arrow_width);
        self.backend.present(&self.canvas);
    }

    /// commented mode: the prompt follows the selected item.
    fn update_commented_prompt(&mut self) {
        if self.cfg.commented && !self.matcher.matches.is_empty() {
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
            // short menus: the prompt spans all rows, tall ones one row
            let prompt_height = if self.layout.lines < 8 {
                self.layout.bar_height * (self.layout.lines + 1)
            } else {
                self.layout.bar_height
            };
            let rect = Rect::new(*x, 0, self.layout.prompt_width, prompt_height);
            let lpad = self.renderer.horizontal_padding / 2;
            let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
            p.set_scheme(Scheme::Selected);
            /* the prompt is drawn with the selected scheme but NOT inverted:
             * the C calls drw_text(..., 0, 1) where the last argument is
             * `rounded` — the prompt is a solid (blue) block with the
             * selected text color on top */
            *x = p.draw_text(rect, lpad, &prompt);
        }
    }

    /// Draw the input field (text, placeholder or password dots)
    /// and the cursor.
    fn draw_input_field(&mut self, x: i32, arrow_width: i32, font_height: i32) {
        let w = if self.layout.lines > 0 || self.matcher.matches.is_empty() {
            self.layout.menu_width - x
        } else {
            self.layout.input_width
        };
        let field_x = x + if self.cfg.left_command.is_some() {
            arrow_width
        } else {
            0
        };
        let field = Rect::new(field_x, 0, w, self.layout.bar_height);
        let lpad = self.renderer.horizontal_padding / 2;

        {
            let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
            p.set_scheme(Scheme::Normal);
            if self.cfg.password {
                let dots = ".".repeat(self.editor.text.len());
                p.draw_text(field, lpad, &dots);
            } else if !self.editor.text.is_empty() {
                p.draw_text(field, lpad, &self.editor.text);
            } else if let Some(placeholder) = self.cfg.placeholder.as_deref() {
                p.set_scheme(Scheme::Fade);
                p.draw_text(field, lpad, placeholder);
                p.set_scheme(Scheme::Normal);
            }
        }

        /* C: curpos = TEXTW(text) - TEXTW(&text[cursor]); the lrpad terms
         * cancel, leaving the width of the text before the cursor. */
        let mut cursor_position = if self.cfg.commented {
            0
        } else {
            self.renderer.text_width(&self.editor.text)
                - self
                    .renderer
                    .text_width(&self.editor.text[self.editor.cursor..])
        };
        cursor_position += self.renderer.horizontal_padding / 2 - 1;
        if cursor_position < w {
            // disable cursor on password prompt
            if !self.cfg.password && self.cfg.toast == 0 {
                let cursor_rect = Rect::new(
                    field_x + cursor_position,
                    2 + (self.layout.bar_height - font_height) / 2,
                    2,
                    font_height - 4,
                );
                let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
                p.set_scheme(Scheme::Normal);
                p.fill_rect(cursor_rect, ColorRole::Foreground);
            }
        }
    }

    /// Draw the vertical list / grid of items.
    fn draw_grid(&mut self, x: i32, y: i32) {
        let start = self.selection.current.unwrap_or(0);
        let end = self.paging.next.unwrap_or(self.matcher.matches.len());
        for (i, pos) in (start..end).enumerate() {
            let cell = self.layout.grid_cell_rect(i, x, y);
            self.draw_item(pos, cell);
        }
    }

    /// Draw the horizontal list of items with the paging arrows.
    fn draw_horizontal_list(&mut self, x: i32) {
        let bar_height = self.layout.bar_height;
        let left_arrow_x = x + self.layout.input_width;
        let arrow_width = self.text_width("<");
        let lpad = self.renderer.horizontal_padding / 2;
        if self.selection.current.map(|c| c > 0).unwrap_or(false) {
            let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
            p.set_scheme(Scheme::Normal);
            p.draw_text(
                Rect::new(left_arrow_x, 0, arrow_width, bar_height),
                lpad,
                "<",
            );
        }

        for (pos, rect) in self.horizontal_item_rects(x) {
            self.draw_item(pos, rect);
        }

        if self.paging.next.is_some() {
            let arrow_width = self.text_width(">");
            if self.show_numbers {
                let numbers = self.numbers.clone();
                let numbers_width = self.text_width(&numbers);
                let menu_width = self.layout.menu_width;
                let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
                p.set_scheme(Scheme::Normal);
                p.draw_text(
                    Rect::new(
                        menu_width - arrow_width - numbers_width,
                        0,
                        arrow_width,
                        bar_height,
                    ),
                    lpad,
                    ">",
                );
            }
        }
    }

    /// Draw the item counter and the left/right command cells.
    fn draw_footer(&mut self, arrow_width: i32) {
        let bar_height = self.layout.bar_height;
        let lpad = self.renderer.horizontal_padding / 2;
        let menu_width = self.layout.menu_width;
        if self.show_numbers {
            let numbers = self.numbers.clone();
            let numbers_width = self.text_width(&numbers);
            let right_padding = if self.cfg.right_command.is_some() {
                arrow_width
            } else {
                0
            };
            let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
            p.set_scheme(Scheme::Normal);
            p.draw_text(
                Rect::new(
                    menu_width - numbers_width - right_padding,
                    0,
                    numbers_width,
                    bar_height,
                ),
                lpad,
                &numbers,
            );
        }
        if self.layout.lines > 0 {
            if self.cfg.left_command.is_some() {
                let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
                p.set_scheme(Scheme::Highlight);
                p.draw_text(Rect::new(0, 0, arrow_width, bar_height), lpad, LEFT_GLYPH);
            }
            if self.cfg.right_command.is_some() {
                let mut p = Painter::new(&mut self.renderer, &mut self.canvas);
                p.set_scheme(Scheme::Highlight);
                p.draw_text(
                    Rect::new(menu_width - arrow_width, 0, arrow_width, bar_height),
                    lpad,
                    RIGHT_GLYPH,
                );
            }
        }
    }
}
