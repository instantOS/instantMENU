//! Input pipeline shell glue: edits → smart-case → re-match → reject
//! revert (port of `insert()`), plus stdin item loading.

use std::io::Read;

use super::layout::GridShape;
use super::matcher::Item;
use super::transition::Transition;
use super::Menu;
use crate::config::Config;
use crate::enums::{EditOp, ExitStatus};

/// The items read from stdin plus the -l/-g values adjusted for their count.
pub struct StdinItems {
    pub items: Vec<Item>,
    pub grid: GridShape,
}

impl Menu {
    /// Port of insert(): edit the text, re-match, and revert the edit when
    /// `reject` is set and the edit emptied the match list. Returns the
    /// transition the edit caused (an instant/commented pick can end the
    /// menu mid-edit).
    pub(super) fn insert_op(&mut self, op: EditOp, reject: bool) -> Transition {
        // only insertion can overflow the TEXT_MAX budget
        let last = reject.then(|| (self.editor.text.clone(), self.editor.cursor));
        if !self.editor.apply(op) {
            return Transition::Nop;
        }
        self.matcher.note_uppercase(&self.editor.text);

        let t = self.do_match();
        if matches!(t, Transition::Nop) && reject && self.matcher.matches.is_empty() {
            /* revert to last text value if theres no match */
            let (text, cursor) = last.expect("reject_no_match snapshot");
            self.editor.text = text;
            self.editor.cursor = cursor;
            return self.do_match();
        }
        t
    }

    /// insert() with the configured reject_no_match behaviour.
    pub(super) fn insert(&mut self, op: EditOp) -> Transition {
        let reject = self.cfg.reject_no_match;
        self.insert_op(op, reject)
    }

    /// -it — initial input text, applied with reject_no_match temporarily
    /// disabled (port of the insert() call in the argv loop; items are empty
    /// at that point, so this only seeds text/cursor/smartcase).
    pub fn initial_text(&mut self, s: &str) -> Option<ExitStatus> {
        let t = self.insert_op(EditOp::Insert(s), false);
        self.settle(t)
    }
}

/// read_stdin — getline-per-line semantics: split on '\n' (a final chunk
/// without trailing newline is still an item), then strip ONE trailing
/// '\n' or '\t' byte and cut at the first NUL like strdup would. The
/// returned grid carries the -l/-g values adjusted for the item count.
pub fn read_stdin(cfg: &Config) -> StdinItems {
    if cfg.password || cfg.input_only {
        return StdinItems {
            items: Vec::new(),
            grid: GridShape {
                lines: 0,
                columns: cfg.columns,
            },
        };
    }

    /* read each line from stdin and add it to the item list */
    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_err() {
        /* keep whatever we got, like getline erroring mid-way */
    }
    let mut count: i32 = 0;
    let mut items = Vec::new();
    let input = input.strip_suffix(b"\n").unwrap_or(&input);
    for raw in input.split(|&b| b == b'\n').filter(|_| !input.is_empty()) {
        let raw = raw.strip_suffix(b"\t").unwrap_or(raw);
        let cut = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        let Ok(line) = std::str::from_utf8(&raw[..cut]) else {
            /* C strdup keeps invalid bytes; items are drawn as text so
             * drop-lossy lines are the closest safe equivalent */
            count += 1;
            continue;
        };
        items.push(Item::new(line));
        count += 1;
    }

    StdinItems {
        items,
        grid: adjusted_grid(cfg.lines, cfg.columns, count),
    }
}

/// The -l/-g adjustment read_stdin made in the C version: lines shrink to
/// fit the item count, then columns shrink to the widest actually-needed
/// grid (only when the grid is multi-column).
pub fn adjusted_grid(lines: i32, columns: i32, count: i32) -> GridShape {
    let i = count;
    let lines = lines.min(i / columns + (i % columns != 0) as i32);
    let columns = if columns != 1 && lines != 0 {
        (i / lines + (i % lines != 0) as i32).min(columns)
    } else {
        columns
    };
    GridShape { lines, columns }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_shrink_to_the_item_count() {
        assert_eq!(adjusted_grid(10, 1, 3), GridShape { lines: 3, columns: 1 });
        // a single column never shrinks further
        assert_eq!(adjusted_grid(10, 1, 0), GridShape { lines: 0, columns: 1 });
    }

    #[test]
    fn multi_column_grids_shrink_to_fit() {
        // 5 items, 2 columns → 3 rows needed → still 2 columns
        assert_eq!(adjusted_grid(10, 2, 5), GridShape { lines: 3, columns: 2 });
        // 4 items, 3 columns → 2 rows → 2 columns suffice
        assert_eq!(adjusted_grid(4, 3, 4), GridShape { lines: 2, columns: 2 });
        // no items: lines drop to 0, the -g value survives
        assert_eq!(adjusted_grid(5, 2, 0), GridShape { lines: 0, columns: 2 });
    }
}
