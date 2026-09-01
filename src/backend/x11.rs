//! X11 backend — x11rb (XCB) + libxkbcommon-x11 for keysym/text lookup.

use std::os::fd::{AsRawFd, RawFd};
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xinerama;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _, CreateGCAux,
    CreateWindowAux, Cursor, EventMask, FontWrapper, GrabMode, GrabStatus, InputFocus, NotifyMode,
    PropMode, Visibility, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::xcb_ffi::XCBConnection;
use xkbcommon::xkb::x11 as xkbx11;
use xkbcommon::xkb::{self, Keycode};

use super::poll::{first_ready, poll_fds, poll_in, remaining_ms, PollOutcome};
use super::{
    scroll, translate_key, Backend, BackendEvent, EventPoll, InputSource, MenuCursor, Modifiers,
    MonitorInfo, MouseButton,
};
use crate::geom::{Point, Rect, Size};
use crate::render::{Canvas, Color};

pub struct X11Backend {
    connection: XCBConnection,
    root: Window,
    root_depth: u8,
    pub window: Window,
    parent: Window,
    pub embed: Option<Window>,
    graphics_context: Option<u32>,
    created: bool,
    managed: bool,
    pointer_grabbed: bool,
    /* cursor images for set_cursor; all NONE when loading failed, in
     * which case the default arrow stays and calls become no-ops */
    default_cursor: Cursor,
    drag_cursor: Cursor,
    resize_h_cursor: Cursor,
    /// The cursor id most recently sent to the server, for dropping
    /// repeats. The window attribute and the grab cursor both live exactly
    /// as long as the menu, so the server never resets them behind our
    /// back and this cannot go stale.
    current_cursor: Cursor,
    /// The window origin in root coordinates: exactly what create_window
    /// and resize_window last placed. Used to map pointer events that the
    /// grab delivers against the root back to menu-local coordinates.
    window_rect: Rect,

    /* keyboard (ctx/keymap keep the C-level keymap alive for the state; only
     * the state is read from) */
    #[allow(dead_code)]
    xkb_context: xkb::Context,
    #[allow(dead_code)]
    xkb_keymap: xkb::Keymap,
    xkb_state: xkb::State,

    monitors: Vec<MonitorInfo>,
    root_width: i32,
    root_height: i32,

    atoms: Atoms,
}

#[derive(Debug, Clone, Copy)]
struct Atoms {
    wm_name: u32,
    net_wm_name: u32,
    utf8_string: u32,
    clipboard: u32,
    wm_class: u32,
    string: u32,
}

impl X11Backend {
    pub fn new(embed: Option<u32>) -> Result<X11Backend, String> {
        let (connection, screen_number) =
            XCBConnection::connect(None).map_err(|e| format!("cannot open display: {e}"))?;
        let screen = connection
            .setup()
            .roots
            .get(screen_number)
            .cloned()
            .ok_or("no screen")?;
        let root = screen.root;

        let (xkb_context, xkb_keymap, xkb_state) = xkb_setup(&connection)?;
        let atoms = intern_atoms(&connection)?;
        let monitors = query_monitors(&connection);
        let (default_cursor, drag_cursor, resize_h_cursor) = load_cursors(
            &connection,
            screen_number,
        )
        .unwrap_or((x11rb::NONE, x11rb::NONE, x11rb::NONE));
        let (root_width, root_height) = (
            screen.width_in_pixels as i32,
            screen.height_in_pixels as i32,
        );

        Ok(X11Backend {
            connection,
            root,
            root_depth: screen.root_depth,
            window: 0,
            parent: embed.unwrap_or(root),
            embed,
            graphics_context: None,
            created: false,
            managed: false,
            pointer_grabbed: false,
            default_cursor,
            drag_cursor,
            resize_h_cursor,
            current_cursor: x11rb::NONE,
            window_rect: Rect::default(),
            xkb_context,
            xkb_keymap,
            xkb_state,
            monitors,
            root_width,
            root_height,
            atoms,
        })
    }

