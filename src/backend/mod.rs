//! Backend abstraction: X11 (x11rb) and Wayland (wayland-client) share the
//! same menu core; this module defines the interface both implement.

pub mod wayland;
pub mod x11;

use crate::render::{Canvas, Color};

/// A monitor, port of `XineramaScreenInfo` usage in instantmenu.c.
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub name: String,
}

/// Backend-agnostic events, port of the XEvent switch in `run()`.
#[derive(Debug, Clone)]
pub enum BackendEvent {
    KeyPress {
        /// X keysym (XKB keysym values).
        sym: u32,
        /// X11-style modifier mask (ShiftMask etc.).
        state: u32,
        /// UTF-8 string produced by the key.
        text: String,
    },
    KeyRelease {
        sym: u32,
        state: u32,
    },
    ButtonPress {
        /// X11 button number (1 left, 2 middle, 3 right, 4/5 wheel).
        button: u8,
        state: u32,
        x: i32,
        y: i32,
    },
    Motion {
        /// Server timestamp (ms) for the 60fps throttle.
        time: u32,
        x: i32,
        y: i32,
    },
    /// Redraw needed (Expose on X11).
    Expose,
    /// Focus went to another window — regrab focus.
    FocusInOther,
    /// Window got obscured — raise it again.
    VisibilityObscured,
    /// Our window was destroyed.
    Destroyed,
    /// Selection contents arrived after `request_selection`.
    SelectionNotify {
        text: String,
    },
}

pub trait Backend {
    fn monitors(&self) -> &[MonitorInfo];
    /// Size of the root window / total output area (`drw->w/h`).
    fn root_size(&self) -> (i32, i32);
    /// Root pointer position (`getrootptr`).
    fn pointer_position(&self) -> Option<(i32, i32)>;
    /// Monitor index of the focused window, if known (X11 only).
    fn focused_monitor(&self) -> Option<usize>;
    /// Size of the embedding parent window (`-W`), or the root window when
    /// not embedding. None when there is no parent to query.
    fn embed_parent_size(&self) -> Option<(i32, i32)>;

    /// Create the menu window (XCreateWindow in setup()).
    /// `grab` = whether the keyboard should be grabbed (Wayland layer-shell
    /// keyboard interactivity; X11 grabs separately).
    fn create_window(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        border_width: i32,
        managed: bool,
        grab: bool,
        class_hint: &str,
        bg: Color,
        border_color: Color,
    ) -> Result<(), String>;
    /// XMapRaised + embedding reparenting when `-W` was given.
    fn map_window(&mut self);
    /// Embedding: reparent + select input on parent + grab focus.
    fn embed_setup(&mut self, x: i32, y: i32);
    /// XGrabKeyboard retry loop (dies on failure like the C version).
    fn grab_keyboard(&mut self);
    /// Focus grab loop; `title` is set as WM_NAME in managed mode.
    fn grab_focus(&mut self, title: &str);
    /// Set the window title (WM_NAME / _NET_WM_NAME).
    fn set_title(&mut self, title: &str);

    /// Blit the canvas to the window (`drw_map`).
    fn present(&mut self, canvas: &Canvas);
    /// Raise the window (XRaiseWindow on VisibilityNotify).
    fn raise(&mut self);
    /// Block for the next event, None when the connection died.
    fn next_event(&mut self) -> Option<BackendEvent>;
    /// Ask for the selection/clipboard contents (XConvertSelection).
    fn request_selection(&mut self, clipboard: bool);
    /// X resource "key -> value" pairs (X11 only, empty on Wayland).
    fn resource_pairs(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    fn is_wayland(&self) -> bool;
}

/// Modifier masks, X11 values (both backends map into these).
pub const SHIFT_MASK: u32 = 1 << 0;
pub const LOCK_MASK: u32 = 1 << 1;
pub const CONTROL_MASK: u32 = 1 << 2;
pub const MOD1_MASK: u32 = 1 << 3;
pub const MOD2_MASK: u32 = 1 << 4;
pub const MOD3_MASK: u32 = 1 << 5;
pub const MOD4_MASK: u32 = 1 << 6;
pub const MOD5_MASK: u32 = 1 << 7;

/// Open the backend: Wayland when WAYLAND_DISPLAY is set, else X11.
/// `embed` is the `-W` window id (X11 only; ignored on Wayland).
pub fn open(embed: Option<u32>) -> Result<Box<dyn Backend>, String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        match wayland::WaylandBackend::new() {
            Ok(b) => return Ok(Box::new(b)),
            Err(e) => eprintln!("instantmenu: wayland connection failed ({e}), trying X11"),
        }
    }
    let x11 = x11::X11Backend::new(embed)?;
    Ok(Box::new(x11))
}
