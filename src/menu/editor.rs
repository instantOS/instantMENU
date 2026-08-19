//! The input line editor: query text + cursor, with the pure editing
//! operations (ports of `insert`, `nextrune`, `movewordedge`).

use crate::enums::{Direction, EditOp};

/// sizeof text in the C version (BUFSIZ) minus the terminator.
pub(super) const TEXT_MAX: usize = 8192 - 1;

/// The menu's input line. Pure state: editing never matches, prints or
/// exits — the shell wires edits to re-matching.
#[derive(Debug, Clone, Default)]
pub(super) struct Editor {
    pub text: String,
    pub cursor: usize,
}

impl Editor {
    pub fn new() -> Self {
        Editor::default()
    }

    /// Port of insert()'s editing half. Returns false when an insert would
    /// overflow the TEXT_MAX budget (the C version truncated at BUFSIZ; the
    /// guard makes truncation unreachable, so the whole insert is rejected).
    pub fn apply(&mut self, op: EditOp) -> bool {
        match op {
            EditOp::Insert(s) => {
                if self.text.len() + s.len() > TEXT_MAX {
                    return false;
                }
                self.text.insert_str(self.cursor, s);
                self.cursor += s.len();
            }
            EditOp::Delete(n) => {
                let cut = (self.cursor as isize - n as isize).max(0) as usize;
                self.text.replace_range(cut..self.cursor, "");
                self.cursor = cut;
            }
        }
        true
    }

    /// Replace the whole text (Tab completion). Capped at TEXT_MAX on a char
    /// boundary — a raw byte cut of a multi-byte item would panic.
    pub fn set_text(&mut self, s: &str) {
        let take = s.floor_char_boundary(TEXT_MAX.min(s.len()));
        self.text = s[..take].to_string();
        self.cursor = take;
    }

    /// Ctrl-k: delete everything after the cursor. Bypasses the reject/no-match
    /// machinery, like the C version.
    pub fn truncate_to_cursor(&mut self) {
        self.text.truncate(self.cursor);
    }

    /// nextrune: location of the next utf8 rune in the given direction
    /// (std's floor/ceil_char_boundary, the optimized form of the C byte
    /// walk). Callers guard cursor > 0 / cursor < len, so the C quirk of
    /// returning cursor + 1 past the end is never observable.
    pub fn next_rune(&self, dir: Direction) -> usize {
        match dir {
            Direction::Forward => self.text.ceil_char_boundary(self.cursor + 1),
            Direction::Backward => self.text.floor_char_boundary(self.cursor - 1),
        }
    }

    pub fn is_delimiter(&self, pos: usize, delimiters: &str) -> bool {
        self.text
            .as_bytes()
            .get(pos)
            .map(|b| delimiters.as_bytes().contains(b))
            .unwrap_or(false)
    }

    /// movewordedge: move the cursor to the start/end of the current word.
    pub fn move_word_edge(&mut self, dir: Direction, delimiters: &str) {
        match dir {
            Direction::Backward => {
                /* move cursor to the start of the word */
                while self.cursor > 0 && self.is_delimiter(self.next_rune(Direction::Backward), delimiters) {
                    self.cursor = self.next_rune(Direction::Backward);
                }
                while self.cursor > 0 && !self.is_delimiter(self.next_rune(Direction::Backward), delimiters) {
                    self.cursor = self.next_rune(Direction::Backward);
                }
            }
            Direction::Forward => {
                /* move cursor to the end of the word */
                while self.cursor < self.text.len() && self.is_delimiter(self.cursor, delimiters) {
                    self.cursor = self.next_rune(Direction::Forward);
                }
                while self.cursor < self.text.len() && !self.is_delimiter(self.cursor, delimiters) {
                    self.cursor = self.next_rune(Direction::Forward);
                }
            }
        }
    }

    /// Ctrl-w: the byte offset where word deletion stops — the word plus
    /// trailing delimiters to the left of the cursor. Equal to the cursor
    /// when there is nothing to delete; the caller runs the actual deletion
    /// through the insert pipeline so matching stays in sync.
    pub fn word_delete_target(&self, delimiters: &str) -> usize {
        let mut target = self.cursor;
        while target > 0 {
            let previous = self.text[..target]
                .char_indices()
                .next_back()
                .map(|(index, _)| index)
                .unwrap_or(0);
            if !self.is_delimiter(previous, delimiters) {
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
            if self.is_delimiter(previous, delimiters) {
                break;
            }
            target = previous;
        }
        target
    }
}
