//! Selection and rectangle animations, plus the left/right command triggers.
//! Animations are shell effects (they present frames through the backend);
//! command triggers return a [`Transition`] instead of spawning and exiting
//! here.

use super::transition::Transition;
use super::Menu;
use crate::enums::{ColorRole, ExitStatus, Scheme, Side};
use crate::geom::Rect;

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
        let frame_count = self.cfg.frame_count;
        let menu_width = self.layout.menu_width;
        let menu_height = self.layout.menu_height;
        let line_height = self.cfg.line_height.pixels();
        let selected_y = self.selected_y;

        for time in 1..=frame_count {
            let t = time as f64 / frame_count as f64;
            let mut p = self.painter();
            p.set_scheme(Scheme::Selected);

            // bottom animation
            if selected_y + line_height < menu_height - 10 {
                let h = ease_out_quint(t) * (menu_height - (line_height - 4) - selected_y) as f64;
                p.fill_rect(
                    Rect::new(0, selected_y + (line_height - 4), menu_width, h as i32),
                    ColorRole::Background,
                );
            }
            // top animation
            let top_height = ease_out_quint(t) * selected_y as f64;
            let top_y = (selected_y + 4) as f64 - ease_out_quint(t) * (selected_y + 4) as f64;
            p.fill_rect(
                Rect::new(0, top_y as i32, menu_width, top_height as i32),
                ColorRole::Background,
            );
            self.backend.present(&self.canvas);
            self.backend.wait_frame();
        }
    }

    /// animate_rect — animate a rectangle from `from` to `to`. The C
    /// version skipped this without -a, but its only caller
    /// (trigger_command) forced the flag on first, so the animation
    /// always runs when frames are enabled.
    fn animate_rect(&mut self, from: Rect, to: Rect) {
        if self.cfg.frame_count == 0 {
            return;
        }
        let frame_count = self.cfg.frame_count;
        for time in 1..=frame_count {
            let f = ease_out_quint(time as f64 / frame_count as f64);
            let mut p = self.painter();
            p.set_scheme(Scheme::Selected);
            p.fill_rect(from.lerp(to, f), ColorRole::Background);
            self.backend.present(&self.canvas);
            self.backend.wait_frame();
        }
    }

    /// trigger_command — animate towards the side and run the left/right
    /// command (or just exit when none is configured).
    pub(super) fn trigger_command(&mut self, side: Side) -> Transition {
        let menu_width = self.layout.menu_width;
        let menu_height = self.layout.menu_height;
        let border_width = self.cfg.border_width;
        let full = Rect::new(0, 0, menu_width, menu_height);
        let c = match side {
            Side::Right => {
                let c = self
                    .cfg
                    .right_command
                    .clone()
                    .or_else(|| self.cfg.left_command.clone());
                self.animate_rect(
                    Rect::new(menu_width + border_width, 0, 0, menu_height),
                    full,
                );
                c
            }
            Side::Left => {
                let c = self
                    .cfg
                    .left_command
                    .clone()
                    .or_else(|| self.cfg.right_command.clone());
                self.animate_rect(Rect::new(0, 0, 0, menu_height), full);
                c
            }
        };
        match c {
            Some(c) => Transition::SpawnAndExit(c),
            None => Transition::Exit(ExitStatus::Success),
        }
    }
}

/// spawn — run a command detached: stdout and stderr go nowhere, stdin is
/// inherited (like the C `system(cmd + " &> /dev/null")`).
pub(super) fn spawn_detached(cmd: &str) {
    let _ = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
