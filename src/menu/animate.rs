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
    /// animate_selection — the selection flash growing from the selected row.
    pub(super) fn animate_selection(&mut self) {
        if !self.cfg.animated || self.cfg.frame_count == 0 {
            return;
        }
        let sc = self.renderer.scheme(Scheme::Selected as usize);
        self.renderer.setscheme(sc);
        let frame_count = self.cfg.frame_count;
        for time in 0..frame_count {
            let t = time as f64 / frame_count as f64;
            // bottom animation
            if self.selected_y + self.cfg.line_height < self.menu_height - 10 {
                let h = ease_out_quint(t)
                    * (self.menu_height - (self.cfg.line_height - 4) - self.selected_y) as f64;
                self.renderer.rect(
                    &mut self.canvas,
                    0,
                    self.selected_y + (self.cfg.line_height - 4),
                    self.menu_width,
                    h as i32,
                    true,
                    true,
                    false,
                );
            }
            // top animation
            let top_height = ease_out_quint(t) * self.selected_y as f64;
            let top_y =
                (self.selected_y + 4) as f64 - ease_out_quint(t) * (self.selected_y + 4) as f64;
            self.renderer.rect(
                &mut self.canvas,
                0,
                top_y as i32,
                self.menu_width,
                top_height as i32,
                true,
                true,
                false,
            );
            self.present();
            std::thread::sleep(Duration::from_micros(19000));
        }
    }

    /// animate_rect — animate a rectangle from (x1,y1,w1,h1) to (x2,y2,w2,h2).
    fn animate_rect(
        &mut self,
        x1: i32,
        y1: i32,
        w1: i32,
        h1: i32,
        x2: i32,
        y2: i32,
        w2: i32,
        h2: i32,
    ) {
        if !self.cfg.animated || self.cfg.frame_count == 0 {
            return;
        }
        let sc = self.renderer.scheme(Scheme::Selected as usize);
        self.renderer.setscheme(sc);
        let frame_count = self.cfg.frame_count;
        for time in 0..frame_count {
            let f = ease_out_quint(time as f64 / frame_count as f64);
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

    /// trigger_command — run the left/right command with its animation.
    pub(super) fn trigger_command(&mut self, direction: i32) {
        self.cfg.animated = true;
        let cmd = if direction != 0 {
            let c = self
                .cfg
                .right_command
                .clone()
                .or_else(|| self.cfg.left_command.clone());
            self.animate_rect(
                self.menu_width + self.cfg.border_width,
                0,
                0,
                self.menu_height,
                0,
                0,
                self.menu_width,
                self.menu_height,
            );
            c
        } else {
            let c = self
                .cfg
                .left_command
                .clone()
                .or_else(|| self.cfg.right_command.clone());
            self.animate_rect(0, 0, 0, self.menu_height, 0, 0, self.menu_width, self.menu_height);
            c
        };

        match cmd {
            Some(c) => self.spawn(&c),
            None => self.finish(0),
        }
    }
}
