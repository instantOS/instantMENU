//! The pixel canvas both backends present. BGRA8, row-major, matching X11
//! ZPixmap and little-endian Wayland ARGB8888 directly.

/// The pixel canvas both backends present. BGRA8, row-major
/// (b, g, r, a in memory, matching little-endian wl_shm ARGB8888 and X11
/// ZPixmap as written by put_image).
pub struct Canvas {
    pub width: i32,
    pub height: i32,
    pub data: Vec<u8>,
}

impl Canvas {
    pub fn new(width: i32, height: i32) -> Self {
        Canvas {
            width,
            height,
            data: vec![0; (width.max(0) as usize) * (height.max(0) as usize) * 4],
        }
    }

    pub fn resize(&mut self, width: i32, height: i32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.data = vec![0; (width.max(0) as usize) * (height.max(0) as usize) * 4];
    }

    /// Fill a clipped rectangle. Only the first scanline is assembled pixel
    /// by pixel; the remaining rows are memcpy-like copies of that scanline.
    /// This matters in debug builds, where a four-byte slice copy per pixel is
    /// otherwise particularly expensive.
    pub fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, rgba: [u8; 4]) {
        if width <= 0 || height <= 0 {
            return;
        }
        let x0 = x.max(0).min(self.width) as usize;
        let y0 = y.max(0).min(self.height) as usize;
        let x1 = (x + width).max(0).min(self.width) as usize;
        let y1 = (y + height).max(0).min(self.height) as usize;
        if x0 >= x1 || y0 >= y1 {
            return;
        }

        let stride = self.width as usize * 4;
        let row_start = y0 * stride + x0 * 4;
        let row_end = y0 * stride + x1 * 4;
        let bgra = [rgba[2], rgba[1], rgba[0], rgba[3]];
        for pixel in self.data[row_start..row_end].chunks_exact_mut(4) {
            pixel.copy_from_slice(&bgra);
        }
        for row in y0 + 1..y1 {
            let dst = row * stride + x0 * 4;
            self.data.copy_within(row_start..row_end, dst);
        }
    }

    /// Blit an alpha mask (8bpp) at (x, y) with the given color — used by the
    /// cosmic-text draw callbacks.
    #[inline]
    pub fn blend_pixel(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let off = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        let alpha = color[3] as u32;
        if alpha >= 255 {
            self.data[off..off + 4].copy_from_slice(&[color[2], color[1], color[0], color[3]]);
        } else if alpha == 0 {
            return;
        } else {
            let inv = 255 - alpha;
            // Canvas channels are BGRA; incoming cosmic-text colors are RGBA.
            for (dst_channel, src_channel) in [(0, 2), (1, 1), (2, 0)] {
                let dst = self.data[off + dst_channel] as u32;
                let src = color[src_channel] as u32;
                self.data[off + dst_channel] = ((src * alpha + dst * inv) / 255) as u8;
            }
            self.data[off + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Canvas;

    #[test]
    fn rectangle_fill_uses_backend_native_bgra_and_clips() {
        let mut canvas = Canvas::new(3, 2);
        canvas.fill_rect(-1, 0, 3, 2, [10, 20, 30, 255]);

        let filled = [30, 20, 10, 255];
        assert_eq!(&canvas.data[0..4], &filled);
        assert_eq!(&canvas.data[4..8], &filled);
        assert_eq!(&canvas.data[8..12], &[0, 0, 0, 0]);
        assert_eq!(&canvas.data[12..16], &filled);
        assert_eq!(&canvas.data[16..20], &filled);
        assert_eq!(&canvas.data[20..24], &[0, 0, 0, 0]);
    }

    #[test]
    fn opaque_glyph_pixels_are_converted_to_bgra() {
        let mut canvas = Canvas::new(1, 1);
        canvas.blend_pixel(0, 0, [10, 20, 30, 255]);
        assert_eq!(canvas.data, [30, 20, 10, 255]);
    }
}
