//! Selection ownership and viewport reconciliation. Callers choose a result,
//! page or direction; this module keeps eligibility, intent and visibility
//! consistent across keyboard, pointer, preselection and search updates.

use super::measure::TextMeasurer;
use super::{paging, Menu, Selection};
use crate::enums::Direction;

impl Menu {
    pub(super) fn clear_selection(&mut self) {
        self.selection = Selection::default();
        self.paging = paging::Paging::default();
        self.use_automatic_selection();
    }
    pub(super) fn select_identity(&mut self, id: usize) {
        if let Some(pos) = self
            .matcher
            .matches
            .iter()
            .position(|&index| self.matcher.items[index].arrival_index == id)
        {
            self.select_result(pos);
        }
    }
    pub(super) fn use_automatic_selection(&mut self) {
        self.selection_is_explicit = false;
    }

    pub(super) fn select_result(&mut self, pos: usize) -> bool {
        if !self.matcher.match_is_selectable(pos) {
            return false;
        }
        let changed = self.selection.selected != Some(pos);
        self.selection_is_explicit = true;
        self.selection.selected = Some(pos);
        if changed {
            self.ensure_selection_visible();
        }
        changed
    }

    pub(super) fn restore_selection(&mut self, keep: Option<paging::SelectionAnchor>) {
        let position_of = |id| {
            self.matcher
                .matches
                .iter()
                .position(|&idx| self.matcher.items[idx].arrival_index == id)
        };
        let selected = keep
            .and_then(|anchor| position_of(anchor.selected_id))
            .filter(|&pos| self.matcher.match_is_selectable(pos));
        let page = keep
            .and_then(|anchor| anchor.page_start_id)
            .and_then(position_of);
        self.selection_is_explicit = selected.is_some();
        self.selection = Selection {
            selected: selected.or_else(|| self.matcher.first_selectable_match()),
            page_start: page.or_else(|| (!self.matcher.matches.is_empty()).then_some(0)),
        };
        self.ensure_selection_visible();
    }

    pub(super) fn selection_anchor(&self) -> Option<paging::SelectionAnchor> {
        if !self.selection_is_explicit {
            return None;
        }
        let id_at = |position: usize| {
            self.matcher
                .matches
                .get(position)
                .map(|&index| self.matcher.items[index].arrival_index)
        };
        Some(paging::SelectionAnchor {
            selected_id: id_at(self.selection.selected?)?,
            page_start_id: self.selection.page_start.and_then(id_at),
        })
    }

    /// Move only when an eligible destination exists. Transient headings
    /// never become the final selection at either end of the list.
    pub(super) fn move_selection(&mut self, direction: Direction) {
        self.selection_is_explicit = true;
        let Some(from) = self.selection.selected else {
            if let Some(first) = self.matcher.first_selectable_match() {
                self.select_result(first);
            }
            return;
        };
        let target = match direction {
            Direction::Backward => (0..from)
                .rev()
                .find(|&pos| self.matcher.match_is_selectable(pos)),
            Direction::Forward => (from + 1..self.matcher.matches.len())
                .find(|&pos| self.matcher.match_is_selectable(pos)),
        };
        let Some(target) = target else {
            return;
        };
        while self.selection.selected != Some(target) {
            let (selection, turned) = match direction {
                Direction::Forward => {
                    paging::advance(&self.selection, self.matcher.matches.len(), &self.paging)
                }
                Direction::Backward => paging::retreat(&self.selection, &self.paging),
            };
            self.selection = selection;
            if turned {
                self.recalc_paging();
            }
        }
        self.ensure_selection_visible();
    }

    /// Start a page at `pos` and select its first selectable item. A trailing
    /// heading cannot trap selection: fall back to the preceding action.
    pub(in crate::menu) fn select_page(&mut self, pos: usize) {
        self.selection_is_explicit = true;
        let selected = (pos..self.matcher.matches.len())
            .find(|&candidate| self.matcher.match_is_selectable(candidate))
            .or_else(|| {
                (0..pos)
                    .rev()
                    .find(|&candidate| self.matcher.match_is_selectable(candidate))
            });
        self.selection = Selection {
            selected,
            page_start: (!self.matcher.matches.is_empty()).then_some(pos),
        };
        self.ensure_selection_visible();
    }

