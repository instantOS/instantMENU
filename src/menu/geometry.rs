//! Window geometry, monitor selection and menu setup. Computes the
//! immutable [`Layout`] once; the C version's mutations of the config
//! (clamped lines, normalized offsets, resolved negative width) happen in
//! locals here instead.

use super::layout::Layout;
use super::Menu;
use crate::backend::{Backend, MonitorInfo};
use crate::config::Position;
use crate::enums::{EditOp, ExitStatus, Scheme};
use crate::geom::{Point, Rect, Size};

impl Menu {
    /// setup — geometry, monitor selection, window creation, first draw.
    /// Returns Some(status) when the menu cannot start, or an early
    /// instant/commented pick already ended it.
    pub fn setup(&mut self) -> Option<ExitStatus> {
        let mut layout = match self.compute_layout() {
            Ok(layout) => layout,
            Err(status) => return Some(status),
        };
        layout.input_width = layout.menu_width / if self.cfg.commented { 10 } else { 3 };
        self.layout = layout;

        let t = self.do_match();
        if let Some(status) = self.settle(t) {
            return Some(status);
        }
        if let Some(status) = self.apply_pre_match() {
            return Some(status);
        }
        self.create_window()
    }

    /// Compute the whole layout: bar height, prompt width, monitor choice
    /// and clamping. Fails only when embedding has no parent window.
    fn compute_layout(&mut self) -> Result<Layout, ExitStatus> {
        let mut layout = Layout {
            lines: self.stdin_grid.lines.max(0),
            columns: self.stdin_grid.columns,
            ..Layout::default()
        };
        let width = self.resolve_auto_width(layout.columns);

        /* make a menu line AT LEAST 'line_height' tall */
        layout.bar_height = (self.renderer.font_height + 12).max(self.cfg.line_height);
        let prompt = self.cfg.prompt.clone();
        layout.prompt_width = if self.cfg.commented {
            layout.bar_height * 15
        } else {
            match prompt.as_deref() {
                Some(p) if !p.is_empty() => {
                    self.text_width(p) - self.renderer.horizontal_padding / 4
                }
                _ => 0,
            }
        };
        layout.menu_height = (layout.lines + 1) * layout.bar_height;

        let monitors: Vec<MonitorInfo> = self.backend.monitors().to_vec();
        let root = self.backend.root_size();

        if monitors.is_empty() {
            self.embed_geometry(root, width, &mut layout)?;
        } else {
            let i = select_monitor(&monitors, self.cfg.monitor, self.backend.as_ref());
            let monitor = monitors[i].rect;
            self.monitor_geometry(monitor, root, width, &mut layout);
            self.adjust_geometry(monitor, root, &mut layout);
        }
        Ok(layout)
    }

    /// negative `-w`: use the wider of |width| and the computed item width.
    /// Runs before the bar height exists, so commented-mode measurement sees
    /// a zero bar height exactly like the C version (which resolved this in
    /// main() before setup()).
    fn resolve_auto_width(&mut self, columns: i32) -> i32 {
        if self.cfg.width > -1 {
            return self.cfg.width;
        }
        const AUTO_WIDTH_WARNING_ITEMS: usize = 256;
        if self.matcher.items.len() >= AUTO_WIDTH_WARNING_ITEMS {
            eprintln!(
                "instantmenu: warning: --width {} requires measuring all {} items; use a positive width for large lists",
                self.cfg.width,
                self.matcher.items.len()
            );
        }
        let prompt = self.cfg.prompt.clone();
        let prompt_width = match &prompt {
            Some(p) => self.text_width(p),
            None => 0,
        };
        let max_width = (self.max_text_width() as f64 * 1.3 * columns.max(1) as f64
            + prompt_width as f64) as i32;
        if -self.cfg.width > max_width {
            -self.cfg.width
        } else {
            max_width
        }
    }

