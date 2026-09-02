//! Wayland protocol event dispatch: the `Dispatch` impls for every global the
//! backend binds.

use std::os::fd::AsRawFd;
use std::time::Instant;

use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_keyboard, wl_output, wl_pointer, wl_region, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_offer_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use xkbcommon::xkb::Keycode;

use super::keyboard::load_keymap;
use super::probe::ProbeTag;
use super::selection::OfferTracker;
use super::shield::ShieldTag;
use super::state::{EventState, OutputEntry, PointerFocus, ToplevelInfo};
use crate::backend::{
    lookup_key, scroll, translate_key, BackendEvent, InputSource, MonitorInfo, MouseButton,
};
use crate::geom::{Point, Rect};

/// Offset added to raw evdev keycodes to get an xkb keycode (X11 keycodes
/// already include it).
const XKB_OFFSET: u32 = 8;

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
    wl_region::WlRegion,
    wl_data_device_manager::WlDataDeviceManager,
    wp_cursor_shape_device_v1::WpCursorShapeDeviceV1,
    WpCursorShapeManagerV1,
    zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    wp_viewporter::WpViewporter,
    wp_viewport::WpViewport,
);

impl Dispatch<wl_surface::WlSurface, ()> for EventState {
    fn event(
        state: &mut Self,
        proxy: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        /* Only the menu surface's placement matters. Probe and shield
         * surfaces also arrive as WlSurface with () userdata; their enters
         * are handled by their own layer-surface dispatchers. */
        if !state.surface.as_ref().is_some_and(|surface| surface == proxy) {
            return;
        }
        if let wl_surface::Event::Enter { output } = event {
            /* The first enter names the output the compositor placed the
             * surface on (the decision when the bootstrap was bound without
             * an output). Later enters cannot move an existing surface. */
            if state.menu_output.is_none() {
                if let Some(i) = state.outputs.iter().position(|entry| entry.proxy == output) {
                    state.menu_output = Some(i);
                }
            }
        }
    }
}

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
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                match interface.as_str() {
                    "wl_compositor" => {
                        state.compositor = Some(registry.bind(name, version.min(6), qh, ()))
                    }
                    "wl_shm" => state.shm = Some(registry.bind(name, version.min(1), qh, ())),
                    "wl_output" => {
                        let proxy: wl_output::WlOutput =
                            registry.bind(name, version.min(4), qh, ());
                        state.outputs.push(OutputEntry {
                            proxy,
                            info: MonitorInfo {
                                rect: Rect::default(),
                                name: String::new(),
                            },
                        });
                    }
                    "wl_seat" => {
                        /* Child wl_keyboard/wl_pointer objects inherit this
                         * version. v10 adds compositor-driven key repeats. */
                        let seat: wl_seat::WlSeat = registry.bind(name, version.min(10), qh, ());
                        state.seat = Some(seat);
                    }
                    "wl_data_device_manager" => {
                        state.data_device_manager =
                            Some(registry.bind(name, version.min(3), qh, ()))
                    }
                    "zwp_primary_selection_device_manager_v1" => {
                        state.primary_manager = Some(registry.bind(name, version.min(1), qh, ()))
                    }
                    "zwlr_layer_shell_v1" => {
                        state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()))
                    }
                    "zxdg_output_manager_v1" => {
                        state.xdg_output_manager = Some(registry.bind(name, version.min(3), qh, ()))
                    }
                    "wp_viewporter" => state.viewporter = Some(registry.bind(name, 1, qh, ())),
                    "wp_cursor_shape_manager_v1" => {
                        state.cursor_shape_manager =
                            Some(registry.bind(name, version.min(1), qh, ()))
                    }
                    "zwlr_foreign_toplevel_manager_v1" => {
                        /* Focus state and output_enter/output_leave are all
                         * present in v1; v2 only adds fullscreen controls. */
                        if state.track_focused_monitor {
                            state.foreign_manager =
                                Some(registry.bind(name, version.min(3), qh, ()))
                        }
                    }
                    "xdg_wm_base" => {
                        state.wm_base = Some(registry.bind(name, version.min(6), qh, ()))
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
        let Some(i) = state.outputs.iter().position(|o| o.proxy == *proxy) else {
            return;
        };
        match event {
            wl_output::Event::Geometry { x, y, .. } => {
                state.outputs[i].info.rect.x = x;
                state.outputs[i].info.rect.y = y;
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) => {
                state.outputs[i].info.rect.w = width;
                state.outputs[i].info.rect.h = height;
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
            let Ok(caps) = capabilities.into_result() else {
                return;
            };
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            } else if !caps.contains(wl_seat::Capability::Keyboard) {
                if let Some(keyboard) = state.keyboard.take() {
                    if keyboard.version() >= 3 {
                        keyboard.release();
                    }
                }
                state.xkb = None;
                state.key_repeat.cancel();
            }
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            } else if !caps.contains(wl_seat::Capability::Pointer) {
                if let Some(pointer) = state.pointer.take() {
                    if pointer.version() >= 3 {
                        pointer.release();
                    }
                }
                state.pointer_focus = PointerFocus::None;
            }
            /* bind the selection devices once we have a seat */
            if state.data_device.is_none() {
                if let Some(manager) = &state.data_device_manager {
                    state.data_device = Some(manager.get_data_device(seat, qh, ()));
                }
            }
            if state.primary_device.is_none() {
                if let Some(manager) = &state.primary_manager {
                    state.primary_device = Some(manager.get_device(seat, qh, ()));
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
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => {
                if let Some(xkb) = load_keymap(fd.as_raw_fd(), size as usize) {
                    state.key_repeat.cancel();
                    state.xkb = Some(xkb);
                }
                /* fd is an OwnedFd and closes on drop */
            }
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                let code = key + XKB_OFFSET;
                match key_state {
                    WEnum::Value(wl_keyboard::KeyState::Pressed) => {
                        let Some(x) = state.xkb.as_mut() else { return };
                        let (sym, text) = translate_key(&mut x.state, Keycode::new(code), true);
                        let mods = x.mods;
                        let repeats = x.key_repeats(code);
                        if repeats {
                            state.key_repeat.arm(code, Instant::now());
                        }
                        state
                            .events
                            .push_back(BackendEvent::KeyPress { sym, mods, text });
                    }
                    WEnum::Value(wl_keyboard::KeyState::Released) => {
                        state.key_repeat.release(code);
                        let Some(x) = state.xkb.as_mut() else { return };
                        let (sym, _) = translate_key(&mut x.state, Keycode::new(code), false);
                        state
                            .events
                            .push_back(BackendEvent::KeyRelease { sym, mods: x.mods });
                    }
                    WEnum::Value(wl_keyboard::KeyState::Repeated) => {
                        /* v10 compositor-side repeat. Never feed another key
                         * down into xkb: the physical key is already down. */
                        state.key_repeat.release(code);
                        let Some(x) = state.xkb.as_ref() else { return };
                        let (sym, text) = lookup_key(&x.state, Keycode::new(code));
                        state.events.push_back(BackendEvent::KeyPress {
                            sym,
                            mods: x.mods,
                            text,
                        });
                    }
                    WEnum::Value(_) | WEnum::Unknown(_) => (),
                }
            }
            wl_keyboard::Event::Enter { surface, keys, .. } => {
                state.key_repeat.cancel();
                state.keyboard_focused = state.surface.as_ref() == Some(&surface);
                if let Some(xkb) = state.xkb.as_mut() {
                    xkb.enter(&keys);
                }
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                state.key_repeat.cancel();
                if state.surface.as_ref() == Some(&surface) {
                    state.keyboard_focused = false;
                    state.events.push_back(BackendEvent::KeyboardLeft);
                }
                if let Some(xkb) = state.xkb.as_mut() {
                    xkb.leave();
                }
            }
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                state.key_repeat.update_info(rate, delay, Instant::now());
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                ..
            } => {
                let Some(x) = state.xkb.as_mut() else { return };
                x.state
                    .update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, 0);
                let mask = mods_depressed | mods_latched | mods_locked;
                x.mods = x.indices.modifiers(mask);
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
        /* Classify the pointer focus once at Enter, then read it on every
         * later event. The classification is a property of the surface, not
         * of each event — recomputing per-event meant a linear scan of
         * `state.shields` for every Motion, Axis, and Button. */
        let mods = state.xkb.as_ref().map(|x| x.mods).unwrap_or_default();
        let focus = match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                /* set_shape requests must carry the newest enter serial;
                 * a stale serial makes the compositor ignore the request */
                state.pointer_enter_serial = serial;
                let f = if state.surface.as_ref() == Some(&surface) {
                    /* only menu coordinates are meaningful; shield coords
                     * (and unknown ones) are discarded */
                    state.pointer_x = surface_x;
                    state.pointer_y = surface_y;
                    PointerFocus::Menu
                } else if state.shields.iter().any(|s| s.surface == surface) {
                    PointerFocus::Shield
                } else if let Some(probe) = state.probes.iter().find(|p| p.surface == surface) {
                    /* the probes exist only to learn the cursor position:
                     * the enter coordinates are surface-local, so add the
                     * output origin (surface coordinates are 1:1 with the
                     * logical space this backend works in) */
                    let origin = state
                        .outputs
                        .get(probe.output_index)
                        .map(|o| o.info.rect.origin())
                        .unwrap_or_else(|| Point::new(0, 0));
                    state.probe_answer = Some(Point::new(
                        origin.x + surface_x as i32,
                        origin.y + surface_y as i32,
                    ));
                    PointerFocus::None
                } else {
                    PointerFocus::None
                };
                state.pointer_focus = f;
                /* The cursor image is (re)asserted on every enter,
                 * unconditionally: compositors may reset it between
                 * surfaces, and set_shape needs the fresh serial anyway.
                 * Off-menu surfaces show the plain arrow; the remembered
                 * cursor survives so re-entering the menu restores it
                 * without waiting for the next motion. */
                match f {
                    PointerFocus::Menu => state.reassert_cursor(),
                    PointerFocus::Shield => state.shield_cursor(),
                    PointerFocus::None => {}
                }
                return;
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_focus = PointerFocus::None;
                return;
            }
            _ => state.pointer_focus,
        };
        if focus == PointerFocus::None {
            /* events that arrived while the pointer is on another client's
             * surface are dropped at the protocol layer; we never see them */
            return;
        }
        match event {
            wl_pointer::Event::Motion {
                time,
                surface_x,
                surface_y,
            } if focus == PointerFocus::Menu => {
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
                state.events.push_back(BackendEvent::Motion {
                    time,
                    pos: Point::new(surface_x as i32, surface_y as i32),
                    source: InputSource::Menu,
                });
            }
            wl_pointer::Event::Button {
                button,
                state: button_state,
                ..
            } => {
                let button_state = match button_state {
                    WEnum::Value(s) => s,
                    WEnum::Unknown(_) => return,
                };
                match (button_state, focus) {
                    (wl_pointer::ButtonState::Pressed, PointerFocus::Shield) => {
                        let Some(button) = MouseButton::from_evdev(button) else {
                            return;
                        };
                        state.events.push_back(BackendEvent::ButtonPress {
                            button,
                            mods,
                            pos: Point::new(state.pointer_x as i32, state.pointer_y as i32),
                            source: InputSource::External,
                        });
                    }
                    (wl_pointer::ButtonState::Pressed, PointerFocus::Menu) => {
                        let Some(button) = MouseButton::from_evdev(button) else {
                            return;
                        };
                        state.events.push_back(BackendEvent::ButtonPress {
                            button,
                            mods,
                            pos: Point::new(state.pointer_x as i32, state.pointer_y as i32),
                            source: InputSource::Menu,
                        });
                    }
                    (wl_pointer::ButtonState::Released, PointerFocus::Menu) => {
                        let Some(button) = MouseButton::from_evdev(button) else {
                            return;
                        };
                        state.events.push_back(BackendEvent::ButtonRelease {
                            button,
                            pos: Point::new(state.pointer_x as i32, state.pointer_y as i32),
                            source: InputSource::Menu,
                        });
                    }
                    _ => {}
                }
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } if focus == PointerFocus::Menu => {
                /* one scroll step per axis batch */
                state.events.push_back(BackendEvent::Scroll {
                    delta: scroll::from_axis_value(value),
                });
            }
            _ => {}
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
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width: _,
                height: _,
            } => {
                surface.ack_configure(serial);
                let was_configured = state.configured;
                state.configured = true;
                state.map_bootstrap();
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

/// xdg_output logical geometry overrides (the `usize` userdata is the index
/// into `outputs` the xdg_output was created for). wl_output mode sizes are
/// physical pixels; these events carry the logical size placement math
/// needs, which also makes scaled outputs lay out correctly.
impl Dispatch<zxdg_output_v1::ZxdgOutputV1, usize> for EventState {
    fn event(
        state: &mut Self,
        _proxy: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        data: &usize,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(output) = state.outputs.get_mut(*data) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                output.info.rect.x = x;
                output.info.rect.y = y;
            }
            zxdg_output_v1::Event::LogicalSize { width, height } if width > 0 && height > 0 => {
                output.info.rect.w = width;
                output.info.rect.h = height;
            }
            _ => {}
        }
    }
}

