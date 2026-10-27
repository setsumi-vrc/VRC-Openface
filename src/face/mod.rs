//! MediaPipe face landmarker pipeline running on ONNX Runtime.
//!
//! The three networks are the stock MediaPipe `face_landmarker.task` models converted
//! with tf2onnx (see `tools/convert_models.py`):
//!
//! 1. `face_detector`        – BlazeFace short range, 128x128 NHWC input, 896 anchors
//! 2. `face_landmarks_detector` – 478 3D landmarks from a 256x256 rotated face crop
//! 3. `face_blendshapes`    – 52 ARKit-style blendshape scores from 146 landmark xy pairs
//!
//! Once a face is found, the next crop is derived from the previous landmarks so the
//! detector only runs when tracking is lost.

mod detector;
mod filter;
mod tongue;

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use ort::session::Session;
use ort::value::Tensor;

use crate::image::{Border, RgbFrame};
use detector::FaceDetector;
use filter::LandmarkFilter;
pub use tongue::{TONGUE_CHANNELS, TongueModel, mouth_crop};

pub const LANDMARK_COUNT: usize = 478;
pub const BLENDSHAPE_COUNT: usize = 52;
const LANDMARK_INPUT: usize = 256;

/// Landmark subset fed to the blendshape network (MediaPipe `kLandmarksSubsetIdxs`).
const BLENDSHAPE_POINTS: [usize; 146] = [
    0, 1, 4, 5, 6, 7, 8, 10, 13, 14, 17, 21, 33, 37, 39, 40, 46, 52, 53, 54, 55, 58, 61, 63, 65, 66, 67, 70, 78, 80,
    81, 82, 84, 87, 88, 91, 93, 95, 103, 105, 107, 109, 127, 132, 133, 136, 144, 145, 146, 148, 149, 150, 152, 153,
    154, 155, 157, 158, 159, 160, 161, 162, 163, 168, 172, 173, 176, 178, 181, 185, 191, 195, 197, 234, 246, 249, 251,
    263, 267, 269, 270, 276, 282, 283, 284, 285, 288, 291, 293, 295, 296, 297, 300, 308, 310, 311, 312, 314, 317, 318,
    321, 323, 324, 332, 334, 336, 338, 356, 361, 362, 365, 373, 374, 375, 377, 378, 379, 380, 381, 382, 384, 385, 386,
    387, 388, 389, 390, 397, 398, 400, 402, 405, 409, 415, 454, 466, 468, 469, 470, 471, 472, 473, 474, 475, 476, 477,
];

/// Oriented square region in pixel space: centre, side length and rotation (radians).
#[derive(Clone, Copy, Debug)]
pub struct FaceRect {
    pub cx: f32,
    pub cy: f32,
    pub side: f32,
    pub angle: f32,
}

impl FaceRect {
    fn is_valid(&self) -> bool {
        self.cx.is_finite()
            && self.cy.is_finite()
            && self.side.is_finite()
            && self.angle.is_finite()
            && self.side >= 2.0
    }
}

pub struct FaceResult {
    /// Landmarks normalised by (width, height, width), like MediaPipe's NormalizedLandmark.
    pub landmarks: Vec<[f32; 3]>,
    /// Landmarks in pixel coordinates, used for the tongue mouth crop.
    pub pixel_landmarks: Vec<[f32; 3]>,
    pub blendshapes: [f32; BLENDSHAPE_COUNT],
}

/// ONNX Runtime allows a single environment per process; every session shares it.
fn environment() -> Result<&'static ort::environment::Environment> {
    static ENVIRONMENT: OnceLock<ort::environment::Environment> = OnceLock::new();
    if let Some(env) = ENVIRONMENT.get() {
        return Ok(env);
    }
    let env = ort::init().with_name("vrc-openface").build().context("initialising ONNX Runtime")?;
    Ok(ENVIRONMENT.get_or_init(|| env))
}

pub fn create_session(path: &Path, gpu_device: Option<i32>) -> Result<Session> {
    session_builder(gpu_device)?.commit_from_file(path).with_context(|| format!("loading model {}", path.display()))
}

/// A model compiled into the executable.
pub fn create_session_from_memory(bytes: &[u8], gpu_device: Option<i32>) -> Result<Session> {
    session_builder(gpu_device)?.commit_from_memory(bytes).context("loading the built-in model")
}

