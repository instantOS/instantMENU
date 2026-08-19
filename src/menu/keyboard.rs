//! Keyboard handling: the keypress switch, alt-tab release handling and list
//! navigation helpers.

use xkbcommon::xkb::keysyms as ks;

use super::{Menu, TEXT_MAX};
use crate::backend::{CONTROL_MASK, MOD1_MASK, MOD4_MASK, SHIFT_MASK};
use crate::enums::{Direction, EditOp, ExitStatus, Side};

impl Menu {
    /// select_number — Ctrl-1..9 select the n-th item and hit Return.
    fn select_number(&mut self, number: usize, state: u32) {
        self.selected = self.current;
        for _ in 0..number {
            self.select_next();
        }
        let state = state ^ CONTROL_MASK;
        self.handle_return(state);
    }

    /// The Return key branch, shared with select_number and the Ctrl-j/m
    /// remaps.
    fn handle_return(&mut self, state: u32) {
        // non-selectable comment
        if self.selected_is_comment() {
            return;
        }

        // puts((sel && !(state & ShiftMask & (!reject_no_match))) ? sel->text : text):
        // with reject_no_match off, shift+return prints the raw input instead
        // of the selection.
        let shift_suppresses = (state & SHIFT_MASK != 0) && !self.cfg.reject_no_match;
        let out = if self.selected_text().is_some() && !shift_suppresses {
            self.selected_text().unwrap_or_default()
        } else {
            self.text.clone()
        };
        self.confirm(&out, state);
    }

    /// key_release — alt-tab release handling.
    pub(super) fn key_release(&mut self, sym: u32, state: u32) {
        let _ = sym;
        if !self.cfg.alt_tab {
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
            if self.selected_is_comment() {
                return;
            }
            let out = self.selected_text().unwrap_or_else(|| self.text.clone());
            self.confirm(&out, state);
        }
    }

