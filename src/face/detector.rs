use anyhow::Result;
use ort::session::Session;
use ort::value::Tensor;

use super::{FaceRect, crop_tensor};
use crate::image::RgbFrame;

const INPUT: usize = 128;
const SCORE_THRESHOLD: f32 = 0.5;
const MERGE_IOU: f32 = 0.3;

/// BlazeFace short-range anchors: 16x16 grid with 2 anchors and 8x8 grid with 6.
fn anchors() -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(896);
    for (size, repeats) in [(16usize, 2usize), (8, 6)] {
        for y in 0..size {
            for x in 0..size {
                for _ in 0..repeats {
                    out.push([(x as f32 + 0.5) / size as f32, (y as f32 + 0.5) / size as f32]);
                }
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

        // Decode every candidate above threshold: [cx, cy, w, h, 6 keypoints (x, y)].
        let mut candidates: Vec<(f32, [f32; 16])> = Vec::new();
        for (i, logit) in logits.iter().enumerate().take(self.anchors.len()) {
            let score = 1.0 / (1.0 + (-logit.clamp(-80.0, 80.0)).exp());
            if score < SCORE_THRESHOLD {
                continue;
            }
            let anchor = self.anchors[i];
            let mut b = [0.0f32; 16];
            for (j, v) in b.iter_mut().enumerate() {
                *v = raw[i * 16 + j] / INPUT as f32;
            }
            b[0] += anchor[0];
            b[1] += anchor[1];
            for k in 0..6 {
                b[4 + k * 2] += anchor[0];
                b[5 + k * 2] += anchor[1];
            }
            candidates.push((score, b));
        }
        drop(outputs);
        let Some(best) = candidates.iter().enumerate().max_by(|a, b| a.1.0.total_cmp(&b.1.0)).map(|(i, _)| i) else {
            return Ok(None);
        };

        // Weighted merge of every box overlapping the best one.
        let bounds = |b: &[f32; 16]| [b[0] - b[2] * 0.5, b[1] - b[3] * 0.5, b[0] + b[2] * 0.5, b[1] + b[3] * 0.5];
        let best_box = bounds(&candidates[best].1);
        let best_area = candidates[best].1[2].max(0.0) * candidates[best].1[3].max(0.0);
        let mut merged = [0.0f32; 16];
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
