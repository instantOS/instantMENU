//! Wayland backend — wlr-layer-shell (override-redirect equivalent) or
//! xdg-shell for `-wm` managed windows.
//!
//! Submodules: `state` (dispatch state + frame path), `dispatch` (protocol
//! event handlers), `shm` (buffer pools), `keyboard` (xkb), `selection`
//! (clipboard/primary transfers), `probe` (pointer-position queries),
//! `shield` (outside-click catchers).

mod dispatch;
mod keyboard;
mod probe;
mod selection;
mod shield;
mod shm;
mod state;

use std::os::fd::AsRawFd;
use std::os::unix::io::RawFd;

use wayland_client::protocol::wl_surface;
use wayland_client::{Connection, EventQueue};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::poll::{first_ready, poll_fds, poll_in, remaining_ms, PollOutcome};
use super::{Backend, BackendEvent, EventPoll, MonitorInfo};
use crate::geom::{Point, Rect, Size};
use crate::render::{Canvas, Color};
use probe::PROBE_TIMEOUT_MS;
use selection::{pump_offer, start_transfer};
use state::EventState;

pub struct WaylandBackend {
    connection: Connection,
    queue: EventQueue<EventState>,
    state: EventState,
}

/* ──────────────────────── Backend impl ─────────────────────────────── */

impl WaylandBackend {
    pub fn new(track_focused_monitor: bool) -> Result<WaylandBackend, String> {
        let connection =
            Connection::connect_to_env().map_err(|e| format!("cannot connect: {e}"))?;
        let mut queue: EventQueue<EventState> = connection.new_event_queue();
        let queue_handle = queue.handle();
        let mut state = EventState::new(queue_handle.clone(), track_focused_monitor);
        let _ = connection.display().get_registry(&state.queue_handle, ());
        /* The first sync discovers globals. Create derived xdg-output
         * objects before the second sync so wl_output, logical-output and
         * foreign-toplevel initial state all arrive in that existing trip. */
        queue
            .roundtrip(&mut state)
            .map_err(|error| format!("registry roundtrip failed: {error}"))?;
        if let Some(manager) = state.xdg_output_manager.clone() {
            for (i, output) in state.outputs.iter().enumerate() {
                state.xdg_outputs.push(manager.get_xdg_output(
                    &output.proxy,
                    &state.queue_handle,
                    i,
                ));
            }
        }
        queue
            .roundtrip(&mut state)
            .map_err(|error| format!("initial-state roundtrip failed: {error}"))?;
        if state.shm.is_none() {
            return Err("compositor has no wl_shm".to_string());
        }

        /* xdg-output has now refined mode pixels into logical geometry. */
        state.monitors = state.outputs.iter().map(|o| o.info.clone()).collect();
        state.finish_xdg_outputs();
        state.finish_toplevel_snapshot();
        connection
            .flush()
            .map_err(|error| format!("initial Wayland flush failed: {error}"))?;

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

    /// wlr-layer-shell surface anchored to the chosen output. `rect` is the
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

        /* Translate the core's absolute rectangle into layer-shell geometry.
         * The core owns placement, so use one stable top-left coordinate
         * system rather than choosing an anchor from the initial height.
         * Choosing Top/Bottom dynamically made later streamed resizes preserve
         * the old edge while the core recomputed a new centered rectangle. */
        let monitor = state
            .outputs
            .get(output_index)
            .map(|o| o.info.rect)
            .unwrap_or(rect);
        place_layer(&layer_surface, rect, monitor);
        layer_surface.set_keyboard_interactivity(if grab {
            zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive
        } else {
            zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand
        });
        surface.commit();
        state.surface = Some(surface.clone());
        state.layer_surface = Some(layer_surface);
        if outside_close {
            state.create_shields(rect);
        }
        Ok(())
    }
}

/// The content rect expanded by the border on all sides (the full surface
/// footprint; X11 gets its border from the server, Wayland paints it).
fn bordered(rect: Rect, border: i32) -> Rect {
    Rect::new(rect.x, rect.y, rect.w + 2 * border, rect.h + 2 * border)
}

