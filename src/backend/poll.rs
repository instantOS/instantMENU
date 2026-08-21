//! Shared fd-polling scaffolding for the backends.
//!
//! Both the X11 and the Wayland event loops wait on their connection fd plus
//! caller-owned extras with `libc::poll`, retry on EINTR, and prioritize the
//! extras (a blocked pipe producer is more time-critical than an
//! already-queued backend event). This module is that one behavior, kept in
//! one place.

use std::os::fd::RawFd;

/// What [`poll_fds`] observed.
pub(crate) enum PollOutcome {
    /// At least one fd has activity; inspect the `revents` fields.
    Ready,
    /// The timeout expired with no activity.
    Timeout,
    /// Poll failed with something other than EINTR: the wait itself is
    /// unusable.
    Closed,
}

/// `libc::poll` with an EINTR retry. `timeout_ms < 0` blocks indefinitely.
pub(crate) fn poll_fds(fds: &mut [libc::pollfd], timeout_ms: i32) -> PollOutcome {
    let n = loop {
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if n >= 0 {
            break n;
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return PollOutcome::Closed;
        }
    };
    if n == 0 {
        PollOutcome::Timeout
    } else {
        PollOutcome::Ready
    }
}

/// Milliseconds left of `timeout` relative to `start`; `Err(())` when the
/// budget is already spent (the caller returns its timeout). A `None`
/// timeout maps to `-1`: block indefinitely.
pub(crate) fn remaining_ms(
    start: std::time::Instant,
    timeout: Option<std::time::Duration>,
) -> Result<i32, ()> {
    match timeout {
        Some(dur) => {
            let elapsed = start.elapsed();
            if elapsed >= dur {
                return Err(());
            }
            Ok((dur - elapsed).as_millis().min(i32::MAX as u128) as i32)
        }
        None => Ok(-1),
    }
}

/// Index of the first fd at or after `from` that reported any watched
/// condition (`POLLIN`, error or hang-up).
pub(crate) fn first_ready(fds: &[libc::pollfd], from: usize) -> Option<usize> {
    (from..fds.len())
        .find(|&i| fds[i].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0)
}

/// A pollfd entry watching `fd` for readability.
pub(crate) fn poll_in(fd: RawFd) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}
