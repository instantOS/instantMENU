//! Search runs independently of the display connection. There is one pending
//! query and one published result, never a queue of obsolete full-list scans.
//! Candidate appends are transferred once; arrivals do not cancel a search of
//! the existing prefix. A changed query cancels between scoring chunks.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use super::matcher::{Item, MatchResult, Matcher};

struct Request {
    generation: u64,
    text: String,
    insensitive: bool,
    complete: bool,
}

pub(super) struct Response {
    generation: u64,
    pub complete: bool,
    count: usize,
    pub matches: Vec<usize>,
    pub result: MatchResult,
    pub finished: bool,
}

#[derive(Default)]
struct Mailbox {
    request: Option<Request>,
    append: Vec<Item>,
    replace: Option<Vec<Item>>,
    result: Option<Response>,
    stopped: bool,
}

pub(super) struct SearchWorker {
    shared: Arc<(Mutex<Mailbox>, Condvar)>,
    generation: Arc<AtomicU64>,
    wake: UnixStream,
    thread: Option<JoinHandle<()>>,
    text: String,
    insensitive: bool,
    count: usize,
    complete: bool,
    applied: u64,
    applied_count: usize,
    waiting: bool,
}

impl SearchWorker {
    pub fn new(policy: &Matcher, text: &str) -> std::io::Result<Self> {
        let (wake, mut notify) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        notify.set_nonblocking(true)?;
        let shared = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let generation = Arc::new(AtomicU64::new(0));
        let shared_worker = Arc::clone(&shared);
        let generation_worker = Arc::clone(&generation);
        let mut matcher = policy.policy_only();
        let handle = thread::Builder::new()
            .name("instantmenu-search".into())
            .spawn(move || {
                loop {
                    let (request, append, replace) = {
                        let (lock, ready) = &*shared_worker;
                        let mut mailbox = lock.lock().unwrap();
                        while mailbox.request.is_none() && !mailbox.stopped {
                            mailbox = ready.wait(mailbox).unwrap();
                        }
                        if mailbox.stopped {
                            return;
                        }
                        (
                            mailbox.request.take().unwrap(),
                            std::mem::take(&mut mailbox.append),
                            mailbox.replace.take(),
                        )
                    };
                    if let Some(items) = replace {
                        matcher.items = items;
                        matcher.invalidate();
                    }
                    matcher.items.extend(append);
                    matcher.set_insensitive(request.insensitive);
                    let cancelled =
                        || generation_worker.load(Ordering::Relaxed) != request.generation;
                    let mut publish = |response| {
                        shared_worker.0.lock().unwrap().result = Some(response);
                        let _ = notify.write(&[1]);
                    };
                    let Some(result) = matcher.search_progressively(
                        &request.text,
                        request.complete,
                        cancelled,
                        |matches, count| {
                            if !cancelled() {
                                publish(Response {
                                    generation: request.generation,
                                    complete: false,
                                    count,
                                    matches,
                                    result: MatchResult::Listed,
                                    finished: false,
                                });
                            }
                        },
                    ) else {
                        continue;
                    };
                    if cancelled() {
                        continue;
                    }
                    let response = Response {
                        generation: request.generation,
                        complete: request.complete,
                        count: matcher.items.len(),
                        matches: std::mem::take(&mut matcher.matches),
                        result,
                        finished: true,
                    };
                    // A full notification socket already contains a wakeup. The
                    // result lives in the mailbox, so notifications may coalesce.
                    publish(response);
                }
            })?;
        Ok(Self {
            shared,
            generation,
            wake,
            thread: Some(handle),
            text: text.to_string(),
            insensitive: policy.insensitive(),
            count: 0,
            complete: false,
            applied: 0,
            applied_count: 0,
            waiting: false,
        })
    }

