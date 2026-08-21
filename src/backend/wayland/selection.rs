//! Selection (clipboard / primary) transfer plumbing.

use std::os::fd::{BorrowedFd, RawFd};

use wayland_client::protocol::wl_data_offer;
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_offer_v1;

/// A selection offer (clipboard or primary) being tracked.
pub(super) struct OfferTracker {
    pub(super) mimes: Vec<String>,
    pub(super) read_fd: Option<RawFd>,
    pending: Vec<u8>,
}

impl OfferTracker {
    pub(super) fn new() -> Self {
        OfferTracker {
            mimes: Vec::new(),
            read_fd: None,
            pending: Vec::new(),
        }
    }

    pub(super) fn best_mime(&self) -> Option<String> {
        for wanted in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"] {
            if let Some(m) = self.mimes.iter().find(|m| m.eq_ignore_ascii_case(wanted)) {
                return Some(m.clone());
            }
        }
        None
    }
}

/// Offers that can stream their content into a pipe. Clipboard and
/// primary-selection offers have identical `receive` methods; this trait
/// lets one code path serve both.
pub(super) trait OfferReceive {
    fn offer_receive(&self, mime: String, fd: BorrowedFd<'_>);
}

impl OfferReceive for wl_data_offer::WlDataOffer {
    fn offer_receive(&self, mime: String, fd: BorrowedFd<'_>) {
        wl_data_offer::WlDataOffer::receive(self, mime, fd)
    }
}

impl OfferReceive for zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1 {
    fn offer_receive(&self, mime: String, fd: BorrowedFd<'_>) {
        zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1::receive(self, mime, fd)
    }
}

/// Start streaming `mime` from the tracked offer into a fresh pipe. On
/// success the non-blocking read end is registered on the tracker for
/// [`pump_offer`] to drain; the write end is closed immediately after the
/// request is queued — the compositor received its own duplicate over the
/// socket, and any remaining writer would keep the reader from ever seeing
/// EOF. False (with both ends closed again) when there is no offer or a
/// transfer is already in flight.
pub(super) fn start_transfer<T: OfferReceive>(
    slot: &mut Option<(T, OfferTracker)>,
    mime: &str,
) -> bool {
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return false;
    }
    /* make the read end non-blocking: pump_offer() must never block the
     * event loop while the compositor streams the selection in */
    unsafe { libc::fcntl(fds[0], libc::F_SETFL, libc::O_NONBLOCK) };

    match slot {
        Some((offer, tracker)) if tracker.read_fd.is_none() => {
            offer.offer_receive(mime.to_string(), unsafe { BorrowedFd::borrow_raw(fds[1]) });
            unsafe { libc::close(fds[1]) };
            tracker.read_fd = Some(fds[0]);
            true
        }
        _ => {
            /* no offer, or a transfer is already in flight */
            unsafe {
                libc::close(fds[0]);
                libc::close(fds[1])
            };
            false
        }
    }
}

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
