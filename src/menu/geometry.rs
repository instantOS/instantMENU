//! Window geometry, monitor selection and menu setup. Computes the
//! complete [`Layout`] at setup and whenever streamed input changes the grid;
//! the C version's mutations of the config (clamped lines, normalized offsets,
//! resolved negative width) happen in locals here instead.

use super::layout::Layout;
use super::Menu;
use crate::backend::{Backend, MonitorInfo};
use crate::config::{MonitorChoice, Position, Width};
use crate::enums::{EditOp, ExitStatus, Scheme};
use crate::geom::{Point, Rect, Size};

impl Menu {
    /// setup — geometry, monitor selection, window creation, first draw.
    /// Returns Some(status) when the menu cannot start, or an early
    /// auto-confirm/single-key pick already ended it.
    pub fn setup(&mut self) -> Option<ExitStatus> {
        let layout = match self.compute_layout() {
            Ok(layout) => layout,
            Err(status) => return Some(status),
        };
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
        let count = self.matcher.layout_item_count() as i32;
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
        layout.prompt_width = if self.cfg.single_key {
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
            self.monitor_geometry(monitor, width, follow_pointer, &mut layout);
            self.adjust_geometry(monitor, &mut layout);
        }
        /* Every Layout leaving this function is complete. Keeping this out of
         * setup() matters because streamed input replaces the Layout during
         * reflow; a partially-derived replacement used to silently reset the
         * input width to zero. */
        layout.input_width = if self.cfg.single_key {
            0
        } else {
            layout.menu_width / 3
        };
        Ok(layout)
    }