    pub(in crate::menu) fn recalc_paging(&mut self) {
        let mut m = TextMeasurer::new(
            &mut self.renderer,
            self.cfg.single_key,
            self.layout.bar_height,
        );
        self.paging = paging::calc_paging(
            &self.selection,
            &self.matcher.items,
            &self.matcher.matches,
            &self.layout,
            &mut m,
        );
    }

    /// Selection and viewport are one invariant: a selected result must be
    /// visible after both result replacement and a layout change.
    pub(super) fn ensure_selection_visible(&mut self) {
        self.recalc_paging();
        if let Some(pos) = self.selection.selected {
            let start = self.selection.page_start.unwrap_or(0);
            let end = self.paging.next.unwrap_or(self.matcher.matches.len());
            if pos < start || pos >= end {
                self.selection.page_start = Some(pos);
                self.recalc_paging();
            }
        }
    }

    /// --preselect: highlight the first selectable match whose output value
    /// equals the configured value, walking the selection there so paging
    /// turns pages exactly like repeated Down presses. The comparison is
    /// exact; without a match the selection stays where the match left it.
    /// While stdin streams in this runs deferred at EOF (`finalize_stream`),
    /// since the target may not have arrived yet.
    pub(in crate::menu) fn apply_preselect(&mut self) {
        let Some(target) = self.cfg.preselect.as_deref() else {
            return;
        };
        let Some(found) = self.matcher.matches.iter().position(|&index| {
            let item = &self.matcher.items[index];
            item.is_selectable() && item.output() == target
        }) else {
            return;
        };
        self.select_page(0);
        while self.selection.selected != Some(found) {
            self.move_selection(Direction::Forward);
        }
    }

    /* ── selection helpers ─────────────────────────────────────────────── */

    pub(in crate::menu) fn selected_text_ref(&self) -> Option<&str> {
        self.selection
            .selected
            .map(|pos| self.matcher.text_of_match(pos))
    }

    pub(super) fn selected_text(&self) -> Option<String> {
        self.selected_text_ref().map(str::to_owned)
    }

    pub(in crate::menu) fn selected_is_heading(&self) -> bool {
        self.selection
            .selected
            .is_some_and(|pos| !self.matcher.match_is_selectable(pos))
    }

    pub(super) fn last_selectable_match(&self) -> Option<usize> {
        (0..self.matcher.matches.len())
            .rev()
            .find(|&pos| self.matcher.match_is_selectable(pos))
    }

    pub(super) fn select_last_item(&mut self) -> bool {
        self.last_selectable_match()
            .is_some_and(|pos| self.select_result(pos))
    }

    pub(super) fn select_end(&mut self) {
        let mut measure = TextMeasurer::new(
            &mut self.renderer,
            self.cfg.single_key,
            self.layout.bar_height,
        );
        self.selection = paging::jump_to_end(
            &self.matcher.items,
            &self.matcher.matches,
            &self.layout,
            &mut measure,
        );
        if let Some(last) = self.last_selectable_match() {
            self.select_result(last);
        }
    }

    pub(super) fn cycle_selection(&mut self, direction: Direction) {
        let Some(current) = self.selection.selected else {
            return;
        };
        let at_boundary = match direction {
            Direction::Backward => !(0..current).any(|pos| self.matcher.match_is_selectable(pos)),
            Direction::Forward => !(current + 1..self.matcher.matches.len())
                .any(|pos| self.matcher.match_is_selectable(pos)),
        };
        if at_boundary {
            match direction {
                Direction::Backward => {
                    self.select_last_item();
                }
                Direction::Forward => self.select_page(0),
            }
        } else {
            self.move_selection(direction);
        }
    }

    pub(super) fn select_column(&mut self, direction: Direction) {
        let Some(current) = self.selection.selected else {
            return;
        };
        let destination = match direction {
            Direction::Backward => {
                current
                    .checked_sub(self.layout.lines as usize)
                    .and_then(|end| {
                        (0..=end)
                            .rev()
                            .find(|&pos| self.matcher.match_is_selectable(pos))
                    })
            }
            Direction::Forward => (current + self.layout.lines as usize
                ..self.matcher.matches.len())
                .find(|&pos| self.matcher.match_is_selectable(pos)),
        };
        if let Some(destination) = destination {
            if destination < self.selection.page_start.unwrap_or(0) {
                self.selection.page_start = Some(self.paging.prev);
            } else if self.paging.next.is_some_and(|next| destination >= next) {
                self.selection.page_start = self.paging.next;
            }
            self.select_result(destination);
        }
    }
}
