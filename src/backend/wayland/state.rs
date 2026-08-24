//! The Wayland dispatch state: every global, surface and buffer the backend
//! binds, plus the frame path (`draw`) that turns a canvas into a committed
//! wl_surface state.

use std::collections::VecDeque;

use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
};
use wayland_client::QueueHandle;
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_offer_v1,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::keyboard::{KeyboardRepeat, Xkb};
use super::probe::Probe;
use super::selection::OfferTracker;
use super::shield::Shield;
use super::shm::{blit_frame, MemfdPool, ShmPool};
use crate::backend::{BackendEvent, MonitorInfo};
use crate::geom::Point;
use crate::render::Color;

pub(super) struct OutputEntry {
    pub(super) proxy: wl_output::WlOutput,
    pub(super) info: MonitorInfo,
}

/// The focus-relevant snapshot of a foreign toplevel (`activated` marks the
/// window with keyboard focus; its entered outputs say which monitor that
/// window is on).
pub(super) struct ToplevelInfo {
    pub(super) proxy: zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    pub(super) activated: bool,
    pub(super) outputs: Vec<wl_output::WlOutput>,
}

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

pub struct EventState {
    pub(super) queue_handle: QueueHandle<Self>,
    pub(super) compositor: Option<wl_compositor::WlCompositor>,
    pub(super) shm: Option<wl_shm::WlShm>,
    pub(super) outputs: Vec<OutputEntry>,
    pub(super) monitors: Vec<MonitorInfo>,
    pub(super) seat: Option<wl_seat::WlSeat>,
    pub(super) keyboard: Option<wl_keyboard::WlKeyboard>,
    pub(super) pointer: Option<wl_pointer::WlPointer>,
    /* last pointer position, surface-local (button events don't carry
     * coordinates on wayland; the position comes from motion/enter) */
    pub(super) pointer_x: f64,
    pub(super) pointer_y: f64,
    /// classification of the surface the pointer is currently on.
    pub(super) pointer_focus: PointerFocus,
    pub(super) data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    pub(super) data_device: Option<wl_data_device::WlDataDevice>,
    pub(super) primary_manager:
        Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    pub(super) primary_device: Option<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1>,
    pub(super) layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    pub(super) wm_base: Option<xdg_wm_base::XdgWmBase>,
    /// logical monitor geometry (wl_output mode sizes are physical pixels,
    /// which misplaces everything on scaled outputs)
    pub(super) xdg_output_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    pub(super) xdg_outputs: Vec<zxdg_output_v1::ZxdgOutputV1>,
    /// Optional zero-copy scaling for the temporary 1x1 pointer-probe buffers.
    pub(super) viewporter: Option<wp_viewporter::WpViewporter>,
    /// focused-window tracking for `focused_monitor`
    pub(super) track_focused_monitor: bool,
    pub(super) foreign_manager:
        Option<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1>,
    pub(super) toplevels: Vec<ToplevelInfo>,
    /// Immutable startup snapshot. More than one entry is deliberately
    /// treated as ambiguous by `focused_monitor`.
    pub(super) focused_outputs: Vec<wl_output::WlOutput>,

    /* pointer probe: temporary input surfaces mapped only during a query */
    pub(super) probes: Vec<Probe>,
    pub(super) probe_pool: Option<MemfdPool>,
    /// global cursor position once a probe has seen the pointer
    pub(super) probe_answer: Option<Point>,

    /* keyboard */
    pub(super) xkb: Option<Xkb>,
    pub(super) key_repeat: KeyboardRepeat,
    /// True only after wl_keyboard.enter names the menu surface. Mapping a
    /// layer buffer is not by itself proof that the seat has switched focus.
    pub(super) keyboard_focused: bool,

    /* window */
    pub(super) surface: Option<wl_surface::WlSurface>,
    pub(super) layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    pub(super) xdg_surface: Option<xdg_surface::XdgSurface>,
    pub(super) xdg_toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub(super) configured: bool,
    /// A transparent 1x1 buffer maps the layer surface early so exclusive
    /// keyboard interactivity is effective while config and fonts load.
    /// The surface itself is retained and becomes the final menu surface.
    pub(super) bootstrap: bool,
    pub(super) bootstrap_mapped: bool,
    pub(super) bootstrap_pool: Option<MemfdPool>,
    pub(super) bootstrap_buffer: Option<wl_buffer::WlBuffer>,
    /// border drawn around the menu content (`--border-width`); X11 gets this
    /// from the server, but Wayland surfaces have no border, so it is painted
    /// into the buffer here.
    pub(super) border_width: i32,
    pub(super) border_color: Color,
    /// set when the `wl_surface.frame` callback fires; `wait_frame` blocks on
    /// this so animations pace themselves to the compositor's vsync.
    pub(super) frame_done: bool,
    /// the pending frame callback; kept alive here because dropping the
    /// proxy destroys the callback, so the `done` event would never arrive
    /// and `wait_frame` would block forever.
    pub(super) frame_callback: Option<wl_callback::WlCallback>,
    /// Latest canvas awaiting a drawable buffer. Before the first Configure
    /// this is the initial frame; later it also coalesces redraws while both
    /// SHM slots are owned by the compositor. New frames replace old ones.
    pub(super) pending_frame: Option<(Vec<u8>, i32, i32)>,

    pub(super) pool: Option<ShmPool>,

