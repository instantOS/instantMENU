//! Selection and rectangle animations, plus the left/right command triggers.

use std::time::Duration;

use super::Menu;
use crate::enums::Scheme;

/// easeOutQuint
fn ease_out_quint(t: f64) -> f64 {
    let u = t - 1.0;
    1.0 + u * u * u
}

impl Menu {
    /// animatesel — the selection flash growing from the selected row.
    pub(super) fn animatesel(&mut self) {
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
    pub(super) fn cmdtrigger(&mut self, direction: i32) {
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
}