    /// `--width auto`: measure the items and use the computed width; a
    /// fixed `--width` passes through, an unset one yields 0 ("pick a
    /// default downstream"). Runs before the bar height exists, so
    /// single-key measurement sees a zero bar height exactly like the
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
        width: i32,
        follow_pointer: Option<Point>,
        layout: &mut Layout,
    ) {
        if self.cfg.follow_cursor {
            if self.cfg.width == Width::Auto {
                layout.menu_width = if width > 0 {
                    width.min(monitor.w)
                } else {
                    self.content_width(layout.prompt_width, monitor.w)
                };
            } else if width != 0 {
                layout.menu_width = width;
            } else {
                // Keep a cursor-following menu within the selected output,
                // including when that output has a non-zero global origin.
                layout.menu_width = self.content_width(layout.prompt_width, monitor.w);
            }
            if let Some(pointer) = follow_pointer {
                let origin = follow_cursor_origin(
                    pointer,
                    monitor,
                    Size::new(layout.menu_width, layout.menu_height),
                );
                layout.x = origin.x;
                layout.y = origin.y;
            }
            return;
        }

        if self.cfg.position == Position::Center {
            if self.cfg.width == Width::Auto {
                // Auto should fit content, not be near-full-width. Use the
                // measured auto width when available, otherwise content width.
                let available = (monitor.w - 100).max(1);
                if width > 0 {
                    layout.menu_width = width.min(available);
                } else {
                    layout.menu_width = self.content_width(layout.prompt_width, available);
                }
            } else {
                // `center` is a near-full-width popup, not content-sized, so it
                // does not use `min_width` (unlike the follow-cursor/embed paths).
                layout.menu_width = if width != 0 && width < monitor.w {
                    width
                } else {
                    monitor.w - 100
                };
            }
            while (layout.lines + 1) * layout.bar_height > monitor.h {
                layout.lines -= 1;
            }
            layout.menu_height = (layout.lines + 1) * layout.bar_height;
        } else if self.cfg.width == Width::Auto {
            // --width auto with an empty corpus would have width==0 and fall
            // back to the full monitor width, flashing very wide before the
            // streamed items arrive and reflow shrinks it (instantstartmenu).
            // Use content width (floored at min_width) instead.
            if width > 0 {
                layout.menu_width = width.min(monitor.w);
            } else {
                layout.menu_width = self.content_width(layout.prompt_width, monitor.w);
            }
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

        if self.cfg.width == Width::Auto {
            if width > 0 {
                layout.menu_width = width.min(parent.w);
            } else {
                layout.menu_width = self.content_width(layout.prompt_width, parent.w);
            }
        } else {
            layout.menu_width = if self.cfg.position == Position::Center {
                self.content_width(layout.prompt_width, parent.w)
            } else if width > 0 && width < parent.w {
                width
            } else {
                parent.w
            };
        }

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

    /// Clamp the computed geometry to the selected monitor and apply full_height.
    fn adjust_geometry(&mut self, monitor: Rect, layout: &mut Layout) {
        let line_height = self.cfg.line_height.pixels();
        if layout.menu_height > monitor.h - 10 {
            layout.menu_height = monitor.h - self.cfg.border_width * 2 - 10;
            layout.lines = monitor.h
                / (if line_height != 0 {
                    line_height
                } else {
                    layout.bar_height
                })
                - 1;
        }

        if layout.menu_width > monitor.w - 10 {
            layout.menu_width = monitor.w - self.cfg.border_width * 2;
        }

        let origin = clamp_origin_to_monitor(
            Point::new(layout.x, layout.y),
            Size::new(layout.menu_width, layout.menu_height),
            monitor,
            self.cfg.border_width,
        );
        layout.x = origin.x;
        layout.y = origin.y;
        if self.cfg.full_height {
            layout.y = monitor.y + 32;
            layout.menu_height = monitor.h - self.cfg.border_width * 2 - 32;
            layout.lines = monitor.h / line_height - 2;
        }
    }

    /// Prematch: select the item that first matched the pretyped text. Runs
    /// once, and only against a complete corpus — mid-stream the winning
    /// item may not have arrived yet. When the corpus completes after the
    /// menu opened (streaming), it only fires while the input still holds
    /// the untouched `-it` seed; editing the text opts out.
    pub(in crate::menu) fn apply_pre_match(&mut self) -> Option<ExitStatus> {
        if !self.cfg.pre_match
            || self.pre_match_applied
            || !self.stream_complete()
            || self.matcher.matches.is_empty()
            || self.editor.text.is_empty()
            || self.initial_seed.as_deref() != Some(self.editor.text.as_str())
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
                    self.selection.page_start = self.selection.selected;
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
        if shape_changed {
            self.layout = new_layout;
            if rect_moved {
                self.canvas.resize(Size::new(rect.w.max(1), rect.h.max(1)));
                self.backend.resize_window(rect);
            }
            /* Paging is derived from Layout. do_match() may have calculated it
             * immediately before this reflow against the old (often initial
             * horizontal) shape, so installing a new Layout and leaving the
             * old page boundary alive is never valid. Drawing and hit-testing
             * both consume this boundary. */
            self.recalc_paging();
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

/// Place a menu beside the pointer, choosing the side independently on each
/// axis from the selected monitor's midpoint. Final edge clamping is handled
/// by [`Menu::adjust_geometry`].
fn follow_cursor_origin(pointer: Point, monitor: Rect, menu: Size) -> Point {
    let x = if pointer.x > monitor.x + monitor.w / 2 {
        pointer.x - menu.w + 20
    } else {
        pointer.x - 20
    };
    let y = if pointer.y > monitor.y + monitor.h / 2 {
        pointer.y - menu.h + 20
    } else {
        pointer.y - 20
    };
    Point::new(x, y)
}

/// Clamp a top-left window origin to an output in global coordinate space.
/// The horizontal inset preserves the existing X11 border allowance; the
/// vertical window bounds historically include their border.
fn clamp_origin_to_monitor(origin: Point, menu: Size, monitor: Rect, border: i32) -> Point {
    let mut x = origin.x.max(monitor.x);
    let mut y = origin.y.max(monitor.y);
    if x + menu.w > monitor.right() {
        x = monitor.right() - menu.w - border * 2;
    }
    if y + menu.h > monitor.bottom() {
        y = monitor.bottom() - menu.h;
    }
    Point::new(x, y)
}

/// Pick the monitor from `--monitor`, the focused monitor, or the pointer.
/// The pointer probe only runs when keyboard focus is unknowable: ordinary
/// startup with working focus info never maps probe surfaces.
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
    /* Ordinary Auto: follow keyboard focus, then the pointer, then the first
     * output. A compositor can legitimately report no focus (no activated
     * window at all, or one spanning several outputs) — the pointer is then
     * the best remaining hint for the monitor the user is looking at, and
     * beats a deterministic first output, which on a laptop is the built-in
     * panel. The Wayland pointer lookup is a bounded probe, so it must stay
     * behind the focus check to keep the happy path probe-free. */
    if let Some(focused) = backend.focused_monitor() {
        if focused < n {
            return focused;
        }
    }
    backend
        .pointer_position()
        .and_then(|pointer| monitor_containing(monitors, pointer))
        .unwrap_or(0)
}

fn monitor_containing(monitors: &[MonitorInfo], pointer: Point) -> Option<usize> {
    monitors
        .iter()
        .position(|monitor| monitor.rect.contains_exclusive(pointer))
}

#[cfg(test)]
mod tests {
    use super::{anchor_origin, clamp_origin_to_monitor, follow_cursor_origin, select_monitor};
    use crate::backend::{stub::TestBackend, MonitorInfo};
    use crate::config::{MonitorChoice, Position};
    use crate::geom::{Point, Rect, Size};

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

    fn backend(focused: Option<usize>, pointer: Option<Point>) -> TestBackend {
        let mut backend = TestBackend::new();
        backend.focused = focused;
        backend.pointer = pointer;
        backend
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
        assert_eq!(backend.focus_calls(), 0);
        assert_eq!(backend.pointer_calls(), 0);
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
        assert_eq!(backend.focus_calls(), 0);
        assert_eq!(backend.pointer_calls(), 0);
    }

    #[test]
    fn auto_uses_focus_then_pointer_then_first_output() {
        /* Focus known: never probe the pointer. */
        let mut focused = backend(Some(1), Some(Point::new(10, 10)));
        assert_eq!(
            select_monitor(&monitors(), MonitorChoice::Auto, &mut focused, false, None,),
            1
        );
        assert_eq!(focused.pointer_calls(), 0);

        /* Focus unknown: the pointer decides, even against output 0. */
        let mut pointer = backend(None, Some(Point::new(150, 50)));
        assert_eq!(
            select_monitor(&monitors(), MonitorChoice::Auto, &mut pointer, false, None,),
            1
        );
        assert_eq!(pointer.pointer_calls(), 1);

        /* Pointer unknown too (probe timeout, no pointer device): the
         * deterministic first-output fallback. */
        let mut neither = backend(None, None);
        assert_eq!(
            select_monitor(&monitors(), MonitorChoice::Auto, &mut neither, false, None,),
            0
        );
        assert_eq!(neither.pointer_calls(), 1);
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

    #[test]
    fn follow_cursor_preserves_negative_monitor_coordinates() {
        let monitor = Rect::new(-1920, -1080, 1920, 1080);
        let menu = Size::new(300, 100);

        assert_eq!(
            follow_cursor_origin(Point::new(-1800, -1000), monitor, menu),
            Point::new(-1820, -1020)
        );
        assert_eq!(
            follow_cursor_origin(Point::new(-100, -100), monitor, menu),
            Point::new(-380, -180)
        );
    }

    #[test]
    fn cursor_origin_clamps_to_negative_monitor_edges() {
        let monitor = Rect::new(-1920, -1080, 1920, 1080);
        let menu = Size::new(300, 100);

        assert_eq!(
            clamp_origin_to_monitor(Point::new(-1940, -1100), menu, monitor, 1),
            Point::new(-1920, -1080)
        );
        assert_eq!(
            clamp_origin_to_monitor(Point::new(-100, -50), menu, monitor, 1),
            Point::new(-302, -100)
        );
    }
}
