//! Wayland backend — wlr-layer-shell (override-redirect equivalent) or
//! xdg-shell for `-wm` managed windows, wl_shm buffers, xkb keyboard state
//! from the compositor keymap.

use std::collections::VecDeque;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::io::RawFd;

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_offer_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};
use xkbcommon::xkb::{self, Keycode, KeyDirection};

use super::{Backend, BackendEvent, MonitorInfo};
use crate::render::{Canvas, Color};

/// xkb keycode = evdev keycode + 8; wayland delivers evdev keycodes.
const XKB_OFFSET: u32 = 8;

pub struct WaylandBackend {
    conn: Connection,
    queue: EventQueue<EventState>,
    state: EventState,
}

/* ─────────────────────────── dispatch state ─────────────────────────── */

struct OutputEntry {
    proxy: wl_output::WlOutput,
    info: MonitorInfo,
}

struct Xkb {
    keymap: xkb::Keymap,
    state: xkb::State,
    mods: u32, // X11-style modifier mask
}

struct ShmSlot {
    buffer: wl_buffer::WlBuffer,
    offset: usize,
    released: bool,
}

struct ShmPool {
    pool: wl_shm_pool::WlShmPool,
    fd: RawFd,
    mem: *mut u8,
    len: usize,
    frame_size: usize,
    slots: Vec<ShmSlot>,
}

/// A selection offer (clipboard or primary) being tracked.
struct OfferTracker {
    mimes: Vec<String>,
    read_fd: Option<RawFd>,
    pending: Vec<u8>,
}

impl OfferTracker {
    fn new() -> Self {
        OfferTracker { mimes: Vec::new(), read_fd: None, pending: Vec::new() }
    }

    fn best_mime(&self) -> Option<String> {
        for wanted in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"] {
            if let Some(m) = self.mimes.iter().find(|m| m.eq_ignore_ascii_case(wanted)) {
                return Some(m.clone());
            }
        }
        None
    }
}

pub struct EventState {
    qh: QueueHandle<Self>,

    /* globals */
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    outputs: Vec<OutputEntry>,
    /// snapshot of output geometry, taken once after the initial roundtrips
    monitors: Vec<MonitorInfo>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /* last pointer position, surface-local (button events don't carry
     * coordinates on wayland; the position comes from motion/enter) */
    ptr_x: f64,
    ptr_y: f64,
    data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    data_device: Option<wl_data_device::WlDataDevice>,
    primary_manager:
        Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    primary_device: Option<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    wm_base: Option<xdg_wm_base::XdgWmBase>,

    /* keyboard */
    xkb: Option<Xkb>,

    /* window */
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    xdg_surface: Option<xdg_surface::XdgSurface>,
    xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    configured: bool,
    width: i32,
    height: i32,
    /// canvas copy drawn once the first Configure arrives
    pending_frame: Option<(Vec<u8>, i32, i32)>,

    pool: Option<ShmPool>,

    /* selection offers */
    clipboard_offer: Option<(wl_data_offer::WlDataOffer, OfferTracker)>,
    primary_offer:
        Option<(zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, OfferTracker)>,

    /* events out */
    events: VecDeque<BackendEvent>,
    dead: bool,
}

impl EventState {
    fn new(qh: QueueHandle<Self>) -> Self {
        EventState {
            qh,
            compositor: None,
            shm: None,
            outputs: Vec::new(),
            monitors: Vec::new(),
            seat: None,
            keyboard: None,
            pointer: None,
            ptr_x: 0.0,
            ptr_y: 0.0,
            data_device_manager: None,
            data_device: None,
            primary_manager: None,
            primary_device: None,
            layer_shell: None,
            wm_base: None,
            xkb: None,
            surface: None,
            layer_surface: None,
            xdg_surface: None,
            xdg_toplevel: None,
            configured: false,
            width: 0,
            height: 0,
            pending_frame: None,
            pool: None,
            clipboard_offer: None,
            primary_offer: None,
            events: VecDeque::new(),
            dead: false,
        }
    }