    /// key_press — remap modifier prefixes, then run the unmodified key switch.
    pub(super) fn key_press(&mut self, sym: u32, state: u32, buf: &str) {
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
            self.draw_menu();
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
                self.insert(EditOp::Insert(".*"));
            }
            s if s == ks::KEY_v => {
                /* paste clipboard */
                self.request_paste(state);
                self.draw_menu();
                return None;
            }
            s if s == ks::KEY_k => {
                /* delete right */
                self.text.truncate(self.cursor);
                self.do_match();
            }
            s if s == ks::KEY_u => {
                /* delete left */
                self.insert(EditOp::Delete(self.cursor));
            }
            s if s == ks::KEY_w => {
                self.delete_word();
            }
            s if s == ks::KEY_y || s == ks::KEY_Y => {
                /* paste selection */
                self.request_paste(state);
                return None;
            }
            s if s == ks::KEY_Left || s == ks::KEY_KP_Left => {
                self.move_word_edge(Direction::Backward);
                self.draw_menu();
                return None;
            }
            s if s == ks::KEY_Right || s == ks::KEY_KP_Right => {
                self.move_word_edge(Direction::Forward);
                self.draw_menu();
                return None;
            }
            s if s == ks::KEY_Return || s == ks::KEY_KP_Enter => {
                // fall through to the main switch with Return
            }
            s if (ks::KEY_1..=ks::KEY_9).contains(&s) => {
                self.select_number((s - ks::KEY_1) as usize, state);
                self.draw_menu();
                return None;
            }
            s if s == ks::KEY_bracketleft => {
                self.finish(ExitStatus::Failure);
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
            self.insert(EditOp::Delete(self.cursor - target));
        }
    }

    /// Shift-prefixed keys (alt-tab wrap-around selection).
    fn shift_key(&mut self) {
        if self.cfg.alt_tab {
            if let Some(s) = self.selected {
                if s == 0 {
                    // wrap to the last item
                    self.selected = Some(self.matches.len() - 1);
                    self.calc_offsets();
                } else {
                    self.select_prev();
                }
            }
        }
    }

    /// Alt-prefixed keys: remap to navigation keys, or run an action. Returns
    /// the (possibly remapped) sym, or None when the key was handled here.
    fn mod1_key(&mut self, sym: u32) -> Option<u32> {
        let mut sym = sym;
        match sym {
            s if s == ks::KEY_F4 => self.finish(ExitStatus::Failure),
            s if s == ks::KEY_b => {
                self.move_word_edge(Direction::Backward);
                self.draw_menu();
                return None;
            }
            s if s == ks::KEY_f => {
                self.move_word_edge(Direction::Forward);
                self.draw_menu();
                return None;
            }
            s if s == ks::KEY_g => sym = ks::KEY_Home,
            s if s == ks::KEY_G => sym = ks::KEY_End,
            s if s == ks::KEY_h => sym = ks::KEY_Up,
            s if s == ks::KEY_j => sym = ks::KEY_Next,
            s if s == ks::KEY_k => sym = ks::KEY_Prior,
            s if s == ks::KEY_l => sym = ks::KEY_Down,
            s if s == ks::KEY_space => {
                if self.cfg.alt_tab {
                    self.tabbed = false;
                    self.cfg.alt_tab = false;
                }
            }
            s if s == ks::KEY_Tab => {
                self.tabbed = true;

                if let Some(s) = self.selected {
                    let last = self.matches.len().saturating_sub(1);
                    if s == last {
                        self.selected = Some(0);
                        self.current = Some(0);
                        self.calc_offsets();
                    } else {
                        self.select_next();
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
            self.finish(ExitStatus::Failure);
        }
    }

    /// Editing keys handled before list navigation. Returns Some(redraw) when
    /// the key was an editing key, None to defer to navigation/insertion.
    fn edit_key(&mut self, sym: u32) -> Option<bool> {
        if sym == ks::KEY_Delete || sym == ks::KEY_KP_Delete {
            if self.cursor >= self.text.len() {
                return Some(false);
            }
            self.cursor = self.next_rune(Direction::Forward);
            // fallthrough to BackSpace
            if self.cursor == 0 {
                return Some(false);
            }
            let next_rune_pos = self.next_rune(Direction::Backward);
            self.insert(EditOp::Delete(self.cursor - next_rune_pos));
            Some(true)
        } else if sym == ks::KEY_BackSpace {
            if self.cursor == 0 {
                return Some(false);
            }
            let next_rune_pos = self.next_rune(Direction::Backward);
            self.insert(EditOp::Delete(self.cursor - next_rune_pos));
            Some(true)
        } else if sym == ks::KEY_End || sym == ks::KEY_KP_End {
            if self.cursor < self.text.len() {
                self.cursor = self.text.len();
            } else if self.next.is_some() {
                self.jump_to_end();
            }
            self.selected = if self.matches.is_empty() {
                None
            } else {
                Some(self.matches.len() - 1)
            };
            Some(true)
        } else if sym == ks::KEY_Escape {
            self.finish(ExitStatus::Failure);
        } else if sym == ks::KEY_Home || sym == ks::KEY_KP_Home {
            if self.selected.is_none() && self.matches.is_empty() {
                self.cursor = 0;
            } else if self.selected == Some(0) {
                self.cursor = 0;
            } else {
                self.selected = Some(0);
                self.current = Some(0);
                self.calc_offsets();
            }
            Some(true)
        } else {
            None
        }
    }

    /// jump to end of list and position items in reverse (End key).
    fn jump_to_end(&mut self) {
        let last = self.matches.len().saturating_sub(1);
        self.current = Some(last);
        self.calc_offsets();
        self.current = Some(self.prev);
        self.calc_offsets();
        loop {
            if self.next.is_none() {
                break;
            }
            match self.current {
                Some(c) if c + 1 <= last => {
                    self.current = Some(c + 1);
                    self.calc_offsets();
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
            self.selected = Some(next);
            self.current = Some(next);
            self.calc_offsets();
        } else if sym == ks::KEY_Prior || sym == ks::KEY_KP_Prior {
            if self.current.is_none() {
                return false;
            }
            self.selected = Some(self.prev);
            self.current = Some(self.prev);
            self.calc_offsets();
        } else if sym == ks::KEY_Return || sym == ks::KEY_KP_Enter {
            self.handle_return(state);
        } else if sym == ks::KEY_Right || sym == ks::KEY_KP_Right {
            return self.move_right(state);
        } else if sym == ks::KEY_Down || sym == ks::KEY_KP_Down {
            self.nav_down();
        } else if sym == ks::KEY_Tab {
            if !self.cfg.alt_tab {
                let Some(s) = self.selected else { return false };
                let selected_text = self.items[self.matches[s]].text.clone();
                let take = selected_text.len().min(TEXT_MAX);
                self.text = selected_text[..take].to_string();
                self.cursor = take;
                self.do_match();
            } else {
                self.tabbed = true;
            }
        } else {
            // insert: composed string from the input method
            if let Some(first) = buf.bytes().next() {
                if !first.is_ascii_control() {
                    self.insert(EditOp::Insert(buf));
                }
            }
        }
        true
    }

    /// Left arrow: move a column left, move the cursor, or run the left
    /// command. Returns whether to redraw.
    fn move_left(&mut self, state: u32) -> bool {
        if self.cfg.columns > 1 {
            let Some(s) = self.selected else { return false };
            let mut temp_selection = s;
            let mut offscreen = false;
            for _ in 0..self.cfg.lines {
                if temp_selection == 0 {
                    return false;
                }
                if Some(temp_selection) == self.current {
                    offscreen = true;
                }
                temp_selection -= 1;
            }
            self.selected = Some(temp_selection);
            if offscreen {
                self.current = Some(self.prev);
                self.calc_offsets();
            }
        } else {
            if (state & (SHIFT_MASK | MOD4_MASK) != 0)
                && (self.cfg.left_command.is_some() || self.cfg.right_command.is_some())
            {
                self.trigger_command(Side::Left);
            } else {
                if self.cursor > 0
                    && (self.selected.is_none() || self.selected == Some(0) || self.cfg.lines > 0)
                {
                    self.cursor = self.next_rune(Direction::Backward);
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
            let Some(s) = self.selected else { return false };
            let mut temp_selection = s;
            let mut offscreen = false;
            for _ in 0..self.cfg.lines {
                if temp_selection + 1 >= self.matches.len() {
                    return false;
                }
                temp_selection += 1;
                if Some(temp_selection) == self.next {
                    offscreen = true;
                }
            }
            self.selected = Some(temp_selection);
            if offscreen {
                self.current = self.next;
                self.calc_offsets();
            }
        } else {
            if (state & (SHIFT_MASK | MOD4_MASK) != 0)
                && (self.cfg.right_command.is_some() || self.cfg.left_command.is_some())
            {
                self.trigger_command(Side::Right);
            } else if self.cursor < self.text.len() {
                self.cursor = self.next_rune(Direction::Forward);
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
        self.select_prev();
    }

    /// Down navigation shared by XK_Down and the XK_Right fallthrough.
    fn nav_down(&mut self) {
        self.select_next();
    }
}
