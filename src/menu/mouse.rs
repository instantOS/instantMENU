//! Mouse handling: hover selection, button presses and paste. All hit-tests
//! read their geometry from [`Header`] (`Menu::header`), the same rects the
//! renderer drew.

use super::layout::Header;
use super::paging;
use super::transition::Transition;
use super::Menu;
use crate::backend::{Modifiers, MouseButton};
use crate::enums::{EditOp, ExitStatus, Side};
use crate::geom::Point;

impl Menu {
    /// set_selection — hover selection on motion.
    pub(super) fn set_selection(&mut self, pos: Point) -> Transition {
        let header = self.header();
        if self.layout.lines > 0 {
            if self.layout.columns > 0 {
                self.hover_columns(pos, &header)
            } else {
                self.hover_vertical(pos)
            }
        } else if !self.matcher.matches.is_empty() {
            self.hover_horizontal(pos, &header)
        } else {
            Transition::Nop
        }
    }

    /// Column/grid hover selection: hit-test the same cell layout as draw_grid.
    fn hover_columns(&mut self, pos: Point, header: &Header) -> Transition {
        let start = self.selection.page_start.unwrap_or(0);
        let end = self.paging.next.unwrap_or(self.matcher.matches.len());
        for (i, item) in (start..end).enumerate() {
            let cell = self.layout.grid_cell_rect(i, header.content_x);
            if cell.contains(pos) {
                if !self.matcher.match_is_selectable(item) {
                    return Transition::Nop;
                }
                if self.selection.selected == Some(item) {
                    return Transition::Nop;
                }
                self.selection.selected = Some(item);
                return Transition::Redraw;
            }
        }
        Transition::Nop
    }

    /// Vertical list hover selection: rows only, any x.
    fn hover_vertical(&mut self, pos: Point) -> Transition {
        let start = self.selection.page_start.unwrap_or(0);
        let end = self.paging.next.unwrap_or(self.matcher.matches.len());
        for (i, item) in (start..end).enumerate() {
            let (top, bottom) = self.layout.row_band(i);
            if pos.y >= top && pos.y <= bottom {
                if !self.matcher.match_is_selectable(item) {
                    return Transition::Nop;
                }
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
    fn hover_horizontal(&mut self, pos: Point, header: &Header) -> Transition {
        for (item, rect) in self.horizontal_item_rects(header.content_x) {
            if rect.contains(pos) {
                if !self.matcher.match_is_selectable(item) {
                    return Transition::Nop;
                }
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
        mods: Modifiers,
        pos: Point,
    ) -> Transition {
        match button {
            /* right-click: exit */
            MouseButton::Right => Transition::Exit(ExitStatus::Failure),
            /* middle-mouse click: paste selection */
            MouseButton::Middle => {
                self.request_paste(mods);
                Transition::Redraw
            }
            MouseButton::Left => self.left_click(mods, pos),
        }
    }

    /// Wheel movement pages through the list (positive scrolls down).
    pub(super) fn scroll(&mut self, delta: i32) -> Transition {
        if delta < 0 {
            if self.paging.prev != 0 || self.selection.page_start.map(|c| c > 0).unwrap_or(false) {
                let page = paging::scroll_up(&self.selection, &self.paging)
                    .page_start
                    .unwrap_or(0);
                self.select_page(page);
                self.recalc_paging();
                return Transition::Redraw;
            }
        } else if let Some(next) = self.paging.next {
            self.select_page(next);
            self.recalc_paging();
            return Transition::Redraw;
        }
        Transition::Nop
    }

    /// left-click: clear the input, or click an item/arrow/command cell.
    fn left_click(&mut self, mods: Modifiers, pos: Point) -> Transition {
        let header = self.header();
        let row_height = self.layout.bar_height;

        /* left-click on input: clear input,
         * NOTE: if there is no left-arrow the space for < is reserved so
         *       add that to the input width */
        let at_page_top =
            self.paging.prev == 0 || self.selection.page_start.map(|c| c == 0).unwrap_or(true);
        let arrow_pad = if at_page_top { header.left_arrow.w } else { 0 };
        let input_hit = !self.cfg.single_key
            && ((self.layout.lines <= 0 && pos.x >= 0 && pos.x <= header.input.right() + arrow_pad)
                || (self.layout.lines > 0 && pos.y >= 0 && pos.y <= row_height));
        if input_hit {
            if let Some(cell) = header.left_command {
                if pos.x < cell.right() {
                    return self.trigger_command(Side::Left);
                }
            }
            if let Some(cell) = header.right_command {
                if pos.x > cell.x {
                    return self.trigger_command(Side::Right);
                }
            }
            let t = self.insert(EditOp::Delete(self.editor.cursor));
            return t.at_least_redraw();
        } else if self.layout.lines > 0 {
            return self.vertical_click(mods, pos, &header);
        } else if !self.matcher.matches.is_empty() {
            return self.horizontal_click(mods, pos, &header);
        }
        Transition::Nop
    }

    /// Left-click a vertical/grid cell. Resolve the clicked item directly;
    /// motion events are not guaranteed to precede a button press.
    fn vertical_click(&mut self, mods: Modifiers, pos: Point, header: &Header) -> Transition {
        let start = self.selection.page_start.unwrap_or(0);
        let end = self.paging.next.unwrap_or(self.matcher.matches.len());
        let clicked = (start..end).enumerate().find_map(|(i, item)| {
            let hit = if self.layout.columns > 0 {
                self.layout
                    .grid_cell_rect(i, header.content_x)
                    .contains(pos)
            } else {
                let (top, bottom) = self.layout.row_band(i);
                pos.y >= top && pos.y <= bottom
            };
            hit.then_some(item)
        });
        let Some(clicked) = clicked else {
            return Transition::Nop;
        };
        if !self.matcher.match_is_selectable(clicked) {
            return Transition::Nop;
        }
        self.selection.selected = Some(clicked);
        let out = self.matcher.text_of_match(clicked).to_string();
        self.confirm(&out, mods).at_least_redraw()
    }

    /// left-click on the horizontal list: arrows and items.
    fn horizontal_click(&mut self, mods: Modifiers, pos: Point, header: &Header) -> Transition {
        /* left arrow: turn back one page, selection follows the page top */
        if (self.paging.prev != 0 || self.selection.page_start.map(|c| c > 0).unwrap_or(false))
            && header.left_arrow.contains(pos)
        {
            self.selection = paging::scroll_up(&self.selection, &self.paging);
            self.recalc_paging();
            return Transition::Redraw;
        }
        for (item, rect) in self.horizontal_item_rects(header.content_x) {
            if rect.contains(pos) {
                if !self.matcher.match_is_selectable(item) {
                    return Transition::Nop;
                }
                let item_text = self.matcher.text_of_match(item).to_string();
                self.selection.selected = Some(item);
                return self.confirm(&item_text, mods).at_least_redraw();
            }
        }
        /* right arrow: turn forward one page, selecting the page top */
        if self.paging.next.is_some() && header.right_arrow.contains(pos) {
            let next = self.paging.next.unwrap();
            self.select_page(next);
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
