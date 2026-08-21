//! Click-catcher shields: invisible fullscreen layer surfaces, one per
//! output, that consume (and dismiss on) presses outside the menu.

use wayland_client::protocol::{wl_buffer, wl_output, wl_surface};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::shm::MemfdPool;
use super::state::EventState;
use crate::geom::Rect;

/// Userdata marker distinguishing shield layer surfaces from the menu's own
/// (same protocol object, different `Dispatch` impls).
pub(super) struct ShieldTag;

/// A click-catcher: an invisible fullscreen layer surface whose input region
/// covers its whole output except the menu rectangle. A Wayland client cannot
/// see clicks on other clients' surfaces, so modal menus catch outside clicks
/// the way GTK context menus do — with a transparent shield under the menu
/// that consumes (and dismisses on) any press outside it.
pub(super) struct Shield {
    pub(super) surface: wl_surface::WlSurface,
    pub(super) layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    pub(super) buffer: wl_buffer::WlBuffer,
    /// whether the first configure has been acked and the buffer attached
    /// (a layer surface must stay bufferless until then).
    pub(super) configured: bool,
}

impl EventState {
    /// Destroy the shields and their pool (recreated on window re-creation).
    pub(super) fn destroy_shields(&mut self) {
        for shield in self.shields.drain(..) {
            shield.layer_surface.destroy();
            shield.surface.destroy();
            shield.buffer.destroy();
        }
        self.shield_pool = None; // drops the pool proxy and its memfd
    }

    /// One click-catcher per output (see [`Shield`]). The shields sit in the
    /// same layer as the menu; their input region excludes the menu rectangle
    /// so the menu keeps all of its own pointer events.
    pub(super) fn create_shields(&mut self, menu_rect: Rect) {
        self.destroy_shields();

        let Some(compositor) = self.compositor.clone() else {
            return;
        };
        let Some(shell) = self.layer_shell.clone() else {
            return;
        };
        let qh = self.queue_handle.clone();
        let outputs: Vec<(wl_output::WlOutput, Rect)> = self
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
        let Some(shm) = self.shm.clone() else {
            return;
        };
        let Some(pool) = MemfdPool::create(&shm, len, &qh) else {
            return;
        };
        self.shield_pool = Some(pool);
        let pool = self.shield_pool.as_ref().unwrap();

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

            let buffer = pool.create_buffer(offset, out_rect.w, out_rect.h, out_rect.w * 4, &qh);
            offset += out_rect.w * out_rect.h * 4;
            shields.push(Shield {
                surface: shield_surface,
                layer_surface,
                buffer,
                configured: false,
            });
        }
        self.shields = shields;
    }
}
