//! Backend abstraction: X11 (x11rb) and Wayland (wayland-client) share the
//! same menu core; this module defines the interface both implement.

pub mod poll;
pub mod wayland;
pub mod x11;

/// Shared in-memory stub backend for tests (see [`stub`]).
#[cfg(test)]
pub(crate) mod stub;

use clap::ValueEnum;

use std::os::fd::RawFd;

use xkbcommon::xkb::{self, KeyDirection, Keycode};

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

/// A mouse button, normalized across backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

impl MouseButton {
    /// X11 pointer-button detail code -> button (core protocol buttons 1-3).
    pub(crate) fn from_x11(detail: u8) -> Option<Self> {
        match detail {
            1 => Some(MouseButton::Left),
            2 => Some(MouseButton::Middle),
            3 => Some(MouseButton::Right),
            _ => None,
        }
    }

    /// Linux evdev button code -> button (`BTN_LEFT`/`BTN_RIGHT`/
    /// `BTN_MIDDLE`, forwarded as-is by the Wayland pointer).
    pub(crate) fn from_evdev(code: u32) -> Option<Self> {
        match code {
            0x110 => Some(MouseButton::Left),
            0x111 => Some(MouseButton::Right),
            0x112 => Some(MouseButton::Middle),
            _ => None,
        }
    }
}

/// Wheel normalization shared by both backends so the direction rule is
/// written exactly once: one event per detent/axis batch, positive delta
/// scrolls down ([`BackendEvent::Scroll`]).
pub(crate) mod scroll {
    /// Magnitude of one wheel step.
    pub(crate) const STEP: i32 = 1;

    /// X11 vertical wheel buttons: 4 scrolls up, 5 down. Horizontal wheel
    /// buttons (6/7) have no menu action.
    pub(crate) fn from_x11_button(detail: u8) -> Option<i32> {
        match detail {
            4 => Some(-STEP),
            5 => Some(STEP),
            _ => None,
        }
    }

    /// Wayland vertical axis value: the sign gives the direction.
    pub(crate) fn from_axis_value(value: f64) -> i32 {
        if value > 0.0 {
            STEP
        } else {
            -STEP
        }
    }
}

/// Modifier keys held during an input event. The core only ever consumed
/// Shift/Ctrl/Alt/Mod4 of the X11 mask; this is that set, named honestly and
/// free of X11 bit values (each backend maps its own protocol state into it).
/// X11 maps `KeyButMask` flag names (`x11::x11_mods`); Wayland resolves the
/// xkb modifier names "Shift"/"Control"/"Mod1"/"Mod4"
/// (`wayland::keyboard::ModIndices`) — adding a modifier means touching both.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Mod4 / the logo key.
    pub logo: bool,
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

