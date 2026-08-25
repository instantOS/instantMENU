//! Menu drawing: `draw_item` and `draw_menu`. All header-row geometry comes
//! from [`Header`] (`Menu::header`) — the same rects mouse hit-testing uses.

use super::layout::Header;
use super::measure::icon_gutter_width;
use super::{Menu, LEFT_GLYPH, RIGHT_GLYPH};
use crate::entry::ItemEntry;
use crate::enums::{ColorRole, Scheme};
use crate::geom::Rect;
use crate::render::TextStyle;

impl Menu {
    /// recalculate_match_counter
    fn recalculate_match_counter(&mut self) {
        let match_count = self.matcher.selectable_match_count();
        if self.cfg.toast.is_some() {
            self.show_match_counter = false;
            return;
        }
        let item_count = self.matcher.selectable_item_count();
        if match_count > 1 {
            if self.layout.lines > 1 {
                self.show_match_counter = match_count > self.layout.lines as usize;
            } else {
                self.show_match_counter = true;
            }
        } else {
            self.show_match_counter = false;
        }
        self.match_counter_text = format!("{match_count}/{item_count}");
    }

    /// draw_item — draws one item in `cell`.
    fn draw_item(&mut self, pos: usize, cell: Rect) {
        let is_selected = self.selection.selected == Some(pos);
        let index = self.matcher.matches[pos];
        let entry = self.matcher.items[index].entry;
        let text = self.matcher.items[index].text.clone();

        let scheme = if entry.heading {
            // headings are not selectable, so their color is constant
            entry.scheme.unwrap_or(if is_selected {
                Scheme::Selected
            } else {
                Scheme::Normal
            })
        } else if is_selected {
            // an ordinary item shows its color while selected …
            entry.scheme.unwrap_or(Scheme::Selected)
        } else if entry.icon.is_some() {
            // … and an unselected icon gutter stays uncolored
            Scheme::Normal
        } else if self.matcher.items[index].already_output {
            Scheme::Output
        } else {
            Scheme::Normal
        };
        self.renderer.set_scheme(scheme);

        let mut temp_padding = 0;
        if entry.icon.is_some() {
            temp_padding = self.draw_icon(entry, is_selected, cell.x, cell.y);
            if entry.heading {
                self.renderer
                    .set_scheme(entry.scheme.unwrap_or(Scheme::Normal));
            }
        }

        let shown = if self.cfg.single_key {
            entry.key.map(|key| key.to_string()).unwrap_or_default()
        } else {
            text
        };

        if is_selected {
            self.selected_y = cell.y;
        }

        let label_x = cell.x
            + if entry.icon.is_some() {
                temp_padding
            } else {
                0
            };
        let label_w = if self.cfg.single_key {
            self.layout.bar_height
        } else {
            cell.w
                - if entry.icon.is_some() {
                    temp_padding
                } else {
                    0
                }
        };
        let left_padding = if self.cfg.single_key {
            (self.layout.bar_height - self.renderer.text_width(&shown)) / 2
        } else {
            self.renderer.cell_inset()
        };

        let style = if entry.heading || is_selected {
            TextStyle::Accented
        } else {
            TextStyle::Normal
        };
        let rect = Rect::new(label_x, cell.y, label_w, self.layout.bar_height);
        let mut p = self.painter();
        p.draw_text_styled(rect, left_padding, &shown, style);
    }

    /// Draw an item's icon gutter at the row's left edge; returns the
    /// cell padding it used. The gutter spans the full bar height (the
    /// `--line-height` value only sets the *minimum* row height), so the
    /// glyph is vertically centered like the label and the accent strip
    /// sits at the row's bottom edge instead of 4px from its top.
    /// `draw_item` has already set the scheme (the entry's color while
    /// selected, Normal otherwise); afterwards the label draws in the
    /// Hover scheme when selected, Normal otherwise.
    fn draw_icon(&mut self, entry: ItemEntry, is_selected: bool, x: i32, y: i32) -> i32 {
        let temp_padding = icon_gutter_width(self.renderer.font_height);
        let icon = entry.icon.unwrap_or('?');
        let mut glyph = [0u8; 4];
        let icon_text = icon.encode_utf8(&mut glyph);
        let left_padding = (temp_padding as f64 / 2.6) as i32;
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
        temp_padding
    }

    pub(super) fn draw_menu(&mut self) {
        if self.slider.is_some() {
            self.draw_slide();
            return;
        }
        let font_height = self.renderer.font_height;

        self.update_single_key_prompt();

        let scheme_norm = self.renderer.color_scheme(Scheme::Normal);
        let mut p = self.painter();
        p.set_scheme(Scheme::Normal);
        p.clear(scheme_norm.bg);

        /* the item counter feeds the header (the ">" sits left of it), so
         * the header geometry is resolved after it is up to date */
        self.recalculate_match_counter();
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

    /// Single-key mode: the prompt follows the selected item's full label.
    fn update_single_key_prompt(&mut self) {
        if self.cfg.single_key && !self.matcher.matches.is_empty() {
            self.single_key_prompt = Some(self.selected_text().unwrap_or_default());
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
        let mut cursor_position = if self.cfg.single_key {
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
        let start = self.selection.page_start.unwrap_or(0);
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
        if self.selection.page_start.map(|c| c > 0).unwrap_or(false) {
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
        if self.show_match_counter {
            let match_counter_text = self.match_counter_text.clone();
            let counter_width = self.cell_width(&match_counter_text);
            let right_padding = if self.cfg.right_command.is_some() {
                header.command_width
            } else {
                0
            };
            let mut p = self.painter();
            p.set_scheme(Scheme::Normal);
            p.draw_text(
                Rect::new(
                    menu_width - counter_width - right_padding,
                    0,
                    counter_width,
                    bar_height,
                ),
                lpad,
                &match_counter_text,
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
