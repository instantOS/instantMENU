//! X11 backend — x11rb (XCB) + libxkbcommon-x11 for keysym/text lookup.

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt, CreateGCAux,
    EventMask, GrabMode, GrabStatus, PropMode, Visibility, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use xkbcommon::xkb::{self, Keycode, KeyDirection};
use xkbcommon::xkb::x11 as xkbx11;

use super::{Backend, BackendEvent, MonitorInfo};
use crate::render::{Canvas, Color};

/// X11 keycode -> xkb keycode offset.
const XKB_OFFSET: u32 = 8;

pub struct X11Backend {
    conn: RustConnection,
    screen_num: usize,
    root: Window,
    pub win: Window,
    parent: Window,
    pub embed: Option<Window>,
    gc: Option<u32>,
    created: bool,
    managed: bool,

    /* keyboard */
    xkb_ctx: xkb::Context,
    xkb_keymap: xkb::Keymap,
    xkb_state: xkb::State,

    monitors: Vec<MonitorInfo>,
    root_w: i32,
    root_h: i32,

    atoms: Atoms,

    /* put_image scratch (BGRA swap buffer) */
    blit_buf: Vec<u8>,
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
    pub fn new() -> Result<X11Backend, String> {
        let conn =
            RustConnection::connect(None).map_err(|e| format!("cannot open display: {e}"))?;
        let screen_num = conn.primary_screen_number();
        let screen = conn
            .setup()
            .roots
            .get(screen_num)
            .cloned()
            .ok_or("no screen")?;
        let root = screen.root;

        /* set up XKB so keysyms/text match what the server keymap produces */
        let xkb_ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let (mut major, mut minor, mut base_event, mut base_error) =
            (0u16, 0u16, 0u8, 0u8);
        if !xkbx11::setup_xkb_extension(
            &conn,
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
        let device = xkbx11::get_core_keyboard_device_id(&conn);
        let xkb_keymap = xkbx11::keymap_new_from_device(
            &xkb_ctx,
            &conn,
            device,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        );
        let xkb_state = xkbx11::state_new_from_device(&xkb_keymap, &conn, device);

        let atom = |name: &str| -> Result<u32, String> {
            conn.intern_atom(false, name.as_bytes())
                .map_err(|e| e.to_string())?
                .reply()
                .map(|r| r.atom)
                .map_err(|e| e.to_string())
        };
        let atoms = Atoms {
            wm_name: AtomEnum::WM_NAME.into(),
            net_wm_name: atom("_NET_WM_NAME")?,
            utf8_string: atom("UTF8_STRING")?,
            clipboard: atom("CLIPBOARD")?,
            wm_class: AtomEnum::WM_CLASS.into(),
            string: AtomEnum::STRING.into(),
        };

        /* monitor list via Xinerama (like the C build with -DXINERAMA) */
        let mut monitors = Vec::new();
        if conn.extension_information().xinerama.is_some() {
            if let Ok(reply) = conn.xinerama().query_screens() {
                if let Ok(screens) = reply.reply() {
                    for (i, s) in screens.screen_info.iter().enumerate() {
                        monitors.push(MonitorInfo {
                            x: s.x_org as i32,
                            y: s.y_org as i32,
                            width: s.width as i32,
                            height: s.height as i32,
                            name: format!("monitor {i}"),
                        });
                    }
                }
            }
        }

        let (root_w, root_h) =
            (screen.width_in_pixels as i32, screen.height_in_pixels as i32);

        Ok(X11Backend {
            conn,
            screen_num,
            root,
            win: 0,
            parent: root,
            embed: None,
            gc: None,
            created: false,
            managed: false,
            xkb_ctx,
            xkb_keymap,
            xkb_state,
            monitors,
            root_w,
            root_h,
            atoms,
            blit_buf: Vec::new(),
        })
    }

    fn screen(&self) -> x11rb::protocol::xproto::Screen {
        self.conn.setup().roots[self.screen_num].clone()
    }

    /// Read the RESOURCE_MANAGER property of the root window and return
    /// "key -> value" pairs (a small Xrm stand-in).
    pub fn resource_pairs(&self) -> Vec<(String, String)> {
        let Ok(reply) = self.conn.get_property(
            false,
            self.root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            0,
            1 << 16,
        ) else {
            return Vec::new();
        };
        let Ok(reply) = reply.reply() else {
            return Vec::new();
        };
        let Ok(text) = String::from_utf8(reply.value) else {
            return Vec::new();
        };

        /* join backslash-continued lines, then split into key: value */
        let mut out = Vec::new();
        let mut pending = String::new();
        for line in text.split('\n') {
            let line = line.trim_end_matches('\r');
            if let Some(cont) = line.strip_suffix('\\') {
                pending.push_str(cont);
                continue;
            }
            let mut full = std::mem::take(&mut pending);
            full.push_str(line);
            if let Some((key, value)) = full.split_once(':') {
                let key = key.trim();
                /* strip a leading program prefix ("instantmenu." or "*") */
                let key = key
                    .strip_prefix("instantmenu.")
                    .or_else(|| key.strip_prefix('*'))
                    .unwrap_or(key);
                out.push((key.trim().to_string(), value.trim().to_string()));
            }
        }
        out
    }

    /// keysym + utf8 text for a raw X11 keycode, keeping the xkb state fresh.
    fn lookup_key(&mut self, keycode: u8, pressed: bool) -> (u32, String) {
        let code = Keycode::new(keycode as u32 + XKB_OFFSET);
        self.xkb_state.update_key(
            code,
            if pressed { KeyDirection::Down } else { KeyDirection::Up },
        );
        let sym = self.xkb_state.key_get_one_sym(code).raw();
        let text = self.xkb_state.key_get_utf8(code);
        (sym, text)
    }

    fn flush(&self) {
        let _ = self.conn.flush();
    }

    /// The grab loop from grabfocus(): 100 tries, managed windows rename
    /// themselves instead of forcing focus.
    fn grab_focus_inner(&mut self, title: Option<&str>) {
        if !self.created {
            return;
        }
        for _ in 0..100 {
            let focused = self
                .conn
                .get_input_focus()
                .and_then(|c| c.reply())
                .map(|r| r.focus)
                .unwrap_or(0);
            if focused == self.win {
                return;
            }
            if self.managed {
                if let Some(title) = title {
                    self.set_title(title);
                }
            } else {
                let _ = self.conn.set_input_focus(
                    self.win,
                    x11rb::protocol::xproto::InputFocus::PARENT,
                    x11rb::CURRENT_TIME,
                );
            }
            self.flush();
            std::thread::sleep(Duration::from_millis(10));
        }
        eprintln!("instantmenu: cannot grab focus");
        std::process::exit(1);
    }
}

impl Backend for X11Backend {
    fn monitors(&self) -> &[MonitorInfo] {
        &self.monitors
    }

