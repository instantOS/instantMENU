//! wl_shm buffer management: memfd-backed pools, the double-buffered slot
//! ring for menu frames, and the border-compositing blit. All raw-pointer
//! and libc work of the Wayland backend is contained here.

use std::os::fd::{BorrowedFd, RawFd};

use wayland_client::protocol::{wl_buffer, wl_shm, wl_shm_pool};
use wayland_client::QueueHandle;

use super::state::EventState;
use crate::render::Color;

/// A wl_shm_pool backed by a sparse memfd of `len` bytes.
///
/// A zero-filled ARGB buffer is fully transparent, so pools that only bind
/// transparent buffers (shields, probes) never fault their pages in.
pub(super) struct MemfdPool {
    pool: wl_shm_pool::WlShmPool,
    fd: RawFd,
}

impl MemfdPool {
    /// memfd_create + ftruncate + bind as a wl_shm_pool. On any failure the
    /// fd is closed and `None` is returned.
    pub(super) fn create(
        shm: &wl_shm::WlShm,
        len: usize,
        qh: &QueueHandle<EventState>,
    ) -> Option<Self> {
        let name = b"instantmenu\0";
        let fd = unsafe { libc::memfd_create(name.as_ptr().cast(), 0) };
        if fd < 0 {
            return None;
        }
        if unsafe { libc::ftruncate(fd, len as libc::off_t) } != 0 {
            unsafe { libc::close(fd) };
            return None;
        }
        let pool = shm.create_pool(unsafe { BorrowedFd::borrow_raw(fd) }, len as i32, qh, ());
        Some(MemfdPool { pool, fd })
    }

    /// Bind a zero-filled ARGB8888 buffer at `offset`.
    pub(super) fn create_buffer(
        &self,
        offset: i32,
        width: i32,
        height: i32,
        stride: i32,
        qh: &QueueHandle<EventState>,
    ) -> wl_buffer::WlBuffer {
        self.pool.create_buffer(
            offset,
            width,
            height,
            stride,
            wl_shm::Format::Argb8888,
            qh,
            (),
        )
    }
}

impl Drop for MemfdPool {
    fn drop(&mut self) {
        self.pool.destroy();
        unsafe { libc::close(self.fd) };
    }
}

struct ShmSlot {
    buffer: wl_buffer::WlBuffer,
    offset: usize,
    /// whether the compositor has returned this buffer (`wl_buffer.release`)
    released: bool,
}

/// The menu's own SHM pool: mmap'd so frames are written directly, with a
/// ring of buffers the compositor returns one by one.
pub(super) struct ShmPool {
    backing: MemfdPool,
    /// mmap base of the whole pool; slot `i` starts at `slots[i].offset`.
    /// Valid (mapped) for the pool's lifetime.
    memory: *mut u8,
    len: usize,
    frame_size: usize,
    slots: Vec<ShmSlot>,
}

impl ShmPool {
    /// Create the pool and map its memory read-write.
    pub(super) fn create(
        shm: &wl_shm::WlShm,
        len: usize,
        frame_size: usize,
        qh: &QueueHandle<EventState>,
    ) -> Option<Self> {
        let backing = MemfdPool::create(shm, len, qh)?;
        let memory = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                backing.fd,
                0,
            )
        };
        if memory == libc::MAP_FAILED {
            drop(backing);
            return None;
        }
        Some(ShmPool {
            backing,
            memory: memory.cast(),
            len,
            frame_size,
            slots: Vec::new(),
        })
    }

    /// Whether this pool can serve another frame of `size` bytes without a
    /// realloc (the menu is fixed-size after setup, so this normally stays
    /// true after the first frame).
    pub(super) fn reusable_for(&self, size: usize, frame_size: usize) -> bool {
        self.len >= size * 2 && self.frame_size == frame_size
    }

    /// A released slot, else a fresh buffer appended while capacity remains.
    /// `None` means every slot is owned by the compositor and the pool is
    /// full — the caller coalesces the frame into `pending_frame`.
    pub(super) fn acquire(
        &mut self,
        width: i32,
        height: i32,
        stride: i32,
        size: usize,
        qh: &QueueHandle<EventState>,
    ) -> Option<usize> {
        match self.slots.iter().position(|s| s.released) {
            Some(i) => {
                self.slots[i].released = false;
                Some(i)
            }
            None => {
                let offset = self.slots.len() * size;
                if offset + size > self.len {
                    return None;
                }
                let buffer = self
                    .backing
                    .create_buffer(offset as i32, width, height, stride, qh);
                self.slots.push(ShmSlot {
                    buffer,
                    offset,
                    released: false,
                });
                Some(self.slots.len() - 1)
            }
        }
    }

    /// Mapped memory of slot `idx`, covering exactly one frame.
    ///
    /// # Safety contract
    /// `idx` must be a live slot index; the slice aliases the mmap, which
    /// outlives every slot.
    pub(super) fn frame_memory(&mut self, idx: usize, size: usize) -> &mut [u8] {
        let offset = self.slots[idx].offset;
        unsafe { std::slice::from_raw_parts_mut(self.memory.add(offset), size) }
    }

    /// The buffer proxy of slot `idx`, for attaching to the surface.
    pub(super) fn slot_buffer(&self, idx: usize) -> &wl_buffer::WlBuffer {
        &self.slots[idx].buffer
    }

    /// Mark the slot owning `buffer` as released; false when the buffer
    /// belongs to another pool (shields/probes never receive releases).
    pub(super) fn release(&mut self, buffer: &wl_buffer::WlBuffer) -> bool {
        for slot in &mut self.slots {
            if slot.buffer == *buffer {
                slot.released = true;
                return true;
            }
        }
        false
    }
}

impl Drop for ShmPool {
    fn drop(&mut self) {
        for slot in &self.slots {
            slot.buffer.destroy();
        }
        unsafe { libc::munmap(self.memory.cast(), self.len) };
        // `backing` drops after the body: pool proxy destroyed, fd closed.
    }
}

/// Copy the canvas into a mapped frame, painting the border ring when
/// `border > 0` — Wayland surfaces have no server-side border like X11
/// windows, so `--border-width` is composited here. Canvas pixels already
/// match little-endian wl_shm ARGB8888 (BGRA byte order).
pub(super) fn blit_frame(
    dst: &mut [u8],
    bgra: &[u8],
    content_w: usize,
    content_h: usize,
    border: usize,
    color: Color,
) {
    if border == 0 {
        dst.copy_from_slice(bgra);
        return;
    }
    let stride = (content_w + 2 * border) * 4;
    let content_stride = content_w * 4;
    let pixel = [color.b(), color.g(), color.r(), color.a()];
    for chunk in dst.chunks_exact_mut(4) {
        chunk.copy_from_slice(&pixel);
    }
    for row in 0..content_h {
        let src = &bgra[row * content_stride..][..content_stride];
        let dst_row = &mut dst[(row + border) * stride + border * 4..][..content_stride];
        dst_row.copy_from_slice(src);
    }
}
