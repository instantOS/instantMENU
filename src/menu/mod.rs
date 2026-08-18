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
    /// sel/curr positions inside `matches`.
    pub sel: Option<usize>,
    pub curr: Option<usize>,
    /// first position of the next page, None on the last page.
    pub next: Option<usize>,
    /// first position of the previous page.
    pub prev: usize,

    /* input */
    pub text: String,
    pub cursor: usize,

    /* geometry */
    pub bh: i32,
    pub mw: i32,
    pub mh: i32,
    pub x: i32,
    pub y: i32,
    pub inputw: i32,
    pub promptw: i32,

    /* state */
    pub numbers: String,
    pub tempnumer: bool,
    pub sely: i32,
    pub tabbed: bool,
    /// dynamic prompt in commented mode (`prompt = sel->text + 1`)
    pub comment_prompt: Option<String>,

    /* case-insensitive matching (the fstrncmp/fstrstr function pointers) */
    pub insensitive: bool,

    stdout: std::io::Stdout,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub text: String,
    pub out: bool,
}

impl Menu {
    pub fn new(cfg: Config, renderer: Renderer, backend: Box<dyn Backend>) -> Self {
        // Port of `-i`/`-s` switching fstrncmp/fstrstr (smartcase starts out
        // insensitive and turns sensitive on uppercase input).
        let insensitive = cfg.smartcase || cfg.insensitive;
        let canvas = Canvas::new(1, 1);
        Menu {
            cfg,
            renderer,
            backend,
            canvas,
            items: Vec::new(),
            matches: Vec::new(),
            sel: None,
            curr: None,
            next: None,
            prev: 0,
            text: String::new(),
            cursor: 0,
            bh: 0,
            mw: 0,
            mh: 0,
            x: 0,
            y: 0,
            inputw: 0,
            promptw: 0,
            numbers: String::new(),
            tempnumer: false,
            sely: 0,
            tabbed: false,
            comment_prompt: None,
            insensitive,
            stdout: std::io::stdout(),
        }
    }

    /* ── helpers over the matches list ─────────────────────────────────── */

    fn sel_text(&self) -> Option<String> {
        self.sel
            .map(|pos| self.items[self.matches[pos]].text.clone())
    }

    /// TEXTW macro
    pub fn textw(&mut self, s: &str) -> i32 {
        if self.cfg.commented {
            self.bh
        } else {
            self.renderer.text_width(s) + self.renderer.lrpad
        }
    }

    /// textw_clamp — width of `s` clamped to `n`. The C version takes
    /// `unsigned n`: 0 yields 0, negatives wrap to "unclamped".
    fn textw_clamp(&mut self, s: &str, n: i32) -> i32 {
        if self.cfg.commented {
            return self.bh;
        }
        if n == 0 {
            return 0;
        }
        if n < 0 {
            return self.textw(s);
        }
        (self.renderer.text_width_clamp(s, n) + self.renderer.lrpad).min(n)
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
    pub fn max_textw(&mut self) -> i32 {
        // Borrowing through `self.textw` would require cloning every item.
        // Split the borrows explicitly instead.
        let commented = self.cfg.commented;
        let lrpad = self.renderer.lrpad;
        let mut len = 0;
        for item in &self.items {
            let width = if commented {
                self.bh
            } else {
                self.renderer.text_width(&item.text) + lrpad
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
