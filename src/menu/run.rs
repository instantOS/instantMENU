//! The menu event loop (port of `run()`).

use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use super::transition::Transition;
use super::Menu;
use crate::backend::{BackendEvent, EventPoll, InputSource};
use crate::enums::ExitStatus;

impl Menu {
    /// run — port of the event loop in run(). Interprets the handlers'
    /// transitions; returns the exit status.
    ///
    /// While stdin streams items in (`begin_stream`), the loop polls the
    /// stdin fd alongside the backend and settles coalesced batches at the
    /// top of every iteration — so handler `continue`s can never starve a
    /// pending settle. EOF settles once with the deferred conclusions
    /// (instant/single-key picks, pre-match, preselection).
    pub fn run(&mut self) -> ExitStatus {
        if self.slider.is_some() {
            return self.run_slide();
        }
        if let Some(toast) = self.cfg.toast {
            return self.run_toast(toast);
        }
        // Small searches complete inline. Only a search that exhausts its
        // event-loop time slice creates a worker and its notification socket.
        self.background_search_enabled = true;

        /* with a complete (non-streamed) corpus, --preselect applies before
         * the first event; streaming defers it to EOF against the full list */
        let mut preselect_pending = self.cfg.preselect.is_some() && self.stream_fd < 0;
        loop {
            if let Some(status) = self.poll_search() {
                return status;
            }
            /* One bounded input turn per iteration, even when backend events
             * are continuously queued. Backend polling must in turn service
             * ready display events before reporting stdin readiness. Neither
             * a busy producer nor a busy pointer can monopolize the loop. */
            if self.stream_active() {
                self.drain_stdin();
            }
            let now = Instant::now();
            if self.stream_settle_due(now) {
                if self.stream_eof {
                    if let Some(status) = self.finalize_stream() {
                        return status;
                    }
                } else {
                    if let Some(status) = self.settle_stream() {
                        return status;
                    }
                }
            }

            if preselect_pending {
                preselect_pending = false;
                self.apply_preselect();
                self.draw_menu();
            }

            let mut extra: [RawFd; 2] = [-1; 2];
            let mut extra_len = 0;
            if self.stream_active() {
                extra[extra_len] = self.stream_fd;
                extra_len += 1;
            }
            if let Some(fd) = self.search.as_ref().and_then(|search| search.fd()) {
                extra[extra_len] = fd;
                extra_len += 1;
            }
            let timeout = self.stream_poll_budget(Instant::now());

            let deferred = if !self.search_pending() {
                self.deferred_events.pop_front()
            } else {
                None
            };
            let ev = match deferred
                .map(EventPoll::Event)
                .unwrap_or_else(|| self.backend.poll_event(timeout, &extra[..extra_len]))
            {
                EventPoll::Event(ev) => ev,
                EventPoll::Readable(_) => continue,
                /* window closed: loop back and settle what arrived */
                EventPoll::Timeout => continue,
                EventPoll::Closed => return ExitStatus::Failure,
            };
            if self.defer_for_search(&ev) {
                self.deferred_events.push_back(ev);
                continue;
            }

            let t = match ev {
                /* Wheel detents and pointer motion both only ever ask for a
                 * repaint, and a fast flick arrives with them interleaved
                 * (scroll while wiggling the mouse). Fold the whole leading
                 * run into one frame: every event is still applied in order,
                 * so the state is identical — only the intermediate paints are
                 * skipped. set_selection stays cheap within a row, and the
                 * trailing motion is never dropped, so a pointer that comes to
                 * rest still ends up with the highlight under it. */
                ev @ (BackendEvent::Scroll { .. } | BackendEvent::Motion { .. }) => {
                    let mut batch = vec![ev];
                    batch.extend(self.backend.drain_repaint());
                    self.apply_repaint_batch(&batch)
                }
                BackendEvent::Destroyed => return ExitStatus::Failure,
                BackendEvent::ButtonPress {
                    source: InputSource::External,
                    ..
                } => {
                    /* click outside the modal menu (pointer grab on X11,
                     * shield surface on Wayland): dismiss, like a GTK
                     * context menu loses its grab */
                    Transition::Exit(ExitStatus::Failure)
                }
                BackendEvent::ButtonPress {
                    button, mods, pos, ..
                } => self.button_press(button, mods, pos),
                BackendEvent::ButtonRelease { .. } => continue,
                BackendEvent::Expose => {
                    self.backend.present(&self.canvas);
                    continue;
                }
                BackendEvent::FocusInOther => {
                    /* regrab focus from parent window */
                    let title = self.prompt().unwrap_or("dmenu").to_string();
                    if self.backend.grab_focus(&title).is_err() {
                        return ExitStatus::Failure;
                    }
                    continue;
                }
                BackendEvent::KeyboardLeft => {
                    /* a release-driven cycle can no longer complete */
                    self.keyboard_left();
                    continue;
                }
                BackendEvent::KeyPress { sym, mods, text } => self.key_press(sym, mods, &text),
                BackendEvent::KeyRelease { sym, mods } => self.key_release(sym, mods),
                BackendEvent::SelectionNotify { text } => self.paste(&text),
                BackendEvent::VisibilityObscured => {
                    self.backend.raise();
                    continue;
                }
            };
            if let Some(status) = self.perform(t) {
                return status;
            }
        }
    }

    /// Toast mode: a passive display for `secs` seconds, then success.
    /// No input is interpreted — expose events keep the toast visible,
    /// destruction or a dead connection fails, anything else (including
    /// stdin data) ends it early like the C version's select() loop.
    fn run_toast(&mut self, secs: f32) -> ExitStatus {
        /* the CLI rejects negative --toast, but Config can be built
         * directly (tests, library use); clamp so the duration stays
         * valid */
        let deadline = Instant::now() + Duration::from_secs_f64(f64::from(secs).max(0.1));
        loop {
            let now = Instant::now();
            if now >= deadline {
                return ExitStatus::Success;
            }
            let remaining = deadline - now;
            match self.backend.poll_event(Some(remaining), &[]) {
                EventPoll::Event(BackendEvent::Destroyed) => return ExitStatus::Failure,
                EventPoll::Event(BackendEvent::Expose) => {
                    self.backend.present(&self.canvas);
                }
                EventPoll::Event(_) => {
                    // Toast mode ignores all user input
                }
                EventPoll::Readable(_) | EventPoll::Timeout => return ExitStatus::Success,
                EventPoll::Closed => return ExitStatus::Failure,
            }
        }
    }
}
