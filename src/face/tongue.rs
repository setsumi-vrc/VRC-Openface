//! Optional tongue tracking: an ONNX model reads a square crop of the mouth and returns the
//! six tongue channels.
//!
//! No tongue model ships with this repository; a build can compile one in (see `build.rs`).
//! A compatible model takes `[1, C, S, S]` floats:
//! C = 1 (luma) or 3 (RGB) planes in [0, 1] of a crop S pixels wide, S read from the model
//! ([`OPEN_SIZE`] when the model leaves it open). It returns the six values of
//! [`TONGUE_CHANNELS`], in that order, in [0, 1].
//!
//! The crop is turned with the mouth corners and reaches below the lower lip, where a tongue
//! that sticks out ends up. It is sampled like the face crops.

use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use ort::session::Session;
use ort::value::Tensor;

#[cfg(embedded_tongue)]
use super::create_session_from_memory;
use super::{FaceRect, create_session, crop_tensor};
use crate::image::RgbFrame;

pub const TONGUE_CHANNELS: [&str; 6] =
    ["tongueOut", "tongueLeft", "tongueRight", "tongueUp", "tongueDown", "tongueRoll"];
const CHANNELS: usize = TONGUE_CHANNELS.len();
/// Crop side for a model whose input size is left open.
pub const OPEN_SIZE: usize = 112;
/// Crop side in mouth widths: room for the tongue on both sides and down past the lower lip.
const SIDE: f32 = 2.2;
/// How far the crop centre sits below the mouth centre, in crop sides.
const DROP: f32 = 0.18;
/// Mouth corners (the subject's right, then left) and chin.
const CORNERS: [usize; 2] = [61, 291];
const CHIN: usize = 152;

/// Where the mouth is in a frame.
struct Mouth {
    /// The square the model sees.
    rect: FaceRect,
}

impl Mouth {
    /// The mouth in a face's pixel landmarks, or `None` when they are missing, not finite or
    /// too close together to crop.
    fn find(landmarks: &[[f32; 3]]) -> Option<Self> {
        let at = |i: usize| landmarks.get(i).map(|p| [p[0], p[1]]).filter(|p| p.iter().all(|v| v.is_finite()));
        let (right, left, chin) = (at(CORNERS[0])?, at(CORNERS[1])?, at(CHIN)?);
        let across = [left[0] - right[0], left[1] - right[1]];
        let width = across[0].hypot(across[1]);
        if width < 4.0 {
            return None;
        }
        let centre = [(right[0] + left[0]) * 0.5, (right[1] + left[1]) * 0.5];
        // The crop's down is a quarter turn from the corner line; on a mirrored frame that
        // points away from the chin, and half a turn more puts the chin at the bottom again.
        let mut angle = across[1].atan2(across[0]);
        if (chin[1] - centre[1]) * across[0] - (chin[0] - centre[0]) * across[1] < 0.0 {
            angle += std::f32::consts::PI;
        }
        let (sin, cos) = angle.sin_cos();
        let side = width * SIDE;
        let drop = side * DROP;
        Some(Self {
            rect: FaceRect { cx: centre[0] - sin * drop, cy: centre[1] + cos * drop, side, angle },
        })
    }
}

pub struct TongueModel {
    session: Session,
    /// Colour planes and crop side the model takes.
    planes: usize,
    size: usize,
    /// Values of the last frame the model ran on, kept while the mouth cannot be cropped.
    last: [f32; CHANNELS],
}

impl TongueModel {
    /// Checks the model's input and output against the contract above, so an unsuitable file
    /// is reported when it is loaded rather than on every frame.
    fn new(session: Session) -> Result<Self> {
        let input = session.inputs().first().and_then(|i| i.dtype().tensor_shape()).context("no tensor input")?;
        let &[_, planes, height, width] = &input[..] else {
            bail!("the input must be [1, C, S, S], not {:?}", &input[..]);
        };
        ensure!(planes == 1 || planes == 3, "the input must have 1 or 3 colour planes, not {planes}");
        let size = match (height, width) {
            (h, w) if h == w && h > 0 => h as usize,
            (h, w) if h <= 0 && w <= 0 => OPEN_SIZE,
            (h, w) => bail!("the input must be square, not {w} x {h}"),
        };
        let output = session.outputs().first().and_then(|o| o.dtype().tensor_shape()).context("no tensor output")?;
        if output.iter().all(|&d| d > 0) {
            let count: i64 = output.iter().product();
            ensure!(count == CHANNELS as i64, "the output must have {CHANNELS} values, not {count}");
        }
        Ok(Self { session, planes: planes as usize, size, last: [0.0; CHANNELS] })
    }

