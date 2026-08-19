//! Selection state and page math — ports of `calcoffsets` plus the pure
//! halves of the navigation helpers.

use super::layout::Layout;
use super::matcher::Item;
use super::measure::Measure;

/// Where the list is pointing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Selection {
    /// selected position inside `matches`.
    pub selected: Option<usize>,
    /// first visible position (page start).
    pub current: Option<usize>,
}

impl Selection {
    /// After a re-match the list restarts at the top: current = 0 when there
    /// are matches, and the selection follows.
    pub fn from_match(match_count: usize) -> Self {
        let top = (match_count > 0).then_some(0);
        Selection {
            selected: top,
            current: top,
        }
    }
}

/// Which matches begin the next and previous pages (the C `next`/`prev`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Paging {
    /// first position of the next page, None on the last page.
    pub next: Option<usize>,
    /// first position of the previous page.
    pub prev: usize,
}

/// calcoffsets — which items begin the next and previous pages. Pure: takes
/// the item texts it measures through the [`Measure`] seam.
pub(super) fn calc_paging(
    sel: &Selection,
    items: &[Item],
    matches: &[usize],
    layout: &Layout,
    measure: &mut dyn Measure,
) -> Paging {
    let n = if layout.lines > 0 {
        layout.lines * layout.columns * layout.bar_height
    } else {
        let langle = measure.text_width("<");
        let rangle = measure.text_width(">");
        layout.menu_width - (layout.prompt_width + layout.input_width + langle + rangle)
    };

    /* calculate which items will begin the next page */
    let mut next = None;
    if let Some(start) = sel.current {
        let mut used: i32 = 0;
        for pos in start..matches.len() {
            used += if layout.lines > 0 {
                layout.bar_height
            } else {
                let text = items[matches[pos]].text.as_str();
                measure.text_width_clamp(text, n)
            };
            if used > n {
                next = Some(pos);
                break;
            }
        }
    }

    /* and the previous page */
    let start = sel.current.unwrap_or(0);
    let mut used: i32 = 0;
    let mut prev = start;
    for pos in (0..start).rev() {
        used += if layout.lines > 0 {
            layout.bar_height
        } else {
            let text = items[matches[pos]].text.as_str();
            measure.text_width_clamp(text, n)
        };
        if used > n {
            break;
        }
        prev = pos;
    }

    Paging { next, prev }
}

/// The pure half of select_next: advance the selection one match, reporting
/// whether the page turned (the caller recalculates paging when it did).
pub(super) fn advance(sel: &Selection, match_count: usize, paging: &Paging) -> (Selection, bool) {
    let mut sel = *sel;
    if let Some(s) = sel.selected {
        if s + 1 < match_count {
            let next_selection = s + 1;
            sel.selected = Some(next_selection);
            if paging.next == Some(next_selection) {
                sel.current = paging.next;
                return (sel, true);
            }
        }
    }
    (sel, false)
}

/// The pure half of select_prev: move the selection one match back.
pub(super) fn retreat(sel: &Selection, paging: &Paging) -> (Selection, bool) {
    let mut sel = *sel;
    if let Some(s) = sel.selected {
        if s > 0 {
            let next_selection = s - 1;
            if sel.current == Some(next_selection + 1) {
                sel.current = Some(paging.prev);
                sel.selected = Some(next_selection);
                return (sel, true);
            }
            sel.selected = Some(next_selection);
        }
    }
    (sel, false)
}
