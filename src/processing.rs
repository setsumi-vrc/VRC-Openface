//! Per-frame post-processing of raw channel scores: neutral calibration, per-channel
//! gain, symmetric face, smoothing and gaze extraction.

use std::f32::consts::PI;

use serde::{Deserialize, Serialize};

use crate::channels::{self, COUNT, SYMMETRY};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Smoothing {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl Smoothing {
    pub const ALL: [Smoothing; 4] = [Smoothing::Off, Smoothing::Low, Smoothing::Medium, Smoothing::High];

    pub fn label(self) -> &'static str {
        match self {
            Smoothing::Off => "Off",
            Smoothing::Low => "Low",
            Smoothing::Medium => "Medium",
            Smoothing::High => "High",
        }
    }

    /// One-euro minimum cutoff in Hz; lower is smoother.
    fn min_cutoff(self) -> Option<f32> {
        match self {
            Smoothing::Off => None,
            Smoothing::Low => Some(4.0),
            Smoothing::Medium => Some(1.5),
            Smoothing::High => Some(0.6),
        }
    }
}

/// Eye gaze in [-1, 1]: +x looks to the user's right, +y looks up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Gaze {
    pub left_x: f32,
    pub left_y: f32,
    pub right_x: f32,
    pub right_y: f32,
}

impl Gaze {
    fn from_channels(v: &[f32; COUNT]) -> Self {
        let c = |n: &str| v[channels::index(n)];
        Gaze {
            left_x: c("eyeLookInLeft") - c("eyeLookOutLeft"),
            left_y: c("eyeLookUpLeft") - c("eyeLookDownLeft"),
            right_x: c("eyeLookOutRight") - c("eyeLookInRight"),
            right_y: c("eyeLookUpRight") - c("eyeLookDownRight"),
        }
    }

    fn minus(self, o: Gaze) -> Gaze {
        Gaze {
            left_x: (self.left_x - o.left_x).clamp(-1.0, 1.0),
            left_y: (self.left_y - o.left_y).clamp(-1.0, 1.0),
            right_x: (self.right_x - o.right_x).clamp(-1.0, 1.0),
            right_y: (self.right_y - o.right_y).clamp(-1.0, 1.0),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Calibration {
    /// Resting score per channel, captured with "Capture neutral".
    pub neutral: Option<Vec<f32>>,
    /// Gaze reading while looking straight ahead, captured with "Center gaze".
    pub gaze_center: Option<Gaze>,
    pub gains: Vec<f32>,
}

impl Default for Calibration {
    fn default() -> Self {
        Self { neutral: None, gaze_center: None, gains: vec![1.0; COUNT] }
    }
}

#[derive(Clone, Copy)]
struct OneEuro {
    value: f32,
    derivative: f32,
}

fn alpha(frequency: f32, cutoff: f32) -> f32 {
    1.0 / (1.0 + frequency / (2.0 * PI * cutoff))
}

const CAPTURE_FRAMES: usize = 45;

enum Capture {
    Neutral,
    Gaze,
}

pub struct Processor {
    filters: Option<[OneEuro; COUNT]>,
    last_ms: u64,
    capture: Option<(Capture, usize, Vec<f32>)>,
}

pub struct Processed {
    pub values: [f32; COUNT],
    pub gaze: Gaze,
}

impl Default for Processor {
    fn default() -> Self {
        Self::new()
    }
}

impl Processor {
    pub fn new() -> Self {
        Self { filters: None, last_ms: 0, capture: None }
    }

    pub fn start_neutral_capture(&mut self) {
        self.capture = Some((Capture::Neutral, 0, vec![0.0; COUNT]));
    }

    pub fn start_gaze_capture(&mut self) {
        self.capture = Some((Capture::Gaze, 0, vec![0.0; 4]));
    }

    /// Progress of a running calibration capture in [0, 1].
    pub fn capture_progress(&self) -> Option<f32> {
        self.capture.as_ref().map(|(_, n, _)| *n as f32 / CAPTURE_FRAMES as f32)
    }

    pub fn process(
        &mut self,
        raw: &[f32; COUNT],
        timestamp_ms: u64,
        smoothing: Smoothing,
        symmetric: bool,
        calibration: &mut Calibration,
    ) -> Processed {
        let raw_gaze = Gaze::from_channels(raw);
        self.feed_capture(raw, raw_gaze, calibration);

        let mut v = *raw;
        if let Some(neutral) = calibration.neutral.as_deref().filter(|n| n.len() == COUNT) {
            for (i, value) in v.iter_mut().enumerate() {
                // Gaze channels are centred separately; tongue has no resting pose.
                if channels::is_eye_look(i) || channels::is_tongue(i) {
                    continue;
                }
                let n = neutral[i].clamp(0.0, 0.95);
                *value = ((*value - n) / (1.0 - n)).clamp(0.0, 1.0);
            }
        }
        for (value, gain) in v.iter_mut().zip(&calibration.gains) {
            *value = (*value * gain).clamp(0.0, 1.0);
        }
        if symmetric {
            for (a, b) in SYMMETRY {
                let (a, b) = (channels::index(a), channels::index(b));
                let mean = (v[a] + v[b]) * 0.5;
                v[a] = mean;
                v[b] = mean;
            }
        }
        self.smooth(&mut v, timestamp_ms, smoothing);

        let mut gaze = Gaze::from_channels(&v);
        if let Some(center) = calibration.gaze_center {
            gaze = gaze.minus(center);
        }
        Processed { values: v, gaze }
    }

    fn feed_capture(&mut self, raw: &[f32; COUNT], gaze: Gaze, calibration: &mut Calibration) {
        let Some((kind, n, sum)) = &mut self.capture else { return };
        match kind {
            Capture::Neutral => sum.iter_mut().zip(raw).for_each(|(s, v)| *s += v),
            Capture::Gaze => {
                for (s, g) in sum.iter_mut().zip([gaze.left_x, gaze.left_y, gaze.right_x, gaze.right_y]) {
                    *s += g;
                }
            }
        }
        *n += 1;
        if *n < CAPTURE_FRAMES {
            return;
        }
        let mean: Vec<f32> = sum.iter().map(|s| s / *n as f32).collect();
        match kind {
            Capture::Neutral => calibration.neutral = Some(mean),
            Capture::Gaze => {
                calibration.gaze_center =
                    Some(Gaze { left_x: mean[0], left_y: mean[1], right_x: mean[2], right_y: mean[3] })
            }
        }
        self.capture = None;
    }

    fn smooth(&mut self, v: &mut [f32; COUNT], timestamp_ms: u64, smoothing: Smoothing) {
        let Some(min_cutoff) = smoothing.min_cutoff() else {
            self.filters = None;
            return;
        };
        let frequency = 1000.0 / timestamp_ms.saturating_sub(self.last_ms).max(1) as f32;
        self.last_ms = timestamp_ms;
        let Some(filters) = &mut self.filters else {
            self.filters = Some(std::array::from_fn(|i| OneEuro { value: v[i], derivative: 0.0 }));
            return;
        };
        let alpha_d = alpha(frequency, 1.0);
        for (value, f) in v.iter_mut().zip(filters.iter_mut()) {
            let derivative = (*value - f.value) * frequency;
            f.derivative += alpha_d * (derivative - f.derivative);
            let a = alpha(frequency, min_cutoff + 0.8 * f.derivative.abs());
            f.value += a * (*value - f.value);
            *value = f.value;
        }
    }
}
