//! Item matching: ports of `match`, `fuzzymatch` and `calcoffsets`.

use super::Menu;

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

        if self.cfg.fuzzy {
            self.fuzzy_match();
            return;
        }

        // separate input text into tokens to be matched individually
        // (strtok collapses runs of spaces)
        let tokens: Vec<&str> = self.text.split(' ').filter(|t| !t.is_empty()).collect();
        let token_count = tokens.len();
        let len = if token_count > 0 { tokens[0].len() } else { 0 };

        let mut exact: Vec<usize> = Vec::new();
        let mut prefix: Vec<usize> = Vec::new();
        let mut substr: Vec<usize> = Vec::new();
        let text_bytes = self.text.as_bytes();
        let textsize = self.text.len() + 1;

        for (i, item) in self.items.iter().enumerate() {
            if !tokens.iter().all(|tok| self.contains(&item.text, tok)) {
                continue; // not all tokens match
            }
            /* exact matches go first, then prefixes, then substrings */
            if token_count == 0 || self.eq_n(text_bytes, item.text.as_bytes(), textsize) {
                exact.push(i);
            } else if self.eq_n(tokens[0].as_bytes(), item.text.as_bytes(), len) {
                prefix.push(i);
            } else if !self.cfg.exact {
                substr.push(i);
            }
        }
        let had_substr = !substr.is_empty();
        self.matches = exact;
        self.matches.extend(prefix);
        self.matches.extend(substr);

        self.current = if self.matches.is_empty() { None } else { Some(0) };
        self.selected = self.current;

        if self.cfg.instant && self.matches.len() == 1 && !had_substr {
            let text = self.items[self.matches[0]].text.clone();
            self.println(&text);
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
                let item_text = item.text.as_bytes();
                let mut pattern_index = 0usize; /* pointer */
                let mut match_start: i32 = -1; /* start of match */
                let mut match_end: i32 = -1; /* end of match */
                let mut i = 0usize;
                /* walk through item text */
                while i < item_text.len() {
                    let c = item_text[i];
                    /* fuzzy match pattern (single byte compare, like
                     * fstrncmp(&text[pattern_index], &c, 1)) */
                    let equal = pattern_index < text_len
                        && if self.insensitive {
                            text_bytes[pattern_index].eq_ignore_ascii_case(&c)
                        } else {
                            text_bytes[pattern_index] == c
                        };
                    if equal {
                        if match_start == -1 {
                            match_start = i as i32;
                        }
                        pattern_index += 1;
                        if pattern_index == text_len {
                            match_end = i as i32;
                            break;
                        }
                    }
                    i += 1;
                }
                /* build list of matches */
                if match_end != -1 {
                    /* compute distance:
                     * add penalty if match starts late (log(match_start+2))
                     * add penalty for a long match without many matching
                     * characters */
                    let distance = ((match_start + 2) as f64).ln()
                        + (match_end - match_start) as f64
                        - text_len as f64;
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
            let text = self.items[self.matches[0]].text.clone();
            self.println(&text);
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
        let mut pos = self.current;
        let mut i: i32 = 0;
        while let Some(p) = pos {
            i += if self.cfg.lines > 0 {
                self.bar_height
            } else {
                let item_text = self.items[self.matches[p]].text.clone();
                self.text_width_clamp(&item_text, n)
            };
            if i > n {
                break;
            }
            pos = if p + 1 < self.matches.len() {
                Some(p + 1)
            } else {
                None
            };
        }
        self.next = pos;

        /* and the previous page */
        let mut prev = self.current.unwrap_or(0);
        let mut i: i32 = 0;
        while prev > 0 {
            i += if self.cfg.lines > 0 {
                self.bar_height
            } else {
                let item_text = self.items[self.matches[prev - 1]].text.clone();
                self.text_width_clamp(&item_text, n)
            };
            if i > n {
                break;
            }
            prev -= 1;
        }
        self.prev = prev;
    }
}
