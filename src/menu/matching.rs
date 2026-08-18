//! Item matching: ports of `match`, `fuzzymatch` and `calcoffsets`.

use super::Menu;
use crate::config::MatchMode;

impl Menu {
    /// fstrncmp(a, b, n) == 0, honoring the case-insensitivity switch.
    /// Byte-wise strncmp emulation: compares up to n bytes, treating the end
    /// of a slice as the C NUL terminator.
    fn eq_n(&self, a: &[u8], b: &[u8], n: usize) -> bool {
        for i in 0..n {
            let ca = a.get(i).copied().unwrap_or(0);
            let cb = b.get(i).copied().unwrap_or(0);
            let (ca, cb) = if self.insensitive {
                (ca.to_ascii_lowercase(), cb.to_ascii_lowercase())
            } else {
                (ca, cb)
            };
            if ca != cb {
                return false;
            }
            if ca == 0 {
                return true; // both terminated
            }
        }
        true
    }

    /// fstrstr, honoring the case switch.
    fn contains(&self, haystack: &str, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let needle = needle.as_bytes();
        haystack.as_bytes().windows(needle.len()).any(|candidate| {
            if self.insensitive {
                candidate.eq_ignore_ascii_case(needle)
            } else {
                candidate == needle
            }
        })
    }

    pub(super) fn do_match(&mut self) {
        if self.cfg.commented {
            let first = self.text.bytes().next();
            if let Some(c) = first {
                for item in &self.items {
                    if item.text.as_bytes().first() == Some(&c) {
                        let text = item.text.clone();
                        self.println(&text);
                        self.finish(0);
                    }
                }
                // exit if no match is found
                self.finish(0);
            }
        }

        if self.cfg.match_mode == MatchMode::Fuzzy {
            self.fuzzy_match();
            return;
        }

        // separate input text into tokens to be matched individually
        // (strtok collapses runs of spaces)
        let tokens: Vec<&str> = self.text.split(' ').filter(|t| !t.is_empty()).collect();
        let first_token = tokens.first().copied().unwrap_or("");
        let len = first_token.len();

        let mut exact: Vec<usize> = Vec::new();
        let mut prefix: Vec<usize> = Vec::new();
        let mut substr: Vec<usize> = Vec::new();
        let text_bytes = self.text.as_bytes();
        let textsize = self.text.len() + 1;

        for (i, item) in self.items.iter().enumerate() {
            if !tokens.iter().all(|tok| self.contains(&item.text, tok)) {
                continue; // not all tokens match
            }
            if tokens.is_empty() || self.eq_n(text_bytes, item.text.as_bytes(), textsize) {
                exact.push(i); /* exact matches always go first */
            } else if self.cfg.match_mode == MatchMode::Dmenu {
                /* dmenu mode also ranks prefixes, then substrings */
                if self.eq_n(first_token.as_bytes(), item.text.as_bytes(), len) {
                    prefix.push(i);
                } else {
                    substr.push(i);
                }
            }
        }
        let had_substr = !substr.is_empty();
        self.matches = exact;
        self.matches.extend(prefix);
        self.matches.extend(substr);

        self.current = if self.matches.is_empty() { None } else { Some(0) };
        self.selected = self.current;

        if self.cfg.instant && self.matches.len() == 1 && !had_substr {
            if let Some(&item_index) = self.matches.first() {
                let text = self.items[item_index].text.clone();
                self.println(&text);
            }
            self.finish(0);
        }

        self.calc_offsets();
    }

    fn fuzzy_match(&mut self) {
        let text_bytes = self.text.as_bytes();
        let text_len = text_bytes.len();

        if text_len == 0 {
            self.matches.clear();
            self.matches.extend(0..self.items.len());
            self.current = (!self.matches.is_empty()).then_some(0);
            self.selected = self.current;
            self.calc_offsets();
            return;
        }

        /* walk through all items */
        let mut scored: Vec<(usize, f64)> = Vec::new();
        for (idx, item) in self.items.iter().enumerate() {
            let mut pattern_index = 0usize;
            let mut match_start = None;
            let mut match_end = None;
            for (i, &c) in item.text.as_bytes().iter().enumerate() {
                /* fuzzy match pattern (single byte compare, like
                 * fstrncmp(&text[pattern_index], &c, 1)) */
                let equal = pattern_index < text_len
                    && if self.insensitive {
                        text_bytes[pattern_index].eq_ignore_ascii_case(&c)
                    } else {
                        text_bytes[pattern_index] == c
                    };
                if equal {
                    match_start.get_or_insert(i);
                    pattern_index += 1;
                    if pattern_index == text_len {
                        match_end = Some(i);
                        break;
                    }
                }
            }
            /* compute distance:
             * add penalty if match starts late (log(match_start+2))
             * add penalty for a long match without many matching characters */
            if let (Some(start), Some(end)) = (match_start, match_end) {
                let distance =
                    ((start + 2) as f64).ln() + (end - start) as f64 - text_len as f64;
                scored.push((idx, distance));
            }
        }

        /* sort matches according to distance */
        scored.sort_by(|a, b| a.1.total_cmp(&b.1));
        self.matches.clear();
        self.matches.extend(scored.into_iter().map(|(idx, _)| idx));
        self.current = if self.matches.is_empty() { None } else { Some(0) };
        self.selected = self.current;

        if self.cfg.instant && self.matches.len() == 1 {
            if let Some(&item_index) = self.matches.first() {
                let text = self.items[item_index].text.clone();
                self.println(&text);
            }
            self.finish(0);
        }

        self.calc_offsets();
    }

    /// calcoffsets — which items begin the next and previous pages.
    pub(super) fn calc_offsets(&mut self) {
        let n = if self.cfg.lines > 0 {
            self.cfg.lines * self.cfg.columns * self.bar_height
        } else {
            let langle = self.text_width("<");
            let rangle = self.text_width(">");
            self.menu_width - (self.prompt_width + self.input_width + langle + rangle)
        };

        /* calculate which items will begin the next page */
        let mut next = None;
        if let Some(start) = self.current {
            let mut used: i32 = 0;
            for pos in start..self.matches.len() {
                used += if self.cfg.lines > 0 {
                    self.bar_height
                } else {
                    let item_text = self.items[self.matches[pos]].text.clone();
                    self.text_width_clamp(&item_text, n)
                };
                if used > n {
                    next = Some(pos);
                    break;
                }
            }
        }
        self.next = next;

        /* and the previous page */
        let start = self.current.unwrap_or(0);
        let mut used: i32 = 0;
        let mut prev = start;
        for pos in (0..start).rev() {
            used += if self.cfg.lines > 0 {
                self.bar_height
            } else {
                let item_text = self.items[self.matches[pos]].text.clone();
                self.text_width_clamp(&item_text, n)
            };
            if used > n {
                break;
            }
            prev = pos;
        }
        self.prev = prev;
    }
}
