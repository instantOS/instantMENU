//! Slide mode (`instantmenu slide`): the GUI slider ported from islide, following the
//! instantCLI TUI semantics (steps, ninths, Home/End, drag-to-set).
//!
//! [`Slider`] is the pure state (value math only); the `impl Menu` block is
//! the shell side — the event loop, key/mouse handlers and the drawing, all
//! going through [`Transition`](super::transition::Transition) like the menu
//! handlers do. islide's trailing quirks (any-key exit, the
//! release-below-the-bar exit, `-a` suffix, multi-digit typing) are
//! deliberately not ported; instantCLI's behaviour is the reference.

use super::animate::spawn_detached;
use super::transition::Transition;
use super::Menu;
use crate::backend::{BackendEvent, InputSource, Modifiers, MouseButton};
use crate::config::SlideSettings;
use crate::enums::{ExitStatus, Scheme};
use crate::geom::{Point, Rect};
use xkbcommon::xkb::keysyms as ks;

/// Pure slider state: the value plus the range/step configuration. Every
/// mutation returns whether the value actually changed, so callers can skip
/// redraws and command dispatches for no-op edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::menu) struct Slider {
    min: i32,
    max: i32,
    value: i32,
    /// The value the slider started at (middle-click resets to it).
    initial: i32,
    step: i32,
    big_step: i32,
    command: Option<String>,
    /// A left button is held; motion snaps the value to the pointer.
    dragging: bool,
}

impl Slider {
    /// Build from (resolved or not) settings; unresolved fields fall back to
    /// the same defaults `SlideSettings::resolve` applies.
    pub(in crate::menu) fn new(s: &SlideSettings) -> Self {
        let value = s.resolved_value();
        Slider {
            min: s.min,
            max: s.max,
            value,
            initial: value,
            step: s.resolved_step(),
            big_step: s.resolved_big_step(),
            command: s.command.clone(),
            dragging: false,
        }
    }

    pub fn value(&self) -> i32 {
        self.value
    }

    /// The filled fraction of the bar (0.0..=1.0).
    fn ratio(&self) -> f64 {
        (self.value - self.min) as f64 / (self.max - self.min) as f64
    }

    /// Set the value, clamped into the range; true when it changed.
    fn set(&mut self, value: i32) -> bool {
        let value = value.clamp(self.min, self.max);
        if value == self.value {
            false
        } else {
            self.value = value;
            true
        }
    }

    /// Move by `delta`, clamped at the range ends; true when it changed.
    fn bump(&mut self, delta: i32) -> bool {
        self.set(self.value.saturating_add(delta))
    }

    /// Jump to a fraction of the range (0.0..=1.0, clamped); true when the
    /// rounded value changed.
    fn snap_to_fraction(&mut self, fraction: f64) -> bool {
        let target = self.min as f64 + (self.max - self.min) as f64 * fraction.clamp(0.0, 1.0);
        self.set(target.round() as i32)
    }
}

/// Which keysym a digit maps to in the ninths grid: 1..9 are 0/9..8/9, 0 is
/// the maximum.
fn digit_fraction(sym: u32) -> Option<f64> {
    match sym {
        s if s == ks::KEY_0 => Some(1.0),
        s if (ks::KEY_1..=ks::KEY_9).contains(&s) => Some((s - ks::KEY_1) as f64 / 9.0),
        _ => None,
    }
}

impl Menu {
    /* ── event loop ─────────────────────────────────────────────────────── */

