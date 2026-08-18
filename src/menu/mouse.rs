//! Mouse handling: hover selection, button presses and paste.

use super::Menu;
use crate::backend::{CONTROL_MASK, SHIFT_MASK};

impl Menu {
    /// setselection — hover selection on motion.
    pub(super) fn setselection(&mut self, ev_x: i32, ev_y: i32) {
        let mut x = 0;
        let mut y = 0;
        let h = self.bh;
        let w;

        if let Some(prompt) = self.prompt() {
            if !prompt.is_empty() {
                x += self.promptw;
            }
        }

        /* input field */
        w = if self.cfg.lines > 0 || self.matches.is_empty() {
            self.mw - x
        } else {
            self.inputw
        };

        if self.cfg.lines > 0 {
            /* (C re-assigns w = mw - x here; already covered above) */
            if self.cfg.columns > 0 {
                // check mouse hover for columns (ported literally)
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
            } else {
                /* vertical list: (ctrl)left-click on item */
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
        } else if !self.matches.is_empty() {
            /* left-click on left arrow */
            x += self.inputw;
            let mut w_arrow = self.textw("<");
            /* horizontal list: (ctrl)left-click on item */
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
            /* left-click on right arrow */
            let _ = w;
        }
    }

    /// buttonpress
    pub(super) fn buttonpress(&mut self, button: u8, state: u32, ev_x: i32, ev_y: i32) {
        let mut x = 0;
        let y = 0;
        let h = self.bh;
        let w;

        /* right-click: exit */
        if button == 3 {
            std::process::exit(1);
        }

        if let Some(prompt) = self.prompt() {
            if !prompt.is_empty() {
                x += self.promptw;
            }
        }

        /* input field */
        w = if self.cfg.lines > 0 || self.matches.is_empty() {
            self.mw - x
        } else {
            self.inputw
        };

        /* left-click on input: clear input,
         * NOTE: if there is no left-arrow the space for < is reserved so
         *       add that to the input width */
        if button == 1 {
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
                /* vertical list: (ctrl)left-click on item
                 * (C sets w = mw - x here but never reads it) */
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
                return;
            } else if !self.matches.is_empty() {
                /* left-click on left arrow */
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
                /* horizontal list: (ctrl)left-click on item */
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

    /// paste — insert selection text.
    pub(super) fn paste(&mut self, text: &str) {
        /* we have been given the current selection, now insert it into input */
        let line = text.split('\n').next().unwrap_or("");
        self.insert(Some(line), line.len() as i32);
        self.drawmenu();
    }
}
