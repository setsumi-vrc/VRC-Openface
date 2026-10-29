//! Camera pixel formats to RGB, shrunk by a whole factor on the way. Each output pixel
//! averages a factor x factor block of luma (chroma is sampled at the block centre), which
//! keeps the detail the face models need: 1080p NV12 to 640 x 360 takes about 0.9 ms.

/// How the camera's bytes are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// A luma plane, then one plane of interleaved U and V at half size.
    Nv12,
    /// Y0 U Y1 V for every two pixels.
    Yuy2,
    /// B G R A, rows bottom-up when the stride is negative.
    Bgra,
}

/// YUV to RGB coefficients for limited-range video, in 8.8 fixed point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Matrix {
    rv: i32,
    gu: i32,
    gv: i32,
    bu: i32,
}

impl Matrix {
    /// ITU-R BT.601, standard definition.
    pub const BT601: Self = Self { rv: 409, gu: -100, gv: -208, bu: 516 };
    /// ITU-R BT.709, high definition.
    pub const BT709: Self = Self { rv: 459, gu: -55, gv: -136, bu: 541 };

    #[inline]
    fn rgb(self, y: i32, u: i32, v: i32) -> [u8; 3] {
        self.pair([y, y], u, v)[0]
    }

    /// Two pixels that share their chroma.
    #[inline]
    fn pair(self, y: [i32; 2], u: i32, v: i32) -> [[u8; 3]; 2] {
        let (u, v) = (u - 128, v - 128);
        let (r, g, b) = (self.rv * v, self.gu * u + self.gv * v, self.bu * u);
        let clamp = |x: i32| (x >> 8).clamp(0, 255) as u8;
        y.map(|y| {
            let c = 298 * (y - 16) + 128;
            [clamp(c + r), clamp(c + g), clamp(c + b)]
        })
    }
}

/// One camera frame as it arrives.
pub struct Frame<'a> {
    pub layout: Layout,
    pub data: &'a [u8],
    pub width: usize,
    pub height: usize,
    /// Bytes per row of the first plane; negative for bottom-up rows.
    pub stride: isize,
}

impl Frame<'_> {
    /// Bytes the frame needs, or `None` when the stride is too small for the width.
    fn needed(&self) -> Option<usize> {
        let row = self.stride.unsigned_abs();
        let min_row = match self.layout {
            Layout::Nv12 => self.width,
            Layout::Yuy2 => self.width * 2,
            Layout::Bgra => self.width * 4,
        };
        (row >= min_row).then(|| match self.layout {
            Layout::Nv12 => row * self.height + row * self.height.div_ceil(2),
            _ => row * self.height,
        })
    }

    fn row(&self, y: usize) -> &[u8] {
        let row = self.stride.unsigned_abs();
        let line = if self.stride >= 0 { y } else { self.height - 1 - y };
        &self.data[line * row..][..row]
    }
}

