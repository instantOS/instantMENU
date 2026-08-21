//! Keyboard handling: the keypress switch, alt-tab release handling and list
//! navigation helpers.
//!
//! Handlers return [`Transition`]; they never draw, print or exit on their
//! own. The C fallthrough tricks (a Ctrl key "handled" by re-entering the
//! main switch with a control-character text that then gets skipped) are
//! made explicit here.

use xkbcommon::xkb::keysyms as ks;

use super::measure::TextMeasurer;
use super::paging::{self, Selection};
use super::transition::Transition;
use super::Menu;
use crate::backend::Modifiers;
use crate::enums::{Direction, EditOp, ExitStatus, Side};

/// How a modifier-prefixed key continues into the main switch.
enum KeyPath {
    /// fall through with the (possibly remapped) sym/modifiers
    Continue(u32, Modifiers),
    /// fully handled; carry the transition to the event loop
    Done(Transition),
}

impl Menu {
    /// select_number — Ctrl-1..9 select the n-th item and hit Return.
    fn select_number(&mut self, number: usize, mut mods: Modifiers) -> Transition {
        self.selection.selected = self.selection.current;
        for _ in 0..number {
            self.select_next();
        }
        mods.ctrl = false;
        self.handle_return(mods)
    }

    /// The Return key branch, shared with select_number and the Ctrl-j/m
    /// remaps.
    fn handle_return(&mut self, mods: Modifiers) -> Transition {
        // non-selectable comment
        if self.selected_is_comment() {
            return Transition::Nop;
        }

        // puts((sel && !(state & ShiftMask & (!reject_no_match))) ? sel->text : text):
        // with reject_no_match off, shift+return prints the raw input instead
        // of the selection.
        let shift_suppresses = mods.shift && !self.cfg.reject_no_match;
        let out = if self.selected_text().is_some() && !shift_suppresses {
            self.selected_text().unwrap_or_default()
        } else {
            self.editor.text.clone()
        };
        self.confirm(&out, mods)
    }

    /// key_release — alt-tab release handling. Unlike every other confirm
    /// path this does not redraw afterwards (the C event loop called
    /// keyrelease outside the drawing branch).
    pub(super) fn key_release(&mut self, sym: u32, mods: Modifiers) -> Transition {
        let _ = sym;
        if !self.alt_tab {
            return Transition::Nop;
        }
        if self.tabbed {
            self.tabbed = false;
            return Transition::Nop;
        }

        if mods.alt {
            if mods.shift {
                return Transition::Nop;
            }
            if self.selected_is_comment() {
                return Transition::Nop;
            }
            let out = self
                .selected_text()
                .unwrap_or_else(|| self.editor.text.clone());
            return self.confirm(&out, mods);
        }
        Transition::Nop
    }

    /// key_press — remap modifier prefixes, then run the unmodified key
    /// switch.
    pub(super) fn key_press(&mut self, sym: u32, mods: Modifiers, buf: &str) -> Transition {
        let (sym, mods) = if mods.ctrl {
            match self.ctrl_key(sym, mods) {
                KeyPath::Continue(sym, mods) => (sym, mods),
                KeyPath::Done(t) => return t,
            }
        } else if mods.shift {
            // shift-prefixed keys run the alt-tab wrap-around selection and
            // still fall through with the original sym (the C switch)
            self.shift_key();
            (sym, mods)
        } else if mods.alt {
            match self.mod1_key(sym, mods) {
                KeyPath::Continue(sym, mods) => (sym, mods),
                KeyPath::Done(t) => return t,
            }
        } else if mods.logo {
            match self.mod4_key(sym, mods) {
                KeyPath::Continue(sym, mods) => (sym, mods),
                KeyPath::Done(t) => return t,
            }
        } else {
            (sym, mods)
        };

        self.main_key(sym, mods, buf)
    }

