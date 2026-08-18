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
            .rejectnomatch
            .then(|| (self.text.clone(), self.cursor));
        let cursor = self.cursor as isize;

        if n > 0 {
            let s = s.unwrap_or("");
            let byte_len = (n as usize).min(s.len()).min(TEXT_MAX - self.text.len());
            self.text.insert_str(cursor as usize, &s[..byte_len]);
            self.cursor = (cursor + byte_len as isize) as usize;

            if self.cfg.smartcase {
                let has_upper = self.text.bytes().any(|b| (65..=90).contains(&b));
                if has_upper {
                    self.cfg.smartcase = false;
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
                let byte_len = s.len().min(TEXT_MAX - self.text.len());
                self.text.insert_str(self.cursor, &s[..byte_len]);
                self.cursor += byte_len;
            }
        }

        self.do_match();

        if self.matches.is_empty() && self.cfg.rejectnomatch {
            /* revert to last text value if theres no match */
            let (text, cursor) = last.expect("rejectnomatch snapshot");
            self.text = text;
            self.cursor = cursor;
            self.do_match();
        }
    }

    /// nextrune: location of the next utf8 rune in the given direction.
    pub(super) fn nextrune(&self, inc: isize) -> usize {
        let bytes = self.text.as_bytes();
        let mut n = self.cursor as isize + inc;
        while n + inc >= 0
            && n >= 0
            && (n as usize) < bytes.len()
            && (bytes[n as usize] & 0xc0) == 0x80
        {
            n += inc;
        }
        n.max(0) as usize
    }

    pub(super) fn is_delimiter(&self, pos: usize) -> bool {
        self.text
            .as_bytes()
            .get(pos)
            .map(|b| self.cfg.worddelimiters.as_bytes().contains(b))
            .unwrap_or(false)
    }

    pub(super) fn movewordedge(&mut self, dir: isize) {
        if dir < 0 {
            /* move cursor to the start of the word */
            while self.cursor > 0 && self.is_delimiter(self.nextrune(-1)) {
                self.cursor = self.nextrune(-1);
            }
            while self.cursor > 0 && !self.is_delimiter(self.nextrune(-1)) {
                self.cursor = self.nextrune(-1);
            }
        } else {
            /* move cursor to the end of the word */
            while self.cursor < self.text.len() && self.is_delimiter(self.cursor) {
                self.cursor = self.nextrune(1);
            }
            while self.cursor < self.text.len() && !self.is_delimiter(self.cursor) {
                self.cursor = self.nextrune(1);
            }
        }
    }

    /// readstdin — getline-per-line semantics: split on '\n' (a final chunk
    /// without trailing newline is still an item), then strip ONE trailing
    /// '\n' or '\t' byte and cut at the first NUL like strdup would.
    pub fn readstdin(&mut self) {
        if self.cfg.passwd || self.cfg.inputonly {
            self.inputw = 0;
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
                out: false,
            });
            count += 1;
        }

        let columns = self.cfg.columns;
        let lines = self.cfg.lines;
        let i = count;
        self.cfg.lines = lines.min(i / columns + (i % columns != 0) as i32);
        if columns != 1 && self.cfg.lines != 0 {
            self.cfg.columns = (i / self.cfg.lines + (i % self.cfg.lines != 0) as i32).min(columns);
        }
    }

    /// `-it` — initial input text, applied with rejectnomatch temporarily
    /// disabled (port of the insert() call in the argv loop; items are empty
    /// at that point, so this only seeds text/cursor/smartcase).
    pub fn initial_text(&mut self, s: &str) {
        let tmp = self.cfg.rejectnomatch;
        self.cfg.rejectnomatch = false;
        self.insert(Some(s), s.len() as i32);
        self.cfg.rejectnomatch = tmp;
    }
}