    /// keysym + utf8 text for a raw X11 keycode, keeping the xkb state fresh.
    fn lookup_key(&mut self, keycode: u8, pressed: bool) -> (u32, String) {
        /* X11 keycodes are already xkb keycodes (the 8-key offset over raw
         * evdev is included); only Wayland's raw evdev codes need a shift. */
        translate_key(&mut self.xkb_state, Keycode::new(keycode as u32), pressed)
    }

    fn flush(&self) {
        let _ = self.connection.flush();
    }

    /// Whether pointer events delivered against the grab window can be
    /// mapped back to menu-local coordinates. The window origin tracked in
    /// `window_rect` is only authoritative for a plain override-redirect
    /// child of the root: managed windows can be reparented or framed by
    /// the WM, and embed parents (`-W`) sit elsewhere in the window tree.
    fn root_events_mappable(&self) -> bool {
        !self.managed && self.parent == self.root
    }

    /// Root coordinates -> menu-local, via the tracked window origin.
    fn root_to_menu(&self, x: i16, y: i16) -> Point {
        Point::new(x as i32 - self.window_rect.x, y as i32 - self.window_rect.y)
    }

    /// The grab loop from grabfocus(): 100 tries, managed windows rename
    /// themselves instead of forcing focus. `Err` when focus never arrived.
    fn grab_focus_inner(&mut self, title: Option<&str>) -> Result<(), String> {
        if !self.created {
            return Ok(());
        }
        for _ in 0..100 {
            let focused = self
                .connection
                .get_input_focus()
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.focus)
                .unwrap_or(0);
            if focused == self.window {
                return Ok(());
            }
            if self.managed {
                if let Some(title) = title {
                    self.set_title(title);
                }
            } else {
                let _ = self.connection.set_input_focus(
                    InputFocus::PARENT,
                    self.window,
                    x11rb::CURRENT_TIME,
                );
            }
            self.flush();
            std::thread::sleep(Duration::from_millis(10));
        }
        Err("cannot grab focus".to_string())
    }

    /// Grab the pointer like a GTK context menu: with owner-events presses
    /// on our own windows are still delivered as usual, while presses
    /// anywhere else are reported to the grab window (the root) and close
    /// the menu. A failed grab only costs the outside-click behavior.
    fn grab_pointer(&mut self) {
        let ok = self
            .connection
            .grab_pointer(
                true,
                self.root,
                EventMask::BUTTON_PRESS,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::CURRENT_TIME,
                x11rb::NONE,
                x11rb::NONE,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.status == GrabStatus::SUCCESS)
            .unwrap_or(false);
        if ok {
            self.pointer_grabbed = true;
        } else {
            eprintln!("instantmenu: cannot grab pointer, clicks outside will not close the menu");
        }
        self.flush();
    }

    fn handle_event(&mut self, ev: Event) -> Option<BackendEvent> {
        match ev {
            Event::KeyPress(k) => {
                let (sym, text) = self.lookup_key(k.detail, true);
                Some(BackendEvent::KeyPress {
                    sym,
                    mods: x11_mods(k.state),
                    text,
                })
            }
            Event::KeyRelease(k) => {
                let (sym, _) = self.lookup_key(k.detail, false);
                Some(BackendEvent::KeyRelease {
                    sym,
                    mods: x11_mods(k.state),
                })
            }
            Event::ButtonPress(b) => {
                /* while the pointer grab is active, presses outside our
                 * window arrive at the grab window (the root) under
                 * owner_events=true; stamp them as External so the run
                 * loop dismisses the modal menu like a GTK context menu.
                 * The grab is created only when outside_close was set.
                 * Wheels outside are swallowed: scrolling outside should
                 * neither dismiss nor scroll the menu, only a real click
                 * dismisses. */
                if self.pointer_grabbed && b.event != self.window && matches!(b.detail, 4..=7) {
                    return None;
                }
                let on_menu = b.event == self.window || !self.pointer_grabbed;
                let source = if on_menu {
                    InputSource::Menu
                } else {
                    InputSource::External
                };
                /* vertical wheel buttons become scroll deltas; horizontal
                 * ones (6/7) have no menu action and drop here */
                if let Some(delta) = scroll::from_x11_button(b.detail) {
                    return Some(BackendEvent::Scroll { delta });
                }
                let button = MouseButton::from_x11(b.detail)?;
                Some(BackendEvent::ButtonPress {
                    button,
                    mods: x11_mods(b.state),
                    pos: Point::new(b.event_x as i32, b.event_y as i32),
                    source,
                })
            }
            Event::ButtonRelease(b) => {
                let button = MouseButton::from_x11(b.detail)?;
                if self.pointer_grabbed && b.event != self.window {
                    /* like presses: under the grab, releases outside our
                     * window are delivered against the grab window. Map
                     * them back where the stored origin is authoritative
                     * (the release position feeds the hover cursor);
                     * otherwise report External so the core ends the drag
                     * without reading unusable coordinates. */
                    let (pos, source) = if self.root_events_mappable() {
                        (self.root_to_menu(b.root_x, b.root_y), InputSource::Menu)
                    } else {
                        (
                            Point::new(b.event_x as i32, b.event_y as i32),
                            InputSource::External,
                        )
                    };
                    return Some(BackendEvent::ButtonRelease { button, pos, source });
                }
                Some(BackendEvent::ButtonRelease {
                    button,
                    pos: Point::new(b.event_x as i32, b.event_y as i32),
                    source: InputSource::Menu,
                })
            }
            Event::MotionNotify(m) => {
                if self.pointer_grabbed && m.event != self.window {
                    /* Under the grab, motion outside our window is
                     * delivered against the grab window (the root). The
                     * event carries root coordinates, so map them back
                     * through the window origin and drags keep tracking
                     * past the window edge exactly like on Wayland
                     * (snap-to-value clamps at the range ends). Where the
                     * origin is not authoritative the events are dropped:
                     * drags pause and resume on re-entry instead of being
                     * mis-mapped, and hovers wait for the pointer. */
                    if !self.root_events_mappable() {
                        return None;
                    }
                    return Some(BackendEvent::Motion {
                        time: m.time,
                        pos: self.root_to_menu(m.root_x, m.root_y),
                        source: InputSource::Menu,
                    });
                }
                Some(BackendEvent::Motion {
                    time: m.time,
                    pos: Point::new(m.event_x as i32, m.event_y as i32),
                    source: InputSource::Menu,
                })
            }
            Event::Expose(e) => {
                if e.count == 0 {
                    Some(BackendEvent::Expose)
                } else {
                    None
                }
            }
            Event::FocusIn(f) => {
                if f.event != self.window {
                    Some(BackendEvent::FocusInOther)
                } else {
                    None
                }
            }
            Event::FocusOut(f) => {
                /* only genuine focus changes — the server also emits
                 * NotifyGrab/NotifyUngrab bookkeeping around our own grabs */
                if f.event == self.window && f.mode == NotifyMode::NORMAL {
                    Some(BackendEvent::KeyboardLeft)
                } else {
                    None
                }
            }
            Event::VisibilityNotify(v) => {
                if v.state != Visibility::UNOBSCURED {
                    Some(BackendEvent::VisibilityObscured)
                } else {
                    None
                }
            }
            Event::DestroyNotify(d) => {
                if d.window == self.window {
                    Some(BackendEvent::Destroyed)
                } else {
                    None
                }
            }
            Event::SelectionNotify(s) if s.property != x11rb::NONE => self
                .selection_text(s.property)
                .map(|text| BackendEvent::SelectionNotify { text }),
            _ => None,
        }
    }

    /// Read the pasted text the selection owner stored in `property`.
    fn selection_text(&self, property: u32) -> Option<String> {
        let reply = self
            .connection
            .get_property(true, self.window, property, AtomEnum::ANY, 0, 8192 / 4 + 1)
            .ok()?;
        let prop = reply.reply().ok()?;
        Some(String::from_utf8_lossy(&prop.value).into_owned())
    }
}

