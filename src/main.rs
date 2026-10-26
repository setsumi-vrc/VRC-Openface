#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use vrc_openface::config::Settings;
use vrc_openface::tracker::{self, Shared, Sources};
use vrc_openface::{app, avatar, oscquery, output, updater};

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
    updater::cleanup();

    let settings = Settings::load();
    let last_avatar = settings.last_avatar_id.clone();
    let shared = Arc::new(Shared::new(settings));
    let models = model_dir();
    log::info!("models: {}", models.display());
    let sources = Sources {
        discovery: oscquery::spawn(),
        receiver: avatar::spawn_receiver(shared.receive_port.clone(), last_avatar),
        feed: output::spawn(shared.perf.clone()),
    };

    let mut viewport = eframe::egui::ViewportBuilder::default();
    match eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(e) => log::warn!("window icon: {e}"),
    }
    let options = eframe::NativeOptions {
        viewport: viewport
            .with_title("VRC Openface")
            // The top bar draws its own window controls.
            .with_decorations(false)
            .with_inner_size([1480.0, 940.0])
            .with_min_inner_size([app::MIN_WINDOW.x, app::MIN_WINDOW.y]),
        // Window state lives with the rest of the app data, so `VRC_OPENFACE_DATA` test
        // runs keep their own and "Erase all app data" removes it too.
        persistence_path: Some(vrc_openface::config::data_dir().join("window")),
        ..Default::default()
    };
    eframe::run_native(
        "VRC Openface",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let commands = tracker::spawn(shared.clone(), sources, models.clone(), move || ctx.request_repaint());
            Ok(Box::new(app::App::new(cc, shared, commands, models)))
        }),
    )
}