/// Foreign toplevels: track which window is activated (keyboard focus) and
/// which outputs it has entered — together those answer
/// `focused_monitor`.
impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for EventState {
    fn event(
        state: &mut Self,
        _proxy: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                state.toplevels.push(ToplevelInfo {
                    proxy: toplevel,
                    activated: false,
                    outputs: Vec::new(),
                });
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                for toplevel in state.toplevels.drain(..) {
                    toplevel.proxy.destroy();
                }
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(EventState, zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (
            zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
            ()
        ),
    ]);
}

impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for EventState {
    fn event(
        state: &mut Self,
        proxy: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(i) = state.toplevels.iter().position(|t| t.proxy == *proxy) else {
            return;
        };
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::State { state: states } => {
                state.toplevels[i].activated = states_contain_activated(&states);
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputEnter { output } => {
                let toplevel = &mut state.toplevels[i];
                if !toplevel.outputs.contains(&output) {
                    toplevel.outputs.push(output);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputLeave { output } => {
                state.toplevels[i].outputs.retain(|o| o != &output);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.remove(i).proxy.destroy();
            }
            _ => {}
        }
    }
}

/// Whether a `state` array contains ACTIVATED (2). The array is a run of
/// u32s in native endianness.
fn states_contain_activated(bytes: &[u8]) -> bool {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .any(|bytes| u32::from_ne_bytes(*bytes) == 2)
}

/// Probe layer surfaces (userdata [`ProbeTag`]): ack the configure, then
/// attach the transparent buffer — only a mapped surface receives
/// `wl_pointer.enter`, and a layer surface must stay bufferless until its
/// first configure has been acked.
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ProbeTag> for EventState {
    fn event(
        state: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &ProbeTag,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            surface.ack_configure(serial);
            if let Some(probe) = state
                .probes
                .iter_mut()
                .find(|p| &p.layer_surface == surface)
            {
                if !probe.configured {
                    probe.configured = true;
                    if let (Some(viewport), Ok(width), Ok(height)) =
                        (&probe.viewport, i32::try_from(width), i32::try_from(height))
                    {
                        if width > 0 && height > 0 {
                            viewport.set_destination(width, height);
                        }
                    }
                    let buffer = probe.buffer.clone();
                    probe.surface.attach(Some(&buffer), 0, 0);
                    probe.surface.commit();
                }
            }
        }
    }
}

/// Shield layer surfaces (userdata [`ShieldTag`]): ack the configure, then
/// attach the transparent buffer — a layer surface must stay bufferless
/// until its first configure has been acked.
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ShieldTag> for EventState {
    fn event(
        state: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &ShieldTag,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                surface.ack_configure(serial);
                if let Some(shield) = state
                    .shields
                    .iter_mut()
                    .find(|s| &s.layer_surface == surface)
                {
                    if !shield.configured {
                        shield.configured = true;
                        let buffer = shield.buffer.clone();
                        shield.surface.attach(Some(&buffer), 0, 0);
                        shield.surface.commit();
                    }
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                /* the compositor closed the shield; clicks on that output
                 * simply no longer close the menu */
                state.shields.retain(|s| &s.layer_surface != surface);
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
            xdg_toplevel::Event::Configure { .. } => {
                /* the WM owns the geometry of managed windows; the menu
                 * draws at its own content size regardless */
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
            /* A redraw that arrived while both slots were busy is the latest
             * authoritative canvas. Submit it as soon as a slot becomes
             * reusable instead of waiting for unrelated user input. */
            if state.pool.as_mut().is_some_and(|pool| pool.release(buffer)) {
                if let Some((frame, w, h)) = state.pending_frame.take() {
                    state.draw(&frame, w, h);
                }
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for EventState {
    fn event(
        state: &mut Self,
        _proxy: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.frame_done = true;
            /* the callback is done; dropping it destroys the object */
            state.frame_callback = None;
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
            wl_data_device::Event::Selection { id } if id.is_none() => {
                state.clipboard_offer = None;
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

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()> for EventState {
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
            zwp_primary_selection_device_v1::Event::Selection { id } if id.is_none() => {
                state.primary_offer = None;
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

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()> for EventState {
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

#[cfg(test)]
mod tests {
    use super::states_contain_activated;

    fn states(values: &[u32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect()
    }

    #[test]
    fn foreign_toplevel_state_finds_activated_anywhere() {
        assert!(states_contain_activated(&states(&[0, 3, 2])));
        assert!(!states_contain_activated(&states(&[0, 1, 3])));
    }

    #[test]
    fn foreign_toplevel_state_ignores_incomplete_trailing_word() {
        let mut bytes = states(&[1]);
        bytes.extend_from_slice(&2_u32.to_ne_bytes()[..3]);
        assert!(!states_contain_activated(&bytes));
    }
}