impl Backend for X11Backend {
    fn monitors(&self) -> &[MonitorInfo] {
        &self.monitors
    }

    fn root_size(&self) -> Size {
        Size::new(self.root_width, self.root_height)
    }

    fn pointer_position(&mut self) -> Option<Point> {
        let reply = self
            .connection
            .query_pointer(self.root)
            .ok()?
            .reply()
            .ok()?;
        Some(Point::new(reply.root_x as i32, reply.root_y as i32))
    }

    fn focused_monitor(&self) -> Option<usize> {
        /* Queue geometry and root-coordinate translation together. */
        let reply = self.connection.get_input_focus().ok()?.reply().ok()?;
        let window = reply.focus;
        if window == self.root || window == 0 || window == 1 {
            return None; // PointerRoot(1)/None(0)
        }
        let geometry = self.connection.get_geometry(window).ok()?;
        let translated = self
            .connection
            .translate_coordinates(window, self.root, 0, 0)
            .ok()?;
        let geometry = geometry.reply().ok()?;
        let translated = translated.reply().ok()?;
        let mut best = 0usize;
        let mut area = 0;
        for (idx, monitor) in self.monitors.iter().enumerate() {
            let a = Rect::new(
                translated.dst_x as i32,
                translated.dst_y as i32,
                geometry.width as i32,
                geometry.height as i32,
            )
            .intersect_area(monitor.rect);
            if a > area {
                area = a;
                best = idx;
            }
        }
        if area == 0 {
            None
        } else {
            Some(best)
        }
    }