/// Express an absolute core rectangle as layer-shell state. Keeping the
/// anchor fixed makes move+resize an exact operation: both initial creation
/// and every streamed reflow update the same top/left margins and size.
fn place_layer(
    layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    rect: Rect,
    monitor: Rect,
) {
    layer_surface
        .set_anchor(zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left);
    let (top, left) = layer_margins(rect, monitor);
    layer_surface.set_margin(top, 0, 0, left);
    layer_surface.set_size(rect.w.max(1) as u32, rect.h.max(1) as u32);
}

fn layer_margins(rect: Rect, monitor: Rect) -> (i32, i32) {
    ((rect.y - monitor.y).max(0), (rect.x - monitor.x).max(0))
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

    fn pointer_position(&mut self) -> Option<Point> {
        if let Some(pos) = self.state.probe_answer {
            return Some(pos);
        }
        self.state.create_probes();
        if self.state.probes.is_empty() {
            return None;
        }
        if self.connection.flush().is_err() {
            self.state.dead = true;
            self.state.destroy_probes();
            return None;
        }
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(PROBE_TIMEOUT_MS);
        while self.state.probe_answer.is_none() && !self.state.dead {
            let Ok(ms) = remaining_ms(start, Some(timeout)) else {
                break;
            };
            let Some(guard) = self.queue.prepare_read() else {
                /* events already queued: dispatch them, then re-check */
                if self.queue.dispatch_pending(&mut self.state).is_err() {
                    break;
                }
                continue;
            };
            let mut fds = [poll_in(guard.connection_fd().as_raw_fd())];
            match poll_fds(&mut fds, ms) {
                PollOutcome::Timeout => break,
                PollOutcome::Closed => {
                    self.state.dead = true;
                    break;
                }
                PollOutcome::Ready => {}
            }
            if guard.read().is_err() {
                self.state.dead = true;
                break;
            }
            if self.queue.dispatch_pending(&mut self.state).is_err() {
                break;
            }
        }
        self.state.destroy_probes();
        self.state.probe_answer
    }

    fn focused_monitor(&self) -> Option<usize> {
        /* A window visible on multiple outputs has no protocol-defined
         * primary output. Report that as ambiguous so Auto can fall back to
         * the pointer instead of depending on arbitrary output_enter order. */
        let [focused] = self.state.focused_outputs.as_slice() else {
            return None;
        };
        self.state
            .outputs
            .iter()
            .position(|output| &output.proxy == focused)
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
        /* Defensive cleanup if a future probe path exits unexpectedly. */
        self.state.destroy_probes();
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
        let output_index = self.state.output_for_point(full.origin());
        let monitor = self
            .state
            .outputs
            .get(output_index)
            .map(|o| o.info.rect)
            .unwrap_or(full);
        if let Some(layer_surface) = &self.state.layer_surface {
            place_layer(layer_surface, Rect::new(full.x, full.y, w, h), monitor);
            if let Some(surface) = &self.state.surface {
                surface.commit();
            }
        }
        /* managed (xdg-toplevel) windows: the WM owns the geometry */
        if !self.state.shields.is_empty() {
            /* recreate the click-catchers so their input region holes track
             * the new menu rectangle */
            self.state.create_shields(Rect::new(full.x, full.y, w, h));
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

        if clipboard {
            start_transfer(&mut self.state.clipboard_offer, &mime);
        } else {
            start_transfer(&mut self.state.primary_offer, &mime);
        }
        /* flush so the compositor actually receives the request and writes
         * to the pipe (next_event() polls it only after this) */
        let _ = self.connection.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::layer_margins;
    use crate::geom::Rect;

    #[test]
    fn layer_margins_follow_every_absolute_reflow() {
        let monitor = Rect::new(100, 50, 1600, 900);
        let initial = Rect::new(450, 480, 900, 30);
        let grown = Rect::new(450, 340, 900, 310);

        assert_eq!(layer_margins(initial, monitor), (430, 350));
        assert_eq!(layer_margins(grown, monitor), (290, 350));
    }

    #[test]
    fn layer_margins_are_monitor_local_and_clamped() {
        let monitor = Rect::new(-1920, -1080, 1920, 1080);
        assert_eq!(
            layer_margins(Rect::new(-1800, -1000, 600, 300), monitor),
            (80, 120)
        );
        assert_eq!(
            layer_margins(Rect::new(-2000, -1200, 600, 300), monitor),
            (0, 0)
        );
    }
}
