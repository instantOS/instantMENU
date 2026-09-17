//! Item acceptance owns output resolution. Event handlers choose a target and
//! completion mode, never construct output strings from display labels.

use super::{Menu, Transition};

pub(super) enum AcceptTarget {
    /// Index in the item corpus, not a position in the filtered match list.
    Item(usize),
    Input,
    /// A bound action is allowed without a selected item.
    None,
}

pub(super) enum AcceptMode {
    Exit,
    KeepOpen,
    Bound(String),
}

impl AcceptMode {
    pub(super) fn from_ctrl(ctrl: bool) -> Self {
        if ctrl {
            Self::KeepOpen
        } else {
            Self::Exit
        }
    }
}

impl Menu {
    /// The single item-to-output boundary, also used before window setup by
    /// auto-confirm and single-key activation. No rendering or I/O here.
    pub(super) fn accept(&mut self, target: AcceptTarget, mode: AcceptMode) -> Transition {
        let value = match target {
            AcceptTarget::Item(index) => {
                let Some(item) = self
                    .matcher
                    .items
                    .get_mut(index)
                    .filter(|i| i.is_selectable())
                else {
                    return Transition::Nop;
                };
                item.already_output = true;
                Some(item.output().to_owned())
            }
            AcceptTarget::Input => Some(self.editor.text.clone()),
            AcceptTarget::None => None,
        };
        match (mode, value) {
            (AcceptMode::Bound(key), value) => Transition::BoundAccept(key, value),
            (AcceptMode::Exit, Some(value)) => Transition::PrintAndExit(value),
            (AcceptMode::KeepOpen, Some(value)) => Transition::Print(value),
            (_, None) => Transition::Nop,
        }
    }

    pub(super) fn selected_target(&self) -> AcceptTarget {
        self.selection
            .selected
            .and_then(|pos| self.matcher.matches.get(pos).copied())
            .map_or(AcceptTarget::None, AcceptTarget::Item)
    }

    /// Accept the clicked/current match, not whatever happened to be hovered.
    pub(super) fn confirm_match(&mut self, pos: usize, mode: AcceptMode) -> Transition {
        if !self.matcher.match_is_selectable(pos) {
            return Transition::Nop;
        }
        self.selection.selected = Some(pos);
        self.animate_selection();
        self.accept(AcceptTarget::Item(self.matcher.matches[pos]), mode)
    }

    /// Keyboard acceptance retains dmenu's input fallback when no item matches.
    pub(super) fn confirm_selection(&mut self, mode: AcceptMode) -> Transition {
        match self.selection.selected {
            Some(pos) => self.confirm_match(pos, mode),
            None => self.accept(AcceptTarget::Input, mode),
        }
    }
}
