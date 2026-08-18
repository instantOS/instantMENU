//! Wayland backend — wlr-layer-shell (override-redirect equivalent) or
//! xdg-shell for `-wm` managed windows, wl_shm buffers, xkb keyboard state
//! from the compositor keymap.
//!
//! Protocol dispatch lives in `dispatch.rs` and selection/keymap plumbing in
//! `selection.rs`.

mod dispatch;
mod selection;

use std::collections::VecDeque;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::io::RawFd;

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, EventQueue, QueueHandle};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_offer_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};
use xkbcommon::xkb;

use selection::pump_offer;
use super::{Backend, BackendEvent, MonitorInfo};
use crate::render::{Canvas, Color};

/// xkb keycode = evdev keycode + 8; wayland delivers evdev keycodes.
const XKB_OFFSET: u32 = 8;

pub struct WaylandBackend {
    connection: Connection,
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
    memory: *mut u8,
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
    queue_handle: QueueHandle<Self>,

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
    pointer_x: f64,
    pointer_y: f64,
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
    fn new(queue_handle: QueueHandle<Self>) -> Self {
        EventState {
            queue_handle,
            compositor: None,
            shm: None,
            outputs: Vec::new(),
            monitors: Vec::new(),
            seat: None,
            keyboard: None,
            pointer: None,
            pointer_x: 0.0,
            pointer_y: 0.0,
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
                    &self.queue_handle,
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
            let dst = pool.memory.add(offset);
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
        let memory = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        if memory == libc::MAP_FAILED {
            unsafe { libc::close(fd) };
            return;
        }
        let pool = unsafe { shm.create_pool(BorrowedFd::borrow_raw(fd), len as i32, &self.queue_handle, ()) };
        self.pool = Some(ShmPool {
            pool,
            fd,
            memory: memory.cast(),
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
                libc::munmap(pool.memory.cast(), pool.len);
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

/* ──────────────────────── Backend impl ─────────────────────────────── */

impl WaylandBackend {
    pub fn new() -> Result<WaylandBackend, String> {
        let connection =
            Connection::connect_to_env().map_err(|e| format!("cannot connect: {e}"))?;
        let mut queue: EventQueue<EventState> = connection.new_event_queue();
        let queue_handle = queue.handle();
        let mut state = EventState::new(queue_handle.clone());
        let _ = connection.display().get_registry(&state.queue_handle, ());
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
        let _ = connection.flush();
        Ok(WaylandBackend { connection, queue, state })
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

    /// xdg-shell managed window.
    fn create_managed(
        &mut self,
        surface: &wl_surface::WlSurface,
        class_hint: &str,
    ) -> Result<(), String> {
        let state = &mut self.state;
        let wm_base = state.wm_base.as_ref().ok_or("no xdg_wm_base")?;
        let xdg_surface = wm_base.get_xdg_surface(surface, &state.queue_handle, ());
        let toplevel = xdg_surface.get_toplevel(&state.queue_handle, ());
        toplevel.set_title(class_hint.to_string());
        toplevel.set_app_id("instantmenu".to_string());
        surface.commit();
        state.surface = Some(surface.clone());
        state.xdg_surface = Some(xdg_surface);
        state.xdg_toplevel = Some(toplevel);
        Ok(())
    }

    /// wlr-layer-shell surface anchored to the chosen output.
    fn create_layer(
        &mut self,
        surface: &wl_surface::WlSurface,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        grab: bool,
    ) -> Result<(), String> {
        let state = &mut self.state;
        let shell = state.layer_shell.as_ref().ok_or(
            "compositor has no wlr-layer-shell (use -wm for managed windows)",
        )?;
        let output_index = state.output_for_point(x, y);
        let output = state.outputs.get(output_index).map(|o| o.proxy.clone());
        let layer_surface = shell.get_layer_surface(
            surface,
            output.as_ref(),
            zwlr_layer_shell_v1::Layer::Top,
            "instantmenu".to_string(),
            &state.queue_handle,
            (),
        );

        /* translate the X11-style absolute geometry into an anchor and
         * margins on the chosen output: anchor the edge the menu sits on,
         * offset from the left; set_size is then honored for both axes */
        let monitor = state.outputs.get(output_index).map(|o| o.info.clone()).unwrap_or(
            MonitorInfo { x: 0, y: 0, width: w, height: h, name: String::new() },
        );

        let mut anchor = zwlr_layer_surface_v1::Anchor::Left;
        let top;
        let bottom;
        if y + h / 2 >= monitor.y + monitor.height / 2 {
            anchor |= zwlr_layer_surface_v1::Anchor::Bottom;
            top = 0;
            bottom = (monitor.y + monitor.height - (y + h)).max(0);
        } else {
            anchor |= zwlr_layer_surface_v1::Anchor::Top;
            top = (y - monitor.y).max(0);
            bottom = 0;
        }
        let left = (x - monitor.x).max(0);

        layer_surface.set_anchor(anchor);
        layer_surface.set_margin(top, 0, bottom, left);
        layer_surface.set_keyboard_interactivity(if grab {
            zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive
        } else {
            zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand
        });
        layer_surface.set_size(w.max(1) as u32, h.max(1) as u32);
        surface.commit();
        state.surface = Some(surface.clone());
        state.layer_surface = Some(layer_surface);
        Ok(())
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
        self.state.width = w;
        self.state.height = h;

        let surface = self
            .state
            .compositor
            .as_ref()
            .ok_or("compositor has no wl_compositor")?
            .create_surface(&self.state.queue_handle, ());

        if managed {
            self.create_managed(&surface, class_hint)
        } else {
            self.create_layer(&surface, x, y, w, h, grab)
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
        let _ = self.connection.flush();
    }

    fn set_title(&mut self, title: &str) {
        if let Some(toplevel) = &self.state.xdg_toplevel {
            toplevel.set_title(title.to_string());
        }
        let _ = self.connection.flush();
    }

    fn present(&mut self, canvas: &Canvas) {
        self.state.draw(&canvas.data, canvas.width, canvas.height);
        let _ = self.connection.flush();
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
                let _ = self.connection.flush();
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
            let _ = self.connection.flush();
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
        let _ = self.connection.flush();
    }

    fn is_wayland(&self) -> bool {
        true
    }
}