    /// Monitor containing the given global-coordinate point, or the first.
    fn output_for_point(&self, x: i32, y: i32) -> usize {
        for (i, out) in self.outputs.iter().enumerate() {
            if x >= out.info.x
                && x < out.info.x + out.info.width
                && y >= out.info.y
                && y < out.info.y + out.info.height
            {
                return i;
            }
        }
        0
    }

    fn draw(&mut self, rgba: &[u8], w: i32, h: i32) {
        if !self.configured || self.surface.is_none() {
            self.pending_frame = Some((rgba.to_vec(), w, h));
            return;
        }
        let width = w.max(1);
        let height = h.max(1);
        let stride = width as usize * 4;
        let needed = stride * height as usize;

        /* (re)create the pool on resize (the menu is fixed-size after setup,
         * so this normally happens once) */
        let needs_pool = match &self.pool {
            Some(p) => p.len < needed * 2 || p.frame_size != needed,
            None => true,
        };
        if needs_pool {
            self.destroy_pool();
            self.create_pool(needed * 2, needed);
        }
        let Some(pool) = &mut self.pool else { return };

        /* find a buffer the compositor has released, else append one */
        let idx = match pool.slots.iter().position(|s| s.released) {
            Some(i) => {
                pool.slots[i].released = false;
                i
            }
            None => {
                let offset = pool.slots.len() * needed;
                if offset + needed > pool.len {
                    return; // pool full and nothing released: drop the frame
                }
                let buffer = pool.pool.create_buffer(
                    offset as i32,
                    width,
                    height,
                    stride as i32,
                    wl_shm::Format::Argb8888,
                    &self.qh,
                    (),
                );
                pool.slots.push(ShmSlot { buffer, offset, released: false });
                pool.slots.len() - 1
            }
        };
        let offset = pool.slots[idx].offset;

        /* copy RGBA -> BGRA (wl_shm ARGB8888 is little-endian xBGRA in memory
         * on LE hosts) */
        unsafe {
            let dst = pool.mem.add(offset);
            let n = width as usize * height as usize;
            for i in 0..n {
                let s = &rgba[i * 4..i * 4 + 4];
                *dst.add(i * 4) = s[2];
                *dst.add(i * 4 + 1) = s[1];
                *dst.add(i * 4 + 2) = s[0];
                *dst.add(i * 4 + 3) = s[3];
            }
        }
        let buffer = pool.slots[idx].buffer.clone();
        if let Some(surface) = &self.surface {
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, width, height);
            surface.commit();
        }
    }

    fn create_pool(&mut self, len: usize, frame_size: usize) {
        let Some(shm) = &self.shm else { return };
        let name = b"instantmenu\0";
        let fd = unsafe { libc::memfd_create(name.as_ptr().cast(), 0) };
        if fd < 0 {
            return;
        }
        if unsafe { libc::ftruncate(fd, len as libc::off_t) } != 0 {
            unsafe { libc::close(fd) };
            return;
        }
        let mem = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if mem == libc::MAP_FAILED {
            unsafe { libc::close(fd) };
            return;
        }
        let pool = unsafe { shm.create_pool(BorrowedFd::borrow_raw(fd), len as i32, &self.qh, ()) };
        self.pool = Some(ShmPool {
            pool,
            fd,
            mem: mem.cast(),
            len,
            frame_size,
            slots: Vec::new(),
        });
    }

    fn destroy_pool(&mut self) {
        if let Some(mut pool) = self.pool.take() {
            for slot in &pool.slots {
                slot.buffer.destroy();
            }
            pool.slots.clear();
            pool.pool.destroy();
            unsafe {
                libc::munmap(pool.mem.cast(), pool.len);
                libc::close(pool.fd);
            }
        }
    }
}

impl Drop for EventState {
    fn drop(&mut self) {
        self.destroy_pool();
    }
}

/* ──────────────────────── Dispatch impls ───────────────────────────── */

macro_rules! noop_dispatch {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Dispatch<$ty, ()> for EventState {
                fn event(
                    _state: &mut Self,
                    _proxy: &$ty,
                    _event: <$ty as wayland_client::Proxy>::Event,
                    _data: &(),
                    _conn: &Connection,
                    _qhandle: &QueueHandle<Self>,
                ) {
                }
            }
        )*
    };
}

