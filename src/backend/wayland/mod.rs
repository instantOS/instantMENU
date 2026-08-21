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
    wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, EventQueue, QueueHandle};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_offer_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use xkbcommon::xkb;

use super::poll::{first_ready, poll_fds, poll_in, remaining_ms, PollOutcome};
use super::{Backend, BackendEvent, EventPoll, Modifiers, MonitorInfo};
use crate::geom::{Point, Rect, Size};
use crate::render::{Canvas, Color};
use selection::pump_offer;

pub struct WaylandBackend {
    connection: Connection,
    queue: EventQueue<EventState>,
    state: EventState,
}

/* ─────────────────────────── dispatch state ─────────────────────────── */

pub(super) struct OutputEntry {
    proxy: wl_output::WlOutput,
    pub(super) info: MonitorInfo,
}

/// The xkb modifier indices the core's [`Modifiers`] map to, resolved once
/// per keymap. A name that does not resolve (`MOD_INVALID`) simply never
/// matches — the modifier is treated as not held.
#[derive(Debug, Clone, Copy)]
pub(super) struct ModIndices {
    shift: u32,
    ctrl: u32,
    alt: u32,
    logo: u32,
}

impl ModIndices {
    pub(super) fn resolve(keymap: &xkb::Keymap) -> Self {
        fn index(keymap: &xkb::Keymap, name: &str) -> u32 {
            keymap.mod_get_index(name)
        }
        ModIndices {
            shift: index(keymap, "Shift"),
            ctrl: index(keymap, "Control"),
            alt: index(keymap, "Mod1"),
            logo: index(keymap, "Mod4"),
        }
    }

    /// Semantic modifiers for a raw xkb modifier mask.
    pub(super) fn modifiers(self, mask: u32) -> Modifiers {
        fn bit(mask: u32, index: u32) -> bool {
            index != xkb::MOD_INVALID && mask & (1 << index) != 0
        }
        Modifiers {
            shift: bit(mask, self.shift),
            ctrl: bit(mask, self.ctrl),
            alt: bit(mask, self.alt),
            logo: bit(mask, self.logo),
        }
    }
}

struct Xkb {
    /// Kept alive alongside the state (the C-level keymap backs it).
    #[allow(dead_code)]
    context: xkb::Context,
    #[allow(dead_code)]
    keymap: xkb::Keymap,
    state: xkb::State,
    indices: ModIndices,
    /// Last-seen modifier state. Wayland button events carry no modifier
    /// state of their own, so pointer events are stamped from this cache.
    mods: Modifiers,
}

impl Xkb {
    fn new(context: xkb::Context, keymap: xkb::Keymap) -> Self {
        let indices = ModIndices::resolve(&keymap);
        let state = xkb::State::new(&keymap);
        Xkb {
            context,
            keymap,
            state,
            indices,
            mods: Modifiers::default(),
        }
    }
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

/// A click-catcher: an invisible fullscreen layer surface whose input region
/// covers its whole output except the menu rectangle. A Wayland client cannot
/// see clicks on other clients' surfaces, so modal menus catch outside clicks
/// the way GTK context menus do — with a transparent shield under the menu
/// that consumes (and dismisses on) any press outside it.
pub(super) struct Shield {
    surface: wl_surface::WlSurface,
    layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    buffer: wl_buffer::WlBuffer,
    /// whether the first configure has been acked and the buffer attached
    /// (a layer surface must stay bufferless until then).
    configured: bool,
}

/// Userdata marker distinguishing shield layer surfaces from the menu's own
/// (same protocol object, different `Dispatch` impls).
pub(super) struct ShieldTag;

/// Classification of which surface has pointer focus. Set at Enter; read
/// on later events so no per-event shield scanning is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PointerFocus {
    /// On the menu window itself; coordinates are menu-local.
    Menu,
    /// On one of the click-catcher shields.
    Shield,
    /// Outside all our surfaces (e.g. after Leave).
    None,
}

/// A selection offer (clipboard or primary) being tracked.
struct OfferTracker {
    mimes: Vec<String>,
    read_fd: Option<RawFd>,
    pending: Vec<u8>,
}