    fn embed_parent_size(&self) -> Option<Size> {
        let geo = self
            .connection
            .get_geometry(self.parent)
            .ok()?
            .reply()
            .ok()?;
        Some(Size::new(geo.width as i32, geo.height as i32))
    }

    fn create_window(
        &mut self,
        rect: Rect,
        border_width: i32,
        managed: bool,
        _grab: bool,
        outside_close: bool,
        class_hint: &str,
        bg: Color,
        border_color: Color,
    ) -> Result<(), String> {
        let window = self.connection.generate_id().map_err(|e| e.to_string())?;
        let attrs = CreateWindowAux::new()
            .override_redirect(u32::from(!managed))
            .background_pixel(x11_pixel(bg))
            .border_pixel(x11_pixel(border_color))
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::KEY_PRESS
                    | EventMask::KEY_RELEASE
                    | EventMask::VISIBILITY_CHANGE
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION,
            );
        self.connection
            .create_window(
                self.root_depth,
                window,
                self.parent,
                rect.x as i16,
                rect.y as i16,
                rect.w as u16,
                rect.h as u16,
                border_width as u16,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &attrs,
            )
            .map_err(|e| e.to_string())?;

        /* XClassHint { res_name, res_class } both set to the wm class */
        let mut class_bytes = Vec::new();
        class_bytes.extend_from_slice(class_hint.as_bytes());
        class_bytes.push(0);
        class_bytes.extend_from_slice(class_hint.as_bytes());
        class_bytes.push(0);
        let _ = self.connection.change_property(
            PropMode::REPLACE,
            window,
            self.atoms.wm_class,
            self.atoms.string,
            8,
            class_bytes.len() as u32,
            &class_bytes,
        );

