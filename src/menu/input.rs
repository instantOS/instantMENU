//! Input pipeline shell glue: edits → smart-case → re-match → reject
//! revert (port of `insert()`), plus the blocking stdin load used when
//! items cannot stream (tty startup, toast mode).

use std::io::Read;

use super::matcher::Item;
use super::stream::LineParser;
use super::transition::Transition;
use super::Menu;
use crate::config::Config;
use crate::enums::{EditOp, ExitStatus};

impl Menu {
    /// Port of insert(): edit the text, re-match, and revert the edit when
    /// `reject` is set and the edit emptied the match list. Returns the
    /// transition the edit caused (an auto-confirm/commented pick can end the
    /// menu mid-edit).
    pub(super) fn insert_op(&mut self, op: EditOp, reject: bool) -> Transition {
        // only insertion can overflow the TEXT_MAX budget
        let last = reject.then(|| (self.editor.text.clone(), self.editor.cursor));
        if !self.editor.apply(op) {
            return Transition::Nop;
        }
        self.matcher.note_uppercase(&self.editor.text);

        let t = self.do_match();
        if matches!(t, Transition::Nop) && reject && self.matcher.matches.is_empty() {
            /* revert to last text value if theres no match */
            let (text, cursor) = last.expect("reject_no_match snapshot");
            self.editor.text = text;
            self.editor.cursor = cursor;
            return self.do_match();
        }
        t
    }

    /// insert() with the configured reject_no_match behaviour. While items
    /// are still streaming in the gate is off: an empty match list means
    /// "nothing arrived yet", not "no match", so typing must never be
    /// rejected against a partial corpus.
    pub(super) fn insert(&mut self, op: EditOp) -> Transition {
        let reject = self.cfg.reject_no_match && self.stream_complete();
        self.insert_op(op, reject)
    }

    /// -it — initial input text, applied with reject_no_match temporarily
    /// disabled (port of the insert() call in the argv loop; runs before any
    /// items exist, so this only seeds text/cursor/smartcase). The seed is
    /// remembered: a deferred --pre-match only fires if the user has not
    /// edited the text since.
    pub fn initial_text(&mut self, s: &str) -> Option<ExitStatus> {
        self.initial_seed = Some(s.to_string());
        let t = self.insert_op(EditOp::Insert(s), false);
        self.settle(t)
    }
}

/// read_stdin — the blocking load for paths where streaming does not apply
/// (interactive tty startup, toast mode). Same getline-per-line semantics as
/// the streamed parse: split on '\n' (a final chunk without trailing newline
/// is still an item), strip ONE trailing '\n' or '\t' byte per line, cut at
/// the first NUL like strdup would, drop invalid UTF-8.
pub fn read_stdin(cfg: &Config) -> Vec<Item> {
    if cfg.password || cfg.input_only || cfg.slide.is_some() {
        return Vec::new();
    }

    /* read each line from stdin and add it to the item list */
    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_err() {
        /* keep whatever we got, like getline erroring mid-way */
    }
    let mut parser = LineParser::default();
    let mut items: Vec<Item> = Vec::new();
    for line in parser.feed(&input) {
        items.push(Item::new(line));
    }
    for line in parser.finish() {
        items.push(Item::new(line));
    }
    if parser.spurious_lone_newline() {
        items.pop();
    }
    items
}
