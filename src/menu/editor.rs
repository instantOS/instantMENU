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
                while self.cursor > 0
                    && self.is_delimiter(self.next_rune(Direction::Backward), delimiters)
                {
                    self.cursor = self.next_rune(Direction::Backward);
                }
                while self.cursor > 0
                    && !self.is_delimiter(self.next_rune(Direction::Backward), delimiters)
                {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Cursor-aware insertion in the middle of multi-byte text.
    #[test]
    fn insert_extends_and_moves_cursor() {
        let mut ed = Editor::new();
        ed.set_text("hällo");
        ed.cursor = "h".len(); // between h and ä
        assert!(ed.apply(EditOp::Insert("x")));
        assert_eq!(ed.text, "hxällo");
        assert_eq!(ed.cursor, 2);
    }

    #[test]
    fn delete_removes_bytes_before_cursor() {
        let mut ed = Editor::new();
        ed.set_text("hällo");
        ed.cursor = "hä".len();
        assert!(ed.apply(EditOp::Delete("ä".len())));
        assert_eq!(ed.text, "hllo");
        assert_eq!(ed.cursor, 1);
    }

    /// The C version truncated at BUFSIZ; going over budget rejects the
    /// whole insert instead.
    #[test]
    fn insert_beyond_text_max_is_rejected() {
        let mut ed = Editor::new();
        let full = "x".repeat(TEXT_MAX);
        assert!(ed.apply(EditOp::Insert(&full)));
        assert!(!ed.apply(EditOp::Insert("x")));
        assert_eq!(ed.text.len(), TEXT_MAX);
    }

    /// A multi-byte item longer than the budget must cut on a char boundary,
    /// not mid-byte.
    #[test]
    fn set_text_caps_on_char_boundary() {
        let mut ed = Editor::new();
        ed.set_text(&"ä".repeat(TEXT_MAX));
        assert!(ed.text.len() <= TEXT_MAX);
        assert!(ed.text.is_char_boundary(ed.text.len()));
        assert!(ed.text.chars().all(|c| c == 'ä'));
    }

    #[test]
    fn next_rune_skips_whole_utf8_chars() {
        let mut ed = Editor::new();
        ed.set_text("hällo");
        ed.cursor = 1;
        assert_eq!(ed.next_rune(Direction::Forward), 3);
        assert_eq!(ed.next_rune(Direction::Backward), 0);
    }

    #[test]
    fn word_edges_walk_over_delimiters() {
        let delimiters = " ";
        let mut ed = Editor::new();
        ed.set_text("foo bar baz");
        ed.cursor = "foo bar".len(); // between r and the second space run? no: end of "bar"
        ed.move_word_edge(Direction::Forward, delimiters);
        assert_eq!(ed.cursor, "foo bar baz".len());
        ed.move_word_edge(Direction::Backward, delimiters);
        assert_eq!(ed.cursor, "foo bar ".len());
    }

    /// Ctrl-w order (C deleteword): delimiters directly left of the cursor
    /// first, then the word. The delimiters *before* the word survive.
    #[test]
    fn word_delete_target_spans_trailing_delimiters_then_word() {
        let mut ed = Editor::new();
        ed.set_text("hello world");
        ed.cursor = ed.text.len();
        assert_eq!(ed.word_delete_target(" "), "hello ".len());
        assert!(ed.apply(EditOp::Delete(ed.cursor - "hello ".len())));
        assert_eq!(ed.text, "hello ");

        // trailing delimiters go together with the word that follows them
        ed.set_text("hello   ");
        ed.cursor = ed.text.len();
        assert_eq!(ed.word_delete_target(" "), 0);
        assert!(ed.apply(EditOp::Delete(ed.text.len())));
        assert_eq!(ed.text, "");
    }
}