    /// The slide-mode event loop. Driven from [`Menu::run`](super::Menu::run)
    /// when `self.slider` is set.
    pub(in crate::menu) fn run_slide(&mut self) -> ExitStatus {
        self.dispatch_slide_command();

        let mut last_time: u32 = 0;
        loop {
            let Some(ev) = self.backend.next_event() else {
                return ExitStatus::Failure;
            };
            let t = match ev {
                BackendEvent::KeyPress { sym, mods, .. } => self.slide_key(sym, mods),
                BackendEvent::KeyRelease { .. } => continue,
                BackendEvent::ButtonPress {
                    source: InputSource::External,
                    ..
                } => return ExitStatus::Failure,
                BackendEvent::ButtonPress {
                    button, mods, pos, ..
                } => self.slide_button(button, mods, pos),
                BackendEvent::ButtonRelease { button, pos, .. } => self.slide_release(button, pos),
                BackendEvent::Scroll { delta } => self.slide_scroll(delta),
                BackendEvent::Motion { time, pos, .. } => {
                    if Menu::motion_throttled(&mut last_time, time) {
                        continue;
                    }
                    self.slide_motion(pos)
                }
                BackendEvent::Destroyed => return ExitStatus::Failure,
                BackendEvent::Expose => {
                    self.backend.present(&self.canvas);
                    continue;
                }
                BackendEvent::FocusInOther => {
                    let title = self.prompt().unwrap_or("slider").to_string();
                    if self.backend.grab_focus(&title).is_err() {
                        return ExitStatus::Failure;
                    }
                    continue;
                }
                BackendEvent::VisibilityObscured => {
                    self.backend.raise();
                    continue;
                }
                BackendEvent::SelectionNotify { .. } => continue,
            };
            if let Some(status) = self.perform(t) {
                return status;
            }
        }
    }

    /* ── value changes ──────────────────────────────────────────────────── */

    /// Run a mutation on the slider state; a change redraws and dispatches
    /// the command, a no-op does nothing.
    fn slide_edit(&mut self, edit: impl FnOnce(&mut Slider) -> bool) -> Transition {
        let Some(slider) = self.slider.as_mut() else {
            return Transition::Nop;
        };
        if !edit(slider) {
            return Transition::Nop;
        }
        match &slider.command {
            // the shell runs the command, then redraws
            Some(cmd) => Transition::Spawn(format!("{cmd} {}", slider.value)),
            None => Transition::Redraw,
        }
    }

    /// Spawn --command with the current value (the initial dispatch at
    /// startup, matching instantCLI).
    fn dispatch_slide_command(&mut self) {
        let Some(slider) = self.slider.as_ref() else {
            return;
        };
        if let Some(cmd) = &slider.command {
            spawn_detached(&format!("{cmd} {}", slider.value));
        }
    }

    /// Snap the value to a pointer x position within the bar.
    fn slide_snap_to_x(&mut self, x: i32) -> Transition {
        let width = self.layout.menu_width;
        self.slide_edit(|s| s.snap_to_fraction(x as f64 / width as f64))
    }

    /* ── keyboard ───────────────────────────────────────────────────────── */

    /// The slide keymap: hjkl/arrows step, digits jump to ninths, Home/End
    /// hit the range ends, Return prints the value, Escape/q cancels.
    pub(super) fn slide_key(&mut self, sym: u32, mods: Modifiers) -> Transition {
        if mods.ctrl {
            // only the universal quit gesture is bound under Ctrl
            return if sym == ks::KEY_c {
                Transition::Exit(ExitStatus::Failure)
            } else {
                Transition::Nop
            };
        }
        if let Some(fraction) = digit_fraction(sym) {
            return self.slide_edit(|s| s.snap_to_fraction(fraction));
        }
        match sym {
            s if s == ks::KEY_Escape || s == ks::KEY_q || s == ks::KEY_Q => {
                Transition::Exit(ExitStatus::Failure)
            }
            s if s == ks::KEY_Return || s == ks::KEY_KP_Enter => {
                let value = self.slider.as_ref().map_or(0, |s| s.value());
                Transition::PrintAndExit(value.to_string())
            }
            s if s == ks::KEY_Left || s == ks::KEY_KP_Left || s == ks::KEY_h => {
                self.slide_edit(|s| s.bump(-s.step))
            }
            s if s == ks::KEY_Right || s == ks::KEY_KP_Right || s == ks::KEY_l => {
                self.slide_edit(|s| s.bump(s.step))
            }
            s if s == ks::KEY_Down || s == ks::KEY_KP_Down || s == ks::KEY_j => {
                self.slide_edit(|s| s.bump(-s.big_step))
            }
            s if s == ks::KEY_Up || s == ks::KEY_KP_Up || s == ks::KEY_k => {
                self.slide_edit(|s| s.bump(s.big_step))
            }
            s if s == ks::KEY_plus || s == ks::KEY_equal || s == ks::KEY_KP_Add => {
                self.slide_edit(|s| s.bump(1))
            }
            s if s == ks::KEY_minus || s == ks::KEY_KP_Subtract => self.slide_edit(|s| s.bump(-1)),
            s if s == ks::KEY_Home || s == ks::KEY_KP_Home => self.slide_edit(|s| s.set(s.min)),
            s if s == ks::KEY_End || s == ks::KEY_KP_End => self.slide_edit(|s| s.set(s.max)),
            // volume keys drive the small step, so the slider can be bound
            // over whatever the keys normally control
            s if s == ks::KEY_XF86AudioRaiseVolume => self.slide_edit(|s| s.bump(s.step)),
            s if s == ks::KEY_XF86AudioLowerVolume => self.slide_edit(|s| s.bump(-s.step)),
            _ => Transition::Nop,
        }
    }

