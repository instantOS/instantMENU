//! Input handling: text insertion/editing and stdin item loading.

use std::io::Read;

use super::{Item, Menu, TEXT_MAX};
use crate::enums::{Direction, EditOp};

impl Menu {
    /// Port of insert(): EditOp::Insert(s) inserts `s` at the cursor (capped
    /// at the TEXT_MAX budget on a char boundary), EditOp::Delete(n) deletes
    /// `n` bytes before the cursor (clamped at the text start).
    pub(super) fn insert(&mut self, op: EditOp) {
        // only insertion can overflow the TEXT_MAX budget
        if let EditOp::Insert(s) = op {
            if self.text.len() + s.len() > TEXT_MAX {
                return;
            }
        }

        let last = self
            .cfg
            .reject_no_match
            .then(|| (self.text.clone(), self.cursor));

        match op {
            EditOp::Insert(s) => {
                // cut at the TEXT_MAX budget on a char boundary; a raw byte
                // cut could land inside a multi-byte char and panic on
                // insert_str
                let byte_len = s.floor_char_boundary(s.len().min(TEXT_MAX - self.text.len()));
                self.text.insert_str(self.cursor, &s[..byte_len]);
                self.cursor += byte_len;

                if self.cfg.smart_case {
                    let has_upper = self.text.bytes().any(|b| b.is_ascii_uppercase());
                    if has_upper {
                        self.cfg.smart_case = false;
                        self.insensitive = false;
                    }
                }
            }
            EditOp::Delete(n) => {
                let cut = (self.cursor as isize - n as isize).max(0) as usize;
                self.text.replace_range(cut..self.cursor, "");
                self.cursor = cut;
            }
        }

        self.do_match();

        if self.matches.is_empty() && self.cfg.reject_no_match {
            /* revert to last text value if theres no match */
            let (text, cursor) = last.expect("reject_no_match snapshot");
            self.text = text;
            self.cursor = cursor;
            self.do_match();
        }
    }

    /// nextrune: location of the next utf8 rune in the given direction
    /// (std's floor/ceil_char_boundary, the optimized form of the C byte
    /// walk). Callers guard cursor > 0 / cursor < len, so the C quirk of
    /// returning cursor + 1 past the end is never observable.
    pub(super) fn next_rune(&self, dir: Direction) -> usize {
        match dir {
            Direction::Forward => self.text.ceil_char_boundary(self.cursor + 1),
            Direction::Backward => self.text.floor_char_boundary(self.cursor - 1),
        }
    }

    pub(super) fn is_delimiter(&self, pos: usize) -> bool {
        self.text
            .as_bytes()
            .get(pos)
            .map(|b| self.cfg.word_delimiters.as_bytes().contains(b))
            .unwrap_or(false)
    }

    pub(super) fn move_word_edge(&mut self, dir: Direction) {
        match dir {
            Direction::Backward => {
                /* move cursor to the start of the word */
                while self.cursor > 0 && self.is_delimiter(self.next_rune(Direction::Backward)) {
                    self.cursor = self.next_rune(Direction::Backward);
                }
                while self.cursor > 0 && !self.is_delimiter(self.next_rune(Direction::Backward)) {
                    self.cursor = self.next_rune(Direction::Backward);
                }
            }
            Direction::Forward => {
                /* move cursor to the end of the word */
                while self.cursor < self.text.len() && self.is_delimiter(self.cursor) {
                    self.cursor = self.next_rune(Direction::Forward);
                }
                while self.cursor < self.text.len() && !self.is_delimiter(self.cursor) {
                    self.cursor = self.next_rune(Direction::Forward);
                }
            }
        }
    }

    /// read_stdin — getline-per-line semantics: split on '\n' (a final chunk
    /// without trailing newline is still an item), then strip ONE trailing
    /// '\n' or '\t' byte and cut at the first NUL like strdup would.
    pub fn read_stdin(&mut self) {
        if self.cfg.password || self.cfg.input_only {
            self.input_width = 0;
            self.cfg.lines = 0;
            return;
        }

        /* read each line from stdin and add it to the item list */
        let mut input = Vec::new();
        if std::io::stdin().read_to_end(&mut input).is_err() {
            /* keep whatever we got, like getline erroring mid-way */
        }
        let mut count: i32 = 0;
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
            self.items.push(Item {
                text: line.to_owned(),
                already_output: false,
            });
            count += 1;
        }

        let columns = self.cfg.columns;
        let lines = self.cfg.lines;
        let i = count;
        self.cfg.lines = lines.min(i / columns + (i % columns != 0) as i32);
        if columns != 1 && self.cfg.lines != 0 {
            self.cfg.columns =
                (i / self.cfg.lines + (i % self.cfg.lines != 0) as i32).min(columns);
        }
    }

    /// `-it` — initial input text, applied with reject_no_match temporarily
    /// disabled (port of the insert() call in the argv loop; items are empty
    /// at that point, so this only seeds text/cursor/smartcase).
    pub fn initial_text(&mut self, s: &str) {
        let tmp = self.cfg.reject_no_match;
        self.cfg.reject_no_match = false;
        self.insert(EditOp::Insert(s));
        self.cfg.reject_no_match = tmp;
    }
}
