//! Backend-agnostic menu core, port of `instantmenu.c`.
//!
//! Architecture: the *pure core* ([`editor`], [`matcher`], [`paging`],
//! [`layout`]) owns the list/input state and does no I/O; the [`Menu`]
//! *shell* owns the renderer, backend and canvas, routes events and is the
//! only place that draws, prints, spawns or exits — via [`Transition`].
//! Behaviour intentionally deviates from the C original in places; those
//! are noted in the code and pinned by the test suite.

mod animate;
mod draw;
mod editor;
mod geometry;
mod input;
mod keyboard;
mod layout;
mod matcher;
mod measure;
mod mouse;
mod paging;
mod run;
mod slide;
mod transition;

use std::io::Write;

use crate::backend::{Backend, CONTROL_MASK, SHIFT_MASK};
use crate::config::Config;
use crate::enums::{ExitStatus, ItemCategory};
use crate::geom::Rect;
use crate::render::{Canvas, Renderer};

use layout::Layout;
use matcher::{MatchResult, Matcher};
use measure::{Measure, TextMeasurer};
use paging::{Paging, Selection};
use slide::Slider;
use transition::Transition;

pub use input::{read_stdin, StdinItems};

/// FontAwesome glyphs drawn in the left/right command cells. The C version
/// used U+F0A0/U+F0A1, which are `fa-hdd-o` and `fa-bullhorn` (not arrows);
/// use the actual arrow codepoints `fa-arrow-left`/`fa-arrow-right`.
const LEFT_GLYPH: &str = "\u{f060}";
const RIGHT_GLYPH: &str = "\u{f061}";

/// The menu shell: pure-core state plus the display machinery. All field
/// access is module-internal; the public surface is
/// [`Menu::new`]/[`Menu::load_items`]/[`Menu::setup`]/[`Menu::run`].
pub struct Menu {
    cfg: Config,
    renderer: Renderer,
    backend: Box<dyn Backend>,
    canvas: Canvas,

    pub(in crate::menu) editor: editor::Editor,
    pub(in crate::menu) matcher: Matcher,
    pub(in crate::menu) selection: Selection,
    pub(in crate::menu) paging: Paging,
    pub(in crate::menu) layout: Layout,
    /// -l/-g as adjusted by stdin (item count), consumed by setup().
    pub(in crate::menu) stdin_grid: layout::GridShape,
    /// --slide: Some(_) = slide mode; owns the value state and receives
    /// events instead of the list machinery.
    pub(in crate::menu) slider: Option<Slider>,

    /* runtime flags */
    /// -A alt-tab behaviour: toggled off by Alt+Space at runtime.
    pub(in crate::menu) alt_tab: bool,
    /// an Alt+Tab happened; the release confirms.
    pub(in crate::menu) tabbed: bool,
    pub(in crate::menu) numbers: String,
    pub(in crate::menu) show_numbers: bool,
    /// y of the selected row, noted during drawing for the selection
    /// animation.
    pub(in crate::menu) selected_y: i32,
    /// dynamic prompt in commented mode (`prompt = selected->text + 1`)
    pub(in crate::menu) comment_prompt: Option<String>,

    out: Box<dyn Write>,
}

impl Menu {
    pub fn new(cfg: Config, renderer: Renderer, backend: Box<dyn Backend>) -> Self {
        let alt_tab = cfg.alt_tab;
        Menu {
            matcher: Matcher::new(Vec::new(), &cfg),
            editor: editor::Editor::new(),
            selection: Selection::default(),
            paging: Paging::default(),
            layout: Layout::default(),
            stdin_grid: layout::GridShape {
                lines: cfg.lines,
                columns: cfg.columns,
            },
            slider: cfg.slide.as_ref().map(Slider::new),
            cfg,
            renderer,
            backend,
            canvas: Canvas::new(crate::geom::Size::new(1, 1)),
            alt_tab,
            tabbed: false,
            numbers: String::new(),
            show_numbers: false,
            selected_y: 0,
            comment_prompt: None,
            out: Box::new(std::io::stdout()),
        }
    }

