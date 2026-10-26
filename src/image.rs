/// Packed 8-bit RGB frame, row-major, no padding.
#[derive(Clone)]
pub struct RgbFrame {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl RgbFrame {
    pub fn new(width: usize, height: usize, data: Vec<u8>) -> Self {
        debug_assert_eq!(data.len(), width * height * 3);
        Self { width, height, data }
    }

    pub fn flip_horizontal(&mut self) {
        let row = self.width * 3;
        for line in self.data.chunks_exact_mut(row) {
            for x in 0..self.width / 2 {
                let (a, b) = (x * 3, (self.width - 1 - x) * 3);
                for c in 0..3 {
                    line.swap(a + c, b + c);
                }
            }
        }
    }

    /// Nearest-neighbour downscale used for the UI preview only.
    pub fn downscale_to_width(&self, max_width: usize) -> RgbFrame {
        if self.width <= max_width {
            return self.clone();
        }
        let scale = max_width as f32 / self.width as f32;
        let height = ((self.height as f32 * scale).round() as usize).max(1);
        let mut data = Vec::with_capacity(max_width * height * 3);
        for y in 0..height {
            let sy = ((y as f32 + 0.5) / scale) as usize;
            let src = &self.data[sy.min(self.height - 1) * self.width * 3..];
            for x in 0..max_width {
                let sx = (((x as f32 + 0.5) / scale) as usize).min(self.width - 1) * 3;
                data.extend_from_slice(&src[sx..sx + 3]);
            }
        }
        RgbFrame::new(max_width, height, data)
    }

    #[inline]
    fn pixel(&self, x: isize, y: isize, border: Border) -> [f32; 3] {
        let (x, y) = match border {
            Border::Constant => {
                if x < 0 || y < 0 || x >= self.width as isize || y >= self.height as isize {
                    return [0.0; 3];
                }
                (x as usize, y as usize)
            }
            Border::Replicate => {
                (x.clamp(0, self.width as isize - 1) as usize, y.clamp(0, self.height as isize - 1) as usize)
            }
        };
        let i = (y * self.width + x) * 3;
        [self.data[i] as f32, self.data[i + 1] as f32, self.data[i + 2] as f32]
    }

    /// Bilinear sample at a source coordinate where pixel centres lie on integers.
    #[inline]
    pub fn sample(&self, x: f32, y: f32, border: Border) -> [f32; 3] {
        let x0 = x.floor();
        let y0 = y.floor();
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let p00 = self.pixel(x0, y0, border);
        let p10 = self.pixel(x0 + 1, y0, border);
        let p01 = self.pixel(x0, y0 + 1, border);
        let p11 = self.pixel(x0 + 1, y0 + 1, border);
        let mut out = [0.0; 3];
        for c in 0..3 {
            let top = p00[c] + (p10[c] - p00[c]) * fx;
            let bottom = p01[c] + (p11[c] - p01[c]) * fx;
            out[c] = top + (bottom - top) * fy;
        }
        out
    }
}

#[derive(Clone, Copy)]
pub enum Border {
    Constant,
    Replicate,
}
