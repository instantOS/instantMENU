//! One pending query and one published result. Candidate updates coalesce
//! without dropping appends, while a new query cancels obsolete scoring.

use super::super::matcher::{
    CorpusState, Item, MatchResult, Matcher, SearchControl, SearchProgress,
};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

#[derive(Clone, Copy)]
pub(in crate::menu) enum ItemOrder {
    Unchanged,
    Changed,
}

struct Request {
    generation: u64,
    text: String,
    insensitive: bool,
    corpus: CorpusState,
}
struct Target {
    text: String,
    insensitive: bool,
    count: usize,
    corpus: CorpusState,
}

pub(in crate::menu) struct Response {
    generation: u64,
    pub corpus: CorpusState,
    count: usize,
    pub matches: Vec<usize>,
    pub result: MatchResult,
    pub scan_finished: bool,
}

enum CorpusUpdate {
    Append(Vec<Item>),
    Replace(Vec<Item>),
}

impl CorpusUpdate {
    fn followed_by(self, update: Self) -> Self {
        let Self::Append(additions) = update else {
            return update;
        };
        match self {
            Self::Append(mut items) => {
                items.extend(additions);
                Self::Append(items)
            }
            Self::Replace(mut items) => {
                items.extend(additions);
                Self::Replace(items)
            }
        }
    }
}

struct Work {
    request: Request,
    update: CorpusUpdate,
}

#[derive(Default)]
struct Mailbox {
    work: Option<Work>,
    result: Option<Response>,
    stopped: bool,
}

#[derive(Default)]
struct Shared {
    mailbox: Mutex<Mailbox>,
    ready: Condvar,
}

pub(in crate::menu) struct SearchWorker {
    shared: Arc<Shared>,
    generation: Arc<AtomicU64>,
    wake: UnixStream,
    thread: Option<JoinHandle<()>>,
    target: Option<Target>,
    query_ready: bool,
    waiting: bool,
}

impl SearchWorker {
    pub fn new(matcher: Matcher) -> std::io::Result<Self> {
        let (wake, notify) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        notify.set_nonblocking(true)?;
        let shared = Arc::new(Shared::default());
        let generation = Arc::new(AtomicU64::new(0));
        let worker_shared = Arc::clone(&shared);
        let worker_generation = Arc::clone(&generation);
        let handle = thread::Builder::new()
            .name("instantmenu-search".into())
            .spawn(move || run_worker(matcher, worker_shared, worker_generation, notify))?;
        Ok(Self {
            shared,
            generation,
            wake,
            thread: Some(handle),
            target: None,
            query_ready: true,
            waiting: false,
        })
    }

    pub fn request(&mut self, source: &Matcher, text: &str, corpus: CorpusState, order: ItemOrder) {
        let previous_count = self.target.as_ref().map_or(0, |target| target.count);
        let replace = matches!(order, ItemOrder::Changed) || source.items.len() < previous_count;
        let same_query = self
            .target
            .as_ref()
            .is_some_and(|target| target.text == text && target.insensitive == source.insensitive);
        let unchanged = same_query
            && self.target.as_ref().is_some_and(|target| {
                target.count == source.items.len() && target.corpus == corpus
            });
        // Completed requests are cached too; a redraw is not a new search.
        if unchanged && !replace {
            return;
        }
        if !same_query || replace {
            self.generation.fetch_add(1, Ordering::Relaxed);
            self.query_ready = false;
        }
        let update = if replace {
            CorpusUpdate::Replace(source.items.clone())
        } else {
            CorpusUpdate::Append(source.items[previous_count..].to_vec())
        };
        self.target = Some(Target {
            text: text.to_owned(),
            insensitive: source.insensitive,
            count: source.items.len(),
            corpus,
        });
        self.waiting = true;
        let mut mailbox = self.shared.mailbox.lock().unwrap();
        let update = match mailbox.work.take() {
            Some(work) => work.update.followed_by(update),
            None => update,
        };
        mailbox.work = Some(Work {
            request: Request {
                generation: self.generation.load(Ordering::Relaxed),
                text: text.to_owned(),
                insensitive: source.insensitive,
                corpus,
            },
            update,
        });
        self.shared.ready.notify_one();
    }

    pub fn pending_query(&self) -> bool {
        !self.query_ready
    }

    pub fn is_busy(&self) -> bool {
        self.waiting
    }

    pub fn fd(&self) -> RawFd {
        self.wake.as_raw_fd()
    }