/// Backend-agnostic events, port of the XEvent switch in `run()`. The
/// vocabulary is semantic, not X11's: modifiers are [`Modifiers`], and wheel
/// movement is its own event instead of emulated buttons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    KeyPress {
        /// XKB keysym.
        sym: u32,
        mods: Modifiers,
        /// UTF-8 string produced by the key.
        text: String,
    },
    KeyRelease {
        sym: u32,
        mods: Modifiers,
    },
    ButtonPress {
        button: MouseButton,
        mods: Modifiers,
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
    /// Wheel movement. Positive `delta` scrolls down (towards later items),
    /// negative up. One event per detent/axis batch; no coordinates —
    /// scrolling is positional in the list, not tied to a point.
    Scroll {
        delta: i32,
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
    /// The extra fd at the given index of the `extra` slice passed to
    /// [`Backend::poll_event`] became readable (or hung up).
    Readable(usize),
    /// The timeout expired before an event arrived.
    Timeout,
    /// The backend connection died or was closed.
    Closed,
}

pub trait Backend {
    fn monitors(&self) -> &[MonitorInfo];
    /// Size of the root window / total output area (`drw->w/h`).
    fn root_size(&self) -> Size;
    /// Global pointer position (`getrootptr`). The Wayland implementation may
    /// block briefly while it maps and tears down temporary input surfaces;
    /// callers should only ask when pointer coordinates are actually needed.
    fn pointer_position(&mut self) -> Option<Point> {
        None
    }
    /// Monitor index of the focused window, if knowable (X11: input focus +
    /// geometry query; Wayland: `zwlr_foreign_toplevel_management`).
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
    /// Embedding: reparent + select input on parent + grab focus. `Err`
    /// when focus could not be taken; the menu cannot run embedded then.
    fn embed_setup(&mut self, _pos: Point) -> Result<(), String> {
        Ok(())
    }
    /// XGrabKeyboard retry loop. `Err` carries the failure message; the
    /// caller decides to exit (the C version died right there).
    fn grab_keyboard(&mut self) -> Result<(), String> {
        Ok(())
    }
    /// Acquire keyboard input before appearance configuration and font
    /// discovery. X11 performs its ordinary server grab; Wayland overrides
    /// this to map a tiny exclusive layer surface on `output` and later
    /// reuses that same surface for the menu. Keeping the operation in the
    /// backend avoids pretending Wayland has an X11-style global grab.
    fn acquire_keyboard(&mut self, _output: usize, _layer_menu: bool) -> Result<(), String> {
        self.grab_keyboard()
    }
    /// Focus grab loop; `title` is set as WM_NAME in managed mode. `Err`
    /// when focus could not be taken.
    fn grab_focus(&mut self, title: &str) -> Result<(), String>;
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
    /// Move and resize the menu window. The rect is in content coordinates,
    /// exactly like the `rect` passed to [`Backend::create_window`] (the
    /// backends add their own border handling). A no-op default is fine for
    /// surfaces whose geometry the compositor owns (managed Wayland windows).
    fn resize_window(&mut self, _rect: Rect) {}
    /// Poll for the next event, up to `timeout`. `None` waits indefinitely.
    /// `extra` fds are watched alongside the backend's own sources; when one
    /// becomes readable (or hangs up), `EventPoll::Readable` returns its
    /// index without consuming anything — the caller owns those fds. Extras
    /// are checked before queued backend events: a blocked pipe writer is
    /// more time-critical than an already-queued event.
    fn poll_event(&mut self, timeout: Option<std::time::Duration>, extra: &[RawFd]) -> EventPoll;
    /// Block for the next event, None when the connection died.
    fn next_event(&mut self) -> Option<BackendEvent> {
        match self.poll_event(None, &[]) {
            EventPoll::Event(ev) => Some(ev),
            EventPoll::Readable(_) | EventPoll::Timeout | EventPoll::Closed => None,
        }
    }
    /// Ask for the selection/clipboard contents (XConvertSelection).
    fn request_selection(&mut self, clipboard: bool);
}

/// Keysym + UTF-8 text for a key event through an xkb state, keeping the
/// state fresh. Shared by both backends; the only per-backend difference is
/// the keycode origin (X11 keycodes are already xkb keycodes, Wayland's raw
/// evdev codes need an 8-key offset applied by the backend).
pub(crate) fn translate_key(state: &mut xkb::State, code: Keycode, pressed: bool) -> (u32, String) {
    state.update_key(
        code,
        if pressed {
            KeyDirection::Down
        } else {
            KeyDirection::Up
        },
    );
    lookup_key(state, code)
}

/// Resolve a key through the current xkb state without changing its physical
/// up/down state. Wayland repeats are logical key presses between the real
/// press and release, so feeding another `KeyDirection::Down` into xkb would
/// corrupt the state machine.
pub(crate) fn lookup_key(state: &xkb::State, code: Keycode) -> (u32, String) {
    (state.key_get_one_sym(code).raw(), state.key_get_utf8(code))
}

/// Open the backend. `choice` is the `--backend` selection: `Auto` prefers
/// Wayland when WAYLAND_DISPLAY is set and falls back to X11; `X11` and
/// `Wayland` honor the explicit choice and error out instead of falling back.
/// `embed` is the `-W` window id (X11 only; ignored on Wayland).
/// `track_focused_monitor` enables Wayland's foreign-toplevel snapshot; keep
/// it false when geometry will use an explicit monitor or the pointer.
pub fn open(
    embed: Option<u32>,
    choice: BackendChoice,
    track_focused_monitor: bool,
) -> Result<Box<dyn Backend>, String> {
    match choice {
        BackendChoice::X11 => Ok(Box::new(x11::X11Backend::new(embed)?)),
        BackendChoice::Wayland => Ok(Box::new(wayland::WaylandBackend::new(
            track_focused_monitor,
        )?)),
        BackendChoice::Auto => {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                match wayland::WaylandBackend::new(track_focused_monitor) {
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
