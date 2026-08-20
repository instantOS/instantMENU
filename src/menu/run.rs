//! The menu event loop (port of `run()`).

use std::time::{Duration, Instant};

use super::transition::Transition;
use super::Menu;
use crate::backend::{BackendEvent, EventPoll, InputSource};
use crate::enums::ExitStatus;

impl Menu {
    /// run — port of the event loop in run(). Interprets the handlers'
    /// transitions; returns the exit status.
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
                match self.backend.poll_event(Some(remaining)) {
                    EventPoll::Event(BackendEvent::Destroyed) => return ExitStatus::Failure,
                    EventPoll::Event(BackendEvent::Expose) => {
                        self.backend.present(&self.canvas);
                    }
                    EventPoll::Event(_) => {
                        // Toast mode ignores all user input
                    }
                    EventPoll::Timeout => return ExitStatus::Success,
                    EventPoll::Closed => return ExitStatus::Failure,
                }
            }
        }

        let mut last_time: u32 = 0;
        let mut preselected = self.cfg.preselected;
        loop {
            let Some(ev) = self.backend.next_event() else {
                return ExitStatus::Failure;
            };

            if preselected != 0 {
                for _ in 0..preselected {
                    self.select_next();
                }
                self.draw_menu();
                preselected = 0;
            }

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
