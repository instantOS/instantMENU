//! Backend-agnostic menu core, port of `instantmenu.c`.
//!
//! The monolithic C file is split by concern instead of being kept as one
//! long file: matching, input handling, drawing, animation, keyboard, mouse,
//! geometry and the event loop each live in their own submodule. The
//! functions still mirror the C originals one-to-one so behaviour can be
//! audited against `old_c_codebase/instantmenu.c`.

mod animate;
mod draw;
mod geometry;
mod input;
mod keyboard;
mod matching;
mod mouse;
mod run;

use std::io::Write;

use crate::backend::Backend;
use crate::config::Config;
use crate::render::{Canvas, Renderer};

/// sizeof text in the C version (BUFSIZ) minus the terminator.
const TEXT_MAX: usize = 8192 - 1;

pub struct Menu {
    pub cfg: Config,
    pub renderer: Renderer,
    pub backend: Box<dyn Backend>,
    pub canvas: Canvas,

    /* items and matches */
    pub items: Vec<Item>,
    /// Ordered item indices of the current matches (the C linked list).
    pub matches: Vec<usize>,
    /// selected/current positions inside `matches`.
    pub selected: Option<usize>,
    pub current: Option<usize>,
    /// first position of the next page, None on the last page.
    pub next: Option<usize>,
    /// first position of the previous page.
    pub prev: usize,

    /* input */
    pub text: String,
    pub cursor: usize,

    /* geometry */
    pub bar_height: i32,
    pub menu_width: i32,
    pub menu_height: i32,
    pub x: i32,
    pub y: i32,
    pub input_width: i32,
    pub prompt_width: i32,

    /* state */
    pub numbers: String,
    pub show_numbers: bool,
    pub selected_y: i32,
    pub tabbed: bool,
    /// dynamic prompt in commented mode (`prompt = selected->text + 1`)
    pub comment_prompt: Option<String>,

    /* case-insensitive matching (the fstrncmp/fstrstr function pointers) */
    pub insensitive: bool,

    stdout: std::io::Stdout,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub text: String,
    pub already_output: bool,
}

impl Menu {
    pub fn new(cfg: Config, renderer: Renderer, backend: Box<dyn Backend>) -> Self {
        // Port of `-i`/`-s` switching fstrncmp/fstrstr (smartcase starts out
        // insensitive and turns sensitive on uppercase input).
        let insensitive = cfg.smart_case || cfg.insensitive;
        let canvas = Canvas::new(1, 1);
        Menu {
            cfg,
            renderer,
            backend,
            canvas,
            items: Vec::new(),
            matches: Vec::new(),
            selected: None,
            current: None,
            next: None,
            prev: 0,
            text: String::new(),
            cursor: 0,
            bar_height: 0,
            menu_width: 0,
            menu_height: 0,
            x: 0,
            y: 0,
            input_width: 0,
            prompt_width: 0,
            numbers: String::new(),
            show_numbers: false,
            selected_y: 0,
            tabbed: false,
            comment_prompt: None,
            insensitive,
            stdout: std::io::stdout(),
        }
    }

    /* ── helpers over the matches list ─────────────────────────────────── */

    fn selected_text(&self) -> Option<String> {
        self.selected
            .map(|pos| self.items[self.matches[pos]].text.clone())
    }

    /// TEXTW macro
    pub fn text_width(&mut self, s: &str) -> i32 {
        if self.cfg.commented {
            self.bar_height
        } else {
            self.renderer.text_width(s) + self.renderer.horizontal_padding
        }
    }

    /// textw_clamp — width of `s` clamped to `n`. The C version takes
    /// `unsigned n`: 0 yields 0, negatives wrap to "unclamped".
    fn text_width_clamp(&mut self, s: &str, n: i32) -> i32 {
        if self.cfg.commented {
            return self.bar_height;
        }
        if n == 0 {
            return 0;
        }
        if n < 0 {
            return self.text_width(s);
        }
        (self.renderer.text_width_clamp(s, n) + self.renderer.horizontal_padding).min(n)
    }

    /// The effective prompt (static `-p` value, or the dynamic commented-mode
    /// prompt which follows the selected item).
    fn prompt(&self) -> Option<&str> {
        match &self.comment_prompt {
            Some(dynamic) => Some(dynamic.as_str()),
            None => self.cfg.prompt.as_deref(),
        }
    }

    /// max_textw — widest item text.
    pub fn max_text_width(&mut self) -> i32 {
        let commented = self.cfg.commented;
        let horizontal_padding = self.renderer.horizontal_padding;
        let mut len = 0;
        for item in &self.items {
            let width = if commented {
                self.bar_height
            } else {
                self.renderer.text_width(&item.text) + horizontal_padding
            };
            len = len.max(width);
        }
        len
    }

    /* ── output helpers ─────────────────────────────────────────────────── */

    fn println(&mut self, s: &str) {
        let _ = writeln!(self.stdout, "{s}");
        let _ = self.stdout.flush();
    }

    /// cleanup + exit
    fn finish(&mut self, code: i32) -> ! {
        let _ = self.stdout.flush();
        std::process::exit(code);
    }
}
