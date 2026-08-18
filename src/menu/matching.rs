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
        if self.insensitive {
            haystack.to_lowercase().contains(&needle.to_lowercase())
        } else {
            haystack.contains(needle)
        }
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
            self.fuzzymatch();
            return;
        }

        // separate input text into tokens to be matched individually
        // (strtok collapses runs of spaces)
        let tokv: Vec<&str> = self.text.split(' ').filter(|t| !t.is_empty()).collect();
        let tokc = tokv.len();
        let len = if tokc > 0 { tokv[0].len() } else { 0 };

        let mut exact: Vec<usize> = Vec::new();
        let mut prefix: Vec<usize> = Vec::new();
        let mut substr: Vec<usize> = Vec::new();
        let text_bytes = self.text.as_bytes();
        let textsize = self.text.len() + 1;

        for (i, item) in self.items.iter().enumerate() {
            if !tokv.iter().all(|tok| self.contains(&item.text, tok)) {
                continue; // not all tokens match
            }
            /* exact matches go first, then prefixes, then substrings */
            if tokc == 0 || self.eq_n(text_bytes, item.text.as_bytes(), textsize) {
                exact.push(i);
            } else if self.eq_n(tokv[0].as_bytes(), item.text.as_bytes(), len) {
                prefix.push(i);
            } else if !self.cfg.exact {
                substr.push(i);
            }
        }
        let had_substr = !substr.is_empty();
        self.matches = exact;
        self.matches.extend(prefix);
        self.matches.extend(substr);

        self.curr = if self.matches.is_empty() { None } else { Some(0) };
        self.sel = self.curr;

        if self.cfg.instant && self.matches.len() == 1 && !had_substr {
            let text = self.items[self.matches[0]].text.clone();
            self.println(&text);
            self.finish(0);
        }

        self.calcoffsets();
    }

    fn fuzzymatch(&mut self) {
        /* bang - we have so much memory */
        let mut matched: Vec<usize> = Vec::new();
        let text_bytes = self.text.as_bytes().to_vec();
        let text_len = text_bytes.len();

        /* walk through all items */
        for (idx, item) in self.items.iter_mut().enumerate() {
            if text_len > 0 {
                let itext = item.text.as_bytes();
                let mut pidx = 0usize; /* pointer */
                let mut sidx: i32 = -1; /* start of match */
                let mut eidx: i32 = -1; /* end of match */
                let mut i = 0usize;
                /* walk through item text */
                while i < itext.len() {
                    let c = itext[i];
                    /* fuzzy match pattern (single byte compare, like
                     * fstrncmp(&text[pidx], &c, 1)) */
                    let equal = pidx < text_len
                        && if self.insensitive {
                            text_bytes[pidx].eq_ignore_ascii_case(&c)
                        } else {
                            text_bytes[pidx] == c
                        };
                    if equal {
                        if sidx == -1 {
                            sidx = i as i32;
                        }
                        pidx += 1;
                        if pidx == text_len {
                            eidx = i as i32;
                            break;
                        }
                    }
                    i += 1;
                }
                /* build list of matches */
                if eidx != -1 {
                    /* compute distance:
                     * add penalty if match starts late (log(sidx+2))
                     * add penalty for a long match without many matching
                     * characters */
                    item.distance =
                        ((sidx + 2) as f64).ln() + (eidx - sidx) as f64 - text_len as f64;
                    matched.push(idx);
                }
            } else {
                matched.push(idx);
            }
        }

        /* sort matches according to distance */
        matched.sort_by(|&a, &b| {
            self.items[a]
                .distance
                .partial_cmp(&self.items[b].distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        self.matches = matched;
        self.curr = if self.matches.is_empty() { None } else { Some(0) };
        self.sel = self.curr;

        if self.cfg.instant && self.matches.len() == 1 {
            let text = self.items[self.matches[0]].text.clone();
            self.println(&text);
            self.finish(0);
        }

        self.calcoffsets();
    }

    /// calcoffsets — which items begin the next and previous pages.
    pub(super) fn calcoffsets(&mut self) {
        let n = if self.cfg.lines > 0 {
            self.cfg.lines * self.cfg.columns * self.bh
        } else {
            let langle = self.textw("<");
            let rangle = self.textw(">");
            self.mw - (self.promptw + self.inputw + langle + rangle)
        };

        /* calculate which items will begin the next page */
        let mut pos = self.curr;
        let mut i: i32 = 0;
        while let Some(p) = pos {
            let item_text = self.item_text(p);
            i += if self.cfg.lines > 0 {
                self.bh
            } else {
                self.textw_clamp(&item_text, n)
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
        let mut prev = self.curr.unwrap_or(0);
        let mut i: i32 = 0;
        while prev > 0 {
            let item_text = self.item_text(prev - 1);
            i += if self.cfg.lines > 0 {
                self.bh
            } else {
                self.textw_clamp(&item_text, n)
            };
            if i > n {
                break;
            }
            prev -= 1;
        }
        self.prev = prev;
    }
}
