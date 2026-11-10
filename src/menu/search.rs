//! Matching orchestration: rank candidates, select an execution strategy,
//! install results and complete corpus-dependent hooks through one path.

mod worker;
use super::accept::{AcceptMode, AcceptTarget};
use super::matcher::{CorpusState, MatchResult, SearchControl};
use super::{paging, Menu, Transition};
use crate::enums::{EditOp, ExitStatus};
use std::time::{Duration, Instant, SystemTime};
pub(super) use worker::{ItemOrder, SearchWorker};

enum SearchDispatch {
    Ready(MatchResult),
    Pending,
}

impl Menu {
    pub(super) fn do_match(&mut self) -> Transition {
        let anchor = self.selection_anchor();
        let order = self.rank_candidates();
        match self.dispatch_search(order) {
            Ok(SearchDispatch::Ready(result)) => self.install_matches(result, anchor),
            Ok(SearchDispatch::Pending) => Transition::Nop,
            Err(error) => {
                eprintln!("instantmenu: cannot start search worker: {error}");
                Transition::Exit(ExitStatus::Failure)
            }
        }
    }

    fn rank_candidates(&mut self) -> ItemOrder {
        if !self.frecency_dirty {
            return ItemOrder::Unchanged;
        }
        self.frecency_dirty = false;
        let Some(frecency) = &self.frecency else {
            return ItemOrder::Unchanged;
        };
        let visible_ids = self.search.is_some().then(|| {
            self.matcher
                .matches
                .iter()
                .map(|&index| self.matcher.items[index].arrival_index)
                .collect::<Vec<_>>()
        });
        frecency.rank(&mut self.matcher.items, SystemTime::now());
        self.matcher.invalidate();
        if let Some(ids) = visible_ids {
            let positions: std::collections::HashMap<_, _> = self
                .matcher
                .items
                .iter()
                .enumerate()
                .map(|(index, item)| (item.arrival_index, index))
                .collect();
            self.matcher.matches = ids
                .into_iter()
                .filter_map(|id| positions.get(&id).copied())
                .collect();
        }
        ItemOrder::Changed
    }

    fn dispatch_search(&mut self, order: ItemOrder) -> std::io::Result<SearchDispatch> {
        let corpus = if self.stream_complete() {
            CorpusState::Complete
        } else {
            CorpusState::Streaming
        };
        if let Some(worker) = &mut self.search {
            worker.request(&self.matcher, &self.editor.text, corpus, order);
            // No new work means the installed matches are already current.
            // Reconcile selection intent without re-scoring or sorting.
            return Ok(if worker.is_busy() {
                SearchDispatch::Pending
            } else {
                SearchDispatch::Ready(MatchResult::Listed)
            });
        }
        if !self.background_search_enabled {
            return Ok(SearchDispatch::Ready(
                self.matcher.search(&self.editor.text, corpus),
            ));
        }
        let started = Instant::now();
        if let Some(result) = self.matcher.search_controlled(
            &self.editor.text,
            corpus,
            SearchControl {
                cancelled: &|| started.elapsed() >= Duration::from_millis(1),
                progress: None,
            },
        ) {
            return Ok(SearchDispatch::Ready(result));
        }
        let mut worker = SearchWorker::new(self.matcher.take_search_state())?;
        worker.request(
            &self.matcher,
            &self.editor.text,
            corpus,
            ItemOrder::Unchanged,
        );
        self.search = Some(worker);
        Ok(SearchDispatch::Pending)
    }

    /// Both inline and background EOF paths finish here. A hook may request
    /// another search (pre-match clears the seed); resume after that result.
    pub(super) fn finish_corpus(&mut self) -> Option<ExitStatus> {
        if !self.finalize_search_pending
            || self.search.as_ref().is_some_and(|worker| worker.is_busy())
        {
            return None;
        }
        if let Some(target) = self.pre_match_target.take() {
            self.select_identity(target);
        }
        if let Some(status) = self.apply_pre_match() {
            return Some(status);
        }
        if self.search.as_ref().is_some_and(|worker| worker.is_busy()) {
            return None;
        }
        self.apply_preselect();
        self.finalize_search_pending = false;
        None
    }

    pub(super) fn apply_pre_match(&mut self) -> Option<ExitStatus> {
        if !self.cfg.pre_match
            || self.pre_match_applied
            || !self.stream_complete()
            || self.matcher.matches.is_empty()
            || self.editor.text.is_empty()
            || self.initial_seed.as_deref() != Some(self.editor.text.as_str())
        {
            return None;
        }
        self.pre_match_applied = true;
        let target = self.matcher.items[self.matcher.matches[0]].arrival_index;
        let transition = self.insert(EditOp::Delete(self.editor.cursor));
        if let Some(status) = self.settle(transition) {
            return Some(status);
        }
        if self.search.as_ref().is_some_and(|worker| worker.is_busy()) {
            self.pre_match_target = Some(target);
        } else {
            self.select_identity(target);
        }
        None
    }

    pub(super) fn search_pending(&self) -> bool {
        self.search.as_ref().is_some_and(|search| {
            search.pending_query()
                || (self.cfg.reject_no_match && self.stream_complete() && search.is_busy())
        })
    }

    pub(super) fn poll_search(&mut self) -> Option<ExitStatus> {
        let response = match self.search.as_mut()?.take_result() {
            Ok(Some(response)) => response,
            Ok(None) => return None,
            Err(error) => {
                eprintln!("instantmenu: search failed: {error}");
                return Some(ExitStatus::Failure);
            }
        };
        if let Some((text, cursor)) = response
            .scan_finished
            .then(|| self.reject_snapshot.take())
            .flatten()
        {
            if response.matches.is_empty() {
                self.editor.text = text;
                self.editor.cursor = cursor;
                let _ = self.do_match();
                self.draw_menu();
                return None;
            }
        }
        let keep = self.selection_anchor();
        self.matcher.matches = response.matches;
        let t = self.install_matches(response.result, keep);
        if let Some(status) = self.perform(t) {
            return Some(status);
        }
        if response.scan_finished {
            if let Some(status) = self.finish_corpus() {
                return Some(status);
            }
        }
        self.draw_menu();
        None
    }
    pub(super) fn install_matches(
        &mut self,
        result: MatchResult,
        keep: Option<paging::SelectionAnchor>,
    ) -> Transition {
        match result {
            MatchResult::Listed => {
                self.restore_selection(keep);
                Transition::Nop
            }
            MatchResult::AutoConfirm(idx) => self.accept(AcceptTarget::Item(idx), AcceptMode::Exit),
            MatchResult::SingleKeyPick(pick) => match pick {
                Some(idx) => self.accept(AcceptTarget::Item(idx), AcceptMode::Exit),
                None => Transition::Exit(ExitStatus::Success),
            },
        }
    }
}
