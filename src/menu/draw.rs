//! Menu drawing: `draw_item` and `draw_menu`. All header-row geometry comes
//! from [`Header`] (`Menu::header`) — the same rects mouse hit-testing uses.

use super::layout::Header;
use super::{Menu, LEFT_GLYPH, RIGHT_GLYPH};
use crate::enums::{output_offset, ColorRole, ItemCategory, Scheme};
use crate::geom::Rect;
use crate::render::TextStyle;

impl Menu {
    /// recalculate_numbers
    fn recalculate_numbers(&mut self) {
        let match_count = self.matcher.matches.len();
        if self.cfg.toast.is_some() {
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
            self.renderer.cell_inset()
        };

        let style = if category == ItemCategory::ColoredComment || is_selected {
            TextStyle::Accented
        } else {
            TextStyle::Normal
        };
        let rect = Rect::new(label_x, cell.y, label_w, self.layout.bar_height);
        let mut p = self.painter();
        p.draw_text_styled(rect, left_padding, shown, style);
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
        let mut p = self.painter();
        let style = if is_selected {
            TextStyle::Accented
        } else {
            TextStyle::Normal
        };
        p.draw_text_styled(rect, left_padding, icon_text, style);
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

        self.update_commented_prompt();

        let scheme_norm = self.renderer.color_scheme(Scheme::Normal);
        let mut p = self.painter();
        p.set_scheme(Scheme::Normal);
        p.clear(scheme_norm.bg);

        /* the item counter feeds the header (the ">" sits left of it), so
         * the header geometry is resolved after it is up to date */
        self.recalculate_numbers();
        let header = self.header();

        self.draw_prompt(&header);
        self.draw_input_field(&header, font_height);

        if self.layout.lines > 0 {
            self.draw_grid(header.content_x);
        } else if !self.matcher.matches.is_empty() {
            self.draw_horizontal_list(&header);
        }

        self.draw_footer(&header);
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

    /// Draw the prompt block.
    fn draw_prompt(&mut self, header: &Header) {
        let Some(rect) = header.prompt else {
            return;
        };
        let Some(prompt) = self.prompt().map(str::to_string) else {
            return;
        };
        let lpad = self.renderer.cell_inset();
        let mut p = self.painter();
        p.set_scheme(Scheme::Selected);
        /* The prompt is a solid block in the selected scheme: selected
         * background fill with the selected foreground text on top (not
         * inverted, and no detail strip). */
        p.draw_text(rect, lpad, &prompt);
    }

    /// Draw the input field (text, placeholder or password dots)
    /// and the cursor.
    fn draw_input_field(&mut self, header: &Header, font_height: i32) {
        let field = header.input;
        let w = field.w;
        let field_x = field.x;
        let lpad = self.renderer.cell_inset();

        // Choose the field text before the painter is created: the painter
        // mutably borrows the whole menu, so cfg/editor cannot be read while
        // it is alive. The placeholder is the only variant drawn faded.
        let (field_text, faded) = if self.cfg.password {
            (Some(".".repeat(self.editor.text.len())), false)
        } else if !self.editor.text.is_empty() {
            (Some(self.editor.text.clone()), false)
        } else {
            (self.cfg.placeholder.clone(), self.cfg.placeholder.is_some())
        };

        {
            let mut p = self.painter();
            p.set_scheme(if faded { Scheme::Fade } else { Scheme::Normal });
            if let Some(text) = field_text.as_deref() {
                p.draw_text(field, lpad, text);
            }
        }

        /* Cursor x = the width of the text before the cursor (full minus suffix). */
        let mut cursor_position = if self.cfg.commented {
            0
        } else {
            self.renderer.text_width(&self.editor.text)
                - self
                    .renderer
                    .text_width(&self.editor.text[self.editor.cursor..])
        };
        cursor_position += self.renderer.cell_inset() - 1;
        if cursor_position < w {
            // disable cursor on password prompt
            if !self.cfg.password && self.cfg.toast.is_none() {
                let cursor_rect = Rect::new(
                    field_x + cursor_position,
                    2 + (self.layout.bar_height - font_height) / 2,
                    2,
                    font_height - 4,
                );
                let mut p = self.painter();
                p.set_scheme(Scheme::Normal);
                p.fill_rect(cursor_rect, ColorRole::Foreground);
            }
        }
    }

    /// Draw the vertical list / grid of items.
    fn draw_grid(&mut self, x: i32) {
        let start = self.selection.current.unwrap_or(0);
        let end = self.paging.next.unwrap_or(self.matcher.matches.len());
        for (i, pos) in (start..end).enumerate() {
            let cell = self.layout.grid_cell_rect(i, x);
            self.draw_item(pos, cell);
        }
    }

    /// Draw the horizontal list of items with the paging arrows. An active
    /// arrow is always drawn at its click target (previously the right one
    /// only appeared together with the item counter, shifted off its own
    /// hit area).
    fn draw_horizontal_list(&mut self, header: &Header) {
        let lpad = self.renderer.cell_inset();
        if self.selection.current.map(|c| c > 0).unwrap_or(false) {
            let mut p = self.painter();
            p.set_scheme(Scheme::Normal);
            p.draw_text(header.left_arrow, lpad, "<");
        }

        for (pos, rect) in self.horizontal_item_rects(header.content_x) {
            self.draw_item(pos, rect);
        }

        if self.paging.next.is_some() {
            let mut p = self.painter();
            p.set_scheme(Scheme::Normal);
            p.draw_text(header.right_arrow, lpad, ">");
        }
    }

    /// Draw the item counter and the left/right command cells.
    fn draw_footer(&mut self, header: &Header) {
        let bar_height = self.layout.bar_height;
        let lpad = self.renderer.cell_inset();
        let menu_width = self.layout.menu_width;
        if self.show_numbers {
            let numbers = self.numbers.clone();
            let numbers_width = self.cell_width(&numbers);
            let right_padding = if self.cfg.right_command.is_some() {
                header.command_width
            } else {
                0
            };
            let mut p = self.painter();
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
            if let Some(cell) = header.left_command {
                let mut p = self.painter();
                p.set_scheme(Scheme::Highlight);
                p.draw_text(cell, lpad, LEFT_GLYPH);
            }
            if let Some(cell) = header.right_command {
                let mut p = self.painter();
                p.set_scheme(Scheme::Highlight);
                p.draw_text(cell, lpad, RIGHT_GLYPH);
            }
        }
    }
}
