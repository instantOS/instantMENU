//! The pixel canvas both backends present. RGBA8, row-major.

/// The pixel canvas both backends present. RGBA8, row-major.
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

    #[inline]
    pub fn fill_pixel(&mut self, x: i32, y: i32, color: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return;
        }
        let off = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        self.data[off..off + 4].copy_from_slice(&color);
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
            self.data[off..off + 4].copy_from_slice(&color);
        } else if alpha == 0 {
            return;
        } else {
            let inv = 255 - alpha;
            for i in 0..3 {
                let dst = self.data[off + i] as u32;
                let src = color[i] as u32;
                self.data[off + i] = ((src * alpha + dst * inv) / 255) as u8;
            }
            self.data[off + 3] = 255;
        }
    }
}
