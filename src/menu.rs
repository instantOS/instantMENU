//! Port of instantmenu.c — the backend-agnostic menu core.
//!
//! The structure mirrors the C file function by function so behaviour can be
//! audited against `old_c_codebase/instantmenu.c`.

use std::io::Write;
use std::time::Duration;

use xkbcommon::xkb::keysyms as ks;

use crate::backend::{
    Backend, BackendEvent, CONTROL_MASK, MOD1_MASK, MOD4_MASK, SHIFT_MASK,
};
use crate::config::Config;
use crate::enums::{outputoffset, ItemCategory, Scheme};
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
    pub stext: String,
    pub out: bool,
    pub distance: f64,
}

/// easeOutQuint
fn ease_out_quint(t: f64) -> f64 {
    let u = t - 1.0;
    1.0 + u * u * u
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

    fn item_text(&self, pos: usize) -> String {
        self.items[self.matches[pos]].text.clone()
    }

    fn sel_text(&self) -> Option<String> {
        self.sel.map(|pos| self.items[self.matches[pos]].text.clone())
    }

    /// TEXTW macro
    fn textw(&mut self, s: &str) -> i32 {
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

    fn present(&mut self) {
        self.backend.present(&self.canvas);
    }

    /* ── matching (port of match/fuzzymatch/appenditem) ────────────────── */

    /// fstrncmp(a, b, n) == 0, honoring the case-insensitivity switch.
    /// Byte-wise strncmp emulation: compares up to n bytes, treating the end
    /// of a slice as the C NUL terminator.
    fn eq_n(&self, a: &[u8], b: &[u8], n: usize) -> bool {
        for i in 0..n {
            let ca = a.get(i).copied().unwrap_or(0);
            let cb = b.get(i).copied().unwrap_or(0);
            let (ca, cb) = if self.insensitive {
                (ca.to_ascii_lowercase(), cb.to_ascii_lowercase())
            } else {
                (ca, cb)
            };
            if ca != cb {
                return false;
            }
            if ca == 0 {
                return true; // both terminated
            }
        }
        true
    }

    /// fstrstr, honoring the case switch.
    fn contains(&self, haystack: &str, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        if self.insensitive {
            haystack.to_lowercase().contains(&needle.to_lowercase())
        } else {
            haystack.contains(needle)
        }
    }

    pub fn do_match(&mut self) {
        if self.cfg.commented {
            let first = self.text.bytes().next();
            if let Some(c) = first {
                for item in &self.items {
                    if item.text.as_bytes().first() == Some(&c) {
                        let text = item.text.clone();
                        self.println(&text);
                        self.finish(0);
                    }
                }
                // exit if no match is found
                self.finish(0);
            }
        }

        if self.cfg.fuzzy {
            self.fuzzymatch();
            return;
        }

        // separate input text into tokens to be matched individually
        // (strtok collapses runs of spaces)
        let tokv: Vec<&str> = self.text.split(' ').filter(|t| !t.is_empty()).collect();
        let tokc = tokv.len();
        let len = tokc.then(|| tokv[0].len()).unwrap_or(0);

        let mut exact: Vec<usize> = Vec::new();
        let mut prefix: Vec<usize> = Vec::new();
        let mut substr: Vec<usize> = Vec::new();
        let text_bytes = self.text.as_bytes();
        let textsize = self.text.len() + 1;

        for (i, item) in self.items.iter().enumerate() {
            if !tokv.iter().all(|tok| self.contains(&item.text, tok)) {
                continue; // not all tokens match
            }
            /* exact matches go first, then prefixes, then substrings */
            if tokc == 0 || self.eq_n(text_bytes, item.text.as_bytes(), textsize) {
                exact.push(i);
            } else if self.eq_n(tokv[0].as_bytes(), item.text.as_bytes(), len) {
                prefix.push(i);
            } else if !self.cfg.exact {
                substr.push(i);
            }
        }
        let had_substr = !substr.is_empty();
        self.matches = exact;
        self.matches.extend(prefix);
        self.matches.extend(substr);

        self.curr = if self.matches.is_empty() { None } else { Some(0) };
        self.sel = self.curr;

        if self.cfg.instant && self.matches.len() == 1 && !had_substr {
            let text = self.items[self.matches[0]].text.clone();
            self.println(&text);
            self.finish(0);
        }

        self.calcoffsets();
    }

    fn fuzzymatch(&mut self) {
        /* bang - we have so much memory */
        let mut matched: Vec<usize> = Vec::new();
        let text_bytes = self.text.as_bytes().to_vec();
        let text_len = text_bytes.len();

        /* walk through all items */
        for (idx, item) in self.items.iter().enumerate() {
            if text_len > 0 {
                let itext = item.text.as_bytes();
                let mut pidx = 0usize; /* pointer */
                let mut sidx: i32 = -1; /* start of match */
                let mut eidx: i32 = -1; /* end of match */
                let mut i = 0usize;
                /* walk through item text */
                while i < itext.len() {
                    let c = itext[i];
                    /* fuzzy match pattern (single byte compare, like
                     * fstrncmp(&text[pidx], &c, 1)) */
                    let equal = pidx < text_len
                        && if self.insensitive {
                            text_bytes[pidx].eq_ignore_ascii_case(&c)
                        } else {
                            text_bytes[pidx] == c
                        };
                    if equal {
                        if sidx == -1 {
                            sidx = i as i32;
                        }
                        pidx += 1;
                        if pidx == text_len {
                            eidx = i as i32;
                            break;
                        }
                    }
                    i += 1;
                }
                /* build list of matches */
                if eidx != -1 {
                    /* compute distance:
                     * add penalty if match starts late (log(sidx+2))
                     * add penalty for a long match without many matching
                     * characters */
                    item.distance =
                        ((sidx + 2) as f64).ln() + (eidx - sidx) as f64 - text_len as f64;
                    matched.push(idx);
                }
            } else {
                matched.push(idx);
            }
        }

        /* sort matches according to distance */
        matched.sort_by(|&a, &b| {
            self.items[a]
                .distance
                .partial_cmp(&self.items[b].distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        self.matches = matched;
        self.curr = if self.matches.is_empty() { None } else { Some(0) };
        self.sel = self.curr;

        if self.cfg.instant && self.matches.len() == 1 {
            let text = self.items[self.matches[0]].text.clone();
            self.println(&text);
            self.finish(0);
        }

        self.calcoffsets();
    }

    /// calcoffsets — which items begin the next and previous pages.
    fn calcoffsets(&mut self) {
        let n = if self.cfg.lines > 0 {
            self.cfg.lines * self.cfg.columns * self.bh
        } else {
            let langle = self.textw("<");
            let rangle = self.textw(">");
            self.mw - (self.promptw + self.inputw + langle + rangle)
        };

        /* calculate which items will begin the next page */
        let mut pos = self.curr;
        let mut i: i32 = 0;
        while let Some(p) = pos {
            let item_text = self.item_text(p);
            i += if self.cfg.lines > 0 {
                self.bh
            } else {
                self.textw_clamp(&item_text, n)
            };
            if i > n {
                break;
            }
            pos = if p + 1 < self.matches.len() {
                Some(p + 1)
            } else {
                None
            };
        }
        self.next = pos;

        /* and the previous page */
        let mut prev = self.curr.unwrap_or(0);
        let mut i: i32 = 0;
        while prev > 0 {
            let item_text = self.item_text(prev - 1);
            i += if self.cfg.lines > 0 {
                self.bh
            } else {
                self.textw_clamp(&item_text, n)
            };
            if i > n {
                break;
            }
            prev -= 1;
        }
        self.prev = prev;
    }

    /* ── input handling (insert/nextrune/movewordedge) ─────────────────── */

    /// Port of insert(): insert `s` at the cursor (n > 0, or n == 0 with a
    /// non-empty string) or delete -n bytes before the cursor (n < 0).
    fn insert(&mut self, s: Option<&str>, n: i32) {
        let n = n as isize;
        if self.text.len() as isize + n > TEXT_MAX as isize {
            return;
        }

        let last = self.text.clone();
        let cursor = self.cursor as isize;

        if n > 0 {
            let s = s.unwrap_or("");
            let byte_len = (n as usize).min(s.len()).min(TEXT_MAX - self.text.len());
            let mut new_text = String::with_capacity(self.text.len() + byte_len);
            new_text.push_str(&self.text[..cursor as usize]);
            new_text.push_str(&s[..byte_len]);
            new_text.push_str(&self.text[cursor as usize..]);
            self.text = new_text;
            self.cursor = (cursor + byte_len as isize) as usize;

            if self.cfg.smartcase {
                let has_upper = self.text.bytes().any(|b| (65..=90).contains(&b));
                if has_upper {
                    self.cfg.smartcase = false;
                    self.insensitive = false;
                }
            }
        } else if n < 0 {
            let cut = (cursor + n).max(0) as usize;
            let mut new_text = String::with_capacity(self.text.len());
            new_text.push_str(&self.text[..cut]);
            new_text.push_str(&self.text[cursor as usize..]);
            self.text = new_text;
            self.cursor = cut;
        } else if let Some(s) = s {
            // n == 0 with a payload: -it inserts with strlen(text)
            if !s.is_empty() {
                let byte_len = s.len().min(TEXT_MAX - self.text.len());
                let mut new_text = String::with_capacity(self.text.len() + byte_len);
                new_text.push_str(&self.text[..self.cursor]);
                new_text.push_str(&s[..byte_len]);
                new_text.push_str(&self.text[self.cursor..]);
                self.text = new_text;
                self.cursor += byte_len;
            }
        }

        self.do_match();

        if self.matches.is_empty() && self.cfg.rejectnomatch {
            /* revert to last text value if theres no match */
            self.text = last;
            self.cursor = (self.cursor as isize - n).max(0) as usize;
            self.do_match();
        }
    }

    /// nextrune: location of the next utf8 rune in the given direction.
    fn nextrune(&self, inc: isize) -> usize {
        let bytes = self.text.as_bytes();
        let mut n = self.cursor as isize + inc;
        while n + inc >= 0
            && n >= 0
            && (n as usize) < bytes.len()
            && (bytes[n as usize] & 0xc0) == 0x80
        {
            n += inc;
        }
        n.max(0) as usize
    }

    fn is_delimiter(&self, pos: usize) -> bool {
        self.text
            .as_bytes()
            .get(pos)
            .map(|b| self.cfg.worddelimiters.as_bytes().contains(b))
            .unwrap_or(false)
    }

    fn movewordedge(&mut self, dir: isize) {
        if dir < 0 {
            /* move cursor to the start of the word */
            while self.cursor > 0 && self.is_delimiter(self.nextrune(-1)) {
                self.cursor = self.nextrune(-1);
            }
            while self.cursor > 0 && !self.is_delimiter(self.nextrune(-1)) {
                self.cursor = self.nextrune(-1);
            }
        } else {
            /* move cursor to the end of the word */
            while self.cursor < self.text.len() && self.is_delimiter(self.cursor) {
                self.cursor = self.nextrune(1);
            }
            while self.cursor < self.text.len() && !self.is_delimiter(self.cursor) {
                self.cursor = self.nextrune(1);
            }
        }
    }

    /* ── numbers ────────────────────────────────────────────────────────── */

    /// recalculatenumbers
    fn recalculatenumbers(&mut self) {
        let numer = self.matches.len();
        if self.cfg.toast != 0 {
            self.tempnumer = false;
            return;
        }
        let denom = self.items.len();
        if numer > 1 {
            if self.cfg.lines > 1 {
                self.tempnumer = numer > self.cfg.lines as usize;
            } else {
                self.tempnumer = true;
            }
        } else {
            self.tempnumer = false;
        }
        self.numbers = format!("{numer}/{denom}");
    }

    /* ── item drawing ──────────────────────────────────────────────────── */

    /// drawitem — draws one item at (x, y, w), returns the advanced x.
    fn drawitem(&mut self, pos: usize, x: i32, y: i32, w: i32) -> i32 {
        let is_sel = self.sel == Some(pos);
        let (text, stext) = {
            let item = &self.items[self.matches[pos]];
            (item.text.clone(), item.stext.clone())
        };
        let bytes = text.as_bytes();

        let mut category = ItemCategory::Normal;
        if bytes.first() == Some(&b'>') {
            if bytes.get(1) == Some(&b'>') {
                category = ItemCategory::ColoredComment;
                let scheme = match bytes.get(2) {
                    Some(b'r') => Some(Scheme::Red),
                    Some(b'g') => Some(Scheme::Green),
                    Some(b'y') => Some(Scheme::Yellow),
                    Some(b'h') => Some(Scheme::Highlight),
                    Some(b'b') => Some(Scheme::Sel),
                    _ => None,
                };
                match scheme {
                    Some(s) => {
                        let sc = self.renderer.scheme(s as usize);
                        self.renderer.setscheme(sc);
                    }
                    None => {
                        category = ItemCategory::Comment;
                        let sc = self.renderer.scheme(Scheme::Norm as usize);
                        self.renderer.setscheme(sc);
                    }
                }
            } else {
                let sc = self.renderer.scheme(Scheme::Norm as usize);
                self.renderer.setscheme(sc);
                category = ItemCategory::Comment;
            }
        } else if bytes.first() == Some(&b':') {
            category = ItemCategory::Colored;
            if is_sel {
                let scheme = match bytes.get(1) {
                    Some(b'r') => Some(Scheme::Red),
                    Some(b'g') => Some(Scheme::Green),
                    Some(b'y') => Some(Scheme::Yellow),
                    Some(b'b') => Some(Scheme::Sel),
                    _ => None,
                };
                match scheme {
                    Some(s) => {
                        let sc = self.renderer.scheme(s as usize);
                        self.renderer.setscheme(sc);
                    }
                    None => {
                        let sc = self.renderer.scheme(Scheme::Sel as usize);
                        self.renderer.setscheme(sc);
                        category = ItemCategory::Normal;
                    }
                }
            } else {
                let sc = self.renderer.scheme(Scheme::Norm as usize);
                self.renderer.setscheme(sc);
            }
        } else {
            let sc = if is_sel {
                self.renderer.scheme(Scheme::Sel as usize)
            } else if self.items[self.matches[pos]].out {
                self.renderer.scheme(Scheme::Out as usize)
            } else {
                self.renderer.scheme(Scheme::Norm as usize)
            };
            self.renderer.setscheme(sc);
        }

        let mut temppadding = 0;
        if category == ItemCategory::Colored && bytes.get(2) == Some(&b' ') {
            temppadding = self.renderer.font_height * 3;
            self.cfg.animated = true;
            // draw the icon (first 6 bytes of text, drawn from byte 3)
            let end = text
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|i| *i < 6)
                .last()
                .unwrap_or(0);
            let end = if text.is_char_boundary(6) {
                6
            } else {
                end.max(3)
            };
            let icon: String = text.chars().skip(3).take_while(|_| false).collect();
            let _ = icon;
            let icon_text = safe_slice(&text, 3, end);
            let lpad = (temppadding as f64 / 2.6) as i32;
            self.renderer.text(
                &mut self.canvas,
                x,
                y,
                temppadding,
                self.cfg.lineheight,
                lpad,
                icon_text,
                false,
                is_sel,
            );
            category = ItemCategory::Icon;
            let sc = if is_sel {
                self.renderer.scheme(Scheme::Hover as usize)
            } else {
                self.renderer.scheme(Scheme::Norm as usize)
            };
            self.renderer.setscheme(sc);
        }

        let output: &str = if self.cfg.commented {
            // single letter display
            &stext[..stext.len().min(1)]
        } else {
            &stext
        };
        let offset = outputoffset(category);
        let shown = safe_slice(output, offset, output.len());

        if is_sel {
            self.sely = y;
        }

        let x_in = x + if category == ItemCategory::Icon { temppadding } else { 0 };
        let w_in = if self.cfg.commented {
            self.bh
        } else {
            w - if category == ItemCategory::Icon { temppadding } else { 0 }
        };
        let lpad = if self.cfg.commented {
            (self.bh - self.renderer.text_width(output)) / 2
        } else {
            self.renderer.lrpad / 2
        };

        self.renderer.text(
            &mut self.canvas,
            x_in,
            y,
            w_in,
            self.bh,
            lpad,
            shown,
            false,
            category == ItemCategory::ColoredComment || is_sel,
        )
    }

    /* ── drawmenu ──────────────────────────────────────────────────────── */

    fn drawmenu(&mut self) {
        let fh = self.renderer.font_height;
        let mut x = 0;
        let y = 0;
        let arrowwidth = self.textw("");

        // commented mode: the prompt follows the selected item
        if self.cfg.commented && !self.matches.is_empty() {
            let sel_text = self.sel_text().unwrap_or_default();
            let stripped = safe_slice(&sel_text, 1, sel_text.len());
            self.comment_prompt = Some(stripped.to_string());
        }

        let scheme_norm = self.renderer.scheme(Scheme::Norm as usize);
        self.renderer.setscheme(scheme_norm);
        self.renderer
            .rect(&mut self.canvas, 0, 0, self.mw, self.mh, true, true, false);

        let prompt = self.prompt().map(|p| p.to_string());
        if let Some(prompt) = prompt.filter(|p| !p.is_empty()) {
            if self.cfg.leftcmd.is_some() {
                x += arrowwidth;
            }
            let sc = self.renderer.scheme(Scheme::Sel as usize);
            self.renderer.setscheme(sc);
            if self.cfg.lines < 8 {
                x = self.renderer.text(
                    &mut self.canvas,
                    x,
                    0,
                    self.promptw,
                    self.bh * (self.cfg.lines + 1),
                    self.renderer.lrpad / 2,
                    &prompt,
                    true,
                    false,
                );
            } else {
                x = self.renderer.text(
                    &mut self.canvas,
                    x,
                    0,
                    self.promptw,
                    self.bh,
                    self.renderer.lrpad / 2,
                    &prompt,
                    true,
                    false,
                );
            }
        }

        /* draw input field */
        let mut w = if self.cfg.lines > 0 || self.matches.is_empty() {
            self.mw - x
        } else {
            self.inputw
        };
        let sc = self.renderer.scheme(Scheme::Norm as usize);
        self.renderer.setscheme(sc);

        if self.cfg.passwd {
            let dots = ".".repeat(self.text.len());
            self.renderer.text(
                &mut self.canvas,
                x,
                0,
                w,
                self.bh,
                self.renderer.lrpad / 2,
                &dots,
                false,
                false,
            );
        } else if !self.text.is_empty() {
            self.renderer.text(
                &mut self.canvas,
                x + if self.cfg.leftcmd.is_some() { arrowwidth } else { 0 },
                0,
                w,
                self.bh,
                self.renderer.lrpad / 2,
                &self.text.clone(),
                false,
                false,
            );
        } else if let Some(searchtext) = self.cfg.searchtext.clone() {
            let sc = self.renderer.scheme(Scheme::Fade as usize);
            self.renderer.setscheme(sc);
            self.renderer.text(
                &mut self.canvas,
                x + if self.cfg.leftcmd.is_some() { arrowwidth } else { 0 },
                0,
                w,
                self.bh,
                self.renderer.lrpad / 2,
                &searchtext,
                false,
                false,
            );
            let sc = self.renderer.scheme(Scheme::Norm as usize);
            self.renderer.setscheme(sc);
        }

        // cursor position: width of text before cursor minus width after
        let before_cursor = self.text[..self.cursor].to_string();
        let after_cursor = self.text[self.cursor..].to_string();
        let mut curpos = self.textw(&before_cursor) - self.textw(&after_cursor);
        curpos += self.renderer.lrpad / 2 - 1;
        if curpos < w {
            let sc = self.renderer.scheme(Scheme::Norm as usize);
            self.renderer.setscheme(sc);
            // disable cursor on password prompt
            if !self.cfg.passwd && self.cfg.toast == 0 {
                self.renderer.rect(
                    &mut self.canvas,
                    x + if self.cfg.leftcmd.is_some() { arrowwidth } else { 0 } + curpos,
                    2 + (self.bh - fh) / 2,
                    2,
                    fh - 4,
                    true,
                    false,
                    false,
                );
            }
        }

        self.recalculatenumbers();

        if self.cfg.lines > 0 {
            /* draw grid */
            let mut i = 0;
            let mut pos = self.curr;
            while let Some(p) = pos {
                if Some(p) == self.next {
                    break;
                }
                let col_width = (self.mw - x) / self.cfg.columns;
                let ix = x + (i / self.cfg.lines) * col_width;
                let iy = y + ((i % self.cfg.lines) + 1) * self.bh;
                self.drawitem(p, ix, iy, col_width);
                i += 1;
                pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
            }
        } else if !self.matches.is_empty() {
            /* draw horizontal list */
            x += self.inputw;
            let mut w_arrow = self.textw("<");
            if self.curr.map(|c| c > 0).unwrap_or(false) {
                let sc = self.renderer.scheme(Scheme::Norm as usize);
                self.renderer.setscheme(sc);
                self.renderer.text(
                    &mut self.canvas,
                    x,
                    0,
                    w_arrow,
                    self.bh,
                    self.renderer.lrpad / 2,
                    "<",
                    false,
                    false,
                );
            }
            x += w_arrow;

            let mut pos = self.curr;
            while let Some(p) = pos {
                if Some(p) == self.next {
                    break;
                }
                let budget = self.mw - x - self.textw(">") - self.textw(&self.numbers.clone());
                let stext = self.items[self.matches[p]].stext.clone();
                let item_width = self.textw_clamp(&stext, budget);
                x = self.drawitem(p, x, 0, item_width);
                pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
            }

            if self.next.is_some() {
                w_arrow = self.textw(">");
                let sc = self.renderer.scheme(Scheme::Norm as usize);
                self.renderer.setscheme(sc);
                if self.tempnumer {
                    let numbers = self.numbers.clone();
                    let numbers_w = self.textw(&numbers);
                    self.renderer.text(
                        &mut self.canvas,
                        self.mw - w_arrow - numbers_w,
                        0,
                        w_arrow,
                        self.bh,
                        self.renderer.lrpad / 2,
                        ">",
                        false,
                        false,
                    );
                }
            }
        }

        let sc = self.renderer.scheme(Scheme::Norm as usize);
        self.renderer.setscheme(sc);
        if self.tempnumer {
            let numbers = self.numbers.clone();
            let numbers_w = self.textw(&numbers);
            let right_pad = if self.cfg.rightcmd.is_some() { arrowwidth } else { 0 };
            self.renderer.text(
                &mut self.canvas,
                self.mw - numbers_w - right_pad,
                0,
                numbers_w,
                self.bh,
                self.renderer.lrpad / 2,
                &numbers,
                false,
                false,
            );
        }
        if self.cfg.lines > 0 {
            if self.cfg.leftcmd.is_some() {
                let sc = self.renderer.scheme(Scheme::Highlight as usize);
                self.renderer.setscheme(sc);
                self.renderer.text(
                    &mut self.canvas,
                    0,
                    0,
                    arrowwidth,
                    self.bh,
                    self.renderer.lrpad / 2,
                    "",
                    false,
                    false,
                );
            }
            if self.cfg.rightcmd.is_some() {
                let sc = self.renderer.scheme(Scheme::Highlight as usize);
                self.renderer.setscheme(sc);
                self.renderer.text(
                    &mut self.canvas,
                    self.mw - arrowwidth,
                    0,
                    arrowwidth,
                    self.bh,
                    self.renderer.lrpad / 2,
                    "",
                    false,
                    false,
                );
            }
        }

        self.present();
    }

    /* ── animations ────────────────────────────────────────────────────── */

    /// animatesel — the selection flash growing from the selected row.
    fn animatesel(&mut self) {
        if !self.cfg.animated || self.cfg.framecount == 0 {
            return;
        }
        let sc = self.renderer.scheme(Scheme::Sel as usize);
        self.renderer.setscheme(sc);
        let framecount = self.cfg.framecount;
        for time in 0..framecount {
            let t = time as f64 / framecount as f64;
            // bottom animation
            if self.sely + self.cfg.lineheight < self.mh - 10 {
                let h = ease_out_quint(t) * (self.mh - (self.cfg.lineheight - 4) - self.sely) as f64;
                self.renderer.rect(
                    &mut self.canvas,
                    0,
                    self.sely + (self.cfg.lineheight - 4),
                    self.mw,
                    h as i32,
                    true,
                    true,
                    false,
                );
            }
            // top animation
            let top_h = ease_out_quint(t) * self.sely as f64;
            let top_y = (self.sely + 4) as f64 - ease_out_quint(t) * (self.sely + 4) as f64;
            self.renderer
                .rect(&mut self.canvas, 0, top_y as i32, self.mw, top_h as i32, true, true, false);
            self.present();
            std::thread::sleep(Duration::from_micros(19000));
        }
    }

    /// animaterect — animate a rectangle from (x1,y1,w1,h1) to (x2,y2,w2,h2).
    fn animaterect(&mut self, x1: i32, y1: i32, w1: i32, h1: i32, x2: i32, y2: i32, w2: i32, h2: i32) {
        if !self.cfg.animated || self.cfg.framecount == 0 {
            return;
        }
        let sc = self.renderer.scheme(Scheme::Sel as usize);
        self.renderer.setscheme(sc);
        let framecount = self.cfg.framecount;
        for time in 0..framecount {
            let f = ease_out_quint(time as f64 / framecount as f64);
            let rx = x1 as f64 + (x2 - x1) as f64 * f;
            let ry = y1 as f64 + (y2 - y1) as f64 * f;
            let rw = w1 as f64 + (w2 - w1) as f64 * f;
            let rh = h1 as f64 + (h2 - h1) as f64 * f;
            self.renderer
                .rect(&mut self.canvas, rx as i32, ry as i32, rw as i32, rh as i32, true, true, false);
            self.present();
            std::thread::sleep(Duration::from_micros(19000));
        }
    }

    /// spawn — run a command detached and exit.
    fn spawn(&mut self, cmd: &str) -> ! {
        let command = format!("{cmd} &> /dev/null");
        let _ = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .spawn();
        self.finish(0)
    }

    /// cmdtrigger — run the left/right command with its animation.
    fn cmdtrigger(&mut self, direction: i32) {
        self.cfg.animated = true;
        let cmd = if direction != 0 {
            let c = self
                .cfg
                .rightcmd
                .clone()
                .or_else(|| self.cfg.leftcmd.clone());
            self.animaterect(
                self.mw + self.cfg.border_width,
                0,
                0,
                self.mh,
                0,
                0,
                self.mw,
                self.mh,
            );
            c
        } else {
            let c = self
                .cfg
                .leftcmd
                .clone()
                .or_else(|| self.cfg.rightcmd.clone());
            self.animaterect(0, 0, 0, self.mh, 0, 0, self.mw, self.mh);
            c
        };

        match cmd {
            Some(c) => self.spawn(&c),
            None => self.finish(0),
        }
    }

    /* ── keyboard ──────────────────────────────────────────────────────── */

    /// selectnumber — Ctrl-1..9 select the n-th item and hit Return.
    fn selectnumber(&mut self, number: usize, state: u32) {
        self.sel = self.curr;
        for _ in 0..number {
            if let Some(s) = self.sel {
                if s + 1 < self.matches.len() {
                    self.sel = Some(s + 1);
                    if self.sel == self.next {
                        self.curr = self.next;
                        self.calcoffsets();
                    }
                }
            }
        }
        let state = state ^ CONTROL_MASK;
        self.handle_return(state);
    }

    /// The Return key branch, shared with selectnumber and the Ctrl-j/m
    /// remaps.
    fn handle_return(&mut self, state: u32) {
        // non-selectable comment
        if let Some(text) = self.sel_text() {
            if text.starts_with('>') {
                return;
            }
        }
        self.animatesel();

        // puts((sel && !(state & ShiftMask & (!rejectnomatch))) ? sel->text : text):
        // with rejectnomatch off, shift+return prints the raw input instead
        // of the selection.
        let shift_suppresses = (state & SHIFT_MASK != 0) && !self.cfg.rejectnomatch;
        let print_sel = self.sel_text().is_some() && !shift_suppresses;
        let out = if print_sel {
            self.sel_text().unwrap_or_default()
        } else {
            self.text.clone()
        };
        self.println(&out);
        if state & CONTROL_MASK == 0 {
            self.finish(0);
        }
        if let Some(pos) = self.sel {
            self.items[self.matches[pos]].out = true;
        }
    }

    /// keyrelease — alt-tab release handling.
    fn keyrelease(&mut self, sym: u32, state: u32) {
        let _ = sym;
        if !self.cfg.alttab {
            return;
        }
        if self.tabbed {
            self.tabbed = false;
            return;
        }

        if state & MOD1_MASK != 0 {
            if state & SHIFT_MASK != 0 {
                return;
            }
            if let Some(text) = self.sel_text() {
                if text.starts_with('>') {
                    return;
                }
            }
            let out = match self.sel_text() {
                Some(t) => t,
                None => self.text.clone(),
            };
            self.println(&out);
            if state & CONTROL_MASK == 0 {
                self.finish(0);
            }
            if let Some(pos) = self.sel {
                self.items[self.matches[pos]].out = true;
            }
        }
    }

    /// keypress — the big keyboard switch.
    fn keypress(&mut self, sym: u32, state: u32, buf: &str) {
        let mut sym = sym;
        let mut state = state;

        if state & CONTROL_MASK != 0 {
            match sym {
                s if sym_eq(s, ks::KEY_a) => sym = ks::KEY_Home,
                s if sym_eq(s, ks::KEY_b) => sym = ks::KEY_Left,
                s if sym_eq(s, ks::KEY_c) => sym = ks::KEY_Escape,
                s if sym_eq(s, ks::KEY_d) => sym = ks::KEY_Delete,
                s if sym_eq(s, ks::KEY_e) => sym = ks::KEY_End,
                s if sym_eq(s, ks::KEY_f) => sym = ks::KEY_Right,
                s if sym_eq(s, ks::KEY_g) => sym = ks::KEY_Escape,
                s if sym_eq(s, ks::KEY_h) => sym = ks::KEY_BackSpace,
                s if sym_eq(s, ks::KEY_i) => sym = ks::KEY_Tab,
                s if sym_eq(s, ks::KEY_j)
                    || sym_eq(s, ks::KEY_J)
                    || sym_eq(s, ks::KEY_m)
                    || sym_eq(s, ks::KEY_M) =>
                {
                    sym = ks::KEY_Return;
                    state &= !CONTROL_MASK;
                }
                s if sym_eq(s, ks::KEY_n) => sym = ks::KEY_Down,
                s if sym_eq(s, ks::KEY_p) => sym = ks::KEY_Up,
                s if sym_eq(s, ks::KEY_s) => {
                    self.insert(Some(".*"), 2);
                }
                s if sym_eq(s, ks::KEY_v) => {
                    /* paste clipboard */
                    self.backend.request_selection(state & SHIFT_MASK != 0);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_k) => {
                    /* delete right */
                    self.text.truncate(self.cursor);
                    self.do_match();
                }
                s if sym_eq(s, ks::KEY_u) => {
                    /* delete left */
                    let cursor = self.cursor as i32;
                    self.insert(None, -cursor);
                }
                s if sym_eq(s, ks::KEY_w) => {
                    /* delete word */
                    while self.cursor > 0 && self.is_delimiter(self.nextrune(-1)) {
                        let nr = self.nextrune(-1);
                        self.insert(None, nr as i32 - self.cursor as i32);
                    }
                    while self.cursor > 0 && !self.is_delimiter(self.nextrune(-1)) {
                        let nr = self.nextrune(-1);
                        self.insert(None, nr as i32 - self.cursor as i32);
                    }
                }
                s if sym_eq(s, ks::KEY_y) || sym_eq(s, ks::KEY_Y) => {
                    /* paste selection */
                    self.backend.request_selection(state & SHIFT_MASK != 0);
                    return;
                }
                s if sym_eq(s, ks::KEY_Left) || sym_eq(s, ks::KEY_KP_Left) => {
                    self.movewordedge(-1);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_Right) || sym_eq(s, ks::KEY_KP_Right) => {
                    self.movewordedge(1);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_Return) || sym_eq(s, ks::KEY_KP_Enter) => {
                    // fall through to the main switch with Return
                }
                s if sym_eq(s, ks::KEY_1) => {
                    self.selectnumber(0, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_2) => {
                    self.selectnumber(1, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_3) => {
                    self.selectnumber(2, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_4) => {
                    self.selectnumber(3, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_5) => {
                    self.selectnumber(4, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_6) => {
                    self.selectnumber(5, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_7) => {
                    self.selectnumber(6, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_8) => {
                    self.selectnumber(7, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_9) => {
                    self.selectnumber(8, state);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_bracketleft) => {
                    self.finish(1);
                }
                _ => return,
            }
        } else if state & SHIFT_MASK != 0 {
            if self.cfg.alttab {
                if let Some(s) = self.sel {
                    if s == 0 {
                        // wrap to the last item
                        self.sel = Some(self.matches.len() - 1);
                        self.calcoffsets();
                    } else if s > 0 {
                        let ns = s - 1;
                        self.sel = Some(ns);
                        if Some(ns + 1) == self.curr {
                            self.curr = Some(self.prev);
                            self.calcoffsets();
                        }
                    }
                }
            }
        } else if state & MOD1_MASK != 0 {
            match sym {
                s if sym_eq(s, ks::KEY_F4) => self.finish(1),
                s if sym_eq(s, ks::KEY_b) => {
                    self.movewordedge(-1);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_f) => {
                    self.movewordedge(1);
                    self.drawmenu();
                    return;
                }
                s if sym_eq(s, ks::KEY_g) => sym = ks::KEY_Home,
                s if sym_eq(s, ks::KEY_G) => sym = ks::KEY_End,
                s if sym_eq(s, ks::KEY_h) => sym = ks::KEY_Up,
                s if sym_eq(s, ks::KEY_j) => sym = ks::KEY_Next,
                s if sym_eq(s, ks::KEY_k) => sym = ks::KEY_Prior,
                s if sym_eq(s, ks::KEY_l) => sym = ks::KEY_Down,
                s if sym_eq(s, ks::KEY_space) => {
                    if self.cfg.alttab {
                        self.tabbed = false;
                        self.cfg.alttab = false;
                    }
                }
                s if sym_eq(s, ks::KEY_Tab) => {
                    self.tabbed = true;

                    if let Some(s) = self.sel {
                        let last = self.matches.len().saturating_sub(1);
                        if s == last {
                            self.sel = Some(0);
                            self.curr = Some(0);
                            self.calcoffsets();
                        } else if s + 1 < self.matches.len() {
                            let ns = s + 1;
                            self.sel = Some(ns);
                            if Some(ns) == self.next {
                                self.curr = self.next;
                                self.calcoffsets();
                            }
                        }
                    }
                }
                _ => return,
            }
        } else if state & MOD4_MASK != 0 {
            if sym_eq(sym, ks::KEY_q) {
                self.finish(1);
            }
        }

        /* main switch */
        if sym_eq(sym, ks::KEY_Delete) || sym_eq(sym, ks::KEY_KP_Delete) {
            if self.cursor >= self.text.len() {
                return;
            }
            self.cursor = self.nextrune(1);
            // fallthrough to BackSpace
            if self.cursor == 0 {
                return;
            }
            let nr = self.nextrune(-1);
            self.insert(None, nr as i32 - self.cursor as i32);
        } else if sym_eq(sym, ks::KEY_BackSpace) {
            if self.cursor == 0 {
                return;
            }
            let nr = self.nextrune(-1);
            self.insert(None, nr as i32 - self.cursor as i32);
        } else if sym_eq(sym, ks::KEY_End) || sym_eq(sym, ks::KEY_KP_End) {
            if self.cursor < self.text.len() {
                self.cursor = self.text.len();
            } else if self.next.is_some() {
                /* jump to end of list and position items in reverse */
                let last = self.matches.len().saturating_sub(1);
                self.curr = Some(last);
                self.calcoffsets();
                self.curr = Some(self.prev);
                self.calcoffsets();
                loop {
                    if self.next.is_none() {
                        break;
                    }
                    match self.curr {
                        Some(c) if c + 1 <= last => {
                            self.curr = Some(c + 1);
                            self.calcoffsets();
                        }
                        _ => break,
                    }
                }
            }
            self.sel = if self.matches.is_empty() { None } else { Some(self.matches.len() - 1) };
        } else if sym_eq(sym, ks::KEY_Escape) {
            self.finish(1);
        } else if sym_eq(sym, ks::KEY_Home) || sym_eq(sym, ks::KEY_KP_Home) {
            if self.sel.is_none() && self.matches.is_empty() {
                self.cursor = 0;
            } else if self.sel == Some(0) {
                self.cursor = 0;
            } else {
                self.sel = Some(0);
                self.curr = Some(0);
                self.calcoffsets();
            }
        } else if sym_eq(sym, ks::KEY_Left) || sym_eq(sym, ks::KEY_KP_Left) {
            if self.cfg.columns > 1 {
                let Some(s) = self.sel else { return };
                let mut tmpsel = s;
                let mut offscreen = false;
                for _ in 0..self.cfg.lines {
                    if tmpsel == 0 {
                        return;
                    }
                    if Some(tmpsel) == self.curr {
                        offscreen = true;
                    }
                    tmpsel -= 1;
                }
                self.sel = Some(tmpsel);
                if offscreen {
                    self.curr = Some(self.prev);
                    self.calcoffsets();
                }
            } else {
                if (state & (SHIFT_MASK | MOD4_MASK) != 0)
                    && (self.cfg.leftcmd.is_some() || self.cfg.rightcmd.is_some())
                {
                    self.cmdtrigger(0);
                } else {
                    if self.cursor > 0
                        && (self.sel.is_none() || self.sel == Some(0) || self.cfg.lines > 0)
                    {
                        self.cursor = self.nextrune(-1);
                    } else if self.cfg.lines > 0 {
                        return;
                    } else {
                        // fallthrough to Up
                        self.nav_up();
                    }
                }
            }
        } else if sym_eq(sym, ks::KEY_Up) || sym_eq(sym, ks::KEY_KP_Up) {
            self.nav_up();
        } else if sym_eq(sym, ks::KEY_Next) || sym_eq(sym, ks::KEY_KP_Next) {
            let Some(next) = self.next else { return };
            self.sel = Some(next);
            self.curr = Some(next);
            self.calcoffsets();
        } else if sym_eq(sym, ks::KEY_Prior) || sym_eq(sym, ks::KEY_KP_Prior) {
            if self.curr.is_none() {
                return;
            }
            self.sel = Some(self.prev);
            self.curr = Some(self.prev);
            self.calcoffsets();
        } else if sym_eq(sym, ks::KEY_Return) || sym_eq(sym, ks::KEY_KP_Enter) {
            self.handle_return(state);
        } else if sym_eq(sym, ks::KEY_Right) || sym_eq(sym, ks::KEY_KP_Right) {
            if self.cfg.columns > 1 {
                let Some(s) = self.sel else { return };
                let mut tmpsel = s;
                let mut offscreen = false;
                for _ in 0..self.cfg.lines {
                    if tmpsel + 1 >= self.matches.len() {
                        return;
                    }
                    tmpsel += 1;
                    if Some(tmpsel) == self.next {
                        offscreen = true;
                    }
                }
                self.sel = Some(tmpsel);
                if offscreen {
                    self.curr = self.next;
                    self.calcoffsets();
                }
            } else {
                if (state & (SHIFT_MASK | MOD4_MASK) != 0)
                    && (self.cfg.rightcmd.is_some() || self.cfg.leftcmd.is_some())
                {
                    self.cmdtrigger(1);
                } else if self.cursor < self.text.len() {
                    self.cursor = self.nextrune(1);
                } else if self.cfg.lines > 0 {
                    return;
                } else {
                    // fallthrough to Down
                    self.nav_down();
                }
            }
        } else if sym_eq(sym, ks::KEY_Down) || sym_eq(sym, ks::KEY_KP_Down) {
            self.nav_down();
        } else if sym_eq(sym, ks::KEY_Tab) {
            if !self.cfg.alttab {
                let Some(s) = self.sel else { return };
                let sel_text = self.items[self.matches[s]].text.clone();
                let take = sel_text.len().min(TEXT_MAX);
                self.text = sel_text[..take].to_string();
                self.cursor = take;
                self.do_match();
            } else {
                self.tabbed = true;
            }
        } else {
            // insert: composed string from the input method
            if let Some(first) = buf.bytes().next() {
                if !first.is_ascii_control() {
                    self.insert(Some(buf), buf.len() as i32);
                }
            }
        }

        self.drawmenu();
    }

    /// Up navigation shared by XK_Up and the XK_Left fallthrough.
    fn nav_up(&mut self) {
        if let Some(s) = self.sel {
            if s > 0 {
                let ns = s - 1;
                if Some(ns + 1) == self.curr {
                    self.curr = Some(self.prev);
                    self.calcoffsets();
                }
                self.sel = Some(ns);
            }
        }
    }

    /// Down navigation shared by XK_Down and the XK_Right fallthrough.
    fn nav_down(&mut self) {
        if let Some(s) = self.sel {
            if s + 1 < self.matches.len() {
                let ns = s + 1;
                self.sel = Some(ns);
                if Some(ns) == self.next {
                    self.curr = self.next;
                    self.calcoffsets();
                }
            }
        }
    }

    /* ── mouse ─────────────────────────────────────────────────────────── */

    /// setselection — hover selection on motion.
    fn setselection(&mut self, ev_x: i32, ev_y: i32) {
        let mut x = 0;
        let mut y = 0;
        let h = self.bh;
        let mut w;

        if let Some(prompt) = self.prompt() {
            if !prompt.is_empty() {
                x += self.promptw;
            }
        }

        /* input field */
        w = if self.cfg.lines > 0 || self.matches.is_empty() {
            self.mw - x
        } else {
            self.inputw
        };

        if self.cfg.lines > 0 {
            w = self.mw - x;
            if self.cfg.columns > 0 {
                // check mouse hover for columns (ported literally)
                let mut i = 0;
                let mut init = false;
                let mut checky = y;
                let mut checkx = x;
                let colwidth = self.mw / self.cfg.columns;
                let mut pos = self.curr;
                while let Some(p) = pos {
                    if Some(p) == self.next {
                        break;
                    }
                    if i >= self.cfg.lines {
                        i = 0;
                        checkx += colwidth;
                        checky = y;
                    } else {
                        if !init {
                            init = true;
                        } else {
                            pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
                            if pos.is_none() {
                                break;
                            }
                        }
                        i += 1;
                        checky += h;
                    }
                    // event in range
                    if ev_y >= checky
                        && ev_y <= (checky + h)
                        && ev_x >= checkx
                        && ev_x <= (checkx + colwidth)
                    {
                        if let Some(pp) = pos {
                            if self.sel == Some(pp) {
                                return;
                            }
                            self.sel = Some(pp);
                            self.drawmenu();
                        }
                        return;
                    }
                }
            } else {
                /* vertical list: (ctrl)left-click on item */
                let mut pos = self.curr;
                while let Some(p) = pos {
                    if Some(p) == self.next {
                        break;
                    }
                    y += h;
                    if ev_y >= y && ev_y <= (y + h) {
                        if self.sel == Some(p) {
                            return;
                        }
                        self.sel = Some(p);
                        self.drawmenu();
                        return;
                    }
                    pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
                }
            }
        } else if !self.matches.is_empty() {
            /* left-click on left arrow */
            x += self.inputw;
            let mut w_arrow = self.textw("<");
            /* horizontal list: (ctrl)left-click on item */
            let mut pos = self.curr;
            while let Some(p) = pos {
                if Some(p) == self.next {
                    break;
                }
                x += w_arrow;
                let item_text = self.items[self.matches[p]].text.clone();
                let rangle = self.textw(">");
                w_arrow = self.textw(&item_text).min(self.mw - x - rangle);
                if ev_x >= x && ev_x <= x + w_arrow {
                    if self.sel == Some(p) {
                        return;
                    }
                    self.sel = Some(p);
                    self.drawmenu();
                    return;
                }
                pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
            }
            /* left-click on right arrow */
            let _ = w;
        }
    }

    /// buttonpress
    fn buttonpress(&mut self, button: u8, state: u32, ev_x: i32, ev_y: i32) {
        let mut x = 0;
        let y = 0;
        let h = self.bh;
        let mut w;

        /* right-click: exit */
        if button == 3 {
            std::process::exit(1);
        }

        if let Some(prompt) = self.prompt() {
            if !prompt.is_empty() {
                x += self.promptw;
            }
        }

        /* input field */
        w = if self.cfg.lines > 0 || self.matches.is_empty() {
            self.mw - x
        } else {
            self.inputw
        };

        /* left-click on input: clear input,
         * NOTE: if there is no left-arrow the space for < is reserved so
         *       add that to the input width */
        if button == 1 {
            let arrowwidth = self.textw("");
            let input_hit = (self.cfg.lines <= 0
                && ev_x >= 0
                && ev_x
                    <= x + w
                        + if self.prev == 0 || self.curr.map(|c| c == 0).unwrap_or(true) {
                            self.textw("<")
                        } else {
                            0
                        })
                || (self.cfg.lines > 0 && ev_y >= y && ev_y <= y + h);
            if input_hit {
                if self.cfg.leftcmd.is_some() && ev_x < self.textw("") {
                    self.cmdtrigger(0);
                } else if ev_x > self.mw - self.textw("") {
                    self.cmdtrigger(1);
                } else {
                    let cursor = self.cursor as i32;
                    self.insert(None, -cursor);
                    self.drawmenu();
                }
                return;
            } else if self.cfg.lines > 0 {
                /* vertical list: (ctrl)left-click on item */
                w = self.mw - x;
                let item = self.sel_text();
                if let Some(text) = &item {
                    if text.starts_with('>') {
                        return;
                    }
                }
                self.animatesel();
                let out = item.unwrap_or_else(|| self.text.clone());
                self.println(&out);
                if state & CONTROL_MASK == 0 {
                    std::process::exit(0);
                }
                if let Some(s) = self.sel {
                    self.items[self.matches[s]].out = true;
                }
                self.drawmenu();
                return;
            } else if !self.matches.is_empty() {
                /* left-click on left arrow */
                x += self.inputw;
                let mut w_arrow = self.textw("<");
                if self.prev != 0 || self.curr.map(|c| c > 0).unwrap_or(false) {
                    if ev_x >= x && ev_x <= x + w_arrow {
                        self.sel = self.curr;
                        self.curr = Some(self.prev);
                        self.calcoffsets();
                        self.drawmenu();
                        return;
                    }
                }
                /* horizontal list: (ctrl)left-click on item */
                let mut pos = self.curr;
                while let Some(p) = pos {
                    if Some(p) == self.next {
                        break;
                    }
                    x += w_arrow;
                    let item_text = self.items[self.matches[p]].text.clone();
                    let rangle = self.textw(">");
                    w_arrow = self.textw(&item_text).min(self.mw - x - rangle);
                    if ev_x >= x && ev_x <= x + w_arrow {
                        if let Some(text) = self.sel_text() {
                            if item_text.starts_with('>') && !text.is_empty() {
                                break;
                            }
                        }
                        self.animatesel();
                        self.println(&item_text);
                        if state & CONTROL_MASK == 0 {
                            std::process::exit(0);
                        }
                        self.sel = Some(p);
                        self.items[self.matches[p]].out = true;
                        self.drawmenu();
                        return;
                    }
                    pos = if p + 1 < self.matches.len() { Some(p + 1) } else { None };
                }
                /* left-click on right arrow */
                let rangle = self.textw(">");
                w_arrow = rangle;
                x = self.mw - w_arrow;
                if self.next.is_some() && ev_x >= x && ev_x <= x + w_arrow {
                    let next = self.next.unwrap();
                    self.sel = Some(next);
                    self.curr = Some(next);
                    self.calcoffsets();
                    self.drawmenu();
                    return;
                }
            }
        }

        /* middle-mouse click: paste selection */
        if button == 2 {
            self.backend.request_selection(state & SHIFT_MASK != 0);
            self.drawmenu();
            return;
        }
        /* scroll up */
        if button == 4 && (self.prev != 0 || self.curr.map(|c| c > 0).unwrap_or(false)) {
            self.sel = self.curr;
            self.curr = Some(self.prev);
            self.calcoffsets();
            self.drawmenu();
            return;
        }
        /* scroll down */
        if button == 5 && self.next.is_some() {
            let next = self.next.unwrap();
            self.sel = Some(next);
            self.curr = Some(next);
            self.calcoffsets();
            self.drawmenu();
        }
    }

    /// paste — insert selection text.
    fn paste(&mut self, text: &str) {
        /* we have been given the current selection, now insert it into input */
        let line = text.split('\n').next().unwrap_or("");
        self.insert(Some(line), line.len() as i32);
        self.drawmenu();
    }

    /* ── stdin ─────────────────────────────────────────────────────────── */

    /// readstdin — getline-per-line semantics: split on '\n' (a final chunk
    /// without trailing newline is still an item), then strip ONE trailing
    /// '\n' or '\t' byte and cut at the first NUL like strdup would.
    pub fn readstdin(&mut self) {
        if self.cfg.passwd || self.cfg.inputonly {
            self.inputw = 0;
            self.cfg.lines = 0;
            return;
        }

        /* read each line from stdin and add it to the item list */
        let mut input = Vec::new();
        if std::io::stdin().read_to_end(&mut input).is_err() {
            /* keep whatever we got, like getline erroring mid-way */
        }
        let mut count: i32 = 0;
        let mut pieces: Vec<&[u8]> = input.split(|&b| b == b'\n').collect();
        // the piece after a trailing '\n' (or of empty input) is EOF, not a line
        if input.is_empty() || input.last() == Some(&b'\n') {
            pieces.pop();
        }
        for raw in pieces {
            let mut line: Vec<u8> = raw.to_vec();
            if line.last() == Some(&b'\t') {
                line.pop(); // only the last byte, like the C code
            }
            let cut = line.iter().position(|b| *b == 0).unwrap_or(line.len());
            line.truncate(cut);
            let Ok(line) = String::from_utf8(line) else {
                /* C strdup keeps invalid bytes; items are drawn as text so
                 * drop-lossy lines are the closest safe equivalent */
                count += 1;
                continue;
            };
            self.items.push(Item {
                text: line.clone(),
                stext: line,
                out: false,
                distance: 0.0,
            });
            count += 1;
        }

        let columns = self.cfg.columns;
        let lines = self.cfg.lines;
        let i = count;
        self.cfg.lines = lines.min(i / columns + (i % columns != 0) as i32);
        if columns != 1 && self.cfg.lines != 0 {
            self.cfg.columns =
                (i / self.cfg.lines + (i % self.cfg.lines != 0) as i32).min(columns);
        }
    }

    /// `-it` — initial input text, applied with rejectnomatch temporarily
    /// disabled (port of the insert() call in the argv loop; items are empty
    /// at that point, so this only seeds text/cursor/smartcase).
    pub fn initial_text(&mut self, s: &str) {
        let tmp = self.cfg.rejectnomatch;
        self.cfg.rejectnomatch = false;
        self.insert(Some(s), s.len() as i32);
        self.cfg.rejectnomatch = tmp;
    }

    /// max_textw — widest item text.
    fn max_textw(&mut self) -> i32 {
        let mut len = 0;
        for item in &self.items {
            let w = self.textw(&item.text.clone());
            len = len.max(w);
        }
        len
    }

    /* ── setup ─────────────────────────────────────────────────────────── */

    /// setup — geometry, monitor selection, window creation, first draw.
    pub fn setup(&mut self) {
        /* init appearance (schemes are already in the renderer) */

        /* calculate menu geometry */
        self.bh = self.renderer.font_height + 12;
        self.bh = self.bh.max(self.cfg.lineheight); /* make a menu line AT LEAST 'lineheight' tall */

        self.cfg.lines = self.cfg.lines.max(0);
        self.mh = (self.cfg.lines + 1) * self.bh;
        let promptw = if self.cfg.commented {
            self.bh * 15
        } else {
            match self.prompt() {
                Some(p) if !p.is_empty() => {
                    let w = self.textw(p);
                    w - self.renderer.lrpad / 4
                }
                _ => 0,
            }
        };
        self.promptw = promptw;

        let monitors: Vec<crate::backend::MonitorInfo> =
            self.backend.monitors().to_vec();
        let (root_w, root_h) = self.backend.root_size();
        let mut x = 0;
        let mut y = 0;

        if !monitors.is_empty() {
            /* select monitor */
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

            let mon = &monitors[i];
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

            if self.mh > root_h - 10 {
                self.mh = root_h - self.cfg.border_width * 2 - 10;
                self.cfg.lines =
                    root_h / (if self.cfg.lineheight != 0 { self.cfg.lineheight } else { self.bh }) - 1;
            }

            if self.mw > root_w - 10 {
                self.mw = root_w - self.cfg.border_width * 2;
            }

            if x < mon.x {
                x = mon.x;
            }
            if x + self.mw > mon.x + mon.width {
                x = mon.x + mon.width - self.mw - self.cfg.border_width * 2;
            }
            if self.cfg.fullheight {
                y = mon.y + 32;
                self.mh = root_h - self.cfg.border_width * 2 - (root_h - mon.height + 32);
                self.cfg.lines = root_h / self.cfg.lineheight - 2;
            } else if y + self.mh > root_h {
                y = root_h - self.mh;
            }
        } else {
            /* embedded window (no monitor info) */
            let Some((wa_w, wa_h)) = self.backend.embed_parent_size() else {
                self.finish(1);
            };
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
        }

        self.inputw = self.mw / (if self.cfg.commented { 10 } else { 3 }); /* input width: ~33% of monitor width */
        self.do_match();

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

        self.x = x;
        self.y = y;

        /* create menu window */
        let managed = self.cfg.managed;
        let class = if managed { "floatmenu" } else { "dmenu" };
        let bg = self.renderer.scheme(Scheme::Norm as usize)[crate::enums::COL_BG];
        let border_color = self.renderer.scheme(Scheme::Sel as usize)[crate::enums::COL_BG];
        if self
            .backend
            .create_window(x, y, self.mw, self.mh, self.cfg.border_width, managed, class, bg, border_color)
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
            self.backend.embed_setup();
        }
        self.canvas.resize(self.mw, self.mh);
        self.drawmenu();
    }

    /* ── main loop ─────────────────────────────────────────────────────── */

    /// run — port of the event loop in run().
    pub fn run(&mut self) {
        if self.cfg.toast != 0 {
            self.drawmenu();
            let toast = self.cfg.toast;
            std::thread::sleep(Duration::from_micros(toast as u64 * 100_000));
            std::process::exit(0);
        }

        let mut lasttime: u32 = 0;
        let mut preselected = self.cfg.preselected;
        loop {
            let Some(ev) = self.backend.next_event() else {
                std::process::exit(1);
            };

            if preselected != 0 {
                for _ in 0..preselected {
                    if let Some(s) = self.sel {
                        if s + 1 < self.matches.len() {
                            self.sel = Some(s + 1);
                            if self.sel == self.next {
                                self.curr = self.next;
                                self.calcoffsets();
                            }
                        }
                    }
                }
                self.drawmenu();
                preselected = 0;
            }

            match ev {
                BackendEvent::Motion { time, x, y } => {
                    if time.wrapping_sub(lasttime) <= 1000 / 60 {
                        continue;
                    }
                    lasttime = time;
                    self.setselection(x, y);
                }
                BackendEvent::Destroyed => {
                    std::process::exit(1);
                }
                BackendEvent::ButtonPress { button, state, x, y } => {
                    self.buttonpress(button, state, x, y);
                }
                BackendEvent::Expose => {
                    self.present();
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
                    self.keypress(sym, state, &text);
                }
                BackendEvent::KeyRelease { sym, state } => {
                    self.keyrelease(sym, state);
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

/// INTERSECT macro: overlap area of rect (x,y,w,h) with a monitor.
fn intersect_area(x: i32, y: i32, w: i32, h: i32, mon: &crate::backend::MonitorInfo) -> i32 {
    (0.max((x + w).min(mon.x + mon.width) - x.max(mon.x)))
        * (0.max((y + h).min(mon.y + mon.height) - y.max(mon.y)))
}

fn sym_eq(a: u32, b: u32) -> bool {
    a == b
}

/// Byte-slice a string safely (C pointer arithmetic on the item text).
fn safe_slice(s: &str, from: usize, to: usize) -> &str {
    if from >= to || from > s.len() {
        return "";
    }
    if s.is_char_boundary(from) && s.is_char_boundary(to.min(s.len())) {
        &s[from..to.min(s.len())]
    } else {
        ""
    }
}
