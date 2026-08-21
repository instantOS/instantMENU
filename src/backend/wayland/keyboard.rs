//! xkb keyboard state: the modifier indices resolved from the compositor
//! keymap and the keymap/state pair itself.

use std::os::unix::io::RawFd;
use std::time::{Duration, Instant};

use xkbcommon::xkb;

use crate::backend::Modifiers;

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

pub(super) struct Xkb {
    /// Kept alive alongside the state (the C-level keymap backs it).
    #[allow(dead_code)]
    context: xkb::Context,
    keymap: xkb::Keymap,
    /// Key state the key/modifiers dispatch handlers drive.
    pub(super) state: xkb::State,
    pub(super) indices: ModIndices,
    /// Last-seen modifier state. Wayland button events carry no modifier
    /// state of their own, so pointer events are stamped from this cache.
    pub(super) mods: Modifiers,
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

    pub(super) fn key_repeats(&self, code: u32) -> bool {
        self.keymap.key_repeats(xkb::Keycode::new(code))
    }

    /// `wl_keyboard.enter` is a state snapshot, not a series of presses.
    /// Rebuild xkb's physical state from it without emitting menu events.
    pub(super) fn enter(&mut self, raw_keys: &[u8]) {
        self.state = xkb::State::new(&self.keymap);
        let (keys, _) = raw_keys.as_chunks::<4>();
        for bytes in keys {
            let raw = u32::from_ne_bytes(*bytes);
            self.state
                .update_key(xkb::Keycode::new(raw + 8), xkb::KeyDirection::Down);
        }
    }

    pub(super) fn leave(&mut self) {
        self.state = xkb::State::new(&self.keymap);
        self.mods = Modifiers::default();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepeatSettings {
    Disabled,
    Enabled { delay: Duration, interval: Duration },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RepeatPhase {
    Delay,
    Interval,
}

#[derive(Debug, Clone, Copy)]
struct ActiveRepeat {
    code: u32,
    deadline: Instant,
    phase: RepeatPhase,
}

/// Client-side half of `wl_keyboard` repeat handling.
///
/// A positive compositor rate asks the client to schedule repeats. A zero
/// rate disables this timer; with keyboard v10 the compositor may then send
/// `KeyState::Repeated` events itself, which the dispatch path handles
/// directly.
pub(super) struct KeyboardRepeat {
    settings: RepeatSettings,
    active: Option<ActiveRepeat>,
}

impl KeyboardRepeat {
    pub(super) fn new() -> Self {
        Self {
            settings: RepeatSettings::Disabled,
            active: None,
        }
    }

    pub(super) fn update_info(&mut self, rate: i32, delay: i32, now: Instant) {
        self.settings = if rate <= 0 || delay < 0 {
            RepeatSettings::Disabled
        } else {
            /* Nanosecond precision avoids a zero interval even for a
             * maliciously large (but protocol-valid) rate. */
            let interval_ns = (1_000_000_000_u64 / rate as u64).max(1);
            RepeatSettings::Enabled {
                delay: Duration::from_millis(delay as u64),
                interval: Duration::from_nanos(interval_ns),
            }
        };

        match (self.settings, self.active.as_mut()) {
            (RepeatSettings::Disabled, _) => self.active = None,
            (RepeatSettings::Enabled { delay, interval }, Some(active)) => {
                /* Apply live settings changes to a held key. Before its first
                 * repeat it still observes the new delay; afterwards it moves
                 * to the new cadence. */
                active.deadline = now
                    + if active.phase == RepeatPhase::Delay {
                        delay
                    } else {
                        interval
                    };
            }
            (RepeatSettings::Enabled { .. }, None) => {}
        }
    }

    pub(super) fn arm(&mut self, code: u32, now: Instant) {
        if let RepeatSettings::Enabled { delay, .. } = self.settings {
            self.active = Some(ActiveRepeat {
                code,
                deadline: now + delay,
                phase: RepeatPhase::Delay,
            });
        }
    }

    pub(super) fn release(&mut self, code: u32) {
        if self.active.is_some_and(|active| active.code == code) {
            self.active = None;
        }
    }

    pub(super) fn cancel(&mut self) {
        self.active = None;
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        self.active.map(|active| active.deadline)
    }

    /// Return one due repeat and schedule the next interval from `now`.
    /// Scheduling from the observation time deliberately avoids a burst of
    /// catch-up keypresses after rendering or process scheduling stalls.
    pub(super) fn take_due(&mut self, now: Instant) -> Option<u32> {
        let RepeatSettings::Enabled { interval, .. } = self.settings else {
            return None;
        };
        let active = self.active.as_mut()?;
        if now < active.deadline {
            return None;
        }
        active.deadline = now + interval;
        active.phase = RepeatPhase::Interval;
        Some(active.code)
    }
}

/// Load the xkb keymap from the compositor's keymap fd.
pub(super) fn load_keymap(fd: RawFd, size: usize) -> Option<Xkb> {
    let map = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            fd,
            0,
        )
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
    Some(Xkb::new(ctx, keymap))
}

#[cfg(test)]
mod tests {
    use super::KeyboardRepeat;
    use std::time::{Duration, Instant};

    #[test]
    fn positive_repeat_info_uses_delay_then_rate() {
        let start = Instant::now();
        let mut repeat = KeyboardRepeat::new();
        repeat.update_info(25, 300, start);
        repeat.arm(38, start);

        assert_eq!(repeat.deadline(), Some(start + Duration::from_millis(300)));
        assert_eq!(repeat.take_due(start + Duration::from_millis(299)), None);
        assert_eq!(
            repeat.take_due(start + Duration::from_millis(300)),
            Some(38)
        );
        assert_eq!(repeat.deadline(), Some(start + Duration::from_millis(340)));
    }

    #[test]
    fn zero_rate_disables_and_cancels_client_repeat() {
        let start = Instant::now();
        let mut repeat = KeyboardRepeat::new();
        repeat.update_info(20, 200, start);
        repeat.arm(24, start);
        repeat.update_info(0, 200, start + Duration::from_millis(50));

        assert_eq!(repeat.deadline(), None);
        assert_eq!(repeat.take_due(start + Duration::from_secs(1)), None);
    }

    #[test]
    fn only_releasing_the_active_key_cancels_it() {
        let start = Instant::now();
        let mut repeat = KeyboardRepeat::new();
        repeat.update_info(10, 100, start);
        repeat.arm(24, start);
        repeat.release(25);
        assert!(repeat.deadline().is_some());
        repeat.release(24);
        assert_eq!(repeat.deadline(), None);
    }

    #[test]
    fn delayed_loop_does_not_emit_a_catch_up_burst() {
        let start = Instant::now();
        let late = start + Duration::from_secs(2);
        let mut repeat = KeyboardRepeat::new();
        repeat.update_info(20, 100, start);
        repeat.arm(24, start);

        assert_eq!(repeat.take_due(late), Some(24));
        assert_eq!(repeat.take_due(late), None);
        assert_eq!(repeat.deadline(), Some(late + Duration::from_millis(50)));
    }

    #[test]
    fn live_rate_change_reschedules_a_repeating_key() {
        let start = Instant::now();
        let changed = start + Duration::from_millis(120);
        let mut repeat = KeyboardRepeat::new();
        repeat.update_info(20, 100, start);
        repeat.arm(24, start);
        assert_eq!(
            repeat.take_due(start + Duration::from_millis(100)),
            Some(24)
        );

        repeat.update_info(10, 400, changed);
        assert_eq!(
            repeat.deadline(),
            Some(changed + Duration::from_millis(100))
        );
    }
}
