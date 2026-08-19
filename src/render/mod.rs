//! Backend-agnostic drawing, port of `drw.c`.
//!
//! All rendering happens into a shared RGBA8 canvas which the X11 and Wayland
//! backends blit to the window. The module is split by concern: color parsing,
//! font resolution, the pixel canvas and the cosmic-text renderer.

mod canvas;
mod color;
mod font;
mod fontconfig;
mod painter;
mod renderer;

pub use canvas::Canvas;
pub use color::{parse_color, scheme_from_strings, Color, SchemeColors, SchemeStrings};
pub use font::{parse_font_name, FontSpec};
pub use painter::{Painter, ACCENT_MAX_HEIGHT, ACCENT_STRIP_HEIGHT, ACCENT_TEXT_Y_OFFSET};
pub use renderer::Renderer;