impl OfferTracker {
    fn new() -> Self {
        OfferTracker {
            mimes: Vec::new(),
            read_fd: None,
            pending: Vec::new(),
        }
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
    pub(super) queue_handle: QueueHandle<Self>,
    pub(super) compositor: Option<wl_compositor::WlCompositor>,
    pub(super) shm: Option<wl_shm::WlShm>,
    pub(super) outputs: Vec<OutputEntry>,
    pub(super) monitors: Vec<MonitorInfo>,
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    /* last pointer position, surface-local (button events don't carry
     * coordinates on wayland; the position comes from motion/enter) */
    pointer_x: f64,
    pointer_y: f64,
    /// classification of the surface the pointer is currently on.
    pointer_focus: PointerFocus,
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
    /// border drawn around the menu content (`--border-width`); X11 gets this
    /// from the server, but Wayland surfaces have no border, so it is painted
    /// into the buffer here.
    border_width: i32,
    border_color: Color,
    /// set when the `wl_surface.frame` callback fires; `wait_frame` blocks on
    /// this so animations pace themselves to the compositor's vsync.
    frame_done: bool,
    /// the pending frame callback; kept alive here because dropping the
    /// proxy destroys the callback, so the `done` event would never arrive
    /// and `wait_frame` would block forever.
    frame_callback: Option<wl_callback::WlCallback>,
    /// canvas copy drawn once the first Configure arrives
    pending_frame: Option<(Vec<u8>, i32, i32)>,

    pool: Option<ShmPool>,

    /* outside-click shields, one per output */
    shields: Vec<Shield>,
    /// the pool backing the shield buffers (kept alive for the menu's
    /// lifetime; the memfd is sparse, see `create_shields`).
    shield_pool: Option<(wl_shm_pool::WlShmPool, RawFd)>,

    /* selection offers */
    clipboard_offer: Option<(wl_data_offer::WlDataOffer, OfferTracker)>,
    primary_offer: Option<(
        zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        OfferTracker,
    )>,

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
            pointer_focus: PointerFocus::None,
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
            border_width: 0,
            border_color: Color::rgb(0, 0, 0),
            frame_done: false,
            frame_callback: None,
            pending_frame: None,
            pool: None,
            shields: Vec::new(),
            shield_pool: None,
            clipboard_offer: None,
            primary_offer: None,
            events: VecDeque::new(),
            dead: false,
        }
    }

    /// Monitor containing the given global-coordinate point, or the first.
    /// Half-open bounds, so a point on the shared edge belongs to the
    /// left/top output only.
    fn output_for_point(&self, pos: Point) -> usize {
        for (i, out) in self.outputs.iter().enumerate() {
            let r = out.info.rect;
            if pos.x >= r.x && pos.x < r.right() && pos.y >= r.y && pos.y < r.bottom() {
                return i;
            }
        }
        0
    }

    fn draw(&mut self, bgra: &[u8], w: i32, h: i32) {
        if !self.configured || self.surface.is_none() {
            self.pending_frame = Some((bgra.to_vec(), w, h));
            return;
        }
        let content_w = w.max(1) as usize;
        let content_h = h.max(1) as usize;
        let border = self.border_width.max(0) as usize;
        let width = (content_w + 2 * border) as i32;
        let height = (content_h + 2 * border) as i32;
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
                pool.slots.push(ShmSlot {
                    buffer,
                    offset,
                    released: false,
                });
                pool.slots.len() - 1
            }
        };
        let offset = pool.slots[idx].offset;

        /* Canvas already matches little-endian wl_shm ARGB8888 (BGRA). When a
         * border width is set, fill the frame with the border color and copy
         * the content into the inset region — Wayland surfaces have no
         * server-side border like X11 windows do. */
        unsafe {
            let dst = pool.memory.add(offset);
            if border == 0 {
                std::ptr::copy_nonoverlapping(bgra.as_ptr(), dst, needed);
            } else {
                let border_bgra = [
                    self.border_color.b(),
                    self.border_color.g(),
                    self.border_color.r(),
                    self.border_color.a(),
                ];
                let pixels = (stride * height as usize) / 4;
                let mut cursor = dst;
                for _ in 0..pixels {
                    std::ptr::copy_nonoverlapping(border_bgra.as_ptr(), cursor, 4);
                    cursor = cursor.add(4);
                }
                let content_stride = content_w * 4;
                for row in 0..content_h {
                    let src = bgra.as_ptr().add(row * content_stride);
                    let dst_row = dst.add((row + border) * stride + border * 4);
                    std::ptr::copy_nonoverlapping(src, dst_row, content_stride);
                }
            }
        }
        let buffer = pool.slots[idx].buffer.clone();
        if let Some(surface) = &self.surface {
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, width, height);
            /* Register a frame callback so wait_frame() can block until this
             * frame is actually presented — this paces animations to vsync
             * instead of a fixed sleep. The proxy must be kept alive until
             * the done event arrives, so it is stored in the state. */
            self.frame_done = false;
            self.frame_callback = Some(surface.frame(&self.queue_handle, ()));
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
        let pool = unsafe {
            shm.create_pool(
                BorrowedFd::borrow_raw(fd),
                len as i32,
                &self.queue_handle,
                (),
            )
        };
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

    /// Create a memfd of `len` bytes and bind it as a wl_shm_pool. Returns
    /// `(pool, fd)`; the caller takes ownership of both and must release them
    /// with `destroy_memfd_pool` on teardown. On any failure the fd is closed
    /// and `None` is returned.
    fn create_memfd_pool(&self, len: usize) -> Option<(wl_shm_pool::WlShmPool, RawFd)> {
        let shm = self.shm.as_ref()?;
        let name = b"instantmenu\0";
        let fd = unsafe { libc::memfd_create(name.as_ptr().cast(), 0) };
        if fd < 0 {
            return None;
        }
        if unsafe { libc::ftruncate(fd, len as libc::off_t) } != 0 {
            unsafe { libc::close(fd) };
            return None;
        }
        let pool = unsafe {
            shm.create_pool(
                BorrowedFd::borrow_raw(fd),
                len as i32,
                &self.queue_handle,
                (),
            )
        };
        Some((pool, fd))
    }

    /// Tear down a pool created by `create_memfd_pool`.
    fn destroy_memfd_pool(pool: wl_shm_pool::WlShmPool, fd: RawFd) {
        pool.destroy();
        unsafe { libc::close(fd) };
    }

    /// Destroy the shields and their pool (recreated on window re-creation).
    fn destroy_shields(&mut self) {
        for shield in self.shields.drain(..) {
            shield.layer_surface.destroy();
            shield.surface.destroy();
            shield.buffer.destroy();
        }
        if let Some((pool, fd)) = self.shield_pool.take() {
            Self::destroy_memfd_pool(pool, fd);
        }
    }
}