    /* ── mouse ──────────────────────────────────────────────────────────── */

    pub(super) fn slide_button(
        &mut self,
        button: MouseButton,
        _mods: Modifiers,
        pos: Point,
    ) -> Transition {
        match button {
            MouseButton::Left => {
                if let Some(slider) = self.slider.as_mut() {
                    slider.dragging = true;
                }
                self.slide_snap_to_x(pos.x)
            }
            // reset to the initial value, like islide's middle click
            MouseButton::Middle => self.slide_edit(|s| s.set(s.initial)),
            MouseButton::Right => Transition::Exit(ExitStatus::Failure),
        }
    }

    /// Wheel movement steps the value: up increases, down decreases (the
    /// old wheel-button mapping).
    pub(super) fn slide_scroll(&mut self, delta: i32) -> Transition {
        self.slide_edit(|s| s.bump(if delta < 0 { s.step } else { -s.step }))
    }

    /// Left button released: end the drag. (X11 implicit grabs and Wayland's
    /// button-held pointer focus keep the motion events coming until here.)
    pub(super) fn slide_release(&mut self, button: MouseButton, _pos: Point) -> Transition {
        if button == MouseButton::Left {
            if let Some(slider) = self.slider.as_mut() {
                slider.dragging = false;
            }
        }
        Transition::Nop
    }

    pub(super) fn slide_motion(&mut self, pos: Point) -> Transition {
        let dragging = self.slider.as_ref().is_some_and(|s| s.dragging);
        if dragging {
            self.slide_snap_to_x(pos.x)
        } else {
            Transition::Nop
        }
    }

    /* ── drawing ────────────────────────────────────────────────────────── */

