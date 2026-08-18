//! Keyboard handling: the keypress switch, alt-tab release handling and list
//! navigation helpers.

use xkbcommon::xkb::keysyms as ks;

use super::{Menu, TEXT_MAX};
use crate::backend::{CONTROL_MASK, MOD1_MASK, MOD4_MASK, SHIFT_MASK};

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

    /// keypress — remap modifier prefixes, then run the unmodified key switch.
    pub(super) fn keypress(&mut self, sym: u32, state: u32, buf: &str) {
        let mut sym = sym;
        let mut state = state;

        if state & CONTROL_MASK != 0 {
            match self.ctrl_key(sym, state) {
                Some((s, st)) => {
                    sym = s;
                    state = st;
                }
                None => return,
            }
        } else if state & SHIFT_MASK != 0 {
            self.shift_key();
        } else if state & MOD1_MASK != 0 {
            match self.mod1_key(sym) {
                Some(s) => sym = s,
                None => return,
            }
        } else if state & MOD4_MASK != 0 {
            self.mod4_key(sym);
        }

        if self.main_key(sym, state, buf) {
            self.drawmenu();
        }
    }

    /// Ctrl-prefixed keys: remap letters to editing keys, or run an action.
    /// Returns the (possibly remapped) (sym, state) to fall through to the
    /// main switch with, or None when the key was fully handled here.
    fn ctrl_key(&mut self, sym: u32, state: u32) -> Option<(u32, u32)> {
        let mut sym = sym;
        let mut state = state;
        match sym {
            s if s == ks::KEY_a => sym = ks::KEY_Home,
            s if s == ks::KEY_b => sym = ks::KEY_Left,
            s if s == ks::KEY_c => sym = ks::KEY_Escape,
            s if s == ks::KEY_d => sym = ks::KEY_Delete,
            s if s == ks::KEY_e => sym = ks::KEY_End,
            s if s == ks::KEY_f => sym = ks::KEY_Right,
            s if s == ks::KEY_g => sym = ks::KEY_Escape,
            s if s == ks::KEY_h => sym = ks::KEY_BackSpace,
            s if s == ks::KEY_i => sym = ks::KEY_Tab,
            s if s == ks::KEY_j
                || s == ks::KEY_J
                || s == ks::KEY_m
                || s == ks::KEY_M =>
            {
                sym = ks::KEY_Return;
                state &= !CONTROL_MASK;
            }
            s if s == ks::KEY_n => sym = ks::KEY_Down,
            s if s == ks::KEY_p => sym = ks::KEY_Up,
            s if s == ks::KEY_s => {
                self.insert(Some(".*"), 2);
            }
            s if s == ks::KEY_v => {
                /* paste clipboard */
                self.backend.request_selection(state & SHIFT_MASK != 0);
                self.drawmenu();
                return None;
            }
            s if s == ks::KEY_k => {
                /* delete right */
                self.text.truncate(self.cursor);
                self.do_match();
            }
            s if s == ks::KEY_u => {
                /* delete left */
                let cursor = self.cursor as i32;
                self.insert(None, -cursor);
            }
            s if s == ks::KEY_w => {
                self.delete_word();
            }
            s if s == ks::KEY_y || s == ks::KEY_Y => {
                /* paste selection */
                self.backend.request_selection(state & SHIFT_MASK != 0);
                return None;
            }
            s if s == ks::KEY_Left || s == ks::KEY_KP_Left => {
                self.movewordedge(-1);
                self.drawmenu();
                return None;
            }
            s if s == ks::KEY_Right || s == ks::KEY_KP_Right => {
                self.movewordedge(1);
                self.drawmenu();
                return None;
            }
            s if s == ks::KEY_Return || s == ks::KEY_KP_Enter => {
                // fall through to the main switch with Return
            }
            s if (ks::KEY_1..=ks::KEY_9).contains(&s) => {
                self.selectnumber((s - ks::KEY_1) as usize, state);
                self.drawmenu();
                return None;
            }
            s if s == ks::KEY_bracketleft => {
                self.finish(1);
            }
            _ => return None,
        }
        Some((sym, state))
    }

    /// delete the word to the left of the cursor (Ctrl-w).
    fn delete_word(&mut self) {
        let mut target = self.cursor;
        while target > 0 {
            let previous = self.text[..target]
                .char_indices()
                .next_back()
                .map(|(index, _)| index)
                .unwrap_or(0);
            if !self.is_delimiter(previous) {
                break;
            }
            target = previous;
        }
        while target > 0 {
            let previous = self.text[..target]
                .char_indices()
                .next_back()
                .map(|(index, _)| index)
                .unwrap_or(0);
            if self.is_delimiter(previous) {
                break;
            }
            target = previous;
        }
        if target != self.cursor {
            self.insert(None, target as i32 - self.cursor as i32);
        }
    }

    /// Shift-prefixed keys (alt-tab wrap-around selection).
    fn shift_key(&mut self) {
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
    }

    /// Alt-prefixed keys: remap to navigation keys, or run an action. Returns
    /// the (possibly remapped) sym, or None when the key was handled here.
    fn mod1_key(&mut self, sym: u32) -> Option<u32> {
        let mut sym = sym;
        match sym {
            s if s == ks::KEY_F4 => self.finish(1),
            s if s == ks::KEY_b => {
                self.movewordedge(-1);
                self.drawmenu();
                return None;
            }
            s if s == ks::KEY_f => {
                self.movewordedge(1);
                self.drawmenu();
                return None;
            }
            s if s == ks::KEY_g => sym = ks::KEY_Home,
            s if s == ks::KEY_G => sym = ks::KEY_End,
            s if s == ks::KEY_h => sym = ks::KEY_Up,
            s if s == ks::KEY_j => sym = ks::KEY_Next,
            s if s == ks::KEY_k => sym = ks::KEY_Prior,
            s if s == ks::KEY_l => sym = ks::KEY_Down,
            s if s == ks::KEY_space => {
                if self.cfg.alttab {
                    self.tabbed = false;
                    self.cfg.alttab = false;
                }
            }
            s if s == ks::KEY_Tab => {
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
            _ => return None,
        }
        Some(sym)
    }

    /// Mod4-prefixed keys (only Mod4-q is bound: quit).
    fn mod4_key(&mut self, sym: u32) {
        if sym == ks::KEY_q {
            self.finish(1);
        }
    }

    /// Editing keys handled before list navigation. Returns Some(redraw) when
    /// the key was an editing key, None to defer to navigation/insertion.
    fn edit_key(&mut self, sym: u32) -> Option<bool> {
        if sym == ks::KEY_Delete || sym == ks::KEY_KP_Delete {
            if self.cursor >= self.text.len() {
                return Some(false);
            }
            self.cursor = self.nextrune(1);
            // fallthrough to BackSpace
            if self.cursor == 0 {
                return Some(false);
            }
            let nr = self.nextrune(-1);
            self.insert(None, nr as i32 - self.cursor as i32);
            Some(true)
        } else if sym == ks::KEY_BackSpace {
            if self.cursor == 0 {
                return Some(false);
            }
            let nr = self.nextrune(-1);
            self.insert(None, nr as i32 - self.cursor as i32);
            Some(true)
        } else if sym == ks::KEY_End || sym == ks::KEY_KP_End {
            if self.cursor < self.text.len() {
                self.cursor = self.text.len();
            } else if self.next.is_some() {
                self.jump_to_end();
            }
            self.sel = if self.matches.is_empty() {
                None
            } else {
                Some(self.matches.len() - 1)
            };
            Some(true)
        } else if sym == ks::KEY_Escape {
            self.finish(1);
        } else if sym == ks::KEY_Home || sym == ks::KEY_KP_Home {
            if self.sel.is_none() && self.matches.is_empty() {
                self.cursor = 0;
            } else if self.sel == Some(0) {
                self.cursor = 0;
            } else {
                self.sel = Some(0);
                self.curr = Some(0);
                self.calcoffsets();
            }
            Some(true)
        } else {
            None
        }
    }

    /// jump to end of list and position items in reverse (End key).
    fn jump_to_end(&mut self) {
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
                Some(c) if c < last => {
                    self.curr = Some(c + 1);
                    self.calcoffsets();
                }
                _ => break,
            }
        }
    }

    /// The unmodified key switch. Returns whether the menu should be redrawn.
    fn main_key(&mut self, sym: u32, state: u32, buf: &str) -> bool {
        if let Some(redraw) = self.edit_key(sym) {
            return redraw;
        }
        self.nav_key(sym, state, buf)
    }

    /// List navigation, actions and raw insertion. Returns whether to redraw.
    fn nav_key(&mut self, sym: u32, state: u32, buf: &str) -> bool {
        if sym == ks::KEY_Left || sym == ks::KEY_KP_Left {
            return self.move_left(state);
        } else if sym == ks::KEY_Up || sym == ks::KEY_KP_Up {
            self.nav_up();
        } else if sym == ks::KEY_Next || sym == ks::KEY_KP_Next {
            let Some(next) = self.next else { return false };
            self.sel = Some(next);
            self.curr = Some(next);
            self.calcoffsets();
        } else if sym == ks::KEY_Prior || sym == ks::KEY_KP_Prior {
            if self.curr.is_none() {
                return false;
            }
            self.sel = Some(self.prev);
            self.curr = Some(self.prev);
            self.calcoffsets();
        } else if sym == ks::KEY_Return || sym == ks::KEY_KP_Enter {
            self.handle_return(state);
        } else if sym == ks::KEY_Right || sym == ks::KEY_KP_Right {
            return self.move_right(state);
        } else if sym == ks::KEY_Down || sym == ks::KEY_KP_Down {
            self.nav_down();
        } else if sym == ks::KEY_Tab {
            if !self.cfg.alttab {
                let Some(s) = self.sel else { return false };
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
        true
    }

    /// Left arrow: move a column left, move the cursor, or run the left
    /// command. Returns whether to redraw.
    fn move_left(&mut self, state: u32) -> bool {
        if self.cfg.columns > 1 {
            let Some(s) = self.sel else { return false };
            let mut tmpsel = s;
            let mut offscreen = false;
            for _ in 0..self.cfg.lines {
                if tmpsel == 0 {
                    return false;
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
                    return false;
                } else {
                    // fallthrough to Up
                    self.nav_up();
                }
            }
        }
        true
    }

    /// Right arrow: move a column right, move the cursor, or run the right
    /// command. Returns whether to redraw.
    fn move_right(&mut self, state: u32) -> bool {
        if self.cfg.columns > 1 {
            let Some(s) = self.sel else { return false };
            let mut tmpsel = s;
            let mut offscreen = false;
            for _ in 0..self.cfg.lines {
                if tmpsel + 1 >= self.matches.len() {
                    return false;
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
                return false;
            } else {
                // fallthrough to Down
                self.nav_down();
            }
        }
        true
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
