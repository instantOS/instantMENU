//! Transition — what an event handler wants the event loop to do next.
//!
//! Handlers never print, spawn, draw or exit on their own; they return a
//! [`Transition`] and [`Menu::perform`](super::Menu::perform) is the single
//! place that interprets them. This replaces the C version's `exit()` /
//! `puts()` calls from inside the matching and input code.

use crate::enums::ExitStatus;

/// The result of handling one event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Transition {
    /// Keep running; nothing changed on screen.
    Nop,
    /// State changed; redraw the menu.
    Redraw,
    /// Print a line and keep the menu running (confirm with Ctrl held).
    Print(String),
    /// Print a line and exit successfully (confirm, instant mode, commented
    /// mode).
    PrintAndExit(String),
    /// Spawn a command detached and keep running (slide mode value change).
    Spawn(String),
    /// Spawn a command detached and exit successfully (left/right commands).
    SpawnAndExit(String),
    /// Exit with this status.
    Exit(ExitStatus),
}

impl Transition {
    /// Redraw unless the transition already ends the loop or prints (a
    /// [`Transition::Print`] redraws anyway). The C key switch expressed this
    /// as a `return true` at the end of the switch.
    pub(super) fn at_least_redraw(self) -> Self {
        match self {
            Transition::Nop => Transition::Redraw,
            other => other,
        }
    }
}
