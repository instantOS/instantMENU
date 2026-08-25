//! instantmenu — Rust port of instantMENU (suckless dmenu fork) with
//! backend-agnostic menu logic and native X11 + Wayland backends.

pub mod appearance;
pub mod backend;
pub mod cli;
pub mod config;
pub mod entry;
pub mod enums;
pub mod geom;
pub mod icons;
pub mod menu;
pub mod render;
