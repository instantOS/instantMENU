//! Mouse handling: hover selection, button presses and paste.

use super::Menu;
use crate::backend::{CONTROL_MASK, SHIFT_MASK};

impl Menu {
    /// x offset after the prompt (0 when there is no prompt).
    fn prompt_offset(&self) -> i32 {
        if let Some(prompt) = self.prompt() {
            if !prompt.is_empty() {
                return self.promptw;
            }
        }
        0
    }

    /// Width of the input field given the x offset after the prompt.
    fn input_width(&self, x: i32) -> i32 {
        if self.cfg.lines > 0 || self.matches.is_empty() {
            self.mw - x
        } else {
            self.inputw
        }
    }

    /// setselection — hover selection on motion.
    pub(super) fn setselection(&mut self, ev_x: i32, ev_y: i32) {
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

    /// Column/grid hover selection.
    fn hover_columns(&mut self, ev_x: i32, ev_y: i32, x: i32) {
        let y = 0;
        let h = self.bh;
        let mut i = 0;
        let mut init = false;
        let mut checky = y;
        let mut checkx = x;
        let colwidth = self.mw / self.cfg.columns;
        let mut pos = self.curr;
        while let Some(p) = pos {
            if Some(p) == self.next {
                break;
            }
            if i >= self.cfg.lines {
                i = 0;
                checkx += colwidth;
                checky = y;
            } else {
                if !init {
                    init = true;
                } else {
                    pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
                    if pos.is_none() {
                        break;
                    }
                }
                i += 1;
                checky += h;
            }
            // event in range
            if ev_y >= checky
                && ev_y <= (checky + h)
                && ev_x >= checkx
                && ev_x <= (checkx + colwidth)
            {
                if let Some(pp) = pos {
                    if self.sel == Some(pp) {
                        return;
                    }
                    self.sel = Some(pp);
                    self.drawmenu();
                }
                return;
            }
        }
    }

    /// Vertical list hover selection.
    fn hover_vertical(&mut self, ev_y: i32) {
        let mut y = 0;
        let h = self.bh;
        let mut pos = self.curr;
        while let Some(p) = pos {
            if Some(p) == self.next {
                break;
            }
            y += h;
            if ev_y >= y && ev_y <= (y + h) {
                if self.sel == Some(p) {
                    return;
                }
                self.sel = Some(p);
                self.drawmenu();
                return;
            }
            pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
        }
    }

    /// Horizontal list hover selection.
    fn hover_horizontal(&mut self, ev_x: i32, mut x: i32) {
        x += self.inputw;
        let mut w_arrow = self.textw("<");
        let mut pos = self.curr;
        while let Some(p) = pos {
            if Some(p) == self.next {
                break;
            }
            x += w_arrow;
            let item_text = self.items[self.matches[p]].text.clone();
            let rangle = self.textw(">");
            w_arrow = self.textw(&item_text).min(self.mw - x - rangle);
            if ev_x >= x && ev_x <= x + w_arrow {
                if self.sel == Some(p) {
                    return;
                }
                self.sel = Some(p);
                self.drawmenu();
                return;
            }
            pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
        }
    }

    /// buttonpress
    pub(super) fn buttonpress(&mut self, button: u8, state: u32, ev_x: i32, ev_y: i32) {
        /* right-click: exit */
        if button == 3 {
            std::process::exit(1);
        }

        let x = self.prompt_offset();
        let w = self.input_width(x);

        if button == 1 {
            self.left_click(state, ev_x, ev_y, x, w);
        }

        /* middle-mouse click: paste selection */
        if button == 2 {
            self.backend.request_selection(state & SHIFT_MASK != 0);
            self.drawmenu();
            return;
        }
        /* scroll up */
        if button == 4 && (self.prev != 0 || self.curr.map(|c| c > 0).unwrap_or(false)) {
            self.sel = self.curr;
            self.curr = Some(self.prev);
            self.calcoffsets();
            self.drawmenu();
            return;
        }
        /* scroll down */
        if button == 5 && self.next.is_some() {
            let next = self.next.unwrap();
            self.sel = Some(next);
            self.curr = Some(next);
            self.calcoffsets();
            self.drawmenu();
        }
    }

    /// left-click: clear the input, or click an item/arrow.
    fn left_click(&mut self, state: u32, ev_x: i32, ev_y: i32, x: i32, w: i32) {
        let y = 0;
        let h = self.bh;

        /* left-click on input: clear input,
         * NOTE: if there is no left-arrow the space for < is reserved so
         *       add that to the input width */
        let _arrowwidth = self.textw("");
        let input_hit = (self.cfg.lines <= 0
            && ev_x >= 0
            && ev_x
                <= x + w
                    + if self.prev == 0 || self.curr.map(|c| c == 0).unwrap_or(true) {
                        self.textw("<")
                    } else {
                        0
                    })
            || (self.cfg.lines > 0 && ev_y >= y && ev_y <= y + h);
        if input_hit {
            if self.cfg.leftcmd.is_some() && ev_x < self.textw("") {
                self.cmdtrigger(0);
            } else if ev_x > self.mw - self.textw("") {
                self.cmdtrigger(1);
            } else {
                let cursor = self.cursor as i32;
                self.insert(None, -cursor);
                self.drawmenu();
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
        let item = self.sel_text();
        if let Some(text) = &item {
            if text.starts_with('>') {
                return;
            }
        }
        self.animatesel();
        let out = item.unwrap_or_else(|| self.text.clone());
        self.println(&out);
        if state & CONTROL_MASK == 0 {
            std::process::exit(0);
        }
        if let Some(s) = self.sel {
            self.items[self.matches[s]].out = true;
        }
        self.drawmenu();
    }

    /// left-click on the horizontal list: arrows and items.
    fn horizontal_click(&mut self, state: u32, ev_x: i32, mut x: i32) {
        x += self.inputw;
        let mut w_arrow = self.textw("<");
        if self.prev != 0 || self.curr.map(|c| c > 0).unwrap_or(false) {
            if ev_x >= x && ev_x <= x + w_arrow {
                self.sel = self.curr;
                self.curr = Some(self.prev);
                self.calcoffsets();
                self.drawmenu();
                return;
            }
        }
        let mut pos = self.curr;
        while let Some(p) = pos {
            if Some(p) == self.next {
                break;
            }
            x += w_arrow;
            let item_text = self.items[self.matches[p]].text.clone();
            let rangle = self.textw(">");
            w_arrow = self.textw(&item_text).min(self.mw - x - rangle);
            if ev_x >= x && ev_x <= x + w_arrow {
                if let Some(text) = self.sel_text() {
                    if item_text.starts_with('>') && !text.is_empty() {
                        break;
                    }
                }
                self.animatesel();
                self.println(&item_text);
                if state & CONTROL_MASK == 0 {
                    std::process::exit(0);
                }
                self.sel = Some(p);
                self.items[self.matches[p]].out = true;
                self.drawmenu();
                return;
            }
            pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
        }
        /* left-click on right arrow */
        let rangle = self.textw(">");
        w_arrow = rangle;
        x = self.mw - w_arrow;
        if self.next.is_some() && ev_x >= x && ev_x <= x + w_arrow {
            let next = self.next.unwrap();
            self.sel = Some(next);
            self.curr = Some(next);
            self.calcoffsets();
            self.drawmenu();
            return;
        }
    }

    /// paste — insert selection text.
    pub(super) fn paste(&mut self, text: &str) {
        /* we have been given the current selection, now insert it into input */
        let line = text.split('\n').next().unwrap_or("");
        self.insert(Some(line), line.len() as i32);
        self.drawmenu();
    }
}
