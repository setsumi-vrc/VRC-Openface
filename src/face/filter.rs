use std::f32::consts::PI;

const MIN_CUTOFF: f32 = 0.05;
const BETA: f32 = 80.0;
const DERIVATE_CUTOFF: f32 = 1.0;

/// One-euro filter over all landmark coordinates. Velocity is normalised by the face
/// size so the response does not depend on how close the user sits to the camera.
#[derive(Default)]
pub struct LandmarkFilter {
    raw: Vec<[f32; 3]>,
    filtered: Vec<[f32; 3]>,
    derivative: Vec<[f32; 3]>,
    timestamp: u64,
}

fn alpha(frequency: f32, cutoff: f32) -> f32 {
    1.0 / (1.0 + frequency / (2.0 * PI * cutoff))
}

impl LandmarkFilter {
    pub fn apply(&mut self, points: &mut [[f32; 3]], timestamp_ms: u64) {
        if self.raw.len() != points.len() {
            self.raw = points.to_vec();
            self.filtered = points.to_vec();
            self.derivative = vec![[0.0; 3]; points.len()];
            self.timestamp = timestamp_ms;
            return;
        }

        let frequency = 1000.0 / timestamp_ms.saturating_sub(self.timestamp).max(1) as f32;
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in points.iter() {
            for i in 0..2 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let scale = (((hi[0] - lo[0]) + (hi[1] - lo[1])) * 0.5).max(1e-6);
        let alpha_d = alpha(frequency, DERIVATE_CUTOFF);

        for (i, point) in points.iter_mut().enumerate() {
            #[allow(clippy::needless_range_loop)]
            for c in 0..3 {
                let derivative = (point[c] - self.raw[i][c]) * frequency / scale;
                let d = &mut self.derivative[i][c];
                *d += alpha_d * (derivative - *d);
                let a = alpha(frequency, MIN_CUTOFF + BETA * d.abs());
                let f = &mut self.filtered[i][c];
                *f += a * (point[c] - *f);
                self.raw[i][c] = point[c];
                point[c] = *f;
            }
        }
        self.timestamp = timestamp_ms;
    }
}
