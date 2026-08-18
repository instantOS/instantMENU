//! Window geometry, monitor selection and menu setup.

use super::Menu;
use crate::backend::MonitorInfo;
use crate::enums::Scheme;

/// INTERSECT macro: overlap area of rect (x,y,w,h) with a monitor.
fn intersect_area(x: i32, y: i32, w: i32, h: i32, monitor: &MonitorInfo) -> i32 {
    (0.max((x + w).min(monitor.x + monitor.width) - x.max(monitor.x)))
        * (0.max((y + h).min(monitor.y + monitor.height) - y.max(monitor.y)))
}

impl Menu {
    /// setup — geometry, monitor selection, window creation, first draw.
    pub fn setup(&mut self) {
        let (x, y) = self.compute_geometry();
        self.input_width = self.menu_width / (if self.cfg.commented { 10 } else { 3 });
        self.do_match();
        self.apply_pre_match();
        self.x = x;
        self.y = y;
        self.create_window(x, y);
    }

    /// Compute the menu size and (x, y) position.
    fn compute_geometry(&mut self) -> (i32, i32) {
        self.bar_height = self.renderer.font_height + 12;
        self.bar_height = self.bar_height.max(self.cfg.line_height); /* make a menu line AT LEAST 'line_height' tall */

        self.cfg.lines = self.cfg.lines.max(0);
        self.menu_height = (self.cfg.lines + 1) * self.bar_height;
        let prompt_width = if self.cfg.commented {
            self.bar_height * 15
        } else {
            match self.prompt().map(|p| p.to_string()) {
                Some(p) if !p.is_empty() => {
                    let w = self.text_width(&p);
                    w - self.renderer.horizontal_padding / 4
                }
                _ => 0,
            }
        };
        self.prompt_width = prompt_width;

        let monitors: Vec<MonitorInfo> = self.backend.monitors().to_vec();
        let (root_width, root_height) = self.backend.root_size();

        if monitors.is_empty() {
            self.embed_geometry(root_width, root_height)
        } else {
            let i = self.select_monitor(&monitors);
            let monitor = &monitors[i];
            self.monitor_geometry(monitor, root_width, root_height)
        }
    }

    /// Pick the monitor index from `-m`, the focused monitor, or the pointer.
    fn select_monitor(&mut self, monitors: &[MonitorInfo]) -> usize {
        let n = monitors.len() as i32;
        let mut i = 0usize;
        let mut area_found = false;
        if self.cfg.monitor >= 0 && self.cfg.monitor < n {
            i = self.cfg.monitor as usize;
        } else if let Some(focused) = self.backend.focused_monitor() {
            if focused < monitors.len() {
                i = focused;
                area_found = true;
            }
        }
        if self.cfg.monitor < 0 && !area_found {
            if let Some((px, py)) = self.backend.pointer_position() {
                for (idx, monitor) in monitors.iter().enumerate() {
                    if intersect_area(px, py, 1, 1, monitor) != 0 {
                        i = idx;
                        break;
                    }
                }
            }
        }
        i
    }

    /// Geometry on a selected monitor (centered / follow_cursor / offset).
    fn monitor_geometry(
        &mut self,
        monitor: &MonitorInfo,
        root_width: i32,
        root_height: i32,
    ) -> (i32, i32) {
        let mut x = 0;
        let mut y = 0;

        if self.cfg.centered {
            if self.cfg.width != 0 && self.cfg.width < monitor.width {
                self.menu_width = self.cfg.width;
            } else {
                self.menu_width = monitor.width - 100;
            }

            while (self.cfg.lines + 1) * self.bar_height > monitor.height {
                self.cfg.lines -= 1;
            }

            self.menu_height = (self.cfg.lines + 1) * self.bar_height;
            x = monitor.x + (monitor.width - self.menu_width) / 2;
            y = monitor.y + (monitor.height - self.menu_height) / 2;

            if y < 0 {
                y = 0;
            }
        } else if self.cfg.follow_cursor {
            if self.cfg.width != 0 {
                self.menu_width = self.cfg.width;
            } else {
                // MIN(MAX(max_text_width() + prompt_width, min_width), wa.width);
                // `wa` still holds the root attributes here in the C code.
                let max_width = (self.max_text_width() + self.prompt_width)
                    .max(self.cfg.min_width)
                    .min(root_width);
                self.menu_width = max_width;
            }
            if let Some((px, py)) = self.backend.pointer_position() {
                x = px;
                y = py;
                if x > monitor.x + (root_width - monitor.x) / 2 {
                    x = x - self.menu_width + 20;
                } else {
                    x = x - 20;
                }
                if y > monitor.y + (root_height - monitor.y) / 2 {
                    y = y - self.menu_height + 20;
                } else {
                    y = y - 20;
                }

                if x < 0 {
                    x = 0;
                }
                if y < 0 {
                    y = 0;
                }
            }
        } else {
            if self.cfg.y_offset <= -1 {
                if self.cfg.y_offset == -1 {
                    self.cfg.y_offset = (monitor.height - self.menu_height) / 2;
                } else {
                    self.cfg.y_offset = (self.renderer.font_height as f32 * 1.55) as i32;
                }
            }
            self.menu_width = if self.cfg.width > 0 && self.cfg.width < monitor.width {
                self.cfg.width
            } else {
                monitor.width
            };
            if self.cfg.x_offset == -1 {
                self.cfg.x_offset = (monitor.width - self.menu_width) / 2;
            }
            x = if self.cfg.right_x_offset {
                monitor.x + monitor.width - self.cfg.x_offset - self.menu_width - 2 * self.cfg.border_width
            } else {
                monitor.x + self.cfg.x_offset
            };
            y = monitor.y
                + if self.cfg.top_bar {
                    self.cfg.y_offset
                } else {
                    monitor.height - self.menu_height - self.cfg.y_offset
                };
        }

        self.adjust_geometry(monitor, root_width, root_height, &mut x, &mut y);
        (x, y)
    }