impl Drop for EventState {
    fn drop(&mut self) {
        self.destroy_pool();
        self.destroy_shields();
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
        /* The first sync discovers globals; the second receives events from
         * the binds performed while dispatching the first sync. */
        for _ in 0..2 {
            if queue.roundtrip(&mut state).is_err() {
                return Err("roundtrip failed".to_string());
            }
        }
        if state.shm.is_none() {
            return Err("compositor has no wl_shm".to_string());
        }
        state.monitors = state.outputs.iter().map(|o| o.info.clone()).collect();
        Ok(WaylandBackend {
            connection,
            queue,
            state,
        })
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

    /// Read and dispatch any events already available on the Wayland socket
    /// without blocking. The animation loop blocks between frames without
    /// dispatching, so `present` pumps the socket itself to process
    /// `wl_buffer.release` and let the next frame reuse a buffer.
    fn pump_pending(&mut self) {
        if let Some(guard) = self.queue.prepare_read() {
            // Non-blocking: `read` returns `WouldBlock` when nothing is buffered.
            let _ = guard.read();
        }
        let _ = self.queue.dispatch_pending(&mut self.state);
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

    /// wlr-layer-shell surface anchored to the chosen output. `w`/`h` are the
    /// full surface size (menu content plus any `--border-width`).
    fn create_layer(
        &mut self,
        surface: &wl_surface::WlSurface,
        rect: Rect,
        grab: bool,
        outside_close: bool,
    ) -> Result<(), String> {
        let state = &mut self.state;
        let shell = state
            .layer_shell
            .as_ref()
            .ok_or("compositor has no wlr-layer-shell (use -wm for managed windows)")?;
        let output_index = state.output_for_point(rect.origin());
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
        let monitor = state
            .outputs
            .get(output_index)
            .map(|o| o.info.rect)
            .unwrap_or(rect);

        let mut anchor = zwlr_layer_surface_v1::Anchor::Left;
        let top;
        let bottom;
        if rect.y + rect.h / 2 >= monitor.y + monitor.h / 2 {
            anchor |= zwlr_layer_surface_v1::Anchor::Bottom;
            top = 0;
            bottom = (monitor.bottom() - rect.bottom()).max(0);
        } else {
            anchor |= zwlr_layer_surface_v1::Anchor::Top;
            top = (rect.y - monitor.y).max(0);
            bottom = 0;
        }
        let left = (rect.x - monitor.x).max(0);

        layer_surface.set_anchor(anchor);
        layer_surface.set_margin(top, 0, bottom, left);
        layer_surface.set_keyboard_interactivity(if grab {
            zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive
        } else {
            zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand
        });
        layer_surface.set_size(rect.w.max(1) as u32, rect.h.max(1) as u32);
        surface.commit();
        state.surface = Some(surface.clone());
        state.layer_surface = Some(layer_surface);
        if outside_close {
            self.create_shields(rect);
        }
        Ok(())
    }

    /// One click-catcher per output (see [`Shield`]). The shields sit in the
    /// same layer as the menu; their input region excludes the menu rectangle
    /// so the menu keeps all of its own pointer events.
    fn create_shields(&mut self, menu_rect: Rect) {
        self.state.destroy_shields();

        let Some(compositor) = self.state.compositor.clone() else {
            return;
        };
        let Some(shell) = self.state.layer_shell.clone() else {
            return;
        };
        let qh = self.state.queue_handle.clone();
        let outputs: Vec<(wl_output::WlOutput, Rect)> = self
            .state
            .outputs
            .iter()
            .filter(|o| o.info.rect.w > 0 && o.info.rect.h > 0)
            .map(|o| (o.proxy.clone(), o.info.rect))
            .collect();
        if outputs.is_empty() {
            return;
        }

        /* One sparse pool backs every shield buffer. A zero-filled ARGB
         * buffer is fully transparent, so the memfd pages are never
         * allocated (or even mapped client-side) — only the buffers are
         * bound. */
        let len: usize = outputs.iter().map(|(_, r)| (r.w * r.h * 4) as usize).sum();
        let Some((pool, fd)) = self.state.create_memfd_pool(len) else {
            return;
        };
        self.state.shield_pool = Some((pool.clone(), fd));

        let mut offset: i32 = 0;
        let mut shields = Vec::with_capacity(outputs.len());
        for (output, out_rect) in outputs {
            let shield_surface = compositor.create_surface(&qh, ());
            let layer_surface = shell.get_layer_surface(
                &shield_surface,
                Some(&output),
                zwlr_layer_shell_v1::Layer::Top,
                "instantmenu".to_string(),
                &qh,
                ShieldTag,
            );
            layer_surface.set_anchor(
                zwlr_layer_surface_v1::Anchor::Top
                    | zwlr_layer_surface_v1::Anchor::Bottom
                    | zwlr_layer_surface_v1::Anchor::Left
                    | zwlr_layer_surface_v1::Anchor::Right,
            );
            /* fullscreen but invisible to layout: no exclusive zone, and the
             * keyboard stays with the menu surface */
            layer_surface.set_exclusive_zone(-1);
            layer_surface
                .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
            layer_surface.set_size(out_rect.w as u32, out_rect.h as u32);

            /* input region: the whole output minus the menu's part of it.
             * The WlRegion role is copied by the compositor, so it can drop
             * at the end of this iteration. */
            let region = compositor.create_region(&qh, ());
            region.add(0, 0, out_rect.w, out_rect.h);
            let x0 = menu_rect.x.max(out_rect.x);
            let y0 = menu_rect.y.max(out_rect.y);
            let x1 = menu_rect.right().min(out_rect.right());
            let y1 = menu_rect.bottom().min(out_rect.bottom());
            if x1 > x0 && y1 > y0 {
                region.subtract(x0 - out_rect.x, y0 - out_rect.y, x1 - x0, y1 - y0);
            }
            shield_surface.set_input_region(Some(&region));
            drop(region);
            shield_surface.commit();

            let buffer = pool.create_buffer(
                offset,
                out_rect.w,
                out_rect.h,
                out_rect.w * 4,
                wl_shm::Format::Argb8888,
                &qh,
                (),
            );
            offset += out_rect.w * out_rect.h * 4;
            shields.push(Shield {
                surface: shield_surface,
                layer_surface,
                buffer,
                configured: false,
            });
        }
        self.state.shields = shields;
    }
}

/// The content rect expanded by the border on all sides (the full surface
/// footprint; X11 gets its border from the server, Wayland paints it).
fn bordered(rect: Rect, border: i32) -> Rect {
    Rect::new(rect.x, rect.y, rect.w + 2 * border, rect.h + 2 * border)
}

impl Backend for WaylandBackend {
    fn monitors(&self) -> &[MonitorInfo] {
        &self.state.monitors
    }

    fn root_size(&self) -> Size {
        /* the union of all outputs */
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        for out in &self.state.outputs {
            min_x = min_x.min(out.info.rect.x);
            min_y = min_y.min(out.info.rect.y);
            max_x = max_x.max(out.info.rect.right());
            max_y = max_y.max(out.info.rect.bottom());
        }
        if min_x > max_x {
            Size::new(0, 0)
        } else {
            Size::new(max_x - min_x, max_y - min_y)
        }
    }

    fn create_window(
        &mut self,
        rect: Rect,
        border_width: i32,
        managed: bool,
        grab: bool,
        outside_close: bool,
        class_hint: &str,
        _bg: Color,
        border_color: Color,
    ) -> Result<(), String> {
        let border_width = border_width.max(0);
        self.state.border_width = border_width;
        self.state.border_color = border_color;

        let surface = self
            .state
            .compositor
            .as_ref()
            .ok_or("compositor has no wl_compositor")?
            .create_surface(&self.state.queue_handle, ());

        if managed {
            self.create_managed(&surface, class_hint)
        } else {
            self.create_layer(&surface, bordered(rect, border_width), grab, outside_close)
        }
    }

    fn grab_focus(&mut self, title: &str) -> Result<(), String> {
        /* there is no focus to grab on Wayland — the layer surface's
         * keyboard interactivity already routes the keyboard here; managed
         * windows announce themselves by title like on X11 */
        if let Some(toplevel) = &self.state.xdg_toplevel {
            toplevel.set_title(title.to_string());
        }
        Ok(())
    }

    fn set_title(&mut self, title: &str) {
        if let Some(toplevel) = &self.state.xdg_toplevel {
            toplevel.set_title(title.to_string());
        }
    }

    fn present(&mut self, canvas: &Canvas) {
        self.pump_pending();
        self.state.draw(&canvas.data, canvas.width, canvas.height);
        let _ = self.connection.flush();
    }

    fn resize_window(&mut self, rect: Rect) {
        let full = bordered(rect, self.state.border_width);
        let w = full.w.max(1);
        let h = full.h.max(1);
        if let Some(layer_surface) = &self.state.layer_surface {
            /* the anchor/margins from create_layer stay valid: a top-anchored
             * surface grows downward and a bottom-anchored one grows upward,
             * which is exactly what reflowing the item grid wants */
            layer_surface.set_size(w as u32, h as u32);
            if let Some(surface) = &self.state.surface {
                surface.commit();
            }
        }
        /* managed (xdg-toplevel) windows: the WM owns the geometry */
        if !self.state.shields.is_empty() {
            /* recreate the click-catchers so their input region holes track
             * the new menu rectangle */
            self.create_shields(Rect::new(full.x, full.y, w, h));
        }
    }

    fn wait_frame(&mut self) {
        /* Block until the frame committed by the last present() is on screen.
         * The frame callback is dispatched by the queue, so release events for
         * previously-used buffers are processed here too. A failed dispatch
         * means the connection is dead — no further event (frame callback or
         * otherwise) can ever arrive, so waiting on would spin forever. */
        while !self.state.frame_done && !self.state.dead {
            if self.queue.blocking_dispatch(&mut self.state).is_err() {
                self.state.dead = true;
                break;
            }
        }
    }

    fn poll_event(&mut self, timeout: Option<std::time::Duration>, extra: &[RawFd]) -> EventPoll {
        let start = std::time::Instant::now();
        loop {
            if let Some(ev) = self.state.events.pop_front() {
                return EventPoll::Event(ev);
            }
            if self.state.dead {
                return EventPoll::Closed;
            }
            if let Some(ev) = self.pump_selection() {
                return EventPoll::Event(ev);
            }

            let timeout_ms = match remaining_ms(start, timeout) {
                Ok(ms) => ms,
                Err(()) => return EventPoll::Timeout,
            };

            /* Wait for the wayland socket or a pending selection pipe. A
             * blocking_dispatch() here would only wake on wayland events, so
             * the clipboard/primary transfer pipes would never be pumped. */
            let guard = self.queue.prepare_read();
            let Some(guard) = guard else {
                /* events are already pending in the queue: dispatch them
                 * before blocking on anything else */
                if self.queue.dispatch_pending(&mut self.state).is_err() {
                    return EventPoll::Closed;
                }
                let _ = self.connection.flush();
                continue;
            };
            let mut fds: Vec<libc::pollfd> = Vec::with_capacity(3 + extra.len());
            fds.push(poll_in(guard.connection_fd().as_raw_fd()));
            if let Some((_, tracker)) = &self.state.clipboard_offer {
                if let Some(fd) = tracker.read_fd {
                    fds.push(poll_in(fd));
                }
            }
            if let Some((_, tracker)) = &self.state.primary_offer {
                if let Some(fd) = tracker.read_fd {
                    fds.push(poll_in(fd));
                }
            }
            /* caller-owned fds (streaming stdin); watched last so the
             * internal indices stay stable */
            let extra_start = fds.len();
            fds.extend(extra.iter().copied().map(poll_in));

            match poll_fds(&mut fds, timeout_ms) {
                PollOutcome::Timeout => {
                    drop(guard);
                    return EventPoll::Timeout;
                }
                PollOutcome::Closed => return EventPoll::Closed,
                PollOutcome::Ready => {}
            }
            /* extras first: a blocked pipe producer is more time-critical
             * than already-queued compositor work */
            if let Some(i) = first_ready(&fds, extra_start) {
                drop(guard);
                return EventPoll::Readable(i - extra_start);
            }
            /* read + dispatch only when the wayland socket is ready; when
             * only a selection pipe fired, drop the guard (cancels the
             * prepared read) and let pump_selection() drain it */
            if first_ready(&fds, 0) == Some(0) && guard.read().is_err() {
                return EventPoll::Closed;
            }
            if self.queue.dispatch_pending(&mut self.state).is_err() {
                return EventPoll::Closed;
            }
            let _ = self.connection.flush();
        }
    }

    fn request_selection(&mut self, clipboard: bool) {
        let mime = if clipboard {
            self.state
                .clipboard_offer
                .as_ref()
                .and_then(|(_, t)| t.best_mime())
        } else {
            self.state
                .primary_offer
                .as_ref()
                .and_then(|(_, t)| t.best_mime())
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
            unsafe {
                libc::close(fds[0]);
                libc::close(fds[1])
            };
        }
        /* flush so the compositor actually receives the request and writes
         * to the pipe (next_event() polls it only after this) */
        let _ = self.connection.flush();
    }
}
