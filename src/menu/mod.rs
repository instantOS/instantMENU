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
mod frecency;
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
mod stream;
mod transition;

use std::collections::HashSet;
use std::io::Write;
use std::os::fd::RawFd;
use std::time::SystemTime;

use crate::backend::{Backend, Modifiers};
use crate::config::Config;
use crate::enums::ExitStatus;
use crate::geom::{Point, Rect};
use crate::render::{Canvas, Painter, Renderer};

use frecency::Frecency;
use layout::{Header, Layout};
use matcher::{Item, MatchResult, Matcher};
use measure::{Measure, TextMeasurer};
use paging::{Paging, Selection};
use slide::Slider;
use transition::Transition;

pub use input::read_stdin;

pub use frecency::resolve_cache_path;

use stream::{Gate, LineParser};

/// FontAwesome glyphs drawn in the left/right command cells. The C version
/// used U+F0A0/U+F0A1, which are `fa-hdd-o` and `fa-bullhorn` (not arrows);
/// use the actual arrow codepoints `fa-arrow-left`/`fa-arrow-right`.
const LEFT_GLYPH: &str = "\u{f060}";
const RIGHT_GLYPH: &str = "\u{f061}";

/// The `--alt-tab` state machine. The C version modelled this with two
/// globals (`alttab`, `tabbed`); the enum makes the reachable combinations
/// explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AltTab {
    /// Mode not active (the default, or cancelled with Alt+Space).
    Off,
    /// Mode active: the next release of the Alt key itself confirms the
    /// selection.
    Armed,
    /// An Alt+Tab cycle is in progress: the Tab press advanced the
    /// selection, and its own release is absorbed and re-arms the state.
    Tabbed,
}

/// The menu shell: pure-core state plus the display machinery. All field
/// access is module-internal; the public surface is
/// [`Menu::new`]/[`Menu::add_items`]/[`Menu::begin_stream`]/[`Menu::setup`]/[`Menu::run`].
pub struct Menu {
    cfg: Config,
    renderer: Renderer,
    backend: Box<dyn Backend>,
    canvas: Canvas,

    pub(in crate::menu) editor: editor::Editor,
    pub(in crate::menu) matcher: Matcher,
    pub(in crate::menu) selection: Selection,
    pub(in crate::menu) paging: Paging,
    /// The match the pointer last rested over, per motion events. Pure
    /// change-detection state: a motion event applies the hover only when
    /// the pointer enters a different row, so a resting (jittering)
    /// pointer is a Nop even right after a rematch reset the selection to
    /// the best match — hover and typing cannot alternate frames.
    pub(in crate::menu) hovered: Option<usize>,
    pub(in crate::menu) layout: Layout,
    /// The -l/-g grid as adjusted for the current item count, recomputed
    /// whenever items stream in (the C version computed it once after
    /// reading all of stdin).
    pub(in crate::menu) stdin_grid: layout::GridShape,
    /// `slide` subcommand: Some(_) = slide mode; owns the value state and
    /// receives events instead of the list machinery.
    pub(in crate::menu) slider: Option<Slider>,
    /// `--frecency-cache`: ranks items on load, records printed selections.
    pub(in crate::menu) frecency: Option<Frecency>,

    /* ── streaming stdin ──────────────────────────────────────────────── */
    /// fd of the streaming stdin pipe, -1 when items are not streamed (no
    /// pipe, a tty, or a mode that ignores stdin). Stays set after EOF.
    stream_fd: RawFd,
    /// stdin reached end-of-file: the corpus is final and pick conclusions
    /// (auto-confirm/single-key/pre-match) may fire.
    stream_eof: bool,
    /// EOF has been settled (rematch + final draw done) exactly once.
    stream_finalized: bool,
    /// Items arrived since the last settle.
    stream_dirty: bool,
    parser: LineParser,
    gate: Gate,
    /// Characters from not-yet-settled items, resolved against fontconfig in
    /// one batch per settle.
    pending_chars: HashSet<char>,
    /// --pre-match runs once, when the corpus it matches against is final.
    pre_match_applied: bool,
    /// The -it seed, for the deferred pre-match "text is still the seed"
    /// check.
    initial_seed: Option<String>,
    /// resolve_auto_width warned about measuring a large corpus already.
    auto_width_warned: bool,

    /* runtime flags */
    /// --alt-tab state machine: see [`AltTab`].
    pub(in crate::menu) alt_tab: AltTab,
    pub(in crate::menu) match_counter_text: String,
    pub(in crate::menu) show_match_counter: bool,
    /// y of the selected row, noted during drawing for the selection
    /// animation.
    pub(in crate::menu) selected_y: i32,
    /// Dynamic label prompt in single-key mode.
    pub(in crate::menu) single_key_prompt: Option<String>,

