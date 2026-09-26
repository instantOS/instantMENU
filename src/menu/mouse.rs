//! Mouse handling: hover selection, button presses and paste. All hit-tests
//! read their geometry from [`Header`] (`Menu::header`), the same rects the
//! renderer drew.

use super::accept::AcceptMode;
use super::layout::Header;
use super::paging;
use super::transition::Transition;
use super::Menu;
use crate::backend::{BackendEvent, Modifiers, MouseButton};
use crate::enums::{EditOp, ExitStatus, Side};
use crate::geom::Point;

impl Menu {
    /// set_selection — hover selection on motion.
    ///
    /// A motion event repaints only when the pointer has actually *moved*.
    /// Both halves of that matter, and they guard against different things:
    ///
    /// - Same position as last time: nothing about the pointer changed, so
    ///   there is nothing to re-decide. Without this the resolved row is
    ///   re-read on every event the server sends for a resting pointer, and
    ///   each read is a fresh chance to disagree with whatever moved the
    ///   selection in the meantime.
    /// - Moved, but landed on the row already highlighted: also a Nop. This
    ///   is what keeps hover and typing from alternating frames — typing
    ///   resets the selection to the best match, and the pointer must
    ///   genuinely change rows to take it back.
    ///
    /// The first rule is what makes a page turn stick. `scroll_one` parks the
    /// selection on the new page top, but the pointer has not moved, and the
    /// row under it is a different match index on every page. Deciding hover
    /// from the resolved row alone therefore made the two fight: the page
    /// turn won when its event came last and the pointer won when its event
    /// came last, so scrolling with the cursor resting flickered between the
    /// page top and the row under the cursor. Deciding from the pointer's
    /// position instead makes the outcome independent of event order — a
    /// resting pointer keeps the page top, and only a real move takes the
    /// selection back.
    pub(super) fn set_selection(&mut self, pos: Point) -> Transition {
        if self.hover_pos == Some(pos) {
            return Transition::Nop;
        }
        self.hover_pos = Some(pos);
        let header = self.header();
        let item = self.hovered_match(pos, &header);
        if item == self.hovered {
            return Transition::Nop;
        }
        self.hovered = item;
        match item {
            Some(item) if self.selection.selected != Some(item) => {
                self.selection.selected = Some(item);
                Transition::Redraw
            }
            _ => Transition::Nop,
        }
    }

    /// The selectable match under `pos` within the visible page window, or
    /// None. Hit-tests the same page window and geometry the renderer drew.
    fn hovered_match(&mut self, pos: Point, header: &Header) -> Option<usize> {
        if self.over_hint(pos) {
            return None;
        }
        let hit = if self.layout.lines > 0 {
            let start = self.selection.page_start.unwrap_or(0);
            let end = self.paging.next.unwrap_or(self.matcher.matches.len());
            (start..end).enumerate().find_map(|(i, item)| {
                let inside = if self.layout.columns > 0 {
                    self.layout
                        .grid_cell_rect(i, header.content_x)
                        .contains(pos)
                } else {
                    let (top, bottom) = self.layout.row_band(i);
                    pos.y >= top && pos.y <= bottom
                };
                inside.then_some(item)
            })
        } else {
            self.horizontal_item_rects(header.content_x)
                .into_iter()
                .find_map(|(item, rect)| rect.contains(pos).then_some(item))
        };
        hit.filter(|&item| self.matcher.match_is_selectable(item))
    }

    fn over_hint(&self, pos: Point) -> bool {
        self.layout.hint_rows > 0
            && pos.y >= self.layout.menu_height - self.layout.hint_rows * self.layout.bar_height
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

    /// One wheel step, mutating the page window. Returns whether the page
    /// actually turned, so a burst can be applied without redrawing between
    /// steps (see [`Menu::scroll_burst`]).
    pub(super) fn scroll_one(&mut self, delta: i32) -> bool {
        if delta < 0 {
            if self.paging.prev != 0 || self.selection.page_start.map(|c| c > 0).unwrap_or(false) {
                let page = paging::scroll_up(&self.selection, &self.paging)
                    .page_start
                    .unwrap_or(0);
                self.select_page(page);
                self.recalc_paging();
                return true;
            }
        } else if let Some(next) = self.paging.next {
            self.select_page(next);
            self.recalc_paging();
            return true;
        }
        false
    }

    /// Apply a burst of repaint-only events and redraw once.
    ///
    /// Every event is applied in order — so the end state is exactly what one
    /// redraw per event would have produced, list-end clamping and hover
    /// included — but only the final state is painted. A fast flick otherwise
    /// queues one full redraw per detent, and each one has to be drawn before
    /// the next event is even read, so the menu falls behind the wheel
    /// instead of landing on the page the user flicked to. Scrolling while
    /// moving the mouse interleaves the two kinds, so the burst has to cover
    /// both: honouring only the detents would still spend a repaint on every
    /// step of the gesture.
    ///
    /// The trailing motion is never dropped, only its intermediate frames
    /// are skipped, so the highlight still ends up under the resting pointer.
    pub(super) fn apply_repaint_batch(&mut self, events: &[BackendEvent]) -> Transition {
        let mut repaint = false;
        for event in events {
            let redrew = match event {
                BackendEvent::Scroll { delta } => self.scroll_one(*delta),
                BackendEvent::Motion { pos, .. } => {
                    !matches!(self.set_selection(*pos), Transition::Nop)
                }
                _ => continue,
            };
            repaint |= redrew;
        }
        if repaint {
            Transition::Redraw
        } else {
            Transition::Nop
        }
    }

    /// left-click: clear the input, or click an item/arrow/command cell.
    fn left_click(&mut self, mods: Modifiers, pos: Point) -> Transition {
        if self.over_hint(pos) {
            return Transition::Nop;
        }
        let header = self.header();
        let row_height = self.layout.bar_height;

        /* left-click on input: clear input,
         * NOTE: if there is no left-arrow the space for < is reserved so
         *       add that to the input width */
        let at_page_top =
            self.paging.prev == 0 || self.selection.page_start.map(|c| c == 0).unwrap_or(true);
        let arrow_pad = if at_page_top { header.left_arrow.w } else { 0 };
        let input_hit = !self.cfg.single_key
            && ((self.layout.lines <= 0
                && pos.x >= 0
                && pos.x <= header.input.right() + arrow_pad)
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
        if self.over_hint(pos) {
            return Transition::Nop;
        }
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
        self.confirm_match(clicked, AcceptMode::from_ctrl(mods.ctrl))
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
                return self.confirm_match(item, AcceptMode::from_ctrl(mods.ctrl));
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
