//! Selection transfer plumbing and xkb keymap loading.

use std::os::unix::io::RawFd;

use xkbcommon::xkb;

use super::{OfferTracker, Xkb};

/// Drain one offer's read pipe; Some(text) when the transfer finished.
pub(super) fn pump_offer<T>(slot: &mut Option<(T, OfferTracker)>) -> Option<String> {
    let (_, tracker) = slot.as_mut()?;
    let fd = tracker.read_fd?;
    let mut buf = [0u8; 4096];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            tracker.pending.extend_from_slice(&buf[..n as usize]);
        } else if n == 0 {
            unsafe { libc::close(fd) };
            tracker.read_fd = None;
            return Some(String::from_utf8_lossy(&tracker.pending).into_owned());
        } else {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::WouldBlock {
                unsafe { libc::close(fd) };
                tracker.read_fd = None;
                return Some(String::from_utf8_lossy(&tracker.pending).into_owned());
            }
            return None; // more to come later
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
