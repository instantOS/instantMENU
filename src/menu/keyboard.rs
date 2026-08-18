//! Keyboard handling: the keypress switch, alt-tab release handling and list
//! navigation helpers.

use xkbcommon::xkb::keysyms as ks;

use super::{Menu, TEXT_MAX};
use crate::backend::{CONTROL_MASK, MOD1_MASK, MOD4_MASK, SHIFT_MASK};

fn sym_eq(a: u32, b: u32) -> bool {
    a == b
}

impl Menu {
    /// selectnumber — Ctrl-1..9 select the n-th item and hit Return.
    fn selectnumber(&mut self, number: usize, state: u32) {
        self.sel = self.curr;
        for _ in 0..number {
            if let Some(s) = self.sel {
                if s + 1 < self.matches.len() {
                    self.sel = Some(s + 1);
                    if self.sel == self.next {
                        self.curr = self.next;
                        self.calcoffsets();
                    }
                }
            }
        }
        let state = state ^ CONTROL_MASK;
        self.handle_return(state);
    }

    /// The Return key branch, shared with selectnumber and the Ctrl-j/m
    /// remaps.
    fn handle_return(&mut self, state: u32) {
        // non-selectable comment
        if let Some(text) = self.sel_text() {
            if text.starts_with('>') {
                return;
            }
        }
        self.animatesel();

        // puts((sel && !(state & ShiftMask & (!rejectnomatch))) ? sel->text : text):
        // with rejectnomatch off, shift+return prints the raw input instead
        // of the selection.
        let shift_suppresses = (state & SHIFT_MASK != 0) && !self.cfg.rejectnomatch;
        let print_sel = self.sel_text().is_some() && !shift_suppresses;
        let out = if print_sel {
            self.sel_text().unwrap_or_default()
        } else {
            self.text.clone()
        };
        self.println(&out);
        if state & CONTROL_MASK == 0 {
            self.finish(0);
        }
        if let Some(pos) = self.sel {
            self.items[self.matches[pos]].out = true;
        }
    }

    /// keyrelease — alt-tab release handling.
    pub(super) fn keyrelease(&mut self, sym: u32, state: u32) {
        let _ = sym;
        if !self.cfg.alttab {
            return;
        }
        if self.tabbed {
            self.tabbed = false;
            return;
        }

        if state & MOD1_MASK != 0 {
            if state & SHIFT_MASK != 0 {
                return;
            }
            if let Some(text) = self.sel_text() {
                if text.starts_with('>') {
                    return;
                }
            }
            let out = match self.sel_text() {
                Some(t) => t,
                None => self.text.clone(),
            };
            self.println(&out);
            if state & CONTROL_MASK == 0 {
                self.finish(0);
            }
            if let Some(pos) = self.sel {
                self.items[self.matches[pos]].out = true;
            }
        }
    }

