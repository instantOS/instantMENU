//! The menu event loop (port of `run()`).

use std::time::Duration;

use super::Menu;
use crate::backend::BackendEvent;
use crate::enums::ExitStatus;

impl Menu {
    /// run — port of the event loop in run(). Interprets the handlers'
    /// transitions; returns the exit status.
    pub fn run(&mut self) -> ExitStatus {
        if self.cfg.toast != 0 {
            let toast = self.cfg.toast;
            std::thread::sleep(Duration::from_micros(toast as u64 * 100_000));
            return ExitStatus::Success;
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
                BackendEvent::Motion { time, pos } => {
                    if time.wrapping_sub(last_time) <= 1000 / 60 {
                        continue;
                    }
                    last_time = time;
                    self.set_selection(pos)
                }
                BackendEvent::Destroyed => return ExitStatus::Failure,
                BackendEvent::ButtonPress { button, state, pos } => {
                    self.button_press(button, state, pos)
                }
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