    /* outside-click shields, one per output */
    pub(super) shields: Vec<Shield>,
    /// the pool backing the shield buffers (kept alive for the menu's
    /// lifetime; the memfd is sparse, see `create_shields`).
    pub(super) shield_pool: Option<MemfdPool>,

    /* selection offers */
    pub(super) clipboard_offer: Option<(wl_data_offer::WlDataOffer, OfferTracker)>,
    pub(super) primary_offer: Option<(
        zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        OfferTracker,
    )>,

    /* events out */
    pub(super) events: VecDeque<BackendEvent>,
    pub(super) dead: bool,
}

impl EventState {
    pub(super) fn new(queue_handle: QueueHandle<Self>, track_focused_monitor: bool) -> Self {
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
            xdg_output_manager: None,
            xdg_outputs: Vec::new(),
            viewporter: None,
            track_focused_monitor,
            foreign_manager: None,
            toplevels: Vec::new(),
            focused_outputs: Vec::new(),
            probes: Vec::new(),
            probe_pool: None,
            probe_answer: None,
            xkb: None,
            key_repeat: KeyboardRepeat::new(),
            keyboard_focused: false,
            surface: None,
            layer_surface: None,
            xdg_surface: None,
            xdg_toplevel: None,
            configured: false,
            bootstrap: false,
            bootstrap_mapped: false,
            bootstrap_pool: None,
            bootstrap_buffer: None,
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
    pub(super) fn output_for_point(&self, pos: Point) -> usize {
        for (i, out) in self.outputs.iter().enumerate() {
            let r = out.info.rect;
            if pos.x >= r.x && pos.x < r.right() && pos.y >= r.y && pos.y < r.bottom() {
                return i;
            }
        }
        0
    }

    /// Composite the canvas into a free SHM slot and commit it. Before the
    /// first Configure there is nothing to attach to — the frame is parked
    /// in `pending_frame` until then; when both slots are owned by the
    /// compositor the newest canvas is parked instead (intermediate frames
    /// coalesce).
    pub(super) fn draw(&mut self, bgra: &[u8], w: i32, h: i32) {
        if !self.configured || self.surface.is_none() {
            self.pending_frame = Some((bgra.to_vec(), w, h));
            return;
        }
        /* This call is newer than any previously queued canvas. If it cannot
         * acquire a slot below it will queue itself again. */
        self.pending_frame = None;

        let content_w = w.max(1) as usize;
        let content_h = h.max(1) as usize;
        let border = self.border_width.max(0) as usize;
        let width = (content_w + 2 * border) as i32;
        let height = (content_h + 2 * border) as i32;
        let stride = width as usize * 4;
        let size = stride * height as usize;

        /* (re)create the pool on resize */
        let stale = match &self.pool {
            Some(pool) => !pool.reusable_for(size, size),
            None => true,
        };
        if stale {
            let Some(shm) = self.shm.clone() else { return };
            self.pool = ShmPool::create(&shm, size * 2, size, &self.queue_handle);
        }
        let Some(pool) = &mut self.pool else { return };
        let Some(idx) = pool.acquire(width, height, stride as i32, size, &self.queue_handle) else {
            /* Never leave stale pixels indefinitely. The buffer release
             * handler submits the newest queued frame; intermediate frames
             * are intentionally coalesced. */
            self.pending_frame = Some((bgra.to_vec(), w, h));
            return;
        };

        blit_frame(
            pool.frame_memory(idx, size),
            bgra,
            content_w,
            content_h,
            border,
            self.border_color,
        );

        let buffer = pool.slot_buffer(idx).clone();
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
        /* The final frame has replaced the transparent bootstrap buffer.
         * Destroying the client-side wl_buffer now is protocol-safe: the
         * committed surface state holds its own compositor-side reference. */
        if !self.bootstrap {
            if let Some(buffer) = self.bootstrap_buffer.take() {
                buffer.destroy();
            }
            self.bootstrap_pool = None;
        }
    }

    pub(super) fn map_bootstrap(&mut self) {
        if !self.bootstrap || self.bootstrap_mapped {
            return;
        }
        let (Some(surface), Some(buffer)) = (&self.surface, &self.bootstrap_buffer) else {
            return;
        };
        surface.attach(Some(buffer), 0, 0);
        surface.damage_buffer(0, 0, 1, 1);
        surface.commit();
        self.bootstrap_mapped = true;
    }

    /// Freeze the foreign-toplevel state into the small snapshot geometry
    /// needs, then unsubscribe. instantmenu does not need a taskbar-style
    /// stream of every window update for the rest of its lifetime.
    pub(super) fn finish_toplevel_snapshot(&mut self) {
        self.focused_outputs = self
            .toplevels
            .iter()
            .find(|t| t.activated)
            .map(|t| t.outputs.clone())
            .unwrap_or_default();
        for toplevel in self.toplevels.drain(..) {
            toplevel.proxy.destroy();
        }
        if let Some(manager) = self.foreign_manager.take() {
            manager.stop();
        }
    }

    pub(super) fn finish_xdg_outputs(&mut self) {
        for output in self.xdg_outputs.drain(..) {
            output.destroy();
        }
        if let Some(manager) = self.xdg_output_manager.take() {
            manager.destroy();
        }
    }
}

impl Drop for EventState {
    fn drop(&mut self) {
        /* pools, shields and probes clean up their protocol objects and fds */
        self.destroy_shields();
        self.destroy_probes();
        self.finish_xdg_outputs();
        for toplevel in self.toplevels.drain(..) {
            toplevel.proxy.destroy();
        }
    }
}