    out: Box<dyn Write>,
}

impl Menu {
    pub fn new(cfg: Config, renderer: Renderer, backend: Box<dyn Backend>) -> Self {
        let alt_tab = if cfg.alt_tab {
            AltTab::Armed
        } else {
            AltTab::Off
        };
        let frecency = cfg.frecency_cache.as_deref().map(Frecency::open);
        Menu {
            matcher: Matcher::new(Vec::new(), &cfg),
            editor: editor::Editor::new(),
            selection: Selection::default(),
            paging: Paging::default(),
            hovered: None,
            layout: Layout::default(),
            stdin_grid: layout::GridShape {
                lines: cfg.lines,
                columns: cfg.columns,
            },
            slider: cfg.slide.as_ref().map(Slider::new),
            frecency,
            stream_fd: -1,
            stream_eof: false,
            stream_finalized: false,
            stream_dirty: false,
            parser: LineParser::default(),
            gate: Gate::default(),
            pending_chars: HashSet::new(),
            pre_match_applied: false,
            initial_seed: None,
            auto_width_warned: false,
            cfg,
            renderer,
            backend,
            canvas: Canvas::new(crate::geom::Size::new(1, 1)),
            alt_tab,
            match_counter_text: String::new(),
            show_match_counter: false,
            selected_y: 0,
            single_key_prompt: None,
            out: Box::new(std::io::stdout()),
        }
    }

    /// Append items to the candidate list. Used by both the blocking load
    /// (tty/toast startup) and every streamed-in batch. New characters are
    /// remembered for the next font-fallback pass — including the glyphs
    /// icon entries *name*, which never occur in the raw text; frecency
    /// ranks each appended slice immediately so arrival order stays
    /// meaningful while the list grows.
    pub fn add_items(&mut self, items: Vec<Item>) {
        if items.is_empty() {
            return;
        }
        for item in &items {
            self.pending_chars.extend(item.text.chars());
            if let Some(icon) = item.entry.icon {
                self.pending_chars.insert(icon);
            }
            if let Some(key) = item.entry.key {
                self.pending_chars.insert(key);
            }
        }
        let start = self.matcher.items.len();
        self.matcher.items.extend(items);
        if let Some(f) = self.frecency.as_ref() {
            f.rank(&mut self.matcher.items[start..], SystemTime::now());
        }
        self.stream_dirty = true;
    }

    /// Start streaming items in from `fd` (an O_NONBLOCK stdin). The menu
    /// window exists by now; run() polls the fd alongside the backend and
    /// settles batches through the coalescing gate until EOF.
    pub fn begin_stream(&mut self, fd: RawFd) {
        self.stream_fd = fd;
    }

    /// True while items may still arrive.
    pub(super) fn stream_active(&self) -> bool {
        self.stream_fd >= 0 && !self.stream_eof
    }

