#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use vrc_openface::config::Settings;
use vrc_openface::tracker::{self, Shared, Snapshot};
use vrc_openface::{app, oscquery};

/// Models are looked up next to the executable first, then in the working directory,
/// then in the source tree for `cargo run`.
fn model_dir() -> PathBuf {
    let candidates = [
        std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("models"))),
        std::env::current_dir().ok().map(|d| d.join("models")),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("models")),
    ];
    candidates
        .iter()
        .flatten()
        .find(|d| d.join("face_landmarks_detector.onnx").is_file())
        .cloned()
        .unwrap_or_else(|| PathBuf::from("models"))
}

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let shared =
        Arc::new(Shared { settings: RwLock::new(Settings::load()), snapshot: Mutex::new(Snapshot::default()) });
    let discovery = oscquery::spawn();
    let models = model_dir();
    log::info!("models: {}", models.display());

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("VRC Openface")
            .with_inner_size([1480.0, 940.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "VRC Openface",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let commands = tracker::spawn(shared.clone(), discovery, models, move || ctx.request_repaint());
            Ok(Box::new(app::App::new(cc, shared, commands)))
        }),
    )
}
