//! Per-frame processing of raw channel scores, in order: frown source selection,
//! One-Euro smoothing, per-avatar calibration, symmetry, then gaze extraction.

use std::collections::{BTreeMap, BTreeSet};
use std::f32::consts::PI;

use serde::{Deserialize, Serialize};

use crate::channels::{self, ASYMMETRIC_BY_DEFAULT, COUNT, SYMMETRY_GROUPS};

/// Mesh frown metric range that maps to 0..1 (corner drop relative to mouth width).
const FROWN_METRIC_RANGE: (f32, f32) = (-0.03, -0.15);
const CAPTURE_FRAMES: usize = 45;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Model,
    Mesh,
}

/// Channels with a geometric (mesh) estimate as an alternative to the model's score.
pub fn has_mesh_source(index: usize) -> bool {
    matches!(channels::name(index), "mouthFrownLeft" | "mouthFrownRight")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessingSettings {
    pub symmetric_face: bool,
    /// Per-group overrides of the global symmetric switch.
    pub symmetry: BTreeMap<String, bool>,
    /// Global One-Euro strength in [0, 1]; 0 is off.
    pub smoothing: f32,
    pub smoothing_channels: BTreeMap<String, f32>,
    pub sources: BTreeMap<String, Source>,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        let sources =
            ["mouthFrownLeft", "mouthFrownRight"].into_iter().map(|c| (c.to_string(), Source::Mesh)).collect();
        Self {
            symmetric_face: false,
            symmetry: BTreeMap::new(),
            smoothing: 0.0,
            smoothing_channels: BTreeMap::new(),
            sources,
        }
    }
}

impl ProcessingSettings {
    pub fn group_symmetric(&self, group: &str) -> bool {
        self.symmetry.get(group).copied().unwrap_or(self.symmetric_face && !ASYMMETRIC_BY_DEFAULT.contains(&group))
    }

    pub fn smoothing_for(&self, index: usize) -> f32 {
        self.smoothing_channels.get(channels::name(index)).copied().unwrap_or(self.smoothing)
    }

    pub fn source(&self, index: usize) -> Source {
        if !has_mesh_source(index) {
            return Source::Model;
        }
        self.sources.get(channels::name(index)).copied().unwrap_or_default()
    }
}

/// Calibration of one input channel for one avatar.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelCalibration {
    pub min: f32,
    pub max: f32,
    pub override_value: Option<f32>,
    pub override_disabled: bool,
}

impl Default for ChannelCalibration {
    fn default() -> Self {
        Self { min: 0.0, max: 1.0, override_value: None, override_disabled: false }
    }
}

impl ChannelCalibration {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn overriding(&self) -> Option<f32> {
        self.override_value.filter(|_| !self.override_disabled)
    }

    fn apply(&self, value: f32) -> f32 {
        if let Some(v) = self.overriding() {
            return v;
        }
        let span = (self.max - self.min).max(1e-3);
        ((value - self.min) / span).clamp(0.0, 1.0)
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

/// Everything calibrated for one avatar.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AvatarCalibration {
    pub channels: BTreeMap<String, ChannelCalibration>,
    /// Left channels whose pair is calibrated separately; pairs are linked by default.
    pub unlinked: BTreeSet<String>,
    pub gaze_center: Option<Gaze>,
    /// Calibrations replaced by the last neutral capture, for undo.
    pub neutral_undo: Option<BTreeMap<String, ChannelCalibration>>,
}

impl AvatarCalibration {
    pub fn get(&self, index: usize) -> ChannelCalibration {
        self.channels.get(channels::name(index)).copied().unwrap_or_default()
    }

    fn pair_key(index: usize) -> Option<String> {
        let n = channels::name(index);
        channels::mirror(index).map(|_| n.strip_suffix("Right").map(|b| format!("{b}Left")).unwrap_or(n.to_string()))
    }

    pub fn linked(&self, index: usize) -> bool {
        Self::pair_key(index).is_some_and(|k| !self.unlinked.contains(&k))
    }

