//! The menu event loop (port of `run()`).

use std::time::Duration;

use super::Menu;
use crate::backend::BackendEvent;

impl Menu {
    /// run — port of the event loop in run().
    pub fn run(&mut self) {
        if self.cfg.toast != 0 {
            self.draw_menu();
            let toast = self.cfg.toast;
            std::thread::sleep(Duration::from_micros(toast as u64 * 100_000));
            std::process::exit(0);
        }

        let mut last_time: u32 = 0;
        let mut preselected = self.cfg.preselected;
        loop {
            let Some(ev) = self.backend.next_event() else {
                std::process::exit(1);
            };

            if preselected != 0 {
                for _ in 0..preselected {
                    if let Some(s) = self.selected {
                        if s + 1 < self.matches.len() {
                            self.selected = Some(s + 1);
                            if self.selected == self.next {
                                self.current = self.next;
                                self.calc_offsets();
                            }
                        }
                    }
                }
                self.draw_menu();
                preselected = 0;
            }

            match ev {
                BackendEvent::Motion { time, x, y } => {
                    if time.wrapping_sub(last_time) <= 1000 / 60 {
                        continue;
                    }
                    last_time = time;
                    self.set_selection(x, y);
                }
                BackendEvent::Destroyed => {
                    std::process::exit(1);
                }
                BackendEvent::ButtonPress { button, state, x, y } => {
                    self.button_press(button, state, x, y);
                }
                BackendEvent::Expose => {
                    self.backend.present(&self.canvas);
                }
                BackendEvent::FocusInOther => {
                    /* regrab focus from parent window */
                    let title = self
                        .prompt()
                        .unwrap_or("dmenu")
                        .to_string();
                    self.backend.grab_focus(&title);
                }
                BackendEvent::KeyPress { sym, state, text } => {
                    self.key_press(sym, state, &text);
                }
                BackendEvent::KeyRelease { sym, state } => {
                    self.key_release(sym, state);
                }
                BackendEvent::SelectionNotify { text } => {
                    self.paste(&text);
                }
                BackendEvent::VisibilityObscured => {
                    self.backend.raise();
                }
            }
        }
    }
}
