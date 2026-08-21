//! Pointer probes: temporary fullscreen input surfaces mapped only while a
//! `pointer_position` query runs.

use wayland_client::protocol::{wl_buffer, wl_output, wl_surface};
use wayland_protocols::wp::viewporter::client::wp_viewport;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::shm::MemfdPool;
use super::state::EventState;
use crate::geom::Rect;

/// Userdata marker for the pointer-probe layer surfaces.
pub(super) struct ProbeTag;

/// A temporary fullscreen input surface mapped on demand to learn where
/// the pointer is. Wayland only reports pointer coordinates for surfaces
/// under the pointer, so before the menu exists there is nothing to ask —
/// mapping an invisible surface beneath the stationary cursor makes the
/// compositor deliver `wl_pointer.enter` with the current position. It is
/// destroyed before [`crate::backend::Backend::pointer_position`] returns
/// so it cannot intercept input during unrelated startup work.
pub(super) struct Probe {
    pub(super) surface: wl_surface::WlSurface,
    pub(super) layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    pub(super) buffer: wl_buffer::WlBuffer,
    /// Scales a 1x1 transparent buffer to the configured surface size when
    /// wp_viewporter is available, avoiding full-output SHM buffers.
    pub(super) viewport: Option<wp_viewport::WpViewport>,
    /// whether the first configure has been acked and the buffer attached
    /// (a layer surface must stay bufferless until then)
    pub(super) configured: bool,
    /// index into `outputs`; turns surface-local enter coordinates into
    /// global ones via the output origin
    pub(super) output_index: usize,
}

/// Maximum lifetime of a probe query. Keeping the whole probe inside this
/// bounded call is more important than overlapping it with unrelated
/// startup work: while mapped, the surfaces necessarily receive pointer
/// input instead of the application beneath them.
pub(super) const PROBE_TIMEOUT_MS: u64 = 100;

impl EventState {
    /// Map one invisible fullscreen surface per output for the active
    /// `pointer_position` query. No-op when there is no pointer or no
    /// layer-shell support (the query then returns None, like before).
    pub(super) fn create_probes(&mut self) {
        if !self.probes.is_empty() || self.probe_answer.is_some() || self.pointer.is_none() {
            return;
        }
        let Some(compositor) = self.compositor.clone() else {
            return;
        };
        let Some(shell) = self.layer_shell.clone() else {
            return;
        };
        let qh = self.queue_handle.clone();
        let outputs: Vec<(usize, wl_output::WlOutput, Rect)> = self
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, o)| o.info.rect.w > 0 && o.info.rect.h > 0)
            .map(|(i, o)| (i, o.proxy.clone(), o.info.rect))
            .collect();
        if outputs.is_empty() {
            return;
        }

        /* With wp_viewporter each surface needs only a 1x1 transparent
         * buffer. The fallback uses full-size sparse buffers. All Wayland
         * SHM sizes and offsets are signed 32-bit values, so reject layouts
         * that cannot be represented instead of overflowing. */
        let scaled = self.viewporter.is_some();
        let len = if scaled {
            outputs.len().checked_mul(4)
        } else {
            outputs.iter().try_fold(0usize, |total, (_, _, rect)| {
                let bytes = (rect.w as usize)
                    .checked_mul(rect.h as usize)?
                    .checked_mul(4)?;
                total.checked_add(bytes)
            })
        };
        let Some(len) = len.filter(|len| *len <= i32::MAX as usize) else {
            return;
        };
        let Some(shm) = self.shm.clone() else {
            return;
        };
        let Some(pool) = MemfdPool::create(&shm, len, &qh) else {
            return;
        };
        self.probe_pool = Some(pool);
        let pool = self.probe_pool.as_ref().unwrap();

        let mut offset: i32 = 0;
        let mut probes = Vec::with_capacity(outputs.len());
        for (idx, output, rect) in outputs {
            let surface = compositor.create_surface(&qh, ());
            let viewport = self.viewporter.as_ref().map(|viewporter| {
                let viewport = viewporter.get_viewport(&surface, &qh, ());
                viewport.set_destination(rect.w, rect.h);
                viewport
            });
            let layer_surface = shell.get_layer_surface(
                &surface,
                Some(&output),
                zwlr_layer_shell_v1::Layer::Top,
                "instantmenu".to_string(),
                &qh,
                ProbeTag,
            );
            layer_surface.set_anchor(
                zwlr_layer_surface_v1::Anchor::Top
                    | zwlr_layer_surface_v1::Anchor::Bottom
                    | zwlr_layer_surface_v1::Anchor::Left
                    | zwlr_layer_surface_v1::Anchor::Right,
            );
            layer_surface.set_exclusive_zone(-1);
            layer_surface
                .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
            layer_surface.set_size(rect.w as u32, rect.h as u32);
            /* the default input region is the whole surface — exactly what
             * a probe wants */
            surface.commit();
            let (buffer_w, buffer_h) = if scaled { (1, 1) } else { (rect.w, rect.h) };
            let buffer = pool.create_buffer(offset, buffer_w, buffer_h, buffer_w * 4, &qh);
            offset += buffer_w * buffer_h * 4;
            probes.push(Probe {
                surface,
                layer_surface,
                buffer,
                viewport,
                configured: false,
                output_index: idx,
            });
        }
        self.probes = probes;
    }

    /// Destroy the probe surfaces once the position is known or the bounded
    /// query gives up. `create_window` also calls this defensively.
    pub(super) fn destroy_probes(&mut self) {
        for probe in self.probes.drain(..) {
            if let Some(viewport) = probe.viewport {
                viewport.destroy();
            }
            probe.layer_surface.destroy();
            probe.surface.destroy();
            probe.buffer.destroy();
        }
        self.probe_pool = None; // drops the pool proxy and its memfd
    }
}