    /// One bar: the selected-scheme progress fill with its detail strip,
    /// and the "prompt  value" label in a normal-scheme box on top (the box
    /// keeps the label readable over the fill, islide's dark label strip).
    pub(in crate::menu) fn draw_slide(&mut self) {
        let Some(slider) = self.slider.as_ref() else {
            return;
        };
        let width = self.layout.menu_width;
        let height = self.layout.menu_height;
        let value = slider.value;
        let ratio = slider.ratio();

        let fill = (ratio * width as f64).round() as i32;

        let label = match self.prompt() {
            Some(p) if !p.is_empty() => format!("{p}  {value}"),
            _ => value.to_string(),
        };
        let lpad = self.renderer.cell_inset();
        let label_width = self.cell_width(&label);

        let mut p = self.painter();
        p.set_scheme(Scheme::Normal);
        let normal_bg = p.scheme().bg;
        p.clear(normal_bg);

        // Selected-scheme progress fill with bottom detail accent strip
        p.set_scheme(Scheme::Selected);
        p.fill_accented_rect(Rect::new(0, 0, fill, height));

        // Normal-scheme label box over the fill keeps the value readable —
        // draw_text fills the cell with the scheme background itself.
        p.set_scheme(Scheme::Normal);
        p.draw_text(Rect::new(0, 0, label_width, height), lpad, &label);

        self.backend.present(&self.canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::{digit_fraction, Slider};
    use crate::config::SlideSettings;

    fn slider(settings: SlideSettings) -> Slider {
        Slider::new(&settings)
    }

    #[test]
    fn resolve_applies_defaults_and_validates() {
        let mut s = SlideSettings::default();
        assert!(s.resolve().is_ok());
        assert_eq!(s.value, Some(50));
        assert_eq!(s.step, Some(1));
        assert_eq!(s.big_step, Some(10));

        let mut s = SlideSettings {
            min: 0,
            max: 7,
            ..SlideSettings::default()
        };
        assert!(s.resolve().is_ok());
        // a tenth of 7 is 0, so the big step floor of 5 applies
        assert_eq!(s.big_step, Some(5));

        let mut s = SlideSettings {
            min: 10,
            max: 10,
            ..SlideSettings::default()
        };
        assert!(s.resolve().is_err());
        let mut s = SlideSettings {
            min: 20,
            max: 10,
            ..SlideSettings::default()
        };
        assert!(s.resolve().is_err());
    }

    #[test]
    fn resolve_clamps_the_initial_value() {
        let mut s = SlideSettings {
            value: Some(500),
            ..SlideSettings::default()
        };
        s.resolve().unwrap();
        assert_eq!(s.value, Some(100));

        let s = SlideSettings {
            value: Some(-3),
            ..SlideSettings::default()
        };
        assert_eq!(slider(s).value(), 0);
    }

    #[test]
    fn big_step_stays_at_least_the_step() {
        let s = SlideSettings {
            step: Some(20),
            big_step: Some(5),
            ..SlideSettings::default()
        };
        assert_eq!(slider(s).big_step, 20);
    }

    #[test]
    fn bump_clamps_at_the_range_ends() {
        let mut s = slider(SlideSettings::default());
        assert!(s.bump(30));
        assert_eq!(s.value(), 80);
        assert!(s.bump(30));
        assert_eq!(s.value(), 100);
        assert!(!s.bump(30)); // no change at the maximum
        assert_eq!(s.value(), 100);
        assert!(s.bump(-1000));
        assert_eq!(s.value(), 0);
        assert!(!s.bump(-1));
    }

    #[test]
    fn negative_ranges_work() {
        let s = SlideSettings {
            min: -100,
            max: 100,
            ..SlideSettings::default()
        };
        let mut s = slider(s);
        assert_eq!(s.value(), 0);
        assert!(s.bump(-30));
        assert_eq!(s.value(), -30);
    }

    #[test]
    fn snap_to_fraction_rounds_and_clamps() {
        let mut s = slider(SlideSettings::default());
        assert!(s.snap_to_fraction(0.0));
        assert_eq!(s.value(), 0);
        assert!(s.snap_to_fraction(0.255));
        assert_eq!(s.value(), 26); // 25.5 rounds away from zero
        assert!(s.snap_to_fraction(2.0)); // out-of-range fractions clamp
        assert_eq!(s.value(), 100);
        assert!(!s.snap_to_fraction(1.0)); // already there
    }

    #[test]
    fn digit_fractions_cover_ninths() {
        assert_eq!(digit_fraction(ks_digit(1)), Some(0.0));
        assert_eq!(digit_fraction(ks_digit(5)), Some(4.0 / 9.0));
        assert_eq!(digit_fraction(ks_digit(9)), Some(8.0 / 9.0));
        assert_eq!(digit_fraction(ks_digit(0)), Some(1.0));
        assert_eq!(digit_fraction(xkbcommon::xkb::keysyms::KEY_a), None);
    }

    fn ks_digit(d: u32) -> u32 {
        xkbcommon::xkb::keysyms::KEY_1 + d - 1
    }
}
