//! Runs the face and tongue models on camera frames and saves the mouth crop the tongue
//! model sees, to check the alignment.
//!
//! cargo run --release --example tongue_check -- [camera index] [seconds] [out dir] [model]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use vrc_openface::camera::CameraSource;
use vrc_openface::face::{FaceLandmarker, TONGUE_CHANNELS, TongueModel, mouth_crop};
use vrc_openface::image::RgbFrame;

fn save(frame: &RgbFrame, path: &Path) -> Result<()> {
    let image = image::RgbImage::from_raw(frame.width as u32, frame.height as u32, frame.data.clone());
    image.context("frame size")?.save(path)?;
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let index: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(0);
    let seconds: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(15);
    let out = PathBuf::from(args.next().unwrap_or_else(|| ".".into()));
    let models = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let model = args.next().map(PathBuf::from).unwrap_or_else(|| models.join("tongue_model.onnx"));
    let mut landmarker = FaceLandmarker::load(&models, None)?;
    let mut tongue = TongueModel::load(&model, None)?;
    let mut source = CameraSource::open(index, (640, 480), 30)?;
    let started = Instant::now();
    let mut last_print = Instant::now();
    let mut best = -1.0f32;
    while started.elapsed() < Duration::from_secs(seconds) {
        let frame = source.read()?;
        let Some(face) = landmarker.detect(&frame, started.elapsed().as_millis() as u64)? else {
            continue;
        };
        let values = tongue.update(&frame, &face.pixel_landmarks)?;
        if values[0] > best {
            best = values[0];
            if let Some(crop) = mouth_crop(&frame, &face.pixel_landmarks, tongue.input_size()) {
                save(&crop, &out.join("crop_best.png"))?;
            }
            save(&frame, &out.join("frame_best.png"))?;
        }
        if last_print.elapsed() > Duration::from_millis(500) {
            last_print = Instant::now();
            let line: Vec<String> = TONGUE_CHANNELS.iter().zip(values).map(|(n, v)| format!("{n} {v:.2}")).collect();
            println!("{}", line.join("  "));
        }
    }
    println!("best tongueOut {best:.2}");
    Ok(())
}
