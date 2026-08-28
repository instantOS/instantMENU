//! Item matching — the token matcher ports dmenu's ordering while fuzzy
//! matching is delegated to frizbee. Item markup is parsed once on arrival;
//! matching consumes the prepared label/search text and never reparses it.

use crate::config::{Config, MatchMode};
use crate::entry::ItemEntry;

/// One parsed candidate line from stdin.
#[derive(Debug, Clone, Default)]
pub struct Item {
    /// Visible label and value printed when selected.
    pub text: String,
    /// Optional output value printed when selected.
    pub value: Option<String>,
    /// Label plus hidden `match=` terms. Absent for the fast plain-item path.
    search_text: Option<String>,
    /// Presentation, single-key and structural metadata.
    pub entry: ItemEntry,
    /// Printed once already (the historical `out` flag).
    pub(crate) already_output: bool,
}

impl Item {
    pub fn new(source: impl Into<String>) -> Self {
        let source = source.into();
        let parsed = crate::entry::parse(&source);
        let label_is_source = parsed.label.len() == source.len()
            && std::ptr::eq(parsed.label.as_ptr(), source.as_ptr());
        let entry = parsed.entry;
        let match_text = parsed.match_text;
        let value = parsed.value;
        let parsed_label = (!label_is_source).then(|| parsed.label.to_owned());
        let text = parsed_label.unwrap_or(source);
        let search_text = match_text.map(|terms| format!("{text} {terms}"));
        Item {
            text,
            value,
            search_text,
            entry,
            already_output: false,
        }
    }

    pub fn label(&self) -> &str {
        &self.text
    }

    pub fn output(&self) -> &str {
        self.value.as_deref().unwrap_or(&self.text)
    }

    pub fn searchable_text(&self) -> &str {
        self.search_text.as_deref().unwrap_or(&self.text)
    }

    pub fn is_selectable(&self) -> bool {
        !self.entry.is_heading()
    }
}

/// What a search concluded. Pick variants let the pure matcher report an
/// early exit without doing I/O itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MatchResult {
    Listed,
    AutoConfirm(usize),
    SingleKeyPick(Option<usize>),
}

/// Candidate corpus plus the matching policy selected by the configuration.
#[derive(Debug, Clone)]
pub(super) struct Matcher {
    pub items: Vec<Item>,
    /// Ordered item indices currently visible. Empty-query headings are in
    /// this list for layout, but are never valid selection targets.
    pub matches: Vec<usize>,
    mode: MatchMode,
    single_key: bool,
    auto_confirm: bool,
    insensitive: bool,
    smart_case: bool,
}

impl Matcher {
    pub fn new(items: Vec<Item>, cfg: &Config) -> Self {
        Matcher {
            items,
            matches: Vec::new(),
            mode: cfg.match_mode,
            single_key: cfg.single_key,
            auto_confirm: cfg.auto_confirm,
            insensitive: cfg.smart_case || cfg.insensitive,
            smart_case: cfg.smart_case,
        }
    }

    pub fn note_uppercase(&mut self, text: &str) {
        if self.smart_case && text.bytes().any(|b| b.is_ascii_uppercase()) {
            self.smart_case = false;
            self.insensitive = false;
        }
    }

    /// Recompute visible matches. Single-key mode is deliberately separate
    /// from fuzzy/token matching: only explicitly keyed items participate,
    /// and one complete Unicode character activates the matching key.
    pub fn search(&mut self, text: &str, complete: bool) -> MatchResult {
        if self.single_key {
            return self.single_key_search(text, complete);
        }
        if self.mode == MatchMode::Fuzzy {
            self.fuzzy_search(text, complete)
        } else {
            self.token_search(text, complete)
        }
    }

    pub fn text_of_match(&self, pos: usize) -> &str {
        self.items[self.matches[pos]].label()
    }

    pub fn output_of_match(&self, pos: usize) -> &str {
        self.items[self.matches[pos]].output()
    }