    pub fn take_result(&mut self) -> std::io::Result<Option<Response>> {
        let mut buffer = [0; 256];
        loop {
            match self.wake.read(&mut buffer) {
                Ok(0) => return Err(std::io::Error::other("search worker disconnected")),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        let Some(response) = self.shared.mailbox.lock().unwrap().result.take() else {
            return Ok(None);
        };
        if response.generation != self.generation.load(Ordering::Relaxed) {
            return Ok(None);
        }
        // Empty provisional output cannot establish "no match" for Return.
        self.query_ready = response.scan_finished || !response.matches.is_empty();
        let target = self
            .target
            .as_ref()
            .expect("a result has a requested target");
        self.waiting = !response.scan_finished
            || response.count != target.count
            || response.corpus != target.corpus;
        Ok(Some(response))
    }
}

fn run_worker(
    mut matcher: Matcher,
    shared: Arc<Shared>,
    generation: Arc<AtomicU64>,
    mut notify: UnixStream,
) {
    loop {
        let work = {
            let mut mailbox = shared.mailbox.lock().unwrap();
            while mailbox.work.is_none() && !mailbox.stopped {
                mailbox = shared.ready.wait(mailbox).unwrap();
            }
            if mailbox.stopped {
                return;
            }
            mailbox.work.take().unwrap()
        };
        match work.update {
            CorpusUpdate::Append(items) => matcher.items.extend(items),
            CorpusUpdate::Replace(items) => {
                matcher.items = items;
                matcher.invalidate();
            }
        }
        let request = work.request;
        matcher.insensitive = request.insensitive;
        let cancelled = || generation.load(Ordering::Relaxed) != request.generation;
        let mut publish = |response| {
            shared.mailbox.lock().unwrap().result = Some(response);
            // A full socket already contains a wakeup; the mailbox owns data.
            let _ = notify.write(&[1]);
        };
        let Some(result) = matcher.search_controlled(
            &request.text,
            request.corpus,
            SearchControl {
                cancelled: &cancelled,
                progress: Some(&mut |SearchProgress { matches, scanned }| {
                    if !cancelled() {
                        publish(Response {
                            generation: request.generation,
                            corpus: request.corpus,
                            count: scanned,
                            matches,
                            result: MatchResult::Listed,
                            scan_finished: false,
                        });
                    }
                }),
            },
        ) else {
            continue;
        };
        if cancelled() {
            continue;
        }
        publish(Response {
            generation: request.generation,
            corpus: request.corpus,
            count: matcher.items.len(),
            matches: std::mem::take(&mut matcher.matches),
            result,
            scan_finished: true,
        });
    }
}

impl Drop for SearchWorker {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.shared.mailbox.lock().unwrap().stopped = true;
        self.shared.ready.notify_one();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::time::{Duration, Instant};

    fn finished(worker: &mut SearchWorker) -> Response {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(response) = worker.take_result().unwrap() {
                if response.scan_finished {
                    return response;
                }
            }
            assert!(Instant::now() < deadline, "search did not finish");
            let mut fd = [crate::backend::poll::poll_in(worker.fd())];
            let _ = crate::backend::poll::poll_fds(&mut fd, 100);
        }
    }

    #[test]
    fn worker_keeps_appends_when_queries_are_superseded() {
        let mut source = Matcher::new(
            vec![Item::new("alpha"), Item::new("beta")],
            &Config::default(),
        );
        let mut worker = SearchWorker::new(source.take_search_state()).unwrap();
        worker.request(
            &source,
            "alpha",
            CorpusState::Streaming,
            ItemOrder::Unchanged,
        );
        source.items.push(Item::new("gamma"));
        worker.request(
            &source,
            "beta",
            CorpusState::Streaming,
            ItemOrder::Unchanged,
        );
        source
            .items
            .push(Item::new("{match=target value=result} chosen"));
        worker.request(
            &source,
            "target",
            CorpusState::Complete,
            ItemOrder::Unchanged,
        );
        let response = finished(&mut worker);
        assert_eq!(response.matches, vec![3]);
        assert!(response.corpus.is_complete());
        assert!(!worker.pending_query());
        assert!(!worker.is_busy());
    }

    #[test]
    fn worker_invalidates_scores_after_reordering_or_case_change() {
        let mut source = Matcher::new(
            vec![Item::new("UPPER"), Item::new("upper")],
            &Config::default(),
        );
        let mut worker = SearchWorker::new(source.take_search_state()).unwrap();
        worker.request(
            &source,
            "UPPER",
            CorpusState::Complete,
            ItemOrder::Unchanged,
        );
        assert_eq!(finished(&mut worker).matches, vec![0]);
        source.items.swap(0, 1);
        worker.request(&source, "UPPER", CorpusState::Complete, ItemOrder::Changed);
        assert_eq!(finished(&mut worker).matches, vec![1]);
        source.insensitive = true;
        worker.request(
            &source,
            "UPPER",
            CorpusState::Complete,
            ItemOrder::Unchanged,
        );
        assert_eq!(finished(&mut worker).matches.len(), 2);
    }

    #[test]
    fn automatic_conclusions_use_the_final_corpus() {
        let mut source = Matcher::new(
            vec![Item::new("target")],
            &Config {
                auto_confirm: true,
                ..Config::default()
            },
        );
        let mut worker = SearchWorker::new(source.take_search_state()).unwrap();
        worker.request(
            &source,
            "target",
            CorpusState::Streaming,
            ItemOrder::Unchanged,
        );
        assert_eq!(finished(&mut worker).result, MatchResult::Listed);
        source.items.push(Item::new("target two"));
        worker.request(
            &source,
            "target",
            CorpusState::Complete,
            ItemOrder::Unchanged,
        );
        assert_eq!(finished(&mut worker).result, MatchResult::Listed);
        worker.request(
            &source,
            "target two",
            CorpusState::Complete,
            ItemOrder::Unchanged,
        );
        assert_eq!(finished(&mut worker).result, MatchResult::AutoConfirm(1));
    }

    #[test]
    fn unchanged_completed_requests_do_not_repeat_work() {
        let mut source = Matcher::new(vec![Item::new("alpha")], &Config::default());
        let mut worker = SearchWorker::new(source.take_search_state()).unwrap();
        worker.request(
            &source,
            "alpha",
            CorpusState::Complete,
            ItemOrder::Unchanged,
        );
        assert_eq!(finished(&mut worker).matches, vec![0]);
        worker.request(
            &source,
            "alpha",
            CorpusState::Complete,
            ItemOrder::Unchanged,
        );
        assert!(!worker.is_busy());
        assert!(worker.shared.mailbox.lock().unwrap().work.is_none());
    }
}