    fn root_size(&self) -> (i32, i32) {
        (self.root_w, self.root_h)
    }

    fn pointer_position(&self) -> Option<(i32, i32)> {
        let reply = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some((reply.root_x as i32, reply.root_y as i32))
    }

    fn focused_monitor(&self) -> Option<usize> {
        /* find the top-level window containing the current input focus */
        let reply = self.conn.get_input_focus().ok()?.reply().ok()?;
        let mut w = reply.focus;
        if w == self.root || w == 0 || w == 1 {
            return None; // PointerRoot(1)/None(0)
        }
        let mut pw = w;
        while w != self.root && w != pw {
            let tree = self.conn.query_tree(w).ok()?.reply().ok()?;
            pw = w;
            w = tree.parent;
        }
        let attrs = self.conn.get_geometry(pw).ok()?.reply().ok()?;
        let mut best = 0usize;
        let mut area = 0;
        for (idx, mon) in self.monitors.iter().enumerate() {
            let a = intersect(
                attrs.x as i32,
                attrs.y as i32,
                attrs.width as i32,
                attrs.height as i32,
                mon,
            );
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

    fn embed_parent_size(&self) -> Option<(i32, i32)> {
        let geo = self.conn.get_geometry(self.parent).ok()?.reply().ok()?;
        Some((geo.width as i32, geo.height as i32))
    }

    fn create_window(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        border_width: i32,
        managed: bool,
        class_hint: &str,
        bg: Color,
        border_color: Color,
    ) -> Result<(), String> {
        let screen = self.screen();
        let win = self.conn.generate_id().map_err(|e| e.to_string())?;
        let attrs = ChangeWindowAttributesAux::new()
            .override_redirect(!managed as u32)
            .background_pixel(x11_pixel(bg))
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::KEY_PRESS
                    | EventMask::KEY_RELEASE
                    | EventMask::VISIBILITY_CHANGE
                    | EventMask::BUTTON_PRESS
                    | EventMask::POINTER_MOTION,
            );
        self.conn
            .create_window(
                screen.root_depth,
                win,
                self.root,
                x as i16,
                y as i16,
                w as u16,
                h as u16,
                border_width as u16,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &attrs,
            )
            .map_err(|e| e.to_string())?;
        self.conn
            .change_window_attributes(
                win,
                &ChangeWindowAttributesAux::new().border_pixel(x11_pixel(border_color)),
            )
            .map_err(|e| e.to_string())?;

        /* XClassHint { res_name, res_class } both set to the wm class */
        let mut class_bytes = Vec::new();
        class_bytes.extend_from_slice(class_hint.as_bytes());
        class_bytes.push(0);
        class_bytes.extend_from_slice(class_hint.as_bytes());
        class_bytes.push(0);
        let _ = self.conn.change_property(
            PropMode::REPLACE,
            win,
            self.atoms.wm_class,
            self.atoms.string,
            8,
            class_bytes.len() as u32,
            &class_bytes,
        );

        self.win = win;
        self.created = true;
        self.managed = managed;
        self.flush();
        Ok(())
    }

    fn map_window(&mut self) {
        if self.created {
            let _ = self.conn.map_window(self.win);
            self.flush();
        }
    }

    fn embed_setup(&mut self, x: i32, y: i32) {
        let Some(parent) = self.embed else { return };
        let win = self.win;
        let _ = self.conn.reparent_window(win, parent, x as i16, y as i16);
        let _ = self.conn.change_window_attributes(
            parent,
            &ChangeWindowAttributesAux::new()
                .event_mask(EventMask::FOCUS_CHANGE | EventMask::SUBSTRUCTURE_NOTIFY),
        );
        /* select FocusChangeMask on all children of the parent */
        if let Ok(reply) = self.conn.query_tree(parent) {
            if let Ok(tree) = reply.reply() {
                for child in tree.children {
                    if child == win {
                        continue;
                    }
                    let _ = self.conn.change_window_attributes(
                        child,
                        &ChangeWindowAttributesAux::new().event_mask(EventMask::FOCUS_CHANGE),
                    );
                }
            }
        }
        self.flush();
        self.grab_focus_inner(None);
    }

    fn grab_keyboard(&mut self) {
        /* XGrabKeyboard(owner_events=true, root) retried 50x like the C code */
        for _ in 0..50 {
            let ok = self
                .conn
                .grab_keyboard(
                    true,
                    self.root,
                    x11rb::CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .and_then(|c| c.reply())
                .map(|r| r.status == GrabStatus::OK)
                .unwrap_or(false);
            if ok {
                self.flush();
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        eprintln!("instantmenu: cannot grab keyboard");
        std::process::exit(1);
    }

    fn grab_focus(&mut self, title: &str) {
        self.grab_focus_inner(Some(title));
    }

    fn set_title(&mut self, title: &str) {
        let win = self.win;
        let _ = self.conn.change_property(
            PropMode::REPLACE,
            win,
            self.atoms.wm_name,
            self.atoms.string,
            8,
            title.len() as u32,
            title.as_bytes(),
        );
        let _ = self.conn.change_property(
            PropMode::REPLACE,
            win,
            self.atoms.net_wm_name,
            self.atoms.utf8_string,
            8,
            title.len() as u32,
            title.as_bytes(),
        );
        self.flush();
    }

    fn present(&mut self, canvas: &Canvas) {
        if !self.created {
            return;
        }
        let screen = self.screen();

        let w = canvas.width.max(0) as usize;
        let h = canvas.height.max(0) as usize;
        if w == 0 || h == 0 {
            return;
        }
        /* RGBA canvas -> BGRA ZPixmap rows (32bpp data into the root depth) */
        self.blit_buf.resize(w * h * 4, 0);
        for i in 0..w * h {
            let src = &canvas.data[i * 4..i * 4 + 4];
            self.blit_buf[i * 4] = src[2];
            self.blit_buf[i * 4 + 1] = src[1];
            self.blit_buf[i * 4 + 2] = src[0];
            self.blit_buf[i * 4 + 3] = src[3];
        }

        if self.gc.is_none() {
            if let Ok(gcid) = self.conn.generate_id() {
                let _ = self
                    .conn
                    .create_gc(gcid, self.win, &CreateGCAux::new().graphics_exposures(false));
                self.gc = Some(gcid);
            }
        }
        if let Some(gc) = self.gc {
            let _ = self.conn.put_image(
                x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
                self.win,
                gc,
                w as u16,
                h as u16,
                0,
                0,
                0,
                screen.root_depth,
                &self.blit_buf,
            );
        }
        self.flush();
    }

    fn raise(&mut self) {
        if self.created {
            let _ = self.conn.configure_window(
                self.win,
                &ConfigureWindowAux::new()
                    .stack_mode(x11rb::protocol::xproto::StackMode::ABOVE),
            );
            self.flush();
        }
    }

    fn next_event(&mut self) -> Option<BackendEvent> {
        loop {
            let ev = match self.conn.wait_for_event() {
                Ok(ev) => ev,
                Err(_) => return None,
            };
            match ev {
                Event::KeyPress(k) => {
                    let (sym, text) = self.lookup_key(k.detail, true);
                    return Some(BackendEvent::KeyPress { sym, state: k.state, text });
                }
                Event::KeyRelease(k) => {
                    let (sym, _) = self.lookup_key(k.detail, false);
                    return Some(BackendEvent::KeyRelease { sym, state: k.state });
                }
                Event::ButtonPress(b) => {
                    return Some(BackendEvent::ButtonPress {
                        button: b.detail as u8,
                        state: b.state,
                        x: b.event_x as i32,
                        y: b.event_y as i32,
                    });
                }
                Event::MotionNotify(m) => {
                    return Some(BackendEvent::Motion {
                        time: m.time,
                        x: m.event_x as i32,
                        y: m.event_y as i32,
                    });
                }
                Event::Expose(e) => {
                    if e.count == 0 {
                        return Some(BackendEvent::Expose);
                    }
                }
                Event::FocusIn(f) => {
                    if f.event != self.win {
                        return Some(BackendEvent::FocusInOther);
                    }
                }
                Event::VisibilityNotify(v) => {
                    if v.state != Visibility::UNOBSCURED {
                        return Some(BackendEvent::VisibilityObscured);
                    }
                }
                Event::DestroyNotify(d) => {
                    if d.window == self.win {
                        return Some(BackendEvent::Destroyed);
                    }
                }
                Event::SelectionNotify(s) => {
                    if s.property != x11rb::NONE {
                        if let Ok(reply) = self.conn.get_property(
                            true,
                            self.win,
                            s.property,
                            AtomEnum::ANY,
                            0,
                            8192 / 4 + 1,
                        ) {
                            if let Ok(prop) = reply.reply() {
                                let text = String::from_utf8_lossy(&prop.value).into_owned();
                                return Some(BackendEvent::SelectionNotify { text });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn request_selection(&mut self, clipboard: bool) {
        let selection = if clipboard {
            self.atoms.clipboard
        } else {
            AtomEnum::PRIMARY.into()
        };
        let _ = self.conn.convert_selection(
            self.win,
            selection,
            self.atoms.utf8_string,
            self.atoms.utf8_string,
            x11rb::CURRENT_TIME,
        );
        self.flush();
    }

    fn is_wayland(&self) -> bool {
        false
    }
}

/// Set the embedding window (called from main before setup).
impl X11Backend {
    pub fn set_embed(&mut self, embed: u32) {
        self.embed = Some(embed);
        self.parent = embed;
    }
}

fn intersect(x: i32, y: i32, w: i32, h: i32, mon: &MonitorInfo) -> i32 {
    (0.max((x + w).min(mon.x + mon.width) - x.max(mon.x)))
        * (0.max((y + h).min(mon.y + mon.height) - y.max(mon.y)))
}

/// X11 pixel value for a 24-bit depth: RGB in the top three bytes.
fn x11_pixel(c: Color) -> u32 {
    ((c.r() as u32) << 16) | ((c.g() as u32) << 8) | c.b() as u32
}
