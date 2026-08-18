//! Window geometry, monitor selection and menu setup.

use super::Menu;
use crate::backend::MonitorInfo;
use crate::enums::{Scheme, COL_BG};

/// INTERSECT macro: overlap area of rect (x,y,w,h) with a monitor.
fn intersect_area(x: i32, y: i32, w: i32, h: i32, mon: &MonitorInfo) -> i32 {
    (0.max((x + w).min(mon.x + mon.width) - x.max(mon.x)))
        * (0.max((y + h).min(mon.y + mon.height) - y.max(mon.y)))
}

impl Menu {
    /// setup — geometry, monitor selection, window creation, first draw.
    pub fn setup(&mut self) {
        let (x, y) = self.compute_geometry();
        self.inputw = self.mw / (if self.cfg.commented { 10 } else { 3 }); /* input width: ~33% of monitor width */
        self.do_match();
        self.apply_prematch();
        self.x = x;
        self.y = y;
        self.create_window(x, y);
    }

    /// Compute the menu size and (x, y) position.
    fn compute_geometry(&mut self) -> (i32, i32) {
        self.bh = self.renderer.font_height + 12;
        self.bh = self.bh.max(self.cfg.lineheight); /* make a menu line AT LEAST 'lineheight' tall */

        self.cfg.lines = self.cfg.lines.max(0);
        self.mh = (self.cfg.lines + 1) * self.bh;
        let promptw = if self.cfg.commented {
            self.bh * 15
        } else {
            match self.prompt().map(|p| p.to_string()) {
                Some(p) if !p.is_empty() => {
                    let w = self.textw(&p);
                    w - self.renderer.lrpad / 4
                }
                _ => 0,
            }
        };
        self.promptw = promptw;

        let monitors: Vec<MonitorInfo> = self.backend.monitors().to_vec();
        let (root_w, root_h) = self.backend.root_size();

        if monitors.is_empty() {
            self.embed_geometry(root_w, root_h)
        } else {
            let i = self.select_monitor(&monitors);
            let mon = &monitors[i];
            self.monitor_geometry(mon, root_w, root_h)
        }
    }

    /// Pick the monitor index from `-m`, the focused monitor, or the pointer.
    fn select_monitor(&mut self, monitors: &[MonitorInfo]) -> usize {
        let n = monitors.len() as i32;
        let mut i = 0usize;
        let mut area_found = false;
        if self.cfg.mon >= 0 && self.cfg.mon < n {
            i = self.cfg.mon as usize;
        } else if let Some(fm) = self.backend.focused_monitor() {
            if fm < monitors.len() {
                i = fm;
                area_found = true;
            }
        }
        if self.cfg.mon < 0 && !area_found {
            if let Some((px, py)) = self.backend.pointer_position() {
                for (idx, mon) in monitors.iter().enumerate() {
                    if intersect_area(px, py, 1, 1, mon) != 0 {
                        i = idx;
                        break;
                    }
                }
            }
        }
        i
    }