    pub fn load(path: &Path, gpu_device: Option<i32>) -> Result<Self> {
        Self::new(create_session(path, gpu_device)?)
    }

    /// The model compiled into the executable, if this build has one.
    #[cfg(embedded_tongue)]
    pub fn load_embedded(gpu_device: Option<i32>) -> Result<Self> {
        const BYTES: &[u8] = include_bytes!("../../models/tongue_model.onnx");
        Self::new(create_session_from_memory(BYTES, gpu_device)?)
    }

    /// Side of the square crop the model takes.
    pub fn input_size(&self) -> usize {
        self.size
    }

    pub fn reset(&mut self) {
        self.last = [0.0; CHANNELS];
    }

    pub fn update(&mut self, frame: &RgbFrame, pixel_landmarks: &[[f32; 3]]) -> Result<[f32; CHANNELS]> {
        let Some(mouth) = Mouth::find(pixel_landmarks) else {
            return Ok(self.last);
        };
        let input = self.planar(&crop_tensor(frame, mouth.rect, self.size, false));
        let tensor = Tensor::from_array(([1usize, self.planes, self.size, self.size], input))?;
        let outputs = self.session.run(ort::inputs![tensor])?;
        let (_, values) = outputs[0].try_extract_tensor::<f32>()?;
        ensure!(values.len() == CHANNELS, "the tongue model returned {} values, not {CHANNELS}", values.len());
        ensure!(values.iter().all(|v| v.is_finite()), "the tongue model returned a value that is not a number");
        self.last = std::array::from_fn(|i| values[i].clamp(0.0, 1.0));
        Ok(self.last)
    }

    /// Interleaved RGB pixels as the model's planes: luma alone, or red, green and blue.
    fn planar(&self, rgb: &[f32]) -> Vec<f32> {
        let area = self.size * self.size;
        let mut out = vec![0.0; self.planes * area];
        for (i, px) in rgb.as_chunks::<3>().0.iter().enumerate() {
            if self.planes == 1 {
                out[i] = 0.299 * px[0] + 0.587 * px[1] + 0.114 * px[2];
            } else {
                for (plane, &v) in px.iter().enumerate() {
                    out[plane * area + i] = v;
                }
            }
        }
        out
    }
}

/// The crop a tongue model sees, `size` pixels wide, as an image to check the alignment.
pub fn mouth_crop(frame: &RgbFrame, pixel_landmarks: &[[f32; 3]], size: usize) -> Option<RgbFrame> {
    let mouth = Mouth::find(pixel_landmarks)?;
    let rgb = crop_tensor(frame, mouth.rect, size, false);
    Some(RgbFrame::new(size, size, rgb.iter().map(|v| (v * 255.0).round() as u8).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An upright face with an 80 pixel wide mouth, its lips 20 pixels apart.
    fn face(mirrored: bool) -> Vec<[f32; 3]> {
        let mut points = vec![[0.0; 3]; 478];
        let (right, left) = if mirrored { (360.0, 280.0) } else { (280.0, 360.0) };
        points[61] = [right, 300.0, 0.0];
        points[291] = [left, 300.0, 0.0];
        points[13] = [320.0, 296.0, 0.0];
        points[14] = [320.0, 316.0, 0.0];
        points[152] = [320.0, 380.0, 0.0];
        points
    }

    #[test]
    fn crop_reaches_below_the_mouth_with_the_chin_down() {
        for mirrored in [false, true] {
            let rect = Mouth::find(&face(mirrored)).unwrap().rect;
            assert!((rect.side - 80.0 * SIDE).abs() < 1e-3);
            assert!((rect.cx - 320.0).abs() < 1e-3 && rect.cy > 300.0, "centre {} {}", rect.cx, rect.cy);
            // The crop's down, (-sin, cos), points at the chin.
            assert!(rect.angle.cos() > 0.999, "mirrored {mirrored}: angle {}", rect.angle);
        }
    }

    #[test]
    fn missing_or_broken_landmarks_give_no_crop() {
        assert!(Mouth::find(&face(false)[..200]).is_none());
        let mut points = face(false);
        points[291][0] = f32::NAN;
        assert!(Mouth::find(&points).is_none());
    }
}