noop_dispatch!(
    wl_compositor::WlCompositor,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_data_device_manager::WlDataDeviceManager,
    zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    wl_surface::WlSurface,
);

impl Dispatch<wl_registry::WlRegistry, ()> for EventState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => {
                let v = version.min(7);
                match interface.as_str() {
                    "wl_compositor" => {
                        state.compositor = Some(registry.bind(name, v.min(6), qh, ()))
                    }
                    "wl_shm" => state.shm = Some(registry.bind(name, 1.min(v), qh, ())),
                    "wl_output" => {
                        let proxy: wl_output::WlOutput = registry.bind(name, v.min(4), qh, ());
                        state.outputs.push(OutputEntry {
                            proxy,
                            info: MonitorInfo {
                                x: 0,
                                y: 0,
                                width: 0,
                                height: 0,
                                name: String::new(),
                            },
                        });
                    }
                    "wl_seat" => {
                        let seat: wl_seat::WlSeat = registry.bind(name, v.min(5), qh, ());
                        state.seat = Some(seat);
                    }
                    "wl_data_device_manager" => {
                        state.data_device_manager =
                            Some(registry.bind(name, v.min(3), qh, ()))
                    }
                    "zwp_primary_selection_device_manager_v1" => {
                        state.primary_manager =
                            Some(registry.bind(name, 1.min(v), qh, ()))
                    }
                    "zwlr_layer_shell_v1" => {
                        state.layer_shell = Some(registry.bind(name, v.min(4), qh, ()))
                    }
                    "xdg_wm_base" => {
                        state.wm_base = Some(registry.bind(name, v.min(6), qh, ()))
                    }
                    _ => {}
                }
            }
            wl_registry::Event::GlobalRemove { name: _ } => {}
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for EventState {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(i) = state.outputs.iter().position(|o| o.proxy == *proxy) else { return };
        match event {
            wl_output::Event::Geometry { x, y, .. } => {
                state.outputs[i].info.x = x;
                state.outputs[i].info.y = y;
            }
            wl_output::Event::Mode { flags, width, height, .. } => {
                if let WEnum::Value(f) = flags {
                    if f.contains(wl_output::Mode::Current) {
                        state.outputs[i].info.width = width;
                        state.outputs[i].info.height = height;
                    }
                }
            }
            wl_output::Event::Name { name } => {
                state.outputs[i].info.name = name;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for EventState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            let Ok(caps) = capabilities.into_result() else { return };
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
            /* bind the selection devices once we have a seat */
            if state.data_device.is_none() {
                if let Some(mgr) = &state.data_device_manager {
                    state.data_device = Some(mgr.get_data_device(seat, qh, ()));
                }
            }
            if state.primary_device.is_none() {
                if let Some(mgr) = &state.primary_manager {
                    state.primary_device = Some(mgr.get_device(seat, qh, ()));
                }
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for EventState {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if let WEnum::Value(f) = format {
                    if f == wl_keyboard::KeymapFormat::XkbV1 {
                        if let Some(xkb) = load_keymap(fd.as_raw_fd(), size as usize) {
                            state.xkb = Some(xkb);
                        }
                    }
                }
                /* fd is an OwnedFd and closes on drop */
            }
            wl_keyboard::Event::Key { key, state: keystate, .. } => {
                let Some(x) = state.xkb.as_mut() else { return };
                let code = Keycode::new(key + XKB_OFFSET);
                let pressed =
                    matches!(keystate, WEnum::Value(wl_keyboard::KeyState::Pressed));
                x.state.update_key(
                    code,
                    if pressed { KeyDirection::Down } else { KeyDirection::Up },
                );
                let sym = x.state.key_get_one_sym(code).raw();
                let mods = x.mods;
                if pressed {
                    let text = x.state.key_get_utf8(code);
                    state.events.push_back(BackendEvent::KeyPress { sym, state: mods, text });
                } else {
                    state.events.push_back(BackendEvent::KeyRelease { sym, state: mods });
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                ..
            } => {
                let Some(x) = state.xkb.as_mut() else { return };
                x.state.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, 0);
                let mask = mods_depressed | mods_latched | mods_locked;
                x.mods = x11_mask(&x.keymap, mask);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for EventState {
    fn event(
        state: &mut Self,
        _proxy: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let mods = state.xkb.as_ref().map(|x| x.mods).unwrap_or(0);
        match event {
            wl_pointer::Event::Enter { surface_x, surface_y, .. } => {
                state.ptr_x = surface_x;
                state.ptr_y = surface_y;
            }
            wl_pointer::Event::Motion { time, surface_x, surface_y } => {
                state.ptr_x = surface_x;
                state.ptr_y = surface_y;
                state.events.push_back(BackendEvent::Motion {
                    time,
                    x: surface_x as i32,
                    y: surface_y as i32,
                });
            }
            wl_pointer::Event::Button { button, state: bstate, .. } => {
                if let WEnum::Value(wl_pointer::ButtonState::Pressed) = bstate {
                    let xbutton = match button {
                        0x110 => 1, // left
                        0x111 => 2, // middle
                        0x112 => 3, // right
                        _ => return,
                    };
                    state.events.push_back(BackendEvent::ButtonPress {
                        button: xbutton,
                        state: mods,
                        x: state.ptr_x as i32,
                        y: state.ptr_y as i32,
                    });
                }
            }
            /* wheel: map to the X11 buttons 4/5 the menu core understands */
            wl_pointer::Event::Axis { axis, value, .. } => {
                if let WEnum::Value(a) = axis {
                    if a == wl_pointer::Axis::VerticalScroll {
                        state.events.push_back(BackendEvent::ButtonPress {
                            button: if value > 0.0 { 5 } else { 4 },
                            state: mods,
                            x: -1,
                            y: -1,
                        });
                    }
                }
            }
            _ => {
                let _ = mods;
            }
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for EventState {
    fn event(
        state: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                surface.ack_configure(serial);
                if width > 0 {
                    state.width = width as i32;
                }
                if height > 0 {
                    state.height = height as i32;
                }
                let was_configured = state.configured;
                state.configured = true;
                if !was_configured {
                    if let Some((frame, w, h)) = state.pending_frame.take() {
                        state.draw(&frame, w, h);
                    }
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.dead = true;
                state.events.push_back(BackendEvent::Destroyed);
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for EventState {
    fn event(
        _state: &mut Self,
        base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for EventState {
    fn event(
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            let was_configured = state.configured;
            state.configured = true;
            if !was_configured {
                if let Some((frame, w, h)) = state.pending_frame.take() {
                    state.draw(&frame, w, h);
                }
            }
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for EventState {
    fn event(
        state: &mut Self,
        _proxy: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                if width > 0 {
                    state.width = width as i32;
                }
                if height > 0 {
                    state.height = height as i32;
                }
            }
            xdg_toplevel::Event::Close => {
                state.dead = true;
                state.events.push_back(BackendEvent::Destroyed);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for EventState {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            if let Some(pool) = &mut state.pool {
                for slot in &mut pool.slots {
                    if slot.buffer == *buffer {
                        slot.released = true;
                        break;
                    }
                }
            }
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for EventState {
    fn event(
        state: &mut Self,
        _proxy: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.clipboard_offer = Some((id, OfferTracker::new()));
            }
            wl_data_device::Event::Selection { id } => {
                if id.is_none() {
                    state.clipboard_offer = None;
                }
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(EventState, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for EventState {
    fn event(
        state: &mut Self,
        _proxy: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            if let Some((_, tracker)) = &mut state.clipboard_offer {
                tracker.mimes.push(mime_type);
            }
        }
    }
}

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()>
    for EventState
{
    fn event(
        state: &mut Self,
        _proxy: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwp_primary_selection_device_v1::Event::DataOffer { offer } => {
                state.primary_offer = Some((offer, OfferTracker::new()));
            }
            zwp_primary_selection_device_v1::Event::Selection { id } => {
                if id.is_none() {
                    state.primary_offer = None;
                }
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(
        EventState,
        zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        [
            zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE => (
                zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
                ()
            ),
        ]
    );
}

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()>
    for EventState
{
    fn event(
        state: &mut Self,
        _proxy: &zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event {
            if let Some((_, tracker)) = &mut state.primary_offer {
                tracker.mimes.push(mime_type);
            }
        }
    }
}

/* ─────────────────────── selection pumping ─────────────────────────── */

/// Drain one offer's read pipe; Some(text) when the transfer finished.
fn pump_offer<T>(slot: &mut Option<(T, OfferTracker)>) -> Option<String> {
    let (_, tracker) = slot.as_mut()?;
    let fd = tracker.read_fd?;
    let mut buf = [0u8; 4096];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            tracker.pending.extend_from_slice(&buf[..n as usize]);
        } else if n == 0 {
            unsafe { libc::close(fd) };
            tracker.read_fd = None;
            return Some(String::from_utf8_lossy(&tracker.pending).into_owned());
        } else {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::WouldBlock {
                unsafe { libc::close(fd) };
                tracker.read_fd = None;
                return Some(String::from_utf8_lossy(&tracker.pending).into_owned());
            }
            return None; // more to come later
        }
    }
}

/// Load the xkb keymap from the compositor's keymap fd.
fn load_keymap(fd: RawFd, size: usize) -> Option<Xkb> {
    let map = unsafe {
        libc::mmap(std::ptr::null_mut(), size, libc::PROT_READ, libc::MAP_PRIVATE, fd, 0)
    };
    if map == libc::MAP_FAILED {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(map.cast::<u8>(), size) };
    let text = String::from_utf8_lossy(bytes).into_owned();
    unsafe { libc::munmap(map, size) };

    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_string(
        &ctx,
        text,
        xkb::KEYMAP_FORMAT_TEXT_V1,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )?;
    let state = xkb::State::new(&keymap);
    Some(Xkb { keymap, state, mods: 0 })
}

/// xkb mod mask -> X11 modifier mask (matched by mod name).
fn x11_mask(keymap: &xkb::Keymap, mask: u32) -> u32 {
    let mut out = 0u32;
    for (name, bit) in [
        ("Shift", super::SHIFT_MASK),
        ("Lock", super::LOCK_MASK),
        ("Control", super::CONTROL_MASK),
        ("Mod1", super::MOD1_MASK),
        ("Mod2", super::MOD2_MASK),
        ("Mod3", super::MOD3_MASK),
        ("Mod4", super::MOD4_MASK),
        ("Mod5", super::MOD5_MASK),
    ] {
        let idx = keymap.mod_get_index(name);
        if idx != xkb::MOD_INVALID && mask & (1u32 << idx) != 0 {
            out |= bit;
        }
    }
    out
}

/* ──────────────────────── Backend impl ─────────────────────────────── */

impl WaylandBackend {
    pub fn new() -> Result<WaylandBackend, String> {
        let conn =
            Connection::connect_to_env().map_err(|e| format!("cannot connect: {e}"))?;
        let mut queue: EventQueue<EventState> = conn.new_event_queue();
        let qh = queue.handle();
        let mut state = EventState::new(qh.clone());
        let _ = conn.display().get_registry(&state.qh, ());
        /* wait for the initial globals + output modes + seat devices */
        for _ in 0..4 {
            if queue.roundtrip(&mut state).is_err() {
                return Err("roundtrip failed".to_string());
            }
        }
        if state.shm.is_none() {
            return Err("compositor has no wl_shm".to_string());
        }
        state.monitors = state.outputs.iter().map(|o| o.info.clone()).collect();
        let _ = conn.flush();
        Ok(WaylandBackend { conn, queue, state })
    }

    fn pump_selection(&mut self) -> Option<BackendEvent> {
        if let Some(text) = pump_offer(&mut self.state.clipboard_offer) {
            return Some(BackendEvent::SelectionNotify { text });
        }
        if let Some(text) = pump_offer(&mut self.state.primary_offer) {
            return Some(BackendEvent::SelectionNotify { text });
        }
        None
    }
}

impl Backend for WaylandBackend {
    fn monitors(&self) -> &[MonitorInfo] {
        &self.state.monitors
    }

    fn root_size(&self) -> (i32, i32) {
        /* the union of all outputs */
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for out in &self.state.outputs {
            min_x = min_x.min(out.info.x);
            min_y = min_y.min(out.info.y);
            max_x = max_x.max(out.info.x + out.info.width);
            max_y = max_y.max(out.info.y + out.info.height);
        }
        if min_x > max_x {
            (0, 0)
        } else {
            (max_x - min_x, max_y - min_y)
        }
    }

    fn pointer_position(&self) -> Option<(i32, i32)> {
        /* Wayland clients cannot query the global pointer */
        None
    }

    fn focused_monitor(&self) -> Option<usize> {
        None
    }

    fn embed_parent_size(&self) -> Option<(i32, i32)> {
        None
    }

    fn create_window(
        &mut self,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        _border_width: i32,
        managed: bool,
        grab: bool,
        class_hint: &str,
        _bg: Color,
        _border_color: Color,
    ) -> Result<(), String> {
        let state = &mut self.state;
        state.width = w;
        state.height = h;

        let surface = state
            .compositor
            .as_ref()
            .ok_or("compositor has no wl_compositor")?
            .create_surface(&state.qh, ());

        if managed {
            let wm_base = state.wm_base.as_ref().ok_or("no xdg_wm_base")?;
            let xdg_surface = wm_base.get_xdg_surface(&surface, &state.qh, ());
            let toplevel = xdg_surface.get_toplevel(&state.qh, ());
            toplevel.set_title(class_hint.to_string());
            toplevel.set_app_id("instantmenu".to_string());
            surface.commit();
            state.surface = Some(surface);
            state.xdg_surface = Some(xdg_surface);
            state.xdg_toplevel = Some(toplevel);
            Ok(())
        } else {
            let shell = state.layer_shell.as_ref().ok_or(
                "compositor has no wlr-layer-shell (use -wm for managed windows)",
            )?;
            let out_idx = state.output_for_point(x, y);
            let output = state.outputs.get(out_idx).map(|o| o.proxy.clone());
            let layer_surface = shell.get_layer_surface(
                &surface,
                output.as_ref(),
                zwlr_layer_shell_v1::Layer::Top,
                "instantmenu".to_string(),
                &state.qh,
                (),
            );

            /* translate the X11-style absolute geometry into an anchor and
             * margins on the chosen output: anchor the edge the menu sits on,
             * offset from the left; set_size is then honored for both axes */
            let mon = state.outputs.get(out_idx).map(|o| o.info.clone()).unwrap_or(
                MonitorInfo { x: 0, y: 0, width: w, height: h, name: String::new() },
            );

            let mut anchor = zwlr_layer_surface_v1::Anchor::Left;
            let top;
            let bottom;
            if y + h / 2 >= mon.y + mon.height / 2 {
                anchor |= zwlr_layer_surface_v1::Anchor::Bottom;
                top = 0;
                bottom = (mon.y + mon.height - (y + h)).max(0);
            } else {
                anchor |= zwlr_layer_surface_v1::Anchor::Top;
                top = (y - mon.y).max(0);
                bottom = 0;
            }
            let left = (x - mon.x).max(0);

            layer_surface.set_anchor(anchor);
            layer_surface.set_margin(top, 0, bottom, left);
            layer_surface.set_keyboard_interactivity(if grab {
                zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive
            } else {
                zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand
            });
            layer_surface.set_size(w.max(1) as u32, h.max(1) as u32);
            surface.commit();
            state.surface = Some(surface);
            state.layer_surface = Some(layer_surface);
            Ok(())
        }
    }

    fn map_window(&mut self) {
        /* mapping is implicit in the first commit + configure */
    }

    fn embed_setup(&mut self, _x: i32, _y: i32) {
        /* embedding is X11-only */
    }

    fn grab_keyboard(&mut self) {
        /* keyboard grabbing is expressed as layer-surface interactivity,
         * requested in create_window */
    }

    fn grab_focus(&mut self, title: &str) {
        if let Some(toplevel) = &self.state.xdg_toplevel {
            toplevel.set_title(title.to_string());
        }
        let _ = self.conn.flush();
    }

    fn set_title(&mut self, title: &str) {
        if let Some(toplevel) = &self.state.xdg_toplevel {
            toplevel.set_title(title.to_string());
        }
        let _ = self.conn.flush();
    }

    fn present(&mut self, canvas: &Canvas) {
        self.state.draw(&canvas.data, canvas.width, canvas.height);
        let _ = self.conn.flush();
    }

    fn raise(&mut self) {
        /* layer surfaces keep their layer stacking; nothing to do */
    }

    fn next_event(&mut self) -> Option<BackendEvent> {
        loop {
            if let Some(ev) = self.state.events.pop_front() {
                return Some(ev);
            }
            if self.state.dead {
                return None;
            }
            if let Some(ev) = self.pump_selection() {
                return Some(ev);
            }

            /* Wait for the wayland socket or a pending selection pipe. A
             * blocking_dispatch() here would only wake on wayland events, so
             * the clipboard/primary transfer pipes would never be pumped. */
            let guard = self.queue.prepare_read();
            let mut fds: Vec<libc::pollfd> = Vec::with_capacity(3);
            if let Some(g) = &guard {
                fds.push(libc::pollfd {
                    fd: g.connection_fd().as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                });
            } else {
                /* events are already pending in the queue: dispatch them
                 * before blocking on anything else */
                if self.queue.dispatch_pending(&mut self.state).is_err() {
                    return None;
                }
                let _ = self.conn.flush();
                continue;
            }
            if let Some((_, tracker)) = &self.state.clipboard_offer {
                if let Some(fd) = tracker.read_fd {
                    fds.push(libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
                }
            }
            if let Some((_, tracker)) = &self.state.primary_offer {
                if let Some(fd) = tracker.read_fd {
                    fds.push(libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
                }
            }
            loop {
                let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
                if n >= 0 {
                    break;
                }
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::Interrupted {
                    return None;
                }
            }
            /* read + dispatch only when the wayland socket is ready; when
             * only a selection pipe fired, drop the guard (cancels the
             * prepared read) and let pump_selection() drain it */
            if fds[0].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
                if let Some(g) = guard {
                    if g.read().is_err() {
                        return None;
                    }
                }
            }
            if self.queue.dispatch_pending(&mut self.state).is_err() {
                return None;
            }
            let _ = self.conn.flush();
        }
    }

    fn request_selection(&mut self, clipboard: bool) {
        let mime = if clipboard {
            self.state.clipboard_offer.as_ref().and_then(|(_, t)| t.best_mime())
        } else {
            self.state.primary_offer.as_ref().and_then(|(_, t)| t.best_mime())
        };
        let Some(mime) = mime else { return };

        let mut fds = [0i32; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return;
        }
        /* make the read end non-blocking: pump_offer() must never block the
         * event loop while the compositor streams the selection in */
        unsafe { libc::fcntl(fds[0], libc::F_SETFL, libc::O_NONBLOCK) };

        /* register the pipe as the transfer target of the offer */
        let sent = if clipboard {
            match &mut self.state.clipboard_offer {
                Some((offer, tracker)) if tracker.read_fd.is_none() => {
                    unsafe { offer.receive(mime.clone(), BorrowedFd::borrow_raw(fds[1])) };
                    unsafe { libc::close(fds[1]) };
                    tracker.read_fd = Some(fds[0]);
                    true
                }
                _ => false,
            }
        } else {
            match &mut self.state.primary_offer {
                Some((offer, tracker)) if tracker.read_fd.is_none() => {
                    unsafe { offer.receive(mime.clone(), BorrowedFd::borrow_raw(fds[1])) };
                    unsafe { libc::close(fds[1]) };
                    tracker.read_fd = Some(fds[0]);
                    true
                }
                _ => false,
            }
        };
        if !sent {
            /* no offer, or a transfer is already in flight */
            unsafe { libc::close(fds[0]); libc::close(fds[1]) };
        }
        /* flush so the compositor actually receives the request and writes
         * to the pipe (next_event() polls it only after this) */
        let _ = self.conn.flush();
    }

    fn is_wayland(&self) -> bool {
        true
    }
}