    pub fn set_linked(&mut self, index: usize, linked: bool) {
        let Some(key) = Self::pair_key(index) else { return };
        if linked {
            self.unlinked.remove(&key);
            // Linking copies this side's calibration to the other side.
            let value = self.get(index);
            if let Some(other) = channels::mirror(index) {
                self.put(other, value);
            }
        } else {
            self.unlinked.insert(key);
        }
    }

    fn put(&mut self, index: usize, value: ChannelCalibration) {
        let name = channels::name(index).to_string();
        if value.is_default() {
            self.channels.remove(&name);
        } else {
            self.channels.insert(name, value);
        }
    }

    /// Sets a channel's calibration, mirrored to its pair while linked.
    pub fn set(&mut self, index: usize, value: ChannelCalibration) {
        self.put(index, value);
        if self.linked(index)
            && let Some(other) = channels::mirror(index)
        {
            self.put(other, value);
        }
    }

    pub fn undo_neutral(&mut self) -> bool {
        let Some(undo) = self.neutral_undo.take() else { return false };
        for (name, value) in undo {
            if let Some(i) = (0..COUNT).find(|&i| channels::name(i) == name) {
                self.put(i, value);
            }
        }
        true
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

enum Capture {
    Neutral,
    Gaze,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureOutcome {
    NeutralSaved,
    GazeSaved,
    NeutralRejected,
}

pub struct Processed {
    /// Smoothed values before calibration, shown as "current input" in the editor.
    pub input: [f32; COUNT],
    pub values: [f32; COUNT],
    pub gaze: Gaze,
}

pub struct Processor {
    filters: Option<[OneEuro; COUNT]>,
    last_ms: u64,
    capture: Option<(Capture, usize, Vec<f32>)>,
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

    pub fn capture_progress(&self) -> Option<f32> {
        self.capture.as_ref().map(|(_, n, _)| *n as f32 / CAPTURE_FRAMES as f32)
    }

    /// `frown_metric` is the mesh estimate (left, right) for the current face.
    pub fn process(
        &mut self,
        raw: &[f32; COUNT],
        frown_metric: Option<(f32, f32)>,
        timestamp_ms: u64,
        settings: &ProcessingSettings,
        calibration: &mut AvatarCalibration,
    ) -> (Processed, Option<CaptureOutcome>) {
        let mut v = *raw;
        if let Some((left, right)) = frown_metric {
            let (lo, hi) = FROWN_METRIC_RANGE;
            for (name, metric) in [("mouthFrownLeft", left), ("mouthFrownRight", right)] {
                let i = channels::index(name);
                if settings.source(i) == Source::Mesh {
                    v[i] = ((metric - lo) / (hi - lo)).clamp(0.0, 1.0);
                }
            }
        }
        self.smooth(&mut v, timestamp_ms, settings);
        let input = v;
        let outcome = self.feed_capture(&input, calibration);

        for (i, value) in v.iter_mut().enumerate().skip(1) {
            let c = calibration.get(i);
            if !c.is_default() {
                *value = c.apply(*value);
            }
        }
        for (group, pairs) in SYMMETRY_GROUPS {
            if !settings.group_symmetric(group) {
                continue;
            }
            for (a, b) in pairs {
                let (a, b) = (channels::index(a), channels::index(b));
                let mean = (v[a] + v[b]) * 0.5;
                v[a] = mean;
                v[b] = mean;
            }
        }

        let mut gaze = Gaze::from_channels(&v);
        if let Some(center) = calibration.gaze_center {
            gaze = gaze.minus(center);
        }
        (Processed { input, values: v, gaze }, outcome)
    }

    fn feed_capture(&mut self, input: &[f32; COUNT], calibration: &mut AvatarCalibration) -> Option<CaptureOutcome> {
        let (kind, n, sum) = self.capture.as_mut()?;
        match kind {
            Capture::Neutral => sum.iter_mut().zip(input).for_each(|(s, v)| *s += v),
            Capture::Gaze => {
                let g = Gaze::from_channels(input);
                for (s, v) in sum.iter_mut().zip([g.left_x, g.left_y, g.right_x, g.right_y]) {
                    *s += v;
                }
            }
        }
        *n += 1;
        if *n < CAPTURE_FRAMES {
            return None;
        }
        let mean: Vec<f32> = sum.iter().map(|s| s / *n as f32).collect();
        let outcome = match kind {
            Capture::Neutral => {
                let blink = (mean[channels::index("eyeBlinkLeft")] + mean[channels::index("eyeBlinkRight")]) * 0.5;
                if blink > 0.5 {
                    CaptureOutcome::NeutralRejected
                } else {
                    // Each channel's minimum moves to its resting value.
                    let mut undo = BTreeMap::new();
                    for (i, &resting) in mean.iter().enumerate().skip(1) {
                        if channels::is_eye_look(i) || channels::is_tongue(i) {
                            continue;
                        }
                        let previous = calibration.get(i);
                        undo.insert(channels::name(i).to_string(), previous);
                        let min = resting.clamp(0.0, (previous.max - 0.05).max(0.0));
                        calibration.put(i, ChannelCalibration { min, ..previous });
                    }
                    calibration.neutral_undo = Some(undo);
                    CaptureOutcome::NeutralSaved
                }
            }
            Capture::Gaze => {
                calibration.gaze_center =
                    Some(Gaze { left_x: mean[0], left_y: mean[1], right_x: mean[2], right_y: mean[3] });
                CaptureOutcome::GazeSaved
            }
        };
        self.capture = None;
        Some(outcome)
    }

    fn smooth(&mut self, v: &mut [f32; COUNT], timestamp_ms: u64, settings: &ProcessingSettings) {
        let frequency = 1000.0 / timestamp_ms.saturating_sub(self.last_ms).max(1) as f32;
        self.last_ms = timestamp_ms;
        let Some(filters) = &mut self.filters else {
            self.filters = Some(std::array::from_fn(|i| OneEuro { value: v[i], derivative: 0.0 }));
            return;
        };
        let alpha_d = alpha(frequency, 1.0);
        for (i, (value, f)) in v.iter_mut().zip(filters.iter_mut()).enumerate() {
            let strength = settings.smoothing_for(i).clamp(0.0, 1.0);
            let derivative = (*value - f.value) * frequency;
            f.derivative += alpha_d * (derivative - f.derivative);
            if strength <= 0.0 {
                f.value = *value;
                continue;
            }
            // Strength 0..1 moves the minimum cutoff from 8 Hz down to 0.25 Hz.
            let min_cutoff = 8.0 * (0.25f32 / 8.0).powf(strength);
            let a = alpha(frequency, min_cutoff + 1.5 * f.derivative.abs());
            f.value += a * (*value - f.value);
            *value = f.value;
        }
    }
}

/// Persistent calibrations, keyed by avatar id ("" when no avatar is known).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CalibrationStore {
    pub avatars: BTreeMap<String, AvatarCalibration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_pairs_mirror_calibration() {
        let mut c = AvatarCalibration::default();
        let left = channels::index("eyeBlinkLeft");
        let right = channels::index("eyeBlinkRight");
        c.set(left, ChannelCalibration { min: 0.2, ..Default::default() });
        assert_eq!(c.get(right).min, 0.2);
        c.set_linked(right, false);
        c.set(left, ChannelCalibration { min: 0.4, ..Default::default() });
        assert_eq!(c.get(right).min, 0.2);
    }

    #[test]
    fn calibration_rescales_and_overrides() {
        let c = ChannelCalibration { min: 0.2, max: 0.6, ..Default::default() };
        assert!((c.apply(0.4) - 0.5).abs() < 1e-6);
        let o = ChannelCalibration { override_value: Some(0.7), ..Default::default() };
        assert_eq!(o.apply(0.1), 0.7);
    }
}