        self.window = window;
        self.created = true;
        self.managed = managed;
        self.window_rect = rect;
        if outside_close {
            self.grab_pointer();
        }
        Ok(())
    }

    fn map_window(&mut self) {
        if self.created {
            let _ = self.connection.map_window(self.window);
        }
    }

    fn embed_setup(&mut self, pos: Point) -> Result<(), String> {
        let Some(parent) = self.embed else {
            return Ok(());
        };
        let window = self.window;
        let _ = self
            .connection
            .reparent_window(window, parent, pos.x as i16, pos.y as i16);
        let _ = self.connection.change_window_attributes(
            parent,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::FOCUS_CHANGE | EventMask::SUBSTRUCTURE_NOTIFY),
        );
        /* select FocusChangeMask on all children of the parent */
        if let Ok(reply) = self.connection.query_tree(parent) {
            if let Ok(tree) = reply.reply() {
                for child in tree.children {
                    if child == window {
                        continue;
                    }
                    let _ = self.connection.change_window_attributes(
                        child,
                        &ChangeWindowAttributesAux::new().event_mask(EventMask::FOCUS_CHANGE),
                    );
                }
            }
        }
        self.flush();
        self.grab_focus_inner(None)
    }

    fn grab_keyboard(&mut self) -> Result<(), String> {
        /* C grabkeyboard(): no-op when embedding or in managed mode */
        if self.embed.is_some() || self.managed {
            return Ok(());
        }
        /* XGrabKeyboard(owner_events=true, root) retried 1000x like the C code */
        for _ in 0..1000 {
            let ok = self
                .connection
                .grab_keyboard(
                    true,
                    self.root,
                    x11rb::CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.status == GrabStatus::SUCCESS)
                .unwrap_or(false);
            if ok {
                self.flush();
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Err("cannot grab keyboard".to_string())
    }

    fn grab_focus(&mut self, title: &str) -> Result<(), String> {
        self.grab_focus_inner(Some(title))
    }

    fn set_title(&mut self, title: &str) {
        let window = self.window;
        let _ = self.connection.change_property(
            PropMode::REPLACE,
            window,
            self.atoms.wm_name,
            self.atoms.string,
            8,
            title.len() as u32,
            title.as_bytes(),
        );
        let _ = self.connection.change_property(
            PropMode::REPLACE,
            window,
            self.atoms.net_wm_name,
            self.atoms.utf8_string,
            8,
            title.len() as u32,
            title.as_bytes(),
        );
        self.flush();
    }

    /// Cursor switching on two levels: the window attribute covers the
    /// ordinary pointer-over-the-window case, and an active outside-click
    /// grab gets its own update because the grab cursor wins while it is
    /// active (Time None leaves the grab's timing untouched). Repeats are
    /// dropped against `current_cursor`; see the field comment for why
    /// that cannot suppress a needed update.
    fn set_cursor(&mut self, cursor: MenuCursor) {
        let target = match cursor {
            MenuCursor::Default => self.default_cursor,
            MenuCursor::Drag => self.drag_cursor,
            MenuCursor::ResizeHorizontal => self.resize_h_cursor,
        };
        if target == x11rb::NONE || target == self.current_cursor {
            return; // loading failed at startup, or already in effect
        }
        self.current_cursor = target;
        let _ = self.connection.change_window_attributes(
            self.window,
            &ChangeWindowAttributesAux::new().cursor(target),
        );
        if self.pointer_grabbed {
            let _ = self.connection.change_active_pointer_grab(
                target,
                x11rb::CURRENT_TIME,
                EventMask::BUTTON_PRESS,
            );
        }
        self.flush();
    }

    fn present(&mut self, canvas: &Canvas) {
        if !self.created {
            return;
        }
        let width = canvas.width.max(0) as usize;
        let height = canvas.height.max(0) as usize;
        if width == 0 || height == 0 {
            return;
        }
        if self.graphics_context.is_none() {
            if let Ok(gcid) = self.connection.generate_id() {
                let _ = self.connection.create_gc(
                    gcid,
                    self.window,
                    &CreateGCAux::new().graphics_exposures(0),
                );
                self.graphics_context = Some(gcid);
            }
        }
        if let Some(gc) = self.graphics_context {
            let _ = self.connection.put_image(
                x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
                self.window,
                gc,
                width as u16,
                height as u16,
                0,
                0,
                0,
                self.root_depth,
                &canvas.data,
            );
        }
        self.flush();
    }

    fn raise(&mut self) {
        if self.created {
            let _ = self.connection.configure_window(
                self.window,
                &ConfigureWindowAux::new().stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
            );
            self.flush();
        }
    }

    fn resize_window(&mut self, rect: Rect) {
        if !self.created {
            return;
        }
        /* the border is a server-side X border and keeps its creation-time
         * width; only the content geometry moves here */
        let _ = self.connection.configure_window(
            self.window,
            &ConfigureWindowAux::new()
                .x(rect.x)
                .y(rect.y)
                .width(rect.w.max(1) as u32)
                .height(rect.h.max(1) as u32),
        );
        self.window_rect = rect;
        self.flush();
    }

    fn poll_event(&mut self, timeout: Option<Duration>, extra: &[RawFd]) -> EventPoll {
        let start = std::time::Instant::now();
        loop {
            match self.connection.poll_for_event() {
                Ok(Some(ev)) => {
                    if let Some(event) = self.handle_event(ev) {
                        return EventPoll::Event(event);
                    }
                    continue;
                }
                Ok(None) => {}
                Err(_) => return EventPoll::Closed,
            }

            let timeout_ms = match remaining_ms(start, timeout) {
                Ok(ms) => ms,
                Err(()) => return EventPoll::Timeout,
            };

            let mut fds: Vec<libc::pollfd> = Vec::with_capacity(1 + extra.len());
            fds.push(poll_in(self.connection.as_raw_fd()));
            let extra_start = fds.len();
            fds.extend(extra.iter().copied().map(poll_in));

            match poll_fds(&mut fds, timeout_ms) {
                PollOutcome::Timeout => return EventPoll::Timeout,
                PollOutcome::Closed => return EventPoll::Closed,
                PollOutcome::Ready => {}
            }

            /* extras first: a blocked pipe producer is more time-critical
             * than an X event that is already queued in the server */
            if let Some(i) = first_ready(&fds, extra_start) {
                return EventPoll::Readable(i - extra_start);
            }
        }
    }

    fn request_selection(&mut self, clipboard: bool) {
        let selection = if clipboard {
            self.atoms.clipboard
        } else {
            AtomEnum::PRIMARY.into()
        };
        let _ = self.connection.convert_selection(
            self.window,
            selection,
            self.atoms.utf8_string,
            self.atoms.utf8_string,
            x11rb::CURRENT_TIME,
        );
        self.flush();
    }
}

/// Set up XKB so keysyms/text match what the server keymap produces.
fn xkb_setup(
    connection: &XCBConnection,
) -> Result<(xkb::Context, xkb::Keymap, xkb::State), String> {
    let xkb_context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let (mut major, mut minor, mut base_event, mut base_error) = (0u16, 0u16, 0u8, 0u8);
    if !xkbx11::setup_xkb_extension(
        connection,
        xkbx11::MIN_MAJOR_XKB_VERSION,
        xkbx11::MIN_MINOR_XKB_VERSION,
        xkbx11::SetupXkbExtensionFlags::NoFlags,
        &mut major,
        &mut minor,
        &mut base_event,
        &mut base_error,
    ) {
        return Err("xkb setup failed".to_string());
    }
    let device = xkbx11::get_core_keyboard_device_id(connection);
    let xkb_keymap = xkbx11::keymap_new_from_device(
        &xkb_context,
        connection,
        device,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    );
    let xkb_state = xkbx11::state_new_from_device(&xkb_keymap, connection, device);
    Ok((xkb_context, xkb_keymap, xkb_state))
}

/// Intern the atoms the backend uses.
fn intern_atoms(connection: &XCBConnection) -> Result<Atoms, String> {
    let net_wm_name = connection
        .intern_atom(false, b"_NET_WM_NAME")
        .map_err(|e| e.to_string())?;
    let utf8_string = connection
        .intern_atom(false, b"UTF8_STRING")
        .map_err(|e| e.to_string())?;
    let clipboard = connection
        .intern_atom(false, b"CLIPBOARD")
        .map_err(|e| e.to_string())?;
    Ok(Atoms {
        wm_name: AtomEnum::WM_NAME.into(),
        net_wm_name: net_wm_name.reply().map_err(|e| e.to_string())?.atom,
        utf8_string: utf8_string.reply().map_err(|e| e.to_string())?.atom,
        clipboard: clipboard.reply().map_err(|e| e.to_string())?.atom,
        wm_class: AtomEnum::WM_CLASS.into(),
        string: AtomEnum::STRING.into(),
    })
}

/// Monitor list via Xinerama (like the C build with -DXINERAMA).
fn query_monitors(connection: &XCBConnection) -> Vec<MonitorInfo> {
    let mut monitors = Vec::new();
    let is_active = xinerama::is_active(connection).ok();
    let screens = xinerama::query_screens(connection).ok();
    if let Some(is_active) = is_active.and_then(|c| c.reply().ok()) {
        if is_active.state != 0 {
            if let Some(screens) = screens.and_then(|c| c.reply().ok()) {
                for (i, s) in screens.screen_info.iter().enumerate() {
                    monitors.push(MonitorInfo {
                        rect: Rect::new(
                            s.x_org as i32,
                            s.y_org as i32,
                            s.width as i32,
                            s.height as i32,
                        ),
                        name: format!("monitor {i}"),
                    });
                }
            }
        }
    }
    monitors
}

/// Cursor images for [`Backend::set_cursor`]: the default arrow, the
/// dragging hand, and the horizontal double arrow. Cursor themes are
/// preferred; the core `cursor` font — the same glyphs dwm's own cursors
/// come from — fills in when a theme lacks a name. `None` on any failure
/// (switching then stays disabled).
fn load_cursors(
    connection: &XCBConnection,
    screen_number: usize,
) -> Option<(Cursor, Cursor, Cursor)> {
    let database = x11rb::resource_manager::new_from_default(connection).ok()?;
    let handle = x11rb::cursor::Handle::new(connection, screen_number, &database)
        .ok()?
        .reply()
        .ok()?;
    let from_theme = |name: &str| handle.load_cursor(connection, name).unwrap_or(x11rb::NONE);
    let or_font = |cursor: Cursor, glyph: u16| -> Cursor {
        if cursor != x11rb::NONE {
            cursor
        } else {
            core_font_cursor(connection, glyph).unwrap_or(x11rb::NONE)
        }
    };
    let default = or_font(from_theme("left_ptr"), GLYPH_LEFT_PTR);
    let drag = or_font(from_theme("grabbing"), GLYPH_FLEUR);
    let resize_h = or_font(from_theme("ew-resize"), GLYPH_SB_H_DOUBLE_ARROW);
    (default != x11rb::NONE && drag != x11rb::NONE && resize_h != x11rb::NONE)
        .then_some((default, drag, resize_h))
}

/// Glyph numbers in the core `cursor` font (`XC_*` in X11/cursorfont.h).
const GLYPH_LEFT_PTR: u16 = 34;
const GLYPH_FLEUR: u16 = 26;
const GLYPH_SB_H_DOUBLE_ARROW: u16 = 54;

/// A cursor straight from the core `cursor` font, black on white like
/// XCreateFontCursor. `None` when the font is unavailable.
fn core_font_cursor(connection: &XCBConnection, glyph: u16) -> Option<Cursor> {
    let cursor = connection.generate_id().ok()?;
    let font = FontWrapper::open_font(connection, b"cursor").ok()?;
    connection
        .create_glyph_cursor(
            cursor,
            font.font(),
            font.font(),
            glyph,
            glyph + 1,
            // foreground color
            0,
            0,
            0,
            // background color
            u16::MAX,
            u16::MAX,
            u16::MAX,
        )
        .ok()?;
    Some(cursor)
}

/// X11 key/button mask -> semantic modifiers. Named protocol flags instead
/// of raw bits; the core only ever sees [`Modifiers`].
fn x11_mods(state: x11rb::protocol::xproto::KeyButMask) -> Modifiers {
    use x11rb::protocol::xproto::KeyButMask;

    Modifiers {
        shift: state.contains(KeyButMask::SHIFT),
        ctrl: state.contains(KeyButMask::CONTROL),
        alt: state.contains(KeyButMask::MOD1),
        logo: state.contains(KeyButMask::MOD4),
    }
}

/// X11 pixel value for a 24-bit depth: RGB in the top three bytes.
fn x11_pixel(color: Color) -> u32 {
    ((color.r() as u32) << 16) | ((color.g() as u32) << 8) | color.b() as u32
}