    /// True when the item corpus is final: nothing streams, or EOF was seen.
    /// Pick conclusions (auto-confirm/single-key/pre-match) and reject-no-match
    /// only act on a complete corpus — mid-stream they would answer from a
    /// prefix of the data.
    pub(super) fn stream_complete(&self) -> bool {
        !self.stream_active()
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

    /// Run the matcher for the current text and select the best match.
    /// The C version printed and exited from inside match(); those cases are
    /// transitions here. While items stream in, pick conclusions are
    /// deferred (see [`Matcher::search`]) and an existing selection survives
    /// the rematch — a batch landing under a user's arrow keys must not yank
    /// the highlight back to the top. Once the corpus is final, typing means
    /// "select whatever matches my query best": the selection resets to the
    /// top and a resting pointer does not override it. Hover is
    /// change-detected on the pointer's row (see [`Menu::set_selection`]),
    /// so jitter around a resting pointer cannot fight the reset into
    /// alternating frames — the flicker an earlier pointer-authoritative
    /// rematch suffered from.
    pub(in crate::menu) fn do_match(&mut self) -> Transition {
        let complete = self.stream_complete();
        match self.matcher.search(&self.editor.text, complete) {
            MatchResult::Listed => {
                let keep = (!complete)
                    .then_some(self.selection.selected)
                    .flatten()
                    .filter(|&pos| pos < self.matcher.matches.len());
                self.selection = Selection {
                    selected: self.matcher.first_selectable_match(),
                    page_start: (!self.matcher.matches.is_empty()).then_some(0),
                };
                if let Some(pos) = keep {
                    if self.matcher.match_is_selectable(pos) {
                        self.selection.selected = Some(pos);
                    }
                }
                self.recalc_paging();
                Transition::Nop
            }
            MatchResult::AutoConfirm(idx) => {
                Transition::PrintAndExit(self.matcher.items[idx].output().to_owned())
            }
            MatchResult::SingleKeyPick(pick) => match pick {
                Some(idx) => Transition::PrintAndExit(self.matcher.items[idx].output().to_owned()),
                None => Transition::Exit(ExitStatus::Success),
            },
        }
    }

    /// Move the selection one item forward, paging when it crosses `next`.
    pub(in crate::menu) fn select_next(&mut self) {
        loop {
            let (sel, turned) =
                paging::advance(&self.selection, self.matcher.matches.len(), &self.paging);
            if sel == self.selection {
                break;
            }
            self.selection = sel;
            if turned {
                self.recalc_paging();
            }
            if self
                .selection
                .selected
                .is_some_and(|pos| self.matcher.match_is_selectable(pos))
            {
                break;
            }
        }
    }

    /// Move the selection one item backward, paging when it crosses `prev`.
    pub(in crate::menu) fn select_prev(&mut self) {
        loop {
            let (sel, turned) = paging::retreat(&self.selection, &self.paging);
            if sel == self.selection {
                break;
            }
            self.selection = sel;
            if turned {
                self.recalc_paging();
            }
            if self
                .selection
                .selected
                .is_some_and(|pos| self.matcher.match_is_selectable(pos))
            {
                break;
            }
        }
    }

    /// Start a page at `pos` and select its first selectable item. A trailing
    /// heading cannot trap selection: fall back to the preceding action.
    pub(in crate::menu) fn select_page(&mut self, pos: usize) {
        let selected = (pos..self.matcher.matches.len())
            .find(|&candidate| self.matcher.match_is_selectable(candidate))
            .or_else(|| {
                (0..pos)
                    .rev()
                    .find(|&candidate| self.matcher.match_is_selectable(candidate))
            });
        self.selection = Selection {
            selected,
            page_start: (!self.matcher.matches.is_empty()).then_some(pos),
        };
    }

    pub(in crate::menu) fn recalc_paging(&mut self) {
        let mut m = TextMeasurer::new(
            &mut self.renderer,
            self.cfg.single_key,
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

    /// --preselect: highlight the first selectable match whose output value
    /// equals the configured value, walking the selection there so paging
    /// turns pages exactly like repeated Down presses. The comparison is
    /// exact; without a match the selection stays where the match left it.
    /// While stdin streams in this runs deferred at EOF (`finalize_stream`),
    /// since the target may not have arrived yet.
    pub(in crate::menu) fn apply_preselect(&mut self) {
        let Some(target) = self.cfg.preselect.as_deref() else {
            return;
        };
        let Some(found) = self.matcher.matches.iter().position(|&index| {
            let item = &self.matcher.items[index];
            item.is_selectable() && item.output() == target
        }) else {
            return;
        };
        self.selection = Selection {
            selected: self.matcher.first_selectable_match(),
            page_start: (!self.matcher.matches.is_empty()).then_some(0),
        };
        while self.selection.selected != Some(found) {
            self.select_next();
        }
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

    pub(in crate::menu) fn selected_output_ref(&self) -> Option<&str> {
        self.selection
            .selected
            .map(|pos| self.matcher.output_of_match(pos))
    }

    fn selected_output(&self) -> Option<String> {
        self.selected_output_ref().map(str::to_owned)
    }

    pub(in crate::menu) fn selected_is_heading(&self) -> bool {
        self.selection
            .selected
            .is_some_and(|pos| !self.matcher.match_is_selectable(pos))
    }

    /// Confirm the selection: animate, print, exit unless Ctrl is held, and
    /// mark the item as already output. Returns the transition for run() to
    /// perform.
    pub(in crate::menu) fn confirm(&mut self, out: &str, mods: Modifiers) -> Transition {
        self.animate_selection();
        if let Some(pos) = self.selection.selected {
            let item = &mut self.matcher.items[self.matcher.matches[pos]];
            item.already_output = true;
        }
        if !mods.ctrl {
            Transition::PrintAndExit(out.to_string())
        } else {
            Transition::Print(out.to_string())
        }
    }

    /// Ask the backend for the primary selection (clipboard when Shift is
    /// held) — shared by Ctrl-v/Ctrl-y and middle-click paste.
    pub(in crate::menu) fn request_paste(&mut self, mods: Modifiers) {
        self.backend.request_selection(mods.shift);
    }

    /// Create a [`Painter`] drawing context over `self.renderer` and `self.canvas`.
    pub(in crate::menu) fn painter(&mut self) -> Painter<'_> {
        self.renderer.painter(&mut self.canvas)
    }

    /* ── text measurement ─────────────────────────────────────────────── */

    /// Cell width (glyph width plus horizontal padding).
    pub(in crate::menu) fn cell_width(&mut self, s: &str) -> i32 {
        TextMeasurer::new(
            &mut self.renderer,
            self.cfg.single_key,
            self.layout.bar_height,
        )
        .cell_width(s)
    }

    /// Widest item cell width, measured through the same seam as paging and
    /// layout so the single-key square-cell rule lives in one place.
    pub(in crate::menu) fn max_cell_width(&mut self) -> i32 {
        let mut m = TextMeasurer::new(
            &mut self.renderer,
            self.cfg.single_key,
            self.layout.bar_height,
        );
        let mut len = 0;
        for item in &self.matcher.items {
            if self.cfg.single_key && item.entry.key.is_none() {
                continue;
            }
            len = len.max(m.item_cell_width(item));
        }
        len
    }

    /// The effective prompt (static -p value, or the dynamic single-key
    /// prompt which follows the selected item).
    pub(in crate::menu) fn prompt(&self) -> Option<&str> {
        match &self.single_key_prompt {
            Some(dynamic) => Some(dynamic.as_str()),
            None => self.cfg.prompt.as_deref(),
        }
    }

    /* ── shared view-layout helpers (draw + hit-testing) ──────────────── */

    /// The resolved header-row geometry: the single source of truth shared
    /// by drawing and mouse hit-testing, so a click target is always exactly
    /// where its pixels were drawn.
    pub(in crate::menu) fn header(&mut self) -> Header {
        let show_match_counter = self.show_match_counter;
        let counter_width = if show_match_counter {
            self.cell_width(&self.match_counter_text.clone())
        } else {
            0
        };
        let has_prompt = self.prompt().is_some_and(|p| !p.is_empty());
        let has_matches = !self.matcher.matches.is_empty();
        let mut m = TextMeasurer::new(
            &mut self.renderer,
            self.cfg.single_key,
            self.layout.bar_height,
        );
        Header::compute(
            &self.layout,
            self.cfg.left_command.is_some(),
            self.cfg.right_command.is_some(),
            has_prompt,
            has_matches,
            show_match_counter,
            counter_width,
            &mut m,
            self.cfg.single_key,
        )
    }

    /// Visible horizontal-list items as `(match_pos, rect)` pairs. The single
    /// source of truth for drawing and hit-testing the horizontal list.
    pub(in crate::menu) fn horizontal_item_rects(&mut self, x: i32) -> Vec<(usize, Rect)> {
        let start = self.selection.page_start.unwrap_or(0);
        let end = self.paging.next.unwrap_or(self.matcher.matches.len());
        let match_counter_text = self.match_counter_text.clone();
        let input_width = self.layout.input_width;
        let menu_width = self.layout.menu_width;
        let bar_height = self.layout.bar_height;
        let mut m = TextMeasurer::new(&mut self.renderer, self.cfg.single_key, bar_height);
        let mut x = x + input_width + m.cell_width("<");
        let mut rects = Vec::with_capacity(end.saturating_sub(start));
        for pos in start..end {
            let item = &self.matcher.items[self.matcher.matches[pos]];
            let budget = menu_width - x - m.cell_width(">") - m.cell_width(&match_counter_text);
            let width = m.item_cell_width_clamp(item, budget);
            rects.push((pos, Rect::new(x, 0, width, bar_height)));
            x += width;
        }
        rects
    }

    /* ── output helpers ─────────────────────────────────────────────────── */

    /// Emit a selection line. Recording runs after the line is out the door
    /// so cache I/O never delays the selection. Password input and slider
    /// values are never recorded (the CLI rejects --frecency-cache for
    /// slide; the slider guard covers library use).
    pub(in crate::menu) fn println(&mut self, s: &str) {
        let _ = writeln!(self.out, "{s}");
        let _ = self.out.flush();
        if let Some(f) = self.frecency.as_mut() {
            if !self.cfg.password && self.slider.is_none() {
                f.record(s, SystemTime::now());
            }
        }
    }
}

#[cfg(test)]
mod tests;
