//! Runs the face pipeline on a still image and prints the blendshapes as JSON.
//!
//! cargo run --release --example still_image -- path/to/face.jpg

use std::path::Path;

use vrc_openface::channels::MEDIAPIPE;
use vrc_openface::face::FaceLandmarker;
use vrc_openface::image::RgbFrame;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).expect("usage: still_image <image>");
    let image = image::open(&path)?.to_rgb8();
    let (width, height) = image.dimensions();
    let frame = RgbFrame::new(width as usize, height as usize, image.into_raw());

    let models = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let mut landmarker = FaceLandmarker::load(&models, None)?;
    // A few passes let the landmark crop settle the way it would on video.
    let mut result = None;
    for t in 0..5 {
        result = landmarker.detect(&frame, t * 33)?;
    }
    let Some(face) = result else {
        println!("null");
        return Ok(());
    };
    let shapes: serde_json::Map<String, serde_json::Value> =
        MEDIAPIPE.iter().zip(face.blendshapes).map(|(n, v)| (n.to_string(), v.into())).collect();
    let nose = face.landmarks[1];
    println!("{}", serde_json::json!({ "blendshapes": shapes, "nose": nose }));
    Ok(())
}