fn session_builder(gpu_device: Option<i32>) -> Result<ort::session::builder::SessionBuilder> {
    let mut builder = Session::builder(environment()?)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .with_intra_threads(2)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .with_inter_threads(1)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .with_config_entry("session.intra_op.allow_spinning", "0")
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .with_config_entry("session.inter_op.allow_spinning", "0")
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(device) = gpu_device {
        builder = with_gpu(builder, device)?;
    }
    Ok(builder)
}

#[cfg(feature = "directml")]
fn with_gpu(
    builder: ort::session::builder::SessionBuilder,
    device: i32,
) -> Result<ort::session::builder::SessionBuilder> {
    builder
        .with_execution_providers([ort::ep::DirectML::default().with_device_id(device).build().error_on_failure()])
        .map_err(|e| anyhow::anyhow!("DirectML unavailable: {e}"))
}

#[cfg(not(feature = "directml"))]
fn with_gpu(
    _builder: ort::session::builder::SessionBuilder,
    _device: i32,
) -> Result<ort::session::builder::SessionBuilder> {
    bail!("this build was compiled without the `directml` feature")
}

/// Builds the NHWC float tensor for a rotated square crop.
///
/// `detector` selects BlazeFace preprocessing ([-1, 1] range, zero border) instead of
/// the landmark model's ([0, 1] range, replicated border).
fn crop_tensor(frame: &RgbFrame, rect: FaceRect, size: usize, detector: bool) -> Vec<f32> {
    let (sin, cos) = rect.angle.sin_cos();
    let scale = rect.side / size as f32;
    // Destination -> source affine map (OpenCV WARP_INVERSE_MAP semantics).
    let m00 = cos * scale;
    let m01 = -sin * scale;
    let m02 = rect.cx - (cos - sin) * rect.side * 0.5;
    let m10 = sin * scale;
    let m11 = cos * scale;
    let m12 = rect.cy - (sin + cos) * rect.side * 0.5;
    let border = if detector { Border::Constant } else { Border::Replicate };
    let mut out = Vec::with_capacity(size * size * 3);
    for v in 0..size {
        for u in 0..size {
            let (uf, vf) = (u as f32, v as f32);
            let px = frame.sample(m00 * uf + m01 * vf + m02, m10 * uf + m11 * vf + m12, border);
            for c in px {
                out.push(if detector { c / 127.5 - 1.0 } else { c / 255.0 });
            }
        }
    }
    out
}

/// Next-frame crop from the current landmarks: bounding square scaled 1.5x,
/// rotated so the eye corners (33 -> 263) are horizontal.
fn landmarks_rect(points: &[[f32; 3]]) -> FaceRect {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in points {
        for i in 0..2 {
            lo[i] = lo[i].min(p[i]);
            hi[i] = hi[i].max(p[i]);
        }
    }
    let dx = points[263][0] - points[33][0];
    let dy = points[263][1] - points[33][1];
    FaceRect {
        cx: (lo[0] + hi[0]) * 0.5,
        cy: (lo[1] + hi[1]) * 0.5,
        side: (hi[0] - lo[0]).max(hi[1] - lo[1]) * 1.5,
        angle: dy.atan2(dx),
    }
}

pub struct FaceLandmarker {
    detector: FaceDetector,
    landmarks: Session,
    blendshapes: Session,
    rect: Option<FaceRect>,
    image_size: (usize, usize),
    filter: LandmarkFilter,
}

impl FaceLandmarker {
    pub fn load(model_dir: &Path, gpu_device: Option<i32>) -> Result<Self> {
        Ok(Self {
            detector: FaceDetector::new(create_session(&model_dir.join("face_detector.onnx"), gpu_device)?),
            landmarks: create_session(&model_dir.join("face_landmarks_detector.onnx"), gpu_device)?,
            blendshapes: create_session(&model_dir.join("face_blendshapes.onnx"), gpu_device)?,
            rect: None,
            image_size: (0, 0),
            filter: LandmarkFilter::default(),
        })
    }

    fn lose_track(&mut self) {
        self.rect = None;
        self.filter = LandmarkFilter::default();
    }