    /// Ctrl-prefixed keys: remap letters to editing keys, or run an action.
    fn ctrl_key(&mut self, sym: u32, mut mods: Modifiers) -> KeyPath {
        let mut sym = sym;
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
            s if s == ks::KEY_j || s == ks::KEY_J || s == ks::KEY_m || s == ks::KEY_M => {
                sym = ks::KEY_Return;
                mods.ctrl = false;
            }
            s if s == ks::KEY_n => sym = ks::KEY_Down,
            s if s == ks::KEY_p => sym = ks::KEY_Up,
            s if s == ks::KEY_s => {
                /* insert ".*"; the C fallthrough re-entered the switch with
                 * a control-character text, netting a bare redraw */
                let t = self.insert(EditOp::Insert(".*"));
                return KeyPath::Done(t.at_least_redraw());
            }
            s if s == ks::KEY_v => {
                /* paste clipboard */
                self.request_paste(mods);
                return KeyPath::Done(Transition::Redraw);
            }
            s if s == ks::KEY_k => {
                /* delete right */
                self.editor.truncate_to_cursor();
                let t = self.do_match();
                return KeyPath::Done(t.at_least_redraw());
            }
            s if s == ks::KEY_u => {
                /* delete left */
                let t = self.insert(EditOp::Delete(self.editor.cursor));
                return KeyPath::Done(t.at_least_redraw());
            }
            s if s == ks::KEY_w => {
                /* delete the word to the left of the cursor */
                let target = self.editor.word_delete_target(&self.cfg.word_delimiters);
                if target != self.editor.cursor {
                    let t = self.insert(EditOp::Delete(self.editor.cursor - target));
                    return KeyPath::Done(t.at_least_redraw());
                }
                return KeyPath::Done(Transition::Redraw);
            }
            s if s == ks::KEY_y || s == ks::KEY_Y => {
                /* paste selection */
                self.request_paste(mods);
                return KeyPath::Done(Transition::Nop);
            }
            s if s == ks::KEY_Left || s == ks::KEY_KP_Left => {
                self.editor
                    .move_word_edge(Direction::Backward, &self.cfg.word_delimiters);
                return KeyPath::Done(Transition::Redraw);
            }
            s if s == ks::KEY_Right || s == ks::KEY_KP_Right => {
                self.editor
                    .move_word_edge(Direction::Forward, &self.cfg.word_delimiters);
                return KeyPath::Done(Transition::Redraw);
            }
            s if s == ks::KEY_Return || s == ks::KEY_KP_Enter => {
                // fall through to the main switch with Return
            }
            s if (ks::KEY_1..=ks::KEY_9).contains(&s) => {
                let t = self.select_number((s - ks::KEY_1) as usize, mods);
                return KeyPath::Done(t.at_least_redraw());
            }
            s if s == ks::KEY_bracketleft => {
                return KeyPath::Done(Transition::Exit(ExitStatus::Failure));
            }
            _ => return KeyPath::Done(Transition::Nop),
        }
        KeyPath::Continue(sym, mods)
    }

    /// Shift-prefixed keys (alt-tab wrap-around selection). Any shifted key
    /// does this while alt-tab mode is on — the C branch never looked at
    /// the keysym — and still falls through to the main switch.
    fn shift_key(&mut self) {
        if self.alt_tab {
            if let Some(s) = self.selection.selected {
                if s == 0 {
                    // wrap to the last item
                    self.selection.selected = Some(self.matcher.matches.len() - 1);
                    self.recalc_paging();
                } else {
                    self.select_prev();
                }
            }
        }
    }

    /// Alt-prefixed keys: remap to navigation keys, or run an action.
    fn mod1_key(&mut self, sym: u32, mods: Modifiers) -> KeyPath {
        let mut sym = sym;
        match sym {
            s if s == ks::KEY_F4 => return KeyPath::Done(Transition::Exit(ExitStatus::Failure)),
            s if s == ks::KEY_b => {
                self.editor
                    .move_word_edge(Direction::Backward, &self.cfg.word_delimiters);
                return KeyPath::Done(Transition::Redraw);
            }
            s if s == ks::KEY_f => {
                self.editor
                    .move_word_edge(Direction::Forward, &self.cfg.word_delimiters);
                return KeyPath::Done(Transition::Redraw);
            }
            s if s == ks::KEY_g => sym = ks::KEY_Home,
            s if s == ks::KEY_G => sym = ks::KEY_End,
            s if s == ks::KEY_h => sym = ks::KEY_Up,
            s if s == ks::KEY_j => sym = ks::KEY_Next,
            s if s == ks::KEY_k => sym = ks::KEY_Prior,
            s if s == ks::KEY_l => sym = ks::KEY_Down,
            s if s == ks::KEY_space => {
                if self.alt_tab {
                    self.tabbed = false;
                    self.alt_tab = false;
                }
            }
            s if s == ks::KEY_Tab => {
                self.tabbed = true;

                if let Some(s) = self.selection.selected {
                    let last = self.matcher.matches.len().saturating_sub(1);
                    if s == last {
                        self.selection = Selection::from_match(self.matcher.matches.len());
                        self.recalc_paging();
                    } else {
                        self.select_next();
                    }
                }
            }
            _ => return KeyPath::Done(Transition::Nop),
        }
        KeyPath::Continue(sym, mods)
    }

    /// Logo-prefixed keys: only logo+q is bound (quit); anything else falls
    /// through to the main switch like the C version.
    fn mod4_key(&mut self, sym: u32, mods: Modifiers) -> KeyPath {
        if sym == ks::KEY_q {
            return KeyPath::Done(Transition::Exit(ExitStatus::Failure));
        }
        KeyPath::Continue(sym, mods)
    }

    /// Editing keys handled before list navigation.
    fn edit_key(&mut self, sym: u32) -> Option<Transition> {
        if sym == ks::KEY_Delete || sym == ks::KEY_KP_Delete {
            if self.editor.cursor >= self.editor.text.len() {
                return Some(Transition::Nop);
            }
            self.editor.cursor = self.editor.next_rune(Direction::Forward);
            // fallthrough to BackSpace
            if self.editor.cursor == 0 {
                return Some(Transition::Nop);
            }
            let next_rune_pos = self.editor.next_rune(Direction::Backward);
            Some(
                self.insert(EditOp::Delete(self.editor.cursor - next_rune_pos))
                    .at_least_redraw(),
            )
        } else if sym == ks::KEY_BackSpace {
            if self.editor.cursor == 0 {
                return Some(Transition::Nop);
            }
            let next_rune_pos = self.editor.next_rune(Direction::Backward);
            Some(
                self.insert(EditOp::Delete(self.editor.cursor - next_rune_pos))
                    .at_least_redraw(),
            )
        } else if sym == ks::KEY_End || sym == ks::KEY_KP_End {
            if self.editor.cursor < self.editor.text.len() {
                self.editor.cursor = self.editor.text.len();
            } else if self.paging.next.is_some() {
                let mut m = TextMeasurer::new(
                    &mut self.renderer,
                    self.cfg.commented,
                    self.layout.bar_height,
                );
                self.selection = paging::jump_to_end(
                    &self.matcher.items,
                    &self.matcher.matches,
                    &self.layout,
                    &mut m,
                );
            }
            self.selection.selected = if self.matcher.matches.is_empty() {
                None
            } else {
                Some(self.matcher.matches.len() - 1)
            };
            Some(Transition::Redraw)
        } else if sym == ks::KEY_Escape {
            Some(Transition::Exit(ExitStatus::Failure))
        } else if sym == ks::KEY_Home || sym == ks::KEY_KP_Home {
            if (self.selection.selected.is_none() && self.matcher.matches.is_empty())
                || self.selection.selected == Some(0)
            {
                self.editor.cursor = 0;
            } else {
                self.selection = Selection::from_match(self.matcher.matches.len());
                self.recalc_paging();
            }
            Some(Transition::Redraw)
        } else {
            None
        }
    }

    /// The unmodified key switch.
    fn main_key(&mut self, sym: u32, mods: Modifiers, buf: &str) -> Transition {
        if let Some(t) = self.edit_key(sym) {
            return t;
        }
        self.nav_key(sym, mods, buf)
    }

    /// List navigation, actions and raw insertion.
    fn nav_key(&mut self, sym: u32, mods: Modifiers, buf: &str) -> Transition {
        if sym == ks::KEY_Left || sym == ks::KEY_KP_Left {
            self.move_left(mods)
        } else if sym == ks::KEY_Up || sym == ks::KEY_KP_Up {
            self.nav_up();
            Transition::Redraw
        } else if sym == ks::KEY_Next || sym == ks::KEY_KP_Next {
            let Some(next) = self.paging.next else {
                return Transition::Nop;
            };
            self.selection = paging::at(next);
            self.recalc_paging();
            Transition::Redraw
        } else if sym == ks::KEY_Prior || sym == ks::KEY_KP_Prior {
            if self.selection.current.is_none() {
                return Transition::Nop;
            }
            self.selection = paging::at(self.paging.prev);
            self.recalc_paging();
            Transition::Redraw
        } else if sym == ks::KEY_Return || sym == ks::KEY_KP_Enter {
            self.handle_return(mods).at_least_redraw()
        } else if sym == ks::KEY_Right || sym == ks::KEY_KP_Right {
            self.move_right(mods)
        } else if sym == ks::KEY_Down || sym == ks::KEY_KP_Down {
            self.nav_down();
            Transition::Redraw
        } else if sym == ks::KEY_Tab {
            if !self.alt_tab {
                let Some(s) = self.selection.selected else {
                    return Transition::Nop;
                };
                let selected_text = self.matcher.text_of_match(s).to_string();
                self.editor.set_text(&selected_text);
                return self.do_match().at_least_redraw();
            }
            self.tabbed = true;
            Transition::Redraw
        } else if sym == ks::KEY_space && self.cfg.space_confirm {
            self.handle_return(mods).at_least_redraw()
        } else {
            // insert: composed string from the input method
            if let Some(first) = buf.bytes().next() {
                if !first.is_ascii_control() {
                    return self.insert(EditOp::Insert(buf)).at_least_redraw();
                }
            }
            Transition::Redraw
        }
    }

    /// Left arrow: move a column left, move the cursor, or run the left
    /// command.
    fn move_left(&mut self, mods: Modifiers) -> Transition {
        if self.layout.columns > 1 {
            let Some(s) = self.selection.selected else {
                return Transition::Nop;
            };
            let mut temp_selection = s;
            let mut offscreen = false;
            for _ in 0..self.layout.lines {
                if temp_selection == 0 {
                    return Transition::Nop;
                }
                if self.selection.current == Some(temp_selection) {
                    offscreen = true;
                }
                temp_selection -= 1;
            }
            self.selection.selected = Some(temp_selection);
            if offscreen {
                self.selection.current = Some(self.paging.prev);
                self.recalc_paging();
            }
            Transition::Redraw
        } else {
            if (mods.shift || mods.logo)
                && (self.cfg.left_command.is_some() || self.cfg.right_command.is_some())
            {
                return self.trigger_command(Side::Left);
            }
            if self.editor.cursor > 0
                && (self.selection.selected.is_none()
                    || self.selection.selected == Some(0)
                    || self.layout.lines > 0)
            {
                self.editor.cursor = self.editor.next_rune(Direction::Backward);
            } else if self.layout.lines > 0 {
                return Transition::Nop;
            } else {
                // fallthrough to Up
                self.nav_up();
            }
            Transition::Redraw
        }
    }

    /// Right arrow: move a column right, move the cursor, or run the right
    /// command.
    fn move_right(&mut self, mods: Modifiers) -> Transition {
        if self.layout.columns > 1 {
            let Some(s) = self.selection.selected else {
                return Transition::Nop;
            };
            let mut temp_selection = s;
            let mut offscreen = false;
            for _ in 0..self.layout.lines {
                if temp_selection + 1 >= self.matcher.matches.len() {
                    return Transition::Nop;
                }
                temp_selection += 1;
                if self.paging.next == Some(temp_selection) {
                    offscreen = true;
                }
            }
            self.selection.selected = Some(temp_selection);
            if offscreen {
                self.selection.current = self.paging.next;
                self.recalc_paging();
            }
            Transition::Redraw
        } else {
            if (mods.shift || mods.logo)
                && (self.cfg.right_command.is_some() || self.cfg.left_command.is_some())
            {
                return self.trigger_command(Side::Right);
            }
            if self.editor.cursor < self.editor.text.len() {
                self.editor.cursor = self.editor.next_rune(Direction::Forward);
            } else if self.layout.lines > 0 {
                return Transition::Nop;
            } else {
                // fallthrough to Down
                self.nav_down();
            }
            Transition::Redraw
        }
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
