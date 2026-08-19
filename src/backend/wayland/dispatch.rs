//! Wayland protocol event dispatch: the `Dispatch` impls for every global the
//! backend binds.

use std::os::fd::AsRawFd;

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
    wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_offer_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};
use xkbcommon::xkb::{Keycode, KeyDirection};

use crate::backend::{MouseButton, XKB_OFFSET};
use super::selection::{load_keymap, x11_mask};
use super::{BackendEvent, EventState, MonitorInfo, OfferTracker, OutputEntry};

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
                let max_version = version.min(7);
                match interface.as_str() {
                    "wl_compositor" => {
                        state.compositor = Some(registry.bind(name, max_version.min(6), qh, ()))
                    }
                    "wl_shm" => state.shm = Some(registry.bind(name, 1.min(max_version), qh, ())),
                    "wl_output" => {
                        let proxy: wl_output::WlOutput = registry.bind(name, max_version.min(4), qh, ());
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
                        let seat: wl_seat::WlSeat = registry.bind(name, max_version.min(5), qh, ());
                        state.seat = Some(seat);
                    }
                    "wl_data_device_manager" => {
                        state.data_device_manager =
                            Some(registry.bind(name, max_version.min(3), qh, ()))
                    }
                    "zwp_primary_selection_device_manager_v1" => {
                        state.primary_manager =
                            Some(registry.bind(name, 1.min(max_version), qh, ()))
                    }
                    "zwlr_layer_shell_v1" => {
                        state.layer_shell = Some(registry.bind(name, max_version.min(4), qh, ()))
                    }
                    "xdg_wm_base" => {
                        state.wm_base = Some(registry.bind(name, max_version.min(6), qh, ()))
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
            wl_keyboard::Event::Key { key, state: key_state, .. } => {
                let Some(x) = state.xkb.as_mut() else { return };
                let code = Keycode::new(key + XKB_OFFSET);
                let pressed =
                    matches!(key_state, WEnum::Value(wl_keyboard::KeyState::Pressed));
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
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
            }
            wl_pointer::Event::Motion { time, surface_x, surface_y } => {
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
                state.events.push_back(BackendEvent::Motion {
                    time,
                    x: surface_x as i32,
                    y: surface_y as i32,
                });
            }
            wl_pointer::Event::Button { button, state: button_state, .. } => {
                if let WEnum::Value(wl_pointer::ButtonState::Pressed) = button_state {
                    let Some(button) = evdev_button(button) else { return };
                    state.events.push_back(BackendEvent::ButtonPress {
                        button,
                        state: mods,
                        x: state.pointer_x as i32,
                        y: state.pointer_y as i32,
                    });
                }
            }
            /* wheel: map to the scroll buttons the menu core understands */
            wl_pointer::Event::Axis { axis, value, .. } => {
                if let WEnum::Value(a) = axis {
                    if a == wl_pointer::Axis::VerticalScroll {
                        state.events.push_back(BackendEvent::ButtonPress {
                            button: if value > 0.0 { MouseButton::ScrollDown } else { MouseButton::ScrollUp },
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

/// Linux evdev button code -> normalized button.
/// BTN_LEFT = 0x110, BTN_RIGHT = 0x111, BTN_MIDDLE = 0x112.
fn evdev_button(code: u32) -> Option<MouseButton> {
    match code {
        0x110 => Some(MouseButton::Left),
        0x111 => Some(MouseButton::Right),
        0x112 => Some(MouseButton::Middle),
        _ => None,
    }
}
