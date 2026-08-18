//! Menu drawing: `drawitem` and `drawmenu`.

use super::Menu;
use crate::enums::{outputoffset, ItemCategory, Scheme};

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

impl Menu {
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

    pub(super) fn drawmenu(&mut self) {
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
        let w = if self.cfg.lines > 0 || self.matches.is_empty() {
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
}
