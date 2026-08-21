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
    /// (instant/commented picks, pre-match, preselection).
    pub fn run(&mut self) -> ExitStatus {
        if self.slider.is_some() {
            return self.run_slide();
        }
        if self.cfg.toast != 0 {
            /* the CLI rejects negative --toast, but Config can be built
             * directly (tests, library use); clamp so the u64 multiply
             * below cannot overflow */
            let toast = self.cfg.toast.max(1);
            let deadline = Instant::now() + Duration::from_micros(toast as u64 * 100_000);
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

        /* whatever is already buffered lands before the first wait; for a
         * fast producer this is usually the whole corpus plus EOF */
        if self.stream_active() {
            self.drain_stdin();
        }

        let mut last_time: u32 = 0;
        let mut preselected = self.cfg.preselected;
        /* when streaming, --preselect applies at EOF against the full list */
        let deferred_preselect = self.stream_fd >= 0;
        loop {
            let now = Instant::now();
            if self.stream_settle_due(now) {
                if self.stream_eof {
                    if let Some(status) = self.finalize_stream() {
                        return status;
                    }
                } else {
                    self.settle_stream();
                }
            }

            if preselected != 0 && !deferred_preselect {
                for _ in 0..preselected {
                    self.select_next();
                }
                self.draw_menu();
                preselected = 0;
            }

            let extra_fds = [self.stream_fd];
            let extra: &[RawFd] = if self.stream_active() {
                &extra_fds
            } else {
                &[]
            };
            let timeout = self.stream_poll_budget(now);

            let ev = match self.backend.poll_event(timeout, extra) {
                EventPoll::Event(ev) => ev,
                EventPoll::Readable(_) => {
                    self.drain_stdin();
                    continue;
                }
                /* window closed: loop back and settle what arrived */
                EventPoll::Timeout => continue,
                EventPoll::Closed => return ExitStatus::Failure,
            };

            let t = match ev {
                BackendEvent::Motion { time, pos, .. } => {
                    if time.wrapping_sub(last_time) <= 1000 / 60 {
                        continue;
                    }
                    last_time = time;
                    self.set_selection(pos)
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
                    button, state, pos, ..
                } => self.button_press(button, state, pos),
                BackendEvent::ButtonRelease { .. } => continue,
                BackendEvent::Expose => {
                    self.backend.present(&self.canvas);
                    continue;
                }
                BackendEvent::FocusInOther => {
                    /* regrab focus from parent window */
                    let title = self.prompt().unwrap_or("dmenu").to_string();
                    self.backend.grab_focus(&title);
                    continue;
                }
                BackendEvent::KeyPress { sym, state, text } => self.key_press(sym, state, &text),
                BackendEvent::KeyRelease { sym, state } => self.key_release(sym, state),
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
}