    /// Clamp the computed geometry to the monitor/root and apply full_height.
    fn adjust_geometry(
        &mut self,
        monitor: &MonitorInfo,
        root_width: i32,
        root_height: i32,
        x: &mut i32,
        y: &mut i32,
    ) {
        if self.menu_height > root_height - 10 {
            self.menu_height = root_height - self.cfg.border_width * 2 - 10;
            self.cfg.lines = root_height
                / (if self.cfg.line_height != 0 {
                    self.cfg.line_height
                } else {
                    self.bar_height
                })
                - 1;
        }

        if self.menu_width > root_width - 10 {
            self.menu_width = root_width - self.cfg.border_width * 2;
        }

        if *x < monitor.x {
            *x = monitor.x;
        }
        if *x + self.menu_width > monitor.x + monitor.width {
            *x = monitor.x + monitor.width - self.menu_width - self.cfg.border_width * 2;
        }
        if self.cfg.full_height {
            *y = monitor.y + 32;
            self.menu_height =
                root_height - self.cfg.border_width * 2 - (root_height - monitor.height + 32);
            self.cfg.lines = root_height / self.cfg.line_height - 2;
        } else if *y + self.menu_height > root_height {
            *y = root_height - self.menu_height;
        }
    }

    /// Geometry when embedding into a parent window (`-W`, no monitor info).
    fn embed_geometry(&mut self, root_width: i32, root_height: i32) -> (i32, i32) {
        let Some((parent_width, parent_height)) = self.backend.embed_parent_size() else {
            self.finish(1);
        };
        let mut x = 0;
        let mut y = 0;
        if self.cfg.centered {
            let max_width = (self.max_text_width() + self.prompt_width)
                .max(self.cfg.min_width)
                .min(parent_width);
            self.menu_width = max_width;
            x = (parent_width - self.menu_width) / 2;
            y = (parent_height - self.menu_height) / 2;
        } else if self.cfg.follow_cursor {
            if let Some((px, py)) = self.backend.pointer_position() {
                x = px;
                y = py;
                if x > root_width / 2 {
                    x -= self.menu_width;
                }
                if y > root_height / 2 {
                    y -= self.menu_height;
                }
            }
            let max_width = (self.max_text_width() + self.prompt_width)
                .max(self.cfg.min_width)
                .min(parent_width);
            self.menu_width = max_width;
        } else {
            x = self.cfg.x_offset;
            y = if self.cfg.top_bar {
                self.cfg.y_offset
            } else {
                parent_height - self.menu_height - self.cfg.y_offset
            };
            self.menu_width = if self.cfg.width > 0 && self.cfg.width < parent_width {
                self.cfg.width
            } else {
                parent_width
            };
        }
        (x, y)
    }

    /// Prematch: select the item that first matched the pretyped text.
    fn apply_pre_match(&mut self) {
        if self.cfg.pre_match && !self.matches.is_empty() && !self.text.is_empty() {
            // remember the item that was the first match for the pretyped text
            let first_match_item = *self.matches.first().unwrap();
            let cursor = self.cursor as i32;
            self.insert(None, -cursor);
            // selected = that item (find its position in the rebuilt match list)
            self.selected = self.matches.iter().position(|&it| it == first_match_item);
            if let Some(next_pos) = self.next {
                for pos in next_pos..self.matches.len() {
                    if self.matches[pos] == first_match_item {
                        self.current = self.selected;
                        break;
                    }
                }
            }
            self.calc_offsets();
            self.cfg.pre_match = false;
        }
    }

    /// Create the window, set the title, map it and draw the first frame.
    fn create_window(&mut self, x: i32, y: i32) {
        let managed = self.cfg.managed;
        let class = if managed { "floatmenu" } else { "dmenu" };
        let bg = self.renderer.color_scheme(Scheme::Normal).bg;
        let border_color = self.renderer.color_scheme(Scheme::Selected).bg;
        if self
            .backend
            .create_window(
                x,
                y,
                self.menu_width,
                self.menu_height,
                self.cfg.border_width,
                managed,
                !self.cfg.no_grab && self.cfg.toast == 0,
                class,
                bg,
                border_color,
            )
            .is_err()
        {
            eprintln!("instantmenu: cannot create window");
            std::process::exit(1);
        }

        if managed {
            let title = self
                .cfg
                .search_text
                .clone()
                .unwrap_or_else(|| "menu".to_string());
            self.backend.set_title(&title);
        }

        self.backend.map_window();
        if self.cfg.embed.is_some() {
            self.backend.embed_setup(x, y);
        }
        self.canvas.resize(self.menu_width, self.menu_height);
        self.draw_menu();
    }
}
