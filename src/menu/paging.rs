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
        let langle = measure.cell_width("<");
        let rangle = measure.cell_width(">");
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
                measure.cell_width_clamp(text, n)
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
            measure.cell_width_clamp(text, n)
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

/// A selection parked on `pos`: both the highlight and the page window start
/// there (wheel down, PageDown and PageUp all land like this).
pub(super) fn at(pos: usize) -> Selection {
    Selection {
        selected: Some(pos),
        current: Some(pos),
    }
}

/// Wheel up: the page window moves back to `paging.prev` and the selection
/// follows the old page top (the C wheel handler's up half).
pub(super) fn scroll_up(sel: &Selection, paging: &Paging) -> Selection {
    Selection {
        selected: sel.current,
        current: Some(paging.prev),
    }
}

/// End key: position the page window so the last item is visible, walking
/// pages forward from the previous page boundary until the list end is in
/// view (the C calcoffsets dance). Pure: re-measures through the seam.
pub(super) fn jump_to_end(
    items: &[Item],
    matches: &[usize],
    layout: &Layout,
    measure: &mut dyn Measure,
) -> Selection {
    let Some(last) = matches.len().checked_sub(1) else {
        return Selection::default();
    };
    let mut sel = Selection {
        selected: Some(last),
        current: Some(last),
    };
    let mut paging = calc_paging(&sel, items, matches, layout, measure);
    sel.current = Some(paging.prev);
    loop {
        paging = calc_paging(&sel, items, matches, layout, measure);
        match paging.next {
            /* next is always past current; `<= last` means "current < last" */
            Some(next) if next <= last => sel.current = Some(next),
            _ => break,
        }
    }
    sel
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 10px per byte, so tests can compute budgets by hand.
    struct FakeMeasure;

    impl Measure for FakeMeasure {
        fn cell_width(&mut self, s: &str) -> i32 {
            10 * s.len() as i32
        }
        fn cell_width_clamp(&mut self, s: &str, n: i32) -> i32 {
            if n == 0 {
                return 0;
            }
            self.cell_width(s).min(n)
        }
    }

    fn layout(lines: i32, columns: i32, bar_height: i32, menu_width: i32) -> Layout {
        Layout {
            lines,
            columns,
            bar_height,
            menu_width,
            // horizontal budget: menu_width - (prompt + input + arrows)
            prompt_width: 0,
            input_width: 100,
            ..Layout::default()
        }
    }

    fn items(texts: &[&str]) -> Vec<Item> {
        texts.iter().map(|t| Item::new(*t)).collect()
    }

    #[test]
    fn selection_resets_to_top_on_match() {
        assert_eq!(Selection::from_match(0).selected, None);
        assert_eq!(Selection::from_match(0).current, None);
        assert_eq!(
            Selection::from_match(3),
            Selection {
                selected: Some(0),
                current: Some(0),
            }
        );
    }

    /// Vertical list: rows * columns * bar_height pixels per page.
    #[test]
    fn vertical_pages_hold_lines_times_columns_rows() {
        let lay = layout(3, 1, 30, 0);
        let its = items(&["a"; 7]);
        let matches: Vec<usize> = (0..7).collect();
        let sel = Selection {
            selected: Some(0),
            current: Some(0),
        };
        // 3 rows of 30px fit into the 90px page; the 4th is the next page
        let paging = calc_paging(&sel, &its, &matches, &lay, &mut FakeMeasure);
        assert_eq!(
            paging,
            Paging {
                next: Some(3),
                prev: 0
            }
        );

        // page 2 (indices 3..6)
        let sel = Selection {
            selected: Some(3),
            current: Some(3),
        };
        let paging = calc_paging(&sel, &its, &matches, &lay, &mut FakeMeasure);
        assert_eq!(
            paging,
            Paging {
                next: Some(6),
                prev: 0
            }
        );
    }

    /// Horizontal list: the page is a pixel budget minus prompt, input field
    /// and the two paging arrows (10px each with FakeMeasure).
    #[test]
    fn horizontal_pages_are_a_pixel_budget() {
        // n = 260 - (0 + 100 + 10 + 10) = 140 → three 40px items
        let lay = layout(0, 1, 30, 260);
        let its = items(&["aaaa", "bbbb", "cccc", "dddd"]);
        let matches: Vec<usize> = (0..4).collect();
        let sel = Selection {
            selected: Some(0),
            current: Some(0),
        };
        let paging = calc_paging(&sel, &its, &matches, &lay, &mut FakeMeasure);
        assert_eq!(
            paging,
            Paging {
                next: Some(3),
                prev: 0
            }
        );

        // wide items clamp to the budget: one per page
        let its = items(&["abcdefghijklmn", "abcdefghijklmn"]);
        let matches: Vec<usize> = (0..2).collect();
        let paging = calc_paging(&sel, &its, &matches, &lay, &mut FakeMeasure);
        assert_eq!(
            paging,
            Paging {
                next: Some(1),
                prev: 0
            }
        );
    }

    /// select_next turns the page exactly when it steps onto `next`.
    #[test]
    fn advance_turns_at_page_boundary() {
        let paging = Paging {
            next: Some(3),
            prev: 0,
        };

        let within = Selection {
            selected: Some(0),
            current: Some(0),
        };
        assert_eq!(
            advance(&within, 7, &paging),
            (
                Selection {
                    selected: Some(1),
                    current: Some(0)
                },
                false
            )
        );

        let boundary = Selection {
            selected: Some(2),
            current: Some(0),
        };
        assert_eq!(
            advance(&boundary, 7, &paging),
            (
                Selection {
                    selected: Some(3),
                    current: Some(3)
                },
                true
            )
        );

        // the last match does not advance further
        let end = Selection {
            selected: Some(6),
            current: Some(6),
        };
        assert_eq!(advance(&end, 7, &paging), (end, false));
    }

    /// select_prev turns the page when the selection is at the page top.
    #[test]
    fn retreat_turns_at_page_top() {
        let paging = Paging {
            next: Some(6),
            prev: 0,
        };

        let within = Selection {
            selected: Some(4),
            current: Some(3),
        };
        assert_eq!(
            retreat(&within, &paging),
            (
                Selection {
                    selected: Some(3),
                    current: Some(3)
                },
                false
            )
        );

        let top = Selection {
            selected: Some(3),
            current: Some(3),
        };
        assert_eq!(
            retreat(&top, &paging),
            (
                Selection {
                    selected: Some(2),
                    current: Some(0)
                },
                true
            )
        );
    }

    /// Wheel down / PageDown park both the selection and the page window on
    /// the given position.
    #[test]
    fn at_parks_selection_and_page() {
        assert_eq!(
            at(4),
            Selection {
                selected: Some(4),
                current: Some(4)
            }
        );
    }

    /// Wheel up moves the page window to `prev` and the selection follows
    /// the old page top (not `prev` itself).
    #[test]
    fn scroll_up_follows_the_old_page_top() {
        let paging = Paging {
            next: Some(6),
            prev: 0,
        };
        let sel = Selection {
            selected: Some(5),
            current: Some(3),
        };
        assert_eq!(
            scroll_up(&sel, &paging),
            Selection {
                selected: Some(3),
                current: Some(0)
            }
        );
    }

    /// End positions the page window so the last item is visible, walking
    /// forward one page boundary at a time.
    #[test]
    fn jump_to_end_walks_to_the_last_page() {
        // 7 items, 3 rows per page: pages start at 0, 3, 6. The last page
        // window starts at 6, so the walk ends with current = 6.
        let lay = layout(3, 1, 30, 0);
        let its = items(&["a"; 7]);
        let matches: Vec<usize> = (0..7).collect();
        let mut m = FakeMeasure;
        let sel = jump_to_end(&its, &matches, &lay, &mut m);
        assert_eq!(
            sel,
            Selection {
                selected: Some(6),
                current: Some(6)
            }
        );

        // a single partial page: the window stays at the top
        let matches: Vec<usize> = (0..2).collect();
        let sel = jump_to_end(&its, &matches, &lay, &mut m);
        assert_eq!(sel.current, Some(0));

        // no matches at all: default state
        let sel = jump_to_end(&its, &[], &lay, &mut m);
        assert_eq!(sel, Selection::default());
    }
}
