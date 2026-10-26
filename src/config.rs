use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::processing::{Calibration, Smoothing};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub tracking_enabled: bool,
    pub camera_index: u32,
    pub resolution: (u32, u32),
    pub camera_fps: u32,
    pub flip_camera: bool,
    pub preview: bool,
    pub landmarks: bool,
    pub smoothing: Smoothing,
    pub symmetric: bool,
    pub tongue: bool,
    pub show_unused: bool,
    pub osc_host: String,
    pub osc_port: u16,
    /// Send every parameter as `FT/v2/<Name>` floats when VRChat's OSCQuery is not found.
    pub send_without_avatar: bool,
    /// DirectML adapter index; `None` runs inference on the CPU.
    pub gpu_device: Option<i32>,
    pub calibration: Calibration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tracking_enabled: true,
            camera_index: 0,
            resolution: (640, 480),
            camera_fps: 30,
            flip_camera: false,
            preview: true,
            landmarks: true,
            smoothing: Smoothing::Off,
            symmetric: false,
            tongue: false,
            show_unused: true,
            osc_host: "127.0.0.1".into(),
            osc_port: 9000,
            send_without_avatar: true,
            gpu_device: None,
            calibration: Calibration::default(),
        }
    }
}

/// `VRC_OPENFACE_CONFIG` overrides the settings file location.
fn path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("VRC_OPENFACE_CONFIG") {
        return Some(PathBuf::from(path));
    }
    ProjectDirs::from("", "", "VRC-Openface").map(|d| d.config_dir().join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        let Some(path) = path() else { return Self::default() };
        let mut settings: Settings =
            fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        settings.calibration.gains.resize(crate::channels::COUNT, 1.0);
        settings
    }

    pub fn save(&self) -> Result<()> {
        let path = path().context("no config directory")?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&path, serde_json::to_string_pretty(self)?).with_context(|| format!("writing {}", path.display()))
    }
}
