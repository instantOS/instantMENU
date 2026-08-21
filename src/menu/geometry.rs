//! Window geometry, monitor selection and menu setup. Computes the
//! immutable [`Layout`] once; the C version's mutations of the config
//! (clamped lines, normalized offsets, resolved negative width) happen in
//! locals here instead.

use super::layout::Layout;
use super::Menu;
use crate::backend::{Backend, MonitorInfo};
use crate::config::{MonitorChoice, Position, Width};
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
    /// and clamping. Fails only when embedding has no parent window. The
    /// grid shape is derived from the current item count, so this can run
    /// again whenever streamed items change it ([`Menu::reflow`]).
    fn compute_layout(&mut self) -> Result<Layout, ExitStatus> {
        let count = self.matcher.items.len() as i32;
        let grid = super::layout::adjusted_grid(self.cfg.lines, self.cfg.columns, count);
        self.stdin_grid = grid;
        let mut layout = Layout {
            lines: grid.lines.max(0),
            columns: grid.columns,
            ..Layout::default()
        };
        let width = self.resolve_auto_width(layout.columns);

        /* make a menu line AT LEAST 'line_height' tall */
        layout.bar_height = (self.renderer.font_height + 12).max(self.cfg.line_height.pixels());
        let prompt = self.cfg.prompt.clone();
        layout.prompt_width = if self.cfg.commented {
            layout.bar_height * 15
        } else {
            match prompt.as_deref() {
                Some(p) if !p.is_empty() => {
                    self.cell_width(p) - self.renderer.horizontal_padding / 4
                }
                _ => 0,
            }
        };
        /* the command-cell width (C's `arrowwidth`), measured once here so
         * header geometry never re-measures it */
        layout.command_width = self.cell_width(super::RIGHT_GLYPH);
        layout.menu_height = (layout.lines + 1) * layout.bar_height;

        let monitors: Vec<MonitorInfo> = self.backend.monitors().to_vec();
        let root = self.backend.root_size();

        if monitors.is_empty() {
            self.embed_geometry(root, width, &mut layout)?;
        } else {
            /* Follow-cursor needs the pointer for both monitor selection and
             * placement. Fetch it once: on X11 this avoids a second request;
             * on Wayland it bounds the temporary input-probe lifetime to one
             * query. */
            let follow_pointer = self
                .cfg
                .follow_cursor
                .then(|| self.backend.pointer_position())
                .flatten();
            let i = select_monitor(
                &monitors,
                self.cfg.monitor,
                self.backend.as_mut(),
                self.cfg.follow_cursor,
                follow_pointer,
            );
            let monitor = monitors[i].rect;
            self.monitor_geometry(monitor, root, width, follow_pointer, &mut layout);
            self.adjust_geometry(monitor, root, &mut layout);
        }
        Ok(layout)
    }

    /// `--width auto`: measure the items and use the computed width; a
    /// fixed `--width` passes through, an unset one yields 0 ("pick a
    /// default downstream"). Runs before the bar height exists, so
    /// commented-mode measurement sees a zero bar height exactly like the
    /// C version (which resolved this in main() before setup()).
    fn resolve_auto_width(&mut self, columns: i32) -> i32 {
        match self.cfg.width {
            Width::Fixed(w) => return w,
            Width::Default => return 0,
            Width::Auto => {}
        }
        const AUTO_WIDTH_WARNING_ITEMS: usize = 256;
        if !self.auto_width_warned && self.matcher.items.len() >= AUTO_WIDTH_WARNING_ITEMS {
            self.auto_width_warned = true;
            eprintln!(
                "instantmenu: warning: --width auto requires measuring all {} items; use a fixed width for large lists",
                self.matcher.items.len()
            );
        }
        let prompt = self.cfg.prompt.clone();
        let prompt_width = match &prompt {
            Some(p) => self.cell_width(p),
            None => 0,
        };
        (self.max_cell_width() as f64 * 1.3 * columns.max(1) as f64 + prompt_width as f64) as i32
    }

    /// Content-based width: widest item text plus the prompt, floored at
    /// [`Config::min_width`] and capped at `cap` (root or parent width).
    fn content_width(&mut self, prompt_width: i32, cap: i32) -> i32 {
        (self.max_cell_width() + prompt_width)
            .max(self.cfg.min_width)
            .min(cap)
    }

    /// Geometry on a selected monitor: follow the cursor, or sit at an
    /// anchor with a pixel nudge.
    fn monitor_geometry(
        &mut self,
        monitor: Rect,
        root: Size,
        width: i32,
        follow_pointer: Option<Point>,
        layout: &mut Layout,
    ) {
        if self.cfg.follow_cursor {
            if width != 0 {
                layout.menu_width = width;
            } else {
                // width = max(widest item cell + prompt, min_width), capped at
                // the root width (the C code still had `wa` holding the root
                // attributes at this point).
                layout.menu_width = self.content_width(layout.prompt_width, root.w);
            }
            if let Some(pointer) = follow_pointer {
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
            return;
        }

        if self.cfg.position == Position::Center {
            // `center` is a near-full-width popup, not content-sized, so it
            // does not use `min_width` (unlike the follow-cursor/embed paths).
            layout.menu_width = if width != 0 && width < monitor.w {
                width
            } else {
                monitor.w - 100
            };
            while (layout.lines + 1) * layout.bar_height > monitor.h {
                layout.lines -= 1;
            }
            layout.menu_height = (layout.lines + 1) * layout.bar_height;
        } else {
            layout.menu_width = if width > 0 && width < monitor.w {
                width
            } else {
                monitor.w
            };
        }

        let origin = anchor_origin(
            self.cfg.position,
            monitor,
            layout.menu_width,
            layout.menu_height,
        );
        layout.x = origin.x + self.cfg.x_offset;
        layout.y = origin.y + self.cfg.y_offset;
    }

    /// Geometry when embedding into a parent window (`-W`, no monitor info).
    fn embed_geometry(
        &mut self,
        root: Size,
        width: i32,
        layout: &mut Layout,
    ) -> Result<(), ExitStatus> {
        let Some(parent) = self.backend.embed_parent_size() else {
            return Err(ExitStatus::Failure);
        };
        let area = Rect::new(0, 0, parent.w, parent.h);

        if self.cfg.follow_cursor {
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
            layout.menu_width = self.content_width(layout.prompt_width, parent.w);
            return Ok(());
        }

        layout.menu_width = if self.cfg.position == Position::Center {
            self.content_width(layout.prompt_width, parent.w)
        } else if width > 0 && width < parent.w {
            width
        } else {
            parent.w
        };

        let origin = anchor_origin(
            self.cfg.position,
            area,
            layout.menu_width,
            layout.menu_height,
        );
        layout.x = origin.x + self.cfg.x_offset;
        layout.y = origin.y + self.cfg.y_offset;
        Ok(())
    }

    /// Clamp the computed geometry to the monitor/root and apply full_height.
    fn adjust_geometry(&mut self, monitor: Rect, root: Size, layout: &mut Layout) {
        let line_height = self.cfg.line_height.pixels();
        if layout.menu_height > root.h - 10 {
            layout.menu_height = root.h - self.cfg.border_width * 2 - 10;
            layout.lines = root.h
                / (if line_height != 0 {
                    line_height
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
            layout.menu_height = root.h - self.cfg.border_width * 2 - (root.h - monitor.h + 32);
            layout.lines = root.h / line_height - 2;
        } else if layout.y + layout.menu_height > root.h {
            layout.y = root.h - layout.menu_height;
        }
    }

    /// Prematch: select the item that first matched the pretyped text. Runs
    /// once, and only against a complete corpus — mid-stream the winning
    /// item may not have arrived yet. When the corpus completes after the
    /// menu opened (streaming), it only fires while the input still holds
    /// the untouched `-it` seed; editing the text opts out.
    pub(in crate::menu) fn apply_pre_match(&mut self) -> Option<ExitStatus> {
        if !(self.cfg.pre_match
            && !self.pre_match_applied
            && self.stream_complete()
            && !self.matcher.matches.is_empty()
            && !self.editor.text.is_empty()
            && self.initial_seed.as_deref() == Some(self.editor.text.as_str()))
        {
            return None;
        }
        self.pre_match_applied = true;
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
        /* modal menus also close on a click outside them (pointer grab /
         * click-catcher surfaces); managed and embedded ones do not */
        let outside_close = !managed
            && self.cfg.embed.is_none()
            && !self.cfg.no_grab
            && self.cfg.toast.is_none()
            && self.cfg.outside_close;
        if self
            .backend
            .create_window(
                rect,
                self.cfg.border_width,
                managed,
                !self.cfg.no_grab && self.cfg.toast.is_none(),
                outside_close,
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
            if let Err(e) = self.backend.embed_setup(rect.origin()) {
                eprintln!("instantmenu: {e}");
                return Some(ExitStatus::Failure);
            }
        }
        self.canvas
            .resize(Size::new(self.layout.menu_width, self.layout.menu_height));
        self.draw_menu();
        None
    }

    /// Recompute the layout for the current item count and resize the
    /// window when the derived shape changed. Called when a streamed batch
    /// settles and at EOF: with `-l N` the grid shrinks to fit the items,
    /// so the window grows as the list fills (and auto-width tracks the
    /// widest item seen). No-op when nothing moved — the common fast case
    /// where EOF lands inside the first coalescing window and the corpus is
    /// already final before the first layout.
    pub(in crate::menu) fn reflow(&mut self) {
        let Ok(new_layout) = self.compute_layout() else {
            return;
        };
        let rect = Rect::new(
            new_layout.x,
            new_layout.y,
            new_layout.menu_width,
            new_layout.menu_height,
        );
        let old_rect = Rect::new(
            self.layout.x,
            self.layout.y,
            self.layout.menu_width,
            self.layout.menu_height,
        );
        /* input_width derives from menu_width; bar_height from the font.
         * Anything that moves the drawn pixels means: adopt the layout,
         * resize canvas + window and let the caller redraw. The grid shape
         * is compared too: in multi-column mode columns can shrink/grow
         * while the window rectangle stays the same. */
        let rect_moved = rect != old_rect;
        let shape_changed = rect_moved
            || new_layout.lines != self.layout.lines
            || new_layout.columns != self.layout.columns;
        if !shape_changed {
            return;
        }
        self.layout = new_layout;
        if rect_moved {
            self.canvas.resize(Size::new(rect.w.max(1), rect.h.max(1)));
            self.backend.resize_window(rect);
        }
    }
}

/// Top-left corner of a `width`×`height` window anchored to `area`, before
/// any `--x-offset`/`--y-offset` nudge.
fn anchor_origin(position: Position, area: Rect, width: i32, height: i32) -> Point {
    let x = match position {
        Position::TopLeft | Position::Left | Position::BottomLeft => area.x,
        Position::Top | Position::Center | Position::Bottom => area.x + (area.w - width) / 2,
        Position::TopRight | Position::Right | Position::BottomRight => area.x + area.w - width,
    };
    let y = match position {
        Position::TopLeft | Position::Top | Position::TopRight => area.y,
        Position::Left | Position::Center | Position::Right => area.y + (area.h - height) / 2,
        Position::BottomLeft | Position::Bottom | Position::BottomRight => area.y + area.h - height,
    };
    Point::new(x, y)
}

/// Pick the monitor from `--monitor`, the focused monitor, or the pointer.
fn select_monitor(
    monitors: &[MonitorInfo],
    choice: MonitorChoice,
    backend: &mut dyn Backend,
    pointer_first: bool,
    preferred_pointer: Option<Point>,
) -> usize {
    let n = monitors.len();
    if let MonitorChoice::Index(idx) = choice {
        if (idx as usize) < n {
            return idx as usize;
        }
        /* Explicit means explicit: an invalid index deterministically falls
         * back to zero and does not activate focus-tracking machinery. */
        return 0;
    }
    /* Follow-cursor's already-fetched point takes priority over keyboard
     * focus; otherwise the menu could be placed relative to a cursor on a
     * different output than the selected monitor. */
    if pointer_first {
        return preferred_pointer
            .and_then(|pointer| monitor_containing(monitors, pointer))
            .unwrap_or(0);
    }
    /* Ordinary Auto: follow keyboard focus, then the pointer. */
    if let Some(focused) = backend.focused_monitor() {
        if focused < n {
            return focused;
        }
    }
    if let Some(pointer) = backend.pointer_position() {
        return monitor_containing(monitors, pointer).unwrap_or(0);
    }
    0
}

fn monitor_containing(monitors: &[MonitorInfo], pointer: Point) -> Option<usize> {
    monitors
        .iter()
        .position(|monitor| monitor.rect.contains_exclusive(pointer))
}

#[cfg(test)]
mod tests {
    use super::{anchor_origin, select_monitor};
    use crate::backend::{Backend, EventPoll, MonitorInfo};
    use crate::config::{MonitorChoice, Position};
    use crate::geom::{Point, Rect, Size};
    use crate::render::{Canvas, Color};
    use std::cell::Cell;

    struct GeometryBackend {
        focused: Option<usize>,
        pointer: Option<Point>,
        focus_calls: Cell<usize>,
        pointer_calls: usize,
    }

    impl Backend for GeometryBackend {
        fn monitors(&self) -> &[MonitorInfo] {
            &[]
        }

        fn root_size(&self) -> Size {
            Size::new(200, 100)
        }

        fn pointer_position(&mut self) -> Option<Point> {
            self.pointer_calls += 1;
            self.pointer
        }

        fn focused_monitor(&self) -> Option<usize> {
            self.focus_calls.set(self.focus_calls.get() + 1);
            self.focused
        }

        fn create_window(
            &mut self,
            _rect: Rect,
            _border_width: i32,
            _managed: bool,
            _grab: bool,
            _outside_close: bool,
            _class_hint: &str,
            _bg: Color,
            _border_color: Color,
        ) -> Result<(), String> {
            Ok(())
        }

        fn grab_focus(&mut self, _title: &str) -> Result<(), String> {
            Ok(())
        }

        fn set_title(&mut self, _title: &str) {}

        fn present(&mut self, _canvas: &Canvas) {}

        fn poll_event(
            &mut self,
            _timeout: Option<std::time::Duration>,
            _extra: &[std::os::fd::RawFd],
        ) -> EventPoll {
            EventPoll::Closed
        }

        fn request_selection(&mut self, _clipboard: bool) {}
    }

    fn monitors() -> Vec<MonitorInfo> {
        vec![
            MonitorInfo {
                rect: Rect::new(0, 0, 100, 100),
                name: "left".into(),
            },
            MonitorInfo {
                rect: Rect::new(100, 0, 100, 100),
                name: "right".into(),
            },
        ]
    }

    fn backend(focused: Option<usize>, pointer: Option<Point>) -> GeometryBackend {
        GeometryBackend {
            focused,
            pointer,
            focus_calls: Cell::new(0),
            pointer_calls: 0,
        }
    }

    #[test]
    fn explicit_monitor_never_queries_focus_or_pointer() {
        let mut backend = backend(Some(1), Some(Point::new(150, 50)));
        assert_eq!(
            select_monitor(
                &monitors(),
                MonitorChoice::Index(99),
                &mut backend,
                false,
                None,
            ),
            0
        );
        assert_eq!(backend.focus_calls.get(), 0);
        assert_eq!(backend.pointer_calls, 0);
    }

    #[test]
    fn follow_cursor_uses_prefetched_pointer_before_focus() {
        let mut backend = backend(Some(0), None);
        assert_eq!(
            select_monitor(
                &monitors(),
                MonitorChoice::Auto,
                &mut backend,
                true,
                Some(Point::new(100, 50)),
            ),
            1
        );
        assert_eq!(backend.focus_calls.get(), 0);
        assert_eq!(backend.pointer_calls, 0);
    }

    #[test]
    fn auto_uses_focus_then_queries_pointer_as_fallback() {
        let mut focused = backend(Some(1), Some(Point::new(10, 10)));
        assert_eq!(
            select_monitor(&monitors(), MonitorChoice::Auto, &mut focused, false, None,),
            1
        );
        assert_eq!(focused.pointer_calls, 0);

        let mut fallback = backend(None, Some(Point::new(150, 50)));
        assert_eq!(
            select_monitor(&monitors(), MonitorChoice::Auto, &mut fallback, false, None,),
            1
        );
        assert_eq!(fallback.pointer_calls, 1);
    }

    /// Each anchor places the window's top-left corner at the matching
    /// corner/edge-center of the area, before any nudge.
    #[test]
    fn anchors_place_the_window_corner() {
        let area = Rect::new(100, 50, 1000, 600);
        let width = 200;
        let height = 100;

        assert_eq!(
            anchor_origin(Position::TopLeft, area, width, height),
            Point::new(100, 50)
        );
        assert_eq!(
            anchor_origin(Position::Top, area, width, height),
            Point::new(500, 50)
        );
        assert_eq!(
            anchor_origin(Position::TopRight, area, width, height),
            Point::new(900, 50)
        );
        assert_eq!(
            anchor_origin(Position::Left, area, width, height),
            Point::new(100, 300)
        );
        assert_eq!(
            anchor_origin(Position::Center, area, width, height),
            Point::new(500, 300)
        );
        assert_eq!(
            anchor_origin(Position::Right, area, width, height),
            Point::new(900, 300)
        );
        assert_eq!(
            anchor_origin(Position::BottomLeft, area, width, height),
            Point::new(100, 550)
        );
        assert_eq!(
            anchor_origin(Position::Bottom, area, width, height),
            Point::new(500, 550)
        );
        assert_eq!(
            anchor_origin(Position::BottomRight, area, width, height),
            Point::new(900, 550)
        );
    }
}
