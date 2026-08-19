//! Input handling: text insertion/editing and stdin item loading.

use std::io::Read;

use super::{Item, Menu, TEXT_MAX};

impl Menu {
    /// Port of insert(): insert `s` at the cursor (n > 0, or n == 0 with a
    /// non-empty string) or delete -n bytes before the cursor (n < 0).
    pub(super) fn insert(&mut self, s: Option<&str>, n: i32) {
        let n = n as isize;
        if self.text.len() as isize + n > TEXT_MAX as isize {
            return;
        }

        let last = self
            .cfg
            .reject_no_match
            .then(|| (self.text.clone(), self.cursor));
        let cursor = self.cursor as isize;

        if n > 0 {
            let s = s.unwrap_or("");
            // cut at the TEXT_MAX budget on a char boundary; a raw byte cut
            // could land inside a multi-byte char and panic on insert_str
            let byte_len = s
                .floor_char_boundary((n as usize).min(s.len()).min(TEXT_MAX - self.text.len()));
            self.text.insert_str(cursor as usize, &s[..byte_len]);
            self.cursor = (cursor + byte_len as isize) as usize;

            if self.cfg.smart_case {
                let has_upper = self.text.bytes().any(|b| b.is_ascii_uppercase());
                if has_upper {
                    self.cfg.smart_case = false;
                    self.insensitive = false;
                }
            }
        } else if n < 0 {
            let cut = (cursor + n).max(0) as usize;
            self.text.replace_range(cut..cursor as usize, "");
            self.cursor = cut;
        } else if let Some(s) = s {
            // n == 0 with a payload: -it inserts with strlen(text)
            if !s.is_empty() {
                let byte_len =
                    s.floor_char_boundary(s.len().min(TEXT_MAX - self.text.len()));
                self.text.insert_str(self.cursor, &s[..byte_len]);
                self.cursor += byte_len;
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
    pub(super) fn next_rune(&self, inc: isize) -> usize {
        if inc > 0 {
            self.text.ceil_char_boundary(self.cursor + 1)
        } else {
            self.text.floor_char_boundary(self.cursor - 1)
        }
    }

    pub(super) fn is_delimiter(&self, pos: usize) -> bool {
        self.text
            .as_bytes()
            .get(pos)
            .map(|b| self.cfg.word_delimiters.as_bytes().contains(b))
            .unwrap_or(false)
    }

    pub(super) fn move_word_edge(&mut self, dir: isize) {
        if dir < 0 {
            /* move cursor to the start of the word */
            while self.cursor > 0 && self.is_delimiter(self.next_rune(-1)) {
                self.cursor = self.next_rune(-1);
            }
            while self.cursor > 0 && !self.is_delimiter(self.next_rune(-1)) {
                self.cursor = self.next_rune(-1);
            }
        } else {
            /* move cursor to the end of the word */
            while self.cursor < self.text.len() && self.is_delimiter(self.cursor) {
                self.cursor = self.next_rune(1);
            }
            while self.cursor < self.text.len() && !self.is_delimiter(self.cursor) {
                self.cursor = self.next_rune(1);
            }
        }
    }

    /// read_stdin — getline-per-line semantics: split on '\n' (a final chunk
    /// without trailing newline is still an item), then strip ONE trailing
    /// '\n' or '\t' byte and cut at the first NUL like strdup would.
    pub fn read_stdin(cfg: &mut crate::config::Config) -> Vec<Item> {
        if cfg.password || cfg.input_only {
            cfg.lines = 0;
            return Vec::new();
        }

        /* read each line from stdin and add it to the item list */
        let mut input = Vec::new();
        if std::io::stdin().read_to_end(&mut input).is_err() {
            /* keep whatever we got, like getline erroring mid-way */
        }
        let mut items = Vec::new();
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
            items.push(Item {
                text: line.to_owned(),
                already_output: false,
            });
            count += 1;
        }

        let columns = cfg.columns;
        let lines = cfg.lines;
        let i = count;
        cfg.lines = lines.min(i / columns + (i % columns != 0) as i32);
        if columns != 1 && cfg.lines != 0 {
            cfg.columns = (i / cfg.lines + (i % cfg.lines != 0) as i32).min(columns);
        }
        items
    }

    /// `-it` — initial input text, applied with reject_no_match temporarily
    /// disabled (port of the insert() call in the argv loop; items are empty
    /// at that point, so this only seeds text/cursor/smartcase).
    pub fn initial_text(&mut self, s: &str) {
        let tmp = self.cfg.reject_no_match;
        self.cfg.reject_no_match = false;
        self.insert(Some(s), s.len() as i32);
        self.cfg.reject_no_match = tmp;
    }
}
