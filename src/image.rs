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
        if self.width == 0 {
            return;
        }
        for line in self.data.chunks_exact_mut(self.width * 3) {
            line.as_chunks_mut::<3>().0.reverse();
        }
    }

    /// Nearest-neighbour downscale used for the UI preview only.
    pub fn downscale_to_width(&self, max_width: usize) -> RgbFrame {
        if self.width <= max_width || max_width == 0 {
            return self.clone();
        }
        let scale = max_width as f32 / self.width as f32;
        let height = ((self.height as f32 * scale).round() as usize).max(1);
        let source = |i: usize, len: usize| (((i as f32 + 0.5) / scale) as usize).min(len - 1);
        // Every output row reads the same source columns.
        let columns: Vec<usize> = (0..max_width).map(|x| source(x, self.width)).collect();
        let pixels = self.data.as_chunks::<3>().0;
        let mut data = Vec::with_capacity(max_width * height * 3);
        for y in 0..height {
            let row = &pixels[source(y, self.height) * self.width..];
            data.extend(columns.iter().flat_map(|&x| row[x]));
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
    // Rounded rather than truncated, so three passes do not darken the image.
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
                dst[q + c] = ((prefix[i + 2 * radius + 1][c] - prefix[i][c] + window / 2) / window) as u8;
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

    #[test]
    fn mirror_and_downscale_keep_pixels() {
        let mut frame = RgbFrame::new(3, 1, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        frame.flip_horizontal();
        assert_eq!(frame.data, [7, 8, 9, 4, 5, 6, 1, 2, 3]);
        let wide = RgbFrame::new(4, 2, (0..24).collect());
        let small = wide.downscale_to_width(2);
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(small.data, [15, 16, 17, 21, 22, 23]);
    }
}