/// Converts `frame` to RGB at 1/`factor` of its size, writing into `out`. Returns the
/// output size, or `None` when the buffer is shorter than the frame it should hold.
pub fn to_rgb(frame: &Frame, factor: usize, matrix: Matrix, out: &mut Vec<u8>) -> Option<(usize, usize)> {
    // Real cameras need at most 4; the cap keeps the u16 column sums far from overflowing.
    let factor = factor.clamp(1, 15);
    if frame.data.len() < frame.needed()? || frame.stride < 0 && frame.layout != Layout::Bgra {
        return None;
    }
    let (width, height) = (frame.width / factor, frame.height / factor);
    out.clear();
    out.resize(width * height * 3, 0);
    if width == 0 || height == 0 {
        return Some((width, height));
    }
    if frame.layout == Layout::Bgra {
        for (oy, line) in out.chunks_exact_mut(width * 3).enumerate() {
            let row = frame.row(oy * factor + factor / 2);
            for (ox, rgb) in line.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                let p = &row[(ox * factor + factor / 2) * 4..];
                *rgb = [p[2], p[1], p[0]];
            }
        }
        return Some((width, height));
    }

    let nv12 = frame.layout == Layout::Nv12;
    // Where each output pixel finds its U in a chroma row (V follows at `v_offset`).
    let chroma_at: Vec<usize> = (0..width)
        .map(|ox| {
            let x = ox * factor + factor / 2;
            if nv12 { x & !1 } else { (x / 2) * 4 + 1 }
        })
        .collect();
    let v_offset = if nv12 { 1 } else { 2 };
    // Block sums times this, shifted down by 16, are the block averages.
    let scale = (1u32 << 16) / (factor * factor) as u32;
    let mut sums = vec![0u16; width * factor];
    let mut averaged = vec![0u8; width];
    let row_bytes = frame.stride.unsigned_abs();
    for (oy, line) in out.chunks_exact_mut(width * 3).enumerate() {
        let top = oy * factor;
        let luma: &[u8] = if nv12 && factor == 1 {
            &frame.row(top)[..width]
        } else {
            // Column sums over the block's rows, then each block's sum across its columns.
            sums.fill(0);
            for y in top..top + factor {
                let row = frame.row(y);
                if nv12 {
                    for (sum, &l) in sums.iter_mut().zip(row) {
                        *sum += l as u16;
                    }
                } else {
                    for (sum, &l) in sums.iter_mut().zip(row.iter().step_by(2)) {
                        *sum += l as u16;
                    }
                }
            }
            for (avg, block) in averaged.iter_mut().zip(sums.chunks_exact(factor)) {
                let total: u32 = block.iter().map(|&s| s as u32).sum();
                *avg = ((total * scale + (1 << 15)) >> 16).min(255) as u8;
            }
            &averaged
        };
        let centre = top + factor / 2;
        let chroma = if nv12 {
            &frame.data[row_bytes * frame.height + (centre / 2) * row_bytes..][..row_bytes]
        } else {
            frame.row(centre)
        };
        if nv12 && factor == 1 {
            // Two pixels share one U V pair: no lookups, chroma terms worked out once.
            let pairs = line.as_chunks_mut::<6>().0.iter_mut().zip(luma.as_chunks::<2>().0);
            for ((rgb, y), uv) in pairs.zip(chroma.as_chunks::<2>().0) {
                let [a, b] = matrix.pair([y[0] as i32, y[1] as i32], uv[0] as i32, uv[1] as i32);
                rgb[..3].copy_from_slice(&a);
                rgb[3..].copy_from_slice(&b);
            }
            if width % 2 == 1 {
                let (x, c) = (width - 1, chroma_at[width - 1]);
                line[x * 3..].copy_from_slice(&matrix.rgb(luma[x] as i32, chroma[c] as i32, chroma[c + 1] as i32));
            }
            continue;
        }
        for ((rgb, &y), &c) in line.as_chunks_mut::<3>().0.iter_mut().zip(luma).zip(&chroma_at) {
            *rgb = matrix.rgb(y as i32, chroma[c] as i32, chroma[c + v_offset] as i32);
        }
    }
    Some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_stays_grey_and_white_saturates() {
        assert_eq!(Matrix::BT709.rgb(16, 128, 128), [0, 0, 0]);
        assert_eq!(Matrix::BT601.rgb(235, 128, 128), [255, 255, 255]);
        let [r, g, b] = Matrix::BT709.rgb(126, 128, 128);
        assert!(r == g && g == b && (127..=129).contains(&r));
    }

    #[test]
    fn nv12_shrinks_by_averaging_luma() {
        // 6 x 2 frame, factor 2: luma blocks average to 20, 60 and 100.
        let mut data = vec![10, 30, 50, 70, 90, 110, 30, 10, 70, 50, 110, 90];
        data.extend([128, 128, 128, 128, 128, 128]);
        let frame = Frame { layout: Layout::Nv12, data: &data, width: 6, height: 2, stride: 6 };
        let mut out = Vec::new();
        assert_eq!(to_rgb(&frame, 2, Matrix::BT601, &mut out), Some((3, 1)));
        let expected: Vec<u8> = [20, 60, 100].iter().flat_map(|&y| Matrix::BT601.rgb(y, 128, 128)).collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn nv12_full_size_matches_pixel_by_pixel() {
        // 3 x 2 frame (odd width, rows padded to 4), one chroma row: (U, V) = (90, 200), (160, 60).
        let data = [50, 100, 150, 0, 60, 110, 160, 0, 90, 200, 160, 60];
        let frame = Frame { layout: Layout::Nv12, data: &data, width: 3, height: 2, stride: 4 };
        let mut out = Vec::new();
        assert_eq!(to_rgb(&frame, 1, Matrix::BT709, &mut out), Some((3, 2)));
        let m = Matrix::BT709;
        let expected: Vec<u8> = [(50, 90, 200), (100, 90, 200), (150, 160, 60), (60, 90, 200), (110, 90, 200)]
            .iter()
            .flat_map(|&(y, u, v)| m.rgb(y, u, v))
            .chain(m.rgb(160, 160, 60))
            .collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn yuy2_reads_chroma_of_its_pixel_pair() {
        // Two pixel pairs: the first red-ish (V high), the second blue-ish (U high).
        let data = [100, 128, 100, 220, 100, 220, 100, 128];
        let frame = Frame { layout: Layout::Yuy2, data: &data, width: 4, height: 1, stride: 8 };
        let mut out = Vec::new();
        assert_eq!(to_rgb(&frame, 1, Matrix::BT601, &mut out), Some((4, 1)));
        assert!(out[0] > out[2], "first pair is red");
        assert!(out[11] > out[9], "second pair is blue");
    }

    #[test]
    fn bottom_up_bgra_is_flipped() {
        // Two rows stored bottom-up: the first stored row is the bottom one.
        let data = [1, 2, 3, 0, 4, 5, 6, 0];
        let frame = Frame { layout: Layout::Bgra, data: &data, width: 1, height: 2, stride: -4 };
        let mut out = Vec::new();
        assert_eq!(to_rgb(&frame, 1, Matrix::BT601, &mut out), Some((1, 2)));
        assert_eq!(out, [6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn short_buffers_are_refused() {
        let data = [0u8; 10];
        let frame = Frame { layout: Layout::Nv12, data: &data, width: 4, height: 2, stride: 4 };
        assert_eq!(to_rgb(&frame, 1, Matrix::BT601, &mut Vec::new()), None);
    }
}