    /// Provide the stdin items and the -l/-g values adjusted for their count.
    pub fn load_items(&mut self, stdin: input::StdinItems) {
        self.stdin_grid = stdin.grid;
        self.matcher.items = stdin.items;
    }

    /* ── transition interpretation — the only place with these effects ── */

    /// Interpret a transition from an event handler. Some(exit) ends the
    /// event loop with that status.
    pub(in crate::menu) fn perform(&mut self, t: Transition) -> Option<ExitStatus> {
        match t {
            Transition::Nop => None,
            Transition::Redraw => {
                self.draw_menu();
                None
            }
            Transition::Print(line) => {
                self.println(&line);
                self.draw_menu();
                None
            }
            Transition::PrintAndExit(line) => {
                self.println(&line);
                Some(ExitStatus::Success)
            }
            Transition::Spawn(cmd) => {
                animate::spawn_detached(&cmd);
                self.draw_menu();
                None
            }
            Transition::SpawnAndExit(cmd) => {
                animate::spawn_detached(&cmd);
                Some(ExitStatus::Success)
            }
            Transition::Exit(status) => Some(status),
        }
    }

    /// Interpret a transition during setup, before the window exists. Only
    /// the endings can occur there (pre-match / -it run against the items
    /// before any drawing); a drawing or spawning transition is a bug.
    pub(in crate::menu) fn settle(&mut self, t: Transition) -> Option<ExitStatus> {
        match t {
            Transition::Nop => None,
            Transition::PrintAndExit(line) => {
                self.println(&line);
                Some(ExitStatus::Success)
            }
            Transition::Exit(status) => Some(status),
            Transition::Redraw
            | Transition::Print(_)
            | Transition::Spawn(_)
            | Transition::SpawnAndExit(_) => {
                unreachable!("drawing/spawning transitions cannot occur before the window exists")
            }
        }
    }

    /* ── re-matching and selection ─────────────────────────────────────── */

    /// Run the matcher for the current text and reset selection/paging.
    /// The C version printed and exited from inside match(); those cases are
    /// transitions here.
    pub(in crate::menu) fn do_match(&mut self) -> Transition {
        match self.matcher.search(&self.editor.text) {
            MatchResult::Listed => {
                self.selection = Selection::from_match(self.matcher.matches.len());
                self.recalc_paging();
                Transition::Nop
            }
            MatchResult::InstantPick(idx) => {
                Transition::PrintAndExit(self.matcher.items[idx].text.clone())
            }
            MatchResult::CommentPick(pick) => match pick {
                Some(idx) => Transition::PrintAndExit(self.matcher.items[idx].text.clone()),
                None => Transition::Exit(ExitStatus::Success),
            },
        }
    }

    /// Move the selection one item forward, paging when it crosses `next`.
    pub(in crate::menu) fn select_next(&mut self) {
        let (sel, turned) =
            paging::advance(&self.selection, self.matcher.matches.len(), &self.paging);
        self.selection = sel;
        if turned {
            self.recalc_paging();
        }
    }

    /// Move the selection one item backward, paging when it crosses `prev`.
    pub(in crate::menu) fn select_prev(&mut self) {
        let (sel, turned) = paging::retreat(&self.selection, &self.paging);
        self.selection = sel;
        if turned {
            self.recalc_paging();
        }
    }

    pub(in crate::menu) fn recalc_paging(&mut self) {
        let mut m = TextMeasurer::new(
            &mut self.renderer,
            self.cfg.commented,
            self.layout.bar_height,
        );
        self.paging = paging::calc_paging(
            &self.selection,
            &self.matcher.items,
            &self.matcher.matches,
            &self.layout,
            &mut m,
        );
    }

    /* ── selection helpers ─────────────────────────────────────────────── */

    pub(in crate::menu) fn selected_text_ref(&self) -> Option<&str> {
        self.selection
            .selected
            .map(|pos| self.matcher.text_of_match(pos))
    }

    fn selected_text(&self) -> Option<String> {
        self.selected_text_ref().map(str::to_owned)
    }