    /// Geometry on a selected monitor (centered / follow_cursor / offset).
    fn monitor_geometry(&mut self, monitor: Rect, root: Size, width: i32, layout: &mut Layout) {
        if self.cfg.position == Position::Centered {
            if width != 0 && width < monitor.w {
                layout.menu_width = width;
            } else {
                layout.menu_width = monitor.w - 100;
            }

            while (layout.lines + 1) * layout.bar_height > monitor.h {
                layout.lines -= 1;
            }

            layout.menu_height = (layout.lines + 1) * layout.bar_height;
            layout.x = monitor.x + (monitor.w - layout.menu_width) / 2;
            layout.y = monitor.y + (monitor.h - layout.menu_height) / 2;

            if layout.y < 0 {
                layout.y = 0;
            }
        } else if self.cfg.follow_cursor {
            if width != 0 {
                layout.menu_width = width;
            } else {
                // MIN(MAX(max_text_width() + prompt_width, min_width), wa.width);
                // `wa` still holds the root attributes here in the C code.
                let max_width = (self.max_text_width() + layout.prompt_width)
                    .max(self.cfg.min_width)
                    .min(root.w);
                layout.menu_width = max_width;
            }
            if let Some(pointer) = self.backend.pointer_position() {
                let mut x = pointer.x;
                let mut y = pointer.y;
                if x > monitor.x + (root.w - monitor.x) / 2 {
                    x = x - layout.menu_width + 20;
                } else {
                    x -= 20;
                }
                if y > monitor.y + (root.h - monitor.y) / 2 {
                    y = y - layout.menu_height + 20;
                } else {
                    y -= 20;
                }

                let clamped = Point::new(x, y).clamp_non_negative();
                layout.x = clamped.x;
                layout.y = clamped.y;
            }
        } else {
            let mut y_offset = self.cfg.y_offset;
            if y_offset <= -1 {
                if y_offset == -1 {
                    y_offset = (monitor.h - layout.menu_height) / 2;
                } else {
                    y_offset = (self.renderer.font_height as f32 * 1.55) as i32;
                }
            }
            layout.menu_width = if width > 0 && width < monitor.w {
                width
            } else {
                monitor.w
            };
            let mut x_offset = self.cfg.x_offset;
            if x_offset == -1 {
                x_offset = (monitor.w - layout.menu_width) / 2;
            }
            layout.x = if self.cfg.right_x_offset {
                monitor.x + monitor.w - x_offset - layout.menu_width - 2 * self.cfg.border_width
            } else {
                monitor.x + x_offset
            };
            layout.y = monitor.y
                + if self.cfg.position == Position::Top {
                    y_offset
                } else {
                    monitor.h - layout.menu_height - y_offset
                };
        }
    }

    /// Geometry when embedding into a parent window (`-W`, no monitor info).
    fn embed_geometry(&mut self, root: Size, width: i32, layout: &mut Layout) -> Result<(), ExitStatus> {
        let Some(parent) = self.backend.embed_parent_size() else {
            return Err(ExitStatus::Failure);
        };
        if self.cfg.position == Position::Centered {
            let max_width = (self.max_text_width() + layout.prompt_width)
                .max(self.cfg.min_width)
                .min(parent.w);
            layout.menu_width = max_width;
            layout.x = (parent.w - layout.menu_width) / 2;
            layout.y = (parent.h - layout.menu_height) / 2;
        } else if self.cfg.follow_cursor {
            if let Some(pointer) = self.backend.pointer_position() {
                let mut x = pointer.x;
                let mut y = pointer.y;
                // C quirk: menu_width is still 0 here (it is computed below),
                // so this subtraction never does anything.
                if x > root.w / 2 {
                    x -= layout.menu_width;
                }
                if y > root.h / 2 {
                    y -= layout.menu_height;
                }
                layout.x = x;
                layout.y = y;
            }
            let max_width = (self.max_text_width() + layout.prompt_width)
                .max(self.cfg.min_width)
                .min(parent.w);
            layout.menu_width = max_width;
        } else {
            layout.x = self.cfg.x_offset;
            layout.y = if self.cfg.position == Position::Top {
                self.cfg.y_offset
            } else {
                parent.h - layout.menu_height - self.cfg.y_offset
            };
            layout.menu_width = if width > 0 && width < parent.w {
                width
            } else {
                parent.w
            };
        }
        Ok(())
    }