    /// keypress — the big keyboard switch.
    pub(super) fn keypress(&mut self, sym: u32, state: u32, buf: &str) {
        let mut sym = sym;
        let mut state = state;

        if state & CONTROL_MASK != 0 {
            match sym {
                s if sym_eq(s, ks::KEY_a) => sym = ks::KEY_Home,
                s if sym_eq(s, ks::KEY_b) => sym = ks::KEY_Left,
                s if sym_eq(s, ks::KEY_c) => sym = ks::KEY_Escape,
                s if sym_eq(s, ks::KEY_d) => sym = ks::KEY_Delete,
                s if sym_eq(s, ks::KEY_e) => sym = ks::KEY_End,
                s if sym_eq(s, ks::KEY_f) => sym = ks::KEY_Right,
                s if sym_eq(s, ks::KEY_g) => sym = ks::KEY_Escape,
                s if sym_eq(s, ks::KEY_h) => sym = ks::KEY_BackSpace,
                s if sym_eq(s, ks::KEY_i) => sym = ks::KEY_Tab,
                s if sym_eq(s, ks::KEY_j)
                    || sym_eq(s, ks::KEY_J)
                    || sym_eq(s, ks::KEY_m)
                    || sym_eq(s, ks::KEY_M) =>
                {
                    sym = ks::KEY_Return;
                    state &= !CONTROL_MASK;
                }
                s if sym_eq(s, ks::KEY_n) => sym = ks::KEY_Down,
                s if sym_eq(s, ks::KEY_p) => sym = ks::KEY_Up,
                s if sym_eq(s, ks::KEY_s) => {
                    self.insert(Some(".*"), 2);
                }
                s if sym_eq(s, ks::KEY_v) => {
                    /* paste clipboard */
                    self.backend.request_selection(state & SHIFT_MASK != 0);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_k) => {
                    /* delete right */
                    self.text.truncate(self.cursor);
                    self.do_match();
                }
                s if sym_eq(s, ks::KEY_u) => {
                    /* delete left */
                    let cursor = self.cursor as i32;
                    self.insert(None, -cursor);
                }
                s if sym_eq(s, ks::KEY_w) => {
                    /* delete word */
                    while self.cursor > 0 && self.is_delimiter(self.nextrune(-1)) {
                        let nr = self.nextrune(-1);
                        self.insert(None, nr as i32 - self.cursor as i32);
                    }
                    while self.cursor > 0 && !self.is_delimiter(self.nextrune(-1)) {
                        let nr = self.nextrune(-1);
                        self.insert(None, nr as i32 - self.cursor as i32);
                    }
                }
                s if sym_eq(s, ks::KEY_y) || sym_eq(s, ks::KEY_Y) => {
                    /* paste selection */
                    self.backend.request_selection(state & SHIFT_MASK != 0);
                    return;
                }
                s if sym_eq(s, ks::KEY_Left) || sym_eq(s, ks::KEY_KP_Left) => {
                    self.movewordedge(-1);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_Right) || sym_eq(s, ks::KEY_KP_Right) => {
                    self.movewordedge(1);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_Return) || sym_eq(s, ks::KEY_KP_Enter) => {
                    // fall through to the main switch with Return
                }
                s if sym_eq(s, ks::KEY_1) => {
                    self.selectnumber(0, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_2) => {
                    self.selectnumber(1, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_3) => {
                    self.selectnumber(2, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_4) => {
                    self.selectnumber(3, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_5) => {
                    self.selectnumber(4, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_6) => {
                    self.selectnumber(5, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_7) => {
                    self.selectnumber(6, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_8) => {
                    self.selectnumber(7, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_9) => {
                    self.selectnumber(8, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_bracketleft) => {
                    self.finish(1);
                }
                _ => return,
            }
        } else if state & SHIFT_MASK != 0 {
            if self.cfg.alttab {
                if let Some(s) = self.sel {
                    if s == 0 {
                        // wrap to the last item
                        self.sel = Some(self.matches.len() - 1);
                        self.calcoffsets();
                    } else if s > 0 {
                        let ns = s - 1;
                        self.sel = Some(ns);
                        if Some(ns + 1) == self.curr {
                            self.curr = Some(self.prev);
                            self.calcoffsets();
                        }
                    }
                }
            }
        } else if state & MOD1_MASK != 0 {
            match sym {
                s if sym_eq(s, ks::KEY_F4) => self.finish(1),
                s if sym_eq(s, ks::KEY_b) => {
                    self.movewordedge(-1);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_f) => {
                    self.movewordedge(1);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_g) => sym = ks::KEY_Home,
                s if sym_eq(s, ks::KEY_G) => sym = ks::KEY_End,
                s if sym_eq(s, ks::KEY_h) => sym = ks::KEY_Up,
                s if sym_eq(s, ks::KEY_j) => sym = ks::KEY_Next,
                s if sym_eq(s, ks::KEY_k) => sym = ks::KEY_Prior,
                s if sym_eq(s, ks::KEY_l) => sym = ks::KEY_Down,
                s if sym_eq(s, ks::KEY_space) => {
                    if self.cfg.alttab {
                        self.tabbed = false;
                        self.cfg.alttab = false;
                    }
                }
                s if sym_eq(s, ks::KEY_Tab) => {
                    self.tabbed = true;

                    if let Some(s) = self.sel {
                        let last = self.matches.len().saturating_sub(1);
                        if s == last {
                            self.sel = Some(0);
                            self.curr = Some(0);
                            self.calcoffsets();
                        } else if s + 1 < self.matches.len() {
                            let ns = s + 1;
                            self.sel = Some(ns);
                            if Some(ns) == self.next {
                                self.curr = self.next;
                                self.calcoffsets();
                            }
                        }
                    }
                }
                _ => return,
            }
        } else if state & MOD4_MASK != 0 {
            if sym_eq(sym, ks::KEY_q) {
                self.finish(1);
            }
        }

        /* main switch */
        if sym_eq(sym, ks::KEY_Delete) || sym_eq(sym, ks::KEY_KP_Delete) {
            if self.cursor >= self.text.len() {
                return;
            }
            self.cursor = self.nextrune(1);
            // fallthrough to BackSpace
            if self.cursor == 0 {
                return;
            }
            let nr = self.nextrune(-1);
            self.insert(None, nr as i32 - self.cursor as i32);
        } else if sym_eq(sym, ks::KEY_BackSpace) {
            if self.cursor == 0 {
                return;
            }
            let nr = self.nextrune(-1);
            self.insert(None, nr as i32 - self.cursor as i32);
        } else if sym_eq(sym, ks::KEY_End) || sym_eq(sym, ks::KEY_KP_End) {
            if self.cursor < self.text.len() {
                self.cursor = self.text.len();
            } else if self.next.is_some() {
                /* jump to end of list and position items in reverse */
                let last = self.matches.len().saturating_sub(1);
                self.curr = Some(last);
                self.calcoffsets();
                self.curr = Some(self.prev);
                self.calcoffsets();
                loop {
                    if self.next.is_none() {
                        break;
                    }
                    match self.curr {
                        Some(c) if c + 1 <= last => {
                            self.curr = Some(c + 1);
                            self.calcoffsets();
                        }
                        _ => break,
                    }
                }
            }
            self.sel = if self.matches.is_empty() { None } else { Some(self.matches.len() - 1) };
        } else if sym_eq(sym, ks::KEY_Escape) {
            self.finish(1);
        } else if sym_eq(sym, ks::KEY_Home) || sym_eq(sym, ks::KEY_KP_Home) {
            if self.sel.is_none() && self.matches.is_empty() {
                self.cursor = 0;
            } else if self.sel == Some(0) {
                self.cursor = 0;
            } else {
                self.sel = Some(0);
                self.curr = Some(0);
                self.calcoffsets();
            }
        } else if sym_eq(sym, ks::KEY_Left) || sym_eq(sym, ks::KEY_KP_Left) {
            if self.cfg.columns > 1 {
                let Some(s) = self.sel else { return };
                let mut tmpsel = s;
                let mut offscreen = false;
                for _ in 0..self.cfg.lines {
                    if tmpsel == 0 {
                        return;
                    }
                    if Some(tmpsel) == self.curr {
                        offscreen = true;
                    }
                    tmpsel -= 1;
                }
                self.sel = Some(tmpsel);
                if offscreen {
                    self.curr = Some(self.prev);
                    self.calcoffsets();
                }
            } else {
                if (state & (SHIFT_MASK | MOD4_MASK) != 0)
                    && (self.cfg.leftcmd.is_some() || self.cfg.rightcmd.is_some())
                {
                    self.cmdtrigger(0);
                } else {
                    if self.cursor > 0
                        && (self.sel.is_none() || self.sel == Some(0) || self.cfg.lines > 0)
                    {
                        self.cursor = self.nextrune(-1);
                    } else if self.cfg.lines > 0 {
                        return;
                    } else {
                        // fallthrough to Up
                        self.nav_up();
                    }
                }
            }
        } else if sym_eq(sym, ks::KEY_Up) || sym_eq(sym, ks::KEY_KP_Up) {
            self.nav_up();
        } else if sym_eq(sym, ks::KEY_Next) || sym_eq(sym, ks::KEY_KP_Next) {
            let Some(next) = self.next else { return };
            self.sel = Some(next);
            self.curr = Some(next);
            self.calcoffsets();
        } else if sym_eq(sym, ks::KEY_Prior) || sym_eq(sym, ks::KEY_KP_Prior) {
            if self.curr.is_none() {
                return;
            }
            self.sel = Some(self.prev);
            self.curr = Some(self.prev);
            self.calcoffsets();
        } else if sym_eq(sym, ks::KEY_Return) || sym_eq(sym, ks::KEY_KP_Enter) {
            self.handle_return(state);
        } else if sym_eq(sym, ks::KEY_Right) || sym_eq(sym, ks::KEY_KP_Right) {
            if self.cfg.columns > 1 {
                let Some(s) = self.sel else { return };
                let mut tmpsel = s;
                let mut offscreen = false;
                for _ in 0..self.cfg.lines {
                    if tmpsel + 1 >= self.matches.len() {
                        return;
                    }
                    tmpsel += 1;
                    if Some(tmpsel) == self.next {
                        offscreen = true;
                    }
                }
                self.sel = Some(tmpsel);
                if offscreen {
                    self.curr = self.next;
                    self.calcoffsets();
                }
            } else {
                if (state & (SHIFT_MASK | MOD4_MASK) != 0)
                    && (self.cfg.rightcmd.is_some() || self.cfg.leftcmd.is_some())
                {
                    self.cmdtrigger(1);
                } else if self.cursor < self.text.len() {
                    self.cursor = self.nextrune(1);
                } else if self.cfg.lines > 0 {
                    return;
                } else {
                    // fallthrough to Down
                    self.nav_down();
                }
            }
        } else if sym_eq(sym, ks::KEY_Down) || sym_eq(sym, ks::KEY_KP_Down) {
            self.nav_down();
        } else if sym_eq(sym, ks::KEY_Tab) {
            if !self.cfg.alttab {
                let Some(s) = self.sel else { return };
                let sel_text = self.items[self.matches[s]].text.clone();
                let take = sel_text.len().min(TEXT_MAX);
                self.text = sel_text[..take].to_string();
                self.cursor = take;
                self.do_match();
            } else {
                self.tabbed = true;
            }
        } else {
            // insert: composed string from the input method
            if let Some(first) = buf.bytes().next() {
                if !first.is_ascii_control() {
                    self.insert(Some(buf), buf.len() as i32);
                }
            }
        }

        self.drawmenu();
    }

    /// Up navigation shared by XK_Up and the XK_Left fallthrough.
    fn nav_up(&mut self) {
        if let Some(s) = self.sel {
            if s > 0 {
                let ns = s - 1;
                if Some(ns + 1) == self.curr {
                    self.curr = Some(self.prev);
                    self.calcoffsets();
                }
                self.sel = Some(ns);
            }
        }
    }

    /// Down navigation shared by XK_Down and the XK_Right fallthrough.
    fn nav_down(&mut self) {
        if let Some(s) = self.sel {
            if s + 1 < self.matches.len() {
                let ns = s + 1;
                self.sel = Some(ns);
                if Some(ns) == self.next {
                    self.curr = self.next;
                    self.calcoffsets();
                }
            }
        }
    }
}