    /// The selected item is a non-selectable comment (starts with '>').
    pub(in crate::menu) fn selected_is_comment(&self) -> bool {
        self.selected_text_ref()
            .is_some_and(|t| ItemCategory::from_prefix(t, true).0.is_comment())
    }

    /// Confirm the selection: animate, print, exit unless Ctrl is held, and
    /// mark the item as already output. Returns the transition for run() to
    /// perform.
    pub(in crate::menu) fn confirm(&mut self, out: &str, state: u32) -> Transition {
        self.animate_selection();
        if let Some(pos) = self.selection.selected {
            let item = &mut self.matcher.items[self.matcher.matches[pos]];
            item.already_output = true;
        }
        if state & CONTROL_MASK == 0 {
            Transition::PrintAndExit(out.to_string())
        } else {
            Transition::Print(out.to_string())
        }
    }

    /// Ask the backend for the primary selection (clipboard when Shift is
    /// held) — shared by Ctrl-v/Ctrl-y and middle-click paste.
    pub(in crate::menu) fn request_paste(&mut self, state: u32) {
        self.backend.request_selection(state & SHIFT_MASK != 0);
    }

    /* ── text measurement (the TEXTW macro) ───────────────────────────── */

    /// TEXTW macro
    pub(in crate::menu) fn text_width(&mut self, s: &str) -> i32 {
        TextMeasurer::new(
            &mut self.renderer,
            self.cfg.commented,
            self.layout.bar_height,
        )
        .text_width(s)
    }

    /// max_textw — widest item text.
    pub(in crate::menu) fn max_text_width(&mut self) -> i32 {
        let commented = self.cfg.commented;
        let horizontal_padding = self.renderer.horizontal_padding;
        let bar_height = self.layout.bar_height;
        let mut len = 0;
        for item in &self.matcher.items {
            let width = if commented {
                bar_height
            } else {
                self.renderer.text_width(&item.text) + horizontal_padding
            };
            len = len.max(width);
        }
        len
    }

    /// The effective prompt (static -p value, or the dynamic commented-mode
    /// prompt which follows the selected item).
    pub(in crate::menu) fn prompt(&self) -> Option<&str> {
        match &self.comment_prompt {
            Some(dynamic) => Some(dynamic.as_str()),
            None => self.cfg.prompt.as_deref(),
        }
    }

    /* ── shared view-layout helpers (draw + hit-testing) ──────────────── */

    /// Width reserved for the left/right command cells (C's `arrowwidth`).
    pub(in crate::menu) fn command_cell_width(&mut self) -> i32 {
        self.text_width(RIGHT_GLYPH)
    }

    /// Visible horizontal-list items as `(match_pos, rect)` pairs. The single
    /// source of truth for drawing and hit-testing the horizontal list.
    pub(in crate::menu) fn horizontal_item_rects(&mut self, x: i32) -> Vec<(usize, Rect)> {
        let start = self.selection.current.unwrap_or(0);
        let end = self.paging.next.unwrap_or(self.matcher.matches.len());
        let numbers = self.numbers.clone();
        let input_width = self.layout.input_width;
        let menu_width = self.layout.menu_width;
        let bar_height = self.layout.bar_height;
        let mut m = TextMeasurer::new(&mut self.renderer, self.cfg.commented, bar_height);
        let mut x = x + input_width + m.text_width("<");
        let mut rects = Vec::with_capacity(end.saturating_sub(start));
        for pos in start..end {
            let text = self.matcher.text_of_match(pos);
            let budget = menu_width - x - m.text_width(">") - m.text_width(&numbers);
            let width = m.text_width_clamp(text, budget);
            rects.push((pos, Rect::new(x, 0, width, bar_height)));
            x += width;
        }
        rects
    }

    /* ── output helpers ─────────────────────────────────────────────────── */

    pub(in crate::menu) fn println(&mut self, s: &str) {
        let _ = writeln!(self.out, "{s}");
        let _ = self.out.flush();
    }
}

#[cfg(test)]
mod tests;