    /// Clamp the computed geometry to the monitor/root and apply full_height.
    fn adjust_geometry(&mut self, monitor: Rect, root: Size, layout: &mut Layout) {
        if layout.menu_height > root.h - 10 {
            layout.menu_height = root.h - self.cfg.border_width * 2 - 10;
            layout.lines = root.h
                / (if self.cfg.line_height != 0 {
                    self.cfg.line_height
                } else {
                    layout.bar_height
                })
                - 1;
        }

        if layout.menu_width > root.w - 10 {
            layout.menu_width = root.w - self.cfg.border_width * 2;
        }

        if layout.x < monitor.x {
            layout.x = monitor.x;
        }
        if layout.x + layout.menu_width > monitor.x + monitor.w {
            layout.x = monitor.x + monitor.w - layout.menu_width - self.cfg.border_width * 2;
        }
        if self.cfg.full_height {
            layout.y = monitor.y + 32;
            layout.menu_height =
                root.h - self.cfg.border_width * 2 - (root.h - monitor.h + 32);
            layout.lines = root.h / self.cfg.line_height - 2;
        } else if layout.y + layout.menu_height > root.h {
            layout.y = root.h - layout.menu_height;
        }
    }

    /// Prematch: select the item that first matched the pretyped text.
    fn apply_pre_match(&mut self) -> Option<ExitStatus> {
        if !(self.cfg.pre_match
            && !self.matcher.matches.is_empty()
            && !self.editor.text.is_empty())
        {
            return None;
        }
        // remember the item that was the first match for the pretyped text
        let first_match_item = self.matcher.matches[0];
        let t = self.insert(EditOp::Delete(self.editor.cursor));
        if let Some(status) = self.settle(t) {
            return Some(status);
        }
        // selected = that item (find its position in the rebuilt match list)
        self.selection.selected = self
            .matcher
            .matches
            .iter()
            .position(|&it| it == first_match_item);
        if let Some(next_pos) = self.paging.next {
            for pos in next_pos..self.matcher.matches.len() {
                if self.matcher.matches[pos] == first_match_item {
                    self.selection.current = self.selection.selected;
                    break;
                }
            }
        }
        self.recalc_paging();
        None
    }

    /// Create the window, set the title, map it and draw the first frame.
    fn create_window(&mut self) -> Option<ExitStatus> {
        let managed = self.cfg.managed;
        let class = if managed { "floatmenu" } else { "dmenu" };
        let bg = self.renderer.color_scheme(Scheme::Normal).bg;
        let border_color = self.renderer.color_scheme(Scheme::Selected).bg;
        let rect = Rect::new(
            self.layout.x,
            self.layout.y,
            self.layout.menu_width,
            self.layout.menu_height,
        );
        if self
            .backend
            .create_window(
                rect,
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
            return Some(ExitStatus::Failure);
        }

        if managed {
            let title = self
                .cfg
                .placeholder
                .clone()
                .unwrap_or_else(|| "menu".to_string());
            self.backend.set_title(&title);
        }

        self.backend.map_window();
        if self.cfg.embed.is_some() {
            self.backend.embed_setup(rect.origin());
        }
        self.canvas
            .resize(Size::new(self.layout.menu_width, self.layout.menu_height));
        self.draw_menu();
        None
    }
}

/// Pick the monitor index from `-m`, the focused monitor, or the pointer.
fn select_monitor(monitors: &[MonitorInfo], cfg_monitor: i32, backend: &dyn Backend) -> usize {
    let n = monitors.len() as i32;
    let mut i = 0usize;
    let mut area_found = false;
    if cfg_monitor >= 0 && cfg_monitor < n {
        i = cfg_monitor as usize;
    } else if let Some(focused) = backend.focused_monitor() {
        if focused < monitors.len() {
            i = focused;
            area_found = true;
        }
    }
    if cfg_monitor < 0 && !area_found {
        if let Some(pointer) = backend.pointer_position() {
            for (idx, monitor) in monitors.iter().enumerate() {
                if Rect::new(pointer.x, pointer.y, 1, 1).intersect_area(monitor.rect) != 0 {
                    i = idx;
                    break;
                }
            }
        }
    }
    i
}
