//! Keyboard handling: the keypress switch, alt-tab release handling and list
//! navigation helpers.
//!
//! Handlers return [`Transition`]; they never draw, print or exit on their
//! own. The C fallthrough tricks (a Ctrl key "handled" by re-entering the
//! main switch with a control-character text that then gets skipped) are
//! made explicit here.

use xkbcommon::xkb::keysyms as ks;

use super::measure::TextMeasurer;
use super::paging;
use super::transition::Transition;
use super::{AltTab, Menu};
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
        self.selection.selected = self.matcher.first_selectable_match();
        for _ in 0..number {
            self.select_next();
        }
        mods.ctrl = false;
        self.handle_return(mods)
    }

    /// The Return key branch, shared with select_number and the Ctrl-j/m
    /// remaps.
    fn handle_return(&mut self, mods: Modifiers) -> Transition {
        if self.selected_is_heading() {
            return Transition::Nop;
        }

        // puts((sel && !(state & ShiftMask & (!reject_no_match))) ? sel->text : text):
        // with reject_no_match off, shift+return prints the raw input instead
        // of the selection.
        let shift_suppresses = mods.shift && !self.cfg.reject_no_match;
        let out = if self.selected_output().is_some() && !shift_suppresses {
            self.selected_output().unwrap_or_default()
        } else {
            self.editor.text.clone()
        };
        self.confirm(&out, mods)
    }

    /// key_release — alt-tab release handling. Unlike every other confirm
    /// path this does not redraw afterwards (the C event loop called
    /// keyrelease outside the drawing branch).
    ///
    /// The confirm fires on the release of the Alt key itself, identified by
    /// keysym — not on the modifier bitmask. Wayland compositors deliver the
    /// modifiers event *before* the key release, so by the time Alt's own
    /// release arrives the cached modifier state no longer has alt set; the
    /// X11 quirk the C version relied on (a release reports the released
    /// key's own modifier as still held) does not carry over. This also
    /// deliberately narrows the C behaviour, which confirmed on *any* key
    /// released while Alt was held.
    pub(super) fn key_release(&mut self, sym: u32, mods: Modifiers) -> Transition {
        match self.alt_tab {
            AltTab::Off => Transition::Nop,
            AltTab::Tabbed => {
                /* the release ending a cycle is absorbed and re-arms */
                self.alt_tab = AltTab::Armed;
                Transition::Nop
            }
            AltTab::Armed => {
                if !is_alt_key(sym) || mods.shift || self.selected_is_heading() {
                    return Transition::Nop;
                }
                let out = self
                    .selected_output()
                    .unwrap_or_else(|| self.editor.text.clone());
                self.confirm(&out, mods)
            }
        }
    }

    /// The backend lost the keyboard (Wayland `wl_keyboard.leave`, X11
    /// FocusOut). A cycle in progress can no longer complete normally —
    /// its Tab release never arrives — so conclude it and re-arm.
    pub(super) fn keyboard_left(&mut self) {
        if self.alt_tab == AltTab::Tabbed {
            self.alt_tab = AltTab::Armed;
        }
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
            // Shift+Tab runs the alt-tab wrap-around selection and still
            // falls through with the original sym (the C switch)
            self.shift_key(sym);
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

    /// Shift+Tab in alt-tab mode: move the selection back one item, wrapping
    /// to the last. Any shifted key still falls through to the main switch.
    /// The C branch ran the wrap for *every* shifted key — typing a capital
    /// letter moved the selection too; only Tab is bound now.
    ///
    /// ISO_Left_Tab must be matched here: the standard xkb keymap defines
    /// `key <TAB> { [ Tab, ISO_Left_Tab ] }`, so with Shift held the backends'
    /// `key_get_one_sym` reports ISO_Left_Tab and KEY_Tab never arrives (the
    /// bare-Tab arm only fires on exotic keymaps).
    fn shift_key(&mut self, sym: u32) {
        if self.alt_tab == AltTab::Off {
            return;
        }
        if !matches!(sym, ks::KEY_ISO_Left_Tab | ks::KEY_Tab | ks::KEY_KP_Tab) {
            return;
        }
        if let Some(s) = self.selection.selected {
            if self.select_prev_position(s).is_none() {
                // wrap to the last item
                self.selection.selected = self.last_selectable_match();
                self.recalc_paging();
            } else {
                self.select_prev();
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
                if self.alt_tab != AltTab::Off {
                    self.alt_tab = AltTab::Off;
                }
            }
            s if s == ks::KEY_Tab => {
                /* Alt+Tab advances the selection and starts a cycle: its own
                 * release is absorbed and re-arms (repeat presses keep
                 * advancing). Without the mode it falls through to the main
                 * switch, where Tab completes the selection. The C version
                 * compounded the two paths, advancing the selection *and*
                 * completing the selected item's text. */
                if self.alt_tab != AltTab::Off {
                    if self.alt_tab == AltTab::Armed {
                        self.alt_tab = AltTab::Tabbed;
                    }

                    if let Some(s) = self.selection.selected {
                        if self.select_next_position(s).is_none() {
                            self.select_page(0);
                            self.recalc_paging();
                        } else {
                            self.select_next();
                        }
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
                    self.cfg.single_key,
                    self.layout.bar_height,
                );
                self.selection = paging::jump_to_end(
                    &self.matcher.items,
                    &self.matcher.matches,
                    &self.layout,
                    &mut m,
                );
            }
            self.selection.selected = self.last_selectable_match();
            Some(Transition::Redraw)
        } else if sym == ks::KEY_Escape {
            Some(Transition::Exit(ExitStatus::Failure))
        } else if sym == ks::KEY_Home || sym == ks::KEY_KP_Home {
            if (self.selection.selected.is_none() && self.matcher.matches.is_empty())
                || self.selection.selected == Some(0)
            {
                self.editor.cursor = 0;
            } else {
                self.select_page(0);
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
            self.select_page(next);
            self.recalc_paging();
            Transition::Redraw
        } else if sym == ks::KEY_Prior || sym == ks::KEY_KP_Prior {
            if self.selection.page_start.is_none() {
                return Transition::Nop;
            }
            self.select_page(self.paging.prev);
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
            if self.alt_tab == AltTab::Off {
                let Some(s) = self.selection.selected else {
                    return Transition::Nop;
                };
                let selected_text = self.matcher.text_of_match(s).to_string();
                self.editor.set_text(&selected_text);
                return self.do_match().at_least_redraw();
            }
            /* plain Tab in alt-tab mode marks a cycle, so the next release
             * is absorbed (the C main switch did the same) */
            if self.alt_tab == AltTab::Armed {
                self.alt_tab = AltTab::Tabbed;
            }
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
                if self.selection.page_start == Some(temp_selection) {
                    offscreen = true;
                }
                temp_selection -= 1;
            }
            let Some(temp_selection) = (0..=temp_selection)
                .rev()
                .find(|&pos| self.matcher.match_is_selectable(pos))
            else {
                return Transition::Nop;
            };
            self.selection.selected = Some(temp_selection);
            if offscreen {
                self.selection.page_start = Some(self.paging.prev);
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
            let Some(temp_selection) = (temp_selection..self.matcher.matches.len())
                .find(|&pos| self.matcher.match_is_selectable(pos))
            else {
                return Transition::Nop;
            };
            self.selection.selected = Some(temp_selection);
            if offscreen {
                self.selection.page_start = self.paging.next;
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

    fn last_selectable_match(&self) -> Option<usize> {
        (0..self.matcher.matches.len())
            .rev()
            .find(|&pos| self.matcher.match_is_selectable(pos))
    }

    fn select_prev_position(&self, from: usize) -> Option<usize> {
        (0..from)
            .rev()
            .find(|&pos| self.matcher.match_is_selectable(pos))
    }

    fn select_next_position(&self, from: usize) -> Option<usize> {
        (from + 1..self.matcher.matches.len()).find(|&pos| self.matcher.match_is_selectable(pos))
    }
}

/// The keysyms the standard xkb compat rules map to Mod1 (Alt) — the C
/// version detected them through the Mod1Mask bit instead.
fn is_alt_key(sym: u32) -> bool {
    matches!(
        sym,
        ks::KEY_Alt_L | ks::KEY_Alt_R | ks::KEY_Meta_L | ks::KEY_Meta_R
    )
}
