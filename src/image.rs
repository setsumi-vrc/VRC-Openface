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

    /// Three passes of a separable box blur (close to Gaussian), used to blur the preview.
    pub fn blur(&mut self, radius: usize) {
        if radius == 0 || self.width == 0 || self.height == 0 {
            return;
        }
        let mut tmp = vec![0u8; self.data.len()];
        for _ in 0..3 {
            box_pass(&self.data, &mut tmp, self.width, self.height, radius, true);
            box_pass(&tmp, &mut self.data, self.width, self.height, radius, false);
        }
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

/// One box-blur pass along rows (`horizontal`) or columns, clamping at the edges.
fn box_pass(src: &[u8], dst: &mut [u8], width: usize, height: usize, radius: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (height, width) } else { (width, height) };
    let at = |line: usize, i: usize| if horizontal { (line * width + i) * 3 } else { (i * width + line) * 3 };
    let window = (2 * radius + 1) as u32;
    // prefix[c][j] = sum of the edge-clamped line from position -radius up to j - radius - 1.
    let mut prefix = vec![[0u32; 3]; len + 2 * radius + 1];
    for line in 0..lines {
        for j in 1..prefix.len() {
            let k = (j - 1).saturating_sub(radius).min(len - 1);
            let p = at(line, k);
            for c in 0..3 {
                prefix[j][c] = prefix[j - 1][c] + src[p + c] as u32;
            }
        }
        for i in 0..len {
            let q = at(line, i);
            for c in 0..3 {
                dst[q + c] = ((prefix[i + 2 * radius + 1][c] - prefix[i][c]) / window) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_keeps_flat_images_flat_and_spreads_edges() {
        let mut flat = RgbFrame::new(5, 4, vec![100; 5 * 4 * 3]);
        flat.blur(2);
        assert!(flat.data.iter().all(|&v| v == 100));
        let mut data = vec![0u8; 9 * 3];
        data[4 * 3] = 255;
        let mut dot = RgbFrame::new(9, 1, data);
        dot.blur(1);
        assert!(dot.data[3 * 3] > 0 && dot.data[4 * 3] < 255);
    }
}