    pub fn match_is_selectable(&self, pos: usize) -> bool {
        self.matches
            .get(pos)
            .is_some_and(|&index| self.items[index].is_selectable())
    }

    pub fn first_selectable_match(&self) -> Option<usize> {
        (0..self.matches.len()).find(|&pos| self.match_is_selectable(pos))
    }

    pub fn selectable_match_count(&self) -> usize {
        self.matches
            .iter()
            .filter(|&&index| self.items[index].is_selectable())
            .count()
    }

    pub fn selectable_item_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.is_selectable() && (!self.single_key || item.entry.key.is_some()))
            .count()
    }

    pub fn layout_item_count(&self) -> usize {
        if self.single_key {
            self.selectable_item_count()
        } else {
            self.items.len()
        }
    }

    fn single_key_search(&mut self, text: &str, complete: bool) -> MatchResult {
        let mut chars = text.chars();
        let key = chars.next();
        let is_one_character = key.is_some() && chars.next().is_none();

        self.matches.clear();
        self.matches.extend(
            self.items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    item.is_selectable()
                        && item.entry.key.is_some()
                        && key.is_none_or(|key| item.entry.key == Some(key))
                })
                .map(|(index, _)| index),
        );

        if complete && key.is_some() {
            let pick = is_one_character
                .then(|| self.matches.first().copied())
                .flatten();
            return MatchResult::SingleKeyPick(pick);
        }
        MatchResult::Listed
    }

    /// Byte-wise strncmp emulation honoring the case setting.
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
                return true;
            }
        }
        true
    }

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

    /// Every whitespace-separated token must occur in the prepared search
    /// text. Exact/prefix ranking uses only the visible label, so hidden
    /// keywords never make a label lose its natural rank.
    fn token_search(&mut self, text: &str, complete: bool) -> MatchResult {
        let tokens: Vec<&str> = text.split(' ').filter(|t| !t.is_empty()).collect();
        let first_token = tokens.first().copied().unwrap_or("");
        let len = first_token.len();
        let mut exact = Vec::new();
        let mut prefix = Vec::new();
        let mut substr = Vec::new();
        let text_bytes = text.as_bytes();
        let textsize = text.len() + 1;

        for (i, item) in self.items.iter().enumerate() {
            // Headings structure the unfiltered list; they are not results.
            if !tokens.is_empty() && !item.is_selectable() {
                continue;
            }
            if !tokens
                .iter()
                .all(|token| self.contains(item.searchable_text(), token))
            {
                continue;
            }
            if tokens.is_empty() || self.eq_n(text_bytes, item.label().as_bytes(), textsize) {
                exact.push(i);
            } else if self.mode == MatchMode::Dmenu {
                if self.eq_n(first_token.as_bytes(), item.label().as_bytes(), len) {
                    prefix.push(i);
                } else {
                    substr.push(i);
                }
            }
        }
        self.matches = exact;
        self.matches.extend(prefix);
        self.matches.extend(substr);
        self.auto_confirm_result(complete)
    }

    fn fuzzy_search(&mut self, text: &str, complete: bool) -> MatchResult {
        if text.is_empty() {
            self.matches.clear();
            self.matches.extend(0..self.items.len());
            return self.auto_confirm_result(complete);
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
        let candidates: Vec<(usize, &str)> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.is_selectable())
            .map(|(index, item)| (index, item.searchable_text()))
            .collect();
        let haystacks: Vec<&str> = candidates.iter().map(|(_, text)| *text).collect();
        self.matches.clear();
        self.matches.extend(
            fuzzy
                .match_list(&haystacks)
                .into_iter()
                .map(|matched| candidates[matched.index as usize].0),
        );
        self.auto_confirm_result(complete)
    }

    /// Auto-confirm counts selectable results, not structural headings.
    fn auto_confirm_result(&self, complete: bool) -> MatchResult {
        if self.auto_confirm && complete {
            let mut selectable = self
                .matches
                .iter()
                .copied()
                .filter(|&index| self.items[index].is_selectable());
            if let (Some(only), None) = (selectable.next(), selectable.next()) {
                return MatchResult::AutoConfirm(only);
            }
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

    #[test]
    fn plain_items_reuse_their_input_allocation() {
        let source = String::from("Display");
        let source_ptr = source.as_ptr();
        let item = Item::new(source);
        assert_eq!(item.text.as_ptr(), source_ptr);
    }

    #[test]
    fn dmenu_ranks_exact_prefix_substring() {
        let mut m = matcher(
            |c| c.match_mode = MatchMode::Dmenu,
            &["foobar", "foo", "xfoo", "barfoo"],
        );
        assert_eq!(m.search("foo", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![1, 0, 2, 3]);
    }

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
    fn hidden_match_terms_work_without_changing_label_or_exact_rank() {
        let mut m = matcher(
            |c| c.match_mode = MatchMode::Dmenu,
            &["{match='monitor screen'} Display", "monitor configuration"],
        );
        assert_eq!(m.items[0].label(), "Display");
        m.search("Display", true);
        assert_eq!(m.matches, vec![0]);
        m.search("monitor", true);
        assert_eq!(m.matches, vec![1, 0]);
        m.search("screen", true);
        assert_eq!(m.matches, vec![0]);
    }

    #[test]
    fn hidden_match_terms_participate_in_fuzzy_matching() {
        let mut m = matcher(|_| (), &["{match='monitor screen'} Display", "Terminal"]);
        m.search("monitor", true);
        assert_eq!(m.matches, vec![0]);
        assert_eq!(m.text_of_match(0), "Display");
    }

    #[test]
    fn insensitive_matches_across_case() {
        let mut m = matcher(|c| c.insensitive = true, &["foo", "bar"]);
        assert_eq!(m.search("FOO", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![0]);
    }

    #[test]
    fn smart_case_flips_once_and_never_resets() {
        let mut m = matcher(|c| c.smart_case = true, &["FOO", "foo"]);
        m.search("foo", true);
        assert_eq!(m.matches, vec![1, 0]);
        m.note_uppercase("Foo");
        m.search("foo", true);
        assert_eq!(m.matches, vec![1]);
        m.note_uppercase("foo");
        m.search("foo", true);
        assert_eq!(m.matches, vec![1]);
    }

    #[test]
    fn default_matching_is_case_sensitive() {
        let mut m = matcher(|_| (), &["foo"]);
        m.search("FOO", true);
        assert!(m.matches.is_empty());
    }

    #[test]
    fn single_key_mode_uses_explicit_unicode_keys_and_returns_labels() {
        let mut m = matcher(
            |c| c.single_key = true,
            &["{key=d} Display", "No key", "{key=λ} Lambda"],
        );
        assert_eq!(m.search("", true), MatchResult::Listed);
        assert_eq!(m.matches, vec![0, 2]);
        assert_eq!(m.search("λ", true), MatchResult::SingleKeyPick(Some(2)));
        assert_eq!(m.items[2].label(), "Lambda");
        assert_eq!(m.search("x", true), MatchResult::SingleKeyPick(None));
        assert_eq!(m.search("dd", true), MatchResult::SingleKeyPick(None));
    }

    #[test]
    fn single_key_pick_waits_for_stream_completion() {
        let mut m = matcher(|c| c.single_key = true, &["{key=n} Network"]);
        assert_eq!(m.search("n", false), MatchResult::Listed);
        assert_eq!(m.matches, vec![0]);
        assert_eq!(m.search("n", true), MatchResult::SingleKeyPick(Some(0)));
    }

    #[test]
    fn duplicate_single_keys_keep_input_order() {
        let mut m = matcher(
            |c| c.single_key = true,
            &["{key=q} First", "{key=q} Second"],
        );
        assert_eq!(m.search("q", true), MatchResult::SingleKeyPick(Some(0)));
    }

    #[test]
    fn headings_are_visible_only_before_filtering() {
        let mut m = matcher(|_| (), &["{heading} Applications", "Display", "Terminal"]);
        m.search("", true);
        assert_eq!(m.matches, vec![0, 1, 2]);
        assert_eq!(m.first_selectable_match(), Some(1));
        m.search("Application", true);
        assert!(m.matches.is_empty());
        m.search("Display", true);
        assert_eq!(m.matches, vec![1]);
    }

    #[test]
    fn auto_confirm_ignores_headings() {
        let mut m = matcher(
            |c| c.auto_confirm = true,
            &["{heading} Applications", "Display"],
        );
        assert_eq!(m.search("", true), MatchResult::AutoConfirm(1));
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

    #[test]
    fn auto_confirm_mode_picks_match_metadata() {
        let mut m = matcher(
            |c| {
                c.match_mode = MatchMode::Dmenu;
                c.auto_confirm = true;
            },
            &[
                "{match=shutdown icon=power} Power off",
                "{match=reboot icon=restart} Restart",
            ],
        );
        assert_eq!(m.search("shutdown", true), MatchResult::AutoConfirm(0));
    }

    #[test]
    fn auto_confirm_is_deferred_while_streaming() {
        let mut m = matcher(
            |c| {
                c.match_mode = MatchMode::Dmenu;
                c.auto_confirm = true;
            },
            &["abc"],
        );
        assert_eq!(m.search("abc", false), MatchResult::Listed);
        assert_eq!(m.search("abc", true), MatchResult::AutoConfirm(0));
    }

    #[test]
    fn fuzzy_ranks_tighter_matches_first() {
        let mut m = matcher(|_| (), &["foobar", "fobar"]);
        m.search("fb", true);
        assert_eq!(m.matches, vec![1, 0]);
    }

    #[test]
    fn fuzzy_tolerates_typos() {
        let mut m = matcher(|_| (), &["firefox", "thunderbird"]);
        m.search("firefx", true);
        assert_eq!(m.matches, vec![0]);
        m.search("firxyx", true);
        assert!(m.matches.is_empty());
    }

    #[test]
    fn fuzzy_ties_keep_input_order() {
        let mut m = matcher(|_| (), &["foo", "foo"]);
        m.search("foo", true);
        assert_eq!(m.matches, vec![0, 1]);
    }

    #[test]
    fn exact_mode_lists_only_exact_labels() {
        let mut m = matcher(
            |c| c.match_mode = MatchMode::Exact,
            &["foo", "foobar", "{match=foo} hidden"],
        );
        m.search("foo", true);
        assert_eq!(m.matches, vec![0]);
    }

    #[test]
    fn value_is_hidden_from_label_and_search_but_used_for_output() {
        let item = Item::new("{value=one} same");
        assert_eq!(item.label(), "same");
        assert_eq!(item.output(), "one");
        assert_eq!(item.searchable_text(), "same");
        // value does not affect searchable text
        let with_match = Item::new("{value=two match=alt} Label");
        assert_eq!(with_match.label(), "Label");
        assert_eq!(with_match.output(), "two");
        assert_eq!(with_match.searchable_text(), "Label alt");

        // plain item falls back to label
        let plain = Item::new("plain");
        assert_eq!(plain.label(), "plain");
        assert_eq!(plain.output(), "plain");

        // matcher output helpers distinguish duplicates
        let mut m = matcher(|_| (), &["{value=one} same", "{value=two} same"]);
        m.search("same", true);
        assert_eq!(m.matches, vec![0, 1]);
        assert_eq!(m.text_of_match(0), "same");
        assert_eq!(m.output_of_match(0), "one");
        assert_eq!(m.output_of_match(1), "two");
    }
}
