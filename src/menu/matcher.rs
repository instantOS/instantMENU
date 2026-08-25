//! Item matching — the token matcher ports the C `match`; fuzzy matching is
//! delegated to frizbee (SIMD Smith-Waterman). Pure: no I/O, no exit. The C
//! version printed and called exit() from inside match(); here those cases
//! become [`MatchResult`] values the shell translates into transitions.

use crate::config::{Config, MatchMode};
use crate::entry::ItemEntry;

/// One candidate line from stdin.
#[derive(Debug, Clone, Default)]
pub struct Item {
    pub text: String,
    /// How the line's prefix renders (comment / colored / icon entry);
    /// parsed once here, so drawing and measuring agree.
    pub entry: ItemEntry,
    /// printed once already (the C `out` flag) — drawn with the Out scheme.
    pub(crate) already_output: bool,
}

impl Item {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let entry = crate::entry::parse(&text);
        Item {
            text,
            entry,
            already_output: false,
        }
    }

    /// The label as it is drawn: the text minus its prefix (and, for icon
    /// entries, the icon field), on a UTF-8 boundary.
    pub fn label(&self) -> &str {
        self.text.get(self.entry.label..).unwrap_or(&self.text)
    }
}

/// What a search concluded. `AutoConfirm`/`CommentPick` are the C version's
/// print-and-exit paths (-n auto-confirm mode, commented instantASSIST mode).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MatchResult {
    /// `matches` was recomputed; the menu keeps running.
    Listed,
    /// auto-confirm mode found exactly one match: print this item and exit.
    AutoConfirm(usize),
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
    auto_confirm: bool,
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
            auto_confirm: cfg.auto_confirm,
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

    /// Port of match(): recompute `matches` for `text`. `complete` tells
    /// whether the item corpus is final (stdin reached EOF, or there is no
    /// stream). While items are still streaming in, the pick-and-exit
    /// conclusions (auto-confirm mode, commented mode) are deferred: a single
    /// match can still gain competitors, and the first item starting with a
    /// byte may not have arrived yet. Commented mode falls back to normal
    /// matching for display until then.
    pub fn search(&mut self, text: &str, complete: bool) -> MatchResult {
        if self.commented && complete {
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
            self.fuzzy_search(text, complete)
        } else {
            self.token_search(text, complete)
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
    fn token_search(&mut self, text: &str, complete: bool) -> MatchResult {
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
        self.matches = exact;
        self.matches.extend(prefix);
        self.matches.extend(substr);

        if self.auto_confirm && complete && self.matches.len() == 1 {
            return MatchResult::AutoConfirm(self.matches[0]);
        }
        MatchResult::Listed
    }

    /// The fuzzy matcher: frizbee's Smith-Waterman with affine gaps, typo
    /// tolerant — one typo per four query characters, so a slipped key still
    /// finds its app. Scores break ties by input order, keeping pipeline
    /// ordering (history, frecency) intact for equal matches.
    fn fuzzy_search(&mut self, text: &str, complete: bool) -> MatchResult {
        if text.is_empty() {
            // empty query: everything matches. If auto-confirm is enabled and
            // there is only one item, pick it immediately.
            self.matches.clear();
            self.matches.extend(0..self.items.len());
            if self.auto_confirm && complete && self.matches.len() == 1 {
                return MatchResult::AutoConfirm(self.matches[0]);
            }
            return MatchResult::Listed;
        }

        let max_typos = if self.auto_confirm {
            0
        } else {
            (text.chars().count() / 4) as u16
        };
        let config = frizbee::Config::default()
            .max_typos(Some(max_typos))
            .casing(if self.insensitive {
                frizbee::CaseMatching::Ignore
            } else {
                frizbee::CaseMatching::Respect
            });
        let mut fuzzy = frizbee::Matcher::new(text, &config);
        let haystacks: Vec<&str> = self.items.iter().map(|i| i.text.as_str()).collect();
        self.matches.clear();
        self.matches.extend(
            fuzzy
                .match_list(&haystacks)
                .into_iter()
                .map(|m| m.index as usize),
        );

        if self.auto_confirm && complete && self.matches.len() == 1 {
            return MatchResult::AutoConfirm(self.matches[0]);
        }
        MatchResult::Listed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn matcher(cfg_mut: impl FnOnce(&mut Config), items: &[&str]) -> Matcher {
        let mut cfg = Config::default();
        cfg_mut(&mut cfg);
        Matcher::new(items.iter().map(|s| Item::new(*s)).collect(), &cfg)
    }

    /// dmenu ranking: exact, then prefix, then substring.
    #[test]
    fn dmenu_ranks_exact_prefix_substring() {
        let mut m = matcher(
            |c| c.match_mode = MatchMode::Dmenu,
            &["foobar", "foo", "xfoo", "barfoo"],
        );
        assert_eq!(m.search("foo", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![1, 0, 2, 3]);
    }

    /// Every whitespace-separated token must appear; token order does not
    /// matter (strtok semantics).
    #[test]
    fn dmenu_tokens_are_and_combined() {
        let mut m = matcher(
            |c| c.match_mode = MatchMode::Dmenu,
            &["foo bar", "foo", "bar"],
        );
        assert_eq!(m.search("bar foo", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![0]);
    }

    #[test]
    fn insensitive_matches_across_case() {
        let mut m = matcher(|c| c.insensitive = true, &["foo", "bar"]);
        assert_eq!(m.search("FOO", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![0]);
    }

    /// Smart case starts insensitive; one uppercase letter turns it
    /// sensitive for good (the C flag was never reset). While insensitive,
    /// frizbee still ranks the case-identical item first (matching_case_bonus).
    #[test]
    fn smart_case_flips_once_and_never_resets() {
        let mut m = matcher(|c| c.smart_case = true, &["FOO", "foo"]);
        assert_eq!(m.search("foo", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![1, 0]);

        m.note_uppercase("Foo");
        assert_eq!(m.search("foo", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![1]);

        // lowercase input must not switch back
        m.note_uppercase("foo");
        assert_eq!(m.search("foo", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![1]);
    }

    /// smart_case is off by default: matching is case-sensitive.
    #[test]
    fn default_matching_is_case_sensitive() {
        let mut m = matcher(|_| (), &["foo"]);
        assert_eq!(m.search("FOO", true), MatchResult::Listed);
        assert!(m.matches.is_empty());
    }

    /// commented mode: the first query byte picks the first item starting
    /// with it; an empty query falls through to the normal matcher.
    #[test]
    fn commented_mode_picks_by_first_byte() {
        let mut m = matcher(|c| c.commented = true, &["yes", "no", "maybe"]);
        assert_eq!(m.search("n", true), MatchResult::CommentPick(Some(1)));
        assert_eq!(m.search("zzz", true), MatchResult::CommentPick(None));
        assert_eq!(m.search("", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![0, 1, 2]);
    }

    #[test]
    fn auto_confirm_mode_picks_the_single_exact_match() {
        let mut m = matcher(
            |c| {
                c.match_mode = MatchMode::Dmenu;
                c.auto_confirm = true;
            },
            &["abc", "bcd"],
        );
        assert_eq!(m.search("abc", true), MatchResult::AutoConfirm(0));
    }

    /// Auto-confirm is based on the number of candidates, regardless of how
    /// dmenu ranked the sole match. Menu entries commonly start with hidden
    /// metadata or an icon, making the user's visible keyword a substring.
    #[test]
    fn auto_confirm_mode_picks_the_single_substring_match() {
        let mut m = matcher(
            |c| {
                c.match_mode = MatchMode::Dmenu;
                c.auto_confirm = true;
            },
            &[":r:icon: shutdown", ":b:icon: reboot"],
        );
        assert_eq!(m.search("shut", true), MatchResult::AutoConfirm(0));
        assert_eq!(m.matches, vec![0]);
    }

    /// Empty query + single item in dmenu mode counts as an exact match and
    /// fires auto-confirm mode...
    #[test]
    fn auto_confirm_mode_fires_on_empty_dmenu_query() {
        let mut m = matcher(
            |c| {
                c.match_mode = MatchMode::Dmenu;
                c.auto_confirm = true;
            },
            &["only"],
        );
        assert_eq!(m.search("", true), MatchResult::AutoConfirm(0));
    }

    /// ...and fuzzy mode also fires auto-confirm mode on empty query with a single item.
    #[test]
    fn auto_confirm_mode_fires_on_empty_fuzzy_query() {
        let mut m = matcher(|c| c.auto_confirm = true, &["only"]);
        assert_eq!(m.search("", true), MatchResult::AutoConfirm(0));
    }

    /// While the corpus is still streaming in (`complete == false`), auto-confirm
    /// is deferred: one match can still gain competitors.
    #[test]
    fn auto_confirm_pick_is_deferred_until_the_corpus_is_complete() {
        let mut m = matcher(
            |c| {
                c.match_mode = MatchMode::Dmenu;
                c.auto_confirm = true;
            },
            &["abc"],
        );
        assert_eq!(m.search("abc", false), MatchResult::Listed);
        // the same query concludes once stdin has reached EOF
        assert_eq!(m.search("abc", true), MatchResult::AutoConfirm(0));
    }

    /// Commented mode with an incomplete corpus falls back to normal
    /// matching for display instead of picking (and exiting) by the first
    /// byte — the item that would win may not have arrived yet.
    #[test]
    fn commented_pick_is_deferred_until_the_corpus_is_complete() {
        let mut m = matcher(|c| c.commented = true, &["yes", "no", "maybe"]);
        assert_eq!(m.search("n", false), MatchResult::Listed);
        assert_eq!(m.matches, vec![1]); // normal matching filters instead
        assert_eq!(m.search("n", true), MatchResult::CommentPick(Some(1)));
    }

    /// Fuzzy scores by subsequence position: same spread but a later start
    /// ranks worse.
    #[test]
    fn fuzzy_ranks_tighter_matches_first() {
        let mut m = matcher(|_| (), &["foobar", "fobar"]);
        assert_eq!(m.search("fb", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![1, 0]);
    }

    /// One typo per four query characters: a slipped key still matches,
    /// two needle chars without a home are past the budget and filtered.
    #[test]
    fn fuzzy_tolerates_typos() {
        let mut m = matcher(|_| (), &["firefox", "thunderbird"]);
        assert_eq!(m.search("firefx", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![0]);
        m.search("firxyx", true);
        assert!(m.matches.is_empty());
    }

    /// Equal scores keep input order (stable ranking), so history/frecency
    /// ordering survives equal-quality matches.
    #[test]
    fn fuzzy_ties_keep_input_order() {
        let mut m = matcher(|_| (), &["foo", "foo"]);
        assert_eq!(m.search("foo", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![0, 1]);
    }

    /// Exact mode: only exact matches are listed (no prefix/substr ranking).
    #[test]
    fn exact_mode_lists_only_exact_matches() {
        let mut m = matcher(|c| c.match_mode = MatchMode::Exact, &["foo", "foobar"]);
        assert_eq!(m.search("foo", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![0]);
    }

    /// Auto-confirm on single item at startup in fuzzy mode.
    #[test]
    fn auto_confirm_single_item_startup() {
        let mut m = matcher(|c| c.auto_confirm = true, &["only_item"]);
        assert_eq!(m.search("", true), MatchResult::AutoConfirm(0));
    }

    /// Auto-confirm in fuzzy mode picks the exact match without false positives from typos.
    #[test]
    fn auto_confirm_picks_single_fuzzy_match() {
        let mut m = matcher(|c| c.auto_confirm = true, &["code", "cord", "cold"]);
        // "code" matches only "code" in strict fuzzy mode (0 typos) -> AutoConfirm
        assert_eq!(m.search("code", true), MatchResult::AutoConfirm(0));
    }
}