    pub fn request(&mut self, source: &Matcher, text: &str, complete: bool, reordered: bool) {
        let replace = reordered || source.items.len() < self.count;
        let changed = self.text != text || self.insensitive != source.insensitive() || replace;
        if !changed && self.count == source.items.len() && self.complete == complete && self.waiting
        {
            return;
        }
        if changed {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        let append = if replace {
            Vec::new()
        } else {
            source.items[self.count..].to_vec()
        };
        let replacement = replace.then(|| source.items.clone());
        self.text = text.to_string();
        self.insensitive = source.insensitive();
        self.count = source.items.len();
        self.complete = complete;
        self.waiting = true;
        let (lock, ready) = &*self.shared;
        let mut mailbox = lock.lock().unwrap();
        if let Some(items) = replacement {
            mailbox.replace = Some(items);
            mailbox.append.clear();
        }
        mailbox.append.extend(append);
        mailbox.request = Some(Request {
            generation: self.generation.load(Ordering::Relaxed),
            text: self.text.clone(),
            insensitive: self.insensitive,
            complete,
        });
        ready.notify_one();
    }

    pub fn pending_query(&self) -> bool {
        self.applied != self.generation.load(Ordering::Relaxed)
            || (self.applied_count == 0 && self.count > 0)
    }

    pub fn fd(&self) -> Option<RawFd> {
        self.waiting.then(|| self.wake.as_raw_fd())
    }

    pub fn take_result(&mut self) -> Option<Response> {
        let mut buffer = [0; 256];
        while self.wake.read(&mut buffer).is_ok_and(|n| n > 0) {}
        let response = self.shared.0.lock().unwrap().result.take()?;
        if response.generation != self.generation.load(Ordering::Relaxed) {
            return None;
        }
        self.applied = response.generation;
        self.applied_count = response.count;
        self.waiting = !response.finished
            || response.count != self.count
            || response.complete != self.complete;
        Some(response)
    }
}

use super::Menu;
use crate::backend::BackendEvent;
use crate::enums::ExitStatus;
use xkbcommon::xkb::keysyms as ks;

impl Menu {
    pub(super) fn search_pending(&self) -> bool {
        self.search.as_ref().is_some_and(|search| {
            search.pending_query()
                || (self.cfg.reject_no_match && self.stream_complete() && search.fd().is_some())
        })
    }

    /// Edits supersede pending queries, while actions such as Return must
    /// use results for the current query. Cancellation never waits for search.
    pub(super) fn defer_for_search(&self, event: &BackendEvent) -> bool {
        if !self.search_pending() {
            return false;
        }
        match event {
            BackendEvent::KeyPress { sym, mods, text } => {
                if *sym == ks::KEY_Escape || (mods.ctrl && matches!(*sym, ks::KEY_c | ks::KEY_g)) {
                    return false;
                }
                if self.cfg.reject_no_match && self.stream_complete() {
                    return true;
                }
                if self
                    .cfg
                    .bindings
                    .iter()
                    .any(|binding| binding.matches(*sym, *mods))
                {
                    return true;
                }
                if !mods.ctrl && !mods.alt && !mods.logo {
                    return !text.chars().any(|c| !c.is_control())
                        && !matches!(
                            *sym,
                            ks::KEY_BackSpace
                                | ks::KEY_Delete
                                | ks::KEY_Left
                                | ks::KEY_Right
                                | ks::KEY_Home
                                | ks::KEY_End
                        );
                }
                !(mods.ctrl
                    && matches!(
                        *sym,
                        ks::KEY_a
                            | ks::KEY_b
                            | ks::KEY_d
                            | ks::KEY_e
                            | ks::KEY_f
                            | ks::KEY_h
                            | ks::KEY_k
                            | ks::KEY_u
                            | ks::KEY_w
                    ))
            }
            BackendEvent::KeyRelease { sym, .. } => matches!(*sym, ks::KEY_Alt_L | ks::KEY_Alt_R),
            _ => false,
        }
    }

    pub(super) fn poll_search(&mut self) -> Option<ExitStatus> {
        let was_pending = self.search_pending();
        let response = self.search.as_mut()?.take_result()?;
        if let Some((text, cursor)) = response
            .finished
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
        let keep = (!was_pending || !self.stream_complete())
            .then_some(self.selection.selected)
            .flatten()
            .and_then(|pos| self.matcher.matches.get(pos))
            .map(|&idx| self.matcher.items[idx].arrival_index);
        self.matcher.matches = response.matches;
        let t = self.install_matches(response.result, keep);
        if let Some(status) = self.perform(t) {
            return Some(status);
        }
        if let Some(target) = self.pre_match_target.take() {
            if let Some(pos) = self
                .matcher
                .matches
                .iter()
                .position(|&i| self.matcher.items[i].arrival_index == target)
            {
                self.selection.selected = Some(pos);
                self.selection.page_start = Some(pos);
                self.recalc_paging();
            }
        }
        if self.finalize_search_pending
            && response.complete
            && self
                .search
                .as_ref()
                .is_some_and(|search| search.fd().is_none())
        {
            if let Some(status) = self.apply_pre_match() {
                return Some(status);
            }
            if !self.search_pending() {
                self.apply_preselect();
                self.finalize_search_pending = false;
            }
        }
        self.draw_menu();
        None
    }
}

impl Drop for SearchWorker {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.shared.0.lock().unwrap().stopped = true;
        self.shared.1.notify_one();
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}
