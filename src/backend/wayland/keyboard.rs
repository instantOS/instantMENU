//! xkb keyboard state: the modifier indices resolved from the compositor
//! keymap and the keymap/state pair itself.

use std::os::unix::io::RawFd;

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
    #[allow(dead_code)]
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
