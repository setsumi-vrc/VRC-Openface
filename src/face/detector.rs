use anyhow::Result;
use ort::session::Session;
use ort::value::Tensor;

use super::{FaceRect, crop_tensor};
use crate::image::RgbFrame;

const INPUT: usize = 128;
const SCORE_THRESHOLD: f32 = 0.5;
const MERGE_IOU: f32 = 0.3;
/// Box centre and size followed by six keypoints, each (x, y).
const VALUES: usize = 16;

/// BlazeFace short-range anchors: 16x16 grid with 2 anchors and 8x8 grid with 6.
fn anchors() -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(16 * 16 * 2 + 8 * 8 * 6);
    for (size, repeats) in [(16usize, 2usize), (8, 6)] {
        let centre = |i: usize| (i as f32 + 0.5) / size as f32;
        for y in 0..size {
            for x in 0..size {
                out.extend(std::iter::repeat_n([centre(x), centre(y)], repeats));
            }
        }
    }
    out
}

pub struct FaceDetector {
    session: Session,
    anchors: Vec<[f32; 2]>,
}

impl FaceDetector {
    pub fn new(session: Session) -> Self {
        Self { session, anchors: anchors() }
    }

    /// Full-frame detection. Returns the landmark crop for the most confident face.
    pub fn detect(&mut self, frame: &RgbFrame) -> Result<Option<FaceRect>> {
        let (width, height) = (frame.width as f32, frame.height as f32);
        let side = width.max(height);
        let input = crop_tensor(frame, FaceRect { cx: width * 0.5, cy: height * 0.5, side, angle: 0.0 }, INPUT, true);
        let tensor = Tensor::from_array(([1usize, INPUT, INPUT, 3], input))?;
        let outputs = self.session.run(ort::inputs![tensor])?;
        let (_, raw) = outputs[0].try_extract_tensor::<f32>()?;
        let (_, logits) = outputs[1].try_extract_tensor::<f32>()?;

        if raw.len() < self.anchors.len() * VALUES || logits.len() < self.anchors.len() {
            anyhow::bail!("face detector returned {} boxes and {} scores", raw.len() / VALUES, logits.len());
        }

        // Decode every candidate above threshold: [cx, cy, w, h, 6 keypoints (x, y)].
        let mut candidates: Vec<(f32, [f32; VALUES])> = Vec::new();
        for ((anchor, logit), values) in self.anchors.iter().zip(logits.iter()).zip(raw.as_chunks::<VALUES>().0) {
            let score = 1.0 / (1.0 + (-logit.clamp(-80.0, 80.0)).exp());
            if score.is_nan() || score < SCORE_THRESHOLD {
                continue;
            }
            // The box size (2, 3) is absolute; every x and y is an offset from the anchor centre.
            let b: [f32; VALUES] = std::array::from_fn(|j| {
                let shift = match j {
                    2 | 3 => 0.0,
                    j => anchor[j % 2],
                };
                values[j] / INPUT as f32 + shift
            });
            candidates.push((score, b));
        }
        drop(outputs);
        let Some((_, best)) = candidates.iter().max_by(|a, b| a.0.total_cmp(&b.0)) else {
            return Ok(None);
        };

        // Weighted merge of every box overlapping the best one.
        let bounds = |b: &[f32; VALUES]| [b[0] - b[2] * 0.5, b[1] - b[3] * 0.5, b[0] + b[2] * 0.5, b[1] + b[3] * 0.5];
        let best_box = bounds(best);
        let best_area = best[2].max(0.0) * best[3].max(0.0);
        let mut merged = [0.0f32; VALUES];
        let mut total = 0.0f32;
        for (score, b) in &candidates {
            let bb = bounds(b);
            let ix = (bb[2].min(best_box[2]) - bb[0].max(best_box[0])).max(0.0);
            let iy = (bb[3].min(best_box[3]) - bb[1].max(best_box[1])).max(0.0);
            let intersection = ix * iy;
            let area = b[2].max(0.0) * b[3].max(0.0);
            let union = (area + best_area - intersection).max(1e-8);
            if intersection / union > MERGE_IOU {
                for (m, v) in merged.iter_mut().zip(b) {
                    *m += v * score;
                }
                total += score;
            }
        }
        if total <= 0.0 {
            return Ok(None);
        }
        for m in &mut merged {
            *m /= total;
        }

        // Keypoints 0 and 1 are the eyes; their direction gives the roll angle.
        let dx = merged[6] - merged[4];
        let dy = merged[7] - merged[5];
        Ok(Some(FaceRect {
            cx: (merged[0] - 0.5) * side + width * 0.5,
            cy: (merged[1] - 0.5) * side + height * 0.5,
            side: merged[2].max(merged[3]) * side * 1.5,
            angle: dy.atan2(dx),
        }))
    }
}
