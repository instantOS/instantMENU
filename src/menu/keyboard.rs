//! Execute semantic keyboard actions. Key decoding and search dependencies
//! live in actions.rs; this module never reinterprets raw key bindings.

use super::accept::{AcceptMode, AcceptTarget};
use super::actions::KeyAction;
use super::{AltTab, Menu, Transition};
use crate::backend::{BackendEvent, Modifiers};
use crate::enums::{Direction, EditOp, ExitStatus};
use xkbcommon::xkb::keysyms as ks;

impl Menu {
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
                if self.search_pending() {
                    self.deferred_events
                        .push_back(BackendEvent::KeyRelease { sym, mods });
                    return Transition::Nop;
                }
                self.confirm_selection(AcceptMode::from_ctrl(mods.ctrl))
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

    pub(super) fn key_press(&mut self, sym: u32, mods: Modifiers, text: &str) -> Transition {
        let key = self.decode_key(sym, mods, text);
        let serial_edit = self.cfg.reject_no_match && self.stream_complete();
        if self.search_pending()
            && (key.wait_for_shortcut
                || (!matches!(&key.action, KeyAction::Cancel)
                    && (serial_edit || key.action.needs_results())))
        {
            self.deferred_events.push_back(BackendEvent::KeyPress {
                sym,
                mods,
                text: text.to_owned(),
            });
            return Transition::Nop;
        }
        if key.disable_alt_tab {
            self.alt_tab = AltTab::Off;
        }
        self.execute_key(key.action)
    }

    fn execute_key(&mut self, action: KeyAction<'_>) -> Transition {
        use KeyAction::*;
        match action {
            Ignore => Transition::Nop,
            Redraw => Transition::Redraw,
            Cancel => Transition::Exit(ExitStatus::Failure),
            Insert(text) => self.insert(EditOp::Insert(text)).at_least_redraw(),
            Backspace => self.delete_character(Direction::Backward),
            Delete => self.delete_character(Direction::Forward),
            TruncateRight => {
                self.editor.truncate_to_cursor();
                self.use_automatic_selection();
                self.do_match().at_least_redraw()
            }
            DeleteLeft => self
                .insert(EditOp::Delete(self.editor.cursor))
                .at_least_redraw(),
            DeleteWord => {
                let target = self.editor.word_delete_target(&self.cfg.word_delimiters);
                if target == self.editor.cursor {
                    return Transition::Redraw;
                }
                self.insert(EditOp::Delete(self.editor.cursor - target))
                    .at_least_redraw()
            }
            Word(direction) => {
                self.editor
                    .move_word_edge(direction, &self.cfg.word_delimiters);
                Transition::Redraw
            }
            Paste { mods, redraw } => {
                self.request_paste(mods);
                if redraw {
                    Transition::Redraw
                } else {
                    Transition::Nop
                }
            }
            Cursor(direction) => {
                self.editor.cursor = self.editor.next_rune(direction);
                Transition::Redraw
            }
            CursorHome => {
                self.editor.cursor = 0;
                Transition::Redraw
            }
            Home => {
                self.select_page(0);
                Transition::Redraw
            }
            End => {
                if self.editor.cursor < self.editor.text.len() {
                    self.editor.cursor = self.editor.text.len();
                }
                self.select_end();
                Transition::Redraw
            }
            Column(direction) => {
                self.select_column(direction);
                Transition::Redraw
            }
            Previous => {
                self.move_selection(Direction::Backward);
                Transition::Redraw
            }
            Next => {
                self.move_selection(Direction::Forward);
                Transition::Redraw
            }
            Page(Direction::Backward) => {
                if self.selection.page_start.is_none() {
                    return Transition::Nop;
                }
                self.select_page(self.paging.prev);
                Transition::Redraw
            }
            Page(Direction::Forward) => {
                if let Some(next) = self.paging.next {
                    self.select_page(next);
                    Transition::Redraw
                } else if self.select_last_item() {
                    Transition::Redraw
                } else {
                    Transition::Nop
                }
            }
            Command(side) => self.trigger_command(side),
            Confirm(mods) => self.handle_return(mods).at_least_redraw(),
            Complete => {
                let Some(text) = self.selected_text_ref().map(str::to_owned) else {
                    return Transition::Nop;
                };
                self.editor.set_text(&text);
                self.use_automatic_selection();
                self.do_match().at_least_redraw()
            }
            Cycle {
                direction,
                absorb_release,
            } => {
                if absorb_release && self.alt_tab == AltTab::Armed {
                    self.alt_tab = AltTab::Tabbed;
                }
                if let Some(direction) = direction {
                    self.cycle_selection(direction);
                }
                Transition::Redraw
            }
            Number { offset, mut mods } => {
                self.select_page(0);
                for _ in 0..offset {
                    self.move_selection(Direction::Forward);
                }
                mods.ctrl = false;
                self.handle_return(mods).at_least_redraw()
            }
            Binding(key) => self.accept(self.selected_target(), AcceptMode::Bound(key)),
            Entry(index) => self.accept(AcceptTarget::Item(index), AcceptMode::Exit),
        }
    }

    fn delete_character(&mut self, direction: Direction) -> Transition {
        match direction {
            Direction::Forward if self.editor.cursor >= self.editor.text.len() => {
                return Transition::Nop
            }
            Direction::Backward if self.editor.cursor == 0 => return Transition::Nop,
            Direction::Forward => self.editor.cursor = self.editor.next_rune(Direction::Forward),
            Direction::Backward => {}
        }
        let previous = self.editor.next_rune(Direction::Backward);
        self.insert(EditOp::Delete(self.editor.cursor - previous))
            .at_least_redraw()
    }

    fn handle_return(&mut self, mods: Modifiers) -> Transition {
        if self.selected_is_heading() {
            return Transition::Nop;
        }
        if mods.shift && !self.cfg.reject_no_match {
            self.accept(AcceptTarget::Input, AcceptMode::from_ctrl(mods.ctrl))
        } else {
            self.confirm_selection(AcceptMode::from_ctrl(mods.ctrl))
        }
    }
}

fn is_alt_key(sym: u32) -> bool {
    matches!(
        sym,
        ks::KEY_Alt_L | ks::KEY_Alt_R | ks::KEY_Meta_L | ks::KEY_Meta_R
    )
}