    pub fn detect(&mut self, frame: &RgbFrame, timestamp_ms: u64) -> Result<Option<FaceResult>> {
        let (width, height) = (frame.width, frame.height);
        if self.image_size != (width, height) {
            self.lose_track();
            self.image_size = (width, height);
        }
        let rect = match self.rect {
            Some(rect) => Some(rect),
            None => self.detector.detect(frame)?,
        };
        let Some(rect) = rect.filter(FaceRect::is_valid) else {
            self.lose_track();
            return Ok(None);
        };

        let input = crop_tensor(frame, rect, LANDMARK_INPUT, false);
        let tensor = Tensor::from_array(([1usize, LANDMARK_INPUT, LANDMARK_INPUT, 3], input))?;
        let outputs = self.landmarks.run(ort::inputs![tensor])?;
        let (_, raw) = outputs[0].try_extract_tensor::<f32>()?;
        let (_, presence) = outputs[1].try_extract_tensor::<f32>()?;
        // The face-presence output is a logit: below zero means the crop lost the face.
        if raw.len() < LANDMARK_COUNT * 3
            || !raw.iter().all(|v| v.is_finite())
            || presence.first().is_none_or(|p| *p < 0.0)
        {
            drop(outputs);
            self.lose_track();
            return Ok(None);
        }

        // Crop space -> image pixel space.
        let (sin, cos) = rect.angle.sin_cos();
        let z_scale = rect.side * (cos.powi(2) + (width as f32 / height as f32 * sin).powi(2)).sqrt();
        let mut points: Vec<[f32; 3]> = raw[..LANDMARK_COUNT * 3]
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| {
                let x = (p[0] / LANDMARK_INPUT as f32 - 0.5) * rect.side;
                let y = (p[1] / LANDMARK_INPUT as f32 - 0.5) * rect.side;
                [x * cos - y * sin + rect.cx, x * sin + y * cos + rect.cy, p[2] / LANDMARK_INPUT as f32 * z_scale]
            })
            .collect();
        drop(outputs);

        self.rect = Some(landmarks_rect(&points));
        self.filter.apply(&mut points, timestamp_ms);

        let mut subset = Vec::with_capacity(BLENDSHAPE_POINTS.len() * 2);
        for &i in &BLENDSHAPE_POINTS {
            subset.extend_from_slice(&points[i][..2]);
        }
        let tensor = Tensor::from_array(([1usize, BLENDSHAPE_POINTS.len(), 2], subset))?;
        let outputs = self.blendshapes.run(ort::inputs![tensor])?;
        let (_, values) = outputs[0].try_extract_tensor::<f32>()?;
        if values.len() < BLENDSHAPE_COUNT || !values.iter().all(|v| v.is_finite()) {
            bail!("face model returned invalid blendshapes");
        }
        let mut blendshapes = [0.0; BLENDSHAPE_COUNT];
        for (dst, src) in blendshapes.iter_mut().zip(values) {
            *dst = src.clamp(0.0, 1.0);
        }

        let (w, h) = (width as f32, height as f32);
        let landmarks = points.iter().map(|p| [p[0] / w, p[1] / h, p[2] / w]).collect();
        Ok(Some(FaceResult { landmarks, pixel_landmarks: points, blendshapes }))
    }
}

/// Mesh frown estimate per mouth corner: how far each corner sits above (+) or below (-)
/// the centre line between the inner lips, relative to mouth width, measured in a frame
/// rotated so the eyes are level. Returns (left, right) from the user's perspective.
pub fn frown_metric(points: &[[f32; 3]]) -> Option<(f32, f32)> {
    if points.len() < LANDMARK_COUNT {
        return None;
    }
    let (right_eye, left_eye) = (points[33], points[263]);
    let roll = (left_eye[1] - right_eye[1]).atan2(left_eye[0] - right_eye[0]);
    let (sin, cos) = roll.sin_cos();
    // Vertical coordinate after removing head roll (image y grows downward).
    let level = |p: [f32; 3]| -p[0] * sin + p[1] * cos;
    let (right_corner, left_corner) = (points[61], points[291]);
    let width = ((left_corner[0] - right_corner[0]).powi(2) + (left_corner[1] - right_corner[1]).powi(2)).sqrt();
    if width < 1.0 {
        return None;
    }
    let centre = (level(points[13]) + level(points[14])) * 0.5;
    let metric = |corner: [f32; 3]| (centre - level(corner)) / width;
    Some((metric(left_corner), metric(right_corner)))
}
