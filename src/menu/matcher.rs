//! Item matching — ports of `match` and `fuzzymatch`, pure: no I/O, no
//! exit. The C version printed and called exit() from inside match(); here
//! those cases become [`MatchResult`] values the shell translates into
//! transitions.

use crate::config::{Config, MatchMode};

/// One candidate line from stdin.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub text: String,
    /// printed once already (the C `out` flag) — drawn with the Out scheme.
    pub(crate) already_output: bool,
}

impl Item {
    pub fn new(text: impl Into<String>) -> Self {
        Item {
            text: text.into(),
            already_output: false,
        }
    }
}

/// What a search concluded. `InstantPick`/`CommentPick` are the C version's
/// print-and-exit paths (-n instant mode, commented instantASSIST mode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MatchResult {
    /// `matches` was recomputed; the menu keeps running.
    Listed,
    /// instant mode found exactly one match: print this item and exit.
    InstantPick(usize),
    /// commented mode: print this item (or nothing) and exit.
    CommentPick(Option<usize>),
}

/// The item list plus the matching algorithm and its case-sensitivity state.
#[derive(Debug, Clone)]
pub(super) struct Matcher {
    pub items: Vec<Item>,
    /// Ordered item indices of the current matches (the C linked list).
    pub matches: Vec<usize>,
    mode: MatchMode,
    commented: bool,
    instant: bool,
    /* case-insensitive matching (the fstrncmp/fstrstr function pointers) */
    insensitive: bool,
    /// smart case: insensitive until the query contains uppercase.
    smart_case: bool,
}

impl Matcher {
    pub fn new(items: Vec<Item>, cfg: &Config) -> Self {
        // Port of -i/-s switching fstrncmp/fstrstr (smartcase starts out
        // insensitive and turns sensitive on uppercase input).
        Matcher {
            items,
            matches: Vec::new(),
            mode: cfg.match_mode,
            commented: cfg.commented,
            instant: cfg.instant,
            insensitive: cfg.smart_case || cfg.insensitive,
            smart_case: cfg.smart_case,
        }
    }

    /// Smart case: once the query holds an uppercase letter, matching turns
    /// case-sensitive for good (the C flag was never reset).
    pub fn note_uppercase(&mut self, text: &str) {
        if self.smart_case && text.bytes().any(|b| b.is_ascii_uppercase()) {
            self.smart_case = false;
            self.insensitive = false;
        }
    }

    /// Port of match(): recompute `matches` for `text`.
    pub fn search(&mut self, text: &str) -> MatchResult {
        if self.commented {
            // instantASSIST: the first byte of the query picks the first
            // item starting with it; an empty query falls through to the
            // normal matcher (C behaviour).
            if let Some(c) = text.bytes().next() {
                let pick = self
                    .items
                    .iter()
                    .position(|item| item.text.as_bytes().first() == Some(&c));
                return MatchResult::CommentPick(pick);
            }
        }

        if self.mode == MatchMode::Fuzzy {
            self.fuzzy_search(text)
        } else {
            self.token_search(text)
        }
    }

    pub fn text_of_match(&self, pos: usize) -> &str {
        self.items[self.matches[pos]].text.as_str()
    }

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

    /// The dmenu/exact matcher: every whitespace-separated token must appear
    /// in the item; exact, then prefix, then substring matches (dmenu mode
    /// only ranks prefixes; exact mode lists everything that matches).
    fn token_search(&mut self, text: &str) -> MatchResult {
        // separate input text into tokens to be matched individually
        // (strtok collapses runs of spaces)
        let tokens: Vec<&str> = text.split(' ').filter(|t| !t.is_empty()).collect();
        let first_token = tokens.first().copied().unwrap_or("");
        let len = first_token.len();

        let mut exact: Vec<usize> = Vec::new();
        let mut prefix: Vec<usize> = Vec::new();
        let mut substr: Vec<usize> = Vec::new();
        let text_bytes = text.as_bytes();
        let textsize = text.len() + 1;

        for (i, item) in self.items.iter().enumerate() {
            if !tokens.iter().all(|tok| self.contains(&item.text, tok)) {
                continue; // not all tokens match
            }
            if tokens.is_empty() || self.eq_n(text_bytes, item.text.as_bytes(), textsize) {
                exact.push(i); /* exact matches always go first */
            } else if self.mode == MatchMode::Dmenu {
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

        if self.instant && self.matches.len() == 1 && !had_substr {
            return MatchResult::InstantPick(self.matches[0]);
        }
        MatchResult::Listed
    }

    /// The fuzzy matcher: subsequence match scored by match position and
    /// spread, best first.
    fn fuzzy_search(&mut self, text: &str) -> MatchResult {
        let text_bytes = text.as_bytes();
        let text_len = text_bytes.len();

        if text_len == 0 {
            // empty query: everything matches, and — unlike the token
            // matcher — instant mode does not fire (C early return).
            self.matches.clear();
            self.matches.extend(0..self.items.len());
            return MatchResult::Listed;
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
                let distance = ((start + 2) as f64).ln() + (end - start) as f64 - text_len as f64;
                scored.push((idx, distance));
            }
        }

        /* sort matches according to distance */
        scored.sort_by(|a, b| a.1.total_cmp(&b.1));
        self.matches.clear();
        self.matches.extend(scored.into_iter().map(|(idx, _)| idx));

        if self.instant && self.matches.len() == 1 {
            return MatchResult::InstantPick(self.matches[0]);
        }
        MatchResult::Listed
    }
}