    /// Geometry on a selected monitor (centered / followcursor / offset).
    fn monitor_geometry(&mut self, mon: &MonitorInfo, root_w: i32, root_h: i32) -> (i32, i32) {
        let mut x = 0;
        let mut y = 0;

        if self.cfg.centered {
            if self.cfg.dmw != 0 && self.cfg.dmw < mon.width {
                self.mw = self.cfg.dmw;
            } else {
                self.mw = mon.width - 100;
            }

            while (self.cfg.lines + 1) * self.bh > mon.height {
                self.cfg.lines -= 1;
            }

            self.mh = (self.cfg.lines + 1) * self.bh;
            x = mon.x + (mon.width - self.mw) / 2;
            y = mon.y + (mon.height - self.mh) / 2;

            if y < 0 {
                y = 0;
            }
        } else if self.cfg.followcursor {
            if self.cfg.dmw != 0 {
                self.mw = self.cfg.dmw;
            } else {
                // MIN(MAX(max_textw() + promptw, min_width), wa.width);
                // `wa` still holds the root attributes here in the C code.
                let maxw = (self.max_textw() + self.promptw)
                    .max(self.cfg.min_width)
                    .min(root_w);
                self.mw = maxw;
            }
            if let Some((px, py)) = self.backend.pointer_position() {
                x = px;
                y = py;
                if x > mon.x + (root_w - mon.x) / 2 {
                    x = x - self.mw + 20;
                } else {
                    x = x - 20;
                }
                if y > mon.y + (root_h - mon.y) / 2 {
                    y = y - self.mh + 20;
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
            if self.cfg.dmy <= -1 {
                if self.cfg.dmy == -1 {
                    self.cfg.dmy = (mon.height - self.mh) / 2;
                } else {
                    self.cfg.dmy = (self.renderer.font_height as f32 * 1.55) as i32;
                }
            }
            self.mw = if self.cfg.dmw > 0 && self.cfg.dmw < mon.width {
                self.cfg.dmw
            } else {
                mon.width
            };
            if self.cfg.dmx == -1 {
                self.cfg.dmx = (mon.width - self.mw) / 2;
            }
            x = if self.cfg.rightxoffset {
                mon.x + mon.width - self.cfg.dmx - self.mw - 2 * self.cfg.border_width
            } else {
                mon.x + self.cfg.dmx
            };
            y = mon.y
                + if self.cfg.topbar {
                    self.cfg.dmy
                } else {
                    mon.height - self.mh - self.cfg.dmy
                };
        }

        self.adjust_geometry(mon, root_w, root_h, &mut x, &mut y);
        (x, y)
    }

    /// Clamp the computed geometry to the monitor/root and apply fullheight.
    fn adjust_geometry(&mut self, mon: &MonitorInfo, root_w: i32, root_h: i32, x: &mut i32, y: &mut i32) {
        if self.mh > root_h - 10 {
            self.mh = root_h - self.cfg.border_width * 2 - 10;
            self.cfg.lines =
                root_h / (if self.cfg.lineheight != 0 { self.cfg.lineheight } else { self.bh }) - 1;
        }

        if self.mw > root_w - 10 {
            self.mw = root_w - self.cfg.border_width * 2;
        }

        if *x < mon.x {
            *x = mon.x;
        }
        if *x + self.mw > mon.x + mon.width {
            *x = mon.x + mon.width - self.mw - self.cfg.border_width * 2;
        }
        if self.cfg.fullheight {
            *y = mon.y + 32;
            self.mh = root_h - self.cfg.border_width * 2 - (root_h - mon.height + 32);
            self.cfg.lines = root_h / self.cfg.lineheight - 2;
        } else if *y + self.mh > root_h {
            *y = root_h - self.mh;
        }
    }

    /// Geometry when embedding into a parent window (`-W`, no monitor info).
    fn embed_geometry(&mut self, root_w: i32, root_h: i32) -> (i32, i32) {
        let Some((wa_w, wa_h)) = self.backend.embed_parent_size() else {
            self.finish(1);
        };
        let mut x = 0;
        let mut y = 0;
        if self.cfg.centered {
            let maxw = (self.max_textw() + self.promptw)
                .max(self.cfg.min_width)
                .min(wa_w);
            self.mw = maxw;
            x = (wa_w - self.mw) / 2;
            y = (wa_h - self.mh) / 2;
        } else if self.cfg.followcursor {
            if let Some((px, py)) = self.backend.pointer_position() {
                x = px;
                y = py;
                if x > root_w / 2 {
                    x -= self.mw;
                }
                if y > root_h / 2 {
                    y -= self.mh;
                }
            }
            let maxw = (self.max_textw() + self.promptw)
                .max(self.cfg.min_width)
                .min(wa_w);
            self.mw = maxw;
        } else {
            x = self.cfg.dmx;
            y = if self.cfg.topbar {
                self.cfg.dmy
            } else {
                wa_h - self.mh - self.cfg.dmy
            };
            self.mw = if self.cfg.dmw > 0 && self.cfg.dmw < wa_w {
                self.cfg.dmw
            } else {
                wa_w
            };
        }
        (x, y)
    }

    /// Prematch: select the item that first matched the pretyped text.
    fn apply_prematch(&mut self) {
        if self.cfg.prematch && !self.matches.is_empty() && !self.text.is_empty() {
            // remember the item that was the first match for the pretyped text
            let tmpmatch_item = self.matches[0];
            let cursor = self.cursor as i32;
            self.insert(None, -cursor);
            // sel = that item (find its position in the rebuilt match list)
            self.sel = self.matches.iter().position(|&it| it == tmpmatch_item);
            if let Some(next_pos) = self.next {
                let mut pos = next_pos;
                while pos + 1 < self.matches.len() {
                    if self.matches[pos] == tmpmatch_item {
                        self.curr = self.sel;
                        break;
                    }
                    pos += 1;
                }
            }
            self.calcoffsets();
            self.cfg.prematch = false;
        }
    }

    /// Create the window, set the title, map it and draw the first frame.
    fn create_window(&mut self, x: i32, y: i32) {
        let managed = self.cfg.managed;
        let class = if managed { "floatmenu" } else { "dmenu" };
        let bg = self.renderer.schemes[Scheme::Norm as usize][COL_BG];
        let border_color = self.renderer.schemes[Scheme::Sel as usize][COL_BG];
        if self
            .backend
            .create_window(
                x,
                y,
                self.mw,
                self.mh,
                self.cfg.border_width,
                managed,
                !self.cfg.nograb && self.cfg.toast == 0,
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
                .searchtext
                .clone()
                .unwrap_or_else(|| "menu".to_string());
            self.backend.set_title(&title);
        }

        self.backend.map_window();
        if self.cfg.embed.is_some() {
            self.backend.embed_setup(x, y);
        }
        self.canvas.resize(self.mw, self.mh);
        self.drawmenu();
    }
}
