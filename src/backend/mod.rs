//! Backend abstraction: X11 (x11rb) and Wayland (wayland-client) share the
//! same menu core; this module defines the interface both implement.

pub mod wayland;
pub mod x11;

use clap::ValueEnum;

use crate::geom::{Point, Rect, Size};
use crate::render::{Canvas, Color};

/// Backend selection for `--backend`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BackendChoice {
    /// Auto-detect: Wayland when WAYLAND_DISPLAY is set, else X11.
    Auto,
    /// X11 (runs on Xwayland when started from a Wayland session).
    X11,
    /// Wayland (layer-shell / xdg-shell).
    Wayland,
}

/// A monitor, port of `XineramaScreenInfo` usage in instantmenu.c.
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// The monitor's bounds in root coordinates.
    pub rect: Rect,
    pub name: String,
}

/// A mouse button (or scroll-wheel direction), normalized across backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    ScrollUp,
    ScrollDown,
}

/// Which surface a pointer event arrived on. Backends stamp every pointer
/// event at dispatch time; the run loop dismisses the modal menu whenever a
/// button arrives with `External` — the shared rule that replaces the
/// per-backend "outside click" plumbing (X11 pointer grab, Wayland
/// click-catcher shields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSource {
    /// On the menu window itself; coordinates are menu-local.
    Menu,
    /// Anywhere outside the menu window: under the X11 pointer grab this is
    /// `b.event != window`, on Wayland it is a click on a shield surface.
    External,
}

/// Backend-agnostic events, port of the XEvent switch in `run()`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
        button: MouseButton,
        state: u32,
        pos: Point,
        source: InputSource,
    },
    ButtonRelease {
        button: MouseButton,
        pos: Point,
        source: InputSource,
    },
    Motion {
        /// Server timestamp (ms) for the 60fps throttle.
        time: u32,
        pos: Point,
        /// `Menu` when the pointer is over the menu window; the event is
        /// dropped otherwise, so this is always `Menu` when present.
        source: InputSource,
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

/// Result of polling for a backend event with an optional timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventPoll {
    /// An event arrived from the backend.
    Event(BackendEvent),
    /// The timeout expired before an event arrived.
    Timeout,
    /// The backend connection died or was closed.
    Closed,
}

pub trait Backend {
    fn monitors(&self) -> &[MonitorInfo];
    /// Size of the root window / total output area (`drw->w/h`).
    fn root_size(&self) -> Size;
    /// Root pointer position (`getrootptr`).
    fn pointer_position(&self) -> Option<Point> {
        None
    }
    /// Monitor index of the focused window, if known (X11 only).
    fn focused_monitor(&self) -> Option<usize> {
        None
    }
    /// Size of the embedding parent window (`-W`), or the root window when
    /// not embedding. None when there is no parent to query.
    fn embed_parent_size(&self) -> Option<Size> {
        None
    }

    /// Create the menu window (XCreateWindow in setup()).
    /// `grab` = whether the keyboard should be grabbed (Wayland layer-shell
    /// keyboard interactivity; X11 grabs separately). `outside_close` = the
    /// menu is modal, so the backend should arrange for clicks outside the
    /// menu to be observed (pointer grab on X11, click-catcher surfaces on
    /// Wayland); the backends stamp the resulting button events with
    /// [`InputSource::External`] and the run loop dismisses on them.
    #[allow(clippy::too_many_arguments)] // port of the XCreateWindow call
    fn create_window(
        &mut self,
        rect: Rect,
        border_width: i32,
        managed: bool,
        grab: bool,
        outside_close: bool,
        class_hint: &str,
        bg: Color,
        border_color: Color,
    ) -> Result<(), String>;
    /// XMapRaised + embedding reparenting when `-W` was given.
    fn map_window(&mut self) {}
    /// Embedding: reparent + select input on parent + grab focus.
    fn embed_setup(&mut self, _pos: Point) {}
    /// XGrabKeyboard retry loop (dies on failure like the C version).
    fn grab_keyboard(&mut self) {}
    /// Focus grab loop; `title` is set as WM_NAME in managed mode.
    fn grab_focus(&mut self, title: &str);
    /// Set the window title (WM_NAME / _NET_WM_NAME).
    fn set_title(&mut self, title: &str);

    /// Blit the canvas to the window (`drw_map`).
    fn present(&mut self, canvas: &Canvas);
    /// Block until the frame committed by the last `present` is on screen.
    /// The default paces animations at a fixed ~19 ms; Wayland overrides this
    /// to wait for the compositor's frame callback (vsync).
    fn wait_frame(&mut self) {
        std::thread::sleep(std::time::Duration::from_micros(19000));
    }
    /// Raise the window (XRaiseWindow on VisibilityNotify).
    fn raise(&mut self) {}
    /// Poll for the next event, up to `timeout`. `None` waits indefinitely.
    fn poll_event(&mut self, timeout: Option<std::time::Duration>) -> EventPoll;
    /// Block for the next event, None when the connection died.
    fn next_event(&mut self) -> Option<BackendEvent> {
        match self.poll_event(None) {
            EventPoll::Event(ev) => Some(ev),
            EventPoll::Timeout | EventPoll::Closed => None,
        }
    }
    /// Ask for the selection/clipboard contents (XConvertSelection).
    fn request_selection(&mut self, clipboard: bool);
    /// X resource "key -> value" pairs (X11 only, empty on Wayland).
    fn resource_pairs(&self) -> Vec<(String, String)> {
        Vec::new()
    }
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

/// Offset added to raw evdev keycodes (Wayland) to get an xkb keycode.
/// X11 keycodes already include this offset.
pub const XKB_OFFSET: u32 = 8;

/// Open the backend. `choice` is the `--backend` selection: `Auto` prefers
/// Wayland when WAYLAND_DISPLAY is set and falls back to X11; `X11` and
/// `Wayland` honor the explicit choice and error out instead of falling back.
/// `embed` is the `-W` window id (X11 only; ignored on Wayland).
pub fn open(embed: Option<u32>, choice: BackendChoice) -> Result<Box<dyn Backend>, String> {
    match choice {
        BackendChoice::X11 => Ok(Box::new(x11::X11Backend::new(embed)?)),
        BackendChoice::Wayland => Ok(Box::new(wayland::WaylandBackend::new()?)),
        BackendChoice::Auto => {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                match wayland::WaylandBackend::new() {
                    Ok(b) => return Ok(Box::new(b)),
                    Err(e) => {
                        eprintln!("instantmenu: wayland connection failed ({e}), trying X11")
                    }
                }
            }
            Ok(Box::new(x11::X11Backend::new(embed)?))
        }
    }
}
